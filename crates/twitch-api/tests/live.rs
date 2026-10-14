//! Asks Twitch, once each, the questions perch asks without a contract: the
//! unpublished `SideNav` query behind the rail's Recommended group, and the
//! raw GQL behind "Live 3 hours ago" and Shared Chat's partner names.
//!
//! These fail quietly in the app by design (HANDOFF, Known limits 17 and 18),
//! so a retired hash can go unnoticed for weeks. Ignored, and never run in CI:
//! anonymous GQL from a shared runner's address invites blocks and flaky
//! failures. Run by hand before a release, through `scripts/live-check.sh`, or
//!
//! ```text
//! cargo test -p twitch-api --test live -- --ignored --nocapture
//! ```
//!
//! No token is sent: every query here is anonymous, on the website's own
//! Client-ID, as the app asks them.

use twitch_api::recommend;

/// Channels big enough that Twitch knows them whatever the hour. Offline
/// seeds answer too, which is how the rail asks about followed channels that
/// are off.
const SEEDS: [&str; 3] = ["xqc", "shroud", "summit1g"];

#[test]
#[ignore = "asks Twitch over the network; run by hand"]
fn side_nav_still_answers() {
    let mut answered = 0;
    for seed in SEEDS {
        match recommend::similar_channels(seed) {
            Ok(channels) => {
                println!(
                    "SideNav for {seed}: {} live similar channels",
                    channels.len()
                );
                answered += 1;
            }
            Err(e) => println!("SideNav for {seed}: {e}"),
        }
    }
    assert!(
        answered > 0,
        "SideNav refused every seed: the hash may be retired"
    );
}

#[test]
#[ignore = "asks Twitch over the network; run by hand"]
fn last_broadcasts_still_answer() {
    let logins: Vec<String> = SEEDS.iter().map(|s| s.to_string()).collect();
    let answer = recommend::last_broadcasts(&logins).expect("the users query was refused");
    println!("last broadcasts: {answer:?}");
    assert!(!answer.is_empty(), "no channel's last broadcast came back");
}

#[test]
#[ignore = "asks Twitch over the network; run by hand"]
fn channel_names_still_answer() {
    // xQc's numeric id, as a Shared Chat line would name its room.
    let names = recommend::channel_names(&["71092938".to_string()])
        .expect("the users-by-id query was refused");
    println!("channel names: {names:?}");
    assert!(
        names.iter().any(|channel| channel.login == "xqc"),
        "the id did not come back as its login"
    );
}
