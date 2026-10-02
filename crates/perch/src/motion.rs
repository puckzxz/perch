//! Motion: the vocabulary, and the one piece of state it needs.
//!
//! GPUI has no CSS-style transitions. `.hover()` swaps styles instantly, and
//! [`gpui::AnimationExt::with_animation`] always runs forward from zero,
//! starting the moment GPUI first sees that element id. So there are exactly
//! two shapes available, and this module is both of them:
//!
//! - **Arrivals** ([`arrive`], [`waiting`]) need no state. The element is
//!   mounted, GPUI has not seen the id before, the animation runs.
//! - **Two-way changes** ([`Fade`]) do, because nothing about "the pointer
//!   left" is visible to an element that is being re-rendered from scratch
//!   every frame. The state lives on the view; the animation reads it.
//!
//! Durations and easings are in [`crate::theme`] with the rest of the design
//! tokens. Nothing here invents its own timing.

use std::cell::Cell;
use std::time::Duration;

use gpui::{
    px, Animation, AnimationElement, AnimationExt, AnyElement, ElementId, IntoElement,
    SharedString, Styled,
};

use crate::theme;

/// A two-state opacity fade, driven by state the view holds.
///
/// The trick is the element id: GPUI keys animation state on it, so a *new* id
/// is a new animation and an unchanged one is a finished animation holding its
/// last value. Flipping visibility therefore means minting a fresh id, which is
/// what `flips` is for.
#[derive(Default)]
pub struct Fade {
    visible: bool,
    /// How many times visibility has changed. Part of the element id, so each
    /// change restarts the clock.
    ///
    /// Zero means "never changed", and renders the resting state with no
    /// animation element at all. Without that, everything currently hidden
    /// would play its fade-out once on launch.
    flips: u32,
    /// The state the last frame drew, `visible` and `flips`, once one has:
    /// what [`set`](Self::set) goes back to when a flip is undone before any
    /// frame drew it. A cell because [`apply`](Self::apply) reads the fade
    /// from a render.
    drawn: Cell<Option<(bool, u32)>>,
}

impl Fade {
    /// Starts hidden and stays that way until something reveals it.
    pub fn hidden() -> Self {
        Self::default()
    }

    /// Starts visible, and fades in the first time it is rendered.
    ///
    /// For things created in response to something the user did, where the
    /// creation *is* the event worth showing.
    pub fn entering() -> Self {
        Self {
            visible: true,
            flips: 1,
            drawn: Cell::new(None),
        }
    }

    /// Returns whether this actually changed anything, so callers can skip a
    /// repaint on the mouse-move events that do not cross a boundary — which
    /// is most of them.
    ///
    /// A flip undone before any frame drew it is no flip: the fade goes back
    /// to the id the last frame drew under, so whatever was showing goes on
    /// as it was. Otherwise a hide and a show in one event — a pick from a
    /// pane's quality menu closes the menu, which may let the bar go, and the
    /// switch it starts holds the bar up again — minted a fresh id, and the
    /// bar that never went blinked out and faded back in.
    pub fn set(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        self.flips = match self.drawn.get() {
            Some((drawn, flips)) if drawn == visible => flips,
            _ => self.flips + 1,
        };
        true
    }

    /// Whether this is showing, or on its way to showing: what an element
    /// that blocks the pointer asks before it does so (see `apply`).
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Wrap `element` so its opacity follows this state.
    ///
    /// `id` only has to separate this fade from the others in the same view;
    /// GPUI already scopes element state per entity. The flip count is folded
    /// in here rather than by the caller, since that is mechanism rather than
    /// meaning.
    ///
    /// Keep the wrapped element mounted on every frame, or start the fade over
    /// with [`Fade::hidden`] when the element goes away. gpui keeps an
    /// animation's state only from one frame to the next, so an element that
    /// comes back after a frame without it replays its last flip from the
    /// start: a bar that was hidden long ago fades out all over again.
    pub fn apply<E>(&self, id: impl Into<ElementId>, duration: Duration, element: E) -> AnyElement
    where
        E: Styled + IntoElement + 'static,
    {
        // Hidden is `invisible`, not merely transparent. At opacity zero the
        // element is still there to be clicked: a control faded out of a
        // corner that looked empty went on taking clicks there, and navigated
        // away from under the pointer. gpui paints nothing of an invisible
        // element and registers none of its listeners, so hidden means gone
        // to the pointer too — at rest, and from the last frame of a fade-out,
        // which a finished animation goes on rendering for as long as it is on
        // screen.
        //
        // Gone to the listeners, not to the hit test. gpui still lays an
        // invisible element out and inserts its hitbox, which it does in
        // prepaint, before paint returns early for it (div.rs:1676-1680 and
        // 1809). So something under here that blocks the pointer goes on
        // blocking it while hidden, with nothing drawn: `.occlude()` only
        // while `is_visible`.
        self.drawn.set(Some((self.visible, self.flips)));
        if self.flips == 0 {
            return if self.visible {
                element.into_any_element()
            } else {
                element.invisible().into_any_element()
            };
        }

        let appearing = self.visible;
        element
            .with_animation(
                self.animation_id(id),
                Animation::new(duration).with_easing(theme::ease_fade()),
                move |element, delta| {
                    if appearing {
                        element.opacity(delta)
                    } else if delta >= 1.0 {
                        element.opacity(0.0).invisible()
                    } else {
                        element.opacity(1.0 - delta)
                    }
                },
            )
            .into_any_element()
    }

    /// The element id this fade animates under right now.
    ///
    /// `NamedChild` composes rather than replaces, so the caller's id — which
    /// says *which* thing is fading — survives having the flip count appended.
    fn animation_id(&self, id: impl Into<ElementId>) -> ElementId {
        ElementId::NamedChild(
            Box::new(id.into()),
            SharedString::from(self.flips.to_string()),
        )
    }
}

/// A one-shot arrival for something that has just been mounted: fades up while
/// closing the last `rise` pixels of distance.
///
/// The movement is small on purpose. It exists to point the eye at where the
/// thing came from, not to be watched.
pub fn arrive<E>(id: impl Into<ElementId>, rise: f32, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(
        id.into(),
        Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
        move |element, delta| element.opacity(delta).mt(px(rise * (1.0 - delta))),
    )
}

/// Breathe, for as long as something is being waited on.
///
/// Repeating, so it runs until the state it describes is gone. Only ever put
/// this on a state that ends: an error that pulses forever is both irritating
/// and a permanent 60 fps repaint.
pub fn waiting<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(
        id.into(),
        Animation::new(theme::PULSE_PERIOD)
            .repeat()
            .with_easing(gpui::pulsating_between(theme::PULSE_FLOOR, 1.0)),
        |element, delta| element.opacity(delta),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_hidden_fade_has_nothing_to_play() {
        let fade = Fade::hidden();
        assert!(!fade.visible);
        assert_eq!(fade.flips, 0);
    }

    /// Things built in response to an action - a toast, a panel - should fade
    /// up the first time they are drawn, because the arrival is the event.
    #[test]
    fn entering_animates_on_its_first_render() {
        let fade = Fade::entering();
        assert!(fade.visible);
        assert_eq!(fade.flips, 1);
    }

    /// Most mouse moves land inside the element the pointer is already in.
    /// Reporting those as changes would repaint the window for nothing.
    #[test]
    fn setting_the_same_value_reports_no_change() {
        let mut fade = Fade::hidden();
        assert!(!fade.set(false));
        assert_eq!(fade.flips, 0);

        let mut fade = Fade::entering();
        assert!(!fade.set(true));
        assert_eq!(fade.flips, 1);
    }

    /// What an occluder asks before it blocks the pointer, so it has to
    /// change the moment `set` does, ahead of the fade it starts.
    #[test]
    fn is_visible_follows_set() {
        let mut fade = Fade::hidden();
        assert!(!fade.is_visible());
        fade.set(true);
        assert!(fade.is_visible());
        fade.set(false);
        assert!(!fade.is_visible());
        assert!(Fade::entering().is_visible());
    }

    #[test]
    fn every_change_is_reported_once() {
        let mut fade = Fade::hidden();
        assert!(fade.set(true));
        assert!(fade.set(false));
        assert!(fade.set(true));
        assert_eq!(fade.flips, 3);
    }

    /// A flip undone before a frame drew it goes back to the id that frame
    /// drew under, so a bar hidden and shown again in one event goes on as
    /// it was rather than fading in from nothing; a flip a frame did draw is
    /// a change like any other.
    #[test]
    fn a_flip_undone_before_a_frame_is_no_flip() {
        let mut fade = Fade::hidden();
        fade.set(true);
        fade.drawn.set(Some((fade.visible, fade.flips)));
        let shown = fade.animation_id("controls");
        assert!(fade.set(false), "it did change, for now");
        assert!(fade.set(true));
        assert_eq!(fade.animation_id("controls"), shown);

        fade.set(false);
        fade.drawn.set(Some((fade.visible, fade.flips)));
        fade.set(true);
        assert_ne!(fade.animation_id("controls"), shown);
    }

    /// The whole mechanism rests on this: GPUI keys animation state on the
    /// element id, so a flip that reused its id would hand back the *finished*
    /// animation from last time and nothing would move.
    #[test]
    fn each_flip_animates_under_a_new_id() {
        let mut fade = Fade::hidden();
        let hidden = fade.animation_id("controls");
        fade.set(true);
        let shown = fade.animation_id("controls");
        assert_ne!(hidden, shown);
    }

    /// Two fades that flip in step must still stay apart, or one pane's
    /// controls drive another's.
    #[test]
    fn different_callers_stay_apart_at_the_same_flip_count() {
        let mut left = Fade::hidden();
        let mut right = Fade::hidden();
        left.set(true);
        right.set(true);
        assert_ne!(
            left.animation_id(("pane-header", 0usize)),
            right.animation_id(("pane-header", 1usize))
        );
    }
}
