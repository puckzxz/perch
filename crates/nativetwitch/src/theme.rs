//! Colour and spacing tokens.
//!
//! One place for every colour in the app. Scattered hex literals are how a UI
//! ends up looking incidental — a slightly different grey per file, borders that
//! do not agree, and no way to change the mood without a search-and-replace.
//!
//! The direction is deliberately quiet: this is a window you leave open for
//! hours, so the chrome should recede and the video should be the only bright
//! thing on screen.

use gpui::{rgb, rgba, Hsla};

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

// ── Metrics ──────────────────────────────────────────────────────────

/// Chat pane default, and the bounds a drag is clamped to.
pub const CHAT_WIDTH_MIN: f32 = 260.0;
pub const CHAT_WIDTH_MAX: f32 = 640.0;
/// Chat height bounds when stacked below the video on a tall window.
pub const CHAT_HEIGHT_MIN: f32 = 160.0;
pub const CHAT_HEIGHT_MAX: f32 = 900.0;

/// Below this window aspect ratio the window is treated as portrait and chat
/// moves under the video instead of beside it.
pub const PORTRAIT_ASPECT: f32 = 1.1;
