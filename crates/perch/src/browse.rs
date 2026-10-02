//! The browse page: what to watch next, what is popular, what is on, and
//! what you have watched.
//!
//! A page rather than a sidebar. Picking what to watch and watching it are
//! different activities, and giving the picker the whole window means
//! thumbnails big enough to actually choose by.
//!
//! The three lists of what is on are the same grid of the same card, because
//! they are the same question asked three ways. Only categories look
//! different, and only because box art is a different shape from a
//! thumbnail. The history is a grid of recordings, drawn by the card a
//! channel's page uses; see `history_page`. Home, the tab the page opens on,
//! is who you follow and what you left half-watched, together; see `home`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use emotes::ImageCache;
use gpui::{
    div, img, prelude::*, px, AnyElement, App, Context, ImgResourceLoader, Resource, ScrollHandle,
    SharedString, Stateful, Window,
};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use settings::history::History;
use twitch_api::{Category, Channel, LiveStream, Video, VideoKind};

use crate::channel_page;
use crate::controls;
use crate::history_page;
use crate::home;
use crate::last_live::LastLive;
use crate::layout;
use crate::motion;
use crate::theme;
use crate::twitch::{ListKey, Request};

/// The narrowest a card is allowed to get before the grid drops a column.
///
/// Cards are *derived* from the window rather than fixed, because a fixed width
/// leaves whatever the row could not use as a gutter down one side — at 1600px
/// a 300px card left 306px of nothing, one card short of a fifth column. The
/// grid now takes the width it has and divides it, so the slack goes into the
/// cards instead of beside them.
pub(crate) const CARD_MIN: f32 = 260.0;
/// And the widest, so a card on an ultrawide does not become a poster.
pub(crate) const CARD_MAX: f32 = 380.0;
const THUMBNAIL_WIDTH: u32 = 440;
const THUMBNAIL_HEIGHT: u32 = 248;

/// How long a stream preview is worth keeping.
///
/// A channel's preview lives at a fixed URL and Twitch replaces the picture
/// behind it every few minutes, so caching by URL alone shows whatever was
/// there the first time you looked. Roughly matches Twitch's own cadence:
/// shorter just refetches identical bytes.
const THUMBNAIL_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// Category cards are narrower, because box art is portrait and a row of tall
/// cards at stream width would be a wall. Fluid for the same reason stream
/// cards are.
const CATEGORY_MIN: f32 = 150.0;
const CATEGORY_MAX: f32 = 200.0;
/// Twitch box art is 3:4.
const BOX_ART_WIDTH: u32 = 285;
const BOX_ART_HEIGHT: u32 = 380;

/// How many category matches a search shows.
///
/// Twitch matches category names loosely — "moonmoon" returns twenty-odd games
/// with "moon" in them — and an uncapped list buries the channel you were
/// actually looking for. They are relevance-ordered, so a dozen is a hint
/// rather than a list.
const SEARCH_CATEGORY_LIMIT: usize = 12;

/// How wide each card should be to fill `width` with as many columns as fit.
///
/// Pure, and tested: the shape of a page is the kind of thing that looks right
/// at the one window size you happen to have open and wrong at every other.
/// `gap` is the space *between* cards, so N cards have N-1 of them.
pub fn card_width(width: f32, min: f32, max: f32, gap: f32) -> f32 {
    let usable = (width - 2.0 * theme::PAGE_PAD).max(min);
    let columns = columns(width, min, gap);
    let each = (usable - (columns - 1) as f32 * gap) / columns as f32;
    each.clamp(min, max)
}

/// How many cards of at least `min` fit side by side in `width`, `gap`
/// apart: the columns [`card_width`] divides the row into, and so how many
/// cards one row of a grid holds. Never fewer than one, however narrow.
///
/// Its own function for Home, whose Continue watching shows one row of the
/// history and has to know how long a row is.
pub fn columns(width: f32, min: f32, gap: f32) -> usize {
    let usable = (width - 2.0 * theme::PAGE_PAD).max(min);
    // How many `min`-wide cards fit, counting the gap each one after the first
    // brings with it.
    (((usable + gap) / (min + gap)).floor() as usize).max(1)
}

/// Which of the browse page's lists is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// Who you follow that is live, the recordings you did not finish, then
    /// everyone else you follow: "what should I watch", answered on one
    /// page, and the one the app opens on. It was the Following tab, which
    /// had the follows alone; see `home`.
    #[default]
    Home,
    Popular,
    Categories,
    /// The recordings you have watched, each where you left it. The one tab
    /// that asks Twitch nothing: it is the app's own memory.
    History,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Home, Tab::Popular, Tab::Categories, Tab::History];

    /// The tab's name: the words on its pill, and in the palette's `Go to`
    /// row for it.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Popular => "Popular",
            Tab::Categories => "Categories",
            Tab::History => "History",
        }
    }
}

/// Everything the browse page shows that is not your follows list.
///
/// Held rather than fetched per render: these are network round trips, so they
/// are asked for when a tab is opened and kept until the app closes.
#[derive(Default)]
pub struct Discovery {
    pub tab: Tab,
    pub popular: Listing<LiveStream>,
    pub categories: Listing<Category>,
    /// Set while looking inside one category, which takes over the page.
    pub open: Option<Category>,
    /// Set while showing search results, which also take over the page.
    pub search: Option<SearchResults>,
    /// Set while looking at one channel's past broadcasts, which take over
    /// the page the way a category does. See `channel_page`.
    pub channel: Option<ChannelPage>,
    /// Streams within [`open`](Self::open).
    pub streams: Listing<LiveStream>,
    /// The lists asked for and not yet answered, one entry per request.
    ///
    /// Keyed by list rather than one flag for all of them. With one flag, a
    /// reply for a list you had already left cleared "Loading…" from the one
    /// you were waiting on — and with back and forward, leaving a list before
    /// it answers is routine. A list asked for twice is here twice, and each
    /// answer takes one away; see [`finish`](Self::finish). Emptied when the
    /// worker stops, since nothing it was holding will ever answer.
    pub pending: Vec<ListKey>,
    /// The last browse list that could not be had, the list it was, and
    /// why — said on that list only (see [`shown_error`](Self::shown_error)),
    /// so a failure that lands after you have moved on does not blank the
    /// list you moved to. Deliberately separate from `SignIn::Error`: the
    /// session is fine, and blanking the whole page would say otherwise.
    pub error: Option<(ListKey, Unanswered)>,
}

/// Why a browse list has nothing it asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Unanswered {
    /// It was never asked: nobody is signed in yet, and every list needs a
    /// token. Not a failure — Twitch was never tried — and the list fills by
    /// itself once a sign-in lands (`RootView::fill_shown`).
    SignedOut,
    /// Twitch was asked and the request failed, in these words.
    Failed(SharedString),
}

impl Unanswered {
    /// What the list says instead of its contents: a heading, the words
    /// under it, and whether those are a fault's.
    ///
    /// Two headings, because they are two different news. A search typed
    /// while signed out used to say "Could not reach Twitch" over "Sign in
    /// to Twitch to browse" — a network failure, for a request that was
    /// never sent.
    pub fn notice(&self) -> (SharedString, SharedString, bool) {
        match self {
            Unanswered::SignedOut => (
                "Not signed in".into(),
                "Sign in to Twitch to browse.".into(),
                false,
            ),
            Unanswered::Failed(reason) => ("Could not reach Twitch".into(), reason.clone(), true),
        }
    }
}

/// What the browse page is showing: one of the lists that take it over, or
/// else a tab. Borrowed from the [`Discovery`] it was read from.
pub enum Place<'a> {
    Tab(Tab),
    Category(&'a Category),
    Search(&'a SearchResults),
    Channel(&'a ChannelPage),
}

impl Place<'_> {
    /// The request for the first page of this place's list: what filling it
    /// asks for, and what refreshing it asks for again. `None` for the
    /// Home and History tabs, whose lists no browse request fills — the
    /// follows poll's and the app's own.
    ///
    /// The one place a place turns into a request. Filling, refreshing and
    /// [`Discovery::shown_key`] all read it, so the list the page waits on
    /// is by construction the list its request fills: the key comes from
    /// the request, through `Request::list_key`, rather than from a second
    /// match that had to be kept agreeing with it by hand.
    pub fn first_page(&self) -> Option<Request> {
        match self {
            Place::Tab(Tab::Popular) => Some(Request::Popular { after: None }),
            Place::Tab(Tab::Categories) => Some(Request::Categories { after: None }),
            Place::Tab(Tab::Home | Tab::History) => None,
            Place::Category(category) => Some(Request::Category {
                category: (*category).clone(),
                after: None,
            }),
            Place::Search(results) => Some(Request::Search(results.query.to_string())),
            // The shelf on screen, from the top.
            Place::Channel(page) => Some(Request::Videos {
                login: page.login.clone(),
                user_id: page.user_id.clone(),
                kind: page.kind,
                after: None,
            }),
        }
    }
}

impl Discovery {
    /// Which list is on screen.
    ///
    /// The one copy of the order the takeovers stack in — a channel's page
    /// over a search over a category over the tab — which the page, refresh,
    /// Load more, `Esc` and the back-and-forward trail all have to agree on.
    /// Each used to spell it out for itself, and a list one of them read in a
    /// different order would refresh, or step out of, something other than
    /// what was showing.
    pub fn place(&self) -> Place<'_> {
        if let Some(channel) = &self.channel {
            Place::Channel(channel)
        } else if let Some(results) = &self.search {
            Place::Search(results)
        } else if let Some(category) = &self.open {
            Place::Category(category)
        } else {
            Place::Tab(self.tab)
        }
    }

    /// A request for `key` has gone to the worker.
    pub fn start(&mut self, key: ListKey) {
        self.pending.push(key);
    }

    /// An answer for `key` came back, or its failure did. Takes away one
    /// request for it, not all of them: a refresh sent while the first ask
    /// is still out is a second answer to wait for.
    pub fn finish(&mut self, key: &ListKey) {
        if let Some(at) = self.pending.iter().position(|pending| pending == key) {
            self.pending.remove(at);
        }
    }

    /// Which list the page is showing, as the request that fills it names
    /// it: `None` for the Home and History tabs, which no browse request
    /// fills. Read off [`Place::first_page`], so it cannot name a list other
    /// than the one that request is waited on as.
    pub fn shown_key(&self) -> Option<ListKey> {
        self.place()
            .first_page()
            .and_then(|request| request.list_key())
    }

    /// Whether the list on screen is waiting for an answer.
    pub fn is_loading(&self) -> bool {
        self.shown_key()
            .is_some_and(|shown| self.is_pending(&shown))
    }

    /// Why the list on screen could not be had, if it was the one that failed.
    pub fn shown_error(&self) -> Option<&Unanswered> {
        self.error_for(&self.shown_key()?)
    }

    /// Whether `key`'s list is waiting for an answer, whichever list is on
    /// screen: what the guide asks of the lists it shares with this page
    /// (`crate::guide`), which are often not the one the page shows.
    pub fn is_pending(&self, key: &ListKey) -> bool {
        self.pending.contains(key)
    }

    /// Why `key`'s list could not be had, if it was the last one that failed.
    pub fn error_for(&self, key: &ListKey) -> Option<&Unanswered> {
        let (failed, reason) = self.error.as_ref()?;
        (failed == key).then_some(reason)
    }
}

/// One channel's page: who, and what it has kept — its past broadcasts, its
/// highlights and its uploads, one kind on screen at a time.
pub struct ChannelPage {
    pub login: String,
    pub display_name: String,
    /// Helix's id for the channel, which is what its videos are listed by.
    /// `None` until the worker has looked it up for a channel that arrived
    /// with a name alone.
    pub user_id: Option<String>,
    /// Which kind of video the page is showing.
    pub kind: VideoKind,
    /// One list per kind, in the order of [`channel_page::SHELVES`], each with
    /// its own cursor and each fetched the first time it is shown: a channel
    /// that keeps two months of broadcasts would otherwise bury its
    /// highlights under a hundred of them a page.
    pub shelves: [Listing<Video>; 3],
}

impl ChannelPage {
    /// A page for a channel, on its past broadcasts, with nothing fetched yet.
    pub fn new(login: String, display_name: String, user_id: Option<String>) -> Self {
        Self {
            login,
            display_name,
            user_id,
            kind: VideoKind::Archive,
            shelves: Default::default(),
        }
    }

    fn shelf_index(kind: VideoKind) -> usize {
        channel_page::SHELVES
            .iter()
            .position(|shelf| *shelf == kind)
            .unwrap_or(0)
    }

    /// The list for one kind of video.
    pub fn shelf(&self, kind: VideoKind) -> &Listing<Video> {
        &self.shelves[Self::shelf_index(kind)]
    }

    pub fn shelf_mut(&mut self, kind: VideoKind) -> &mut Listing<Video> {
        &mut self.shelves[Self::shelf_index(kind)]
    }

    /// The list on screen.
    pub fn videos(&self) -> &Listing<Video> {
        self.shelf(self.kind)
    }
}

/// A list that arrives a page at a time.
///
/// Twitch caps a page at 100, and "popular" has no natural end — it is every
/// live channel there is. So the cursor is kept beside the items rather than
/// being walked to exhaustion inside the API layer the way the follows lists
/// are: those finish, this one does not, and how far to go is the user's call.
pub struct Listing<T> {
    pub items: Vec<T>,
    /// Where the next page starts. `None` means there is no more, which is what
    /// takes the Load more row away.
    pub next: Option<String>,
}

// Hand-written rather than derived: `derive(Default)` would demand `T: Default`,
// and an empty list needs no such thing from its element type.
impl<T> Default for Listing<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next: None,
        }
    }
}

impl<T> Listing<T> {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Take a page, either replacing the list or extending it.
    pub fn absorb(&mut self, items: Vec<T>, next: Option<String>, append: bool) {
        if append {
            self.items.extend(items);
        } else {
            self.items = items;
        }
        self.next = next;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.next = None;
    }
}

/// What a search turned up. Channels and categories at once, because a name
/// like "zomboid" is as likely to mean the game as a channel — and channels
/// both live and not, because the one you were looking for may not be on.
#[derive(Default)]
pub struct SearchResults {
    pub query: SharedString,
    pub categories: Vec<Category>,
    pub streams: Vec<LiveStream>,
    /// The channels that answered and are not live, as names.
    pub channels: Vec<Channel>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.streams.is_empty() && self.channels.is_empty()
    }
}

/// Something the user did on the browse page.
///
/// One callback carrying an enum rather than one callback per control: the page
/// is generic over its owner, so every extra closure is another type parameter
/// threaded through every helper.
#[derive(Debug, Clone)]
pub enum Action {
    /// Watch this channel alone.
    Watch(String),
    /// Add it beside whatever is already playing.
    Add(String),
    OpenCategory(Category),
    CloseCategory,
    /// Ask Twitch for this, as though it had been typed into the title bar's
    /// search box: what Home's filter offers when nothing matches.
    Search(String),
    CloseSearch,
    /// Look at a channel's past broadcasts. The id rides along when the list
    /// this came from had it, and is looked up when it did not.
    OpenChannel {
        login: String,
        display_name: String,
        user_id: Option<String>,
    },
    CloseChannel,
    /// Show another kind of the open channel's videos: its past broadcasts,
    /// its highlights or its uploads.
    ShowShelf(VideoKind),
    /// Play this recording alone, or beside whatever is already playing.
    /// Boxed because a video is a dozen strings and every other action is a
    /// name.
    WatchVideo(Box<Video>),
    AddVideo(Box<Video>),
    /// Take one recording off the history, and where it was left with it.
    ForgetVideo(String),
    /// Take every recording off the history.
    ClearHistory,
    /// Show one of the tabs, as its pill does: Home's Continue watching
    /// raises it for the whole history when it has more than one row.
    ShowTab(Tab),
    /// Open the settings sheet. Only the not-signed-in state raises this: it is
    /// the one empty state whose instruction is "Open settings", and telling
    /// somebody where a button is instead of giving them the button is the sort
    /// of thing a page does when nobody has read it back.
    OpenSettings,
    /// Fetch the next page of whichever list is on screen.
    LoadMore,
    /// Pin this channel to the top of the rail, or take it off. From the
    /// rail's rows only, for now, so only channels you follow.
    SetPinned {
        login: String,
        pinned: bool,
    },
}

impl Action {
    /// Open an offline channel's page, which is what a click on its name does
    /// wherever the name is: Home, search results and the rail.
    /// The id rides along when the list had one.
    pub fn open_channel(channel: &Channel) -> Self {
        Action::OpenChannel {
            login: channel.login.clone(),
            display_name: channel.display_name.clone(),
            user_id: Some(channel.user_id.clone()).filter(|id| !id.is_empty()),
        }
    }
}

/// How far sign-in has got.
#[derive(Clone)]
pub enum SignIn {
    Connecting,
    NeedsClientId,
    AwaitingCode {
        user_code: SharedString,
        verification_uri: SharedString,
    },
    SignedIn(SharedString),
    Error(SharedString),
}

impl SignIn {
    pub fn summary(&self) -> SharedString {
        match self {
            SignIn::SignedIn(login) => format!("Signed in as {login}").into(),
            SignIn::Connecting => "Connecting…".into(),
            SignIn::NeedsClientId => "Not signed in".into(),
            SignIn::AwaitingCode { user_code, .. } => {
                format!("Enter {user_code} at twitch.tv/activate").into()
            }
            SignIn::Error(reason) => reason.clone(),
        }
    }
}

/// "3h 12m" since the stream started, or `None` if the timestamp is unusable.
///
/// Twitch sends RFC 3339; anything else is a shape change on their side and
/// should degrade to showing nothing rather than a wrong number.
pub fn uptime(started_at: &str) -> Option<String> {
    let started = DateTime::parse_from_rfc3339(started_at).ok()?;
    let elapsed = Utc::now().signed_duration_since(started.with_timezone(&Utc));
    if elapsed.num_seconds() < 0 {
        return None;
    }
    let hours = elapsed.num_hours();
    let minutes = elapsed.num_minutes() % 60;
    Some(if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    })
}

pub fn format_viewers(count: u64) -> String {
    // Thresholds sit where the *rounded* value changes unit, not at the round
    // number: 999,950 rounds to 1000.0 in thousands, and printing "1000.0k" is
    // the kind of thing a viewer notices during exactly the events that draw
    // that many people.
    let millions = count as f64 / 1_000_000.0;
    let thousands = count as f64 / 1_000.0;
    // 999.95 thousand is what `{:.1}` rounds up to "1000.0", so that is where
    // millions begin. Thousands begin at exactly a thousand: the unit below
    // is a whole number, which does not round.
    if thousands >= 999.95 {
        format!("{millions:.1}M")
    } else if count >= 1_000 {
        format!("{thousands:.1}k")
    } else {
        count.to_string()
    }
}

/// Text that gets one line and an ellipsis if it does not fit, and gives up
/// the rest of itself when the pointer rests on it.
///
/// Not `.truncate()`, which is what this used to be and which quietly does not
/// work here: gpui only ellipsises when the measure pass has a definite width,
/// and a child of a flex *column* does not get one — so a card title was sliced
/// through the middle of a letter at the card's edge, eating its own padding on
/// the way out. `line_clamp` takes the wrapping path instead, where the width
/// is known, and stops after one line.
///
/// The tooltip is here rather than on the three call sites because the cutting
/// is: every line built this way is a line that may have been cut, and the one
/// that most often is — the title — is the one worth reading in full. It costs
/// an id, which is why this takes one.
pub(crate) fn one_line(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
) -> Stateful<gpui::Div> {
    let text = text.into();
    div()
        .id(id.into())
        .w_full()
        .text_ellipsis()
        .line_clamp(1)
        .tooltip(controls::full_text([text.clone()]))
        .child(text)
}

/// Release the decoded previews that refreshes have replaced.
///
/// A refreshed preview lands at a new filename - see `emotes::ImageCache` for
/// why it has to - and GPUI decodes an image once per *path*: a new path means a
/// new `RenderImage` with a new id, a new entry in `App::loading_assets`, and a
/// new sprite-atlas tile. Nothing takes any of the three away on its own. The
/// grid is not virtualised and painting is not culled, so an idle browse page
/// mints all three for every stream in the list every `THUMBNAIL_MAX_AGE` and
/// keeps every generation it has ever drawn.
///
/// Call this before anything builds a card, and from nowhere else. The cache
/// records a path as retired only once the replacement is in its ready map, so
/// every card built later in the same render pass already asks for the new file.
/// Release one *after* a card has asked for it and that card is left drawing a
/// path whose file is gone and whose decoded copy has just been thrown away:
/// GPUI would re-read the file, fail, and memoise the failure, which is an empty
/// card for the rest of the session rather than for a frame.
///
/// All three calls below are deliberate. `get_asset` is the only public way to
/// reach a decoded image and it re-inserts the entry it read, so `remove_asset`
/// is not optional and has to run even when nothing was decoded. And all three
/// are free of the phase assertions their neighbours carry - `paint_image` opens
/// with `debug_assert_paint` - which is what makes them legal from `render`.
pub fn release_retired_previews(cache: &ImageCache, window: &mut Window, cx: &mut App) {
    for path in cache.take_retired() {
        let resource = Resource::Path(path.into());
        if let Some(Ok(image)) = window.get_asset::<ImgResourceLoader>(&resource, cx) {
            let _ = window.drop_image(image);
        }
        cx.remove_asset::<ImgResourceLoader>(&resource);
    }
}

/// Where a live channel's preview is fetched from, at the size the cards
/// draw it.
fn preview_url(stream: &LiveStream) -> String {
    twitch_api::thumbnail(&stream.thumbnail_url, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
}

/// A live channel's preview, from the cache, or `None` while it is fetched.
///
/// The card's picture, and a starting pane's poster (`watch::status`): the
/// same URL and the same entry, so a pane opened from a card shows at once
/// the picture the card was showing, with nothing more to fetch. Refreshed on
/// Twitch's cadence ([`THUMBNAIL_MAX_AGE`]); a refresh retires the old file,
/// which is why [`release_retired_previews`] runs before anything calls this.
///
/// Nothing for a stream with no template to fill — one the guide drew from
/// the rail's recommendations, whose answers carry no preview
/// (`guide::as_stream`) — rather than a request for an empty address.
pub(crate) fn stream_preview(cache: &ImageCache, stream: &LiveStream) -> Option<PathBuf> {
    if stream.thumbnail_url.is_empty() {
        return None;
    }
    cache.get_or_request_fresh(&preview_url(stream), THUMBNAIL_MAX_AGE)
}

/// What a stream card offers besides a press on it, which watches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardOffers {
    /// The browse page's: its past broadcasts at the top-left, and at the
    /// top-right `+ Add` while there is something to add it beside
    /// (`RootView::can_add`).
    Page { can_add: bool },
    /// The guide's, over the watch page (`crate::guide`): `Watch` at the
    /// top-left, which a press anywhere on the card does too, and `+ Add` at
    /// the top-right, always — at four panes it says so in a toast, as the
    /// rail's `+` does. No past broadcasts: they are a page, and the guide is
    /// for staying on this one. Both pills' tooltips only while the window
    /// is `window_hovered`, since the guide reaches within a few pixels of
    /// the window's edges (see `HANDOFF.md`, "A tooltip outlives a pointer
    /// that leaves the window").
    Guide { window_hovered: bool },
}

/// The line under a card's title: the game, and after it a note where the
/// list has one — the guide's recommendations say which channel led to each
/// ("Like forsen"), as the rail's rows do.
fn card_meta(game: &str, note: Option<&str>) -> String {
    match note.filter(|note| !note.is_empty()) {
        Some(note) if game.is_empty() => note.to_string(),
        Some(note) => format!("{game} · {note}"),
        None => game.to_string(),
    }
}

/// One live channel.
///
/// Clicking the card watches it alone; the small "+" adds it beside whatever is
/// already playing. Two separate affordances because replacing what you are
/// watching and adding to it are different intentions, and guessing between
/// them from a single click gets it wrong half the time. What else it offers
/// is `offers`'s, and `note` goes after the game ([`card_meta`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn card<V: 'static>(
    index: usize,
    stream: &LiveStream,
    note: Option<&str>,
    width: f32,
    cache: &ImageCache,
    offers: CardOffers,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let on_click = on_action.clone();
    let on_add = on_action.clone();
    let on_corner = on_action;
    let login = stream.user_login.clone();
    let add_login = stream.user_login.clone();
    // The top-left corner's pill: the channel's past broadcasts on the
    // browse page, and in the guide the card's own press, said in words.
    // Tooltips everywhere on the browse page, which has the window's own
    // margin around it; in the guide, only while the pointer is in the
    // window.
    let tips = match offers {
        CardOffers::Page { .. } => true,
        CardOffers::Guide { window_hovered } => window_hovered,
    };
    let (corner_id, corner_words, corner_tip, corner) = match offers {
        CardOffers::Page { .. } => (
            "card-videos",
            "Past broadcasts",
            None,
            Action::OpenChannel {
                login: stream.user_login.clone(),
                display_name: stream.display_name.clone(),
                user_id: Some(stream.user_id.clone()).filter(|id| !id.is_empty()),
            },
        ),
        CardOffers::Guide { .. } => (
            "card-watch",
            "Watch",
            Some("Play in place of the pane you opened the guide from"),
            Action::Watch(stream.user_login.clone()),
        ),
    };
    let can_add = match offers {
        CardOffers::Page { can_add } => can_add,
        CardOffers::Guide { .. } => true,
    };
    let thumbnail = stream_preview(cache, stream);

    let preview_height = px(width * 9.0 / 16.0);
    let preview = match thumbnail {
        Some(path) => img(path).w_full().h(preview_height).into_any_element(),
        // Sized placeholder, so the grid does not reflow as images arrive.
        None => div()
            .w_full()
            .h(preview_height)
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    // What is true only right now, over the picture — which is where broadcast
    // UIs have put a viewer count for decades, and where it is not competing
    // with the name and the title for the eye.
    //
    // This replaces a `LIVE` badge. Every list on this page is live-only —
    // offline follows are names under their own heading — so that badge said
    // the same thing on every card in every list, in the app's only saturated
    // red, while the number that actually varies sat in grey underneath. The
    // dot keeps the signal; the count carries the information.
    let watching = [
        Some(format_viewers(stream.viewer_count)),
        uptime(&stream.started_at),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");

    div()
        .id(("stream-card", index))
        .w(px(width))
        .flex()
        .flex_col()
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_raised()))
        .active(|style| style.bg(theme::pressed()))
        .on_click(cx.listener(move |view, _event, window, cx| {
            on_click(view, Action::Watch(login.clone()), window, cx)
        }))
        .child(
            div()
                .relative()
                .group("card")
                .child(preview)
                .child(
                    controls::badge()
                        .absolute()
                        .bottom(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .child(controls::live_dot())
                        .child(SharedString::from(watching)),
                )
                // The other thing a channel has besides the stream on its
                // card: what it broadcast before. Revealed the way `+ Add`
                // is, on the opposite corner, so a card at rest is still a
                // picture. Both are hidden rather than transparent: at zero
                // opacity a control still takes a click, and a tap with no
                // hover before it — a touchscreen's — landed on one nobody
                // could see.
                .child(
                    controls::pill((corner_id, index), corner_words, controls::Variant::Pill)
                        .absolute()
                        .top(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .invisible()
                        .group_hover("card", |style| style.visible())
                        .when_some(corner_tip.filter(|_| tips), |pill, tip| {
                            pill.tooltip(controls::tip(tip))
                        })
                        .on_click(cx.listener(move |view, _event, window, cx| {
                            cx.stop_propagation();
                            on_corner(view, corner.clone(), window, cx)
                        })),
                )
                .when(can_add, |thumb| {
                    thumb.child(
                        controls::pill(("add-stream", index), "+ Add", controls::Variant::Pill)
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .right(px(theme::GAP_TIGHT))
                            .invisible()
                            .group_hover("card", |style| style.visible())
                            .when(tips, |pill| {
                                pill.tooltip(controls::tip("Open beside what is playing"))
                            })
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                // Without this the card underneath also fires
                                // and replaces every open pane.
                                cx.stop_propagation();
                                on_add(view, Action::Add(add_login.clone()), window, cx)
                            })),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_TIGHT))
                .p(px(theme::PANEL_PAD))
                .child(
                    one_line(("card-name", index), stream.display_name.clone())
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text()),
                )
                .child(
                    one_line(("card-title", index), stream.title.clone())
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_muted()),
                )
                .child(
                    one_line(("card-game", index), card_meta(&stream.game_name, note))
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim()),
                ),
        )
}

/// The scroll positions of the browse page's lists, one per list.
///
/// Held by the owner rather than made per render, because a scroll handle is
/// state: the offset lives in it, and a scrollbar has to read the same one the
/// list writes. One per list rather than one shared, or switching tabs would
/// carry the old list's offset onto the new one.
#[derive(Default, Clone)]
pub struct Scrolls {
    pub home: ScrollHandle,
    pub popular: ScrollHandle,
    pub categories: ScrollHandle,
    pub category: ScrollHandle,
    pub search: ScrollHandle,
    pub channel: ScrollHandle,
    pub history: ScrollHandle,
}

/// The scrolling body of a list. Separate from the rows inside it, so a search
/// can stack two kinds of result in one scroll rather than two.
///
/// Returns the scroller and the scrollbar for it as two elements, because the
/// scrollbar has to sit *over* the list in a `relative` parent rather than
/// inside it, and only the caller knows what else goes in that parent.
///
/// `bottom` is room left under the last row on top of the page's padding —
/// a [`layout::Room`]'s, which is the mini player's while it is up — so the
/// end of every list can be scrolled out from under it.
pub(crate) fn scroller(
    id: &'static str,
    scroll: &ScrollHandle,
    bottom: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SECTION))
        .p(px(theme::PAGE_PAD))
        .pb(px(theme::PAGE_PAD + bottom))
}

/// A list and its scrollbar, stacked.
///
/// There used to be no scrollbar on any of these: the wheel worked and nothing
/// said so, and on a page whose bottom half is a hundred offline follows the
/// only sign of more was content cut at the edge.
pub(crate) fn scrollable(list: impl IntoElement, scroll: &ScrollHandle) -> gpui::Div {
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .flex_col()
        .child(list)
        .child(
            div()
                .absolute()
                .inset_0()
                .child(Scrollbar::vertical(scroll).scrollbar_show(ScrollbarShow::Hover)),
        )
}

/// A wrapping row of cards.
pub(crate) fn wrap_row(gap: f32) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(gap))
        .content_start()
}

/// One offline follow: a name, and on Home when it was last live ("Live 3
/// hours ago", `last_live::LastLive::words`) beside it in the dim meta
/// colour, inside the same pill so the whole of it is the click. `None` is
/// the name alone, as search results show it and as Home does until Twitch
/// has said.
///
/// A name rather than a card on purpose. A card is mostly a picture, and an
/// offline channel has none — a thumbnail URL that is stale by hours at best,
/// or a profile picture that costs another request per refresh and says nothing
/// about the channel. Names also pack: a hundred follows is five rows here and
/// a wall of identical grey rectangles as cards.
///
/// Clicking one opens the channel's page: what it broadcast before, which is
/// the thing an offline channel has. Its chat used to be what a click opened
/// — it connects whether or not anyone is streaming — and it is still one
/// click away, from that page's bar.
pub(crate) fn offline_pill<V: 'static>(
    id: impl Into<gpui::ElementId>,
    channel: &Channel,
    last_live: Option<String>,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let action = Action::open_channel(channel);
    controls::pill(
        id,
        SharedString::from(channel.display_name.clone()),
        controls::Variant::Pill,
    )
    .when_some(last_live, |pill, words| {
        // Its own colour rather than the label's, lifting with the name
        // under the pointer by a step of its own (`last_live_color`), so the
        // age stays the quieter of the two either way.
        pill.group(OFFLINE_PILL_GROUP)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_WORD))
            .child(
                div()
                    .text_size(px(theme::TEXT_META))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(last_live_color(false))
                    .group_hover(OFFLINE_PILL_GROUP, |style| {
                        style.text_color(last_live_color(true))
                    })
                    .child(SharedString::from(words)),
            )
    })
    .on_click(
        cx.listener(move |view, _event, window, cx| on_action(view, action.clone(), window, cx)),
    )
}

/// The group an offline pill's age watches for the pointer.
const OFFLINE_PILL_GROUP: &str = "offline-pill";

/// The colour of when an offline name was last live, on its pill: the dim
/// meta colour at rest, and the pill's own resting label colour under the
/// pointer, where the name has lifted to full text and the dim one would
/// read too faintly on the wash.
fn last_live_color(hovered: bool) -> gpui::Hsla {
    if hovered {
        theme::text_muted()
    } else {
        theme::text_dim()
    }
}

/// Home's filter box. As wide as a name, not as wide as the page.
pub(crate) const FILTER_WIDTH: f32 = 260.0;

pub(crate) fn heading(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .text_color(theme::text_dim())
        .child(text.into())
}

pub(crate) fn stream_row<V: 'static>(
    streams: &[LiveStream],
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> gpui::Div {
    let card_width = card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
    let mut row = wrap_row(theme::GAP_SECTION);
    for (index, stream) in streams.iter().enumerate() {
        row = row.child(card(
            index,
            stream,
            None,
            card_width,
            cache,
            CardOffers::Page { can_add },
            on_action.clone(),
            cx,
        ));
    }
    row
}

pub(crate) fn category_row<V: 'static>(
    categories: &[Category],
    width: f32,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> gpui::Div {
    let card_width = card_width(width, CATEGORY_MIN, CATEGORY_MAX, theme::GAP);
    let mut row = wrap_row(theme::GAP);
    for (index, category) in categories.iter().enumerate() {
        row = row.child(category_card(
            index,
            category,
            card_width,
            cache,
            on_action.clone(),
            cx,
        ));
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn stream_grid<V: 'static>(
    id: &'static str,
    streams: &Listing<LiveStream>,
    loading: bool,
    room: layout::Room,
    cache: &ImageCache,
    can_add: bool,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let list = scroller(id, scroll, room.bottom)
        .child(stream_row(
            &streams.items,
            room.width,
            cache,
            can_add,
            on_action.clone(),
            cx,
        ))
        .children(load_more(streams.next.is_some(), loading, on_action, cx));
    scrollable(list, scroll).into_any_element()
}

/// The row at the end of a paginated list.
///
/// A button rather than loading as you approach the bottom. Each press is a
/// request plus a hundred thumbnails to fetch and cache, and a list that keeps
/// growing while you scroll spends that on your way past rather than on your
/// say-so. It is also inside the scroller, so reaching it *is* the gesture of
/// having got to the end.
pub(crate) fn load_more<V: 'static>(
    more: bool,
    loading: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Option<impl IntoElement> {
    if !more {
        return None;
    }

    // While a page is in flight the row says so and stops taking clicks, so a
    // second press cannot queue a second page against the same cursor.
    let row = if loading {
        controls::waiting("Loading…").into_any_element()
    } else {
        controls::pill("load-more", "Load more", controls::Variant::Pill)
            .on_click(cx.listener(move |view, _event, window, cx| {
                on_action(view, Action::LoadMore, window, cx)
            }))
            .into_any_element()
    };

    Some(
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .py(px(theme::PAGE_PAD))
            .child(row),
    )
}

/// Everything a search turned up, channels first.
///
/// A live channel is directly watchable; a category is another click. Searching
/// a streamer's name and having to scroll past twenty games to reach them is
/// the wrong way round.
#[allow(clippy::too_many_arguments)]
fn search_view<V: 'static>(
    results: &SearchResults,
    discovery: &Discovery,
    room: layout::Room,
    cache: &ImageCache,
    can_add: bool,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let body = if results.is_empty() {
        browse_placeholder(
            discovery,
            format!("Nothing matches “{}”.", results.query).into(),
        )
    } else {
        let shown = SEARCH_CATEGORY_LIMIT.min(results.categories.len());
        // Who is on, then who is not, then games: the order of how directly
        // each is the thing searched for. The headings are like Home's,
        // because they divide the same way.
        let mut offline = wrap_row(theme::GAP_TIGHT);
        for (index, channel) in results.channels.iter().enumerate() {
            offline = offline.child(offline_pill(
                ("search-offline", index),
                channel,
                None,
                on_action.clone(),
                cx,
            ));
        }
        let list = scroller("search-results", scroll, room.bottom)
            .when(!results.streams.is_empty(), |list| {
                list.child(heading("Live")).child(stream_row(
                    &results.streams,
                    room.width,
                    cache,
                    can_add,
                    on_action.clone(),
                    cx,
                ))
            })
            .when(!results.channels.is_empty(), |list| {
                list.child(heading("Offline")).child(offline)
            })
            .when(shown > 0, |list| {
                list.child(heading("Categories")).child(category_row(
                    &results.categories[..shown],
                    room.width,
                    cache,
                    on_action.clone(),
                    cx,
                ))
            });
        scrollable(list, scroll).into_any_element()
    };

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(context_bar(
            "leave-search",
            "← Back",
            results.query.clone(),
            Action::CloseSearch,
            None,
            on_action,
            cx,
        ))
        .child(body)
        .into_any_element()
}

/// One category: its box art, and its name underneath.
fn category_card<V: 'static>(
    index: usize,
    category: &Category,
    width: f32,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let chosen = category.clone();
    let art = cache.get_or_request(&twitch_api::thumbnail(
        &category.box_art_url,
        BOX_ART_WIDTH,
        BOX_ART_HEIGHT,
    ));
    let art_height = width * BOX_ART_HEIGHT as f32 / BOX_ART_WIDTH as f32;

    let cover = match art {
        Some(path) => img(path).w_full().h(px(art_height)).into_any_element(),
        // Sized placeholder, so the grid does not reflow as images arrive.
        None => div()
            .w_full()
            .h(px(art_height))
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    div()
        .id(("category-card", index))
        .w(px(width))
        .flex()
        .flex_col()
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_raised()))
        .active(|style| style.bg(theme::pressed()))
        .child(cover)
        .child(
            div().p(px(theme::PANEL_PAD)).child(
                one_line(("category-name", index), category.name.clone())
                    .text_size(px(theme::TEXT_BODY))
                    .font_weight(theme::weight_title())
                    .text_color(theme::text()),
            ),
        )
        .on_click(cx.listener(move |view, _event, window, cx| {
            on_action(view, Action::OpenCategory(chosen.clone()), window, cx)
        }))
}

/// A line above a list that has taken over the page, saying where you are and
/// how to leave. `trailing` is one more control on the right, for a page that
/// has one other thing to offer.
pub(crate) fn context_bar<V: 'static>(
    id: &'static str,
    back: &'static str,
    title: SharedString,
    action: Action,
    trailing: Option<AnyElement>,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP))
        .px(px(theme::PAGE_PAD))
        .py(px(theme::GAP_TIGHT))
        .child(controls::pill(id, back, controls::Variant::Pill).on_click(
            cx.listener(move |view, _event, window, cx| {
                on_action(view, action.clone(), window, cx)
            }),
        ))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(theme::TEXT_TITLE))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .child(title),
        )
        .child(div().flex_1())
        .children(trailing)
}

/// Waiting for the user to authorise the app.
///
/// The only empty state with something to *do*, so it is the only one with a
/// control. Twitch puts the code in the query string of `verification_uri`, so
/// opening it fills the code in; typing it by hand is the fallback, not the
/// instruction.
fn awaiting_code<V: 'static>(
    user_code: &SharedString,
    verification_uri: &SharedString,
    cx: &mut Context<V>,
) -> AnyElement {
    let uri = verification_uri.to_string();

    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::GAP))
        .child(
            div()
                .text_size(px(theme::TEXT_TITLE))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .child(user_code.clone()),
        )
        .child(
            // The one accent on the page, because it is the one thing to do.
            controls::pill(
                "open-activate",
                "Open twitch.tv/activate",
                controls::Variant::Primary,
            )
            .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&uri))),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_BODY))
                .text_center()
                .text_color(theme::text_dim())
                .child("Opens in your browser with the code already filled in."),
        )
        .into_any_element()
}

/// A centred title and explanation, for a list with nothing in it.
pub(crate) fn notice(title: SharedString, detail: SharedString, error: bool) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::GAP))
        .child(
            div()
                .text_size(px(theme::TEXT_TITLE))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .child(title),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_size(px(theme::TEXT_BODY))
                .line_height(px(theme::LINE_BODY))
                .text_center()
                .text_color(if error {
                    theme::danger()
                } else {
                    theme::text_dim()
                })
                .child(detail),
        )
}

/// A message filling the page when there is nothing to show.
pub(crate) fn empty_state<V: 'static>(
    sign_in: &SignIn,
    follows_loaded: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    if let SignIn::AwaitingCode {
        user_code,
        verification_uri,
    } = sign_in
    {
        return awaiting_code(user_code, verification_uri, cx);
    }

    let (title, detail): (SharedString, SharedString) = match sign_in {
        SignIn::NeedsClientId => (
            "Not signed in".into(),
            "Open settings and paste a Twitch Client ID to see who you follow.".into(),
        ),
        // Handled above, with a control rather than a sentence.
        SignIn::AwaitingCode { .. } => return div().into_any_element(),
        SignIn::Error(reason) => ("Sign-in problem".into(), reason.clone()),
        SignIn::Connecting => ("Connecting…".into(), "Asking Twitch who is live.".into()),
        // A signed-in session with empty lists is two different states, and
        // only one of them is an answer. The worker reports `SignedIn` before
        // it has polled anything, so until the first list lands this is still a
        // question being asked.
        SignIn::SignedIn(_) if !follows_loaded => {
            ("Loading…".into(), "Asking Twitch who you follow.".into())
        }
        SignIn::SignedIn(_) => (
            "Nobody is live".into(),
            "None of the channels you follow are streaming right now.".into(),
        ),
    };

    let body = notice(title, detail, matches!(sign_in, SignIn::Error(_)));

    // The two states you can do something about get the thing to do, rather
    // than a sentence naming it.
    let body = match sign_in {
        SignIn::NeedsClientId | SignIn::Error(_) => body.child(
            controls::pill(
                "empty-settings",
                "Open settings",
                controls::Variant::Primary,
            )
            .on_click(cx.listener(move |view, _event, window, cx| {
                on_action(view, Action::OpenSettings, window, cx)
            })),
        ),
        _ => body,
    };

    // Only the states that are actually still going breathe. "Nobody is live"
    // and "Not signed in" are answers, not progress, and a pulsing answer both
    // misleads and repaints forever.
    match sign_in {
        SignIn::Connecting => motion::waiting("connecting", body).into_any_element(),
        SignIn::SignedIn(_) if !follows_loaded => {
            motion::waiting("loading-follows", body).into_any_element()
        }
        _ => body.into_any_element(),
    }
}

/// What a browse list shows when it has nothing in it yet.
pub(crate) fn browse_placeholder(discovery: &Discovery, empty: SharedString) -> AnyElement {
    list_placeholder(
        "browse-loading",
        discovery.shown_error(),
        discovery.is_loading(),
        empty,
    )
}

/// What a list with nothing in it shows: why it could not be had, if it
/// failed; that it is being asked for, breathing under the id `waiting`, if
/// it is; and otherwise `empty`, which is the answer. The browse page's
/// lists, and the guide's (`crate::guide`).
pub(crate) fn list_placeholder(
    waiting: &'static str,
    error: Option<&Unanswered>,
    loading: bool,
    empty: SharedString,
) -> AnyElement {
    if let Some(unanswered) = error {
        let (title, detail, error) = unanswered.notice();
        return notice(title, detail, error).into_any_element();
    }
    if loading {
        // Ends as soon as the request does, which is what makes a repeating
        // animation safe here.
        return motion::waiting(
            waiting,
            notice("Loading…".into(), "Asking Twitch what is on.".into(), false),
        )
        .into_any_element();
    }
    notice("Nothing here".into(), empty, false).into_any_element()
}

/// The whole page, in the `room` it has: the width its cards divide, and what
/// every list leaves free at its foot for the mini player.
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    follows: &[LiveStream],
    offline: &[Channel],
    last_live: &LastLive,
    filter: &str,
    filter_box: AnyElement,
    discovery: &Discovery,
    sign_in: &SignIn,
    follows_loaded: bool,
    history: &History,
    room: layout::Room,
    cache: &Arc<ImageCache>,
    can_add: bool,
    scrolls: &Scrolls,
    channel_live: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    // Search, categories and a channel all take over the page rather than
    // nesting inside a tab, so there is only ever one thing to scroll.
    let body = match discovery.place() {
        Place::Channel(channel) => channel_page::view(
            channel,
            discovery,
            history,
            channel_live,
            room,
            cache,
            can_add,
            &scrolls.channel,
            on_action,
            cx,
        ),
        Place::Search(results) => search_view(
            results,
            discovery,
            room,
            cache,
            can_add,
            &scrolls.search,
            on_action,
            cx,
        ),
        Place::Category(category) => {
            let list = if discovery.streams.is_empty() {
                browse_placeholder(
                    discovery,
                    format!("Nobody is streaming {} right now.", category.name).into(),
                )
            } else {
                stream_grid(
                    "category-streams",
                    &discovery.streams,
                    discovery.is_loading(),
                    room,
                    cache,
                    can_add,
                    &scrolls.category,
                    on_action.clone(),
                    cx,
                )
            };

            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(context_bar(
                    "leave-category",
                    "← Categories",
                    SharedString::from(category.name.clone()),
                    Action::CloseCategory,
                    None,
                    on_action,
                    cx,
                ))
                .child(list)
                .into_any_element()
        }
        Place::Tab(Tab::Home) => home::view(
            home::Lists {
                live: follows,
                offline,
                last_live,
                history,
                sign_in,
                follows_loaded,
            },
            filter,
            filter_box,
            room,
            cache,
            can_add,
            &scrolls.home,
            on_action,
            cx,
        ),
        Place::Tab(Tab::Popular) if discovery.popular.is_empty() => browse_placeholder(
            discovery,
            "Twitch reported nothing live, which would be a first.".into(),
        ),
        Place::Tab(Tab::Popular) => stream_grid(
            "popular-grid",
            &discovery.popular,
            discovery.is_loading(),
            room,
            cache,
            can_add,
            &scrolls.popular,
            on_action,
            cx,
        ),
        Place::Tab(Tab::Categories) if discovery.categories.is_empty() => {
            browse_placeholder(discovery, "No categories came back.".into())
        }
        Place::Tab(Tab::Categories) => {
            let list = scroller("categories-grid", &scrolls.categories, room.bottom)
                .child(category_row(
                    &discovery.categories.items,
                    room.width,
                    cache,
                    on_action.clone(),
                    cx,
                ))
                .children(load_more(
                    discovery.categories.next.is_some(),
                    discovery.is_loading(),
                    on_action,
                    cx,
                ));
            scrollable(list, &scrolls.categories).into_any_element()
        }
        Place::Tab(Tab::History) => history_page::view(
            history,
            room,
            cache,
            can_add,
            &scrolls.history,
            on_action,
            cx,
        ),
    };

    // `flex_1` + `min_h_0`, not `size_full`. This is a flex child sitting under
    // the browse page's tab strip, so asking for the full window height
    // overflows the column by exactly the strip's height and pushes the bottom
    // of the list off-screen. It went unnoticed while the last thing in the
    // list was page padding; a Load more row at the end made it a button you
    // could see and could not reach.
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(theme::bg())
        .child(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When an offline name was last live reads on its pill at rest and,
    /// lifted a step, under the pointer's wash.
    #[test]
    fn last_live_reads_on_its_pill() {
        let pill = theme::surface_raised();
        for (state, hovered, under) in [
            ("at rest", false, pill),
            ("hovered", true, pill.blend(theme::hover())),
        ] {
            let ratio = theme::contrast(last_live_color(hovered), under);
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "when a name was last live reads {ratio:.2}:1 {state}"
            );
        }
    }

    fn a_video(id: &str, kind: VideoKind) -> Video {
        Video {
            id: id.into(),
            stream_id: None,
            user_id: "2".into(),
            user_login: "someone".into(),
            user_name: "Someone".into(),
            title: String::new(),
            created_at: String::new(),
            length_secs: 60,
            thumbnail_url: String::new(),
            view_count: 0,
            kind,
            muted_segments: Vec::new(),
        }
    }

    /// Each kind of video keeps a list of its own, and the page shows the one
    /// it is on: a page of highlights arriving must not land among the past
    /// broadcasts, nor take their cursor.
    #[test]
    fn a_channel_page_keeps_a_list_per_kind() {
        let mut page = ChannelPage::new("someone".into(), "Someone".into(), None);
        assert_eq!(page.kind, VideoKind::Archive, "a page opens on broadcasts");

        page.shelf_mut(VideoKind::Archive).absorb(
            vec![a_video("1", VideoKind::Archive)],
            Some("more-broadcasts".into()),
            false,
        );
        page.shelf_mut(VideoKind::Highlight).absorb(
            vec![a_video("2", VideoKind::Highlight)],
            None,
            false,
        );

        assert_eq!(page.videos().items[0].id, "1");
        page.kind = VideoKind::Highlight;
        assert_eq!(page.videos().items[0].id, "2");
        assert!(page.videos().next.is_none(), "took the broadcasts' cursor");
        assert!(page.shelf(VideoKind::Upload).is_empty());
    }

    /// The takeovers stack in one order, and the page, refresh, Load more,
    /// `Esc` and the trail all read it from `place`: each layer hides the ones
    /// under it, and taking it away shows the next.
    #[test]
    fn a_channel_page_outranks_search_category_and_tab() {
        let mut discovery = Discovery {
            tab: Tab::Popular,
            ..Discovery::default()
        };
        assert!(matches!(discovery.place(), Place::Tab(Tab::Popular)));

        discovery.open = Some(Category {
            id: "509658".into(),
            name: "Just Chatting".into(),
            box_art_url: String::new(),
        });
        assert!(matches!(discovery.place(), Place::Category(c) if c.id == "509658"));

        discovery.search = Some(SearchResults {
            query: "zomboid".into(),
            ..Default::default()
        });
        assert!(matches!(discovery.place(), Place::Search(r) if r.query == "zomboid"));

        discovery.channel = Some(ChannelPage::new("someone".into(), "Someone".into(), None));
        assert!(matches!(discovery.place(), Place::Channel(p) if p.login == "someone"));

        discovery.channel = None;
        assert!(
            matches!(discovery.place(), Place::Search(_)),
            "closing the channel shows the search under it"
        );
    }

    fn a_category(id: &str) -> Category {
        Category {
            id: id.into(),
            name: id.into(),
            box_art_url: String::new(),
        }
    }

    /// Popular asked for, then a category opened before it answers: the
    /// popular page's reply is not the category's, and must not end its wait.
    #[test]
    fn a_reply_for_another_list_leaves_this_one_loading() {
        let mut discovery = Discovery {
            tab: Tab::Popular,
            ..Discovery::default()
        };
        discovery.start(ListKey::Popular);
        assert!(discovery.is_loading());

        discovery.open = Some(a_category("509658"));
        discovery.start(ListKey::Category("509658".into()));
        discovery.finish(&ListKey::Popular);
        assert!(
            discovery.is_loading(),
            "the popular reply ended the category's wait"
        );

        discovery.finish(&ListKey::Category("509658".into()));
        assert!(!discovery.is_loading());
    }

    /// A failure is said on the list that failed — not on the one the user
    /// has moved to since, and again when they come back to it.
    #[test]
    fn an_error_shows_only_on_the_list_that_failed() {
        let mut discovery = Discovery {
            tab: Tab::Popular,
            ..Discovery::default()
        };
        discovery.error = Some((
            ListKey::Popular,
            Unanswered::Failed("Twitch is down".into()),
        ));
        assert_eq!(
            discovery.shown_error(),
            Some(&Unanswered::Failed("Twitch is down".into()))
        );

        discovery.tab = Tab::Categories;
        assert!(discovery.shown_error().is_none(), "said on another list");
        discovery.tab = Tab::Popular;
        assert!(
            discovery.shown_error().is_some(),
            "not said on coming back to the list that failed"
        );

        // One shelf of a channel's page failing is not another shelf failing.
        discovery.channel = Some(ChannelPage::new("someone".into(), "Someone".into(), None));
        assert!(
            discovery.shown_error().is_none(),
            "said on a channel's page"
        );
        discovery.error = Some((
            ListKey::Videos {
                login: "someone".into(),
                kind: VideoKind::Highlight,
            },
            Unanswered::Failed("no such thing".into()),
        ));
        assert!(
            discovery.shown_error().is_none(),
            "the highlights failed, not the broadcasts on screen"
        );
        if let Some(page) = discovery.channel.as_mut() {
            page.kind = VideoKind::Highlight;
        }
        assert!(discovery.shown_error().is_some());
    }

    /// A list asked for while nobody is signed in was never sent, and says
    /// so without calling it a failure to reach Twitch, which is reserved
    /// for a request that went and failed.
    #[test]
    fn signed_out_is_not_a_failure_to_reach_twitch() {
        let (title, detail, error) = Unanswered::SignedOut.notice();
        assert_eq!(title.as_ref(), "Not signed in");
        assert_eq!(detail.as_ref(), "Sign in to Twitch to browse.");
        assert!(!error, "a sign-in still to come is not drawn as a fault");

        let (title, detail, error) = Unanswered::Failed("timed out".into()).notice();
        assert_eq!(title.as_ref(), "Could not reach Twitch");
        assert_eq!(detail.as_ref(), "timed out");
        assert!(error);
    }

    /// A starting pane's poster is the picture its channel's card shows:
    /// one URL, so one cache entry, and nothing more to fetch for a pane
    /// opened from a card.
    #[test]
    fn the_pane_poster_is_the_cards_picture() {
        let stream = LiveStream {
            id: "318576165606".into(),
            user_login: "forsen".into(),
            user_id: "22484632".into(),
            display_name: "Forsen".into(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: 0,
            thumbnail_url:
                "https://static-cdn.jtvnw.net/previews-ttv/live_user_forsen-{width}x{height}.jpg"
                    .into(),
            started_at: String::new(),
        };
        assert_eq!(
            preview_url(&stream),
            "https://static-cdn.jtvnw.net/previews-ttv/live_user_forsen-440x248.jpg"
        );
    }

    /// Two asks for one list are two answers to wait for.
    #[test]
    fn finishing_one_request_leaves_its_twin_pending() {
        let mut discovery = Discovery {
            tab: Tab::Categories,
            ..Discovery::default()
        };
        discovery.start(ListKey::Categories);
        discovery.start(ListKey::Categories);

        discovery.finish(&ListKey::Categories);
        assert!(discovery.is_loading(), "one answer ended both waits");

        discovery.finish(&ListKey::Categories);
        assert!(!discovery.is_loading());
        // An answer nobody was waiting for changes nothing.
        discovery.finish(&ListKey::Categories);
        assert!(discovery.pending.is_empty());
    }

    /// Every place the page can show waits on the list its first page fills,
    /// by name — a search by the query it sent, a category by its id even
    /// where the id and the name differ, a channel's page by its login and
    /// the shelf on screen — and the two tabs no browse request fills wait
    /// on nothing.
    #[test]
    fn each_place_waits_on_the_list_its_first_page_fills() {
        let mut discovery = Discovery::default();
        for tab in [Tab::Home, Tab::History] {
            discovery.tab = tab;
            assert!(discovery.place().first_page().is_none(), "{tab:?}");
            assert_eq!(discovery.shown_key(), None, "{tab:?}");
        }
        discovery.tab = Tab::Popular;
        assert_eq!(discovery.shown_key(), Some(ListKey::Popular));
        discovery.tab = Tab::Categories;
        assert_eq!(discovery.shown_key(), Some(ListKey::Categories));

        discovery.open = Some(Category {
            id: "509658".into(),
            name: "Just Chatting".into(),
            box_art_url: String::new(),
        });
        assert_eq!(
            discovery.shown_key(),
            Some(ListKey::Category("509658".into()))
        );

        discovery.search = Some(SearchResults {
            query: "zomboid".into(),
            ..Default::default()
        });
        assert_eq!(
            discovery.shown_key(),
            Some(ListKey::Search("zomboid".into()))
        );

        let mut page = ChannelPage::new("someone".into(), "Someone".into(), Some("2".into()));
        page.kind = VideoKind::Upload;
        discovery.channel = Some(page);
        assert_eq!(
            discovery.shown_key(),
            Some(ListKey::Videos {
                login: "someone".into(),
                kind: VideoKind::Upload,
            })
        );
        assert!(
            matches!(
                discovery.place().first_page(),
                Some(Request::Videos { user_id: Some(id), after: None, .. }) if id == "2"
            ),
            "a channel's first page is asked for by its id, from the top"
        );
    }

    /// The difference between a Load more and a fresh tab is one bool, and
    /// getting it wrong either doubles the list or throws away what you were
    /// looking at.
    #[test]
    fn a_listing_appends_a_page_but_replaces_a_first_one() {
        let mut listing: Listing<u32> = Listing::default();
        assert!(listing.is_empty());
        assert!(listing.next.is_none(), "an empty list offers no Load more");

        listing.absorb(vec![1, 2, 3], Some("page2".into()), false);
        assert_eq!(listing.items, vec![1, 2, 3]);
        assert_eq!(listing.next.as_deref(), Some("page2"));

        listing.absorb(vec![4, 5], Some("page3".into()), true);
        assert_eq!(listing.items, vec![1, 2, 3, 4, 5], "a page did not append");

        // A refresh starts again rather than continuing.
        listing.absorb(vec![9], None, false);
        assert_eq!(listing.items, vec![9], "a fresh page did not replace");
        assert!(
            listing.next.is_none(),
            "the end of the list still offered more"
        );
    }

    /// Running out of pages has to take the row away, not leave a button that
    /// asks Twitch for nothing.
    #[test]
    fn the_last_page_clears_the_cursor() {
        let mut listing: Listing<u32> = Listing::default();
        listing.absorb(vec![1], Some("more".into()), false);
        assert!(listing.next.is_some());

        listing.absorb(vec![2], None, true);
        assert_eq!(listing.items, vec![1, 2]);
        assert!(listing.next.is_none());

        listing.clear();
        assert!(listing.is_empty());
        assert!(listing.next.is_none(), "clear left a cursor behind");
    }

    /// How many cards a row of `width` ends up holding, worked back out of the
    /// width each one got. What the grid is actually judged on.
    fn columns(width: f32) -> usize {
        let each = card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
        let usable = width - 2.0 * theme::PAGE_PAD;
        (((usable + theme::GAP_SECTION) / (each + theme::GAP_SECTION)).round() as usize).max(1)
    }

    /// The row is filled, not merely fitted. A fixed 300px card at 1600px left
    /// 306px of gutter down one side — one card short of another column, and
    /// the whole reason this is derived rather than declared.
    #[test]
    fn cards_take_the_width_they_are_given() {
        for width in [900.0, 1280.0, 1600.0, 2560.0, 3440.0] {
            let each = card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
            let columns = columns(width) as f32;
            let used = columns * each + (columns - 1.0) * theme::GAP_SECTION;
            let slack = (width - 2.0 * theme::PAGE_PAD) - used;
            assert!(
                slack.abs() < 1.0,
                "{width}px left {slack:.1}px of the row unused"
            );
        }
    }

    /// A card never gets so narrow that the thumbnail stops being worth
    /// looking at, nor so wide that four channels fill an ultrawide.
    #[test]
    fn card_width_stays_between_its_bounds() {
        for width in [320.0, 600.0, 1600.0, 5120.0] {
            let each = card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
            assert!(
                (CARD_MIN..=CARD_MAX).contains(&each),
                "{width}px gave a {each}px card"
            );
        }
    }

    /// Widening the window may add columns but must never *remove* one, which
    /// is the kind of thing an off-by-one in the divisor does silently.
    #[test]
    fn columns_never_decrease_as_the_window_grows() {
        let mut last = 0;
        let mut width = 400.0;
        while width < 4000.0 {
            let columns = columns(width);
            assert!(
                columns >= last,
                "{width}px dropped from {last} columns to {columns}"
            );
            last = columns;
            width += 7.0;
        }
    }

    /// The line under a card's title is the game, with a note after it
    /// where the list gives one, and the note alone with no game to say.
    #[test]
    fn a_cards_note_follows_its_game() {
        assert_eq!(card_meta("Chess", None), "Chess");
        assert_eq!(
            card_meta("Chess", Some("Like forsen")),
            "Chess · Like forsen"
        );
        assert_eq!(card_meta("", Some("Like forsen")), "Like forsen");
        assert_eq!(card_meta("Chess", Some("")), "Chess");
    }

    #[test]
    fn formats_viewer_counts_compactly() {
        assert_eq!(format_viewers(0), "0");
        assert_eq!(format_viewers(999), "999");
        assert_eq!(format_viewers(1500), "1.5k");
        assert_eq!(format_viewers(2_400_000), "2.4M");
    }

    /// The unit changes where the rounded number would otherwise read as a
    /// thousand of the smaller one.
    #[test]
    fn viewer_counts_never_print_a_thousand_of_the_smaller_unit() {
        assert_eq!(format_viewers(999_949), "999.9k");
        assert_eq!(format_viewers(999_950), "1.0M");
        assert_eq!(format_viewers(999), "999");
        assert_eq!(format_viewers(1_000), "1.0k");
    }

    #[test]
    fn uptime_reports_hours_and_minutes() {
        let started = Utc::now() - chrono::Duration::minutes(195);
        let text = uptime(&started.to_rfc3339()).unwrap();
        assert_eq!(text, "3h 15m");
    }

    #[test]
    fn uptime_omits_hours_under_one() {
        let started = Utc::now() - chrono::Duration::minutes(7);
        assert_eq!(uptime(&started.to_rfc3339()).unwrap(), "7m");
    }

    /// A shape change on Twitch's side should show nothing, never a wrong
    /// number that looks authoritative.
    #[test]
    fn unparseable_timestamps_yield_nothing() {
        assert!(uptime("").is_none());
        assert!(uptime("last tuesday").is_none());
    }

    #[test]
    fn future_timestamps_yield_nothing() {
        let ahead = Utc::now() + chrono::Duration::hours(1);
        assert!(uptime(&ahead.to_rfc3339()).is_none());
    }
}
