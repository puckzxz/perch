//! Handing [`crate::theme`]'s tokens to the widget library.
//!
//! `gpui-component` keeps its own global palette, and `gpui_component::init`
//! seeds it from `cx.window_appearance()` — the **operating system's** light or
//! dark setting. Nothing else in this app asks the OS anything: the palette in
//! `theme.rs` is dark, full stop. So on a machine set to light mode every text
//! field, dropdown, button, slider and scrollbar came up light while the window
//! around them stayed dark, and on a dark machine they merely disagreed more
//! quietly — the Save button was the library's near-white `primary` rather than
//! this app's teal, and its base font size was 16px against a type scale whose
//! largest role is 15.
//!
//! One function, called once, after `init` and before any widget is built. It
//! is the only place the two palettes touch, so "one place for every colour in
//! the app" survives having a widget library in it.

use gpui::App;
use gpui_component::{Theme, ThemeMode};

use crate::theme;

/// Point the widget library at this app's tokens.
///
/// Must run **after** `gpui_component::init`, which installs the global this
/// overwrites, and before the first widget is constructed.
pub fn apply(cx: &mut App) {
    let theme_ref = Theme::global_mut(cx);

    // Pinned rather than synced. The app has one appearance; asking the OS
    // would only ever produce a window that disagrees with itself.
    theme_ref.mode = ThemeMode::Dark;

    theme_ref.font_size = gpui::px(theme::TEXT_BODY);
    theme_ref.radius = gpui::px(theme::RADIUS);
    theme_ref.radius_lg = gpui::px(theme::RADIUS_LG);

    let colors = &mut theme_ref.colors;

    // Surfaces and text, in the same three tiers the rest of the app uses.
    colors.background = theme::bg();
    colors.foreground = theme::text();
    colors.border = theme::border();
    colors.muted = theme::surface_raised();
    colors.muted_foreground = theme::text_dim();
    colors.popover = theme::surface_raised();
    colors.popover_foreground = theme::text();
    colors.input = theme::border();
    colors.overlay = theme::scrim();
    colors.title_bar = theme::surface();
    colors.title_bar_border = theme::border();

    // The accent, everywhere the library would otherwise put its own.
    //
    // `primary_foreground` is the *background* colour on purpose: white on this
    // teal measures 3.2:1, under AA, while the window's own near-black reads
    // 5.8:1 on it. A primary button is the one control in a view that has to be
    // legible.
    colors.primary = theme::accent();
    colors.primary_hover = theme::accent();
    colors.primary_active = theme::accent();
    colors.primary_foreground = theme::bg();
    colors.accent = theme::accent_dim();
    colors.accent_foreground = theme::text();
    colors.ring = theme::accent();
    colors.caret = theme::accent();
    colors.selection = theme::accent_dim();
    colors.link = theme::accent();
    colors.link_hover = theme::text();
    colors.link_active = theme::accent();

    // Secondary is what a ghost or outline button falls back to.
    colors.secondary = theme::surface_raised();
    colors.secondary_hover = theme::hover();
    colors.secondary_active = theme::pressed();
    colors.secondary_foreground = theme::text();

    colors.danger = theme::danger();
    colors.danger_hover = theme::danger();
    colors.danger_active = theme::danger();
    colors.danger_foreground = theme::bg();

    // Lists: the dropdown a Select opens, and anything else with rows.
    colors.list = theme::surface_raised();
    colors.list_even = theme::surface_raised();
    colors.list_hover = theme::hover();
    colors.list_active = theme::accent_dim();
    colors.list_active_border = theme::accent();
    colors.list_head = theme::surface();

    // The volume slider, which sits on live video and was drawing in the
    // library's greys.
    colors.slider_bar = theme::accent();
    colors.slider_thumb = theme::text();
    colors.progress_bar = theme::accent();

    colors.scrollbar = gpui::transparent_black();
    colors.scrollbar_thumb = theme::hover();
    colors.scrollbar_thumb_hover = theme::pressed();

    colors.switch = theme::surface_raised();
    colors.switch_thumb = theme::text();
    colors.skeleton = theme::surface_raised();
    colors.drop_target = theme::accent_dim();
    colors.drag_border = theme::accent();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one claim this module makes that is worth checking without a window:
    /// a primary button's label has to be readable on a primary button.
    ///
    /// White would be the obvious choice and is the wrong one — it measures
    /// 3.2:1 on this teal. The test is here rather than in `theme` because the
    /// pairing is this file's decision.
    #[test]
    fn a_primary_button_is_legible() {
        let ratio = theme::contrast(theme::bg(), theme::accent());
        assert!(
            ratio >= theme::MIN_CONTRAST,
            "primary_foreground on primary reads {ratio:.2}:1"
        );

        let white: gpui::Hsla = gpui::rgb(0xffffff).into();
        assert!(
            theme::contrast(white, theme::accent()) < theme::MIN_CONTRAST,
            "white now clears the bar on the accent, so the comment above is stale"
        );
    }
}
