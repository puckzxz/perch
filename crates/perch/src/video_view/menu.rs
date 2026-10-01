//! The menus the player's control bar opens: which one is open, the box it
//! opens in, and its rows.
//!
//! Two of them: the quality, and More — what else there is to do with the
//! pane, which also carries the quality and the maximize control when the
//! bar is too narrow for them. Hand-rolled rather than gpui-component's `DropdownMenu`, which
//! serves only that library's own `Button`. One is open at a time, all of
//! them open from the bar's right-hand cluster (`bar::button_row`), and `Esc`
//! closes whichever it is, through `RootView::on_go_browse`. The rows act on
//! the press, never on a click; `menu_row` says why, and a test below holds
//! this file to it. The rest of a double-click whose first press a row took
//! is swallowed whole, by `run_guard`, so it cannot land on whatever the
//! closed menu left under the pointer — or, after `Pop out`, which takes the
//! player and that guard with it out of the window, by the root's
//! (`RootView::run_guard`).

use gpui::{
    canvas, div, prelude::*, px, Animation, AnimationExt, Context, DispatchPhase, Div, ElementId,
    MouseButton, MouseDownEvent, SharedString, Stateful, Window,
};

use super::{bar, VideoEvent, VideoView};
use crate::seek_bar;
use crate::stage::Place;
use crate::target;
use crate::theme;
use crate::watch::PaneAction;

/// A menu the control bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The renditions this stream offers, under the settings' choice.
    Quality,
    /// The rest: pop the pane out, open it on twitch.tv, copy its link —
    /// and the quality and the maximize control, while the bar has no room
    /// for them.
    More,
}

impl Menu {
    /// What the menu's rise is keyed on, so a menu opened in place of another
    /// rises as itself rather than arriving as the finished animation of the
    /// one it replaced.
    fn id(self) -> &'static str {
        match self {
            Menu::Quality => "quality-menu",
            Menu::More => "more-menu",
        }
    }
}

/// What pressing `which`'s button leaves open: the same menu again closes it,
/// and any other menu opens in place of whatever was open.
pub(super) fn toggled(open: Option<Menu>, which: Menu) -> Option<Menu> {
    if open == Some(which) {
        None
    } else {
        Some(which)
    }
}

impl VideoView {
    /// Open `which` over the bar: the palette's keyboard path to it, and
    /// More's quality row, which opens the quality menu in More's place. An
    /// open menu holds the bar up (`sync_controls`), so the bar comes up with
    /// it, wherever the pointer is.
    ///
    /// Nothing anywhere but a pane, or on a player still waiting for its
    /// first frame: a tile and a starting player draw no bar to hang a menu
    /// from, and a pop-out's bar has no menu button and no anchor. A menu
    /// opened there would be invisible, would take the next `Esc` for itself
    /// with nothing on screen changing, could not be dismissed by a press
    /// elsewhere, since the anchor that hears it is not drawn, and would flip
    /// the bar's fade while the bar is not mounted, the replay
    /// `motion::Fade::apply` warns of.
    pub fn open_menu(&mut self, which: Menu, cx: &mut Context<Self>) {
        if self.place != Place::Pane || !self.has_picture() || self.menu == Some(which) {
            return;
        }
        self.menu = Some(which);
        self.sync_controls();
        cx.notify();
    }

    /// What a menu's own button does.
    pub(super) fn toggle_menu(&mut self, which: Menu, cx: &mut Context<Self>) {
        self.menu = toggled(self.menu, which);
        self.sync_controls();
        cx.notify();
    }

    /// Close whichever menu is open, if one is. Returns whether one was, so
    /// `Esc` can take back the menu before it takes you off the page.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu.take().is_none() {
            return false;
        }
        self.sync_controls();
        cx.notify();
        true
    }

    /// Whether one of the bar's menus is open. The pane's header over the
    /// picture steps aside for it (`watch::Slot::point`): the two would hang
    /// over the same picture, and the menu is what was asked for last.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The box `which` opens in, hung from the right-hand cluster's right
    /// edge and rising above the bar.
    pub(super) fn menu_box(&self, which: Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = match which {
            Menu::Quality => self.quality_rows(cx),
            Menu::More => self.more_rows(cx),
        };

        div()
            .absolute()
            .right_0()
            .flex()
            .flex_col()
            .min_w(px(theme::MENU_MIN_WIDTH))
            .rounded(px(theme::RADIUS_LG))
            .overflow_hidden()
            .bg(theme::surface_raised())
            .border_1()
            .border_color(theme::border())
            // The box rises above the bar, out from under the bar's own
            // occluder, so it blocks the pointer itself: without this a press
            // on a row would also reach the picture underneath, where a
            // double-click is fullscreen.
            .occlude()
            // Which hides the press from the root's own mouse-down, so the
            // keys come back from here by hand; see `return_keys`.
            .capture_any_mouse_down(cx.listener(Self::return_keys))
            .children(rows)
            // Rises the last few pixels into place, so it reads as coming out
            // of the button rather than being stamped over the video. It is
            // mounted only while open, which is what makes a plain one-shot
            // enough: there is no closed state to animate back to.
            .with_animation(
                ElementId::from(which.id()),
                Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
                |menu, delta| {
                    menu.opacity(delta)
                        .bottom(px(theme::MENU_BOTTOM - theme::MENU_RISE * (1.0 - delta)))
                },
            )
    }

    /// The quality menu: the settings' choice first, ruled off from the
    /// renditions. Picking a rendition holds it for as long as the pane is
    /// open, and until this row existed nothing handed the pane back short of
    /// closing it.
    fn quality_rows(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let picked = self.qualities.picked;
        let mut rows = vec![quality_option(
            "quality-default",
            self.qualities.default.clone(),
            !picked,
            None,
            cx,
        )
        .border_b_1()
        .border_color(theme::border())];

        for (index, name) in self.qualities.available.iter().enumerate() {
            let selected = picked && name.as_str() == self.qualities.playing.as_ref();
            rows.push(quality_option(
                ("quality-option", index),
                SharedString::from(name.clone()),
                selected,
                Some(name.clone()),
                cx,
            ));
        }
        rows
    }

    /// More: what the bar has folded away first — the quality while its
    /// pill has, then the maximize control once it has too — ruled off from
    /// the rest; then `Pop out`, then the pane's way out to twitch.tv and its
    /// link. All but the quality are the root's to do — it knows the panes,
    /// its windows, and the clipboard and the browser are the app's — so they
    /// go up as `VideoEvent::Pane`.
    ///
    /// A recording opens and copies at the moment it is at, and says so:
    /// `Copy link at 1:02:03`, from its first whole second on, by the rule
    /// the link itself is written by (`target::moment`). Read as the menu is
    /// drawn, which a playing picture does with every frame, so the time on
    /// the row is the time a press copies.
    fn more_rows(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let mut folded = Vec::new();
        if !self.fit.quality {
            folded.push(menu_row(
                "more-quality",
                format!("Quality · {}", self.qualities.playing).into(),
                false,
                // In place of this menu, as if from the pill.
                |this, _window, cx| this.open_menu(Menu::Quality, cx),
                cx,
            ));
        }
        // The bar's maximize control, in the words it has there, while the
        // bar has no room for it (`bar::fit` folds it after the pill).
        if let Some(control) = bar::maximize_control(self.maximize).filter(|_| !self.fit.maximize) {
            folded.push(menu_row(
                control.row_id,
                control.words.into(),
                false,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::Maximize)),
                cx,
            ));
        }
        let ruled = folded.len();
        let mut rows: Vec<Stateful<Div>> = folded
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                row.when(index + 1 == ruled, |row| {
                    row.border_b_1().border_color(theme::border())
                })
            })
            .collect();
        // The pane's picture into a window of its own, where the pop-out is
        // offered. Only ever on a pane: no other place draws this menu
        // (`open_menu`), and a popped pane's bar has Bring back instead. The
        // player leaves the main window on this press, and `run_guard` with
        // it, so the root guards the rest of the run (`RootView::run_guard`).
        if crate::root::pop_out_offered() {
            rows.push(menu_row(
                "more-pop-out",
                "Pop out".into(),
                false,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::PopOut)),
                cx,
            ));
        }
        rows.push(menu_row(
            "more-open",
            "Open on twitch.tv".into(),
            false,
            |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::OpenOnTwitch)),
            cx,
        ));
        let moment = self
            .stream
            .timeline()
            .and_then(|_| target::moment(self.stream.position()));
        let copy = match moment {
            Some(secs) => format!("Copy link at {}", seek_bar::timecode(secs as f64)),
            None => "Copy link".to_string(),
        };
        rows.push(menu_row(
            "more-copy",
            copy.into(),
            false,
            |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::CopyLink)),
            cx,
        ));
        rows
    }

    /// The rest of a run of presses whose first one a row took: an element
    /// for the player to hold, a `canvas` that listens at the window as it
    /// paints, the way `RootView::side_buttons` does.
    ///
    /// A row acts on the press and takes the menu away with it, but the
    /// platform counts a second press near the first as a double-click
    /// whatever is under it by then (gpui's windows/window.rs:1049-1060).
    /// Under a row that was the picture, whose double-click is fullscreen; a
    /// recording's seek track, which seeks; or, after More's quality row, the
    /// quality menu that opened in More's place, where the press would pick a
    /// rendition and change the stream. So a row marks its run (`row_run`),
    /// and this stops every later press of that run in the capture phase,
    /// before anything under the pointer hears it. The bubble phase, where a
    /// click begins, never comes, so the release makes no click either. The
    /// next run's first press ends it.
    ///
    /// Window-level because nothing narrower hears all of those: the bar and
    /// the menu each block the pointer from what is under them. Held ahead
    /// of the bar, so this listens before anything on the bar or in a menu
    /// does, the anchor's dismiss included, and a quality menu just opened
    /// from More stays open for a press of its own.
    pub(super) fn run_guard(cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let swallowed = owner
                        .update(cx, |this, _| {
                            this.row_run = rest_of_row_run(this.row_run, event.click_count);
                            this.row_run
                        })
                        .unwrap_or(false);
                    if swallowed {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

/// Whether a press `click_count` deep into its run is the rest of a run a
/// row took, given whether the run so far was a row's. The first press of a
/// run never is, and ends whatever run went before, so this is also what
/// `row_run` reads once the press has been heard.
///
/// The root's guard asks the same of a run whose first press took a player
/// out from under the pointer, `Pop out` on one of these rows included, out
/// of the reach of the player's own guard (`RootView::run_guard`).
pub(crate) fn rest_of_row_run(row_run: bool, click_count: usize) -> bool {
    row_run && click_count > 1
}

/// One row of the quality menu. `request` is what choosing it asks for: a
/// rendition, or `None` for the settings' choice.
fn quality_option(
    id: impl Into<ElementId>,
    label: SharedString,
    selected: bool,
    request: Option<String>,
    cx: &mut Context<VideoView>,
) -> Stateful<Div> {
    menu_row(
        id,
        label,
        selected,
        move |_this, _window, cx| {
            // Choosing what is already chosen changes nothing. Asking anyway
            // could still change the stream: the settings' choice, asked for
            // again, is made for the pane's size now, which a pane that has
            // shrunk does not play (`RootView::request_quality`).
            if !selected {
                cx.emit(VideoEvent::QualityRequested(request.clone()));
            }
        },
        cx,
    )
}

/// One row of a menu: closes the menu and runs `on_press`, on the press.
///
/// Not on a click, because the click would never come. The anchor's
/// `on_mouse_down_out` asks whether a press landed outside the anchor's own
/// bounds (gpui's div.rs:226-236), and a menu hangs above its anchor, outside
/// them, so a press on a row closes the menu in the capture phase. gpui makes
/// a click out of a press and a release, and fires it from the listeners of
/// the frame current at the release (div.rs:2213-2245): drawn after the menu
/// closed, with no row left in it. The press itself is dispatched against the
/// frame it landed on, and its bubble phase, where this runs, comes after the
/// capture phase that closed the menu, while the row is still there to hear
/// it. What acting on the press costs is drag-off-to-cancel, which a short
/// list of rows can do without.
///
/// Only on the first press of a run, which is also when it marks the run as
/// a row's, for [`VideoView::run_guard`] to swallow the rest of. A later
/// press of a run is never a row's to take: whatever the run's first press
/// landed on chose, and the row under the second may only be there because
/// the first opened its menu.
fn menu_row(
    id: impl Into<ElementId>,
    label: SharedString,
    selected: bool,
    on_press: impl Fn(&mut VideoView, &mut Window, &mut Context<VideoView>) + 'static,
    cx: &mut Context<VideoView>,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .px(px(theme::PANEL_PAD))
        .py(px(theme::CONTROL_PAD_Y))
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        // One line, however long: the menu hangs from a button narrower than
        // it, so its width is whatever its rows say, and a row such as the
        // quality menu's first, which says the settings' choice in the
        // sheet's words (`Auto (matches the video pane)`), would otherwise
        // wrap at the menu's least width.
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if selected {
            theme::accent()
        } else {
            theme::text()
        })
        .hover(|style| style.bg(theme::hover()))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                if event.click_count > 1 {
                    return;
                }
                this.row_run = true;
                this.close_menu(cx);
                on_press(this, window, cx);
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_the_open_menu_closes_it() {
        assert_eq!(toggled(Some(Menu::Quality), Menu::Quality), None);
    }

    #[test]
    fn a_closed_menu_opens_on_its_button() {
        assert_eq!(toggled(None, Menu::Quality), Some(Menu::Quality));
    }

    /// One menu at a time: More's button over an open quality menu swaps
    /// one for the other rather than stacking them, and the other way round.
    #[test]
    fn opening_one_menu_closes_the_other() {
        assert_eq!(toggled(Some(Menu::Quality), Menu::More), Some(Menu::More));
        assert_eq!(
            toggled(Some(Menu::More), Menu::Quality),
            Some(Menu::Quality)
        );
        assert_eq!(toggled(Some(Menu::More), Menu::More), None);
    }

    /// A double- or triple-click on a row: the row took the first press, so
    /// the rest go nowhere — not to the picture's fullscreen, the seek track
    /// or the quality menu More's row opened under the pointer.
    #[test]
    fn the_rest_of_a_rows_run_is_swallowed() {
        assert!(rest_of_row_run(true, 2));
        assert!(rest_of_row_run(true, 3));
    }

    /// The next run's first press is heard as usual and ends the row's run,
    /// so a double-click after it is a double-click again. A run that never
    /// started on a row is never touched.
    #[test]
    fn a_new_run_is_nobody_elses() {
        assert!(!rest_of_row_run(true, 1));
        assert!(!rest_of_row_run(false, 1));
        assert!(!rest_of_row_run(false, 2));
    }

    /// A row that waited for a click would compile, draw, highlight under the
    /// pointer and do nothing when chosen: the menu closes on the press and
    /// takes the row with it before the release (see `menu_row`). No test
    /// without a window can press one, and gpui's test harness is a feature
    /// this build does not lock, so the rule is held here against the file's
    /// own source instead. Everything above the tests must take its presses
    /// through `on_mouse_down`, and nothing in it may listen for a click.
    #[test]
    fn menu_rows_act_on_the_press() {
        let source = include_str!("menu.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has code above its tests");
        // Spaces and line ends out, so formatting cannot hide a call.
        let code: String = code.split_whitespace().collect();
        assert!(
            !code.contains(".on_click("),
            "something in a menu waits for a click, which never comes once the press has closed the menu"
        );
        assert!(
            code.contains(".on_mouse_down(MouseButton::Left,"),
            "menu_row no longer acts on the press"
        );
    }
}
