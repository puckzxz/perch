//! The watch page: one to four streams, each with its own player and chat.
//!
//! Every pane is independent — its own volume, its own quality, its own chat —
//! because the point of watching two streams at once is that they are two
//! streams, not one with a picture-in-picture. The arrangement comes from
//! [`crate::layout`], which derives a grid from the window rather than looking
//! one up per stream count.

use gpui::{
    div, prelude::*, px, Context, ElementId, Entity, IntoElement, SharedString, Task, Window,
};
use streamlink::StreamSupervisor;

use crate::chat::ChatView;
use crate::layout;
use crate::motion;
use crate::theme;
use crate::video_view::VideoView;

/// Beyond this, panes are too small to read chat in and the CPU cost stops
/// being worth it.
pub const MAX_PANES: usize = 4;

/// Element ids inside a pane identify its **channel**, never its position.
///
/// Closing a pane reindexes every pane after it. Position-keyed ids make the
/// survivor inherit the closed pane's element state - including whether GPUI
/// thinks it is hovered - and since `on_hover` only fires when that value
/// *changes*, a stale `true` means the header never returns until the pointer
/// leaves the pane and comes back. Same lesson as the animated-emote ids: the
/// id has to name the thing, not the slot it happens to be in.
fn pane_id(channel: &str, role: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("pane-{role}-{channel}")))
}

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
    /// Whether this pane's header is showing. Explicit rather than
    /// `group_hover`, which can only produce an instant switch, and which stops
    /// evaluating under a mouse capture.
    pub header: motion::Fade,
}

impl Slot {
    pub fn video(&self) -> Option<&Entity<VideoView>> {
        match &self.state {
            StreamState::Playing(view) => Some(view),
            _ => None,
        }
    }
}

/// What a pane shows in place of a picture.
struct Status {
    text: SharedString,
    /// Something is still happening. Distinct from `error` because the two
    /// need opposite treatment: one should look alive, the other should sit
    /// still and be read.
    working: bool,
    error: bool,
}

fn status_message(slot: &Slot) -> Option<Status> {
    let status = |text: SharedString, working, error| Status {
        text,
        working,
        error,
    };
    match &slot.state {
        StreamState::Playing(_) => None,
        StreamState::Starting => Some(status("starting stream…".into(), true, false)),
        StreamState::Offline => Some(status(
            format!("{} is offline", slot.channel).into(),
            false,
            false,
        )),
        StreamState::Failed(reason) => Some(status(reason.clone(), false, true)),
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
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let video = match (&slot.state, status_message(slot)) {
        (StreamState::Playing(view), _) => view.clone().into_any_element(),
        (_, Some(status)) => {
            let label = div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(if status.error {
                    theme::danger()
                } else {
                    theme::text_dim()
                })
                .child(status.text);

            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                // Only the states that are going somewhere breathe. A pulsing
                // error would be both irritating and a repaint that never
                // stops.
                .child(if status.working {
                    motion::waiting(pane_id(&slot.channel, "status"), label).into_any_element()
                } else {
                    label.into_any_element()
                })
                .into_any_element()
        }
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

    let video_pane =
        div()
            .id(pane_id(&slot.channel, "video"))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme::player_bg())
            .relative()
            .on_hover(cx.listener(move |view, hovered: &bool, _window, cx| {
                on_hover(view, index, *hovered, cx)
            }))
            .child(video)
            .child(
                // The channel name and close control ride over the video and only
                // appear on hover, so a grid of four panes shows four pictures
                // rather than four title bars.
                //
                // Anchored to the pane's *right*, and deliberately so. Page
                // navigation lives in the window's top-left corner, which is
                // also the top-left of the first pane; every other corner of
                // every grid shape is far from it. Left-anchoring these put the
                // first pane's controls underneath the page navigation, where
                // one click ran both.
                slot.header.apply(
                    pane_id(&slot.channel, "header"),
                    theme::MOTION_HOVER,
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
                        .child(div().flex_1())
                        .child(
                            div()
                                .px(px(theme::CONTROL_PAD_X))
                                .py(px(theme::CONTROL_PAD_Y))
                                .rounded_sm()
                                .bg(theme::surface_raised())
                                .text_size(px(theme::TEXT_LABEL))
                                .font_weight(theme::weight_label())
                                .text_color(theme::text())
                                .child(SharedString::from(slot.channel.clone())),
                        )
                        .when(layout.closable, |header| {
                            header.child(
                                div()
                                    .id(pane_id(&slot.channel, "close"))
                                    .px(px(theme::CONTROL_PAD_X))
                                    .py(px(theme::CONTROL_PAD_Y))
                                    .rounded_sm()
                                    .bg(theme::surface_raised())
                                    .text_size(px(theme::TEXT_LABEL))
                                    .font_weight(theme::weight_label())
                                    .text_color(theme::text_muted())
                                    .cursor_pointer()
                                    .hover(|style| style.text_color(theme::danger()))
                                    .active(|style| style.bg(theme::pressed()))
                                    .child("close")
                                    .on_click(cx.listener(move |view, _event, window, cx| {
                                        on_close(view, index, window, cx)
                                    })),
                            )
                        }),
                ),
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
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + Clone + 'static,
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
            line = line.child(pane(
                index,
                slot,
                cell,
                on_close.clone(),
                on_hover.clone(),
                cx,
            ));
        }
        grid = grid.child(line);
    }
    grid
}
