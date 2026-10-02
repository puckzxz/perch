//! Channels like the ones you watch: the data behind the rail's "Recommended"
//! group.
//!
//! Helix has no endpoint for this and never has. Twitch's own website fills
//! its sidebar from a GraphQL query it does not publish, `SideNav`, asked as a
//! persisted query by its hash,
//! `b9660765905e84e7b6a1ed18937b49ef0569e9b2a1c8f7a40a1bf289fbe2ced6`, on the
//! website's own Client-ID, `kimne78kx3ncx6brgo4mv6wki5h1ko`, and it answers
//! anonymously. Given a channel as context, one of the shelves in the answer,
//! `provider-side-nav-similar-streamer-currently-watching-1`, is five live
//! channels that channel's viewers also watch, and it is there when the
//! channel itself is offline too. That shelf is the only one read. Another,
//! `provider-side-nav-recommended-streams-1`, comes back beside it whatever
//! the context — a channel that does not exist gets that one and nothing else
//! — so it says nothing about the channel asked about and is ignored. Xtra
//! sends the same query with the same hash (`loadChannelSuggestions` in its
//! `GraphQLRepository`), which is as much of a contract as an unpublished
//! query gets.
//!
//! The second half, [`last_broadcasts`], is `users(logins:)` asked as a plain
//! query rather than a persisted one, on the same endpoint and Client-ID: when
//! each channel last went live, and whether it is live now. Helix has the
//! second and not the first.
//!
//! The third, [`channel_names`], is `users(ids:)`, asked the same way: the
//! login and display name of a channel known only by its numeric id, which is
//! all a Shared Chat line says about the room it was copied from
//! (`twitch_chat::ChatMessage::source_room`). Helix has that one too, behind
//! a token; asked here, a chat pane's label needs nothing of the session.
//! Verified anonymously on 2 October 2026.
//!
//! The first two were verified on 1 October 2026 with anonymous requests
//! carrying no credentials. Nothing here sends the user's token: `isLoggedIn`
//! is false, and a token issued to this app's Client-ID has no business
//! beside the website's.
//!
//! **This is not an API anyone offered.** Asked on Twitch's developer forum in
//! July 2019 whether third parties may use `gql.twitch.tv`, BarryCarlyon
//! answered that it is undocumented and third parties should not be using it:
//! "use at your own risk", and asked which risk, both — that it changes
//! underneath you without notice, and that a client gets blocked. That is
//! the stance taken here, the same one chat replay takes in `twitch-chat`:
//! nothing else in the app depends on this, and when it breaks the
//! recommendations go away quietly. A retired hash
//! answers `PersistedQueryNotFound` inside an HTTP 200; that, or any other
//! `errors` entry not known to pass, is [`Error::QueryRefused`], which a
//! caller takes as "hide the group and stop asking" rather than as something
//! to show. Twitch reports its own passing failures the same way — "service
//! error", "service timeout" and the like, the ones TwitchDropsMiner retries
//! — and those are [`Error::Network`], to ask again later, as a 5xx is. Any
//! HTTP 4xx, which is what a block would most likely look like though none
//! was seen, is [`Error::Api`] whatever its body. A shelf that
//! has been renamed or dropped is an empty list rather than an error, which
//! hides the group just the same. `fixtures/side-nav.json`,
//! `fixtures/last-broadcasts.json` and `fixtures/channel-names.json` are
//! answers as Twitch sent them on those days, trimmed, and are what the
//! parsers are tested against.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::{agent, answered, body_message, text, text_or_empty, Channel, Error};

const ENDPOINT: &str = "https://gql.twitch.tv/gql";
/// The website's Client-ID, which every client of this endpoint uses. Not the
/// app's own: that one is for Helix, and this endpoint does not know it.
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const SIDE_NAV: &str = "SideNav";
/// The persisted query's hash, as Xtra sends it and as Twitch still answered
/// it on 1 October 2026. The day Twitch changes the query, this answers
/// `PersistedQueryNotFound`.
const SIDE_NAV_HASH: &str = "b9660765905e84e7b6a1ed18937b49ef0569e9b2a1c8f7a40a1bf289fbe2ced6";
/// What the one shelf worth reading has in its id. Matched as a part rather
/// than whole, since the `-1` on the end reads like a counter.
const SIMILAR_SHELF: &str = "similar-streamer";
/// Asked with variables rather than with the logins spelled into the text, so
/// nothing a login contains can change what the query says.
const LAST_BROADCASTS: &str = "query LastBroadcasts($logins: [String!]) { \
     users(logins: $logins) { login lastBroadcast { startedAt } stream { id } } }";
/// Asked with variables for the reason [`LAST_BROADCASTS`] is.
const CHANNEL_NAMES: &str = "query ChannelNames($ids: [ID!]) { \
     users(ids: $ids) { id login displayName } }";
/// How many logins one [`last_broadcasts`] request carries, and how many ids
/// one [`channel_names`] request does.
///
/// A choice, not a limit Twitch was seen to enforce: 101 logins were answered
/// in full on 1 October 2026, and so were 250 with repeats among them. A
/// hundred is Helix's cap on the same question, keeps one answer around ten
/// kilobytes, and leaves room under whatever limit there is that nobody went
/// looking for. The hundred ids a `users(ids:)` request carries are the same
/// choice by analogy, and were not probed past a handful.
const LOGINS_PER_REQUEST: usize = 100;
/// The `errors` messages that mean a failure on Twitch's side that passes:
/// the ones TwitchDropsMiner retries (`gql_request` in its `twitch.py`), and
/// "server error", which it reads as that one field missing. It retries
/// `PersistedQueryNotFound` once too; here that is a retired hash, and
/// refuses. Compared whole, as Twitch writes them.
const PASSING: [&str; 6] = [
    "service error",
    "service timeout",
    "service unavailable",
    "request cancelled",
    "context deadline exceeded",
    "server error",
];

// ── Similar channels ─────────────────────────────────────────────────

/// A live channel Twitch says the viewers of another channel also watch.
///
/// Its own type rather than a [`LiveStream`](crate::LiveStream), because the
/// shelf carries a profile picture and a stream id and no thumbnail or start
/// time, and a `LiveStream` with those left blank would be wrong wherever one
/// is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarChannel {
    pub login: String,
    pub user_id: String,
    pub display_name: String,
    pub title: String,
    pub game_name: String,
    /// The category's id, as Helix's `game_id` takes it.
    pub game_id: String,
    pub viewer_count: u64,
    /// A finished URL, 70x70, with no placeholders to fill.
    pub profile_image_url: String,
    pub stream_id: String,
}

/// The `SideNav` request for channels like `seed`, exactly as it was verified:
/// the variables are the website's own, with `isLoggedIn` false.
pub fn side_nav_request(seed: &str) -> Value {
    json!({
        "operationName": SIDE_NAV,
        "variables": {
            "creatorAnniversariesFeature": false,
            "input": {
                "contextChannelName": seed,
                "recommendationContext": {
                    "channelName": seed,
                    "clientApp": "twilight",
                    "location": "channel",
                    "platform": "web",
                },
            },
            "isLoggedIn": false,
            "withFreeformTags": true,
        },
        "extensions": {
            "persistedQuery": { "version": 1, "sha256Hash": SIDE_NAV_HASH },
        },
    })
}

/// The similar-channel shelf of a `SideNav` answer, in Twitch's order.
///
/// Every other shelf is ignored. No similar shelf at all — a channel that does
/// not exist, or a shelf Twitch has renamed — is an empty list, not an error:
/// either way there is nothing to recommend, and the group hides. An `errors`
/// array is an error, even beside data: [`Error::Network`] for a failure
/// Twitch says passes, [`Error::QueryRefused`] for anything else.
pub fn parse_side_nav(body: &Value) -> Result<Vec<SimilarChannel>, Error> {
    if let Some(error) = gql_error(body) {
        return Err(error);
    }
    Ok(edges(body.pointer("/data/sideNav/sections"))
        .filter(|shelf| text(shelf, "id").is_some_and(|id| id.contains(SIMILAR_SHELF)))
        .flat_map(|shelf| edges(shelf.get("content")))
        .filter_map(similar_channel)
        .collect())
}

/// Live channels whose viewers also watch `seed`: five when it was measured,
/// though nothing promises the number.
///
/// One request per seed, and the website asks once per page it shows, so a
/// caller asking for every channel somebody follows should ask rarely.
pub fn similar_channels(seed: &str) -> Result<Vec<SimilarChannel>, Error> {
    let seed = seed.trim();
    if seed.is_empty() {
        return Ok(Vec::new());
    }
    parse_side_nav(&post(&side_nav_request(seed))?)
}

/// One shelf entry, or `None` for one that cannot be shown.
fn similar_channel(node: &Value) -> Option<SimilarChannel> {
    // A shelf is a list of anything; every entry seen was a stream, and only
    // a stream is somewhere to go.
    if text(node, "__typename").is_some_and(|kind| kind != "Stream") {
        return None;
    }
    let broadcaster = node.get("broadcaster")?;
    let login = text(broadcaster, "login").filter(|login| !login.is_empty())?;
    let game = |key: &str| node.get("game").and_then(|game| text(game, key));
    Some(SimilarChannel {
        login: login.to_string(),
        user_id: text_or_empty(broadcaster, "id"),
        display_name: text(broadcaster, "displayName")
            .filter(|name| !name.is_empty())
            .unwrap_or(login)
            .to_string(),
        title: broadcaster
            .pointer("/broadcastSettings/title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // What the website's own sidebar shows; `name` is the same word
        // unless a language is asked for, and none is.
        game_name: game("displayName")
            .or_else(|| game("name"))
            .unwrap_or_default()
            .to_string(),
        game_id: game("id").unwrap_or_default().to_string(),
        viewer_count: node
            .get("viewersCount")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        profile_image_url: text_or_empty(broadcaster, "profileImageURL"),
        stream_id: text_or_empty(node, "id"),
    })
}

/// The nodes of a GraphQL connection: `{"edges": [{"node": ...}]}`, the shape
/// both the shelves and each shelf's content come in. A missing connection is
/// no nodes.
fn edges(connection: Option<&Value>) -> impl Iterator<Item = &Value> {
    connection
        .and_then(|connection| connection.get("edges"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|edge| edge.get("node"))
}

// ── Last broadcasts ──────────────────────────────────────────────────

/// When a channel last went live, and whether it is live now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastBroadcast {
    /// RFC 3339, with a fraction of a second of up to six digits: Twitch
    /// drops trailing zeros, so `.17837` comes back as well as `.469059`.
    /// `None` for a channel that has never broadcast, which Twitch writes as
    /// a `lastBroadcast` whose `startedAt` is null.
    pub started_at: Option<String>,
    /// Whether the channel has a stream right now. For a channel that is
    /// live, `started_at` is when that stream began.
    pub live: bool,
}

/// One request's body, for a batch of up to a hundred logins
/// (`LOGINS_PER_REQUEST`).
pub fn last_broadcasts_request(logins: &[String]) -> Value {
    json!({
        "query": LAST_BROADCASTS,
        "variables": { "logins": logins },
    })
}

/// Each channel in a `users` answer, keyed by its login as Twitch writes it,
/// which is lowercase whatever case it was asked in.
///
/// Twitch answers a login it has no account for (renamed, banned, never
/// existed) with a bare `null` in that login's place. That login is simply
/// absent from the map rather than an error, the way a missing picture is in
/// [`profile_images`](crate::profile_images). An `errors` array is an error
/// as it is for [`parse_side_nav`], even when it names one user's field.
pub fn parse_last_broadcasts(body: &Value) -> Result<HashMap<String, LastBroadcast>, Error> {
    if let Some(error) = gql_error(body) {
        return Err(error);
    }
    Ok(body
        .pointer("/data/users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|user| {
            let login = text(user, "login")?;
            let started_at = user
                .pointer("/lastBroadcast/startedAt")
                .and_then(Value::as_str)
                .filter(|when| !when.is_empty())
                .map(str::to_string);
            let live = user.get("stream").is_some_and(|stream| !stream.is_null());
            Some((login.to_string(), LastBroadcast { started_at, live }))
        })
        .collect())
}

/// When each of `logins` last went live and whether it is live now, keyed as
/// [`parse_last_broadcasts`] says.
///
/// Asked a hundred at a time (`LOGINS_PER_REQUEST` says why that number). No
/// logins is no request. One batch
/// failing fails the lot: half a list sorted by recency would look whole.
pub fn last_broadcasts(logins: &[String]) -> Result<HashMap<String, LastBroadcast>, Error> {
    let mut all = HashMap::new();
    for request in last_broadcasts_requests(logins) {
        all.extend(parse_last_broadcasts(&post(&request)?)?);
    }
    Ok(all)
}

/// The request bodies [`last_broadcasts`] sends, one per batch.
fn last_broadcasts_requests(logins: &[String]) -> impl Iterator<Item = Value> + '_ {
    logins
        .chunks(LOGINS_PER_REQUEST)
        .map(last_broadcasts_request)
}

// ── Channel names ────────────────────────────────────────────────────

/// One request's body, for a batch of up to a hundred ids
/// (`LOGINS_PER_REQUEST`).
pub fn channel_names_request(ids: &[String]) -> Value {
    json!({
        "query": CHANNEL_NAMES,
        "variables": { "ids": ids },
    })
}

/// Each channel in a `users(ids:)` answer.
///
/// An id Twitch has no account for comes back as a bare `null` in its
/// place, as an unknown login does for [`parse_last_broadcasts`], and is
/// simply not in the list. One with no login is skipped too: there is
/// nothing to name it by. A display name that is missing or empty is the
/// login, the way Helix's own fallbacks run. An `errors` array is an error,
/// as everywhere here.
pub fn parse_channel_names(body: &Value) -> Result<Vec<Channel>, Error> {
    if let Some(error) = gql_error(body) {
        return Err(error);
    }
    Ok(body
        .pointer("/data/users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|user| {
            let login = text(user, "login").filter(|login| !login.is_empty())?;
            let user_id = text(user, "id").filter(|id| !id.is_empty())?;
            let display_name = text(user, "displayName")
                .filter(|name| !name.is_empty())
                .unwrap_or(login);
            Some(Channel {
                login: login.to_string(),
                user_id: user_id.to_string(),
                display_name: display_name.to_string(),
            })
        })
        .collect())
}

/// The channels behind `ids`, as [`parse_channel_names`] reads them: asked
/// a hundred at a time, no ids being no request, and one batch failing
/// failing the lot, as [`last_broadcasts`] does.
pub fn channel_names(ids: &[String]) -> Result<Vec<Channel>, Error> {
    let mut all = Vec::new();
    for request in ids.chunks(LOGINS_PER_REQUEST).map(channel_names_request) {
        all.extend(parse_channel_names(&post(&request)?)?);
    }
    Ok(all)
}

// ── Ranking ──────────────────────────────────────────────────────────

/// A channel to suggest, and the channels that led to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub channel: SimilarChannel,
    /// The seeds whose shelves listed it, in the order the seeds were given,
    /// each once. Never empty; its length is what the ranking counts.
    pub because: Vec<String>,
}

/// Put several seeds' shelves together into one list worth showing.
///
/// `per_seed` is each seed with what [`similar_channels`] answered for it, in
/// the order the seeds matter to the caller; that is the order `because`
/// names them in. A channel several seeds point at comes first, since it is
/// the likeliest to be to the user's taste; then the most watched; then by
/// login, so the order is the same every time for the same answers.
///
/// Dropped before ranking: everything in `exclude` (the channels followed and
/// the ones already open), and the seeds themselves, which are channels the
/// user already watches and so are no recommendation. Logins are compared
/// without regard to case. A channel that appears under several seeds is
/// listed once, with the details from the first seed that listed it, and a
/// seed that lists a channel twice counts once.
pub fn rank<S: AsRef<str>>(
    per_seed: &[(String, Vec<SimilarChannel>)],
    exclude: impl IntoIterator<Item = S>,
    limit: usize,
) -> Vec<Recommendation> {
    let mut skip: HashSet<String> = exclude
        .into_iter()
        .map(|login| login.as_ref().to_lowercase())
        .collect();
    skip.extend(per_seed.iter().map(|(seed, _)| seed.to_lowercase()));

    let mut found: Vec<Recommendation> = Vec::new();
    // Lowercased login to its place in `found`.
    let mut at: HashMap<String, usize> = HashMap::new();
    for (seed, similar) in per_seed {
        for channel in similar {
            let key = channel.login.to_lowercase();
            if skip.contains(&key) {
                continue;
            }
            match at.get(&key) {
                Some(&index) => {
                    let because = &mut found[index].because;
                    if !because.iter().any(|known| known.eq_ignore_ascii_case(seed)) {
                        because.push(seed.clone());
                    }
                }
                None => {
                    at.insert(key, found.len());
                    found.push(Recommendation {
                        channel: channel.clone(),
                        because: vec![seed.clone()],
                    });
                }
            }
        }
    }

    found.sort_by(|a, b| {
        b.because
            .len()
            .cmp(&a.because.len())
            .then(b.channel.viewer_count.cmp(&a.channel.viewer_count))
            .then_with(|| a.channel.login.cmp(&b.channel.login))
    });
    found.truncate(limit);
    found
}

// ── The endpoint ─────────────────────────────────────────────────────

/// Send one GraphQL body and read the answer. A transport failure or a 5xx is
/// [`Error::Network`], as everywhere in the crate; the rest is
/// [`by_status`]. A refusal arrives as HTTP 200 and is left to the parser,
/// which is where the tests can reach it.
fn post(body: &Value) -> Result<Value, Error> {
    let (status, mut response) = answered(
        agent()
            .post(ENDPOINT)
            .header("Client-ID", CLIENT_ID)
            .header("X-Device-Id", device_id())
            .send_json(body),
    )?;
    let json = response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| e.to_string());
    by_status(status, json)
}

/// What an answer below 500 comes to: any 4xx is [`Error::Api`] whatever its
/// body, and anything else is its JSON, or [`Error::Shape`] when it has none.
///
/// Not the crate's [`read_body`](crate::read_body), which makes a 4xx with a
/// body that is not JSON a `Shape`: a block page or a plain-text 429 is the
/// kind of 4xx a block would bring, and it should read as Twitch turning the
/// request away rather than as a malformed answer.
fn by_status(status: u16, json: Result<Value, String>) -> Result<Value, Error> {
    if status >= 400 {
        let message = json.as_ref().ok().and_then(body_message);
        return Err(Error::Api(match message {
            Some(message) => format!("{message} (HTTP {status})"),
            None => format!("HTTP {status}"),
        }));
    }
    json.map_err(|e| Error::Shape(format!("HTTP {status} with an unreadable body: {e}")))
}

/// The error an answer's `errors` array comes to, if it has one.
///
/// Twitch puts two kinds there, both inside an HTTP 200. A failure on its
/// side that passes (one of [`PASSING`]) is [`Error::Network`], for the
/// caller to ask again later; chat replay meets "service error" beside an
/// otherwise good answer, and TwitchDropsMiner retries it. Anything else
/// — `PersistedQueryNotFound` for a retired hash, `failed integrity check`
/// for a client Twitch wants proof from, a message nobody has seen yet — is
/// [`Error::QueryRefused`], and so is an array mixing the two.
///
/// Either way the data beside the errors is not read. A partial answer from
/// a query somebody else owns is not one to reason about: a list quietly
/// missing what made it worth showing is worse than one that comes back
/// next time.
fn gql_error(body: &Value) -> Option<Error> {
    fn message(error: &Value) -> &str {
        text(error, "message").unwrap_or("an error with no message")
    }
    let errors = body.get("errors")?.as_array()?;
    let first = errors.first()?;
    Some(
        match errors
            .iter()
            .find(|error| !PASSING.contains(&message(error)))
        {
            Some(lasting) => Error::QueryRefused(message(lasting).to_string()),
            None => Error::Network(format!(
                "Twitch answered \"{}\" inside an HTTP 200",
                message(first)
            )),
        },
    )
}

/// The `X-Device-Id` every request here carries: 32 hex digits, made once per
/// process.
///
/// The website sends one per browser, from a cookie, and Xtra makes one up
/// whenever it has none, which is what this does. Every probe on 1 October
/// 2026 sent one; what Twitch does with a request that has none was not
/// tried. Once per process rather than per request, so a session of the app
/// looks to Twitch like one visitor rather than a crowd, and none outlives the
/// app.
///
/// No dependency for it. `RandomState` seeds its hasher from the operating
/// system's randomness, the one source of it std exposes, and the time and the
/// process id are mixed in besides. It is not a secret, only something that
/// should not collide with another install's.
fn device_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let half = |salt: u8| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u128(nanos);
            hasher.write_u32(std::process::id());
            hasher.write_u8(salt);
            hasher.finish()
        };
        format!("{:016x}{:016x}", half(0), half(1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SideNav` answer Twitch sent on 1 October 2026 for `forsen`, trimmed:
    /// two entries of the generic shelf, which comes first in the answer, and
    /// three of the five on the similar shelf — one with an emoji title, one
    /// with a single viewer.
    const SIDE_NAV_ANSWER: &str = include_str!("fixtures/side-nav.json");
    /// A `users` answer from the same day, asked by variables for `forsen`,
    /// `Summit1G` (in that case), `esl_csgo`, a login that does not exist,
    /// `asmongold` and `lirik_247`.
    const LAST_BROADCASTS_ANSWER: &str = include_str!("fixtures/last-broadcasts.json");
    /// A `users(ids:)` answer from 2 October 2026, asked by variables for
    /// `12826` (twitch), `141981764` (twitchdev), an id with no account and
    /// `22484632` (forsen), which writes its display name in lowercase.
    const CHANNEL_NAMES_ANSWER: &str = include_str!("fixtures/channel-names.json");

    fn side_nav() -> Value {
        serde_json::from_str(SIDE_NAV_ANSWER).unwrap()
    }

    fn channel(login: &str, viewers: u64) -> SimilarChannel {
        SimilarChannel {
            login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            game_id: String::new(),
            viewer_count: viewers,
            profile_image_url: String::new(),
            stream_id: String::new(),
        }
    }

    fn logins(found: &[Recommendation]) -> Vec<&str> {
        found.iter().map(|r| r.channel.login.as_str()).collect()
    }

    /// The message of a refusal, failing the test on anything else.
    fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
        match result {
            Err(Error::QueryRefused(message)) => message,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The message of a passing failure, failing the test on anything else.
    fn passing<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
        match result {
            Err(Error::Network(message)) => message,
            other => panic!("expected a passing failure, got {other:?}"),
        }
    }

    #[test]
    fn the_similar_shelf_as_twitch_sends_it() {
        let similar = parse_side_nav(&side_nav()).unwrap();
        let order: Vec<&str> = similar.iter().map(|c| c.login.as_str()).collect();
        // The generic shelf comes first in the answer, and neither of its
        // channels (ironmouse, peterpark) is here: the similar shelf's three,
        // in its order, are all there is.
        assert_eq!(order, ["admiralbulldog", "summit1g", "nymn247"]);

        assert_eq!(
            similar[0],
            SimilarChannel {
                login: "admiralbulldog".into(),
                user_id: "30816637".into(),
                display_name: "AdmiralBulldog".into(),
                title: "🔴VODS ON KICK 🔴24/7 STREAM 👉 @admiralbulldog247 🔴PRIME 🔴YOUNG \
                        🔴SHREDDED 🔴LUSCIOUS HAIR 🔴"
                    .into(),
                game_name: "Dota 2".into(),
                game_id: "29595".into(),
                viewer_count: 2095,
                profile_image_url: "https://static-cdn.jtvnw.net/jtv_user_pictures/\
                                    admiralbulldog-profile_image-888d5b80958e636f-70x70.jpeg"
                    .into(),
                stream_id: "316941856597".into(),
            }
        );
        assert_eq!(similar[1].viewer_count, 8953);
        assert_eq!(similar[2].display_name, "NymN247");
        assert_eq!(similar[2].game_name, "Welcome to the Game II");
        assert_eq!(similar[2].viewer_count, 1);
    }

    /// What a channel that does not exist gets, measured: the generic shelf
    /// and nothing else. A renamed shelf reads the same way, and so does any
    /// answer missing the path altogether.
    #[test]
    fn no_similar_shelf_is_an_empty_list_not_an_error() {
        let mut only_generic = side_nav();
        only_generic
            .pointer_mut("/data/sideNav/sections/edges")
            .and_then(Value::as_array_mut)
            .unwrap()
            .truncate(1);
        assert!(parse_side_nav(&only_generic).unwrap().is_empty());

        let mut renamed = side_nav();
        *renamed
            .pointer_mut("/data/sideNav/sections/edges/1/node/id")
            .unwrap() = json!("provider-side-nav-channels-like-this-1");
        assert!(parse_side_nav(&renamed).unwrap().is_empty());

        for body in [
            json!({}),
            json!({ "data": null }),
            json!({ "data": { "sideNav": null } }),
        ] {
            assert!(parse_side_nav(&body).unwrap().is_empty(), "{body}");
        }
    }

    /// The shape a retired hash answers in, as it came back on 1 October 2026
    /// for a made-up one: HTTP 200, no data, one error.
    #[test]
    fn a_retired_hash_is_its_own_error() {
        let body = json!({ "errors": [{
            "message": "PersistedQueryNotFound",
            "extensions": { "code": "PERSISTED_QUERY_NOT_FOUND" }
        }]});
        assert_eq!(refused(parse_side_nav(&body)), "PersistedQueryNotFound");
        assert_eq!(
            refused(parse_last_broadcasts(&body)),
            "PersistedQueryNotFound"
        );
    }

    /// Every failure Twitch says passes is a network error, to ask again
    /// later, whether it comes beside data, alone, or naming one user's field
    /// in a batch. Spelled out rather than read from `PASSING`, so dropping
    /// one from the list fails here.
    #[test]
    fn a_passing_failure_is_a_network_error() {
        for message in [
            "service error",
            "service timeout",
            "service unavailable",
            "request cancelled",
            "context deadline exceeded",
            "server error",
        ] {
            let mut beside_data = side_nav();
            beside_data["errors"] = json!([{ "message": message, "path": ["sideNav"] }]);
            assert!(passing(parse_side_nav(&beside_data)).contains(message));

            let alone = json!({ "errors": [{ "message": message }], "data": null });
            assert!(passing(parse_side_nav(&alone)).contains(message));

            let mut one_user: Value = serde_json::from_str(LAST_BROADCASTS_ANSWER).unwrap();
            one_user["errors"] = json!([{
                "message": message, "path": ["users", 4, "lastBroadcast"]
            }]);
            assert!(passing(parse_last_broadcasts(&one_user)).contains(message));
        }
    }

    /// Anything not known to pass refuses, beside data or not: an integrity
    /// failure, a message nobody has seen, an error with no message, and a
    /// passing failure in the same array as a lasting one. An empty errors
    /// array is no error.
    #[test]
    fn everything_else_refuses_and_no_errors_do_not() {
        let integrity = json!({ "errors": [{ "message": "failed integrity check" }] });
        assert_eq!(
            refused(parse_side_nav(&integrity)),
            "failed integrity check"
        );

        let mut unknown = side_nav();
        unknown["errors"] = json!([{ "message": "something new", "path": ["sideNav"] }]);
        assert_eq!(refused(parse_side_nav(&unknown)), "something new");

        let unexplained = json!({ "errors": [{}] });
        assert!(!refused(parse_last_broadcasts(&unexplained)).is_empty());

        let mixed = json!({ "errors": [
            { "message": "service error" },
            { "message": "PersistedQueryNotFound" }
        ]});
        assert_eq!(refused(parse_side_nav(&mixed)), "PersistedQueryNotFound");

        let mut empty = side_nav();
        empty["errors"] = json!([]);
        assert_eq!(parse_side_nav(&empty).unwrap().len(), 3);
    }

    /// Any 4xx is Twitch turning the request away, whatever its body, so a
    /// block page is not mistaken for a malformed answer; below 400, a body
    /// that is not JSON is.
    #[test]
    fn a_4xx_is_api_whatever_its_body() {
        let html = || Err("expected value at line 1 column 1".to_string());
        assert!(matches!(by_status(403, html()), Err(Error::Api(m)) if m == "HTTP 403"));
        assert!(matches!(
            by_status(429, Ok(json!({ "message": "slow down" }))),
            Err(Error::Api(m)) if m == "slow down (HTTP 429)"
        ));
        assert!(matches!(by_status(200, html()), Err(Error::Shape(_))));
        assert_eq!(
            by_status(200, Ok(json!({ "data": {} }))).unwrap(),
            json!({ "data": {} })
        );
    }

    /// Entries Twitch could send and has not yet: something other than a
    /// stream, a stream with nobody broadcasting it, and one with every
    /// optional field missing, which keeps its login as its name.
    #[test]
    fn an_entry_that_cannot_be_shown_is_skipped() {
        let body = json!({ "data": { "sideNav": { "sections": { "edges": [{ "node": {
            "id": "provider-side-nav-similar-streamer-currently-watching-1",
            "content": { "edges": [
                { "node": { "__typename": "Video", "id": "1",
                            "broadcaster": { "login": "not_a_stream" } } },
                { "node": { "__typename": "Stream", "id": "2", "broadcaster": null } },
                { "node": { "__typename": "Stream", "id": "3",
                            "broadcaster": { "login": "" } } },
                { "node": { "id": "4", "broadcaster": { "login": "bare", "displayName": "" },
                            "game": null } }
            ]}
        }}]}}}});
        let similar = parse_side_nav(&body).unwrap();
        assert_eq!(similar.len(), 1);
        assert_eq!(similar[0].login, "bare");
        assert_eq!(similar[0].display_name, "bare");
        assert_eq!(similar[0].game_name, "");
        assert_eq!(similar[0].viewer_count, 0);
        assert_eq!(similar[0].stream_id, "4");
    }

    /// The body the module docs say was verified, byte for byte as JSON: the
    /// hash, the variables, and `isLoggedIn` false.
    #[test]
    fn the_request_is_the_one_that_was_verified() {
        let verified: Value = serde_json::from_str(
            r#"{"operationName":"SideNav","variables":{"creatorAnniversariesFeature":false,
                "input":{"contextChannelName":"forsen","recommendationContext":{
                "channelName":"forsen","clientApp":"twilight","location":"channel",
                "platform":"web"}},"isLoggedIn":false,"withFreeformTags":true},
                "extensions":{"persistedQuery":{"version":1,"sha256Hash":
                "b9660765905e84e7b6a1ed18937b49ef0569e9b2a1c8f7a40a1bf289fbe2ced6"}}}"#,
        )
        .unwrap();
        assert_eq!(side_nav_request("forsen"), verified);
    }

    #[test]
    fn last_broadcasts_as_twitch_sends_them() {
        let body: Value = serde_json::from_str(LAST_BROADCASTS_ANSWER).unwrap();
        let found = parse_last_broadcasts(&body).unwrap();
        // Six asked, one with no account.
        assert_eq!(found.len(), 5);
        assert!(!found.contains_key("this_login_should_not_exist_zz9"));

        assert_eq!(
            found["forsen"],
            LastBroadcast {
                started_at: Some("2026-09-30T18:59:21.469059Z".into()),
                live: false,
            }
        );
        // Asked as `Summit1G`, answered as Twitch writes it.
        assert!(!found.contains_key("Summit1G"));
        assert!(found["summit1g"].live);
        assert_eq!(
            found["summit1g"].started_at.as_deref(),
            Some("2026-09-30T20:17:02.477609Z")
        );
        // Never broadcast: a `lastBroadcast` whose `startedAt` is null.
        assert_eq!(
            found["esl_csgo"],
            LastBroadcast {
                started_at: None,
                live: false,
            }
        );
        assert_eq!(
            found["asmongold"].started_at.as_deref(),
            Some("2023-06-10T22:05:36.711791Z")
        );
        assert!(found["lirik_247"].live);
        // Five digits of fraction: Twitch drops the trailing zero.
        assert_eq!(
            found["lirik_247"].started_at.as_deref(),
            Some("2026-09-29T18:53:26.17837Z")
        );
    }

    #[test]
    fn an_empty_or_missing_users_list_is_not_an_error() {
        for body in [
            json!({}),
            json!({ "data": { "users": [] } }),
            json!({ "data": { "users": [null, null] } }),
        ] {
            assert!(parse_last_broadcasts(&body).unwrap().is_empty(), "{body}");
        }
    }

    /// The logins travel as a variable, not spelled into the query text.
    #[test]
    fn the_last_broadcasts_request_carries_logins_as_a_variable() {
        let request = last_broadcasts_request(&["forsen".into(), "a\"b".into()]);
        assert_eq!(request["variables"]["logins"], json!(["forsen", "a\"b"]));
        let query = request["query"].as_str().unwrap();
        assert!(query.contains("users(logins: $logins)"));
        assert!(!query.contains("forsen"));
    }

    #[test]
    fn channel_names_as_twitch_sends_them() {
        let body: Value = serde_json::from_str(CHANNEL_NAMES_ANSWER).unwrap();
        let found = parse_channel_names(&body).unwrap();
        let named: Vec<(&str, &str, &str)> = found
            .iter()
            .map(|c| {
                (
                    c.user_id.as_str(),
                    c.login.as_str(),
                    c.display_name.as_str(),
                )
            })
            .collect();
        // Four asked, one with no account.
        assert_eq!(
            named,
            [
                ("12826", "twitch", "Twitch"),
                ("141981764", "twitchdev", "TwitchDev"),
                ("22484632", "forsen", "forsen"),
            ]
        );
    }

    #[test]
    fn a_channel_with_no_name_is_skipped_or_named_by_its_login() {
        let body = json!({ "data": { "users": [
            { "id": "1", "login": "", "displayName": "Nameless" },
            { "id": "", "login": "noid", "displayName": "NoId" },
            { "id": "3", "login": "plain", "displayName": "" },
            { "id": "4", "login": "bare" },
        ] } });
        let found = parse_channel_names(&body).unwrap();
        let names: Vec<&str> = found.iter().map(|c| c.display_name.as_str()).collect();
        assert_eq!(names, ["plain", "bare"]);
        assert!(parse_channel_names(&json!({})).unwrap().is_empty());
        // Refused like every other query here.
        let refusal = json!({ "errors": [{ "message": "failed integrity check" }] });
        assert_eq!(
            refused(parse_channel_names(&refusal)),
            "failed integrity check"
        );
    }

    /// The ids travel as a variable, not spelled into the query text.
    #[test]
    fn the_channel_names_request_carries_ids_as_a_variable() {
        let request = channel_names_request(&["12826".into()]);
        assert_eq!(request["variables"]["ids"], json!(["12826"]));
        let query = request["query"].as_str().unwrap();
        assert!(query.contains("users(ids: $ids)"));
        assert!(!query.contains("12826"));
    }

    #[test]
    fn logins_are_asked_a_hundred_at_a_time() {
        let many: Vec<String> = (0..250).map(|n| format!("user{n}")).collect();
        let sizes: Vec<usize> = last_broadcasts_requests(&many)
            .map(|request| request["variables"]["logins"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, [100, 100, 50]);
        assert_eq!(last_broadcasts_requests(&[]).count(), 0);
    }

    /// Two seeds agreeing beats viewers; viewers beat the alphabet; and
    /// `because` names the seeds in the order they were given.
    #[test]
    fn channels_more_seeds_point_at_come_first() {
        let per_seed = vec![
            (
                "forsen".to_string(),
                vec![
                    channel("small", 10),
                    channel("big", 9000),
                    channel("shared", 5),
                ],
            ),
            (
                "nymn".to_string(),
                vec![
                    channel("shared", 5),
                    channel("tied_b", 100),
                    channel("tied_a", 100),
                ],
            ),
        ];
        let found = rank(&per_seed, Vec::<String>::new(), 10);
        assert_eq!(
            logins(&found),
            ["shared", "big", "tied_a", "tied_b", "small"]
        );
        assert_eq!(found[0].because, ["forsen", "nymn"]);
        assert_eq!(found[1].because, ["forsen"]);
    }

    /// Followed and open channels go, and so do the seeds, whatever case
    /// either side is written in.
    #[test]
    fn excluded_channels_and_the_seeds_themselves_are_dropped() {
        let per_seed = vec![
            (
                "Forsen".to_string(),
                vec![
                    channel("nymn", 50),
                    channel("summit1g", 9000),
                    channel("lirik", 20),
                ],
            ),
            (
                "nymn".to_string(),
                vec![channel("forsen", 30000), channel("xqc", 40)],
            ),
        ];
        let followed = ["SUMMIT1G"];
        let found = rank(&per_seed, followed, 10);
        assert_eq!(logins(&found), ["xqc", "lirik"]);

        let open = HashSet::from(["XQC".to_string()]);
        assert_eq!(logins(&rank(&per_seed, &open, 10)), ["summit1g", "lirik"]);
    }

    /// A seed listing a channel twice, or the same seed given twice in other
    /// cases, does not make that channel look agreed upon.
    #[test]
    fn one_seed_counts_once() {
        let per_seed = vec![
            (
                "forsen".to_string(),
                vec![
                    channel("twice", 1),
                    channel("twice", 1),
                    channel("other", 2),
                ],
            ),
            ("FORSEN".to_string(), vec![channel("twice", 1)]),
        ];
        let found = rank(&per_seed, Vec::<String>::new(), 10);
        assert_eq!(logins(&found), ["other", "twice"]);
        assert_eq!(found[1].because, ["forsen"]);
    }

    /// The first seed's details are the ones kept for a channel listed twice.
    #[test]
    fn a_shared_channel_keeps_the_first_seeds_details() {
        let mut later = channel("shared", 999);
        later.title = "later".into();
        let mut first = channel("shared", 5);
        first.title = "first".into();
        let per_seed = vec![
            ("a".to_string(), vec![first]),
            ("b".to_string(), vec![later]),
        ];
        let found = rank(&per_seed, Vec::<String>::new(), 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel.title, "first");
        assert_eq!(found[0].channel.viewer_count, 5);
    }

    #[test]
    fn the_limit_is_kept() {
        let per_seed = vec![(
            "seed".to_string(),
            (0..8).map(|n| channel(&format!("c{n}"), n)).collect(),
        )];
        let found = rank(&per_seed, Vec::<String>::new(), 3);
        assert_eq!(logins(&found), ["c7", "c6", "c5"]);
        assert!(rank(&per_seed, Vec::<String>::new(), 0).is_empty());
        assert!(rank(&[], Vec::<String>::new(), 5).is_empty());
    }

    /// The shape the website and Xtra send, and the same one for the whole
    /// process.
    #[test]
    fn the_device_id_is_32_hex_digits_made_once() {
        let id = device_id();
        assert_eq!(id.len(), 32);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(device_id(), id);
    }
}
