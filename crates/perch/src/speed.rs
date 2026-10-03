//! How fast a recording plays: the speeds a pane's More offers, and how each
//! is written.
//!
//! A recording only. A live stream plays at the speed it is made, and
//! anything faster would run into the live edge within the minute; the More
//! menu of a live pane has no speed row, and `VideoStream::set_speed` does
//! nothing to one.
//!
//! Per pane and never saved: the speed lives on the pane's
//! `video::PositionHandle`, beside its position, so every player the pane
//! starts — a quality swapped in place included — plays at it from its first
//! pass, the chat replay reads it to tell playback from a seek, and a pane
//! that plays something else, which is a new pane with a new handle, starts
//! at one again. The pitch is kept by mpv itself (`Player::set_speed`).
//!
//! Hundredths of the normal speed rather than a float, so the menu can mark
//! the one chosen by equality, and the handle can hold it in an atomic.

/// A playback speed, in hundredths of the normal one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Speed(u16);

impl Speed {
    /// The recording as it was made.
    pub const NORMAL: Speed = Speed(100);

    /// What More's speed menu offers, slowest first: a step under normal for
    /// a dense explanation, and quarter steps up to double for catching up
    /// on a long broadcast.
    pub const CHOICES: [Speed; 6] = [
        Speed(75),
        Speed(100),
        Speed(125),
        Speed(150),
        Speed(175),
        Speed(200),
    ];

    /// The speed held as `hundredths`, if it is one the menu offers;
    /// anything else is the normal speed, so a handle can never come to
    /// hold a speed nobody could have chosen.
    pub fn from_hundredths(hundredths: u16) -> Speed {
        Self::CHOICES
            .into_iter()
            .find(|speed| speed.0 == hundredths)
            .unwrap_or(Self::NORMAL)
    }

    pub fn hundredths(self) -> u16 {
        self.0
    }

    /// Seconds of the recording a second plays: what mpv is told, and what
    /// the chat replay allows for.
    pub fn factor(self) -> f64 {
        f64::from(self.0) / 100.0
    }

    pub fn is_normal(self) -> bool {
        self == Self::NORMAL
    }

    /// How a speed is written in the menu and on the seek row: `0.75x`,
    /// `1x`, `1.5x`, `2x` — no trailing zeros, the way every player writes
    /// it.
    pub fn label(self) -> String {
        let (whole, part) = (self.0 / 100, self.0 % 100);
        let part = match part {
            0 => String::new(),
            p if p % 10 == 0 => format!(".{}", p / 10),
            p => format!(".{p:02}"),
        };
        format!("{whole}{part}x")
    }
}

impl Default for Speed {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_are_written_without_trailing_zeros() {
        let labels: Vec<String> = Speed::CHOICES.iter().map(|s| s.label()).collect();
        assert_eq!(labels, ["0.75x", "1x", "1.25x", "1.5x", "1.75x", "2x"]);
    }

    /// Slowest first, normal among them, nothing twice, and nothing past
    /// double: the menu is the list as it stands.
    #[test]
    fn the_choices_climb_through_normal_to_double() {
        assert!(Speed::CHOICES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(Speed::CHOICES.contains(&Speed::NORMAL));
        assert_eq!(Speed::CHOICES.first().map(|s| s.factor()), Some(0.75));
        assert_eq!(Speed::CHOICES.last().map(|s| s.factor()), Some(2.0));
    }

    #[test]
    fn a_speed_round_trips_through_its_hundredths() {
        for speed in Speed::CHOICES {
            assert_eq!(Speed::from_hundredths(speed.hundredths()), speed);
        }
        assert_eq!(Speed::from_hundredths(0), Speed::NORMAL, "never stopped");
        assert_eq!(Speed::from_hundredths(300), Speed::NORMAL, "not offered");
        assert_eq!(Speed::default(), Speed::NORMAL);
        assert!(Speed::NORMAL.is_normal());
        assert!(!Speed::from_hundredths(150).is_normal());
    }
}
