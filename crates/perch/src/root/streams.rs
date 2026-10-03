//! Opening, restarting and closing panes: a channel or a recording becomes a
//! `Slot`, streamlink is started for it, and its player is stood up when the
//! stream resolves. Everything a pane is *playing* is decided here, except
//! which rendition and when it restarts for one, which is `renditions`; how
//! it is drawn is `crate::watch`.

use std::sync::atomic::{AtomicU64, Ordering};

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{prelude::*, App, Context, Focusable, SharedString, Task, Window};
use settings::QualityPreference;
use streamlink::{Playlist, StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::{LiveStream, Video, VideoKind};

use super::navigation::Route;
use super::{Page, RootView};
use crate::chat::{ChatView, Feed};
use crate::chat_display::ChatDisplay;
use crate::rewind::{Origin, Rewind};
use crate::video::{Playback, PositionHandle, SizeHandle, StartOptions, Stopped, VideoStream};
use crate::video_view::{self, ChatButton, Qualities, Start, VideoView, Wake};
use crate::watch::{LiveInfo, Lookup, PendingStart, Restart, Slot, Source, StreamState, MAX_PANES};
use crate::{settings_view, vod};

/// Starting render size. Each pane measures itself on the first layout pass and
/// its render thread follows from then on, so this only decides what the first
/// frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// What a recording's pane says when streamlink resolved it with no
/// playlist to open.
pub(super) const NO_PLAYLIST: &str = "the recording came without a playlist";

/// How a pane's stream is started: in place of whatever the pane had, or
/// beside the picture on screen, to take over from it in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum How {
    /// The pane's one stream: opening, `Try again`, the settings sheet's
    /// restarts, and a restart of a pane with no picture yet. Whatever was
    /// resolving beside the pane is dropped.
    Cold,
    /// Beside the stream on screen, as `Slot::pending`, for the reason
    /// given; its player is started inside the pane's view once it resolves
    /// (`renditions`, `video_view::swap`).
    Beside(Restart),
}

/// Numbers every start of every pane's stream, for the life of the
/// process; see `Slot::generation`. One counter rather than one per pane, so
/// a number is never handed out twice — not even after the start it went to
/// was dropped — and so a log line's number names one start.
static GENERATIONS: AtomicU64 = AtomicU64::new(0);

/// The next start's number. Never zero, which is a slot that has had none.
pub(super) fn next_generation() -> u64 {
    GENERATIONS.fetch_add(1, Ordering::Relaxed) + 1
}

/// What a player of a pane on `source` is handed: the relay at `url` for a
/// live stream, or a recording's `playlist`, opened `start_at` seconds in.
/// `None` for a recording that came without its playlist.
///
/// The one place a recording's player is given its label, which is the
/// player's alone (`vod::playlist_label`), so no other player of the
/// recording — one started beside it at another rendition included — writes
/// or deletes its files.
pub(super) fn playback(
    source: &Source,
    url: String,
    quality: &str,
    playlist: Option<Playlist>,
    start_at: f64,
) -> Option<Playback> {
    match (source, playlist) {
        (Source::Video { video, position }, Some(playlist)) => Some(Playback::Vod {
            playlist,
            start_at,
            label: vod::playlist_label(&video.id, quality),
            position: position.clone(),
        }),
        (Source::Video { .. }, None) => None,
        (Source::Live, _) => Some(Playback::Live { url }),
    }
}

impl RootView {
    /// Open `channel`. With `solo`, it becomes the only pane; otherwise it is
    /// added alongside whatever is already playing. One step on the trail.
    ///
    /// A fifth pane is refused where you are, with a toast, rather than on
    /// the watch page: the page used to flip before the count was checked,
    /// so a refused `+ Add` from the browse page took you away from it for
    /// nothing.
    ///
    /// A pane added beside the others shows them all again if one had the
    /// page: a new pane has to be seen, and would otherwise open offstage.
    /// One already open is chosen instead (`choose`), which takes the
    /// maximize to it.
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
                this.restage(cx);
                this.choose(&channel, window, cx);
                return;
            }

            if solo {
                this.retire_slots(|_| false, cx);
            } else if this.slots.len() >= MAX_PANES {
                this.toast(format!("Already watching {MAX_PANES} streams"), cx);
                return;
            }
            this.show_watch_page(window, cx);
            this.stage.unmaximize();

            let slot = this.live_slot(channel.clone(), window, cx);
            this.slots.push(slot);

            // Remembered for the palette, which leads with it next time.
            if this.settings.note_watched(&channel) {
                this.save_settings(cx);
            }
            // A new seed for the rail's recommendations, and one channel
            // fewer to recommend.
            this.update_recommended();
            this.active = Some(channel.clone());
            this.start_stream(channel, How::Cold, window, cx);
            this.restage(cx);
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

    /// A pane on `channel` as it broadcasts, with its live chat: what opening
    /// a channel makes, and what `LIVE` on a rewound recording puts back in
    /// its place ([`replace_with_channel`](Self::replace_with_channel)).
    fn live_slot(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) -> Slot {
        let chat = cx.new(|cx| {
            ChatView::new(
                Feed::Live {
                    channel: channel.clone(),
                    history: self.settings.chat_history,
                },
                self.cache.clone(),
                ChatDisplay::of(&self.settings),
                window,
                cx,
            )
        });
        // The names of Shared Chat partners, and its word when it meets one
        // it cannot name, and its badges; see `root::shared_chat`.
        self.watch_chat(&chat, window, cx);
        let chat_hidden = self.settings.chat_hidden_for(&channel);
        Slot::new(
            channel.clone(),
            channel,
            Source::Live,
            Some(chat),
            0.0,
            chat_hidden,
        )
    }

    /// [`open_video`](Self::open_video), from `start_at` seconds in: where a
    /// link pointed, or where the history says it was left. One step on the
    /// trail, and refused where you are at four panes, as `open_channel` is,
    /// and like it shows every pane for a new one, or chooses one already
    /// open. The pane itself is [`video_slot`](Self::video_slot)'s.
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
                this.restage(cx);
                this.choose(&key, window, cx);
                return;
            }

            if solo {
                this.retire_slots(|_| false, cx);
            } else if this.slots.len() >= MAX_PANES {
                this.toast(format!("Already watching {MAX_PANES} streams"), cx);
                return;
            }
            this.show_watch_page(window, cx);
            this.stage.unmaximize();

            let channel = video.user_login.clone();
            let slot = this.video_slot(video, start_at, window, cx);
            this.slots.push(slot);

            // A recording counts as watching its channel, for the palette.
            if this.settings.note_watched(&channel) {
                this.save_settings(cx);
            }
            // And for the rail's recommendations.
            this.update_recommended();
            this.active = Some(key.clone());
            this.start_stream(key, How::Cold, window, cx);
            this.restage(cx);
            this.sync_quality(window, cx);
        })
    }

    /// A pane on `video`, opening `start_at` seconds in: what opening a
    /// recording makes, wherever it is opened from.
    ///
    /// Its chat is the replay of what was said at the moment on screen,
    /// following the pane's position from the moment it opens; and it goes
    /// into the history now rather than once it plays, so a recording opened
    /// and closed at once, or one that turns out to be gone, is still one you
    /// can find again.
    fn video_slot(
        &mut self,
        video: Video,
        start_at: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Slot {
        // Its muted stretches, for the seek bar, if it came without them:
        // from the history, which does not keep them. See `root::muted`.
        let video = self.with_muted(video);
        self.note_opened(&video, start_at, cx);

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
                    self.cache.clone(),
                    ChatDisplay::of(&self.settings),
                    window,
                    cx,
                )
            })
        });
        // For its badges: a replay carries no Shared Chat rooms, but its
        // speakers wear badges like a live chat's; see `root::badges`.
        if let Some(chat) = &chat {
            self.watch_chat(chat, window, cx);
        }
        // Chat hidden is remembered against the channel, like a live
        // pane's: hiding chat is a statement about the streamer, not the
        // broadcast.
        let chat_hidden = self.settings.chat_hidden_for(&channel);
        Slot::new(
            Slot::video_key(&video.id),
            channel,
            Source::Video {
                video: Box::new(video),
                position,
            },
            chat,
            start_at,
            chat_hidden,
        )
    }

    /// Play `video` from `start_at` in the pane `key` names, in its place:
    /// what a stopped live pane's last broadcast, `Watch from the start` and
    /// a rewind ask for. See [`replace_slot`](Self::replace_slot) for what a
    /// swap keeps and drops; the live slot it drops takes the channel's live
    /// chat with it, and its `Start when they go live`. What the live pane
    /// knew of its broadcast goes onto the recording as `origin`, for its way
    /// back to live (`rewind::back_to_live`).
    pub(super) fn replace_with_video(
        &mut self,
        key: &str,
        video: Video,
        start_at: f64,
        origin: Origin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let video_key = Slot::video_key(&video.id);
        self.replace_slot(
            key,
            video_key,
            move |this, window, cx| {
                let mut slot = this.video_slot(video, start_at, window, cx);
                slot.origin = origin;
                slot
            },
            window,
            cx,
        );
    }

    /// Play `channel` live in the pane `key` names, in its place: `LIVE` on
    /// a recording of the broadcast still going on, which goes back to the
    /// live edge as the rewind left it. Cold, as opening the channel is, with
    /// its live chat. See [`replace_slot`](Self::replace_slot); the
    /// recording's place goes into the history first.
    pub(super) fn replace_with_channel(
        &mut self,
        key: &str,
        channel: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_slot(
            key,
            channel.clone(),
            move |this, window, cx| this.live_slot(channel, window, cx),
            window,
            cx,
        );
    }

    /// Put the slot `make` makes, keyed `new_key`, in the pane `key` names,
    /// in its place: [`replace_with_video`](Self::replace_with_video) and
    /// [`replace_with_channel`](Self::replace_with_channel).
    ///
    /// A swap, not an opening. The pane keeps its place in the grid and the
    /// keys, so nothing moves; there is no count to refuse at, since it is
    /// one pane for one; and the trail takes no step, since the page is the
    /// watch page before and after. The old slot is dropped, which stops its
    /// streamlink and its chat; a recording's place is written down first,
    /// as closing its pane does (`retire_slots`), while a live slot has no
    /// place in the history to write. Mute all's hold stays: it is the
    /// pane's, and a press on a card is no change of level
    /// (`Slot::take_over_from`). A new slot rather than a new key on the old
    /// one, which everything keyed on the pane — element ids, the player's
    /// subscription, the active pane — would then misname.
    ///
    /// If what `new_key` names is already open in another pane, that pane is
    /// chosen instead (`choose`), and this one is left as it is.
    ///
    /// A pane in a window of its own comes home first, as Bring back brings
    /// it (`pop_in`): its pop-out finds its player by the pane's key, which
    /// the new slot does not carry. So `LIVE` or a rewind pressed in a
    /// pop-out closes that window and plays on in the main window, which
    /// comes up on the watch page if it was off it with the mini player off
    /// — there the pane would come home to nothing drawing it, and
    /// `restage` would stop it along with the new slot. A maximized pane
    /// stays maximized, under its new key (`Stage::rename`), and the new pane
    /// is chosen (`choose`), so while another pane has the watch page, this
    /// one takes it rather than playing where nobody can see it.
    fn replace_slot(
        &mut self,
        key: &str,
        new_key: String,
        make: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> Slot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.slot_index(&new_key).is_some() {
            self.choose(&new_key, window, cx);
            return;
        }
        if self.stage.is_popped(key) {
            if !self.draws_home_panes() {
                self.go_watch(cx);
            }
            self.come_home(key, cx);
        }
        let Some(index) = self.slot_index(key) else {
            return;
        };
        if !self.slots[index].is_live() && self.note_watching(cx) {
            self.save_history_soon(cx);
        }
        let mut slot = make(self, window, cx);
        slot.take_over_from(&self.slots[index]);
        self.slots[index] = slot;
        // The old pane's chat options menu does not carry over to its new
        // key, as it does not outlive a pane that leaves (`retire_slots`).
        if self.chat_menu.as_deref() == Some(key) {
            self.chat_menu = None;
        }
        self.stage.rename(key, &new_key);
        self.choose(&new_key, window, cx);
        self.start_stream(new_key, How::Cold, window, cx);
        self.restage(cx);
        self.sync_quality(window, cx);
        cx.notify();
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

    /// Start, or restart, streamlink for an existing slot, as `how` says:
    /// as the pane's one stream, or beside the one on screen.
    ///
    /// Every start is numbered (`Slot::generation`), and its events carry
    /// the number, so each reaches the start it came from — the pane's
    /// stream, or the one resolving beside it — and none reaches a pane that
    /// has moved on from it ([`apply_stream_event`](Self::apply_stream_event)).
    ///
    /// A start beside a live pane's picture names its rendition itself, from
    /// what the pane's menu offers ([`beside_rendition`](Self::beside_rendition)),
    /// so streamlink skips its probe (`StreamOptions::offered`); and takes
    /// the streamlink started ahead for that rendition when the menu opened,
    /// if there is one still good (`warm`). A cold start lets every one of
    /// those go: it is a restart for new settings or a new credential, which
    /// they were started without.
    pub(super) fn start_stream(
        &mut self,
        key: String,
        how: How,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.slot_index(&key) else {
            return;
        };
        let channel = self.slots[index].channel.clone();

        // Quality targets the pane this stream will actually render into.
        let pane_height = self.pane_height_for(&key, window);

        let known = match &how {
            How::Beside(_) => self.beside_rendition(index, pane_height, cx),
            How::Cold => None,
        };
        if let (How::Beside(reason), Some((name, _))) = (&how, &known) {
            if let Some(warm) = self.take_warm(index, name) {
                return self.adopt_warm(index, warm, pane_height, reason.clone(), cx);
            }
        }

        let rendition = known.as_ref().map(|(name, _)| name.clone());
        let options = match known {
            Some((name, offered)) => self.stream_options(Some(name), Some(offered)),
            None => self.stream_options(self.preference(index), None),
        };

        // A recording is only resolved: streamlink names the playlist and
        // retires, and the player opens it itself. See the streamlink crate.
        let (supervisor, events) = match &self.slots[index].source {
            Source::Live => StreamSupervisor::start(channel.clone(), pane_height, options),
            Source::Video { video, .. } => {
                StreamSupervisor::start_video(video.id.clone(), pane_height, options)
            }
        };
        let generation = next_generation();
        let pump = self.pump(key, &channel, generation, events, window, cx);

        match how {
            How::Cold => {
                let slot = &mut self.slots[index];
                slot.supervisor = Some(supervisor);
                slot.pump = Some(pump);
                slot.generation = generation;
                slot.warm.clear();
                slot.cooling = None;
                self.set_pending(index, None, cx);
                if let Some(view) = self.slots[index].video() {
                    view.update(cx, |view, _| view.cancel_swap());
                }
            }
            // The stream on screen keeps its streamlink: killing it would
            // end the picture the new player is getting ready to replace.
            // One resolving already is dropped, superseded; a pick says
            // from now on that it is under way (`set_pending`).
            How::Beside(reason) => {
                let pending = PendingStart {
                    supervisor,
                    pump,
                    generation,
                    for_height: pane_height,
                    reason,
                    rendition,
                };
                self.set_pending(index, Some(pending), cx);
            }
        }
    }

    /// The quality pane `index` asks streamlink for when streamlink is to
    /// choose: the rendition picked from the pane's menu, or the settings'
    /// fixed choice, or nothing for `Auto`, which streamlink matches to the
    /// pane's height.
    fn preference(&self, index: usize) -> Option<String> {
        self.slots[index]
            .quality_override
            .clone()
            .or(match &self.settings.quality {
                QualityPreference::Auto => None,
                QualityPreference::Fixed(name) => Some(name.clone()),
            })
    }

    /// How streamlink is asked for a pane's stream: at `quality`, a name or
    /// a preference, with the account's credential, and from `offered`, the
    /// renditions the pane already knows, when it does
    /// (`StreamOptions::offered`). For every start, and for one started
    /// ahead for the quality menu (`warm`).
    pub(super) fn stream_options(
        &self,
        quality: Option<String>,
        offered: Option<Vec<String>>,
    ) -> StreamOptions {
        StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
            offered,
        }
    }

    /// The task that hands each of a start's streamlink `events` to the
    /// root, under the pane's `key` and the start's `generation`
    /// ([`apply_stream_event`](Self::apply_stream_event)).
    ///
    /// The pane's volume is read once, here, and frozen into the pump: the
    /// pane it belongs to may not start for several seconds, and adjusting
    /// a *different* pane in the meantime must not follow it in.
    pub(super) fn pump(
        &self,
        key: String,
        channel: &str,
        generation: u64,
        mut events: UnboundedReceiver<StreamEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let volume = self
            .volume_override
            .unwrap_or_else(|| self.settings.volume_for(channel));
        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let key = key.clone();
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.apply_stream_event(&key, generation, event, volume, window, cx)
                });
                if ok.is_err() {
                    break;
                }
            }
        })
    }

    /// What streamlink said about the start `generation` of the pane `key`
    /// names. The pane's own stream's events change the pane; those of a
    /// start resolving beside it are `renditions`'s (`pending_event`);
    /// those of a streamlink started ahead for the pane's quality menu, which
    /// is neither until a pick takes it, are kept for that pick
    /// (`warm_event`); and those of a start the pane has moved on from go
    /// nowhere, by the rule the player's frame wakes go by
    /// (`video_view::route`).
    pub(super) fn apply_stream_event(
        &mut self,
        key: &str,
        generation: u64,
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
        let slot = &self.slots[index];
        let pending = slot.pending.as_ref().map(|pending| pending.generation);
        match video_view::route(generation, slot.generation, pending) {
            Wake::Current => {}
            Wake::Pending => return self.pending_event(index, event, cx),
            Wake::Stale => return self.warm_event(index, generation, event),
        }

        match event {
            StreamEvent::Resolving => self.set_slot_state(index, StreamState::Starting, cx),
            StreamEvent::Ready {
                url,
                quality,
                available,
                playlist,
            } => {
                // Playing again: whatever the pane learned about its
                // channel's past broadcasts when it last stopped is about
                // that stop, and the next one asks afresh; and an archive
                // found to rewind into was the last broadcast's. And which
                // broadcast this is, read while the live list still has it,
                // for finding its recording once it ends, or to rewind.
                let broadcast = self
                    .stream_info(&self.slots[index].channel)
                    .map(|stream| stream.id.clone())
                    .filter(|id| !id.is_empty());
                let live_since = self.live_since(&self.slots[index]);
                let back_to_live = self.still_live(&self.slots[index]);
                let slot = &mut self.slots[index];
                // An ask still out is forgotten here, and its answer is
                // let go by when it comes; see `Slot::stray_answers`.
                slot.stray_answers +=
                    usize::from(slot.archives.waiting()) + usize::from(slot.rewind.asking());
                slot.archives = Lookup::NotAsked;
                slot.rewind = Rewind::default();
                slot.broadcast = if slot.is_live() { broadcast } else { None };
                // What the player is handed: the relay for a live stream, or
                // the recording's playlist and where to open it.
                let start_at = self.slots[index].resume_at;
                let Some(playback) =
                    playback(&self.slots[index].source, url, &quality, playlist, start_at)
                else {
                    self.set_slot_state(index, StreamState::Failed(NO_PLAYLIST.into()), cx);
                    return;
                };
                // A pane Mute all is holding starts silent, at the level its
                // slider will show; and one started while you browse — a
                // quality change from the settings, a restart of a pane with
                // no picture yet — starts as the tile it replaces, not as a
                // big player with its controls up in the corner of the browse
                // page. One started in a pop-out starts there, with its
                // window's focus.
                let quiet = self.slots[index].quiet;
                let start = Start {
                    key: SharedString::from(key.to_string()),
                    place: self.place_of(key),
                    quiet,
                    focus: self.focus_of(key),
                    chat: ChatButton::of(
                        self.slots[index].chat_hidden,
                        self.slots[index].chat.is_some(),
                    ),
                    maximize: self.maximize_button_of(key),
                    // Nothing, but for a pick still resolving beside the
                    // player this one replaces, which the bar goes on saying.
                    switching: self.slots[index]
                        .pending
                        .as_ref()
                        .and_then(PendingStart::switching)
                        .cloned(),
                    // What a live pane's timeline runs from, if a list
                    // says; one that says later reaches the player through
                    // `sync_live_since`.
                    live_since,
                    // Whether a recording's broadcast is still on, for its
                    // way back to live; kept up by `sync_back_to_live`.
                    back_to_live,
                    // A recording's muted stretches as far as the root
                    // knows them; its own look, if one is out, reaches the
                    // player through `take_muted`.
                    muted: self.slots[index]
                        .recording()
                        .map(|video| video.muted_segments.clone())
                        .unwrap_or_default(),
                    // What More offers about hearing one pane alone; kept
                    // up by `sync_hear_only`.
                    hear_only: self.hear_only_now(),
                    // Whether the guide is up from this pane, for the bar's
                    // Guide button; kept up by `sync_guide_buttons`.
                    guide_from_here: self.guide.up_from(key),
                };
                // The pane's one player: its own size until the pane is
                // measured, playing, and its position heard by the pane —
                // the chat replay, the history — from the start.
                let options = StartOptions {
                    size: SizeHandle::new(RENDER_WIDTH, RENDER_HEIGHT),
                    volume: if quiet { 0 } else { volume },
                    paused: false,
                    publish: true,
                };
                match VideoStream::start(options, playback) {
                    Ok((stream, frames)) => {
                        let qualities = self.qualities(index, quality, available);
                        let view = cx.new(|cx| {
                            VideoView::from_stream(
                                stream, frames, qualities, volume, generation, start, cx,
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
                        self.set_slot_state(index, StreamState::Playing(view), cx);
                    }
                    Err(e) => {
                        self.set_slot_state(index, StreamState::Failed(e.to_string().into()), cx)
                    }
                }
            }
            StreamEvent::Offline => {
                self.set_slot_state(index, StreamState::Offline, cx);
                // What the channel broadcast last, for the pane to offer.
                self.ask_broadcasts(index);
            }
            StreamEvent::Failed { reason } => {
                self.set_slot_state(index, StreamState::Failed(reason.into()), cx)
            }
            // Twitch is playing an ad that streamlink is filtering out; the
            // header says so (`ad_breaks`).
            StreamEvent::AdBreak { secs } => self.ad_break_began(index, secs, cx),
        }
        cx.notify();
    }

    /// What the menu of the pane at `index` offers, with `quality` playing
    /// out of `available`: for its first player, and for one started beside
    /// it.
    pub(super) fn qualities(
        &self,
        index: usize,
        quality: String,
        available: Vec<String>,
    ) -> Qualities {
        Qualities {
            playing: SharedString::from(quality),
            available,
            default: settings_view::quality_label(&self.settings.quality),
            picked: self.slots[index].quality_override.is_some(),
        }
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
    ///
    /// Helix's lists only. The rail's recommendations are fetched too, but
    /// are not `LiveStream`s; what a pane says about its channel reads them
    /// as well, through [`live_info`](Self::live_info) and
    /// [`channel_name`](Self::channel_name).
    pub(super) fn stream_info(&self, channel: &str) -> Option<&LiveStream> {
        self.listed_stream(channel).or_else(|| {
            self.pane_streams
                .streams
                .iter()
                .find(|stream| stream.user_login == channel)
        })
    }

    /// [`stream_info`](Self::stream_info) from the lists the app fetches for
    /// their own sake alone, without the streams asked for an open pane that
    /// none of them carries: which panes still need asking about
    /// (`root::pane_streams`).
    pub(super) fn listed_stream(&self, channel: &str) -> Option<&LiveStream> {
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
            self.guide.streams.items.as_slice(),
            search,
        ]
        .into_iter()
        .flatten()
        .find(|stream| stream.user_login == channel)
    }

    /// What a live pane's header says about its channel's broadcast — how
    /// many are watching, since when, what is on — from whichever list knows
    /// it: [`stream_info`](Self::stream_info)'s, and failing those the rail's
    /// recommendations, which are the only list that has a channel opened
    /// from the Recommended group (`Recommended::channel`).
    pub(super) fn live_info(&self, channel: &str) -> Option<LiveInfo<'_>> {
        self.stream_info(channel)
            .map(LiveInfo::from)
            .or_else(|| self.recommended.channel(channel).map(LiveInfo::from))
    }

    /// The name `login` writes itself as, from whichever list knows it: a
    /// live list, the offline follows, or the rail's recommendations. `None`
    /// when none does.
    ///
    /// The one lookup behind [`display_name`](Self::display_name) and the
    /// names a recommendation's reason gives its seeds, so a channel opened
    /// from the Recommended group is "AdmiralBulldog" on the row, in the pane
    /// it opens, and in the reasons that name it later.
    pub(super) fn channel_name(&self, login: &str) -> Option<&str> {
        self.stream_info(login)
            .map(|stream| stream.display_name.as_str())
            .or_else(|| {
                self.offline
                    .iter()
                    .find(|channel| channel.login == login)
                    .map(|channel| channel.display_name.as_str())
            })
            .or_else(|| {
                self.recommended
                    .channel(login)
                    .map(|channel| channel.display_name.as_str())
            })
    }

    /// What a pane calls its channel: the name the channel writes itself as,
    /// from whichever list knows it ([`channel_name`](Self::channel_name)),
    /// or the login when none does. A recording's own, for a recording.
    ///
    /// One answer for the pane header, its status line, the mini player and
    /// the palette. They used to ask different lists, so a channel you
    /// follow that was offline was "Nubzombie" in the palette and
    /// "nubzombie" in the pane it opened.
    pub(super) fn display_name(&self, slot: &Slot) -> String {
        if let Some(video) = slot.recording() {
            return video.user_name.clone();
        }
        self.channel_name(&slot.channel)
            .unwrap_or(&slot.channel)
            .to_string()
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
        // And whatever was resolving beside it: there is no picture left to
        // take over from, and `Try again` starts afresh.
        self.set_pending(index, None, cx);
        // Replacing the state drops the player, and with it the frozen frame.
        let state = match reason {
            Stopped::Ended => StreamState::Ended,
            Stopped::Failed(message) => StreamState::Failed(message.into()),
        };
        let ended = matches!(state, StreamState::Ended);
        self.set_slot_state(index, state, cx);
        // A recording that reached its end is finished, and one that failed
        // is left where it failed: either way the history hears now.
        if !self.slots[index].is_live() && self.note_watching(cx) {
            self.save_history_soon(cx);
        }
        // A broadcast that ended has a recording to watch from its start,
        // if the channel keeps one: ask for it, knowing which broadcast this
        // was — read now if the live list did not have it when the picture
        // came. The pane forgot any earlier answer when it began to play,
        // so this always asks afresh; that answer predates this recording.
        if ended && self.slots[index].is_live() {
            if self.slots[index].broadcast.is_none() {
                self.slots[index].broadcast = self
                    .stream_info(&self.slots[index].channel)
                    .map(|stream| stream.id.clone())
                    .filter(|id| !id.is_empty());
            }
            self.ask_broadcasts(index);
        }
        cx.notify();
    }

    /// Ask for a pane's stream again: after it stopped, or after the channel
    /// came back on. Whatever the pane was showing becomes "Starting…".
    pub(super) fn retry_stream(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(key) else {
            return;
        };
        self.set_slot_state(index, StreamState::Starting, cx);
        self.start_stream(key.to_string(), How::Cold, window, cx);
        cx.notify();
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

    /// Mute all, or unmute all: every pane held silent, or let go.
    ///
    /// Per pane and for the session only. Each slot remembers it, so a player
    /// rebuilt under it is born quiet; a pane's first deliberate change of
    /// level ends it for that pane; and nothing of it reaches the settings —
    /// Mute all is not somebody deciding a channel should open silent. Letting
    /// go never un-mutes a pane that was muted by hand before it.
    ///
    /// More's `Hear all again` is the letting go (`hearing`).
    pub(super) fn set_quiet_all(&mut self, quiet: bool, cx: &mut Context<Self>) {
        for slot in &mut self.slots {
            slot.quiet = quiet;
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_hushed(quiet, cx));
            }
        }
        self.sync_hear_only(cx);
        cx.notify();
    }

    /// Leave the watch page.
    ///
    /// With the miniplayer on, the streams keep going, with their sound, as
    /// tiles in the mini player — cheaper to draw as well as smaller, since
    /// render size follows the element and a small one is scaled into a small
    /// buffer. Not cheaper to fetch: the rendition stays the one chosen for
    /// the watch grid, because `pane_height_for` measures that grid whichever
    /// page is up, so the tile never trades away the picture you go back to. With
    /// the miniplayer off they stop, which is what somebody who came here to
    /// pick the next thing wanted: a stream in a tile is still decoding and
    /// still pulling bytes. One step on the trail — which `Alt+←` takes back
    /// while something is still playing to go back to.
    ///
    /// A pane in a window of its own plays on either way: it is somewhere
    /// else on screen, and was put there to be kept. Which panes stop is
    /// `retire_homeless`'s, the one rule for it.
    pub(super) fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.record(|this| {
            this.page = Page::Browse;
            // A pane's chat options menu does not wait on another page to
            // come back up with it.
            this.chat_menu = None;
            this.retire_homeless(cx);
            this.restage(cx);
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
            this.restage(cx);
        })
    }

    /// Go back to watching with `key` the pane the keys talk to: what a click
    /// on a mini-player tile asks for, and the palette's quality row from the
    /// browse page. Chosen (`choose`), so a pane that had the watch page
    /// when it was left hands it to the one clicked.
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
    pub(super) fn go_watch_pane(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for slot in &mut self.slots {
            slot.hovered = true;
        }
        self.go_watch(cx);
        self.choose(&key, window, cx);
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
    ///
    /// The rail's recommendations hear of it here too, for the same reason:
    /// a pane gone is a seed that may have gone with it, and a channel that
    /// may be recommended again. And so does the stage, which closes the
    /// window of a pane that was popped out (`restage`).
    pub(super) fn retire_slots(&mut self, keep: impl Fn(&Slot) -> bool, cx: &mut Context<Self>) {
        let recording_leaves = self.slots.iter().any(|slot| !slot.is_live() && !keep(slot));
        if recording_leaves && self.note_watching(cx) {
            self.save_history_soon(cx);
        }
        // A pane's chat options menu goes with its pane, whichever way it
        // leaves (a close from the keys or the palette, Stop all), rather
        // than waiting for a watch page that may not come back to clear it:
        // left set, it would open again on its own the next time that
        // channel is opened.
        if let Some(open) = self.chat_menu.as_deref() {
            if self
                .slots
                .iter()
                .any(|slot| slot.key == open && !keep(slot))
            {
                self.chat_menu = None;
            }
        }
        self.slots.retain(keep);
        if self.slots.is_empty() {
            self.trail.forget(|route| *route == Route::Watch);
        }
        self.update_recommended();
        self.restage(cx);
    }
}
