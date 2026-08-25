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
        (cost(q.height, pane_height), u32::MAX - q.fps, q.name.clone())
    });
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const TWITCH: [&str; 8] = [
        "audio_only", "160p30", "360p30", "480p30", "720p60", "1080p60", "worst", "best",
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
    fn exact_third_beats_an_arbitrary_ratio() {
        // 1080/360 = 3 exactly; 480 -> 360 is 1.33x.
        let picked = select(&names(&["480p30", "1080p60"]), 360).unwrap();
        assert_eq!(picked.name, "1080p60");
    }
}
