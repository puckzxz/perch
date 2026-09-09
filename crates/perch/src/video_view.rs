//! The video pane, and the controls that sit on top of it.
//!
//! Those controls are the payoff of rendering video as a real GPUI element.
//! Embedding mpv as a child window — the other way to do this — puts the video
//! in its own OS window that always paints above everything, so nothing can
//! overlap it. Here the video is just an element, and UI composites over it
//! like any other layer.

use std::sync::Arc;

use gpui::{
    canvas, div, img, prelude::*, px, Animation, AnimationExt, Bounds, ClickEvent, Context,
    ElementId, Entity, EventEmitter, Hsla, Pixels, RenderImage, SharedString, Subscription, Task,
    Window,
};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

use crate::controls;
use crate::motion;
use crate::seek_bar;
use crate::theme;
use crate::video::{Stopped, VideoStream};

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
    /// The stream stopped and will not resume. The root handles it because
    /// what is left to do - retire this player, take streamlink down with it,
    /// and say so in the pane - is all outside the player.
    Stopped(Stopped),
}

pub struct VideoView {
    stream: VideoStream,
    /// The stream's frame as the sprite atlas knows it.
    ///
    /// Every frame from one stream carries the same `ImageId` (see
    /// `video::VideoStream::start`), so this is really "the atlas key this view
    /// owns" — kept so `on_release_in` has something to hand `drop_image`, and
    /// so `render` can tell a genuinely new frame from a repaint of the one the
    /// atlas already holds.
    current: Option<Arc<RenderImage>>,
    /// Volume before muting, so unmute restores rather than guessing.
    volume_before_mute: u8,
    volume_slider: Entity<SliderState>,
    quality: SharedString,
    /// Other qualities this channel offers, highest first.
    available: Vec<String>,
    quality_menu_open: bool,
    /// Whether the pointer is over this player, measured from the pane's own
    /// bounds rather than taken from GPUI's `on_hover`.
    ///
    /// `on_hover` answers "is this hovered *and* is nothing being dragged":
    ///
    /// ```ignore
    /// let is_hovered = has_mouse_down.borrow().is_none()
    ///     && !cx.has_active_drag()
    ///     && hitbox.is_hovered(window);
    /// ```
    ///
    /// gpui-component's `Slider` drags via `on_drag`, so working the volume
    /// slider makes every hover listener in the window report false — including
    /// this one, whose control bar contains the slider being used. It also
    /// cannot report the pointer leaving the window, because that delivers no
    /// mouse move. Asking where the pointer is fixes both.
    hovered: bool,
    /// Whether the control bar is up, and how far through fading it is.
    /// Derived from `hovered` and the quality menu by `sync_controls`.
    controls: motion::Fade,
    /// True while the player is a thumbnail on the browse page. Backgrounded
    /// players are muted and draw no controls.
    background: bool,
    /// Volume to restore when coming back to the foreground.
    volume_before_background: u8,
    /// A scrub in progress: where along the bar the pointer has dragged the
    /// thumb. The seek happens when it lets go — see `seek_bar` for why not
    /// on every move.
    scrub: Option<f32>,
    /// Where the seek bar's track was laid out last frame, so a scrub can
    /// follow the pointer after it has left the track.
    track: Option<Bounds<Pixels>>,
    _pump: Task<()>,
    /// Keeps the release hook alive; see [`VideoView::from_stream`].
    _release: Subscription,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while frames.next().await.is_some() {
                // The same wake carries both: a new frame to draw, and - once
                // - the news that there will not be another. See
                // `VideoStream::stopped`.
                let alive = this.update(cx, |this, cx| {
                    if let Some(reason) = this.stream.take_stopped() {
                        cx.emit(VideoEvent::Stopped(reason));
                    }
                    cx.notify();
                });
                if alive.is_err() {
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

        // GPUI does not refcount atlas tiles against the `Arc`:
        // `Window::drop_image` is the only thing that calls
        // `sprite_atlas.remove`, so the one tile this stream owns stays
        // resident for the life of the window unless it is freed here. This
        // view is destroyed on every ordinary action — Ctrl+W, closing a pane,
        // going back to browse, and every quality or credentials change, which
        // replace `Playing` with `Starting` — so each would otherwise leak a
        // full-size frame. `Drop` cannot do this; it has no `Window`.
        // `on_release_in` does.
        let release = cx.on_release_in(window, |this: &mut Self, window, _cx| {
            if let Some(frame) = this.current.take() {
                let _ = window.drop_image(frame);
            }
        });

        Self {
            stream,
            current: None,
            volume_before_mute: volume.max(1),
            volume_slider,
            quality,
            available,
            quality_menu_open: false,
            hovered: false,
            controls: motion::Fade::hidden(),
            background: false,
            volume_before_background: volume,
            scrub: None,
            track: None,
            _pump: pump,
            _release: release,
        }
    }

    /// Seconds into the recording, for whoever restarts this player and wants
    /// the new one to pick up where this one was. Zero on a live stream.
    pub fn position(&self) -> f64 {
        self.stream.position()
    }

    /// Skip a recording by `delta` seconds, clamped to its length. Does
    /// nothing on a live stream, which has nowhere to go.
    pub fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        let Some(timeline) = self.stream.timeline() else {
            return;
        };
        let position = self.stream.position();
        let target = (position + delta).clamp(0.0, timeline.extent(position));
        self.stream.seek_to(target);
        cx.notify();
    }

    /// The pointer went down on the seek bar at `fraction` of its length.
    fn begin_scrub(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.scrub = Some(fraction);
        self.sync_controls();
        cx.notify();
    }

    /// The pointer let go, wherever it is: seek to where the scrub got to.
    fn end_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(fraction) = self.scrub.take() else {
            return;
        };
        if let Some(timeline) = self.stream.timeline() {
            let extent = timeline.extent(self.stream.position());
            self.stream.seek_to(fraction as f64 * extent);
        }
        self.sync_controls();
        cx.notify();
    }

    /// Follow the pointer while a scrub is running, from the probe. Returns
    /// whether the thumb moved.
    fn follow_scrub(&mut self, pointer: Pixels) -> bool {
        let (Some(_), Some(track)) = (self.scrub, self.track) else {
            return false;
        };
        let fraction = seek_bar::fraction_at(&track, pointer);
        if self.scrub == Some(fraction) {
            return false;
        }
        self.scrub = Some(fraction);
        true
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

    /// Step the volume, clamped to the ends.
    ///
    /// Goes through `set_volume` rather than `apply_volume` so the slider
    /// thumb follows: `apply_volume` deliberately does not write back, because
    /// it is what an in-progress drag calls.
    pub fn nudge_volume(&mut self, delta: i16, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.stream.volume() as i16 + delta).clamp(0, 100) as u8;
        self.set_volume(next, window, cx);
    }

    pub fn toggle_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // And while the thumb is held, wherever the pointer has dragged it:
        // a bar that faded out mid-scrub would take the thumb with it.
        let visible =
            !self.background && (self.hovered || self.quality_menu_open || self.scrub.is_some());
        self.controls.set(visible)
    }

    /// Report where the pointer is, from the probe. Returns whether this needs
    /// a repaint.
    fn set_hovered(&mut self, hovered: bool) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        self.sync_controls()
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        self.stream.set_paused(!self.stream.is_paused());
        cx.notify();
    }

    /// What the pane header says about this player. Muted and paused are the
    /// two states that used to be invisible until you hovered the video - a
    /// stream saved muted opened silent with nothing on screen to say why.
    pub fn is_muted(&self) -> bool {
        self.stream.volume() == 0
    }

    pub fn is_paused(&self) -> bool {
        self.stream.is_paused()
    }

    /// Width over height of the stream itself, once a frame has decoded.
    ///
    /// The stream's shape, not the frame's size: the render size follows the
    /// pane, so sizing the pane from it would be a feedback loop, but the
    /// source's aspect is a fixed property of the broadcast.
    pub fn source_aspect(&self) -> Option<f32> {
        self.stream
            .source_size()
            .map(|(width, height)| width as f32 / height as f32)
    }

    fn quality_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = div()
            .absolute()
            .right_0()
            .flex()
            .flex_col()
            .min_w(px(120.))
            .rounded(px(theme::RADIUS_LG))
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

    /// The seek bar, on a recording. A live stream has no timeline and gets
    /// nothing here, so its control bar is exactly what it was.
    fn seek_row(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let timeline = self.stream.timeline()?;
        let position = self.stream.position();
        let extent = timeline.extent(position);
        // While the thumb is held the left-hand time follows it rather than
        // the playhead: that is the number the scrub is choosing.
        let shown = self
            .scrub
            .map(|fraction| fraction as f64 * extent)
            .unwrap_or(position);
        let state = seek_bar::State {
            played: (position / extent) as f32,
            scrub: self.scrub,
            position: seek_bar::timecode(shown).into(),
            extent: seek_bar::timecode(extent).into(),
        };
        Some(seek_bar::element(
            state,
            |this: &mut Self, fraction, _window, cx| this.begin_scrub(fraction, cx),
            |this: &mut Self, _window, cx| this.end_scrub(cx),
            |this: &mut Self, bounds| this.track = Some(bounds),
            cx,
        ))
    }

    fn control_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            // Clicks on the bar stay on the bar. Hit-testing is flat, so
            // without this a click on `pause` would also reach the pane
            // underneath, where a double-click now means fullscreen. The hover
            // probe is a canvas and sees through it, so the bar still counts
            // as "over the video" for the purpose of staying visible.
            .occlude()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .px(px(theme::PANEL_PAD))
            .py(px(theme::GAP_TIGHT))
            // Sits over live video, so it carries its own contrast rather than
            // relying on whatever happens to be on screen behind it.
            .bg(theme::overlay())
            .children(self.seek_row(cx))
            .child(self.button_row(cx))
    }

    /// Pause, mute, volume and quality: the row every stream has.
    fn button_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let volume = self.stream.volume();
        let paused = self.stream.is_paused();

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .child(
                controls::pill(
                    "pause",
                    if paused { "play" } else { "pause" },
                    controls::Variant::OnVideo,
                )
                .on_click(cx.listener(|this, _event, _window, cx| this.toggle_playback(cx))),
            )
            .child(
                controls::pill(
                    "mute",
                    if volume == 0 { "unmute" } else { "mute" },
                    controls::Variant::OnVideo,
                )
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
            .child(
                div()
                    .relative()
                    .child(
                        controls::pill("quality", self.quality.clone(), controls::Variant::OnVideo)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.quality_menu_open = !this.quality_menu_open;
                                this.sync_controls();
                                cx.notify();
                            })),
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

        // One tile for the life of the stream: every frame carries the same
        // `ImageId`, so this overwrites the pixels already in the atlas instead
        // of asking GPUI to build and throw away a texture per frame.
        //
        // Guarded on the `Arc`, not on the id: `render` runs on every window
        // draw, not every decoded frame — `impl Element for Entity<V>` has no
        // cache key — so chat traffic, the control fade and hover all arrive
        // here with the frame the atlas already holds. Without the guard each
        // of those would re-upload up to 14.7 MB. And `RenderImage`'s
        // `PartialEq` is id-only, while every frame of this stream now shares
        // one id, so comparing ids would skip *every* real frame and freeze the
        // picture. Pointer identity is the only thing that tells them apart.
        //
        // Skipping is safe whichever way the last draw went: if `update_image`
        // returned true the tile holds these bytes, and if it returned false it
        // removed the key and the paint that followed inserted these same
        // bytes. If the key is absent for any other reason — a device loss
        // clears `tiles_by_key` — `img` below re-inserts it.
        let is_new = self
            .current
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(current, &frame));
        if is_new {
            // False on the first frame, and again once a resize changed the
            // frame's size, having left the atlas with no entry for the id;
            // `img` below then inserts it the ordinary way.
            window.update_image(&frame, 0);
            self.current = Some(frame.clone());
        }

        // Measure the pane every frame: the render thread follows it, and so
        // does hover. Without the first the buffer stays at its initial size and
        // a 1440p stream is downscaled before it ever reaches the window; see
        // `hovered` for why the second is measured here rather than reported.
        // A scrub follows the pointer from here too, and needs the probe to
        // keep running: a paused recording sends no frames, and nothing else
        // would repaint while the thumb is being dragged across it.
        if self.scrub.is_some() {
            window.request_animation_frame();
        }

        let stream_size = self.stream.size_handle();
        let this = cx.entity().downgrade();
        let probe = canvas(
            move |bounds, window, cx| {
                let scale = window.scale_factor();
                let width = (f32::from(bounds.size.width) * scale).round() as u32;
                let height = (f32::from(bounds.size.height) * scale).round() as u32;
                stream_size.request(width, height);

                let pointer = window.mouse_position();
                let inside = window.is_window_hovered() && bounds.contains(&pointer);
                this.update(cx, |view: &mut Self, cx| {
                    let hovered = view.set_hovered(inside);
                    let scrubbed = view.follow_scrub(pointer.x);
                    if hovered || scrubbed {
                        cx.notify();
                    }
                })
                .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        // A flex container for the same reason the pane around it is one:
        // the frame's aspect ratio must size nothing. `img` writes the
        // frame's ratio onto its style, and as a block child with a
        // percentage height that ratio decided the height whenever the
        // percentage could not resolve - which fed the next frame's size, and
        // that frame's ratio, and so on. As a stretched flex item the image is
        // the pane's size, whatever shape the frame it holds is.
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(backdrop)
            .id("video-pane")
            // Only here to wake a repaint. Its *value* is wrong during a drag,
            // so the probe above decides; but a paused stream sends no frames,
            // and without this nothing would ask the probe to run again.
            .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
            // The gesture every player has. A single click does nothing on
            // purpose - it is how a pane is made the active one, and pausing
            // on a click would turn choosing a pane into stopping it.
            .on_click(|event: &ClickEvent, window, _cx| {
                if event.click_count() == 2 {
                    window.toggle_fullscreen();
                }
            })
            .child(probe)
            // Fade the first frames in rather than cutting from black, which
            // makes a channel switch read as deliberate instead of a glitch.
            .child(img(frame).flex_1().min_h_0().w_full().with_animation(
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
