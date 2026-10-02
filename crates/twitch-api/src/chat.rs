//! Sending a chat message, and finding out whether the sign-in may.
//!
//! Reading chat is anonymous IRC (the `twitch-chat` crate) and stays that
//! way: a message sent from here comes back to the pane the way anybody
//! else's does, when IRC echoes it. Sending is Helix's Send Chat Message
//! (`POST /chat/messages`), which takes the channel's numeric id
//! (`broadcaster_id`, the `room-id` a chat's `ROOMSTATE` carries), the
//! signed-in user's id (`sender_id`, which must be the token's) and the
//! words, and needs the `user:write:chat` scope ([`CHAT_SCOPE`]). Twitch
//! can answer 200 and still not send the message: `is_sent` false, with a
//! `drop_reason` saying why in a sentence, which is the case for most of
//! the room's own rules (sub-only, followers-only, emote-only, a message
//! held for a moderator). What a send came to is [`SendOutcome`].
//!
//! A sign-in from before the scope was asked for still reads everything
//! else, so the scope is looked up rather than assumed: Twitch's token
//! validation (`GET id.twitch.tv/oauth2/validate`) answers with the scopes
//! the token carries ([`token_scopes`]), and Twitch asks every app to
//! validate its token at startup and hourly besides.
//!
//! Helix, like the rest of this crate's root, and kept in its own file only
//! because the root is long enough already.

use serde_json::{json, Value};

use super::{agent, answered, body_message, entries, read_body, text, Error, HELIX};

/// What sending a chat message needs of the sign-in. Asked for at sign-in
/// beside reading the follows (`SCOPES`); a token from before it was asked
/// for lacks it, and sends nothing until the user signs in again.
pub const CHAT_SCOPE: &str = "user:write:chat";

/// Where Twitch says what a token is and what it may do.
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";

/// The scopes `token` carries, from Twitch's token validation. A token
/// Twitch no longer honours is [`Error::NotSignedIn`].
pub fn token_scopes(token: &str) -> Result<Vec<String>, Error> {
    let (status, json) = read_body(
        agent()
            .get(VALIDATE_URL)
            .header("Authorization", &format!("OAuth {token}"))
            .call(),
    )?;
    parse_scopes(status, &json)
}

/// Whether a token carrying `scopes` may send chat.
pub fn may_chat(scopes: &[String]) -> bool {
    scopes.iter().any(|scope| scope == CHAT_SCOPE)
}

/// What a validation answer below 500 comes to.
fn parse_scopes(status: u16, json: &Value) -> Result<Vec<String>, Error> {
    match status {
        200..=299 => Ok(json
            .get("scopes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()),
        401 => Err(Error::NotSignedIn),
        other => Err(Error::Api(
            body_message(json)
                .map(|m| format!("{m} (HTTP {other})"))
                .unwrap_or_else(|| format!("HTTP {other}")),
        )),
    }
}

/// What a send came to, short of a failure to ask at all (a transport
/// failure or a 5xx is [`Error::Network`], as everywhere in the crate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Twitch took it. It shows in chat when IRC echoes it.
    Sent,
    /// Twitch answered, and did not send it: `is_sent` false. `message` is
    /// Twitch's own sentence for why (`drop_reason.message`), `code` its
    /// name for it (`drop_reason.code`); either may be empty.
    Dropped { code: String, message: String },
    /// The token lacks [`CHAT_SCOPE`]: a sign-in from before it was asked
    /// for. Signing in again is the only way on.
    MissingScope,
    /// Any other refusal: 403 when the sender may not chat in the room at
    /// all, 422 for a message too long, 429 for too many too fast, 400 for
    /// a request Twitch could not read. `message` is Twitch's wording, when
    /// it gave any.
    Refused {
        status: u16,
        message: Option<String>,
    },
}

/// Send `message` to the chat of the channel whose numeric id is
/// `broadcaster_id`, as `sender_id`, who must be the token's own user.
pub fn send_message(
    client_id: &str,
    token: &str,
    broadcaster_id: &str,
    sender_id: &str,
    message: &str,
) -> Result<SendOutcome, Error> {
    let (status, mut response) = answered(
        agent()
            .post(&format!("{HELIX}/chat/messages"))
            .header("Client-Id", client_id)
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(json!({
                "broadcaster_id": broadcaster_id,
                "sender_id": sender_id,
                "message": message,
            })),
    )?;
    // Read leniently: a refusal's body is a courtesy, and a 429 from an edge
    // may not be JSON at all. What the status says is answer enough then.
    let json = response.body_mut().read_json::<Value>().ok();
    parse_send(status, json.as_ref())
}

/// What a Send Chat Message answer below 500 comes to.
///
/// A 401 is two different things. Twitch says which in the body: a token
/// without the scope ("Missing scope: user:write:chat") is
/// [`SendOutcome::MissingScope`], which signing in again fixes; any other
/// 401 is a token Twitch no longer honours, [`Error::NotSignedIn`].
fn parse_send(status: u16, json: Option<&Value>) -> Result<SendOutcome, Error> {
    let message = json.and_then(body_message).map(str::to_string);
    match status {
        200..=299 => {
            let entry = json
                .and_then(|json| entries(json).next())
                .ok_or_else(|| Error::Shape("send answer had no data".into()))?;
            if entry.get("is_sent").and_then(Value::as_bool) == Some(true) {
                return Ok(SendOutcome::Sent);
            }
            let reason = entry.get("drop_reason");
            let field = |key| {
                reason
                    .and_then(|reason| text(reason, key))
                    .unwrap_or_default()
                    .to_string()
            };
            Ok(SendOutcome::Dropped {
                code: field("code"),
                message: field("message"),
            })
        }
        401 if message
            .as_deref()
            .is_some_and(|m| m.to_ascii_lowercase().contains("scope")) =>
        {
            Ok(SendOutcome::MissingScope)
        }
        401 => Err(Error::NotSignedIn),
        status => Ok(SendOutcome::Refused { status, message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validation answers with the scopes, and whether one of them is
    /// sending chat is what the composer asks.
    #[test]
    fn validation_reads_the_scopes() {
        let body = json!({
            "client_id": "abc", "login": "someone", "user_id": "141981764",
            "scopes": ["user:read:follows", "user:write:chat"], "expires_in": 5520838
        });
        let scopes = parse_scopes(200, &body).unwrap();
        assert_eq!(scopes, ["user:read:follows", "user:write:chat"]);
        assert!(may_chat(&scopes));

        let old = parse_scopes(200, &json!({ "scopes": ["user:read:follows"] })).unwrap();
        assert!(!may_chat(&old), "a sign-in from before the scope");
        assert!(parse_scopes(200, &json!({})).unwrap().is_empty());

        let refused = json!({ "status": 401, "message": "invalid access token" });
        assert!(matches!(
            parse_scopes(401, &refused),
            Err(Error::NotSignedIn)
        ));
        assert!(matches!(parse_scopes(400, &refused), Err(Error::Api(_))));
    }

    /// Sign-in asks for the scope a send needs, space-separated as OAuth
    /// writes a list of them.
    #[test]
    fn sign_in_asks_for_sending() {
        assert!(crate::SCOPES.split(' ').any(|scope| scope == CHAT_SCOPE));
        assert!(crate::SCOPES
            .split(' ')
            .any(|scope| scope == "user:read:follows"));
    }

    #[test]
    fn a_sent_message_is_sent() {
        let body = json!({ "data": [ { "message_id": "abc-123-def", "is_sent": true } ] });
        assert_eq!(parse_send(200, Some(&body)).unwrap(), SendOutcome::Sent);
    }

    /// Twitch's 200 with `is_sent` false carries its own reason, which is
    /// what the chat says.
    #[test]
    fn a_dropped_message_says_why() {
        let body = json!({ "data": [ {
            "message_id": "",
            "is_sent": false,
            "drop_reason": {
                "code": "msg_subsonly",
                "message": "This room is in subscribers-only mode."
            }
        } ] });
        assert_eq!(
            parse_send(200, Some(&body)).unwrap(),
            SendOutcome::Dropped {
                code: "msg_subsonly".into(),
                message: "This room is in subscribers-only mode.".into(),
            }
        );

        // No reason at all is still a drop, with nothing to quote.
        let bare = json!({ "data": [ { "is_sent": false } ] });
        assert_eq!(
            parse_send(200, Some(&bare)).unwrap(),
            SendOutcome::Dropped {
                code: String::new(),
                message: String::new(),
            }
        );

        assert!(matches!(
            parse_send(200, Some(&json!({ "data": [] }))),
            Err(Error::Shape(_))
        ));
        assert!(matches!(parse_send(200, None), Err(Error::Shape(_))));
    }

    /// A 401 for the scope is the one refusal signing in again fixes; any
    /// other 401 is a token Twitch no longer honours.
    #[test]
    fn a_missing_scope_is_told_from_a_dead_token() {
        let scope = json!({
            "error": "Unauthorized", "status": 401,
            "message": "Missing scope: user:write:chat"
        });
        assert_eq!(
            parse_send(401, Some(&scope)).unwrap(),
            SendOutcome::MissingScope
        );
        let dead = json!({ "status": 401, "message": "Invalid OAuth token" });
        assert!(matches!(
            parse_send(401, Some(&dead)),
            Err(Error::NotSignedIn)
        ));
        assert!(matches!(parse_send(401, None), Err(Error::NotSignedIn)));
    }

    #[test]
    fn other_refusals_keep_their_status_and_wording() {
        let banned = json!({ "status": 403, "message": "You are banned" });
        assert_eq!(
            parse_send(403, Some(&banned)).unwrap(),
            SendOutcome::Refused {
                status: 403,
                message: Some("You are banned".into()),
            }
        );
        assert_eq!(
            parse_send(429, None).unwrap(),
            SendOutcome::Refused {
                status: 429,
                message: None,
            }
        );
    }
}
