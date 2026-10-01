//! The window's title bar, which Perch draws itself in place of the
//! platform's: the rail button, back and forward and the search box at the
//! left, a strip to drag the window by, the settings gear — with the sign-in
//! beside it while there is none — and on Windows the minimise, maximise and
//! close buttons.
//!
//! Drawn rather than left to the platform so the bar can carry the app's own
//! controls — a platform title bar holds a title and nothing else, and that
//! is a whole row of the window spent on the word `perch`. The cost is that
//! the bar has to do everything the platform's did, and most of what can go
//! wrong there goes wrong silently:
//!
//! - **The bar must `occlude`.** The root tracks focus, and its mouse-down
//!   handler marks every press it hears as handled. gpui reports a handled
//!   non-client press to Windows as done, so without the occluder in the way
//!   a drag, a double-click to maximise and the top resize edge would all
//!   quietly do nothing.
//! - **Only an empty spacer drags.** gpui answers a hit test with the first
//!   window-control area under the pointer in paint order, parent before
//!   child, so a drag area *around* the controls would answer for them: the
//!   gear would become a handle, and a caption button a caption. The spacer
//!   is a sibling of every control and never an ancestor of one.
//! - **The drag and caption areas start `layout::drag_top` down**, leaving the
//!   top edge to the platform's resize band while the window is windowed.
//! - **The caption buttons do nothing themselves**; see
//!   `controls::caption_button`.
//! - **The search box is never inside a drag area.** A press there is the
//!   platform's, and the moves that follow arrive as non-client moves with no
//!   button held, so dragging to select the text in the box would not work.
//!   Being a sibling of the spacer is not enough on its own: in a window too
//!   narrow for it the box would run on under the spacer, so its group clips
//!   it at the spacer's edge instead.
//! - **The window is never narrower than the bar.** The platform kept its
//!   caption buttons in reach at any width; this bar is a row that runs off
//!   the window's right edge, Close first, so [`window_min_size`] is the
//!   window's least size (`main`).
//!
//! On macOS the platform keeps its traffic lights and the bar leaves room for
//! them; gpui 0.2.2 drags a Mac window only by AppKit's own strip, so the
//! spacer asks for the platform's double-click behaviour by hand. Elsewhere
//! the window manager keeps its own decorations, and the bar sits under them
//! with only the app's controls in it.
//!
//! Which of those a platform gets is [`shape`], a pure function, so the macOS
//! and Linux answers are tested on the machine that cannot build for them.

use gpui::{
    div, prelude::*, px, size, AnyElement, Context, Pixels, SharedString, Size, Window,
    WindowControlArea,
};
use gpui_component::input::Input;

use super::RootView;
use crate::assets::Icon;
use crate::browse::SignIn;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::{layout, theme};

/// The platforms whose title bars differ — and, for `pop_out::offered`,
/// where a pop-out has been proven to stay on top. Taken from `cfg!` rather
/// than `#[cfg]` so every arm compiles on every platform: the Windows build
/// type-checks the macOS one, which is the closest thing to a macOS build
/// this machine can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Windows,
    MacOs,
    /// Linux and the rest, where the window manager draws the frame.
    Other,
}

impl Platform {
    pub(super) const CURRENT: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Other
    };
}

/// What a platform's title bar has to make room for, and do, besides the
/// app's own controls.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    /// How tall the bar is drawn: `layout::title_bar_height`, the same number
    /// the page's `Body` takes off the window, so the two cannot disagree.
    height: f32,
    /// Kept clear at the left for controls the platform draws itself: the
    /// traffic lights.
    leading_inset: f32,
    /// Whether Perch draws minimise, maximise and close itself.
    caption_buttons: bool,
    /// Whether a double-click on the spacer has to be handed to the platform
    /// by hand. On Windows it is the platform's already: the spacer answers
    /// as a caption, and Windows maximises a caption that is double-clicked.
    asks_for_double_click: bool,
}

/// The title bar this platform gets, or none at all in fullscreen — where the
/// picture is everything.
///
/// Whether there is a bar, and how tall, is not decided here: both come from
/// `layout::title_bar_height`, which is also what the page's `Body` takes off
/// the window. Decided twice, a change to one side would leave the watch grid
/// running off the bottom of the window or short of it.
fn shape(platform: Platform, fullscreen: bool) -> Option<Shape> {
    let height = layout::title_bar_height(fullscreen);
    if height <= 0.0 {
        return None;
    }
    Some(match platform {
        Platform::Windows => Shape {
            height,
            leading_inset: 0.0,
            caption_buttons: true,
            asks_for_double_click: false,
        },
        Platform::MacOs => Shape {
            height,
            leading_inset: theme::TRAFFIC_LIGHT_INSET,
            caption_buttons: false,
            asks_for_double_click: true,
        },
        Platform::Other => Shape {
            height,
            leading_inset: 0.0,
            caption_buttons: false,
            asks_for_double_click: false,
        },
    })
}

/// The narrowest the bar can be with nothing that must stay on it pushed off
/// its right end: the room kept for the traffic lights, the rail button and
/// back and forward at the left, the least of the drag strip, the gear with
/// its margin and the sign-in text's, and on Windows the three caption
/// buttons.
///
/// The bar is a plain row as wide as the window. In a window narrower than
/// this its last children ran off the right edge, Maximise and Close first,
/// where nothing can press them — the platform's own caption never let that
/// happen. The search box is not counted: it is the first thing the bar gives
/// up (see `title_bar_leading`), and a window this narrow has no use for it.
/// While the sign-in is said in the bar it shares the leading group's room,
/// so the arrows can be cut short then; the caption buttons cannot. Something
/// added to the bar that must stay on screen is added here too.
fn least_width(shape: Shape) -> f32 {
    // Padding either side of the three buttons, and the two gaps between them.
    let leading = 3.0 * theme::ICON_BUTTON + 4.0 * theme::GAP_TIGHT;
    let gear = theme::ICON_BUTTON + 2.0 * theme::GAP_TIGHT;
    let captions = if shape.caption_buttons {
        3.0 * theme::CAPTION_BUTTON_WIDTH
    } else {
        0.0
    };
    shape.leading_inset + leading + theme::TITLE_BAR_DRAG_MIN + gear + captions
}

/// The least the window may be made, for `WindowOptions::window_min_size`: as
/// wide as this platform's bar at its [`least_width`], and as tall as the bar
/// itself.
pub(crate) fn window_min_size() -> Size<Pixels> {
    let width = shape(Platform::CURRENT, false).map_or(0.0, least_width);
    size(px(width), px(layout::title_bar_height(false)))
}

impl RootView {
    /// The bar across the top of the window, or nothing in fullscreen.
    pub(super) fn title_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shape = shape(Platform::CURRENT, window.is_fullscreen())?;
        let maximized = window.is_maximized();
        let drag_top = layout::drag_top(maximized);

        // The id is there whatever the platform, because `on_click` needs a
        // stateful element and adding one inside `.when` would change the
        // element's type halfway through the chain.
        let spacer = div()
            .id("title-drag")
            .flex_1()
            .min_w(px(theme::TITLE_BAR_DRAG_MIN))
            .h_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(drag_top))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .window_control_area(WindowControlArea::Drag),
            )
            .when(shape.asks_for_double_click, |spacer| {
                spacer.on_click(|event, window, _cx| {
                    if event.click_count() == 2 {
                        window.titlebar_double_click();
                    }
                })
            });

        // The account control is the gear: who is signed in rides on its
        // tooltip rather than taking room in the bar, since once signed in it
        // is a fact nobody needs to read again.
        // The id follows whether there is a sign-in, as the words do, so a
        // sign-in landing while the pointer rests on the gear does not leave
        // the old words up; see `controls::tip`.
        let (settings_id, settings_tip): (&str, SharedString) = match &self.sign_in {
            SignIn::SignedIn(login) => (
                "title-settings-signed-in",
                format!(
                    "{} · signed in as {login}",
                    Hint::Settings.tooltip("Settings")
                )
                .into(),
            ),
            _ => ("title-settings", Hint::Settings.tooltip("Settings").into()),
        };
        let settings = controls::icon_button(settings_id, Icon::Settings, Variant::Chrome)
            .mr(px(theme::GAP_TIGHT))
            .tooltip(controls::tip(settings_tip))
            .on_click(cx.listener(|this, _event, window, cx| {
                // The bar is occluded, so the root's own mouse-down never
                // hears this press and never takes focus back from a text box
                // that had it. Taken back here, or the shortcuts would stay
                // with the box. Every handler in the bar starts this way.
                this.focus.focus(window);
                // Live even with a modal up. It closes the sheet it opened, as
                // `Ctrl+,` does, and with the palette up it puts the sheet in
                // the palette's place, as `Ctrl+,` does there too; see
                // `toggle_settings`.
                this.toggle_settings(window, cx);
            }));

        // Until there is a sign-in, what it is waiting for is said in the bar,
        // where it is on both pages: "Enter CODE at twitch.tv/activate" is
        // something to act on, on another device, and it has to stay up while
        // you do. Cut short rather than pushed out when the bar is narrow.
        let account = (!matches!(self.sign_in, SignIn::SignedIn(_))).then(|| {
            div()
                .min_w_0()
                .truncate()
                .mr(px(theme::GAP_TIGHT))
                .text_size(px(theme::TEXT_META))
                .text_color(theme::text_dim())
                .child(self.sign_in.summary())
        });

        let bar = div()
            .id("title-bar")
            // Mandatory; see the module.
            .occlude()
            .flex_none()
            .h(px(shape.height))
            .flex()
            .flex_row()
            .items_center()
            .bg(theme::surface())
            .border_b_1()
            .border_color(theme::border())
            .pl(px(shape.leading_inset))
            .child(self.title_bar_leading(cx))
            .child(spacer)
            .children(account)
            .child(settings)
            .when(shape.caption_buttons, |bar| {
                // The glyph follows the window, which repaints on every
                // change of bounds, maximising included.
                let max_glyph = if maximized {
                    Icon::Restore
                } else {
                    Icon::Maximize
                };
                // Read once for all three: gpui repaints when the pointer
                // leaves the window or comes back, and the buttons go dark
                // or lit with it; see `controls::caption_button`.
                let hovered = window.is_window_hovered();
                bar.child(controls::caption_button(
                    WindowControlArea::Min,
                    Icon::Minimize,
                    drag_top,
                    hovered,
                ))
                .child(controls::caption_button(
                    WindowControlArea::Max,
                    max_glyph,
                    drag_top,
                    hovered,
                ))
                .child(controls::caption_button(
                    WindowControlArea::Close,
                    Icon::Close,
                    drag_top,
                    hovered,
                ))
            });

        Some(bar.into_any_element())
    }

    /// The rail button, back and forward, and the search box: the page-level
    /// controls, at the left of the bar, before the drag strip.
    ///
    /// One group, so one veil covers them all while a modal is up: the sheet
    /// and the palette cover the page under the bar, and nothing in the bar
    /// should change that page behind them. The veil only stops the pointer;
    /// the keyboard is kept off the box by the sheet taking focus when it
    /// opens (`toggle_settings`) and by the palette's own box having it, and
    /// back and forward are bound on the pages and not in a modal. The gear
    /// and the caption buttons are outside the group and stay live, and so
    /// does the drag strip.
    fn title_bar_leading(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modal = self.modal_open();

        let rail = controls::icon_button("title-rail", Icon::Rail, Variant::Chrome)
            .tooltip(controls::tip(Hint::Rail.tooltip("Show or hide the rail")))
            .on_click(cx.listener(|this, _event, window, cx| {
                // See the gear's handler.
                this.focus.focus(window);
                this.toggle_sidebar(window, cx);
            }));

        // Back and forward, each drawn as waiting while it has nowhere to go:
        // the arrows stay where they are rather than the box moving under
        // the pointer, and a press on a waiting one does nothing because
        // there is nothing there to press.
        let back = if self.can_back() {
            controls::icon_button("title-back", Icon::Back, Variant::Chrome)
                .tooltip(controls::tip(Hint::Back.tooltip("Back")))
                .on_click(cx.listener(|this, _event, window, cx| {
                    // See the gear's handler.
                    this.focus.focus(window);
                    this.go_back(window, cx);
                }))
                .into_any_element()
        } else {
            controls::icon_waiting(Icon::Back).into_any_element()
        };
        let forward = if self.can_forward() {
            controls::icon_button("title-forward", Icon::Forward, Variant::Chrome)
                .tooltip(controls::tip(Hint::Forward.tooltip("Forward")))
                .on_click(cx.listener(|this, _event, window, cx| {
                    // See the gear's handler.
                    this.focus.focus(window);
                    this.go_forward(window, cx);
                }))
                .into_any_element()
        } else {
            controls::icon_waiting(Icon::Forward).into_any_element()
        };

        // Its own width while the bar has room. In a narrow window it gives
        // some of that up, down to `SEARCH_MIN_WIDTH`, rather than the drag
        // strip giving up all of its own: the strip is a flex-grown spacer
        // that keeps `TITLE_BAR_DRAG_MIN`. A width rather than growing into
        // the bar, because whatever the box grew into and did not draw on
        // would be neither box nor drag strip — a dead patch of bar.
        let search = div()
            .w(px(theme::SEARCH_WIDTH))
            .min_w(px(theme::SEARCH_MIN_WIDTH))
            .child(Input::new(&self.search).cleanable(true));

        div()
            .relative()
            // The group shrinks with the bar, down to nothing if it has to,
            // and clips what no longer fits. Narrower than the rail button
            // and the box's least width together, the box is cut off at its
            // right end, where the group meets the drag strip. Unclipped, it
            // ran on under the strip: the platform's hit test answered there
            // as a caption, so drag-selecting its text broke, and in a narrow
            // enough window the box covered the whole strip and then the
            // minimise button, and pressing either only focused the box. The
            // clip bounds hit testing as well as painting. It is the group
            // that gives way rather than the sign-in text beside the gear,
            // which shrinks alongside it: that text is up only while nobody
            // is signed in, when the box cannot search anyway.
            .min_w_0()
            .overflow_hidden()
            // The bar's full height, and a gap at the right, so the clip
            // leaves room for the shadow the box casts past its edges.
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .px(px(theme::GAP_TIGHT))
            .child(rail)
            .child(back)
            .child(forward)
            .child(search)
            // Last, so it is over them all. It takes the pointer without
            // doing anything with it; the modal is what to answer.
            .when(modal, |group| {
                group.child(div().absolute().inset_0().occlude())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLATFORMS: [Platform; 3] = [Platform::Windows, Platform::MacOs, Platform::Other];

    #[test]
    fn windows_draws_its_own_caption_buttons() {
        let shape = shape(Platform::Windows, false).expect("a windowed bar");
        assert!(shape.caption_buttons);
        assert!(
            !shape.asks_for_double_click,
            "Windows maximises a double-clicked caption itself"
        );
        assert_eq!(shape.leading_inset, 0.0);
    }

    #[test]
    fn macos_leaves_room_for_the_traffic_lights_and_asks_for_double_click() {
        let shape = shape(Platform::MacOs, false).expect("a windowed bar");
        assert!(
            !shape.caption_buttons,
            "the traffic lights are the platform's"
        );
        assert!(shape.asks_for_double_click);
        assert_eq!(shape.leading_inset, theme::TRAFFIC_LIGHT_INSET);
        assert!(
            shape.leading_inset > theme::TRAFFIC_LIGHT_X,
            "the inset has to start past the lights, not at them"
        );
    }

    #[test]
    fn other_platforms_draw_neither() {
        let shape = shape(Platform::Other, false).expect("a windowed bar");
        assert!(!shape.caption_buttons, "the window manager draws its own");
        assert!(!shape.asks_for_double_click);
        assert_eq!(shape.leading_inset, 0.0);
    }

    #[test]
    fn fullscreen_has_no_title_bar() {
        for platform in PLATFORMS {
            assert_eq!(shape(platform, true), None, "{platform:?} kept its bar");
        }
    }

    /// The window cannot be made too narrow for its own bar. On every
    /// platform the least width holds the drag strip's least and the gear;
    /// Windows adds exactly its three caption buttons, and macOS the room for
    /// its traffic lights. And it cannot be made shorter than the bar.
    #[test]
    fn the_window_is_never_narrower_than_its_title_bar() {
        let width = |platform| least_width(shape(platform, false).expect("a windowed bar"));
        let plain = width(Platform::Other);
        assert!(plain >= theme::TITLE_BAR_DRAG_MIN + theme::ICON_BUTTON);
        assert_eq!(
            width(Platform::Windows) - plain,
            3.0 * theme::CAPTION_BUTTON_WIDTH,
            "Maximise and Close fall off a window narrower than this"
        );
        assert_eq!(width(Platform::MacOs) - plain, theme::TRAFFIC_LIGHT_INSET);

        let least = window_min_size();
        assert_eq!(least.width, px(width(Platform::CURRENT)));
        assert_eq!(least.height, px(theme::TITLE_BAR_HEIGHT));
    }

    /// The bar draws exactly what the page gives up for it, on every platform,
    /// windowed and fullscreen: the height the bar is drawn at and the height
    /// `Body` takes off the window are one number.
    #[test]
    fn the_bar_is_as_tall_as_the_room_the_page_leaves_it() {
        for platform in PLATFORMS {
            for fullscreen in [false, true] {
                let drawn = shape(platform, fullscreen).map_or(0.0, |shape| shape.height);
                assert_eq!(
                    drawn,
                    layout::title_bar_height(fullscreen),
                    "{platform:?} draws a {drawn}px bar with fullscreen {fullscreen}"
                );
            }
        }
    }
}
