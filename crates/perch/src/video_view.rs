//! The video pane, and the controls that sit on top of it.
//!
//! Those controls are the payoff of rendering video as a real GPUI element.
//! Embedding mpv as a child window — the other way to do this — puts the video
//! in its own OS window that always paints above everything, so nothing can
//! overlap it. Here the video is just an element, and UI composites over it
//! like any other layer.
//!
//! This file is the player: what it starts with, its sound, its hover, the
//! switch to a mini-player tile, and the picture. What is drawn over the
//! picture lives beside it, in child modules that see the player's private
//! fields: `bar`, the control bar along the bottom — its icons, and what fits
//! at the pane's width — and `menu`, the menus that bar opens, which one is
//! open, and how their rows take a press.
//!
//! What the bar offers that is not the player's to do — the pane's chat,
//! opening it on twitch.tv, copying its link — it asks the root for, as
//! [`VideoEvent::Pane`], the same way it asks for a new quality.
//!
//! A player draws nothing at all until its picture arrives, not even a
//! backdrop: whoever holds it says what is happening meanwhile — the pane's
//! status screen, under it, or a mini-player tile's word — and paints the
//! black behind it. So the first frame fades in over what was being waited
//! for rather than out of black; see [`VideoView::covers`].

mod bar;
mod menu;

pub use menu::Menu;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, img, prelude::*, Animation, AnimationExt, Bounds, ClickEvent, Context, ElementId,
    Entity, EventEmitter, FocusHandle, MouseDownEvent, Pixels, Point, RenderImage, SharedString,
    Subscription, Task, Window,
};
use gpui_component::slider::{SliderEvent, SliderState};

use crate::loudness::Loudness;
use crate::motion;
use crate::seek_bar;
use crate::theme;
use crate::video::{Stopped, VideoStream};
use crate::watch::PaneAction;

pub enum VideoEvent {
    /// The user changed volume; worth persisting to settings.
    VolumeChanged(u8),
    /// The user picked a quality from the pane's menu: a rendition, or `None`
    /// to hand the pane back to whatever the settings pick at its size.
    /// Switching means restarting streamlink, so the root handles it rather
    /// than the player.
    QualityRequested(Option<String>),
    /// The stream stopped and will not resume. The root handles it because
    /// what is left to do - retire this player, take streamlink down with it,
    /// and say so in the pane - is all outside the player.
    Stopped(Stopped),
    /// Something on the bar or in More that is the pane's rather than the
    /// player's: its chat, its link. Answered by `RootView::on_pane_action`,
    /// as the pane's own controls are, for the pane this player belongs to.
    Pane(PaneAction),
}

/// What the bar's chat glyph offers: to hide the pane's chat, to show it, or
/// nothing, for a recording with no chat to replay — drawn still, so the
/// right-hand cluster keeps its shape and its place.
///
/// A mirror, and the only one the bar keeps: the pane's `chat_hidden` and
/// `chat` are the root's, and the player cannot see them. Written in exactly
/// two places — [`Start::chat`] when the player is made, and
/// `RootView::toggle_chat`, the one thing that changes `chat_hidden` after a
/// pane opens, through [`VideoView::set_chat`]. Anything else that comes to
/// write `chat_hidden` has to call `set_chat` too, or the glyph will offer
/// the opposite of what a press does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatButton {
    /// Chat is beside the picture: the glyph hides it.
    Shown,
    /// Chat is hidden: the glyph brings it back.
    Hidden,
    /// There is no chat to show — a highlight or an upload.
    Unavailable,
}

impl ChatButton {
    /// The glyph for a pane whose chat is `chat_hidden`, and which has one
    /// at all. With none, what was saved for the channel does not matter.
    pub fn of(chat_hidden: bool, has_chat: bool) -> Self {
        match (has_chat, chat_hidden) {
            (false, _) => ChatButton::Unavailable,
            (true, true) => ChatButton::Hidden,
            (true, false) => ChatButton::Shown,
        }
    }
}

/// What a pane's quality menu offers, and which of it is chosen.
pub struct Qualities {
    /// The rendition playing, which is what the menu's button says.
    pub playing: SharedString,
    /// Every rendition the stream offers, highest first.
    pub available: Vec<String>,
    /// What the settings pick when a pane is not told otherwise, in the words
    /// the settings sheet shows it in (`settings_view::quality_label`):
    /// `Auto (matches the video pane)`, `Best available`, `1080p`.
    pub default: SharedString,
    /// Whether this pane was told otherwise — a rendition picked from its own
    /// menu, which holds until the pane closes or the default is chosen again.
    pub picked: bool,
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
    /// When `current` was first filled: the moment the first frame began
    /// to fade in, which [`covers`](Self::covers) counts from.
    first_frame: Option<Instant>,
    /// The level the user chose, and whether Mute all is holding the pane
    /// silent on top of it. What mpv hears is read from here and nowhere
    /// else; see `loudness`.
    loudness: Loudness,
    volume_slider: Entity<SliderState>,
    qualities: Qualities,
    /// The menu open over the control bar, if any: one at a time, so opening
    /// one is closing whichever was open (`menu::toggled`).
    menu: Option<Menu>,
    /// Whether the run of presses going on began on a menu row, whose later
    /// presses — the second of a double-click — go nowhere; see
    /// `VideoView::run_guard`.
    row_run: bool,
    /// The root's focus, which a press on the bar or on a menu hands the keys
    /// back to; see `return_keys`.
    root_focus: FocusHandle,
    /// What the bar's chat glyph offers; see [`ChatButton`].
    chat: ChatButton,
    /// What the bar has room for at the pane's width, measured by the probe
    /// (`bar::fit`): the volume figure and slider, and the quality pill,
    /// which folds into More when it does not fit.
    fit: bar::Fit,
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
    /// Derived from `hovered` and the open menu by `sync_controls`.
    controls: motion::Fade,
    /// True while the player is a tile in the mini player on the browse page.
    /// Presentation only: a compact player draws no control bar, answers no
    /// hover, labels no seek bar and does not go fullscreen on a double-click,
    /// because the tile is a way back to the watch page rather than a player
    /// of its own. What it sounds like is `loudness`, and nothing here.
    compact: bool,
    /// A scrub in progress: where along the bar the pointer has dragged the
    /// thumb. The seek happens when it lets go — see `seek_bar` for why not
    /// on every move.
    scrub: Option<f32>,
    /// Where the seek bar's track was laid out last frame, so a scrub can
    /// follow the pointer after it has left the track.
    track: Option<Bounds<Pixels>>,
    /// Where along the seek bar the pointer is, while it is over the bar:
    /// what the label above it names. Measured by the probe against `track`,
    /// for the reason `hovered` is.
    pointing: Option<f32>,
    _pump: Task<()>,
    /// Keeps the release hook alive; see [`VideoView::from_stream`].
    _release: Subscription,
}

impl EventEmitter<VideoEvent> for VideoView {}

/// How a player is born, which is how its pane already is: a player rebuilt
/// while you browse — a quality change, the re-pick after a resize — has to
/// come up as the tile it is replacing, not as a watch-page player.
pub struct Start {
    /// Drawn as a tile in the mini player; see `VideoView::compact`.
    pub compact: bool,
    /// Held silent by Mute all; see `Loudness`.
    pub quiet: bool,
    /// The root's focus handle, kept as `VideoView::root_focus`.
    pub focus: FocusHandle,
    /// What the pane's chat is at the start; see [`ChatButton`].
    pub chat: ChatButton,
}

impl VideoView {
    /// Takes an already-started stream so a failure to open can be shown in the
    /// window rather than panicking inside entity construction.
    ///
    /// `level` is the level the pane opens at, which the stream was started at
    /// too unless `start.quiet`, in which case the stream started silent and
    /// the slider still shows `level`. Taken rather than read off the stream
    /// for that reason.
    pub fn from_stream(
        stream: VideoStream,
        mut frames: futures::channel::mpsc::Receiver<()>,
        qualities: Qualities,
        level: u8,
        start: Start,
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

        let loudness = Loudness::new(level, start.quiet);
        let volume_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(loudness.level() as f32)
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
            first_frame: None,
            loudness,
            volume_slider,
            qualities,
            menu: None,
            row_run: false,
            root_focus: start.focus,
            chat: start.chat,
            // Until the probe has measured the pane: the bar is hidden on
            // the first frame, and the probe's first pass corrects it.
            fit: bar::Fit::EVERYTHING,
            hovered: false,
            controls: motion::Fade::hidden(),
            compact: start.compact,
            scrub: None,
            track: None,
            pointing: None,
            _pump: pump,
            _release: release,
        }
    }

    /// Seconds into the recording, for whoever restarts this player and wants
    /// the new one to pick up where this one was. Zero on a live stream.
    pub fn position(&self) -> f64 {
        self.stream.position()
    }

    /// How long the recording is, as the player measures it — the playlist's
    /// length, which grows while a broadcast is still being made. `None` on a
    /// live stream.
    pub fn timeline(&self) -> Option<seek_bar::Timeline> {
        self.stream.timeline()
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

    /// Note where along the seek bar the pointer is, from the probe — `None`
    /// for a pointer that is not over this player at all. Returns whether the
    /// label moved.
    fn follow_pointer(&mut self, pointer: Option<Point<Pixels>>) -> bool {
        let pointing = match (pointer, self.track) {
            // A compact player draws no bar, so wherever its track was last
            // laid out is somewhere else on the screen by now.
            (Some(pointer), Some(track)) if !self.compact => seek_bar::hover_at(&track, pointer),
            _ => None,
        };
        if self.pointing == pointing {
            return false;
        }
        self.pointing = pointing;
        true
    }

    /// Set volume without writing back to the slider, which is already where
    /// the user put it. Writing back would fight an in-progress drag.
    ///
    /// Every path that reports a level to be remembered comes through here,
    /// and only a level the user chose does: the hush has its own way in,
    /// [`set_hushed`](Self::set_hushed), which reports nothing.
    fn apply_volume(&mut self, volume: u8, cx: &mut Context<Self>) {
        self.loudness.set(volume);
        self.stream.set_volume(self.loudness.audible());
        cx.emit(VideoEvent::VolumeChanged(self.loudness.level()));
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
        let next = (self.loudness.level() as i16 + delta).clamp(0, 100) as u8;
        self.set_volume(next, window, cx);
    }

    /// `M`: to silence and back. Through `set_volume`, so the slider follows,
    /// and so the first press on a pane Mute all is holding ends the hold the
    /// way any deliberate level does — said to the root, which stops counting
    /// the pane as quiet.
    pub fn toggle_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_volume(self.loudness.toggled(), window, cx);
    }

    /// Move the player between the watch page and a tile in the mini player.
    ///
    /// How it is drawn and nothing else: the pane keeps its sound, which is
    /// the point of the mini player. Everything the pointer was doing to the
    /// big player is let go here, because the tile will never hear the
    /// release — a held scrub in particular would go on asking for a frame
    /// every frame for as long as the tile was up.
    ///
    /// The bar's fade starts over, hidden, rather than being set hidden. The
    /// tile never draws the bar, and gpui keeps an animation's state only
    /// from one frame to the next, so a fade that came back with its history
    /// would replay its last flip from the start on the first frame of the
    /// big player: the bar flashing up and fading away over the picture.
    pub fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact == compact {
            return;
        }
        self.compact = compact;
        self.menu = None;
        self.row_run = false;
        self.hovered = false;
        self.pointing = None;
        self.scrub = None;
        self.track = None;
        self.controls = motion::Fade::hidden();
        cx.notify();
    }

    /// Hold the pane silent for Mute all, or let it go.
    ///
    /// Round `apply_volume` on purpose: a hush is not a level anybody chose,
    /// so it reports nothing to be remembered, and it leaves the slider where
    /// the user put it — which is also why this needs no `Window`.
    pub fn set_hushed(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let changed = if quiet {
            self.loudness.hush()
        } else {
            self.loudness.unhush()
        };
        if changed {
            self.stream.set_volume(self.loudness.audible());
            cx.notify();
        }
    }

    /// Recompute whether the control bar should be up, and report whether that
    /// changed anything.
    ///
    /// It stays up while a menu is open even after the pointer leaves, or
    /// reaching for an option would dismiss the menu on the way. That is also
    /// what lets the palette open a menu with the pointer nowhere near.
    fn sync_controls(&mut self) -> bool {
        // And while the thumb is held, wherever the pointer has dragged it:
        // a bar that faded out mid-scrub would take the thumb with it.
        let visible =
            !self.compact && (self.hovered || self.menu.is_some() || self.scrub.is_some());
        self.controls.set(visible)
    }

    /// Hand the keys back to the root, for a press on the control bar or on
    /// a menu.
    ///
    /// Both block the pointer from what is under them, the root included,
    /// and the root's `track_focus` takes focus back only on a press it
    /// hears. Without this, a cursor left in the title bar's search box would
    /// keep the keys through a press on the bar, and the next `Space` would
    /// type a space into the search rather than pause.
    fn return_keys(&mut self, _: &MouseDownEvent, window: &mut Window, _: &mut Context<Self>) {
        self.root_focus.focus(window);
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
    /// Silent for either reason: muted by hand, or held by Mute all.
    pub fn is_muted(&self) -> bool {
        self.loudness.audible() == 0
    }

    pub fn is_paused(&self) -> bool {
        self.stream.is_paused()
    }

    /// The qualities this stream offers, highest first, and the one playing:
    /// what a re-pick after the pane changes size chooses from.
    pub fn available(&self) -> &[String] {
        &self.qualities.available
    }

    pub fn quality(&self) -> &str {
        &self.qualities.playing
    }

    /// Whether this pane's quality was picked from its own menu, for a choice
    /// that did not need a new player to take effect.
    pub fn set_picked(&mut self, picked: bool, cx: &mut Context<Self>) {
        self.qualities.picked = picked;
        cx.notify();
    }

    /// What the pane's chat now is, after `RootView::toggle_chat` changed
    /// it; see [`ChatButton`] for why this is the only other way in.
    pub fn set_chat(&mut self, chat: ChatButton, cx: &mut Context<Self>) {
        if self.chat != chat {
            self.chat = chat;
            cx.notify();
        }
    }

    /// Note what the bar has room for, from the probe. Returns whether that
    /// changed, the only time a repaint is worth it: the width moves with
    /// every pixel of a window being dragged, and what fits changes a few
    /// times across all of them.
    fn set_fit(&mut self, fit: bar::Fit) -> bool {
        if self.fit == fit {
            return false;
        }
        self.fit = fit;
        true
    }

    /// Whether a frame has decoded, and so whether `render` draws the picture
    /// and the bar over it rather than nothing at all. Asks what `render`
    /// asks, so the two cannot disagree; once true it stays true for the life
    /// of the stream.
    pub fn has_picture(&self) -> bool {
        self.stream.latest_frame().is_some()
    }

    /// Whether the picture covers everything under it: its first frame has
    /// been drawn and has finished fading in. Until then the pane goes on
    /// drawing what it was waiting for under the player, which draws nothing
    /// before its first frame and only part of it during the fade — so the
    /// picture arrives over the poster, never over black. Once true it stays
    /// true for the life of this player; a quality change makes a new one,
    /// which starts over.
    pub fn covers(&self) -> bool {
        covered(self.first_frame.map(|first| first.elapsed()))
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
}

/// Whether a picture whose first frame was drawn `since_first` ago covers
/// what is under it: once the first frame's fade-in, [`theme::MOTION_VIDEO`],
/// has run its course, and never before there is a first frame.
fn covered(since_first: Option<Duration>) -> bool {
    since_first.is_some_and(|since| since >= theme::MOTION_VIDEO)
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(frame) = self.stream.latest_frame() else {
            // Nothing, and no backdrop: whoever holds the player says what
            // is happening until its picture arrives, under it — the pane's
            // status screen, the tile's word — and that has to show through.
            // A player that painted its own black here, or a word of its
            // own, hid the poster the pane was showing the moment the
            // player existed, seconds before there was a picture to replace
            // it with.
            return div().size_full().into_any_element();
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
            self.first_frame.get_or_insert_with(Instant::now);
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
                // In logical pixels, the bar's own: the bar spans the pane.
                let fit = bar::fit(f32::from(bounds.size.width), bar::RIGHT_BUTTONS);
                this.update(cx, |view: &mut Self, cx| {
                    let hovered = view.set_hovered(inside);
                    let scrubbed = view.follow_scrub(pointer.x);
                    let pointed = view.follow_pointer(inside.then_some(pointer));
                    let fitted = view.set_fit(fit);
                    if hovered || scrubbed || pointed || fitted {
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
        //
        // No backdrop here either. Both of the player's containers — the
        // pane, a mini-player tile — paint the player's black themselves, so
        // the letterbox is black once the picture covers the pane; until it
        // does, it is the poster under the fading first frame.
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .id("video-pane")
            // Only here to wake a repaint. Its *value* is wrong during a drag,
            // so the probe above decides; but a paused stream sends no frames,
            // and without this nothing would ask the probe to run again.
            .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
            // The gesture every player has. A single click does nothing on
            // purpose - it is how a pane is made the active one, and pausing
            // on a click would turn choosing a pane into stopping it. Not on
            // a compact tile, whose click is the way back to the watch page.
            // A double-click whose first press chose a menu row never gets
            // here: `run_guard`, below, stops the second press.
            .when(!self.compact, |pane| {
                pane.on_click(|event: &ClickEvent, window, _cx| {
                    if event.click_count() == 2 {
                        window.toggle_fullscreen();
                    }
                })
            })
            .child(probe)
            // Ahead of the bar and its menus, so it hears a press before
            // anything on them does; see `run_guard`. A tile has no menus.
            .when(!self.compact, |pane| pane.child(Self::run_guard(cx)))
            // Fade the first frames in rather than cutting to them, which
            // makes a channel switch read as deliberate instead of a glitch;
            // the poster under it is what it fades in over.
            .child(img(frame).flex_1().min_h_0().w_full().with_animation(
                ElementId::from("video-fade-in"),
                Animation::new(theme::MOTION_VIDEO),
                |element, delta| element.opacity(delta),
            ))
            .when(!self.compact, |pane| {
                // Hidden until the pointer is over the video, so nothing covers
                // the picture while you are just watching - and faded rather
                // than cut, because over a moving image a hard switch reads as
                // part of the video instead of a response to the pointer.
                pane.child(
                    self.controls.apply(
                        "controls",
                        theme::MOTION_HOVER,
                        div()
                            .absolute()
                            .inset_0()
                            .child(self.control_bar(window, cx)),
                    ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture covers the pane only once its first frame has faded all
    /// the way in; before a first frame, and during the fade, what is under
    /// it still shows.
    #[test]
    fn the_picture_covers_only_once_faded_in() {
        assert!(!covered(None), "no frame yet");
        assert!(!covered(Some(Duration::ZERO)), "the first frame just drawn");
        assert!(!covered(Some(theme::MOTION_VIDEO / 2)), "half faded in");
        assert!(covered(Some(theme::MOTION_VIDEO)));
        assert!(covered(Some(Duration::from_secs(60))));
    }

    /// A recording with no chat to replay offers none, whatever the channel
    /// was saved as: its glyph is the still one either way.
    #[test]
    fn the_chat_glyph_follows_the_pane() {
        assert_eq!(ChatButton::of(false, true), ChatButton::Shown);
        assert_eq!(ChatButton::of(true, true), ChatButton::Hidden);
        for hidden in [false, true] {
            assert_eq!(ChatButton::of(hidden, false), ChatButton::Unavailable);
        }
    }
}
