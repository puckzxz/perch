//! Home: the tab the browse page opens on, and the one that answers "what
//! should I watch".
//!
//! Three answers, in the order they are likely to be the one: who you follow
//! that is live now, then the recordings you did not finish, then everyone
//! else you follow. It used to be the Following tab, which had the first and
//! the last; picking a twelve-hour broadcast back up meant knowing to look in
//! the History tab, a place you go rather than one you land on.
//!
//! Everything here is the app's already. The live cards are the ones every
//! list of streams draws, the recordings are the History tab's own cards
//! (`history_page::entry_card`), and the offline names are the ones search
//! results show, each saying when it was last live (`crate::last_live`).
//! What this module adds is which of them, and in what order: [`continuing`]
//! and [`by_last_watched`], pure and tested. Each heading carries how many
//! are under it, after the filter ([`counted`]).

use std::collections::HashMap;

use chrono::Utc;
use emotes::ImageCache;
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle};
use settings::history::{History, Watched};
use twitch_api::{Channel, LiveStream};

use crate::browse::{self, Action, SignIn, Tab};
use crate::last_live::LastLive;
use crate::{controls, history_page, layout, palette, theme};

/// The most recordings Continue watching shows, however wide the window.
///
/// One row is the point: this is a reminder sitting between who is live and
/// who is not, and the History tab is a click away for the rest. Six is a
/// row at 4K, and stops an ultrawide's row turning into the whole history.
const CONTINUING_MAX: usize = 6;

/// Everything Home lists, as the root holds it.
pub struct Lists<'a> {
    /// The live follows, in the order the follows poll keeps them.
    pub live: &'a [LiveStream],
    /// Everyone else you follow, in the order you last watched them (see
    /// [`by_last_watched`]), or held where they stand while the pointer is on
    /// the page. Drawn in the order given.
    pub offline: &'a [Channel],
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
/// more than fitted, and how many matched in all.
#[derive(Debug, PartialEq)]
pub struct Continuing<'a> {
    pub shown: Vec<&'a Watched>,
    /// More matched than the row holds, which is what offers "Show all".
    pub more: bool,
    /// Every unfinished recording that matched, shown or not: the count on
    /// the heading, which says how much "Show all" has behind it.
    pub total: usize,
}

/// The unfinished recordings that match `filter`, most recently watched
/// first, at most `row` of them — and never more than [`CONTINUING_MAX`],
/// nor fewer than one while there is one to show.
///
/// Unfinished is the History tab's own test (`history_page::unfinished`),
/// so the two Continue watchings list the same recordings. Most recently
/// watched first is the history's own order: opening a recording or getting
/// further into it moves it to the front (`History::opened`,
/// `History::progressed`). A filter matches as the palette's does, by the
/// channel's name or by the title holding what was typed.
pub fn continuing<'a>(history: &'a History, filter: &str, row: usize) -> Continuing<'a> {
    let cap = row.clamp(1, CONTINUING_MAX);
    let mut matching = history
        .videos
        .iter()
        .filter(|watched| history_page::unfinished(watched))
        .filter(|watched| palette::watched_matches(watched, filter));
    let shown: Vec<&Watched> = matching.by_ref().take(cap).collect();
    let rest = matching.count();
    Continuing {
        total: shown.len() + rest,
        more: rest > 0,
        shown,
    }
}

/// A heading with how many are under it: "Live now · 23". The count is of
/// what the section lists after the filter, so typing in the box counts
/// down with it; Continue watching counts every match, including those
/// behind "Show all".
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
    live: bool,
    /// "Live now" over the cards, only when something else is on the page:
    /// a list of live follows and nothing else needs no title, and drawing
    /// one over it alone was what the Following tab never did.
    live_heading: bool,
    continuing: bool,
    offline: bool,
}

impl Sections {
    /// From how much each section has, after the filter. Continue watching
    /// and Offline always say what they are when they are there: the one has
    /// "Show all" on its heading's line, and the other comes after two kinds
    /// of card.
    fn new(live: usize, continuing: usize, offline: usize) -> Self {
        Self {
            live: live > 0,
            live_heading: live > 0 && (continuing > 0 || offline > 0),
            continuing: continuing > 0,
            offline: offline > 0,
        }
    }

    fn any(self) -> bool {
        self.live || self.continuing || self.offline
    }
}

/// The page.
///
/// Under one scroller, so there is only ever one thing to scroll, with the
/// filter box over it. The box narrows all three sections as it is typed: a
/// name you follow, or a recording by its channel or its title. It is the
/// palette's matcher, and it exists because the offline list is the longest
/// thing in the app and the box in the title bar asks Twitch, not the app.
///
/// With no follows at all — signed out, signing in, the first poll still
/// out — the page is the follows' empty state (`browse::empty_state`),
/// sign-in included, with Continue watching over it when there is anything
/// to continue: the history is the app's own, and needs no sign-in to play
/// from.
#[allow(clippy::too_many_arguments)]
pub fn view<V: 'static>(
    lists: Lists,
    filter: &str,
    filter_box: AnyElement,
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
    // after a client id change. No filter box here, so nothing filters.
    if lists.live.is_empty() && lists.offline.is_empty() {
        let continuing = continuing(lists.history, "", row);
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

    let filter = filter.trim();
    let live: Vec<LiveStream> = lists
        .live
        .iter()
        .filter(|stream| {
            palette::matches(&stream.display_name, filter)
                || palette::matches(&stream.user_login, filter)
        })
        .cloned()
        .collect();
    let continuing = continuing(lists.history, filter, row);
    let offline: Vec<&Channel> = lists
        .offline
        .iter()
        .filter(|channel| {
            palette::matches(&channel.display_name, filter)
                || palette::matches(&channel.login, filter)
        })
        .collect();
    let sections = Sections::new(live.len(), continuing.shown.len(), offline.len());

    let mut page = browse::scroller("browse-grid", scroll, room.bottom).child(
        div()
            .flex_none()
            .w(px(browse::FILTER_WIDTH))
            .child(filter_box),
    );

    if sections.live {
        page = page
            .when(sections.live_heading, |page| {
                page.child(browse::heading(counted("Live now", live.len())))
            })
            .child(browse::stream_row(
                &live,
                room.width,
                cache,
                can_add,
                on_action.clone(),
                cx,
            ));
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

    if sections.offline {
        let now = Utc::now();
        let mut row = browse::wrap_row(theme::GAP_TIGHT);
        for (index, channel) in offline.iter().enumerate() {
            row = row.child(browse::offline_pill(
                ("offline-follow", index),
                channel,
                lists.last_live.words(&channel.login, now),
                on_action.clone(),
                cx,
            ));
        }
        page = page
            .child(browse::heading(counted("Offline", offline.len())))
            .child(row);
    }

    // The same notice every other empty list gets, rather than a heading over
    // nothing — and the way on from it: this box never leaves the app, so a
    // name it cannot find is one to ask Twitch about instead.
    if !sections.any() && !filter.is_empty() {
        let query = filter.to_string();
        page = page.child(
            browse::notice(
                format!("Nothing here matches “{filter}”").into(),
                "This box only looks through the channels you follow and the recordings \
                 you have watched."
                    .into(),
                false,
            )
            .child(
                controls::pill(
                    "filter-search",
                    format!("Search Twitch for “{filter}”"),
                    controls::Variant::Primary,
                )
                .on_click(cx.listener(move |view, _event, window, cx| {
                    on_action(view, Action::Search(query.clone()), window, cx)
                })),
            ),
        );
    }

    browse::scrollable(page, scroll).into_any_element()
}

/// Continue watching: its heading, with "Show all" on the same line when the
/// history has more than the row, and the row. Nothing at all when there is
/// nothing to continue, so the section leaves no heading over an empty row.
///
/// "Show all" goes to the History tab rather than unfolding here: that tab is
/// the whole list, finished ones included, with the one control that clears
/// it, and a second copy of it inside Home would be two places to keep.
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
                    .tooltip(controls::tip("Everything you have watched, on History"))
                    .on_click(cx.listener(move |view, _event, window, cx| {
                        on_action(view, Action::ShowTab(Tab::History), window, cx)
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

        let wide = continuing(&history, "", 4);
        assert_eq!(ids(&wide), ["a", "c", "d"], "a finished one was listed");
        assert!(!wide.more, "offered more when everything fitted");
        assert_eq!(wide.total, 3);

        let narrow = continuing(&history, "", 2);
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
        assert_eq!(continuing(&history, "", 0).shown.len(), 1);
        assert_eq!(continuing(&history, "", 11).shown.len(), CONTINUING_MAX);
        assert!(continuing(&history, "", 11).more);
    }

    /// Nothing to continue is no section, not an empty one — including a
    /// history of nothing but finished recordings.
    #[test]
    fn nothing_unfinished_shows_nothing() {
        assert!(continuing(&History::default(), "", 3).shown.is_empty());
        let done = History {
            videos: vec![watched("a", "xqc", true)],
        };
        let none = continuing(&done, "", 3);
        assert!(none.shown.is_empty());
        assert!(!none.more);
    }

    /// The filter narrows it by channel, a few letters at a time, or by a
    /// title holding what was typed; and "more" counts only what matched.
    #[test]
    fn the_filter_narrows_continue_watching() {
        let history = History {
            videos: vec![
                watched("a", "xqc", false),
                watched("b", "forsen", false),
                watched("c", "xqc", false),
            ],
        };
        let by_name = continuing(&history, "xq", 1);
        assert_eq!(ids(&by_name), ["a"]);
        assert!(by_name.more, "the other xqc recording was not offered");
        assert_eq!(by_name.total, 2, "counted what the filter left out");

        assert_eq!(ids(&continuing(&history, "forsen plays", 3)), ["b"]);
        assert!(continuing(&history, "nobody", 3).shown.is_empty());
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
    /// the other two headings come with their sections.
    #[test]
    fn the_live_heading_shows_only_beside_another_section() {
        let alone = Sections::new(3, 0, 0);
        assert!(alone.live && !alone.live_heading);

        assert!(Sections::new(3, 1, 0).live_heading);
        assert!(Sections::new(3, 0, 5).live_heading);

        let nobody_live = Sections::new(0, 2, 5);
        assert!(!nobody_live.live && !nobody_live.live_heading);
        assert!(nobody_live.continuing && nobody_live.offline);

        assert!(!Sections::new(0, 0, 0).any());
    }
}
