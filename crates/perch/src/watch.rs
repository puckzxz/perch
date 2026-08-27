//! The watch page: one to four streams, each with its own player and chat.
//!
//! Every pane is independent — its own volume, its own quality, its own chat —
//! because the point of watching two streams at once is that they are two
//! streams, not one with a picture-in-picture. The arrangement comes from
//! [`crate::layout`], which derives a grid from the window rather than looking
//! one up per stream count.

use gpui::{
    canvas, div, prelude::*, px, Context, ElementId, Entity, IntoElement, Pixels, SharedString,
    Size, Task, Window,
};
use streamlink::StreamSupervisor;

use twitch_api::LiveStream;

use crate::browse;
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
    /// Whether the pointer is over this pane's video, measured rather than
    /// reported — see `VideoView::hovered` for why that distinction matters.
    /// The page navigation is revealed by any pane being hovered.
    pub hovered: bool,
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
    /// Beside: how wide chat is, with the video taking the rest.
    chat_width: f32,
    /// Below: how tall the video is, with chat taking the rest. The two are
    /// opposites on purpose — a tall window is tall because you want more chat,
    /// not more letterboxing.
    video_height: f32,
    /// False when there is only one pane; closing the last one is what the
    /// page-level navigation is for.
    closable: bool,
}

/// Everything true about a stream that is not playback: who it is, how many
/// people are there, how long it has been going.
///
/// This lives above chat rather than over the video. It is static information,
/// and static information on a moving picture is the thing you end up staring
/// past for three hours. Chat is already a panel, so it costs nothing here.
fn chat_header<V: 'static>(
    index: usize,
    slot: &Slot,
    info: Option<&LiveStream>,
    closable: bool,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let name = info
        .map(|stream| stream.display_name.clone())
        .unwrap_or_else(|| slot.channel.clone());

    // Both are absent for a channel opened by name that you do not follow:
    // the follows poll is where these numbers come from, and it only knows
    // about channels you follow.
    let meta = info
        .into_iter()
        .flat_map(|stream| {
            [
                Some(format!(
                    "{} watching",
                    browse::format_viewers(stream.viewer_count)
                )),
                browse::uptime(&stream.started_at),
            ]
        })
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::ROW_PAD_X))
        .pb(px(theme::GAP_TIGHT))
        .border_b_1()
        .border_color(theme::border())
        .child(
            // The same dot the browse cards use, for the same reason: it says
            // the numbers beside it are live rather than a playback position.
            div()
                .flex_none()
                .w(px(6.))
                .h(px(6.))
                .rounded_full()
                .bg(theme::live()),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(theme::TEXT_BODY))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .child(SharedString::from(name)),
        )
        .when(!meta.is_empty(), |header| {
            header.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_muted())
                    .child(SharedString::from(meta)),
            )
        })
        .child(div().flex_1())
        .when(closable, |header| {
            header.child(
                div()
                    .id(pane_id(&slot.channel, "close"))
                    .flex_none()
                    .px(px(theme::CONTROL_PAD_X))
                    .py(px(theme::CONTROL_PAD_Y))
                    .rounded_sm()
                    .text_size(px(theme::TEXT_LABEL))
                    .font_weight(theme::weight_label())
                    .text_color(theme::text_dim())
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme::danger()))
                    .active(|style| style.bg(theme::pressed()))
                    .child("close")
                    .on_click(cx.listener(move |view, _event, window, cx| {
                        on_close(view, index, window, cx)
                    })),
            )
        })
}

/// One pane: a player, and its chat with a header.
///
/// Nothing static is drawn over the video. What appears there on hover is
/// playback only.
#[allow(clippy::too_many_arguments)]
fn pane<V: 'static>(
    index: usize,
    slot: &Slot,
    info: Option<&LiveStream>,
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
        .flex()
        .flex_col()
        .py(px(theme::GAP_TIGHT))
        .bg(theme::surface())
        .map(|pane| {
            if layout.portrait {
                pane.flex_1().min_h_0().w_full()
            } else {
                pane.flex_none().w(px(layout.chat_width)).h_full()
            }
        })
        .child(chat_header(
            index,
            slot,
            info,
            layout.closable,
            on_close,
            cx,
        ))
        .child(div().flex_1().min_h_0().child(slot.chat.clone()));

    // Where the pointer actually is, rather than what `on_hover` claims while
    // something in the window is being dragged. The listener below only exists
    // to wake a repaint so this runs again.
    let owner = cx.entity().downgrade();
    let hover_probe = canvas(
        move |bounds, window, cx| {
            let inside = window.is_window_hovered() && bounds.contains(&window.mouse_position());
            owner
                .update(cx, |view, cx| on_hover(view, index, inside, cx))
                .ok();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();

    let video_pane = div()
        .id(pane_id(&slot.channel, "video"))
        .map(|pane| {
            if layout.portrait {
                pane.flex_none().h(px(layout.video_height)).w_full()
            } else {
                pane.flex_1().min_w_0()
            }
        })
        .min_h_0()
        .overflow_hidden()
        .bg(theme::player_bg())
        .relative()
        .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
        .child(hover_probe)
        .child(video);

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
    follows: &[LiveStream],
    window_size: Size<Pixels>,
    chat_width: f32,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let width = f32::from(window_size.width);
    let aspect = width / f32::from(window_size.height).max(1.0);
    let (rows, cols) = layout::grid_shape(slots.len(), aspect);
    let cell = PaneLayout {
        portrait: layout::cell_is_portrait(layout::cell_aspect(aspect, rows, cols)),
        // Clamped here rather than where it is stored: settings is a file
        // anyone can hand-edit, and a nonsense width would hide the video.
        chat_width: chat_width.clamp(theme::CHAT_WIDTH_MIN, theme::CHAT_WIDTH_MAX),
        video_height: layout::video_box_height(width / cols as f32),
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
            let info = follows
                .iter()
                .find(|stream| stream.user_login == slot.channel);
            line = line.child(pane(
                index,
                slot,
                info,
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
