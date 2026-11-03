//! A live picture that fell behind gets back to the live edge by playing a
//! little faster, the way Twitch's own player and FrankerFaceZ's
//! (`player.allow-catchup`, on by default) do.
//!
//! mpv jumps to the edge only when a live pane is unpaused, as [`resume`]
//! decides. After the network stalls, streamlink fetches
//! the segments it missed and hands them over at once, so mpv resumes with
//! that backlog in its cache and plays it at normal speed, behind for good.
//! Measured on 14 October 2026 against a live channel with streamlink
//! suspended for eight seconds: the cache sat at about 1.6 s before the
//! stall and at 7.8 s for as long as anyone watched after it, the picture
//! 6.2 s further from live than it had been, and chat, which arrives as it
//! is said, that much ahead of it.
//!
//! So while a live pane holds more than [`ABOVE`] seconds it plays at
//! [`FACTOR`] until it is down to [`UNTIL`]: six seconds behind is two
//! minutes of five-percent-fast play, which mpv's pitch correction keeps
//! from being heard as higher. Measured the same way with this in place:
//! 6.3 s behind after the stall, a quarter of a second less every five
//! seconds, and back within half a second of where it had been after two
//! minutes, holding there. Not a jump to the edge, which would throw
//! away what was said in those seconds. At the edge a low-latency stream
//! keeps about a second and a half, so the threshold never trips there; a
//! stream without low latency arrives a two-second segment at a time and
//! may cross it for a moment, which costs a moment of slightly faster play.
//! Pure, so the rule is tested; the render thread observes
//! `demuxer-cache-duration` and applies what this says (`video.rs`).

/// How much a live pane may hold before it catches up, in seconds.
pub const ABOVE: f64 = 3.0;

/// How far down it catches up before playing at normal speed again: just
/// over the 1.5 to 1.6 s a low-latency stream holds at the edge, so it stops
/// about a fifth of a second from where it was, with that much still in hand.
pub const UNTIL: f64 = 1.75;

/// The speed it catches up at.
pub const FACTOR: f64 = 1.05;

/// Whether a live pane is catching up, as each reading of its cache moves it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CatchUp {
    catching: bool,
}

impl CatchUp {
    /// The speed to play at, given how many seconds the cache holds (`None`
    /// before mpv has said) and whether the pane is paused, which is not
    /// falling behind: unpausing a live pane jumps to the edge anyway.
    ///
    /// The reading the pause left is not acted on. The cache goes on
    /// filling while paused, so it says the pane is as far behind as it was
    /// paused for, which the jump undoes; the render thread forgets it as
    /// it asks for the jump, and this is handed `None`, at normal speed,
    /// until another reading comes. One mpv took in the moment before its
    /// core took the jump can still come after that, and is acted on until
    /// the next: the render thread forgets readings again when mpv says
    /// playback restarted after the jump, which ends such a catch-up early,
    /// but mpv hands a client its queued events before any change of a
    /// property (client.c's `mpv_wait_event`), so a reading it took before
    /// the restart can still follow the word of it. Rare, and a moment at
    /// [`FACTOR`] at worst.
    pub fn speed(&mut self, buffered: Option<f64>, paused: bool) -> f64 {
        self.catching = match (buffered, paused) {
            (_, true) | (None, _) => false,
            (Some(secs), false) if secs > ABOVE => true,
            (Some(secs), false) if secs < UNTIL => false,
            _ => self.catching,
        };
        if self.catching {
            FACTOR
        } else {
            1.0
        }
    }
}

/// How an unpaused live pane gets back to the edge; see [`resume`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resume {
    /// Nothing to do: the cache holds no more than [`ABOVE`], which a
    /// catch-up leaves alone too, so a jump would only cost a restart.
    Stay,
    /// Seek within the cache to this time: its end, less [`UNTIL`], which
    /// leaves as much in hand as a catch-up stops at (`Player::seek_to_live`).
    Seek(f64),
    /// Open the stream again, which streamlink's relay answers from the live
    /// edge, with a fresh resolve of the channel (`Player::reopen`).
    Reopen,
}

/// How a live pane unpaused now gets back to the edge, given how much its
/// cache holds (`demuxer-cache-duration`), where the cache ends
/// (`demuxer-cache-time`; either `None` before mpv has said) and whether
/// it filled while the pane was paused (`demuxer-cache-idle`).
///
/// While paused, mpv goes on reading the relay into its cache, so the
/// cache's end stays at the live edge, and a seek there is instant: 0.5 s
/// behind live after a twenty-second pause, against 20.0 s with no jump.
/// Until the cache is full — 150 MiB, a little over two minutes of 1440p —
/// when mpv stops reading, and its end falls behind with every second the
/// pane stays paused: 14.3 s behind, the same way, with the cache held to
/// fill in seven. Only opening the stream again gets back from there: 0.2 s
/// behind live, two and a half seconds after the press, holding the paused
/// frame meanwhile (all measured 2 November 2026; see HANDOFF, "A live
/// pane's jump to the edge").
pub fn resume(buffered: Option<f64>, cache_end: Option<f64>, filled_while_paused: bool) -> Resume {
    if filled_while_paused {
        return Resume::Reopen;
    }
    match (buffered, cache_end) {
        (Some(secs), _) if secs <= ABOVE => Resume::Stay,
        (_, Some(end)) => Resume::Seek(end - UNTIL),
        _ => Resume::Reopen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pause short enough to leave no more in the cache than a pane at
    /// the edge holds jumps nowhere; a longer one jumps to the cache's end,
    /// keeping what a catch-up stops at in hand.
    #[test]
    fn an_unpause_jumps_to_the_end_of_the_cache() {
        assert_eq!(resume(Some(1.6), Some(100.0), false), Resume::Stay);
        assert_eq!(resume(Some(ABOVE), Some(100.0), false), Resume::Stay);
        assert_eq!(
            resume(Some(21.1), Some(100.0), false),
            Resume::Seek(100.0 - UNTIL)
        );
        assert_eq!(
            resume(None, Some(100.0), false),
            Resume::Seek(100.0 - UNTIL)
        );
    }

    /// A cache that filled while paused stopped reading, so its end is not
    /// the edge, however much it holds; and with no reading of where it
    /// ends there is nowhere to seek to. Both open the stream again.
    #[test]
    fn an_unpause_opens_the_stream_again_when_the_cache_cannot_reach_the_edge() {
        assert_eq!(resume(Some(1.6), Some(100.0), true), Resume::Reopen);
        assert_eq!(resume(Some(140.0), Some(100.0), true), Resume::Reopen);
        assert_eq!(resume(Some(21.1), None, false), Resume::Reopen);
        assert_eq!(resume(None, None, false), Resume::Reopen);
    }

    #[test]
    fn at_the_edge_it_plays_at_normal_speed() {
        let mut catch_up = CatchUp::default();
        assert_eq!(catch_up.speed(None, false), 1.0);
        assert_eq!(catch_up.speed(Some(1.6), false), 1.0);
        assert_eq!(catch_up.speed(Some(2.9), false), 1.0);
    }

    /// Behind, it plays faster until it is back near the edge, and not in
    /// between: the gap between the two thresholds is what stops it
    /// flickering on a cache that wobbles around one of them.
    #[test]
    fn behind_it_catches_up_until_it_is_near_the_edge() {
        let mut catch_up = CatchUp::default();
        assert_eq!(catch_up.speed(Some(7.8), false), FACTOR);
        assert_eq!(catch_up.speed(Some(2.5), false), FACTOR);
        assert_eq!(catch_up.speed(Some(1.8), false), FACTOR);
        assert_eq!(catch_up.speed(Some(1.7), false), 1.0);
        assert_eq!(catch_up.speed(Some(2.5), false), 1.0);
    }

    /// A pause is not falling behind, and stops a catch-up under way.
    #[test]
    fn a_pause_plays_at_normal_speed() {
        let mut catch_up = CatchUp::default();
        assert_eq!(catch_up.speed(Some(7.8), false), FACTOR);
        assert_eq!(catch_up.speed(Some(7.8), true), 1.0);
        assert_eq!(catch_up.speed(Some(2.5), false), 1.0);
    }

    /// The unpause as the render thread hands it over: thirty seconds held
    /// while paused, then no reading at all once it has asked for the jump
    /// to the edge, then the reading from after it. Nothing on the way
    /// catches up.
    #[test]
    fn an_unpause_waits_for_a_reading_from_after_the_jump() {
        let mut catch_up = CatchUp::default();
        assert_eq!(catch_up.speed(Some(30.0), true), 1.0);
        assert_eq!(catch_up.speed(None, false), 1.0);
        assert_eq!(catch_up.speed(Some(0.4), false), 1.0);
        assert_eq!(catch_up.speed(Some(1.6), false), 1.0);
    }
}
