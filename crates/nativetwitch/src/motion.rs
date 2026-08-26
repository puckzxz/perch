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
        }
    }

    /// Returns whether this actually changed anything, so callers can skip a
    /// repaint on the mouse-move events that do not cross a boundary — which
    /// is most of them.
    pub fn set(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return false;
        }
        self.visible = visible;
        self.flips += 1;
        true
    }

    /// Wrap `element` so its opacity follows this state.
    ///
    /// `id` only has to separate this fade from the others in the same view;
    /// GPUI already scopes element state per entity. The flip count is folded
    /// in here rather than by the caller, since that is mechanism rather than
    /// meaning.
    pub fn apply<E>(&self, id: impl Into<ElementId>, duration: Duration, element: E) -> AnyElement
    where
        E: Styled + IntoElement + 'static,
    {
        if self.flips == 0 {
            let resting = if self.visible { 1.0 } else { 0.0 };
            return element.opacity(resting).into_any_element();
        }

        let appearing = self.visible;
        element
            .with_animation(
                self.animation_id(id),
                Animation::new(duration).with_easing(theme::ease_fade()),
                move |element, delta| element.opacity(if appearing { delta } else { 1.0 - delta }),
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

    #[test]
    fn every_change_is_reported_once() {
        let mut fade = Fade::hidden();
        assert!(fade.set(true));
        assert!(fade.set(false));
        assert!(fade.set(true));
        assert_eq!(fade.flips, 3);
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
