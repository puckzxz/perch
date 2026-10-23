//! The title bar's search box, as the root drives it: what it suggests as you
//! type, and the Twitch search it runs.
//!
//! The box answers twice. At once, from what Perch already knows: the
//! palette's own channels and recordings for what is typed
//! (`palette::suggestions`), hung under the box in the palette's rows, with
//! asking Twitch as the first of them. And from Twitch, on `Enter`, as it
//! always has, or by itself once the typing pauses ([`SEARCH_SETTLE`]), so a
//! name nobody follows needs no more than to be typed — but not by itself
//! on the watch page while something plays, since the results' page stops
//! it. A run of typing that asks Twitch more than once is one step on the
//! trail: a search already up is asked again in place
//! ([`search_settled`](RootView::search_settled)).
//!
//! The suggestions are up while the box has the keyboard, something is
//! typed in it and nothing has put them away — `Esc`, a pick, `Enter`. They
//! are drawn under the bar, in the root's content wrapper with the toasts,
//! never deferred: a deferred element escapes that wrapper's clip, which is
//! what keeps everything under the bar off its drag strip and caption
//! buttons, and in a short window anything anchored there would be lifted
//! over them. Where the box is comes from a probe in the bar
//! ([`search_probe`](RootView::search_probe)), one frame late at worst, and
//! the box seldom moves.

use std::time::Duration;

use gpui::{canvas, div, prelude::*, px, App, Context, Focusable, KeyDownEvent, Point, Window};

use super::{Page, RootView};
use crate::{palette, theme};

/// How long the typing has to pause before the box asks Twitch by itself:
/// long enough not to ask about every word on the way to the one meant,
/// short enough that stopping is asking.
const SEARCH_SETTLE: Duration = Duration::from_millis(600);

/// The fewest letters the box asks Twitch about by itself. One or two match
/// half of Twitch and say nothing; `Enter` asks about anything.
const SEARCH_SETTLE_MIN_LETTERS: usize = 3;

impl RootView {
    /// What the box suggests for what is typed in it now: asking Twitch,
    /// then the palette's channels and recordings for it. Worked out at
    /// render, as the palette's list is, from lists the root owns.
    fn search_suggestions(&self, cx: &App) -> Vec<palette::Entry> {
        palette::suggestions(
            self.search.read(cx).value().as_ref(),
            &self.follows,
            &self.offline,
            &self.settings.recent,
            &self.history.videos,
            &self.open_panes(cx),
            self.can_add(),
        )
    }

    /// Whether the suggestions are up: the box has the keyboard and
    /// something typed, nothing has put them away since, and no modal is up.
    fn suggesting(&self, window: &Window, cx: &App) -> bool {
        !self.search_dismissed
            && !self.modal_open()
            && self.search.focus_handle(cx).is_focused(window)
            && !self.search.read(cx).value().trim().is_empty()
    }

    /// The box's text changed: the suggestions up again, from the top, and
    /// Twitch asked once the typing pauses. A newer keystroke drops the
    /// older wait with its task.
    pub(super) fn search_typed(&mut self, cx: &mut Context<Self>) {
        self.search_selected = 0;
        self.search_dismissed = false;
        self.search_settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_SETTLE).await;
            this.update(cx, |this, cx| this.search_settled(cx)).ok();
        }));
        cx.notify();
    }

    /// The typing paused: ask Twitch for what the box says, unless it is
    /// too short to be worth it or its results are what the page shows
    /// already. Results up already are asked again in place, as a refresh
    /// asks (`ask_search`), so a run of typing is one step on the trail; the
    /// first ask is a step of its own, and brings the browse page up, as
    /// `Enter` does (`run_search`).
    ///
    /// Not on the watch page while something plays there: the browse page
    /// coming up stops it (`go_browse`), and a pause in the typing is not
    /// somebody asking for that. There the box suggests what Perch knows,
    /// and Twitch is asked on `Enter`, or a press on the row that says so.
    fn search_settled(&mut self, cx: &mut Context<Self>) {
        self.search_settle = None;
        let query = self.search.read(cx).value().trim().to_string();
        if query.chars().count() < SEARCH_SETTLE_MIN_LETTERS {
            return;
        }
        if self.page == Page::Watch && self.leaving_stops_panes() {
            return;
        }
        let shown = self.discovery.search.as_ref().map(|results| &results.query);
        if shown.is_some_and(|shown| shown.as_ref() == query) {
            return;
        }
        if shown.is_some() && self.page == Page::Browse {
            self.scrolls.search.set_offset(Point::default());
            self.ask_search(query, cx);
        } else {
            self.run_search(query, cx);
        }
    }

    /// `Enter` in the box: the suggestion chosen with the arrows, or the
    /// first, which asks Twitch. On an empty box, which suggests nothing,
    /// it takes the results away, as it always has (`run_search`).
    pub(super) fn search_entered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).value().trim().is_empty() {
            self.search_settle = None;
            self.run_search(String::new(), cx);
            return;
        }
        self.run_suggestion(self.search_selected, window, cx);
    }

    /// Run the suggestion at `index`. Asking Twitch keeps the box as it is,
    /// as `Enter` always has, with its suggestions put away until the next
    /// keystroke. Anything else is something Perch knows, opened as the
    /// palette opens it (`run_command`): the keys go back to the root, and
    /// the box empties unless the results for what it says are up — a query
    /// left in the box with nothing of its own on the page reads as though
    /// it were. Either way nothing is asked of Twitch after it.
    fn run_suggestion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.search_suggestions(cx);
        let Some(entry) = entries
            .get(index.min(entries.len().saturating_sub(1)))
            .cloned()
        else {
            return;
        };
        self.search_settle = None;
        self.search_dismissed = true;
        match entry.command {
            palette::Command::Search(query) => self.run_search(query, cx),
            command => {
                let typed = self.search.read(cx).value().trim().to_string();
                let shown = self
                    .discovery
                    .search
                    .as_ref()
                    .is_some_and(|results| results.query.as_ref() == typed);
                if !shown {
                    self.search
                        .update(cx, |state, cx| state.set_value("", window, cx));
                }
                self.focus.focus(window);
                self.run_command(command, window, cx);
            }
        }
        cx.notify();
    }

    /// Arrows and `Esc` while the suggestions are up, read on the way past
    /// as the palette's are (`on_palette_key`), since the box has the
    /// keyboard and binds them itself. `Esc` puts the suggestions away and
    /// leaves the box and what is typed in it.
    pub(super) fn on_search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.suggesting(window, cx) {
            return;
        }
        let count = self.search_suggestions(cx).len();
        match event.keystroke.key.as_str() {
            "up" | "down" if count > 0 => {
                let delta = if event.keystroke.key == "up" { -1 } else { 1 };
                let next = self.search_selected as isize + delta;
                self.search_selected = next.rem_euclid(count as isize) as usize;
                cx.notify();
            }
            "escape" => {
                self.search_dismissed = true;
                cx.notify();
            }
            _ => {}
        }
    }

    /// A probe over the box in the bar, noting where it is in the window —
    /// its left edge, which the suggestions hang from — and asking for one
    /// more frame when that moved, for the suggestions drawn before it in
    /// the frame to read it (`request_animation_frame`: a notify from a
    /// probe asks for no frame; see `HANDOFF.md`).
    pub(super) fn search_probe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        canvas(
            move |bounds, window, cx| {
                let left = f32::from(bounds.origin.x);
                let moved = owner
                    .update(cx, |this: &mut RootView, _cx| {
                        let moved = this.search_left != left;
                        this.search_left = left;
                        moved
                    })
                    .unwrap_or(false);
                if moved {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// The suggestions, under the box, while they are up: the palette's
    /// rows on the palette's raised panel, `theme::SEARCH_WIDTH` wide, the
    /// chosen one tinted — at first asking Twitch, which `Enter` runs. A
    /// click runs a row; a press anywhere else takes the keyboard from the
    /// box, which puts them away.
    pub(super) fn search_suggestions_panel(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !self.suggesting(window, cx) {
            return None;
        }
        let entries = self.search_suggestions(cx);
        let selected = self.search_selected.min(entries.len().saturating_sub(1));
        let rows = palette::rows(
            &entries,
            selected,
            |this: &mut RootView, index, window, cx| this.run_suggestion(index, window, cx),
            cx,
        );
        Some(
            div()
                .id("search-suggestions")
                .absolute()
                .top(px(theme::GAP_TIGHT))
                .left(px(self.search_left))
                .w(px(theme::SEARCH_WIDTH))
                // A press on it is not a press on the page under it, nor the
                // root's, which would take the keyboard from the box first.
                .occlude()
                .py(px(theme::GAP_TIGHT))
                .rounded(px(theme::RADIUS_LG))
                .bg(theme::surface_raised())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                .child(rows),
        )
    }
}
