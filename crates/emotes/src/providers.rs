//! Third-party emote sets: FrankerFaceZ, BetterTTV and 7TV.
//!
//! These work differently from Twitch's own emotes. Twitch tells us exactly
//! where its emotes sit via the IRC `emotes` tag; third-party emotes have no
//! positional data at all, so they are name lookups applied to whatever text is
//! left over. That is why matching has to be word-exact — otherwise `LUL`
//! starts matching inside `LULW`.
//!
//! Each provider has a global set and a per-channel set keyed on the Twitch
//! room id, which the IRC tags already carry.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use futures::channel::mpsc as futures_mpsc;
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(20);

/// One provider's fetch. `None` for the global set, `Some(room_id)` for a
/// channel's. Returns name/URL pairs.
type LoadFn = fn(&ureq::Agent, Option<&str>) -> Result<Vec<(String, String)>, String>;

/// One provider's worth of name/URL pairs, tagged with which provider it came
/// from. A `Vec` of these is a fetch, in precedence order.
type Batch = (Provider, Vec<(String, String)>);

/// Which set won a given name. Later providers override earlier ones, so the
/// order here is also the precedence order, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Ffz,
    Bttv,
    SevenTv,
}

/// Name to image URL, shared between the loader thread and the UI.
#[derive(Clone, Default)]
pub struct EmoteSets {
    map: Arc<RwLock<HashMap<String, String>>>,
}

impl EmoteSets {
    pub fn lookup(&self, name: &str) -> Option<String> {
        self.map.read().unwrap().get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.map.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn merge(&self, entries: Vec<(String, String)>) {
        let mut map = self.map.write().unwrap();
        for (name, url) in entries {
            map.insert(name, url);
        }
    }
}

/// Loads emote sets in the background and reports when the table changes.
pub struct EmoteLoader {
    sets: EmoteSets,
    queue: mpsc::Sender<Option<String>>,
    _worker: std::thread::JoinHandle<()>,
}

impl EmoteLoader {
    /// Start the loader and immediately queue the global sets.
    ///
    /// The channel fires after each provider lands, so the UI can repaint
    /// progressively rather than waiting for all six requests.
    pub fn new() -> (Self, futures_mpsc::UnboundedReceiver<()>) {
        let sets = EmoteSets::default();
        let (queue, rx) = mpsc::channel::<Option<String>>();
        let (notify, notify_rx) = futures_mpsc::unbounded();

        let worker = std::thread::Builder::new()
            .name("emote-loader".into())
            .spawn({
                let sets = sets.clone();
                move || {
                    let agent: ureq::Agent = ureq::Agent::config_builder()
                        .timeout_global(Some(TIMEOUT))
                        .build()
                        .into();

                    while let Ok(room) = rx.recv() {
                        // Merged and announced as each provider lands, not once
                        // all three have. 7TV is by far the largest payload, so
                        // buffering until the slowest returned would mean the
                        // pane showed no third-party emotes at all until it did.
                        let mut deliver = |_provider: Provider, entries| {
                            sets.merge(entries);
                            let _ = notify.unbounded_send(());
                        };

                        match room.as_deref() {
                            // The same three sets for every channel, so fetched
                            // once for the whole process rather than once per
                            // pane. See [`global_sets`].
                            None => global_sets(&agent, &mut deliver),
                            channel => {
                                fetch_sets(&agent, channel, &mut deliver);
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn emote loader");

        let loader = Self {
            sets,
            queue,
            _worker: worker,
        };
        loader.load_global();
        (loader, notify_rx)
    }

    pub fn sets(&self) -> EmoteSets {
        self.sets.clone()
    }

    pub fn load_global(&self) {
        let _ = self.queue.send(None);
    }

    /// Queue this channel's sets. `room_id` is the numeric Twitch id from the
    /// `room-id` IRC tag, not the channel name.
    pub fn load_channel(&self, room_id: String) {
        let _ = self.queue.send(Some(room_id));
    }
}

/// Weakest provider first, so stronger ones overwrite when merged in order.
const LOADERS: [(Provider, LoadFn); 3] = [
    (Provider::Ffz, load_ffz),
    (Provider::Bttv, load_bttv),
    (Provider::SevenTv, load_7tv),
];

/// What one pass at the global endpoints produced.
struct GlobalCache {
    batches: Vec<Batch>,
    /// Whether every provider answered with something. A partial pass is still
    /// served rather than discarded — see [`global_sets`].
    complete: bool,
    fetched: Instant,
}

/// The global sets, as fetched. `None` until the first pass returns.
static GLOBAL_SETS: OnceLock<Mutex<Option<GlobalCache>>> = OnceLock::new();

/// How long a partial global pass is served before anyone tries again.
///
/// Retrying immediately is what makes the shared lock dangerous: with nothing
/// cached, every pane in turn takes the lock and spends its own three requests
/// on the same dead endpoint, so four panes serialise into four full timeouts
/// instead of overlapping. One retry per cooldown, process-wide, bounds that.
const GLOBAL_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Fetch every provider's set for `room`, handing each to `deliver` as it lands
/// rather than after the slowest one. Returns how many answered with entries.
///
/// Streaming rather than returning a `Vec` is the point: 7TV is by far the
/// biggest payload, and buffering meant a pane showed no third-party emotes at
/// all until it returned, however quickly FFZ had.
fn fetch_sets(
    agent: &ureq::Agent,
    room: Option<&str>,
    deliver: &mut impl FnMut(Provider, Vec<(String, String)>),
) -> usize {
    let mut answered = 0;
    for (provider, load) in LOADERS {
        match load(agent, room) {
            Ok(entries) if !entries.is_empty() => {
                answered += 1;
                deliver(provider, entries);
            }
            Ok(_) => {}
            Err(e) => eprintln!("emotes: {provider:?} failed: {e}"),
        }
    }
    answered
}

/// The global sets, fetched once per process and replayed to everyone after.
///
/// Every pane has its own [`EmoteLoader`], and each one used to fetch these
/// three endpoints for itself: ~147KB of identical JSON per pane (7TV alone is
/// 117KB), four times over at four panes, and again on every channel change.
/// They are the same for every channel and change on the order of weeks.
///
/// Replayed as raw entries rather than shared as one `EmoteSets`, so each pane
/// still merges into its own table — otherwise one channel's own emotes would
/// start resolving inside another channel's chat.
///
/// The lock is held across the fetch, so that only the first pane asks. What
/// makes that safe is that a *partial* pass is cached too: an empty result is
/// counted as a failure here (unlike a channel set, where a provider legitimately
/// has nothing, a 404 on a global endpoint is a bad edge, not an answer), but it
/// is still served for [`GLOBAL_RETRY_AFTER`] rather than sending the next pane
/// through the same three timeouts behind the same lock.
fn global_sets(agent: &ureq::Agent, deliver: &mut impl FnMut(Provider, Vec<(String, String)>)) {
    let cell = GLOBAL_SETS.get_or_init(|| Mutex::new(None));
    // A panic in another loader thread must not disable global emotes for the
    // rest of the session; the data behind the lock is still sound.
    let mut cached = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(entry) = cached.as_ref() {
        if entry.complete || entry.fetched.elapsed() < GLOBAL_RETRY_AFTER {
            for (provider, entries) in &entry.batches {
                deliver(*provider, entries.clone());
            }
            return;
        }
    }

    let mut batches = Vec::new();
    let answered = fetch_sets(agent, None, &mut |provider, entries| {
        batches.push((provider, entries.clone()));
        deliver(provider, entries);
    });

    *cached = Some(GlobalCache {
        batches,
        complete: answered == LOADERS.len(),
        fetched: Instant::now(),
    });
}

/// Fetch JSON, treating 404 as "this channel has no set here" rather than an
/// error: plenty of channels use one provider and not another.
fn get_json(agent: &ureq::Agent, url: &str) -> Result<Option<Value>, String> {
    let mut response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(format!("{url}: {e}")),
    };
    response
        .body_mut()
        .read_json::<Value>()
        .map(Some)
        .map_err(|e| format!("{url}: bad json: {e}"))
}

/// Providers sometimes return protocol-relative URLs (`//cdn.example/x`).
fn absolute(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

fn load_ffz(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let url = match room {
        Some(id) => format!("https://api.frankerfacez.com/v1/room/id/{id}"),
        None => "https://api.frankerfacez.com/v1/set/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    let Some(sets) = json.get("sets").and_then(Value::as_object) else {
        return Ok(out);
    };
    for set in sets.values() {
        let Some(emoticons) = set.get("emoticons").and_then(Value::as_array) else {
            continue;
        };
        for emote in emoticons {
            let Some(name) = emote.get("name").and_then(Value::as_str) else {
                continue;
            };
            let urls = emote.get("urls").and_then(Value::as_object);
            // FFZ keys sizes as "1", "2", "4"; prefer 2x, fall back to 1x.
            let url = urls
                .and_then(|u| u.get("2").or_else(|| u.get("1")))
                .and_then(Value::as_str);
            if let Some(url) = url {
                out.push((name.to_string(), absolute(url)));
            }
        }
    }
    Ok(out)
}

fn load_bttv(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let url = match room {
        Some(id) => format!("https://api.betterttv.net/3/cached/users/twitch/{id}"),
        None => "https://api.betterttv.net/3/cached/emotes/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };

    // Global returns a bare array; the channel endpoint splits emotes across
    // two keys.
    let mut lists: Vec<&Vec<Value>> = Vec::new();
    if let Some(array) = json.as_array() {
        lists.push(array);
    }
    for key in ["channelEmotes", "sharedEmotes"] {
        if let Some(array) = json.get(key).and_then(Value::as_array) {
            lists.push(array);
        }
    }

    let mut out = Vec::new();
    for list in lists {
        for emote in list {
            let (Some(id), Some(code)) = (
                emote.get("id").and_then(Value::as_str),
                emote.get("code").and_then(Value::as_str),
            ) else {
                continue;
            };
            out.push((
                code.to_string(),
                format!("https://cdn.betterttv.net/emote/{id}/2x"),
            ));
        }
    }
    Ok(out)
}

fn load_7tv(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let url = match room {
        Some(id) => format!("https://7tv.io/v3/users/twitch/{id}"),
        None => "https://7tv.io/v3/emote-sets/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };

    // The user endpoint nests the set; the global endpoint is the set itself.
    let emotes = json
        .get("emote_set")
        .and_then(|s| s.get("emotes"))
        .or_else(|| json.get("emotes"))
        .and_then(Value::as_array);
    let Some(emotes) = emotes else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for emote in emotes {
        let Some(name) = emote.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(host) = emote.get("data").and_then(|d| d.get("host")) else {
            continue;
        };
        let Some(base) = host.get("url").and_then(Value::as_str) else {
            continue;
        };

        // Prefer WebP: it is what 7TV always ships, and the image crate reads
        // static WebP. Animated WebP may render as a still, which beats nothing.
        let file = host
            .get("files")
            .and_then(Value::as_array)
            .and_then(|files| {
                files
                    .iter()
                    .filter_map(|f| f.get("name").and_then(Value::as_str))
                    .find(|n| n.starts_with("2x") && n.ends_with(".webp"))
            })
            .unwrap_or("2x.webp");

        out.push((name.to_string(), format!("{}/{file}", absolute(base))));
    }
    Ok(out)
}
