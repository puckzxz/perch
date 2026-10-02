//! Rewinding a live pane, worked out: the timeline its bar draws across the
//! broadcast so far, where a press on it lands, and the recording a press
//! opens — the broadcast's own, still being made — at that moment. Pure and
//! tested. The drawing is the player's (`video_view::bar`, through the same
//! `seek_bar` a recording uses), the asking and the opening are the root's
//! (`root::rewind`), and the request is the worker's `Request::Broadcasts`,
//! the one a stopped pane asks with.
//!
//! The timeline runs from the broadcast's start, as the live lists give it
//! (`LiveStream::started_at`), to now. It needs no player of its own: a
//! press back along it opens the broadcast's archive at that moment in the
//! pane's place, the way `Watch from the start` opens an ended one
//! (`RootView::replace_with_video`), and the pane is a recording from then
//! on, with its own seek bar and its chat replayed.
//!
//! A press is a moment on the wall clock rather than seconds into anything,
//! because the two things it is measured against start at slightly
//! different times: the broadcast at `started_at`, its archive at its own
//! `created_at`, a few seconds later or, after a reconnect split it, hours
//! later. The moment is the one thing both agree on ([`position_in`]).
//!
//! The archive is found by the matching `Watch from the start` uses
//! (`channel_page::archive_of`), asked about now rather than at an end, and
//! looked up only when somebody presses: nothing polls for it. Once found it
//! is kept for the broadcast ([`Rewind`]); not finding it is not kept, since
//! Twitch may simply not have listed it yet, and the next press asks again.
//!
//! And the way back: a pane playing that archive while its broadcast is
//! still going offers `LIVE` on its bar, which replaces it with the channel
//! again. Whether it is that archive is [`live_now`], by the same rule that
//! rules out the broadcast before a restart, against the live lists the app
//! already keeps; nothing polls for that either. Those lists are not all
//! fresh, so [`back_to_live`] weighs them against what the pane knows by
//! itself ([`Origin`]): the follows poll's word over a list fetched once, the
//! broadcast a pane saw end, and the one a pane rewound from.

use chrono::{DateTime, Utc};
use twitch_api::{Video, VideoKind};

use crate::channel_page;

/// How near the live edge a press does nothing, in seconds. The archive runs
/// behind the stream by about this much, so a press here would open a
/// recording at a moment it has not reached yet, and it is what a press
/// meant as "stay live" lands on.
pub const EDGE_SECS: f64 = 30.0;

/// The longest a broadcast can be: Twitch ends a stream at 48 hours. A start
/// further back than this is a snapshot of an older broadcast, not this one,
/// and no timeline is drawn from it.
pub const LONGEST_SECS: f64 = 48.0 * 3600.0;

/// How far before the broadcast's start an archive's listed end may fall and
/// still be this broadcast's, in seconds: the archive's own start and the
/// live list's start are written by different parts of Twitch, a few
/// seconds apart either way. An archive listed as ending earlier than this
/// is a broadcast that finished before this one began — the one before a
/// restart — whatever its id or its picture say.
const BEGUN_SLACK_SECS: i64 = 60;

/// A press on a live pane's timeline: the moment it asks for, and the start
/// of the broadcast the timeline was drawn from, which says which broadcast
/// that moment belongs to. Sent by the player as `PaneAction::Rewind`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    /// The moment on the wall clock the press landed on ([`pressed_at`]).
    pub at: DateTime<Utc>,
    /// When the broadcast began, as the player's timeline had it.
    pub since: DateTime<Utc>,
}

/// A broadcast's start as the live lists write it, RFC 3339. `None` for a
/// list that says nothing, or something that is not a time.
pub fn parse_start(started_at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(started_at)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// How long the timeline is, in seconds, for a broadcast that started at
/// `started`, or `None` where none is offered.
///
/// Only once there is something behind the live edge to go back to, more
/// than [`EDGE_SECS`] of it, and never for a start that is in the future (a
/// clock out of step) or further back than [`LONGEST_SECS`].
pub fn span(started: DateTime<Utc>, now: DateTime<Utc>) -> Option<f64> {
    let secs = now.signed_duration_since(started).num_milliseconds() as f64 / 1000.0;
    (secs > EDGE_SECS && secs <= LONGEST_SECS).then_some(secs)
}

/// Seconds into the broadcast at `fraction` of a timeline `span` long,
/// clamped to it: a drag that leaves the bar to the left is the start.
pub fn secs_at(fraction: f32, span: f64) -> f64 {
    f64::from(fraction.clamp(0.0, 1.0)) * span.max(0.0)
}

/// Whether `secs` into a broadcast `span` long is within [`EDGE_SECS`] of
/// the live edge, where a press does nothing.
pub fn near_edge(secs: f64, span: f64) -> bool {
    span - secs < EDGE_SECS
}

/// The moment a press at `fraction` of the timeline asks for, for a
/// broadcast that started at `started`: `None` where no timeline is offered
/// ([`span`]), or where the press is too near the edge to mean anything
/// ([`near_edge`]).
pub fn pressed_at(
    started: DateTime<Utc>,
    fraction: f32,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let span = span(started, now)?;
    let secs = secs_at(fraction, span);
    (!near_edge(secs, span))
        .then(|| started + chrono::Duration::milliseconds((secs * 1000.0).round() as i64))
}

/// The recording of the broadcast going on now, which began at `since` —
/// the one whose id is `broadcast`, when that is known — among a channel's
/// `archives`.
///
/// Only an archive that had not finished before `since` can be it
/// ([`BEGUN_SLACK_SECS`]): after a restart the broadcast before it can
/// still be listed as going on until close to now, and a stale id names it
/// outright, so it is ruled out before anything else is asked. Among the
/// rest, `channel_page::archive_of`, asked about now: an archive that
/// started by now and is listed as going on until close to it. And failing
/// that, one Twitch still draws its placeholder picture for, which says the
/// recording is being made now, however stale its listed length; the id
/// chooses among those too. No archive is no answer: an older broadcast's
/// recording is not this one's.
pub fn archive_for(
    archives: &[Video],
    broadcast: Option<&str>,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<Video> {
    let current: Vec<Video> = archives
        .iter()
        .filter(|video| going_at(video, since))
        .cloned()
        .collect();
    if let Some(found) = channel_page::archive_of(&current, broadcast, now) {
        return Some(found.clone());
    }
    let being_made: Vec<&Video> = current
        .iter()
        .filter(|video| channel_page::in_progress(video, now))
        .filter(|video| channel_page::placeholder(&video.thumbnail_url))
        .filter(|video| started(video).is_some_and(|started| started <= now))
        .collect();
    broadcast
        .and_then(|id| {
            being_made
                .iter()
                .find(|video| video.stream_id.as_deref() == Some(id))
                .copied()
        })
        .or_else(|| being_made.into_iter().max_by_key(|video| started(video)))
        .cloned()
}

/// Whether `archive` is the recording of the broadcast going on now, which
/// the live lists say began at `since` with the id `broadcast`: what offers a
/// pane playing it the way back to the live edge (`LIVE` on its bar).
///
/// The rule [`archive_for`] rules the broadcast before a restart out by: an
/// archive that had finished before `since` ([`BEGUN_SLACK_SECS`]) is an
/// earlier broadcast's, and so is anything but an archive. Its listed length
/// can be as stale as it likes — the pane's copy is the one it was opened
/// with, maybe hours ago — since a length only ever grows, so a listed end
/// at or after the start is a real one. A start more than [`LONGEST_SECS`]
/// back is a snapshot of an older broadcast, which no archive is the
/// recording of now. And the two ids, where both are known, must agree:
/// unlike the pane's own `Slot::broadcast` in [`archive_for`], this one comes
/// from the same list entry as `since`, so it is as fresh as the start is,
/// and it is what tells a snapshot of a broadcast that has since ended from
/// the one on now.
pub fn live_now(
    archive: &Video,
    broadcast: Option<&str>,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    let believable = now.signed_duration_since(since).num_seconds() as f64 <= LONGEST_SECS;
    let ids_agree = match (
        archive.stream_id.as_deref().filter(|id| !id.is_empty()),
        broadcast.filter(|id| !id.is_empty()),
    ) {
        (Some(archive), Some(broadcast)) => archive == broadcast,
        _ => true,
    };
    archive.kind == VideoKind::Archive
        && believable
        && started(archive).is_some_and(|started| started <= now)
        && going_at(archive, since)
        && ids_agree
}

/// What a recording pane knows by itself of the live broadcast it came out
/// of: carried onto the recording's slot when it takes a live pane's place
/// (`RootView::replace_with_video`), and asked by [`back_to_live`] next to
/// what the live lists say. Kept on the slot for as long as the recording
/// plays; a recording opened from a card, a link or the history knows
/// nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Origin {
    /// Nothing of a broadcast: the recording did not take a live pane's
    /// place.
    #[default]
    Unknown,
    /// Rewound into from a live pane's timeline, drawn from a broadcast
    /// that began at `since`. Stands in for the live lists once none of
    /// them carries the channel — the category or the search it was opened
    /// from has been replaced since — so a pane that got here by rewinding
    /// still has its way back. No id: the archive was chosen for this
    /// broadcast, so its own id is the broadcast's.
    Rewound { since: DateTime<Utc> },
    /// Opened from a live pane that had seen its broadcast end, or found the
    /// channel off (`Watch from the start`, `Watch here`): the broadcast with
    /// the id `broadcast` is over, whatever a list that lags behind says.
    Ended { broadcast: Option<String> },
}

/// What the live lists the app keeps say about a recording's channel, for
/// [`back_to_live`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Listed<'a> {
    /// A list carries it, broadcasting since `since`, with the id
    /// `broadcast` where the list gives one.
    On {
        since: DateTime<Utc>,
        broadcast: Option<&'a str>,
    },
    /// You follow it and the follows poll, the one list kept fresh, has it
    /// off: whatever a list fetched once and kept says, it is off.
    Off,
    /// No list carries it, or none says when its broadcast began.
    Unknown,
}

/// Whether a pane playing `archive` is offered the way back to the live
/// edge (`LIVE` on its bar), from what the lists say of its channel
/// (`listed`) and what the pane knows by itself (`origin`).
///
/// The follows poll having the channel off is the last word: Popular, a
/// category and a search are fetched once and kept, and still list a
/// broadcast that ended hours ago. A list that does carry it is asked by
/// [`live_now`], except about the very broadcast the pane saw end, which a
/// list a poll behind may still name. And with no list carrying it at all,
/// a pane that rewound into the archive goes by the broadcast it rewound
/// from. The bar also wants the archive still growing, which is the
/// player's to know (`video_view::bar`).
pub fn back_to_live(
    archive: &Video,
    listed: Listed<'_>,
    origin: &Origin,
    now: DateTime<Utc>,
) -> bool {
    match (listed, origin) {
        (Listed::Off, _) => false,
        (
            Listed::On {
                broadcast: Some(id),
                ..
            },
            Origin::Ended {
                broadcast: Some(ended),
            },
        ) if !id.is_empty() && id == ended => false,
        (Listed::On { since, broadcast }, _) => live_now(archive, broadcast, since, now),
        (Listed::Unknown, Origin::Rewound { since }) => live_now(archive, None, *since, now),
        (Listed::Unknown, _) => false,
    }
}

/// Where in `archive` the moment `at` is, in seconds, as of `now`: from the
/// archive's own start, never before it — a moment ahead of a recording that
/// starts late, after a reconnect split the broadcast, is its start — and
/// never past [`EDGE_SECS`] short of how long it has been going.
pub fn position_in(archive: &Video, at: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let Some(started) = started(archive) else {
        return 0.0;
    };
    let secs = at.signed_duration_since(started).num_milliseconds() as f64 / 1000.0;
    let latest = (channel_page::elapsed_secs(archive, now).unwrap_or(0.0) - EDGE_SECS).max(0.0);
    secs.clamp(0.0, latest)
}

/// `archive` as it is `now`: as long as the time since it started, which is
/// exact for a recording still being made where the listed length is only
/// as fresh as the list it came in. An archive kept since an earlier press
/// opens as the recording still going that it is, rather than one listed as
/// ending an hour ago (`channel_page::in_progress`, which the history reads
/// it by when the pane opens).
pub fn as_of(archive: &Video, now: DateTime<Utc>) -> Video {
    let mut video = archive.clone();
    if let Some(elapsed) = channel_page::elapsed_secs(archive, now) {
        video.length_secs = video.length_secs.max(elapsed.floor() as u64);
    }
    video
}

fn started(video: &Video) -> Option<DateTime<Utc>> {
    parse_start(&video.created_at)
}

/// Where `video` ends as listed: its start plus its listed length.
fn listed_end(video: &Video) -> Option<DateTime<Utc>> {
    Some(started(video)? + chrono::Duration::seconds(video.length_secs as i64))
}

/// Whether `video` had not finished, as listed, by the time a broadcast
/// began at `since`, give or take [`BEGUN_SLACK_SECS`]: the one test both
/// [`archive_for`] and [`live_now`] put an archive to before anything else,
/// which the broadcast before a restart fails.
fn going_at(video: &Video, since: DateTime<Utc>) -> bool {
    let begun_by = since - chrono::Duration::seconds(BEGUN_SLACK_SECS);
    listed_end(video).is_some_and(|end| end >= begun_by)
}

/// A live pane's rewind: the broadcast's archive once found, an ask out for
/// it, and a press waiting on that ask. On the pane's slot, and made afresh
/// whenever the pane begins to play (`RootView::apply_stream_event`).
///
/// The archive is kept with the start of the broadcast it was found for,
/// and a press on a broadcast that began at any other time finds nothing in
/// hand: a stream that restarts or reconnects into a new broadcast while
/// the pane plays on moves its timeline's start (`RootView::sync_live_since`)
/// without the pane ever beginning to play again, and the last broadcast's
/// recording is not this one's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rewind {
    /// The broadcast's recording, once an ask has found it, and the start of
    /// the broadcast it was found for. Kept for as long as that broadcast
    /// plays: it is the same recording at every press.
    archive: Option<(DateTime<Utc>, Video)>,
    /// Whether the worker has been asked and has not answered.
    asking: bool,
    /// The newest press made while there was no archive in hand, carried out
    /// when the answer comes.
    pressed: Option<Moment>,
}

/// What a press on the timeline comes to; see [`Rewind::press`].
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// Open this, the archive in hand.
    Open(Box<Video>),
    /// Ask the worker for the channel's past broadcasts, and say so with
    /// [`Rewind::asked`]; the press is remembered until the answer.
    Ask,
    /// An ask is out already: the press is remembered, in place of any
    /// earlier one, until its answer.
    Wait,
}

impl Rewind {
    /// A press asking for `moment`. An archive kept for another broadcast
    /// than the one pressed on is let go.
    pub fn press(&mut self, moment: Moment) -> Press {
        match &self.archive {
            Some((since, archive)) if *since == moment.since => {
                return Press::Open(Box::new(archive.clone()));
            }
            Some(_) => self.archive = None,
            None => {}
        }
        self.pressed = Some(moment);
        if self.asking {
            Press::Wait
        } else {
            Press::Ask
        }
    }

    /// The ask a press called for has gone out.
    pub fn asked(&mut self) {
        self.asking = true;
    }

    /// The ask a press called for could not go out: forget the press, which
    /// nothing will answer.
    pub fn unasked(&mut self) {
        self.pressed = None;
    }

    /// Whether an ask is out: what the worker's answer about the channel is
    /// taken for, ahead of a stopped pane's own ask.
    pub fn asking(&self) -> bool {
        self.asking
    }

    /// The press waiting on the ask that is out, if one is: which broadcast
    /// its answer is searched for.
    pub fn waiting(&self) -> Option<Moment> {
        self.pressed
    }

    /// The archive in hand, if an ask has found one.
    pub fn archive(&self) -> Option<&Video> {
        self.archive.as_ref().map(|(_, archive)| archive)
    }

    /// The answer to the ask that was out: the archive it `found` for the
    /// press waiting on it ([`waiting`](Self::waiting)), if any. A found one
    /// is kept, for that press's broadcast; none found keeps nothing, so the
    /// next press asks again. Returns the press that was waiting, if one was.
    pub fn settle(&mut self, found: Option<Video>) -> Option<Moment> {
        self.asking = false;
        let pressed = self.pressed.take();
        if let (Some(moment), Some(archive)) = (pressed, found) {
            self.archive = Some((moment.since, archive));
        }
        pressed
    }

    /// Forget an ask still out, and the press waiting on it, for a worker
    /// that will never answer: `RootView::forget_asks`.
    pub fn forget(&mut self) {
        self.asking = false;
        self.pressed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        parse_start(rfc3339).unwrap()
    }

    /// An archive of broadcast `stream_id`, started at `created_at` and
    /// listed as `length_secs` long, with Twitch's picture or its
    /// placeholder.
    fn archive(
        id: &str,
        stream_id: &str,
        created_at: &str,
        length_secs: u64,
        placeholder: bool,
    ) -> Video {
        Video {
            id: id.into(),
            stream_id: Some(stream_id.into()),
            user_id: "2".into(),
            user_login: "someone".into(),
            user_name: "Someone".into(),
            title: String::new(),
            created_at: created_at.into(),
            length_secs,
            thumbnail_url: if placeholder {
                "https://vod-secure.twitch.tv/_404/404_processing_%{width}x%{height}.png".into()
            } else {
                "https://cdn/x.jpg".into()
            },
            view_count: 0,
            kind: VideoKind::Archive,
            muted_segments: Vec::new(),
        }
    }

    #[test]
    fn a_timeline_is_offered_only_for_a_believable_broadcast() {
        let now = at("2026-10-02T20:00:00Z");
        assert_eq!(span(at("2026-10-02T18:00:00Z"), now), Some(7200.0));
        assert_eq!(
            span(at("2026-10-02T19:59:50Z"), now),
            None,
            "ten seconds in: nothing behind the edge to go back to"
        );
        assert_eq!(span(at("2026-10-02T20:05:00Z"), now), None, "a start ahead");
        assert_eq!(
            span(at("2026-09-30T18:00:00Z"), now),
            None,
            "two days back is an older broadcast's start"
        );
        assert_eq!(
            span(at("2026-09-30T20:00:00Z"), now),
            Some(LONGEST_SECS),
            "exactly the longest a stream runs"
        );
    }

    #[test]
    fn a_drag_lands_on_seconds_inside_the_broadcast() {
        assert_eq!(secs_at(0.0, 7200.0), 0.0);
        assert_eq!(secs_at(0.25, 7200.0), 1800.0);
        assert_eq!(secs_at(1.0, 7200.0), 7200.0);
        assert_eq!(secs_at(-0.5, 7200.0), 0.0, "off the left end");
        assert_eq!(secs_at(1.5, 7200.0), 7200.0, "off the right end");
        assert_eq!(secs_at(0.5, -10.0), 0.0, "no length, no time");
    }

    #[test]
    fn a_press_at_the_live_edge_does_nothing() {
        assert!(near_edge(7200.0, 7200.0));
        assert!(near_edge(7180.0, 7200.0), "twenty seconds back");
        assert!(!near_edge(7170.0, 7200.0), "thirty seconds back");
        assert!(!near_edge(0.0, 7200.0));

        let started = at("2026-10-02T18:00:00Z");
        let now = at("2026-10-02T20:00:00Z");
        assert_eq!(pressed_at(started, 1.0, now), None);
        assert_eq!(pressed_at(started, 0.999, now), None, "seven seconds back");
        assert_eq!(
            pressed_at(started, 0.5, now),
            Some(at("2026-10-02T19:00:00Z"))
        );
        assert_eq!(pressed_at(started, 0.0, now), Some(started));
        assert_eq!(
            pressed_at(started, 0.5, at("2026-10-02T18:00:10Z")),
            None,
            "no timeline is offered ten seconds in"
        );
    }

    /// The channel's newest, newest first, as the worker answers: today's
    /// broadcast, still being made and listed a while ago, and yesterday's.
    fn listed() -> Vec<Video> {
        vec![
            archive("today", "stream-today", "2026-10-02T18:00:05Z", 6800, false),
            archive(
                "yesterday",
                "stream-yesterday",
                "2026-10-01T18:00:00Z",
                4 * 3600,
                false,
            ),
        ]
    }

    /// When today's broadcast began, as the live list has it.
    fn today_since() -> DateTime<Utc> {
        at("2026-10-02T18:00:00Z")
    }

    fn chosen(
        archives: &[Video],
        broadcast: Option<&str>,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<String> {
        archive_for(archives, broadcast, since, now).map(|video| video.id)
    }

    #[test]
    fn the_archive_being_made_now_is_the_one_chosen() {
        let now = at("2026-10-02T20:00:00Z");
        let since = today_since();
        assert_eq!(
            chosen(&listed(), Some("stream-today"), since, now).as_deref(),
            Some("today")
        );
        assert_eq!(
            chosen(&listed(), None, since, now).as_deref(),
            Some("today"),
            "without the broadcast's id, the newest going on now"
        );
        assert_eq!(
            chosen(&listed(), Some("stream-yesterday"), since, now).as_deref(),
            Some("today"),
            "a stale id never reaches yesterday's"
        );
    }

    /// A channel that keeps no past broadcasts, or one Twitch has not listed
    /// yet: no archive, rather than yesterday's.
    #[test]
    fn no_archive_of_this_broadcast_is_no_answer() {
        let now = at("2026-10-02T20:00:00Z");
        let since = today_since();
        let yesterday_only = vec![listed().remove(1)];
        assert_eq!(chosen(&yesterday_only, None, since, now), None);
        assert_eq!(
            chosen(&yesterday_only, Some("stream-today"), since, now),
            None
        );
        assert_eq!(chosen(&[], None, since, now), None);
    }

    /// The broadcast before a restart: it ended at 18:00, the stream came
    /// back at 18:03, and at 18:06 its archive is still listed as going on
    /// until close to now, maybe still with the placeholder picture.
    fn before_restart(placeholder: bool) -> Video {
        archive(
            "earlier",
            "stream-earlier",
            "2026-10-02T14:00:00Z",
            4 * 3600,
            placeholder,
        )
    }

    /// Before Twitch lists the new broadcast's archive, the one that ended
    /// minutes before it began is the only one going on lately: no answer,
    /// whatever its id or its picture.
    #[test]
    fn the_archive_before_a_restart_is_never_chosen() {
        let since = at("2026-10-02T18:03:00Z");
        let now = at("2026-10-02T18:06:00Z");
        for placeholder in [false, true] {
            let earlier = vec![before_restart(placeholder)];
            assert_eq!(chosen(&earlier, None, since, now), None);
            assert_eq!(chosen(&earlier, Some("stream-new"), since, now), None);
            assert_eq!(
                chosen(&earlier, Some("stream-earlier"), since, now),
                None,
                "a stale id names it outright"
            );
        }
    }

    /// Both listed, and the id the pane holds is still the broadcast before
    /// the restart's: the archive of the one on screen.
    #[test]
    fn a_stale_id_after_a_restart_finds_the_current_archive() {
        let since = at("2026-10-02T18:03:00Z");
        let now = at("2026-10-02T18:06:00Z");
        let both = vec![
            archive("new", "stream-new", "2026-10-02T18:03:04Z", 120, false),
            before_restart(false),
        ];
        assert_eq!(
            chosen(&both, Some("stream-earlier"), since, now).as_deref(),
            Some("new")
        );
        assert_eq!(chosen(&both, None, since, now).as_deref(), Some("new"));
    }

    /// Listed long ago, so its length puts its end far behind now, but
    /// still wearing the placeholder picture: it is being made now.
    #[test]
    fn a_placeholder_picture_finds_an_archive_listed_long_ago() {
        let now = at("2026-10-02T22:00:00Z");
        let since = today_since();
        let stale = vec![archive(
            "today",
            "stream-today",
            "2026-10-02T18:00:05Z",
            600,
            true,
        )];
        assert_eq!(
            chosen(&stale, Some("stream-today"), since, now).as_deref(),
            Some("today")
        );
        let finished = vec![archive(
            "today",
            "stream-today",
            "2026-10-02T18:00:05Z",
            600,
            false,
        )];
        assert_eq!(chosen(&finished, Some("stream-today"), since, now), None);
    }

    /// A pane playing today's archive while the live list still carries
    /// today's broadcast is offered the way back, however long ago its copy
    /// of the archive was listed; yesterday's never is.
    #[test]
    fn the_way_back_is_offered_on_the_archive_of_the_broadcast_on_now() {
        let since = today_since();
        let now = at("2026-10-02T23:00:00Z");
        let [today, yesterday] = <[Video; 2]>::try_from(listed()).unwrap();
        assert!(live_now(&today, Some("stream-today"), since, now));
        assert!(
            live_now(&today, None, since, now),
            "a list with no id goes by the start alone"
        );
        let mut opened_early = today.clone();
        opened_early.length_secs = 30;
        assert!(
            live_now(&opened_early, Some("stream-today"), since, now),
            "listed thirty seconds in, five hours ago"
        );
        assert!(!live_now(&yesterday, Some("stream-today"), since, now));
        assert!(!live_now(&yesterday, None, since, now));
    }

    /// The broadcast before a restart, opened from its channel's page while
    /// it was still listed as going on: a new broadcast on now is no way
    /// back to it, whatever its picture says.
    #[test]
    fn the_archive_before_a_restart_offers_no_way_back() {
        let since = at("2026-10-02T18:03:00Z");
        let now = at("2026-10-02T18:06:00Z");
        for placeholder in [false, true] {
            let earlier = before_restart(placeholder);
            assert!(!live_now(&earlier, Some("stream-new"), since, now));
            assert!(!live_now(&earlier, None, since, now));
        }
    }

    /// The live list's id is as fresh as its start, so one that names
    /// another broadcast is a snapshot of that one, not the archive's.
    #[test]
    fn a_list_naming_another_broadcast_offers_no_way_back() {
        let since = today_since();
        let now = at("2026-10-02T20:00:00Z");
        let today = &listed()[0];
        assert!(!live_now(today, Some("stream-tomorrow"), since, now));
        assert!(
            live_now(today, Some(""), since, now),
            "an empty id says nothing"
        );
    }

    /// Only a broadcast's archive, and only against a start that can be
    /// the broadcast going on now.
    #[test]
    fn the_way_back_needs_an_archive_and_a_believable_start() {
        let since = today_since();
        let now = at("2026-10-02T20:00:00Z");
        for kind in [VideoKind::Highlight, VideoKind::Upload, VideoKind::Other] {
            let mut cut = listed().remove(0);
            cut.kind = kind;
            assert!(!live_now(&cut, Some("stream-today"), since, now));
        }
        let today = &listed()[0];
        assert!(
            !live_now(today, None, since, at("2026-10-05T20:00:00Z")),
            "a start three days back is an older broadcast's"
        );
        assert!(
            !live_now(today, None, since, at("2026-10-02T18:00:01Z")),
            "an archive that has not begun yet"
        );
    }

    /// The follows poll is fresher than any list fetched once and kept: a
    /// followed channel it has off offers no way back, whatever Popular, a
    /// category or a search still carries.
    #[test]
    fn a_followed_channel_the_poll_has_off_offers_no_way_back() {
        let since = today_since();
        let now = at("2026-10-02T20:00:00Z");
        let today = &listed()[0];
        let rewound = Origin::Rewound { since };
        assert!(!back_to_live(today, Listed::Off, &rewound, now));
        assert!(!back_to_live(today, Listed::Off, &Origin::Unknown, now));
        let on = Listed::On {
            since,
            broadcast: Some("stream-today"),
        };
        assert!(back_to_live(today, on, &Origin::Unknown, now));
    }

    /// A recording opened from a pane that saw its broadcast end is not
    /// offered the way back on a list a poll behind that still names that
    /// broadcast; a new broadcast on the same channel is not the archive's
    /// either, which `live_now` already says.
    #[test]
    fn a_broadcast_seen_ending_offers_no_way_back() {
        let since = today_since();
        let now = at("2026-10-02T20:00:00Z");
        let today = &listed()[0];
        let lagging = Listed::On {
            since,
            broadcast: Some("stream-today"),
        };
        let ended = Origin::Ended {
            broadcast: Some("stream-today".into()),
        };
        assert!(!back_to_live(today, lagging, &ended, now));
        let unknown_end = Origin::Ended { broadcast: None };
        assert!(
            back_to_live(today, lagging, &unknown_end, now),
            "with no id seen, the list decides"
        );
        let no_id = Listed::On {
            since,
            broadcast: Some(""),
        };
        assert!(back_to_live(today, no_id, &ended, now));
    }

    /// No list carrying the channel: a pane that rewound into the archive
    /// goes by the broadcast it rewound from, and anything else has no way
    /// back.
    #[test]
    fn a_rewound_pane_keeps_its_way_back_when_no_list_carries_the_channel() {
        let since = today_since();
        let now = at("2026-10-02T20:00:00Z");
        let [today, yesterday] = <[Video; 2]>::try_from(listed()).unwrap();
        let rewound = Origin::Rewound { since };
        assert!(back_to_live(&today, Listed::Unknown, &rewound, now));
        assert!(!back_to_live(&yesterday, Listed::Unknown, &rewound, now));
        assert!(!back_to_live(
            &today,
            Listed::Unknown,
            &Origin::Unknown,
            now
        ));
        let ended = Origin::Ended { broadcast: None };
        assert!(!back_to_live(&today, Listed::Unknown, &ended, now));
        assert!(
            !back_to_live(
                &today,
                Listed::Unknown,
                &rewound,
                at("2026-10-05T20:00:00Z")
            ),
            "not past the longest a broadcast can be"
        );
    }

    #[test]
    fn a_moment_lands_in_the_archive_by_its_own_start() {
        let now = at("2026-10-02T20:00:00Z");
        let today = &listed()[0];
        assert_eq!(
            position_in(today, at("2026-10-02T19:00:05Z"), now),
            3600.0,
            "five seconds after the stream's start is the archive's zero"
        );
        assert_eq!(
            position_in(today, at("2026-10-02T18:00:00Z"), now),
            0.0,
            "before the archive began is its start"
        );
        assert_eq!(
            position_in(today, now, now),
            7195.0 - EDGE_SECS,
            "never past what the archive can have written"
        );
    }

    #[test]
    fn an_archive_kept_since_opens_as_long_as_it_is_now() {
        let now = at("2026-10-02T20:00:00Z");
        let today = &listed()[0];
        let fresh = as_of(today, now);
        assert_eq!(fresh.length_secs, 7195);
        assert!(channel_page::in_progress(&fresh, now));
        let later = at("2026-10-02T23:00:00Z");
        assert!(!channel_page::in_progress(today, later), "as listed");
        assert!(channel_page::in_progress(&as_of(today, later), later));
    }

    /// A press on today's broadcast at `rfc3339`.
    fn moment(rfc3339: &str) -> Moment {
        Moment {
            at: at(rfc3339),
            since: today_since(),
        }
    }

    #[test]
    fn a_press_asks_once_then_waits_then_opens_what_was_found() {
        let mut rewind = Rewind::default();
        let first = moment("2026-10-02T19:00:00Z");
        let second = moment("2026-10-02T19:30:00Z");
        assert_eq!(rewind.press(first), Press::Ask);
        rewind.asked();
        assert!(rewind.asking());
        assert_eq!(rewind.press(second), Press::Wait, "one ask at a time");
        assert_eq!(rewind.waiting(), Some(second));

        let today = listed().remove(0);
        assert_eq!(
            rewind.settle(Some(today.clone())),
            Some(second),
            "the newest press is the one carried out"
        );
        assert!(!rewind.asking());
        assert_eq!(rewind.archive(), Some(&today));
        assert_eq!(rewind.press(first), Press::Open(Box::new(today)));
    }

    /// The stream restarted while the pane played on: the archive kept for
    /// the broadcast before is let go, and the press asks afresh.
    #[test]
    fn an_archive_kept_for_another_broadcast_is_not_opened() {
        let mut rewind = Rewind::default();
        let first = moment("2026-10-02T19:00:00Z");
        assert_eq!(rewind.press(first), Press::Ask);
        rewind.asked();
        rewind.settle(Some(listed().remove(0)));

        let restarted = Moment {
            at: at("2026-10-02T21:10:00Z"),
            since: at("2026-10-02T21:00:00Z"),
        };
        assert_eq!(rewind.press(restarted), Press::Ask);
        assert_eq!(rewind.archive(), None);
    }

    /// Nothing found is not kept: Twitch may list it a minute later, so the
    /// next press asks again.
    #[test]
    fn nothing_found_asks_again_at_the_next_press() {
        let mut rewind = Rewind::default();
        let pressed = moment("2026-10-02T19:00:00Z");
        assert_eq!(rewind.press(pressed), Press::Ask);
        rewind.asked();
        assert_eq!(rewind.settle(None), Some(pressed));
        assert_eq!(rewind.archive(), None);
        assert_eq!(rewind.press(pressed), Press::Ask);
    }

    /// An answer with no press waiting — the ask was out when the pane
    /// stopped, or the press could not be sent — carries nothing out.
    #[test]
    fn a_press_that_went_nowhere_is_forgotten() {
        let mut rewind = Rewind::default();
        let pressed = moment("2026-10-02T19:00:00Z");
        assert_eq!(rewind.press(pressed), Press::Ask);
        rewind.unasked();
        assert!(!rewind.asking());
        assert_eq!(rewind.settle(Some(listed().remove(0))), None);
        assert_eq!(rewind.archive(), None, "found for no press, kept for none");

        assert_eq!(rewind.press(pressed), Press::Ask);
        rewind.asked();
        rewind.forget();
        assert!(!rewind.asking());
        assert_eq!(rewind.press(pressed), Press::Ask, "asked afresh");
    }
}
