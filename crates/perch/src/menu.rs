//! The one menu: the box a short list of things to do opens in, its rows,
//! the rule between its groups, a heading over rows that choose between
//! things, and how it arrives.
//!
//! Perch has four menus — the one about a channel (`channel_menu`), the
//! player's quality and speeds (`video_view::menu`), a pane's chat options
//! (`watch::chat_menu`) and a chat row's copy menu (`chat::copy_menu`). They
//! hang from different things and are answered by different views, and
//! until November 2026 each built its own box and its own row out of the
//! shared tokens. The tokens kept the colours and the type alike; nothing
//! kept the rest, so one cast a shadow and three did not, a rule between
//! groups was a row's border in two of them and an element of its own in a
//! third, and the box had a gap above its first row in one and none in the
//! others. Side by side on the watch page they read as two families.
//!
//! This is what they share: what a menu looks like, and how a row takes a
//! press. Where a menu hangs, what closes it and what a row does stay with
//! the menu that knows.
//!
//! **A row acts on the press, never on a click, and only on the first press
//! of a run.** A menu closes on a press outside whatever it hangs from, in
//! the capture phase, and its rows are outside that; gpui makes a click out
//! of a press and a release, and fires it from the frame current at the
//! release, which was drawn after the menu closed and has no row in it
//! (div.rs:3145-3189). The press is dispatched against the frame it landed
//! on, and its bubble phase, where a row hears it, comes after the capture
//! phase that closed the menu. So a row that waited for a click would draw,
//! highlight under the pointer and do nothing when chosen. [`row`] takes the
//! press for every menu, which is what holds them all to it, and a test
//! below reads their sources for a click. A later press of the same run is
//! never a row's: whatever the run's first press landed on chose, and the
//! row under the second may only be there because the first opened its menu.

use gpui::{
    div, prelude::*, px, Animation, App, Div, ElementId, MouseButton, MouseDownEvent, SharedString,
    Stateful, Window,
};

use crate::{controls, theme};

/// The box: a column of rows on the app's one raised surface
/// (`controls::raised`), at least [`theme::MENU_MIN_WIDTH`] wide, with a
/// word's gap above its first row and under its last, so a row's highlight
/// never touches the corner it is clipped to.
///
/// The caller places it, blocks the pointer with it (`occlude`: a menu is
/// over things that would hear a press on a row too), and says what a press
/// outside it does. Its leading is gpui's own, set here because a menu drawn
/// from inside a chat would otherwise take the chat's text size and leading
/// (`chat_display`); a menu is the app's, whichever view draws it.
pub fn surface() -> Div {
    div()
        .flex()
        .flex_col()
        .min_w(px(theme::MENU_MIN_WIDTH))
        .py(px(theme::GAP_WORD))
        .overflow_hidden()
        .map(controls::raised)
        .line_height(gpui::phi())
}

/// One row: its words, in the accent when it is the one `chosen`, and what
/// its press does. `label` is any element, so a row may breathe its words
/// (the quality being switched to); anything the caller adds after it goes
/// to the row's far end (the chat options' `On` and `Off`).
///
/// One line, however long: a menu hangs from something narrower than it, so
/// its width is whatever its rows say, and a row such as the quality menu's
/// first (`Auto (matches the video pane)`) would otherwise wrap at the
/// menu's least width. See the module for why `on_press` hears the press
/// and only the first of a run.
pub fn row(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    chosen: bool,
    on_press: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::GAP))
        .px(px(theme::PANEL_PAD))
        .py(px(theme::CONTROL_PAD_Y))
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if chosen {
            theme::accent()
        } else {
            theme::text()
        })
        .hover(|style| style.bg(theme::hover()))
        .child(label)
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            if event.click_count > 1 {
                return;
            }
            on_press(event, window, cx)
        })
}

/// The line between two groups of rows, with a word's gap either side.
pub fn rule() -> Div {
    div()
        .flex_none()
        .my(px(theme::GAP_WORD))
        .h(px(theme::HAIRLINE))
        .bg(theme::border())
}

/// Words over rows that choose between things, saying what they choose:
/// the chat options' `Text size`. Not a row: nothing happens on a press.
pub fn heading(words: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(theme::PANEL_PAD))
        .pt(px(theme::CONTROL_PAD_Y))
        .text_size(px(theme::TEXT_META))
        .text_color(theme::text_dim())
        .whitespace_nowrap()
        .child(words.into())
}

/// How a menu arrives: the one animation, for a caller's `with_animation`.
/// A menu is mounted only while it is open, so a one-shot is enough: there
/// is no closed state to animate back to. Every menu fades in on it; one
/// that opens out of a button also moves the last few pixels towards it
/// ([`theme::MENU_RISE`]), in whichever direction it hangs.
pub fn arrival() -> Animation {
    Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter())
}

#[cfg(test)]
mod tests {
    /// Every menu's own source, above its tests, and this one's.
    fn sources() -> Vec<(&'static str, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        [
            "menu.rs",
            "channel_menu.rs",
            "video_view/menu.rs",
            "watch/chat_menu.rs",
            "chat/copy_menu.rs",
        ]
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(dir.join(path))
                .unwrap_or_else(|error| panic!("{path} is unreadable: {error}"));
            let code = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests");
            // Spaces and line ends out, so formatting cannot hide a call.
            (path, code.split_whitespace().collect())
        })
        .collect()
    }

    /// A row that waited for a click would compile, draw, highlight under
    /// the pointer and do nothing when chosen: the menu closes on the press
    /// and takes the row with it before the release (see the module). No
    /// test without a window can press one, and gpui's test harness is a
    /// feature this build does not lock, so the rule is held against the
    /// sources instead: nothing in a menu listens for a click, the row here
    /// takes its presses through `on_mouse_down`, and every menu draws its
    /// rows through it rather than building one of its own.
    #[test]
    fn menu_rows_act_on_the_press() {
        for (path, code) in sources() {
            assert!(
                !code.contains(".on_click("),
                "{path}: something in a menu waits for a click, which never comes once the press has closed the menu"
            );
            if path == "menu.rs" {
                assert!(
                    code.contains(".on_mouse_down(MouseButton::Left,"),
                    "menu::row no longer acts on the press"
                );
            } else {
                assert!(
                    code.contains("menu::row("),
                    "{path} draws its rows without menu::row"
                );
                assert!(
                    !code.contains(".on_mouse_down(MouseButton::Left,"),
                    "{path} takes a row's press itself, past menu::row's rule"
                );
            }
        }
    }

    /// One box for every menu: none builds a surface of its own, so none
    /// can come out without its shadow or its gap again.
    #[test]
    fn every_menu_opens_in_the_one_box() {
        for (path, code) in sources() {
            if path == "menu.rs" {
                continue;
            }
            assert!(
                code.contains("menu::surface()"),
                "{path} does not open in menu::surface"
            );
            assert!(
                !code.contains("theme::surface_raised()"),
                "{path} paints a raised surface of its own"
            );
        }
    }
}
