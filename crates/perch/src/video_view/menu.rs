//! The menus the player's control bar opens: which one is open, the box it
//! opens in, and its rows.
//!
//! Two of them: the quality, from its pill or from the icon the pill gives
//! way to on a narrow bar; and a recording's speeds, from the speed on its
//! seek row. Hand-rolled rather than gpui-component's `DropdownMenu`, which
//! serves only that library's own `Button`. One is open at a time, both
//! hang from the bar's right-hand cluster (`bar::pane_cluster`), and `Esc`
//! closes whichever it is, through `RootView::on_go_browse`. The rows act on
//! the press, never on a click; `menu_row` says why, and a test below holds
//! this file to it. The rest of a double-click whose first press a row took
//! is swallowed whole, by `run_guard`, so it cannot land on whatever the
//! closed menu left under the pointer.
//!
//! There was a third, More, until November 2026: the compressor, a
//! recording's speed, the pop-out, the pin, the way to twitch.tv and the
//! link. Four of those were on the pane's own menu as well, the ⋯ in its
//! header a few hundred pixels away, so More went and the rest moved: the
//! compressor to that menu (`crate::channel_menu`), the speed to the seek
//! row for good (`VideoView::speed_tag`), and the quality, which More
//! carried on a narrow bar, to an icon of its own. The bar is for playing
//! the stream; what else can be done with it is one menu.

use gpui::{
    canvas, div, prelude::*, px, Animation, AnimationExt, AnyElement, Context, DispatchPhase, Div,
    ElementId, MouseButton, MouseDownEvent, SharedString, Stateful, Window,
};

use super::{Switching, VideoEvent, VideoView};
use crate::motion;
use crate::speed::Speed;
use crate::stage::Place;
use crate::theme;

/// A menu the control bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The renditions this stream offers, under the settings' choice.
    Quality,
    /// A recording's speeds (`crate::speed`), from the speed on its seek
    /// row.
    Speed,
}

impl Menu {
    /// What the menu's rise is keyed on, so a menu opened in place of another
    /// rises as itself rather than arriving as the finished animation of the
    /// one it replaced.
    fn id(self) -> &'static str {
        match self {
            Menu::Quality => "quality-menu",
            Menu::Speed => "speed-menu",
        }
    }
}

/// Whether going from `old` to `new` opens the quality menu (`Some(true)`),
/// closes it (`Some(false)`), or leaves it as it was. The speeds opening in
/// its place close it, and it opening in theirs opens it.
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
    /// Open `which` over the bar: the palette's keyboard path to the
    /// quality. (A menu's own button — the pill, the icon in its place, the
    /// seek row's speed — toggles it, through
    /// [`toggle_menu`](Self::toggle_menu).) An open menu holds the bar up
    /// (`sync_controls`), so the bar comes up with it, wherever the pointer
    /// is.
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

    /// The rest of a run of presses whose first one a row took: an element
    /// for the player to hold, a `canvas` that listens at the window as it
    /// paints, the way `RootView::side_buttons` does.
    ///
    /// A row acts on the press and takes the menu away with it, but the
    /// platform counts a second press near the first as a double-click
    /// whatever is under it by then (gpui's windows/window.rs:1049-1060).
    /// Under a row that was the picture, whose double-click is fullscreen,
    /// or a recording's seek track, which seeks. So a row marks its run
    /// (`row_run`),
    /// and this stops every later press of that run in the capture phase,
    /// before anything under the pointer hears it. The bubble phase, where a
    /// click begins, never comes, so the release makes no click either. The
    /// next run's first press ends it.
    ///
    /// Window-level because nothing narrower hears all of those: the bar and
    /// the menu each block the pointer from what is under them. Held ahead
    /// of the bar, so this listens before anything on the bar or in a menu
    /// does, the anchor's dismiss included.
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
/// out from under the pointer, as a pane menu's `Pop out` does
/// (`RootView::run_guard`).
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

/// What the quality's icon says under the pointer, where the bar is too
/// narrow for its pill: what plays, or what a pick under way is switching
/// to. The pill says it in its own words; an icon has to be asked.
pub(super) fn folded_quality(
    playing: &SharedString,
    switching: Option<&Switching>,
) -> SharedString {
    match switching {
        Some(switching) => format!("Quality · switching to {}", switching.to).into(),
        None => format!("Quality · {playing}").into(),
    }
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

    /// The quality's icon says what plays, or what a pick under way is
    /// switching to.
    #[test]
    fn the_folded_quality_says_a_switch() {
        let playing = SharedString::from("720p60");
        assert_eq!(folded_quality(&playing, None).as_ref(), "Quality · 720p60");
        assert_eq!(
            folded_quality(&playing, Some(&switching("480p30", false))).as_ref(),
            "Quality · switching to 480p30"
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

    /// One menu at a time: the seek row's speed over an open quality menu
    /// swaps one for the other rather than stacking them, and the other way
    /// round.
    #[test]
    fn opening_one_menu_closes_the_other() {
        assert_eq!(toggled(Some(Menu::Quality), Menu::Speed), Some(Menu::Speed));
        assert_eq!(
            toggled(Some(Menu::Speed), Menu::Quality),
            Some(Menu::Quality)
        );
        assert_eq!(toggled(Some(Menu::Speed), Menu::Speed), None);
    }

    /// A double- or triple-click on a row: the row took the first press, so
    /// the rest go nowhere — not to the picture's fullscreen, nor the seek
    /// track.
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
    /// the speeds, and nothing else is.
    #[test]
    fn only_the_quality_menu_coming_or_going_is_told() {
        assert_eq!(quality_menu_flip(None, Some(Menu::Quality)), Some(true));
        assert_eq!(
            quality_menu_flip(Some(Menu::Speed), Some(Menu::Quality)),
            Some(true),
            "the pill pressed over the open speeds opens it in their place"
        );
        assert_eq!(quality_menu_flip(Some(Menu::Quality), None), Some(false));
        assert_eq!(
            quality_menu_flip(Some(Menu::Quality), Some(Menu::Speed)),
            Some(false)
        );
        assert_eq!(quality_menu_flip(None, Some(Menu::Speed)), None);
        assert_eq!(quality_menu_flip(Some(Menu::Speed), None), None);
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
