//! A channel's stream schedule: the broadcasts it has said are coming, and
//! whether it is on a break.
//!
//! Helix's Get Channel Stream Schedule (`/schedule?broadcaster_id=`) answers
//! with the segments that start after now, soonest first, each with its start
//! (and usually its end), a title and a category that may both be missing,
//! and `canceled_until`, which is set on an occurrence of a recurring segment
//! the broadcaster has called off and null otherwise. Beside them, `vacation`
//! is the break the broadcaster has set, if any, during which every segment
//! is called off. It takes the user's token (an app token would do too) and
//! needs no scope. A channel that has never made a schedule answers HTTP 404,
//! which is not a failure but an empty schedule ([`channel_schedule`]).
//!
//! Helix, like the rest of this crate's root, and kept in its own file only
//! because the root is long enough already. `fixtures/schedule.json` is an
//! answer in the shape Twitch's reference gives, and what the parser is
//! tested against.

use serde_json::Value;

use super::{helix_get, text, Error};

/// How many segments to ask for. The app shows only the next one that is
/// going ahead, so a handful is room for a few called off ahead of it
/// without asking for Helix's 25.
const SEGMENTS_ASKED: &str = "10";

/// One broadcast a channel has scheduled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segment {
    /// When it is to start, in RFC 3339 (UTC).
    pub start_time: String,
    /// Empty when the broadcaster gave it none.
    pub title: String,
    /// The category's name, `None` when the segment has none.
    pub category: Option<String>,
    /// This occurrence is called off (`canceled_until` is set).
    pub canceled: bool,
}

/// A break the broadcaster has set, during which nothing it scheduled goes
/// ahead. Both ends in RFC 3339 (UTC).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vacation {
    pub start_time: String,
    pub end_time: String,
}

/// What a channel has said is coming. Empty for a channel with no schedule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schedule {
    /// Soonest first, as Helix lists them.
    pub segments: Vec<Segment>,
    pub vacation: Option<Vacation>,
}

/// What a schedule answer means. Not a list like the rest of Helix's
/// answers: `data` is one object, with the segments inside it. A segment
/// with no start is left out, there being nothing to say about when; so is a
/// break missing either end.
fn parse_schedule(json: &Value) -> Schedule {
    let Some(data) = json.get("data") else {
        return Schedule::default();
    };
    let segments = data
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|segment| {
            Some(Segment {
                start_time: text(segment, "start_time")?.to_string(),
                title: text(segment, "title").unwrap_or_default().to_string(),
                category: segment
                    .get("category")
                    .and_then(|category| text(category, "name"))
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
                canceled: text(segment, "canceled_until").is_some(),
            })
        })
        .collect();
    let vacation = data.get("vacation").and_then(|vacation| {
        Some(Vacation {
            start_time: text(vacation, "start_time")?.to_string(),
            end_time: text(vacation, "end_time")?.to_string(),
        })
    });
    Schedule { segments, vacation }
}

/// One channel's schedule, by its numeric id: the segments from now on, and
/// its break if it has set one. A channel that has never scheduled anything
/// is an empty schedule rather than an error, which is how Helix's 404 is
/// read here and nowhere else.
pub fn channel_schedule(
    client_id: &str,
    token: &str,
    broadcaster_id: &str,
) -> Result<Schedule, Error> {
    match helix_get(
        client_id,
        token,
        "/schedule",
        &[
            ("broadcaster_id", broadcaster_id),
            ("first", SEGMENTS_ASKED),
        ],
    ) {
        Ok(json) => Ok(parse_schedule(&json)),
        Err(Error::NotFound) => Ok(Schedule::default()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A schedule answer in the shape Twitch's reference gives: an occurrence
    /// called off, one going ahead, and a one-off with no title, no category
    /// and no end.
    const SCHEDULE_ANSWER: &str = include_str!("fixtures/schedule.json");

    #[test]
    fn a_schedule_answer_is_its_segments_soonest_first() {
        let json: Value = serde_json::from_str(SCHEDULE_ANSWER).unwrap();
        assert_eq!(
            parse_schedule(&json),
            Schedule {
                segments: vec![
                    Segment {
                        start_time: "2026-10-06T23:00:00Z".into(),
                        title: "Pinball tournament, week three".into(),
                        category: Some("Just Chatting".into()),
                        canceled: true,
                    },
                    Segment {
                        start_time: "2026-10-08T00:00:00Z".into(),
                        title: "Elden Ring, the DLC at last".into(),
                        category: Some("Elden Ring".into()),
                        canceled: false,
                    },
                    Segment {
                        start_time: "2026-10-10T17:00:00Z".into(),
                        title: String::new(),
                        category: None,
                        canceled: false,
                    },
                ],
                vacation: None,
            }
        );
    }

    /// A break is kept with both its ends; a segment with no start, and a
    /// break missing an end, are not.
    #[test]
    fn a_break_is_kept_and_the_broken_bits_are_not() {
        let on_break = json!({ "data": {
            "segments": [{ "title": "no start" }],
            "vacation": {
                "start_time": "2026-10-01T00:00:00Z",
                "end_time": "2026-10-12T23:59:59Z"
            }
        }});
        assert_eq!(
            parse_schedule(&on_break),
            Schedule {
                segments: Vec::new(),
                vacation: Some(Vacation {
                    start_time: "2026-10-01T00:00:00Z".into(),
                    end_time: "2026-10-12T23:59:59Z".into(),
                }),
            }
        );
        let half = json!({ "data": {
            "segments": null,
            "vacation": { "start_time": "2026-10-01T00:00:00Z" }
        }});
        assert_eq!(parse_schedule(&half), Schedule::default());
        assert_eq!(parse_schedule(&json!({})), Schedule::default());
    }
}
