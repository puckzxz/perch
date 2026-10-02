//! A live rail row's preview: the card that comes up beside a row the
//! pointer rests on.
//!
//! A rail row is a name, a game and a number, which is enough to find
//! someone and not enough to choose between strangers: what they are doing
//! right now is on the browse page's cards and in the guide, a page or a
//! panel away. So a live row the pointer rests on for a moment
//! ([`PREVIEW_DELAY`]) brings up that card beside the rail — the picture,
//! the title, the game, the viewers and how long the stream has been on —
//! and it goes the moment the pointer leaves the row. A recommendation's
//! answer carries no picture (see `guide::as_stream`), so its card is the
//! words alone, with the reason it is offered after the game, as the
//! guide's cards say it. Offline rows, and pins nobody can place, show
//! nothing: their row already says all there is.
//!
//! The card sits to the right of the rail, over the page, never over the
//! row: the row's pin and `+` stay where the pointer can reach them. It is
//! drawn deferred and anchored (`sidebar::row`), which takes it out of the
//! rail's scroller, whose clip would otherwise cut it off at the rail's
//! edge, and it has no listeners, so it inserts no hitbox and takes nothing
//! from the page under it or from the rows. Nothing about it moves the
//! rail: the hold that keeps the rows still under the pointer
//! (`RootView::hold_live`) measures the rail's own bounds, which the card
//! is not part of.
//!
//! When it shows is [`Preview`]'s, pure and tested; the root keeps one
//! (`RootView::rail_preview`), the rows' probes tell it where the pointer
//! is, and a timer wakes the rail when a wait is over.

use std::time::{Duration, Instant};

use emotes::ImageCache;
use gpui::{div, img, prelude::*, px, AnyElement, SharedString};
use twitch_api::LiveStream;

use crate::browse::{self, format_viewers};
use crate::controls;
use crate::guide;
use crate::theme;

/// How long the pointer rests on a live row before its card comes up. Long
/// enough that running the pointer down the rail to a row further on does
/// not flash a card up at every row it crosses; short enough that resting
/// on one to see what it is does not feel like waiting.
pub const PREVIEW_DELAY: Duration = Duration::from_millis(400);

/// How soon after one card goes the next row's comes up at once, without
/// the wait. A card already up is someone reading the rail, and stepping to
/// the row below should bring that one's card straight up, the way a
/// toolbar's tooltips follow the pointer once the first has shown. Long
/// enough to cross a group's heading between two rows.
pub const PREVIEW_WARM: Duration = Duration::from_millis(300);

/// How wide the card is: the guide's widest card, so it reads as one of the
/// guide's. The picture on it is the one every card fetches, from one
/// address at one size (`browse::stream_preview`), so a channel seen on Home
/// or in the guide has nothing more to fetch here.
pub(crate) const PREVIEW_WIDTH: f32 = guide::CARD_MAX;

/// How many lines a title may take on the card. More than a browse card's
/// one: a card here is only ever up because somebody wants to read it, and
/// it has no tooltip to give the rest.
const TITLE_LINES: usize = 2;

/// The picture's height: 16:9 across the card.
const PICTURE_HEIGHT: f32 = PREVIEW_WIDTH * 9.0 / 16.0;

/// The tallest a card can be, worked out from how [`card`] lays one out:
/// its border, the picture, the padding, the name, a title on all its
/// lines and the game line, with a tight gap between each line of words.
/// Every line on the card has a set height for this to be exact. A card
/// without a picture is shorter, its extra line of viewers well short of
/// the picture it stands in for. What `sidebar::previews_fit` keeps room
/// for under the title bar.
pub(crate) const PREVIEW_TALLEST: f32 = 2.0
    + PICTURE_HEIGHT
    + 2.0 * theme::PANEL_PAD
    + theme::LINE_BODY
    + TITLE_LINES as f32 * theme::LINE_TIGHT
    + theme::LINE_TIGHT
    + 2.0 * theme::GAP_TIGHT;

/// When a rail row's card shows: the row under the pointer, by key, and
/// since when.
///
/// Fed by the rows' probes ([`point`](Self::point)), read by the rail as it
/// draws ([`shown`](Self::shown)). A press on the rail puts the card away
/// until the pointer leaves the row ([`dismiss`](Self::dismiss)), as a
/// press puts a tooltip away: the row has done what it was pressed for, and
/// the card would sit over the page it opened.
#[derive(Debug, Default)]
pub struct Preview {
    /// The row the pointer is on, and the moment its wait is counted from.
    pointed: Option<(String, Instant)>,
    /// Put away by a press, until the pointer leaves the row.
    dismissed: bool,
    /// When the last card that was up went away because the pointer left
    /// its row: the start of [`PREVIEW_WARM`].
    cooled: Option<Instant>,
}

/// What a report from a row's probe changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Nothing the rail draws.
    None,
    /// Draw the rail again now: a card came up or went.
    Now,
    /// Draw it now, and again once `Duration` has passed, when the wait
    /// for a newly pointed row is over.
    After(Duration),
}

impl Preview {
    /// The row `key` names says whether the pointer is on it at `now`.
    ///
    /// A row newly pointed at waits [`PREVIEW_DELAY`] — or nothing, within
    /// [`PREVIEW_WARM`] of another card going, or while another is up. A
    /// row the pointer has left takes its card, and the wait, with it. A
    /// row that says it is not pointed at when another row is changes
    /// nothing: the probes report in paint order, so the row the pointer
    /// has moved to may already have spoken.
    pub fn point(&mut self, key: &str, pointed: bool, now: Instant) -> Change {
        let current = self.pointed.as_ref().is_some_and(|(row, _)| row == key);
        match (pointed, current) {
            (true, true) | (false, false) => Change::None,
            (true, false) => {
                let warm = self.shown(now).is_some()
                    || self
                        .cooled
                        .is_some_and(|cooled| now.saturating_duration_since(cooled) < PREVIEW_WARM);
                let since = if warm {
                    now.checked_sub(PREVIEW_DELAY).unwrap_or(now)
                } else {
                    now
                };
                self.pointed = Some((key.to_string(), since));
                self.dismissed = false;
                self.cooled = None;
                if warm {
                    Change::Now
                } else {
                    Change::After(PREVIEW_DELAY)
                }
            }
            (false, true) => {
                if self.shown(now).is_some() {
                    self.cooled = Some(now);
                }
                self.pointed = None;
                self.dismissed = false;
                Change::Now
            }
        }
    }

    /// The row whose card is up at `now`, if one is: the row pointed at,
    /// once its wait is over, unless a press put it away.
    pub fn shown(&self, now: Instant) -> Option<&str> {
        if self.dismissed {
            return None;
        }
        self.pointed
            .as_ref()
            .filter(|(_, since)| now.saturating_duration_since(*since) >= PREVIEW_DELAY)
            .map(|(row, _)| row.as_str())
    }

    /// How much longer the row `key` names waits for its card at `now`:
    /// zero once it is up, and `None` if the pointer is not on that row any
    /// more or a press put its card away. What the root's timer asks when
    /// it wakes, so a wake a moment early sleeps out the rest.
    pub fn wait_left(&self, key: &str, now: Instant) -> Option<Duration> {
        if self.dismissed {
            return None;
        }
        self.pointed
            .as_ref()
            .filter(|(row, _)| row == key)
            .map(|(_, since)| PREVIEW_DELAY.saturating_sub(now.saturating_duration_since(*since)))
    }

    /// The row the pointer is on, card or not.
    pub fn pointed(&self) -> Option<&str> {
        self.pointed.as_ref().map(|(row, _)| row.as_str())
    }

    /// Put the card away until the pointer leaves the row: a press on the
    /// rail. Returns whether one was up or on its way.
    pub fn dismiss(&mut self) -> bool {
        let changed = self.pointed.is_some() && !self.dismissed;
        self.dismissed = true;
        self.cooled = None;
        changed
    }

    /// Forget the row: the rail is not drawn, or something is over it, so
    /// no probe will say the pointer left. Returns whether anything was
    /// held.
    pub fn clear(&mut self) -> bool {
        let changed = self.pointed.is_some();
        *self = Self::default();
        changed
    }
}

/// The card for `stream`, with `note` after its game: what a guide card
/// shows, laid out the way it lays it out, without the controls — nothing
/// on it takes a press. The picture is the cached preview the cards draw
/// (`browse::stream_preview`), with the viewers and the uptime on it in the
/// same badge; a stream with no picture to fetch, a recommendation's, has
/// the badge's words in a line under the game instead. Raised and ruled
/// like a menu, since it floats over the page as one does.
pub fn card(stream: &LiveStream, note: Option<&str>, cache: &ImageCache) -> AnyElement {
    let watching = [
        Some(format_viewers(stream.viewer_count)),
        browse::uptime(&stream.started_at),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let watching = SharedString::from(watching);

    let picture = (!stream.thumbnail_url.is_empty()).then(|| {
        let height = px(PICTURE_HEIGHT);
        let picture = match browse::stream_preview(cache, stream) {
            Some(path) => img(path).w_full().h(height).into_any_element(),
            // Sized, so the card does not grow under the pointer when the
            // picture lands.
            None => div()
                .w_full()
                .h(height)
                .bg(theme::surface())
                .into_any_element(),
        };
        div().relative().child(picture).child(
            controls::badge()
                .absolute()
                .bottom(px(theme::GAP_TIGHT))
                .left(px(theme::GAP_TIGHT))
                .child(controls::live_dot())
                .child(watching.clone()),
        )
    });
    let pictured = picture.is_some();
    let meta = browse::card_meta(&stream.game_name, note);

    let line = |text: SharedString| {
        div()
            .w_full()
            .text_ellipsis()
            .line_clamp(1)
            .text_size(px(theme::TEXT_META))
            .line_height(px(theme::LINE_TIGHT))
            .child(text)
    };

    div()
        .w(px(PREVIEW_WIDTH))
        .flex()
        .flex_col()
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface_raised())
        .border_1()
        .border_color(theme::border())
        .shadow_lg()
        .children(picture)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_TIGHT))
                .p(px(theme::PANEL_PAD))
                .child(
                    div()
                        .w_full()
                        .text_ellipsis()
                        .line_clamp(1)
                        .text_size(px(theme::TEXT_BODY))
                        // Set, as every line here is: `PREVIEW_TALLEST`.
                        .line_height(px(theme::LINE_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .when(!stream.title.is_empty(), |text| {
                    text.child(
                        div()
                            .w_full()
                            .text_ellipsis()
                            .line_clamp(TITLE_LINES)
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(stream.title.clone())),
                    )
                })
                .when(!meta.is_empty(), |text| {
                    text.child(line(meta.into()).text_color(theme::text_dim()))
                })
                .when(!pictured, |text| {
                    text.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::GAP_TIGHT))
                            .child(controls::live_dot())
                            .child(line(watching).text_color(theme::text_muted())),
                    )
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    /// A row rested on shows its card once the wait is over, and not
    /// before; the report that starts the wait asks for a wake-up then.
    #[test]
    fn a_card_comes_up_after_the_wait() {
        let start = Instant::now();
        let mut preview = Preview::default();
        assert_eq!(
            preview.point("live-alice", true, start),
            Change::After(PREVIEW_DELAY)
        );
        assert_eq!(preview.shown(at(start, 399)), None);
        assert_eq!(preview.shown(at(start, 400)), Some("live-alice"));
        assert_eq!(preview.pointed(), Some("live-alice"));
        // The same row saying so again, frame after frame, starts nothing.
        assert_eq!(
            preview.point("live-alice", true, at(start, 200)),
            Change::None
        );
        assert_eq!(preview.shown(at(start, 400)), Some("live-alice"));
    }

    /// What a waking timer is told: the rest of the wait, nothing once the
    /// card is up, and no answer for a row the pointer has left or a card
    /// a press put away.
    #[test]
    fn a_timer_is_told_what_is_left_of_the_wait() {
        let start = Instant::now();
        let mut preview = Preview::default();
        assert_eq!(preview.wait_left("live-alice", start), None);
        preview.point("live-alice", true, start);
        assert_eq!(
            preview.wait_left("live-alice", at(start, 390)),
            Some(Duration::from_millis(10))
        );
        assert_eq!(
            preview.wait_left("live-alice", at(start, 500)),
            Some(Duration::ZERO)
        );
        assert_eq!(preview.wait_left("live-bob", at(start, 500)), None);
        preview.dismiss();
        assert_eq!(preview.wait_left("live-alice", at(start, 500)), None);
    }

    /// Leaving the row takes the card, or the wait for it, away at once.
    #[test]
    fn leaving_the_row_hides_it() {
        let start = Instant::now();
        let mut preview = Preview::default();
        preview.point("live-alice", true, start);
        assert_eq!(
            preview.point("live-alice", false, at(start, 100)),
            Change::Now
        );
        assert_eq!(preview.shown(at(start, 1000)), None);
        assert_eq!(preview.pointed(), None);
    }

    /// The row the pointer left reporting after the row it went to changes
    /// nothing: probes speak in paint order, and that order is not the
    /// pointer's.
    #[test]
    fn another_rows_leaving_does_not_hide_this_one() {
        let start = Instant::now();
        let mut preview = Preview::default();
        preview.point("live-bob", true, start);
        assert_eq!(
            preview.point("live-alice", false, at(start, 10)),
            Change::None
        );
        assert_eq!(preview.shown(at(start, 400)), Some("live-bob"));
    }

    /// Passing over rows on the way somewhere starts a fresh wait at each:
    /// nothing comes up for a row the pointer only crossed.
    #[test]
    fn crossing_rows_restarts_the_wait() {
        let start = Instant::now();
        let mut preview = Preview::default();
        preview.point("live-alice", true, start);
        preview.point("live-alice", false, at(start, 300));
        assert_eq!(
            preview.point("live-bob", true, at(start, 300)),
            Change::After(PREVIEW_DELAY)
        );
        assert_eq!(preview.shown(at(start, 600)), None);
        assert_eq!(preview.shown(at(start, 700)), Some("live-bob"));
    }

    /// Once a card is up, the next row's comes up at once, whichever of
    /// the two rows reports first; and only within the warm moment.
    #[test]
    fn a_card_up_hands_straight_on_to_the_next_row() {
        let start = Instant::now();
        let mut preview = Preview::default();
        preview.point("live-alice", true, start);

        // Moving down: the row left speaks first.
        let mut down = Preview::default();
        down.point("live-alice", true, start);
        down.point("live-alice", false, at(start, 500));
        assert_eq!(down.point("live-bob", true, at(start, 500)), Change::Now);
        assert_eq!(down.shown(at(start, 500)), Some("live-bob"));

        // Moving up: the row reached speaks first.
        assert_eq!(preview.point("live-bob", true, at(start, 500)), Change::Now);
        assert_eq!(
            preview.point("live-alice", false, at(start, 500)),
            Change::None
        );
        assert_eq!(preview.shown(at(start, 500)), Some("live-bob"));

        // Away from the rail for longer than the warm moment: cold again.
        down.point("live-bob", false, at(start, 600));
        assert_eq!(
            down.point("live-carol", true, at(start, 1000)),
            Change::After(PREVIEW_DELAY)
        );
        assert_eq!(down.shown(at(start, 1000)), None);
    }

    /// A press puts the card away until the pointer leaves the row, and a
    /// card put away does not warm the next row.
    #[test]
    fn a_press_puts_it_away_until_the_pointer_leaves() {
        let start = Instant::now();
        let mut preview = Preview::default();
        preview.point("live-alice", true, start);
        assert!(preview.dismiss());
        assert_eq!(preview.shown(at(start, 1000)), None);
        assert!(!preview.dismiss(), "a second press changed something");

        preview.point("live-alice", false, at(start, 1000));
        assert_eq!(
            preview.point("live-bob", true, at(start, 1000)),
            Change::After(PREVIEW_DELAY)
        );
        assert_eq!(preview.shown(at(start, 1400)), Some("live-bob"));
    }

    /// The rail going, or something over it, forgets the row and the warm
    /// moment both.
    #[test]
    fn clearing_forgets_everything() {
        let start = Instant::now();
        let mut preview = Preview::default();
        assert!(!preview.clear());
        preview.point("live-alice", true, start);
        assert!(preview.clear());
        assert_eq!(preview.shown(at(start, 1000)), None);
        assert_eq!(
            preview.point("live-bob", true, at(start, 1000)),
            Change::After(PREVIEW_DELAY)
        );
    }
}
