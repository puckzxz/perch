//! The menus the player's control bar opens: which one is open, the box it
//! opens in, and its rows.
//!
//! Three of them: the quality; More — what else there is to do with the
//! pane, which also carries the quality and the maximize control when the
//! bar is too narrow for them; and a recording's speeds, which open in
//! More's place from its speed row. Hand-rolled rather than gpui-component's `DropdownMenu`, which
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
    canvas, div, prelude::*, px, Animation, AnimationExt, AnyElement, Context, DispatchPhase, Div,
    ElementId, MouseButton, MouseDownEvent, SharedString, Stateful, Window,
};

use super::{bar, Switching, VideoEvent, VideoView};
use crate::motion;
use crate::seek_bar;
use crate::speed::Speed;
use crate::stage::Place;
use crate::target;
use crate::theme;
use crate::watch::PaneAction;

/// A menu the control bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The renditions this stream offers, under the settings' choice.
    Quality,
    /// The rest: hear this pane alone or every pane again, a recording's
    /// speed, pop the pane out, open it on twitch.tv, copy its link — and the
    /// quality and the maximize control, while the bar has no room for them.
    More,
    /// A recording's speeds (`crate::speed`), opened in More's place from
    /// its speed row, or from the speed on the seek row.
    Speed,
}

impl Menu {
    /// What the menu's rise is keyed on, so a menu opened in place of another
    /// rises as itself rather than arriving as the finished animation of the
    /// one it replaced.
    fn id(self) -> &'static str {
        match self {
            Menu::Quality => "quality-menu",
            Menu::More => "more-menu",
            Menu::Speed => "speed-menu",
        }
    }
}

/// Whether going from `old` to `new` opens the quality menu (`Some(true)`),
/// closes it (`Some(false)`), or leaves it as it was. More opening in its
/// place closes it, and it opening in More's place opens it.
pub(super) fn quality_menu_flip(old: Option<Menu>, new: Option<Menu>) -> Option<bool> {
    let (was, is) = (old == Some(Menu::Quality), new == Some(Menu::Quality));
    (was != is).then_some(is)
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
    /// More's quality and speed rows, which open their menus in More's
    /// place. (The seek row's speed tag toggles the speed menu, as a menu's
    /// own button does, through [`toggle_menu`](Self::toggle_menu).) An
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
        self.set_menu(Some(which), cx);
        self.sync_controls();
        cx.notify();
    }

    /// What a menu's own button does.
    pub(super) fn toggle_menu(&mut self, which: Menu, cx: &mut Context<Self>) {
        self.set_menu(toggled(self.menu, which), cx);
        self.sync_controls();
        cx.notify();
    }

    /// Close whichever menu is open, if one is. Returns whether one was, so
    /// `Esc` can take back the menu before it takes you off the page.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu.is_none() {
            return false;
        }
        self.set_menu(None, cx);
        self.sync_controls();
        cx.notify();
        true
    }

    /// The one write of which menu is open, a test below holds this file
    /// and the view to it: it tells the root when the quality menu opens or
    /// closes (`VideoEvent::QualityMenu`, [`quality_menu_flip`]), whichever
    /// way that happened, so streamlink started ahead for it is never left
    /// running behind a menu that has gone.
    pub(super) fn set_menu(&mut self, menu: Option<Menu>, cx: &mut Context<Self>) {
        let flip = quality_menu_flip(self.menu, menu);
        self.menu = menu;
        if let Some(open) = flip {
            cx.emit(VideoEvent::QualityMenu(open));
        }
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
            Menu::Speed => self.speed_rows(cx),
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
    ///
    /// The row marked is the one chosen ([`chosen`]): while a pick is under
    /// way, the row it was made from, breathing, rather than what still
    /// plays — the menu opened again mid-switch says what was asked for.
    fn quality_rows(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let switching = self.switching.as_ref();
        let marked = chosen(self.qualities.picked, &self.qualities.playing, switching);
        let waiting = switching.is_some();
        let mut rows = vec![quality_option(
            "quality-default",
            self.qualities.default.clone(),
            Mark::of(marked == Chosen::Default, waiting),
            None,
            cx,
        )
        .border_b_1()
        .border_color(theme::border())];

        for (index, name) in self.qualities.available.iter().enumerate() {
            let selected = marked == Chosen::Rendition(name.as_str());
            rows.push(quality_option(
                ("quality-option", index),
                SharedString::from(name.clone()),
                Mark::of(selected, waiting),
                Some(name.clone()),
                cx,
            ));
        }
        rows
    }

    /// The speed menu: every speed a recording can play at, slowest first,
    /// the one it plays at marked as the quality menu marks its choice. A
    /// press plays the pane at it from the next frame (`set_speed`); the
    /// one already chosen changes nothing.
    fn speed_rows(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let current = self.stream.speed();
        Speed::CHOICES
            .into_iter()
            .enumerate()
            .map(|(index, speed)| {
                menu_row(
                    ("speed-option", index),
                    speed.label().into(),
                    speed == current,
                    move |this, _window, cx| this.set_speed(speed, cx),
                    cx,
                )
            })
            .collect()
    }

    /// More: what the bar has folded away first — the quality while its
    /// pill has, then the maximize control once it has too — ruled off from
    /// the rest; then, with two panes or more, `Only this one` or `Hear all
    /// again`; then a recording's `Playback speed · 1x`, which opens the
    /// speeds in its place; then `Pop out`, then the pane's way out to
    /// twitch.tv and its link. All but the quality and the speed are the
    /// root's to do — it knows the panes, its windows, and the clipboard and
    /// the browser are the app's — so they go up as `VideoEvent::Pane`.
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
                folded_quality(&self.qualities.playing, self.switching.as_ref()),
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
        // Hearing this pane alone, and back: `Only this one` hushes every
        // other pane the way Mute all does, and once one pane alone is heard
        // every pane's row says `Hear all again` instead (`HearOnly`, a
        // mirror the root keeps). Nothing with one pane. The root works out
        // which a press means from the panes as they are, so a row drawn a
        // frame stale cannot do the opposite of what is heard.
        if let Some((id, words)) = self.hear_only.row() {
            rows.push(menu_row(
                id,
                words.into(),
                false,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::HearOnly)),
                cx,
            ));
        }
        // A recording's speed, saying what it is, which opens the speeds in
        // this menu's place, as the folded quality row opens the qualities.
        // Nothing on a live stream, which plays at the speed it is made.
        if self.stream.timeline().is_some() {
            rows.push(menu_row(
                "more-speed",
                speed_row(self.stream.speed()),
                false,
                |this, _window, cx| this.open_menu(Menu::Speed, cx),
                cx,
            ));
        }
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

/// Which row of a pane's quality menu is marked as the one chosen; see
/// [`chosen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Chosen<'a> {
    /// The settings' choice, the first row.
    Default,
    /// The rendition of this name.
    Rendition(&'a str),
}

/// The row a pane's quality menu marks, with `playing` on screen, `picked`
/// from the menu or not, and a pick `switching` the pane, if one is under
/// way: the row that pick was made from, from the moment it is made;
/// otherwise the rendition playing if it was picked, or the settings' row if
/// not.
pub(super) fn chosen<'a>(
    picked: bool,
    playing: &'a str,
    switching: Option<&'a Switching>,
) -> Chosen<'a> {
    match switching {
        Some(switching) if switching.default => Chosen::Default,
        Some(switching) => Chosen::Rendition(&switching.to),
        None if picked => Chosen::Rendition(playing),
        None => Chosen::Default,
    }
}

/// More's quality row, while the pill is folded into it: what plays, or
/// what a pick under way is switching to.
pub(super) fn folded_quality(
    playing: &SharedString,
    switching: Option<&Switching>,
) -> SharedString {
    match switching {
        Some(switching) => format!("Quality · switching to {}", switching.to).into(),
        None => format!("Quality · {playing}").into(),
    }
}

/// More's speed row: the speed a recording plays at, in the words the speed
/// menu and the seek row write it in.
pub(super) fn speed_row(speed: Speed) -> SharedString {
    format!("Playback speed · {}", speed.label()).into()
}

/// How a row of the quality menu is marked: not at all, as the one chosen,
/// or as the one chosen and still being switched to, which breathes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    None,
    Chosen,
    Switching,
}

impl Mark {
    /// A row `selected` or not, in a menu whose pane is `switching` or not.
    fn of(selected: bool, switching: bool) -> Self {
        match (selected, switching) {
            (false, _) => Mark::None,
            (true, false) => Mark::Chosen,
            (true, true) => Mark::Switching,
        }
    }
}

/// One row of the quality menu. `request` is what choosing it asks for: a
/// rendition, or `None` for the settings' choice.
fn quality_option(
    id: impl Into<ElementId>,
    label: SharedString,
    mark: Mark,
    request: Option<String>,
    cx: &mut Context<VideoView>,
) -> Stateful<Div> {
    let selected = mark != Mark::None;
    // The row being switched to breathes its words, as the pill does; the
    // row itself, and its highlight under the pointer, stay still.
    let label = if mark == Mark::Switching {
        motion::waiting("quality-option-switching", div().child(label)).into_any_element()
    } else {
        label.into_any_element()
    };
    row_of(
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
    row_of(id, label.into_any_element(), selected, on_press, cx)
}

/// [`menu_row`], with its words as any element: the quality menu's row being
/// switched to breathes them (`quality_option`).
fn row_of(
    id: impl Into<ElementId>,
    label: AnyElement,
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
    use std::time::Instant;

    use super::*;

    /// A pick switching a pane to `to`, from its rendition's row or, with
    /// `default`, from the settings' row.
    fn switching(to: &str, default: bool) -> Switching {
        Switching {
            to: SharedString::from(to.to_string()),
            default,
            since: Instant::now(),
        }
    }

    /// At rest the menu marks the rendition playing if it was picked, and
    /// the settings' row if not.
    #[test]
    fn the_menu_marks_what_plays_at_rest() {
        assert_eq!(chosen(true, "720p60", None), Chosen::Rendition("720p60"));
        assert_eq!(chosen(false, "720p60", None), Chosen::Default);
    }

    /// While a pick is under way the menu marks the row it was made from,
    /// whatever still plays and whether that was picked.
    #[test]
    fn the_menu_marks_a_pick_from_the_press() {
        let rendition = switching("480p30", false);
        let default = switching("720p60", true);
        for picked in [false, true] {
            assert_eq!(
                chosen(picked, "720p60", Some(&rendition)),
                Chosen::Rendition("480p30")
            );
            assert_eq!(
                chosen(picked, "1080p60", Some(&default)),
                Chosen::Default,
                "the settings' row, even when they pick a rendition by name"
            );
        }
    }

    /// More's folded quality row says what plays, or what a pick under way
    /// is switching to.
    #[test]
    fn mores_quality_row_says_a_switch() {
        let playing = SharedString::from("720p60");
        assert_eq!(folded_quality(&playing, None).as_ref(), "Quality · 720p60");
        assert_eq!(
            folded_quality(&playing, Some(&switching("480p30", false))).as_ref(),
            "Quality · switching to 480p30"
        );
    }

    /// More's speed row says the speed the recording plays at, in the
    /// speed menu's own words.
    #[test]
    fn mores_speed_row_says_the_speed() {
        assert_eq!(speed_row(Speed::NORMAL).as_ref(), "Playback speed · 1x");
        assert_eq!(
            speed_row(Speed::from_hundredths(175)).as_ref(),
            "Playback speed · 1.75x"
        );
    }

    /// Only the chosen row of a menu whose pane is switching breathes.
    #[test]
    fn only_the_row_being_switched_to_breathes() {
        assert_eq!(Mark::of(false, false), Mark::None);
        assert_eq!(Mark::of(false, true), Mark::None);
        assert_eq!(Mark::of(true, false), Mark::Chosen);
        assert_eq!(Mark::of(true, true), Mark::Switching);
    }

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

    /// The quality menu opening or closing is told, from either side of
    /// More, and nothing else is.
    #[test]
    fn only_the_quality_menu_coming_or_going_is_told() {
        assert_eq!(quality_menu_flip(None, Some(Menu::Quality)), Some(true));
        assert_eq!(
            quality_menu_flip(Some(Menu::More), Some(Menu::Quality)),
            Some(true),
            "More's quality row opens it in More's place"
        );
        assert_eq!(quality_menu_flip(Some(Menu::Quality), None), Some(false));
        assert_eq!(
            quality_menu_flip(Some(Menu::Quality), Some(Menu::More)),
            Some(false)
        );
        assert_eq!(quality_menu_flip(None, Some(Menu::More)), None);
        assert_eq!(quality_menu_flip(Some(Menu::More), None), None);
        assert_eq!(
            quality_menu_flip(Some(Menu::Quality), Some(Menu::Quality)),
            None
        );
        assert_eq!(quality_menu_flip(None, None), None);
    }

    /// Every write of the open menu goes through `set_menu`, so none can
    /// close the quality menu without the root hearing it and leave the
    /// streamlink it started ahead running for nobody. Read from the view's
    /// own sources, the facade and every file beside it.
    #[test]
    fn every_change_of_the_open_menu_goes_through_set_menu() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sources: Vec<_> = std::fs::read_dir(dir.join("video_view"))
            .expect("the view's sources are missing")
            .map(|entry| entry.expect("an unreadable source").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        sources.push(dir.join("video_view.rs"));
        let mut writes = 0;
        for path in sources {
            let source = std::fs::read_to_string(&path).expect("an unreadable source");
            let code = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests");
            let code: String = code.split_whitespace().collect();
            writes += code
                .match_indices("self.menu")
                .filter(|&(at, field)| {
                    let after = &code[at + field.len()..];
                    (after.starts_with('=') && !after.starts_with("=="))
                        || [".take(", ".replace(", ".insert(", ".as_mut("]
                            .iter()
                            .any(|mutator| after.starts_with(mutator))
                })
                .count();
        }
        assert_eq!(writes, 1, "set_menu is the one write of the open menu");
    }
}
