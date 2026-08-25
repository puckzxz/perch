//! The video pane, and the controls that sit on top of it.
//!
//! Those controls are the payoff of rendering video as a real GPUI element.
//! Embedding mpv as a child window — the other way to do this — puts the video
//! in its own OS window that always paints above everything, so nothing can
//! overlap it. Here the video is just an element, and UI composites over it
//! like any other layer.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, img, prelude::*, px, rgb, rgba, Animation, AnimationExt, Context, ElementId, EventEmitter,
    RenderImage, SharedString, Task, Window,
};

use crate::video::VideoStream;

/// Volume is set by clicking one of these segments rather than dragging a
/// slider: it needs no element-bounds arithmetic, and it reads as a deliberate
/// design rather than a thin approximation of a native control.
const VOLUME_STEPS: u8 = 10;

pub enum VideoEvent {
    /// The user changed volume; worth persisting to settings.
    VolumeChanged(u8),
}

pub struct VideoView {
    stream: VideoStream,
    /// The frame currently painted, and the one before it. GPUI uploads every
    /// distinct `RenderImage` into its sprite atlas and `RenderImage::new` mints
    /// a fresh id per frame, so without evicting the frame before last the atlas
    /// grows by one frame every frame until VRAM runs out.
    current: Option<Arc<RenderImage>>,
    previous: Option<Arc<RenderImage>>,
    /// Volume before muting, so unmute restores rather than guessing.
    volume_before_mute: u8,
    quality: SharedString,
    _pump: Task<()>,
}

impl EventEmitter<VideoEvent> for VideoView {}

impl VideoView {
    /// Takes an already-started stream so a failure to open can be shown in the
    /// window rather than panicking inside entity construction.
    pub fn from_stream(
        stream: VideoStream,
        mut frames: futures::channel::mpsc::Receiver<()>,
        quality: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while frames.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        let volume_before_mute = stream.volume().max(1);
        Self {
            stream,
            current: None,
            previous: None,
            volume_before_mute,
            quality,
            _pump: pump,
        }
    }

    fn set_volume(&mut self, volume: u8, cx: &mut Context<Self>) {
        if volume > 0 {
            self.volume_before_mute = volume;
        }
        self.stream.set_volume(volume);
        cx.emit(VideoEvent::VolumeChanged(volume));
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let next = if self.stream.volume() == 0 {
            self.volume_before_mute
        } else {
            0
        };
        self.set_volume(next, cx);
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let volume = self.stream.volume();
        let filled = (volume as f32 / 100.0 * VOLUME_STEPS as f32).round() as u8;

        let mut bar = div().flex().flex_row().items_center().gap(px(2.));
        for step in 1..=VOLUME_STEPS {
            let target = step * (100 / VOLUME_STEPS);
            let lit = step <= filled;
            bar = bar.child(
                div()
                    .id(("volume-step", step as usize))
                    .w(px(6.))
                    .h(px(14.))
                    .rounded_xs()
                    .bg(if lit {
                        rgb(0xb392ff)
                    } else {
                        rgb(0x4a4358)
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0xcdb6ff)))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.set_volume(target, cx);
                    })),
            );
        }

        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            // Sits over live video, so it needs its own contrast rather than
            // relying on whatever happens to be on screen.
            .bg(rgba(0x0e0e12cc))
            .child(
                div()
                    .id("mute")
                    .w(px(20.))
                    .text_color(rgb(0xf2eff7))
                    .cursor_pointer()
                    .child(if volume == 0 { "🔇" } else { "🔊" })
                    .on_click(cx.listener(|this, _event, _window, cx| this.toggle_mute(cx))),
            )
            .child(bar)
            .child(
                div()
                    .w(px(34.))
                    .text_xs()
                    .text_color(rgb(0x948ca5))
                    .child(SharedString::from(format!("{volume}%"))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x948ca5))
                    .child(self.quality.clone()),
            )
    }
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(frame) = self.stream.latest_frame() else {
            return div()
                .size_full()
                .bg(rgb(0x0e0e12))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x6b6478))
                .child("buffering…")
                .into_any_element();
        };

        // Retire the frame before last: it is no longer on screen, so its atlas
        // entry can go. Never drop `current` itself - that one is still painted.
        if let Some(current) = self.current.take() {
            if let Some(previous) = self.previous.take() {
                if previous.id != current.id {
                    let _ = window.drop_image(previous);
                }
            }
            self.previous = Some(current);
        }
        self.current = Some(frame.clone());

        // Fade the first frames in rather than cutting from black, which makes
        // a channel switch feel deliberate instead of like a glitch.
        div()
            .relative()
            .size_full()
            .group("video")
            .child(
                img(frame).size_full().with_animation(
                    ElementId::from("video-fade-in"),
                    Animation::new(Duration::from_millis(260)),
                    |element, delta| element.opacity(delta),
                ),
            )
            .child(
                // Hidden until the pointer is over the video, so nothing covers
                // the picture while you are just watching.
                div()
                    .absolute()
                    .inset_0()
                    .opacity(0.0)
                    .group_hover("video", |style| style.opacity(1.0))
                    .child(self.controls(cx)),
            )
            .into_any_element()
    }
}
