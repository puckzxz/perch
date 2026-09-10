//! Opening, restarting and closing panes: a channel or a recording becomes a
//! `Slot`, streamlink is started for it, and its player is stood up when the
//! stream resolves. Everything a pane is *playing* is decided here; how it is
//! drawn is `crate::watch`.

use gpui::{prelude::*, Context, SharedString, Window};
use settings::QualityPreference;
use streamlink::{quality, StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::{LiveStream, Video, VideoKind};

use super::{Page, RootView};
use crate::chat::{ChatView, Feed};
use crate::layout;
use crate::video::{self, Playback, PositionHandle, Stopped, VideoStream};
use crate::video_view::{VideoEvent, VideoView};
use crate::watch::{Slot, Source, StreamState, MAX_PANES};

/// Starting render size. Each pane measures itself on the first layout pass and
/// its render thread follows from then on, so this only decides what the first
/// frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

impl RootView {
    /// Open `channel`. With `solo`, it becomes the only pane; otherwise it is
    /// added alongside whatever is already playing.
    pub(super) fn open_channel(
        &mut self,
        channel: String,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page = Page::Watch;

        if self.slot_index(&channel).is_some() {
            // Already open, so switch to it rather than restarting it. Solo
            // closes the others; dropping them stops their streamlink and mpv.
            if solo {
                self.slots.retain(|slot| slot.channel == channel);
            }
            self.set_background(false, cx);
            cx.notify();
            return;
        }

        if solo {
            self.slots.clear();
        } else if self.slots.len() >= MAX_PANES {
            self.toast(format!("already watching {MAX_PANES} streams"), cx);
            return;
        }

        let chat = cx.new(|cx| {
            ChatView::new(
                Feed::Live {
                    channel: channel.clone(),
                    history: self.settings.chat_history,
                },
                self.cache.clone(),
                window,
                cx,
            )
        });
        self.slots.push(Slot {
            key: channel.clone(),
            channel: channel.clone(),
            source: Source::Live,
            quality_override: None,
            state: StreamState::Starting,
            chat: Some(chat),
            resume_at: 0.0,
            supervisor: None,
            pump: None,
            hovered: false,
            chat_hidden: self.settings.chat_hidden_for(&channel),
            stalled_at: None,
        });

        // Remembered for the palette, which leads with it next time.
        if self.settings.note_watched(&channel) {
            self.save_settings(cx);
        }
        self.active = Some(channel.clone());
        self.start_stream(channel, window, cx);
        self.set_background(false, cx);
        // The others share the window with one more pane now.
        self.sync_quality(window, cx);
    }

    /// Open a recording. With `solo`, it becomes the only pane; otherwise it
    /// is added alongside whatever is already playing.
    ///
    /// The mirror of [`open_channel`](Self::open_channel), keyed by the video
    /// rather than the channel — a channel's stream and one of its recordings
    /// are two panes, not one. Its chat is the replay of what was said at the
    /// moment on screen, following the pane's position from the moment it
    /// opens, so the pane fills with the first seconds of chat while the
    /// player is still being resolved.
    pub(super) fn open_video(
        &mut self,
        video: Video,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_video_at(video, solo, 0.0, window, cx);
    }

    /// [`open_video`](Self::open_video), from `start_at` seconds in: where a
    /// link pointed.
    pub(super) fn open_video_at(
        &mut self,
        video: Video,
        solo: bool,
        start_at: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page = Page::Watch;
        let key = Slot::video_key(&video.id);

        if self.slot_index(&key).is_some() {
            if solo {
                self.slots.retain(|slot| slot.key == key);
            }
            self.set_background(false, cx);
            cx.notify();
            return;
        }

        if solo {
            self.slots.clear();
        } else if self.slots.len() >= MAX_PANES {
            self.toast(format!("already watching {MAX_PANES} streams"), cx);
            return;
        }

        let channel = video.user_login.clone();
        let position = PositionHandle::new();
        // Archives only. A highlight is cut from ranges of a broadcast, so
        // its offsets mean nothing to a replay; the app lists none today, and
        // one that arrives plays as picture alone.
        let chat = matches!(video.kind, VideoKind::Archive).then(|| {
            cx.new(|cx| {
                ChatView::new(
                    Feed::Replay {
                        video_id: video.id.clone(),
                        channel: channel.clone(),
                        room_id: video.user_id.clone(),
                        position: position.clone(),
                    },
                    self.cache.clone(),
                    window,
                    cx,
                )
            })
        });
        self.slots.push(Slot {
            key: key.clone(),
            channel: channel.clone(),
            source: Source::Video {
                video: Box::new(video),
                position,
            },
            quality_override: None,
            state: StreamState::Starting,
            chat,
            resume_at: start_at,
            supervisor: None,
            pump: None,
            hovered: false,
            // Remembered against the channel, like a live pane's: hiding
            // chat is a statement about the streamer, not the broadcast.
            chat_hidden: self.settings.chat_hidden_for(&channel),
            stalled_at: None,
        });

        // A recording counts as watching its channel, for the palette.
        if self.settings.note_watched(&channel) {
            self.save_settings(cx);
        }
        self.active = Some(key.clone());
        self.start_stream(key, window, cx);
        self.set_background(false, cx);
        self.sync_quality(window, cx);
    }

    /// Start, or restart, streamlink for an existing slot.
    pub(super) fn start_stream(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.slot_index(&key) else {
            return;
        };
        let channel = self.slots[index].channel.clone();

        let quality = self.slots[index]
            .quality_override
            .clone()
            .or(match &self.settings.quality {
                QualityPreference::Auto => None,
                QualityPreference::Fixed(name) => Some(name.clone()),
            });
        let options = StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
        };

        // Quality targets the pane this stream will actually render into.
        let pane_height = self.pane_height(window);

        // A recording is only resolved: streamlink names the playlist and
        // retires, and the player opens it itself. See the streamlink crate.
        let (supervisor, mut events) = match &self.slots[index].source {
            Source::Live => StreamSupervisor::start(channel.clone(), pane_height, options),
            Source::Video { video, .. } => {
                StreamSupervisor::start_video(video.id.clone(), pane_height, options)
            }
        };
        // Read once, here, and frozen into the pump: the pane it belongs to
        // may not start for several seconds, and adjusting a *different* pane
        // in the meantime must not follow it in.
        let volume = self
            .volume_override
            .unwrap_or_else(|| self.settings.volume_for(&channel));

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let key = key.clone();
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.apply_stream_event(&key, event, volume, window, cx)
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        self.slots[index].supervisor = Some(supervisor);
        self.slots[index].pump = Some(pump);
    }

    pub(super) fn apply_stream_event(
        &mut self,
        key: &str,
        event: StreamEvent,
        volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Look the slot up by key rather than by a captured index: panes
        // close while streams are still starting, and an index would go stale.
        let Some(index) = self.slot_index(key) else {
            return;
        };

        match event {
            StreamEvent::Resolving => self.slots[index].set_state(StreamState::Starting),
            StreamEvent::Ready {
                url,
                quality,
                available,
                playlist,
            } => {
                // What the player is handed: the relay for a live stream, or
                // the recording's playlist and where to open it.
                let playback = match (&self.slots[index].source, playlist) {
                    (Source::Video { video, position }, Some(playlist)) => Playback::Vod {
                        playlist,
                        start_at: self.slots[index].resume_at,
                        label: format!("{}-{quality}", video.id),
                        position: position.clone(),
                    },
                    (Source::Video { .. }, None) => {
                        self.slots[index].set_state(StreamState::Failed(
                            "the recording came without a playlist".into(),
                        ));
                        cx.notify();
                        return;
                    }
                    (Source::Live, _) => Playback::Live { url },
                };
                match VideoStream::start(RENDER_WIDTH, RENDER_HEIGHT, volume, playback) {
                    Ok((stream, frames)) => {
                        let label = SharedString::from(quality);
                        let view = cx.new(|cx| {
                            VideoView::from_stream(stream, frames, label, available, window, cx)
                        });
                        let owner = key.to_string();
                        cx.subscribe_in(
                            &view,
                            window,
                            move |this: &mut RootView, _, event, window, cx| match event {
                                VideoEvent::VolumeChanged(volume) => {
                                    // Remembered against the channel rather
                                    // than globally, so coming back to a
                                    // streamer finds them where you left them.
                                    // A slider drag emits a change per pixel,
                                    // so the write waits for the run to end.
                                    if this.settings.set_volume_for(&owner, *volume) {
                                        this.save_settings_soon(cx);
                                    }
                                    cx.notify();
                                }
                                VideoEvent::QualityRequested(name) => {
                                    if let Some(index) = this.slot_index(&owner) {
                                        this.slots[index].quality_override = Some(name.clone());
                                        this.restart_stream(index, window, cx);
                                        cx.notify();
                                    }
                                }
                                VideoEvent::Stopped(reason) => {
                                    this.stream_stopped(&owner, reason.clone(), cx)
                                }
                            },
                        )
                        .detach();
                        self.slots[index].set_state(StreamState::Playing(view));
                    }
                    Err(e) => {
                        self.slots[index].set_state(StreamState::Failed(e.to_string().into()))
                    }
                }
            }
            StreamEvent::Offline => self.slots[index].set_state(StreamState::Offline),
            StreamEvent::Failed { reason } => {
                self.slots[index].set_state(StreamState::Failed(reason.into()))
            }
        }
        cx.notify();
    }

    /// Everything the app currently knows about a channel it is watching.
    ///
    /// Every live list it has fetched, not just your follows. The follows poll
    /// used to be the only source, so a pane opened from Popular, from inside
    /// a category or from a search had no viewer count and no title — the very
    /// panes most likely to be somebody you had never watched before. Follows
    /// come first because that list is the one kept fresh by a poll.
    ///
    /// This is a snapshot, not a subscription: a title changed mid-stream is
    /// wrong here until whichever list it came from is fetched again.
    pub(super) fn stream_info(&self, channel: &str) -> Option<&LiveStream> {
        let search = self
            .discovery
            .search
            .as_ref()
            .map(|results| results.streams.as_slice())
            .unwrap_or_default();
        [
            self.follows.as_slice(),
            self.discovery.popular.items.as_slice(),
            self.discovery.streams.items.as_slice(),
            search,
        ]
        .into_iter()
        .flatten()
        .find(|stream| stream.user_login == channel)
    }

    /// A playing stream stopped on its own: the broadcast ended, or mpv gave
    /// up on it.
    ///
    /// Retiring the player is the point. Its last frame is still on screen and
    /// nothing will replace it, so leaving it there is the bug this exists to
    /// fix — a finished stream looked exactly like a paused one. The pane keeps
    /// its chat, which is where people say goodnight.
    pub(super) fn stream_stopped(&mut self, key: &str, reason: Stopped, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(key) else {
            return;
        };
        // Where "watch again" and "try again" start a recording from: the
        // top once it has finished, and where it got to when it failed.
        self.slots[index].resume_at = match reason {
            Stopped::Ended => 0.0,
            Stopped::Failed(_) => self.slots[index]
                .video()
                .map(|view| view.read(cx).position())
                .unwrap_or(0.0),
        };
        // streamlink outlives the stream it was serving: its external HTTP
        // server runs in the continuous mode by default, so it sits waiting
        // for another request that is never coming, holding a process and a
        // port. Dropping the supervisor kills it; dropping the pump stops
        // listening to a worker that has nothing left to say. `try again`
        // starts both again.
        self.slots[index].supervisor = None;
        self.slots[index].pump = None;
        // Replacing the state drops the player, and with it the frozen frame.
        let state = match reason {
            Stopped::Ended => StreamState::Ended,
            Stopped::Failed(message) => StreamState::Failed(message.into()),
        };
        self.slots[index].set_state(state);
        cx.notify();
    }

    /// Ask for a pane's stream again: after it stopped, or after the channel
    /// came back on. Whatever the pane was showing becomes "starting…".
    pub(super) fn retry_stream(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(key) else {
            return;
        };
        self.slots[index].set_state(StreamState::Starting);
        self.start_stream(key.to_string(), window, cx);
        cx.notify();
    }

    /// Start pane `index`'s stream over, from where a recording has got to.
    /// For a quality change from anywhere: the pane's menu, the settings
    /// sheet, or the pane changing size.
    pub(super) fn restart_stream(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        // A recording picks up where it was, not from the top. Read before
        // the state changes, since that is what drops the player.
        if let Some(position) = slot.video().map(|view| view.read(cx).position()) {
            slot.resume_at = position;
        }
        slot.set_state(StreamState::Starting);
        let key = slot.key.clone();
        self.start_stream(key, window, cx);
    }

    /// How tall a pane is right now, in physical pixels: what a quality is
    /// chosen against, when a stream starts and again in
    /// [`sync_quality`](Self::sync_quality). With several panes the window is
    /// shared, so each one asks for proportionally less.
    pub(super) fn pane_height(&self, window: &Window) -> u32 {
        let scale = window.scale_factor();
        let body = self.body(window);
        let (rows, _) = layout::grid_shape(self.slots.len().max(1), body.aspect());
        (body.height * scale / rows as f32)
            .round()
            .clamp(180.0, video::MAX_RENDER_HEIGHT as f32) as u32
    }

    /// Choose each pane's quality again, for the size it is now — and move
    /// only upwards.
    ///
    /// A quality is picked when a stream opens, against the pane it will
    /// render into — and the pane changes size every time another opens or
    /// closes, the rail folds, or the window does. A pane opened as one of
    /// four stayed at the small rendition after the other three closed,
    /// which was a soft picture in a large pane for as long as nobody touched
    /// the menu. So the choice is made again whenever the grid changes, and
    /// a *sharper* answer restarts the stream: a restart is a few seconds of
    /// black while streamlink resolves again, worth it for the picture. A
    /// pane that has shrunk keeps what it has — the smaller pane hides
    /// nothing, the CPU it costs is what it cost when it opened, and a
    /// restart there would trade a visible interruption for a saving. A
    /// quality picked by hand from the pane's own menu is left alone; that
    /// choice was about this pane, whatever its size.
    pub(super) fn sync_quality(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane_height = self.pane_height(window);
        let restart: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.quality_override.is_none())
            .filter_map(|(index, slot)| {
                let view = slot.video()?.read(cx);
                let wanted = match &self.settings.quality {
                    QualityPreference::Auto => quality::select(view.available(), pane_height),
                    QualityPreference::Fixed(name) => {
                        quality::select_named(view.available(), name, pane_height)
                    }
                }?;
                let playing = quality::parse_quality(view.quality()).map_or(0, |q| q.height);
                (wanted.height > playing).then_some(index)
            })
            .collect();
        for index in &restart {
            eprintln!(
                "video: {} re-choosing quality for a {pane_height}px pane",
                self.slots[*index].key
            );
            self.restart_stream(*index, window, cx);
        }
        if !restart.is_empty() {
            cx.notify();
        }
    }

    pub(super) fn close_slot(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.slots.len() {
            // Dropping the slot stops its streamlink and its mpv.
            self.slots.remove(index);
        }
        if self.slots.is_empty() {
            self.page = Page::Browse;
        }
        // Whoever is left has more of the window.
        self.sync_quality(window, cx);
        cx.notify();
    }

    /// Mute or unmute every pane at once, for moving between pages.
    pub(super) fn set_background(&mut self, background: bool, cx: &mut Context<Self>) {
        for slot in &self.slots {
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_background(background, cx));
            }
        }
    }

    /// Leave the watch page.
    ///
    /// With the miniplayer on, the streams keep going as muted thumbnails —
    /// genuinely cheaper, not just smaller, since render size follows the
    /// element and a small one decodes into a small buffer. With it off they
    /// stop, which is what somebody who came here to pick the next thing
    /// wanted: a backgrounded stream is still decoding and still pulling bytes.
    pub(super) fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Browse;
        if self.settings.miniplayer {
            self.set_background(true, cx);
        } else {
            // Dropping the slots stops each streamlink and its mpv.
            self.slots.clear();
        }
        cx.notify();
    }

    pub(super) fn go_watch(&mut self, cx: &mut Context<Self>) {
        if self.slots.is_empty() {
            return;
        }
        self.page = Page::Watch;
        self.set_background(false, cx);
        cx.notify();
    }

    pub(super) fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.slots.clear();
        self.page = Page::Browse;
        cx.notify();
    }
}
