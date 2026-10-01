//! Opening, restarting and closing panes: a channel or a recording becomes a
//! `Slot`, streamlink is started for it, and its player is stood up when the
//! stream resolves. Everything a pane is *playing* is decided here; how it is
//! drawn is `crate::watch`.

use gpui::{prelude::*, App, Context, Focusable, SharedString, Window};
use settings::QualityPreference;
use streamlink::{quality, StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::{LiveStream, Video, VideoKind};

use super::navigation::Route;
use super::{Page, RootView};
use crate::chat::{ChatView, Feed};
use crate::video::{self, Playback, PositionHandle, Stopped, VideoStream};
use crate::video_view::{ChatButton, Qualities, Start, VideoView};
use crate::watch::{Slot, Source, StreamState, MAX_PANES};
use crate::{layout, motion, settings_view};

/// Starting render size. Each pane measures itself on the first layout pass and
/// its render thread follows from then on, so this only decides what the first
/// frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

impl RootView {
    /// Open `channel`. With `solo`, it becomes the only pane; otherwise it is
    /// added alongside whatever is already playing. One step on the trail.
    ///
    /// A fifth pane is refused where you are, with a toast, rather than on
    /// the watch page: the page used to flip before the count was checked,
    /// so a refused `+ Add` from the browse page took you away from it for
    /// nothing.
    pub(super) fn open_channel(
        &mut self,
        channel: String,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record(|this| {
            if this.slot_index(&channel).is_some() {
                // Already open, so switch to it rather than restarting it. Solo
                // closes the others; dropping them stops their streamlink and mpv.
                this.show_watch_page(window, cx);
                if solo {
                    this.retire_slots(|slot| slot.channel == channel, cx);
                }
                this.set_compact(false, cx);
                cx.notify();
                return;
            }

            if solo {
                this.retire_slots(|_| false, cx);
            } else if this.slots.len() >= MAX_PANES {
                this.toast(format!("already watching {MAX_PANES} streams"), cx);
                return;
            }
            this.show_watch_page(window, cx);

            let chat = cx.new(|cx| {
                ChatView::new(
                    Feed::Live {
                        channel: channel.clone(),
                        history: this.settings.chat_history,
                    },
                    this.cache.clone(),
                    window,
                    cx,
                )
            });
            this.slots.push(Slot::new(
                channel.clone(),
                channel.clone(),
                Source::Live,
                Some(chat),
                0.0,
                this.settings.chat_hidden_for(&channel),
            ));

            // Remembered for the palette, which leads with it next time.
            if this.settings.note_watched(&channel) {
                this.save_settings(cx);
            }
            this.active = Some(channel.clone());
            this.start_stream(channel, window, cx);
            this.set_compact(false, cx);
            // The others share the window with one more pane now.
            this.sync_quality(window, cx);
        })
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
    ///
    /// It opens where it was left, if it has been watched before — from any
    /// card, on any page. See `root::history`.
    pub(super) fn open_video(
        &mut self,
        video: Video,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start_at = self.resume_point(&video.id);
        self.open_video_at(video, solo, start_at, window, cx);
    }

    /// [`open_video`](Self::open_video), from `start_at` seconds in: where a
    /// link pointed, or where the history says it was left. One step on the
    /// trail, and refused where you are at four panes, as `open_channel` is.
    pub(super) fn open_video_at(
        &mut self,
        video: Video,
        solo: bool,
        start_at: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record(|this| {
            let key = Slot::video_key(&video.id);

            if this.slot_index(&key).is_some() {
                this.show_watch_page(window, cx);
                if solo {
                    this.retire_slots(|slot| slot.key == key, cx);
                }
                this.set_compact(false, cx);
                cx.notify();
                return;
            }

            if solo {
                this.retire_slots(|_| false, cx);
            } else if this.slots.len() >= MAX_PANES {
                this.toast(format!("already watching {MAX_PANES} streams"), cx);
                return;
            }
            this.show_watch_page(window, cx);

            // Into the history now rather than once it plays, so a recording
            // opened and closed at once, or one that turns out to be gone, is
            // still one you can find again.
            this.note_opened(&video, start_at, cx);

            let channel = video.user_login.clone();
            // Already where the pane is opening, so the chat replay starts there
            // rather than at the top and then jumping.
            let position = PositionHandle::starting_at(start_at);
            // Archives only. A highlight is cut from ranges of a broadcast, so
            // its offsets mean nothing to a replay, and an upload had no chat to
            // replay; both play as picture alone.
            let chat = matches!(video.kind, VideoKind::Archive).then(|| {
                cx.new(|cx| {
                    ChatView::new(
                        Feed::Replay {
                            video_id: video.id.clone(),
                            channel: channel.clone(),
                            room_id: video.user_id.clone(),
                            position: position.clone(),
                        },
                        this.cache.clone(),
                        window,
                        cx,
                    )
                })
            });
            // Chat hidden is remembered against the channel, like a live
            // pane's: hiding chat is a statement about the streamer, not the
            // broadcast.
            let chat_hidden = this.settings.chat_hidden_for(&channel);
            this.slots.push(Slot::new(
                key.clone(),
                channel.clone(),
                Source::Video {
                    video: Box::new(video),
                    position,
                },
                chat,
                start_at,
                chat_hidden,
            ));

            // A recording counts as watching its channel, for the palette.
            if this.settings.note_watched(&channel) {
                this.save_settings(cx);
            }
            this.active = Some(key.clone());
            this.start_stream(key, window, cx);
            this.set_compact(false, cx);
            this.sync_quality(window, cx);
        })
    }

    /// Onto the watch page, for a pane just opened or switched to, taking
    /// the keyboard back from the title bar's search box if the box has it.
    ///
    /// The box is in the title bar, over both pages, so going to the watch
    /// page no longer takes it off screen. When it lived on the browse page
    /// it went with that page, and focus fell back to the root by itself.
    /// Most ways here start with a press on the page, which hands focus to
    /// the root anyway; a toast, a launch handed over and a linked recording
    /// arriving do not, and left the cursor in the box — so every key on the
    /// watch page stood aside for it, and Space typed a space into the search.
    /// Only the box: a text box on the settings sheet keeps the cursor when a
    /// launch opens a pane behind the sheet.
    fn show_watch_page(&mut self, window: &mut Window, cx: &App) {
        self.page = Page::Watch;
        if self.search.focus_handle(cx).is_focused(window) {
            self.focus.focus(window);
        }
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
                // A pane Mute all is holding starts silent, at the level its
                // slider will show; and one started while you browse — the
                // re-pick after a resize, a quality change from the settings —
                // starts as the tile it replaces, not as a big player with
                // its controls up in the corner of the browse page.
                let quiet = self.slots[index].quiet;
                let start = Start {
                    compact: self.page != Page::Watch,
                    quiet,
                    focus: self.focus.clone(),
                    chat: ChatButton::of(
                        self.slots[index].chat_hidden,
                        self.slots[index].chat.is_some(),
                    ),
                };
                let audible = if quiet { 0 } else { volume };
                match VideoStream::start(RENDER_WIDTH, RENDER_HEIGHT, audible, playback) {
                    Ok((stream, frames)) => {
                        let qualities = Qualities {
                            playing: SharedString::from(quality),
                            available,
                            default: settings_view::quality_label(&self.settings.quality),
                            picked: self.slots[index].quality_override.is_some(),
                        };
                        let view = cx.new(|cx| {
                            VideoView::from_stream(
                                stream, frames, qualities, volume, start, window, cx,
                            )
                        });
                        // What the player asks for, resolved by the pane's key
                        // when it arrives; see `on_video_event`.
                        let owner = key.to_string();
                        cx.subscribe_in(
                            &view,
                            window,
                            move |this: &mut RootView, _, event, window, cx| {
                                this.on_video_event(&owner, event, window, cx)
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

    /// What a pane calls its channel: the name the channel writes itself as,
    /// from whichever list knows it, or the login when none does.
    ///
    /// One answer for the pane header, its status line, the mini player and
    /// the palette. They used to ask different lists, so a channel you
    /// follow that was offline was "Nubzombie" in the palette and
    /// "nubzombie" in the pane it opened.
    pub(super) fn display_name(&self, slot: &Slot) -> String {
        if let Some(video) = slot.recording() {
            return video.user_name.clone();
        }
        self.stream_info(&slot.channel)
            .map(|stream| stream.display_name.clone())
            .or_else(|| {
                self.offline
                    .iter()
                    .find(|channel| channel.login == slot.channel)
                    .map(|channel| channel.display_name.clone())
            })
            .unwrap_or_else(|| slot.channel.clone())
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
        // Where "Watch again" and "Try again" start a recording from: the
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
        // listening to a worker that has nothing left to say. `Try again`
        // starts both again.
        self.slots[index].supervisor = None;
        self.slots[index].pump = None;
        // Replacing the state drops the player, and with it the frozen frame.
        let state = match reason {
            Stopped::Ended => StreamState::Ended,
            Stopped::Failed(message) => StreamState::Failed(message.into()),
        };
        self.slots[index].set_state(state);
        // A recording that reached its end is finished, and one that failed
        // is left where it failed: either way the history hears now.
        if !self.slots[index].is_live() && self.note_watching(cx) {
            self.save_history_soon(cx);
        }
        cx.notify();
    }

    /// Ask for a pane's stream again: after it stopped, or after the channel
    /// came back on. Whatever the pane was showing becomes "Starting…".
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

    /// What the settings would have a pane of `pane_height` play, from the
    /// renditions its stream offers. The one answer to that question, for
    /// the re-pick after a resize and for a pane handed back to the default.
    fn settings_pick(&self, available: &[String], pane_height: u32) -> Option<quality::Quality> {
        match &self.settings.quality {
            QualityPreference::Auto => quality::select(available, pane_height),
            QualityPreference::Fixed(name) => quality::select_named(available, name, pane_height),
        }
    }

    /// A quality chosen from pane `index`'s own menu: a rendition, which holds
    /// until the pane closes, or `None`, which hands the pane back to the
    /// settings.
    ///
    /// The stream restarts only when that changes what plays — a restart is
    /// seconds of black, and pinning the rendition already playing, or going
    /// back to a default that picks the same one, should cost nothing. Going
    /// back re-picks for the pane as it is now, down as well as up: that is
    /// an answer somebody asked for, where the re-pick after a resize is not.
    pub(super) fn request_quality(
        &mut self,
        index: usize,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.slots.get(index).and_then(Slot::video).cloned() else {
            return;
        };
        let (playing, available) = {
            let view = view.read(cx);
            (view.quality().to_string(), view.available().to_vec())
        };
        let target = match &name {
            Some(name) => Some(name.clone()),
            None => self
                .settings_pick(&available, self.pane_height(window))
                .map(|pick| pick.name),
        };
        let picked = name.is_some();
        self.slots[index].quality_override = name;
        if target.as_deref() == Some(playing.as_str()) {
            // Nothing to restart, so the menu the pane already has says it.
            view.update(cx, |view, cx| view.set_picked(picked, cx));
        } else {
            self.restart_stream(index, window, cx);
        }
        cx.notify();
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
                let wanted = self.settings_pick(view.available(), pane_height)?;
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
        if let Some(key) = self.slots.get(index).map(|slot| slot.key.clone()) {
            self.retire_slots(|slot| slot.key != key, cx);
        }
        if self.slots.is_empty() {
            self.page = Page::Browse;
        }
        // Whoever is left has more of the window.
        self.sync_quality(window, cx);
        cx.notify();
    }

    /// Draw every player as a tile, or as a pane, for moving between pages.
    /// How they look and nothing else: see `VideoView::set_compact`.
    ///
    /// Leaving the watch page also starts each pane's header over the
    /// picture over, hidden, as the player does its bar: the browse page
    /// never draws it, and a fade that came back after frames without its
    /// element would replay its last flip on the way back (see
    /// `motion::Fade::apply`). A reveal still running goes with it. Where
    /// the pointer was is left alone: it decides which pane takes the keys
    /// on the way back, and `go_watch_pane` sets it for that.
    pub(super) fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        for slot in &mut self.slots {
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_compact(compact, cx));
            }
            if compact {
                slot.header = motion::Fade::hidden();
                slot.revealed = false;
            }
        }
    }

    /// Mute all, or unmute all: every pane held silent, or let go.
    ///
    /// Per pane and for the session only. Each slot remembers it, so a player
    /// rebuilt under it is born quiet; a pane's first deliberate change of
    /// level ends it for that pane; and nothing of it reaches the settings —
    /// Mute all is not somebody deciding a channel should open silent. Letting
    /// go never un-mutes a pane that was muted by hand before it.
    pub(super) fn set_quiet_all(&mut self, quiet: bool, cx: &mut Context<Self>) {
        for slot in &mut self.slots {
            slot.quiet = quiet;
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_hushed(quiet, cx));
            }
        }
        cx.notify();
    }

    /// Leave the watch page.
    ///
    /// With the miniplayer on, the streams keep going, with their sound, as
    /// tiles in the mini player — cheaper to draw as well as smaller, since
    /// render size follows the element and a small one is scaled into a small
    /// buffer. Not cheaper to fetch: the rendition stays the one chosen for
    /// the watch grid, because `pane_height` measures that grid whichever page
    /// is up, so the tile never trades away the picture you go back to. With
    /// the miniplayer off they stop, which is what somebody who came here to
    /// pick the next thing wanted: a stream in a tile is still decoding and
    /// still pulling bytes. One step on the trail — which `Alt+←` takes back
    /// while something is still playing to go back to.
    pub(super) fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.record(|this| {
            this.page = Page::Browse;
            if this.settings.miniplayer {
                this.set_compact(true, cx);
            } else {
                this.retire_slots(|_| false, cx);
            }
            cx.notify();
        })
    }

    /// Back to watching, when there is anything to watch. One step on the
    /// trail.
    pub(super) fn go_watch(&mut self, cx: &mut Context<Self>) {
        self.record(|this| {
            if this.slots.is_empty() {
                return;
            }
            this.page = Page::Watch;
            this.set_compact(false, cx);
            cx.notify();
        })
    }

    /// Go back to watching with `key` the pane the keys talk to: what a click
    /// on a mini-player tile asks for.
    ///
    /// The pointer arrives on the watch page where the tile was, which can be
    /// over another pane's video. A pane's hover is measured every frame and
    /// its rising edge makes it active (`Slot::point`, from the watch page's
    /// probe), and `Slot::hovered` is whatever it was when the watch page was
    /// last up — so on the first frame that other pane would take the keys
    /// from the one just clicked, and M, the arrows and Ctrl+W would land on
    /// it. Counting every pane as already pointed at leaves that frame only
    /// falling edges, which change nothing: the pane under the pointer
    /// becomes active the next time the pointer comes into it. Its header
    /// over the picture, where it has one, still comes up, since `point`
    /// works that out every frame and not from the edge.
    pub(super) fn go_watch_pane(&mut self, key: String, cx: &mut Context<Self>) {
        self.active = Some(key);
        for slot in &mut self.slots {
            slot.hovered = true;
        }
        self.go_watch(cx);
    }

    pub(super) fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.retire_slots(|_| false, cx);
        self.page = Page::Browse;
        cx.notify();
    }

    /// Close every pane `keep` says no to.
    ///
    /// The one way panes are closed, because a recording's place has to be
    /// written down before its pane goes: the position lives on the slot,
    /// and a pane closed any other way takes the last few seconds of it with
    /// it. Dropping a slot stops its streamlink and its mpv.
    ///
    /// The last pane going takes the watch page off the trail too, both ways:
    /// there is nothing there to go back to, and a step that skipped it would
    /// be a press that seemed to do nothing. Every way the last pane goes
    /// comes through here — a close, Stop all, leaving with the mini player
    /// off, turning it off in the settings.
    pub(super) fn retire_slots(&mut self, keep: impl Fn(&Slot) -> bool, cx: &mut Context<Self>) {
        let recording_leaves = self.slots.iter().any(|slot| !slot.is_live() && !keep(slot));
        if recording_leaves && self.note_watching(cx) {
            self.save_history_soon(cx);
        }
        self.slots.retain(keep);
        if self.slots.is_empty() {
            self.trail.forget(|route| *route == Route::Watch);
        }
    }
}
