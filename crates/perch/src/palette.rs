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
use twitch_api::LiveStream;

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
fn matches(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = haystack.chars().flat_map(char::to_lowercase);
    needle
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| chars.any(|c| c == wanted))
}

/// Everything the palette can offer right now, filtered by `query`.
///
/// Channels first and commands after, because the overwhelmingly common reason
/// to open this is to go somewhere rather than to do something — and within the
/// channels, the order the follows list is already in.
pub fn entries(
    query: &str,
    follows: &[LiveStream],
    watching: &[String],
    can_add: bool,
) -> Vec<Entry> {
    let mut entries = Vec::new();

    for stream in follows {
        let open = watching.contains(&stream.user_login);
        if !matches(&stream.display_name, query) && !matches(&stream.user_login, query) {
            continue;
        }
        entries.push(Entry {
            command: Command::Watch(stream.user_login.clone()),
            title: SharedString::from(stream.display_name.clone()),
            kind: if open {
                "watching".into()
            } else {
                "watch".into()
            },
        });
        // Only worth offering when it is different from the row above it.
        if can_add && !open && !watching.is_empty() {
            entries.push(Entry {
                command: Command::Add(stream.user_login.clone()),
                title: SharedString::from(format!("{} — add a pane", stream.display_name)),
                kind: "add".into(),
            });
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
        let entries = entries("", &follows, &[], true);

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

        let alone = entries("", &follows, &[], true);
        assert!(!alone.iter().any(|entry| entry.kind == "add"));

        let beside = entries("", &follows, &["quin69".into()], true);
        assert!(beside.iter().any(|entry| entry.kind == "add"));

        let full = entries("", &follows, &["quin69".into()], false);
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
        let entries = entries("", &follows, &["forsen".into()], true);
        assert_eq!(entries[0].kind, "watching");
        assert_eq!(entries[0].command, Command::Watch("forsen".into()));
    }

    #[test]
    fn every_open_pane_can_be_closed_by_name() {
        let watching: Vec<String> = vec!["forsen".into(), "quin69".into()];
        let entries = entries("close quin", &[], &watching, true);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, Command::Close(1));
    }

    /// Filtering by login matters as much as by display name: they differ for
    /// anyone whose name is not ASCII, and the login is what you type.
    #[test]
    fn a_channel_is_found_by_either_of_its_names() {
        let follows = [stream("kato_junichi0817", "加藤純一")];
        assert_eq!(entries("kato", &follows, &[], false).len(), 1);
        assert_eq!(entries("加藤", &follows, &[], false).len(), 1);
    }
}
