//! What an overlay on the watch page covers, so the panes under it stop
//! counting the pointer as theirs.
//!
//! Every hover a pane has is measured, not reported (see `HANDOFF.md`,
//! "measured, not reported"): a `canvas` probe asks where the pointer is and
//! whether it is inside the pane's bounds. A probe inserts no hitbox, which
//! is what lets it see through the player's own control bar, and also what
//! lets it see through anything else drawn over the pane. A person's card
//! (`chat::viewer_card`) is such a thing: in a short chat it rises out of
//! its list box over the pane's header and picture, or over the pane above,
//! and with the pointer on it the pane above went on measuring the pointer
//! as its own — it became the active one, and brought the player's bar up
//! behind the card.
//!
//! So an overlay says where it is, here, under its own [`Cover`], and every
//! pane probe asks [`pointer_over`] instead of testing its bounds alone: the
//! pointer is the pane's only while it is in the window, inside the pane,
//! and on no overlay. One veil per window: a pane's player in a window of
//! its own measures against that window, which nothing in the main one
//! veils.
//!
//! A card belongs to its pane, so two probes leave their own card out (the
//! `own` that [`pointer_over`] is given): the pane's hover, whose pane the
//! card is part of, and its chat's hold, which counts the pointer on its card
//! as on the chat (`ChatView::render`). The same pane's player does not:
//! its bar would come up behind the card.
//!
//! An overlay measures itself, as everything here does: a probe in it
//! writes its laid-out bounds ([`cover`]), and its cover is lifted
//! ([`lift`]) on every frame it is not drawn, before any probe runs — a
//! card's by its chat as it renders without one, as it is closed and as the
//! chat goes. A probe that ran earlier in the frame than the overlay's reads
//! the bounds from the frame before, which is the opening frame only — the
//! overlay's probe then asks for one more, and every probe reads it there.

use gpui::{AnyWindowHandle, App, Bounds, EntityId, Global, Pixels, Point, Window};

/// What covers part of the watch page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    /// A person's card, by the chat whose card it is.
    Card(EntityId),
}

/// One overlay, where it was laid out, and in which window.
#[derive(Clone, Copy, PartialEq)]
struct Covering {
    by: Cover,
    window: AnyWindowHandle,
    bounds: Bounds<Pixels>,
}

/// Every overlay over the watch page now: empty while nothing covers any
/// of it.
struct Veil {
    covers: Vec<Covering>,
}

impl Global for Veil {}

impl Veil {
    fn of(by: Cover, cx: &App) -> Option<(AnyWindowHandle, Bounds<Pixels>)> {
        cx.try_global::<Veil>()?
            .covers
            .iter()
            .find(|covering| covering.by == by)
            .map(|covering| (covering.window, covering.bounds))
    }
}

/// Note that `by` is laid out at `bounds` in `window` this frame. Returns
/// whether that moved it, the only time the probes need another frame to
/// read it in.
pub fn cover(by: Cover, bounds: Bounds<Pixels>, window: &Window, cx: &mut App) -> bool {
    set(by, Some((window.window_handle(), bounds)), cx)
}

/// `by` covers nothing now: on every frame it is not drawn. Free when there
/// was nothing to lift.
pub fn lift(by: Cover, cx: &mut App) -> bool {
    set(by, None, cx)
}

/// Where `by` was last laid out, if in `window`: for a chat, which counts
/// the pointer on its own card as on the chat.
pub fn bounds_of(by: Cover, window: &Window, cx: &App) -> Option<Bounds<Pixels>> {
    Veil::of(by, cx)
        .filter(|(drawn_in, _)| *drawn_in == window.window_handle())
        .map(|(_, bounds)| bounds)
}

/// Read before it is written, and written only on a change: every write of
/// a global queues an effect for its observers, and this is asked on every
/// frame.
fn set(by: Cover, over: Option<(AnyWindowHandle, Bounds<Pixels>)>, cx: &mut App) -> bool {
    if Veil::of(by, cx) == over {
        return false;
    }
    let mut covers = cx
        .try_global::<Veil>()
        .map_or_else(Vec::new, |veil| veil.covers.clone());
    covers.retain(|covering| covering.by != by);
    covers.extend(over.map(|(window, bounds)| Covering { by, window, bounds }));
    cx.set_global(Veil { covers });
    true
}

/// Whether the pointer is over `bounds` in `window`, as a probe asks it: the
/// window has the pointer, the pointer is inside `bounds`, and it is on no
/// overlay in that window but `own`, the probe's own pane's card
/// ([`reaches`]).
pub fn pointer_over(bounds: Bounds<Pixels>, own: Option<Cover>, window: &Window, cx: &App) -> bool {
    let handle = window.window_handle();
    let covers = cx
        .try_global::<Veil>()
        .map_or(&[][..], |veil| veil.covers.as_slice())
        .iter()
        .filter(|covering| covering.window == handle)
        .map(|covering| (covering.by, covering.bounds));
    window.is_window_hovered() && reaches(window.mouse_position(), bounds, covers, own)
}

/// Whether `pointer` is the element's at `bounds` with `covers` drawn over
/// the page: inside the one, and inside none of the others but `own`.
fn reaches(
    pointer: Point<Pixels>,
    bounds: Bounds<Pixels>,
    covers: impl IntoIterator<Item = (Cover, Bounds<Pixels>)>,
    own: Option<Cover>,
) -> bool {
    bounds.contains(&pointer)
        && !covers
            .into_iter()
            .any(|(by, covered)| Some(by) != own && covered.contains(&pointer))
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

    /// A pane covering the page, and another pane's card over the lower
    /// left of it.
    const PANE: (f32, f32, f32, f32) = (0.0, 0.0, 1000.0, 800.0);
    const CARD: (f32, f32, f32, f32) = (10.0, 400.0, 300.0, 390.0);

    fn pane() -> Bounds<Pixels> {
        rect(PANE.0, PANE.1, PANE.2, PANE.3)
    }

    fn card() -> (Cover, Bounds<Pixels>) {
        (
            Cover::Card(EntityId::from(9)),
            rect(CARD.0, CARD.1, CARD.2, CARD.3),
        )
    }

    /// With nothing over the page, the pane has the pointer wherever it is
    /// inside it, and nowhere else.
    #[test]
    fn with_no_veil_the_pane_has_its_own_bounds() {
        assert!(reaches(point(px(500.0), px(600.0)), pane(), [], None));
        assert!(!reaches(point(px(1200.0), px(600.0)), pane(), [], None));
    }

    /// On the card the pointer is nobody's under it; above the card it is
    /// the pane's again, and so is the strip beside it.
    #[test]
    fn the_veil_takes_the_pointer_from_what_is_under_it() {
        assert!(!reaches(
            point(px(100.0), px(600.0)),
            pane(),
            [card()],
            None
        ));
        assert!(reaches(point(px(100.0), px(200.0)), pane(), [card()], None));
        assert!(reaches(point(px(4.0), px(600.0)), pane(), [card()], None));
    }

    /// A veil away from the pane changes nothing about it.
    #[test]
    fn a_veil_elsewhere_leaves_the_pane_alone() {
        let veil = (
            Cover::Card(EntityId::from(9)),
            rect(2000.0, 0.0, 100.0, 100.0),
        );
        assert!(reaches(point(px(500.0), px(600.0)), pane(), [veil], None));
    }

    /// Two panes stacked, the lower one's card risen over the foot of the
    /// upper. The pointer on the card is not the upper pane's, whose own
    /// chat has a card of its own to leave out, nor its player's; off the
    /// card it is. The lower pane leaves its own card out, and another
    /// chat's card over it takes the pointer from it all the same.
    #[test]
    fn a_card_is_its_own_panes_and_covers_the_rest() {
        let (upper_chat, lower_chat) = (EntityId::from(1), EntityId::from(2));
        let upper = rect(0.0, 0.0, 1000.0, 400.0);
        let lower = rect(0.0, 400.0, 1000.0, 400.0);
        let card = (Cover::Card(lower_chat), rect(20.0, 300.0, 300.0, 260.0));
        let on_card = point(px(100.0), px(350.0));
        assert!(!reaches(
            on_card,
            upper,
            [card],
            Some(Cover::Card(upper_chat))
        ));
        assert!(!reaches(on_card, upper, [card], None));
        assert!(reaches(point(px(500.0), px(350.0)), upper, [card], None));

        let on_both = point(px(100.0), px(450.0));
        assert!(reaches(
            on_both,
            lower,
            [card],
            Some(Cover::Card(lower_chat))
        ));
        assert!(!reaches(on_both, lower, [card], None));
        let other = (Cover::Card(upper_chat), rect(0.0, 420.0, 1000.0, 380.0));
        assert!(!reaches(
            on_both,
            lower,
            [card, other],
            Some(Cover::Card(lower_chat))
        ));
    }
}
