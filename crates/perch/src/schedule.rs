//! When an offline channel says it will be on next, worked out: what Helix's
//! answers are kept as, which scheduled broadcast is the next one, and the
//! line that says it ("Next stream: Tue 7:00 PM · Title · Game", or "On a
//! break until 12 Oct"). Pure and tested. The asking is `root::schedule`,
//! the request is the worker's `Request::Schedule`, the data is
//! `twitch_api::schedule`, and the drawing is the channel page's header
//! (`channel_page::view`) and an offline pane's status screen
//! (`watch::status`).
//!
//! Only where somebody is deciding about one channel: its page, open while it
//! is off, and a pane on it that found it off. Not for every offline follow
//! on Home or in the rail, which would be one Helix request per follow; that
//! is what `crate::last_live` is for, a hundred logins a request. A channel
//! is asked about once a session, the first time one of those views draws it
//! while somebody is signed in, and the answer is kept for the session: a
//! schedule is set days ahead, and the line is worked out from it at draw
//! time, so a broadcast whose time has passed gives way to the one after it
//! without asking again. A failure is a line in the log, once, and no line on
//! screen; the channel is not asked about again this session. Nothing here
//! is saved.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike as _, Local, TimeZone, Utc};
use settings::channel_key;
use twitch_api::schedule::{Schedule, Segment};

use crate::clock;

/// Every schedule asked for this session, and the asks for them, by
/// `settings::channel_key`. Lives on the root and is never saved.
#[derive(Debug, Default)]
pub struct Schedules {
    /// Each channel's answer. An empty schedule is an answer too: the
    /// channel has none, and is not asked about again.
    answers: HashMap<String, Schedule>,
    /// Channels with an ask out that the worker has not answered.
    out: HashSet<String>,
    /// Channels whose ask failed, which are not asked about again this
    /// session, so a channel Helix keeps failing on costs one line in the
    /// log rather than one a frame.
    failed: HashSet<String>,
}

impl Schedules {
    /// Whether `login` is worth asking the worker about now: never answered,
    /// never failed, and not already asked about.
    pub fn wants(&self, login: &str) -> bool {
        let key = channel_key(login);
        !self.answers.contains_key(&key) && !self.out.contains(&key) && !self.failed.contains(&key)
    }

    /// Note that an ask about `login` has gone to the worker.
    pub fn asked(&mut self, login: &str) {
        self.out.insert(channel_key(login));
    }

    /// Take the worker's answer about `login`, and say what is worth a line
    /// in the log, if anything: a failure, which is the last ask about that
    /// channel this session.
    pub fn answered(&mut self, login: &str, result: Result<Schedule, String>) -> Option<String> {
        let key = channel_key(login);
        self.out.remove(&key);
        match result {
            Ok(schedule) => {
                self.answers.insert(key, schedule);
                None
            }
            Err(reason) => {
                self.failed.insert(key);
                Some(format!("{login}: {reason}"))
            }
        }
    }

    /// Forget the asks out, for a worker that will never answer them: the
    /// next view to draw one of those channels asks again.
    pub fn forget(&mut self) {
        self.out.clear();
    }

    /// The line for `login` at `now`, in this machine's time zone and clock
    /// ([`clock::stamp`]), or `None` when there is nothing to say: no answer
    /// yet, no schedule, or nothing on it still to come.
    pub fn words(&self, login: &str, now: DateTime<Utc>) -> Option<String> {
        let schedule = self.answers.get(&channel_key(login))?;
        let next = next(schedule, now)?;
        Some(wording(&next, now, &Local, |when| clock::stamp(*when)))
    }
}

/// What a schedule says is next.
#[derive(Debug, Clone, PartialEq)]
pub enum Next<'a> {
    /// The broadcaster is on a break that has begun and ends then.
    Break { until: DateTime<Utc> },
    /// This broadcast, which starts then.
    Stream {
        segment: &'a Segment,
        at: DateTime<Utc>,
    },
}

/// What `schedule` says is next at `now`: the break, while one is on;
/// otherwise the soonest broadcast still to start that has not been called
/// off, and does not fall in a break still to come (Twitch calls those off
/// itself, but a segment it has not marked would otherwise promise a
/// broadcast the channel has said it will not make). `None` when there is
/// none, or a time does not parse.
pub fn next(schedule: &Schedule, now: DateTime<Utc>) -> Option<Next<'_>> {
    let vacation = schedule
        .vacation
        .as_ref()
        .and_then(|vacation| Some((parse(&vacation.start_time)?, parse(&vacation.end_time)?)));
    if let Some((start, end)) = vacation {
        if start <= now && now < end {
            return Some(Next::Break { until: end });
        }
    }
    let on_break = |at: DateTime<Utc>| vacation.is_some_and(|(start, end)| start <= at && at < end);
    schedule
        .segments
        .iter()
        .filter(|segment| !segment.canceled)
        .filter_map(|segment| Some((segment, parse(&segment.start_time)?)))
        .filter(|(_, at)| *at > now && !on_break(*at))
        .min_by_key(|(_, at)| *at)
        .map(|(segment, at)| Next::Stream { segment, at })
}

fn parse(rfc3339: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|when| when.with_timezone(&Utc))
}

/// The line for `next`, at `now`, with days counted on `zone`'s calendar and
/// a time of day written by `time_of_day` (the system's short time format,
/// `clock::stamp`, outside tests).
///
/// A broadcast reads "Next stream: Tue 7:00 PM", then its title and its
/// category where it has them, each after a " · " as a pane header's facts
/// are. The day is "Today" or "Tomorrow" when it is one of those, the
/// weekday's short name inside the week, and past that the date the way the
/// channel page writes one ("14 Oct", with the year when it is not this
/// year's), and the weekday with it: "Tue 14 Oct". A break reads "On a break
/// until 12 Oct", by the day it ends on.
pub fn wording<Tz: TimeZone>(
    next: &Next<'_>,
    now: DateTime<Utc>,
    zone: &Tz,
    time_of_day: impl Fn(&DateTime<Tz>) -> String,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let today = now.with_timezone(zone);
    match next {
        Next::Break { until } => {
            let until = until.with_timezone(zone);
            format!("On a break until {}", date(&until, &today))
        }
        Next::Stream { segment, at } => {
            let at = at.with_timezone(zone);
            let days = (at.date_naive() - today.date_naive()).num_days();
            let day = match days {
                i64::MIN..=0 => "Today".to_string(),
                1 => "Tomorrow".to_string(),
                2..=6 => at.format("%a").to_string(),
                _ => format!("{} {}", at.format("%a"), date(&at, &today)),
            };
            let mut line = format!("Next stream: {day} {}", time_of_day(&at));
            for fact in [Some(segment.title.trim()), segment.category.as_deref()]
                .into_iter()
                .flatten()
                .filter(|fact| !fact.is_empty())
            {
                line.push_str(" · ");
                line.push_str(fact);
            }
            line
        }
    }
}

/// A day the way the channel page writes one (`channel_page::when`): "14
/// Oct", with the year when it is not `today`'s.
fn date<Tz: TimeZone>(when: &DateTime<Tz>, today: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if when.year() == today.year() {
        when.format("%-d %b").to_string()
    } else {
        when.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use twitch_api::schedule::Vacation;

    fn at(when: &str) -> DateTime<Utc> {
        parse(when).unwrap()
    }

    fn segment(start_time: &str, title: &str, category: Option<&str>, canceled: bool) -> Segment {
        Segment {
            start_time: start_time.into(),
            title: title.into(),
            category: category.map(str::to_string),
            canceled,
        }
    }

    /// US Eastern in October, and a twelve-hour clock, so the tests read the
    /// same wherever they run.
    fn eastern() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }

    fn says(next: &Next<'_>, now: DateTime<Utc>) -> String {
        wording(next, now, &eastern(), |when| {
            when.format("%-I:%M %p").to_string()
        })
    }

    /// The soonest broadcast still to come that is going ahead: not the one
    /// called off ahead of it, nor one already started, whatever order they
    /// were listed in.
    #[test]
    fn the_next_stream_is_the_soonest_going_ahead() {
        let schedule = Schedule {
            segments: vec![
                segment("2026-10-10T17:00:00Z", "", None, false),
                segment("2026-10-06T23:00:00Z", "Pinball", None, true),
                segment("2026-10-03T12:00:00Z", "Started", None, false),
                segment("2026-10-08T00:00:00Z", "Elden Ring", None, false),
            ],
            vacation: None,
        };
        let now = at("2026-10-03T16:00:00Z");
        match next(&schedule, now) {
            Some(Next::Stream { segment, at: when }) => {
                assert_eq!(segment.title, "Elden Ring");
                assert_eq!(when, at("2026-10-08T00:00:00Z"));
            }
            other => panic!("expected the Elden Ring stream, got {other:?}"),
        }
        assert_eq!(next(&Schedule::default(), now), None, "no schedule");
        let all_off = Schedule {
            segments: vec![segment("2026-10-06T23:00:00Z", "", None, true)],
            vacation: None,
        };
        assert_eq!(next(&all_off, now), None, "everything called off");
    }

    /// A break that has begun is what is said; one still to come only
    /// passes over what falls in it; one that is over is nothing.
    #[test]
    fn a_break_is_said_while_it_is_on() {
        let schedule = Schedule {
            segments: vec![
                segment("2026-10-05T23:00:00Z", "In the break", None, false),
                segment("2026-10-14T23:00:00Z", "Back", None, false),
            ],
            vacation: Some(Vacation {
                start_time: "2026-10-04T00:00:00Z".into(),
                end_time: "2026-10-12T23:59:59Z".into(),
            }),
        };
        assert_eq!(
            next(&schedule, at("2026-10-06T12:00:00Z")),
            Some(Next::Break {
                until: at("2026-10-12T23:59:59Z")
            })
        );
        let before = next(&schedule, at("2026-10-03T12:00:00Z"));
        assert!(
            matches!(before, Some(Next::Stream { segment, .. }) if segment.title == "Back"),
            "a broadcast inside a coming break is not next, got {before:?}"
        );
        let after = next(&schedule, at("2026-10-13T12:00:00Z"));
        assert!(
            matches!(after, Some(Next::Stream { segment, .. }) if segment.title == "Back"),
            "a break that is over says nothing, got {after:?}"
        );
    }

    /// Today, tomorrow, a weekday inside the week, a date past it; the
    /// title and the category after it where there are any.
    #[test]
    fn the_line_reads_like_a_listing() {
        // Saturday 3 October, noon in New York.
        let now = at("2026-10-03T16:00:00Z");
        let elden = segment("", "Elden Ring, the DLC", Some("Elden Ring"), false);
        let stream = |when: &str| Next::Stream {
            segment: &elden,
            at: at(when),
        };
        assert_eq!(
            says(&stream("2026-10-03T23:00:00Z"), now),
            "Next stream: Today 7:00 PM · Elden Ring, the DLC · Elden Ring"
        );
        assert_eq!(
            says(&stream("2026-10-04T23:00:00Z"), now),
            "Next stream: Tomorrow 7:00 PM · Elden Ring, the DLC · Elden Ring"
        );
        // Midnight UTC on Wednesday is Tuesday evening in New York.
        assert_eq!(
            says(&stream("2026-10-07T00:00:00Z"), now),
            "Next stream: Tue 8:00 PM · Elden Ring, the DLC · Elden Ring"
        );
        assert_eq!(
            says(&stream("2026-10-14T23:00:00Z"), now),
            "Next stream: Wed 14 Oct 7:00 PM · Elden Ring, the DLC · Elden Ring"
        );
        assert_eq!(
            says(&stream("2027-01-05T23:00:00Z"), now),
            "Next stream: Tue 5 Jan 2027 7:00 PM · Elden Ring, the DLC · Elden Ring"
        );

        let bare = segment("", "  ", None, false);
        assert_eq!(
            says(
                &Next::Stream {
                    segment: &bare,
                    at: at("2026-10-05T13:30:00Z"),
                },
                now
            ),
            "Next stream: Mon 9:30 AM",
            "no title and no category leave the time alone"
        );
    }

    /// A break is said by the day it ends on, in the viewer's own calendar.
    #[test]
    fn a_break_reads_by_its_last_day() {
        let now = at("2026-10-06T16:00:00Z");
        assert_eq!(
            says(
                &Next::Break {
                    until: at("2026-10-12T23:59:59Z")
                },
                now
            ),
            "On a break until 12 Oct"
        );
        assert_eq!(
            says(
                &Next::Break {
                    until: at("2027-01-02T12:00:00Z")
                },
                now
            ),
            "On a break until 2 Jan 2027"
        );
    }

    /// One ask a channel, however it is written; an answer, empty or not,
    /// is never asked again, nor is a failure, which is one line.
    #[test]
    fn a_channel_is_asked_about_once() {
        let mut schedules = Schedules::default();
        assert!(schedules.wants("Forsen"));
        schedules.asked("Forsen");
        assert!(!schedules.wants("forsen"), "asked again while out");
        assert_eq!(schedules.answered("forsen", Ok(Schedule::default())), None);
        assert!(!schedules.wants("forsen"), "an empty schedule is an answer");
        assert_eq!(
            schedules.words("forsen", at("2026-10-03T16:00:00Z")),
            None,
            "no schedule, no line"
        );

        schedules.asked("nymn");
        assert_eq!(
            schedules
                .answered("nymn", Err("HTTP 500".into()))
                .as_deref(),
            Some("nymn: HTTP 500")
        );
        assert!(!schedules.wants("nymn"), "a failure is not asked again");

        schedules.asked("lirik");
        schedules.forget();
        assert!(
            schedules.wants("lirik"),
            "an ask a dead worker had is asked again"
        );
    }
}
