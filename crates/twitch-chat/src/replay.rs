//! The chat that was happening while a recording was made.
//!
//! Twitch keeps a recording's chat and serves it to its own website through
//! a GraphQL query it does not publish: `VideoCommentsByOffsetOrCursor`, asked
//! by offset for the first page and by cursor for the rest. It answers
//! anonymously, about fifty comments a page in the order they were said, each
//! with who said it, their colour, the text in fragments with Twitch's own
//! emotes marked, and when it was really said. TwitchDownloader has depended
//! on the same query since 2023, which is as much of a contract as an
//! unpublished query gets; if it goes, the pane shows one notice and the
//! picture keeps playing.
//!
//! This is [`history`](crate::history) in reverse: rather than a third party
//! for what was said before you joined, it is Twitch for what was said at the
//! moment the picture shows. A thread owns the fetch and a buffer, reads the
//! recording's position a few times a second, keeps a minute of comments
//! ahead of it, and emits each as a [`ChatEvent::Message`] as the position
//! passes it. Every comment becomes an ordinary [`ChatMessage`], so everything
//! downstream — the tokenizer, the emote cache, the mention colours, the
//! minute breaks — is the code the live pane already runs.
//!
//! A seek is a discontinuity: the position moved further than time did. The
//! buffer is thrown away and the fetch starts over at the new offset, with
//! the [`PREFILL`] seconds before it delivered first so the pane lands in a
//! conversation rather than blank. A pause simply stops the position. Speed
//! is always one.
//!
//! Three facts about the endpoint that cost time to find:
//!
//! - **Cursor paging fails Twitch's integrity check on the website's own
//!   Client-ID** (`kimne78kx3ncx6brgo4mv6wki5h1ko`, `IntegrityCheckFailed`)
//!   and works on the mobile one below, which TwitchDownloader has used since
//!   May 2023. Should that change, asking again by the last comment's offset
//!   and dropping duplicates by id is gap-free — the page for offset N starts
//!   a few seconds before N, measured — and is what [`Source::next`] falls
//!   back to.
//! - **The emote positions Twitch sends count UTF-8 bytes**, not characters:
//!   after `🎣 ` a fragment says its emote starts at 5 where the IRC tag
//!   would say 2. They are ignored, and the `emotes` tag the chat view reads
//!   is built by counting characters across the fragments instead.
//! - **A sub notice is a plain comment** whose text is Twitch's sentence
//!   followed by the user's note, with nothing to mark it. It is rendered as
//!   the message it has become. No `/me` marker was seen either.
//!
//! An offset past the end answers `comments: null` beside a "service error";
//! a highlight or an upload answers an empty list — a highlight is cut from
//! ranges of a broadcast and its offsets mean nothing — and a video that does
//! not exist answers `video: null`. Replay exists for a broadcast still being
//! recorded, about thirty seconds behind live.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use serde_json::{json, Value};

use crate::history::USER_AGENT;
use crate::message::{self, ChatMessage};
use crate::ChatEvent;

const ENDPOINT: &str = "https://gql.twitch.tv/gql";
/// Twitch's mobile Client-ID. See the module docs for why not the website's.
const CLIENT_ID: &str = "kd1unb4b3q4t58fwlpcbzcbnm76a8fp";
const QUERY: &str = "VideoCommentsByOffsetOrCursor";
const QUERY_HASH: &str = "b70a3591ff0f4e0313d126c6a1502d79a1c02baebb288227c582044aa76adf6a";
const TIMEOUT: Duration = Duration::from_secs(10);

/// How far ahead of the position the buffer is kept. A page is a few seconds
/// of a busy chat, so this is several pages; a quiet one's covers it many
/// times over.
const AHEAD: f64 = 60.0;
/// What is shown from before a position the replay lands on, so a seek
/// arrives in a conversation rather than an empty pane.
const PREFILL: f64 = 20.0;
/// Pages the prefill walks before giving up on reaching the target through
/// them and asking for the target outright: twenty seconds of a busy chat
/// is more pages than a seek should wait for.
const PREFILL_PAGES: usize = 3;
/// How often the position is read. An atomic load, so it costs nothing; the
/// figure is the granularity of when a line appears.
const POLL: Duration = Duration::from_millis(100);
/// The least time between one refill request and the next, so catching up
/// after a seek is a request or two a second rather than a burst. Thirty
/// back to back were answered without complaint; there is no need to find
/// out where the limit is.
const PACE: Duration = Duration::from_millis(500);
/// How often to ask again once the comments have run out, for a broadcast
/// still being recorded, whose replay grows about thirty seconds behind live.
const PROBE: Duration = Duration::from_secs(10);
/// The position never goes backwards on its own, so a step back beyond this
/// is a seek; the slack is for the moment a reposition lands a little short.
const BACK_SLACK: f64 = 1.0;
/// A step forward beyond the time that passed, plus this, is a seek. The slack
/// covers the poll interval and the player reporting in bursts.
const FORWARD_SLACK: f64 = 2.0;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// One line of the replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    /// Whole seconds into the recording. Twitch keeps nothing finer; see
    /// [`Schedule::release`] for how a busy second is spread out.
    pub offset: u64,
    pub message: ChatMessage,
}

/// A page of comments, oldest first, and how to ask for the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub comments: Vec<Comment>,
    /// The cursor for the page after this one; `None` at the end.
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Nothing to retry: the video is not on Twitch.
    Unavailable(String),
    /// Worth asking again: the network, or Twitch, did not answer properly.
    Transient(String),
}

/// A replay running against one recording. Dropping it stops the thread.
pub struct Replay {
    stop: Arc<AtomicBool>,
}

impl Replay {
    /// Replay `video_id`'s chat against `position`, which answers with the
    /// seconds into the recording the picture is showing — read a few times a
    /// second, from whatever thread this one is.
    ///
    /// `room_id` is the channel's numeric id, announced first so third-party
    /// emote sets load the way they do for a live pane; `channel` is the
    /// login, for [`ChatEvent::Connected`].
    pub fn start(
        video_id: String,
        channel: String,
        room_id: String,
        position: impl Fn() -> f64 + Send + 'static,
    ) -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let spawned = std::thread::Builder::new()
            .name("chat-replay".into())
            .spawn({
                let stop = stop.clone();
                move || run(video_id, channel, room_id, position, tx, stop)
            });
        if let Err(e) = spawned {
            eprintln!("chat replay: could not start: {e}");
        }
        (Self { stop }, rx)
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        // Not joined, for the reason `ChatClient` is not: the thread may be
        // inside a request, and this runs on the UI thread as a pane closes.
        // It tests `stop` between polls and between pages and retires.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The whole replay, from the first page until the pane closes.
fn run(
    video_id: String,
    channel: String,
    room_id: String,
    position: impl Fn() -> f64,
    tx: mpsc::UnboundedSender<ChatEvent>,
    stop: Arc<AtomicBool>,
) {
    // False once nobody is listening, which is the other way this ends.
    let send = |event: ChatEvent| tx.unbounded_send(event).is_ok();
    if !send(ChatEvent::RoomState { room_id }) {
        return;
    }

    let mut source = Source::new(video_id);
    let mut schedule = Schedule::default();
    // The position last seen and when, for telling a seek from playback.
    let mut last: Option<(Instant, f64)> = None;
    // Where the buffer was last aligned to, and whether it needs to be again.
    let mut aligned = position();
    let mut load_needed = true;
    let mut first_load = true;
    // Failures: said once per streak, retried with a growing wait.
    let mut failing = false;
    let mut backoff = Duration::from_secs(1);
    let mut retry_after = Instant::now();
    let mut last_request = Instant::now() - PACE;
    let mut last_probe = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let at = position();
        if let Some((then, before)) = last {
            if is_seek(before, now.duration_since(then), at) {
                load_needed = true;
                aligned = at;
            }
        }
        last = Some((now, at));

        if load_needed && now >= retry_after {
            match load(&mut source, &mut schedule, aligned) {
                Ok(backlog) => {
                    // The reset waits for the backlog to be in hand, so a seek
                    // swaps one conversation for another rather than blanking
                    // the pane for the round trip in between.
                    if !first_load && !send(ChatEvent::Reset) {
                        return;
                    }
                    for comment in backlog {
                        if !send(ChatEvent::Message(Box::new(comment.message))) {
                            return;
                        }
                    }
                    if !send(ChatEvent::Connected {
                        channel: channel.clone(),
                    }) {
                        return;
                    }
                    // Nothing at the start and nothing after it: this
                    // broadcast keeps no chat. Said once, and then there is
                    // nothing left for this thread to do.
                    if first_load && aligned < 1.0 && schedule.is_empty() && source.exhausted {
                        send(ChatEvent::Unavailable {
                            reason: "no chat replay for this broadcast".into(),
                        });
                        return;
                    }
                    first_load = false;
                    load_needed = false;
                    failing = false;
                    backoff = Duration::from_secs(1);
                }
                Err(Error::Unavailable(reason)) => {
                    send(ChatEvent::Unavailable { reason });
                    return;
                }
                Err(Error::Transient(reason)) => {
                    if !failing {
                        failing = true;
                        if !send(ChatEvent::Disconnected { reason }) {
                            return;
                        }
                    }
                    retry_after = now + backoff;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }

        for comment in schedule.due(at) {
            if !send(ChatEvent::Message(Box::new(comment.message))) {
                return;
            }
        }

        // Keep a minute ahead: the next page while there is one, and once
        // there is not, a look every so often at whether more has appeared,
        // which for a broadcast still being recorded it will have.
        let behind = schedule.end().is_none_or(|end| (end as f64) < at + AHEAD);
        if !load_needed && behind && now >= retry_after && now.duration_since(last_request) >= PACE
        {
            let fetched = if !source.exhausted {
                last_request = now;
                source.next().map(|page| page.map(|page| page.comments))
            } else if now.duration_since(last_probe) >= PROBE {
                last_request = now;
                last_probe = now;
                let from = source.last_offset.unwrap_or(at as u64);
                source.at(from).map(|page| Some(page.comments))
            } else {
                Ok(None)
            };
            match fetched {
                Ok(Some(comments)) => {
                    schedule.push(comments);
                    failing = false;
                    backoff = Duration::from_secs(1);
                }
                Ok(None) => {}
                Err(Error::Unavailable(reason)) => {
                    send(ChatEvent::Unavailable { reason });
                    return;
                }
                Err(Error::Transient(reason)) => {
                    if !failing {
                        failing = true;
                        if !send(ChatEvent::Disconnected { reason }) {
                            return;
                        }
                    }
                    retry_after = now + backoff;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }

        let deadline = now + POLL;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Start the buffer over at `target`. What comes back is the backlog: what
/// was said in the [`PREFILL`] seconds before it, oldest first; the buffer
/// holds what comes after.
fn load(source: &mut Source, schedule: &mut Schedule, target: f64) -> Result<Vec<Comment>, Error> {
    schedule.clear();
    let from = (target - PREFILL).max(0.0) as u64;
    let mut page = source.at(from)?;
    let mut walked = 1;
    loop {
        let reached = page.next.is_none()
            || page
                .comments
                .last()
                .is_some_and(|comment| comment.offset as f64 > target);
        schedule.push(page.comments);
        if reached || walked >= PREFILL_PAGES {
            break;
        }
        match source.next()? {
            Some(next) => {
                page = next;
                walked += 1;
            }
            None => break,
        }
    }
    // A busy chat: the pages did not reach the target, so the rest of the way
    // is one ask at the target itself. Whatever it repeats is dropped by id.
    let reached = schedule.end().is_some_and(|end| end as f64 > target);
    if !reached && !source.exhausted {
        let page = source.at(target as u64)?;
        schedule.push(page.comments);
    }
    Ok(schedule.due(target))
}

/// Whether the position moved further than time did: a seek, rather than
/// playback or a pause.
fn is_seek(before: f64, elapsed: Duration, after: f64) -> bool {
    after < before - BACK_SLACK || after > before + elapsed.as_secs_f64() + FORWARD_SLACK
}

/// One video's comments, a page at a time.
struct Source {
    agent: ureq::Agent,
    video_id: String,
    /// How to ask for the page after the last one, while there is one.
    cursor: Option<String>,
    /// The offset of the last comment received: where to ask again from when
    /// a cursor is refused, and where to look for more once they ran out.
    last_offset: Option<u64>,
    /// Whether Twitch has said there is no page after the last one.
    exhausted: bool,
}

impl Source {
    fn new(video_id: String) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            // The reason for a refusal is in the body — the same trap
            // `history` and `twitch-api` document.
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self {
            agent,
            video_id,
            cursor: None,
            last_offset: None,
            exhausted: false,
        }
    }

    /// The page holding `offset`, which starts a few seconds before it.
    /// Paging continues from there.
    fn at(&mut self, offset: u64) -> Result<Page, Error> {
        let page = self.request(json!({
            "videoID": self.video_id,
            "contentOffsetSeconds": offset,
        }))?;
        self.absorb(&page);
        Ok(page)
    }

    /// The page after the last one, or `None` once there is no more. Asked by
    /// cursor; a cursor Twitch will not take is answered by asking from the
    /// last offset instead, which repeats a few seconds the caller drops.
    fn next(&mut self) -> Result<Option<Page>, Error> {
        let Some(cursor) = self.cursor.clone() else {
            return Ok(None);
        };
        let page = match self.request(json!({
            "videoID": self.video_id,
            "cursor": cursor,
        })) {
            Ok(page) => page,
            Err(Error::Transient(reason)) => {
                eprintln!("chat replay: cursor refused ({reason}); asking by offset instead");
                self.request(json!({
                    "videoID": self.video_id,
                    "contentOffsetSeconds": self.last_offset.unwrap_or(0),
                }))?
            }
            Err(other) => return Err(other),
        };
        self.absorb(&page);
        Ok(Some(page))
    }

    fn absorb(&mut self, page: &Page) {
        self.cursor = page.next.clone();
        self.exhausted = page.next.is_none();
        if let Some(last) = page.comments.last() {
            self.last_offset = Some(last.offset);
        }
    }

    fn request(&self, variables: Value) -> Result<Page, Error> {
        let body = json!([{
            "operationName": QUERY,
            "variables": variables,
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": QUERY_HASH } },
        }]);
        let mut response = self
            .agent
            .post(ENDPOINT)
            .header("Client-ID", CLIENT_ID)
            .send_json(&body)
            .map_err(|e| Error::Transient(e.to_string()))?;
        let status = response.status().as_u16();
        let json: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| Error::Transient(format!("unreadable answer: {e}")))?;
        if status >= 400 {
            let reason = json
                .get("message")
                .and_then(Value::as_str)
                .map(|m| format!("{m} (HTTP {status})"))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(Error::Transient(reason));
        }
        parse_page(&json)
    }
}

/// What one answer means. Public so the shape can be tested against a page
/// as Twitch actually sends one.
pub fn parse_page(body: &Value) -> Result<Page, Error> {
    // One element per operation in the request, and there was one.
    let response = body.get(0).unwrap_or(body);
    let video = match response.pointer("/data/video") {
        Some(Value::Null) => {
            return Err(Error::Unavailable("this recording is not on Twitch".into()))
        }
        Some(video) => video,
        // No data at all: the request itself was refused, and the reason is
        // in `errors` — an integrity check, an unknown query hash.
        None => {
            let reason = response
                .pointer("/errors/0/message")
                .and_then(Value::as_str)
                .unwrap_or("Twitch answered in an unexpected shape");
            return Err(Error::Transient(reason.to_string()));
        }
    };
    let comments = match video.get("comments") {
        // Past the end of the chat: a "service error" beside a null. Nothing
        // there, and nothing after it.
        Some(Value::Null) | None => {
            return Ok(Page {
                comments: Vec::new(),
                next: None,
            })
        }
        Some(comments) => comments,
    };
    let edges = comments
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Transient("the comments came in an unexpected shape".into()))?;
    let has_next = comments
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next = if has_next {
        edges
            .last()
            .and_then(|edge| edge.get("cursor"))
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    };
    let comments = edges
        .iter()
        .filter_map(|edge| edge.get("node"))
        .filter_map(comment)
        .collect();
    Ok(Page { comments, next })
}

/// One node as a line, or `None` for one that cannot be shown.
fn comment(node: &Value) -> Option<Comment> {
    let id = node.get("id")?.as_str()?.to_string();
    let offset = node.get("contentOffsetSeconds")?.as_u64()?;
    // A deleted account leaves a null commenter. Skipped: a line with nobody
    // saying it is not a chat message.
    let commenter = node.get("commenter").filter(|c| !c.is_null())?;
    let login = commenter.get("login")?.as_str()?.to_string();
    let display_name = commenter
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(&login)
        .to_string();
    let message = node.get("message")?;
    let (text, emotes) = assemble(message.get("fragments").and_then(Value::as_array)?);
    let color = message
        .get("userColor")
        .and_then(Value::as_str)
        .and_then(message::parse_hex_color)
        .unwrap_or_else(|| message::fallback_color(&login));
    let sent_at = node
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(unix_millis);
    Some(Comment {
        id,
        offset,
        message: ChatMessage {
            login,
            display_name,
            color,
            text,
            is_action: false,
            emotes,
            sent_at,
        },
    })
}

/// The text of a comment and its `emotes` tag, in the IRC form the chat view
/// reads: `id:start-end` per emote, inclusive, in **characters**, joined by
/// `/`. Twitch's own positions count bytes and are not used.
fn assemble(fragments: &[Value]) -> (String, Option<String>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut at = 0usize;
    for fragment in fragments {
        let piece = fragment
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let len = piece.chars().count();
        let emote = fragment
            .get("emote")
            .filter(|emote| !emote.is_null())
            .and_then(|emote| emote.get("emoteID"))
            .and_then(Value::as_str);
        if let (Some(id), true) = (emote, len > 0) {
            spans.push(format!("{id}:{at}-{}", at + len - 1));
        }
        text.push_str(piece);
        at += len;
    }
    let tag = (!spans.is_empty()).then(|| spans.join("/"));
    (text, tag)
}

/// `2026-09-08T14:01:56.719Z` as Unix milliseconds, which is what
/// `tmi-sent-ts` carries and what the stamps are made from.
fn unix_millis(rfc3339: &str) -> Option<u64> {
    let when = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?;
    u64::try_from(when.timestamp_millis()).ok()
}

/// Ids of lines already said, kept so a page that repeats them — the offset
/// fallback, the probe at the end — does not say them twice. Bounded: an
/// overlap is a page or so, and a six-hour chat is a great many ids.
const RECENT: usize = 256;

/// What has been fetched and not yet said, in order, and when each is due.
#[derive(Default)]
struct Schedule {
    pending: VecDeque<Comment>,
    /// Every id pending, plus the last [`RECENT`] said.
    seen: HashSet<String>,
    recent: VecDeque<String>,
    /// The second the last line said fell in, and how many of that second
    /// have been said, so the rest of it is spread over what remains.
    head: Option<(u64, usize)>,
}

impl Schedule {
    fn clear(&mut self) {
        self.pending.clear();
        self.seen.clear();
        self.recent.clear();
        self.head = None;
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The offset of the last line buffered.
    fn end(&self) -> Option<u64> {
        self.pending.back().map(|comment| comment.offset)
    }

    /// Add a page, dropping what has been seen. Pages arrive in order, but an
    /// ask by offset overlaps the pages before it, so the buffer is kept
    /// sorted rather than trusted.
    fn push(&mut self, comments: Vec<Comment>) {
        let mut added = false;
        for comment in comments {
            if self.seen.insert(comment.id.clone()) {
                self.pending.push_back(comment);
                added = true;
            }
        }
        if added {
            self.pending
                .make_contiguous()
                .sort_by_key(|comment| (comment.offset, comment.message.sent_at));
        }
    }

    /// Everything due at `position` seconds, oldest first, taken out of the
    /// buffer.
    fn due(&mut self, position: f64) -> Vec<Comment> {
        let mut out = Vec::new();
        while let Some(front) = self.pending.front() {
            if self.release(front.offset) > position {
                break;
            }
            let comment = self.pending.pop_front().expect("front was just there");
            self.head = match self.head {
                Some((second, said)) if second == comment.offset => Some((second, said + 1)),
                _ => Some((comment.offset, 1)),
            };
            self.recent.push_back(comment.id.clone());
            if self.recent.len() > RECENT {
                if let Some(old) = self.recent.pop_front() {
                    self.seen.remove(&old);
                }
            }
            out.push(comment);
        }
        out
    }

    /// When the next pending line of second `offset` is due.
    ///
    /// Twitch keeps offsets to the second, and a busy second holds fifty
    /// lines. Released on the tick they would land as a block once a second;
    /// spread evenly across it they read as chat. The lines already said of
    /// that second count, so a page that adds more to it does not bunch up
    /// what is left.
    fn release(&self, offset: u64) -> f64 {
        let said = match self.head {
            Some((second, said)) if second == offset => said,
            _ => 0,
        };
        let left = self
            .pending
            .iter()
            .take_while(|comment| comment.offset == offset)
            .count();
        offset as f64 + said as f64 / (said + left).max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five comments of a page Twitch sent on 9 September 2026, trimmed from
    /// fifty-four: a plain one, one with no colour, an emote followed by a
    /// combining character, text then an emote, and an emote then a tag
    /// character.
    const PAGE: &str = include_str!("fixtures/video-comments.json");

    fn fixture() -> Page {
        parse_page(&serde_json::from_str(PAGE).unwrap()).unwrap()
    }

    #[test]
    fn a_page_as_twitch_sends_one() {
        let page = fixture();
        assert_eq!(page.comments.len(), 5);
        // `hasNextPage`, and the cursor to ask with is the last edge's.
        let raw: Value = serde_json::from_str(PAGE).unwrap();
        let last_cursor = raw.pointer("/0/data/video/comments/edges/4/cursor");
        assert_eq!(page.next.as_deref(), last_cursor.and_then(Value::as_str));
        assert!(page.next.is_some());

        let first = &page.comments[0];
        assert_eq!(first.id, "16a26064-4b08-4f3e-9b2d-5c5a824b0e6b");
        assert_eq!(first.offset, 3599);
        assert_eq!(first.message.login, "dembro");
        assert_eq!(first.message.display_name, "dembro");
        assert_eq!(first.message.color, 0x777777);
        assert_eq!(first.message.text, "monkaOMEGA Grag ass");
        assert_eq!(first.message.emotes, None);
        assert!(!first.message.is_action);
        // `2026-09-08T14:01:56.719Z`, as the stamps want it.
        assert_eq!(first.message.sent_at, Some(1_788_876_116_719));
        // A fraction of two digits parses too: `14:01:57.08Z`.
        assert_eq!(page.comments[1].message.sent_at, Some(1_788_876_117_080));
    }

    #[test]
    fn no_colour_falls_back_the_way_irc_does() {
        let page = fixture();
        let plain = &page.comments[1];
        assert_eq!(plain.message.login, "xemirl");
        assert_eq!(plain.message.color, message::fallback_color("xemirl"));
    }

    #[test]
    fn emotes_become_the_tag_the_chat_view_reads() {
        let page = fixture();
        // An emote fragment, then "  ͏": the tag covers the emote only.
        let scared = &page.comments[2];
        assert_eq!(scared.message.text, "elisScared  \u{34f}");
        assert_eq!(
            scared.message.emotes.as_deref(),
            Some("emotesv2_b64756f6e7884d7db216873066b0f80a:0-9")
        );
        // Text first, then the emote at character 19.
        let remake = &page.comments[3];
        assert_eq!(remake.message.text, "wow another remake forsenSleeper");
        assert_eq!(remake.message.emotes.as_deref(), Some("89641:19-31"));
        // An emote and then a tag character, which is four bytes and one
        // character.
        let meltdown = &page.comments[4];
        assert_eq!(meltdown.message.text, "elisMeltdown \u{e0000}");
        assert_eq!(
            meltdown.message.emotes.as_deref(),
            Some("emotesv2_44bce938a5714e6fb695342a58b801fa:0-11")
        );
    }

    /// The regression the module docs warn about: Twitch's positions count
    /// bytes. After an emoji and a space the emote is at character 2, and
    /// Twitch says 5.
    #[test]
    fn emote_positions_count_characters_not_twitchs_bytes() {
        let node = json!({
            "id": "x", "contentOffsetSeconds": 7,
            "commenter": { "id": "1", "login": "fisher", "displayName": "Fisher" },
            "createdAt": "2026-09-08T14:01:56Z",
            "message": {
                "fragments": [
                    { "emote": null, "text": "🎣 " },
                    { "emote": { "id": "25;5;9", "emoteID": "25", "from": 5 }, "text": "Kappa" },
                    { "emote": null, "text": " 日本" },
                    { "emote": { "id": "1902;14;18", "emoteID": "1902", "from": 14 }, "text": "Keepo" }
                ],
                "userBadges": [],
                "userColor": "#FF0000"
            }
        });
        let line = comment(&node).unwrap();
        assert_eq!(line.message.text, "🎣 Kappa 日本Keepo");
        assert_eq!(line.message.emotes.as_deref(), Some("25:2-6/1902:10-14"));
    }

    #[test]
    fn a_deleted_commenter_is_skipped() {
        let node = json!({
            "id": "x", "contentOffsetSeconds": 7, "commenter": null,
            "createdAt": "2026-09-08T14:01:56Z",
            "message": { "fragments": [{ "emote": null, "text": "hi" }], "userColor": null }
        });
        assert!(comment(&node).is_none());
    }

    #[test]
    fn past_the_end_is_empty_and_final() {
        let body = json!([{
            "errors": [{ "message": "service error", "path": ["video", "comments"] }],
            "data": { "video": { "id": "1", "comments": null } }
        }]);
        assert_eq!(
            parse_page(&body).unwrap(),
            Page {
                comments: Vec::new(),
                next: None
            }
        );
    }

    #[test]
    fn a_missing_video_is_final_and_a_refused_request_is_not() {
        let missing = json!([{ "data": { "video": null } }]);
        assert!(matches!(parse_page(&missing), Err(Error::Unavailable(_))));

        let refused = json!([{
            "errors": [{ "message": "failed integrity check" }],
            "extensions": {}
        }]);
        assert_eq!(
            parse_page(&refused),
            Err(Error::Transient("failed integrity check".into()))
        );
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        let body = json!([{ "data": { "video": { "id": "1", "comments": {
            "edges": [{ "cursor": "abc", "node": {
                "id": "x", "contentOffsetSeconds": 7,
                "commenter": { "id": "1", "login": "a", "displayName": "A" },
                "createdAt": "2026-09-08T14:01:56Z",
                "message": { "fragments": [{ "emote": null, "text": "hi" }], "userColor": null }
            }}],
            "pageInfo": { "hasNextPage": false, "hasPreviousPage": true }
        }}}}]);
        let page = parse_page(&body).unwrap();
        assert_eq!(page.comments.len(), 1);
        assert_eq!(page.next, None);
    }

    fn line(id: &str, offset: u64, sent_at: u64) -> Comment {
        Comment {
            id: id.to_string(),
            offset,
            message: ChatMessage {
                login: "a".into(),
                display_name: "A".into(),
                color: 0,
                text: id.to_string(),
                is_action: false,
                emotes: None,
                sent_at: Some(sent_at),
            },
        }
    }

    fn ids(comments: &[Comment]) -> Vec<&str> {
        comments.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn a_busy_second_is_spread_across_it() {
        let mut schedule = Schedule::default();
        schedule.push(vec![
            line("a", 10, 1),
            line("b", 10, 2),
            line("c", 10, 3),
            line("d", 10, 4),
            line("e", 12, 5),
        ]);
        assert!(schedule.due(9.99).is_empty());
        assert_eq!(ids(&schedule.due(10.0)), ["a"]);
        assert!(schedule.due(10.24).is_empty());
        assert_eq!(ids(&schedule.due(10.5)), ["b", "c"]);
        assert_eq!(ids(&schedule.due(11.9)), ["d"]);
        assert_eq!(ids(&schedule.due(12.0)), ["e"]);
        assert!(schedule.is_empty());
    }

    #[test]
    fn a_page_that_adds_to_the_current_second_counts_what_was_said() {
        let mut schedule = Schedule::default();
        schedule.push(vec![line("a", 10, 1), line("b", 10, 2)]);
        assert_eq!(ids(&schedule.due(10.6)), ["a", "b"]);
        // Two more of second 10 arrive: four in all, two said, so the rest
        // are due at .5 and .75 rather than starting the second over.
        schedule.push(vec![line("c", 10, 3), line("d", 10, 4)]);
        assert_eq!(ids(&schedule.due(10.6)), ["c"]);
        assert!(schedule.due(10.74).is_empty());
        assert_eq!(ids(&schedule.due(10.75)), ["d"]);
    }

    #[test]
    fn repeats_are_dropped_and_order_is_kept() {
        let mut schedule = Schedule::default();
        schedule.push(vec![line("a", 10, 1), line("b", 11, 2)]);
        assert_eq!(ids(&schedule.due(10.0)), ["a"]);
        // An ask by offset repeats what was said and what is pending, and
        // brings one that was missing from between.
        schedule.push(vec![
            line("a", 10, 1),
            line("b", 11, 2),
            line("c", 10, 3),
            line("d", 12, 4),
        ]);
        assert_eq!(ids(&schedule.due(20.0)), ["c", "b", "d"]);
        assert_eq!(schedule.end(), None);
    }

    #[test]
    fn everything_before_a_position_is_the_backlog() {
        let mut schedule = Schedule::default();
        schedule.push((0..30).map(|n| line(&n.to_string(), 100 + n, n)).collect());
        let backlog = schedule.due(115.0);
        assert_eq!(backlog.len(), 16, "seconds 100 through 115");
        assert_eq!(schedule.end(), Some(129));
    }

    #[test]
    fn playback_and_a_pause_are_not_seeks() {
        let tick = Duration::from_millis(100);
        assert!(!is_seek(50.0, tick, 50.1));
        assert!(!is_seek(50.0, tick, 50.0), "paused");
        assert!(
            !is_seek(50.0, Duration::from_secs(30), 50.0),
            "paused a while"
        );
        // The player reports in bursts; a little ahead of the clock is fine.
        assert!(!is_seek(50.0, tick, 51.5));
        // A reposition landing a hair short of where it was is not one either.
        assert!(!is_seek(50.0, tick, 49.5));
    }

    #[test]
    fn a_jump_either_way_is_a_seek() {
        let tick = Duration::from_millis(100);
        assert!(is_seek(50.0, tick, 60.0), "the arrow key");
        assert!(is_seek(50.0, tick, 40.0));
        assert!(is_seek(50.0, tick, 0.0), "watch again");
        assert!(is_seek(50.0, Duration::from_secs(5), 3000.0), "the bar");
    }
}
