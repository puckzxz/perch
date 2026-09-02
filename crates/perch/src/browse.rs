//! The browse page: what you follow, what is popular, and what is on.
//!
//! A page rather than a sidebar. Picking what to watch and watching it are
//! different activities, and giving the picker the whole window means
//! thumbnails big enough to actually choose by.
//!
//! All three lists are the same grid of the same card, because they are the
//! same question asked three ways. Only categories look different, and only
//! because box art is a different shape from a thumbnail.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use emotes::ImageCache;
use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, App, Context, ImgResourceLoader, Resource,
    SharedString, Window,
};
use twitch_api::{Category, FollowedChannel, LiveStream};

use crate::controls;
use crate::motion;
use crate::theme;

/// The narrowest a card is allowed to get before the grid drops a column.
///
/// Cards are *derived* from the window rather than fixed, because a fixed width
/// leaves whatever the row could not use as a gutter down one side — at 1600px
/// a 300px card left 306px of nothing, one card short of a fifth column. The
/// grid now takes the width it has and divides it, so the slack goes into the
/// cards instead of beside them.
const CARD_MIN: f32 = 260.0;
/// And the widest, so a card on an ultrawide does not become a poster.
const CARD_MAX: f32 = 380.0;
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
    // How many `min`-wide cards fit, counting the gap each one after the first
    // brings with it.
    let columns = (((usable + gap) / (min + gap)).floor() as usize).max(1);
    let each = (usable - (columns - 1) as f32 * gap) / columns as f32;
    each.clamp(min, max)
}

/// Which of the browse page's lists is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Following,
    Popular,
    Categories,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Following, Tab::Popular, Tab::Categories];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Following => "following",
            Tab::Popular => "popular",
            Tab::Categories => "categories",
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
    /// Streams within [`open`](Self::open).
    pub streams: Listing<LiveStream>,
    /// A request is in flight. One at a time, so one flag is enough.
    pub loading: bool,
    /// A browse request failed. Deliberately separate from `SignIn::Error`:
    /// the session is fine, and blanking the whole page would say otherwise.
    pub error: Option<SharedString>,
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

/// What a search turned up. Both kinds at once, because a name like "zomboid"
/// is as likely to mean the game as a channel.
#[derive(Default)]
pub struct SearchResults {
    pub query: SharedString,
    pub categories: Vec<Category>,
    pub streams: Vec<LiveStream>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.streams.is_empty()
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
    CloseSearch,
    /// Open the settings sheet. Only the not-signed-in state raises this: it is
    /// the one empty state whose instruction is "open settings", and telling
    /// somebody where a button is instead of giving them the button is the sort
    /// of thing a page does when nobody has read it back.
    OpenSettings,
    /// Fetch the next page of whichever list is on screen.
    LoadMore,
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
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// Text that gets one line and an ellipsis if it does not fit.
///
/// Not `.truncate()`, which is what this used to be and which quietly does not
/// work here: gpui only ellipsises when the measure pass has a definite width,
/// and a child of a flex *column* does not get one — so a card title was sliced
/// through the middle of a letter at the card's edge, eating its own padding on
/// the way out. `line_clamp` takes the wrapping path instead, where the width
/// is known, and stops after one line.
fn one_line() -> gpui::Div {
    div().w_full().text_ellipsis().line_clamp(1)
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

/// One live channel.
///
/// Clicking the card watches it alone; the small "+" adds it beside whatever is
/// already playing. Two separate affordances because replacing what you are
/// watching and adding to it are different intentions, and guessing between
/// them from a single click gets it wrong half the time.
fn card<V: 'static>(
    index: usize,
    stream: &LiveStream,
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let on_click = on_action.clone();
    let on_add = on_action;
    let login = stream.user_login.clone();
    let add_login = stream.user_login.clone();
    let thumbnail = cache.get_or_request_fresh(
        &twitch_api::thumbnail(&stream.thumbnail_url, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT),
        THUMBNAIL_MAX_AGE,
    );

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
                    div()
                        .absolute()
                        .bottom(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::GAP_TIGHT))
                        .px(px(theme::GAP_TIGHT))
                        .py(px(3.))
                        .rounded(px(theme::RADIUS))
                        // Carries its own contrast, because it sits on whatever
                        // the stream happens to be showing.
                        .bg(theme::overlay())
                        .text_size(px(theme::TEXT_META))
                        .font_weight(theme::weight_label())
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(rgb(0xffffff))
                        .child(
                            div()
                                .flex_none()
                                .w(px(6.))
                                .h(px(6.))
                                .rounded_full()
                                .bg(theme::live()),
                        )
                        .child(SharedString::from(watching)),
                )
                .when(can_add, |thumb| {
                    thumb.child(
                        controls::pill(("add-stream", index), "+ add", controls::Variant::Pill)
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .right(px(theme::GAP_TIGHT))
                            .opacity(0.0)
                            .group_hover("card", |style| style.opacity(1.0))
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
                    one_line()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .child(
                    one_line()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(stream.title.clone())),
                )
                .child(
                    one_line()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(stream.game_name.clone())),
                ),
        )
}

/// The scrolling body of a list. Separate from the rows inside it, so a search
/// can stack two kinds of result in one scroll rather than two.
fn scroller(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SECTION))
        .p(px(theme::PAGE_PAD))
}

/// A wrapping row of cards.
fn wrap_row(gap: f32) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(gap))
        .content_start()
}

/// One offline follow: a name, and nothing else there is to say.
///
/// A name rather than a card on purpose. A card is mostly a picture, and an
/// offline channel has none — a thumbnail URL that is stale by hours at best,
/// or a profile picture that costs another request per refresh and says nothing
/// about the channel. Names also pack: a hundred follows is five rows here and
/// a wall of identical grey rectangles as cards.
///
/// Clicking one still opens it. The video pane says "offline", but the *chat*
/// connects either way, which is the reason to go there.
fn offline_pill<V: 'static>(
    index: usize,
    channel: &FollowedChannel,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let login = channel.login.clone();
    controls::pill(
        ("offline-follow", index),
        SharedString::from(channel.display_name.clone()),
        controls::Variant::Pill,
    )
    .on_click(cx.listener(move |view, _event, window, cx| {
        on_action(view, Action::Watch(login.clone()), window, cx)
    }))
}

/// The Following tab: who is live, then who is not.
///
/// Both under one scroller rather than two, so there is still only ever one
/// thing to scroll. Each heading disappears with its list, which is what keeps
/// a fully-live follows list looking exactly as it did before offline channels
/// existed.
fn following_view<V: 'static>(
    follows: &[LiveStream],
    offline: &[FollowedChannel],
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let mut page = scroller("browse-grid");

    if !follows.is_empty() {
        page = page
            .when(!offline.is_empty(), |page| page.child(heading("live")))
            .child(stream_row(
                follows,
                width,
                cache,
                can_add,
                on_action.clone(),
                cx,
            ));
    }

    if !offline.is_empty() {
        let mut row = wrap_row(theme::GAP_TIGHT);
        for (index, channel) in offline.iter().enumerate() {
            row = row.child(offline_pill(index, channel, on_action.clone(), cx));
        }
        page = page.child(heading("offline")).child(row);
    }

    page.into_any_element()
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .text_color(theme::text_dim())
        .child(text)
}

fn stream_row<V: 'static>(
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
            card_width,
            cache,
            can_add,
            on_action.clone(),
            cx,
        ));
    }
    row
}

fn category_row<V: 'static>(
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
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    scroller(id)
        .child(stream_row(
            &streams.items,
            width,
            cache,
            can_add,
            on_action.clone(),
            cx,
        ))
        .children(load_more(streams.next.is_some(), loading, on_action, cx))
        .into_any_element()
}

/// The row at the end of a paginated list.
///
/// A button rather than loading as you approach the bottom. Each press is a
/// request plus a hundred thumbnails to fetch and cache, and a list that keeps
/// growing while you scroll spends that on your way past rather than on your
/// say-so. It is also inside the scroller, so reaching it *is* the gesture of
/// having got to the end.
fn load_more<V: 'static>(
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
        controls::waiting("loading…").into_any_element()
    } else {
        controls::pill("load-more", "load more", controls::Variant::Pill)
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
fn search_view<V: 'static>(
    results: &SearchResults,
    discovery: &Discovery,
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let body = if results.is_empty() {
        browse_placeholder(
            discovery,
            format!("Nothing live matches “{}”.", results.query).into(),
        )
    } else {
        let shown = SEARCH_CATEGORY_LIMIT.min(results.categories.len());
        scroller("search-results")
            .when(!results.streams.is_empty(), |list| {
                list.child(heading("channels")).child(stream_row(
                    &results.streams,
                    width,
                    cache,
                    can_add,
                    on_action.clone(),
                    cx,
                ))
            })
            .when(shown > 0, |list| {
                list.child(heading("categories")).child(category_row(
                    &results.categories[..shown],
                    width,
                    cache,
                    on_action.clone(),
                    cx,
                ))
            })
            .into_any_element()
    };

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(context_bar(
            "leave-search",
            "← back",
            results.query.clone(),
            Action::CloseSearch,
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
                one_line()
                    .text_size(px(theme::TEXT_BODY))
                    .font_weight(theme::weight_title())
                    .text_color(theme::text())
                    .child(SharedString::from(category.name.clone())),
            ),
        )
        .on_click(cx.listener(move |view, _event, window, cx| {
            on_action(view, Action::OpenCategory(chosen.clone()), window, cx)
        }))
}

/// A line above a list that has taken over the page, saying where you are and
/// how to leave.
fn context_bar<V: 'static>(
    id: &'static str,
    back: &'static str,
    title: SharedString,
    action: Action,
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
fn notice(title: SharedString, detail: SharedString, error: bool) -> gpui::Div {
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
fn empty_state<V: 'static>(
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
fn browse_placeholder(discovery: &Discovery, empty: SharedString) -> AnyElement {
    if let Some(reason) = &discovery.error {
        return notice("Could not reach Twitch".into(), reason.clone(), true).into_any_element();
    }
    if discovery.loading {
        // Ends as soon as the request does, which is what makes a repeating
        // animation safe here.
        return motion::waiting(
            "browse-loading",
            notice("Loading…".into(), "Asking Twitch what is on.".into(), false),
        )
        .into_any_element();
    }
    notice("Nothing here".into(), empty, false).into_any_element()
}

/// The whole page.
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    follows: &[LiveStream],
    offline: &[FollowedChannel],
    discovery: &Discovery,
    sign_in: &SignIn,
    follows_loaded: bool,
    width: f32,
    cache: &Arc<ImageCache>,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    // Search and categories both take over the page rather than nesting inside
    // a tab, so there is only ever one thing to scroll.
    let body = if let Some(results) = &discovery.search {
        search_view(results, discovery, width, cache, can_add, on_action, cx)
    } else if let Some(category) = &discovery.open {
        let list = if discovery.streams.is_empty() {
            browse_placeholder(
                discovery,
                format!("Nobody is streaming {} right now.", category.name).into(),
            )
        } else {
            stream_grid(
                "category-streams",
                &discovery.streams,
                discovery.loading,
                width,
                cache,
                can_add,
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
                "← categories",
                SharedString::from(category.name.clone()),
                Action::CloseCategory,
                on_action,
                cx,
            ))
            .child(list)
            .into_any_element()
    } else {
        match discovery.tab {
            // Sign-in lives on this tab, so an empty follows list has more to
            // say than "nothing here". Both lists have to be empty: signed in
            // with everybody offline is not the same as not signed in, and
            // testing only the live one would hide the sign-in prompt behind a
            // stale offline list after a client id change.
            Tab::Following if follows.is_empty() && offline.is_empty() => {
                empty_state(sign_in, follows_loaded, on_action, cx)
            }
            Tab::Following => {
                following_view(follows, offline, width, cache, can_add, on_action, cx)
            }
            Tab::Popular if discovery.popular.is_empty() => browse_placeholder(
                discovery,
                "Twitch reported nothing live, which would be a first.".into(),
            ),
            Tab::Popular => stream_grid(
                "popular-grid",
                &discovery.popular,
                discovery.loading,
                width,
                cache,
                can_add,
                on_action,
                cx,
            ),
            Tab::Categories if discovery.categories.is_empty() => {
                browse_placeholder(discovery, "No categories came back.".into())
            }
            Tab::Categories => scroller("categories-grid")
                .child(category_row(
                    &discovery.categories.items,
                    width,
                    cache,
                    on_action.clone(),
                    cx,
                ))
                .children(load_more(
                    discovery.categories.next.is_some(),
                    discovery.loading,
                    on_action,
                    cx,
                ))
                .into_any_element(),
        }
    };

    // `flex_1` + `min_h_0`, not `size_full`. This is a flex child sitting under
    // the browse header, so asking for the full window height overflows the
    // column by exactly the header's height and pushes the bottom of the list
    // off-screen. It went unnoticed while the last thing in the list was page
    // padding; a Load more row at the end made it a button you could see and
    // could not reach.
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

    #[test]
    fn formats_viewer_counts_compactly() {
        assert_eq!(format_viewers(0), "0");
        assert_eq!(format_viewers(999), "999");
        assert_eq!(format_viewers(1500), "1.5k");
        assert_eq!(format_viewers(2_400_000), "2.4M");
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
