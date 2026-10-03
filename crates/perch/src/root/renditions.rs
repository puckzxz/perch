//! What each pane plays, and when it restarts: the rendition a pane's stream
//! is resolved at, chosen against that pane's own height
//! ([`pane_height_for`](RootView::pane_height_for)); the re-pick when the
//! grid changes, which only ever moves upwards
//! ([`sync_quality`](RootView::sync_quality)); a pick from the pane's own
//! menu ([`request_quality`](RootView::request_quality)); and how each of
//! them changes what plays ([`change_rendition`](RootView::change_rendition)).
//!
//! A pane with a picture is not restarted for a rendition. The new one is
//! resolved beside the stream on screen (`Slot::pending`), and its player is
//! started inside the pane's own view, silent, to take over in place once it
//! is ready (`video_view::swap`): no black, no poster, the sound and the
//! pause carried over, and a recording's place kept. The root's half of
//! that is here — what a start resolving beside a pane does with each of its
//! events ([`pending_event`](RootView::pending_event)), and what the pane
//! keeps once the new player has taken over or been given up on
//! ([`on_swapped`](RootView::on_swapped),
//! [`on_swap_failed`](RootView::on_swap_failed)). A pane without a picture,
//! and the settings sheet's restarts, start cold
//! ([`restart_stream`](RootView::restart_stream)).
//!
//! Every write of the start resolving beside a pane goes through
//! [`set_pending`](RootView::set_pending), which also tells the pane's player
//! what a pick from its menu is switching it to, so the bar says so from the
//! press until the new rendition takes over or the pick fails. A test below
//! holds the root to it.
//!
//! Named for renditions rather than for quality: `streamlink::quality` is
//! the module every question here is put to, and goes by that name.

use std::time::Instant;

use gpui::{App, Context, SharedString, Window};
use settings::QualityPreference;
use streamlink::{quality, StreamEvent};

use super::streams::{self, How, NO_PLAYLIST};
use super::RootView;
use crate::layout;
use crate::settings_view;
use crate::stage::Place;
use crate::video::{StartOptions, VideoStream};
use crate::video_view::{lead, Switching};
use crate::watch::{PendingStart, Restart, Slot, StreamState};

impl RootView {
    /// Start pane `index`'s stream over, cold, from where a recording has
    /// got to: the picture goes, and the pane says it is starting until the
    /// new player's first frame. For the settings sheet's quality and
    /// credential changes, which restart every pane at once and are left
    /// cold on purpose — a swap per pane would be two players and two
    /// streamlink sessions per pane at once — and for a rendition change on
    /// a pane with no picture to keep ([`change_rendition`](Self::change_rendition)).
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
        let key = slot.key.clone();
        self.set_slot_state(index, StreamState::Starting, cx);
        self.start_stream(key, How::Cold, window, cx);
    }

    /// Change what pane `index` plays, for `reason`: beside the picture on
    /// screen, to take over in place, when the pane has one that covers it
    /// ([`restart_how`]); otherwise cold. A start already resolving beside
    /// the pane is superseded, and its player, if it has one, called off.
    pub(super) fn change_rendition(
        &mut self,
        index: usize,
        reason: Restart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        let covered = slot.video().is_some_and(|view| view.read(cx).covers());
        match restart_how(covered, reason) {
            How::Cold => self.restart_stream(index, window, cx),
            how @ How::Beside(_) => {
                if let Some(view) = slot.video() {
                    view.update(cx, |view, _| view.cancel_swap());
                }
                let key = slot.key.clone();
                self.start_stream(key, how, window, cx);
            }
        }
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

    /// The rendition a start beside pane `index`'s picture serves, for a
    /// pane `pane_height` tall, and what the pane's menu offers, which it is
    /// chosen from: the question streamlink would otherwise answer after a
    /// probe of its own, a second streamlink resolving the channel again
    /// (`StreamOptions::offered`), asked the way it would ask it — the
    /// rendition picked by hand, or what the settings pick at that height.
    /// The name is also what finds the streamlink started ahead for it
    /// (`warm`). `None` for a recording, whose start needs the probe's
    /// playlists, and for a pane whose player knows of no rendition.
    pub(super) fn beside_rendition(
        &self,
        index: usize,
        pane_height: u32,
        cx: &App,
    ) -> Option<(String, Vec<String>)> {
        let slot = &self.slots[index];
        if !slot.is_live() {
            return None;
        }
        let available = slot.video()?.read(cx).available().to_vec();
        let pick = match &slot.quality_override {
            Some(name) => quality::select_named(&available, name, pane_height),
            None => self.settings_pick(&available, pane_height),
        }?;
        Some((pick.name, available))
    }

    /// A quality chosen from pane `index`'s own menu: a rendition, which holds
    /// until the pane closes, or `None`, which hands the pane back to the
    /// settings.
    ///
    /// What plays changes only when that changes it — a change runs a second
    /// stream beside the first for seconds, and pinning the rendition already
    /// playing, or going back to a default that picks the same one, should
    /// cost nothing; it only calls off a rendition still resolving for an
    /// earlier ask. Going back re-picks for the pane as it is now, down as
    /// well as up: that is an answer somebody asked for, where the re-pick
    /// after a resize is not.
    ///
    /// A change carries what the pane's bar says until it is done
    /// ([`Switching`]): the rendition picked, or for the settings' row what
    /// the settings pick now. For a stream none of whose renditions the
    /// settings can choose from — no rendition named by its height — that is
    /// nothing, and the bar says the settings' choice in a word instead
    /// (`settings_view::quality_word`: `Auto`, `Best`), short enough for the
    /// room the pill keeps; the row pressed keeps the sheet's longer words.
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
            None => {
                let pane_height = self.pane_height_for(&self.slots[index].key, window);
                self.settings_pick(&available, pane_height)
                    .map(|pick| pick.name)
            }
        };
        let picked = name.is_some();
        self.slots[index].quality_override = name;
        if target.as_deref() == Some(playing.as_str()) {
            // Nothing to change, so the menu the pane already has says it.
            self.set_pending(index, None, cx);
            view.update(cx, |view, cx| {
                view.cancel_swap();
                view.set_picked(picked, cx);
            });
        } else {
            let switching = Switching {
                to: target.map_or_else(
                    || settings_view::quality_word(&self.settings.quality),
                    SharedString::from,
                ),
                default: !picked,
                since: Instant::now(),
            };
            self.pick(index, switching, window, cx);
        }
        cx.notify();
    }

    /// Carry out `switching`, a pick from pane `index`'s menu of something
    /// other than what plays, given the pick already under way there, if one
    /// is ([`again`]).
    ///
    /// The same pick again asks nothing new, so the start resolving for it,
    /// and a player it may already have lined up, go on, and the wait the
    /// swap's log line counts goes on from the first press. The same
    /// rendition from the other row — the settings' row naming what was
    /// picked by hand, or the other way round — only moves the menu's mark
    /// (and the override `request_quality` has already set), so the start
    /// goes on too. Anything else starts afresh, superseding it.
    fn pick(
        &mut self,
        index: usize,
        switching: Switching,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let under_way = self.slots[index]
            .pending
            .as_ref()
            .and_then(PendingStart::switching);
        match again(under_way, &switching) {
            Again::Same => {}
            Again::OtherRow => {
                if let Some(mut pending) = self.set_pending(index, None, cx) {
                    pending.reason = Restart::Pick(Switching {
                        since: pending.switching().map_or(switching.since, |old| old.since),
                        ..switching
                    });
                    self.set_pending(index, Some(pending), cx);
                }
            }
            Again::New => self.change_rendition(index, Restart::Pick(switching), window, cx),
        }
    }

    /// Set what resolves beside pane `index`'s picture (`Slot::pending`),
    /// and hand back the start it replaces, if any: the only write of it,
    /// which a test holds the root to.
    ///
    /// And the one place the pane's player hears what a pick from its menu
    /// is switching it to ([`PendingStart::switching`],
    /// `VideoView::set_switching`), so the bar says a pick is under way for
    /// exactly as long as its start is pending: from the press
    /// (`request_quality`, through `start_stream`), through streamlink's
    /// resolve and the new player getting ready, until it takes over
    /// (`on_swapped`), is given up on (`on_swap_failed`, `drop_pending`),
    /// called off (`request_quality`'s nothing to change), superseded by
    /// another start (`start_stream`, beside or cold), or goes with the
    /// pane's stream (`stream_stopped`). The player cannot work it out
    /// itself: its own pending swap begins only once streamlink has
    /// resolved, and `begin_swap` drops it and makes it again.
    ///
    /// A pane closed or replaced whole drops its slot, pending start and
    /// player together, so nothing here is left to tell; a player made
    /// afresh is told at its birth (`Start::switching`).
    pub(super) fn set_pending(
        &mut self,
        index: usize,
        pending: Option<PendingStart>,
        cx: &mut Context<Self>,
    ) -> Option<PendingStart> {
        let slot = &mut self.slots[index];
        let replaced = std::mem::replace(&mut slot.pending, pending);
        let switching = slot
            .pending
            .as_ref()
            .and_then(PendingStart::switching)
            .cloned();
        if let Some(view) = slot.video() {
            view.update(cx, |view, cx| view.set_switching(switching, cx));
        }
        replaced
    }

    /// How tall the pane `key` names is right now, in physical pixels: what
    /// its quality is chosen against, when its stream starts and again in
    /// [`sync_quality`](Self::sync_quality). With several panes the window is
    /// shared, so each one asks for proportionally less.
    ///
    /// Its cell of the watch grid, the one grid ([`grid`](Self::grid)), on
    /// whichever page is up: a mini-player tile is not one of the sizes a
    /// quality is chosen for. The cell's share of the body's height, seams
    /// included (`Grid::share_height`, which says why), at the main window's
    /// scale, which is what `window` always is. The maximized pane's cell is
    /// the whole body, the one cell that grid has while it lasts.
    ///
    /// Except a pane maximized away, which has no cell in that grid: it is
    /// measured by the grid it comes back to, every pane's, so the maximize
    /// never moves it, and showing every pane again moves none back down.
    ///
    /// And a pane in a window of its own, which is measured by that
    /// window: the picture's height there, in that window's own physical
    /// pixels (`PoppedOut::height`), which its window keeps the root told
    /// of. Its cell in the main window says only where it will come back to.
    pub(super) fn pane_height_for(&self, key: &str, window: &Window) -> u32 {
        let own_window = self.stage.popped(key).map(|popped| popped.height);
        let maximized_away = self
            .stage
            .maximized()
            .is_some_and(|maximized| maximized != key);
        let grid = if maximized_away {
            layout::Grid::of(self.body(window), self.slots.len())
        } else {
            self.grid(window)
        };
        measured_height(own_window, grid.share_height * window.scale_factor())
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
    /// a *sharper* answer changes what plays — beside the picture, which it
    /// takes over in place (`change_rendition`), so the change costs a second
    /// stream for a few seconds rather than any black. A pane that has shrunk
    /// keeps what it has: the smaller pane hides nothing, and a sharper
    /// picture is what is worth paying for, not a cheaper one. A quality
    /// picked by hand from the pane's own menu is left alone; that choice was
    /// about this pane, whatever its size. So is a pane already resolving a
    /// rendition at least as sharp ([`wants_swap`]), which every call while
    /// it resolves would otherwise start again. One resolving a pick of the
    /// settings' row that a sharper answer supersedes stays a pick
    /// ([`re_pick`]), so the bar goes on saying the pane is switching.
    ///
    /// Each pane is measured on its own ([`pane_height_for`](Self::pane_height_for)),
    /// and its log line says the height it was measured at. A pane in a
    /// window of its own is measured by that window, whose resizes call this
    /// as the main window's do (`pop_out_moved`); a small pop-out keeps what
    /// it played at home, by the same upwards-only rule. A maximized pane is
    /// measured by the whole body, and so moves up when it is given the page
    /// (`maximize_changed`); the panes maximized away are left until the
    /// grid comes back, when this runs again for them — from
    /// `maximize_changed`, or from `restage` for a maximize that ends there.
    pub(super) fn sync_quality(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restart: Vec<(usize, u32, Restart)> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.quality_override.is_none())
            .filter(|(_, slot)| self.place_of(&slot.key) != Place::Offstage)
            .filter_map(|(index, slot)| {
                let pane_height = self.pane_height_for(&slot.key, window);
                let view = slot.video()?.read(cx);
                let wanted = self.settings_pick(view.available(), pane_height)?;
                let playing = quality::parse_quality(view.quality()).map_or(0, |q| q.height);
                // What a start resolving beside the pane will most likely
                // play: the same question, asked for the height it started
                // for.
                let resolving = slot.pending.as_ref().map(|pending| {
                    self.settings_pick(view.available(), pending.for_height)
                        .map_or(0, |pick| pick.height)
                });
                let under_way = slot.pending.as_ref().and_then(PendingStart::switching);
                (wanted.height > playing && wants_swap(resolving, wanted.height))
                    .then(|| (index, pane_height, re_pick(under_way, &wanted.name)))
            })
            .collect();
        let restarted = !restart.is_empty();
        for (index, pane_height, reason) in restart {
            eprintln!(
                "video: {} re-choosing quality for a {pane_height}px pane",
                self.slots[index].key
            );
            self.change_rendition(index, reason, window, cx);
        }
        if restarted {
            cx.notify();
        }
    }

    /// What streamlink said about the start resolving beside the picture of
    /// pane `index` (`Slot::pending`); see `apply_stream_event`, which sends
    /// it here.
    ///
    /// Resolving changes nothing: the pane is playing, and saying it is
    /// starting would drop its picture. A failure drops the start and leaves
    /// the pane as it is. Ready starts the new player inside the pane's view
    /// (`VideoView::begin_swap`) — silent, at the pane's size, heard by
    /// nobody, and a recording ahead of where the pane is
    /// ([`swap_start_at`]) — unless it is a re-pick that came back naming
    /// what already plays.
    pub(super) fn pending_event(
        &mut self,
        index: usize,
        event: StreamEvent,
        cx: &mut Context<Self>,
    ) {
        let (url, quality, available, playlist) = match event {
            StreamEvent::Resolving => return,
            StreamEvent::Offline => return self.drop_pending(index, "the channel is off", cx),
            StreamEvent::Failed { reason } => return self.drop_pending(index, &reason, cx),
            // An ad on the stream getting ready says nothing about the
            // picture on screen, which plays on; the start is not taken
            // over until it has frames of its own anyway.
            StreamEvent::AdBreak { .. } => return,
            StreamEvent::Ready {
                url,
                quality,
                available,
                playlist,
            } => (url, quality, available, playlist),
        };
        let slot = &self.slots[index];
        let (Some(pending), Some(view)) = (slot.pending.as_ref(), slot.video().cloned()) else {
            return self.drop_pending(index, "there is no picture to take over from", cx);
        };
        let (generation, picked) = (pending.generation, pending.reason.is_pick());
        let (playing, position, paused, speed, size) = {
            let view = view.read(cx);
            (
                view.quality().to_string(),
                view.position(),
                view.is_paused(),
                view.speed().factor(),
                view.size_handle(),
            )
        };
        if !worth_swapping(picked, &playing, &quality) {
            eprintln!(
                "video: {} already plays {quality}; nothing to swap",
                slot.key
            );
            self.set_pending(index, None, cx);
            return;
        }
        let start_at = swap_start_at(position, paused, speed);
        let Some(playback) = streams::playback(&slot.source, url, &quality, playlist, start_at)
        else {
            return self.drop_pending(index, NO_PLAYLIST, cx);
        };
        let options = StartOptions {
            size,
            volume: 0,
            paused: false,
            publish: false,
        };
        match VideoStream::start(options, playback) {
            Ok((stream, frames)) => {
                let qualities = self.qualities(index, quality, available);
                view.update(cx, |view, cx| {
                    view.begin_swap(stream, frames, qualities, generation, cx)
                });
            }
            Err(e) => self.drop_pending(index, &e.to_string(), cx),
        }
    }

    /// Give up on the start resolving beside pane `index`'s picture, which
    /// plays on as it was, and say so in the log — and on screen, for a
    /// rendition picked by hand ([`pick_failed`](Self::pick_failed)). A
    /// re-pick that fails waits for the next time the pane grows.
    fn drop_pending(&mut self, index: usize, why: &str, cx: &mut Context<Self>) {
        let Some(pending) = self.set_pending(index, None, cx) else {
            return;
        };
        if let Some(view) = self.slots[index].video() {
            view.update(cx, |view, _| view.cancel_swap());
        }
        let picked = pending.reason.is_pick();
        eprintln!(
            "video: {} couldn't start a {} beside the picture: {why}",
            self.slots[index].key,
            if picked { "pick" } else { "re-pick" }
        );
        if picked {
            self.pick_failed(index, "Couldn't switch quality".to_string(), cx);
        }
    }

    /// The player started beside the picture of the pane `owner` names, for
    /// the start `generation`, has taken over. That start's streamlink and
    /// pump become the pane's, and the old ones go — only now, after the
    /// player reading the old stream has been stopped (`swap`'s promote).
    pub(super) fn on_swapped(&mut self, owner: &str, generation: u64, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(owner) else {
            return;
        };
        if !self.is_pending(index, generation) {
            eprintln!("video: {owner} swapped to start {generation}, which it was not waiting for");
        } else if let Some(pending) = self.set_pending(index, None, cx) {
            let slot = &mut self.slots[index];
            slot.supervisor = Some(pending.supervisor);
            slot.pump = Some(pending.pump);
            slot.generation = generation;
            // An ad the old streamlink said is not this one's: the new
            // player took over with a picture, so it is not in one.
            slot.end_ad_break();
            // Whether the menu marks the settings' row or the rendition, by
            // the override as it is now: the player lined up was told it as
            // it was when streamlink answered, and the other row's press for
            // the same rendition since moves the mark without a start of its
            // own (`pick`).
            let picked = slot.quality_override.is_some();
            if let Some(view) = slot.video() {
                view.update(cx, |view, cx| view.set_picked(picked, cx));
            }
        }
        cx.notify();
    }

    /// Whether the start resolving beside pane `index`'s picture is the
    /// start `generation`: what a swap's news is about, or a start the pane
    /// has since moved on from.
    fn is_pending(&self, index: usize, generation: u64) -> bool {
        self.slots[index]
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation == generation)
    }

    /// The player started beside the picture of the pane `owner` names, for
    /// the start `generation`, at `quality`, was given up on. Its streamlink
    /// goes; the pane plays on as it was.
    pub(super) fn on_swap_failed(
        &mut self,
        owner: &str,
        generation: u64,
        quality: &SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.slot_index(owner) else {
            return;
        };
        if !self.is_pending(index, generation) {
            return;
        }
        let picked = self
            .set_pending(index, None, cx)
            .is_some_and(|pending| pending.reason.is_pick());
        if picked {
            self.pick_failed(index, format!("Couldn't switch to {quality}"), cx);
        }
    }

    /// A rendition picked by hand for pane `index` could not be switched to:
    /// say so, and hand the pane back to what it plays, so its menu and what
    /// its next start asks for agree with the picture.
    fn pick_failed(&mut self, index: usize, message: String, cx: &mut Context<Self>) {
        let back = self.slots[index].video().and_then(|view| {
            let view = view.read(cx);
            view.picked().then(|| view.quality().to_string())
        });
        self.slots[index].quality_override = back;
        self.toast(message, cx);
    }
}

/// The height in physical pixels a pane's quality is chosen for: its own
/// window's picture, when it is in a window of its own (`own_window`), and
/// otherwise its `cell`'s share of the main window's grid.
fn measured_height(own_window: Option<f32>, cell: f32) -> u32 {
    layout::quality_height(own_window.unwrap_or(cell))
}

/// How a rendition change starts: beside the picture when there is one that
/// covers the pane, so it can take over in place; cold otherwise, since a
/// pane still starting or fading in has nothing on screen to keep.
fn restart_how(covered: bool, reason: Restart) -> How {
    if covered {
        How::Beside(reason)
    } else {
        How::Cold
    }
}

/// Where a recording's new player opens, given where the pane is and how
/// fast it plays: there, if it is paused, and the swap's lead on if it is
/// playing (`video_view::lead`: four seconds of the clock, twice as far
/// into the recording at double speed), so the new player has a picture
/// just before the pane reaches it.
fn swap_start_at(position: f64, paused: bool, speed: f64) -> f64 {
    if paused {
        position
    } else {
        position + lead(speed)
    }
}

/// Whether a re-pick wanting a rendition `wanted` px tall is worth a start,
/// with one already resolving for a rendition `resolving` px tall, if any:
/// only a taller want supersedes it.
fn wants_swap(resolving: Option<u32>, wanted: u32) -> bool {
    resolving.is_none_or(|height| height < wanted)
}

/// Whether a start that resolved to `resolved` is worth swapping to, with
/// `playing` on screen, for a pick from the menu (`picked`) or a re-pick: a
/// re-pick that names what already plays is not, and a pick always is —
/// somebody asked.
fn worth_swapping(picked: bool, playing: &str, resolved: &str) -> bool {
    picked || playing != resolved
}

/// What a new pick makes of the one already under way on its pane; see
/// [`RootView::pick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Again {
    /// Nothing new was asked: the start under way goes on as it is.
    Same,
    /// The same rendition from the other row: the start goes on, and the
    /// pane's bar marks the row now pressed.
    OtherRow,
    /// Something else, or nothing was under way: a start of its own.
    New,
}

/// Whether `new`, a pick from a pane's menu, asks anything of the pick
/// `under_way` there, if one is, that its start is not already doing.
fn again(under_way: Option<&Switching>, new: &Switching) -> Again {
    match under_way {
        Some(old) if old.to == new.to && old.default == new.default => Again::Same,
        Some(old) if old.to == new.to => Again::OtherRow,
        _ => Again::New,
    }
}

/// Why pane re-picks, for the settings at its size now wanting `wanted`,
/// with a pick `under_way` beside its picture, if one is.
///
/// A re-pick, silent, as a rule. But one that supersedes a pick still under
/// way — a pick of the settings' row, since a pane picked by hand is never
/// re-picked (`sync_quality`), whose pane then grew — carries the pick on to
/// what the settings want now, with its wait counted from the press: the
/// bar goes on saying so rather than falling quiet as if it had ended, and
/// a failure is still said.
fn re_pick(under_way: Option<&Switching>, wanted: &str) -> Restart {
    match under_way {
        Some(old) => Restart::Pick(Switching {
            to: SharedString::from(wanted.to_string()),
            default: true,
            since: old.since,
        }),
        None => Restart::RePick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pick from the menu, of `to`.
    fn pick(to: &str) -> Restart {
        Restart::Pick(Switching {
            to: SharedString::from(to.to_string()),
            default: false,
            since: Instant::now(),
        })
    }

    /// A pane in a window of its own is measured by that window, larger or
    /// smaller than the cell it keeps at home, and by the same clamp a cell
    /// is; a pane at home by its cell.
    #[test]
    fn a_popped_pane_is_measured_by_its_own_window() {
        assert_eq!(
            measured_height(Some(1080.0), 405.0),
            1080,
            "a large pop-out"
        );
        assert_eq!(measured_height(Some(270.0), 540.0), 270, "a small pop-out");
        assert_eq!(
            measured_height(Some(120.0), 540.0),
            180,
            "the least a pane asks for"
        );
        assert_eq!(measured_height(None, 540.0), 540, "a pane at home");
    }

    /// The maximized pane is measured by the whole body, the grid's one
    /// cell while it lasts, and so asks for more than its cell among four
    /// did: being given the page is a re-pick upward.
    #[test]
    fn a_maximized_pane_is_measured_by_the_whole_body() {
        let body = layout::Body::of(gpui::size(gpui::px(1600.0), gpui::px(900.0)), 0.0, 0.0);
        let whole = layout::Grid::of(body, 1).share_height;
        let cell = layout::Grid::of(body, 4).share_height;
        assert_eq!(whole, body.height);
        assert!(measured_height(None, whole) > measured_height(None, cell));
    }

    /// Only a picture that covers its pane is kept through a rendition
    /// change; a pane still starting or fading in starts cold, whatever
    /// asked.
    #[test]
    fn only_a_covered_picture_restarts_beside() {
        for reason in [Restart::RePick, pick("480p30")] {
            assert_eq!(
                restart_how(true, reason.clone()),
                How::Beside(reason.clone())
            );
            assert_eq!(restart_how(false, reason), How::Cold);
        }
    }

    /// A playing recording's new player opens ahead of the pane, to be
    /// reached; a paused one's opens where the pane is waiting.
    #[test]
    fn a_recording_swap_starts_ahead_unless_paused() {
        assert_eq!(swap_start_at(600.0, false, 1.0), 600.0 + lead(1.0));
        assert_eq!(swap_start_at(600.0, true, 1.0), 600.0);
        assert_eq!(
            swap_start_at(600.0, false, 2.0),
            600.0 + 2.0 * lead(1.0),
            "twice as far into a recording playing twice as fast"
        );
        assert_eq!(swap_start_at(600.0, true, 2.0), 600.0);
    }

    /// A re-pick that wants no sharper a rendition than the one resolving
    /// leaves it be, rather than starting the same thing again.
    #[test]
    fn a_pending_for_the_same_height_is_not_started_twice() {
        assert!(!wants_swap(Some(1080), 1080));
        assert!(!wants_swap(Some(1080), 720));
        assert!(wants_swap(None, 720), "nothing resolving");
    }

    /// One that wants sharper supersedes it.
    #[test]
    fn a_taller_want_supersedes_a_pending() {
        assert!(wants_swap(Some(720), 1080));
    }

    /// A re-pick that resolves to what already plays is dropped; a pick from
    /// the menu is carried out even then.
    #[test]
    fn a_same_rendition_re_pick_is_dropped_but_a_pick_is_not() {
        assert!(!worth_swapping(false, "720p60", "720p60"));
        assert!(worth_swapping(false, "720p60", "1080p60"));
        assert!(worth_swapping(true, "720p60", "720p60"));
    }

    /// What a pending start tells the pane's player: what a pick asked for,
    /// and nothing for a re-pick, which is silent.
    #[test]
    fn only_a_pick_says_what_it_is_switching_to() {
        assert!(pick("480p30").is_pick());
        assert!(!Restart::RePick.is_pick());
        let Restart::Pick(switching) = pick("480p30") else {
            unreachable!("pick makes a pick");
        };
        assert_eq!(switching.to.as_ref(), "480p30");
    }

    /// A pick from the menu of `to`, from its rendition's row or, with
    /// `default`, the settings' row, made at `since`.
    fn switching(to: &str, default: bool, since: Instant) -> Switching {
        Switching {
            to: SharedString::from(to.to_string()),
            default,
            since,
        }
    }

    /// The same pick again asks nothing new; the same rendition from the
    /// other row only moves the mark; anything else, or a pick with nothing
    /// under way, starts afresh.
    #[test]
    fn a_pick_again_starts_only_what_is_new() {
        let now = Instant::now();
        let low = switching("480p30", false, now);
        assert_eq!(again(None, &low), Again::New);
        assert_eq!(again(Some(&low), &low), Again::Same);
        assert_eq!(
            again(Some(&low), &switching("480p30", true, now)),
            Again::OtherRow
        );
        assert_eq!(
            again(Some(&low), &switching("720p60", false, now)),
            Again::New
        );
    }

    /// A re-pick is silent, but one superseding a pick of the settings' row
    /// still under way stays a pick, of what the settings want now, counted
    /// from the press.
    #[test]
    fn a_re_pick_carries_on_a_pick_under_way() {
        assert_eq!(re_pick(None, "1080p60"), Restart::RePick);
        let pressed = Instant::now();
        let under_way = switching("720p60", true, pressed);
        assert_eq!(
            re_pick(Some(&under_way), "1080p60"),
            Restart::Pick(switching("1080p60", true, pressed))
        );
    }

    /// Where the statement holding byte `at` of stripped `code` starts: after
    /// the last `;`, `{` or `}` before it.
    fn statement_start(code: &str, at: usize) -> usize {
        code[..at].rfind([';', '{', '}']).map_or(0, |end| end + 1)
    }

    /// The braced block `code` opens with, up to and including its matching
    /// `}`, or all of `code` if it never closes.
    fn block(code: &str) -> &str {
        let mut depth = 0;
        for (at, c) in code.char_indices() {
            match c {
                '{' => depth += 1,
                '}' if depth == 1 => return &code[..=at],
                '}' => depth -= 1,
                _ => {}
            }
        }
        code
    }

    /// How many times stripped `code` writes a field called `pending`:
    /// assigns it, takes, replaces, inserts or borrows it mutably — by a
    /// method, by `&mut` or `ref mut` anywhere in the statement before it,
    /// by a `ref mut` in a match on it — or binds it in a pattern of a slot,
    /// which match ergonomics can make mutable without saying so. Another
    /// field whose name starts the same — `pending_event` — is not it.
    fn pending_writes(code: &str) -> usize {
        const MUTATORS: [&str; 8] = [
            ".take(",
            ".take_if(",
            ".replace(",
            ".insert(",
            ".get_or_insert",
            ".as_mut(",
            ".as_deref_mut(",
            ".iter_mut(",
        ];
        let fields = code
            .match_indices(".pending")
            .filter(|&(at, field)| {
                let after = &code[at + field.len()..];
                if after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                    return false;
                }
                let assigned = after.starts_with('=') && !after.starts_with("==");
                let mutated = MUTATORS.iter().any(|mutator| after.starts_with(mutator));
                // `&mut slot.pending`, `&mut self.slots.get_mut(i)?.pending`
                // or `if let Some(ref mut p) = slot.pending`, which read
                // `&mutslot.pending` and the like once the spaces are out.
                let statement = &code[statement_start(code, at)..at];
                let borrowed = statement.contains("&mut") || statement.contains("refmut");
                // `match slot.pending { Some(ref mut p) => .. }`.
                let matched = after.starts_with('{') && block(after).contains("refmut");
                assigned || mutated || borrowed || matched
            })
            .count();
        // `let Slot { pending, .. } = slot`, or a match arm's: a pattern,
        // since what follows a struct's braces is `=` only for one.
        let patterns = ["Slot{", "Self{"]
            .iter()
            .flat_map(|name| code.match_indices(name))
            .filter(|&(at, name)| {
                let open = at + name.len() - 1;
                let fields = block(&code[open..]);
                let after = &code[open + fields.len()..];
                let binds = fields.match_indices("pending").any(|(field, word)| {
                    let before = fields[..field].ends_with(['{', ',']);
                    let next = fields[field + word.len()..].chars().next();
                    before && matches!(next, Some(',' | '}' | ':'))
                });
                binds && after.starts_with('=') && !after.starts_with("==")
            })
            .count();
        fields + patterns
    }

    /// The counter above finds every way of writing the field it is meant
    /// to, and none of the reads.
    #[test]
    fn pending_writes_are_counted() {
        for write in [
            "slot.pending=None;",
            "self.slots[index].pending=Some(start);",
            "slot.pending.take()",
            "slot.pending.take_if(|p|p.generation==generation)",
            "std::mem::replace(&mutslot.pending,next)",
            "letp=&mutself.slots[*index].pending;",
            "letp=&mutself.slots.get_mut(index)?.pending;",
            "letp=&mutself.slots[index+1].pending;",
            "slot.pending.as_mut()",
            "slot.pending.insert(start)",
            "slot.pending.iter_mut()",
            "ifletSome(refmutp)=slot.pending{p.reason=Restart::RePick;}",
            "matchself.slots[i].pending{Some(refmutp)=>{}None=>{}}",
            "letSlot{pending,..}=&mutself.slots[index];",
            "letSelf{key,pending,..}=self;",
            "matchslot{Slot{pending:Some(p),..}=>{}_=>{}}",
        ] {
            assert_eq!(pending_writes(write), 1, "{write}");
        }
        for read in [
            "slot.pending.as_ref()",
            "slot.pending.is_none()",
            "slot.pending==None",
            "&slot.pending",
            "self.pending_event(index,event,cx)",
            "pending.generation",
            "matchslot.pending{Some(refp)=>{}None=>{}}",
            "Slot{key,pending:None,generation:0}",
            "letslot=Self{pending:None,..base};",
        ] {
            assert_eq!(pending_writes(read), 0, "{read}");
        }
    }

    /// Every write of a pane's pending start goes through `set_pending`, so
    /// none can leave the bar saying a pick is under way after its start has
    /// gone, or saying nothing while it resolves. Read from the root's own
    /// sources, every file there is, so a new one is held to it without
    /// being listed — as `panes`'s test holds the root to `set_slot_state`.
    /// And from `watch`, which owns `Slot`: a method of its own that wrote
    /// the field, such as one clearing it with the slot's state, would skip
    /// the player as surely. It writes none; `Slot::new` starts with nothing
    /// pending by naming the field, not writing one, and has no player to
    /// tell.
    #[test]
    fn every_change_of_a_pending_start_goes_through_set_pending() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sources: Vec<_> = std::fs::read_dir(src.join("root"))
            .expect("the root's sources are missing")
            .map(|entry| entry.expect("an unreadable source").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        sources.push(src.join("watch.rs"));
        let mut writes = 0;
        for path in sources {
            let source = std::fs::read_to_string(&path).expect("an unreadable source");
            let code = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests");
            // Spaces and line ends out, so formatting cannot hide a write.
            let code: String = code.split_whitespace().collect();
            let found = pending_writes(&code);
            let name = path.file_name().and_then(|name| name.to_str());
            if name == Some("renditions.rs") {
                writes += found;
            } else {
                assert_eq!(
                    found,
                    0,
                    "{} writes a pane's pending start itself; use set_pending",
                    path.display()
                );
            }
        }
        assert_eq!(writes, 1, "set_pending is the one write");
    }
}
