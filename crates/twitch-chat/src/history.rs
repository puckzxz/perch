//! The messages a channel sent before we joined.
//!
//! Twitch offers no way to read them. IRC gives you what arrives after your
//! JOIN and nothing before it, and there is no Helix endpoint for scrollback —
//! the website's own history is rendered server-side and not exposed. Every
//! client that shows history solves it the same way, with a community service
//! that idles in the channels people ask about and keeps the last day of lines.
//! Chatterino and DankChat both use the one below; this is that same request.
//!
//! It answers with **raw IRC**, which is the whole reason this is cheap: the
//! lines go straight through [`crate::message::parse_line`] exactly as if they
//! had come off the socket, so a backfilled message renders identically to a
//! live one and there is no second code path to keep in step. They arrive with
//! their original `tmi-sent-ts`, so the timestamps are real rather than the
//! moment we fetched them.
//!
//! Two things to be aware of. This is the only place the app asks a
//! **third party** for content, and doing so tells that service which channels
//! are being watched — `Settings::chat_history` exists to turn it off. And the
//! service joins a channel the first time anybody asks for it, so the very
//! first request for a channel nobody watches comes back empty and the one
//! after it does not.

use std::time::Duration;

use serde_json::Value;

use crate::message::{self, IrcMessage};

const ENDPOINT: &str = "https://recent-messages.robotty.de/api/v2/recent-messages";

/// The service's own ceiling. Asking for more is not an error, it is silently
/// capped, so clamping here keeps the request honest about what it will get.
const MAX_LIMIT: usize = 800;

/// Short on purpose. This runs before the IRC connect, so every second spent
/// here is a second the pane sits empty — and an empty backfill is a much
/// smaller loss than a slow join.
const TIMEOUT: Duration = Duration::from_secs(8);

/// A free service run by one person. Say who is calling.
///
/// The version is this crate's, which is the workspace's, which is the one
/// `perch` is released under — they are one number, inherited, precisely so
/// that this string cannot go stale. It said `perch/0.1.0` through two releases
/// when they were separate. `perch` has a test holding it to that.
pub const USER_AGENT: &str = concat!("perch/", env!("CARGO_PKG_VERSION"));

/// Whether `channel` could be a Twitch login at all.
///
/// Logins are lowercase ASCII, digits and underscores, up to 25 characters. The
/// service rejects anything else with a 400, and a rejection costs a round trip
/// we can see coming.
fn is_login(channel: &str) -> bool {
    !channel.is_empty()
        && channel.len() <= 25
        && channel
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Up to `limit` recent lines for `channel`, oldest first, already parsed.
///
/// Best-effort: an empty list is a normal outcome, not a failure, and the
/// caller is expected to carry on either way.
pub fn recent(channel: &str, limit: usize) -> Result<Vec<IrcMessage>, String> {
    if !is_login(channel) {
        return Err(format!("`{channel}` is not a channel login"));
    }
    let limit = limit.min(MAX_LIMIT);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // The reason for a refusal is in the body, not the status line — the
        // same trap `twitch-api` documents. Letting the status short-circuit
        // the read would turn every explained failure into `HTTP 400`.
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();

    let mut response = agent
        .get(format!("{ENDPOINT}/{channel}?limit={limit}"))
        .call()
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let json: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("unreadable response: {e}"))?;

    // The service reports a channel it has not joined yet as a 200 with an
    // `error` beside an empty list, so the status alone does not settle it.
    if status >= 400 {
        return Err(json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the history service refused the request")
            .to_string());
    }

    let lines = json
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("the history service answered in an unexpected shape")?;

    Ok(lines
        .iter()
        .filter_map(Value::as_str)
        .filter_map(message::parse_line)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_logins_are_worth_a_round_trip() {
        assert!(is_login("theburntpeanut"));
        assert!(is_login("some_one_99"));
        // Twitch caps a login at 25 characters; the service rejects longer.
        assert!(!is_login(&"a".repeat(26)));
        assert!(!is_login(""));
        // A channel is stored lowercase everywhere in the app, so an uppercase
        // one here means a bug upstream rather than a name to try.
        assert!(!is_login("TheBurntPeanut"));
        assert!(!is_login("has space"));
        // The one that matters: nothing can escape into the path.
        assert!(!is_login("../../etc/passwd"));
        assert!(!is_login("a?limit=1"));
    }

    /// The service adds `historical=1` and `rm-received-ts` and otherwise hands
    /// back the line Twitch sent. Proof that it needs no special parser.
    #[test]
    fn a_historical_line_parses_like_any_other() {
        let line = r"@historical=1;rm-received-ts=1787746330000;display-name=Valaakai;color=#B22222;tmi-sent-ts=1787746329844 :valaakai!valaakai@valaakai.tmi.twitch.tv PRIVMSG #theburntpeanut :!peanuts";
        let irc = message::parse_line(line).unwrap();
        let chat = crate::ChatMessage::from_irc(&irc).unwrap();
        assert_eq!(chat.display_name, "Valaakai");
        assert_eq!(chat.text, "!peanuts");
        assert_eq!(chat.sent_at, Some(1787746329844));
    }
}
