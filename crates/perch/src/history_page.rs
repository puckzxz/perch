//! The history tab: every recording opened, newest first, each one where it
//! was left.
//!
//! The thing it answers is "that twelve-hour stream I was halfway through".
//! Without it the way back was the channel, then the recording, then the
//! seek bar, dragged to a moment remembered roughly. Here it is one card, and
//! the card picks up where it was left — so does the same recording found on
//! its channel's page, since opening a recording from anywhere asks the
//! history first.
//!
//! What is listed is `settings::history`; what the root does about it — the
//! noting, the saving, the resuming — is `root::history`. This module is the
//! page, and the one translation between a Twitch video and a history entry,
//! which keeps what a pane needs to play a recording again and nothing else.

use chrono::{DateTime, Utc};
use emotes::ImageCache;
use gpui::{div, prelude::*, AnyElement, Context, ScrollHandle};
use settings::history::{History, Watched};
use twitch_api::{Video, VideoKind};

use crate::browse::{self, Action};
use crate::channel_page::{self, Card};
use crate::controls;
use crate::theme;

/// What the history keeps of `video`, left at `position` of `length` seconds.
///
/// The length is the longer of what Twitch listed and what the player found:
/// a recording listed while it was still being made is as long as it has
/// been played. A recording still being made wears Twitch's placeholder for a
/// picture; once the player knows it is finished (`growing` false) that is
/// dropped, since it would go on saying "still going" on a card for good.
pub fn watched(
    video: &Video,
    position: f64,
    length: f64,
    growing: bool,
    finished: bool,
    now: u64,
) -> Watched {
    let thumbnail_url = if !growing && channel_page::placeholder(&video.thumbnail_url) {
        String::new()
    } else {
        video.thumbnail_url.clone()
    };
    Watched {
        id: video.id.clone(),
        channel_login: video.user_login.clone(),
        channel_name: video.user_name.clone(),
        channel_id: video.user_id.clone(),
        title: video.title.clone(),
        created_at: video.created_at.clone(),
        length_secs: video.length_secs.max(length.max(0.0).floor() as u64),
        thumbnail_url,
        kind: video.kind.as_str().to_string(),
        position_secs: position.max(0.0).floor() as u64,
        finished,
        watched_at: now,
    }
}

/// The video a history entry was made from, as far as the history kept it:
/// enough to list it and to play it, and none of what only a listing shows —
/// its views, its muted stretches.
///
/// Twitch's placeholder picture is dropped here too once the length the
/// history holds puts the broadcast's end well in the past, for an entry
/// noted before the player could say it had finished.
pub fn video(watched: &Watched, now: DateTime<Utc>) -> Video {
    let mut video = Video {
        id: watched.id.clone(),
        stream_id: None,
        user_id: watched.channel_id.clone(),
        user_login: watched.channel_login.clone(),
        user_name: watched.channel().to_string(),
        title: watched.title.clone(),
        created_at: watched.created_at.clone(),
        length_secs: watched.length_secs,
        thumbnail_url: watched.thumbnail_url.clone(),
        view_count: 0,
        kind: VideoKind::parse(&watched.kind),
        muted_segments: Vec::new(),
    };
    if channel_page::placeholder(&video.thumbnail_url)
        && !channel_page::listed_as_going(&video, now)
    {
        video.thumbnail_url.clear();
    }
    video
}

/// A history entry's line under its title: whose it is, what it is unless it
/// is a past broadcast, and when it was.
///
/// The kind because the list mixes them now that a channel's page offers its
/// highlights and uploads too, and a minute-long highlight looked like a
/// broadcast that had barely begun. A past broadcast is most of the list and
/// says nothing, the way a channel's page says nothing on its first shelf.
fn byline(watched: &Watched, now: DateTime<Utc>) -> String {
    let kind = VideoKind::parse(&watched.kind);
    [
        Some(watched.channel().to_string()),
        (kind != VideoKind::Archive).then(|| channel_page::kind_tag(kind).to_string()),
        channel_page::when(&watched.created_at, now),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The tab: what is part-watched first, since picking one of those up is the
/// reason to be here, then what was finished.
#[allow(clippy::too_many_arguments)]
pub fn view<V: 'static>(
    history: &History,
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    if history.videos.is_empty() {
        return browse::notice(
            "Nothing watched yet".into(),
            "Past broadcasts, highlights and uploads you open are kept here, each one \
             where you left it."
                .into(),
            false,
        )
        .into_any_element();
    }

    let now = Utc::now();
    let card_width = browse::card_width(
        width,
        browse::CARD_MIN,
        browse::CARD_MAX,
        theme::GAP_SECTION,
    );
    // Numbered across both sections, since the cards' element ids are made
    // from it and the two rows share one page.
    let (going, done): (Vec<&Watched>, Vec<&Watched>) =
        history.videos.iter().partition(|watched| !watched.finished);
    let mut next_index = 0;
    let mut row_of = |entries: &[&Watched], cx: &mut Context<V>| {
        let mut row = browse::wrap_row(theme::GAP_SECTION);
        for watched in entries {
            let recording = video(watched, now);
            let item = Card {
                index: next_index,
                video: &recording,
                watched: Some(watched),
                byline: byline(watched, now),
                forgettable: true,
            };
            next_index += 1;
            row = row.child(channel_page::card(
                item,
                card_width,
                cache,
                can_add,
                now,
                on_action.clone(),
                cx,
            ));
        }
        row
    };

    // The one control the page has, on the line with the first heading so
    // it takes no row of its own.
    let clear = controls::destructive("clear-history", "clear history").on_click(cx.listener({
        let on_action = on_action.clone();
        move |view, _event, window, cx| on_action(view, Action::ClearHistory, window, cx)
    }));
    let first_heading = if going.is_empty() {
        "finished"
    } else {
        "continue watching"
    };
    let mut list = browse::scroller("history-grid", scroll).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(browse::heading(first_heading))
            .child(div().flex_1())
            .child(clear),
    );
    if !going.is_empty() {
        list = list.child(row_of(&going, cx));
        if !done.is_empty() {
            list = list.child(browse::heading("finished"));
        }
    }
    if !done.is_empty() {
        list = list.child(row_of(&done, cx));
    }
    browse::scrollable(list, scroll).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_video() -> Video {
        Video {
            id: "2884829425".into(),
            stream_id: Some("320493935452".into()),
            user_id: "71092938".into(),
            user_login: "xqc".into(),
            user_name: "xQc".into(),
            title: "WICKED".into(),
            created_at: "2026-09-26T19:48:47Z".into(),
            length_secs: 42_002,
            thumbnail_url:
                "https://static-cdn.jtvnw.net/cf_vods/x/thumb/thumb0-%{width}x%{height}.jpg".into(),
            view_count: 120_000,
            kind: VideoKind::Archive,
            muted_segments: Vec::new(),
        }
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// The contract with `settings::history`: everything a pane needs to play
    /// a recording again survives the trip through the file, and what only a
    /// listing shows does not pretend to.
    #[test]
    fn a_video_comes_back_out_of_the_history_able_to_play() {
        let original = a_video();
        let kept = watched(&original, 4_000.7, 41_990.0, false, false, 7);
        assert_eq!(kept.position_secs, 4_000);
        assert_eq!(kept.kind, "archive");
        assert_eq!(kept.watched_at, 7);

        let back = video(&kept, at("2026-09-27T12:00:00Z"));
        assert_eq!(
            back,
            Video {
                stream_id: None,
                view_count: 0,
                ..original
            }
        );
    }

    /// Listed short while it was still being made, it is as long as the
    /// player found it — never shorter than Twitch said.
    #[test]
    fn the_history_keeps_the_longer_length() {
        let listed = Video {
            length_secs: 1_000,
            ..a_video()
        };
        assert_eq!(
            watched(&listed, 0.0, 5_000.0, true, false, 0).length_secs,
            5_000
        );
        assert_eq!(
            watched(&listed, 0.0, 10.0, false, false, 0).length_secs,
            1_000
        );
    }

    /// Twitch's stand-in picture says "still being recorded", which is true
    /// only while it is: once the player or the calendar says otherwise, the
    /// card has no picture rather than a wrong one.
    #[test]
    fn a_placeholder_picture_is_dropped_once_the_broadcast_is_over() {
        let going = Video {
            thumbnail_url:
                "https://vod-secure.twitch.tv/_404/404_processing_%{width}x%{height}.png".into(),
            created_at: "2026-09-27T10:00:00Z".into(),
            length_secs: 3_600,
            ..a_video()
        };

        // Still growing when the player last said: kept.
        let kept = watched(&going, 60.0, 3_700.0, true, false, 0);
        assert!(channel_page::placeholder(&kept.thumbnail_url));
        // An hour or so on it could still be going, by the listing.
        assert!(channel_page::placeholder(
            &video(&kept, at("2026-09-27T11:05:00Z")).thumbnail_url
        ));
        // A day on it cannot.
        assert_eq!(video(&kept, at("2026-09-28T11:00:00Z")).thumbnail_url, "");

        // Finished, by the player's word: dropped at once.
        assert_eq!(
            watched(&going, 60.0, 3_700.0, false, false, 0).thumbnail_url,
            ""
        );
    }

    /// Midday either side, so a local-time calendar cannot put the two on
    /// different days than UTC's wherever the test runs.
    #[test]
    fn a_history_entry_is_described_by_whose_it_is_and_when() {
        let midday = Video {
            created_at: "2026-09-26T12:00:00Z".into(),
            ..a_video()
        };
        let kept = watched(&midday, 0.0, 0.0, false, false, 0);
        assert_eq!(byline(&kept, at("2026-09-27T12:00:00Z")), "xQc · yesterday");

        let nameless = Watched {
            channel_name: String::new(),
            ..kept.clone()
        };
        assert_eq!(
            byline(&nameless, at("2026-09-27T12:00:00Z")),
            "xqc · yesterday"
        );

        // Anything but a past broadcast says what it is.
        let highlight = Watched {
            kind: VideoKind::Highlight.as_str().into(),
            ..kept.clone()
        };
        assert_eq!(
            byline(&highlight, at("2026-09-27T12:00:00Z")),
            "xQc · highlight · yesterday"
        );
        let upload = Watched {
            kind: VideoKind::Upload.as_str().into(),
            ..kept
        };
        assert_eq!(
            byline(&upload, at("2026-09-27T12:00:00Z")),
            "xQc · upload · yesterday"
        );
    }
}
