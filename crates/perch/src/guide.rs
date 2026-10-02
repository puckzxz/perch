//! The guide: what else is on, raised over the lower part of the watch page
//! while every pane plays on, with its sound.
//!
//! The watch page used to offer only the live follows without leaving it —
//! the rail's `+`, a go-live toast's `+ Add`. Popular, a category, the rail's
//! Recommended group seen as pictures: each was the browse page, and leaving
//! the watch page stops what plays there when the mini player is off. The
//! guide brings those lists to the watch page instead, from a Guide button on
//! every pane's bar (`video_view::bar`), and is gone again with that button,
//! `Esc`, a press anywhere outside it, or once something in it has been
//! watched or added.
//!
//! Nothing here is a list of its own. Four tabs ([`Tab`]): Following is the
//! live follows the follows poll keeps; Recommended is the rail's group
//! (`recommended::Recommended::shown`), each card saying which channel led to
//! it; Popular and Categories are the browse page's own lists, in
//! `browse::Discovery`, asked for by the same keyed requests and waited on by
//! the same keys, with the same Load more — so a list the browse page has is
//! already in the guide, and the other way round. A category picked in the
//! guide opens inside it, with a way back to the categories; its streams are
//! the one list the guide keeps (`Guide::streams`), since the browse page's
//! own category belongs to that page's back and forward, and a reply for a
//! category open in both lands in both. What the guide shows is
//! [`Guide::shows`]; what it asks for to fill that, [`Guide::first_page_due`]
//! and [`Guide::next_page`], each through the request the browse page would
//! send ([`Shows::first_page`]).
//!
//! The cards are the browse page's (`browse::card`), smaller, offering
//! `Watch` and `+ Add` (`browse::CardOffers::Guide`). Watch plays the channel
//! in place of the pane the guide was opened from ([`Guide::opener`], kept
//! however the pointer crosses the other panes on its way to a card, and the
//! active pane only once that one has gone: [`watch_target`]), and Add opens
//! it beside the others; which is which is [`opening`], and whether the guide
//! closes after is [`closes_after`]. A Guide button pressed while the guide
//! is up closes it if it is the opener's, and otherwise moves the guide to
//! that pane ([`press`]). The panel's size is [`panel`]: tall enough for two
//! rows of cards, never more than half the page.
//!
//! The root runs it (`root/guide.rs`): opening and closing, the requests, and
//! the panel's shell — where it sits, the press outside that closes it, and
//! the veil that keeps the panes under it from counting the pointer as theirs
//! (`crate::veil`).

use emotes::ImageCache;
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle, SharedString, Window};
use twitch_api::recommend::SimilarChannel;
use twitch_api::{Category, LiveStream};

use crate::assets::Icon;
use crate::browse::{self, Action, CardOffers, Discovery, Listing, SignIn};
use crate::layout::Body;
use crate::twitch::{ListKey, Request};
use crate::{controls, theme};

/// The guide's tabs, in the order their pills stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Who you follow that is live: the first answer to "what else is on".
    #[default]
    Following,
    /// The rail's Recommended group, as cards.
    Recommended,
    Popular,
    Categories,
}

impl Tab {
    pub const ALL: [Tab; 4] = [
        Tab::Following,
        Tab::Recommended,
        Tab::Popular,
        Tab::Categories,
    ];

    /// The words on its pill.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Following => "Following",
            Tab::Recommended => "Recommended",
            Tab::Popular => "Popular",
            Tab::Categories => "Categories",
        }
    }
}

/// What the guide is showing: one of its tabs' lists, or the streams of the
/// category picked under Categories.
#[derive(Debug, PartialEq)]
pub enum Shows<'a> {
    Following,
    Recommended,
    Popular,
    Categories,
    Category(&'a Category),
}

impl Shows<'_> {
    /// The request for the first page of what is shown, the one the browse
    /// page sends for the same list (`browse::Place::first_page`), so the
    /// two wait on and fill one list. `None` for Following and Recommended,
    /// which no browse request fills: the follows poll's and the rail's.
    pub fn first_page(&self) -> Option<Request> {
        match self {
            Shows::Following | Shows::Recommended => None,
            Shows::Popular => Some(Request::Popular { after: None }),
            Shows::Categories => Some(Request::Categories { after: None }),
            Shows::Category(category) => Some(Request::Category {
                category: (*category).clone(),
                after: None,
            }),
        }
    }

    /// The list it is, as its request names it: what its loading and its
    /// failure are read by (`Discovery::is_pending`, `Discovery::error_for`).
    pub fn key(&self) -> Option<ListKey> {
        self.first_page().and_then(|request| request.list_key())
    }

    /// Whether it is a list of follows or of the rail's recommendations,
    /// which hold still while the pointer is on them, as the rail and Home
    /// do (`RootView::hold_live`).
    pub fn holds(&self) -> bool {
        matches!(self, Shows::Following | Shows::Recommended)
    }
}

/// One scroll position per list the guide shows, kept for the session, so
/// switching tabs does not carry one list's offset onto the next; see
/// `browse::Scrolls`.
#[derive(Default, Clone)]
pub struct Scrolls {
    pub following: ScrollHandle,
    pub recommended: ScrollHandle,
    pub popular: ScrollHandle,
    pub categories: ScrollHandle,
    pub category: ScrollHandle,
}

/// The guide's state, the root's for the session. Its tab and the category
/// open in it outlive a close, so the guide opens again where it was left:
/// adding a second stream from a category is the same two presses as the
/// first.
#[derive(Default)]
pub struct Guide {
    /// Whether it is up over the watch page.
    pub open: bool,
    /// The pane whose Guide button raised it, by key: the one a Watch in it
    /// replaces. Kept apart from the active pane, which the pointer moves
    /// as it crosses whatever of the panes the guide leaves showing on its
    /// way down to a card; see [`watch_target`].
    pub opener: Option<String>,
    pub tab: Tab,
    /// The category picked under Categories, whose streams that tab shows
    /// until the way back is pressed.
    pub category: Option<Category>,
    /// That category's streams, a page at a time; see the module for why
    /// the guide keeps its own.
    pub streams: Listing<LiveStream>,
    pub scrolls: Scrolls,
}

impl Guide {
    /// What the guide shows: its tab's list, or under Categories with a
    /// category picked, that category's streams. A category stays picked
    /// while another tab is up, and comes back with Categories.
    pub fn shows(&self) -> Shows<'_> {
        match (self.tab, &self.category) {
            (Tab::Following, _) => Shows::Following,
            (Tab::Recommended, _) => Shows::Recommended,
            (Tab::Popular, _) => Shows::Popular,
            (Tab::Categories, None) => Shows::Categories,
            (Tab::Categories, Some(category)) => Shows::Category(category),
        }
    }

    /// Whether it is up from the pane `key` names: what that pane's Guide
    /// button says, since a press there closes it only then ([`press`]).
    pub fn up_from(&self, key: &str) -> bool {
        self.open && self.opener.as_deref() == Some(key)
    }

    /// One of its tab pills pressed. Categories pressed while it is already
    /// the tab goes back to the list of categories from a category open in
    /// it, as a tab pressed on the browse page goes back to that tab's list;
    /// from another tab it comes back as it was left, category and all.
    pub fn show_tab(&mut self, tab: Tab) {
        if tab == Tab::Categories && self.tab == Tab::Categories {
            self.leave_category();
        }
        self.tab = tab;
    }

    /// Back from a category to the list of categories: `← Categories`, or
    /// the Categories pill pressed again.
    pub fn leave_category(&mut self) {
        self.category = None;
        self.streams.clear();
    }

    /// Whether the rail's recommendations are on screen here, which lets
    /// them be asked for with the rail folded away; see
    /// `RootView::ask_recommended`.
    pub fn shows_recommended(&self) -> bool {
        self.open && self.tab == Tab::Recommended
    }

    /// The list shown, if it is one the browse requests fill: the browse
    /// page's Popular or Categories, or the guide's own category streams.
    fn listing<'a>(&'a self, discovery: &'a Discovery) -> Option<ListingRef<'a>> {
        match self.shows() {
            Shows::Following | Shows::Recommended => None,
            Shows::Popular => Some(ListingRef::Streams(&discovery.popular)),
            Shows::Categories => Some(ListingRef::Categories(&discovery.categories)),
            Shows::Category(_) => Some(ListingRef::Streams(&self.streams)),
        }
    }

    /// What to ask for to fill what is shown: its first page, while it is
    /// empty and not already asked for — by this guide or by the browse
    /// page, since the two share every key. Asked once and kept, as the
    /// browse page's lists are (`RootView::fill_shown`).
    pub fn first_page_due(&self, discovery: &Discovery) -> Option<Request> {
        let shows = self.shows();
        let key = shows.key()?;
        let empty = self.listing(discovery)?.is_empty();
        (empty && !discovery.is_pending(&key))
            .then(|| shows.first_page())
            .flatten()
    }

    /// The request for the next page of what is shown, from the list's own
    /// cursor, as the browse page's Load more asks (`RootView::load_more`):
    /// nothing when it has no more, or for a list no request fills.
    pub fn next_page(&self, discovery: &Discovery) -> Option<Request> {
        let after = self.listing(discovery)?.next()?;
        match self.shows() {
            Shows::Following | Shows::Recommended => None,
            Shows::Popular => Some(Request::Popular { after: Some(after) }),
            Shows::Categories => Some(Request::Categories { after: Some(after) }),
            Shows::Category(category) => Some(Request::Category {
                category: category.clone(),
                after: Some(after),
            }),
        }
    }

    /// Take a page of `category`'s streams, if that is the category open
    /// here; a reply for one left since repopulates nothing. Copied, since
    /// the browse page may be taking the same page for the same category.
    pub fn absorb(
        &mut self,
        category: &Category,
        items: &[LiveStream],
        next: Option<String>,
        append: bool,
    ) {
        if self
            .category
            .as_ref()
            .is_some_and(|open| open.id == category.id)
        {
            self.streams.absorb(items.to_vec(), next, append);
        }
    }
}

/// A list a browse request fills, borrowed for what [`Guide`] asks of it.
enum ListingRef<'a> {
    Streams(&'a Listing<LiveStream>),
    Categories(&'a Listing<Category>),
}

impl ListingRef<'_> {
    fn is_empty(&self) -> bool {
        match self {
            ListingRef::Streams(list) => list.is_empty(),
            ListingRef::Categories(list) => list.is_empty(),
        }
    }

    fn next(&self) -> Option<String> {
        match self {
            ListingRef::Streams(list) => list.next.clone(),
            ListingRef::Categories(list) => list.next.clone(),
        }
    }
}

/// What a card in the guide was pressed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// `Watch`, or a press on the card: in place of the pane the guide was
    /// opened from ([`watch_target`]).
    Watch,
    /// `+ Add`: beside the panes there are.
    Add,
}

/// Where a pick plays.
#[derive(Debug, PartialEq, Eq)]
pub enum Opening<'a> {
    /// In place of the pane this key names, which keeps its place in the
    /// grid, as a recording swapped in for a stopped pane does
    /// (`RootView::replace_with_channel`). A channel already open in another
    /// pane is chosen there instead, by the same path.
    InPlace(&'a str),
    /// In a pane of its own beside the others, as `+ Add` anywhere opens one
    /// (`browse::Action::Add`): refused with the "Already watching 4
    /// streams" toast at four, and the pane it is in chosen if it is open.
    Beside,
    /// Alone, for a Watch with no pane to put it in place of — which the
    /// guide, opened from a pane's bar, never meets, but which is the only
    /// sensible answer.
    Alone,
}

/// The pane a Watch replaces: the guide's `opener` while that pane is still
/// open (`is_open`), or else the `active` one — the opener closed, by its ×
/// or a channel ending into a closed pane, while the guide stayed up.
pub fn watch_target<'a>(
    opener: Option<&'a str>,
    active: Option<&'a str>,
    is_open: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    opener.filter(|key| is_open(key)).or(active)
}

/// Where `pick` plays, with `target` the pane a Watch replaces
/// ([`watch_target`]): Watch in its place, Add beside the panes.
pub fn opening(pick: Pick, target: Option<&str>) -> Opening<'_> {
    match (pick, target) {
        (Pick::Watch, Some(key)) => Opening::InPlace(key),
        (Pick::Watch, None) => Opening::Alone,
        (Pick::Add, _) => Opening::Beside,
    }
}

/// What a press on the Guide button of the pane `pressed` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Raise the guide from that pane, or, while it is up from another,
    /// move it there: that pane is now the one a Watch replaces.
    OpenFrom,
    /// Put it away.
    Close,
}

/// What a press on the Guide button of the pane `pressed` does, with the
/// guide `open` or not and raised from `opener`: closes it only from the pane
/// it was raised from. In a grid whose top panes' bars show above the guide,
/// another pane's Guide pressed means "from this pane", not "away".
pub fn press(open: bool, opener: Option<&str>, pressed: &str) -> Press {
    if open && opener == Some(pressed) {
        Press::Close
    } else {
        Press::OpenFrom
    }
}

/// Whether the guide closes once `pick` has been carried out, with `panes`
/// open beforehand and the channel `open_already` in one of them or not:
/// after everything that plays something, and not after an Add refused at
/// four panes, which plays nothing — the toast says why, and the guide stays
/// up for a Watch in place instead.
pub fn closes_after(pick: Pick, panes: usize, open_already: bool) -> bool {
    !(pick == Pick::Add && panes >= crate::watch::MAX_PANES && !open_already)
}

/// The most of the watch page's height the guide takes: half. The panes
/// behind it are still the point, and keep playing where they can be seen.
pub const MOST: f32 = 0.5;

/// The guide's header: its tab pills and its close control, in a row as tall
/// as an icon button with a tight gap above and below.
pub const HEADER: f32 = theme::ICON_BUTTON + 2.0 * theme::GAP_TIGHT;

/// How far in from the page's sides and foot the panel floats.
pub const INSET: f32 = theme::GAP;

/// The guide's stream cards, narrower than the browse page's
/// (`browse::CARD_MIN` to `browse::CARD_MAX`): the guide has half the page at
/// most, and two rows of the browse page's cards would want more than that.
pub const CARD_MIN: f32 = 200.0;
pub const CARD_MAX: f32 = 280.0;

/// How many rows of cards the guide is tall enough for when the page has the
/// room.
const ROWS: f32 = 2.0;

/// How tall a stream card `width` wide is: its 16:9 picture, then its name
/// at gpui's own line height and its two meta lines, padded
/// (`browse::card`).
pub fn card_height(width: f32) -> f32 {
    width * 9.0 / 16.0 + theme::VIDEO_CARD_TEXT + theme::GAP_TIGHT + theme::LINE_TIGHT
}

/// Where the guide sits on the watch page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    /// In from the page's left and right edges and from its foot.
    pub inset: f32,
    /// Between the insets.
    pub width: f32,
    pub height: f32,
}

/// The guide's panel on a page `body` big: across the page inside [`INSET`],
/// at the foot, and as tall as its header and [`ROWS`] rows of cards at the
/// width it has, inside the list's padding — but never taller than [`MOST`]
/// of the page, which in any window shorter than a large one is what decides
/// it. Nothing negative, however small the window.
pub fn panel(body: Body) -> Panel {
    let width = (body.width - 2.0 * INSET).max(0.0);
    let card = browse::card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
    let wanted = HEADER
        + 2.0 * theme::PAGE_PAD
        + ROWS * card_height(card)
        + (ROWS - 1.0) * theme::GAP_SECTION;
    Panel {
        inset: INSET,
        width,
        height: wanted.min(body.height.max(0.0) * MOST),
    }
}

/// A live channel from the rail's recommendations as the cards draw one:
/// what the answer said of it, and no preview, which those answers do not
/// carry — the card waits on its placeholder rather than asking for one.
/// Only for a channel no list of streams has; one that is in a list is
/// drawn from there, picture and all.
pub fn as_stream(channel: &SimilarChannel) -> LiveStream {
    LiveStream {
        id: channel.stream_id.clone(),
        user_login: channel.login.clone(),
        user_id: channel.user_id.clone(),
        display_name: channel.display_name.clone(),
        title: channel.title.clone(),
        game_name: channel.game_name.clone(),
        viewer_count: channel.viewer_count,
        thumbnail_url: String::new(),
        started_at: String::new(),
    }
}

/// Everything the guide lists, as the root holds it.
pub struct Lists<'a> {
    /// The live follows, in the order the follows poll keeps them.
    pub follows: &'a [LiveStream],
    pub follows_loaded: bool,
    pub sign_in: &'a SignIn,
    /// The rail's recommendations, as streams, each with the words that say
    /// which channel led to it.
    pub recommended: &'a [(LiveStream, String)],
    /// Whether Twitch refused the query the recommendations come from, which
    /// is the end of them for the session (`Recommended::refused`).
    pub recommended_refused: bool,
    pub discovery: &'a Discovery,
}

/// The panel's contents, `panel` big: the header with its tab pills and its
/// close control, then whatever [`Guide::shows`], scrolling. `on_tab` shows
/// a tab, `on_close` closes the guide, and `on_action` is a card's, a
/// category's, the way back from one or Load more, as the browse page's
/// controls send them.
#[allow(clippy::too_many_arguments)]
pub fn contents<V: 'static>(
    guide: &Guide,
    lists: Lists,
    panel: Panel,
    window_hovered: bool,
    cache: &ImageCache,
    on_tab: impl Fn(&mut V, Tab, &mut Window, &mut Context<V>) + Clone + 'static,
    on_close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(header(guide.tab, window_hovered, on_tab, on_close, cx))
        .child(body(
            guide,
            lists,
            panel,
            window_hovered,
            cache,
            on_action,
            cx,
        ))
}

/// The tab pills, the open one marked as the browse page marks its own, and
/// at the far end the way out: an × that closes the guide, saying so.
fn header<V: 'static>(
    open: Tab,
    window_hovered: bool,
    on_tab: impl Fn(&mut V, Tab, &mut Window, &mut Context<V>) + Clone + 'static,
    on_close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let pills = Tab::ALL.map(|tab| {
        let variant = if tab == open {
            controls::Variant::Selected
        } else {
            controls::Variant::Pill
        };
        let on_tab = on_tab.clone();
        controls::pill(("guide-tab", tab as usize), tab.label(), variant)
            .on_click(cx.listener(move |view, _event, window, cx| on_tab(view, tab, window, cx)))
    });
    div()
        .flex_none()
        .h(px(HEADER))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::PANEL_PAD))
        .border_b_1()
        .border_color(theme::border())
        // The pills give way at a narrow page rather than pushing the way out
        // off the panel: they scroll sideways under the wheel, which gpui
        // turns sideways itself for a box that scrolls only that way, so a
        // pill past the edge is still a turn of the wheel away rather than
        // cut off for good.
        .child(
            div()
                .id("guide-tabs")
                .flex_1()
                .min_w_0()
                .overflow_x_scroll()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_TIGHT))
                .children(pills),
        )
        .child(
            controls::icon_button("guide-close", Icon::Close, controls::Variant::Chrome)
                .when(window_hovered, |close| {
                    close.tooltip(controls::tip("Close the guide"))
                })
                .on_click(cx.listener(move |view, _event, window, cx| on_close(view, window, cx))),
        )
}

/// What [`Guide::shows`], in the panel's room under its header.
fn body<V: 'static>(
    guide: &Guide,
    lists: Lists,
    panel: Panel,
    window_hovered: bool,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let discovery = lists.discovery;
    let shows = guide.shows();
    let key = shows.key();
    let loading = key.as_ref().is_some_and(|key| discovery.is_pending(key));
    let error = key.as_ref().and_then(|key| discovery.error_for(key));
    let width = panel.width;
    let scrolls = &guide.scrolls;
    // The cards' tooltips only while the pointer is in the window, as the
    // bar's are: the guide's cards come within its inset of the window's
    // foot and sides (see `HANDOFF.md`, "A tooltip outlives a pointer that
    // leaves the window").
    let offers = CardOffers::Guide { window_hovered };
    match shows {
        Shows::Following if lists.follows.is_empty() => {
            browse::empty_state(lists.sign_in, lists.follows_loaded, on_action, cx)
        }
        Shows::Following => {
            let cards = lists.follows.iter().map(|stream| (stream, None));
            grid(
                "guide-following",
                &scrolls.following,
                width,
                cards,
                None,
                offers,
                cache,
                on_action,
                cx,
            )
        }
        Shows::Recommended if lists.recommended.is_empty() => {
            match recommended_notice(lists.sign_in, lists.recommended_refused) {
                Some((title, detail)) => browse::notice(title, detail, false).into_any_element(),
                None => browse::empty_state(lists.sign_in, lists.follows_loaded, on_action, cx),
            }
        }
        Shows::Recommended => {
            let cards = lists
                .recommended
                .iter()
                .map(|(stream, reason)| (stream, Some(reason.as_str())));
            grid(
                "guide-recommended",
                &scrolls.recommended,
                width,
                cards,
                None,
                offers,
                cache,
                on_action,
                cx,
            )
        }
        Shows::Popular if discovery.popular.is_empty() => browse::list_placeholder(
            "guide-loading",
            error,
            loading,
            "Twitch reported nothing live, which would be a first.".into(),
            lists.sign_in,
            on_action,
            cx,
        ),
        Shows::Popular => {
            let cards = discovery.popular.items.iter().map(|stream| (stream, None));
            let more = Some((discovery.popular.next.is_some(), loading));
            grid(
                "guide-popular",
                &scrolls.popular,
                width,
                cards,
                more,
                offers,
                cache,
                on_action,
                cx,
            )
        }
        Shows::Categories if discovery.categories.is_empty() => browse::list_placeholder(
            "guide-loading",
            error,
            loading,
            "No categories came back.".into(),
            lists.sign_in,
            on_action,
            cx,
        ),
        Shows::Categories => {
            let list = browse::scroller("guide-categories", &scrolls.categories, 0.0)
                .child(browse::category_row(
                    &discovery.categories.items,
                    width,
                    cache,
                    on_action.clone(),
                    cx,
                ))
                .children(browse::load_more(
                    discovery.categories.next.is_some(),
                    loading,
                    on_action,
                    cx,
                ));
            browse::scrollable(list, &scrolls.categories).into_any_element()
        }
        Shows::Category(category) => {
            let list = if guide.streams.is_empty() {
                browse::list_placeholder(
                    "guide-loading",
                    error,
                    loading,
                    format!("Nobody is streaming {} right now.", category.name).into(),
                    lists.sign_in,
                    on_action.clone(),
                    cx,
                )
            } else {
                let cards = guide.streams.items.iter().map(|stream| (stream, None));
                let more = Some((guide.streams.next.is_some(), loading));
                grid(
                    "guide-category",
                    &scrolls.category,
                    width,
                    cards,
                    more,
                    offers,
                    cache,
                    on_action.clone(),
                    cx,
                )
            };
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(browse::context_bar(
                    "guide-leave-category",
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
    }
}

/// A scrolling grid of the guide's cards, each with its note if it has one,
/// and Load more at its foot where `more` says the list has a next page —
/// with whether that page is being asked for now.
#[allow(clippy::too_many_arguments)]
fn grid<'a, V: 'static>(
    id: &'static str,
    scroll: &ScrollHandle,
    width: f32,
    cards: impl Iterator<Item = (&'a LiveStream, Option<&'a str>)>,
    more: Option<(bool, bool)>,
    offers: CardOffers,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let card_width = browse::card_width(width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
    let mut row = browse::wrap_row(theme::GAP_SECTION);
    for (index, (stream, note)) in cards.enumerate() {
        row = row.child(browse::card(
            index,
            stream,
            note,
            card_width,
            cache,
            offers,
            on_action.clone(),
            cx,
        ));
    }
    let (next, loading) = more.unwrap_or((false, false));
    let list = browse::scroller(id, scroll, 0.0)
        .child(row)
        .children(browse::load_more(next, loading, on_action, cx));
    browse::scrollable(list, scroll).into_any_element()
}

/// What Recommended says with nothing to show, as a heading and the words
/// under it: that Twitch refused the query, which is the end of them until
/// Perch is next started (the rail drops the group then too); that there is
/// nothing yet, and where they come from; or that sign-in is under way.
/// `None` where sign-in is waiting on you — a code to type, a client id, a
/// failure, a press of Sign in — which the follows' own empty state says better, with the code
/// or the way to settings (`browse::empty_state`).
fn recommended_notice(sign_in: &SignIn, refused: bool) -> Option<(SharedString, SharedString)> {
    if refused {
        return Some((
            "No recommendations this session".into(),
            "Twitch would not give them this time. They are asked for again the next time              Perch starts."
                .into(),
        ));
    }
    match sign_in {
        SignIn::SignedIn(_) => Some((
            "Nothing to recommend yet".into(),
            "Recommendations come from the channels you watch, a moment after one opens.".into(),
        )),
        SignIn::Connecting => Some((
            "Connecting…".into(),
            "Signing in to Twitch, which recommendations need.".into(),
        )),
        SignIn::NeedsClientId
        | SignIn::SignedOut
        | SignIn::AwaitingCode { .. }
        | SignIn::Error(_) => None,
    }
}

/// How far up the guide rises as it comes in, from where it settles: the
/// menus' rise (`theme::MENU_RISE`), since it comes up from the bar as they
/// do.
pub const RISE: f32 = theme::MENU_RISE;

/// Whether the guide's panel has room to be drawn on a page `body` big: a
/// header's worth of height at least, or it would be a strip of border with
/// nothing to press.
pub fn fits(body: Body) -> bool {
    panel(body).height >= HEADER && panel(body).width > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::{Place, Tab as BrowseTab};

    fn category(id: &str) -> Category {
        Category {
            id: id.into(),
            name: format!("Game {id}"),
            box_art_url: String::new(),
        }
    }

    fn body(width: f32, height: f32) -> Body {
        Body::of(gpui::size(gpui::px(width), gpui::px(height)), 0.0, 0.0)
    }

    fn stream(login: &str) -> LiveStream {
        LiveStream {
            id: String::new(),
            user_login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: 0,
            thumbnail_url: String::new(),
            started_at: String::new(),
        }
    }

    /// Each tab shows its own list, and Categories shows the category picked
    /// under it until the way back — while a picked category leaves the
    /// other tabs showing theirs.
    #[test]
    fn each_tab_shows_its_own_list() {
        let mut guide = Guide::default();
        assert_eq!(
            guide.shows(),
            Shows::Following,
            "the guide opens on the follows"
        );
        for (tab, shows) in [
            (Tab::Following, Shows::Following),
            (Tab::Recommended, Shows::Recommended),
            (Tab::Popular, Shows::Popular),
            (Tab::Categories, Shows::Categories),
        ] {
            guide.tab = tab;
            assert_eq!(guide.shows(), shows);
        }
        let chess = category("743");
        guide.category = Some(chess.clone());
        assert_eq!(guide.shows(), Shows::Category(&chess));
        guide.tab = Tab::Popular;
        assert_eq!(guide.shows(), Shows::Popular);
        guide.tab = Tab::Categories;
        assert_eq!(
            guide.shows(),
            Shows::Category(&chess),
            "it comes back picked"
        );
    }

    /// Popular, Categories and a category are the browse page's lists by
    /// key, so the two wait on, fail on and fill one list; the follows and
    /// the recommendations are nobody's request, and hold under the pointer.
    #[test]
    fn the_lists_it_asks_for_are_the_browse_pages() {
        let chess = category("743");
        let browse_key = |place: Place| place.first_page().and_then(|request| request.list_key());
        assert_eq!(
            Shows::Popular.key(),
            browse_key(Place::Tab(BrowseTab::Popular))
        );
        assert_eq!(
            Shows::Categories.key(),
            browse_key(Place::Tab(BrowseTab::Categories))
        );
        assert_eq!(
            Shows::Category(&chess).key(),
            browse_key(Place::Category(&chess))
        );
        for shows in [Shows::Following, Shows::Recommended] {
            assert_eq!(shows.key(), None);
            assert!(shows.holds());
        }
        assert!(!Shows::Popular.holds());
    }

    /// A list is asked for while it is empty and nobody has asked — the
    /// browse page included — and not again once it has an answer.
    #[test]
    fn an_empty_list_is_asked_for_once() {
        let mut guide = Guide {
            open: true,
            tab: Tab::Popular,
            ..Guide::default()
        };
        let mut discovery = Discovery::default();
        assert!(matches!(
            guide.first_page_due(&discovery),
            Some(Request::Popular { after: None })
        ));
        discovery.start(ListKey::Popular);
        assert!(guide.first_page_due(&discovery).is_none(), "already asked");
        discovery.finish(&ListKey::Popular);
        discovery
            .popular
            .absorb(vec![stream("forsen")], None, false);
        assert!(
            guide.first_page_due(&discovery).is_none(),
            "already answered"
        );

        guide.tab = Tab::Following;
        assert!(
            guide.first_page_due(&discovery).is_none(),
            "the poll's, not a request"
        );

        guide.tab = Tab::Categories;
        guide.category = Some(category("743"));
        assert!(matches!(
            guide.first_page_due(&discovery),
            Some(Request::Category { after: None, .. })
        ));
    }

    /// Load more goes on from the cursor of the list shown, and offers
    /// nothing where the list has no more.
    #[test]
    fn load_more_follows_the_shown_lists_cursor() {
        let mut discovery = Discovery::default();
        discovery
            .categories
            .absorb(vec![category("1")], Some("cat-2".into()), false);
        discovery.popular.absorb(vec![stream("a")], None, false);
        let mut guide = Guide {
            tab: Tab::Categories,
            ..Guide::default()
        };
        assert!(matches!(
            guide.next_page(&discovery),
            Some(Request::Categories { after: Some(after) }) if after == "cat-2"
        ));
        guide.tab = Tab::Popular;
        assert!(guide.next_page(&discovery).is_none(), "the last page");

        guide.tab = Tab::Categories;
        guide.category = Some(category("743"));
        guide
            .streams
            .absorb(vec![stream("b")], Some("s-2".into()), false);
        assert!(matches!(
            guide.next_page(&discovery),
            Some(Request::Category { after: Some(after), category }) if after == "s-2" && category.id == "743"
        ));
    }

    /// A page of a category's streams lands only while that category is
    /// open here.
    #[test]
    fn a_reply_for_a_category_left_lands_nowhere() {
        let mut guide = Guide {
            category: Some(category("743")),
            ..Guide::default()
        };
        guide.absorb(&category("509658"), &[stream("other")], None, false);
        assert!(guide.streams.is_empty());
        guide.absorb(&category("743"), &[stream("chess")], None, false);
        assert_eq!(guide.streams.items.len(), 1);
        guide.absorb(&category("743"), &[stream("more")], None, true);
        assert_eq!(guide.streams.items.len(), 2);
    }

    /// Pressing Categories again goes back to the list of categories from
    /// one open in it; coming to Categories from another tab keeps it open.
    #[test]
    fn categories_pressed_again_goes_back_to_the_list() {
        let chess = category("743");
        let mut guide = Guide {
            tab: Tab::Categories,
            category: Some(chess.clone()),
            ..Guide::default()
        };
        guide
            .streams
            .absorb(vec![stream("chess")], Some("s-2".into()), false);
        guide.show_tab(Tab::Popular);
        guide.show_tab(Tab::Categories);
        assert_eq!(
            guide.shows(),
            Shows::Category(&chess),
            "back as it was left"
        );
        guide.show_tab(Tab::Categories);
        assert_eq!(guide.shows(), Shows::Categories);
        assert!(guide.streams.is_empty() && guide.streams.next.is_none());
    }

    /// A Watch replaces the pane the guide was opened from, however the
    /// pointer has moved the active pane since, and the active pane only
    /// once the opener has gone.
    #[test]
    fn watch_replaces_the_pane_the_guide_was_opened_from() {
        let open = |key: &str| ["top", "bottom"].contains(&key);
        assert_eq!(
            watch_target(Some("top"), Some("bottom"), open),
            Some("top"),
            "the pointer crossed the bottom pane on its way to a card"
        );
        assert_eq!(
            watch_target(Some("closed"), Some("bottom"), open),
            Some("bottom")
        );
        assert_eq!(watch_target(None, Some("bottom"), open), Some("bottom"));
        assert_eq!(watch_target(None, None, open), None);
    }

    /// The Guide button closes the guide only from the pane it was raised
    /// from; another pane's moves it there, and a closed guide opens.
    #[test]
    fn another_panes_guide_button_moves_the_guide() {
        assert_eq!(press(false, None, "a"), Press::OpenFrom);
        assert_eq!(press(false, Some("a"), "a"), Press::OpenFrom, "put away");
        assert_eq!(press(true, Some("a"), "a"), Press::Close);
        assert_eq!(press(true, Some("a"), "b"), Press::OpenFrom);

        let mut guide = Guide {
            open: true,
            opener: Some("a".into()),
            ..Guide::default()
        };
        assert!(guide.up_from("a") && !guide.up_from("b"));
        guide.open = false;
        assert!(!guide.up_from("a"), "put away");
    }

    /// Recommended with nothing in it says why: refused for the session,
    /// nothing yet, or signing in; and leaves a sign-in waiting on you to
    /// the follows' empty state, which has the code or the way to settings.
    #[test]
    fn an_empty_recommended_says_why() {
        let signed_in = SignIn::SignedIn("me".into());
        let heading = |sign_in: &SignIn, refused| {
            recommended_notice(sign_in, refused).map(|(title, _)| title.to_string())
        };
        assert_eq!(
            heading(&signed_in, true).as_deref(),
            Some("No recommendations this session")
        );
        assert_eq!(
            heading(&signed_in, false).as_deref(),
            Some("Nothing to recommend yet")
        );
        assert_eq!(
            heading(&SignIn::Connecting, false).as_deref(),
            Some("Connecting…")
        );
        let awaiting = SignIn::AwaitingCode {
            user_code: "ABCD".into(),
            verification_uri: "https://www.twitch.tv/activate".into(),
        };
        for sign_in in [
            awaiting,
            SignIn::NeedsClientId,
            SignIn::SignedOut,
            SignIn::Error("no".into()),
        ] {
            assert_eq!(heading(&sign_in, false), None);
        }
    }

    /// Watch goes in place of the pane it is given, and Add beside the
    /// panes, whichever that is; a Watch with no pane to replace plays alone.
    #[test]
    fn watch_replaces_its_target_and_add_opens_beside() {
        assert_eq!(
            opening(Pick::Watch, Some("forsen")),
            Opening::InPlace("forsen")
        );
        assert_eq!(opening(Pick::Add, Some("forsen")), Opening::Beside);
        assert_eq!(opening(Pick::Watch, None), Opening::Alone);
        assert_eq!(opening(Pick::Add, None), Opening::Beside);
    }

    /// The guide closes after whatever plays something, and stays up after
    /// an Add the fifth pane would have been, which plays nothing.
    #[test]
    fn a_refused_add_leaves_the_guide_up() {
        let full = crate::watch::MAX_PANES;
        assert!(closes_after(Pick::Watch, full, false));
        assert!(closes_after(Pick::Add, full - 1, false));
        assert!(!closes_after(Pick::Add, full, false));
        assert!(
            closes_after(Pick::Add, full, true),
            "a channel already open is chosen, which is something"
        );
    }

    /// Every whole body from a sliver to a 4K window, a step at a time.
    fn bodies() -> impl Iterator<Item = Body> {
        (0..=40).flat_map(|w| (0..=24).map(move |h| body(w as f32 * 100.0, h as f32 * 100.0)))
    }

    /// However big or small the page, the guide takes no more than half its
    /// height, is never negative, and stays inside its width.
    #[test]
    fn the_guide_never_takes_more_than_half_the_page() {
        for body in bodies() {
            let panel = panel(body);
            assert!(
                panel.height <= body.height * MOST + 0.001,
                "{body:?} gave {panel:?}"
            );
            assert!(
                panel.height >= 0.0 && panel.width >= 0.0,
                "{body:?} gave {panel:?}"
            );
            assert!(panel.width + 2.0 * panel.inset <= body.width.max(2.0 * panel.inset));
        }
    }

    /// A short page gives the guide exactly half of itself: two rows of
    /// cards would want more.
    #[test]
    fn a_short_page_gives_it_half() {
        let short = body(1600.0, 300.0);
        assert_eq!(panel(short).height, 150.0);
        let laptop = body(1366.0, 650.0);
        assert_eq!(panel(laptop).height, 325.0);
    }

    /// A tall page gives it only the room for its header and two rows of
    /// cards, well short of half: the panes keep the rest.
    #[test]
    fn a_tall_page_gives_it_two_rows() {
        let tall = body(1600.0, 2000.0);
        let panel = panel(tall);
        assert!(panel.height < 1000.0, "{panel:?}");
        let card = browse::card_width(panel.width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
        assert!(
            panel.height >= HEADER + 2.0 * card_height(card),
            "{panel:?}"
        );
    }

    /// A narrow page's guide spans it inside its insets, with cards of one
    /// column at the guide's narrowest and widest, and still at most half.
    #[test]
    fn a_narrow_page_spans_its_width() {
        let narrow = body(420.0, 900.0);
        let panel = panel(narrow);
        assert_eq!(panel.width, 420.0 - 2.0 * INSET);
        assert!(panel.height <= 450.0);
        let card = browse::card_width(panel.width, CARD_MIN, CARD_MAX, theme::GAP_SECTION);
        assert!((CARD_MIN..=CARD_MAX).contains(&card));
        assert_eq!(
            browse::columns(panel.width, CARD_MIN, theme::GAP_SECTION),
            1
        );
    }

    /// A wide page fits more cards to a row, at the same height rule: half
    /// the page when two rows would want more.
    #[test]
    fn a_wide_page_fits_more_cards_to_a_row() {
        let wide = body(3400.0, 1000.0);
        let panel = panel(wide);
        assert_eq!(panel.height, 500.0);
        assert!(browse::columns(panel.width, CARD_MIN, theme::GAP_SECTION) > 4);
    }

    /// No room for a header is no guide: a window shorter than two headers.
    #[test]
    fn a_page_too_short_for_its_header_draws_no_guide() {
        assert!(fits(body(1600.0, 900.0)));
        assert!(!fits(body(1600.0, HEADER)));
        assert!(!fits(body(0.0, 900.0)));
    }

    /// A recommendation drawn as a card is the channel the answer named,
    /// with no picture to ask for.
    #[test]
    fn a_recommendation_asks_for_no_picture() {
        let channel = SimilarChannel {
            login: "zackrawrr".into(),
            user_id: "552120296".into(),
            display_name: "zackrawrr".into(),
            title: "news".into(),
            game_name: "Just Chatting".into(),
            game_id: "509658".into(),
            viewer_count: 41_000,
            profile_image_url: String::new(),
            stream_id: "1".into(),
        };
        let stream = as_stream(&channel);
        assert_eq!(stream.user_login, "zackrawrr");
        assert_eq!(stream.viewer_count, 41_000);
        assert!(stream.thumbnail_url.is_empty());
    }
}
