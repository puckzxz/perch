//! A pane's header laid out wide under its picture — the strip — where a
//! beside pane's column has black under the picture to give it
//! (`layout::header_under_picture`), and the bar that then heads its chat.
//!
//! One stream is how Perch is watched almost all the time, and beside its
//! chat a 16:9 picture in a window of ordinary shape leaves black above and
//! below it: 151 px at 1920 by 1080, and as much at 4K maximized. The
//! header used to sit in the 340 px chat column beside that black, where a
//! channel's name was cut to its first six letters once `LIVE`, the numbers,
//! `muted` and three icons had their room. Under the picture it has the
//! picture's width: the channel's face, its name at a title's size, the
//! numbers saying what they count, the title on a line of its own and the
//! game under it.
//!
//! It says what the compact header says (`header::Facts`), through the
//! header's own builders, plus a pin, which the compact header left to the
//! bar's More. The chat options go to the bar over chat, the one other
//! place they hang over the chat they change.

use gpui::{div, img, prelude::*, px, Context, Div, SharedString, Stateful, Window};

use super::header::{self, Facts};
use super::{pane_id, PaneAction, PaneInfo, Slot};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::{sidebar, theme};

/// The header under the picture: the channel's face, then who is on with
/// the numbers and the tags, what is on and its game, and at the end the
/// pin, the channel's past broadcasts, browsing where the pane has no bar
/// to offer it, out or back, and the ×. It takes the rest of the column under the picture, on the page's
/// own ground rather than the player's black, so it reads as the pane's
/// facts rather than as more of the picture.
///
/// The same handle as the compact header (`header::handle`): with two panes
/// or more it is what a pane is dragged by, and its rule marks the pane the
/// keys talk to (`marked`). Its tooltips follow `window_hovered`, as the
/// header's do.
#[allow(clippy::too_many_arguments)]
pub(super) fn pane_strip<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    marked: bool,
    movable: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let facts = Facts::of(slot, pane, cx);
    // Room to say what the first number counts, which the compact header,
    // 340 px wide, leaves to the badge in front of it.
    let numbers = facts.numbers(true);
    let name = header::name_link(slot, &facts, theme::TEXT_TITLE, window_hovered, cx);
    let pin = pin_toggle(slot, pane.pinned, window_hovered, on_pane.clone(), cx);
    let videos = pane
        .past_broadcasts
        .then(|| past_broadcasts(slot, window_hovered, on_pane.clone(), cx));
    let browse = header::browse_button(slot, pane, window_hovered, on_pane.clone(), cx);
    let pop = header::pop_button(slot, pane, window_hovered, on_pane.clone(), cx);
    let close = header::close_button(slot, window_hovered, on_pane.clone(), cx);
    let about = about_lines(slot, &facts, window_hovered);

    header::handle(slot, movable, on_pane, cx)
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(theme::GAP))
        .px(px(theme::PAGE_PAD))
        .py(px(theme::PANEL_PAD))
        .bg(theme::bg())
        // The rule that marks the pane the keys talk to, as the compact
        // header's does, kept clear when unmarked so marking it never moves
        // a line.
        .border_b_1()
        .border_color(if marked {
            theme::accent()
        } else {
            theme::border_clear()
        })
        .children(pane.avatar.clone().map(|path| {
            img(path)
                .flex_none()
                .size(px(theme::STRIP_AVATAR))
                .rounded_full()
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_WORD))
                .child(header::with_status(
                    div()
                        .w_full()
                        .h(px(theme::ICON_BUTTON))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::GAP_TIGHT))
                        .child(name),
                    &facts,
                    numbers,
                ))
                .children(about),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_TIGHT))
                .child(pin)
                .children(videos)
                .children(browse)
                .children(pop)
                .child(close),
        )
}

/// What is on, under who is on: the title on a line of its own at the size
/// you read, and the game under it in the supporting tier — or a
/// recording's title and when it was. Each cut to its line, with the whole
/// of both a hover away on the title, through the builder the browse cards
/// use.
fn about_lines(slot: &Slot, facts: &Facts, window_hovered: bool) -> Vec<Stateful<Div>> {
    let mut lines = facts.about.iter().cloned();
    let title = lines.next();
    let rest = lines.next();
    let whole = facts.about.clone();
    title
        .map(|title| {
            line(pane_id(&slot.key, "about"), title)
                .text_size(px(theme::TEXT_BODY))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text())
                .when(window_hovered, |line| {
                    line.tooltip(controls::full_text(whole))
                })
        })
        .into_iter()
        .chain(rest.map(|rest| {
            line(pane_id(&slot.key, "about-more"), rest)
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_TIGHT))
                .text_color(theme::text_muted())
        }))
        .collect()
}

/// One line of the strip's text, cut with an ellipsis where it runs out:
/// `w_full`, since a child of a flex column needs a definite width before
/// gpui's measure pass will ellipsise it.
fn line(id: impl Into<gpui::ElementId>, text: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .text_ellipsis()
        .line_clamp(1)
        .child(text)
}

/// The pane's channel pinned to the rail, or not: `Pin`, or `Pinned` as the
/// chosen pill, whose press unpins it. More's row (`Pin to the rail`) did
/// this alone before; under the picture there is room to show the state
/// and offer it in one. The press says which it was offered as
/// (`PaneAction::Pin`), so a pill drawn a frame stale pins what is pinned
/// rather than taking it off. Its id follows the state, so a tooltip that
/// came up over one does not outlive the press that changed it.
fn pin_toggle<V: 'static>(
    slot: &Slot,
    pinned: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let key = slot.key.clone();
    let (id, words, variant) = if pinned {
        ("pinned", "Pinned", Variant::Selected)
    } else {
        ("pin", "Pin", Variant::Pill)
    };
    controls::pill(pane_id(&slot.key, id), words, variant)
        .when(window_hovered, |pill| {
            pill.tooltip(controls::tip(sidebar::pin_words(pinned)))
        })
        .on_click(cx.listener(move |view, _event, window, cx| {
            if !cx.has_active_drag() {
                on_pane(view, &key, PaneAction::Pin(!pinned), window, cx)
            }
        }))
}

/// The channel's page, its past broadcasts and schedule, beside the pane,
/// which plays on: the same words a browse card's corner pill says it in.
fn past_broadcasts<V: 'static>(
    slot: &Slot,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let key = slot.key.clone();
    controls::pill(
        pane_id(&slot.key, "past-broadcasts"),
        "Past broadcasts",
        Variant::Pill,
    )
    .when(window_hovered, |pill| {
        pill.tooltip(controls::tip(
            "Their past broadcasts and schedule, beside this",
        ))
    })
    .on_click(cx.listener(move |view, _event, window, cx| {
        if !cx.has_active_drag() {
            on_pane(view, &key, PaneAction::ChannelPage, window, cx)
        }
    }))
}

/// The bar over a pane's chat while its header is under the picture: what
/// the column is, the chat options (`header::chat_options`, every chat's
/// text size and times) and hiding chat, which the player's bar offers too
/// but only while the pointer is on the picture. On the chat panel, so it
/// costs a row of chat rather than anything of the picture.
pub(super) fn chat_bar<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let options = header::chat_options(slot, pane, window_hovered, on_pane.clone(), cx);
    let key = slot.key.clone();
    let hide = controls::icon_button(
        pane_id(&slot.key, "hide-chat"),
        Icon::Folded,
        Variant::OnVideo,
    )
    .when(window_hovered, |button| {
        button.tooltip(controls::tip(Hint::Chat.tooltip("Hide chat")))
    })
    .on_click(cx.listener(move |view, _event, window, cx| {
        on_pane(view, &key, PaneAction::ToggleChat, window, cx)
    }));

    div()
        .flex_none()
        .w_full()
        .h(px(theme::CHAT_BAR_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .pl(px(theme::ROW_PAD_X))
        .pr(px(theme::GAP_TIGHT))
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .text_size(px(theme::TEXT_LABEL))
                .font_weight(theme::weight_label())
                .line_height(px(theme::LINE_TIGHT))
                .text_color(theme::text_muted())
                .child("Chat"),
        )
        .child(div().flex_1())
        .child(options)
        .child(hide)
}
