//! The seek bar on a past broadcast, and the arithmetic behind it.
//!
//! Drawn by hand rather than with the widget library's slider, for three
//! reasons that are each enough on their own. That slider reports no drag end,
//! and a seek has to happen on release — mpv takes half a second to a second
//! and a half per seek, so seeking on every pointer move would queue dozens of
//! them. Its drag goes through gpui's `on_drag`, which makes every hover probe
//! in the window read false for as long as it lasts, the trap the volume
//! slider already documents. And its value is a thing you set; a playhead is a
//! thing that moves on its own and is only sometimes under the pointer.
//!
//! The pure parts — where a pointer lands on a bar, how long a recording is
//! right now, what a second reads as — are here and tested. The element takes
//! callbacks, so it knows nothing about the player that owns it, the way
//! `controls` knows nothing about pages.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    canvas, div, prelude::*, px, Bounds, Context, DefiniteLength, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, SharedString, Window,
};

use crate::theme;

/// Room for `h:mm:ss` at meta size, so the track does not shift by a few
/// pixels as the hours column arrives.
const TIME_WIDTH: f32 = 54.0;

/// How long a recording is, and whether it is still getting longer.
#[derive(Debug, Clone, Copy)]
pub struct Timeline {
    /// Seconds, as Twitch reported it when the video was listed.
    pub length: f64,
    /// When `length` was true.
    pub fetched_at: Instant,
    /// Still being recorded: the length grows by a second a second.
    pub growing: bool,
}

impl Timeline {
    /// The bar's right-hand end, given where the playhead is.
    ///
    /// Twitch's number, not mpv's. On a broadcast still being recorded mpv's
    /// `duration` is only how far it has read, and it grows in jumps as the
    /// cache fills; Twitch said how long the recording was when it was listed,
    /// and it has grown by exactly the time since. Never less than the
    /// position, so a recording that turns out longer than listed extends the
    /// bar rather than pushing the playhead off the end of it — and never
    /// zero, so nothing divides by it.
    pub fn extent(&self, position: f64) -> f64 {
        let grown = if self.growing {
            self.fetched_at.elapsed().as_secs_f64()
        } else {
            0.0
        };
        (self.length + grown).max(position).max(1.0)
    }
}

/// `h:mm:ss`, or `m:ss` under an hour: the way every player writes a time,
/// and the way the length is written on the card that opened the video.
pub fn timecode(secs: f64) -> String {
    let total = secs.max(0.0).floor() as u64;
    let (hours, minutes, seconds) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Where along a track laid out at `bounds` a pointer at `x` is, as a
/// fraction of it, clamped to the track: a drag that leaves the window to the
/// left means the start, not a negative time.
pub fn fraction_at(bounds: &Bounds<Pixels>, x: Pixels) -> f32 {
    let width = f32::from(bounds.size.width);
    if width <= 0.0 {
        return 0.0;
    }
    ((f32::from(x) - f32::from(bounds.origin.x)) / width).clamp(0.0, 1.0)
}

/// What the bar shows this frame.
pub struct State {
    /// The playhead, as a fraction of the extent.
    pub played: f32,
    /// Where a scrub in progress has got to, which the thumb follows instead
    /// of the playhead until the pointer lets go.
    pub scrub: Option<f32>,
    /// The time under the thumb, written out.
    pub position: SharedString,
    /// The length, written out.
    pub extent: SharedString,
}

/// The bar, wired to whatever owns it.
///
/// `on_press` starts a scrub at a fraction of the track. `on_release` ends
/// whichever scrub is running, wherever the pointer is by then — the owner
/// knows where it got to. `on_bounds` reports where the track was laid out
/// this frame, which the owner needs to follow the pointer *during* a scrub:
/// the pointer leaves the track on the first frame of any drag worth making,
/// so the moves that matter never arrive here.
pub fn element<V: 'static>(
    state: State,
    on_press: impl Fn(&mut V, f32, &mut Window, &mut Context<V>) + 'static,
    on_release: impl Fn(&mut V, &mut Window, &mut Context<V>) + Clone + 'static,
    on_bounds: impl Fn(&mut V, Bounds<Pixels>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let shown = state.scrub.unwrap_or(state.played).clamp(0.0, 1.0);

    // The track's bounds, as laid out this frame. Shared between the probe
    // that measures them and the press that needs them to turn a pointer into
    // a fraction: a press arrives after this frame's layout has run, and the
    // cell is made afresh with the element, so it can never hold last frame's.
    let laid_out: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let owner = cx.entity().downgrade();
    let measure = canvas(
        {
            let laid_out = laid_out.clone();
            move |bounds, _window, cx| {
                laid_out.set(Some(bounds));
                owner.update(cx, |view, _| on_bounds(view, bounds)).ok();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();

    let rail_top = (theme::SEEK_HIT - theme::SEEK_RAIL) / 2.0;
    let rail = div()
        .absolute()
        .left_0()
        .right_0()
        .top(px(rail_top))
        .h(px(theme::SEEK_RAIL))
        .rounded(px(theme::SEEK_RAIL / 2.0))
        .bg(theme::seek_rail())
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(DefiniteLength::Fraction(shown))
                .rounded(px(theme::SEEK_RAIL / 2.0))
                .bg(theme::accent()),
        );

    // Centred on the fraction: positioned by its left edge, then pulled back
    // by half its own width.
    let thumb = div()
        .absolute()
        .top(px((theme::SEEK_HIT - theme::SEEK_THUMB) / 2.0))
        .left(DefiniteLength::Fraction(shown))
        .ml(px(-theme::SEEK_THUMB / 2.0))
        .w(px(theme::SEEK_THUMB))
        .h(px(theme::SEEK_THUMB))
        .rounded_full()
        .bg(theme::text());

    let release_inside = on_release.clone();
    let track = div()
        .id("seek-track")
        .flex_1()
        .h(px(theme::SEEK_HIT))
        .relative()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let laid_out = laid_out.clone();
                move |view, event: &MouseDownEvent, window, cx| {
                    if let Some(bounds) = laid_out.get() {
                        on_press(view, fraction_at(&bounds, event.position.x), window, cx);
                    }
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |view, _: &MouseUpEvent, window, cx| release_inside(view, window, cx)),
        )
        // A drag rarely ends over an eighteen-pixel strip. This is the one
        // listener that hears a release anywhere else in the window.
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(move |view, _: &MouseUpEvent, window, cx| on_release(view, window, cx)),
        )
        .child(measure)
        .child(rail)
        .child(thumb);

    let time = |text: SharedString| {
        div()
            .flex_none()
            .min_w(px(TIME_WIDTH))
            .text_size(px(theme::TEXT_META))
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text())
            .child(text)
    };

    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .child(time(state.position))
        .child(track)
        .child(time(state.extent).text_right())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};
    use std::time::Duration;

    #[test]
    fn times_read_the_way_a_player_writes_them() {
        assert_eq!(timecode(0.0), "0:00");
        assert_eq!(timecode(59.9), "0:59", "floored, never rounded up");
        assert_eq!(timecode(60.0), "1:00");
        assert_eq!(timecode(3599.0), "59:59");
        assert_eq!(timecode(3600.0), "1:00:00");
        assert_eq!(timecode(21497.8), "5:58:17");
        assert_eq!(timecode(-5.0), "0:00", "nothing before the start");
    }

    #[test]
    fn a_pointer_lands_on_the_track_as_a_fraction_and_never_off_it() {
        let track = Bounds {
            origin: point(px(100.), px(0.)),
            size: size(px(400.), px(18.)),
        };
        assert_eq!(fraction_at(&track, px(100.)), 0.0);
        assert_eq!(fraction_at(&track, px(300.)), 0.5);
        assert_eq!(fraction_at(&track, px(500.)), 1.0);
        assert_eq!(fraction_at(&track, px(-40.)), 0.0, "off the left end");
        assert_eq!(fraction_at(&track, px(900.)), 1.0, "off the right end");

        let unlaid = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(0.), px(0.)),
        };
        assert_eq!(fraction_at(&unlaid, px(10.)), 0.0, "no width, no division");
    }

    /// The extent is Twitch's length, grown by the clock while a broadcast is
    /// still going, and never shorter than where the playhead already is.
    #[test]
    fn a_finished_recording_has_a_fixed_extent_and_a_live_one_grows() {
        let finished = Timeline {
            length: 21497.0,
            fetched_at: Instant::now() - Duration::from_secs(120),
            growing: false,
        };
        assert_eq!(finished.extent(100.0), 21497.0);
        assert_eq!(
            finished.extent(21600.0),
            21600.0,
            "a playhead past the listed length extends the bar"
        );

        let live = Timeline {
            length: 1000.0,
            fetched_at: Instant::now() - Duration::from_secs(120),
            growing: true,
        };
        let extent = live.extent(0.0);
        assert!(
            (1119.0..1125.0).contains(&extent),
            "two minutes on from a listing of 1000 s should be ~1120, got {extent}"
        );

        let empty = Timeline {
            length: 0.0,
            fetched_at: Instant::now(),
            growing: false,
        };
        assert_eq!(
            empty.extent(0.0),
            1.0,
            "never zero, so nothing divides by it"
        );
    }
}
