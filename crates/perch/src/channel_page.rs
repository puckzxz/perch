//! A channel's page: its past broadcasts, and the way back to what it is
//! doing now.
//!
//! It takes over the browse page the way a category does, so there is only
//! ever one thing to scroll. The cards borrow the stream card's bones — a 16:9
//! picture, a fact in the corner of it, a name and a line under it — because
//! a recording is chosen the same way a stream is, by looking. What is in the
//! corner differs: a stream's card says how many are watching now, and a
//! recording's says how long it is, which is the one number that decides
//! whether there is time for it.
//!
//! The recording's card is the history page's card too — see
//! `history_page` — so a recording looks the same wherever it is found, and
//! one part-watched says how far in it was left on both.

use chrono::{DateTime, Datelike as _, Local, Utc};
use emotes::ImageCache;
use gpui::{
    div, img, prelude::*, px, AnyElement, Context, DefiniteLength, ScrollHandle, SharedString,
};
use settings::history::{History, Watched};
use twitch_api::{Video, VideoKind, VIDEO_THUMBNAIL};

use crate::browse::{self, Action, ChannelPage, Discovery};
use crate::controls;
use crate::seek_bar;
use crate::theme;

/// How long after a broadcast's listed end it may still be going.
///
/// Helix does not say whether a recording is still being made. Twitch's
/// placeholder thumbnail says so while it lasts, and this covers the gap after
/// it: a recording whose listed end is this close to now was still growing
/// when the list was fetched, and reading it as finished would cap its seek
/// bar at the length it had then.
const STILL_GOING_SECS: i64 = 600;

/// The kinds of video a channel's page lists, in the order its switch shows
/// them: what it broadcast, what it cut from that, and what it uploaded.
pub const SHELVES: [VideoKind; 3] = [VideoKind::Archive, VideoKind::Highlight, VideoKind::Upload];

/// What the page's switch calls a kind.
pub fn shelf_name(kind: VideoKind) -> &'static str {
    match kind {
        VideoKind::Archive => "past broadcasts",
        VideoKind::Highlight => "highlights",
        VideoKind::Upload => "uploads",
        VideoKind::Other => "videos",
    }
}

/// The word a pane playing a video of this kind wears in its header, where a
/// live pane has its numbers: what the thing on screen is, since the picture
/// alone does not say.
pub fn kind_tag(kind: VideoKind) -> &'static str {
    match kind {
        VideoKind::Archive => "replay",
        VideoKind::Highlight => "highlight",
        VideoKind::Upload => "upload",
        VideoKind::Other => "video",
    }
}

/// Whether `video` is a broadcast still being recorded, as best the list can
/// tell. See [`STILL_GOING_SECS`] for the two signals.
///
/// Broadcasts only. The second signal is a listed end close to now, which a
/// highlight cut or a video uploaded a few minutes ago has too — and neither
/// is growing, whatever its dates say.
pub fn in_progress(video: &Video, now: DateTime<Utc>) -> bool {
    video.kind == VideoKind::Archive
        && (placeholder(&video.thumbnail_url) || listed_as_going(video, now))
}

/// Whether a thumbnail is the picture Twitch serves in place of one while a
/// broadcast is still being recorded.
pub fn placeholder(thumbnail_url: &str) -> bool {
    thumbnail_url.contains("404_processing")
}

/// Whether the listed length puts the broadcast's end close enough to now
/// that it may still be going. See [`STILL_GOING_SECS`].
pub fn listed_as_going(video: &Video, now: DateTime<Utc>) -> bool {
    let Some(started) = started_at(video) else {
        return false;
    };
    let listed_end = started + chrono::Duration::seconds(video.length_secs as i64);
    listed_end + chrono::Duration::seconds(STILL_GOING_SECS) >= now
}

/// How long a broadcast still being recorded is *now*: the time since it
/// started, which is exact where the listed length is only as fresh as the
/// list. `None` for a video whose start Twitch did not say.
pub fn elapsed_secs(video: &Video, now: DateTime<Utc>) -> Option<f64> {
    let started = started_at(video)?;
    Some(now.signed_duration_since(started).num_milliseconds().max(0) as f64 / 1000.0)
}

fn started_at(video: &Video) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&video.created_at)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// "5h 58m", "58m", or "45s": the shape the cards and the uptime in a
/// stream's header already use, so a length reads like an uptime.
pub fn length(secs: u64) -> String {
    let (hours, minutes) = (secs / 3600, (secs / 60) % 60);
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if secs >= 60 {
        format!("{minutes}m")
    } else {
        format!("{secs}s")
    }
}

/// When a broadcast was, as somebody scanning a list thinks of it: "today",
/// "yesterday", "3 days ago", then the date once it is far enough back that
/// counting stops helping. Days are counted on the local calendar, so a
/// broadcast that ended after midnight is yesterday's rather than two days
/// old at breakfast.
pub fn when(created_at: &str, now: DateTime<Utc>) -> Option<String> {
    let started = DateTime::parse_from_rfc3339(created_at).ok()?;
    let started = started.with_timezone(&Local);
    let today = now.with_timezone(&Local).date_naive();
    let days = (today - started.date_naive()).num_days();
    Some(match days {
        i64::MIN..=0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => format!("{days} days ago"),
        7..=13 => "last week".to_string(),
        14..=27 => format!("{} weeks ago", days / 7),
        _ if started.year() == now.with_timezone(&Local).year() => {
            started.format("%-d %b").to_string()
        }
        _ => started.format("%-d %b %Y").to_string(),
    })
}

/// The line under a recording's title, in a card and in a pane header: when
/// it was and how long it is, or how long it has been so far.
pub fn describe(video: &Video, now: DateTime<Utc>) -> String {
    let going = in_progress(video, now);
    let length = if going {
        elapsed_secs(video, now)
            .map(|secs| format!("{} so far", length(secs as u64)))
            .unwrap_or_else(|| length(video.length_secs))
    } else {
        length(video.length_secs)
    };
    let start = if going {
        Some("streaming now".to_string())
    } else {
        when(&video.created_at, now)
    };
    start
        .into_iter()
        .chain(std::iter::once(length))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How tall the bar along the bottom of a watched recording's picture is.
const WATCHED_BAR: f32 = 3.0;

/// One card's worth: a recording, and what the page showing it knows about
/// it besides. The two pages that show recordings differ only here.
pub(crate) struct Card<'a> {
    /// Unique on the page: the card's element ids are made from it.
    pub index: usize,
    pub video: &'a Video,
    /// Where it was left, if it has been watched.
    pub watched: Option<&'a Watched>,
    /// The line under the title: when it was and how many have watched it,
    /// on a channel's page; whose it is and when, in the history.
    pub byline: String,
    /// Whether the card offers to forget it, which only the history does.
    pub forgettable: bool,
}

/// One past broadcast.
///
/// Clicking the card plays it alone — from where it was left, when it has
/// been watched — and the small "+" adds it beside whatever is already
/// playing, the same two gestures a stream's card has.
pub(crate) fn card<V: 'static>(
    card: Card,
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    now: DateTime<Utc>,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let Card {
        index,
        video,
        watched,
        byline,
        forgettable,
    } = card;
    let on_click = on_action.clone();
    let on_add = on_action.clone();
    let on_forget = on_action;
    let chosen = video.clone();
    let added = video.clone();
    let forgotten = video.id.clone();

    // Kept for good rather than refreshed: a recording's picture is a frame
    // of it and never changes. The placeholder Twitch serves while a
    // broadcast is still being recorded has a URL of its own, so the real
    // picture is a new fetch when it arrives.
    let thumbnail = cache.get_or_request(&twitch_api::thumbnail(
        &video.thumbnail_url,
        VIDEO_THUMBNAIL.0,
        VIDEO_THUMBNAIL.1,
    ));
    let preview_height = px(width * 9.0 / 16.0);
    let preview = match thumbnail {
        Some(path) => img(path).w_full().h(preview_height).into_any_element(),
        // Sized placeholder, so the grid does not reflow as images arrive.
        None => div()
            .w_full()
            .h(preview_height)
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    // The corner a stream's card gives to its viewer count. A recording has
    // one number that matters in the same way, its length; one still being
    // recorded carries the dot, because that number is still moving. One
    // part-watched says where it was left instead, in the seek bar's own
    // words, because that is where a click will take you.
    let going = in_progress(video, now);
    let so_far = going.then(|| elapsed_secs(video, now)).flatten();
    let corner = match watched {
        Some(watched) if watched.finished => format!("watched · {}", length(video.length_secs)),
        Some(watched) if watched.position_secs > 0 => {
            let known = so_far
                .unwrap_or(0.0)
                .max(watched.length_secs.max(video.length_secs) as f64);
            format!(
                "{} / {}",
                seek_bar::timecode(watched.position_secs as f64),
                seek_bar::timecode(known)
            )
        }
        _ => so_far
            .map(|secs| format!("{} so far", length(secs as u64)))
            .unwrap_or_else(|| length(video.length_secs)),
    };

    // How much of it has been watched, the way every video site draws it:
    // along the bottom edge of the picture, in the colour of the seek bar's
    // played part.
    let watched_bar = watched.map(|watched| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(WATCHED_BAR))
            .bg(theme::seek_rail())
            .child(
                div()
                    .h_full()
                    .w(DefiniteLength::Fraction(watched.progress()))
                    .bg(theme::accent()),
            )
    });

    div()
        .id(("video-card", index))
        .w(px(width))
        .flex()
        .flex_col()
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_raised()))
        .active(|style| style.bg(theme::pressed()))
        .on_click(cx.listener(move |view, _event, window, cx| {
            on_click(
                view,
                Action::WatchVideo(Box::new(chosen.clone())),
                window,
                cx,
            )
        }))
        .child(
            div()
                .relative()
                .group("video-card")
                .child(preview)
                .child(
                    controls::badge()
                        .absolute()
                        .bottom(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .when(going, |badge| badge.child(controls::live_dot()))
                        .child(SharedString::from(corner)),
                )
                .children(watched_bar)
                // In the corner the live card gives to its channel's page,
                // revealed the same way, so a card at rest is still a
                // picture.
                .when(forgettable, |thumb| {
                    thumb.child(
                        controls::pill(("forget-video", index), "forget", controls::Variant::Pill)
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .left(px(theme::GAP_TIGHT))
                            .opacity(0.0)
                            .group_hover("video-card", |style| style.opacity(1.0))
                            .tooltip(|window, cx| {
                                gpui_component::tooltip::Tooltip::new(
                                    "Take this off your history, and where you left it",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                // Or the card underneath plays what was just
                                // forgotten.
                                cx.stop_propagation();
                                on_forget(view, Action::ForgetVideo(forgotten.clone()), window, cx)
                            })),
                    )
                })
                .when(can_add, |thumb| {
                    thumb.child(
                        controls::pill(("add-video", index), "+ add", controls::Variant::Pill)
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .right(px(theme::GAP_TIGHT))
                            .opacity(0.0)
                            .group_hover("video-card", |style| style.opacity(1.0))
                            .tooltip(|window, cx| {
                                gpui_component::tooltip::Tooltip::new("Open beside what is playing")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                // Without this the card underneath also fires
                                // and replaces every open pane.
                                cx.stop_propagation();
                                on_add(view, Action::AddVideo(Box::new(added.clone())), window, cx)
                            })),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_TIGHT))
                .p(px(theme::PANEL_PAD))
                .child(
                    browse::one_line(("video-title", index), video.title.clone())
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text()),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(byline)),
                ),
        )
}

/// A recording's line on its channel's page: when it was, and how many have
/// watched it.
fn byline(video: &Video, now: DateTime<Utc>) -> String {
    [
        when(&video.created_at, now),
        Some(format!(
            "{} views",
            browse::format_viewers(video.view_count)
        )),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The page: a bar saying whose broadcasts these are and how to leave, then
/// the grid.
///
/// `live` is whether the channel is on right now, which decides the one other
/// control the bar offers: the stream, when there is one, or else the chat,
/// which is what clicking an offline name used to open and stays one click
/// away.
///
/// `history` is what has been watched, so a recording part-watched says so on
/// its card and a click on it picks up where it was left.
#[allow(clippy::too_many_arguments)]
pub fn view<V: 'static>(
    channel: &ChannelPage,
    discovery: &Discovery,
    history: &History,
    live: bool,
    width: f32,
    cache: &ImageCache,
    can_add: bool,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let now = Utc::now();
    let videos = channel.videos();

    // Which kind is on screen, where the page's one heading used to say
    // "past broadcasts": the same pills as the browse tabs, because it is the
    // same choice one level down.
    let mut shelves = div().flex().flex_row().gap(px(theme::GAP_TIGHT));
    for (slot, kind) in SHELVES.into_iter().enumerate() {
        let variant = if kind == channel.kind {
            controls::Variant::Selected
        } else {
            controls::Variant::Pill
        };
        let on_action = on_action.clone();
        shelves = shelves.child(
            controls::pill(("channel-shelf", slot), shelf_name(kind), variant).on_click(
                cx.listener(move |view, _event, window, cx| {
                    on_action(view, Action::ShowShelf(kind), window, cx)
                }),
            ),
        );
    }

    let list = browse::scroller("channel-videos", scroll).child(shelves);
    let list = if videos.is_empty() {
        let name = &channel.display_name;
        let nothing = match channel.kind {
            VideoKind::Archive => format!(
                "{name} has no past broadcasts. Twitch keeps them only for channels that ask it to."
            ),
            VideoKind::Highlight => format!("{name} has made no highlights."),
            VideoKind::Upload | VideoKind::Other => format!("{name} has uploaded no videos."),
        };
        list.child(browse::browse_placeholder(discovery, nothing.into()))
    } else {
        let card_width = browse::card_width(
            width,
            browse::CARD_MIN,
            browse::CARD_MAX,
            theme::GAP_SECTION,
        );
        let mut row = browse::wrap_row(theme::GAP_SECTION);
        for (index, video) in videos.items.iter().enumerate() {
            let item = Card {
                index,
                video,
                watched: history.get(&video.id),
                byline: byline(video, now),
                forgettable: false,
            };
            row = row.child(card(
                item,
                card_width,
                cache,
                can_add,
                now,
                on_action.clone(),
                cx,
            ));
        }
        list.child(row).children(browse::load_more(
            videos.next.is_some(),
            discovery.loading,
            on_action.clone(),
            cx,
        ))
    };
    let body = browse::scrollable(list, scroll).into_any_element();

    let login = channel.login.clone();
    let other = controls::pill(
        "channel-now",
        if live { "watch live" } else { "open chat" },
        controls::Variant::Pill,
    )
    .on_click(cx.listener({
        let on_action = on_action.clone();
        move |view, _event, window, cx| on_action(view, Action::Watch(login.clone()), window, cx)
    }));

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(browse::context_bar(
            "leave-channel",
            "← back",
            SharedString::from(channel.display_name.clone()),
            Action::CloseChannel,
            Some(other.into_any_element()),
            on_action,
            cx,
        ))
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(created_at: &str, length_secs: u64, thumbnail: &str) -> Video {
        Video {
            id: "1".into(),
            stream_id: None,
            user_id: "2".into(),
            user_login: "someone".into(),
            user_name: "Someone".into(),
            title: String::new(),
            created_at: created_at.into(),
            length_secs,
            thumbnail_url: thumbnail.into(),
            view_count: 0,
            kind: twitch_api::VideoKind::Archive,
            muted_segments: Vec::new(),
        }
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn lengths_read_like_uptimes() {
        assert_eq!(length(21497), "5h 58m");
        assert_eq!(length(3600), "1h 0m");
        assert_eq!(length(3480), "58m");
        assert_eq!(length(45), "45s");
        assert_eq!(length(0), "0s");
    }

    /// Both signals, and the case that has neither: Twitch's placeholder
    /// picture says a recording is still being made, and so does a listed
    /// end within a few minutes of now.
    #[test]
    fn a_broadcast_still_going_is_told_from_one_that_finished() {
        let now = at("2026-09-08T20:00:00Z");
        let placeholder = video(
            "2026-09-08T14:59:16Z",
            100,
            "https://vod-secure.twitch.tv/_404/404_processing_%{width}x%{height}.png",
        );
        assert!(in_progress(&placeholder, now));

        // Listed as ending five minutes ago: still growing when listed.
        let fresh = video(
            "2026-09-08T13:00:00Z",
            6 * 3600 + 55 * 60,
            "https://cdn/x.jpg",
        );
        assert!(in_progress(&fresh, now));

        // Listed as ending an hour ago: finished.
        let done = video("2026-09-08T13:00:00Z", 6 * 3600, "https://cdn/x.jpg");
        assert!(!in_progress(&done, now));

        assert!(!in_progress(
            &video("not a date", 100, "https://cdn/x.jpg"),
            now
        ));
    }

    /// A highlight cut a few minutes ago ends close to now on paper, which is
    /// the second signal a broadcast is still going — and it is not.
    #[test]
    fn only_a_broadcast_can_still_be_going() {
        let now = at("2026-09-08T20:00:00Z");
        for kind in [VideoKind::Highlight, VideoKind::Upload] {
            let fresh = Video {
                kind,
                ..video("2026-09-08T19:50:00Z", 300, "https://cdn/x.jpg")
            };
            assert!(!in_progress(&fresh, now), "{kind:?} read as still going");
        }
    }

    #[test]
    fn a_broadcast_still_going_is_as_long_as_the_time_since_it_started() {
        let now = at("2026-09-08T20:00:00Z");
        let going = video("2026-09-08T14:00:00Z", 100, "https://cdn/x.jpg");
        assert_eq!(elapsed_secs(&going, now), Some(6.0 * 3600.0));
        assert_eq!(elapsed_secs(&video("nope", 1, ""), now), None);
    }

    /// Counted on the calendar rather than in 24-hour blocks, and in words
    /// only while the count still means something.
    #[test]
    fn when_counts_days_the_way_people_do() {
        // Midday, so a local-time calendar cannot land on a different day
        // than UTC's anywhere a test runs.
        let now = at("2026-09-08T12:00:00Z");
        assert_eq!(when("2026-09-08T10:00:00Z", now).as_deref(), Some("today"));
        assert_eq!(
            when("2026-09-07T12:00:00Z", now).as_deref(),
            Some("yesterday")
        );
        assert_eq!(
            when("2026-09-05T12:00:00Z", now).as_deref(),
            Some("3 days ago")
        );
        assert_eq!(
            when("2026-08-30T12:00:00Z", now).as_deref(),
            Some("last week")
        );
        assert_eq!(
            when("2026-08-20T12:00:00Z", now).as_deref(),
            Some("2 weeks ago")
        );
        assert_eq!(when("2026-07-01T12:00:00Z", now).as_deref(), Some("1 Jul"));
        assert_eq!(
            when("2025-12-25T12:00:00Z", now).as_deref(),
            Some("25 Dec 2025")
        );
        assert_eq!(
            when("2026-09-09T12:00:00Z", now).as_deref(),
            Some("today"),
            "the future is now"
        );
        assert_eq!(when("garbage", now), None);
    }

    #[test]
    fn a_recording_is_described_by_when_and_how_long() {
        let now = at("2026-09-08T12:00:00Z");
        let done = video("2026-09-07T12:00:00Z", 21497, "https://cdn/x.jpg");
        assert_eq!(describe(&done, now), "yesterday · 5h 58m");

        // Listed an hour ago at a minute long, but still wearing Twitch's
        // placeholder picture: still going, and two hours in by now.
        let going = video(
            "2026-09-08T10:00:00Z",
            60,
            "https://vod-secure.twitch.tv/_404/404_processing_%{width}x%{height}.png",
        );
        assert_eq!(describe(&going, now), "streaming now · 2h 0m so far");
    }
}
