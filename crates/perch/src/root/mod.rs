//! The app: one view that owns the pages, the stream slots, the follows
//! lists and the palette.
//!
//! `RootView` is one type because gpui's key dispatch, focus and event pumps
//! all want a single owner. It used to be one file too — two and a half
//! thousand lines — which is more than anyone, human or otherwise, holds at
//! once. The state and the render loop are here; everything else is an `impl
//! RootView` block in the sibling named for what it does:
//!
//! | module | role |
//! |---|---|
//! | `shortcuts` | what each key does |
//! | `commands` | the palette: what it offers, and running a row |
//! | `follows` | the Twitch worker's events: sign-in, who is live, replies |
//! | `browsing` | the browse page's requests: tabs, search, categories, channels |
//! | `streams` | opening, restarting and closing panes |
//! | `prefs` | the settings sheet, the divider drag, the rail |
//! | `chrome` | pills, toasts, the now-playing bar, the rail |
//! | `pages` | the two pages, assembled |
//!
//! A sibling reaches the fields directly — they are private to this module,
//! and a child module is inside it — so the split costs no accessors. Its
//! methods are `pub(super)`: callable from the rest of the root, and nowhere
//! else.

mod browsing;
mod chrome;
mod commands;
mod follows;
mod pages;
mod prefs;
mod shortcuts;
mod streams;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use emotes::ImageCache;
use gpui::{
    div, prelude::*, Context, Entity, FocusHandle, MouseButton, ScrollHandle, SharedString,
    Subscription, Task, Window,
};
use gpui_component::input::{InputEvent, InputState};
use settings::Settings;
use twitch_api::{FollowedChannel, LiveStream};

use crate::browse::{self, Discovery, SignIn};
use crate::layout::Body;
use crate::settings_view::SettingsPanel;
use crate::target::Target;
use crate::twitch::TwitchService;
use crate::video_view::VideoView;
use crate::watch::{ResizeStart, Slot, StreamState, MAX_PANES};
use crate::{cpu_log, keys, motion, sidebar, theme, vod, APP_NAME};

/// Emotes and thumbnails are reproducible, so they live in the platform's
/// cache directory rather than roaming with settings.
///
/// Each platform is asked by name rather than falling through to `temp_dir`,
/// which was the old behaviour everywhere `LOCALAPPDATA` was unset. On macOS
/// that resolves to a per-process `/var/folders/…/T` the OS prunes on a
/// schedule nobody chose, so every emote and thumbnail would quietly
/// re-download every so often. `temp_dir` is still the last resort, which is
/// what it was meant to be.
pub(crate) fn image_cache_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));

    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));

    base.unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
        .join("images")
}

/// A drag of the video/chat divider in progress.
struct Resize {
    start: ResizeStart,
    /// The sizes when the pointer went down, so the drag is measured from where
    /// it began rather than accumulated frame by frame — the second of those
    /// drifts, and drifts worst when the pointer is moving fastest.
    chat_width: f32,
    video_share: f32,
}

/// A transient notice. It owns its own fade because a toast that vanished
/// mid-sentence read as a dropped frame rather than as time passing.
struct Toast {
    id: u64,
    text: SharedString,
    fade: motion::Fade,
    /// What clicking it does, when it says something you can act on.
    action: Option<ToastAction>,
}

/// What a toast offers. A "went live" toast used to be a fact to read and
/// then go and find in the list; now it is the way there.
#[derive(Clone)]
enum ToastAction {
    /// Watch this channel: alone from the text, or beside what is playing
    /// from the `+ add` pill next to it.
    Watch(String),
}

/// A recording named by a link, waiting to be looked up. A link carries an
/// id and where to start and nothing the pane needs — no title, no channel —
/// and the lookup needs a signed-in session, so a link given at launch waits
/// here for sign-in to land. See `RootView::open_video_link`.
struct LinkedVideo {
    id: String,
    start_secs: Option<u64>,
    /// Sent to the worker; the answer arrives as `TwitchEvent::Video`.
    requested: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Page {
    Browse,
    Watch,
}

pub(crate) struct RootView {
    settings: Settings,
    settings_path: PathBuf,
    cache: Arc<ImageCache>,

    page: Page,
    slots: Vec<Slot>,

    follows: Vec<LiveStream>,
    /// Everyone followed, live or not. Kept apart from `follows` all the way to
    /// the screen; see `twitch_api::FollowedChannel`.
    offline: Vec<FollowedChannel>,
    /// A follows request the user asked for by hand is outstanding. Only their
    /// requests set this, so the minute-by-minute poll does not blink the
    /// control every time it runs.
    refreshing: bool,
    /// Login to profile picture, for the follows rail.
    ///
    /// Merged rather than replaced on each poll: the pictures arrive a moment
    /// after the list they belong to, and a rail that blanked itself every
    /// minute while it waited for them would be worse than one that is briefly
    /// out of date by one avatar.
    avatars: HashMap<String, String>,
    /// Whether a follows list has ever come back.
    ///
    /// The worker announces `SignedIn` before it polls anything, and the poll
    /// that follows walks up to ten pages twice. In that window the page had a
    /// signed-in session and two empty lists, which it read as an answer and
    /// said so: "Nobody is live — none of the channels you follow are streaming
    /// right now", on every launch, for as long as the request took.
    follows_loaded: bool,
    /// Who was live at the last poll, so newly-live channels can be told apart
    /// from ones that were already streaming. Without this every poll would
    /// re-announce everybody.
    known_live: HashSet<String>,
    sign_in: SignIn,
    /// Everything the browse page shows besides your follows.
    discovery: Discovery,
    search: Entity<InputState>,
    /// The Following tab's filter. Typed into, never sent anywhere: it narrows
    /// the two lists already on the page with the palette's own matcher.
    filter: Entity<InputState>,
    /// Scroll positions for the browse lists and the rail, held here so the
    /// scrollbars drawn over them read the same state the lists write.
    scrolls: browse::Scrolls,
    rail_scroll: ScrollHandle,
    twitch: TwitchService,
    _twitch_pump: Task<()>,

    /// Which pane the player shortcuts act on, held as a pane's key rather
    /// than an index: closing a pane reindexes every pane after it, and a
    /// stored index would quietly start acting on somebody else — the same
    /// trap that keys pane element ids on the key.
    active: Option<String>,
    /// Focus lives on the root and stays there. GPUI derives the whole key
    /// dispatch path from what is focused, and with nothing focused the context
    /// stack is empty — which fails every predicate, so no shortcut fires at
    /// all. Nothing else in the app wants focus except the text inputs, which
    /// take it on click and hand it back the same way.
    focus: FocusHandle,
    /// Focus is never reassigned when the focused element simply disappears —
    /// which is what happens when the settings sheet closes and takes its
    /// buttons with it. Without this, every shortcut stops working from then
    /// on, silently and for the rest of the session.
    _focus_lost: Subscription,

    /// `--volume`, if it was given. A session-wide override rather than a
    /// stored preference: someone starting the app quiet this once should not
    /// have that silently overwrite the level every channel remembers, and
    /// should not be ignored on the channels that remember one.
    volume_override: Option<u8>,

    /// A divider being dragged: where it started, and the sizes it started
    /// from.
    ///
    /// Held on the root rather than in the pane, because the pointer leaves the
    /// six-pixel handle on the first frame of any drag worth making — the move
    /// events that matter arrive at the window.
    resize: Option<Resize>,

    /// The command palette's box, kept for the life of the app rather than
    /// built per opening: it carries the subscription that runs a command on
    /// Enter, and re-subscribing on every `Ctrl+K` would stack those up.
    palette_input: Entity<InputState>,
    palette_open: bool,
    /// Which row Enter would run. An index into the entries computed at render,
    /// clamped there — the list changes under it on every keystroke.
    palette_selected: usize,

    settings_panel: Option<Entity<SettingsPanel>>,
    /// Whether the page navigation is up. It follows the video chrome rather
    /// than sitting there permanently: a control you never look at should not
    /// be on the picture for three hours.
    nav: motion::Fade,
    toasts: Vec<Toast>,
    next_toast: u64,
    _cache_pump: Task<()>,

    /// Recordings opened by link that are still to be looked up.
    linked_videos: Vec<LinkedVideo>,
    /// Which scheduled settings save is the newest; see `save_settings_soon`.
    save_epoch: u64,
    /// Which run of window resizes is the newest; see `on_window_resized`.
    resize_epoch: u64,
    /// Keeps the resize observer alive; a dropped `Subscription` unsubscribes.
    _bounds: Subscription,
}

impl RootView {
    pub(crate) fn new(
        targets: Vec<Target>,
        volume_override: Option<u8>,
        warnings: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Playlists an earlier run left behind; see `vod::sweep_scratch`.
        vod::sweep_scratch();

        let settings_path = settings::default_path(APP_NAME);
        let settings = Settings::load(&settings_path).unwrap_or_else(|e| {
            eprintln!("settings: {e}; using defaults");
            Settings::default()
        });

        // A cache directory that cannot be created is not a reason to have no
        // window. This runs before there is one, so a panic here was a release
        // build that silently never appeared; the temp directory is the last
        // resort, as it is for the path itself.
        let (cache, mut cache_ready) = match ImageCache::new(image_cache_dir()) {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("image cache: {e}; using the temp directory instead");
                ImageCache::new(std::env::temp_dir().join(APP_NAME).join("images"))
                    .expect("failed to open an image cache even in the temp directory")
            }
        };
        let cache = Arc::new(cache);

        // One pump for every image consumer. Repainting the root repaints its
        // children, so browse cards and chat emotes both pick up new images.
        let cache_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while cache_ready.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        let focus = cx.focus_handle();
        let _focus_lost = cx.on_focus_lost(window, |this: &mut Self, window, _cx| {
            this.focus.focus(window);
        });

        // A pane's quality is chosen against its size, and the window is the
        // one thing that changes that without going through this view.
        let _bounds = cx.observe_window_bounds(window, |this: &mut Self, window, cx| {
            this.on_window_resized(window, cx)
        });

        let (service, twitch_pump) = Self::spawn_twitch(settings_path.clone(), window, cx);

        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("search channels and categories"));
        // Searching on every keystroke would be three requests per letter.
        cx.subscribe(&search, |this: &mut RootView, state, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let query = state.read(cx).value().trim().to_string();
                this.run_search(query, cx);
            }
        })
        .detach();

        // The opposite of the search box: every keystroke, and nothing leaves
        // the app. See `browse::following_view`.
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("filter your follows"));
        cx.subscribe(&filter, |_: &mut RootView, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();

        let palette_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("channel, or a command"));
        cx.subscribe_in(
            &palette_input,
            window,
            |this: &mut RootView, _, event, window, cx| {
                match event {
                    // Retyping changes what is under the cursor, so the cursor
                    // goes back to the top rather than staying on a row that
                    // now means something else.
                    InputEvent::Change => this.palette_selected = 0,
                    InputEvent::PressEnter { .. } => this.run_selected_command(window, cx),
                    _ => {}
                }
                cx.notify();
            },
        )
        .detach();

        let mut view = Self {
            settings,
            settings_path,
            cache,
            page: Page::Browse,
            slots: Vec::new(),
            follows: Vec::new(),
            offline: Vec::new(),
            refreshing: false,
            avatars: HashMap::new(),
            follows_loaded: false,
            known_live: HashSet::new(),
            sign_in: SignIn::Connecting,
            discovery: Discovery::default(),
            search,
            filter,
            scrolls: browse::Scrolls::default(),
            rail_scroll: ScrollHandle::new(),
            twitch: service,
            _twitch_pump: twitch_pump,
            active: None,
            focus,
            _focus_lost,
            volume_override,
            resize: None,
            palette_input,
            palette_open: false,
            palette_selected: 0,
            settings_panel: None,
            nav: motion::Fade::hidden(),
            toasts: Vec::new(),
            next_toast: 0,
            _cache_pump: cache_pump,
            linked_videos: Vec::new(),
            save_epoch: 0,
            resize_epoch: 0,
            _bounds,
        };

        // Nothing else ever asks for focus, so this is what makes every
        // shortcut work — see the field.
        window.focus(&view.focus);

        // Only what was named on the command line opens a stream. Launching
        // straight into whatever was on last time means the app starts costing
        // CPU and bandwidth before anyone has asked it to.
        for (index, target) in targets.into_iter().take(MAX_PANES).enumerate() {
            match target {
                Target::Channel(channel) => view.open_channel(channel, index == 0, window, cx),
                // Named by a link, so it has to be looked up first, and that
                // needs a session; it opens once there is one.
                Target::Video { id, start_secs } => view.open_video_link(id, start_secs, cx),
            }
        }
        // Anything the command line could not use. Said here, in the window,
        // because a release build has no console for it to have been said in.
        for warning in warnings {
            view.toast(warning, cx);
        }
        view
    }

    /// Write the preferences down, and say so on screen if that fails.
    ///
    /// One place rather than eight copies of the same `if let Err`, and a toast
    /// rather than a log line: a release build's stderr is a file nobody is
    /// watching, and a save that fails silently is a preference that comes back
    /// on the next launch with no explanation.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.settings.save_preferences(&self.settings_path) {
            eprintln!("settings: could not save: {e}");
            self.toast(format!("could not save settings: {e}"), cx);
        }
    }

    /// Whether "open beside what is playing" is an offer worth making: only
    /// when something is playing, and there is room for another.
    ///
    /// The palette already made this distinction; the cards and the rail did
    /// not, and offered `+ add` on an empty watch page, where it did exactly
    /// what a click on the card did.
    fn can_add(&self) -> bool {
        !self.slots.is_empty() && self.slots.len() < MAX_PANES
    }

    /// The room the page body has: the window, less the rail when it is open.
    /// The one place a [`Body`] is made; see the type for why.
    fn body(&self, window: &Window) -> Body {
        let rail = if self.settings.sidebar_collapsed {
            0.0
        } else {
            sidebar::WIDTH
        };
        Body::of(window.viewport_size(), rail)
    }

    /// What the keymap tests its predicates against.
    ///
    /// The sheet *replaces* the page name rather than adding to it, so a
    /// shortcut scoped to a page cannot fire through a modal without every
    /// binding having to remember to say so.
    fn key_context(&self) -> &'static str {
        if self.settings_panel.is_some() || self.palette_open {
            return keys::CONTEXT_MODAL;
        }
        match self.page {
            Page::Watch => keys::CONTEXT_WATCH,
            Page::Browse => keys::CONTEXT_BROWSE,
        }
    }

    /// The pane a player shortcut acts on: the last one pointed at, or the
    /// first if the pointer has not been in one yet.
    ///
    /// A stale channel simply does not resolve, which is the whole reason for
    /// storing one rather than an index.
    fn active_slot(&self) -> Option<usize> {
        self.active
            .as_deref()
            .and_then(|channel| self.slot_index(channel))
            .or_else(|| (!self.slots.is_empty()).then_some(0))
    }

    fn active_video(&self) -> Option<Entity<VideoView>> {
        self.slots.get(self.active_slot()?)?.video().cloned()
    }

    /// The pane with this key: a login for a live stream, or
    /// [`Slot::video_key`] for a recording.
    fn slot_index(&self, key: &str) -> Option<usize> {
        self.slots.iter().position(|slot| slot.key == key)
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Publish what this frame is, for the CPU log, when there is one. It is
        // here rather than at every state change because a frame is the unit
        // the sampler is trying to explain. Guarded because the two cache
        // counts each take a lock, which is more than a diagnostic that is off
        // should cost a frame.
        if cpu_log::active() {
            cpu_log::note_frame(
                match self.page {
                    Page::Browse => cpu_log::Page::Browse,
                    Page::Watch => cpu_log::Page::Watch,
                },
                self.slots.len(),
                self.slots
                    .iter()
                    .filter(|slot| matches!(slot.state, StreamState::Playing(_)))
                    .count(),
                self.cache.ready_len(),
                self.cache.inflight_len(),
            );
        }

        // First, before anything builds a card. A preview that has been replaced
        // has to be released while nothing is asking for it any more, and this
        // whole function runs before any element it returns lays itself out.
        // Here rather than in `browse_page` for two reasons: that has no
        // `&mut Window`, and the drain has to run on watch-page frames too, or a
        // wave of retirements sits undrained with its images resident until the
        // user happens to navigate back. See `browse::release_retired_previews`.
        browse::release_retired_previews(&self.cache, window, cx);

        let page = match self.page {
            Page::Browse => self.browse_page(window, cx).into_any_element(),
            Page::Watch => self.watch_page(window, cx).into_any_element(),
        };

        div()
            // Focus and context are what make the keymap reachable at all; see
            // `keys`. `track_focus` rather than `id().focusable()` on purpose —
            // giving the root div an id would re-namespace every descendant
            // element id in the app, including the ones animated images depend
            // on.
            .track_focus(&self.focus)
            .key_context(self.key_context())
            .on_action(cx.listener(Self::on_toggle_playback))
            .on_action(cx.listener(Self::on_toggle_mute))
            .on_action(cx.listener(Self::on_toggle_chat))
            .on_action(cx.listener(Self::on_volume_up))
            .on_action(cx.listener(Self::on_volume_down))
            .on_action(cx.listener(Self::on_seek_back))
            .on_action(cx.listener(Self::on_seek_forward))
            .on_action(cx.listener(Self::on_close_pane))
            .on_action(cx.listener(Self::on_go_browse))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_palette))
            .on_action(cx.listener(Self::on_reset_layout))
            .on_action(cx.listener(Self::on_toggle_fullscreen))
            .on_action(cx.listener(Self::on_activate_pane))
            .on_action(cx.listener(Self::on_next_pane))
            .on_action(cx.listener(Self::on_previous_pane))
            .on_action(cx.listener(Self::on_back))
            .on_key_down(cx.listener(Self::on_palette_key))
            // A divider drag is followed here rather than on the handle: the
            // pointer leaves a six-pixel target on the first frame of any pull
            // worth making, and these are the only listeners that still hear it.
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .relative()
            .size_full()
            .bg(theme::bg())
            .text_color(theme::text())
            .child(motion::arrive(
                // Browse and watch share no layout at all, so cutting between
                // them reads as the window being replaced rather than as
                // moving within one app. No movement, only a fade: anything
                // that slides drags the eye across the whole page.
                ("page", self.page as u32),
                0.0,
                div().size_full().child(page),
            ))
            .child(self.toast_stack(cx))
            .children(self.palette_sheet(cx))
            .children(self.settings_panel.clone())
    }
}
