//! A pointer at rest: where it was, and when it last moved.
//!
//! "The pointer is on it" holds things up all over the watch page: the
//! player's bar and the header over a picture, and a chat's new rows. That is
//! right while the pointer is reaching for something and wrong once it has
//! simply been left there — after a double-click into fullscreen it sits on
//! the picture, and after a press in chat it sits on chat — so the bar stayed
//! over the picture, and chat stayed on `Chat paused`, until somebody moved
//! it off. A pointer that has not moved for a while asks for nothing: what it
//! was holding up is let go, and the first move takes hold of it again.
//!
//! [`Rest`] is the one reading of that, kept by whoever measures the pointer
//! already — a player's probe and a chat's render, each frame — and asked
//! with the wait that suits what it holds ([`theme::POINTER_REST`] for a
//! picture's chrome, [`theme::CHAT_REST`] for chat's rows). It is measured,
//! like every hover in this app, rather than reported: a draw notes where the
//! pointer is, so nothing here needs a move event to hear of one.
//!
//! [`theme::POINTER_REST`]: crate::theme::POINTER_REST
//! [`theme::CHAT_REST`]: crate::theme::CHAT_REST

use std::time::{Duration, Instant};

use gpui::{Pixels, Point};

/// How far the pointer may stray from where it came to rest and still count
/// as resting, in logical pixels either way. A mouse on a desk is not
/// perfectly still — a sensor on a glossy surface reports a pixel now and
/// then — and without this such a mouse would never rest at all. Measured
/// from where it stopped, not from the last frame, so a pointer moved slowly
/// on purpose is still seen to move once it has gone this far.
const SLACK: f32 = 2.0;

/// Where the pointer came to a stop, and when; see the module.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rest {
    /// `None` while the pointer is somewhere else, so that wherever it comes
    /// back to is a move.
    stopped: Option<(Point<Pixels>, Instant)>,
}

impl Rest {
    /// The pointer is at `pointer` now. Returns whether that is a move: its
    /// first time here, or further than [`SLACK`] from where it stopped.
    pub fn note(&mut self, pointer: Point<Pixels>, now: Instant) -> bool {
        let moved = self.stopped.is_none_or(|(at, _)| {
            f32::from((pointer.x - at.x).abs()) > SLACK
                || f32::from((pointer.y - at.y).abs()) > SLACK
        });
        if moved {
            self.stopped = Some((pointer, now));
        }
        moved
    }

    /// The pointer has gone somewhere else, so it is not resting here, and
    /// its way back is a move even to the pixel it left from.
    pub fn forget(&mut self) {
        self.stopped = None;
    }

    /// Whether the pointer has been where it is for `after` or longer. Never
    /// while it is somewhere else: nothing is resting where nothing is.
    pub fn resting(&self, now: Instant, after: Duration) -> bool {
        self.stopped
            .is_some_and(|(_, since)| now.saturating_duration_since(since) >= after)
    }

    /// How long until the pointer has rested for `after`, for whoever has to
    /// wake to see it: nothing while it is somewhere else, and zero once it
    /// has.
    pub fn until(&self, now: Instant, after: Duration) -> Option<Duration> {
        self.stopped
            .map(|(_, since)| after.saturating_sub(now.saturating_duration_since(since)))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::*;

    const WAIT: Duration = Duration::from_secs(3);

    fn at(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    /// A pointer nobody has seen here is not resting here, however long it
    /// has been since.
    #[test]
    fn nothing_rests_where_the_pointer_is_not() {
        let rest = Rest::default();
        let now = Instant::now();
        assert!(!rest.resting(now, WAIT));
        assert!(!rest.resting(now + WAIT * 10, Duration::ZERO));
        assert_eq!(rest.until(now, WAIT), None);
    }

    /// It rests once it has been still for the wait, and not a moment
    /// before.
    #[test]
    fn a_still_pointer_rests_after_the_wait() {
        let mut rest = Rest::default();
        let start = Instant::now();
        assert!(rest.note(at(100.0, 100.0), start), "arriving is a move");
        assert!(!rest.resting(start, WAIT));
        let nearly = start + WAIT - Duration::from_millis(1);
        assert!(!rest.note(at(100.0, 100.0), nearly));
        assert!(!rest.resting(nearly, WAIT));
        assert!(rest.resting(start + WAIT, WAIT));
        assert!(rest.resting(start + WAIT * 20, WAIT), "and stays resting");
    }

    /// A move starts the wait again, from where the pointer is now.
    #[test]
    fn a_move_starts_the_wait_again() {
        let mut rest = Rest::default();
        let start = Instant::now();
        rest.note(at(100.0, 100.0), start);
        let later = start + WAIT * 2;
        assert!(rest.resting(later, WAIT));
        assert!(rest.note(at(140.0, 100.0), later));
        assert!(!rest.resting(later, WAIT), "it has only just stopped");
        assert!(rest.resting(later + WAIT, WAIT));
    }

    /// A pixel of jitter is not a move, but a slow drift is one once it has
    /// gone further than the slack from where the pointer stopped.
    #[test]
    fn jitter_is_not_a_move_and_a_slow_drift_is() {
        let mut rest = Rest::default();
        let start = Instant::now();
        rest.note(at(100.0, 100.0), start);
        assert!(!rest.note(at(101.0, 99.0), start + WAIT));
        assert!(rest.resting(start + WAIT, WAIT), "still where it stopped");
        // A pixel a frame, each within the slack of the one before, but
        // measured from where it stopped.
        assert!(!rest.note(at(102.0, 100.0), start + WAIT));
        assert!(rest.note(at(103.0, 100.0), start + WAIT));
        assert!(!rest.resting(start + WAIT, WAIT));
    }

    /// Leaving and coming back to the same pixel is a move: the pointer went
    /// somewhere, and what it comes back to wants waking.
    #[test]
    fn coming_back_is_a_move() {
        let mut rest = Rest::default();
        let start = Instant::now();
        rest.note(at(100.0, 100.0), start);
        rest.forget();
        assert!(!rest.resting(start + WAIT * 2, WAIT));
        assert!(rest.note(at(100.0, 100.0), start + WAIT * 2));
        assert!(!rest.resting(start + WAIT * 2, WAIT));
    }

    /// The time left counts down to nothing and stays there, so a waker
    /// asked late does not wait again.
    #[test]
    fn the_time_left_counts_down_to_nothing() {
        let mut rest = Rest::default();
        let start = Instant::now();
        rest.note(at(100.0, 100.0), start);
        assert_eq!(rest.until(start, WAIT), Some(WAIT));
        let second = Duration::from_secs(1);
        assert_eq!(rest.until(start + second, WAIT), Some(WAIT - second));
        assert_eq!(rest.until(start + WAIT * 4, WAIT), Some(Duration::ZERO));
    }
}
