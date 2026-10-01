//! The one button in this app.
//!
//! There were ten. `pill` and `tab_pill` in the shell, another `pill` on the
//! video, the offline follow, the Load more row, the context bar's back, the
//! card's `+ Add`, the activate button, chat's jump-to-live and the pane's
//! close — every one of them a `div` with its own padding, its own hover and
//! its own idea of what a pressed control looks like. Six recipes for one
//! control, which is the drift `theme.rs` opens by warning about: the *tokens*
//! were shared the whole time; the *component* was not.
//!
//! So: one builder, and a variant for each job a button in this app actually
//! does. Anything that needs a shape not on this list is a new variant here
//! rather than a tenth `div`.
//!
//! A picture instead of a word is the same control in a square,
//! [`icon_button`], wearing the same variants, and [`icon_waiting`] while it
//! is not on offer. What a control says under the pointer is [`tip`], or
//! [`full_text`] for the rest of a line cut short. A heading that folds away
//! what it heads is [`fold`], built on [`group_heading`], the plain heading
//! it sits among. The window's minimise, maximise and close are here too, as
//! [`caption_button`], because they look like controls — but they are the
//! platform's to press, not the app's.

use gpui::{
    div, prelude::*, px, svg, AnyView, App, Div, ElementId, SharedString, Stateful, Window,
    WindowControlArea,
};

use crate::assets::Icon;
use crate::theme;

/// How wide a tooltip carrying full text is.
///
/// A width, not a maximum, and it goes on the text's own container rather than
/// on the tooltip: gpui wraps only where the measure pass has a definite width,
/// a tooltip is laid out against `AvailableSpace::min_size()`, and a width on
/// the tooltip's outer box does not reach the text through the library's own
/// flex row. Both other spellings were built and looked at — each drew a
/// ninety-character title as one line running most of the way across a 4K
/// window.
///
/// Roughly a card's text column, so a title reads at the width it was written
/// under. Fixed rather than fitted to the text, because it cannot be: measuring
/// a string needs the font, and a tooltip that is a different width every time
/// is its own kind of noise.
const TOOLTIP_WIDTH: f32 = 300.0;

/// The whole of something a line had to cut short.
///
/// Titles are clamped to one line everywhere they appear, because a card is
/// only so wide and a pane header is narrower — and the half of a title that
/// did not fit is often the half that says what the stream is. This is the
/// way to the rest of it, and there is one of them so that hovering a title on
/// a browse card and hovering one in a pane header do the same thing.
///
/// Later lines are quieter than the first, which is what makes a title and the
/// game under it read as one thing and its footnote rather than as two facts.
/// Empty ones are dropped: a channel with no game set would otherwise get a
/// blank row.
pub fn full_text(
    lines: impl IntoIterator<Item = SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let lines: Vec<SharedString> = lines
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect();

    move |window, cx| {
        let lines = lines.clone();
        gpui_component::tooltip::Tooltip::element(move |_, _| {
            div()
                .w(px(TOOLTIP_WIDTH))
                .flex()
                .flex_col()
                .gap(px(theme::GAP_WORD))
                .children(lines.iter().enumerate().map(|(row, line)| {
                    div()
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(if row == 0 {
                            theme::text()
                        } else {
                            theme::text_dim()
                        })
                        .child(line.clone())
                }))
        })
        .build(window, cx)
    }
}

/// A tooltip that is one line of words: what a control does, and — through
/// `keys::Hint`, never spelled out by hand — the key that does it too.
///
/// The one builder for those, beside [`full_text`] for text a line cut
/// short, so every plain tooltip in the app is drawn the same way. It hands
/// back the builder rather than taking the element, because gpui's `tooltip`
/// takes a builder, and takes it once per element (div.rs:536-549): text
/// handed to it does not compile, and a second call is a debug assertion.
///
/// What the words say is decided when the tooltip comes up and not again,
/// so a control whose words follow a state it can change — Pause and Play,
/// Mute all and Unmute all — keys its element id on that state as well. The
/// first frame drawn under the other id drops the open tooltip with the rest
/// of the element's state, and the next one to come up says the new words;
/// [`caption_button`] does the same for its press.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

/// Declares [`Variant`] once and derives, for the tests only, the list of every
/// variant from that same declaration — so a new one cannot exist without
/// `every_variant_is_legible_where_it_sits` measuring it. The same shape as
/// `assets::perch_icons!`, and `ALL` is `cfg(test)` for the same reason: this
/// crate has no library half, so a constant only the tests read is dead code
/// to `clippy --all-targets`.
macro_rules! variants {
    ($($(#[$doc:meta])* $variant:ident,)*) => {
        /// What a control is *for*, which is what decides how it looks.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Variant {
            $($(#[$doc])* $variant,)*
        }

        impl Variant {
            #[cfg(test)]
            const ALL: &'static [Variant] = &[$(Variant::$variant),*];
        }
    };
}

variants! {
    /// The ordinary case: a filled pill on a panel. Refresh, back.
    Pill,
    /// A pill that is currently the answer — the open tab, the chosen quality.
    Selected,
    /// On top of live video, where a filled background would be one more thing
    /// covering the picture. Rests at `text_muted` on the bar's wash and lifts
    /// to full text under the pointer: the hover fill alone is a white wash at
    /// four percent, which on a dark bar over a moving picture is all but
    /// invisible, so the label's own lift is the cue that it will take the
    /// press. Measured on the brightest picture rather than on black.
    OnVideo,
    /// The one thing to do in a view: sign in, jump to live. Bordered in the
    /// accent rather than filled with it, so it reads as important without
    /// becoming the brightest thing on screen.
    Primary,
    /// A control that is present but not being offered — the settings sheet's
    /// Close, beside the Save that is the sheet's one thing to do.
    Quiet,
    /// A control that destroys something. Drawn like `Quiet`, and turns the
    /// colour of the thing it is about to do only under the pointer.
    Destructive,
    /// Furniture in the window's own frame — the title bar's gear. No fill
    /// until the pointer is on it, because a bar of filled squares along the
    /// top of the window is chrome you stare past for three hours.
    Chrome,
}

impl Variant {
    fn background(self) -> Option<gpui::Hsla> {
        match self {
            Variant::Pill | Variant::Primary => Some(theme::surface_raised()),
            Variant::Selected => Some(theme::accent_dim()),
            Variant::OnVideo | Variant::Quiet | Variant::Destructive | Variant::Chrome => None,
        }
    }

    fn foreground(self) -> gpui::Hsla {
        match self {
            Variant::Pill | Variant::Chrome | Variant::OnVideo => theme::text_muted(),
            Variant::Selected | Variant::Primary => theme::text(),
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

    /// The fill under the pointer. A chosen pill stays chosen: the hover wash
    /// used to replace its tint, so the tab you had just clicked looked like
    /// every other tab for as long as the pointer rested on it.
    ///
    /// A filled control gets the wash laid over its own fill, never the bare
    /// wash in its place. The wash is translucent, which is right for a
    /// control with no fill of its own and wrong for one with: on a card's
    /// picture the "Past broadcasts" pill went see-through the moment the
    /// pointer reached it, the opposite of a control lighting up.
    fn hover_background(self) -> gpui::Hsla {
        match (self, self.background()) {
            (Variant::Selected, _) => theme::accent_dim(),
            (_, Some(fill)) => fill.blend(theme::hover()),
            (_, None) => theme::hover(),
        }
    }

    /// The fill while held down: the press wash over the control's own fill,
    /// for the same reason as [`Variant::hover_background`].
    fn pressed_background(self) -> gpui::Hsla {
        match self.background() {
            Some(fill) => fill.blend(theme::pressed()),
            None => theme::pressed(),
        }
    }

    /// The resting fill, and the accent border `Primary` wears: what a variant
    /// does to any shape of control, a label's pill and an icon's square alike.
    fn dress(self, mut control: Stateful<Div>) -> Stateful<Div> {
        if let Some(background) = self.background() {
            control = control.bg(background);
        }
        if self == Variant::Primary {
            control = control.border_1().border_color(theme::accent());
        }
        control
    }
}

/// The group an icon button's glyph watches for the pointer; see [`icon_button`].
const ICON_GROUP: &str = "icon-button";

/// The group a caption button's glyph watches; see [`caption_button`].
const CAPTION_GROUP: &str = "caption-button";

/// The group a fold's chevron watches; see [`fold`].
const FOLD_GROUP: &str = "fold";

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
    let control = variant.dress(
        div()
            .id(id.into())
            .flex_none()
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded(px(theme::RADIUS))
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .line_height(px(theme::LINE_TIGHT))
            .text_color(variant.foreground())
            .cursor_pointer(),
    );

    // One `hover` call, and only here. gpui's `hover` may be set once per
    // element - a second call is a debug assertion, which is how a debug
    // build with two panes open used to panic on the close button - so the
    // variant decides what the pointer does rather than a caller adding to it.
    let hover_text = variant.hover_foreground();
    let hover_fill = variant.hover_background();
    let pressed_fill = variant.pressed_background();
    control
        .hover(move |style| style.bg(hover_fill).text_color(hover_text))
        // Stronger than hover rather than a different colour, so a press reads
        // as more of the same gesture. On video there is no shadow or border to
        // deform, so this is the only channel a press has.
        .active(move |style| style.bg(pressed_fill))
        .child(label.into())
}

/// A control that is a picture rather than a word: a square of
/// [`theme::ICON_BUTTON`] with an [`Icon`] in the middle. Same variants, same
/// one `hover`, same press as [`pill`].
///
/// The glyph takes its colour through `group_hover` rather than from the
/// square's hover, because an `svg` paints with its own `text_color` and
/// inherits none — a glyph never given one draws nothing at all, and one
/// given only the square's would never lift under the pointer.
pub fn icon_button(id: impl Into<ElementId>, icon: Icon, variant: Variant) -> Stateful<Div> {
    let hover_fill = variant.hover_background();
    let pressed_fill = variant.pressed_background();
    let hover_glyph = variant.hover_foreground();
    variant
        .dress(
            div()
                .id(id.into())
                .group(ICON_GROUP)
                .flex_none()
                .size(px(theme::ICON_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme::RADIUS))
                .cursor_pointer(),
        )
        // The one `hover`; see `pill`.
        .hover(move |style| style.bg(hover_fill))
        .active(move |style| style.bg(pressed_fill))
        .child(
            svg()
                .path(icon.path())
                .flex_none()
                .size(px(theme::ICON))
                .text_color(variant.foreground())
                .group_hover(ICON_GROUP, move |style| style.text_color(hover_glyph)),
        )
}

/// An [`icon_button`] that is not being offered right now: back with nowhere
/// to go back to.
///
/// The same square in the same place, so the bar does not shift when it
/// comes and goes, with the glyph in `text_dim` and no hover, no pointer and
/// no handler — the picture's [`waiting`]. Never drawn at a lower opacity,
/// which is a control that still takes the click (see `HANDOFF.md`, "Things
/// not to redo"); this one has nothing to take it with.
pub fn icon_waiting(icon: Icon) -> Div {
    div()
        .flex_none()
        .size(px(theme::ICON_BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icon.path())
                .flex_none()
                .size(px(theme::ICON))
                .text_color(theme::text_dim()),
        )
}

/// One of the minimise, maximise and close buttons on a title bar Perch draws
/// itself, on Windows.
///
/// Deliberately not a clickable control. It has no click handler, no tooltip
/// and takes no focus: the press belongs to the platform, which gpui hands it
/// through `window_control_area` as `HTMINBUTTON`, `HTMAXBUTTON` or `HTCLOSE`.
/// Anything here that handled the press would stop that — gpui reports a
/// handled non-client press as done, and Windows never acts on it. Nor would
/// gpui's own `zoom_window` and `remove_window` do as well by hand: the first
/// maximises but never restores, and the second skips the close handler that
/// saves where the window was.
///
/// The area starts `drag_top` below the top of the button rather than at it,
/// so a windowed window keeps its top resize edge across the buttons too; see
/// `layout::drag_top`. The hover and the press light that area and nothing
/// above it. Lit across the button's full height, the strip over the area
/// turned Close red under a resize cursor, and a press there — which the
/// platform takes as the start of a resize, and whose release its size loop
/// keeps — left the button showing pressed, for a button that had done
/// nothing. The glyph rides in the area too, so it lifts with the area's
/// hover, held at the middle of the whole bar by padding under it as tall as
/// the strip over it.
///
/// Lit — hovered or pressed — only while `window_hovered`, which the caller
/// takes from `window.is_window_hovered()`. gpui hears the pointer leave the
/// window as a flag and nothing more: no move is sent, so the next frame
/// hit-tests the last position it saw, and these buttons sit on the edge the
/// pointer usually leaves by. Ungated, Close would stay red with the pointer
/// off the window, and Minimize come back from the taskbar still lit. The id
/// follows the flag for the press's sake. A press dragged off the window and
/// let go outside never comes back as a release, so gpui would hold the
/// button pressed until the next release anywhere in the window; the first
/// frame drawn under the other id drops that with the rest of the element's
/// state.
pub fn caption_button(
    area: WindowControlArea,
    icon: Icon,
    drag_top: f32,
    window_hovered: bool,
) -> Div {
    let close = area == WindowControlArea::Close;
    let (hover_fill, pressed_fill, hover_glyph) = if close {
        (
            theme::caption_close(),
            theme::caption_close_pressed(),
            theme::caption_close_glyph(),
        )
    } else {
        (theme::hover(), theme::pressed(), theme::text())
    };
    let id = if window_hovered {
        "caption-button"
    } else {
        "caption-button-at-rest"
    };
    // The button's slot in the bar, full height, drawing nothing itself: the
    // strip above the area is the platform's resize edge, and stays dark.
    div()
        .relative()
        .flex_none()
        .w(px(theme::CAPTION_BUTTON_WIDTH))
        .h_full()
        .child(
            // What answers as the button, and so the only part that lights.
            div()
                .id((id, area as usize))
                .group(CAPTION_GROUP)
                .absolute()
                .top(px(drag_top))
                .left_0()
                .right_0()
                .bottom_0()
                .pb(px(drag_top))
                .flex()
                .items_center()
                .justify_center()
                .window_control_area(area)
                .when(window_hovered, |button| {
                    button
                        .hover(move |style| style.bg(hover_fill))
                        .active(move |style| style.bg(pressed_fill))
                })
                .child(
                    svg()
                        .path(icon.path())
                        .flex_none()
                        .size(px(theme::CAPTION_GLYPH))
                        .text_color(theme::text_muted())
                        .when(window_hovered, |glyph| {
                            glyph.group_hover(CAPTION_GROUP, move |style| {
                                style.text_color(hover_glyph)
                            })
                        }),
                ),
        )
}

/// The box a group's name sits in over the rows it heads, with nothing in it
/// yet: the rail's `Pinned` and `Live`, and the start of a [`fold`] for its
/// `Offline`. Not a control; here because [`fold`] is built on it, and the
/// fold sits in the same column as the headings above it, at the same size,
/// weight and colour, on the same inset. One recipe for both, so the fold
/// cannot drift from them by somebody restyling one and not the other.
pub fn group_heading() -> Div {
    div()
        .flex_none()
        .px(px(theme::PANEL_PAD))
        .py(px(theme::GAP_TIGHT))
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text_dim())
}

/// A heading that folds away the rows under it: the rail's offline follows,
/// under their count. `open` says whether the rows are showing, and the
/// chevron in front points at them when they are and away when they are not.
///
/// A heading first — a [`group_heading`], and no fill until the pointer is on
/// it — because it sits in a column of channels, and a filled bar there would
/// read as one more. The chevron takes its colour through `group_hover`, for
/// the reason [`icon_button`]'s glyph does.
pub fn fold(id: impl Into<ElementId>, label: impl Into<SharedString>, open: bool) -> Stateful<Div> {
    let icon = if open { Icon::Unfolded } else { Icon::Folded };
    group_heading()
        .id(id.into())
        .group(FOLD_GROUP)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_WORD))
        .rounded(px(theme::RADIUS))
        .cursor_pointer()
        // The one `hover`; see `pill`.
        .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
        .active(|style| style.bg(theme::pressed()))
        .child(
            svg()
                .path(icon.path())
                .flex_none()
                .size(px(theme::ICON))
                .text_color(theme::text_dim())
                .group_hover(FOLD_GROUP, |style| style.text_color(theme::text())),
        )
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
/// would change it. The controls that change it are on the video's bar,
/// whose tooltips name their keys, and every key is listed in the settings
/// sheet.
pub fn tag(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_none()
        .px(px(theme::GAP_TIGHT))
        .py(px(theme::TAG_PAD_Y))
        .rounded(px(theme::RADIUS))
        .bg(theme::accent_dim())
        .text_size(px(theme::TEXT_META))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text())
        .child(label.into())
}

/// A fact drawn on a picture: the viewers and uptime on a stream's thumbnail,
/// a recording's length or where it was left, the time under the pointer on a
/// seek bar. A row, so a live dot can sit in front of the number.
///
/// It lands on whatever the picture is doing, so it brings its own contrast:
/// the `overlay` wash, and text at full strength — the only tier that passes
/// on that wash over a white frame. There were three of these,
/// drawn by hand in three files at three different paddings, and two with a
/// white of their own rather than the app's.
pub fn badge() -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::GAP_TIGHT))
        .py(px(theme::BADGE_PAD_Y))
        .rounded(px(theme::RADIUS))
        .bg(theme::overlay())
        .text_size(px(theme::TEXT_META))
        .font_weight(theme::weight_label())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(theme::text())
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
/// page is in flight, or an ended pane's `Watch from the start` while its
/// channel's archives are being asked for.
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

    /// What is actually behind a variant's label, rather than its
    /// `background()`: the selected variant's fill is a wash, so the surface
    /// under it is what decides. The ones with no background of their own are
    /// measured against the worst thing they can land on: for `Quiet` the pane
    /// surface, for `Chrome` the title bar, and for `OnVideo` the brightest
    /// picture there is — a white frame under the bar's `video_chrome` wash.
    /// Not a black picture: light text on video fails on the light frames,
    /// never on the dark ones.
    ///
    /// A `match`, so a new variant does not compile until somebody has said
    /// where it sits. And it is measured as soon as it exists: the test walks
    /// `Variant::ALL`, which the enum's own declaration writes.
    fn sits_on(variant: Variant) -> gpui::Hsla {
        match variant {
            Variant::Pill | Variant::Primary => theme::surface_raised(),
            Variant::Selected | Variant::Quiet | Variant::Destructive | Variant::Chrome => {
                theme::surface()
            }
            Variant::OnVideo => theme::over_white(theme::video_chrome()),
        }
    }

    /// Every variant has to be legible on the surface it is drawn on.
    #[test]
    fn every_variant_is_legible_where_it_sits() {
        for &variant in Variant::ALL {
            let ratio = theme::contrast(variant.foreground(), sits_on(variant));
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "a {variant:?} label reads {ratio:.2}:1 on what it sits on"
            );
        }
    }

    /// A badge sits on a picture that could be anything, so it is held to the
    /// worst one: pure white under the wash, where the wash is at its lightest.
    #[test]
    fn a_badge_reads_on_the_brightest_picture() {
        let ratio = theme::contrast(theme::text(), theme::over_white(theme::overlay()));
        assert!(
            ratio >= theme::MIN_CONTRAST,
            "a badge reads {ratio:.2}:1 over a white picture"
        );
    }

    /// A pane header's × is a `Destructive` icon: on the chat panel, which
    /// `sits_on` measures, and with chat hidden on the band over the top of
    /// the picture, the bar's `video_chrome`. There its resting glyph is
    /// measured on the brightest picture, and its red under the pointer on
    /// the same with the hover wash over it, and the press's.
    #[test]
    fn the_pane_header_reads_on_its_band() {
        let band = theme::over_white(theme::video_chrome());
        let resting = theme::contrast(Variant::Destructive.foreground(), band);
        assert!(
            resting >= theme::MIN_CONTRAST,
            "a resting × reads {resting:.2}:1 on the band over a white picture"
        );
        for (state, fill) in [("hovered", theme::hover()), ("pressed", theme::pressed())] {
            let ratio = theme::contrast(Variant::Destructive.hover_foreground(), band.blend(fill));
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "a {state} × reads {ratio:.2}:1 on the band over a white picture"
            );
        }
    }

    /// A control with a fill of its own keeps it opaque under the pointer and
    /// while held. The washes are translucent so that they can lie over
    /// anything, and laid in place of a fill they let the picture under a
    /// card's pill show straight through it on hover.
    #[test]
    fn a_filled_control_stays_opaque_under_the_pointer() {
        for &variant in Variant::ALL {
            if variant.background().is_none() || variant == Variant::Selected {
                continue;
            }
            for (state, fill) in [
                ("hovered", variant.hover_background()),
                ("pressed", variant.pressed_background()),
            ] {
                assert_eq!(
                    fill.a, 1.0,
                    "a {state} {variant:?} control lets the picture through"
                );
            }
        }
    }

    /// An on-video control rests quiet and lifts to full text under the
    /// pointer, so the lifted label has to read on the bar with the hover wash
    /// on it, and with the press on it too: a press always comes with the
    /// pointer over the control, so a pressed label is a lifted one. Measured
    /// on the brightest picture, like the resting label.
    #[test]
    fn an_on_video_control_lifts_under_the_pointer() {
        let bar = theme::over_white(theme::video_chrome());
        let lifted = Variant::OnVideo.hover_foreground();
        for (state, fill) in [("hovered", theme::hover()), ("pressed", theme::pressed())] {
            let ratio = theme::contrast(lifted, bar.blend(fill));
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "a {state} on-video label reads {ratio:.2}:1 over a white picture"
            );
        }
        assert_ne!(
            Variant::OnVideo.foreground(),
            lifted,
            "an on-video label has to lift under the pointer, or the hover              wash is its only cue"
        );
    }
}
