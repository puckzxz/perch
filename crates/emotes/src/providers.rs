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
//!
//! Each emote also has a [`Kind`], read from its provider's answer: most are
//! pictures of their own, and some are laid over the emote beside them or
//! change it (see `stack`). And FrankerFaceZ's global answer is cut to the
//! sets it names under `default_sets`: the rest belong to the people listed
//! under `users`, its supporters' effects among them.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use futures::channel::mpsc as futures_mpsc;
use serde_json::Value;

use crate::tokenize::{Kind, Named};

const TIMEOUT: Duration = Duration::from_secs(20);

/// One emote as a provider's answer gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    url: String,
    kind: Kind,
}

/// One provider's fetch. `None` for the global set, `Some(room_id)` for a
/// channel's.
type LoadFn = fn(&ureq::Agent, Option<&str>) -> Result<Vec<Entry>, String>;

/// One provider's worth of emotes, tagged with which provider it came from.
/// A `Vec` of these is a fetch, in precedence order.
type Batch = (Provider, Vec<Entry>);

/// Whose emote it is. For the third-party sets, which one won a given name:
/// later providers override earlier ones, so the order here is also the
/// precedence order, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Ffz,
    Bttv,
    SevenTv,
    /// Twitch's own. Placed by the `emotes` tag at the positions Twitch
    /// gives, never looked up by name, and so never overridden by the sets
    /// above.
    Twitch,
}

/// Name to emote, shared between the loader thread and the UI.
#[derive(Clone, Default)]
pub struct EmoteSets {
    map: Arc<RwLock<HashMap<String, Named>>>,
}

impl EmoteSets {
    pub fn lookup(&self, name: &str) -> Option<Named> {
        self.map.read().unwrap().get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.map.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add `provider`'s emotes, each one replacing whatever had its name.
    fn merge(&self, provider: Provider, entries: Vec<Entry>) {
        let mut map = self.map.write().unwrap();
        for Entry { name, url, kind } in entries {
            map.insert(
                name,
                Named {
                    url,
                    provider,
                    kind,
                },
            );
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
                        let mut deliver = |provider: Provider, entries| {
                            sets.merge(provider, entries);
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
    deliver: &mut impl FnMut(Provider, Vec<Entry>),
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
fn global_sets(agent: &ureq::Agent, deliver: &mut impl FnMut(Provider, Vec<Entry>)) {
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

fn load_ffz(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<Entry>, String> {
    let url = match room {
        Some(id) => format!("https://api.frankerfacez.com/v1/room/id/{id}"),
        None => "https://api.frankerfacez.com/v1/set/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };
    ffz_entries(&json, room.is_none())
}

/// FrankerFaceZ's flag on a modifier that is an effect rather than a
/// picture: the first of its `MODIFIER_FLAGS` (Hidden, then FlipX, FlipY,
/// GrowX and on), in `src/modules/chat/emotes.js`, read on 14 October 2026.
/// Every effect in its global answer that day has it: `ffzX` is 3, `ffzW` 9,
/// `ffzCursed` 16385.
const FFZ_HIDDEN: u64 = 1;

/// The emotes in a FrankerFaceZ answer: every set in a room's, and in the
/// global one (`global`), only the sets it names under `default_sets`.
///
/// The global answer also carries sets that FrankerFaceZ gives only to the
/// logins listed under `users`, its supporters' effects among them
/// (`ffzHyper`, `ffzSpin` and six more). Merged whole, those were pictures
/// for everyone. A global answer without `default_sets` is an error, never a
/// reason to take every set: the provider is retried, as a failed one is.
fn ffz_entries(json: &Value, global: bool) -> Result<Vec<Entry>, String> {
    let defaults: Option<Vec<String>> = if global {
        let ids = json
            .get("default_sets")
            .and_then(Value::as_array)
            .ok_or("no default_sets")?;
        Some(
            ids.iter()
                .filter_map(Value::as_u64)
                .map(|id| id.to_string())
                .collect(),
        )
    } else {
        None
    };

    let mut out = Vec::new();
    let Some(sets) = json.get("sets").and_then(Value::as_object) else {
        return Ok(out);
    };
    for (key, set) in sets {
        if defaults.as_ref().is_some_and(|ids| !ids.contains(key)) {
            continue;
        }
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
                out.push(Entry {
                    name: name.to_string(),
                    url: absolute(url),
                    kind: ffz_kind(emote),
                });
            }
        }
    }
    Ok(out)
}

/// A FrankerFaceZ emote's kind: a modifier with the Hidden flag is an effect
/// on the emote before it, one without is laid over it, and the rest are
/// pictures of their own.
fn ffz_kind(emote: &Value) -> Kind {
    let modifier = emote.get("modifier").and_then(Value::as_bool) == Some(true);
    let flags = emote
        .get("modifier_flags")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    match (modifier, flags & FFZ_HIDDEN != 0) {
        (false, _) => Kind::Normal,
        (true, true) => Kind::EffectOnPrevious,
        (true, false) => Kind::Overlay,
    }
}

fn load_bttv(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<Entry>, String> {
    let url = match room {
        Some(id) => format!("https://api.betterttv.net/3/cached/users/twitch/{id}"),
        None => "https://api.betterttv.net/3/cached/emotes/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };
    Ok(bttv_entries(&json, room.is_none()))
}

/// BetterTTV's zero-width emotes, which its answer does not mark: a fixed
/// list of global codes, the one Chatterino keeps (`ZERO_WIDTH_EMOTES`, in
/// `src/providers/bttv/BttvEmotes.cpp`) and 7TV's extension too
/// (`BTTV_ZeroWidth`, `src/common/Transform.ts`), read on 14 October 2026.
/// Global emotes only, as both apply it: a channel's own emote with one of
/// these codes is a picture of its own. Only `cvMask` and `cvHazmat` were in
/// the global answer that day; the rest come and go with the seasons.
const BTTV_ZERO_WIDTH: [&str; 8] = [
    "SoSnowy",
    "IceCold",
    "SantaHat",
    "TopHat",
    "ReinDeer",
    "CandyCane",
    "cvMask",
    "cvHazmat",
];

/// The emotes in a BetterTTV answer, the global one (`global`) or a
/// channel's. Its `modifier` emotes (`w!`, `h!` and the rest) are effects
/// on the emote after them.
fn bttv_entries(json: &Value, global: bool) -> Vec<Entry> {
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
            let kind = if emote.get("modifier").and_then(Value::as_bool) == Some(true) {
                Kind::EffectOnNext
            } else if global && BTTV_ZERO_WIDTH.contains(&code) {
                Kind::Overlay
            } else {
                Kind::Normal
            };
            out.push(Entry {
                name: code.to_string(),
                url: format!("https://cdn.betterttv.net/emote/{id}/2x"),
                kind,
            });
        }
    }
    out
}

fn load_7tv(agent: &ureq::Agent, room: Option<&str>) -> Result<Vec<Entry>, String> {
    let url = match room {
        Some(id) => format!("https://7tv.io/v3/users/twitch/{id}"),
        None => "https://7tv.io/v3/emote-sets/global".to_string(),
    };
    let Some(json) = get_json(agent, &url)? else {
        return Ok(Vec::new());
    };
    Ok(seventv_entries(&json))
}

/// 7TV's zero-width flag, which it keeps in two places: on the set's entry
/// (`flags`, bit 0, what Chatterino reads) and on the emote itself
/// (`data.flags`, bit 8, what 7TV's extension and BetterTTV read). The two
/// agreed on every entry seen on 14 October 2026, and either one is taken.
const SEVENTV_ENTRY_ZERO_WIDTH: u64 = 1;
const SEVENTV_EMOTE_ZERO_WIDTH: u64 = 1 << 8;

/// The emotes in a 7TV answer, a channel's or the global set.
fn seventv_entries(json: &Value) -> Vec<Entry> {
    // The user endpoint nests the set; the global endpoint is the set itself.
    let emotes = json
        .get("emote_set")
        .and_then(|s| s.get("emotes"))
        .or_else(|| json.get("emotes"))
        .and_then(Value::as_array);
    let Some(emotes) = emotes else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for emote in emotes {
        let Some(name) = emote.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(data) = emote.get("data") else {
            continue;
        };
        let Some(host) = data.get("host") else {
            continue;
        };
        let Some(base) = host.get("url").and_then(Value::as_str) else {
            continue;
        };

        // Prefer WebP: it is what 7TV always ships. An animated one animates:
        // gpui decodes every frame of it (`img.rs`, the WebP arm) and steps
        // through them as it does a GIF's, for any image with an element id.
        // Checked on 14 October 2026 by decoding 7TV's PepePls with the
        // image crate gpui uses (40 frames, 40 ms apart).
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

        let flags = |value: &Value| value.get("flags").and_then(Value::as_u64).unwrap_or(0);
        let zero_width = flags(emote) & SEVENTV_ENTRY_ZERO_WIDTH != 0
            || flags(data) & SEVENTV_EMOTE_ZERO_WIDTH != 0;
        out.push(Entry {
            name: name.to_string(),
            url: format!("{}/{file}", absolute(base)),
            kind: if zero_width {
                Kind::Overlay
            } else {
                Kind::Normal
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FrankerFaceZ's global answer as sent on 14 October 2026, trimmed: its
    /// `default_sets`, one emote of each set (two of the effects), and its
    /// `users` with two of the logins replaced by stand-ins.
    const FFZ_GLOBAL: &str = include_str!("fixtures/ffz-global.json");
    /// BetterTTV's global answer as sent on 14 October 2026, trimmed to
    /// `SourPls`, `cvMask` and `w!`.
    const BTTV_GLOBAL: &str = include_str!("fixtures/bttv-global.json");
    /// 7TV's global set as sent on 14 October 2026, trimmed to `PETPET`, a
    /// zero-width emote, and `EZ`, each with its 2x WebP alone.
    const SEVENTV_GLOBAL: &str = include_str!("fixtures/7tv-global.json");

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The kind `entries` give `name`, or `None` when it is not there.
    fn kind_of(entries: &[Entry], name: &str) -> Option<Kind> {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    }

    #[test]
    fn ffz_globals_are_its_default_sets_only() {
        let entries = ffz_entries(&json(FFZ_GLOBAL), true).unwrap();
        assert_eq!(kind_of(&entries, "LilZ"), Some(Kind::Normal));
        assert_eq!(kind_of(&entries, "ffzX"), Some(Kind::EffectOnPrevious));
        assert_eq!(kind_of(&entries, "ffzW"), Some(Kind::EffectOnPrevious));
        // Set 1532818, given only to the logins under `users`.
        assert_eq!(kind_of(&entries, "ffzHyper"), None);
        assert_eq!(entries.len(), 3);
        let lilz = entries.iter().find(|entry| entry.name == "LilZ").unwrap();
        assert_eq!(lilz.url, "https://cdn.frankerfacez.com/emote/28136/2");
    }

    #[test]
    fn an_ffz_global_answer_without_default_sets_is_an_error() {
        let mut answer = json(FFZ_GLOBAL);
        answer.as_object_mut().unwrap().remove("default_sets");
        assert!(ffz_entries(&answer, true).is_err());
    }

    /// A room's answer has no `default_sets`, and every set in it is the
    /// room's.
    #[test]
    fn an_ffz_room_keeps_every_set() {
        let room = serde_json::json!({
            "room": { "set": 1 },
            "sets": {
                "1": { "emoticons": [{ "name": "OneEmote", "urls": { "1": "//cdn.example/1/1" } }] },
                "2": { "emoticons": [{ "name": "TwoEmote", "urls": { "2": "//cdn.example/2/2" } }] }
            }
        });
        let entries = ffz_entries(&room, false).unwrap();
        assert_eq!(kind_of(&entries, "OneEmote"), Some(Kind::Normal));
        assert_eq!(kind_of(&entries, "TwoEmote"), Some(Kind::Normal));
        assert_eq!(entries[0].url, "https://cdn.example/1/1");
    }

    #[test]
    fn an_ffz_modifier_is_an_overlay_unless_hidden() {
        let emote = |modifier: bool, flags: u64| serde_json::json!({ "modifier": modifier, "modifier_flags": flags });
        assert_eq!(ffz_kind(&emote(false, 0)), Kind::Normal);
        assert_eq!(ffz_kind(&emote(false, 3)), Kind::Normal);
        assert_eq!(ffz_kind(&emote(true, 0)), Kind::Overlay);
        assert_eq!(ffz_kind(&emote(true, 3)), Kind::EffectOnPrevious);
        assert_eq!(ffz_kind(&serde_json::json!({})), Kind::Normal);
    }

    #[test]
    fn bttv_modifiers_are_effects_on_the_next_emote() {
        let entries = bttv_entries(&json(BTTV_GLOBAL), true);
        assert_eq!(kind_of(&entries, "w!"), Some(Kind::EffectOnNext));
        assert_eq!(kind_of(&entries, "SourPls"), Some(Kind::Normal));
    }

    /// `cvMask` is laid over the emote before it in the global set, and a
    /// channel's own `cvMask` is a picture like any other.
    #[test]
    fn bttv_zero_width_is_a_fixed_list_of_globals() {
        let entries = bttv_entries(&json(BTTV_GLOBAL), true);
        assert_eq!(kind_of(&entries, "cvMask"), Some(Kind::Overlay));

        let channel = serde_json::json!({
            "channelEmotes": [{ "id": "a1", "code": "cvMask", "modifier": false }],
            "sharedEmotes": [{ "id": "b2", "code": "catJAM" }]
        });
        let entries = bttv_entries(&channel, false);
        assert_eq!(kind_of(&entries, "cvMask"), Some(Kind::Normal));
        assert_eq!(kind_of(&entries, "catJAM"), Some(Kind::Normal));
    }

    #[test]
    fn seventv_zero_width_is_either_flag() {
        let entries = seventv_entries(&json(SEVENTV_GLOBAL));
        assert_eq!(kind_of(&entries, "PETPET"), Some(Kind::Overlay));
        assert_eq!(kind_of(&entries, "EZ"), Some(Kind::Normal));
        let ez = entries.iter().find(|entry| entry.name == "EZ").unwrap();
        assert_eq!(
            ez.url,
            "https://cdn.7tv.app/emote/01K6WWG0R4T5A9FVNN4TWCWXA7/2x.webp"
        );

        let channel = |entry_flags: Option<u64>, emote_flags: Option<u64>| {
            let mut entry = serde_json::json!({
                "name": "Layer",
                "data": { "host": { "url": "//cdn.example/layer" } }
            });
            if let Some(flags) = entry_flags {
                entry["flags"] = flags.into();
            }
            if let Some(flags) = emote_flags {
                entry["data"]["flags"] = flags.into();
            }
            let set = serde_json::json!({ "emote_set": { "emotes": [entry] } });
            kind_of(&seventv_entries(&set), "Layer")
        };
        assert_eq!(channel(Some(1), Some(0)), Some(Kind::Overlay));
        assert_eq!(channel(Some(0), Some(256)), Some(Kind::Overlay));
        assert_eq!(channel(None, None), Some(Kind::Normal));
    }

    /// A name two providers both have is the stronger one's, picture, provider
    /// and kind together.
    #[test]
    fn a_name_keeps_the_provider_and_kind_that_won() {
        let sets = EmoteSets::default();
        let entry = |url: &str, kind| Entry {
            name: "Clash".into(),
            url: url.into(),
            kind,
        };
        sets.merge(Provider::Ffz, vec![entry("ffz", Kind::Normal)]);
        sets.merge(Provider::SevenTv, vec![entry("7tv", Kind::Overlay)]);
        assert_eq!(
            sets.lookup("Clash"),
            Some(Named {
                url: "7tv".into(),
                provider: Provider::SevenTv,
                kind: Kind::Overlay,
            })
        );
        assert_eq!(sets.lookup("Other"), None);
    }
}
