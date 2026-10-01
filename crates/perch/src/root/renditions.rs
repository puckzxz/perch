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
//! Named for renditions rather than for quality: `streamlink::quality` is
//! the module every question here is put to, and goes by that name.

use gpui::{Context, SharedString, Window};
use settings::QualityPreference;
use streamlink::{quality, StreamEvent};

use super::streams::{self, How, NO_PLAYLIST};
use super::RootView;
use crate::layout;
use crate::video::{StartOptions, VideoStream};
use crate::video_view::SWAP_LEAD;
use crate::watch::{Restart, Slot, StreamState};

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
            self.slots[index].pending = None;
            view.update(cx, |view, cx| {
                view.cancel_swap();
                view.set_picked(picked, cx);
            });
        } else {
            self.change_rendition(index, Restart::Pick, window, cx);
        }
        cx.notify();
    }

    /// How tall the pane `key` names is right now, in physical pixels: what
    /// its quality is chosen against, when its stream starts and again in
    /// [`sync_quality`](Self::sync_quality). With several panes the window is
    /// shared, so each one asks for proportionally less.
    ///
    /// Its cell of the watch grid, the one grid ([`grid`](Self::grid)), on
    /// whichever page is up: a mini-player tile is not one of the sizes a
    /// quality is chosen for. The cell's share of the body's height, seams
    /// included (`Grid::share_height`, which says why). Asked by the pane's
    /// key although every pane is one cell of that grid today — a pane in a
    /// window of its own included, since it keeps its cell — so that a pane
    /// measured by room of its own is asked the same question, and every
    /// caller already says which pane it means.
    pub(super) fn pane_height_for(&self, _key: &str, window: &Window) -> u32 {
        layout::quality_height(self.grid(window).share_height * window.scale_factor())
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
    /// about this pane, whatever its size. So, for now, is a pane in a window
    /// of its own, whose window is not the grid's cell this measures. And so
    /// is a pane already resolving a rendition at least as sharp
    /// ([`wants_swap`]), which every call while it resolves would otherwise
    /// start again.
    ///
    /// Each pane is measured on its own ([`pane_height_for`](Self::pane_height_for)),
    /// and its log line says the height it was measured at.
    pub(super) fn sync_quality(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restart: Vec<(usize, u32)> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.quality_override.is_none())
            .filter(|(_, slot)| !self.stage.is_popped(&slot.key))
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
                (wanted.height > playing && wants_swap(resolving, wanted.height))
                    .then_some((index, pane_height))
            })
            .collect();
        for (index, pane_height) in &restart {
            eprintln!(
                "video: {} re-choosing quality for a {pane_height}px pane",
                self.slots[*index].key
            );
            self.change_rendition(*index, Restart::RePick, window, cx);
        }
        if !restart.is_empty() {
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
        let (generation, reason) = (pending.generation, pending.reason);
        let (playing, position, paused, size) = {
            let view = view.read(cx);
            (
                view.quality().to_string(),
                view.position(),
                view.is_paused(),
                view.size_handle(),
            )
        };
        if !worth_swapping(reason, &playing, &quality) {
            eprintln!(
                "video: {} already plays {quality}; nothing to swap",
                slot.key
            );
            self.slots[index].pending = None;
            return;
        }
        let start_at = swap_start_at(position, paused);
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
        let Some(pending) = self.slots[index].pending.take() else {
            return;
        };
        if let Some(view) = self.slots[index].video() {
            view.update(cx, |view, _| view.cancel_swap());
        }
        eprintln!(
            "video: {} couldn't start a {:?} beside the picture: {why}",
            self.slots[index].key, pending.reason
        );
        if pending.reason == Restart::Pick {
            self.pick_failed(index, "couldn't switch quality".to_string(), cx);
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
        let slot = &mut self.slots[index];
        match slot.pending.take() {
            Some(pending) if pending.generation == generation => {
                slot.supervisor = Some(pending.supervisor);
                slot.pump = Some(pending.pump);
                slot.generation = generation;
            }
            other => {
                slot.pending = other;
                eprintln!(
                    "video: {owner} swapped to start {generation}, which it was not waiting for"
                );
            }
        }
        cx.notify();
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
        let slot = &mut self.slots[index];
        if slot
            .pending
            .as_ref()
            .is_none_or(|pending| pending.generation != generation)
        {
            return;
        }
        let reason = slot.pending.take().map(|pending| pending.reason);
        if reason == Some(Restart::Pick) {
            self.pick_failed(index, format!("couldn't switch to {quality}"), cx);
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

/// Where a recording's new player opens, given where the pane is: there, if
/// it is paused, and [`SWAP_LEAD`] seconds on if it is playing, so the new
/// player has a picture just before the pane reaches it.
fn swap_start_at(position: f64, paused: bool) -> f64 {
    if paused {
        position
    } else {
        position + SWAP_LEAD
    }
}

/// Whether a re-pick wanting a rendition `wanted` px tall is worth a start,
/// with one already resolving for a rendition `resolving` px tall, if any:
/// only a taller want supersedes it.
fn wants_swap(resolving: Option<u32>, wanted: u32) -> bool {
    resolving.is_none_or(|height| height < wanted)
}

/// Whether a start for `reason` that resolved to `resolved` is worth
/// swapping to, with `playing` on screen: a re-pick that names what already
/// plays is not, and a pick from the menu always is — somebody asked.
fn worth_swapping(reason: Restart, playing: &str, resolved: &str) -> bool {
    reason == Restart::Pick || playing != resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a picture that covers its pane is kept through a rendition
    /// change; a pane still starting or fading in starts cold, whatever
    /// asked.
    #[test]
    fn only_a_covered_picture_restarts_beside() {
        for reason in [Restart::RePick, Restart::Pick] {
            assert_eq!(restart_how(true, reason), How::Beside(reason));
            assert_eq!(restart_how(false, reason), How::Cold);
        }
    }

    /// A playing recording's new player opens ahead of the pane, to be
    /// reached; a paused one's opens where the pane is waiting.
    #[test]
    fn a_recording_swap_starts_ahead_unless_paused() {
        assert_eq!(swap_start_at(600.0, false), 600.0 + SWAP_LEAD);
        assert_eq!(swap_start_at(600.0, true), 600.0);
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
        assert!(!worth_swapping(Restart::RePick, "720p60", "720p60"));
        assert!(worth_swapping(Restart::RePick, "720p60", "1080p60"));
        assert!(worth_swapping(Restart::Pick, "720p60", "720p60"));
    }
}
