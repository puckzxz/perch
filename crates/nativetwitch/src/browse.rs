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
use gpui::{div, img, prelude::*, px, rgb, AnyElement, Context, SharedString};
use twitch_api::{Category, FollowedChannel, LiveStream};

use crate::motion;
use crate::theme;

/// Card width. Wide enough for a legible 16:9 thumbnail, narrow enough that a
/// 1600px window fits four across.
const CARD_WIDTH: f32 = 300.0;
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
/// cards at stream width would be a wall.
const CATEGORY_WIDTH: f32 = 160.0;
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
    pub popular: Vec<LiveStream>,
    pub categories: Vec<Category>,
    /// Set while looking inside one category, which takes over the page.
    pub open: Option<Category>,
    /// Set while showing search results, which also take over the page.
    pub search: Option<SearchResults>,
    /// Streams within [`open`](Self::open).
    pub streams: Vec<LiveStream>,
    /// A request is in flight. One at a time, so one flag is enough.
    pub loading: bool,
    /// A browse request failed. Deliberately separate from `SignIn::Error`:
    /// the session is fine, and blanking the whole page would say otherwise.
    pub error: Option<SharedString>,
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

/// One live channel.
///
/// Clicking the card watches it alone; the small "+" adds it beside whatever is
/// already playing. Two separate affordances because replacing what you are
/// watching and adding to it are different intentions, and guessing between
/// them from a single click gets it wrong half the time.
fn card<V: 'static>(
    index: usize,
    stream: &LiveStream,
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

    let preview = match thumbnail {
        Some(path) => img(path)
            .w_full()
            .h(px(CARD_WIDTH * 9.0 / 16.0))
            .into_any_element(),
        // Sized placeholder, so the grid does not reflow as images arrive.
        None => div()
            .w_full()
            .h(px(CARD_WIDTH * 9.0 / 16.0))
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    let meta = [
        Some(format_viewers(stream.viewer_count) + " watching"),
        uptime(&stream.started_at),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");

    div()
        .id(("stream-card", index))
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .rounded_md()
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
                    // Live badge, bottom-left of the thumbnail where broadcast
                    // UIs have put it for decades.
                    div()
                        .absolute()
                        .bottom(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .px(px(theme::CONTROL_PAD_X))
                        .py(px(theme::CONTROL_PAD_Y))
                        .rounded_sm()
                        .bg(theme::live())
                        .text_size(px(theme::TEXT_MICRO))
                        .font_weight(theme::weight_shout())
                        .text_color(rgb(0xffffff))
                        .child("LIVE"),
                )
                .when(can_add, |thumb| {
                    thumb.child(
                        div()
                            .id(("add-stream", index))
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .right(px(theme::GAP_TIGHT))
                            .px(px(theme::CONTROL_PAD_X))
                            .py(px(theme::CONTROL_PAD_Y))
                            .rounded_sm()
                            .bg(theme::surface_raised())
                            .text_size(px(theme::TEXT_LABEL))
                            .font_weight(theme::weight_label())
                            .text_color(theme::text())
                            .cursor_pointer()
                            .opacity(0.0)
                            .group_hover("card", |style| style.opacity(1.0))
                            .hover(|style| style.bg(theme::accent_dim()))
                            .active(|style| style.bg(theme::pressed()))
                            .child("+ add")
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
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_muted())
                        .truncate()
                        .child(SharedString::from(stream.title.clone())),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(if stream.game_name.is_empty() {
                            meta.clone()
                        } else {
                            format!("{} · {meta}", stream.game_name)
                        })),
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
    div()
        .id(("offline-follow", index))
        .px(px(theme::CONTROL_PAD_X))
        .py(px(theme::CONTROL_PAD_Y))
        .rounded_sm()
        .bg(theme::surface())
        .text_size(px(theme::TEXT_LABEL))
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text_dim())
        .cursor_pointer()
        .hover(|style| style.bg(theme::hover()).text_color(theme::text_muted()))
        .active(|style| style.bg(theme::pressed()))
        .child(SharedString::from(channel.display_name.clone()))
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
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let mut page = scroller("browse-grid");

    if !follows.is_empty() {
        page = page
            .when(!offline.is_empty(), |page| page.child(heading("live")))
            .child(stream_row(follows, cache, can_add, on_action.clone(), cx));
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
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> gpui::Div {
    let mut row = wrap_row(theme::GAP_SECTION);
    for (index, stream) in streams.iter().enumerate() {
        row = row.child(card(index, stream, cache, can_add, on_action.clone(), cx));
    }
    row
}

fn category_row<V: 'static>(
    categories: &[Category],
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> gpui::Div {
    let mut row = wrap_row(theme::GAP);
    for (index, category) in categories.iter().enumerate() {
        row = row.child(category_card(index, category, cache, on_action.clone(), cx));
    }
    row
}

fn stream_grid<V: 'static>(
    id: &'static str,
    streams: &[LiveStream],
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    scroller(id)
        .child(stream_row(streams, cache, can_add, on_action, cx))
        .into_any_element()
}

/// Everything a search turned up, channels first.
///
/// A live channel is directly watchable; a category is another click. Searching
/// a streamer's name and having to scroll past twenty games to reach them is
/// the wrong way round.
fn search_view<V: 'static>(
    results: &SearchResults,
    discovery: &Discovery,
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
                list.child(heading("Live channels")).child(stream_row(
                    &results.streams,
                    cache,
                    can_add,
                    on_action.clone(),
                    cx,
                ))
            })
            .when(shown > 0, |list| {
                list.child(heading("Categories")).child(category_row(
                    &results.categories[..shown],
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
    let art_height = CATEGORY_WIDTH * BOX_ART_HEIGHT as f32 / BOX_ART_WIDTH as f32;

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
        .w(px(CATEGORY_WIDTH))
        .flex()
        .flex_col()
        .rounded_md()
        .overflow_hidden()
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_raised()))
        .active(|style| style.bg(theme::pressed()))
        .child(cover)
        .child(
            div()
                .p(px(theme::PANEL_PAD))
                .text_size(px(theme::TEXT_BODY))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .truncate()
                .child(SharedString::from(category.name.clone())),
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
        .child(
            div()
                .id(id)
                .px(px(theme::CONTROL_PAD_X))
                .py(px(theme::CONTROL_PAD_Y))
                .rounded_sm()
                .bg(theme::surface_raised())
                .text_size(px(theme::TEXT_LABEL))
                .font_weight(theme::weight_label())
                .text_color(theme::text_muted())
                .cursor_pointer()
                .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
                .active(|style| style.bg(theme::pressed()))
                .child(back)
                .on_click(cx.listener(move |view, _event, window, cx| {
                    on_action(view, action.clone(), window, cx)
                })),
        )
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
            div()
                .id("open-activate")
                .px(px(theme::PANEL_PAD))
                .py(px(theme::CONTROL_PAD_Y))
                .rounded_md()
                .bg(theme::surface_raised())
                // The one accent on the page, because it is the one thing to do.
                .border_1()
                .border_color(theme::accent())
                .text_size(px(theme::TEXT_LABEL))
                .font_weight(theme::weight_label())
                .text_color(theme::text())
                .cursor_pointer()
                .hover(|style| style.bg(theme::hover()))
                .active(|style| style.bg(theme::pressed()))
                .child("Open twitch.tv/activate")
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
fn empty_state<V: 'static>(sign_in: &SignIn, cx: &mut Context<V>) -> AnyElement {
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
        SignIn::SignedIn(_) => (
            "Nobody is live".into(),
            "None of the channels you follow are streaming right now.".into(),
        ),
    };

    let body = notice(title, detail, matches!(sign_in, SignIn::Error(_)));

    // Only the state that is actually still going breathes. "Nobody is live"
    // and "Not signed in" are answers, not progress, and a pulsing answer both
    // misleads and repaints forever.
    match sign_in {
        SignIn::Connecting => motion::waiting("connecting", body).into_any_element(),
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
    cache: &Arc<ImageCache>,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    // Search and categories both take over the page rather than nesting inside
    // a tab, so there is only ever one thing to scroll.
    let body = if let Some(results) = &discovery.search {
        search_view(results, discovery, cache, can_add, on_action, cx)
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
            Tab::Following if follows.is_empty() && offline.is_empty() => empty_state(sign_in, cx),
            Tab::Following => following_view(follows, offline, cache, can_add, on_action, cx),
            Tab::Popular if discovery.popular.is_empty() => browse_placeholder(
                discovery,
                "Twitch reported nothing live, which would be a first.".into(),
            ),
            Tab::Popular => stream_grid(
                "popular-grid",
                &discovery.popular,
                cache,
                can_add,
                on_action,
                cx,
            ),
            Tab::Categories if discovery.categories.is_empty() => {
                browse_placeholder(discovery, "No categories came back.".into())
            }
            Tab::Categories => scroller("categories-grid")
                .child(category_row(&discovery.categories, cache, on_action, cx))
                .into_any_element(),
        }
    };

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme::bg())
        .child(body)
}

#[cfg(test)]
mod tests {
    use super::*;

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
