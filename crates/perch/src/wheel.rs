//! `Alt` and the wheel over a picture, turned into volume steps.
//!
//! The wheel over a pane's picture, or a pop-out's, changes that pane's
//! level only while `Alt` is held, which the user asked for by name: a wheel
//! turned over a video on the way to somewhere else must never change what
//! it sounds like. Without `Alt` the wheel does nothing to the player at all.
//! A step is a volume key's step (`keys::VOLUME_STEP`) taken through the
//! same path, `VideoView::nudge_volume`, so the slider follows, the hush
//! ends and the channel remembers the level as it would from a key.
//!
//! Two kinds of wheel arrive, and both have to come out as steps of the same
//! size:
//!
//! - **A wheel that clicks.** On Windows every turn of it arrives as lines
//!   (gpui's `ScrollDelta::Lines`), a click being as many lines as the
//!   system scrolls per notch — three unless somebody changed it — and
//!   always a whole number of them (gpui's windows/events.rs). One event is
//!   one step, whatever the system's lines per notch, so a step is a click
//!   on every machine.
//! - **A wheel that glides**: a precision touchpad, which Windows also
//!   reports as lines but in fractions of one, many times a second; or a
//!   trackpad elsewhere, in pixels. Those are added up, and a step is taken
//!   each time a click's worth has gathered ([`LINES_PER_STEP`],
//!   [`PIXELS_PER_STEP`]), so a swipe moves the level by a few steps rather
//!   than by one per event. Travel the other way starts the count again, so
//!   a change of mind is not first spent undoing what had gathered.
//!
//! The two meet in a touchpad's event that happens to come out whole: its
//! delta a multiple of 40 at three lines a notch is one line, two, and so
//! on. Taken as clicks, those would each take a step and throw away what
//! had gathered, which in a fast swipe undoes the gathering. So a whole
//! number of lines is a click only when it is a click's worth or more, or
//! when nothing is gathering — which is how a wheel set to one or two lines
//! a notch still steps once a click, since a click never leaves anything
//! gathered. Under a click's worth, in the middle of a glide, it is
//! gathered like the fractions around it.
//!
//! Up — away from you, or two fingers pushed up — is louder.

use gpui::ScrollDelta;

/// How many lines of a gliding wheel make a step: one click's worth at
/// Windows' default of three lines a notch.
pub const LINES_PER_STEP: f32 = 3.0;

/// How many pixels of a trackpad make a step. A first guess: a short,
/// deliberate swipe moves the level a few steps.
pub const PIXELS_PER_STEP: f32 = 50.0;

/// The most steps one event may take, so an event no wheel would send — a
/// flung trackpad's thousand pixels — cannot cross the whole range at once.
const MOST_STEPS: i16 = 4;

/// The travel a gliding wheel has gathered towards its next step. See the
/// module.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wheel {
    /// In steps, signed: positive is louder. Always less than one either
    /// way between events.
    gathered: f32,
}

impl Wheel {
    /// The whole steps `delta` adds up to, positive for louder, keeping what
    /// is left over for the next event.
    pub fn steps(&mut self, delta: &ScrollDelta) -> i16 {
        let travel = match delta {
            ScrollDelta::Lines(lines) => {
                let y = lines.y;
                let whole = y != 0.0 && y.fract() == 0.0;
                if whole && (y.abs() >= LINES_PER_STEP || self.gathered == 0.0) {
                    // A click: one step, and nothing gathered goes with it.
                    self.gathered = 0.0;
                    return y.signum() as i16;
                }
                y / LINES_PER_STEP
            }
            ScrollDelta::Pixels(pixels) => f32::from(pixels.y) / PIXELS_PER_STEP,
        };
        if travel == 0.0 || !travel.is_finite() {
            return 0;
        }
        if self.gathered != 0.0 && self.gathered.signum() != travel.signum() {
            self.gathered = 0.0;
        }
        self.gathered += travel;
        let whole = self.gathered.trunc();
        self.gathered -= whole;
        (whole as i16).clamp(-MOST_STEPS, MOST_STEPS)
    }

    /// Forget what has gathered: a wheel turned without `Alt`, which is
    /// nothing to do with the level, ends a run of them.
    pub fn reset(&mut self) {
        self.gathered = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::*;

    fn lines(y: f32) -> ScrollDelta {
        ScrollDelta::Lines(point(0.0, y))
    }

    fn pixels(y: f32) -> ScrollDelta {
        ScrollDelta::Pixels(point(px(0.0), px(y)))
    }

    /// A click of the wheel is one step, however many lines the system
    /// scrolls per notch, up louder and down quieter.
    #[test]
    fn a_click_is_one_step_whatever_the_lines_per_notch() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.steps(&lines(3.0)), 1);
        assert_eq!(wheel.steps(&lines(-3.0)), -1);
        assert_eq!(wheel.steps(&lines(1.0)), 1, "one line a notch");
        assert_eq!(wheel.steps(&lines(10.0)), 1, "ten lines a notch");
    }

    /// A precision touchpad's fractions of a line add up to a step a click's
    /// worth at a time, and nothing is lost between them.
    #[test]
    fn a_touchpad_gathers_into_steps() {
        let mut wheel = Wheel::default();
        let taken: i16 = (0..8).map(|_| wheel.steps(&lines(0.75))).sum();
        assert_eq!(taken, 2, "six lines is two clicks' worth");
        assert_eq!(wheel.steps(&lines(0.75)), 0);
    }

    /// A touchpad's event that comes out a whole line or two in the middle
    /// of a glide is gathered with the fractions around it, rather than
    /// taken as a click that throws the gathering away.
    #[test]
    fn whole_lines_in_a_glide_are_gathered() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.steps(&lines(0.5)), 0);
        assert_eq!(wheel.steps(&lines(1.0)), 0, "half a click gathered");
        assert_eq!(wheel.steps(&lines(2.0)), 1, "three and a half lines");
        assert_eq!(wheel.steps(&lines(0.5)), 0);
        assert_eq!(wheel.steps(&lines(1.0)), 0);
        assert_eq!(wheel.steps(&lines(2.0)), 1, "seven lines in all");
        assert_eq!(wheel.steps(&lines(-3.0)), -1, "a click's worth is a click");
        assert_eq!(wheel.steps(&lines(-1.0)), -1, "nothing was gathering");
    }

    /// A trackpad in pixels the same way: a short swipe is a few steps, not
    /// one per event.
    #[test]
    fn a_trackpad_does_not_jump() {
        let mut wheel = Wheel::default();
        let taken: Vec<i16> = (0..10).map(|_| wheel.steps(&pixels(12.0))).collect();
        assert_eq!(taken.iter().sum::<i16>(), 2);
        assert!(taken.iter().all(|step| step.abs() <= 1));
        assert_eq!(wheel.steps(&pixels(-600.0)), -MOST_STEPS, "a fling is held");
    }

    /// Travel the other way starts again, rather than first undoing what
    /// had gathered.
    #[test]
    fn a_change_of_direction_starts_over() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.steps(&pixels(45.0)), 0);
        assert_eq!(wheel.steps(&pixels(-45.0)), 0);
        assert_eq!(wheel.steps(&pixels(-10.0)), -1);
    }

    /// A reset, or a click, forgets what a glide had gathered.
    #[test]
    fn a_reset_forgets_the_glide() {
        let mut wheel = Wheel::default();
        wheel.steps(&pixels(45.0));
        wheel.reset();
        assert_eq!(wheel.steps(&pixels(10.0)), 0);
        wheel.steps(&pixels(35.0));
        assert_eq!(wheel.steps(&lines(-3.0)), -1);
        assert_eq!(wheel.steps(&pixels(10.0)), 0, "the click took it");
    }

    /// Sideways travel, and nothing at all, are no steps.
    #[test]
    fn sideways_is_nothing() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.steps(&ScrollDelta::Lines(point(3.0, 0.0))), 0);
        assert_eq!(wheel.steps(&pixels(0.0)), 0);
        assert_eq!(wheel, Wheel::default());
    }
}
