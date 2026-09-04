//! The follows rail.
//!
//! A list of who is live, down the right-hand edge, on both pages. It exists
//! because the two halves of this app were further apart than the work is:
//! picking a channel meant leaving the one you were watching, going to a grid
//! of thumbnails, and coming back. Most of the time the question is not "what
//! is on" — it is "who else is live", which is a list of names and numbers and
//! fits in a column.
//!
//! On the left, opposite chat. Chat belongs to the pane it is part of and sits
//! on the right of it; the rail belongs to the window. Putting both on the same
//! edge made two unrelated columns of names next to each other, and made
//! switching streams a reach across the whole window from the thing you were
//! reading.
//!
//! It folds away, and stays folded — see [`settings::Settings::sidebar_collapsed`].
//! A window left open for three hours on one stream should be able to be just
//! the stream.

use std::collections::HashMap;
use std::sync::Arc;

use emotes::ImageCache;
use gpui::{div, img, prelude::*, px, Context, ScrollHandle, SharedString};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use twitch_api::LiveStream;

use crate::browse::{format_viewers, Action};
use crate::controls;
use crate::theme;

/// How wide the rail is when it is open.
///
/// Narrow enough that a 16:9 video beside it still has room to be a video, wide
/// enough for a name and a game underneath it. Fixed rather than derived: this
/// is a list of one-line rows, and there is nothing for extra width to do.
pub const WIDTH: f32 = 236.0;

/// The avatar. Small on purpose — it is here to be recognised at a glance, not
/// looked at, and a row taller than two lines of text stops the rail being a
/// list.
const AVATAR: f32 = 30.0;

/// Twitch serves profile pictures at a handful of sizes; this is the smallest
/// that still looks right on a HiDPI display at [`AVATAR`].
const AVATAR_REQUEST: &str = "70x70";

/// One live channel, as a row.
#[allow(clippy::too_many_arguments)]
fn row<V: 'static>(
    index: usize,
    stream: &LiveStream,
    avatar: Option<&String>,
    watching: bool,
    can_add: bool,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let on_click = on_action.clone();
    let on_add = on_action;
    let login = stream.user_login.clone();
    let add_login = stream.user_login.clone();

    // Twitch's URLs carry the size in the filename rather than as a parameter,
    // so this is a substitution rather than a query. A miss simply leaves the
    // original, which is a larger picture of the right person.
    let picture = avatar
        .map(|url| url.replace("300x300", AVATAR_REQUEST))
        .and_then(|url| cache.get_or_request(&url));

    let face = match picture {
        Some(path) => img(path)
            .w(px(AVATAR))
            .h(px(AVATAR))
            .rounded_full()
            .into_any_element(),
        // Sized placeholder, so a rail whose pictures have not arrived is the
        // same shape as one whose pictures have.
        None => div()
            .w(px(AVATAR))
            .h(px(AVATAR))
            .rounded_full()
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    div()
        .id(("sidebar-row", index))
        .group("rail-row")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::PANEL_PAD))
        .py(px(theme::GAP_TIGHT))
        .rounded(px(theme::RADIUS))
        .cursor_pointer()
        // A channel already open reads as chosen rather than as offered, which
        // is the same thing the browse page's tab pills say about themselves.
        .when(watching, |row| row.bg(theme::accent_dim()))
        .hover(|style| style.bg(theme::hover()))
        .active(|style| style.bg(theme::pressed()))
        .on_click(cx.listener(move |view, _event, window, cx| {
            on_click(view, Action::Watch(login.clone()), window, cx)
        }))
        .child(div().flex_none().child(face))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .w_full()
                        .text_ellipsis()
                        .line_clamp(1)
                        .text_size(px(theme::TEXT_LABEL))
                        .font_weight(theme::weight_title())
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text())
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .child(
                    div()
                        .w_full()
                        .text_ellipsis()
                        .line_clamp(1)
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(stream.game_name.clone())),
                ),
        )
        // The count, and the dot that says the count is of people watching now.
        // Swapped for "+ add" under the pointer: the row is already a click, so
        // the second thing you might mean has to be somewhere the first is not.
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(GAP_COUNT))
                .when(can_add, |count| {
                    count.group_hover("rail-row", |style| style.invisible())
                })
                .child(controls::live_dot())
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(format_viewers(stream.viewer_count))),
                ),
        )
        .when(can_add, |row| {
            row.child(
                controls::pill(("sidebar-add", index), "+", controls::Variant::Pill)
                    .absolute()
                    .right(px(theme::PANEL_PAD))
                    .invisible()
                    .group_hover("rail-row", |style| style.visible())
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Open beside what is playing")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |view, _event, window, cx| {
                        // Without this the row underneath also fires and
                        // replaces every open pane with this one.
                        cx.stop_propagation();
                        on_add(view, Action::Add(add_login.clone()), window, cx)
                    })),
            )
        })
}

/// Between the live dot and the number it belongs to. Tighter than
/// [`theme::GAP_TIGHT`], because they are one thing rather than two.
const GAP_COUNT: f32 = 4.0;

/// The rail, or `None` when it is folded away.
#[allow(clippy::too_many_arguments)]
pub fn rail<V: 'static>(
    follows: &[LiveStream],
    avatars: &HashMap<String, String>,
    watching: &[String],
    can_add: bool,
    collapsed: bool,
    cache: &Arc<ImageCache>,
    scroll: &ScrollHandle,
    on_toggle: impl Fn(&mut V, &mut gpui::Window, &mut Context<V>) + 'static,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Option<impl IntoElement> {
    if collapsed {
        return None;
    }

    let header = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::PANEL_PAD))
        .py(px(theme::GAP_TIGHT))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(theme::TEXT_LABEL))
                .font_weight(theme::weight_label())
                .text_color(theme::text_dim())
                .child("live now"),
        )
        .child(
            controls::pill("sidebar-collapse", "←", controls::Variant::Quiet)
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new("Hide the rail (B)").build(window, cx)
                })
                .on_click(cx.listener(move |view, _event, window, cx| on_toggle(view, window, cx))),
        );

    // Its own scroller. The rail is as tall as the window and a hundred live
    // follows is longer than that, and it must not scroll the page behind it.
    let mut list = div()
        .id("sidebar-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .px(px(theme::GAP_TIGHT))
        .pb(px(theme::GAP_TIGHT));

    for (index, stream) in follows.iter().enumerate() {
        list = list.child(row(
            index,
            stream,
            avatars.get(&stream.user_login),
            watching.contains(&stream.user_login),
            can_add,
            cache,
            on_action.clone(),
            cx,
        ));
    }

    if follows.is_empty() {
        list = list.child(
            div()
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP))
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text_dim())
                .child("Nobody you follow is live."),
        );
    }

    Some(
        div()
            .flex_none()
            .w(px(WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::surface())
            .border_r_1()
            .border_color(theme::border())
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(list)
                    .child(
                        div().absolute().inset_0().child(
                            Scrollbar::vertical(scroll).scrollbar_show(ScrollbarShow::Hover),
                        ),
                    ),
            ),
    )
}

/// The control that brings a folded rail back.
///
/// Separate from [`rail`] because it outlives it: when the rail is gone this is
/// the only thing left that knows it exists.
pub fn expand<V: 'static>(
    on_toggle: impl Fn(&mut V, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    controls::pill("sidebar-expand", "→", controls::Variant::Pill)
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new("Show who is live (B)").build(window, cx)
        })
        .on_click(cx.listener(move |view, _event, window, cx| on_toggle(view, window, cx)))
}
