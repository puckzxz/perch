//! Asks the three third-party emote providers for their global sets, once,
//! the way a pane does: FrankerFaceZ, BetterTTV and 7TV. None of them is a
//! contract, and a provider that stops answering only leaves its emotes as
//! words in chat, which nobody reports. Ignored, and never run in CI. Run by
//! hand before a release, through `scripts/live-check.sh`, or
//!
//! ```text
//! cargo test -p emotes --test live -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use emotes::EmoteLoader;

/// One global emote each provider has carried for years, by name.
const KNOWN: [(&str, &str); 3] = [
    ("FrankerFaceZ", "LilZ"),
    ("BetterTTV", "SourPls"),
    ("7TV", "EZ"),
];

#[test]
#[ignore = "asks FFZ, BTTV and 7TV over the network; run by hand"]
fn every_provider_still_answers() {
    let (loader, _ready) = EmoteLoader::new();
    loader.load_global();
    let deadline = Instant::now() + Duration::from_secs(20);
    let missing = || {
        let sets = loader.sets();
        KNOWN
            .iter()
            .filter(|(_, name)| sets.lookup(name).is_none())
            .map(|(provider, name)| format!("{provider} ({name})"))
            .collect::<Vec<_>>()
    };
    while !missing().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("global emotes loaded: {}", loader.sets().len());
    let missing = missing();
    assert!(
        missing.is_empty(),
        "no answer, or no known emote, from: {missing:?}"
    );
}
