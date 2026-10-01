//! The command palette.
//!
//! While watching, the player's controls are keys or a hover-revealed
//! overlay and the page-level ones sit in the title bar; everything you could
//! reach while *browsing* was a click — the picker had no keyboard path at all
//! past the search box. This is the one control that answers both: a channel
//! to open, a recording to carry on with, a pane to close, a page to go to,
//! typed rather than aimed at.
//!
//! It is not a second search box. The search box asks *Twitch* a question and
//! costs a request; this filters what the app already knows — who is live, what
//! is open, what has been watched, what it can do — and costs nothing, which is
//! why it can run on every keystroke.

use gpui::{div, prelude::*, px, Context, SharedString};
use settings::history::Watched;
use twitch_api::{Channel, LiveStream};

use crate::browse::Tab;
use crate::seek_bar;
use crate::target::{self, Target};
use crate::theme;

/// How many rows the list shows at once.
///
/// A palette that fills the window is a page, and a page is the thing this
/// exists to avoid going to.
const MAX_ROWS: usize = 8;

/// The panel's width, how far down the page it sits — measured from under the
/// title bar, because the palette lives in the root's content area — and how
/// far it drops in as it opens.
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
    /// Play a recording from the history, where it was left.
    Resume(String),
    /// Show one of the browse page's tabs, from wherever the app is.
    ShowTab(Tab),
    GoWatch,
    StopAll,
    ToggleSidebar,
    ToggleSettings,
    Refresh,
}

/// A pane that is open, as the palette sees it.
#[derive(Debug, Clone)]
pub struct OpenPane {
    /// The channel a live pane plays, which is what "already watching" is
    /// matched against. `None` for a recording: a replay of a channel is not
    /// watching it.
    pub login: Option<String>,
    /// What a row about the pane calls it: the channel's name as it writes
    /// it, with the kind of recording after it for one — "(replay)".
    pub title: String,
}

/// A live pane known only by its login, which is all the tests need.
#[cfg(test)]
impl From<&str> for OpenPane {
    fn from(login: &str) -> Self {
        Self {
            login: Some(login.to_string()),
            title: login.to_string(),
        }
    }
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

/// The shortest query a recording's title is searched for. Titles are long,
/// and matched like names — a letter at a time — two letters would find
/// nearly all of them; so a title has to contain what was typed, whole, and
/// what was typed has to be long enough to mean something.
const TITLE_QUERY_MIN: usize = 3;

/// A row for a recording in the history: whose, and what, and where it was
/// left — which is what tells apart a channel that titles every broadcast
/// the same.
fn resume_row(watched: &Watched) -> Entry {
    let name = if watched.channel_name.is_empty() {
        &watched.channel_login
    } else {
        &watched.channel_name
    };
    Entry {
        command: Command::Resume(watched.id.clone()),
        title: SharedString::from(format!("{name} — {}", watched.title)),
        kind: if watched.finished {
            "watched".into()
        } else {
            format!(
                "resume at {}",
                seek_bar::timecode(watched.position_secs as f64)
            )
            .into()
        },
    }
}

/// Whether a history entry answers to `query`: its channel, by the same
/// few-letters match as every name here, or its title, containing the query
/// whole — see [`TITLE_QUERY_MIN`].
fn watched_matches(watched: &Watched, query: &str) -> bool {
    if matches(&watched.channel_name, query) || matches(&watched.channel_login, query) {
        return true;
    }
    query.chars().count() >= TITLE_QUERY_MIN
        && watched.title.to_lowercase().contains(&query.to_lowercase())
}

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
///
/// The recordings in `history` are offered once something is typed, after
/// the channels that answer to it: picking one up is a name and Enter away.
/// With nothing typed, one of them leads the whole list — the newest, if it
/// is part-watched and its channel leads the recents, which is what it looks
/// like when the last thing opened was that recording. That makes the most
/// likely thing to want, carrying on with it, the row Enter runs.
#[allow(clippy::too_many_arguments)]
pub fn entries(
    query: &str,
    follows: &[LiveStream],
    offline: &[Channel],
    recent: &[String],
    history: &[Watched],
    watching: &[OpenPane],
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
        let open = watching
            .iter()
            .any(|pane| pane.login.as_deref() == Some(login));
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
        let last = history.first().filter(|watched| {
            !watched.finished
                && recent
                    .first()
                    .is_some_and(|login| *login == settings::channel_key(&watched.channel_login))
        });
        entries.extend(last.map(resume_row));
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
        // An offline channel opens its page, the way its name does on the
        // Following tab. It used to open a pane that could only say the
        // channel was offline, which is what the row already said.
        for channel in offline {
            if !matches(&channel.display_name, query) && !matches(&channel.login, query) {
                continue;
            }
            matched_follow = true;
            entries.push(Entry {
                command: Command::Videos {
                    login: channel.login.clone(),
                    display_name: channel.display_name.clone(),
                    user_id: Some(channel.user_id.clone()).filter(|id| !id.is_empty()),
                },
                title: SharedString::from(channel.display_name.clone()),
                kind: "offline".into(),
            });
        }

        // What was typed, taken at its word: a channel nobody here follows,
        // or a link to a recording. Only once no follow matches, because
        // until then the typing is a filter over the lists and a row
        // offering to open `f` under everyone whose name has an f in it is
        // noise; a link answers to nobody's name and is always offered.
        // The channel's rows go with the other rows that open a channel now,
        // and its past broadcasts with everyone else's, below.
        let typed = target::parse(query);
        let typed_channel = match &typed {
            Some(Target::Channel(login)) if !matched_follow => Some(login.clone()),
            _ => None,
        };
        if let Some(login) = &typed_channel {
            offer(&mut entries, login, format!("Open {login}"), "channel");
        }

        // What has been watched, newest first: after the rows that open a
        // channel now, and before the ones that list a channel's past — a
        // recording already begun is the more particular answer.
        entries.extend(
            history
                .iter()
                .filter(|watched| watched_matches(watched, query))
                .map(resume_row),
        );

        // A live channel's past broadcasts, after the rows that open it now —
        // its own row watches it. An offline channel's row is already its
        // page. Only once something is typed, for the same reason the offline
        // rows wait: with nothing typed this would be a second row for every
        // channel above the commands.
        for stream in follows {
            if !matches(&stream.display_name, query) && !matches(&stream.user_login, query) {
                continue;
            }
            entries.push(Entry {
                command: Command::Videos {
                    login: stream.user_login.clone(),
                    display_name: stream.display_name.clone(),
                    user_id: Some(stream.user_id.clone()).filter(|id| !id.is_empty()),
                },
                title: SharedString::from(format!("{} — past broadcasts", stream.display_name)),
                kind: "videos".into(),
            });
        }

        if let Some(login) = typed_channel {
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
        if let Some(Target::Video { id, start_secs }) = typed {
            entries.push(Entry {
                command: Command::OpenVideo {
                    id: id.clone(),
                    start_secs,
                },
                title: SharedString::from(format!("Open recording {id}")),
                kind: "recording".into(),
            });
        }
    }

    for (index, pane) in watching.iter().enumerate() {
        let title = format!("Close {}", pane.title);
        if matches(&title, query) {
            entries.push(Entry {
                command: Command::Close(index),
                title: SharedString::from(title),
                kind: "pane".into(),
            });
        }
    }

    // One row per tab, each named for the tab it opens. There used to be one
    // "Go to follows", which went to whichever tab had been left open — the
    // history, as often as not — and said otherwise.
    let tabs = Tab::ALL.map(|tab| (Command::ShowTab(tab), format!("Go to {}", tab.label())));
    let playing = !watching.is_empty();
    let commands = tabs.into_iter().chain(
        [
            // Only with something to go back to, or to stop: a command that
            // does nothing is a row in the way of one that would.
            (playing, Command::GoWatch, "Back to watching"),
            (playing, Command::StopAll, "Stop all streams"),
            (true, Command::ToggleSidebar, "Toggle the follows rail"),
            (true, Command::Refresh, "Refresh this list"),
            (true, Command::ToggleSettings, "Settings"),
        ]
        .into_iter()
        .filter(|(offered, _, _)| *offered)
        .map(|(_, command, title)| (command, title.to_string())),
    );
    for (command, title) in commands {
        if matches(&title, query) {
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
                // The row Enter runs keeps its tint under the pointer, or
                // resting the mouse on the list hides which row that is.
                .when(index == selected, |row| row.bg(theme::accent_dim()))
                .hover(move |style| {
                    style.bg(if index == selected {
                        theme::accent_dim()
                    } else {
                        theme::hover()
                    })
                })
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
        // frame and the box ends up against the top of the page.
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
        let entries = entries("", &follows, &[], &[], &[], &[], true);

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

        let alone = entries("", &follows, &[], &[], &[], &[], true);
        assert!(!alone.iter().any(|entry| entry.kind == "add"));

        let beside = entries("", &follows, &[], &[], &[], &["quin69".into()], true);
        assert!(beside.iter().any(|entry| entry.kind == "add"));

        let full = entries("", &follows, &[], &[], &[], &["quin69".into()], false);
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
        let entries = entries("", &follows, &[], &[], &[], &["forsen".into()], true);
        assert_eq!(entries[0].kind, "watching");
        assert_eq!(entries[0].command, Command::Watch("forsen".into()));
    }

    #[test]
    fn every_open_pane_can_be_closed_by_name() {
        let watching: Vec<OpenPane> = vec!["forsen".into(), "quin69".into()];
        let entries = entries("close quin", &[], &[], &[], &[], &watching, true);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, Command::Close(1));
    }

    /// Filtering by login matters as much as by display name: they differ for
    /// anyone whose name is not ASCII, and the login is what you type.
    #[test]
    fn a_channel_is_found_by_either_of_its_names() {
        let follows = [stream("kato_junichi0817", "加藤純一")];
        let by_login = entries("kato", &follows, &[], &[], &[], &[], false);
        assert_eq!(by_login.iter().filter(|e| e.kind == "watch").count(), 1);
        let by_name = entries("加藤", &follows, &[], &[], &[], &[], false);
        assert_eq!(by_name.iter().filter(|e| e.kind == "watch").count(), 1);
    }

    /// The offline list is the one that actually needs a filter, so it is in
    /// here — but only once something is typed, or an empty palette would be
    /// a hundred people who are not streaming above the commands.
    #[test]
    fn offline_follows_appear_only_for_a_typed_query_and_after_live_ones() {
        let follows = [stream("forsen", "Forsen")];
        let offline = [Channel {
            login: "fextralife".into(),
            user_id: "9".into(),
            display_name: "Fextralife".into(),
        }];

        let blank = entries("", &follows, &offline, &[], &[], &[], false);
        assert!(!blank.iter().any(|entry| entry.kind == "offline"));

        let typed = entries("f", &follows, &offline, &[], &[], &[], false);
        let live = typed
            .iter()
            .position(|entry| entry.kind == "watch")
            .unwrap();
        let off = typed
            .iter()
            .position(|entry| entry.kind == "offline")
            .unwrap();
        assert!(live < off, "an offline channel outranked a live one");
        // Its page, as on the Following tab — not a pane saying it is off.
        assert_eq!(
            typed[off].command,
            Command::Videos {
                login: "fextralife".into(),
                display_name: "Fextralife".into(),
                user_id: Some("9".into()),
            }
        );
    }

    /// A live channel's past broadcasts are a row of their own, after the
    /// rows that open a channel now, and only once something is typed. An
    /// offline channel's row already is its page, so it gets no second one.
    /// An id the list already had rides along; an empty one is nothing rather
    /// than an empty string.
    #[test]
    fn past_broadcasts_are_offered_for_live_channels_once_typed() {
        let follows = [stream("forsen", "Forsen")];
        let offline = [Channel {
            login: "fextralife".into(),
            user_id: "9".into(),
            display_name: "Fextralife".into(),
        }];

        let blank = entries("", &follows, &offline, &[], &[], &[], false);
        assert!(!blank.iter().any(|entry| entry.kind == "videos"));

        let typed = entries("f", &follows, &offline, &[], &[], &[], false);
        let videos: Vec<&Entry> = typed
            .iter()
            .filter(|entry| entry.kind == "videos")
            .collect();
        assert_eq!(videos.len(), 1, "an offline channel got a second row");
        assert_eq!(
            videos[0].command,
            Command::Videos {
                login: "forsen".into(),
                display_name: "Forsen".into(),
                user_id: None,
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

        let blank = entries("", &follows, &[], &recent, &[], &[], false);
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
        let typed = entries("q", &follows, &[], &recent, &[], &[], false);
        assert!(!typed.iter().any(|entry| entry.kind == "recent"));
    }

    /// A channel nobody follows is still a channel: typed, it gets a row to
    /// open it and one for its recordings. While a follow still matches the
    /// typing it is a filter, and gets neither; nor does a thing that is not
    /// a login.
    #[test]
    fn a_channel_nobody_follows_can_be_opened_by_name() {
        let follows = [stream("forsen", "Forsen")];

        let unknown = entries("xqc", &follows, &[], &[], &[], &[], false);
        assert_eq!(unknown[0].command, Command::Watch("xqc".into()));
        assert_eq!(unknown[0].kind, "channel");
        assert!(matches!(
            &unknown[1].command,
            Command::Videos { login, user_id: None, .. } if login == "xqc"
        ));

        let known = entries("forsen", &follows, &[], &[], &[], &[], false);
        assert!(!known.iter().any(|entry| entry.kind == "channel"));
        let filtering = entries("f", &follows, &[], &[], &[], &[], false);
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

        let nonsense = entries("not a login", &follows, &[], &[], &[], &[], false);
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

    fn watched(id: &str, login: &str, title: &str, position_secs: u64) -> Watched {
        Watched {
            id: id.into(),
            channel_login: login.into(),
            channel_name: login.to_uppercase(),
            title: title.into(),
            length_secs: 36_000,
            position_secs,
            ..Default::default()
        }
    }

    /// Opened last and part-watched: with nothing typed it is the row Enter
    /// runs. Finished, or not the last thing opened, it waits to be typed.
    #[test]
    fn the_recording_last_opened_leads_an_empty_palette() {
        let follows = [stream("forsen", "Forsen")];
        let history = [
            watched("2", "xqc", "WICKED", 4_000),
            watched("1", "forsen", "Games and shit!", 100),
        ];

        let recent: Vec<String> = vec!["xqc".into(), "forsen".into()];
        let blank = entries("", &follows, &[], &recent, &history, &[], false);
        assert_eq!(blank[0].command, Command::Resume("2".into()));
        assert_eq!(blank[0].title, "XQC — WICKED");
        assert_eq!(blank[0].kind, "resume at 1:06:40");
        assert_eq!(blank[1].command, Command::Watch("xqc".into()));
        assert_eq!(
            blank
                .iter()
                .filter(|entry| matches!(entry.command, Command::Resume(_)))
                .count(),
            1,
            "one recording leads; the rest wait to be typed"
        );

        let live_since: Vec<String> = vec!["forsen".into(), "xqc".into()];
        let blank = entries("", &follows, &[], &live_since, &history, &[], false);
        assert_eq!(
            blank[0].command,
            Command::Watch("forsen".into()),
            "something else was opened since"
        );

        let finished = [Watched {
            finished: true,
            ..watched("2", "xqc", "WICKED", 36_000)
        }];
        let blank = entries("", &follows, &[], &recent, &finished, &[], false);
        assert!(
            !blank
                .iter()
                .any(|entry| matches!(entry.command, Command::Resume(_))),
            "a finished recording is not carried on with"
        );
    }

    /// Typed, a recording answers to its channel the way every name does, or
    /// to a piece of its title — whole, and long enough to mean something,
    /// since nearly any two letters appear in order in a title somewhere.
    /// It comes after the rows that open a channel now and before the one
    /// that lists the channel's past.
    #[test]
    fn a_recording_is_found_by_its_channel_or_a_piece_of_its_title() {
        let follows = [stream("xqc", "xQc")];
        let history = [
            watched("2", "xqc", "NOPIXEL DRAMA", 4_000),
            Watched {
                finished: true,
                ..watched("1", "forsen", "Games and shit!", 36_000)
            },
        ];

        let by_channel = entries("xqc", &follows, &[], &[], &history, &[], false);
        let kinds: Vec<&str> = by_channel.iter().map(|entry| entry.kind.as_ref()).collect();
        assert_eq!(kinds[..3], ["watch", "resume at 1:06:40", "videos"]);

        // A word that is also a login: the channel it names comes first, as a
        // channel always does, then the recording.
        let by_title = entries("games", &[], &[], &[], &history, &[], false);
        let kinds: Vec<&str> = by_title.iter().map(|entry| entry.kind.as_ref()).collect();
        assert_eq!(kinds[..3], ["channel", "watched", "videos"]);
        assert_eq!(by_title[1].command, Command::Resume("1".into()));

        let scattered = entries("gs", &[], &[], &[], &history, &[], false);
        assert!(
            !scattered
                .iter()
                .any(|entry| matches!(entry.command, Command::Resume(_))),
            "two letters in order are not a title"
        );
    }

    #[test]
    fn the_history_is_a_command_away() {
        let found = entries("history", &[], &[], &[], &[], &[], false);
        assert!(found
            .iter()
            .any(|entry| entry.command == Command::ShowTab(Tab::History)));
    }

    /// Every tab has a row that goes to it, named for it. The one row there
    /// used to be said "follows" and went wherever the page had been left.
    #[test]
    fn every_tab_is_a_command_away_by_its_own_name() {
        for tab in Tab::ALL {
            let found = entries(tab.label(), &[], &[], &[], &[], &[], false);
            let row = found
                .iter()
                .find(|entry| entry.command == Command::ShowTab(tab))
                .unwrap_or_else(|| panic!("no row goes to {}", tab.label()));
            assert!(row.title.contains(tab.label()));
        }
    }

    /// Going back to watching, or stopping everything, with nothing playing
    /// would do nothing, so neither is offered until something is.
    #[test]
    fn commands_about_what_is_playing_wait_for_something_to_play() {
        let idle = entries("", &[], &[], &[], &[], &[], false);
        for command in [Command::GoWatch, Command::StopAll] {
            assert!(
                !idle.iter().any(|entry| entry.command == command),
                "{command:?} offered with nothing playing"
            );
        }

        let playing = entries("", &[], &[], &[], &[], &["forsen".into()], true);
        for command in [Command::GoWatch, Command::StopAll] {
            assert!(
                playing.iter().any(|entry| entry.command == command),
                "{command:?} missing while something plays"
            );
        }
    }
}
