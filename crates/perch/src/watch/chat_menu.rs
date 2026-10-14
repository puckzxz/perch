//! A pane's chat options menu: how big every chat's text is, whether every
//! message carries its time, and putting the chat and video sizes back to
//! where they are derived, which is otherwise `Ctrl+0` alone. Opened from
//! the icon in the pane's
//! header (`header::pane_header`), on a pane with a chat panel only, and
//! hung over the top of that pane's chat, under the header's right-hand
//! cluster, the way the bar's menus hang over the picture from theirs
//! (`video_view::menu`). A chat box shorter than the menu — a stacked
//! pane's, with the divider dragged down — has it risen from the box's
//! foot instead, over the header and the picture ([`placed`]), so its last
//! row — `Reset pane sizes`, the one press that puts a chat dragged small
//! back — is never the one cut off.
//!
//! What it sets is every chat's, not the pane's: it is about the reader's
//! eyes and the screen, which are the same in every pane, and so are the
//! sizes the last row puts back. The root keeps which pane's menu is open,
//! one at a time across them all (`RootView::chat_menu`, [`toggled`]), and
//! what the rows set (`RootView::set_chat_display`, `RootView::reset_sizes`);
//! this file only draws it.
//!
//! Dismissed as the bar's menus are. The icon sits in an anchor that closes
//! the menu on a press anywhere outside it (`PaneAction::CloseChatMenu`), so
//! a press on the icon itself is not "elsewhere" and toggles instead. The
//! menu hangs outside the anchor, so a press on one of its rows closes it
//! in the capture phase too — and where a risen menu covers the anchor, the
//! row's own answer closes it, as every row's does
//! (`RootView::on_pane_action`). The rows act on the press for the reason
//! `video_view::menu::menu_row` gives; a test below holds this file to it.
//! The rest of a double-click whose first press a row took is swallowed by
//! the root's guard (`RootView::run_guard`), so it cannot land on a link in
//! the chat the closed menu leaves under the pointer. `Esc` closes it too
//! (`RootView::on_go_browse`).

use gpui::{
    div, prelude::*, px, Animation, AnimationExt, AnyElement, Context, Div, ElementId, MouseButton,
    MouseDownEvent, SharedString, Window,
};
use settings::ChatTextSize;

use super::{pane_id, PaneAction};
use crate::chat_display::{self, ChatDisplay};
use crate::theme;

/// Which pane's chat options menu is open once the icon on the pane `key`
/// names is pressed, with the one `open` open, if one is: that pane's own
/// again closes it, and any other pane's opens in its place, so there is
/// never more than one.
pub fn toggled(open: Option<&str>, key: &str) -> Option<String> {
    if open == Some(key) {
        None
    } else {
        Some(key.to_string())
    }
}

/// The menu for the pane `key` names, marking what `display` has chosen:
/// the four text sizes under a heading, ruled off from the times, and
/// `Reset pane sizes` ruled off at the foot. A layer
/// for the pane's chat box (`watch::pane`), right-aligned with the header's
/// cluster and hung a word's gap under the header's rule where the box has
/// the room, risen from the box's foot where it has not ([`placed`]), and
/// dropping the last few pixels into place as it opens.
pub(super) fn menu<V: 'static>(
    key: &str,
    display: ChatDisplay,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let focus_key = key.to_string();
    let mut rows: Vec<AnyElement> = vec![heading("Text size")];
    for (index, size) in ChatTextSize::ALL.into_iter().enumerate() {
        rows.push(
            row(
                pane_id(key, &format!("chat-size-{index}")),
                chat_display::size_label(size).into(),
                display.size == size,
                PaneAction::SetChatDisplay(ChatDisplay { size, ..display }),
                key,
                on_pane.clone(),
                cx,
            )
            .into_any_element(),
        );
    }
    rows.push(
        row(
            pane_id(key, "chat-times"),
            "Time on every message".into(),
            display.times,
            PaneAction::SetChatDisplay(ChatDisplay {
                times: !display.times,
                ..display
            }),
            key,
            on_pane.clone(),
            cx,
        )
        // Its state in words as well as the accent: one row alone has no
        // neighbour to tell its colour against, as each size has three.
        .child(
            div()
                .flex_none()
                .text_size(px(theme::TEXT_META))
                .text_color(theme::text_dim())
                .child(toggle_state(display.times)),
        )
        .border_t_1()
        .border_color(theme::border())
        .into_any_element(),
    );
    // The divider beside chat and the one between a stacked pane's video
    // and chat are dragged; this puts both back. Never chosen, so never in
    // the accent; ruled off, since it is about the page, not the text.
    rows.push(
        row(
            pane_id(key, "reset-sizes"),
            "Reset pane sizes".into(),
            false,
            PaneAction::ResetSizes,
            key,
            on_pane.clone(),
            cx,
        )
        .border_t_1()
        .border_color(theme::border())
        .into_any_element(),
    );

    let menu = div()
        .flex()
        .flex_col()
        // Half of where it goes; see `placed`.
        .flex_shrink_0()
        .mb_auto()
        .min_w(px(theme::MENU_MIN_WIDTH))
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface_raised())
        .border_1()
        .border_color(theme::border())
        // Over chat, whose links and rows would otherwise hear the press on
        // a row too.
        .occlude()
        // Which hides the press from the root's own mouse-down, so the keys
        // come back from here, as the bar's menus hand them back
        // (`VideoView::return_keys`): a press on the heading or the rule,
        // which is no row's, takes them out of the title bar's search box
        // too. The press is on the pane, so it activates it as a press
        // anywhere else in it does, and that is what gives the keys back.
        .capture_any_mouse_down(cx.listener(move |view, _: &MouseDownEvent, window, cx| {
            on_pane(view, &focus_key, PaneAction::Activate, window, cx)
        }))
        .children(rows)
        // Mounted only while open, so a one-shot is enough, as the bar's
        // menus are; it drops rather than rises because it hangs below what
        // opened it. An offset from wherever it is laid out, so it drops
        // the same few pixels into either place `placed` gives it.
        .with_animation(
            pane_id(key, "chat-menu"),
            Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
            |menu, delta| {
                menu.opacity(delta)
                    .top(px(-theme::MENU_RISE * (1.0 - delta)))
            },
        );
    placed(menu)
}

/// Where the menu sits in its pane's chat box: in a column over the box,
/// laid out from its foot (`justify_end`), with the menu's own margin under
/// it taking whatever room the box has to spare (`mb_auto`). In a box with
/// the room, the margin takes all of it and the menu hangs at the top, a
/// word's gap under the header's rule. In a box shorter than the menu there
/// is no room for the margin to take, and a column laid out from its foot
/// overflows at its top, not at its foot: the menu rises over the header
/// and the picture, and its last row stays in the box. That is CSS's
/// flexbox, which taffy follows (`distribute_remaining_free_space`): an
/// auto margin takes free space only while there is some, and an
/// end-aligned line that does not fit overflows at its start. The menu
/// never shrinks to fit instead (`flex_shrink_0`), which its
/// `overflow_hidden` would otherwise let it do, cutting off its last rows.
/// The column has no listeners, so it inserts no hitbox and takes nothing
/// from the chat beside and under the menu.
///
/// Hung from the top by an inset, as it was, the menu ran on past a short
/// box's foot: on a stacked pane with the divider dragged down, under the
/// next pane, which is drawn after it, or past the window's foot, where the
/// page's clip cut it off — `Reset pane sizes` first. Not drawn deferred
/// and anchored, as the copy menu and the rail's card are: deferred, it
/// would escape that clip, which is what keeps everything under the title
/// bar off the bar's caption areas (`RootView::render`), and in a short
/// enough window, lifted to fit, it would have reached over them. Risen
/// from its own box it stays inside the clip, and it is drawn after the
/// picture and the header it covers, which come before chat in the cell.
/// Only a cell shorter than the menu has it rise further: over the pane
/// above, drawn before it, or into the page's clip at the top, where its
/// heading goes before its last row does.
fn placed(menu: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_end()
        .justify_end()
        .py(px(theme::GAP_WORD))
        .pr(px(theme::ROW_PAD_X))
        .child(menu)
}

/// What the times row says beside its words: whether every message carries
/// its time now, in words, not only in the row's colour.
fn toggle_state(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

/// The words over the sizes, which say what the four rows choose between.
/// Not a row: nothing happens on a press.
fn heading(words: &'static str) -> AnyElement {
    div()
        .px(px(theme::PANEL_PAD))
        .pt(px(theme::CONTROL_PAD_Y))
        .text_size(px(theme::TEXT_META))
        .text_color(theme::text_dim())
        .whitespace_nowrap()
        .child(words)
        .into_any_element()
}

/// One row: in the accent when it is what is `chosen`, with anything the
/// caller adds after its words pushed to the far end, and on the press,
/// asks for `action`: every chat drawn another way, or the sizes reset. The
/// menu has already closed by then, in the capture phase, or closes with
/// the answer where a risen menu covers its anchor (see the module), and
/// the root takes the rest of the press's run (`RootView::on_pane_action`).
///
/// Only the first press of a run, as the bar's rows: a later one is never a
/// row's to take, since whatever the run's first press landed on chose.
#[allow(clippy::too_many_arguments)]
fn row<V: 'static>(
    id: impl Into<ElementId>,
    label: SharedString,
    chosen: bool,
    action: PaneAction,
    key: &str,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> gpui::Stateful<gpui::Div> {
    let key = key.to_string();
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
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                if event.click_count > 1 {
                    return;
                }
                on_pane(view, &key, action.clone(), window, cx)
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The times row's state, read in words beside it, says which way the
    /// row now stands rather than what a press would do.
    #[test]
    fn the_times_row_says_its_state() {
        assert_eq!(toggle_state(true), "On");
        assert_eq!(toggle_state(false), "Off");
    }

    #[test]
    fn a_closed_menu_opens_on_its_icon() {
        assert_eq!(toggled(None, "forsen"), Some("forsen".to_string()));
    }

    #[test]
    fn its_own_icon_closes_the_open_menu() {
        assert_eq!(toggled(Some("forsen"), "forsen"), None);
    }

    /// One menu at a time: another pane's icon opens that pane's in place of
    /// the one open, rather than a second beside it.
    #[test]
    fn another_panes_icon_moves_the_menu_there() {
        assert_eq!(toggled(Some("forsen"), "xqc"), Some("xqc".to_string()));
    }

    /// The bar's menu test, for this one: a row that waited for a click
    /// would never hear it, since the press closes the menu and takes the
    /// row away before the release. Held against the file's own source, as
    /// no test without a window can press one.
    #[test]
    fn menu_rows_act_on_the_press() {
        let code = code();
        assert!(
            !code.contains(".on_click("),
            "something in the chat options menu waits for a click, which never comes"
        );
        assert!(
            code.contains(".on_mouse_down(MouseButton::Left,"),
            "the chat options menu's rows no longer act on the press"
        );
    }

    /// What [`placed`] rests on, held against the file's source for the
    /// same reason, as no test without a window can lay the menu out. Each
    /// one undone loses the menu's foot, or its top, in some box: without
    /// the column laid out from its foot, a box shorter than the menu has it
    /// run on past the box again, `Reset pane sizes` first; without the
    /// margin under it, the menu sits at the foot of every box, however
    /// tall; and a menu free to shrink is cut short by its own clip.
    #[test]
    fn a_short_box_raises_the_menu_rather_than_losing_its_foot() {
        let code = code();
        for (needed, lost) in [
            (".justify_end()", "runs past the foot of a short chat box"),
            (".mb_auto()", "sits at the foot of every chat box"),
            (".flex_shrink_0()", "loses its last rows to its own clip"),
        ] {
            assert!(
                code.contains(needed),
                "without `{needed}` the chat options menu {lost}"
            );
        }
    }

    /// This file's code above its tests, with the whitespace taken out, so
    /// what a test looks for is not at the mercy of how it is wrapped.
    fn code() -> String {
        include_str!("chat_menu.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the file has code above its tests")
            .split_whitespace()
            .collect()
    }
}
