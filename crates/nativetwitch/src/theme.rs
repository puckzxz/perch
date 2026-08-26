//! Colour and spacing tokens.
//!
//! One place for every colour in the app. Scattered hex literals are how a UI
//! ends up looking incidental — a slightly different grey per file, borders that
//! do not agree, and no way to change the mood without a search-and-replace.
//!
//! The direction is deliberately quiet: this is a window you leave open for
//! hours, so the chrome should recede and the video should be the only bright
//! thing on screen.

use std::time::Duration;

use gpui::{ease_in_out, ease_out_quint, rgb, rgba, FontWeight, Hsla};

/// Behind the video, and nothing else. Pure black rather than near-black: any
/// lift here shows as a grey halo around letterboxed content, which is exactly
/// where the eye is least forgiving.
pub fn player_bg() -> Hsla {
    rgb(0x000000).into()
}

// ── Surfaces, darkest to lightest ────────────────────────────────────

/// The window's base layer.
pub fn bg() -> Hsla {
    rgb(0x0d0d10).into()
}

/// Panels sitting on the base: chat, browse cards, the sidebar.
pub fn surface() -> Hsla {
    rgb(0x131317).into()
}

/// Raised things: hovered rows, popovers, the settings sheet.
pub fn surface_raised() -> Hsla {
    rgb(0x1a1a21).into()
}

/// Hover wash over an existing surface, kept translucent so it works on top of
/// whatever is beneath rather than needing a variant per background.
pub fn hover() -> Hsla {
    rgba(0xffffff0a).into()
}

/// Held down. Stronger than `hover` rather than a different colour, so a press
/// reads as more of the same gesture instead of a separate state.
pub fn pressed() -> Hsla {
    rgba(0xffffff1a).into()
}

/// Every other chat row, for the same reason ledgers have ruled lines.
pub fn stripe() -> Hsla {
    rgba(0xffffff05).into()
}

/// Scrim behind a modal.
pub fn scrim() -> Hsla {
    rgba(0x00000099).into()
}

// ── Lines ────────────────────────────────────────────────────────────

/// Structural borders between panes.
pub fn border() -> Hsla {
    rgb(0x24242c).into()
}

/// Hairlines within a pane, e.g. between chat messages. Deliberately fainter
/// than `border`, since separating peers needs less weight than separating
/// regions.
pub fn divider() -> Hsla {
    rgba(0xffffff0d).into()
}

// ── Text ─────────────────────────────────────────────────────────────

pub fn text() -> Hsla {
    rgb(0xe8e6ed).into()
}

/// Labels, metadata, anything supporting.
pub fn text_muted() -> Hsla {
    rgb(0x9a97a5).into()
}

/// Placeholders and disabled states.
pub fn text_dim() -> Hsla {
    rgb(0x66636f).into()
}

// ── Accent and status ────────────────────────────────────────────────

/// Used sparingly: selection, focus, the one control that matters in a view.
pub fn accent() -> Hsla {
    rgb(0x9d7bff).into()
}

pub fn accent_dim() -> Hsla {
    rgba(0x9d7bff33).into()
}

/// The live dot. The only saturated red in the app, so it reads as status
/// rather than decoration.
pub fn live() -> Hsla {
    rgb(0xe5534b).into()
}

pub fn danger() -> Hsla {
    rgb(0xf08a80).into()
}

// ── Type ─────────────────────────────────────────────────────────────
//
// Five roles rather than five sizes. The app previously used one size,
// text_xs, for nineteen different jobs - button labels, chat notices, stream
// metadata, settings labels - so nothing had rank. Weight was the same story:
// BOLD was the only weight in the codebase, which means emphasis had no
// degrees, only on and off.

/// Page and panel titles.
pub const TEXT_TITLE: f32 = 15.0;
/// Chat messages and card names: the content you actually read.
pub const TEXT_BODY: f32 = 13.0;
/// Interactive control labels. Same size as meta but a heavier weight, so a
/// thing you can click never looks like a thing you can only read.
pub const TEXT_LABEL: f32 = 11.5;
/// Supporting information: viewers, uptime, status, help text.
pub const TEXT_META: f32 = 11.5;
/// Badges only.
pub const TEXT_MICRO: f32 = 9.5;

/// Leading for running text. Chat is dense and repetitive; default leading
/// makes consecutive lines hard to separate.
pub const LINE_BODY: f32 = 19.0;
/// Leading for single-line labels, where extra space just inflates the row.
pub const LINE_TIGHT: f32 = 15.0;

/// Titles and names. Semibold rather than bold, leaving bold for the one
/// element that genuinely has to shout.
pub fn weight_title() -> FontWeight {
    FontWeight::SEMIBOLD
}

/// Control labels: enough weight to read as interactive, not enough to compete
/// with a title.
pub fn weight_label() -> FontWeight {
    FontWeight::MEDIUM
}

/// Reserved for the live badge.
pub fn weight_shout() -> FontWeight {
    FontWeight::BOLD
}

// ── Spacing ──────────────────────────────────────────────────────────
//
// Named by role rather than by size. The point is not the numbers but that
// two things playing the same role get the same value: the app previously
// mixed px_2/px_3/px_4 for the same kind of padding in different files, which
// is what made it read as unconsidered rather than any single gap being wrong.

/// Outer margin of a page.
pub const PAGE_PAD: f32 = 20.0;
/// Inside a card, panel or sheet.
pub const PANEL_PAD: f32 = 12.0;
/// Inside a pill or button.
pub const CONTROL_PAD_X: f32 = 10.0;
pub const CONTROL_PAD_Y: f32 = 5.0;
/// Between a label and the thing it labels.
pub const GAP_TIGHT: f32 = 6.0;
/// Between peers in a row or column.
pub const GAP: f32 = 10.0;
/// Between distinct sections.
pub const GAP_SECTION: f32 = 18.0;
/// Seam between panes in the watch grid. Deliberately thin: it separates
/// pictures, and anything wider reads as a border around each one.
pub const PANE_GAP: f32 = 3.0;
/// Vertical rhythm inside a chat row.
pub const ROW_PAD_X: f32 = 12.0;
pub const ROW_PAD_Y: f32 = 5.0;

// ── Motion ────────────────────────────────────────────────────────────
//
// Named by what the movement is *for*, like spacing. Motion here has one job:
// to say that a thing changed, rather than that a different thing is now on
// screen. Anything long enough to wait for is too long.

/// Revealing or hiding something under the pointer. Short enough that the
/// control feels attached to the cursor rather than chasing it.
pub const MOTION_HOVER: Duration = Duration::from_millis(120);
/// Something arriving or leaving of its own accord: a menu, a toast, a page.
/// These get slightly longer because you did not ask for them at a precise
/// moment, so there is nothing for them to feel behind.
pub const MOTION_ENTER: Duration = Duration::from_millis(200);
/// First frames coming up from black. Deliberately the slowest thing in the
/// app: it is a picture resolving, not a control responding, and cutting
/// straight to video reads as a glitch.
pub const MOTION_VIDEO: Duration = Duration::from_millis(300);
/// One breath of a waiting indicator. Slow on purpose — a fast pulse reads as
/// alarm, and this only ever means "still working".
pub const PULSE_PERIOD: Duration = Duration::from_millis(1600);
/// How faint a waiting indicator gets at the bottom of its breath. Never zero:
/// something that vanishes entirely looks broken rather than busy.
pub const PULSE_FLOOR: f32 = 0.45;

/// For a two-way change — visible to hidden and back. Symmetric, because the
/// reveal and the hide are one event in opposite directions.
pub fn ease_fade() -> impl Fn(f32) -> f32 {
    ease_in_out
}

/// For a one-way arrival. Decelerating, so the thing leaves the mark
/// immediately and settles, rather than sliding to a stop.
pub fn ease_enter() -> impl Fn(f32) -> f32 {
    ease_out_quint()
}

// ── Metrics ──────────────────────────────────────────────────────────

/// What a hand-edited `chat_width` is clamped to. There is no height pair:
/// stacked chat takes whatever the video leaves rather than a stored size.
pub const CHAT_WIDTH_MIN: f32 = 260.0;
pub const CHAT_WIDTH_MAX: f32 = 640.0;

/// Below this window aspect ratio the window is treated as portrait and chat
/// moves under the video instead of beside it.
pub const PORTRAIT_ASPECT: f32 = 1.1;
