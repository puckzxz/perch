//! A live picture that fell behind gets back to the live edge by playing a
//! little faster, the way Twitch's own player and FrankerFaceZ's
//! (`player.allow-catchup`, on by default) do.
//!
//! mpv jumps to the edge only when a live pane is unpaused
//! (`Player::seek_to_live`). After the network stalls, streamlink fetches
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

#[cfg(test)]
mod tests {
    use super::*;

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
