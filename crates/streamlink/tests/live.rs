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
//!
//! A failure is said in the words of whatever failed: streamlink's own error,
//! or stderr when it printed nothing, and the directory query's refusal apart
//! from a directory with nobody in it. Twitch retiring a query under
//! streamlink, as in November 2025, is what this exists to catch, and read as
//! "nobody is live" it would send whoever runs it after a channel name.

use std::process::Command;

use serde_json::Value;
use streamlink::quality;

/// The website's Client-ID, which the directory query below answers on
/// anonymously, as `twitch_api::recommend` asks its own.
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// The logins at the top of Twitch's live directory, most watched first, or
/// why the directory would not say.
fn live_now() -> Result<Vec<String>, String> {
    let query = "query { streams(first: 5) { edges { node { broadcaster { login } } } } }";
    let mut response = ureq::post("https://gql.twitch.tv/gql")
        .header("Client-ID", WEB_CLIENT_ID)
        .send_json(serde_json::json!({ "query": query }))
        .map_err(|e| e.to_string())?;
    let body: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("unreadable answer: {e}"))?;
    if let Some(errors) = body.get("errors") {
        return Err(errors.to_string());
    }
    Ok(body
        .pointer("/data/streams/edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|edge| edge.pointer("/node/broadcaster/login")?.as_str())
        .map(str::to_string)
        .collect())
}

/// What `streamlink --json` names for `channel`; `None` when it is off,
/// which streamlink says as "No playable streams". Anything else it says
/// is the failure, in its words: the JSON's `error`, or stderr when stdout
/// had nothing to read, with how it exited.
fn renditions(channel: &str) -> Result<Option<Vec<String>>, String> {
    let binary = std::env::var("STREAMLINK_PATH").unwrap_or_else(|_| "streamlink".into());
    let output = Command::new(binary)
        .args(["--json", "--twitch-supported-codecs", "h264,h265,av1"])
        .arg(format!("twitch.tv/{channel}"))
        .output()
        .expect("streamlink did not run: is it on PATH, or STREAMLINK_PATH set?");
    let said = || {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let text = if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        format!("{} ({})", text.trim(), output.status)
    };
    let Ok(json) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Err(said());
    };
    if let Some(error) = json.get("error").and_then(Value::as_str) {
        if error.contains("No playable streams") {
            return Ok(None);
        }
        return Err(format!("{error} ({})", output.status));
    }
    match json.get("streams").and_then(Value::as_object) {
        Some(streams) => Ok(Some(streams.keys().cloned().collect())),
        None => Err(said()),
    }
}

#[test]
#[ignore = "runs streamlink against Twitch; run by hand"]
fn every_rendition_streamlink_names_is_understood() {
    let named = std::env::var("PERCH_LIVE_CHANNEL").ok();
    let channels: Vec<String> = match named.clone() {
        Some(channel) => vec![channel],
        None => {
            let listed = live_now().unwrap_or_else(|e| {
                panic!("Twitch's directory query was refused: {e}. Set PERCH_LIVE_CHANNEL")
            });
            assert!(
                !listed.is_empty(),
                "Twitch's directory listed nobody live: set PERCH_LIVE_CHANNEL"
            );
            listed
        }
    };
    // The first channel streamlink finds live. One that is off is passed
    // over; any other answer is the failure this check is for, and stops it.
    let mut found = None;
    for channel in &channels {
        match renditions(channel) {
            Ok(Some(names)) => {
                found = Some((channel, names));
                break;
            }
            Ok(None) => println!("{channel}: off, as streamlink sees it"),
            Err(e) => panic!("streamlink failed on {channel}: {e}"),
        }
    }
    let (channel, names) = found.unwrap_or_else(|| match &named {
        Some(channel) => panic!("PERCH_LIVE_CHANNEL={channel} is off: name a live channel"),
        None => panic!("streamlink found none of {channels:?} live: set PERCH_LIVE_CHANNEL"),
    });
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
