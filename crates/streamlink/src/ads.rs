//! What streamlink says about a Twitch ad it is filtering out.
//!
//! While an ad plays, streamlink's twitch plugin drops its segments and sends
//! the relay nothing, so the picture freezes on its last frame with nothing
//! to say why. The plugin does log it, at info, on the console streamlink
//! writes its log to — stdout, the stream `Starting server` is read from —
//! in two lines (`src/streamlink/plugins/twitch.py`, checked on 2 October
//! 2026):
//!
//! ```text
//! [plugins.twitch][info] Waiting for pre-roll ads to finish, be patient
//! [plugins.twitch][info] Detected advertisement break of 30 seconds
//! ```
//!
//! The first once, on a first playlist that has ads and no content yet; the
//! second once per break it finds a length for (`second` when it is one),
//! rounded up from Twitch's own figure for the pod. Nothing is logged when
//! the break ends. `Will skip ad segments`, logged whenever the HLS reader
//! starts, is no break and must not read as one.
//!
//! Matched loosely — words, not the logger's prefix or the exact sentence —
//! so a rewording keeps working as long as it still says it, and a line that
//! says nothing of the kind is nothing: a streamlink that logs neither line,
//! or one whose configuration has its log level above info, shows no notice
//! rather than a wrong one.

/// The length of the ad break `line` announces, in whole seconds, when it
/// announces one: `Some(Some(secs))` for a break of known length,
/// `Some(None)` for one with none (the pre-roll's wait), and `None` for a
/// line that is about something else.
pub(crate) fn ad_break(line: &str) -> Option<Option<u32>> {
    let lower = line.to_ascii_lowercase();
    let pre_roll =
        (lower.contains("pre-roll") || lower.contains("preroll")) && lower.contains("ads");
    let detected = lower.contains("advertisement break") || lower.contains("ad break");
    if detected {
        return Some(seconds_in(&lower));
    }
    pre_roll.then_some(None)
}

/// The number just before the first `second` in `lower`, if there is one:
/// `30` in `break of 30 seconds`.
fn seconds_in(lower: &str) -> Option<u32> {
    let before = &lower[..lower.find("second")?];
    let digits: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.chars().rev().collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two lines streamlink's twitch plugin writes, as they arrive.
    #[test]
    fn streamlinks_own_lines_are_breaks() {
        assert_eq!(
            ad_break("[plugins.twitch][info] Detected advertisement break of 30 seconds"),
            Some(Some(30))
        );
        assert_eq!(
            ad_break("[plugins.twitch][info] Detected advertisement break of 1 second"),
            Some(Some(1))
        );
        assert_eq!(
            ad_break("[plugins.twitch][info] Waiting for pre-roll ads to finish, be patient"),
            Some(None)
        );
    }

    /// Loose: another prefix, another case, another wording that still says
    /// it, and a length that is missing or unreadable is a break with none.
    #[test]
    fn a_rewording_still_reads_as_a_break() {
        assert_eq!(ad_break("Detected ad break of 90 seconds"), Some(Some(90)));
        assert_eq!(
            ad_break("[twitch][INFO] DETECTED ADVERTISEMENT BREAK OF 15 SECONDS"),
            Some(Some(15))
        );
        assert_eq!(ad_break("Detected advertisement break"), Some(None));
        assert_eq!(ad_break("Detected ad break of many seconds"), Some(None));
        assert_eq!(ad_break("waiting for preroll ads"), Some(None));
    }

    /// Everything else streamlink says is nothing, the reader's own word
    /// about skipping ads above all: it comes at every start, ad or none.
    #[test]
    fn other_lines_are_nothing() {
        for line in [
            "[stream.hls][info] Will skip ad segments",
            "[cli][info] Starting server, access with one of:",
            "[cli][info]  http://127.0.0.1:52344/",
            "[cli][info] Available streams: audio_only, 160p (worst), 1080p60 (best)",
            "[stream.hls][warning] Failed to reload playlist: 404",
            "",
        ] {
            assert_eq!(ad_break(line), None, "{line}");
        }
    }
}
