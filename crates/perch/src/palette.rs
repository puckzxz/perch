//! The command palette.
//!
//! Everything you can reach while watching is a key or a hover-revealed
//! overlay, and everything you can reach while *browsing* was a click — the
//! picker had no keyboard path at all past the search box. This is the one
//! control that answers both: a channel to open, a pane to close, a page to go
//! to, typed rather than aimed at.
//!
//! It is not a second search box. The search box asks *Twitch* a question and
//! costs a request; this filters what the app already knows — who is live, what
//! is open, what it can do — and costs nothing, which is why it can run on
//! every keystroke.

use gpui::{div, prelude::*, px, Context, SharedString};
use twitch_api::{FollowedChannel, LiveStream};

use crate::target::{self, Target};
use crate::theme;

/// How many rows the list shows at once.
///
/// A palette that fills the window is a page, and a page is the thing this
/// exists to avoid going to.
const MAX_ROWS: usize = 8;

/// The panel's width, how far down the window it sits, and how far it drops in
/// as it opens.
///
/// Near the top rather than centred: this is a thing you type at, and the answer
/// grows downwards from it.
const WIDTH: f32 = 520.0;
const TOP: f32 = 120.0;
pub const RISE: f32 = 10.0;

/// Something the palette can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Watch this channel, closing whatever else is open.
    Watch(String),
    /// Open it beside what is already playing.
    Add(String),
    /// Close the pane at this index.
    Close(usize),
    /// Look at a channel's past broadcasts.
    Videos {
        login: String,
        display_name: String,
        user_id: Option<String>,
    },
    /// Play a recording named by its id — a link somebody pasted — from
    /// `start_secs` in, if the link said where.
    OpenVideo {
        id: String,
        start_secs: Option<u64>,
    },
    GoBrowse,
    GoWatch,
    StopAll,
    ToggleSidebar,
    ToggleSettings,
    Refresh,
}

/// One row: what it does, and what it says it does.
#[derive(Debug, Clone)]
pub struct Entry {
    pub command: Command,
    /// What the row is *about* — a channel's name, or the verb itself.
    pub title: SharedString,
    /// The kind of thing it is, on the right. Deliberately not a description:
    /// the title already says what happens, and a sentence per row turns a list
    /// you scan into a list you read.
    pub kind: SharedString,
}

/// Case-insensitive subsequence match, which is what makes `qb` find
/// `QuickyBaby`.
///
/// Not a fuzzy *score* — the list is already in an order that means something
/// (live channels by viewers, then commands), and re-ranking it by how well a
/// three-letter query matched would throw that away for no gain at this size.
///
/// Public because the Following tab's filter uses the same test: one idea of
/// what "matches" means, wherever the user types a few letters of a name.
pub fn matches(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = haystack.chars().flat_map(char::to_lowercase);
    needle
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| chars.any(|c| c == wanted))
}

/// How many recently watched channels an empty palette leads with.
const RECENT_SHOWN: usize = 5;

/// Everything the palette can offer right now, filtered by `query`.
///
/// Channels first and commands after, because the overwhelmingly common reason
/// to open this is to go somewhere rather than to do something — and within the
/// channels, the order the follows list is already in. With nothing typed the
/// list leads with what was watched most recently, since the usual reason to
/// open a channel is that it was open yesterday, and the live follows come
/// after it, less the ones the recents already named. Offline follows come
/// after the live ones and only once something has been typed: with an empty
/// query they would be a hundred rows of people who are not streaming, but
/// with a name typed they are the reason to open the palette at all — the
/// offline list is the longest thing in the app and this is its only filter
/// besides the one on the Following tab.
///
/// Anything typed that reads as a channel or a recording — a login, or a
/// twitch.tv link — gets rows of its own once nothing followed answers to it
/// at all, so a channel nobody follows is a name and Enter away rather than
/// unreachable. While a follow still matches, the typing is a filter and the
/// row would be noise under it; a link is unambiguous and is always offered.
pub fn entries(
    query: &str,
    follows: &[LiveStream],
    offline: &[FollowedChannel],
    recent: &[String],
    watching: &[String],
    can_add: bool,
) -> Vec<Entry> {
    let mut entries = Vec::new();

    // The name a channel writes itself as, when any list knows it.
    let name_of = |login: &str| -> String {
        follows
            .iter()
            .find(|stream| stream.user_login == login)
            .map(|stream| stream.display_name.clone())
            .or_else(|| {
                offline
                    .iter()
                    .find(|channel| channel.login == login)
                    .map(|channel| channel.display_name.clone())
            })
            .unwrap_or_else(|| login.to_string())
    };
    // Whether the query is narrowing the follows, or naming something else.
    let mut matched_follow = false;
    // A row to open a channel, and beside it one to add it — only when that
    // is different from the row above it.
    let offer = |entries: &mut Vec<Entry>, login: &str, title: String, kind: &'static str| {
        let open = watching.iter().any(|open| open == login);
        entries.push(Entry {
            command: Command::Watch(login.to_string()),
            title: SharedString::from(title.clone()),
            kind: if open { "watching".into() } else { kind.into() },
        });
        if can_add && !open && !watching.is_empty() {
            entries.push(Entry {
                command: Command::Add(login.to_string()),
                title: SharedString::from(format!("{title} — add a pane")),
                kind: "add".into(),
            });
        }
    };

    let mut led_with: Vec<&str> = Vec::new();
    if query.is_empty() {
        for login in recent.iter().take(RECENT_SHOWN) {
            offer(&mut entries, login, name_of(login), "recent");
            led_with.push(login.as_str());
        }
    }

    for stream in follows {
        if led_with.contains(&stream.user_login.as_str()) {
            continue;
        }
        if !matches(&stream.display_name, query) && !matches(&stream.user_login, query) {
            continue;
        }
        matched_follow = true;
        offer(
            &mut entries,
            &stream.user_login,
            stream.display_name.clone(),
            "watch",
        );
    }

    if !query.is_empty() {
        for channel in offline {
            if !matches(&channel.display_name, query) && !matches(&channel.login, query) {
                continue;
            }
            matched_follow = true;
            entries.push(Entry {
                command: Command::Watch(channel.login.clone()),
                title: SharedString::from(channel.display_name.clone()),
                kind: "offline".into(),
            });
        }

        // Everyone's past broadcasts, live or not, after the rows that open
        // them now. Only once something is typed, for the same reason the
        // offline rows wait: with nothing typed this would be a second row
        // for every channel above the commands.
        let past = follows
            .iter()
            .map(|stream| (&stream.user_login, &stream.display_name, &stream.user_id))
            .chain(
                offline
                    .iter()
                    .map(|channel| (&channel.login, &channel.display_name, &channel.user_id)),
            );
        for (login, display_name, user_id) in past {
            if !matches(display_name, query) && !matches(login, query) {
                continue;
            }
            entries.push(Entry {
                command: Command::Videos {
                    login: login.clone(),
                    display_name: display_name.clone(),
                    user_id: Some(user_id.clone()).filter(|id| !id.is_empty()),
                },
                title: SharedString::from(format!("{display_name} — past broadcasts")),
                kind: "videos".into(),
            });
        }

        // What was typed, taken at its word: a channel nobody here follows,
        // or a link to a recording. Only once no follow matches, because
        // until then the typing is a filter over the lists and a row
        // offering to open `f` under everyone whose name has an f in it is
        // noise; a link answers to nobody's name and is always offered.
        match target::parse(query) {
            Some(Target::Channel(login)) if !matched_follow => {
                offer(&mut entries, &login, format!("Open {login}"), "channel");
                entries.push(Entry {
                    command: Command::Videos {
                        login: login.clone(),
                        display_name: login.clone(),
                        user_id: None,
                    },
                    title: SharedString::from(format!("{login} — past broadcasts")),
                    kind: "videos".into(),
                });
            }
            Some(Target::Video { id, start_secs }) => entries.push(Entry {
                command: Command::OpenVideo {
                    id: id.clone(),
                    start_secs,
                },
                title: SharedString::from(format!("Open recording {id}")),
                kind: "recording".into(),
            }),
            _ => {}
        }
    }

    for (index, channel) in watching.iter().enumerate() {
        let title = format!("Close {channel}");
        if matches(&title, query) {
            entries.push(Entry {
                command: Command::Close(index),
                title: SharedString::from(title),
                kind: "pane".into(),
            });
        }
    }

    let commands: [(Command, &str); 6] = [
        (Command::GoBrowse, "Go to follows"),
        (Command::GoWatch, "Back to watching"),
        (Command::StopAll, "Stop all streams"),
        (Command::ToggleSidebar, "Toggle the follows rail"),
        (Command::Refresh, "Refresh this list"),
        (Command::ToggleSettings, "Settings"),
    ];
    for (command, title) in commands {
        if matches(title, query) {
            entries.push(Entry {
                command,
                title: SharedString::from(title),
                kind: "command".into(),
            });
        }
    }

    entries
}

/// The list, as rows. `selected` is an index into `entries`, already clamped.
fn rows<V: 'static>(
    entries: &[Entry],
    selected: usize,
    on_run: impl Fn(&mut V, usize, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    // A window onto the list rather than the whole of it, kept around the
    // selection so arrowing past the bottom scrolls instead of stopping.
    let first = selected.saturating_sub(MAX_ROWS - 1);
    let mut list = div().flex().flex_col().px(px(theme::GAP_TIGHT));

    for (offset, entry) in entries.iter().skip(first).take(MAX_ROWS).enumerate() {
        let index = first + offset;
        let on_run = on_run.clone();
        list = list.child(
            div()
                .id(("palette-row", index))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP))
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP_TIGHT))
                .rounded(px(theme::RADIUS))
                .cursor_pointer()
                .when(index == selected, |row| row.bg(theme::accent_dim()))
                .hover(|style| style.bg(theme::hover()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .w_full()
                        .text_ellipsis()
                        .line_clamp(1)
                        .text_size(px(theme::TEXT_BODY))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text())
                        .child(entry.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(entry.kind.clone()),
                )
                .on_click(
                    cx.listener(move |view, _event, window, cx| on_run(view, index, window, cx)),
                ),
        );
    }

    if entries.is_empty() {
        list = list.child(
            div()
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP))
                .text_size(px(theme::TEXT_META))
                .text_color(theme::text_dim())
                .child("Nothing matches."),
        );
    }

    list
}

/// The palette itself: a scrim, a box, an input and the list.
pub fn sheet<V: 'static>(
    input: impl IntoElement,
    entries: &[Entry],
    selected: usize,
    on_run: impl Fn(&mut V, usize, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        // A modal has to swallow input rather than merely cover it; see the
        // settings sheet, which learned the same thing.
        .occlude()
        .flex()
        .flex_col()
        .items_center()
        .bg(theme::scrim())
        // The offset is on a wrapper, not on the panel: `arrive` animates the
        // panel's own top margin, so one set there is overwritten on the first
        // frame and the box ends up against the top of the window.
        .child(
            div().pt(px(TOP)).child(crate::motion::arrive(
                "palette",
                RISE,
                div()
                    .w(px(WIDTH))
                    .flex()
                    .flex_col()
                    .rounded(px(theme::RADIUS_LG))
                    .bg(theme::surface_raised())
                    .border_1()
                    .border_color(theme::border())
                    .shadow_lg()
                    .child(div().p(px(theme::PANEL_PAD)).child(input))
                    .child(
                        div()
                            .pb(px(theme::GAP_TIGHT))
                            .child(rows(entries, selected, on_run, cx)),
                    ),
            )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, name: &str) -> LiveStream {
        LiveStream {
            user_login: login.into(),
            user_id: String::new(),
            display_name: name.into(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: 0,
            thumbnail_url: String::new(),
            started_at: String::new(),
        }
    }

    #[test]
    fn a_subsequence_matches_the_way_initials_do() {
        assert!(matches("QuickyBaby", "qb"));
        assert!(matches("QuickyBaby", "quick"));
        assert!(matches("QuickyBaby", "QUICKY"));
        assert!(!matches("QuickyBaby", "xyz"));
        assert!(!matches("QuickyBaby", "byq"), "order has to be kept");
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches("anything", ""));
        assert!(matches("", ""));
    }

    /// The palette is opened to go somewhere far more often than to do
    /// something, so a channel must never sit below a command.
    #[test]
    fn channels_come_before_commands() {
        let follows = [stream("forsen", "Forsen")];
        let entries = entries("", &follows, &[], &[], &[], true);

        let first_command = entries
            .iter()
            .position(|entry| entry.kind == "command")
            .expect("the commands should be there");
        let last_channel = entries
            .iter()
            .rposition(|entry| entry.kind == "watch")
            .expect("the channel should be there");
        assert!(last_channel < first_command);
    }

    /// "Add a pane" beside nothing is the same thing as "watch", and offering
    /// both doubles the list for no choice.
    #[test]
    fn adding_is_only_offered_when_something_is_already_open() {
        let follows = [stream("forsen", "Forsen")];

        let alone = entries("", &follows, &[], &[], &[], true);
        assert!(!alone.iter().any(|entry| entry.kind == "add"));

        let beside = entries("", &follows, &[], &[], &["quin69".into()], true);
        assert!(beside.iter().any(|entry| entry.kind == "add"));

        let full = entries("", &follows, &[], &[], &["quin69".into()], false);
        assert!(
            !full.iter().any(|entry| entry.kind == "add"),
            "a fifth pane cannot be added, so it should not be offered"
        );
    }

    /// A channel that is already open still opens — solo — but the row says
    /// which one you are looking at.
    #[test]
    fn an_open_channel_says_so() {
        let follows = [stream("forsen", "Forsen")];
        let entries = entries("", &follows, &[], &[], &["forsen".into()], true);
        assert_eq!(entries[0].kind, "watching");
        assert_eq!(entries[0].command, Command::Watch("forsen".into()));
    }

    #[test]
    fn every_open_pane_can_be_closed_by_name() {
        let watching: Vec<String> = vec!["forsen".into(), "quin69".into()];
        let entries = entries("close quin", &[], &[], &[], &watching, true);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, Command::Close(1));
    }

    /// Filtering by login matters as much as by display name: they differ for
    /// anyone whose name is not ASCII, and the login is what you type.
    #[test]
    fn a_channel_is_found_by_either_of_its_names() {
        let follows = [stream("kato_junichi0817", "加藤純一")];
        let by_login = entries("kato", &follows, &[], &[], &[], false);
        assert_eq!(by_login.iter().filter(|e| e.kind == "watch").count(), 1);
        let by_name = entries("加藤", &follows, &[], &[], &[], false);
        assert_eq!(by_name.iter().filter(|e| e.kind == "watch").count(), 1);
    }

    /// The offline list is the one that actually needs a filter, so it is in
    /// here — but only once something is typed, or an empty palette would be
    /// a hundred people who are not streaming above the commands.
    #[test]
    fn offline_follows_appear_only_for_a_typed_query_and_after_live_ones() {
        let follows = [stream("forsen", "Forsen")];
        let offline = [FollowedChannel {
            login: "fextralife".into(),
            user_id: "9".into(),
            display_name: "Fextralife".into(),
        }];

        let blank = entries("", &follows, &offline, &[], &[], false);
        assert!(!blank.iter().any(|entry| entry.kind == "offline"));

        let typed = entries("f", &follows, &offline, &[], &[], false);
        let live = typed
            .iter()
            .position(|entry| entry.kind == "watch")
            .unwrap();
        let off = typed
            .iter()
            .position(|entry| entry.kind == "offline")
            .unwrap();
        assert!(live < off, "an offline channel outranked a live one");
        assert_eq!(typed[off].command, Command::Watch("fextralife".into()));
    }

    /// Past broadcasts are offered for everyone, after the rows that open a
    /// channel now, and only once something is typed. An id the list already
    /// had rides along; an empty one is nothing rather than an empty string.
    #[test]
    fn past_broadcasts_are_offered_for_live_and_offline_alike_once_typed() {
        let follows = [stream("forsen", "Forsen")];
        let offline = [FollowedChannel {
            login: "fextralife".into(),
            user_id: "9".into(),
            display_name: "Fextralife".into(),
        }];

        let blank = entries("", &follows, &offline, &[], &[], false);
        assert!(!blank.iter().any(|entry| entry.kind == "videos"));

        let typed = entries("f", &follows, &offline, &[], &[], false);
        let videos: Vec<&Entry> = typed
            .iter()
            .filter(|entry| entry.kind == "videos")
            .collect();
        assert_eq!(videos.len(), 2);
        assert_eq!(
            videos[0].command,
            Command::Videos {
                login: "forsen".into(),
                display_name: "Forsen".into(),
                user_id: None,
            }
        );
        assert_eq!(
            videos[1].command,
            Command::Videos {
                login: "fextralife".into(),
                display_name: "Fextralife".into(),
                user_id: Some("9".into()),
            }
        );
        let last_now = typed
            .iter()
            .rposition(|entry| entry.kind == "watch" || entry.kind == "offline")
            .unwrap();
        let first_past = typed
            .iter()
            .position(|entry| entry.kind == "videos")
            .unwrap();
        assert!(last_now < first_past, "a recording outranked a channel");
    }

    /// With nothing typed the list leads with what was watched last, and a
    /// channel that is both recent and live is one row, not two.
    #[test]
    fn recently_watched_leads_an_empty_palette_once() {
        let follows = [stream("forsen", "Forsen"), stream("xqc", "xQc")];
        let recent: Vec<String> = vec!["xqc".into(), "quin69".into()];

        let blank = entries("", &follows, &[], &recent, &[], false);
        assert_eq!(blank[0].command, Command::Watch("xqc".into()));
        assert_eq!(blank[0].kind, "recent");
        assert_eq!(
            blank[0].title, "xQc",
            "named the way the live list names it"
        );
        assert_eq!(blank[1].command, Command::Watch("quin69".into()));
        assert_eq!(blank[1].kind, "recent");
        assert_eq!(
            blank
                .iter()
                .filter(|entry| entry.command == Command::Watch("xqc".into()))
                .count(),
            1,
            "a recent live channel was listed twice"
        );
        assert_eq!(blank[2].command, Command::Watch("forsen".into()));

        // Typed, the recents step aside: the lists and the typed name cover it.
        let typed = entries("q", &follows, &[], &recent, &[], false);
        assert!(!typed.iter().any(|entry| entry.kind == "recent"));
    }

    /// A channel nobody follows is still a channel: typed, it gets a row to
    /// open it and one for its recordings. While a follow still matches the
    /// typing it is a filter, and gets neither; nor does a thing that is not
    /// a login.
    #[test]
    fn a_channel_nobody_follows_can_be_opened_by_name() {
        let follows = [stream("forsen", "Forsen")];

        let unknown = entries("xqc", &follows, &[], &[], &[], false);
        assert_eq!(unknown[0].command, Command::Watch("xqc".into()));
        assert_eq!(unknown[0].kind, "channel");
        assert!(matches!(
            &unknown[1].command,
            Command::Videos { login, user_id: None, .. } if login == "xqc"
        ));

        let known = entries("forsen", &follows, &[], &[], &[], false);
        assert!(!known.iter().any(|entry| entry.kind == "channel"));
        let filtering = entries("f", &follows, &[], &[], &[], false);
        assert!(
            !filtering.iter().any(|entry| entry.kind == "channel"),
            "a filter keystroke offered to open a channel called that"
        );
        assert_eq!(
            known
                .iter()
                .filter(|entry| entry.command == Command::Watch("forsen".into()))
                .count(),
            1
        );

        let nonsense = entries("not a login", &follows, &[], &[], &[], false);
        assert!(!nonsense.iter().any(|entry| entry.kind == "channel"));
    }

    /// A pasted link to a recording opens it, from where the link points.
    #[test]
    fn a_recording_link_becomes_a_row() {
        let found = entries(
            "https://www.twitch.tv/videos/123?t=1m",
            &[],
            &[],
            &[],
            &[],
            false,
        );
        assert_eq!(
            found[0].command,
            Command::OpenVideo {
                id: "123".into(),
                start_secs: Some(60),
            }
        );
        assert_eq!(found[0].kind, "recording");
    }
}
