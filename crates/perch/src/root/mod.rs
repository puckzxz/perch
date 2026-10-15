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
//! | `account` | the sign-in as the user drives it: Sign out from the settings sheet, Sign in, Sign in again for the chat scope, and the worker started over, as a new client id does too |
//! | `chat_send` | sending chat: what each live chat's composer is told about the user (`sync_chat_access`), a message handed to the worker, and its answer handed back to the chat it came from |
//! | `browsing` | the browse page's requests: tabs, search, categories, channels |
//! | `navigation` | back and forward: where the app is as a `Route`, recording each step on the trail (`crate::trail`), and the three ways along it |
//! | `streams` | opening, restarting and closing panes, and swapping one for a recording in place |
//! | `renditions` | what each pane plays, and when it restarts: the quality chosen against each pane's own height (`pane_height_for`), the upward re-pick when the grid changes, a pick from the pane's menu; a new rendition resolved beside the picture and kept once its player has taken over in place (`video_view::swap`) |
//! | `panes` | where each pane is drawn, applied: `restage`, the one funnel every change of state, membership, page, pop-out or maximize ends in; `set_slot_state`, the only write of a pane's state; `video_in_main`, the only way the main window reaches a player; a pane given the watch page and every pane shown again (`toggle_maximize`, `show_all_panes`), and `choose`, which takes the maximize to the pane chosen; two panes swapping places in the order (`move_pane`), from a header dropped on a pane or `Shift+←`/`Shift+→` |
//! | `pop_out` | a pane in a window of its own, on top of other apps: moving its picture there and back, the window, and `to_root`, the only way back from it |
//! | `hearing` | which panes are heard: a pane menu's `Only this one` and `Hear all again`, through Mute all's hush, read from the hushes as the menu is drawn (`hear_only_now`); More's `Audio compressor`, every pane's at once (`toggle_compressor`) |
//! | `ad_breaks` | a pane's ad-break notice: begun when streamlink says it is filtering an ad, counted down once a second, taken down when it is over |
//! | `broadcasts` | what a stopped live pane asks about its channel's past broadcasts, and whether it can start by itself |
//! | `channel_menus` | the right-click menu about a channel (`crate::channel_menu`): opened on a card, an offline name, a rail row, a name in chat or a pane, what it knows each time it is drawn, and a pick carried out — a pane's by its key, as its own controls go |
//! | `pane_actions` | what a pane asks for, by its key: its controls, and its player's requests; the header a pane key reveals; `run_guard`, which swallows the rest of a double-click whose first press took a player from under the pointer |
//! | `launches` | what the command line named, at startup and from later launches |
//! | `history` | what has been watched: noting where each recording got to, resuming there |
//! | `saving` | writing the settings and the history down: each save a copy handed to one writer thread (`settings::writer`), a failed one said, and the wait for the last saves as the window closes |
//! | `muted` | a recording's muted stretches: heard from every Helix answer that lists videos, given to a pane that opens one without them, asked for once when nothing has been heard, and taken onto every pane playing it and its player's seek bar |
//! | `prefs` | the settings sheet, the divider drag, how every chat is drawn, the rail folding and what is pinned to it |
//! | `recommended` | the rail's Recommended group: when to ask the worker, and what its answer becomes |
//! | `shared_chat` | the chats' Shared Chat labels: hearing an unnamed partner, asking its name, handing it to every chat; and `watch_chat`, which listens to every chat made |
//! | `badges` | the chats' badges: hearing a chat's room, asking the worker for the global and the room's badge books, handing each to the chats it is for |
//! | `cheers` | the live chats' cheers: hearing a live chat's room, asking the worker for its cheermotes, handing the answer to the live chats in that room |
//! | `viewer_cards` | the chats' viewer cards: hearing a card open, asking the worker who its person is, telling every open card what is known (`sync_viewer_cards`), and a card's `Watch` and `Channel page` |
//! | `schedule` | when an offline channel says it will be on next: asked of the worker once a session, the first time its page or a pane that found it off is drawn, and the answer kept |
//! | `chrome` | pills, toasts, the rail |
//! | `title_bar` | the bar Perch draws across the top of the window: the rail button, Browse, search, settings, and on Windows the caption buttons |
//! | `search_box` | the title bar's search box: what it suggests as you type (the palette's channels and recordings, under asking Twitch), and the Twitch search it runs on `Enter` or once the typing pauses |
//! | `pages` | the two pages, assembled, each only its own column |
//!
//! A sibling reaches the fields directly — they are private to this module,
//! and a child module is inside it — so the split costs no accessors. Its
//! methods are `pub(super)`: callable from the rest of the root, and nowhere
//! else.

mod account;
mod ad_breaks;
mod badges;
mod broadcasts;
mod browsing;
mod channel_menus;
mod chat_send;
mod cheers;
mod chrome;
mod commands;
mod follows;
mod hearing;
mod history;
mod last_live;
mod launches;
mod muted;
mod navigation;
mod pages;
mod pane_actions;
mod pane_streams;
mod panes;
mod pop_out;
mod prefs;
mod recommended;
mod renditions;
mod rewind;
mod saving;
mod schedule;
mod search_box;
mod shared_chat;
mod shortcuts;
mod streams;
mod title_bar;
mod viewer_cards;
mod warm;

pub(crate) use self::pop_out::offered_here as pop_out_offered;
pub(crate) use self::title_bar::window_min_size;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use emotes::ImageCache;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    div, prelude::*, px, AnyWindowHandle, Bounds, Context, Entity, FocusHandle, Pixels,
    ScrollHandle, SharedString, Subscription, Task, WeakEntity, Window,
};
use gpui_component::input::{InputEvent, InputState};
use settings::history::{Forgotten, History};
use settings::writer::Writer;
use settings::Settings;
use twitch_api::{Channel, LiveStream};

use self::follows::LiveList;
use self::navigation::Route;
use self::pop_out::PoppedOut;
use crate::browse::{self, Discovery, SignIn};
use crate::chat::ChatView;
use crate::chat_badges::Library;
use crate::last_live::LastLive;
use crate::launch::Launch;
use crate::layout::Body;
use crate::recommended::Recommended;
use crate::schedule::Schedules;
use crate::settings_view::SettingsPanel;
use crate::shared_chat::SourceRooms;
use crate::stage::Stage;
use crate::trail::Trail;
use crate::twitch::{SignInStart, TwitchService};
use crate::video_view::VideoView;
use crate::watch::{ResizeStart, Slot, StreamState, MAX_PANES};
use crate::{
    chat_cheers, chat_profiles, cpu_log, keys, layout, motion, notifications, rail_preview,
    sidebar, theme, vod, APP_NAME,
};

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
    /// How long it has left, which stops going down while the pointer is on
    /// the stack; see `RootView::hold_toasts`.
    countdown: chrome::Countdown,
    /// The one timer that takes it down when its countdown runs out, held
    /// only while the countdown runs. Dropping it is what cancels it, so a
    /// held toast, an answered one and one taken down keep no timer behind.
    timer: Option<Task<()>>,
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
    /// Read by each sign-in worker the root starts; the root's own saves
    /// go through `saves`, which has the path.
    settings_path: PathBuf,
    /// Every recording opened, and where each was left. A file of its own
    /// beside the settings; see `settings::history`.
    history: History,
    /// Where every save of the settings and the history goes, to be written
    /// off the UI thread in the order it was made; see `root::saving`.
    saves: Writer,
    /// Says on screen a save that failed; see `start_saving`.
    _saves_pump: Task<()>,
    /// Which scheduled history save is the newest; see `save_history_soon`.
    history_epoch: u64,
    /// Notes where the recordings playing have got to, every few seconds;
    /// see `keep_history`. Dropping it stops that.
    _history_tick: Task<()>,
    cache: Arc<ImageCache>,

    page: Page,
    /// The panes, in their order: the grid's, the keys `1`–`4` and `Tab`
    /// walk, and the palette's. The one account of it,
    /// which a header dropped on another pane and `Shift+←`/`Shift+→`
    /// rearrange (`move_pane`) for the session: the order is never saved.
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
    /// The same channels as `offline`, in Following's order rather than the
    /// rail's: by when you last watched them (`following::by_last_watched`), not
    /// by name. Its own list because each holds still under the pointer by
    /// the same rule — sorted only where `offline` is put back in name order,
    /// never while held — and a sort drawn every frame from the other would
    /// move names under the pointer; see `RootView::hold_live`.
    following_offline: Vec<Channel>,
    /// The pins in the order the rail last drew them: `sidebar::pin_order`'s,
    /// or held still under the pointer; see `RootView::order_pins`. Only an
    /// order: which channels are pinned is `settings.pinned`'s to say.
    pin_order: Vec<String>,
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
    /// Whether the last list of everyone followed was all of them, not cut
    /// short at its page cap (`twitch_api::Followed::complete`). False until
    /// that list has come back — the live list alone, which also sets
    /// `follows_loaded`, says nothing of who is followed and offline — so a
    /// chat's followers-only check never says "only followers" on less than
    /// the whole list; see `composer::follows`.
    follows_complete: bool,
    /// Whether a list of everyone followed has come back since the worker
    /// started, cut short or not: what tells a channel nobody follows from
    /// one followed and off (`RootView::followed`). `follows_loaded` cannot,
    /// since the live list sets it too, seconds before this list arrives, and
    /// for those seconds every offline follow would read as not followed and
    /// be asked about as if it were not (`root::pane_streams`).
    followed_loaded: bool,
    /// Who was live at the last poll, so newly-live channels can be told apart
    /// from ones that were already streaming. Without this every poll would
    /// re-announce everybody.
    known_live: HashSet<String>,
    /// Whether the worker's first live list has come back, which seeds
    /// `known_live` in silence. Its own flag rather than an empty
    /// `known_live`, which is also every poll after one where nobody followed
    /// was live; false again with the worker stopped (`stop_twitch`), so a
    /// restarted one's first poll is silent too.
    streams_seeded: bool,
    /// The broadcast each followed channel was last seen live with, so a
    /// desktop notification is sent once a broadcast; see `notifications`.
    /// Kept across a restarted worker, which only forgets `known_live`: a
    /// broadcast seen before it is no news after.
    seen_broadcasts: notifications::Seen,
    /// Whether the pointer is over the rail, and over Following, as
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
    /// Which live rail row the pointer is on, and whether its preview card
    /// is up yet: told by the rows' probes (`RootView::point_rail_row`),
    /// put away by a press on the rail, forgotten whenever the rail is not
    /// drawn or something is over it. See `crate::rail_preview`.
    rail_preview: rail_preview::Preview,
    /// The rail's Recommended group: the asks out, the answers in, and the
    /// rows shown. For this session only; see `crate::recommended`.
    recommended: Recommended,
    /// When each offline follow was last live, by login, and the asks for
    /// it: the words under the names on Following and in the rail. For this
    /// session only; see `crate::last_live`.
    last_live: LastLive,
    /// The names of the channels Shared Chat lines were copied from, by
    /// their numeric ids, and the asks for them: every chat's labels, which
    /// each chat mirrors. For this session only; see `crate::shared_chat`.
    source_rooms: SourceRooms,
    /// The chat badges Helix has answered with, the global book and each
    /// room's, and the asks for them: what every chat draws its badges with,
    /// which each chat mirrors. For this session only; see
    /// `crate::chat_badges`.
    badges: Library,
    /// The cheermotes Helix has answered with, by room, and the asks for
    /// them: what every live chat draws its cheers with, which each mirrors.
    /// For this session only; see `crate::chat_cheers`. Named by its module,
    /// since `Library` alone is the badges'.
    cheers: chat_cheers::Library,
    /// The chatters' accounts Helix has answered with, by login, and the
    /// asks for them: what a chat's viewer card shows of its person beyond
    /// their lines, which each open card is told. For this session only;
    /// see `crate::chat_profiles`.
    profiles: chat_profiles::Profiles,
    /// Each channel's stream schedule Helix has answered with, and the asks
    /// for them: the line saying when an offline channel is on next, on its
    /// page and on a pane that found it off. For this session only; see
    /// `crate::schedule`.
    schedules: Schedules,
    /// The streams of channels nothing else the app asks would say are on —
    /// a live pane's that no fetched list carries, a stopped pane's or a
    /// pin's that nobody follows — asked of Helix by login, in one ask, so
    /// such a pane has a timeline and a header and starts when the channel
    /// does, and such a pin says whether it is live; see `root::pane_streams`.
    /// Never merged into `follows`.
    pane_streams: pane_streams::PaneStreams,
    sign_in: SignIn,
    /// Whether the signed-in token may send chat, from Twitch's token
    /// validation (`TwitchEvent::ChatScope`) or a send refused for want of
    /// the scope; `None` until either has said, and from every restart of
    /// the worker. What a composer closes with `Sign in again to chat` on;
    /// see `chat_send`.
    chat_scope: Option<bool>,
    /// The sends handed to the worker and not yet answered, by the root's
    /// own number for each, with the chat each came from; see
    /// `RootView::send_chat`.
    chat_sends: HashMap<u64, WeakEntity<ChatView>>,
    /// The last of those numbers handed out.
    chat_send_seq: u64,
    /// Everything the browse page shows besides your follows.
    discovery: Discovery,
    /// Where the app has been, for back and forward. History only: where it
    /// is now is always read from `page` and `discovery`; see `navigation`.
    trail: Trail<Route>,
    /// A step is being recorded, or replayed, so the functions it runs
    /// through do not record steps of their own; see `RootView::record`.
    recording: bool,
    /// The title bar's search box; see `search_box`.
    search: Entity<InputState>,
    /// Which of the box's suggestions `Enter` runs: an index into those
    /// worked out at render, clamped there, back to the first — asking
    /// Twitch — on every keystroke.
    search_selected: usize,
    /// The suggestions put away — `Esc`, a pick, `Enter` — until the next
    /// keystroke.
    search_dismissed: bool,
    /// The wait for the typing to pause before Twitch is asked; a newer
    /// keystroke drops it (`search_typed`).
    search_settle: Option<Task<()>>,
    /// Where the box's left edge is in the window, as its probe last
    /// measured it (`search_probe`): where the suggestions hang from.
    search_left: f32,
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

    /// The pane whose header is being dragged onto another, by key: from
    /// the moment the drag begins (`PaneAction::BeginMove`) until it is let
    /// go, on a pane or not, or `Esc` lets go of it. What the watch page
    /// mounts its drop layers by (`watch::page`), and what keeps the panes
    /// the pointer crosses on the way from taking the keys. gpui knows that
    /// something is being dragged (`App::has_active_drag`) but not what — the
    /// volume slider drags too — so the root keeps which. For the session
    /// only, like the order it changes.
    pane_move: Option<String>,

    /// The pane whose chat options menu is open, by key, if one is: one at a
    /// time across every pane (`watch::chat_menu_toggled`). Opened by the icon
    /// in a pane's header, closed by a press anywhere else, a row of it
    /// included, or `Esc`. For the session only, like everything about a
    /// menu.
    chat_menu: Option<String>,

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
    /// The right-click menu about a channel, while it is open; see
    /// `channel_menus`. Numbered by how many have opened, which keys each
    /// one's arrival.
    channel_menu: Option<channel_menus::OpenChannelMenu>,
    channel_menus_opened: u64,
    /// Which row Enter would run. An index into the entries computed at render,
    /// clamped there — the list changes under it on every keystroke.
    palette_selected: usize,

    settings_panel: Option<Entity<SettingsPanel>>,
    toasts: Vec<Toast>,
    next_toast: u64,
    /// Whether the pointer was on a toast at the last frame, from the
    /// stack's probes: while it is, no toast counts down. See
    /// `RootView::hold_toasts`.
    toasts_held: bool,
    _cache_pump: Task<()>,

    /// Recordings opened by link that are still to be looked up.
    linked_videos: Vec<LinkedVideo>,
    /// What Helix has said of recordings' muted stretches this session, and
    /// which a pane is asking about; see `muted`.
    muted: muted::Muted,
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
        // Before any player starts, each of which reads it from its first
        // sample; see `crate::compressor`.
        crate::compressor::set(settings.compressor);
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

        // Before the worker, which reads the settings file only once what
        // was handed to the writer ahead of it has landed.
        let (saves, saves_pump) = Self::start_saving(settings_path.clone(), history_path, cx);
        let (service, twitch_pump) = Self::spawn_twitch(
            settings_path.clone(),
            SignInStart::Stored,
            saves.landed(),
            window,
            cx,
        );

        // In the title bar, over both pages; see `search_box` for what it
        // suggests and when it asks Twitch, and `run_search` for what a
        // search from the watch page does.
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search channels, categories and your history")
        });
        cx.subscribe_in(
            &search,
            window,
            |this: &mut RootView, _, event, window, cx| match event {
                InputEvent::Change => this.search_typed(cx),
                InputEvent::PressEnter { .. } => this.search_entered(window, cx),
                _ => {}
            },
        )
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
            saves,
            _saves_pump: saves_pump,
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
            following_offline: Vec::new(),
            pin_order: Vec::new(),
            refreshing: false,
            avatars: HashMap::new(),
            follows_loaded: false,
            follows_complete: false,
            followed_loaded: false,
            known_live: HashSet::new(),
            streams_seeded: false,
            seen_broadcasts: notifications::Seen::default(),
            rail_pointed: false,
            following_pointed: false,
            rail_offline_open: false,
            rail_preview: rail_preview::Preview::default(),
            recommended: Recommended::default(),
            last_live: LastLive::default(),
            source_rooms: SourceRooms::default(),
            badges: Library::default(),
            cheers: chat_cheers::Library::default(),
            profiles: chat_profiles::Profiles::default(),
            schedules: Schedules::default(),
            pane_streams: pane_streams::PaneStreams::default(),
            sign_in: SignIn::Connecting,
            chat_scope: None,
            chat_sends: HashMap::new(),
            chat_send_seq: 0,
            discovery: Discovery::default(),
            trail: Trail::default(),
            recording: false,
            search,
            search_selected: 0,
            search_dismissed: false,
            search_settle: None,
            search_left: 0.0,
            scrolls: browse::Scrolls::default(),
            rail_scroll: ScrollHandle::new(),
            twitch: service,
            _twitch_pump: twitch_pump,
            active: None,
            focus,
            _focus_lost,
            volume_override: launch.volume,
            pane_move: None,
            chat_menu: None,
            resize: None,
            palette_input,
            palette_open: false,
            channel_menu: None,
            channel_menus_opened: 0,
            palette_selected: 0,
            settings_panel: None,
            toasts: Vec::new(),
            next_toast: 0,
            toasts_held: false,
            _cache_pump: cache_pump,
            linked_videos: Vec::new(),
            muted: muted::Muted::default(),
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

    /// Whether the browse page shares the window with the panes: up, with
    /// a pane at home to draw beside it. Browsing never puts a pane away —
    /// it docks on the left (`layout::panel_width`) and the panes go on
    /// playing beside it, in their own grid, where a click on one is a click
    /// on a pane. A pane in a window of its own is on screen already, so
    /// panes that are all popped out leave the browse page the window.
    pub(super) fn docks_stage(&self) -> bool {
        self.page == Page::Browse
            && self
                .slots
                .iter()
                .any(|slot| !self.stage.is_popped(&slot.key))
    }

    /// Whether the main window draws its panes: on the watch page, and beside
    /// the browse page while it shares the window with them
    /// ([`docks_stage`](Self::docks_stage)). Where a pane is placed
    /// (`stage::Stage::place`) follows this and nothing else.
    pub(super) fn stage_drawn(&self) -> bool {
        self.page == Page::Watch || self.docks_stage()
    }

    /// How wide the browse page is: the body, or its share while the panes
    /// are beside it ([`layout::panel_width`]).
    pub(super) fn browse_width(&self, window: &Window) -> f32 {
        let body = self.body(window);
        if self.docks_stage() {
            layout::panel_width(body.width)
        } else {
            body.width
        }
    }

    /// The room the panes have: the body, less the browse page while it
    /// shares the window with them.
    fn stage_body(&self, window: &Window) -> Body {
        let body = self.body(window);
        if self.docks_stage() {
            body.beside(layout::panel_width(body.width))
        } else {
            body
        }
    }

    /// The watch grid as it is drawn now: [`layout::Grid::of`] the room the
    /// panes have ([`stage_body`](Self::stage_body)) and the
    /// [`cells`](Self::cells). The one grid the page is drawn in and a
    /// divider drag is measured against, so the two cannot disagree about a
    /// pane's size. A pane's quality is chosen for the grid it has with the
    /// browse page away, the watch page's ([`watch_grid`](Self::watch_grid)),
    /// so browsing beside it never chooses for it.
    fn grid(&self, window: &Window) -> layout::Grid {
        layout::Grid::of(self.stage_body(window), self.cells().len())
    }

    /// The grid the watch page draws, the browse page away: what a pane's
    /// quality is chosen for (`pane_height_for`). The panes are smaller while
    /// the browse page is beside them, and a quality chosen there would be
    /// the first thing undone on the way back to watching; and since a
    /// rendition only ever moves up, it would not be undone at all.
    fn watch_grid(&self, window: &Window) -> layout::Grid {
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

        // The browse page while it is up, and the panes while the main window
        // draws them: the watch page, or beside the browse page
        // (`docks_stage`). Two siblings rather than one page, so a pane is
        // the same element whether browsing is beside it or not, and opening
        // or closing the browse page remounts no player and replays no fade.
        let browse = (self.page == Page::Browse).then(|| {
            let width = self.docks_stage().then(|| self.browse_width(window));
            (width, self.browse_page(window, cx).into_any_element())
        });
        let stage = self
            .stage_drawn()
            .then(|| self.watch_page(window, cx).into_any_element());
        // Drawn here, once, beside whichever page is up, rather than by each
        // page: it does not fade out and back in with the page, and the probe
        // that holds its order still while it is pointed at has one owner.
        let rail_shown = self.rail_shown(window);
        let rail = self.follows_rail(rail_shown, window, cx).map(|rail| {
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
            .on_action(cx.listener(Self::on_move_pane_earlier))
            .on_action(cx.listener(Self::on_move_pane_later))
            .on_key_down(cx.listener(Self::on_palette_key))
            .on_key_down(cx.listener(Self::on_search_key))
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
                // Everything under the bar. The modals and the toasts are in
                // here rather than on the root, so their `inset_0` and their
                // offsets start below the bar by construction: a scrim cannot
                // cover the caption buttons, and a toast cannot land on the
                // drag strip — `block_mouse_except_scroll` does not hide a
                // drag area from the platform's hit test, so a toast there
                // would move the window when pressed. No id, for the reason
                // the root has none: an id namespaces every element id
                // beneath it.
                //
                // Clipped, because starting below the bar is not the same as
                // staying below it: the mini player browsing once had,
                // anchored to the bottom, grew up over the bar in a short
                // window, where the bar's drag
                // and caption areas still answered the platform under it. The
                // clip bounds hit testing as well as painting, so nothing in
                // here can be pressed outside it, and it adds no hitbox.
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
                        // The browse page, alone or docked beside the panes,
                        // and the panes. Neither column has an id, nor this
                        // row, so a pane's element ids are its own whichever
                        // of them is drawn (see `watch::page`).
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .flex()
                            .flex_row()
                            .children(browse.map(|(width, page)| {
                                div()
                                    .h_full()
                                    .map(|column| match width {
                                        Some(width) => column
                                            .flex_none()
                                            .w(px(width))
                                            .border_r_1()
                                            .border_color(theme::border()),
                                        None => column.flex_1().min_w_0(),
                                    })
                                    .child(motion::arrive(
                                        // The browse page fades in as it
                                        // comes, alone or beside the panes,
                                        // and not again as a tab, a category
                                        // or a channel changes under it. No
                                        // movement, only a fade: anything that
                                        // slides drags the eye across the
                                        // whole page. The panes beside it do
                                        // not fade: they were there already.
                                        "browse-page",
                                        0.0,
                                        div().size_full().child(page),
                                    ))
                            }))
                            .children(
                                stage.map(|stage| div().flex_1().min_w_0().h_full().child(stage)),
                            ),
                    )
                    .child(self.toast_stack(cx))
                    .children(self.search_suggestions_panel(window, cx))
                    .children(self.channel_menu_view(cx))
                    .children(self.palette_sheet(cx))
                    .children(self.settings_panel.clone()),
            )
            // A divider drag is followed by the window rather than by the
            // handle or the root, and so is the end of a header's drag; see
            // `drag_listeners`. So are the mouse's back and forward buttons,
            // for the same reason.
            .child(self.drag_listeners(cx))
            .child(self.side_buttons(cx))
    }
}
