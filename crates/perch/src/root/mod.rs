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
//! | `navigation` | back and forward: where the app is as a `Route`, recording each step on the trail (`crate::trail`), and the three ways along it |
//! | `streams` | opening, restarting and closing panes, and swapping one for a recording in place |
//! | `renditions` | what each pane plays, and when it restarts: the quality chosen against each pane's own height (`pane_height_for`), the upward re-pick when the grid changes, a pick from the pane's menu; a new rendition resolved beside the picture and kept once its player has taken over in place (`video_view::swap`) |
//! | `panes` | where each pane is drawn, applied: `restage`, the one funnel every change of state, membership, page, pop-out or maximize ends in; `set_slot_state`, the only write of a pane's state; `video_in_main`, the only way the main window reaches a player; `retire_homeless`, the one rule for which panes stop when nothing in the main window would draw them; a pane given the watch page and every pane shown again (`toggle_maximize`, `show_all_panes`), and `choose`, which takes the maximize to the pane chosen |
//! | `pop_out` | a pane in a window of its own, on top of other apps: moving its picture there and back, the window, and `to_root`, the only way back from it |
//! | `broadcasts` | what a stopped live pane asks about its channel's past broadcasts, and whether it can start by itself |
//! | `pane_actions` | what a pane asks for: its controls, and its player's requests; the header a pane key reveals |
//! | `launches` | what the command line named, at startup and from later launches |
//! | `history` | what has been watched: noting where each recording got to, resuming there |
//! | `prefs` | the settings sheet, the divider drag, the rail folding and what is pinned to it |
//! | `recommended` | the rail's Recommended group: when to ask the worker, and what its answer becomes |
//! | `chrome` | pills, toasts, the rail |
//! | `mini_player` | what plays on while you browse, in the corner of the page |
//! | `title_bar` | the bar Perch draws across the top of the window: the rail button, back and forward, search, settings, and on Windows the caption buttons |
//! | `pages` | the two pages, assembled, each only its own column |
//!
//! A sibling reaches the fields directly — they are private to this module,
//! and a child module is inside it — so the split costs no accessors. Its
//! methods are `pub(super)`: callable from the rest of the root, and nowhere
//! else.

mod broadcasts;
mod browsing;
mod chrome;
mod commands;
mod follows;
mod history;
mod launches;
mod mini_player;
mod navigation;
mod pages;
mod pane_actions;
mod panes;
mod pop_out;
mod prefs;
mod recommended;
mod renditions;
mod shortcuts;
mod streams;
mod title_bar;

pub(crate) use self::pop_out::offered_here as pop_out_offered;
pub(crate) use self::title_bar::window_min_size;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use emotes::ImageCache;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    div, prelude::*, AnyWindowHandle, Bounds, Context, Entity, FocusHandle, Pixels, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use gpui_component::input::{InputEvent, InputState};
use settings::history::{Forgotten, History};
use settings::Settings;
use twitch_api::{Channel, LiveStream};

use self::follows::LiveList;
use self::navigation::Route;
use self::pop_out::PoppedOut;
use crate::browse::{self, Discovery, SignIn};
use crate::launch::Launch;
use crate::layout::Body;
use crate::recommended::Recommended;
use crate::settings_view::SettingsPanel;
use crate::stage::Stage;
use crate::trail::Trail;
use crate::twitch::TwitchService;
use crate::video_view::VideoView;
use crate::watch::{ResizeStart, Slot, StreamState, MAX_PANES};
use crate::{cpu_log, keys, layout, motion, sidebar, theme, vod, APP_NAME};

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
    /// from the `+ Add` pill next to it.
    Watch(String),
    /// Put back what was just taken off the history, from the `Undo` pill.
    Undo(Forgotten),
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
    /// Every recording opened, and where each was left. A file of its own
    /// beside the settings; see `settings::history`.
    history: History,
    history_path: PathBuf,
    /// Which scheduled history save is the newest; see `save_history_soon`.
    history_epoch: u64,
    /// Notes where the recordings playing have got to, every few seconds;
    /// see `keep_history`. Dropping it stops that.
    _history_tick: Task<()>,
    cache: Arc<ImageCache>,

    page: Page,
    /// The panes, in their order: the grid's, the keys `1`–`4` and `Tab`
    /// walk, the mini player's and the palette's. The one account of it.
    slots: Vec<Slot>,
    /// Which panes are drawn in windows of their own, with each window, and
    /// which pane has the watch page to itself, if one has; see
    /// `crate::stage` and `panes`. Every other pane is the main window's.
    stage: Stage<PoppedOut>,
    /// The main window, which everything a pop-out asks of the root is
    /// answered with (`pop_out::to_root`).
    main_window: AnyWindowHandle,
    /// Where the last pop-out was when it closed, for the next one to open
    /// at while a display still holds it and no pop-out open covers it
    /// (`layout::pop_out_bounds`). For this session only: nothing of the
    /// pop-out is saved.
    pop_out_last: Option<Bounds<Pixels>>,

    follows: Vec<LiveStream>,
    /// Everyone followed, live or not. Kept apart from `follows` all the way to
    /// the screen; see `twitch_api::Channel`.
    offline: Vec<Channel>,
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
    /// Whether the pointer is over the rail, and over the Following tab, as
    /// the last frame measured it. While either is, a poll updates the
    /// follows, live and offline, where they stand rather than sorting them —
    /// see `RootView::hold_live`.
    rail_pointed: bool,
    following_pointed: bool,
    /// Whether the rail's offline follows are unfolded. For this session
    /// only, and folded at the start of each: unfolded, it is the longest
    /// list in the app, up to a thousand rows built every frame, and most
    /// evenings nobody is looking for someone who is not on.
    rail_offline_open: bool,
    /// The rail's Recommended group: the asks out, the answers in, and the
    /// rows shown. For this session only; see `crate::recommended`.
    recommended: Recommended,
    sign_in: SignIn,
    /// Everything the browse page shows besides your follows.
    discovery: Discovery,
    /// Where the app has been, for back and forward. History only: where it
    /// is now is always read from `page` and `discovery`; see `navigation`.
    trail: Trail<Route>,
    /// A step is being recorded, or replayed, so the functions it runs
    /// through do not record steps of their own; see `RootView::record`.
    recording: bool,
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
    /// trap that keys pane element ids on the key. While a pane is
    /// maximized, choosing another moves the maximize with it
    /// (`RootView::choose`), so the keys talk to the pane on the page.
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
    toasts: Vec<Toast>,
    next_toast: u64,
    _cache_pump: Task<()>,

    /// Recordings opened by link that are still to be looked up.
    linked_videos: Vec<LinkedVideo>,
    /// Opens what later launches hand over; see `launches`. Dropping it is
    /// what tells `instance` there is no window left to hand to.
    _launch_pump: Task<()>,
    /// Which scheduled settings save is the newest; see `save_settings_soon`.
    save_epoch: u64,
    /// Which run of window resizes is the newest; see `on_window_resized`.
    resize_epoch: u64,
    /// Which pane-header reveal is the newest, so only its timer takes the
    /// header back down; see `reveal_header`.
    reveal_epoch: u64,
    /// Whether the run of presses going on in the main window began on a
    /// control that took away what was under the pointer, so that the rest
    /// of the run is heard by nothing; see `run_guard`.
    run_taken: bool,
    /// Keeps the resize observer alive; a dropped `Subscription` unsubscribes.
    _bounds: Subscription,
}

impl RootView {
    /// The app, opening what this launch named; `launches` brings what later
    /// ones name, handed over by `instance`.
    pub(crate) fn new(
        launch: Launch,
        launches: UnboundedReceiver<Vec<String>>,
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
        let history_path = settings::history::default_path(APP_NAME);
        let history = History::load(&history_path).unwrap_or_else(|e| {
            eprintln!("history: {e}; starting with none");
            History::default()
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

        // In the title bar, over both pages; see `run_search` for what a
        // search from the watch page does.
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search channels and categories"));
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
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter your follows"));
        cx.subscribe(&filter, |_: &mut RootView, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();

        let palette_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Channel, or a command"));
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
            history,
            history_path,
            history_epoch: 0,
            _history_tick: Self::keep_history(cx),
            cache,
            page: Page::Browse,
            slots: Vec::new(),
            stage: Stage::default(),
            main_window: window.window_handle(),
            pop_out_last: None,
            follows: Vec::new(),
            offline: Vec::new(),
            refreshing: false,
            avatars: HashMap::new(),
            follows_loaded: false,
            known_live: HashSet::new(),
            rail_pointed: false,
            following_pointed: false,
            rail_offline_open: false,
            recommended: Recommended::default(),
            sign_in: SignIn::Connecting,
            discovery: Discovery::default(),
            trail: Trail::default(),
            recording: false,
            search,
            filter,
            scrolls: browse::Scrolls::default(),
            rail_scroll: ScrollHandle::new(),
            twitch: service,
            _twitch_pump: twitch_pump,
            active: None,
            focus,
            _focus_lost,
            volume_override: launch.volume,
            resize: None,
            palette_input,
            palette_open: false,
            palette_selected: 0,
            settings_panel: None,
            toasts: Vec::new(),
            next_toast: 0,
            _cache_pump: cache_pump,
            linked_videos: Vec::new(),
            _launch_pump: Self::pump_launches(launches, window, cx),
            save_epoch: 0,
            resize_epoch: 0,
            reveal_epoch: 0,
            run_taken: false,
            _bounds,
        };

        // Nothing else ever asks for focus, so this is what makes every
        // shortcut work — see the field.
        window.focus(&view.focus);

        // Only what was named on the command line opens a stream. Launching
        // straight into whatever was on last time means the app starts costing
        // CPU and bandwidth before anyone has asked it to.
        view.open_targets(launch.targets, window, cx);
        // Anything the command line could not use. Said here, in the window,
        // because a release build has no console for it to have been said in.
        for warning in launch.warnings {
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
    /// not, and offered `+ Add` on an empty watch page, where it did exactly
    /// what a click on the card did.
    fn can_add(&self) -> bool {
        !self.slots.is_empty() && self.slots.len() < MAX_PANES
    }

    /// The room the page body has: the window, less the rail and the title
    /// bar when each is drawn. Neither is in fullscreen. The one place a
    /// [`Body`] is made; see the type for why.
    fn body(&self, window: &Window) -> Body {
        let rail = if self.rail_shown(window) {
            sidebar::WIDTH
        } else {
            0.0
        };
        Body::of(
            window.viewport_size(),
            rail,
            layout::title_bar_height(window.is_fullscreen()),
        )
    }

    /// The watch grid as it is cut now: [`layout::Grid::of`] the body and
    /// the [`cells`](Self::cells). The one grid the page is drawn in, a
    /// divider drag is measured against and a pane's quality is chosen for,
    /// so the three cannot disagree about a pane's size. Only a pane
    /// maximized away is measured by another, the one it comes back to
    /// (`pane_height_for`).
    fn grid(&self, window: &Window) -> layout::Grid {
        layout::Grid::of(self.body(window), self.cells().len())
    }

    /// The panes the watch grid draws, as positions in `slots`, in order:
    /// one cell for every pane, a pane in a window of its own included,
    /// since its cell stays where it was with its chat in it — or the
    /// maximized pane's alone (`stage::Stage::cells`).
    fn cells(&self) -> Vec<usize> {
        self.stage.cells(&panes::pane_keys(&self.slots))
    }

    /// Whether the rail is drawn beside the page; see [`layout::rail_shown`].
    /// Read by [`body`](Self::body) and by the render that draws the rail, so
    /// the two cannot disagree about it.
    fn rail_shown(&self, window: &Window) -> bool {
        layout::rail_shown(self.settings.sidebar_collapsed, window.is_fullscreen())
    }

    /// Whether the settings sheet or the palette is up, so that nothing
    /// behind it may change the page: the keys (through
    /// [`key_context`](Self::key_context)), the title bar's veil over its
    /// rail button, arrows and search box, and the mouse's side buttons.
    /// Asked here by all three, so a modal added later is one more line here
    /// rather than three places to remember.
    fn modal_open(&self) -> bool {
        self.settings_panel.is_some() || self.palette_open
    }

    /// What the keymap tests its predicates against.
    ///
    /// A modal *replaces* the page name rather than adding to it, so a
    /// shortcut scoped to a page cannot fire through one without every
    /// binding having to remember to say so.
    fn key_context(&self) -> &'static str {
        if self.modal_open() {
            // Which modal, for the one key that tells them apart: `Ctrl+K`
            // closes the palette, and does nothing over the sheet.
            return if self.settings_panel.is_some() {
                keys::CONTEXT_SHEET
            } else {
                keys::CONTEXT_MODAL
            };
        }
        match self.page {
            Page::Watch => keys::CONTEXT_WATCH,
            Page::Browse => keys::CONTEXT_BROWSE,
        }
    }

    /// The pane a player shortcut acts on: the last one pointed at or
    /// chosen, or else the first the watch grid draws ([`cells`](Self::cells))
    /// — the first pane, or the maximized one while a pane has the page.
    ///
    /// A stale channel simply does not resolve, which is the whole reason for
    /// storing one rather than an index. What it falls back on is a cell the
    /// grid draws rather than the first pane, which a maximize can leave
    /// drawn nowhere: a popped pane chosen while another had the page, then
    /// closed, would otherwise hand `Space`, `Ctrl+W` and `Z` to a pane
    /// nobody can see.
    fn active_slot(&self) -> Option<usize> {
        self.active
            .as_deref()
            .and_then(|channel| self.slot_index(channel))
            .or_else(|| self.cells().first().copied())
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
        // Drawn here, once, beside whichever page is up, rather than by each
        // page: it does not fade out and back in with the page, and the probe
        // that holds its order still while it is pointed at has one owner.
        let rail_shown = self.rail_shown(window);
        let rail = self.follows_rail(rail_shown, cx).map(|rail| {
            self.holding(LiveList::Rail, true, rail, cx)
                .flex_none()
                .h_full()
        });

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
            .on_action(cx.listener(Self::on_step_out))
            .on_action(cx.listener(Self::on_navigate_back))
            .on_action(cx.listener(Self::on_navigate_forward))
            .on_action(cx.listener(Self::on_toggle_pop_out))
            .on_action(cx.listener(Self::on_toggle_maximize))
            .on_key_down(cx.listener(Self::on_palette_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .text_color(theme::text())
            // Ahead of everything else drawn here, so it hears a press before
            // anything under the pointer does; see `run_guard`.
            .child(self.run_guard(cx))
            .children(self.title_bar(window, cx))
            .child(
                // Everything under the bar. The modals, the toasts and the
                // mini player are in here rather than on the root, so their
                // `inset_0` and their offsets start below the bar by
                // construction: a scrim cannot cover the caption buttons, and
                // a toast cannot land on the drag strip —
                // `block_mouse_except_scroll` does not hide a drag area from
                // the platform's hit test, so a toast there would move the
                // window when pressed. No id, for the reason the root has
                // none: an id namespaces every element id beneath it.
                //
                // Clipped, because starting below the bar is not the same as
                // staying below it. Anchored to the bottom, the mini player
                // grows upward, and in a window shorter than it and the bar
                // together it reached over the bar, where the bar's drag and
                // caption areas still answered the platform under its tiles:
                // pressing a picture moved, maximised or closed the window.
                // The clip bounds hit testing as well as painting, so nothing
                // in here can be pressed outside it, and it adds no hitbox.
                // Tooltips and a text box's menu are drawn by the window
                // after everything else, so they still reach past it.
                //
                // A row: the rail, then the page beside it. The overlays
                // after them — the toasts and the two modals — are absolute,
                // out of the row, and cover both.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .children(rail)
                    .child(
                        // The page, and the mini player over it. The player
                        // is placed against this column rather than the row,
                        // so it floats over the page and never the rail: in
                        // a narrow window, placed against the row, it reached
                        // over the rail's last rows, which have no room at
                        // their foot to be scrolled out from under it. The
                        // clip keeps it off the rail in a window narrower
                        // still, where it is cut off at its left instead.
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(motion::arrive(
                                // Browse and watch share no layout at all, so
                                // cutting between them reads as the window
                                // being replaced rather than as moving within
                                // one app. No movement, only a fade: anything
                                // that slides drags the eye across the whole
                                // page.
                                ("page", self.page as u32),
                                0.0,
                                div().size_full().child(page),
                            ))
                            // Outside the fade, so the page changing under it
                            // — a tab, a category, a channel — never rebuilds
                            // it, and drawn before everything after this
                            // column, so the toasts, the palette and the sheet
                            // cover it.
                            .children(self.mini_player(cx)),
                    )
                    .child(self.toast_stack(cx))
                    .children(self.palette_sheet(cx))
                    .children(self.settings_panel.clone()),
            )
            // A divider drag is followed by the window rather than by the
            // handle or the root; see `drag_listeners`. So are the mouse's
            // back and forward buttons, for the same reason.
            .child(self.drag_listeners(cx))
            .child(self.side_buttons(cx))
    }
}
