//! The menus the player's control bar opens: which one is open, the box it
//! opens in, and its rows.
//!
//! Hand-rolled rather than gpui-component's `DropdownMenu`, which serves only
//! that library's own `Button`. One is open at a time, all of them open from
//! the bar's right-hand cluster (`bar::button_row`), and `Esc` closes whichever
//! it is, through `RootView::on_go_browse`. The rows act on the press, never
//! on a click; `menu_row` says why, and a test below holds this file to it.

use gpui::{
    div, prelude::*, px, Animation, AnimationExt, Context, Div, ElementId, MouseButton,
    SharedString, Stateful, Window,
};

use super::{VideoEvent, VideoView};
use crate::theme;

/// A menu the control bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The renditions this stream offers, under the settings' choice.
    Quality,
}

impl Menu {
    /// What the menu's rise is keyed on, so a menu opened in place of another
    /// rises as itself rather than arriving as the finished animation of the
    /// one it replaced.
    fn id(self) -> &'static str {
        match self {
            Menu::Quality => "quality-menu",
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
    /// Open `which` over the bar, for the palette's keyboard path to it. An
    /// open menu holds the bar up (`sync_controls`), so the bar comes up with
    /// it, wherever the pointer is.
    ///
    /// Nothing on a compact tile, or on a player still waiting for its first
    /// frame: neither draws a bar to hang a menu from. A menu opened there
    /// would be invisible, would take the next `Esc` for itself with nothing
    /// on screen changing, could not be dismissed by a press elsewhere, since
    /// the anchor that hears it is not drawn, and would flip the bar's fade
    /// while the bar is not mounted, the replay `motion::Fade::apply` warns
    /// of.
    pub fn open_menu(&mut self, which: Menu, cx: &mut Context<Self>) {
        if self.compact || !self.has_picture() || self.menu == Some(which) {
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

    /// The box `which` opens in, hung from the right-hand cluster's right
    /// edge and rising above the bar.
    pub(super) fn menu_box(&self, which: Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = match which {
            Menu::Quality => self.quality_rows(cx),
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
            // Choosing what is already chosen changes nothing, and asking
            // anyway would restart the stream to arrive where it was.
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
            cx.listener(move |this, _event, window, cx| {
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
