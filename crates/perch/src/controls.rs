//! The one button in this app.
//!
//! There were ten. `pill` and `tab_pill` in the shell, another `pill` on the
//! video, the offline follow, the Load more row, the context bar's back, the
//! card's `+ add`, the activate button, chat's jump-to-live and the pane's
//! close — every one of them a `div` with its own padding, its own hover and
//! its own idea of what a pressed control looks like. Six recipes for one
//! control, which is the drift `theme.rs` opens by warning about: the *tokens*
//! were shared the whole time; the *component* was not.
//!
//! So: one builder, and a variant for each job a button in this app actually
//! does. Anything that needs a shape not on this list is a new variant here
//! rather than a tenth `div`.

use gpui::{div, prelude::*, px, ElementId, SharedString, Stateful};

use crate::theme;

/// What a control is *for*, which is what decides how it looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// The ordinary case: a filled pill on a panel. Refresh, settings, back.
    Pill,
    /// A pill that is currently the answer — the open tab, the chosen quality.
    Selected,
    /// On top of live video, where a filled background would be one more thing
    /// covering the picture. Carries its own weight through text alone until
    /// hovered.
    OnVideo,
    /// The one thing to do in a view: sign in, jump to live. Bordered in the
    /// accent rather than filled with it, so it reads as important without
    /// becoming the brightest thing on screen.
    Primary,
    /// A control that is present but not being offered — close, in a header
    /// whose job is the channel's name.
    Quiet,
    /// A control that destroys something. Drawn like `Quiet`, and turns the
    /// colour of the thing it is about to do only under the pointer.
    Destructive,
}

impl Variant {
    fn background(self) -> Option<gpui::Hsla> {
        match self {
            Variant::Pill | Variant::Primary => Some(theme::surface_raised()),
            Variant::Selected => Some(theme::accent_dim()),
            Variant::OnVideo | Variant::Quiet | Variant::Destructive => None,
        }
    }

    fn foreground(self) -> gpui::Hsla {
        match self {
            Variant::Pill => theme::text_muted(),
            Variant::Selected | Variant::OnVideo | Variant::Primary => theme::text(),
            Variant::Quiet | Variant::Destructive => theme::text_dim(),
        }
    }

    /// The label's colour under the pointer. One signal for a destructive
    /// control, and the same lift to full text for everything else.
    fn hover_foreground(self) -> gpui::Hsla {
        match self {
            Variant::Destructive => theme::danger(),
            _ => theme::text(),
        }
    }
}

/// A control, styled and ready for `.on_click(..)`.
///
/// Returns the `Stateful<Div>` rather than a finished element so callers can
/// still hang a listener, a tooltip or an extra child on it — the styling is
/// what had to be shared, not the wiring.
pub fn pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: Variant,
) -> Stateful<gpui::Div> {
    let mut control = div()
        .id(id.into())
        .flex_none()
        .px(px(theme::CONTROL_PAD_X))
        .py(px(theme::CONTROL_PAD_Y))
        .rounded(px(theme::RADIUS))
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(variant.foreground())
        .cursor_pointer();

    if let Some(background) = variant.background() {
        control = control.bg(background);
    }
    if variant == Variant::Primary {
        control = control.border_1().border_color(theme::accent());
    }

    // One `hover` call, and only here. gpui's `hover` may be set once per
    // element - a second call is a debug assertion, which is how a debug
    // build with two panes open used to panic on the close button - so the
    // variant decides what the pointer does rather than a caller adding to it.
    let hover_text = variant.hover_foreground();
    control
        .hover(move |style| style.bg(theme::hover()).text_color(hover_text))
        // Stronger than hover rather than a different colour, so a press reads
        // as more of the same gesture. On video there is no shadow or border to
        // deform, so this is the only channel a press has.
        .active(|style| style.bg(theme::pressed()))
        .child(label.into())
}

/// A control that destroys something, which earns exactly one signal: it turns
/// the colour of the thing it is about to do, and only under the pointer.
pub fn destructive(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<gpui::Div> {
    pill(id, label, Variant::Destructive)
}

/// A state, said in a word: `muted`, `paused`. Not a control - no pointer, no
/// hover - so it reads as a fact about the pane rather than as a button that
/// would change it. The controls that change it are on the video, and the
/// keys are in the settings sheet.
pub fn tag(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_none()
        .px(px(theme::GAP_TIGHT))
        .py(px(1.))
        .rounded(px(theme::RADIUS))
        .bg(theme::accent_dim())
        .text_size(px(theme::TEXT_META))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text())
        .child(label.into())
}

/// The dot that says "this number is of people watching right now".
///
/// One function rather than three copies: it sits beside the viewer count on
/// a browse card, in a rail row and in a pane header, and it was drawn by
/// hand in each — the same six pixels in the same red, three times, which is
/// exactly the drift `theme.rs` opens by warning about.
pub fn live_dot() -> gpui::Div {
    div()
        .flex_none()
        .w(px(theme::LIVE_DOT))
        .h(px(theme::LIVE_DOT))
        .rounded_full()
        .bg(theme::live())
}

/// A control that is not being offered right now: the Load more row while its
/// page is in flight.
///
/// Rendered as the same shape with no pointer and no hover, rather than as
/// nothing — a row that disappears while you are reaching for it is worse than
/// one that says wait.
pub fn waiting(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_none()
        .px(px(theme::CONTROL_PAD_X))
        .py(px(theme::CONTROL_PAD_Y))
        .rounded(px(theme::RADIUS))
        .bg(theme::surface_raised())
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text_dim())
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant has to be legible on the surface it is drawn on. The two
    /// with no background of their own are checked against the darkest thing
    /// they can land on, which for `OnVideo` is a black picture and for `Quiet`
    /// is the pane surface.
    #[test]
    fn every_variant_is_legible_where_it_sits() {
        let cases = [
            (Variant::Pill, theme::surface_raised()),
            (Variant::Selected, theme::surface()),
            (Variant::OnVideo, theme::player_bg()),
            (Variant::Primary, theme::surface_raised()),
            (Variant::Quiet, theme::surface()),
            (Variant::Destructive, theme::surface()),
        ];

        for (variant, behind) in cases {
            // Against what is actually behind the label rather than against
            // `background()`: the selected variant's fill is a wash, so the
            // surface under it is what decides.
            let ratio = theme::contrast(variant.foreground(), behind);
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "a control label reads {ratio:.2}:1 on what it sits on"
            );
        }
    }
}
