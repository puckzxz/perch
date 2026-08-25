//! The watch page: one to four streams, each with its own player and chat.
//!
//! Every pane is independent — its own volume, its own quality, its own chat —
//! because the point of watching two streams at once is that they are two
//! streams, not one with a picture-in-picture. The arrangement comes from
//! [`crate::layout`], which derives a grid from the window rather than looking
//! one up per stream count.

use gpui::{div, prelude::*, px, Context, Entity, IntoElement, SharedString, Task, Window};
use streamlink::StreamSupervisor;

use crate::chat::ChatView;
use crate::layout;
use crate::theme;
use crate::video_view::VideoView;

/// Beyond this, panes are too small to read chat in and the CPU cost stops
/// being worth it.
pub const MAX_PANES: usize = 4;

pub enum StreamState {
    Starting,
    Playing(Entity<VideoView>),
    Offline,
    Failed(SharedString),
}

/// One stream and everything that belongs to it.
///
/// Dropping a slot stops its streamlink (the supervisor) and its mpv (the
/// view), so removing a pane needs no explicit teardown.
pub struct Slot {
    pub channel: String,
    /// A quality picked from this pane's controls, overriding the saved
    /// preference until the pane closes.
    pub quality_override: Option<String>,
    pub state: StreamState,
    pub chat: Entity<ChatView>,
    pub supervisor: Option<StreamSupervisor>,
    pub pump: Option<Task<()>>,
}

impl Slot {
    pub fn video(&self) -> Option<&Entity<VideoView>> {
        match &self.state {
            StreamState::Playing(view) => Some(view),
            _ => None,
        }
    }
}

fn status_message(slot: &Slot) -> Option<(SharedString, bool)> {
    match &slot.state {
        StreamState::Playing(_) => None,
        StreamState::Starting => Some(("starting stream…".into(), false)),
        StreamState::Offline => {
            Some((format!("{} is offline", slot.channel).into(), false))
        }
        StreamState::Failed(reason) => Some((reason.clone(), true)),
    }
}

/// How every pane in the current grid is arranged. Identical for all of them,
/// so it is computed once and passed down rather than recomputed per pane.
#[derive(Clone, Copy)]
struct PaneLayout {
    /// Chat under the video rather than beside it.
    portrait: bool,
    chat_width: f32,
    chat_height: f32,
    /// False when there is only one pane; closing the last one is what the
    /// page-level navigation is for.
    closable: bool,
}

/// One pane: a player with its chat, and a header that only appears on hover.
fn pane<V: 'static>(
    index: usize,
    slot: &Slot,
    layout: PaneLayout,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let video = match (&slot.state, status_message(slot)) {
        (StreamState::Playing(view), _) => view.clone().into_any_element(),
        (_, Some((text, is_error))) => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(if is_error {
                theme::danger()
            } else {
                theme::text_dim()
            })
            .child(text)
            .into_any_element(),
        _ => div().into_any_element(),
    };

    let chat_pane = div()
        .flex_none()
        .py(px(theme::GAP_TIGHT))
        .bg(theme::surface())
        .map(|pane| {
            if layout.portrait {
                pane.h(px(layout.chat_height)).w_full()
            } else {
                pane.w(px(layout.chat_width)).h_full()
            }
        })
        .child(slot.chat.clone());

    let video_pane = div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(theme::player_bg())
        .relative()
        .group("pane")
        .child(video)
        .child(
            // The channel name and close control ride over the video and only
            // appear on hover, so a grid of four panes shows four pictures
            // rather than four title bars.
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(theme::GAP_TIGHT))
                .p(px(theme::GAP_TIGHT))
                .opacity(0.0)
                .group_hover("pane", |style| style.opacity(1.0))
                .child(
                    div()
                        .px(px(theme::CONTROL_PAD_X))
                        .py(px(theme::CONTROL_PAD_Y))
                        .rounded_sm()
                        .bg(theme::surface_raised())
                        .text_xs()
                        .text_color(theme::text())
                        .child(SharedString::from(slot.channel.clone())),
                )
                .child(div().flex_1())
                .when(layout.closable, |header| {
                    header.child(
                        div()
                            .id(("close-pane", index))
                            .px(px(theme::CONTROL_PAD_X))
                            .py(px(theme::CONTROL_PAD_Y))
                            .rounded_sm()
                            .bg(theme::surface_raised())
                            .text_xs()
                            .text_color(theme::text_muted())
                            .cursor_pointer()
                            .hover(|style| style.text_color(theme::danger()))
                            .child("close")
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                on_close(view, index, window, cx)
                            })),
                    )
                }),
        );

    div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .map(|cell| {
            if layout.portrait {
                cell.flex_col()
            } else {
                cell.flex_row()
            }
        })
        .child(video_pane)
        .child(chat_pane)
}

/// The whole watch page.
pub fn page<V: 'static>(
    slots: &[Slot],
    window_aspect: f32,
    chat_width: f32,
    chat_height: f32,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let (rows, cols) = layout::grid_shape(slots.len(), window_aspect);
    let cell = PaneLayout {
        portrait: layout::cell_is_portrait(layout::cell_aspect(window_aspect, rows, cols)),
        chat_width,
        chat_height,
        closable: slots.len() > 1,
    };

    let mut grid = div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(theme::PANE_GAP))
        .bg(theme::bg());

    for row in 0..rows {
        let mut line = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .gap(px(theme::PANE_GAP));
        for col in 0..cols {
            let index = row * cols + col;
            let Some(slot) = slots.get(index) else {
                continue;
            };
            line = line.child(pane(index, slot, cell, on_close.clone(), cx));
        }
        grid = grid.child(line);
    }
    grid
}
