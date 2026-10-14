//! Asks the three third-party emote providers for their global sets, once,
//! the way a pane does: FrankerFaceZ, BetterTTV and 7TV. None of them is a
//! contract, and a provider that stops answering only leaves its emotes as
//! words in chat, which nobody reports; nor does one that stops marking its
//! overlays and effects, which only leaves them drawn as pictures of their
//! own. Ignored, and never run in CI. Run by hand before a release, through
//! `scripts/live-check.sh`, or
//!
//! ```text
//! cargo test -p emotes --test live -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use emotes::{Effect, EmoteLoader, EmoteSets, Kind};

/// One global emote each provider has carried for years, by name.
const KNOWN: [(&str, &str); 3] = [
    ("FrankerFaceZ", "LilZ"),
    ("BetterTTV", "SourPls"),
    ("7TV", "EZ"),
];

/// 7TV's zero-width globals as of 14 October 2026. They come and go with the
/// seasons, so the test says which are still there rather than asserting it.
const SEVENTV_ZERO_WIDTH: [&str; 3] = ["PETPET", "SteerR", "RainTime"];

/// The global sets as a pane loads them, once every name in `wanted` has
/// landed or 20 seconds have passed, and the names still missing.
fn global_sets(wanted: &[&str]) -> (EmoteSets, Vec<String>) {
    let (loader, _ready) = EmoteLoader::new();
    loader.load_global();
    let deadline = Instant::now() + Duration::from_secs(20);
    let missing = || {
        let sets = loader.sets();
        wanted
            .iter()
            .filter(|name| sets.lookup(name).is_none())
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    while !missing().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    (loader.sets(), missing())
}

#[test]
#[ignore = "asks FFZ, BTTV and 7TV over the network; run by hand"]
fn every_provider_still_answers() {
    let names: Vec<&str> = KNOWN.iter().map(|(_, name)| *name).collect();
    let (sets, missing) = global_sets(&names);
    println!("global emotes loaded: {}", sets.len());
    let missing: Vec<String> = KNOWN
        .iter()
        .filter(|(_, name)| missing.iter().any(|gone| gone == name))
        .map(|(provider, name)| format!("{provider} ({name})"))
        .collect();
    assert!(
        missing.is_empty(),
        "no answer, or no known emote, from: {missing:?}"
    );
}

/// The kinds a pane reads off the same answers: BetterTTV's zero-width
/// `cvMask` and its `w!`, FrankerFaceZ's `ffzX`, and none of the effects
/// FrankerFaceZ gives only its supporters.
#[test]
#[ignore = "asks FFZ, BTTV and 7TV over the network; run by hand"]
fn global_emotes_keep_their_kinds() {
    let (sets, missing) = global_sets(&["cvMask", "w!", "ffzX"]);
    assert!(missing.is_empty(), "never loaded: {missing:?}");
    let kind = |name: &str| sets.lookup(name).map(|named| named.kind);
    assert_eq!(kind("cvMask"), Some(Kind::Overlay));
    assert_eq!(kind("w!"), Some(Kind::EffectOnNext));
    assert_eq!(kind("ffzX"), Some(Kind::EffectOnPrevious));
    assert_eq!(
        kind("ffzHyper"),
        None,
        "FrankerFaceZ's supporters' effects are in everyone's set"
    );
    let overlays: Vec<&str> = SEVENTV_ZERO_WIDTH
        .into_iter()
        .filter(|name| kind(name) == Some(Kind::Overlay))
        .collect();
    println!("7TV zero-width globals still there: {overlays:?}");
}

/// The effects Perch draws, as the providers serve them on 14 October 2026:
/// BetterTTV's nine, written before an emote, and FrankerFaceZ's four in its
/// default sets, written after one. A name gone, or marked as something
/// else, would be drawn as a picture or a word of its own.
#[test]
#[ignore = "asks FFZ, BTTV and 7TV over the network; run by hand"]
fn every_effect_drawn_is_still_served() {
    let bttv = ["w!", "h!", "v!", "z!", "c!", "l!", "r!", "p!", "s!"];
    let ffz = ["ffzW", "ffzX", "ffzY", "ffzCursed"];
    let wanted: Vec<&str> = bttv.iter().chain(&ffz).copied().collect();
    let (sets, missing) = global_sets(&wanted);
    assert!(missing.is_empty(), "never loaded: {missing:?}");
    for (names, kind) in [
        (&bttv[..], Kind::EffectOnNext),
        (&ffz[..], Kind::EffectOnPrevious),
    ] {
        for name in names {
            assert_eq!(
                sets.lookup(name).map(|named| named.kind),
                Some(kind),
                "{name}"
            );
            assert!(Effect::named(name).is_some(), "{name} is not drawn");
        }
    }
}
