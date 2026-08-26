//! The video pane, and the controls that sit on top of it.
//!
//! Those controls are the payoff of rendering video as a real GPUI element.
//! Embedding mpv as a child window — the other way to do this — puts the video
//! in its own OS window that always paints above everything, so nothing can
//! overlap it. Here the video is just an element, and UI composites over it
//! like any other layer.

use std::sync::Arc;

use gpui::{
    canvas, div, img, prelude::*, px, rgba, Animation, AnimationExt, Context, ElementId, Entity,
    EventEmitter, Hsla, RenderImage, SharedString, Task, Window,
};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

use crate::browse;
use crate::motion;
use crate::theme;
use crate::video::VideoStream;

/// Where the quality menu rests above the control bar, and how far below that
/// it starts when opening.
const MENU_BOTTOM: f32 = 32.0;
const MENU_RISE: f32 = 6.0;

pub enum VideoEvent {
    /// The user changed volume; worth persisting to settings.
    VolumeChanged(u8),
    /// The user picked a different quality. Switching means restarting
    /// streamlink, so the root handles it rather than the player.
    QualityRequested(String),
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
    volume_slider: Entity<SliderState>,
    quality: SharedString,
    /// Other qualities this channel offers, highest first.
    available: Vec<String>,
    quality_menu_open: bool,
    /// When the broadcast started, for the uptime readout. Absent when the
    /// channel was opened by name rather than picked from the follows list.
    started_at: Option<String>,
    /// Whether the pointer is over this player.
    ///
    /// Tracked explicitly rather than derived from `group_hover` at paint time.
    /// Pressing the volume slider takes a mouse capture, which stops the group's
    /// hover state evaluating, so a hover-derived opacity made the whole control
    /// bar vanish mid-drag.
    hovered: bool,
    /// Whether the control bar is up, and how far through fading it is.
    /// Derived from `hovered` and the quality menu by `sync_controls`.
    controls: motion::Fade,
    /// True while the player is a thumbnail on the browse page. Backgrounded
    /// players are muted and draw no controls.
    background: bool,
    /// Volume to restore when coming back to the foreground.
    volume_before_background: u8,
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
        available: Vec<String>,
        started_at: Option<String>,
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

        let volume = stream.volume();
        let volume_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(volume as f32)
        });
        cx.subscribe(&volume_slider, |this: &mut Self, _, event, cx| {
            let SliderEvent::Change(value) = event;
            this.apply_volume(value.start().round() as u8, cx);
        })
        .detach();

        Self {
            stream,
            current: None,
            previous: None,
            volume_before_mute: volume.max(1),
            volume_slider,
            quality,
            available,
            quality_menu_open: false,
            started_at,
            hovered: false,
            controls: motion::Fade::hidden(),
            background: false,
            volume_before_background: volume,
            _pump: pump,
        }
    }

    /// Set volume without writing back to the slider, which is already where
    /// the user put it. Writing back would fight an in-progress drag.
    fn apply_volume(&mut self, volume: u8, cx: &mut Context<Self>) {
        if volume > 0 {
            self.volume_before_mute = volume;
        }
        self.stream.set_volume(volume);
        cx.emit(VideoEvent::VolumeChanged(volume));
        cx.notify();
    }

    fn set_volume(&mut self, volume: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.volume_slider.update(cx, |state, cx| {
            state.set_value(volume as f32, window, cx);
        });
        self.apply_volume(volume, cx);
    }

    fn toggle_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.stream.volume() == 0 {
            self.volume_before_mute
        } else {
            0
        };
        self.set_volume(next, window, cx);
    }

    /// Move the player between the watch page and the browse thumbnail.
    ///
    /// Muting here deliberately bypasses `apply_volume`: that emits an event
    /// the root persists, and navigating to the follows page should not save
    /// "volume: 0" as the user's preference.
    pub fn set_background(&mut self, background: bool, cx: &mut Context<Self>) {
        if self.background == background {
            return;
        }
        self.background = background;
        if background {
            self.volume_before_background = self.stream.volume();
            self.stream.set_volume(0);
        } else {
            self.stream.set_volume(self.volume_before_background);
        }
        self.quality_menu_open = false;
        self.hovered = false;
        self.sync_controls();
        cx.notify();
    }

    /// Recompute whether the control bar should be up, and report whether that
    /// changed anything.
    ///
    /// It stays up while the quality menu is open even after the pointer
    /// leaves, or reaching for an option would dismiss the menu on the way.
    fn sync_controls(&mut self) -> bool {
        let visible = !self.background && (self.hovered || self.quality_menu_open);
        self.controls.set(visible)
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        self.stream.set_paused(!self.stream.is_paused());
        cx.notify();
    }

    fn pill(id: &'static str, label: SharedString) -> gpui::Stateful<gpui::Div> {
        div()
            .id(ElementId::from(id))
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded_sm()
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text())
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()))
            // These sit on video with no shadow or border to deform, so a
            // press has only the one channel to show itself in.
            .active(|style| style.bg(theme::pressed()))
            .child(label)
    }

    fn quality_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = div()
            .absolute()
            .right_0()
            .flex()
            .flex_col()
            .min_w(px(120.))
            .rounded_md()
            .overflow_hidden()
            .bg(theme::surface_raised())
            .border_1()
            .border_color(theme::border());

        let current = self.quality.to_string();
        for (index, name) in self.available.iter().enumerate() {
            let selected = *name == current;
            let chosen = name.clone();
            menu = menu.child(
                div()
                    .id(("quality-option", index))
                    .px(px(theme::PANEL_PAD))
                    .py(px(theme::CONTROL_PAD_Y))
                    .text_size(px(theme::TEXT_LABEL))
                    .font_weight(theme::weight_label())
                    .cursor_pointer()
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::text()
                    })
                    .hover(|style| style.bg(theme::hover()))
                    .active(|style| style.bg(theme::pressed()))
                    .child(SharedString::from(name.clone()))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.quality_menu_open = false;
                        this.sync_controls();
                        cx.emit(VideoEvent::QualityRequested(chosen.clone()));
                        cx.notify();
                    })),
            );
        }

        // Rises the last few pixels into place, so it reads as coming out of
        // the button rather than being stamped over the video. It is mounted
        // only while open, which is what makes a plain one-shot enough: there
        // is no closed state to animate back to.
        menu.with_animation(
            ElementId::from("quality-menu"),
            Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
            |menu, delta| {
                menu.opacity(delta)
                    .bottom(px(MENU_BOTTOM - MENU_RISE * (1.0 - delta)))
            },
        )
    }

    fn control_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let volume = self.stream.volume();
        let paused = self.stream.is_paused();

        let uptime = self
            .started_at
            .as_deref()
            .and_then(browse::uptime)
            .map(SharedString::from);

        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .px(px(theme::PANEL_PAD))
            .py(px(theme::GAP_TIGHT))
            // Sits over live video, so it carries its own contrast rather than
            // relying on whatever happens to be on screen behind it.
            .bg(rgba(0x000000b3))
            .child(
                Self::pill("pause", if paused { "play" } else { "pause" }.into())
                    .on_click(cx.listener(|this, _event, _window, cx| this.toggle_pause(cx))),
            )
            .child(
                Self::pill("mute", if volume == 0 { "unmute" } else { "mute" }.into())
                    .on_click(cx.listener(|this, _event, window, cx| this.toggle_mute(window, cx))),
            )
            .child(
                div()
                    .w(px(120.))
                    .child(Slider::new(&self.volume_slider).horizontal()),
            )
            .child(
                div()
                    .w(px(38.))
                    .text_size(px(theme::TEXT_META))
                    .text_right()
                    .text_color(theme::text_muted())
                    .child(SharedString::from(format!("{volume}%"))),
            )
            .child(div().flex_1())
            .children(uptime.map(|text| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    .child(
                        // A live dot, so uptime reads as "on air for" rather
                        // than as a playback position.
                        div().w(px(6.)).h(px(6.)).rounded_full().bg(theme::live()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_muted())
                            .child(text),
                    )
            }))
            .child(
                div()
                    .relative()
                    .child(
                        Self::pill("quality", self.quality.clone()).on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.quality_menu_open = !this.quality_menu_open;
                                this.sync_controls();
                                cx.notify();
                            },
                        )),
                    )
                    .when(self.quality_menu_open, |anchor| {
                        anchor.child(self.quality_menu(cx))
                    }),
            )
    }
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let backdrop: Hsla = theme::player_bg();

        let Some(frame) = self.stream.latest_frame() else {
            // Breathing rather than still: a stream takes a few seconds to
            // arrive, and a motionless word is indistinguishable from a hang.
            // Only the text pulses - taking the backdrop with it would strobe
            // the whole pane.
            return div()
                .size_full()
                .bg(backdrop)
                .flex()
                .items_center()
                .justify_center()
                .child(motion::waiting(
                    "buffering",
                    div().text_color(theme::text_dim()).child("buffering…"),
                ))
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

        // Measure the pane every frame and let the render thread follow it.
        // Without this the buffer stays at its initial size and a 1440p stream
        // is downscaled before it ever reaches the window.
        let stream_size = self.stream.size_handle();
        let probe = canvas(
            move |bounds, window, _cx| {
                let scale = window.scale_factor();
                let width = (f32::from(bounds.size.width) * scale).round() as u32;
                let height = (f32::from(bounds.size.height) * scale).round() as u32;
                stream_size.request(width, height);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .relative()
            .size_full()
            .bg(backdrop)
            .id("video-pane")
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                this.hovered = *hovered;
                if this.sync_controls() {
                    cx.notify();
                }
            }))
            .child(probe)
            // Fade the first frames in rather than cutting from black, which
            // makes a channel switch read as deliberate instead of a glitch.
            .child(img(frame).size_full().with_animation(
                ElementId::from("video-fade-in"),
                Animation::new(theme::MOTION_VIDEO),
                |element, delta| element.opacity(delta),
            ))
            .when(!self.background, |pane| {
                // Hidden until the pointer is over the video, so nothing covers
                // the picture while you are just watching - and faded rather
                // than cut, because over a moving image a hard switch reads as
                // part of the video instead of a response to the pointer.
                pane.child(self.controls.apply(
                    "controls",
                    theme::MOTION_HOVER,
                    div().absolute().inset_0().child(self.control_bar(cx)),
                ))
            })
            .into_any_element()
    }
}
