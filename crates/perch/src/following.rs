//! Following: the tab the browse page opens on, and the one that answers
//! "what should I watch".
//!
//! The channels you keep up with, grouped as the rail groups them
//! (`sidebar::groups`), so the two never disagree about who goes where: the
//! pins, who you follow that is live, the rail's recommendations and
//! everyone else you follow — and after who is live, the recordings you did
//! not finish. Pinned, Live now, Continue watching, Recommended, Offline. A
//! pin is in Pinned and nowhere else, as in the rail. Picking a twelve-hour
//! broadcast back up used to mean knowing to look in the history, a place
//! you go rather than one you land on.
//!
//! Everything here is the app's already. The live cards are the ones every
//! list of streams draws — a recommendation's saying which channel led to
//! it, as its rail row does — the recordings are the Library tab's own cards
//! (`history_page::entry_card`), and the offline names are the ones search
//! results show, each saying when it was last live (`crate::last_live`).
//! What this module adds is which of them, and in what order: [`continuing`],
//! [`by_last_watched`] and [`split_pins`], pure and tested. Each heading
//! carries how many are under it ([`counted`]), but Recommended's, which is
//! a ranked handful rather than a count of anything yours. It had a filter
//! box of its own over the page, until the title bar's search box came to
//! answer from what Perch knows as you type (`root::search_box`).

use std::collections::HashMap;

use chrono::Utc;
use emotes::ImageCache;
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle};
use settings::history::{History, Watched};
use twitch_api::{Channel, LiveStream};

use crate::browse::{self, Action, SignIn, Tab};
use crate::last_live::LastLive;
use crate::recommended::{self, Suggestion};
use crate::sidebar::{self, Row};
use crate::{controls, history_page, layout, theme};

/// The most recordings Continue watching shows, however wide the window.
///
/// One row is the point: this is a reminder sitting between who is live and
/// who is not, and the Library tab is a click away for the rest. Six is a
/// row at 4K, and stops an ultrawide's row turning into the whole history.
const CONTINUING_MAX: usize = 6;

/// Everything Following lists, as the root holds it; the browse page's
/// other places read the sign-in and the history from it too.
#[derive(Clone, Copy)]
pub struct Lists<'a> {
    /// The live follows, in the order the follows poll keeps them.
    pub live: &'a [LiveStream],
    /// Everyone else you follow, in the order you last watched them (see
    /// [`by_last_watched`]), or held where they stand while the pointer is on
    /// the page. Drawn in the order given.
    pub offline: &'a [Channel],
    /// What Helix said of the channels the follows poll does not ask about,
    /// the pins nobody follows among them (`root::pane_streams`): a pin
    /// found here is live, as in the rail.
    pub asked: &'a [LiveStream],
    /// What is pinned, in the order the rail draws it
    /// (`sidebar::pin_order`), or held still under the pointer.
    pub pinned: &'a [String],
    /// The rail's recommendations, ranked, or held still the same way.
    pub recommended: &'a [Suggestion],
    /// When each offline follow was last live, as far as Twitch has said:
    /// the words beside its name.
    pub last_live: &'a LastLive,
    pub history: &'a History,
    pub sign_in: &'a SignIn,
    /// Whether a follows poll has answered yet, which is what tells an empty
    /// list that is still being asked for from one that is the answer.
    pub follows_loaded: bool,
}

/// What Continue watching shows: the recordings, whether the history has
/// more than fitted, and how many there are in all.
#[derive(Debug, PartialEq)]
pub struct Continuing<'a> {
    pub shown: Vec<&'a Watched>,
    /// More than the row holds, which is what offers "Show all".
    pub more: bool,
    /// Every unfinished recording, shown or not: the count on the heading,
    /// which says how much "Show all" has behind it.
    pub total: usize,
}

/// The unfinished recordings, most recently watched first, at most `row` of
/// them — and never more than [`CONTINUING_MAX`], nor fewer than one while
/// there is one to show.
///
/// Unfinished is the Library tab's own test (`history_page::unfinished`),
/// so the two Continue watchings list the same recordings. Most recently
/// watched first is the history's own order: opening a recording or getting
/// further into it moves it to the front (`History::opened`,
/// `History::progressed`).
pub fn continuing(history: &History, row: usize) -> Continuing<'_> {
    let cap = row.clamp(1, CONTINUING_MAX);
    let mut unfinished = history
        .videos
        .iter()
        .filter(|watched| history_page::unfinished(watched));
    let shown: Vec<&Watched> = unfinished.by_ref().take(cap).collect();
    let rest = unfinished.count();
    Continuing {
        total: shown.len() + rest,
        more: rest > 0,
        shown,
    }
}

/// A heading with how many are under it: "Live now · 23". Continue
/// watching counts every unfinished recording, including those behind "Show
/// all".
fn counted(label: &str, count: usize) -> String {
    format!("{label} · {count}")
}

/// Put the offline follows in the order you last watched them, most recent
/// first; the ones never watched follow, by name, as the list always was.
///
/// When is read from two lists the app already keeps. `recent` — the
/// settings' recently watched channels, live or recorded — comes first and
/// is exact, but holds eight. The history's recordings, newest first, carry
/// on after it for the channels that fell off the end of `recent`, and
/// every one of them was watched before anything still in it, so joining
/// the two keeps the order true.
///
/// A sort, so never while the page is held under the pointer: the root calls
/// it only where it would have put the names back in name order — a poll
/// that lands while nothing is held, and the moment the hold lets go (see
/// `RootView::hold_live`). Drawn every frame instead, it moved a channel
/// whose stream had just ended from the end of the names, where the hold had
/// put it, up to where you last watched it, which is near the top if you
/// watched it live, shifting every name after it under the pointer.
pub fn by_last_watched(channels: &mut [Channel], recent: &[String], history: &History) {
    let mut rank: HashMap<String, usize> = HashMap::new();
    let watched = recent.iter().map(String::as_str).chain(
        history
            .videos
            .iter()
            .map(|watched| watched.channel_login.as_str()),
    );
    for login in watched {
        let next = rank.len();
        rank.entry(settings::channel_key(login)).or_insert(next);
    }
    twitch_api::by_name(channels);
    channels.sort_by_cached_key(|channel| {
        rank.get(&settings::channel_key(&channel.login))
            .copied()
            .unwrap_or(usize::MAX)
    });
}

/// Which sections the page draws, and which of them wear a heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sections {
    pinned: bool,
    live: bool,
    /// "Live now" over the cards, only when something else is on the page:
    /// a list of live follows and nothing else needs no title, and drawing
    /// one over it alone was what the tab never did.
    live_heading: bool,
    continuing: bool,
    recommended: bool,
    offline: bool,
}

impl Sections {
    /// From how much each section has. Every other
    /// section always says what it is when it is there: Continue watching
    /// has "Show all" on its heading's line, and the rest come before or
    /// after another kind of card.
    fn new(
        pinned: usize,
        live: usize,
        continuing: usize,
        recommended: usize,
        offline: usize,
    ) -> Self {
        Self {
            pinned: pinned > 0,
            live: live > 0,
            live_heading: live > 0 && pinned + continuing + recommended + offline > 0,
            continuing: continuing > 0,
            recommended: recommended > 0,
            offline: offline > 0,
        }
    }
}

/// Pinned as the page draws it, from the rail's rows of it in their order:
/// who is on as cards, as Live now draws them, and everyone else as names,
/// as Offline does — a pin no list has placed yet by its login, which is all
/// the rail shows of it either.
pub fn split_pins<'a>(rows: &[Row<'a>]) -> (Vec<&'a LiveStream>, Vec<Channel>) {
    let mut live = Vec::new();
    let mut off = Vec::new();
    for row in rows {
        match *row {
            Row::Live(stream) => live.push(stream),
            Row::Offline(channel) => off.push(channel.clone()),
            Row::Unknown(login) => off.push(Channel {
                login: login.to_string(),
                user_id: String::new(),
                display_name: login.to_string(),
            }),
            // Never a pin's row; see `sidebar::groups`.
            Row::Recommended(_) => {}
        }
    }
    (live, off)
}

/// The page, under one scroller, so there is only ever one thing to scroll.
///
/// With no follows at all — signed out, signing in, the first poll still
/// out — the page is the follows' empty state (`browse::empty_state`),
/// sign-in included, with Continue watching over it when there is anything
/// to continue: the history is the app's own, and needs no sign-in to play
/// from.
pub fn view<V: 'static>(
    lists: Lists,
    room: layout::Room,
    cache: &ImageCache,
    can_add: bool,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let row = browse::columns(room.width, browse::CARD_MIN, theme::GAP_SECTION);

    // Sign-in lives on this page, so an empty follows list has more to say
    // than "nothing here". Both lists have to be empty: signed in with
    // everybody offline is not the same as not signed in, and testing only
    // the live one would hide the sign-in prompt behind a stale offline list
    // after a client id change.
    if lists.live.is_empty() && lists.offline.is_empty() {
        let continuing = continuing(lists.history, row);
        let empty = browse::empty_state(lists.sign_in, lists.follows_loaded, on_action.clone(), cx);
        if continuing.shown.is_empty() {
            return empty;
        }
        return div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(theme::GAP_SECTION))
                    .p(px(theme::PAGE_PAD))
                    .children(continuing_section(
                        &continuing,
                        room,
                        cache,
                        can_add,
                        on_action,
                        cx,
                    )),
            )
            .child(empty)
            .into_any_element();
    }

    let groups = sidebar::groups(
        lists.live,
        lists.offline,
        lists.asked,
        lists.recommended,
        lists.pinned,
        lists.follows_loaded,
    );
    let (pinned_live, pinned_off) = split_pins(&groups.pinned);
    let live = &groups.live;
    let continuing = continuing(lists.history, row);
    // Drawn as a live card from what the rail's answer said of each, with
    // the live preview at the address every live channel's is
    // (`recommended::as_stream`), and why it is offered after its game.
    let suggested: Vec<(LiveStream, &str)> = groups
        .recommended
        .iter()
        .map(|suggestion| {
            (
                recommended::as_stream(&suggestion.channel),
                suggestion.reason.as_str(),
            )
        })
        .collect();
    let offline = &groups.offline;
    let sections = Sections::new(
        pinned_live.len() + pinned_off.len(),
        live.len(),
        continuing.shown.len(),
        suggested.len(),
        offline.len(),
    );
    let now = Utc::now();

    let mut page = browse::scroller("browse-grid", scroll);

    // Each row of cards under an id of its own: the cards are numbered
    // within their row, and two rows on one page would otherwise share ids.
    if sections.pinned {
        page = page
            .child(browse::heading(counted(
                "Pinned",
                pinned_live.len() + pinned_off.len(),
            )))
            .when(!pinned_live.is_empty(), |page| {
                page.child(
                    browse::cards_row(
                        pinned_live.iter().map(|stream| (*stream, None)),
                        room.width,
                        cache,
                        can_add,
                        on_action.clone(),
                        cx,
                    )
                    .id("pinned-live"),
                )
            })
            .when(!pinned_off.is_empty(), |page| {
                page.child(names_row(
                    "pinned-off",
                    pinned_off.iter(),
                    lists.last_live,
                    now,
                    on_action.clone(),
                    cx,
                ))
            });
    }

    if sections.live {
        page = page
            .when(sections.live_heading, |page| {
                page.child(browse::heading(counted("Live now", live.len())))
            })
            .child(
                browse::cards_row(
                    live.iter().map(|stream| (*stream, None)),
                    room.width,
                    cache,
                    can_add,
                    on_action.clone(),
                    cx,
                )
                .id("live-now"),
            );
    }

    if sections.continuing {
        page = page.children(continuing_section(
            &continuing,
            room,
            cache,
            can_add,
            on_action.clone(),
            cx,
        ));
    }

    if sections.recommended {
        page = page.child(browse::heading("Recommended")).child(
            browse::cards_row(
                suggested.iter().map(|(stream, why)| (stream, Some(*why))),
                room.width,
                cache,
                can_add,
                on_action.clone(),
                cx,
            )
            .id("recommended"),
        );
    }

    if sections.offline {
        page = page
            .child(browse::heading(counted("Offline", offline.len())))
            .child(names_row(
                "offline-follow",
                offline.iter().copied(),
                lists.last_live,
                now,
                on_action.clone(),
                cx,
            ));
    }

    browse::scrollable(page, scroll).into_any_element()
}

/// A wrap of offline names, each saying when it was last live where Twitch
/// has said (`browse::offline_pill`), numbered under `id`.
fn names_row<'a, V: 'static>(
    id: &'static str,
    channels: impl Iterator<Item = &'a Channel>,
    last_live: &LastLive,
    now: chrono::DateTime<Utc>,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> gpui::Div {
    let mut row = browse::wrap_row(theme::GAP_TIGHT);
    for (index, channel) in channels.enumerate() {
        row = row.child(browse::offline_pill(
            (id, index),
            channel,
            last_live.words(&channel.login, now),
            on_action.clone(),
            cx,
        ));
    }
    row
}

/// Continue watching: its heading, with "Show all" on the same line when the
/// history has more than the row, and the row. Nothing at all when there is
/// nothing to continue, so the section leaves no heading over an empty row.
///
/// "Show all" goes to the Library tab rather than unfolding here: that tab is
/// the whole list, finished ones included, with the one control that clears
/// it, and a second copy of it inside Following would be two places to keep.
fn continuing_section<V: 'static>(
    continuing: &Continuing,
    room: layout::Room,
    cache: &ImageCache,
    can_add: bool,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Option<impl IntoElement> {
    if continuing.shown.is_empty() {
        return None;
    }
    let now = Utc::now();
    let card_width = browse::card_width(
        room.width,
        browse::CARD_MIN,
        browse::CARD_MAX,
        theme::GAP_SECTION,
    );
    let mut row = browse::wrap_row(theme::GAP_SECTION);
    for (index, watched) in continuing.shown.iter().enumerate() {
        row = row.child(history_page::entry_card(
            index,
            watched,
            card_width,
            cache,
            can_add,
            now,
            on_action.clone(),
            cx,
        ));
    }

    let heading = div()
        .flex()
        .flex_row()
        .items_center()
        .child(browse::heading(counted(
            "Continue watching",
            continuing.total,
        )))
        .child(div().flex_1())
        .when(continuing.more, |line| {
            line.child(
                controls::pill("continue-all", "Show all", controls::Variant::Pill)
                    .tooltip(controls::tip("Everything you have watched, in Library"))
                    .on_click(cx.listener(move |view, _event, window, cx| {
                        on_action(view, Action::ShowTab(Tab::Library), window, cx)
                    })),
            )
        });

    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_SECTION))
            .child(heading)
            .child(row),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watched(id: &str, channel: &str, finished: bool) -> Watched {
        Watched {
            id: id.into(),
            channel_login: channel.into(),
            channel_name: String::new(),
            channel_id: String::new(),
            title: format!("{channel} plays something"),
            created_at: String::new(),
            length_secs: 3_600,
            thumbnail_url: String::new(),
            kind: "archive".into(),
            position_secs: if finished { 3_590 } else { 600 },
            finished,
            watched_at: 0,
        }
    }

    fn channel(login: &str) -> Channel {
        Channel {
            login: login.into(),
            display_name: login.into(),
            user_id: String::new(),
        }
    }

    fn ids<'a>(continuing: &Continuing<'a>) -> Vec<&'a str> {
        continuing
            .shown
            .iter()
            .map(|watched| watched.id.as_str())
            .collect()
    }

    /// Only what is left to finish, in the history's own most-recent-first
    /// order, one row of it — and the rest offered, rather than lost.
    #[test]
    fn continue_watching_is_one_row_of_the_unfinished_newest_first() {
        let history = History {
            videos: vec![
                watched("a", "xqc", false),
                watched("b", "forsen", true),
                watched("c", "quin69", false),
                watched("d", "xqc", false),
            ],
        };

        let wide = continuing(&history, 4);
        assert_eq!(ids(&wide), ["a", "c", "d"], "a finished one was listed");
        assert!(!wide.more, "offered more when everything fitted");
        assert_eq!(wide.total, 3);

        let narrow = continuing(&history, 2);
        assert_eq!(ids(&narrow), ["a", "c"]);
        assert!(narrow.more, "the third was neither shown nor offered");
        assert_eq!(narrow.total, 3, "counted only what fitted");
    }

    /// However narrow, one card; however wide, six.
    #[test]
    fn continue_watching_shows_between_one_and_six() {
        let history = History {
            videos: (0..10)
                .map(|n| watched(&n.to_string(), "xqc", false))
                .collect(),
        };
        assert_eq!(continuing(&history, 0).shown.len(), 1);
        assert_eq!(continuing(&history, 11).shown.len(), CONTINUING_MAX);
        assert!(continuing(&history, 11).more);
    }

    /// Nothing to continue is no section, not an empty one — including a
    /// history of nothing but finished recordings.
    #[test]
    fn nothing_unfinished_shows_nothing() {
        assert!(continuing(&History::default(), 3).shown.is_empty());
        let done = History {
            videos: vec![watched("a", "xqc", true)],
        };
        let none = continuing(&done, 3);
        assert!(none.shown.is_empty());
        assert!(!none.more);
    }

    /// The settings' recents first, then the history's channels, then the
    /// never-watched by name — and one channel once, however many recordings
    /// of it the history holds.
    #[test]
    fn offline_names_go_by_when_they_were_last_watched() {
        let mut offline = vec![
            channel("ccc"),
            channel("bbb"),
            channel("aaa"),
            channel("ddd"),
            channel("eee"),
        ];
        let recent = vec!["ddd".to_string(), "nobody_followed".to_string()];
        let history = History {
            videos: vec![
                watched("1", "ddd", false),
                watched("2", "bbb", true),
                watched("3", "eee", false),
                watched("4", "bbb", false),
            ],
        };
        by_last_watched(&mut offline, &recent, &history);
        let order: Vec<&str> = offline.iter().map(|c| c.login.as_str()).collect();
        assert_eq!(order, ["ddd", "bbb", "eee", "aaa", "ccc"]);
    }

    /// The recents are keyed the settings' way, so a capital or a leading
    /// hash in either list does not lose a channel its place.
    #[test]
    fn offline_order_ignores_case() {
        let mut offline = vec![channel("aaa"), channel("Forsen")];
        by_last_watched(&mut offline, &["#forsen".to_string()], &History::default());
        assert_eq!(offline[0].login, "Forsen");
    }

    /// A heading says how many are under it.
    #[test]
    fn headings_carry_their_counts() {
        assert_eq!(counted("Live now", 23), "Live now · 23");
        assert_eq!(counted("Offline", 0), "Offline · 0");
    }

    /// "Live now" titles the cards only when something else shares the page;
    /// the other headings come with their sections.
    #[test]
    fn the_live_heading_shows_only_beside_another_section() {
        let alone = Sections::new(0, 3, 0, 0, 0);
        assert!(alone.live && !alone.live_heading);

        assert!(Sections::new(1, 3, 0, 0, 0).live_heading);
        assert!(Sections::new(0, 3, 1, 0, 0).live_heading);
        assert!(Sections::new(0, 3, 0, 2, 0).live_heading);
        assert!(Sections::new(0, 3, 0, 0, 5).live_heading);

        let nobody_live = Sections::new(0, 0, 2, 0, 5);
        assert!(!nobody_live.live && !nobody_live.live_heading);
        assert!(nobody_live.continuing && nobody_live.offline);
    }

    fn live(login: &str) -> LiveStream {
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

    /// Pinned splits as it is drawn: who is on as cards, everyone else as
    /// names in the rail's order, a pin nothing has placed by its login.
    #[test]
    fn pins_split_into_cards_and_names() {
        let on = live("xqc");
        let off = channel("forsen");
        let rows = [
            Row::Offline(&off),
            Row::Live(&on),
            Row::Unknown("handpinned"),
        ];

        let (cards, names) = split_pins(&rows);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].user_login, "xqc");
        let logins: Vec<&str> = names.iter().map(|c| c.login.as_str()).collect();
        assert_eq!(logins, ["forsen", "handpinned"]);
        assert_eq!(names[1].display_name, "handpinned");
    }
}
