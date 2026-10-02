//! When each offline follow was last live, worked out: when to ask Twitch,
//! what its answers are kept as, and the words a name wears for it ("Live 3
//! hours ago"). Pure and tested. The asking is `root::last_live`, the request
//! is the worker's `Request::LastLive`, the drawing is Home's offline names
//! and the rail's offline rows, and the data is
//! `twitch_api::recommend::last_broadcasts`, whose docs say where it comes
//! from: Twitch's unpublished GraphQL endpoint, asked anonymously, a hundred
//! logins a request.
//!
//! That endpoint is unpublished, so the stance is the one the rail's
//! Recommended group takes: ask rarely, and when it breaks, go quietly. An
//! ask goes out for every offline follow once the follows list first arrives,
//! then only for the ones nobody has asked about since — a channel whose
//! stream has just ended, or one newly followed — and for all of them again
//! once every [`REFRESH`] at most ([`LastLive::next_ask`]). Never at every
//! follows poll: that is a minute, and a hundred follows would be a request
//! a minute for words that change once a day. A failure is a line in the log
//! and no words on the names it was for; nothing else changes
//! ([`LastLive::answered`]). A refusal, Twitch declining to run the query at
//! all, is a line too, and then no more asking this session. Nothing here is
//! saved.
//!
//! Twitch's answer says when the last stream *started*, not when it ended,
//! which on its own is out by however long that stream ran: a channel that
//! has just ended six hours on would read "Live 6 hours ago". So the root
//! also notes, at every follows poll, the time of the last poll that listed
//! each channel live ([`LastLive::went_live`]), and the words count from the
//! later of the two. That is right to within a poll for any stream that
//! ended while Perch was open; one that ended before launch is still counted
//! from when it started.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use settings::channel_key;
use twitch_api::recommend::LastBroadcast;

use crate::channel_page;
use crate::twitch::RecommendError;

/// How long the answers stand before every offline follow is asked about
/// again.
///
/// The words count from when a stream started, and the counting is done at
/// draw time, so an answer an hour old still says the right thing about a
/// channel that has not been on since. What goes stale is a channel that
/// went live and ended between two polls, which the follows poll never saw
/// live; fifteen minutes bounds how long that one says the day before, at
/// two requests a time for a couple of hundred follows.
pub const REFRESH: Duration = Duration::from_secs(15 * 60);

/// One ask for the worker: which logins, and whether it is the full one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub logins: Vec<String>,
    /// Every offline follow, which starts the [`REFRESH`] interval again.
    /// Otherwise only the ones not asked about since the last full ask.
    pub full: bool,
}

/// The asks out and the answers in. Lives on the root for the session and is
/// never saved.
#[derive(Debug, Default)]
pub struct LastLive {
    /// Each login's last answer, by `settings::channel_key`. Kept through a
    /// failed ask, so a dropped request takes no words off a name that had
    /// them.
    answers: HashMap<String, LastBroadcast>,
    /// When the last full ask went out. `None` before the first, and after a
    /// worker that will never answer has been forgotten.
    full_at: Option<Instant>,
    /// Every login asked about since the last full ask, answered or not, by
    /// `settings::channel_key`: one whose ask failed waits for the next full
    /// ask rather than being asked again at every poll until it works.
    tried: HashSet<String>,
    /// The ask the worker has and has not answered yet.
    out: Option<Ask>,
    /// When a follows poll last listed each login live, by
    /// `settings::channel_key`: a time the channel was certainly live, and
    /// for a stream that ended this session a far better one than when it
    /// started. Kept through a full answer, which knows nothing of it.
    seen_live: HashMap<String, DateTime<Utc>>,
    /// Twitch would not run the query (`RecommendError::Refused`), so it is
    /// not asked again this session; the polls' own times still give words.
    refused: bool,
}

impl LastLive {
    /// What to ask the worker now, if anything, given the offline follows as
    /// they stand at `now`.
    ///
    /// Nothing while an ask is out, so there is one at a time; nothing with
    /// nobody offline. Otherwise everyone when the last full ask is
    /// [`REFRESH`] old or there has been none, and failing that only the ones
    /// nobody has asked about since. So the follows poll can call this every
    /// minute, and it asks about everyone once in fifteen, and about a
    /// channel whose stream just ended once, at the poll that noticed. And
    /// nothing at all once Twitch has refused the query.
    pub fn next_ask<'a>(
        &self,
        offline: impl IntoIterator<Item = &'a str>,
        now: Instant,
    ) -> Option<Ask> {
        if self.out.is_some() || self.refused {
            return None;
        }
        let mut logins: Vec<String> = offline.into_iter().map(channel_key).collect();
        let due = self
            .full_at
            .is_none_or(|at| now.saturating_duration_since(at) >= REFRESH);
        if !due {
            logins.retain(|login| !self.tried.contains(login));
        }
        (!logins.is_empty()).then_some(Ask { logins, full: due })
    }

    /// Note that `ask` has gone to the worker, at `now`. Only once it has:
    /// an ask nobody could take is not one to wait on.
    pub fn asked(&mut self, ask: Ask, now: Instant) {
        if ask.full {
            self.full_at = Some(now);
            self.tried = ask.logins.iter().cloned().collect();
        } else {
            self.tried.extend(ask.logins.iter().cloned());
        }
        self.out = Some(ask);
    }

    /// Take the worker's answer to the ask that was out, and say what is
    /// worth a line in the log, if anything: a failure, once per ask.
    ///
    /// A full answer replaces every login's, so a channel no longer followed
    /// stops being kept; a partial one adds to them. A login the answer left
    /// out — a channel renamed or gone — has no words. A failure keeps the
    /// answers there are, and the next full ask is when it is tried again.
    /// A refusal keeps them too, and is the last ask of the session, as it
    /// is for Recommended (`recommended::Recommended::answered`).
    pub fn answered(
        &mut self,
        result: Result<HashMap<String, LastBroadcast>, RecommendError>,
    ) -> Option<String> {
        let ask = self.out.take();
        match result {
            Ok(answers) => {
                if ask.is_some_and(|ask| ask.full) {
                    self.answers.clear();
                }
                self.answers.extend(
                    answers
                        .into_iter()
                        .map(|(login, last)| (channel_key(&login), last)),
                );
                None
            }
            Err(RecommendError::Refused(message)) => {
                if self.refused {
                    return None;
                }
                self.refused = true;
                Some(format!(
                    "Twitch would not run the query ({message}); \
                     not asking again this session"
                ))
            }
            Err(RecommendError::Failed(message)) => Some(message),
        }
    }

    /// Note that the follows poll at `now` listed these channels live, and
    /// forget what Twitch said about each, so it is asked about afresh once
    /// its stream ends: the answer in hand says when the stream before this
    /// one started, and will be a day out the moment this one is over.
    /// Called with the live list at every follows poll.
    pub fn went_live<'a>(&mut self, logins: impl IntoIterator<Item = &'a str>, now: DateTime<Utc>) {
        for login in logins {
            let key = channel_key(login);
            self.answers.remove(&key);
            self.tried.remove(&key);
            self.seen_live.insert(key, now);
        }
    }

    /// Forget the ask that was out, for a worker that will never answer it:
    /// one replaced by a new client id, or one stopped because sign-in
    /// failed. The interval starts over, so the next worker's first follows
    /// list asks about everyone.
    pub fn forget(&mut self) {
        self.out = None;
        self.full_at = None;
        self.tried.clear();
    }

    /// What an offline name says about when it was last live, at `now`,
    /// counted from the later of when its last stream started and the last
    /// poll that saw it live; or `None` when there is nothing to say: no
    /// answer and never seen live, a channel that has never broadcast, or
    /// one Twitch says is live — the follows poll will move that one to the
    /// live list within the minute, and "Live 2 minutes ago" on a name among
    /// the offline ones would be wrong both ways.
    pub fn words(&self, login: &str, now: DateTime<Utc>) -> Option<String> {
        let key = channel_key(login);
        let started = match self.answers.get(&key) {
            Some(last) if last.live => return None,
            Some(last) => last.started_at.as_deref().and_then(|started_at| {
                DateTime::parse_from_rfc3339(started_at)
                    .ok()
                    .map(|started| started.with_timezone(&Utc))
            }),
            None => None,
        };
        let latest = started.max(self.seen_live.get(&key).copied())?;
        wording(&latest.to_rfc3339(), now)
    }
}

/// "Live 40 minutes ago", "Live 3 hours ago", then in days the way the
/// channel page counts a recording's age (`channel_page::when`): "Live
/// yesterday", "Live 3 days ago", "Live last week", "Live 2 weeks ago", and
/// "Live on 1 Jul" once counting stops helping. `started_at` is when the
/// channel was last known live, in RFC 3339; `None` when it does not parse.
///
/// Under a day it counts hours rather than going by the calendar: a stream
/// that started at eleven last night is "3 hours ago" at two in the morning,
/// which is how anyone still up thinks of it, where the calendar would call
/// it yesterday.
pub fn wording(started_at: &str, now: DateTime<Utc>) -> Option<String> {
    let started = DateTime::parse_from_rfc3339(started_at)
        .ok()?
        .with_timezone(&Utc);
    let ago = now.signed_duration_since(started);
    let hours = ago.num_hours();
    if hours < 24 {
        return Some(match hours {
            i64::MIN..=0 => {
                // A clock a little behind Twitch's would otherwise say "0".
                let minutes = ago.num_minutes().max(1);
                format!("Live {minutes} minute{} ago", plural(minutes))
            }
            _ => format!("Live {hours} hour{} ago", plural(hours)),
        });
    }
    let when = channel_page::when(started_at, now)?;
    // Its words ("yesterday", "3 days ago", "last week") follow "Live" as
    // they are; a date wants an "on" in front of it to read as one.
    let date = when.starts_with(|c: char| c.is_ascii_digit()) && !when.ends_with("ago");
    Some(if date {
        format!("Live on {when}")
    } else {
        format!("Live {when}")
    })
}

fn plural(count: i64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(when: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(when)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn last(started_at: Option<&str>, live: bool) -> LastBroadcast {
        LastBroadcast {
            started_at: started_at.map(str::to_string),
            live,
        }
    }

    fn logins(ask: &Ask) -> Vec<&str> {
        ask.logins.iter().map(String::as_str).collect()
    }

    /// Minutes, then hours for the first day, then the channel page's days,
    /// and a date with an "on".
    #[test]
    fn last_live_reads_like_somebody_counting() {
        // Midday, so the local calendar `when` counts on agrees with UTC's
        // anywhere a test runs.
        let now = at("2026-09-08T12:00:00Z");
        let says = |when: &str| wording(when, now).unwrap();
        assert_eq!(says("2026-09-08T11:20:00Z"), "Live 40 minutes ago");
        assert_eq!(says("2026-09-08T12:00:30Z"), "Live 1 minute ago");
        assert_eq!(says("2026-09-08T11:00:00Z"), "Live 1 hour ago");
        assert_eq!(says("2026-09-08T09:00:00Z"), "Live 3 hours ago");
        assert_eq!(says("2026-09-07T13:00:00Z"), "Live 23 hours ago");
        assert_eq!(says("2026-09-07T12:00:00Z"), "Live yesterday");
        assert_eq!(says("2026-09-05T12:00:00Z"), "Live 3 days ago");
        assert_eq!(says("2026-08-30T12:00:00Z"), "Live last week");
        assert_eq!(says("2026-08-20T12:00:00Z"), "Live 2 weeks ago");
        assert_eq!(says("2026-07-01T12:00:00Z"), "Live on 1 Jul");
        assert_eq!(wording("not a time", now), None);
    }

    /// A name says nothing it does not know: no answer, a channel that never
    /// broadcast, or one Twitch already calls live. Looked up in any case.
    #[test]
    fn words_only_for_a_known_past_broadcast() {
        let now = at("2026-09-08T12:00:00Z");
        let mut known = LastLive::default();
        known.asked(
            Ask {
                logins: vec!["a".into(), "b".into(), "c".into()],
                full: true,
            },
            Instant::now(),
        );
        known.answered(Ok(HashMap::from([
            ("a".into(), last(Some("2026-09-08T09:00:00Z"), false)),
            ("b".into(), last(None, false)),
            ("c".into(), last(Some("2026-09-08T11:58:00Z"), true)),
        ])));
        assert_eq!(known.words("A", now).as_deref(), Some("Live 3 hours ago"));
        assert_eq!(known.words("b", now), None);
        assert_eq!(known.words("c", now), None);
        assert_eq!(known.words("nobody", now), None);
    }

    /// Everyone on the first list; then only who is new to it, one ask at a
    /// time; then everyone again once the interval is up — and nothing at
    /// a poll in between with nobody new.
    #[test]
    fn asks_everyone_once_then_only_newcomers_until_the_interval() {
        let start = Instant::now();
        let mut last_live = LastLive::default();
        let first = last_live.next_ask(["Alice", "bob"], start).unwrap();
        assert!(first.full);
        assert_eq!(logins(&first), ["alice", "bob"]);
        last_live.asked(first, start);

        assert_eq!(
            last_live.next_ask(["alice", "bob", "carol"], start),
            None,
            "asked again while the first ask was out"
        );
        last_live.answered(Ok(HashMap::new()));

        let minute = start + Duration::from_secs(60);
        assert_eq!(last_live.next_ask(["alice", "bob"], minute), None);
        let newcomer = last_live
            .next_ask(["alice", "bob", "carol"], minute)
            .unwrap();
        assert!(!newcomer.full);
        assert_eq!(logins(&newcomer), ["carol"]);
        last_live.asked(newcomer, minute);
        last_live.answered(Ok(HashMap::new()));
        assert_eq!(last_live.next_ask(["alice", "bob", "carol"], minute), None);

        let later = start + REFRESH;
        let again = last_live
            .next_ask(["alice", "bob", "carol"], later)
            .unwrap();
        assert!(again.full);
        assert_eq!(logins(&again), ["alice", "bob", "carol"]);

        assert_eq!(LastLive::default().next_ask([], start), None);
    }

    /// A channel seen live loses its old answer and is asked about again at
    /// the first poll that has it offline — not at every poll after.
    #[test]
    fn a_stream_that_ends_is_asked_about_once() {
        let start = Instant::now();
        let mut last_live = LastLive::default();
        last_live.asked(last_live.next_ask(["alice", "bob"], start).unwrap(), start);
        last_live.answered(Ok(HashMap::from([(
            "alice".into(),
            last(Some("2026-09-01T12:00:00Z"), false),
        )])));

        let now = at("2026-09-08T12:00:00Z");
        last_live.went_live(["Alice"], now);
        assert_eq!(
            last_live.words("alice", now).as_deref(),
            Some("Live 1 minute ago"),
            "kept a stale answer"
        );

        let ended = last_live.next_ask(["alice", "bob"], start).unwrap();
        assert_eq!(logins(&ended), ["alice"]);
        last_live.asked(ended, start);
        last_live.answered(Err(RecommendError::Failed("network".into())));
        assert_eq!(last_live.next_ask(["alice", "bob"], start), None);
    }

    /// A failure is one line and nothing else: the answers in hand stay,
    /// and nothing is asked again until the interval.
    #[test]
    fn a_failure_keeps_what_was_known() {
        let start = Instant::now();
        let now = at("2026-09-08T12:00:00Z");
        let mut last_live = LastLive::default();
        last_live.asked(last_live.next_ask(["alice"], start).unwrap(), start);
        last_live.answered(Ok(HashMap::from([(
            "alice".into(),
            last(Some("2026-09-08T09:00:00Z"), false),
        )])));

        let later = start + REFRESH;
        last_live.asked(last_live.next_ask(["alice"], later).unwrap(), later);
        assert_eq!(
            last_live
                .answered(Err(RecommendError::Failed("HTTP 503".into())))
                .as_deref(),
            Some("HTTP 503")
        );
        assert_eq!(
            last_live.words("alice", now).as_deref(),
            Some("Live 3 hours ago")
        );
        assert_eq!(last_live.next_ask(["alice"], later), None);
    }

    /// Twitch only says when a stream started. One the polls saw live until
    /// 11:58 reads from then, not from its 06:00 start.
    #[test]
    fn counts_from_the_last_poll_that_saw_it_live() {
        let start = Instant::now();
        let mut last_live = LastLive::default();
        last_live.went_live(["alice"], at("2026-09-08T11:58:00Z"));
        last_live.asked(last_live.next_ask(["alice"], start).unwrap(), start);
        last_live.answered(Ok(HashMap::from([(
            "alice".into(),
            last(Some("2026-09-08T06:00:00Z"), false),
        )])));
        assert_eq!(
            last_live
                .words("alice", at("2026-09-08T12:00:00Z"))
                .as_deref(),
            Some("Live 2 minutes ago")
        );
    }

    /// A refusal is one line, and the last ask of the session: not even the
    /// interval brings another.
    #[test]
    fn a_refusal_stops_the_asking() {
        let start = Instant::now();
        let mut last_live = LastLive::default();
        last_live.asked(last_live.next_ask(["alice"], start).unwrap(), start);
        assert!(last_live
            .answered(Err(RecommendError::Refused(
                "PersistedQueryNotFound".into()
            )))
            .is_some());
        assert_eq!(last_live.next_ask(["alice", "bob"], start + REFRESH), None);
    }
}
