//! Asks, once each, the two services chat rides without a contract: Twitch's
//! unpublished `VideoCommentsByOffsetOrCursor` query on the mobile Client-ID,
//! behind a recording's chat replay, and the recent-messages service behind
//! a live pane's backfill.
//!
//! Both fail quietly in the app by design: the replay says one notice row and
//! the video plays on, and a backfill that fails opens the pane empty
//! (HANDOFF, Known limits 11 and 17). Ignored, and never run in CI. Run by
//! hand before a release, through `scripts/live-check.sh`, or
//!
//! ```text
//! cargo test -p twitch-chat --test live -- --ignored --nocapture
//! ```
//!
//! `PERCH_LIVE_VIDEO` names a recording to replay; without it one is found
//! from a big channel's newest past broadcast, anonymously.

use serde_json::{json, Value};

/// Big channels that keep their past broadcasts.
const CHANNELS: [&str; 4] = ["xqc", "shroud", "summit1g", "lirik"];

/// The website's Client-ID, which the raw users query below answers on
/// anonymously, as `twitch_api::recommend` asks its own.
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// The newest past broadcast of `login`, by id, if it keeps any; or why the
/// query that asks could not say, in its own words.
fn newest_archive(login: &str) -> Result<Option<String>, String> {
    let query = format!(
        r#"query {{ user(login: "{login}") {{ videos(first: 1, type: ARCHIVE, sort: TIME) {{ edges {{ node {{ id }} }} }} }} }}"#
    );
    let mut response = ureq::post("https://gql.twitch.tv/gql")
        .header("Client-ID", WEB_CLIENT_ID)
        .send_json(json!({ "query": query }))
        .map_err(|e| e.to_string())?;
    let body: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("unreadable answer: {e}"))?;
    if let Some(errors) = body.get("errors") {
        return Err(errors.to_string());
    }
    Ok(body
        .pointer("/data/user/videos/edges/0/node/id")
        .and_then(Value::as_str)
        .map(str::to_string))
}

#[test]
#[ignore = "asks Twitch over the network; run by hand"]
fn chat_replay_still_answers() {
    // Only a helper: the query this check is for is the replay below, which
    // a recording named by hand reaches without it. So a refusal here is
    // said beside the advice rather than ending the check by itself.
    let mut refused = Vec::new();
    let video = std::env::var("PERCH_LIVE_VIDEO")
        .ok()
        .or_else(|| {
            CHANNELS
                .iter()
                .find_map(|login| match newest_archive(login) {
                    Ok(found) => found,
                    Err(e) => {
                        refused.push(format!("{login}: {e}"));
                        None
                    }
                })
        })
        .unwrap_or_else(|| {
            if refused.is_empty() {
                panic!("no recording to ask about: set PERCH_LIVE_VIDEO");
            }
            panic!(
                "no recording to ask about, the lookup was refused ({}): set PERCH_LIVE_VIDEO",
                refused.join("; ")
            )
        });
    // A minute in, where any broadcast has had something said.
    let page = twitch_chat::replay::page_at(&video, 60)
        .unwrap_or_else(|e| panic!("the replay query was refused for video {video}: {e:?}"));
    println!(
        "replay of {video}: {} comments, more after: {}",
        page.comments.len(),
        page.next.is_some()
    );
}

#[test]
#[ignore = "asks the recent-messages service over the network; run by hand"]
fn recent_messages_still_answer() {
    // The service joins a channel the first time anybody asks, so an empty
    // answer is still an answer; a refusal is the failure.
    for login in CHANNELS {
        let lines = twitch_chat::history::recent(login, 20)
            .unwrap_or_else(|e| panic!("recent-messages refused {login}: {e}"));
        println!("recent messages for {login}: {}", lines.len());
    }
}
