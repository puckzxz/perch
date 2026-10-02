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
//! The hush has a second way in: `Only this one`, from a pane's More menu,
//! which holds every other pane silent through the same hush and so saves
//! nothing either ([`HearOnly`]).
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

/// What a pane's More menu offers about hearing one pane alone: nothing,
/// `Only this one`, or `Hear all again`.
///
/// `Only this one` hushes every other pane through Mute all's hush (never
/// saved, levels untouched) and lets the pane it was chosen on be heard;
/// `Hear all again` lets every hush go. The same row in every pane, since
/// what it says is about all of them: once one pane alone is heard, every
/// pane's menu offers the way back, the hushed ones' included.
///
/// Worked out from the panes' hushes alone ([`hear_only`]), never kept as a
/// fact of its own, so nothing can leave it saying the opposite of what is
/// heard: a hushed pane somebody sets the level of is heard again, and the
/// row goes back to `Only this one` with nobody telling it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HearOnly {
    /// Fewer than two panes: there is no other pane to hush.
    Hidden,
    /// `Only this one`.
    Offered,
    /// One pane alone is free of the hush and every other is held:
    /// `Hear all again`.
    Held,
}

impl HearOnly {
    /// The row, when there is one: its id and its words. The id follows the
    /// words, as every control whose words follow a state does, so the row
    /// that says the other thing is a new element rather than the old one
    /// relabelled under the pointer.
    pub fn row(self) -> Option<(&'static str, &'static str)> {
        match self {
            HearOnly::Hidden => None,
            HearOnly::Offered => Some(("more-hear-only", "Only this one")),
            HearOnly::Held => Some(("more-hear-all", "Hear all again")),
        }
    }
}

/// What every pane's More menu offers, given each pane's hush, in any order.
/// Held when there are two panes or more and exactly one is not hushed —
/// whether `Only this one` or Mute all and then a level set on one pane got
/// it there, since `Hear all again` means the same in both.
pub fn hear_only(quiet: impl IntoIterator<Item = bool>) -> HearOnly {
    let (mut panes, mut heard) = (0usize, 0usize);
    for hushed in quiet {
        panes += 1;
        heard += usize::from(!hushed);
    }
    match (panes, heard) {
        (0 | 1, _) => HearOnly::Hidden,
        (_, 1) => HearOnly::Held,
        _ => HearOnly::Offered,
    }
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

    /// A lone pane has nobody to hush; two or more offer it until one
    /// alone is heard, and then every pane offers the way back.
    #[test]
    fn only_this_one_until_one_alone_is_heard() {
        assert_eq!(hear_only(Vec::<bool>::new()), HearOnly::Hidden);
        assert_eq!(hear_only([false]), HearOnly::Hidden);
        assert_eq!(hear_only([true]), HearOnly::Hidden);
        assert_eq!(hear_only([false, false]), HearOnly::Offered);
        assert_eq!(hear_only([false, true]), HearOnly::Held);
        assert_eq!(hear_only([true, true, false]), HearOnly::Held);
        assert_eq!(hear_only([false, false, true]), HearOnly::Offered);
        assert_eq!(
            hear_only([true, true]),
            HearOnly::Offered,
            "Mute all: one pane can still be chosen"
        );
        assert_eq!(
            HearOnly::Offered.row(),
            Some(("more-hear-only", "Only this one"))
        );
        assert_eq!(
            HearOnly::Held.row(),
            Some(("more-hear-all", "Hear all again"))
        );
        assert_eq!(HearOnly::Hidden.row(), None);
    }

    /// Hearing one alone is Mute all's hush on the others: their levels
    /// stay what was chosen, so `Hear all again` brings each back to it.
    #[test]
    fn hearing_one_alone_leaves_the_others_levels() {
        let mut chosen = Loudness::new(40, false);
        let mut other = Loudness::new(70, false);
        chosen.unhush();
        other.hush();
        assert_eq!((chosen.audible(), other.audible()), (40, 0));
        assert_eq!(other.level(), 70);
        other.unhush();
        assert_eq!(other.audible(), 70);
    }

    #[test]
    fn levels_past_the_top_are_held_to_it() {
        let mut loudness = Loudness::new(250, false);
        assert_eq!(loudness.level(), 100);
        loudness.set(130);
        assert_eq!(loudness.audible(), 100);
    }
}
