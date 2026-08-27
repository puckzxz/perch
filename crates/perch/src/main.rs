//! Twitch in one native window.
//!
//!     perch [channel...] [--volume 0-100]
//!
//! Two pages: browse what you follow, and watch up to four of them at once.
//! The watch page is deliberately bare — players and their chats, nothing else
//! — because chrome you stare past for three hours should not be there.

// No console window in a real build. Debug builds keep one, because that is
// where `--help` and a live stderr are worth more than the tidiness. See
// `diagnostics` for where the output goes instead.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browse;
mod chat;
mod chat_text;
mod diagnostics;
mod keys;
mod layout;
mod motion;
mod settings_view;
mod theme;
mod twitch;
mod video;
mod video_view;
mod watch;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use browse::{Action, Discovery, SearchResults, SignIn, Tab};
use chat::ChatView;
use emotes::ImageCache;
use gpui::{
    div, prelude::*, px, size, AnyView, App, Application, Bounds, Context, ElementId, Entity,
    FocusHandle, SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use gpui_component::input::{Input, InputEvent, InputState};
use settings::{QualityPreference, Settings};
use settings_view::{SettingsEvent, SettingsPanel};
use streamlink::{StreamEvent, StreamOptions, StreamSupervisor};
use twitch::{Request, TwitchEvent, TwitchService};
use twitch_api::{FollowedChannel, LiveStream};
use video::VideoStream;
use video_view::{VideoEvent, VideoView};
use watch::{Slot, StreamState, MAX_PANES};

pub const APP_NAME: &str = "perch";

/// Starting render size. Each pane measures itself on the first layout pass and
/// its render thread follows from then on, so this only decides what the first
/// frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// How long a "went live" toast stays up.
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

/// Emotes and thumbnails are reproducible, so they live in local app data
/// rather than roaming with settings.
fn image_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
        .join("images")
}

/// A transient notice. It owns its own fade because a toast that vanished
/// mid-sentence read as a dropped frame rather than as time passing.
struct Toast {
    id: u64,
    text: SharedString,
    fade: motion::Fade,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Page {
    Browse,
    Watch,
}

struct RootView {
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
    /// Who was live at the last poll, so newly-live channels can be told apart
    /// from ones that were already streaming. Without this every poll would
    /// re-announce everybody.
    known_live: HashSet<String>,
    sign_in: SignIn,
    /// Everything the browse page shows besides your follows.
    discovery: Discovery,
    search: Entity<InputState>,
    twitch: TwitchService,
    _twitch_pump: Task<()>,

    /// Which pane the player shortcuts act on, held as a channel rather than
    /// an index: closing a pane reindexes every pane after it, and a stored
    /// index would quietly start acting on somebody else — the same trap that
    /// keys pane element ids on the channel.
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

    settings_panel: Option<Entity<SettingsPanel>>,
    /// Whether the page navigation is up. It follows the video chrome rather
    /// than sitting there permanently: a control you never look at should not
    /// be on the picture for three hours.
    nav: motion::Fade,
    toasts: Vec<Toast>,
    next_toast: u64,
    _cache_pump: Task<()>,
}

impl RootView {
    fn new(
        channels: Vec<String>,
        volume_override: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_path = settings::default_path(APP_NAME);
        let settings = Settings::load(&settings_path).unwrap_or_else(|e| {
            eprintln!("settings: {e}; using defaults");
            Settings::default()
        });

        let (cache, mut cache_ready) =
            ImageCache::new(image_cache_dir()).expect("failed to open image cache");
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

        let mut view = Self {
            settings,
            settings_path,
            cache,
            page: Page::Browse,
            slots: Vec::new(),
            follows: Vec::new(),
            offline: Vec::new(),
            refreshing: false,
            known_live: HashSet::new(),
            sign_in: SignIn::Connecting,
            discovery: Discovery::default(),
            search,
            twitch: service,
            _twitch_pump: twitch_pump,
            active: None,
            focus,
            _focus_lost,
            volume_override,
            settings_panel: None,
            nav: motion::Fade::hidden(),
            toasts: Vec::new(),
            next_toast: 0,
            _cache_pump: cache_pump,
        };

        // Nothing else ever asks for focus, so this is what makes every
        // shortcut work — see the field.
        window.focus(&view.focus);

        // Only channels named on the command line open a stream. Launching
        // straight into whatever was on last time means the app starts costing
        // CPU and bandwidth before anyone has asked it to.
        for (index, channel) in channels.into_iter().take(MAX_PANES).enumerate() {
            view.open_channel(channel, index == 0, window, cx);
        }
        view
    }

    // ── Keyboard ─────────────────────────────────────────────────────

    /// What the keymap tests its predicates against.
    ///
    /// The sheet *replaces* the page name rather than adding to it, so a
    /// shortcut scoped to a page cannot fire through a modal without every
    /// binding having to remember to say so.
    fn key_context(&self) -> &'static str {
        if self.settings_panel.is_some() {
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

    fn on_toggle_playback(
        &mut self,
        _: &keys::TogglePlayback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.toggle_playback(cx));
        }
    }

    fn on_toggle_mute(
        &mut self,
        _: &keys::ToggleMute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.toggle_mute(window, cx));
        }
    }

    fn on_volume_up(&mut self, _: &keys::VolumeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_volume(keys::VOLUME_STEP, window, cx);
    }

    fn on_volume_down(
        &mut self,
        _: &keys::VolumeDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_volume(-keys::VOLUME_STEP, window, cx);
    }

    fn nudge_volume(&mut self, delta: i16, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.nudge_volume(delta, window, cx));
        }
    }

    /// Show or hide the active pane's chat, and remember it for that channel.
    ///
    /// Per pane rather than per app: the whole watch page is built on panes
    /// being independent, and the reason to hide chat — watching one stream for
    /// the game while reading another's chat — only makes sense if it is.
    fn on_toggle_chat(
        &mut self,
        _: &keys::ToggleChat,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_slot() else {
            return;
        };
        let hidden = !self.slots[index].chat_hidden;
        self.slots[index].chat_hidden = hidden;

        let channel = self.slots[index].channel.clone();
        if self.settings.set_chat_hidden_for(&channel, hidden) {
            if let Err(e) = self.settings.save_preferences(&self.settings_path) {
                eprintln!("settings: could not save: {e}");
            }
        }
        cx.notify();
    }

    fn on_close_pane(&mut self, _: &keys::ClosePane, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active_slot() {
            self.close_slot(index, cx);
        }
    }

    fn on_go_browse(&mut self, _: &keys::GoBrowse, _window: &mut Window, cx: &mut Context<Self>) {
        self.go_browse(cx);
    }

    fn on_toggle_settings(
        &mut self,
        _: &keys::ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }

    fn on_focus_search(
        &mut self,
        _: &keys::FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    // ── Twitch ───────────────────────────────────────────────────────

    fn spawn_twitch(
        settings_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (TwitchService, Task<()>) {
        let (service, mut events) = TwitchService::start(settings_path);
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this: &mut RootView, cx| {
                        this.apply_twitch_event(event, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        (service, pump)
    }

    fn apply_twitch_event(&mut self, event: TwitchEvent, cx: &mut Context<Self>) {
        match event {
            TwitchEvent::NeedsClientId => self.sign_in = SignIn::NeedsClientId,
            TwitchEvent::AwaitingCode {
                user_code,
                verification_uri,
            } => {
                self.sign_in = SignIn::AwaitingCode {
                    user_code: user_code.into(),
                    verification_uri: verification_uri.into(),
                }
            }
            TwitchEvent::SignedIn { login } => {
                self.sign_in = SignIn::SignedIn(login.into());
                // Whatever the user opened while signed out can be fetched now.
                self.fill_tab();
            }
            TwitchEvent::Streams(streams) => self.on_streams(streams, cx),
            TwitchEvent::FollowedChannels(channels) => {
                // Filtered against the live list rather than trusted: the two
                // requests are seconds apart, so somebody can go live between
                // them and would otherwise appear in both places at once.
                self.offline = channels
                    .into_iter()
                    .filter(|channel| !self.known_live.contains(&channel.login))
                    .collect();
                self.refreshing = false;
                cx.notify();
            }
            TwitchEvent::FollowsError(reason) => {
                // Only worth saying when somebody asked. The poll runs every
                // minute, and an outage that lasts an hour should not be sixty
                // toasts about a list that is still on screen.
                if self.refreshing {
                    self.toast(format!("could not refresh: {reason}"), cx);
                } else {
                    eprintln!("follows: {reason}");
                }
                self.refreshing = false;
                cx.notify();
            }
            TwitchEvent::Error(reason) => {
                // Terminal: the worker has returned, so no refresh it was
                // holding is ever going to be answered.
                self.refreshing = false;
                self.sign_in = SignIn::Error(reason.into());
            }

            TwitchEvent::Popular(page) => {
                self.discovery
                    .popular
                    .absorb(page.items, page.next, page.append);
                self.discovery.loading = false;
            }
            TwitchEvent::Categories(page) => {
                self.discovery
                    .categories
                    .absorb(page.items, page.next, page.append);
                self.discovery.loading = false;
            }
            TwitchEvent::CategoryStreams { category, streams } => {
                // A reply for a category the user has already left must not
                // repopulate the page behind them.
                let still_open = self
                    .discovery
                    .open
                    .as_ref()
                    .is_some_and(|open| open.id == category.id);
                if still_open {
                    self.discovery
                        .streams
                        .absorb(streams.items, streams.next, streams.append);
                }
                self.discovery.loading = false;
            }
            TwitchEvent::SearchResults {
                query,
                categories,
                streams,
            } => {
                // Same guard as a category: an answer to a question the user
                // has moved on from must not replace what they are reading now.
                let current = self
                    .discovery
                    .search
                    .as_ref()
                    .is_some_and(|open| open.query.as_ref() == query);
                if current {
                    self.discovery.search = Some(SearchResults {
                        query: query.into(),
                        categories,
                        streams,
                    });
                }
                self.discovery.loading = false;
            }
            TwitchEvent::BrowseError(reason) => {
                self.discovery.error = Some(reason.into());
                self.discovery.loading = false;
            }
        }
        cx.notify();
    }

    // ── Browsing ─────────────────────────────────────────────────────

    /// Only claim to be loading if something is going to answer.
    ///
    /// Browsing needs a token like everything else. Before sign-in the worker
    /// is still parked on the device-code poll, so a request would sit in the
    /// queue behind it and the page would pulse "Loading…" until the user
    /// noticed the code on another tab. Say what is actually wrong instead, and
    /// come back to it in [`fill_tab`](Self::fill_tab) once signed in.
    fn fetch(&mut self, request: Request) {
        self.discovery.error = None;
        self.discovery.loading = false;

        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            self.discovery.error = Some("Sign in to Twitch to browse.".into());
            return;
        }
        // Fails once the worker has returned, which it does when sign-in fails.
        if self.twitch.request(request) {
            self.discovery.loading = true;
        } else {
            self.discovery.error = Some("Not connected to Twitch.".into());
        }
    }

    /// Fetch whatever the open tab needs and does not already have.
    ///
    /// Asked for once and kept: these are network round trips, and the top of
    /// Twitch does not move in the time it takes to look at another tab.
    fn fill_tab(&mut self) {
        match self.discovery.tab {
            Tab::Popular if self.discovery.popular.is_empty() => {
                self.fetch(Request::Popular { after: None })
            }
            Tab::Categories if self.discovery.categories.is_empty() => {
                self.fetch(Request::Categories { after: None })
            }
            _ => {}
        }
    }

    /// Ask again for whatever is on screen.
    ///
    /// One control, whichever list is up, because "refresh" means the thing you
    /// are looking at. The discovery lists are otherwise fetched once per tab
    /// and kept forever, which is right for a page you glance at and wrong for
    /// one you have had open all evening.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(results) = &self.discovery.search {
            let query = results.query.to_string();
            self.run_search(query, cx);
            return;
        }
        // Refresh starts the list again rather than continuing it: the point is
        // to see what is on *now*, and appending a fresh page one onto a stale
        // page two would be neither.
        if let Some(category) = self.discovery.open.clone() {
            self.fetch(Request::Category {
                category,
                after: None,
            });
        } else {
            match self.discovery.tab {
                Tab::Following => self.refresh_follows(),
                Tab::Popular => self.fetch(Request::Popular { after: None }),
                Tab::Categories => self.fetch(Request::Categories { after: None }),
            }
        }
        cx.notify();
    }

    /// Poll the follows lists now instead of at the next minute.
    ///
    /// Deliberately not routed through `fetch`, which owns the browse page's
    /// loading and error state: a follows refresh is not a browse request, and
    /// borrowing that flag would put "Loading…" over the popular tab.
    fn refresh_follows(&mut self) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        self.refreshing = self.twitch.request(Request::Follows);
    }

    fn on_refresh(&mut self, _: &keys::Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn show_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.discovery.tab = tab;
        self.discovery.open = None;
        self.discovery.search = None;
        self.discovery.error = None;
        self.fill_tab();
        cx.notify();
    }

    /// Run a search, or clear the results if the box is empty.
    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        if query.is_empty() {
            self.discovery.search = None;
            self.discovery.error = None;
            cx.notify();
            return;
        }

        // Seeded with the query before the answer arrives, so the page can say
        // what it is waiting for and can recognise a stale reply when it lands.
        self.discovery.open = None;
        self.discovery.search = Some(SearchResults {
            query: query.clone().into(),
            ..Default::default()
        });
        self.fetch(Request::Search(query));
        cx.notify();
    }

    fn on_browse_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Watch(channel) => self.open_channel(channel, true, window, cx),
            Action::Add(channel) => self.open_channel(channel, false, window, cx),
            Action::OpenCategory(category) => {
                self.discovery.search = None;
                self.discovery.streams.clear();
                self.discovery.open = Some(category.clone());
                self.fetch(Request::Category {
                    category,
                    after: None,
                });
                cx.notify();
            }
            Action::CloseCategory => {
                self.discovery.open = None;
                self.discovery.streams.clear();
                self.discovery.error = None;
                cx.notify();
            }
            Action::CloseSearch => {
                self.discovery.search = None;
                self.discovery.error = None;
                cx.notify();
            }
            Action::LoadMore => self.load_more(cx),
        }
    }

    /// Ask for the next page of whichever list is on screen.
    ///
    /// The cursor comes from the list itself rather than from the click, so a
    /// stale button cannot ask for a page that has already arrived — and a
    /// list with nothing left simply has no cursor, which is also what takes
    /// the row away.
    ///
    /// Nothing here is reachable while `loading` is set: the row renders as
    /// "Loading…" and stops taking clicks, so one cursor cannot be spent twice.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        // Search results are not paginated — see `SEARCH_PAGE_SIZE`, where a
        // short list is the feature.
        if self.discovery.search.is_some() {
            return;
        }

        let request = if let Some(category) = self.discovery.open.clone() {
            self.discovery
                .streams
                .next
                .clone()
                .map(|after| Request::Category {
                    category,
                    after: Some(after),
                })
        } else {
            match self.discovery.tab {
                Tab::Popular => self
                    .discovery
                    .popular
                    .next
                    .clone()
                    .map(|after| Request::Popular { after: Some(after) }),
                Tab::Categories => self
                    .discovery
                    .categories
                    .next
                    .clone()
                    .map(|after| Request::Categories { after: Some(after) }),
                Tab::Following => None,
            }
        };

        if let Some(request) = request {
            self.fetch(request);
            cx.notify();
        }
    }

    /// Take a fresh follows list and announce anyone who just came online.
    ///
    /// The first poll seeds the known set silently: on launch everyone is
    /// "newly" live, and eight toasts at once would be worse than none.
    fn on_streams(&mut self, streams: Vec<LiveStream>, cx: &mut Context<Self>) {
        let now_live: HashSet<String> = streams.iter().map(|s| s.user_login.clone()).collect();

        if !self.known_live.is_empty() {
            let mut newly: Vec<&LiveStream> = streams
                .iter()
                .filter(|s| !self.known_live.contains(&s.user_login))
                .collect();
            newly.sort_by_key(|stream| std::cmp::Reverse(stream.viewer_count));
            for stream in newly {
                self.toast(
                    format!("{} went live · {}", stream.display_name, stream.game_name),
                    cx,
                );
            }
        }

        // Anyone who just went live is no longer offline. The offline list
        // arrives from its own request moments later and will agree, but not
        // before a repaint that would show them in both lists.
        self.offline
            .retain(|channel| !now_live.contains(&channel.login));

        self.known_live = now_live;
        self.follows = streams;
        cx.notify();
    }

    fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let id = self.next_toast;
        self.next_toast += 1;
        self.toasts.push(Toast {
            id,
            text: text.into(),
            fade: motion::Fade::entering(),
        });

        // Two stages, because dropping the element is what stops it being
        // drawn: start the fade at the end of the lifetime, and only remove it
        // once the fade has had time to run.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_LIFETIME).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                if let Some(toast) = this.toasts.iter_mut().find(|toast| toast.id == id) {
                    toast.fade.set(false);
                    cx.notify();
                }
            });
            cx.background_executor().timer(theme::MOTION_ENTER).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                this.toasts.retain(|toast| toast.id != id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ── Streams ──────────────────────────────────────────────────────

    fn slot_index(&self, channel: &str) -> Option<usize> {
        self.slots.iter().position(|slot| slot.channel == channel)
    }

    /// Open `channel`. With `solo`, it becomes the only pane; otherwise it is
    /// added alongside whatever is already playing.
    fn open_channel(
        &mut self,
        channel: String,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page = Page::Watch;

        if self.slot_index(&channel).is_some() {
            // Already open, so switch to it rather than restarting it. Solo
            // closes the others; dropping them stops their streamlink and mpv.
            if solo {
                self.slots.retain(|slot| slot.channel == channel);
            }
            self.set_background(false, cx);
            cx.notify();
            return;
        }

        if solo {
            self.slots.clear();
        } else if self.slots.len() >= MAX_PANES {
            self.toast(format!("already watching {MAX_PANES} streams"), cx);
            return;
        }

        let chat = cx.new(|cx| {
            ChatView::new(
                channel.clone(),
                self.settings.chat_history,
                self.cache.clone(),
                window,
                cx,
            )
        });
        self.slots.push(Slot {
            channel: channel.clone(),
            quality_override: None,
            state: StreamState::Starting,
            chat,
            supervisor: None,
            pump: None,
            hovered: false,
            chat_hidden: self.settings.chat_hidden_for(&channel),
        });

        self.active = Some(channel.clone());
        self.start_stream(channel, window, cx);
        self.set_background(false, cx);

        // A record of what was last watched, even though launch no longer
        // reopens it automatically.
        if let Some(first) = self.slots.first() {
            self.settings.last_channel = Some(first.channel.clone());
            if let Err(e) = self.settings.save_preferences(&self.settings_path) {
                eprintln!("settings: could not save: {e}");
            }
        }
    }

    /// Start, or restart, streamlink for an existing slot.
    fn start_stream(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(&channel) else {
            return;
        };

        let quality = self.slots[index]
            .quality_override
            .clone()
            .or(match &self.settings.quality {
                QualityPreference::Auto => None,
                QualityPreference::Fixed(name) => Some(name.clone()),
            });
        let options = StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
        };

        // Quality targets the pane this stream will actually render into, in
        // physical pixels. With several panes the window is shared, so each one
        // asks for proportionally less.
        let scale = window.scale_factor();
        let aspect = window_aspect(window);
        let (rows, _) = layout::grid_shape(self.slots.len().max(1), aspect);
        let pane_height = (f32::from(window.viewport_size().height) * scale / rows as f32)
            .round()
            .clamp(180.0, video::MAX_RENDER_HEIGHT as f32) as u32;

        let (supervisor, mut events) =
            StreamSupervisor::start(channel.clone(), pane_height, options);
        // Read once, here, and frozen into the pump: the pane it belongs to
        // may not start for several seconds, and adjusting a *different* pane
        // in the meantime must not follow it in.
        let volume = self
            .volume_override
            .unwrap_or_else(|| self.settings.volume_for(&channel));

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let channel = channel.clone();
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.apply_stream_event(&channel, event, volume, window, cx)
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        self.slots[index].supervisor = Some(supervisor);
        self.slots[index].pump = Some(pump);
    }

    fn apply_stream_event(
        &mut self,
        channel: &str,
        event: StreamEvent,
        volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Look the slot up by channel rather than by a captured index: panes
        // close while streams are still starting, and an index would go stale.
        let Some(index) = self.slot_index(channel) else {
            return;
        };

        match event {
            StreamEvent::Resolving => self.slots[index].state = StreamState::Starting,
            StreamEvent::Ready {
                url,
                quality,
                available,
            } => {
                match VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume) {
                    Ok((stream, frames)) => {
                        let label = SharedString::from(quality);
                        let view = cx.new(|cx| {
                            VideoView::from_stream(stream, frames, label, available, window, cx)
                        });
                        let owner = channel.to_string();
                        cx.subscribe_in(
                            &view,
                            window,
                            move |this: &mut RootView, _, event, window, cx| match event {
                                VideoEvent::VolumeChanged(volume) => {
                                    // Remembered against the channel rather
                                    // than globally, so coming back to a
                                    // streamer finds them where you left them.
                                    // The guard matters: a slider drag emits a
                                    // change per pixel, and each one of these
                                    // is a full read-modify-write of the file.
                                    if this.settings.set_volume_for(&owner, *volume) {
                                        if let Err(e) =
                                            this.settings.save_preferences(&this.settings_path)
                                        {
                                            eprintln!("settings: could not save: {e}");
                                        }
                                    }
                                    cx.notify();
                                }
                                VideoEvent::QualityRequested(name) => {
                                    if let Some(index) = this.slot_index(&owner) {
                                        this.slots[index].quality_override = Some(name.clone());
                                        this.slots[index].state = StreamState::Starting;
                                        this.start_stream(owner.clone(), window, cx);
                                        cx.notify();
                                    }
                                }
                            },
                        )
                        .detach();
                        self.slots[index].state = StreamState::Playing(view);
                    }
                    Err(e) => self.slots[index].state = StreamState::Failed(e.to_string().into()),
                }
            }
            StreamEvent::Offline => self.slots[index].state = StreamState::Offline,
            StreamEvent::Failed { reason } => {
                self.slots[index].state = StreamState::Failed(reason.into())
            }
        }
        cx.notify();
    }

    fn close_slot(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.slots.len() {
            // Dropping the slot stops its streamlink and its mpv.
            self.slots.remove(index);
        }
        if self.slots.is_empty() {
            self.page = Page::Browse;
        }
        cx.notify();
    }

    /// Mute or unmute every pane at once, for moving between pages.
    fn set_background(&mut self, background: bool, cx: &mut Context<Self>) {
        for slot in &self.slots {
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_background(background, cx));
            }
        }
    }

    /// Leave the watch page, keeping streams as muted thumbnails.
    ///
    /// Thumbnails are genuinely cheaper, not just smaller: render size follows
    /// the element, so a small miniplayer decodes into a small buffer.
    fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Browse;
        self.set_background(true, cx);
        cx.notify();
    }

    fn go_watch(&mut self, cx: &mut Context<Self>) {
        if self.slots.is_empty() {
            return;
        }
        self.page = Page::Watch;
        self.set_background(false, cx);
        cx.notify();
    }

    fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.slots.clear();
        self.page = Page::Browse;
        cx.notify();
    }

    // ── Settings ─────────────────────────────────────────────────────

    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.take().is_some() {
            cx.notify();
            return;
        }

        let panel = cx.new(|cx| {
            SettingsPanel::new(self.settings.clone(), self.sign_in.summary(), window, cx)
        });
        cx.subscribe_in(
            &panel,
            window,
            |this: &mut RootView, _, event, window, cx| {
                match event {
                    SettingsEvent::Dismissed => this.settings_panel = None,
                    SettingsEvent::Saved(updated) => {
                        let client_id_changed =
                            this.settings.credentials.client_id != updated.credentials.client_id;
                        let stream_changed = this.settings.quality != updated.quality
                            || this.settings.credentials.auth_token
                                != updated.credentials.auth_token;

                        this.settings = (**updated).clone();
                        // A new client id invalidates any stored sign-in, so
                        // that case drops the tokens rather than keeping them.
                        let saved = if client_id_changed {
                            this.settings.save_forgetting_sign_in(&this.settings_path)
                        } else {
                            this.settings.save_preferences(&this.settings_path)
                        };
                        if let Err(e) = saved {
                            eprintln!("settings: could not save: {e}");
                        }
                        this.settings_panel = None;

                        // Apply immediately rather than asking for a restart,
                        // which is the entire reason this panel exists.
                        if client_id_changed {
                            this.sign_in = SignIn::Connecting;
                            this.follows.clear();
                            this.offline.clear();
                            this.known_live.clear();
                            // Browsing was fetched with the old app's token, so
                            // it goes with it.
                            this.discovery = Discovery::default();
                            let (service, pump) =
                                Self::spawn_twitch(this.settings_path.clone(), window, cx);
                            this.twitch = service;
                            this._twitch_pump = pump;
                        }
                        if stream_changed {
                            let channels: Vec<String> =
                                this.slots.iter().map(|s| s.channel.clone()).collect();
                            for channel in channels {
                                if let Some(index) = this.slot_index(&channel) {
                                    this.slots[index].quality_override = None;
                                    this.slots[index].state = StreamState::Starting;
                                }
                                this.start_stream(channel, window, cx);
                            }
                        }
                    }
                }
                cx.notify();
            },
        )
        .detach();

        self.settings_panel = Some(panel);
        cx.notify();
    }

    // ── Chrome ───────────────────────────────────────────────────────

    fn pill(
        &self,
        id: &'static str,
        label: SharedString,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(ElementId::from(id))
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded_sm()
            .bg(theme::surface_raised())
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text_muted())
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            .active(|style| style.bg(theme::pressed()))
            .child(label)
            .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
    }

    /// A pill that says whether it is the list you are looking at.
    fn tab_pill(&self, tab: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.discovery.tab == tab;
        div()
            .id(ElementId::from(tab.label()))
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded_sm()
            .bg(if selected {
                theme::accent_dim()
            } else {
                theme::surface_raised()
            })
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .line_height(px(theme::LINE_TIGHT))
            .text_color(if selected {
                theme::text()
            } else {
                theme::text_muted()
            })
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            .active(|style| style.bg(theme::pressed()))
            .child(tab.label())
            .on_click(cx.listener(move |this, _event, _window, cx| this.show_tab(tab, cx)))
    }

    fn toast_stack(&self) -> impl IntoElement {
        let mut stack = div()
            .absolute()
            .top_4()
            .right_4()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .items_end();

        for toast in &self.toasts {
            stack = stack.child(
                toast.fade.apply(
                    ("toast", toast.id),
                    theme::MOTION_ENTER,
                    div()
                        // Per card rather than on the stack: the stack is
                        // `items_end`, so its box is as wide as the widest
                        // toast and would blanket the search box beside it.
                        .block_mouse_except_scroll()
                        .px(px(theme::PANEL_PAD))
                        .py(px(theme::GAP))
                        .rounded_md()
                        .bg(theme::surface_raised())
                        .border_l_2()
                        .border_color(theme::accent())
                        .shadow_lg()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_BODY))
                        .text_color(theme::text())
                        .child(toast.text.clone()),
                ),
            );
        }
        stack
    }

    /// Muted thumbnails of whatever is playing, shown while browsing.
    fn miniplayers(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.slots.is_empty() {
            return None;
        }

        let mut strip = div()
            .absolute()
            .bottom_4()
            .right_4()
            .flex()
            .flex_row()
            .items_end()
            .gap(px(theme::GAP_TIGHT));

        for slot in &self.slots {
            let Some(video) = slot.video() else {
                continue;
            };
            let id = ElementId::from(SharedString::from(format!("mini-{}", slot.channel)));
            strip = strip.child(
                div()
                    .id(id)
                    .w(px(220.))
                    .rounded_md()
                    .overflow_hidden()
                    .bg(theme::player_bg())
                    .border_1()
                    .border_color(theme::border())
                    // The whole tile is a button back into the stream, which
                    // nothing about a bordered thumbnail says on its own.
                    .hover(|style| style.border_color(theme::accent()))
                    .shadow_lg()
                    .cursor_pointer()
                    // Per tile rather than on the strip. The strip is
                    // `items_end`, so its box is as tall as a tile even where
                    // it only holds the short "stop all" pill — occluding that
                    // would leave an invisible dead patch over the card grid.
                    .block_mouse_except_scroll()
                    .on_click(cx.listener(|this, _event, _window, cx| this.go_watch(cx)))
                    .child(div().h(px(124.)).w_full().child(video.clone()))
                    .child(
                        div()
                            .px(px(theme::ROW_PAD_X))
                            .py(px(theme::CONTROL_PAD_Y))
                            .bg(theme::surface())
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(format!("{} · muted", slot.channel))),
                    ),
            );
        }

        Some(
            strip
                .child(
                    // Wrapped so the blocking area is exactly the pill. Clicking
                    // this used to stop every stream *and* open whichever card
                    // happened to be behind it.
                    div().block_mouse_except_scroll().child(self.pill(
                        "mini-stop",
                        "stop all".into(),
                        cx,
                        |this, _window, cx| this.stop_all(cx),
                    )),
                )
                .into_any_element(),
        )
    }

    // ── Pages ────────────────────────────────────────────────────────

    fn browse_page(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let watching = self.slots.len();
        let header = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP))
            .px(px(theme::PAGE_PAD))
            .py(px(theme::PANEL_PAD))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .text_size(px(theme::TEXT_TITLE))
                    .font_weight(theme::weight_title())
                    .text_color(theme::text())
                    .child(APP_NAME),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(self.sign_in.summary()),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .gap(px(theme::GAP_TIGHT))
                    .children(Tab::ALL.map(|tab| self.tab_pill(tab, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .w(px(260.))
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .when(watching > 0, |header| {
                header.child(self.pill(
                    "resume",
                    format!("watching {watching}").into(),
                    cx,
                    |this, _window, cx| this.go_watch(cx),
                ))
            })
            .child(self.pill(
                "refresh",
                if self.refreshing {
                    "refreshing…".into()
                } else {
                    "refresh".into()
                },
                cx,
                |this, _window, cx| this.refresh(cx),
            ))
            .child(self.pill(
                "open-settings",
                "settings".into(),
                cx,
                |this, window, cx| this.toggle_settings(window, cx),
            ));

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .child(header)
            .child(browse::page(
                &self.follows,
                &self.offline,
                &self.discovery,
                &self.sign_in,
                &self.cache,
                self.slots.len() < MAX_PANES,
                |this: &mut RootView, action, window, cx| this.on_browse_action(action, window, cx),
                cx,
            ))
            .children(self.miniplayers(cx))
    }

    fn watch_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(watch::page(
                &self.slots,
                &self.follows,
                window.viewport_size(),
                self.settings.chat_width,
                |this: &mut RootView, index, _window, cx| this.close_slot(index, cx),
                |this: &mut RootView, index, hovered, cx| {
                    // Only repaint when the pointer crosses a boundary; most
                    // moves are within the pane it is already in.
                    match this.slots.get_mut(index) {
                        Some(slot) if slot.hovered != hovered => slot.hovered = hovered,
                        _ => return,
                    }
                    // Sticky, unlike `hovered`: a keyboard shortcut has to keep
                    // working once the pointer has moved into chat or off the
                    // window entirely, and the pane you last looked at is the
                    // one you meant.
                    if hovered {
                        this.active = this.slots.get(index).map(|slot| slot.channel.clone());
                    }
                    let over_video = this.slots.iter().any(|slot| slot.hovered);
                    this.nav.set(over_video);
                    cx.notify();
                },
                cx,
            ))
            .child(
                // The only page-level control on the watch page, in the corner
                // a back control belongs in, and revealed by the same gesture
                // as everything else: point at the video and the controls come
                // up, look away and the picture is all that is left.
                //
                // Settings is not here on purpose: it is set once and forgotten,
                // and per-stream quality already lives in the control bar. It is
                // on the follows page, one click away.
                self.nav.apply(
                    "watch-nav",
                    theme::MOTION_HOVER,
                    div()
                        .absolute()
                        .top(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .child(
                            self.pill("back", "← follows".into(), cx, |this, _window, cx| {
                                this.go_browse(cx)
                            }),
                        ),
                ),
            )
    }
}

/// Window width divided by height, guarding against a zero-height window during
/// a minimise.
fn window_aspect(window: &Window) -> f32 {
    let size = window.viewport_size();
    f32::from(size.width) / f32::from(size.height).max(1.0)
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            Page::Browse => self.browse_page(cx).into_any_element(),
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
            .on_action(cx.listener(Self::on_close_pane))
            .on_action(cx.listener(Self::on_go_browse))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_refresh))
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
            .child(self.toast_stack())
            .children(self.settings_panel.clone())
    }
}

fn main() {
    // Before anything that could go wrong: a windowed build has no console, so
    // stderr must be pointed somewhere first or the first failure is silent.
    // If this fails there is, by construction, nowhere to say so.
    if !cfg!(debug_assertions) {
        let _ = diagnostics::capture_stderr();
    }

    let mut args = std::env::args().skip(1);
    let mut channels: Vec<String> = Vec::new();
    let mut volume = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--volume" => {
                volume = args
                    .next()
                    .and_then(|v| v.parse::<u8>().ok())
                    .map(|v| v.min(100))
            }
            "--help" | "-h" => {
                eprintln!("usage: {APP_NAME} [channel...] [--volume 0-100]");
                eprintln!();
                eprintln!("A release build is windowed, so this text goes to");
                eprintln!("{}", diagnostics::log_path().display());
                eprintln!();
                eprintln!("Name up to {MAX_PANES} channels to open them side by side.");
                eprintln!("With no channel, opens on the follows page.");
                eprintln!(
                    "Settings live at {}",
                    settings::default_path(APP_NAME).display()
                );
                std::process::exit(0);
            }
            other => channels.push(other.trim_start_matches('#').to_string()),
        }
    }

    Application::new().run(move |cx: &mut App| {
        // Must come before any gpui-component widget is constructed.
        gpui_component::init(cx);
        // After it, not before: same-depth ties are won by whoever registered
        // last, and these bindings are the ones that must stand aside.
        keys::init(cx);

        let bounds = Bounds::centered(None, size(px(1600.), px(920.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let root = cx.new(|cx| RootView::new(channels.clone(), volume, window, cx));
            // Root is required as the window's first child so overlay layers
            // have somewhere to render.
            cx.new(|cx| gpui_component::Root::new(AnyView::from(root), window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
