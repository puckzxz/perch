//! A pane's ad break, while streamlink is filtering one out: the notice in
//! its header, how long it has left, and when it is over.
//!
//! Streamlink drops a Twitch ad's segments and sends the relay nothing while
//! it plays, so the picture holds its last frame with no word of why. The
//! twitch plugin says when it finds one, and how long it is when it knows
//! (`streamlink`'s `ads`, `StreamEvent::AdBreak`); nothing says when it
//! ends. So the pane's header carries a tag, off the picture, `ad break ·
//! 0:25` counting down, or `ad break` with no length to count, until one of
//! two things says it is over (`AdBreak::over`): the length has run out, or
//! the picture has moved again after standing still since the notice — the
//! player's [`frames_resumed`], which also ends one with no length. A notice
//! with no length that sees neither goes after [`NO_LENGTH_CAP`], so a word
//! missed cannot leave the tag up for the rest of the stream.
//!
//! The pane's slot holds it (`watch::Slot::ad_break`), any change of the
//! pane's state drops it, and the root ticks it once a second while it is up
//! (`RootView::ad_break_began`). Pure and tested here.

use std::time::{Duration, Instant};

use crate::seek_bar;

/// How long the picture must have stood still for its next frame to count
/// as it moving again. Far longer than a frame of any stream, and shorter
/// than any ad.
pub const FRAME_GAP: Duration = Duration::from_secs(1);

/// How long a notice with no length stays up when nothing says it is over.
/// Twitch's pre-rolls run well under this.
pub const NO_LENGTH_CAP: Duration = Duration::from_secs(120);

/// One ad break, from the moment streamlink said so. See the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdBreak {
    /// When streamlink said it, or last said a length for it.
    since: Instant,
    /// How long it is, in whole seconds, when streamlink said.
    secs: Option<u32>,
}

impl AdBreak {
    /// The break after streamlink says one of `secs` at `now`, with `old`
    /// the one up already, if any.
    ///
    /// A length is counted from when it is said: the pre-roll's wait comes
    /// first with none and its length a moment later, and that length is
    /// the break's. A word with no length while one is counting leaves the
    /// count alone.
    pub fn said(old: Option<AdBreak>, secs: Option<u32>, now: Instant) -> AdBreak {
        match (old, secs) {
            (Some(old), None) => old,
            (_, secs) => AdBreak { since: now, secs },
        }
    }

    /// How long it has left at `now`, when it has a length.
    pub fn left(&self, now: Instant) -> Option<Duration> {
        self.secs.map(|secs| {
            Duration::from_secs(u64::from(secs))
                .saturating_sub(now.saturating_duration_since(self.since))
        })
    }

    /// Whether it is over at `now`, with the picture last moving again at
    /// `resumed`: its length run out, the picture moving again since it was
    /// said, or with no length, [`NO_LENGTH_CAP`] gone by.
    pub fn over(&self, now: Instant, resumed: Option<Instant>) -> bool {
        if resumed.is_some_and(|resumed| resumed > self.since) {
            return true;
        }
        match self.left(now) {
            Some(left) => left.is_zero(),
            None => now.saturating_duration_since(self.since) >= NO_LENGTH_CAP,
        }
    }

    /// What the header's tag says at `now`: `ad break · 0:25`, counted up to
    /// the whole second so it never reads `0:00` while still up, or `ad
    /// break` with no length.
    pub fn tag(&self, now: Instant) -> String {
        match self.left(now) {
            Some(left) => {
                let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
                format!("ad break · {}", seek_bar::timecode(secs as f64))
            }
            None => "ad break".to_string(),
        }
    }

    /// How long from `now` until the tag's words next change: the next whole
    /// second of its count, or a second on for one with no length, which
    /// still has to be asked whether it is over.
    pub fn next_tick(&self, now: Instant) -> Duration {
        let second = Duration::from_secs(1);
        match self.left(now) {
            Some(left) if !left.is_zero() => {
                let into = Duration::from_nanos(u64::from(left.subsec_nanos()));
                if into.is_zero() {
                    second
                } else {
                    into
                }
            }
            _ => second,
        }
    }
}

/// Whether a frame arriving at `now` is the picture moving again, after the
/// frame before it at `previous` — or none at all, for the first, which is
/// what ends a pre-roll.
pub fn frames_resumed(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_none_or(|previous| now.saturating_duration_since(previous) >= FRAME_GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A break of known length counts down in whole seconds, rounded up,
    /// and is over when it has run out.
    #[test]
    fn a_break_counts_down_to_its_end() {
        let start = Instant::now();
        let ad = AdBreak::said(None, Some(30), start);
        assert_eq!(ad.tag(start), "ad break · 0:30");
        assert_eq!(
            ad.tag(start + Duration::from_millis(4_200)),
            "ad break · 0:26"
        );
        assert_eq!(ad.tag(start + secs(29)), "ad break · 0:01");
        assert!(!ad.over(start + secs(29), None));
        assert!(ad.over(start + secs(30), None));
        assert_eq!(
            AdBreak::said(None, Some(90), start).tag(start),
            "ad break · 1:30"
        );
    }

    /// With no length, the tag says only that it is one, and it waits for
    /// the picture, or the cap.
    #[test]
    fn a_break_with_no_length_waits() {
        let start = Instant::now();
        let ad = AdBreak::said(None, None, start);
        assert_eq!(ad.tag(start + secs(40)), "ad break");
        assert!(!ad.over(start + secs(40), None));
        assert!(ad.over(start + NO_LENGTH_CAP, None));
    }

    /// The picture moving again since the notice ends it, length or none;
    /// moving before it does not.
    #[test]
    fn the_picture_moving_again_ends_it() {
        let start = Instant::now() + secs(5);
        for length in [Some(30), None] {
            let ad = AdBreak::said(None, length, start);
            assert!(ad.over(start + secs(12), Some(start + secs(11))));
            assert!(!ad.over(start + secs(12), Some(start - secs(5))));
        }
    }

    /// The pre-roll's wait, then its length: the length counts from when it
    /// came. A word with no length after one leaves the count alone.
    #[test]
    fn a_length_said_later_is_the_breaks() {
        let start = Instant::now();
        let waiting = AdBreak::said(None, None, start);
        let counted = AdBreak::said(Some(waiting), Some(20), start + secs(2));
        assert_eq!(counted.tag(start + secs(2)), "ad break · 0:20");
        let again = AdBreak::said(Some(counted), None, start + secs(5));
        assert_eq!(again, counted);
        let next = AdBreak::said(Some(counted), Some(60), start + secs(30));
        assert_eq!(next.tag(start + secs(30)), "ad break · 1:00");
    }

    /// The tick lands on the count's next whole second, and a second on
    /// otherwise.
    #[test]
    fn the_tick_follows_the_count() {
        let start = Instant::now();
        let ad = AdBreak::said(None, Some(30), start);
        assert_eq!(ad.next_tick(start), secs(1));
        assert_eq!(
            ad.next_tick(start + Duration::from_millis(4_200)),
            Duration::from_millis(800)
        );
        assert_eq!(AdBreak::said(None, None, start).next_tick(start), secs(1));
        assert_eq!(ad.next_tick(start + secs(31)), secs(1));
    }

    /// The first frame is the picture moving, and so is one after a stand
    /// of a second or more; frames at a stream's rate are not.
    #[test]
    fn frames_resume_only_after_standing_still() {
        let then = Instant::now();
        assert!(frames_resumed(None, then));
        assert!(frames_resumed(Some(then), then + FRAME_GAP));
        assert!(!frames_resumed(
            Some(then),
            then + Duration::from_millis(33)
        ));
    }
}
