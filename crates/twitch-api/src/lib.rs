//! The slice of the Twitch Helix API this app needs: signing in, finding out
//! who you follow that is live, and browsing what else is on.
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

/// Reading your follows is all this app asks for. Browsing — top streams and
/// categories — needs no scope at all, only a valid token.
pub const SCOPES: &str = "user:read:follows";

/// One request's worth of results. Twitch caps this at 100.
const PAGE_SIZE: &str = "100";
/// Search returns matches in relevance order, and nobody reads past the first
/// screen of a search. A smaller page also keeps the follow-up lookup cheap.
const SEARCH_PAGE_SIZE: &str = "40";
/// How many `user_login` parameters Helix accepts in one `/streams` request.
const MAX_LOGINS_PER_REQUEST: usize = 100;
/// How many pages of follows to walk before giving up.
///
/// Both followed endpoints paginate, and both cap a page at 100. Assuming only
/// `/channels/followed` did meant `/streams/followed` silently returned the top
/// hundred live channels and nothing distinguished that from "a hundred live" —
/// so a channel hovering around rank 100 dropped out and came back on alternate
/// polls, firing a went-live toast each time it reappeared.
///
/// `/channels/followed` is still the one whose `first` defaults to 20 rather
/// than 100, so forgetting the parameter there shows a fifth of the list with
/// no sign anything is missing. Ten pages is a thousand channels, well past
/// what anyone browses.
const MAX_FOLLOW_PAGES: usize = 10;

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
    /// Twitch is asking to be polled less often. Also progress, but unlike
    /// [`Pending`](Error::Pending) it carries an instruction: RFC 8628 §3.5
    /// requires the caller to add five seconds to its interval each time this
    /// arrives. Collapsing it into `Pending` threw that away and left the
    /// client hammering at the rate Twitch had just asked it to reduce, until
    /// the device code expired and a correctly typed one still reported
    /// "the sign-in code expired".
    #[error("polling too fast")]
    SlowDown,
    /// The token exchange succeeded but the identity lookup after it did not.
    ///
    /// Kept apart from [`Network`](Error::Network) because the device code has
    /// already been redeemed: polling again cannot work, so a caller that
    /// retries transport failures must not retry this one. It would spend the
    /// rest of the sign-in window on a code that can never yield a session.
    #[error("signed in, but could not read the account: {0}")]
    IdentityLookup(String),
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
#[derive(Clone)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub user_id: String,
    pub login: String,
}

/// By hand, so a `{:?}` cannot put either token in a log. The identity is
/// what a reader wants from it anyway.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("user_id", &self.user_id)
            .field("login", &self.login)
            .finish()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The one HTTP agent every request here shares.
///
/// One rather than one per call, because ureq caches its TLS configuration -
/// the parsed root store included - per agent. Building a fresh agent per
/// request rebuilt that store and opened a fresh connection every time, for a
/// worker that makes a request a minute for as long as the app is open.
fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            // Twitch puts the meaningful part of a failure in the body, not the
            // status line: while the user has not typed the code yet, the device
            // flow answers HTTP 400 with `authorization_pending`. Letting the
            // status short-circuit the read turns that normal, expected state
            // into a fatal error and sign-in gives up on the very first poll.
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Read a JSON body regardless of status, plus the status itself.
///
/// A server-side failure is reported as [`Error::Network`] rather than being
/// parsed: Twitch's edge answers an outage with an HTML page, and reading that
/// as JSON produced a [`Error::Shape`] that lost the status - so a 502 during
/// sign-in looked like a malformed reply and was treated as terminal, where a
/// transport error would have been retried.
fn read_body(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<(u16, Value), Error> {
    let mut response = result.map_err(|e| Error::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if status >= 500 {
        return Err(Error::Network(format!("Twitch answered HTTP {status}")));
    }
    let json = response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| Error::Shape(format!("HTTP {status} with an unreadable body: {e}")))?;
    Ok((status, json))
}

/// Twitch's error bodies carry the reason under `message`.
fn body_message(json: &Value) -> Option<&str> {
    json.get("message").and_then(Value::as_str)
}

/// Begin sign-in. Show the returned code and URL, then poll [`poll_token`].
pub fn start_device_flow(client_id: &str) -> Result<DeviceCode, Error> {
    if client_id.is_empty() {
        return Err(Error::NoClientId);
    }
    let (status, json) = read_body(
        agent()
            .post(DEVICE_URL)
            .send_form([("client_id", client_id), ("scopes", SCOPES)]),
    )?;

    if status >= 400 {
        return Err(Error::Api(
            body_message(&json)
                .unwrap_or("the device endpoint rejected this client id")
                .to_string(),
        ));
    }

    serde_json::from_value(json)
        .map_err(|e| Error::Shape(format!("device response was missing a field: {e}")))
}

/// One poll of the token endpoint.
///
/// Returns [`Error::Pending`] while the user has not authorised yet, which is
/// the normal case for the first several calls.
pub fn poll_token(client_id: &str, device_code: &str) -> Result<Session, Error> {
    let (status, json) = read_body(agent().post(TOKEN_URL).send_form([
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ]))?;

    if status >= 400 {
        return Err(classify_token_error(
            body_message(&json).unwrap_or("sign-in failed"),
        ));
    }
    session_from_token_response(client_id, &json)
}

/// Map a device-flow error body to an outcome.
///
/// `authorization_pending` is the normal state for every poll before the user
/// finishes typing the code, and `slow_down` means poll less often. Both are
/// progress, not failure; treating them as errors aborts sign-in on the first
/// attempt, which is exactly what used to happen. They stay *separate* outcomes
/// because only one of them asks the caller to change anything.
fn classify_token_error(message: &str) -> Error {
    match message {
        m if m.contains("authorization_pending") => Error::Pending,
        m if m.contains("slow_down") => Error::SlowDown,
        m if m.contains("expired") => Error::Expired,
        other => Error::Api(other.to_string()),
    }
}

/// Exchange a refresh token for a new pair, for a user whose identity is known.
///
/// Twitch refresh tokens are single use: the old one dies the moment this
/// succeeds, so the caller must persist the result immediately.
///
/// The identity is passed in rather than looked up, and that is the whole point
/// of the signature. This used to call [`current_user`] before returning, which
/// put a *second* network request after the point of no return — so a dropped
/// packet during that call returned `Err`, the caller discarded the pair it had
/// just been issued, and the old refresh token was already spent. Two seconds
/// of bad wifi signed the user out permanently and sent them back through the
/// device flow. On a refresh the id and login are already on disk; there is
/// nothing to ask Twitch for.
pub fn refresh(
    client_id: &str,
    refresh_token: &str,
    user_id: &str,
    login: &str,
) -> Result<Session, Error> {
    let (status, json) = read_body(agent().post(TOKEN_URL).send_form([
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]))?;
    if status >= 400 {
        return Err(Error::Api(
            body_message(&json).unwrap_or("refresh failed").to_string(),
        ));
    }
    let tokens = tokens_from_response(&json)?;
    Ok(Session {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: now_secs().saturating_add(tokens.expires_in),
        user_id: user_id.to_string(),
        login: login.to_string(),
    })
}

/// The token half of a token-endpoint response, before any identity is attached.
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

/// Parse tokens out of a token-endpoint response. No network, so it cannot fail
/// after the exchange has already happened.
fn tokens_from_response(json: &Value) -> Result<Tokens, Error> {
    // The body is deliberately not quoted in either error: the field that is
    // present is a token, and an error message is the one string most likely
    // to end up in a log.
    let access_token = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Shape("token response had no access_token".into()))?
        .to_string();
    // Required, not optional. Defaulting this to an empty string persisted a
    // refresh token that cannot work, turning a malformed response into a
    // sign-out one launch later rather than an error now.
    let refresh_token = json
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Shape("token response had no refresh_token".into()))?
        .to_string();
    let expires_in = json
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    Ok(Tokens {
        access_token,
        refresh_token,
        expires_in,
    })
}

/// Build a session from a first-time token response, looking the user up.
///
/// Only sign-in needs this: it is the one case where nothing is known about the
/// user yet, so unlike [`refresh`] there is no way to avoid a second request.
/// The failure is reported as [`Error::IdentityLookup`] rather than passed
/// through, because by this point the device code has been spent — a caller
/// that retries a `Network` error would otherwise re-poll a redeemed code until
/// the sign-in window ran out.
fn session_from_token_response(client_id: &str, json: &Value) -> Result<Session, Error> {
    let tokens = tokens_from_response(json)?;
    let user = current_user(client_id, &tokens.access_token)
        .map_err(|e| Error::IdentityLookup(e.to_string()))?;
    Ok(Session {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: now_secs().saturating_add(tokens.expires_in),
        user_id: user.0,
        login: user.1,
    })
}

/// True when the token is expired or close enough that it should be renewed.
pub fn needs_refresh(expires_at: u64) -> bool {
    now_secs() + REFRESH_MARGIN.as_secs() >= expires_at
}

/// Whether the access token has actually run out, as opposed to being due
/// for renewal. A refresh that could not be *attempted* - the network was
/// down - leaves a session that is still good for this long.
pub fn has_expired(expires_at: u64) -> bool {
    now_secs() >= expires_at
}

// ── Helix ────────────────────────────────────────────────────────────

fn helix_get(
    client_id: &str,
    token: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Value, Error> {
    let mut request = agent()
        .get(&format!("{HELIX}{path}"))
        .header("Client-Id", client_id)
        .header("Authorization", &format!("Bearer {token}"));
    // Built as pairs rather than formatted into the path so ureq escapes them:
    // category names reach us from Twitch and go back as ids, but a hand-typed
    // one would otherwise break the URL.
    for (key, value) in query {
        request = request.query(*key, *value);
    }

    let (status, json) = read_body(request.call())?;

    match status {
        200..=299 => Ok(json),
        401 => Err(Error::NotSignedIn),
        // Surface Twitch's own wording; "HTTP 403" alone tells nobody whether
        // the scope, the client id or the token is at fault.
        other => Err(Error::Api(
            body_message(&json)
                .map(|m| format!("{m} (HTTP {other})"))
                .unwrap_or_else(|| format!("HTTP {other}")),
        )),
    }
}

/// `(user_id, login)` for the token's owner.
fn current_user(client_id: &str, token: &str) -> Result<(String, String), Error> {
    let json = helix_get(client_id, token, "/users", &[])?;
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

/// Most-watched first.
///
/// Every list of streams this module hands back is ordered this way, and Helix
/// promises no order of its own — `/streams` happens to come back sorted and
/// `/streams/followed` did too until it started being paginated, at which point
/// "sorted within each page" stopped meaning sorted.
fn by_viewers(streams: &mut [LiveStream]) {
    streams.sort_by_key(|stream| std::cmp::Reverse(stream.viewer_count));
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

/// Every Helix list answers as `{"data": [...]}`. This is the one place that
/// knows so; each parser below says only what one entry means.
fn entries(json: &Value) -> impl Iterator<Item = &Value> {
    json.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// A string field of an entry, absent or non-string reading as `None`.
fn text<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

/// A string field that may be missing, as an owned `String`.
fn text_or_empty(entry: &Value, key: &str) -> String {
    text(entry, key).unwrap_or_default().to_string()
}

fn parse_streams(json: &Value) -> Vec<LiveStream> {
    entries(json)
        .filter_map(|entry| {
            let user_login = text(entry, "user_login")?;
            Some(LiveStream {
                user_login: user_login.to_string(),
                display_name: text(entry, "user_name").unwrap_or(user_login).to_string(),
                title: text_or_empty(entry, "title"),
                game_name: text_or_empty(entry, "game_name"),
                viewer_count: entry
                    .get("viewer_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                thumbnail_url: text_or_empty(entry, "thumbnail_url"),
                started_at: text_or_empty(entry, "started_at"),
            })
        })
        .collect()
}

/// Walk one of the followed-list endpoints to its end.
///
/// Both endpoints paginate, both take the same query, and both have a natural
/// end - you follow a fixed number of people - so the loop is shared and only
/// the path and the parser differ. `MAX_FOLLOW_PAGES` bounds it regardless.
fn walk_follow_pages<T>(
    client_id: &str,
    token: &str,
    path: &str,
    user_id: &str,
    parse: fn(&Value) -> Vec<T>,
) -> Result<Vec<T>, Error> {
    let mut all: Vec<T> = Vec::new();
    let mut cursor: Option<String> = None;

    for _ in 0..MAX_FOLLOW_PAGES {
        // Scoped so the borrow of `cursor` ends before it is reassigned.
        let json = {
            let mut query = vec![("user_id", user_id), ("first", PAGE_SIZE)];
            if let Some(after) = &cursor {
                query.push(("after", after.as_str()));
            }
            helix_get(client_id, token, path, &query)?
        };
        all.extend(parse(&json));

        cursor = next_cursor(&json);
        if cursor.is_none() {
            break;
        }
    }
    Ok(all)
}

/// Live channels the signed-in user follows, most viewers first.
///
/// Paginated for the same reason `/channels/followed` is: Helix caps a page at
/// 100, and somebody following a thousand channels can easily have more than
/// that live at once. See [`MAX_FOLLOW_PAGES`] for what the single-page version
/// used to do to the went-live toasts.
pub fn followed_streams(
    client_id: &str,
    token: &str,
    user_id: &str,
) -> Result<Vec<LiveStream>, Error> {
    let mut streams = walk_follow_pages(
        client_id,
        token,
        "/streams/followed",
        user_id,
        parse_streams,
    )?;
    by_viewers(&mut streams);
    Ok(streams)
}

/// One page of a Helix list, and where the next one starts.
///
/// Twitch caps a page at 100 however many you ask for, so a list somebody might
/// scroll to the end of has to be fetched a page at a time rather than in one
/// go. The follows lists walk their pages internally because they have a
/// natural end — you follow a fixed number of people. The browse lists do not:
/// "popular" is every live channel on Twitch, so the cursor comes back out to
/// the caller and the user decides how far to go.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// `None` once Twitch has nothing more to give.
    pub next: Option<String>,
}

/// The `after` value for the next page, or `None` at the end of the list.
///
/// Twitch signals the end two ways — the key missing, and the key present but
/// empty — and only one of them is documented.
fn next_cursor(json: &Value) -> Option<String> {
    json.get("pagination")
        .and_then(|page| page.get("cursor"))
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_string)
}

/// A channel you follow, live or not.
///
/// Deliberately not a [`LiveStream`] with the fields left blank. Three things
/// read that list as *who is live* — the went-live toasts, the LIVE badge, and
/// the chat header's viewer count — and an offline channel sitting in it would
/// be wrong in all three at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedChannel {
    pub login: String,
    pub display_name: String,
}

fn parse_followed_channels(json: &Value) -> Vec<FollowedChannel> {
    entries(json)
        .filter_map(|entry| {
            let login = text(entry, "broadcaster_login")?;
            Some(FollowedChannel {
                login: login.to_string(),
                display_name: text(entry, "broadcaster_name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or(login)
                    .to_string(),
            })
        })
        .collect()
}

/// How many logins `/users` takes in one request. Helix's cap, not a choice.
const USERS_PER_REQUEST: usize = 100;

fn parse_profile_images(json: &Value) -> Vec<(String, String)> {
    entries(json)
        .filter_map(|entry| {
            let login = text(entry, "login")?;
            let image = text(entry, "profile_image_url").filter(|url| !url.is_empty())?;
            Some((login.to_string(), image.to_string()))
        })
        .collect()
}

/// Avatars for `logins`, as `(login, url)` pairs.
///
/// A channel's picture is the one thing about it that `/streams` does not carry
/// — the `thumbnail_url` there is the stream's own preview — so the follows
/// rail, which is a list of people rather than of pictures of games, needs this
/// second request to be recognisable at a glance.
///
/// Anyone Twitch does not answer for is simply absent from the result rather
/// than an error: a deleted account among two hundred follows should cost that
/// one row its picture, not the whole rail.
///
/// Needs no scope beyond the token already held; `/users` is public data.
pub fn profile_images(
    client_id: &str,
    token: &str,
    logins: &[String],
) -> Result<Vec<(String, String)>, Error> {
    let mut all = Vec::new();

    for batch in logins.chunks(USERS_PER_REQUEST) {
        // Repeated `login=` pairs rather than a comma-joined list: that is the
        // shape Helix takes, and building them as pairs is what gets each one
        // escaped.
        let query: Vec<(&str, &str)> = batch
            .iter()
            .map(|login| ("login", login.as_str()))
            .collect();
        let json = helix_get(client_id, token, "/users", &query)?;
        all.extend(parse_profile_images(&json));
    }

    Ok(all)
}

/// Every channel the signed-in user follows, in name order.
///
/// Needs `user:read:follows`, the same scope the live list already uses, so
/// this costs a request rather than another sign-in.
///
/// Sorted by name because Twitch returns them by when you followed, which is an
/// order nobody remembers.
pub fn followed_channels(
    client_id: &str,
    token: &str,
    user_id: &str,
) -> Result<Vec<FollowedChannel>, Error> {
    let mut all = walk_follow_pages(
        client_id,
        token,
        "/channels/followed",
        user_id,
        parse_followed_channels,
    )?;
    all.sort_by_key(|channel| channel.display_name.to_lowercase());
    Ok(all)
}

// ── Browsing ─────────────────────────────────────────────────────────

/// A Twitch category: usually a game, sometimes not ("Just Chatting").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// Template with `{width}`/`{height}` placeholders; use [`thumbnail`].
    /// Box art is 3:4, unlike stream thumbnails.
    pub box_art_url: String,
}

/// The most-watched live streams right now, or the most-watched within one
/// category.
///
/// Helix returns these in descending viewer order already; the sort is here so
/// the guarantee is ours rather than borrowed.
pub fn top_streams(
    client_id: &str,
    token: &str,
    category_id: Option<&str>,
    after: Option<&str>,
) -> Result<Page<LiveStream>, Error> {
    let mut query = vec![("first", PAGE_SIZE)];
    if let Some(id) = category_id {
        query.push(("game_id", id));
    }
    if let Some(cursor) = after {
        query.push(("after", cursor));
    }
    let json = helix_get(client_id, token, "/streams", &query)?;
    let mut items = parse_streams(&json);
    // Sorted within the page rather than across pages, which is enough: Helix
    // hands these back in descending viewer order, so every stream on page two
    // sits below every stream on page one and appending keeps the whole list
    // ordered.
    by_viewers(&mut items);
    Ok(Page {
        items,
        next: next_cursor(&json),
    })
}

/// The categories with the most viewers right now, in Twitch's own order.
///
/// Twitch does not report a viewer count per category here, only the ranking,
/// so there is no number to show beside the name.
pub fn top_categories(
    client_id: &str,
    token: &str,
    after: Option<&str>,
) -> Result<Page<Category>, Error> {
    let mut query = vec![("first", PAGE_SIZE)];
    if let Some(cursor) = after {
        query.push(("after", cursor));
    }
    let json = helix_get(client_id, token, "/games/top", &query)?;
    Ok(Page {
        items: parse_categories(&json),
        next: next_cursor(&json),
    })
}

/// Categories whose name matches `query`.
///
/// Twitch matches on substrings here, unlike the exact-name `/games` lookup, so
/// this is what a search box wants.
pub fn search_categories(
    client_id: &str,
    token: &str,
    query: &str,
) -> Result<Vec<Category>, Error> {
    let json = helix_get(
        client_id,
        token,
        "/search/categories",
        &[("query", query), ("first", SEARCH_PAGE_SIZE)],
    )?;
    Ok(parse_categories(&json))
}

/// Logins of live channels whose name matches `query`.
///
/// Only logins, because `/search/channels` answers with a *profile* picture and
/// no viewer count — a different shape from every other list in the app. The
/// caller feeds these to [`streams_by_login`] so the results are ordinary
/// streams like everything else.
pub fn search_channels(client_id: &str, token: &str, query: &str) -> Result<Vec<String>, Error> {
    let json = helix_get(
        client_id,
        token,
        "/search/channels",
        &[
            ("query", query),
            ("live_only", "true"),
            ("first", SEARCH_PAGE_SIZE),
        ],
    )?;
    Ok(parse_logins(&json))
}

/// Full stream records for named channels, skipping any that are offline.
///
/// Helix takes up to 100 `user_login` parameters in one request, so a page of
/// search results costs exactly one more round trip.
pub fn streams_by_login(
    client_id: &str,
    token: &str,
    logins: &[String],
) -> Result<Vec<LiveStream>, Error> {
    if logins.is_empty() {
        return Ok(Vec::new());
    }

    let mut query: Vec<(&str, &str)> = vec![("first", PAGE_SIZE)];
    query.extend(
        logins
            .iter()
            .take(MAX_LOGINS_PER_REQUEST)
            .map(|login| ("user_login", login.as_str())),
    );

    let json = helix_get(client_id, token, "/streams", &query)?;
    let mut streams = parse_streams(&json);
    by_viewers(&mut streams);
    Ok(streams)
}

fn parse_logins(json: &Value) -> Vec<String> {
    entries(json)
        .filter_map(|entry| text(entry, "broadcaster_login").map(str::to_string))
        .collect()
}

fn parse_categories(json: &Value) -> Vec<Category> {
    entries(json)
        .filter_map(|entry| {
            // A category with no id cannot be opened, so it is not worth
            // showing.
            let id = text(entry, "id")?;
            Some(Category {
                id: id.to_string(),
                name: text(entry, "name").unwrap_or(id).to_string(),
                box_art_url: text_or_empty(entry, "box_art_url"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `slow_down` is an instruction, and collapsing it into `Pending` throws
    /// the instruction away. Keeping them distinct is the whole fix, so the
    /// test asserts they are distinct rather than asserting one shape.
    #[test]
    fn slow_down_is_not_the_same_outcome_as_pending() {
        assert!(matches!(
            classify_token_error("authorization_pending"),
            Error::Pending
        ));
        assert!(matches!(classify_token_error("slow_down"), Error::SlowDown));
        assert!(matches!(
            classify_token_error("expired_token"),
            Error::Expired
        ));
        assert!(matches!(
            classify_token_error("invalid device code"),
            Error::Api(_)
        ));
    }

    /// A response with no `refresh_token` must fail here rather than persist an
    /// empty one, which would sign the user out on the *next* launch instead —
    /// far from the thing that caused it.
    #[test]
    fn a_token_response_without_a_refresh_token_is_a_shape_error() {
        let json: Value = serde_json::from_str(r#"{"access_token":"a","expires_in":100}"#).unwrap();
        assert!(matches!(tokens_from_response(&json), Err(Error::Shape(_))));

        let json: Value =
            serde_json::from_str(r#"{"refresh_token":"r","expires_in":100}"#).unwrap();
        assert!(matches!(tokens_from_response(&json), Err(Error::Shape(_))));
    }

    #[test]
    fn a_token_response_without_an_expiry_gets_the_default_hour() {
        let json: Value =
            serde_json::from_str(r#"{"access_token":"a","refresh_token":"r"}"#).unwrap();
        let tokens = tokens_from_response(&json).expect("both tokens are present");
        assert_eq!(tokens.access_token, "a");
        assert_eq!(tokens.refresh_token, "r");
        assert_eq!(tokens.expires_in, 3600);
    }

    /// Twitch ends a list two ways and only documents one of them.
    #[test]
    fn a_missing_or_empty_cursor_both_mean_the_last_page() {
        let end: Value = serde_json::from_str(r#"{"pagination":{}}"#).unwrap();
        assert_eq!(next_cursor(&end), None);

        let empty: Value = serde_json::from_str(r#"{"pagination":{"cursor":""}}"#).unwrap();
        assert_eq!(next_cursor(&empty), None);

        let none: Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert_eq!(next_cursor(&none), None);

        let more: Value = serde_json::from_str(r#"{"pagination":{"cursor":"abc"}}"#).unwrap();
        assert_eq!(next_cursor(&more), Some("abc".to_string()));
    }

    #[test]
    fn parses_a_users_payload_into_avatars() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"id":"1","login":"forsen","display_name":"Forsen",
                  "profile_image_url":"https://cdn/forsen.png"},
                 {"id":"2","login":"quin69","display_name":"Quin69",
                  "profile_image_url":"https://cdn/quin.png"}
               ]}"#,
        )
        .unwrap();

        let images = parse_profile_images(&json);
        assert_eq!(images.len(), 2);
        assert_eq!(
            images[0],
            ("forsen".into(), "https://cdn/forsen.png".into())
        );
    }

    /// One entry without a picture must not cost the rest theirs, which is the
    /// whole reason this is a filter rather than a map.
    #[test]
    fn a_user_without_a_picture_is_skipped_not_fatal() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"login":"nopic","profile_image_url":""},
                 {"login":"nokey"},
                 {"profile_image_url":"https://cdn/orphan.png"},
                 {"login":"fine","profile_image_url":"https://cdn/fine.png"}
               ]}"#,
        )
        .unwrap();

        let images = parse_profile_images(&json);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].0, "fine");
    }

    #[test]
    fn an_empty_users_payload_is_not_an_error() {
        let json: Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(parse_profile_images(&json).is_empty());
        assert!(parse_profile_images(&Value::Null).is_empty());
    }

    /// Helix takes a hundred logins per request and the rail can be asked for
    /// more than that, so the batching is the part worth pinning down.
    #[test]
    fn logins_are_batched_at_helix_cap() {
        let logins: Vec<String> = (0..250).map(|n| format!("user{n}")).collect();
        let batches: Vec<usize> = logins.chunks(USERS_PER_REQUEST).map(|b| b.len()).collect();
        assert_eq!(batches, vec![100, 100, 50]);
    }

    #[test]
    fn parses_a_followed_channels_payload() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"broadcaster_id":"1","broadcaster_login":"forsen",
                  "broadcaster_name":"Forsen","followed_at":"2019-01-01T00:00:00Z"},
                 {"broadcaster_id":"2","broadcaster_login":"theburntpeanut",
                  "broadcaster_name":"TheBurntPeanut","followed_at":"2024-01-01T00:00:00Z"}
               ],"pagination":{"cursor":"abc"}}"#,
        )
        .unwrap();

        let channels = parse_followed_channels(&json);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].login, "forsen");
        assert_eq!(channels[1].display_name, "TheBurntPeanut");
    }

    /// A display name is optional in practice — some accounts have none, and
    /// Twitch sends the key empty rather than omitting it — but a login is
    /// what makes the entry usable at all.
    #[test]
    fn a_followed_channel_without_a_name_falls_back_to_its_login() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"broadcaster_login":"someone","broadcaster_name":""},
                 {"broadcaster_name":"No Login Here"},
                 {"broadcaster_login":"other","broadcaster_name":"Other","new_field":1}
               ]}"#,
        )
        .unwrap();

        let channels = parse_followed_channels(&json);
        assert_eq!(channels.len(), 2, "the entry with no login should be gone");
        assert_eq!(channels[0].display_name, "someone");
        assert_eq!(channels[1].display_name, "Other");
    }

    #[test]
    fn an_empty_follows_payload_is_not_an_error() {
        let json: Value = serde_json::from_str(r#"{"data":[],"pagination":{}}"#).unwrap();
        assert!(parse_followed_channels(&json).is_empty());
        assert!(parse_followed_channels(&Value::Null).is_empty());
    }

    #[test]
    fn reads_logins_from_a_channel_search() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"broadcaster_login":"moonmoon","display_name":"MOONMOON","is_live":true},
                 {"display_name":"No Login Here","is_live":true},
                 {"broadcaster_login":"ben_","display_name":"Ben_","is_live":true}
               ]}"#,
        )
        .unwrap();

        assert_eq!(parse_logins(&json), vec!["moonmoon", "ben_"]);
    }

    #[test]
    fn parses_a_top_categories_payload() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"id":"509658","name":"Just Chatting",
                  "box_art_url":"https://cdn.test/jc-{width}x{height}.jpg"},
                 {"id":"32982","name":"Grand Theft Auto V",
                  "box_art_url":"https://cdn.test/gta-{width}x{height}.jpg"}
               ],"pagination":{"cursor":"abc"}}"#,
        )
        .unwrap();

        let categories = parse_categories(&json);
        assert_eq!(categories.len(), 2);
        assert_eq!(categories[0].name, "Just Chatting");
        assert_eq!(categories[1].id, "32982");
        assert_eq!(
            thumbnail(&categories[0].box_art_url, 285, 380),
            "https://cdn.test/jc-285x380.jpg"
        );
    }

    /// Twitch is free to add fields and to send entries we cannot use. Neither
    /// should cost us the rest of the page.
    #[test]
    fn categories_without_an_id_are_skipped() {
        let json: Value = serde_json::from_str(
            r#"{"data":[
                 {"name":"No Id Here","box_art_url":"https://cdn.test/x.jpg"},
                 {"id":"1","name":"Usable","box_art_url":"","some_new_field":7}
               ]}"#,
        )
        .unwrap();

        let categories = parse_categories(&json);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "Usable");
    }

    #[test]
    fn a_payload_with_no_data_is_empty_not_an_error() {
        let json: Value = serde_json::from_str(r#"{"pagination":{}}"#).unwrap();
        assert!(parse_categories(&json).is_empty());
        assert!(parse_streams(&json).is_empty());
    }

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
        assert_eq!(
            thumbnail(template, 100, 50),
            "https://cdn.test/x-100x50.jpg"
        );
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

    /// These strings come straight off the wire; classifying them wrongly is
    /// invisible in a type system and breaks sign-in entirely. Neither is a
    /// failure — see `slow_down_is_not_the_same_outcome_as_pending` for why
    /// they are nonetheless two outcomes and not one.
    #[test]
    fn pending_states_are_not_failures() {
        for message in ["authorization_pending", "slow_down"] {
            assert!(
                matches!(
                    classify_token_error(message),
                    Error::Pending | Error::SlowDown
                ),
                "{message} was treated as a failure"
            );
        }
    }

    #[test]
    fn expiry_is_distinguished_from_other_failures() {
        assert!(matches!(
            classify_token_error("device code has expired"),
            Error::Expired
        ));
        assert!(matches!(
            classify_token_error("invalid client"),
            Error::Api(_)
        ));
    }

    #[test]
    fn refresh_margin_triggers_before_expiry() {
        assert!(needs_refresh(now_secs() + 60));
        assert!(!needs_refresh(now_secs() + 3600));
        // Already expired.
        assert!(needs_refresh(0));
    }
}
