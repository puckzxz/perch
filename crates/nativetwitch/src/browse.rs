//! The browse page: everyone you follow who is live.
//!
//! A page rather than a sidebar. Picking what to watch and watching it are
//! different activities, and giving the picker the whole window means
//! thumbnails big enough to actually choose by.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use emotes::ImageCache;
use gpui::{div, img, prelude::*, px, rgb, AnyElement, Context, SharedString};
use twitch_api::LiveStream;

use crate::motion;
use crate::theme;

/// Card width. Wide enough for a legible 16:9 thumbnail, narrow enough that a
/// 1600px window fits four across.
const CARD_WIDTH: f32 = 300.0;
const THUMBNAIL_WIDTH: u32 = 440;
const THUMBNAIL_HEIGHT: u32 = 248;

/// How far sign-in has got.
#[derive(Clone)]
pub enum SignIn {
    Connecting,
    NeedsClientId,
    AwaitingCode {
        user_code: SharedString,
        verification_uri: SharedString,
    },
    SignedIn(SharedString),
    Error(SharedString),
}

impl SignIn {
    pub fn summary(&self) -> SharedString {
        match self {
            SignIn::SignedIn(login) => format!("Signed in as {login}").into(),
            SignIn::Connecting => "Connecting…".into(),
            SignIn::NeedsClientId => "Not signed in".into(),
            SignIn::AwaitingCode { user_code, .. } => {
                format!("Enter {user_code} at twitch.tv/activate").into()
            }
            SignIn::Error(reason) => reason.clone(),
        }
    }
}

/// "3h 12m" since the stream started, or `None` if the timestamp is unusable.
///
/// Twitch sends RFC 3339; anything else is a shape change on their side and
/// should degrade to showing nothing rather than a wrong number.
pub fn uptime(started_at: &str) -> Option<String> {
    let started = DateTime::parse_from_rfc3339(started_at).ok()?;
    let elapsed = Utc::now().signed_duration_since(started.with_timezone(&Utc));
    if elapsed.num_seconds() < 0 {
        return None;
    }
    let hours = elapsed.num_hours();
    let minutes = elapsed.num_minutes() % 60;
    Some(if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    })
}

pub fn format_viewers(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// One live channel.
///
/// Clicking the card watches it alone; the small "+" adds it beside whatever is
/// already playing. Two separate affordances because replacing what you are
/// watching and adding to it are different intentions, and guessing between
/// them from a single click gets it wrong half the time.
#[allow(clippy::too_many_arguments)]
fn card<V: 'static>(
    index: usize,
    stream: &LiveStream,
    cache: &ImageCache,
    can_add: bool,
    on_click: impl Fn(&mut V, String, &mut gpui::Window, &mut Context<V>) + 'static,
    on_add: impl Fn(&mut V, String, &mut gpui::Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let login = stream.user_login.clone();
    let add_login = stream.user_login.clone();
    let thumbnail = cache.get_or_request(&twitch_api::thumbnail(
        &stream.thumbnail_url,
        THUMBNAIL_WIDTH,
        THUMBNAIL_HEIGHT,
    ));

    let preview = match thumbnail {
        Some(path) => img(path)
            .w_full()
            .h(px(CARD_WIDTH * 9.0 / 16.0))
            .into_any_element(),
        // Sized placeholder, so the grid does not reflow as images arrive.
        None => div()
            .w_full()
            .h(px(CARD_WIDTH * 9.0 / 16.0))
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    let meta = [
        Some(format_viewers(stream.viewer_count) + " watching"),
        uptime(&stream.started_at),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");

    div()
        .id(("stream-card", index))
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .rounded_md()
        .overflow_hidden()
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_raised()))
        .active(|style| style.bg(theme::pressed()))
        .on_click(
            cx.listener(move |view, _event, window, cx| on_click(view, login.clone(), window, cx)),
        )
        .child(
            div()
                .relative()
                .group("card")
                .child(preview)
                .child(
                    // Live badge, bottom-left of the thumbnail where broadcast
                    // UIs have put it for decades.
                    div()
                        .absolute()
                        .bottom(px(theme::GAP_TIGHT))
                        .left(px(theme::GAP_TIGHT))
                        .px(px(theme::CONTROL_PAD_X))
                        .py(px(theme::CONTROL_PAD_Y))
                        .rounded_sm()
                        .bg(theme::live())
                        .text_size(px(theme::TEXT_MICRO))
                        .font_weight(theme::weight_shout())
                        .text_color(rgb(0xffffff))
                        .child("LIVE"),
                )
                .when(can_add, |thumb| {
                    thumb.child(
                        div()
                            .id(("add-stream", index))
                            .absolute()
                            .top(px(theme::GAP_TIGHT))
                            .right(px(theme::GAP_TIGHT))
                            .px(px(theme::CONTROL_PAD_X))
                            .py(px(theme::CONTROL_PAD_Y))
                            .rounded_sm()
                            .bg(theme::surface_raised())
                            .text_size(px(theme::TEXT_LABEL))
                            .font_weight(theme::weight_label())
                            .text_color(theme::text())
                            .cursor_pointer()
                            .opacity(0.0)
                            .group_hover("card", |style| style.opacity(1.0))
                            .hover(|style| style.bg(theme::accent_dim()))
                            .active(|style| style.bg(theme::pressed()))
                            .child("+ add")
                            .on_click(cx.listener(move |view, _event, window, cx| {
                                // Without this the card underneath also fires
                                // and replaces every open pane.
                                cx.stop_propagation();
                                on_add(view, add_login.clone(), window, cx)
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
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_muted())
                        .truncate()
                        .child(SharedString::from(stream.title.clone())),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child(SharedString::from(if stream.game_name.is_empty() {
                            meta.clone()
                        } else {
                            format!("{} · {meta}", stream.game_name)
                        })),
                ),
        )
}

/// A message filling the page when there is nothing to show.
fn empty_state(sign_in: &SignIn) -> AnyElement {
    let (title, detail): (SharedString, SharedString) = match sign_in {
        SignIn::NeedsClientId => (
            "Not signed in".into(),
            "Open settings and paste a Twitch Client ID to see who you follow.".into(),
        ),
        SignIn::AwaitingCode {
            user_code,
            verification_uri,
        } => (
            user_code.clone(),
            format!("Enter that code at {verification_uri}").into(),
        ),
        SignIn::Error(reason) => ("Sign-in problem".into(), reason.clone()),
        SignIn::Connecting => ("Connecting…".into(), "Asking Twitch who is live.".into()),
        SignIn::SignedIn(_) => (
            "Nobody is live".into(),
            "None of the channels you follow are streaming right now.".into(),
        ),
    };

    let body = div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(theme::GAP))
        .child(
            div()
                .text_size(px(theme::TEXT_TITLE))
                .font_weight(theme::weight_title())
                .text_color(theme::text())
                .child(title),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_size(px(theme::TEXT_BODY))
                .line_height(px(theme::LINE_BODY))
                .text_center()
                .text_color(match sign_in {
                    SignIn::Error(_) => theme::danger(),
                    _ => theme::text_dim(),
                })
                .child(detail),
        );

    // Only the state that is actually still going breathes. "Nobody is live"
    // and "Not signed in" are answers, not progress, and a pulsing answer both
    // misleads and repaints forever.
    match sign_in {
        SignIn::Connecting => motion::waiting("connecting", body).into_any_element(),
        _ => body.into_any_element(),
    }
}

/// The whole page.
#[allow(clippy::too_many_arguments)]
pub fn page<V: 'static>(
    follows: &[LiveStream],
    sign_in: &SignIn,
    cache: &Arc<ImageCache>,
    can_add: bool,
    on_open: impl Fn(&mut V, String, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    on_add: impl Fn(&mut V, String, &mut gpui::Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let mut grid = div()
        .id("browse-grid")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(theme::GAP_SECTION))
        .p(px(theme::PAGE_PAD))
        .content_start();

    for (index, stream) in follows.iter().enumerate() {
        grid = grid.child(card(
            index,
            stream,
            cache,
            can_add,
            on_open.clone(),
            on_add.clone(),
            cx,
        ));
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme::bg())
        .child(if follows.is_empty() {
            empty_state(sign_in)
        } else {
            grid.into_any_element()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_viewer_counts_compactly() {
        assert_eq!(format_viewers(0), "0");
        assert_eq!(format_viewers(999), "999");
        assert_eq!(format_viewers(1500), "1.5k");
        assert_eq!(format_viewers(2_400_000), "2.4M");
    }

    #[test]
    fn uptime_reports_hours_and_minutes() {
        let started = Utc::now() - chrono::Duration::minutes(195);
        let text = uptime(&started.to_rfc3339()).unwrap();
        assert_eq!(text, "3h 15m");
    }

    #[test]
    fn uptime_omits_hours_under_one() {
        let started = Utc::now() - chrono::Duration::minutes(7);
        assert_eq!(uptime(&started.to_rfc3339()).unwrap(), "7m");
    }

    /// A shape change on Twitch's side should show nothing, never a wrong
    /// number that looks authoritative.
    #[test]
    fn unparseable_timestamps_yield_nothing() {
        assert!(uptime("").is_none());
        assert!(uptime("last tuesday").is_none());
    }

    #[test]
    fn future_timestamps_yield_nothing() {
        let ahead = Utc::now() + chrono::Duration::hours(1);
        assert!(uptime(&ahead.to_rfc3339()).is_none());
    }
}
