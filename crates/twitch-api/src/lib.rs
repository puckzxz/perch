//! The slice of the Twitch Helix API this app needs: signing in, and finding
//! out who you follow that is live.
//!
//! Sign-in uses the **device code flow**, which is the right one for a desktop
//! app: it needs no redirect URI, no local web server, and no client secret —
//! the user is shown a short code to type at twitch.tv/activate while the app
//! polls. Nothing secret is ever embedded in the binary.
//!
//! Blocking calls throughout, to be driven from a worker thread like the rest
//! of the app. No UI types appear here.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;

const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const HELIX: &str = "https://api.twitch.tv/helix";

/// Reading your follows is all this app asks for.
pub const SCOPES: &str = "user:read:follows";

/// Refresh this long before expiry rather than waiting for a 401.
const REFRESH_MARGIN: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no client id configured. Create an application at dev.twitch.tv and paste its Client ID into settings.")]
    NoClientId,
    #[error("not signed in")]
    NotSignedIn,
    #[error("network error: {0}")]
    Network(String),
    #[error("Twitch rejected the request: {0}")]
    Api(String),
    #[error("unexpected response from Twitch: {0}")]
    Shape(String),
    /// The user has not finished entering the code yet. Keep polling.
    #[error("authorization pending")]
    Pending,
    /// The user took too long; start a new device flow.
    #[error("the sign-in code expired")]
    Expired,
}

// ── Sign-in ──────────────────────────────────────────────────────────

/// What to show the user while they authorise the app.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    /// The short code the user types, e.g. `ABCD1234`.
    pub user_code: String,
    /// Where they type it, normally `https://www.twitch.tv/activate`.
    pub verification_uri: String,
    /// Seconds between polls, as dictated by Twitch.
    pub interval: u64,
    pub expires_in: u64,
}

/// Tokens plus the identity they belong to.
#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub user_id: String,
    pub login: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

/// Read a JSON body even when the status is an error, since Twitch puts the
/// useful part (`authorization_pending` and friends) in the body.
fn read_body(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, Error> {
    let mut response = match result {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(_)) => {
            return Err(Error::Api("request failed with an error status".into()))
        }
        Err(e) => return Err(Error::Network(e.to_string())),
    };
    response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| Error::Shape(e.to_string()))
}

/// Begin sign-in. Show the returned code and URL, then poll [`poll_token`].
pub fn start_device_flow(client_id: &str) -> Result<DeviceCode, Error> {
    if client_id.is_empty() {
        return Err(Error::NoClientId);
    }
    let mut response = agent()
        .post(DEVICE_URL)
        .send_form([("client_id", client_id), ("scopes", SCOPES)])
        .map_err(|e| Error::Network(e.to_string()))?;

    let json: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| Error::Shape(e.to_string()))?;

    serde_json::from_value(json.clone())
        .map_err(|_| Error::Shape(format!("device response was {json}")))
}

/// One poll of the token endpoint.
///
/// Returns [`Error::Pending`] while the user has not authorised yet, which is
/// the normal case for the first several calls.
pub fn poll_token(client_id: &str, device_code: &str) -> Result<Session, Error> {
    let json = read_body(agent().post(TOKEN_URL).send_form([
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ]))?;

    if let Some(message) = json.get("message").and_then(Value::as_str) {
        return Err(match message {
            m if m.contains("authorization_pending") => Error::Pending,
            m if m.contains("expired") => Error::Expired,
            other => Error::Api(other.to_string()),
        });
    }
    session_from_token_response(client_id, &json)
}

/// Exchange a refresh token for a new pair.
///
/// Twitch refresh tokens are single use: the old one dies the moment this
/// succeeds, so the caller must persist the result immediately.
pub fn refresh(client_id: &str, refresh_token: &str) -> Result<Session, Error> {
    let json = read_body(agent().post(TOKEN_URL).send_form([
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]))?;
    session_from_token_response(client_id, &json)
}

fn session_from_token_response(client_id: &str, json: &Value) -> Result<Session, Error> {
    let access_token = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Shape(format!("no access_token in {json}")))?
        .to_string();
    let refresh_token = json
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let expires_in = json.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);

    let user = current_user(client_id, &access_token)?;
    Ok(Session {
        access_token,
        refresh_token,
        expires_at: now_secs() + expires_in,
        user_id: user.0,
        login: user.1,
    })
}

/// True when the token is expired or close enough that it should be renewed.
pub fn needs_refresh(expires_at: u64) -> bool {
    now_secs() + REFRESH_MARGIN.as_secs() >= expires_at
}

// ── Helix ────────────────────────────────────────────────────────────

fn helix_get(client_id: &str, token: &str, path: &str) -> Result<Value, Error> {
    let mut response = agent()
        .get(&format!("{HELIX}{path}"))
        .header("Client-Id", client_id)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => Error::NotSignedIn,
            ureq::Error::StatusCode(code) => Error::Api(format!("HTTP {code}")),
            other => Error::Network(other.to_string()),
        })?;

    response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| Error::Shape(e.to_string()))
}

/// `(user_id, login)` for the token's owner.
fn current_user(client_id: &str, token: &str) -> Result<(String, String), Error> {
    let json = helix_get(client_id, token, "/users")?;
    let user = json
        .get("data")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .ok_or_else(|| Error::Shape("no user in /users response".into()))?;

    let id = user
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Shape("user had no id".into()))?;
    let login = user.get("login").and_then(Value::as_str).unwrap_or(id);
    Ok((id.to_string(), login.to_string()))
}

/// A followed channel that is currently broadcasting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStream {
    pub user_login: String,
    pub display_name: String,
    pub title: String,
    pub game_name: String,
    pub viewer_count: u64,
    /// Template with `{width}` and `{height}` placeholders; use [`thumbnail`].
    pub thumbnail_url: String,
    pub started_at: String,
}

/// Fill in a thumbnail template.
///
/// Twitch returns the URL with literal `{width}`/`{height}` placeholders; using
/// it unmodified yields a 404.
pub fn thumbnail(template: &str, width: u32, height: u32) -> String {
    // The %-prefixed forms must go first: replacing the bare `{width}` first
    // would turn `%{width}` into `%440` and leave the stray percent behind.
    template
        .replace("%{width}", &width.to_string())
        .replace("%{height}", &height.to_string())
        .replace("{width}", &width.to_string())
        .replace("{height}", &height.to_string())
}

fn parse_streams(json: &Value) -> Vec<LiveStream> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let user_login = entry.get("user_login").and_then(Value::as_str)?;
                    Some(LiveStream {
                        user_login: user_login.to_string(),
                        display_name: entry
                            .get("user_name")
                            .and_then(Value::as_str)
                            .unwrap_or(user_login)
                            .to_string(),
                        title: entry
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        game_name: entry
                            .get("game_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        viewer_count: entry
                            .get("viewer_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        thumbnail_url: entry
                            .get("thumbnail_url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        started_at: entry
                            .get("started_at")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Live channels the signed-in user follows, most viewers first.
pub fn followed_streams(
    client_id: &str,
    token: &str,
    user_id: &str,
) -> Result<Vec<LiveStream>, Error> {
    let json = helix_get(
        client_id,
        token,
        &format!("/streams/followed?user_id={user_id}&first=100"),
    )?;
    let mut streams = parse_streams(&json);
    streams.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    Ok(streams)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_thumbnail_placeholders() {
        let template = "https://cdn.test/live_user_x-{width}x{height}.jpg";
        assert_eq!(
            thumbnail(template, 440, 248),
            "https://cdn.test/live_user_x-440x248.jpg"
        );
    }

    /// Twitch has served both `{width}` and `%{width}` over the years.
    #[test]
    fn handles_the_percent_prefixed_placeholder_variant() {
        let template = "https://cdn.test/x-%{width}x%{height}.jpg";
        assert_eq!(thumbnail(template, 100, 50), "https://cdn.test/x-100x50.jpg");
    }

    #[test]
    fn parses_a_followed_streams_payload() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                {"user_login":"alice","user_name":"Alice","title":"hi",
                 "game_name":"Chess","viewer_count":12,
                 "thumbnail_url":"https://cdn.test/a-{width}x{height}.jpg",
                 "started_at":"2026-08-25T10:00:00Z"}
            ]}"#,
        )
        .unwrap();

        let streams = parse_streams(&json);
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].user_login, "alice");
        assert_eq!(streams[0].display_name, "Alice");
        assert_eq!(streams[0].viewer_count, 12);
    }

    #[test]
    fn tolerates_missing_optional_fields() {
        let json: Value = serde_json::from_str(r#"{"data":[{"user_login":"bob"}]}"#).unwrap();
        let streams = parse_streams(&json);
        assert_eq!(streams.len(), 1);
        // Display name falls back to the login rather than being blank.
        assert_eq!(streams[0].display_name, "bob");
        assert_eq!(streams[0].viewer_count, 0);
    }

    #[test]
    fn skips_entries_without_a_login() {
        let json: Value = serde_json::from_str(r#"{"data":[{"title":"orphan"}]}"#).unwrap();
        assert!(parse_streams(&json).is_empty());
    }

    #[test]
    fn empty_payload_is_not_an_error() {
        let json: Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(parse_streams(&json).is_empty());
        let missing: Value = serde_json::from_str("{}").unwrap();
        assert!(parse_streams(&missing).is_empty());
    }

    #[test]
    fn refresh_margin_triggers_before_expiry() {
        assert!(needs_refresh(now_secs() + 60));
        assert!(!needs_refresh(now_secs() + 3600));
        // Already expired.
        assert!(needs_refresh(0));
    }
}
