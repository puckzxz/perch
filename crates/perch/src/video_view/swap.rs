//! A rendition swapped in place: a pane's new player started beside the one
//! on screen, inside the same view, which takes over once it has a picture
//! and, on a recording, has caught up with where the pane is.
//!
//! Inside the same view rather than in a new one, as a cold restart makes,
//! because everything the pane has on screen is the view's and so stays: the
//! picture faded in once and covers the pane (`first_frame`, `covers`), so no
//! poster comes back and nothing fades in again; the level and Mute all's
//! hold are the ones the user left (`loudness`), not a fresh `--volume`; and
//! whatever holds the view — a pane, a tile, a pop-out — goes on holding the
//! same entity. Only the stream underneath changes, at the moment
//! [`promote`](VideoView::promote) hands it over.
//!
//! Until then the new player is nobody's business. It is started silent, at
//! the pane's own size (the shared `SizeHandle`), and publishing no position
//! (`video::Positions`), so the seek bar, the chat replay and the history go
//! on following the player on screen; pause, volume and seeks act on that one
//! alone, and are never forwarded. Instead, on every wake of either stream
//! and on a tick of its own, the pending player is held to the one on screen
//! by [`align`], a pure function of where each is: a live stream takes over
//! on its first frame, since two sessions of a broadcast cannot be lined up
//! anyway; a recording is started ahead of the pane and held still once it
//! has a picture until the pane reaches it, or sent again when it falls
//! behind or the user seeks away from it.
//!
//! Each stream's wakes carry the generation the root numbered its start with
//! (`RootView::start_stream`), and [`route`] sends each to the stream it
//! belongs to, or nowhere once that stream is gone — the same rule the root
//! sends its streamlink events by. A swap that cannot finish leaves the
//! player on screen as it was, and says so (`VideoEvent::SwapFailed`).

use std::time::{Duration, Instant};

use futures::channel::mpsc::Receiver;
use gpui::{Context, Task};

use super::{Qualities, VideoEvent, VideoView};
use crate::video::{Stopped, VideoStream};

/// How far ahead of a playing recording its new player is started, in
/// seconds: about what opening the new player costs, mpv's open and its
/// first frame — not streamlink's resolve, which is over by the time the
/// pane's position is read (`RootView::pending_event`) — so the new player
/// has a picture just before the pane gets there, and waits a moment rather
/// than arriving late.
///
/// An estimate, to be tuned from the log. A swap's first `holding` line gives
/// where the new player was when it had a picture and where the pane was:
/// the first less the second is what was left of the lead. A first
/// `reseeking` line from behind the pane says the lead was not enough. Not the `swapped`
/// line's time, which runs from the new player's start to the hand-over and
/// so comes out at about this lead whenever the lead was enough, however
/// quickly the player opened. Too small, and swaps reseek; too large, and
/// two players run side by side for longer.
pub const SWAP_LEAD: f64 = 4.0;

/// How long a swap may take, from its player starting, before it is given up
/// and the pane keeps what it plays: a session Twitch never serves, or a
/// recording that cannot be lined up, is not worth two players for ever.
const SWAP_CEILING: Duration = Duration::from_secs(45);

/// How many times a recording's new player is sent somewhere else before it
/// takes over wherever it is.
const MAX_RESEEKS: u8 = 3;

/// The longest a reseek is waited for before its player's position is
/// believed again; see [`Settle`].
const SETTLE: Duration = Duration::from_secs(3);

/// How far past the place it was sent to a reseeked player has to have
/// played, and how far at most, to count as playing there; see [`Settle`].
const SETTLE_MOVED: f64 = 0.2;
const SETTLE_WITHIN: f64 = 1.0;

/// How often a pending player is held to the one on screen when neither
/// has woken it: a paused pane sends no frames, and a held player none
/// either.
const TICK: Duration = Duration::from_millis(100);

/// A pane's new player, getting ready beside the one on screen.
pub(super) struct Pending {
    stream: VideoStream,
    /// What the menu will say once it takes over.
    qualities: Qualities,
    /// The start of the pane's stream it belongs to; see [`route`].
    generation: u64,
    /// Whether it plays a recording, which [`align`] lines up, rather than a
    /// live stream, which it does not.
    recording: bool,
    /// Held still, waiting for the pane to reach it. Only ever after it has a
    /// picture: whether mpv draws a first frame for a file it opens paused is
    /// not known, so a player is never asked to.
    held: bool,
    /// How many times it has been sent somewhere else; see [`MAX_RESEEKS`].
    /// Started over by a seek of the user's (`VideoView::retarget_swap`).
    attempts: u8,
    /// The reseek it was last sent on, while that may still be landing.
    settle: Option<Settle>,
    /// When it was started, for [`SWAP_CEILING`] and the log line.
    since: Instant,
    /// Its frame pump, which becomes the pane's when it takes over.
    pump: Task<()>,
    /// Holds it to the pane between wakes; dropped with it.
    _tick: Task<()>,
}

/// A reseek that may not have landed yet: a recording is moved by reopening
/// it (`vod`), which takes about a second. Its position says where it was
/// sent as soon as its render thread takes the seek, between frames
/// (`VideoStream::seek_to`), while its picture is still the one from before;
/// until then it still says where the player was. So it counts as landed
/// only once it has played on from there — a little way past the target, and
/// not yet far — or once [`SETTLE`] has passed.
#[derive(Clone, Copy, Debug)]
struct Settle {
    until: Instant,
    target: f64,
}

impl Settle {
    /// Whether the reseek is still being waited for, with its player at
    /// `position` at `now`. A position further off than [`SETTLE_WITHIN`] is
    /// where the player was before the seek was applied: every reseek sends
    /// a player at least that far.
    fn holds(&self, now: Instant, position: f64) -> bool {
        let played = position - self.target;
        now < self.until && !(SETTLE_MOVED..=SETTLE_WITHIN).contains(&played)
    }
}

/// Where the two players are, for [`align`].
#[derive(Clone, Copy, Debug)]
struct AlignInput {
    /// Whether this is a recording; a live stream is never lined up.
    recording: bool,
    /// Whether the new player has a picture yet.
    has_frame: bool,
    /// Whether a reseek it was sent on is still landing; see [`Settle`].
    settling: bool,
    /// The player on screen: where it is, and whether it is paused.
    old_pos: f64,
    old_paused: bool,
    /// Where the new one is.
    new_pos: f64,
    /// How many times the new one has been sent somewhere else.
    attempts: u8,
}

/// What to do with the new player now.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Align {
    /// Nothing yet: it has no picture, or a reseek is landing.
    Wait,
    /// Hold it still: it is ahead of the pane, which will reach it.
    Hold,
    /// Send it to this many seconds in, and let it play.
    Reseek(f64),
    /// Hand the pane over to it.
    Promote,
    /// Hand it over anyway: it has been sent somewhere else as often as it
    /// may be, and is still not where the pane is. Logged, so a swap that
    /// jumped says so.
    Unaligned,
}

/// When a pane's new player takes over, and what to do with it until then.
///
/// - Without a picture, or with a reseek landing, nothing.
/// - Live, at once: two sessions of a broadcast sit at different distances
///   from its edge, so there is nothing to line up.
/// - A paused pane takes over when the new player is no more than half a
///   second behind or a second and a half ahead — a paused picture that moves
///   that little reads as the same moment — and otherwise sends it to the
///   pane's place.
/// - A playing pane takes over when the new player is at most a tenth of a
///   second ahead and no more than half a second behind. Further behind, it
///   is sent further ahead than last time. Ahead by about what it was started
///   ahead by, it is held until the pane reaches it; ahead by more than that,
///   the pane must have jumped back — a seek of the user's — and it is sent
///   just ahead of the pane again.
///
/// Every send counts, and after [`MAX_RESEEKS`] the new player takes over
/// where it is.
fn align(i: AlignInput) -> Align {
    if !i.has_frame || i.settling {
        return Align::Wait;
    }
    if !i.recording {
        return Align::Promote;
    }
    let reseek = |target: f64| {
        if i.attempts < MAX_RESEEKS {
            Align::Reseek(target)
        } else {
            Align::Unaligned
        }
    };
    let ahead = i.new_pos - i.old_pos;
    if i.old_paused {
        return if (-0.5..=1.5).contains(&ahead) {
            Align::Promote
        } else {
            reseek(i.old_pos)
        };
    }
    if ahead < -0.5 {
        return reseek(i.old_pos + SWAP_LEAD * f64::from(i.attempts + 1));
    }
    if ahead <= 0.1 {
        return Align::Promote;
    }
    // The furthest ahead a send of this swap's own may have put it: the lead
    // grows with each send after falling behind, and a player sent there is
    // waited for rather than sent back.
    let furthest = SWAP_LEAD * f64::from(i.attempts.max(1)) + 2.0;
    if ahead > furthest {
        reseek(i.old_pos + SWAP_LEAD)
    } else {
        Align::Hold
    }
}

/// Which stream of a pane a wake, or a streamlink event, belongs to: the one
/// on screen, the one getting ready beside it, or neither — a stream the
/// pane has moved on from, whose last wakes can still be on their way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    Current,
    Pending,
    Stale,
}

/// Send a wake tagged `generation` to its stream: `current` is the
/// generation of the stream on screen, `pending` that of the one beside it,
/// if any. The one rule for the player's frame wakes and for the root's
/// streamlink events (`RootView::apply_stream_event`).
pub fn route(generation: u64, current: u64, pending: Option<u64>) -> Wake {
    if generation == current {
        Wake::Current
    } else if pending == Some(generation) {
        Wake::Pending
    } else {
        Wake::Stale
    }
}

impl VideoView {
    /// Wake the player for every frame of the stream `generation` names,
    /// and once more when that stream's channel closes. Bound to no window,
    /// so the player can move between windows (`set_place`): a task bound to
    /// the main one would stop waking it the moment the main window went,
    /// with a pop-out still drawing it.
    pub(super) fn pump(
        mut frames: Receiver<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while frames.next().await.is_some() {
                if this
                    .update(cx, |this, cx| this.woken(generation, cx))
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| this.closed(generation, cx));
        })
    }

    /// The generation of the player getting ready, if there is one.
    fn pending_generation(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.generation)
    }

    /// A wake from the stream `generation` names. The one on screen's
    /// carries both a new frame to draw and — once — the news that there
    /// will not be another (`VideoStream::stopped`); it also holds a pending
    /// player to it, as often as it has frames. The pending one's draws
    /// nothing until it takes over, so it repaints nothing.
    fn woken(&mut self, generation: u64, cx: &mut Context<Self>) {
        match route(generation, self.generation, self.pending_generation()) {
            Wake::Current => {
                if let Some(reason) = self.stream.take_stopped() {
                    cx.emit(VideoEvent::Stopped(reason));
                } else {
                    self.pending_tick(cx);
                }
                cx.notify();
            }
            Wake::Pending => self.pending_tick(cx),
            Wake::Stale => {}
        }
    }

    /// The channel of the stream `generation` names closed: its thread has
    /// ended. For the player on screen that was said on its last wake; a
    /// pending one cannot take over any more.
    fn closed(&mut self, generation: u64, cx: &mut Context<Self>) {
        if route(generation, self.generation, self.pending_generation()) == Wake::Pending {
            self.fail_swap("its player closed", cx);
        }
    }

    /// Start `stream`, a new player of this pane at the rendition
    /// `qualities` names, beside the one on screen, to take over once it is
    /// ready; see the module. The root's start `generation`, which its
    /// wakes carry. Whatever was getting ready before is dropped.
    ///
    /// The stream must have been started silent, publishing nothing, and
    /// with this player's size handle (`size_handle`); `RootView` starts it so.
    pub fn begin_swap(
        &mut self,
        stream: VideoStream,
        frames: Receiver<()>,
        qualities: Qualities,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        self.cancel_swap();
        let pump = Self::pump(frames, generation, cx);
        let tick = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            let ticked = this.update(cx, |this, cx| {
                if route(generation, this.generation, this.pending_generation()) == Wake::Pending {
                    this.pending_tick(cx);
                }
            });
            if ticked.is_err() {
                return;
            }
        });
        eprintln!(
            "video: {} starting {} beside {}",
            self.key, qualities.playing, self.qualities.playing
        );
        self.pending = Some(Pending {
            recording: stream.timeline().is_some(),
            stream,
            qualities,
            generation,
            held: false,
            attempts: 0,
            settle: None,
            since: Instant::now(),
            pump,
            _tick: tick,
        });
    }

    /// Drop the player getting ready, if there is one, and say nothing: for
    /// the root, which has moved on from it — a new start superseding it, a
    /// cold restart, or its stream failing to resolve.
    pub fn cancel_swap(&mut self) {
        if let Some(pending) = self.pending.take() {
            eprintln!(
                "video: {} called off the swap to {}",
                self.key, pending.qualities.playing
            );
        }
    }

    /// A seek of the user's: a pending player is lined up with the pane's new
    /// place from the start, with all its reseeks, rather than given up on.
    pub(super) fn retarget_swap(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.attempts = 0;
        }
    }

    /// Hold the pending player to the one on screen: give up on it if it
    /// stopped or has taken too long, and otherwise do what [`align`] says.
    fn pending_tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if let Some(reason) = pending.stream.take_stopped() {
            let why = match reason {
                Stopped::Ended => "it ended".to_string(),
                Stopped::Failed(message) => message,
            };
            return self.fail_swap(&why, cx);
        }
        if now.duration_since(pending.since) >= SWAP_CEILING {
            return self.fail_swap("it was not ready in time", cx);
        }
        let new_pos = pending.stream.position();
        let input = AlignInput {
            recording: pending.recording,
            has_frame: pending.stream.latest_frame().is_some(),
            settling: pending
                .settle
                .is_some_and(|settle| settle.holds(now, new_pos)),
            old_pos: self.stream.position(),
            old_paused: self.stream.is_paused(),
            new_pos,
            attempts: pending.attempts,
        };
        match align(input) {
            Align::Wait => {}
            Align::Hold => {
                if !pending.held {
                    pending.stream.set_paused(true);
                    pending.held = true;
                    eprintln!(
                        "video: {} holding {} at {new_pos:.2} for the pane at {:.2}",
                        self.key, pending.qualities.playing, input.old_pos
                    );
                }
            }
            Align::Reseek(target) => {
                pending.stream.set_paused(false);
                pending.held = false;
                pending.stream.seek_to(target);
                pending.settle = Some(Settle {
                    until: now + SETTLE,
                    target,
                });
                pending.attempts += 1;
                eprintln!(
                    "video: {} reseeking {} from {new_pos:.2} to {target:.2} for the pane at {:.2} (try {})",
                    self.key, pending.qualities.playing, input.old_pos, pending.attempts
                );
            }
            Align::Promote => self.promote(cx),
            Align::Unaligned => {
                eprintln!(
                    "video: {} unaligned after {MAX_RESEEKS} reseeks; taking over anyway",
                    self.key
                );
                self.promote(cx);
            }
        }
    }

    /// Hand the pane over to the pending player: it plays or pauses as the
    /// pane does, at the level the pane is heard at, takes a seek the old
    /// player had not got to yet, and its position becomes the pane's; then
    /// the old stream goes — its thread stops within
    /// a frame, or 200 ms if it has none (`VideoStream`'s drop) — and its
    /// frame pump with it. `first_frame` is left alone: the picture already
    /// covers the pane, and the next frame drawn is simply the new player's
    /// (`render` frees the old one's tile when the id changes).
    ///
    /// Says so to the root (`VideoEvent::Swapped`), which drops the old
    /// stream's streamlink only now, after its player has been stopped: a
    /// relay killed under a live player ends its stream, and the pane with it.
    fn promote(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Pending {
            stream,
            qualities,
            generation,
            since,
            pump,
            ..
        } = pending;
        stream.set_paused(self.stream.is_paused());
        stream.set_volume(self.loudness.audible());
        self.stream.set_volume(0);
        // A seek of the user's that the old player has not taken yet goes
        // over too, or it is lost with the old player, which stops before it
        // reaches it. Its position does not say that seek yet, so `align`
        // decided on where the pane was before it; the new player takes over
        // there and then goes where the user asked, as the old one would have.
        let carried = self.stream.take_seek();
        if let Some(secs) = carried {
            stream.seek_to(secs);
        }
        stream.publish_position();
        let (from_pos, to_pos) = (self.stream.position(), stream.position());
        let old = std::mem::replace(&mut self.stream, stream);
        let from = std::mem::replace(&mut self.qualities, qualities).playing;
        self.generation = generation;
        self._pump = pump;
        drop(old);
        let then = carried
            .map(|secs| format!(", then {secs:.2}"))
            .unwrap_or_default();
        eprintln!(
            "video: {} swapped {from}->{} after {} ms; position {from_pos:.2}->{to_pos:.2}{then}",
            self.key,
            self.qualities.playing,
            since.elapsed().as_millis()
        );
        cx.emit(VideoEvent::Swapped { generation });
        cx.notify();
    }

    /// Give up on the pending player, leaving the one on screen as it is,
    /// and say so to the root (`VideoEvent::SwapFailed`).
    fn fail_swap(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        eprintln!(
            "video: {} couldn't swap to {}: {why}",
            self.key, pending.qualities.playing
        );
        cx.emit(VideoEvent::SwapFailed {
            generation: pending.generation,
            quality: pending.qualities.playing.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording ten minutes in, playing, with its new player started
    /// four seconds ahead and showing a picture.
    fn recording() -> AlignInput {
        AlignInput {
            recording: true,
            has_frame: true,
            settling: false,
            old_pos: 600.0,
            old_paused: false,
            new_pos: 604.0,
            attempts: 0,
        }
    }

    /// Wherever the two live players are, the new one takes over on its
    /// first frame: there is nothing to line up.
    #[test]
    fn a_live_swap_promotes_on_its_first_frame() {
        for (old, new) in [(0.0, 0.0), (0.0, 30.0), (12.0, 3.0)] {
            let live = AlignInput {
                recording: false,
                old_pos: old,
                new_pos: new,
                ..recording()
            };
            assert_eq!(align(live), Align::Promote);
            assert_eq!(
                align(AlignInput {
                    old_paused: true,
                    ..live
                }),
                Align::Promote
            );
        }
    }

    /// Without a picture there is nothing to hand over to, live or not,
    /// however well the positions line up.
    #[test]
    fn nothing_promotes_before_a_first_frame() {
        for is_recording in [true, false] {
            let input = AlignInput {
                recording: is_recording,
                has_frame: false,
                new_pos: 600.0,
                ..recording()
            };
            assert_eq!(align(input), Align::Wait);
        }
    }

    /// A paused pane takes over a new player that is near its place: a
    /// paused picture that moves that little reads as the same moment.
    #[test]
    fn a_paused_recording_promotes_near_its_place() {
        for new in [600.0, 600.4, 601.5, 599.6] {
            let input = AlignInput {
                old_paused: true,
                new_pos: new,
                ..recording()
            };
            assert_eq!(align(input), Align::Promote, "at {new}");
        }
    }

    /// One started ahead before the pane was paused, or one behind it, is
    /// sent to the pane's place rather than handed a frame from elsewhere.
    #[test]
    fn a_paused_recording_far_ahead_reseeks_to_the_old_place() {
        for new in [604.0, 601.6, 598.0] {
            let input = AlignInput {
                old_paused: true,
                new_pos: new,
                ..recording()
            };
            assert_eq!(align(input), Align::Reseek(600.0), "at {new}");
        }
    }

    /// Ahead of a playing pane by about its lead, the new player is held
    /// still until the pane reaches it, and then takes over.
    #[test]
    fn a_playing_recording_ahead_is_held_until_the_old_player_reaches_it() {
        for new in [604.0, 602.0, 600.2, 605.9] {
            let input = AlignInput {
                new_pos: new,
                ..recording()
            };
            assert_eq!(align(input), Align::Hold, "at {new}");
        }
        for new in [600.05, 600.0, 599.6] {
            let input = AlignInput {
                new_pos: new,
                ..recording()
            };
            assert_eq!(align(input), Align::Promote, "at {new}");
        }
    }

    /// A new player the pane has overtaken — it took longer to open than its
    /// lead — is sent ahead again, further each time.
    #[test]
    fn a_recording_that_fell_behind_reseeks_further_ahead() {
        let behind = AlignInput {
            new_pos: 598.0,
            ..recording()
        };
        assert_eq!(align(behind), Align::Reseek(604.0));
        assert_eq!(
            align(AlignInput {
                attempts: 1,
                ..behind
            }),
            Align::Reseek(608.0)
        );
    }

    /// Far further ahead than it was ever sent, the new player is not where
    /// the pane is going: the pane jumped back. It is sent just ahead of the
    /// pane's new place rather than held for a minute.
    #[test]
    fn a_user_seek_back_reseeks_instead_of_holding() {
        let input = AlignInput {
            old_pos: 540.0,
            ..recording()
        };
        assert_eq!(align(input), Align::Reseek(544.0));
    }

    /// A player sent further ahead after falling behind is waited for where
    /// it was sent, not sent back to the shorter lead.
    #[test]
    fn a_grown_lead_is_held_rather_than_undone() {
        let input = AlignInput {
            new_pos: 607.0,
            attempts: 2,
            ..recording()
        };
        assert_eq!(align(input), Align::Hold);
    }

    /// After three sends the new player takes over wherever it is, and says
    /// it was not lined up.
    #[test]
    fn reseeks_give_up_after_three_and_promote() {
        let cases = [
            AlignInput {
                new_pos: 598.0,
                ..recording()
            },
            AlignInput {
                old_pos: 500.0,
                ..recording()
            },
            AlignInput {
                old_paused: true,
                ..recording()
            },
        ];
        for input in cases {
            let gave_up = AlignInput {
                attempts: MAX_RESEEKS,
                ..input
            };
            assert!(matches!(align(input), Align::Reseek(_)), "{input:?}");
            assert_eq!(align(gave_up), Align::Unaligned, "{gave_up:?}");
        }
    }

    /// While a reseek is landing nothing is decided on its position, which
    /// says where it was sent before the picture is there.
    #[test]
    fn a_settling_reseek_waits() {
        let input = AlignInput {
            settling: true,
            new_pos: 598.0,
            ..recording()
        };
        assert_eq!(align(input), Align::Wait);

        let now = Instant::now();
        let settle = Settle {
            until: now + SETTLE,
            target: 604.0,
        };
        assert!(settle.holds(now, 598.0), "the seek is not applied yet");
        assert!(settle.holds(now, 604.0), "sent there, not playing there");
        assert!(
            !settle.holds(now, 604.3),
            "playing on from where it was sent"
        );
        assert!(settle.holds(now, 607.0), "still where it was before");
        assert!(!settle.holds(now + SETTLE, 604.0), "waited long enough");
    }

    /// A wake goes to the stream it came from, and nowhere once the pane has
    /// moved on from that stream: a promoted swap's old pump, or a swap
    /// called off, can still have wakes on their way.
    #[test]
    fn wakes_from_a_superseded_stream_are_ignored() {
        assert_eq!(route(5, 5, Some(6)), Wake::Current);
        assert_eq!(route(6, 5, Some(6)), Wake::Pending);
        assert_eq!(route(3, 5, Some(6)), Wake::Stale);
        assert_eq!(route(6, 5, None), Wake::Stale);
        assert_eq!(route(5, 5, None), Wake::Current);
    }
}
