//! How loud one pane is: the level the user chose, and whether Mute all is
//! holding it silent.
//!
//! Two things used to share one number. The level a pane plays at was also
//! the level it reported as the user's choice, so anything that silenced a
//! pane without being a choice — leaving the watch page, once — had to go
//! round the reporting by hand, and a way round that was missed would save
//! `volume: 0` against the channel. Here the two are kept apart: `level` is
//! what the user chose, the slider shows and the settings remember, and the
//! hush is a silence on top of it that never touches it. A pane reports a
//! level only from [`Loudness::set`], so Mute all, and the compact player it
//! is offered from, cannot save a 0.
//!
//! Pure and tested, because this is the one place that decides what mpv
//! hears and what the channel remembers.

/// One pane's loudness. See the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loudness {
    /// What the user chose, 0 to 100: the slider's value, the control bar's
    /// figure, and the only level that is ever reported to be remembered.
    level: u8,
    /// What `M` brings back to a pane whose level is 0, so unmuting restores
    /// rather than guessing. Never 0.
    before_mute: u8,
    /// Held silent by Mute all. Not a preference: it lasts the session, ends
    /// at the pane's first deliberate change, and is never saved.
    hushed: bool,
}

impl Loudness {
    /// A pane opening at `level`, and silent from the start when `hushed`.
    ///
    /// A pane opened at 0 — a channel saved muted — unmutes to 1% rather
    /// than staying silent, which is the least a press of `M` can mean.
    pub fn new(level: u8, hushed: bool) -> Self {
        let level = level.min(100);
        Self {
            level,
            before_mute: level.max(1),
            hushed,
        }
    }

    /// What the user chose, whatever the hush is doing.
    pub fn level(&self) -> u8 {
        self.level
    }

    /// What mpv should play at: the level, or nothing while hushed.
    pub fn audible(&self) -> u8 {
        if self.hushed {
            0
        } else {
            self.level
        }
    }

    /// The user chose `level`, from the slider, a key or `M`.
    ///
    /// A choice ends the hush, because a pane somebody has just set the
    /// volume of is one they mean to hear — and a hush outliving it would
    /// leave the slider moving with nothing changing.
    pub fn set(&mut self, level: u8) {
        let level = level.min(100);
        self.level = level;
        if level > 0 {
            self.before_mute = level;
        }
        self.hushed = false;
    }

    /// What `M` sets: from silence, whether muted or hushed, back to the
    /// level — or to the level before muting, when the level is itself 0 —
    /// and otherwise 0.
    pub fn toggled(&self) -> u8 {
        if self.audible() > 0 {
            0
        } else if self.level > 0 {
            self.level
        } else {
            self.before_mute
        }
    }

    /// Hold the pane silent, leaving its level alone. Returns whether that
    /// changed anything.
    pub fn hush(&mut self) -> bool {
        !std::mem::replace(&mut self.hushed, true)
    }

    /// Let the pane be heard at its level again. Returns whether that changed
    /// anything; a pane the user muted stays muted, since its level is 0.
    pub fn unhush(&mut self) -> bool {
        std::mem::replace(&mut self.hushed, false)
    }
}

/// Whether to offer Unmute all rather than Mute all: when there is something
/// playing and every pane of it is hushed. One audible pane is enough to
/// make muting the useful offer.
pub fn unmute_all_offered(quiet: impl IntoIterator<Item = bool>) -> bool {
    let mut any = false;
    for hushed in quiet {
        if !hushed {
            return false;
        }
        any = true;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only level a pane ever reports is [`Loudness::level`], and only
    /// from `set`. Hushing changes what is heard and leaves that alone, so
    /// Mute all cannot save a 0 against the channel.
    #[test]
    fn hushing_never_asks_to_emit() {
        let mut loudness = Loudness::new(60, false);
        assert!(loudness.hush());
        assert_eq!(loudness.audible(), 0);
        assert_eq!(loudness.level(), 60, "a hush is not a level");
        assert!(!loudness.hush(), "a second hush changes nothing");
        assert!(loudness.unhush());
        assert_eq!(loudness.level(), 60);
        assert!(!loudness.unhush(), "nor does a second unhush");
    }

    #[test]
    fn unhushing_restores_the_level_before_the_hush() {
        let mut loudness = Loudness::new(45, false);
        loudness.hush();
        loudness.unhush();
        assert_eq!(loudness.audible(), 45);
    }

    /// Unmute all is about Mute all, not about every silence: a pane muted
    /// by hand before the hush is still muted after it.
    #[test]
    fn unmute_all_leaves_a_pane_the_user_muted_silent() {
        let mut loudness = Loudness::new(70, false);
        loudness.set(0);
        loudness.hush();
        loudness.unhush();
        assert_eq!(loudness.audible(), 0);
        assert_eq!(loudness.level(), 0);
    }

    #[test]
    fn a_level_set_while_hushed_ends_the_hush() {
        let mut loudness = Loudness::new(30, true);
        loudness.set(55);
        assert_eq!(loudness.audible(), 55);
        assert!(loudness.hush(), "the hush had already ended");
    }

    #[test]
    fn m_on_a_hushed_pane_brings_back_its_level() {
        let mut loudness = Loudness::new(80, false);
        loudness.hush();
        assert_eq!(loudness.toggled(), 80);
        loudness.set(loudness.toggled());
        assert_eq!(loudness.audible(), 80);
    }

    #[test]
    fn mute_then_unmute_returns_to_the_level_not_one_percent() {
        let mut loudness = Loudness::new(65, false);
        assert_eq!(loudness.toggled(), 0);
        loudness.set(loudness.toggled());
        assert_eq!(loudness.audible(), 0);
        assert_eq!(loudness.toggled(), 65);
        loudness.set(loudness.toggled());
        assert_eq!(loudness.audible(), 65);
    }

    /// A pane rebuilt while Mute all holds — a quality change from the
    /// settings, `Try again` — starts silent, with the level its slider
    /// should show.
    #[test]
    fn a_pane_born_quiet_is_silent_but_keeps_its_level() {
        let loudness = Loudness::new(40, true);
        assert_eq!(loudness.audible(), 0);
        assert_eq!(loudness.level(), 40);
        assert_eq!(loudness.toggled(), 40);
    }

    #[test]
    fn unmute_all_is_offered_only_when_every_pane_is_quiet() {
        assert!(
            !unmute_all_offered(Vec::<bool>::new()),
            "nothing playing offers nothing"
        );
        assert!(unmute_all_offered([true]));
        assert!(unmute_all_offered([true, true, true]));
        assert!(!unmute_all_offered([true, false]));
        assert!(!unmute_all_offered([false]));
    }

    #[test]
    fn levels_past_the_top_are_held_to_it() {
        let mut loudness = Loudness::new(250, false);
        assert_eq!(loudness.level(), 100);
        loudness.set(130);
        assert_eq!(loudness.audible(), 100);
    }
}
