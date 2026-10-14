//! Runs the streamlink on this machine against a live channel, once, and
//! checks that every rendition it names is one perch can read: a resolution
//! `quality::parse_quality` takes, or a name perch deliberately leaves out of
//! its menu (`best`, `worst`, `audio_only`, a `_portrait` rendition).
//! streamlink renamed its renditions in 8.6.0 (PR 7087: a recording's audio
//! became `audio_only`, and portrait streams gained `_portrait` names), and a
//! name nothing reads is a rendition that silently never plays. Ignored, and
//! never run in CI. Run by hand before a release, through
//! `scripts/live-check.sh`, or
//!
//! ```text
//! cargo test -p streamlink --test live -- --ignored --nocapture
//! ```
//!
//! `PERCH_LIVE_CHANNEL` names the channel; without it, the first of the
//! channels at the top of Twitch's directory right now, asked anonymously, so
//! the check does not depend on who happens to be live at the hour it is run.
//! `STREAMLINK_PATH` names the binary, as in the app.

use std::process::Command;

use serde_json::Value;
use streamlink::quality;

/// The website's Client-ID, which the directory query below answers on
/// anonymously, as `twitch_api::recommend` asks its own.
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// The logins at the top of Twitch's live directory, most watched first.
fn live_now() -> Vec<String> {
    let query = "query { streams(first: 5) { edges { node { broadcaster { login } } } } }";
    let Ok(mut response) = ureq::post("https://gql.twitch.tv/gql")
        .header("Client-ID", WEB_CLIENT_ID)
        .send_json(serde_json::json!({ "query": query }))
    else {
        return Vec::new();
    };
    let Ok(body) = response.body_mut().read_json::<Value>() else {
        return Vec::new();
    };
    body.pointer("/data/streams/edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|edge| edge.pointer("/node/broadcaster/login")?.as_str())
        .map(str::to_string)
        .collect()
}

/// What `streamlink --json` names for `channel`, or `None` when it is off.
fn renditions(channel: &str) -> Option<Vec<String>> {
    let binary = std::env::var("STREAMLINK_PATH").unwrap_or_else(|_| "streamlink".into());
    let output = Command::new(binary)
        .args(["--json", "--twitch-supported-codecs", "h264,h265,av1"])
        .arg(format!("twitch.tv/{channel}"))
        .output()
        .expect("streamlink did not run: is it on PATH, or STREAMLINK_PATH set?");
    let json: Value = serde_json::from_slice(&output.stdout).ok()?;
    let streams = json.get("streams")?.as_object()?;
    Some(streams.keys().cloned().collect())
}

#[test]
#[ignore = "runs streamlink against Twitch; run by hand"]
fn every_rendition_streamlink_names_is_understood() {
    let channels: Vec<String> = match std::env::var("PERCH_LIVE_CHANNEL") {
        Ok(channel) => vec![channel],
        Err(_) => live_now(),
    };
    let (channel, names) = channels
        .iter()
        .find_map(|channel| renditions(channel).map(|names| (channel, names)))
        .expect("no live channel answered streamlink: set PERCH_LIVE_CHANNEL");
    println!("{channel}: {names:?}");
    let left_out =
        |name: &str| matches!(name, "best" | "worst" | "audio_only") || name.ends_with("_portrait");
    let unread: Vec<&String> = names
        .iter()
        .filter(|name| !left_out(name) && quality::parse_quality(name).is_none())
        .collect();
    assert!(
        unread.is_empty(),
        "renditions perch cannot read: {unread:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| quality::parse_quality(name).is_some()),
        "no rendition perch can play"
    );
}
