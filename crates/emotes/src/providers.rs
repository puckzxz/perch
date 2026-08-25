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
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::channel::mpsc as futures_mpsc;
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(20);

/// One provider's fetch. `None` for the global set, `Some(room_id)` for a
/// channel's. Returns name/URL pairs.
type LoadFn = fn(&ureq::Agent, Option<&str>) -> Result<Vec<(String, String)>, String>;

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
                        // Weakest provider first so stronger ones overwrite.
                        let loaders: [(Provider, LoadFn); 3] = [
                            (Provider::Ffz, load_ffz),
                            (Provider::Bttv, load_bttv),
                            (Provider::SevenTv, load_7tv),
                        ];

                        for (provider, load) in loaders {
                            match load(&agent, room.as_deref()) {
                                Ok(entries) if !entries.is_empty() => {
                                    sets.merge(entries);
                                    let _ = notify.unbounded_send(());
                                }
                                Ok(_) => {}
                                Err(e) => eprintln!("emotes: {provider:?} failed: {e}"),
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
