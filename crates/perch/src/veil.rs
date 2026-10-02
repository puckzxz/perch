//! What an overlay on the watch page covers, so the panes under it stop
//! counting the pointer as theirs.
//!
//! Every hover a pane has is measured, not reported (see `HANDOFF.md`,
//! "Hover is measured"): a `canvas` probe asks where the pointer is and
//! whether it is inside the pane's bounds. A probe inserts no hitbox, which
//! is what lets it see through the player's own control bar, and also what
//! lets it see through anything else drawn over the pane. With the guide up
//! over the lower half of the watch page, the panes beneath it went on
//! measuring the pointer as over them while it worked in the guide: their
//! bars came up behind it, their headers came up above it, a chat under it
//! held its rows, and crossing the guide from one pane's half to another's
//! made each pane the active one in turn — so `Watch` replaced whichever
//! pane the pointer had last crossed, not the one the guide was opened from.
//! (The part of a pane above the guide is still the pane's, and crossing it
//! still makes it active, which is why the guide keeps its own opener as
//! well: `guide::Guide::opener`.)
//!
//! So an overlay says where it is, here, and every pane probe asks
//! [`pointer_over`] instead of testing its bounds alone: the pointer is the
//! pane's only while it is in the window, inside the pane, and not on the
//! veil. One veil, for one window: a pane's player in a window of its own
//! measures against that window, which nothing veils.
//!
//! The overlay measures itself, as everything here does: a probe in it
//! writes its laid-out bounds ([`cover`]), and the root lifts the veil
//! ([`lift`]) on every frame the overlay is not drawn, before any probe
//! runs. A probe that ran earlier in the frame than the overlay's reads the
//! bounds from the frame before, which is the opening frame only — the
//! overlay's probe then asks for one more, and every probe reads it there.

use gpui::{AnyWindowHandle, App, Bounds, Global, Pixels, Point, Window};

/// Where the overlay is, and in which window: `None` while nothing covers
/// any of the watch page.
struct Veil {
    over: Option<(AnyWindowHandle, Bounds<Pixels>)>,
}

impl Global for Veil {}

/// Note that the overlay is laid out at `bounds` in `window` this frame.
/// Returns whether that moved it, the only time the probes need another
/// frame to read it in.
pub fn cover(bounds: Bounds<Pixels>, window: &Window, cx: &mut App) -> bool {
    set(Some((window.window_handle(), bounds)), cx)
}

/// Nothing covers the watch page now: from the root, on every frame the
/// overlay is not drawn. Free when there was nothing to lift.
pub fn lift(cx: &mut App) -> bool {
    set(None, cx)
}

/// Read before it is written, and written only on a change: every write of
/// a global queues an effect for its observers, and this is asked on every
/// frame.
fn set(over: Option<(AnyWindowHandle, Bounds<Pixels>)>, cx: &mut App) -> bool {
    let now = cx.try_global::<Veil>().and_then(|veil| veil.over);
    if now == over {
        return false;
    }
    cx.set_global(Veil { over });
    true
}

/// Whether the pointer is over `bounds` in `window`, as a pane's probe asks
/// it: the window has the pointer, the pointer is inside `bounds`, and it is
/// not on the overlay ([`reaches`]).
pub fn pointer_over(bounds: Bounds<Pixels>, window: &Window, cx: &App) -> bool {
    let veil = cx.try_global::<Veil>().and_then(|veil| {
        veil.over
            .filter(|(drawn_in, _)| *drawn_in == window.window_handle())
            .map(|(_, covered)| covered)
    });
    window.is_window_hovered() && reaches(window.mouse_position(), bounds, veil)
}

/// Whether `pointer` is the element's at `bounds` with `veil` drawn over the
/// page: inside the one, and not inside the other.
fn reaches(pointer: Point<Pixels>, bounds: Bounds<Pixels>, veil: Option<Bounds<Pixels>>) -> bool {
    bounds.contains(&pointer) && !veil.is_some_and(|veil| veil.contains(&pointer))
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size};

    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    /// A pane covering the page, and the guide over its lower half.
    const PANE: (f32, f32, f32, f32) = (0.0, 0.0, 1000.0, 800.0);
    const GUIDE: (f32, f32, f32, f32) = (10.0, 400.0, 980.0, 390.0);

    fn pane() -> Bounds<Pixels> {
        rect(PANE.0, PANE.1, PANE.2, PANE.3)
    }

    fn guide() -> Bounds<Pixels> {
        rect(GUIDE.0, GUIDE.1, GUIDE.2, GUIDE.3)
    }

    /// With nothing over the page, the pane has the pointer wherever it is
    /// inside it, and nowhere else.
    #[test]
    fn with_no_veil_the_pane_has_its_own_bounds() {
        assert!(reaches(point(px(500.0), px(600.0)), pane(), None));
        assert!(!reaches(point(px(1200.0), px(600.0)), pane(), None));
    }

    /// On the guide the pointer is nobody's under it; above the guide it is
    /// the pane's again, and so is the strip beside the guide's inset.
    #[test]
    fn the_veil_takes_the_pointer_from_what_is_under_it() {
        let veil = Some(guide());
        assert!(!reaches(point(px(500.0), px(600.0)), pane(), veil));
        assert!(reaches(point(px(500.0), px(200.0)), pane(), veil));
        assert!(reaches(point(px(4.0), px(600.0)), pane(), veil));
    }

    /// A veil away from the pane changes nothing about it.
    #[test]
    fn a_veil_elsewhere_leaves_the_pane_alone() {
        let veil = Some(rect(2000.0, 0.0, 100.0, 100.0));
        assert!(reaches(point(px(500.0), px(600.0)), pane(), veil));
    }
}
