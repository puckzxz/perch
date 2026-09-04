//! Choosing which Twitch quality to pull for a given pane size.
//!
//! This is a performance decision, not a bandwidth one. Measured on a live
//! 1080p60 stream, the cost of one rendered frame depends far more on the
//! *ratio* between source and pane than on the pixel count:
//!
//! | source → pane            | CPU (one core) |
//! |--------------------------|----------------|
//! | 1080p → 960x540 (½)      | 35%            |
//! | 1080p → 1920x1080 (1:1)  | 79%            |
//! | 1080p → 1280x720 (0.67×) | 100%           |
//! | 720p  → 1920x1080 (up)   | 117–196%       |
//!
//! Note the third row: an *arbitrary downscale* costs more than rendering at
//! native size despite producing fewer pixels. So the rule is to prefer clean
//! ratios and never upscale.

/// One entry from streamlink's quality list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quality {
    /// The name to hand back to streamlink, e.g. `"720p60"`.
    pub name: String,
    /// Vertical resolution, e.g. 720.
    pub height: u32,
    /// Frames per second where the name says so, else 30.
    pub fps: u32,
}

/// Parse a streamlink quality name. Returns `None` for `audio_only`, `best`,
/// `worst` and anything else without a resolution.
pub fn parse_quality(name: &str) -> Option<Quality> {
    let (height, rest) = name.split_once('p')?;
    let height: u32 = height.parse().ok()?;
    // The suffix is either empty ("720p") or an fps ("720p60").
    let fps = if rest.is_empty() {
        30
    } else {
        rest.parse().ok()?
    };
    Some(Quality {
        name: name.to_string(),
        height,
        fps,
    })
}

/// Lower is better. Encodes the table above as a preference order.
fn cost(source_height: u32, pane_height: u32) -> u32 {
    // A pane with no height has no ratio to prefer. Treated as "smallest
    // source wins" rather than dividing by it: `select` is public, and a
    // caller that has not laid out yet should get an answer, not a panic on
    // the supervisor thread that leaves the pane at "starting" forever.
    if pane_height == 0 {
        return 1000 + source_height;
    }
    if source_height < pane_height {
        return 1000 + (pane_height - source_height); // upscaling: always worst
    }
    if source_height == pane_height {
        return 1; // 1:1, no scaler at all
    }
    if source_height % pane_height == 0 {
        return 2; // exact 1/2, 1/3 …: the fast path
    }
    // Arbitrary ratio. Prefer less downscaling, since the scaler runs either way.
    100 + (source_height - pane_height) / 100
}

/// Pick the quality to request for a pane `pane_height` pixels tall.
///
/// Returns `None` only when the list holds no real resolutions at all.
pub fn select(available: &[String], pane_height: u32) -> Option<Quality> {
    let mut candidates: Vec<Quality> = available
        .iter()
        .filter_map(|name| parse_quality(name))
        .collect();
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|q| {
        // Tie-break on higher fps, then on name for determinism.
        (
            cost(q.height, pane_height),
            u32::MAX - q.fps,
            q.name.clone(),
        )
    });
    candidates.into_iter().next()
}

/// A saved quality preference, resolved against what a channel actually offers.
///
/// Preferences are stored by height (`"1080p"`) rather than by exact streamlink
/// name, because names vary per channel and per encoder: one stream offers
/// `480p30`, another `480p`, a third `936p60`. Matching on the exact string
/// means a preference silently stops applying the moment a channel names its
/// renditions differently.
pub fn select_named(available: &[String], preference: &str, pane_height: u32) -> Option<Quality> {
    let preference = preference.trim();

    if preference.eq_ignore_ascii_case("auto") || preference.is_empty() {
        return select(available, pane_height);
    }

    let mut candidates: Vec<Quality> = available
        .iter()
        .filter_map(|name| parse_quality(name))
        .collect();
    if candidates.is_empty() {
        return None;
    }

    if preference.eq_ignore_ascii_case("best") || preference.eq_ignore_ascii_case("source") {
        candidates.sort_by_key(|q| (q.height, q.fps));
        return candidates.pop();
    }

    // A preference naming an exact rendition ("720p60") is taken literally, but
    // a bare height ("720p") means "that height, best frame rate" - otherwise it
    // would match a 30fps rendition and silently ignore the 60fps one.
    let bare_height = preference.ends_with('p');
    if !bare_height {
        if let Some(exact) = candidates.iter().find(|q| q.name == preference) {
            return Some(exact.clone());
        }
    }

    // Match on height, preferring the higher frame rate.
    if let Some(wanted) = parse_quality(preference).map(|q| q.height) {
        let mut same_height: Vec<Quality> = candidates
            .iter()
            .filter(|q| q.height == wanted)
            .cloned()
            .collect();
        same_height.sort_by_key(|q| q.fps);
        if let Some(best) = same_height.pop() {
            return Some(best);
        }
    }

    // The channel does not offer anything like it today; fall back rather than
    // fail, because refusing to play is worse than playing a nearby quality.
    select(available, pane_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// `select` is public and the app clamps its pane height before calling,
    /// but the function has to stand on its own: a zero divides nothing, and
    /// the cheapest rendition is the only sensible answer for a pane that has
    /// no size yet.
    #[test]
    fn a_pane_with_no_height_gets_the_smallest_stream_not_a_panic() {
        let picked = select(&names(&["1080p60", "720p60", "360p30"]), 0).unwrap();
        assert_eq!(picked.name, "360p30");
    }

    const TWITCH: [&str; 8] = [
        "audio_only",
        "160p30",
        "360p30",
        "480p30",
        "720p60",
        "1080p60",
        "worst",
        "best",
    ];

    #[test]
    fn parses_quality_names() {
        assert_eq!(
            parse_quality("720p60"),
            Some(Quality {
                name: "720p60".into(),
                height: 720,
                fps: 60
            })
        );
        assert_eq!(parse_quality("480p").map(|q| q.fps), Some(30));
        assert_eq!(parse_quality("audio_only"), None);
        assert_eq!(parse_quality("best"), None);
    }

    #[test]
    fn prefers_an_exact_match() {
        let picked = select(&names(&TWITCH), 1080).unwrap();
        assert_eq!(picked.name, "1080p60");
    }

    /// The measurement that drives this module: 1080p halved is far cheaper
    /// than 720p scaled by an awkward ratio, even though 720p is "closer".
    #[test]
    fn prefers_an_exact_half_over_a_nearer_awkward_ratio() {
        let picked = select(&names(&TWITCH), 540).unwrap();
        assert_eq!(picked.name, "1080p60");
    }

    #[test]
    fn never_upscales_when_a_larger_source_exists() {
        let picked = select(&names(&TWITCH), 900).unwrap();
        assert_eq!(picked.name, "1080p60");
    }

    #[test]
    fn falls_back_to_the_largest_when_the_pane_exceeds_every_source() {
        let picked = select(&names(&["360p30", "480p30"]), 1440).unwrap();
        assert_eq!(picked.name, "480p30");
    }

    #[test]
    fn ignores_non_resolution_entries() {
        assert_eq!(select(&names(&["audio_only", "best", "worst"]), 720), None);
    }

    #[test]
    fn prefers_higher_fps_at_equal_cost() {
        let picked = select(&names(&["720p", "720p60"]), 720).unwrap();
        assert_eq!(picked.name, "720p60");
    }

    #[test]
    fn named_preference_matches_by_height_across_naming_styles() {
        // Preference says "480p"; this channel calls it "480p30".
        let picked = select_named(&names(&TWITCH), "480p", 720).unwrap();
        assert_eq!(picked.name, "480p30");
    }

    #[test]
    fn named_preference_prefers_higher_fps_at_the_same_height() {
        let list = names(&["720p", "720p60", "1080p60"]);
        assert_eq!(select_named(&list, "720p", 720).unwrap().name, "720p60");
    }

    #[test]
    fn best_takes_the_highest_available() {
        let list = names(&["audio_only", "160p30", "720p60", "1440p60", "1080p60"]);
        assert_eq!(select_named(&list, "best", 720).unwrap().name, "1440p60");
    }

    #[test]
    fn auto_defers_to_pane_based_selection() {
        // 720 pane with 720p60 present: exact 1:1 beats halving 1440p60.
        let list = names(&["720p60", "1440p60"]);
        assert_eq!(select_named(&list, "auto", 720).unwrap().name, "720p60");
    }

    #[test]
    fn unavailable_preference_falls_back_instead_of_failing() {
        // No 1440p on this channel today.
        let picked = select_named(&names(&TWITCH), "1440p", 720).unwrap();
        assert_eq!(picked.name, "720p60");
    }

    #[test]
    fn exact_name_still_wins_when_offered() {
        let picked = select_named(&names(&TWITCH), "360p30", 720).unwrap();
        assert_eq!(picked.name, "360p30");
    }

    #[test]
    fn exact_third_beats_an_arbitrary_ratio() {
        // 1080/360 = 3 exactly; 480 -> 360 is 1.33x.
        let picked = select(&names(&["480p30", "1080p60"]), 360).unwrap();
        assert_eq!(picked.name, "1080p60");
    }
}
