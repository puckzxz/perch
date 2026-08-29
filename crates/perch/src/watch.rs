//! The watch page: one to four streams, each with its own player and chat.
//!
//! Every pane is independent — its own volume, its own quality, its own chat —
//! because the point of watching two streams at once is that they are two
//! streams, not one with a picture-in-picture. The arrangement comes from
//! [`crate::layout`], which derives a grid from the window rather than looking
//! one up per stream count.

use gpui::{
    canvas, div, prelude::*, px, Context, CursorStyle, ElementId, Entity, IntoElement, MouseButton,
    MouseDownEvent, Pixels, SharedString, Size, Task, Window,
};
use streamlink::StreamSupervisor;

use twitch_api::LiveStream;

use crate::browse;
use crate::chat::ChatView;
use crate::controls;
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
    /// Whether this pane's chat is hidden, so the video has the whole cell.
    ///
    /// Per pane, like everything else here, and remembered per channel: a
    /// channel you watch for the game is not a statement about the next one.
    pub chat_hidden: bool,
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
    /// not more letterboxing. Derived from a 16:9 box until somebody drags the
    /// divider, after which it is their share of the cell.
    video_height: f32,
    /// False when there is only one pane; closing the last one is what the
    /// page-level navigation is for.
    closable: bool,
    /// Whether to mark the pane the keyboard is acting on. Only worth saying
    /// with more than one pane on screen: with one, it is the answer to a
    /// question nobody asked.
    mark_active: bool,
}

/// Where a drag of the video/chat divider started.
///
/// The pointer's position and which way the pane is split; the *sizes* it moves
/// come from settings, which is where they end up again — so the drag itself
/// holds nothing that has to be kept in step with anything.
#[derive(Clone, Copy)]
pub struct ResizeStart {
    pub origin: gpui::Point<Pixels>,
    pub portrait: bool,
}

/// The seam between video and chat, as something you can pull.
///
/// Six pixels of grab area drawing a one-pixel line: a divider you can see is
/// not the same thing as a divider you can hit, and the pane gap elsewhere is
/// three pixels precisely because nothing is meant to grab *it*.
fn divider<V: 'static>(
    channel: &str,
    portrait: bool,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .id(pane_id(channel, "divider"))
        .flex_none()
        .map(|handle| {
            if portrait {
                handle.w_full().h(px(theme::DIVIDER_GRAB))
            } else {
                handle.h_full().w(px(theme::DIVIDER_GRAB))
            }
        })
        .cursor(if portrait {
            CursorStyle::ResizeUpDown
        } else {
            CursorStyle::ResizeLeftRight
        })
        .hover(|style| style.bg(theme::accent_dim()))
        .active(|style| style.bg(theme::accent()))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                on_resize(
                    view,
                    ResizeStart {
                        origin: event.position,
                        portrait,
                    },
                    window,
                    cx,
                )
            }),
        )
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
    active: bool,
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

    // Whether this pane is showing a picture, which is not the same as whether
    // it exists: an offline or failed pane used to draw the app's only
    // saturated red beside its name while the video underneath said the channel
    // was not streaming.
    let playing = matches!(slot.state, StreamState::Playing(_));
    let url = format!("https://twitch.tv/{}", slot.channel);
    let tooltip = SharedString::from(format!("Open twitch.tv/{}", slot.channel));

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
        // Which pane the keyboard is talking to. `Space`, `M`, the arrows and
        // `Ctrl+W` all act on the pane you last pointed at, and with four on
        // screen nothing said which that was — so every press was a guess. One
        // line under one header, and only when there is more than one pane to
        // tell apart.
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .when(playing, |header| {
            header.child(
                // The same dot the browse cards use, for the same reason: it
                // says the numbers beside it are live rather than a playback
                // position.
                div()
                    .flex_none()
                    .w(px(6.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(theme::live()),
            )
        })
        .child(
            // Chat here is read-only by design. This is the way out of that:
            // the one thing the app deliberately cannot do, one click from the
            // name of the channel you would be saying it in.
            div()
                .id(pane_id(&slot.channel, "open"))
                .flex_none()
                .text_size(px(theme::TEXT_BODY))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .cursor_pointer()
                .hover(|style| style.text_color(theme::accent()))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&url)))
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
                controls::destructive(pane_id(&slot.channel, "close"), "close").on_click(
                    cx.listener(move |view, _event, window, cx| on_close(view, index, window, cx)),
                ),
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
    active: bool,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
    on_retry: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static,
    on_activate: impl Fn(&mut V, usize, &mut Context<V>) + 'static,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> gpui::AnyElement {
    let video = match (&slot.state, status_message(slot)) {
        (StreamState::Playing(view), _) => view.clone().into_any_element(),
        (_, Some(status)) => {
            let retryable = status.error;
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
                .flex_col()
                .gap(px(theme::GAP))
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
                // A stream that failed to start is the one pane state with
                // something to do about it, and until now the only way to do it
                // was to close the pane and open the channel again.
                .when(retryable, |pane| {
                    pane.child(
                        controls::pill(
                            pane_id(&slot.channel, "retry"),
                            "try again",
                            controls::Variant::Primary,
                        )
                        .on_click(cx.listener(
                            move |view, _event, window, cx| on_retry(view, index, window, cx),
                        )),
                    )
                })
                .into_any_element()
        }
        _ => div().into_any_element(),
    };

    let header = chat_header(
        index,
        slot,
        info,
        layout.closable,
        layout.mark_active && active,
        on_close,
        cx,
    );

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
            // With chat hidden the video is the whole cell, so it stops being
            // sized against chat and simply takes what is left under the
            // header.
            if slot.chat_hidden {
                pane.flex_1().min_w_0()
            } else if layout.portrait {
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

    // With chat hidden the cell is a column whatever the grid shape, because
    // the only thing left beside the video is the header strip.
    //
    // That strip stays rather than going with the chat it used to sit on. It
    // carries the channel's name and the close button, and nothing is drawn
    // over the video on purpose — so losing it would leave a pane with no
    // identity and no way to close it but the keyboard.
    // Pointing at the video makes a pane active, which is right while you are
    // reaching for its controls and wrong the moment you go to read its chat:
    // the pointer sitting in one pane's messages left the *keyboard* still
    // talking to whichever video it crossed last. A click anywhere in the pane
    // — either half — says which one you mean, and says it deliberately.
    //
    // `on_mouse_down` rather than `on_click`, so it lands on the way down and
    // does not wait to find out whether the press was a click, a drag of the
    // volume slider or the start of a text selection. It does not consume the
    // event: a link in chat still opens, and the close button still closes.
    let cell = div().flex_1().min_w_0().min_h_0().flex().on_mouse_down(
        MouseButton::Left,
        cx.listener(move |view, _event, _window, cx| on_activate(view, index, cx)),
    );

    if slot.chat_hidden {
        // A column whatever the grid shape, because the only thing left beside
        // the video is the header strip.
        //
        // That strip stays rather than going with the chat it used to sit on:
        // it carries the channel's name and the close button, and nothing is
        // drawn over the video on purpose — so dropping it would leave a pane
        // with no identity and no way to close it but the keyboard.
        return cell
            .flex_col()
            .bg(theme::surface())
            .child(
                // The same vertical padding chat used to give it. Without this
                // the header sits flush against the top of the cell and the top
                // of the video, and the pane reads as clipped rather than as
                // deliberately bare.
                div()
                    .flex_none()
                    .w_full()
                    .py(px(theme::GAP_TIGHT))
                    // Only the top-left pane, which is the only one the page's
                    // "← follows" overlay can reach. Every other pane's header
                    // starts where it always did.
                    .when(index == 0, |header| header.pl(px(theme::NAV_RESERVE)))
                    .child(header),
            )
            .child(video_pane)
            .into_any_element();
    }

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
        .child(header)
        .child(div().flex_1().min_h_0().child(slot.chat.clone()));

    cell.map(|cell| {
        if layout.portrait {
            cell.flex_col()
        } else {
            cell.flex_row()
        }
    })
    .child(video_pane)
    .child(divider(&slot.channel, layout.portrait, on_resize, cx))
    .child(chat_pane)
    .into_any_element()
}

/// The whole watch page.
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    slots: &[Slot],
    follows: &[LiveStream],
    window_size: Size<Pixels>,
    chat_width: f32,
    video_share: f32,
    active: Option<usize>,
    on_close: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    on_retry: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    on_activate: impl Fn(&mut V, usize, &mut Context<V>) + Clone + 'static,
    on_resize: impl Fn(&mut V, ResizeStart, &mut Window, &mut Context<V>) + Clone + 'static,
    on_hover: impl Fn(&mut V, usize, bool, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let width = f32::from(window_size.width);
    let aspect = width / f32::from(window_size.height).max(1.0);
    let (rows, cols) = layout::grid_shape(slots.len(), aspect);
    let cell_height = f32::from(window_size.height) / rows as f32;
    let cell = PaneLayout {
        portrait: layout::cell_is_portrait(layout::cell_aspect(aspect, rows, cols)),
        // Clamped here rather than where it is stored: settings is a file
        // anyone can hand-edit, and a nonsense width would hide the video.
        chat_width: chat_width.clamp(theme::CHAT_WIDTH_MIN, theme::CHAT_WIDTH_MAX),
        video_height: if video_share > 0.0 {
            cell_height * video_share.clamp(theme::VIDEO_SHARE_MIN, theme::VIDEO_SHARE_MAX)
        } else {
            layout::video_box_height(width / cols as f32)
        },
        closable: slots.len() > 1,
        mark_active: slots.len() > 1,
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
                active == Some(index),
                on_close.clone(),
                on_retry.clone(),
                on_activate.clone(),
                on_resize.clone(),
                on_hover.clone(),
                cx,
            ));
        }
        grid = grid.child(line);
    }
    grid
}
