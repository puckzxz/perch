//! The video pane, and the controls that sit on top of it.
//!
//! Those controls are the payoff of rendering video as a real GPUI element.
//! Embedding mpv as a child window — the other way to do this — puts the video
//! in its own OS window that always paints above everything, so nothing can
//! overlap it. Here the video is just an element, and UI composites over it
//! like any other layer.
//!
//! This file is the player: what it starts with, its sound, its hover, where
//! it is drawn — a pane, a mini-player tile, a window of its own, or nowhere
//! while another pane has the watch page, its [`Place`] — and the picture,
//! with the two gestures it answers, `Alt` and the wheel for the level and a
//! middle press for mute (`picture_gestures`). What is drawn over the
//! picture lives beside it, in child modules that see the player's private
//! fields: `bar`, the control bar along the bottom — its icons, and what fits
//! at the pane's width — and `menu`, the menus that bar opens, which one is
//! open, and how their rows take a press.
//!
//! What the bar offers that is not the player's to do — the pane's chat,
//! opening it on twitch.tv, copying its link, popping it out or bringing it
//! back, giving it the watch page, going back along a live broadcast's
//! timeline into its recording and from there back to live — it asks the
//! root for, as [`VideoEvent::Pane`], the same way it asks for a new
//! quality.
//!
//! A player draws nothing at all until its picture arrives, not even a
//! backdrop: whoever holds it says what is happening meanwhile — the pane's
//! status screen, under it, or a mini-player tile's word — and paints the
//! black behind it. So the first frame fades in over what was being waited
//! for rather than out of black; see [`VideoView::covers`].
//!
//! A player belongs to no window. Its frames wake it through a task of its
//! own and its tile is freed in every window when it goes, so the one view
//! can move between the main window and a pop-out — through
//! [`VideoView::set_place`] and nothing else — and be drawn by exactly one
//! window at a time; see `crate::stage`.
//!
//! Nor is a player one stream for life. A new rendition is started beside
//! the stream on screen, inside the same view, and takes over in place once
//! it is ready — `swap`, which also routes each stream's wakes — so a
//! quality change keeps the picture, the fade that brought it in, the level
//! and everything holding the view.

mod bar;
mod menu;
mod swap;

pub(crate) use menu::rest_of_row_run;
pub use menu::Menu;
pub use swap::{route, Wake, SWAP_LEAD};

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gpui::{
    canvas, div, img, prelude::*, Animation, AnimationExt, Bounds, ClickEvent, Context, Div,
    ElementId, Entity, EventEmitter, FocusHandle, MouseButton, MouseDownEvent, Pixels, Point,
    RenderImage, ScrollWheelEvent, SharedString, Stateful, Subscription, Task, Window,
};
use gpui_component::slider::{SliderEvent, SliderState};

use crate::ad_break;
use crate::controls;
use crate::keys;
use crate::loudness::{HearOnly, Loudness};
use crate::motion;
use crate::rewind;
use crate::seek_bar;
use crate::stage::{MaximizeButton, Place};
use crate::theme;
use crate::video::{SizeHandle, Stopped, VideoStream};
use crate::watch::PaneAction;
use crate::wheel::Wheel;

pub enum VideoEvent {
    /// The user changed volume; worth persisting to settings.
    VolumeChanged(u8),
    /// The user picked a quality from the pane's menu: a rendition, or `None`
    /// to hand the pane back to whatever the settings pick at its size.
    /// Switching means starting streamlink again, so the root handles it
    /// rather than the player.
    QualityRequested(Option<String>),
    /// The stream stopped and will not resume. The root handles it because
    /// what is left to do - retire this player, take streamlink down with it,
    /// and say so in the pane - is all outside the player.
    Stopped(Stopped),
    /// Something on the bar or in More that is the pane's rather than the
    /// player's: its chat, its link, its window, the maximize. Answered by
    /// `RootView::on_pane_action`, as the pane's own controls are, for the
    /// pane this player belongs to.
    Pane(PaneAction),
    /// The player started beside this one for the pane's start `generation`
    /// has taken over (`swap`), and the stream it replaced has been stopped.
    /// The root keeps that start's streamlink as the pane's and drops the
    /// old one's, which until now was feeding the old player.
    Swapped { generation: u64 },
    /// The player started beside this one for `generation`, at `quality`,
    /// was given up on; the one on screen plays on as it was. The root drops
    /// that start's streamlink, and says so when the rendition was picked by
    /// hand.
    SwapFailed {
        generation: u64,
        quality: SharedString,
    },
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

/// A quality change somebody picked from a pane's menu, while it resolves
/// beside the picture and until its player takes over: what the bar says in
/// the meantime, so a pick that takes seconds is seen to be under way.
///
/// The root's fact, carried by the pending start itself
/// (`watch::Restart::Pick`), and mirrored here on [`ChatButton`]'s pattern:
/// written in exactly two places — [`Start::switching`] when the player is
/// made, and [`VideoView::set_switching`], which only `RootView::set_pending`
/// calls, the one write of the pane's pending start. Not the view's own
/// `swap::Pending`, which comes seconds after the press, when streamlink
/// has resolved, and which `begin_swap` drops and makes again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switching {
    /// The rendition it asked for, which the pill names meanwhile: the
    /// menu's rendition, or for the settings' row what the settings pick
    /// at the pane's size now (`RootView::request_quality`).
    pub to: SharedString,
    /// Whether it was the settings' row, handing the pane back, rather than
    /// a rendition: which row of the menu is marked meanwhile.
    pub default: bool,
    /// When it was picked: what the swap's log line counts the wait somebody
    /// actually sat through from, beside its own count from the new player's
    /// start (`swap`'s promote).
    pub since: Instant,
}

pub struct VideoView {
    /// The pane's key (`watch::Slot::key`), which names this player in the
    /// log. A pane's key never changes in place, so neither does this.
    key: SharedString,
    /// The stream on screen.
    stream: VideoStream,
    /// The root's number for the start of the pane's stream that `stream`
    /// came from: what its frame wakes carry, so they reach it and not a
    /// stream that replaced it (`swap::route`).
    generation: u64,
    /// A new rendition getting ready beside `stream`, to take over in place;
    /// see `swap`.
    pending: Option<swap::Pending>,
    /// The stream's frame as the sprite atlas knows it.
    ///
    /// Every frame from one stream carries the same `ImageId` (see
    /// `video::VideoStream::start`), so this is really "the atlas key this view
    /// owns" — kept so the release hook and [`set_place`](Self::set_place)
    /// have something to hand `drop_image`, and so `render` can tell a
    /// genuinely new frame from a repaint of the one the atlas already holds.
    ///
    /// Each window has an atlas of its own, so this is the tile in the window
    /// that drew the view last. `set_place` empties it, and the next draw —
    /// in whichever window — uploads the frame afresh. A swap's new stream
    /// has an id of its own, and `render` frees the old one's tile when the
    /// first frame with the new id arrives.
    current: Option<Arc<RenderImage>>,
    /// When `current` was first filled: the moment the first frame began
    /// to fade in, which [`covers`](Self::covers) counts from.
    first_frame: Option<Instant>,
    /// The level the user chose, and whether Mute all is holding the pane
    /// silent on top of it. What mpv hears is read from here and nowhere
    /// else; see `loudness`.
    loudness: Loudness,
    volume_slider: Entity<SliderState>,
    /// What a gliding wheel has gathered towards its next step of volume,
    /// under `Alt`; see `crate::wheel`.
    wheel: Wheel,
    /// The bar held up for a moment after the wheel last changed the level,
    /// so the slider and the figure are seen moving even once the pointer
    /// has left: the one-shot timer that lets it go
    /// ([`theme::VOLUME_LINGER`]), while it runs. Each step of the wheel
    /// replaces it, which drops — calls off — the one before.
    volume_linger: Option<Task<()>>,
    /// What More's row for hearing one pane alone offers; see
    /// [`HearOnly`]. A mirror, on [`ChatButton`]'s pattern, of the panes'
    /// hushes, which are the root's: written in exactly two places —
    /// [`Start::hear_only`] when the player is made, and
    /// [`set_hear_only`](Self::set_hear_only), which only
    /// `RootView::sync_hear_only` calls, after every change of a pane's hush
    /// and from `restage`, which every change of which panes there are ends
    /// in.
    hear_only: HearOnly,
    /// When the stream on screen last sent a frame: what the next one is
    /// measured against to say whether the picture has moved again after
    /// standing still (`ad_break::frames_resumed`).
    last_frame: Option<Instant>,
    /// When the picture last moved again after standing still, its first
    /// frame included: what ends a pane's ad-break notice
    /// (`ad_break::AdBreak::over`), read by the root as it ticks.
    resumed_at: Option<Instant>,
    qualities: Qualities,
    /// The menu open over the control bar, if any: one at a time, so opening
    /// one is closing whichever was open (`menu::toggled`).
    menu: Option<Menu>,
    /// Whether the run of presses going on began on a menu row, or on the
    /// bar's maximize control, whose later presses — the second of a
    /// double-click — go nowhere; see `VideoView::run_guard`. Both change
    /// what is under the pointer before the run is over.
    row_run: bool,
    /// The focus of the window drawing this player — the root's in the main
    /// window, the pop-out's own in a pop-out — which a press on the bar or
    /// on a menu hands the keys back to; see `return_keys`. Changed with the
    /// window, by [`set_place`](Self::set_place).
    root_focus: FocusHandle,
    /// What the bar's chat glyph offers; see [`ChatButton`].
    chat: ChatButton,
    /// What the bar's maximize control offers, or More's row once it has
    /// folded: to give the pane the watch page, to show every pane again,
    /// or nothing.
    ///
    /// A mirror, on [`ChatButton`]'s pattern: which pane is maximized is the
    /// root's (`crate::stage`), and the player cannot see it. Written in
    /// exactly two places — [`Start::maximize`] when the player is made, and
    /// [`set_maximize`](Self::set_maximize), which only `RootView::restage`
    /// calls, the funnel every change of the stage ends in — so a change of
    /// the maximize, of which panes there are or of what is popped out
    /// cannot leave the control offering the opposite of what a press does.
    maximize: MaximizeButton,
    /// The quality change somebody picked, while it is under way: the pill
    /// names it and breathes, the menu marks its row, and the bar stays up
    /// (`sync_controls`). See [`Switching`] for why it is a mirror, and why
    /// of the root's pending start rather than of `pending` here.
    switching: Option<Switching>,
    /// The bar held up for a moment once a switch has ended, either way, so
    /// what it came to can be read: the one-shot timer that lets it go
    /// ([`theme::SWITCH_LINGER`]), while it runs. Started by
    /// [`set_switching`](Self::set_switching), and dropped — called off — by
    /// the next switch.
    linger: Option<Task<()>>,
    /// What the bar has room for at the pane's width, measured by the probe
    /// (`bar::fit`): the volume figure and slider, and the quality pill and
    /// the maximize control, which fold into More when they do not fit.
    fit: bar::Fit,
    /// Whether the pane is wide enough for a live stream's timeline, measured
    /// by the probe with `fit` (`bar::timeline_fits`). A recording's seek row
    /// is drawn whatever the width, as it always was.
    timeline_fits: bool,
    /// When the live broadcast on screen began, which its timeline runs from
    /// to now; `None` on a recording, and on a live stream no list has said
    /// it of, which then has no timeline.
    ///
    /// A mirror, on [`ChatButton`]'s pattern: the lists are the root's.
    /// Written in exactly two places — [`Start::live_since`] when the player
    /// is made, and [`set_live_since`](Self::set_live_since), which only
    /// `RootView::sync_live_since` calls, after every answer from the worker.
    live_since: Option<DateTime<Utc>>,
    /// Whether the recording on screen is the archive of a broadcast still
    /// going on, which the bar then offers the way back to the live edge
    /// from (`LIVE`, `PaneAction::BackToLive`); always false on a live
    /// stream.
    ///
    /// A mirror on the same pattern, of the same lists: written in exactly
    /// two places — [`Start::back_to_live`] when the player is made, and
    /// [`set_back_to_live`](Self::set_back_to_live), which only
    /// `RootView::sync_back_to_live` calls, after every answer from the
    /// worker. A channel that goes off takes it away at the next answer
    /// that says so; and the bar draws it only while the archive is still
    /// growing, the player's own word that the broadcast goes on.
    back_to_live: bool,
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
    /// Derived from `hovered`, the open menu, a scrub and a switch under way
    /// (`switching`, `linger`) by `sync_controls`.
    controls: motion::Fade,
    /// Where the player is drawn. Presentation only: a tile draws no control
    /// bar, answers no hover and labels no seek bar, because the tile is a
    /// way back to the watch page rather than a player of its own; a player
    /// offstage is drawn by nothing, and is a tile in all of that; only a
    /// pane opens menus and goes fullscreen on a double-click; and a pop-out
    /// is dragged by its picture. What it sounds like is `loudness`, and
    /// nothing here. Changed by [`set_place`](Self::set_place) alone.
    place: Place,
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
    /// The window that drew this player last, since its last change of
    /// place: what `render` holds the one-window rule to, in debug builds
    /// only. Freeing the tile needs no record of it — `drop_image` visits
    /// every window — so a release build keeps none.
    #[cfg(debug_assertions)]
    drawn_in: Option<gpui::AnyWindowHandle>,
    /// The frame pump of `stream`; a swap hands over the new stream's.
    _pump: Task<()>,
    /// Keeps the release hook alive; see [`VideoView::from_stream`].
    _release: Subscription,
}

impl EventEmitter<VideoEvent> for VideoView {}

/// How a player is born, which is how its pane already is: a player rebuilt
/// while you browse — a quality change from the settings, a restart of a pane
/// with no picture yet — has to come up as the tile it is replacing, not as
/// a watch-page player. A rendition swapped in place is no new player, and
/// keeps all of this (`swap`).
pub struct Start {
    /// The pane's key, which names the player in the log; see
    /// `VideoView::key`.
    pub key: SharedString,
    /// Where it is drawn; see `VideoView::place`.
    pub place: Place,
    /// Held silent by Mute all; see `Loudness`.
    pub quiet: bool,
    /// The focus of the window it is drawn in, kept as
    /// `VideoView::root_focus`.
    pub focus: FocusHandle,
    /// What the pane's chat is at the start; see [`ChatButton`].
    pub chat: ChatButton,
    /// What its maximize control offers at the start; see
    /// `VideoView::maximize`.
    pub maximize: MaximizeButton,
    /// What a pick is switching the pane to at the start, if one is under
    /// way; see [`Switching`]. Almost always none: a new player is a cold
    /// start, which ends whatever was resolving beside the old one.
    pub switching: Option<Switching>,
    /// When a live broadcast began, for its timeline; see
    /// `VideoView::live_since`.
    pub live_since: Option<DateTime<Utc>>,
    /// Whether a recording's broadcast is still going on, for its way back
    /// to live; see `VideoView::back_to_live`.
    pub back_to_live: bool,
    /// What More offers about hearing one pane alone at the start; see
    /// `VideoView::hear_only`.
    pub hear_only: HearOnly,
}

impl VideoView {
    /// Takes an already-started stream so a failure to open can be shown in the
    /// window rather than panicking inside entity construction.
    ///
    /// `level` is the level the pane opens at, which the stream was started at
    /// too unless `start.quiet`, in which case the stream started silent and
    /// the slider still shows `level`. Taken rather than read off the stream
    /// for that reason.
    ///
    /// `generation` is the root's number for the start the stream came from,
    /// which its frame wakes carry; see `swap::route`.
    pub fn from_stream(
        stream: VideoStream,
        frames: futures::channel::mpsc::Receiver<()>,
        qualities: Qualities,
        level: u8,
        generation: u64,
        start: Start,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = Self::pump(frames, generation, cx);

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
        // going back to browse, and every cold restart, which replaces
        // `Playing` with `Starting` — so each would otherwise leak a
        // full-size frame. `Drop` cannot do this; it has no `App`. (A
        // rendition swapped in place keeps the view; `render` frees the old
        // stream's tile instead.)
        //
        // Through `App::drop_image`, which visits every window, rather than
        // the one window the view was made in: the view may be drawn in a
        // pop-out by now, and its tile is in that window's atlas. A release
        // runs as effects are flushed, after whatever window was being
        // updated has been put back, so none is skipped as leased.
        let release = cx.on_release(|this: &mut Self, cx| {
            if let Some(frame) = this.current.take() {
                cx.drop_image(frame, None);
            }
        });

        let mut view = Self {
            key: start.key,
            stream,
            generation,
            pending: None,
            current: None,
            first_frame: None,
            loudness,
            volume_slider,
            wheel: Wheel::default(),
            volume_linger: None,
            hear_only: start.hear_only,
            last_frame: None,
            resumed_at: None,
            qualities,
            menu: None,
            row_run: false,
            root_focus: start.focus,
            chat: start.chat,
            maximize: start.maximize,
            switching: start.switching,
            linger: None,
            // Until the probe has measured the pane: the bar is hidden on
            // the first frame, and the probe's first pass corrects it.
            fit: bar::Fit::EVERYTHING,
            timeline_fits: true,
            live_since: start.live_since,
            back_to_live: start.back_to_live,
            hovered: false,
            controls: motion::Fade::hidden(),
            place: start.place,
            scrub: None,
            track: None,
            pointing: None,
            #[cfg(debug_assertions)]
            drawn_in: None,
            _pump: pump,
            _release: release,
        };
        // Up from the start if a pick is under way (`Start::switching`).
        view.sync_controls();
        view
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
        self.seek_to(target);
        cx.notify();
    }

    /// Send the recording on screen to `secs`: every seek of the user's
    /// comes through here. A rendition getting ready beside it is not sent
    /// along — it is lined up with wherever the pane ends up, from scratch
    /// (`swap`) — but takes the seek over if it takes over before the player
    /// on screen has applied it (`swap`'s promote).
    fn seek_to(&mut self, secs: f64) {
        self.stream.seek_to(secs);
        self.retarget_swap();
    }

    /// The pointer went down on the seek bar at `fraction` of its length.
    fn begin_scrub(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.scrub = Some(fraction);
        self.sync_controls();
        cx.notify();
    }

    /// The pointer let go, wherever it is: seek to where the scrub got to.
    ///
    /// On a live stream's timeline, ask the root to go back to that moment
    /// of the broadcast (`PaneAction::Rewind`), which replaces the pane with
    /// its recording; a release within `rewind::EDGE_SECS` of the live edge
    /// asks for nothing, and the pane plays on live.
    fn end_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(fraction) = self.scrub.take() else {
            return;
        };
        if let Some(timeline) = self.stream.timeline() {
            let extent = timeline.extent(self.stream.position());
            self.seek_to(fraction as f64 * extent);
        } else if let Some(since) = self.live_since {
            if let Some(at) = rewind::pressed_at(since, fraction, Utc::now()) {
                let moment = rewind::Moment { at, since };
                cx.emit(VideoEvent::Pane(PaneAction::Rewind(moment)));
            }
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
            // A tile draws no bar, so wherever its track was last laid out
            // is somewhere else on the screen by now.
            (Some(pointer), Some(track)) if draws_bar(self.place) => {
                seek_bar::hover_at(&track, pointer)
            }
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

    /// The wheel over the picture: with `Alt` held, the level a step at a
    /// time, through `nudge_volume` as a volume key goes, so the slider
    /// follows and the channel remembers it; without, nothing, and the wheel
    /// goes on to whatever is under the player. See `crate::wheel`.
    ///
    /// Each step holds the bar up for a moment ([`theme::VOLUME_LINGER`]),
    /// so the slider and the figure are seen to move even if the pointer
    /// leaves the picture between two turns.
    fn on_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.modifiers.alt {
            self.wheel.reset();
            return;
        }
        cx.stop_propagation();
        let steps = self.wheel.steps(&event.delta);
        if steps == 0 {
            return;
        }
        self.nudge_volume(steps * keys::VOLUME_STEP, window, cx);
        self.volume_linger = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(theme::VOLUME_LINGER).await;
            let _ = this.update(cx, |this, cx| {
                this.volume_linger = None;
                if this.sync_controls() {
                    cx.notify();
                }
            });
        }));
        if self.sync_controls() {
            cx.notify();
        }
    }

    /// The gestures a picture answers that a tile's does not: `Alt` and the
    /// wheel for the level ([`on_wheel`](Self::on_wheel)), and a middle
    /// press for mute, the same as `M` and the speaker. On the press, every
    /// press of it: a second one is a second toggle, as a second press of
    /// the speaker would be.
    ///
    /// Hung on the element under the pointer that hears it: the picture in
    /// a pane, and in a pop-out the layer it is dragged by too, which blocks
    /// the pointer from the picture under it. The control bar, which blocks
    /// the pointer as well, hangs the wheel on itself (`control_bar`), and a
    /// middle press on it does nothing. Neither gesture is a drag: a
    /// wheel moves no window, and a middle press is not the left press
    /// Windows takes a caption drag from, so hearing it there leaves the
    /// drag alone (`controls::drag_layer`).
    fn picture_gestures(&self, element: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        element
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.toggle_mute(window, cx)),
            )
    }

    /// Move the player to `place`, drawn in the window whose focus is
    /// `focus`: between the watch page, a tile in the mini player, a window
    /// of its own, and offstage while another pane has the watch page. The
    /// only way a player changes place or window; `RootView::restage` is the
    /// only caller, after `Start`.
    ///
    /// How it is drawn and nothing else: the pane keeps its sound, which is
    /// the point of the mini player, of the pop-out and of a maximize, which
    /// hides the other panes and silences none. Everything the pointer was
    /// doing is let go ([`let_go`](Self::let_go)), since the new place will
    /// never hear the release.
    ///
    /// The tile goes too, from every window's atlas. Each window keeps its
    /// own, and `render` skips uploading a frame it already holds, so a
    /// window the view came back to would paint its own old tile — a
    /// paused picture from before the move, for as long as it stayed
    /// paused. Freed later in the same flush rather than now, since the
    /// window drawing this may be the one being updated; and the notify
    /// dirties every window that drew the view last frame, so none shows a
    /// scene that names the freed tile again before it has drawn a new one.
    /// The first frame is left alone, so the picture does not fade in again
    /// in its new place.
    pub fn set_place(&mut self, place: Place, focus: FocusHandle, cx: &mut Context<Self>) {
        if self.place == place && self.root_focus == focus {
            return;
        }
        if let Some(frame) = self.current.take() {
            cx.defer(move |cx| cx.drop_image(frame, None));
        }
        #[cfg(debug_assertions)]
        {
            self.drawn_in = None;
        }
        self.place = place;
        self.root_focus = focus;
        self.let_go();
        // What holds the bar up without the pointer — a switch under way —
        // holds it up in the new place too, from a fade started over.
        self.sync_controls();
        cx.notify();
    }

    /// Let go of everything the pointer was doing to the player, and close
    /// its menu: for a change of place, where the release will never come —
    /// a held scrub in particular would go on asking for a frame every frame
    /// for as long as the player was somewhere else.
    ///
    /// The bar's fade starts over, hidden, rather than being set hidden. A
    /// tile never draws the bar, and gpui keeps an animation's state only
    /// from one frame to the next and one window at a time, so a fade that
    /// came back with its history would replay its last flip from the start
    /// on the first frame in its new place: the bar flashing up and fading
    /// away over the picture.
    fn let_go(&mut self) {
        self.menu = None;
        self.row_run = false;
        self.hovered = false;
        self.pointing = None;
        self.scrub = None;
        self.track = None;
        self.wheel.reset();
        self.volume_linger = None;
        self.controls = motion::Fade::hidden();
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
    /// changed anything; the rule is [`bar_wanted`]'s.
    fn sync_controls(&mut self) -> bool {
        let visible = bar_wanted(
            self.place,
            BarHolds {
                hovered: self.hovered,
                menu: self.menu.is_some(),
                scrub: self.scrub.is_some(),
                switch: self.switching.is_some() || self.linger.is_some(),
                volume: self.volume_linger.is_some(),
            },
        );
        self.controls.set(visible)
    }

    /// What a pick is switching the pane to now, from `RootView::set_pending`,
    /// the one write of the pane's pending start; see [`Switching`] for why
    /// nothing else calls this. Does nothing when nothing changed.
    ///
    /// A switch that ends — taken over, given up on, called off, or
    /// superseded by a cold start — leaves the bar up for
    /// [`theme::SWITCH_LINGER`], so what it came to can be read: the new
    /// rendition on the pill, or the old one back beside the root's toast.
    /// A one-shot timer lets it go, and repaints only if that brings the bar
    /// down; nothing polls. A new switch calls a linger still running off.
    pub fn set_switching(&mut self, switching: Option<Switching>, cx: &mut Context<Self>) {
        if self.switching == switching {
            return;
        }
        let ended = lingers_after(self.switching.as_ref(), switching.as_ref());
        self.switching = switching;
        self.linger = ended.then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(theme::SWITCH_LINGER).await;
                let _ = this.update(cx, |this, cx| {
                    this.linger = None;
                    if this.sync_controls() {
                        cx.notify();
                    }
                });
            })
        });
        self.sync_controls();
        cx.notify();
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
    /// that did not need a new player to take effect, and once a player
    /// started beside the picture has taken over (`RootView::on_swapped`).
    pub fn set_picked(&mut self, picked: bool, cx: &mut Context<Self>) {
        self.qualities.picked = picked;
        cx.notify();
    }

    /// Whether what plays was picked from the pane's own menu: what a pick
    /// that could not be carried out goes back to.
    pub fn picked(&self) -> bool {
        self.qualities.picked
    }

    /// The render size the pane's probe asks for, which a rendition started
    /// beside this player is handed so it renders at the pane's size from
    /// its first frame (`video::StartOptions::size`).
    pub fn size_handle(&self) -> SizeHandle {
        self.stream.size_handle()
    }

    /// What the pane's chat now is, after `RootView::toggle_chat` changed
    /// it; see [`ChatButton`] for why this is the only other way in.
    pub fn set_chat(&mut self, chat: ChatButton, cx: &mut Context<Self>) {
        if self.chat != chat {
            self.chat = chat;
            cx.notify();
        }
    }

    /// What More offers about hearing one pane alone now, from
    /// `RootView::sync_hear_only`; see `VideoView::hear_only` for why
    /// nothing else calls this. Repaints only on a change.
    pub fn set_hear_only(&mut self, hear_only: HearOnly, cx: &mut Context<Self>) {
        if self.hear_only != hear_only {
            self.hear_only = hear_only;
            cx.notify();
        }
    }

    /// A frame from the stream on screen has arrived, from the pump: noted
    /// for whether the picture has moved again after standing still. A
    /// clock read and a comparison, at the stream's frame rate.
    fn note_frame(&mut self) {
        let now = Instant::now();
        if ad_break::frames_resumed(self.last_frame, now) {
            self.resumed_at = Some(now);
        }
        self.last_frame = Some(now);
    }

    /// When the picture last moved again after standing still, its first
    /// frame included; see `VideoView::resumed_at`.
    pub fn resumed_at(&self) -> Option<Instant> {
        self.resumed_at
    }

    /// When the live broadcast on screen began, from
    /// `RootView::sync_live_since`; see `VideoView::live_since` for why
    /// nothing else calls this. Repaints only on a change.
    pub fn set_live_since(&mut self, live_since: Option<DateTime<Utc>>, cx: &mut Context<Self>) {
        if self.live_since != live_since {
            self.live_since = live_since;
            cx.notify();
        }
    }

    /// Whether the recording on screen is its broadcast's archive while the
    /// broadcast still goes on, from `RootView::sync_back_to_live`; see
    /// `VideoView::back_to_live` for why nothing else calls this. Repaints
    /// only on a change.
    pub fn set_back_to_live(&mut self, back_to_live: bool, cx: &mut Context<Self>) {
        if self.back_to_live != back_to_live {
            self.back_to_live = back_to_live;
            cx.notify();
        }
    }

    /// What the pane's maximize control offers now, from
    /// `RootView::restage`; see `VideoView::maximize` for why nothing else
    /// calls this.
    pub fn set_maximize(&mut self, maximize: MaximizeButton, cx: &mut Context<Self>) {
        if self.maximize != maximize {
            self.maximize = maximize;
            cx.notify();
        }
    }

    /// Note what the bar has room for, from the probe: `fit` on its row of
    /// buttons, and whether a live stream's timeline fits above it. Returns
    /// whether that changed, the only time a repaint is worth it: the width
    /// moves with every pixel of a window being dragged, and what fits
    /// changes a few times across all of them.
    fn set_fit(&mut self, fit: bar::Fit, timeline_fits: bool) -> bool {
        if self.fit == fit && self.timeline_fits == timeline_fits {
            return false;
        }
        self.fit = fit;
        self.timeline_fits = timeline_fits;
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
    /// true for the life of this player, a rendition swapped in place
    /// included (`swap`); a cold restart makes a new player, which starts
    /// over.
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

/// Whether a player in `place` draws its control bar: a pane and a pop-out
/// do, a tile does not, and nor does a player offstage, which is not drawn
/// at all.
fn draws_bar(place: Place) -> bool {
    match place {
        Place::Pane | Place::PopOut => true,
        Place::Tile | Place::Offstage => false,
    }
}

/// What may be holding a player's control bar up; see [`bar_wanted`].
#[derive(Clone, Copy, Debug, Default)]
struct BarHolds {
    /// The pointer is over the player.
    hovered: bool,
    /// One of its menus is open.
    menu: bool,
    /// The seek bar's thumb is held.
    scrub: bool,
    /// A pick is switching its quality, or one ended a moment ago
    /// ([`theme::SWITCH_LINGER`]).
    switch: bool,
    /// The wheel changed the level a moment ago
    /// ([`theme::VOLUME_LINGER`]).
    volume: bool,
}

/// Whether the control bar of a player in `place` is up, given what is
/// holding it.
///
/// Only where a bar is drawn at all ([`draws_bar`]). There, while the
/// pointer is over the player; while a menu is open even after the pointer
/// leaves, or reaching for an option would dismiss the menu on the way —
/// which is also what lets the palette open a menu with the pointer nowhere
/// near; and while the thumb is held, wherever the pointer has dragged it,
/// since a bar that faded out mid-scrub would take the thumb with it.
///
/// And on a pane, while a quality somebody picked is under way and for a
/// moment after: the pill saying so is on the bar, and a bar that faded
/// with the pointer gone hid the only sign the pick was being carried out.
/// Not in a pop-out, whose bar has no pill and no More, so nothing on it
/// would say why it stayed up.
///
/// And for a moment after `Alt` and the wheel changed the level, in a pane
/// and a pop-out alike, so the slider and the figure are seen to move: the
/// pointer is usually on the picture already, but a turn as it leaves
/// should not change the level with nothing on screen saying so.
fn bar_wanted(place: Place, holds: BarHolds) -> bool {
    let switch = holds.switch && place == Place::Pane;
    draws_bar(place) && (holds.hovered || holds.menu || holds.scrub || holds.volume || switch)
}

/// Whether a change of what a pick is switching to, from `old` to `new`,
/// leaves the bar lingering: only a switch that has ended. One switch giving
/// way to another is still under way, and a first one is nothing ending.
fn lingers_after(old: Option<&Switching>, new: Option<&Switching>) -> bool {
    old.is_some() && new.is_none()
}

impl VideoView {
    /// The frame, as the element that fills the player.
    ///
    /// The first frames fade in rather than cut, which makes a channel
    /// switch read as deliberate instead of a glitch; the poster under them
    /// is what they fade in over. Only until the picture covers the pane:
    /// once it has, a player drawn somewhere new — popped out, brought
    /// back, between the pages, back in the grid after another pane had the
    /// page — comes up as it is, rather than fading in again out of the
    /// black behind it.
    fn picture(&self, frame: Arc<RenderImage>) -> gpui::AnyElement {
        let picture = img(frame).flex_1().min_h_0().w_full();
        if self.covers() {
            return picture.into_any_element();
        }
        picture
            .with_animation(
                ElementId::from("video-fade-in"),
                Animation::new(theme::MOTION_VIDEO),
                |element, delta| element.opacity(delta),
            )
            .into_any_element()
    }
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

        // One window at a time; see `crate::stage`. A second window drawing
        // this view would freeze on the first tile it was given, since the
        // upload below is skipped for a frame already held, and the two
        // probes would fight over the render size. Only `set_place` may
        // move it, and it starts this over.
        #[cfg(debug_assertions)]
        {
            let here = window.window_handle();
            debug_assert!(
                self.drawn_in.is_none_or(|drawn| drawn == here),
                "a VideoView drew in a second window without set_place"
            );
            self.drawn_in = Some(here);
        }

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
            // A rendition swapped in place is a new stream, with an id of its
            // own (`swap`): the tile the old one wrote goes now, or it would
            // stay resident for the life of the window, since nothing else
            // names that id again. From this window alone, the only one that
            // has drawn this player since its last change of place, which
            // emptied `current`.
            if let Some(old) = self.current.take().filter(|old| old.id != frame.id) {
                let _ = window.drop_image(old);
            }
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
        let cluster = bar::cluster(self.place, self.maximize);
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
                let fit = bar::fit(f32::from(bounds.size.width), cluster);
                let timeline_fits = bar::timeline_fits(f32::from(bounds.size.width));
                this.update(cx, |view: &mut Self, cx| {
                    let hovered = view.set_hovered(inside);
                    let scrubbed = view.follow_scrub(pointer.x);
                    let pointed = view.follow_pointer(inside.then_some(pointer));
                    let fitted = view.set_fit(fit, timeline_fits);
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
        let pane = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .id("video-pane");
        // Not on a tile, whose picture is only a way back to the watch page.
        let pane = if draws_bar(self.place) {
            self.picture_gestures(pane, cx)
        } else {
            pane
        };
        pane
            // Only here to wake a repaint. Its *value* is wrong during a drag,
            // so the probe above decides; but a paused stream sends no frames,
            // and without this nothing would ask the probe to run again.
            .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
            // The gesture a pane's player has. A single click does nothing on
            // purpose - it is how a pane is made the active one, and pausing
            // on a click would turn choosing a pane into stopping it. Not on
            // a tile, whose click is the way back to the watch page, nor in
            // a pop-out, whose picture is what the window is dragged by.
            // A double-click whose first press chose a menu row never gets
            // here: `run_guard`, below, stops the second press.
            .when(self.place == Place::Pane, |pane| {
                pane.on_click(|event: &ClickEvent, window, _cx| {
                    if event.click_count() == 2 {
                        window.toggle_fullscreen();
                    }
                })
            })
            .child(probe)
            // Ahead of the bar and its menus, so it hears a press before
            // anything on them does; see `run_guard`. Only a pane has menus.
            .when(self.place == Place::Pane, |pane| {
                pane.child(Self::run_guard(cx))
            })
            .child(self.picture(frame))
            // In a pop-out the picture is the window's handle: under the bar,
            // which blocks the pointer only while it is up, so a button on
            // screen is never under the drag.
            .when(self.place == Place::PopOut, |pane| {
                let layer = controls::drag_layer("video-drag", window, cx);
                pane.child(self.picture_gestures(layer, cx))
            })
            .when(draws_bar(self.place), |pane| {
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

    /// A pick switching a pane to `to`, from its rendition's row or, with
    /// `default`, from the settings' row.
    fn switching(to: &str, default: bool) -> Switching {
        Switching {
            to: SharedString::from(to.to_string()),
            default,
            since: Instant::now(),
        }
    }

    /// Every place a player can be.
    const PLACES: [Place; 4] = [Place::Pane, Place::PopOut, Place::Tile, Place::Offstage];

    /// The bar is up while the pointer is on the player, a menu is open or
    /// the thumb is held, wherever a bar is drawn; nowhere else, and not
    /// with nothing holding it.
    #[test]
    fn the_bar_follows_the_pointer_menus_and_scrubs() {
        let holds = [
            BarHolds {
                hovered: true,
                ..BarHolds::default()
            },
            BarHolds {
                menu: true,
                ..BarHolds::default()
            },
            BarHolds {
                scrub: true,
                ..BarHolds::default()
            },
            BarHolds {
                volume: true,
                ..BarHolds::default()
            },
        ];
        for place in PLACES {
            assert!(!bar_wanted(place, BarHolds::default()), "{place:?}");
            for hold in holds {
                assert_eq!(
                    bar_wanted(place, hold),
                    draws_bar(place),
                    "{place:?} {hold:?}"
                );
            }
        }
    }

    /// A quality change somebody picked holds a pane's bar up with the
    /// pointer gone, from the press and through the moment after it ends;
    /// not a pop-out's, whose bar has no pill to say why, nor a tile's or an
    /// offstage player's, which draw none.
    #[test]
    fn a_switch_holds_a_panes_bar_up() {
        let switch = BarHolds {
            switch: true,
            ..BarHolds::default()
        };
        assert!(bar_wanted(Place::Pane, switch));
        for place in [Place::PopOut, Place::Tile, Place::Offstage] {
            assert!(!bar_wanted(place, switch), "{place:?}");
        }
    }

    /// Only a switch that has ended leaves the bar lingering: not the first
    /// one, nor one giving way to another, nor nothing staying nothing.
    #[test]
    fn the_bar_lingers_only_once_a_switch_ends() {
        let a = switching("480p30", false);
        let b = switching("720p60", true);
        assert!(lingers_after(Some(&a), None), "taken over or given up");
        assert!(!lingers_after(None, Some(&a)), "just picked");
        assert!(!lingers_after(Some(&a), Some(&b)), "picked again");
        assert!(!lingers_after(None, None));
    }
}
