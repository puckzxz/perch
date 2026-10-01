//! The control bar along the bottom of a playing picture: the seek row on a
//! recording, then the row of icons every stream has.
//!
//! An `impl VideoView` of its own, so the bar can grow without the player
//! growing with it. It reads the player's fields directly, since a child
//! module sees its parent's private items. Its methods are private to this
//! file, though, so `control_bar`, the one `render` calls, is `pub(super)`.
//!
//! Each control but the quality is an icon — Pause while the pane plays, the
//! speaker crossed out while it is silent, chat crossed out while it is
//! hidden — with a tooltip that says what a press does, and names the key
//! that does the same where there is one (`keys::Hint`), so the bar teaches
//! the keys rather than standing in for them. More has no key, so its
//! tooltip is its name, and the still chat glyph of a recording with no
//! chat replay says why there is nothing to press. The quality pill is
//! words, saying what plays, with no tooltip and no key; the palette's
//! `Choose quality for …` is its keyboard path. Three rules hold for every
//! icon, through the builders here ([`bar_icon`], [`act_button`],
//! [`menu_button`]):
//!
//! - **A tooltip only while the pointer is in the window.** gpui hears the
//!   pointer leave as a flag, never a move, so a tooltip up at the time would
//!   stay where the pointer was last seen; dropping the builder clears it
//!   (div.rs:1660-1668).
//! - **An id keyed on the state the words follow** — `bar-pause` and
//!   `bar-play` — so a key that flips the state drops a tooltip already up
//!   with the old words; see `controls::tip`.
//! - **No control takes the keys back itself.** The bar does that for every
//!   press on it, in the capture phase (`control_bar`).
//!
//! What fits at the pane's width is [`fit`]: a narrow pane drops the volume
//! figure, then the slider, then folds the quality pill into More, and never
//! drops play, volume or a button of the right-hand cluster.

use gpui::{
    div, prelude::*, px, AnyElement, Context, Div, MouseButton, SharedString, Stateful, Window,
};
use gpui_component::slider::Slider;

use super::{ChatButton, Menu, VideoEvent, VideoView};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::seek_bar;
use crate::theme;
use crate::watch::PaneAction;

/// The icon buttons in the bar's right-hand cluster: chat, fullscreen and
/// More. `button_row` lays them out as an array this long, so a control
/// cannot join the cluster without this count changing with it, and with it
/// what [`fit`] leaves room for. The quality pill is not one of them: it is
/// words, and it folds.
pub(super) const RIGHT_BUTTONS: usize = 3;

impl VideoView {
    /// The seek bar, on a recording. A live stream has no timeline and gets
    /// nothing here, so its control bar is exactly what it was.
    fn seek_row(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let timeline = self.stream.timeline()?;
        let position = self.stream.position();
        let extent = timeline.extent(position);
        // While the thumb is held the left-hand time follows it rather than
        // the playhead: that is the number the scrub is choosing.
        let shown = self
            .scrub
            .map(|fraction| fraction as f64 * extent)
            .unwrap_or(position);
        // Mid-scrub the label rides the thumb, whatever the pointer has since
        // wandered over: the time being chosen is the one worth reading.
        let hover = self
            .scrub
            .or(self.pointing)
            .map(|fraction| seek_bar::Hover {
                fraction,
                time: seek_bar::timecode(fraction as f64 * extent).into(),
            });
        let state = seek_bar::State {
            played: (position / extent) as f32,
            scrub: self.scrub,
            hover,
            position: seek_bar::timecode(shown).into(),
            extent: seek_bar::timecode(extent).into(),
        };
        Some(seek_bar::element(
            state,
            |this: &mut Self, fraction, _window, cx| this.begin_scrub(fraction, cx),
            |this: &mut Self, _window, cx| this.end_scrub(cx),
            |this: &mut Self, bounds| this.track = Some(bounds),
            cx,
        ))
    }

    pub(super) fn control_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            // Clicks on the bar stay on the bar. Hit-testing is flat, so
            // without this a click on Pause would also reach the pane
            // underneath, where a double-click means fullscreen. The hover
            // probe is a canvas and sees through it, so the bar still counts
            // as "over the video" for the purpose of staying visible.
            //
            // Only while the bar is up. A hidden bar is `invisible`, which
            // skips its paint and its listeners but not its hitbox (gpui
            // inserts that in prepaint), so an occluder left on would go on
            // blocking the bottom of the picture with nothing drawn there. A
            // real pointer over the picture always has the bar up, so what
            // this keeps honest is synthetic input, and a press in the fade's
            // last moments.
            .when(self.controls.is_visible(), |bar| bar.occlude())
            // Whatever is pressed on the bar, the keys go back to the root;
            // see `return_keys`. In the capture phase, which runs before any
            // control's own handler: the volume slider's thumb stops its
            // press from bubbling, so a handler waiting for the bubble would
            // never hear a drag of it.
            .capture_any_mouse_down(cx.listener(Self::return_keys))
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .px(px(theme::PANEL_PAD))
            .py(px(theme::GAP_TIGHT))
            // Sits over live video, so it carries its own contrast rather than
            // relying on whatever happens to be on screen behind it. The
            // denser of the two picture washes, because the bar carries more
            // than full-strength text: the resting glyphs, the quality pill
            // and the volume figure are `text_muted`, which only passes over
            // a white frame on this one.
            .bg(theme::video_chrome())
            .children(self.seek_row(cx))
            .child(self.button_row(window, cx))
    }

    /// Play, volume, then the right-hand cluster — quality, chat, fullscreen
    /// and More: the row every stream has.
    fn button_row(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The figure is the level chosen, beside the slider showing it; the
        // speaker says whether the pane can be heard, which a pane held
        // silent by Mute all cannot, whatever its level.
        let volume = self.loudness.level();
        let window_hovered = window.is_window_hovered();

        let (id, icon, words) = if self.stream.is_paused() {
            ("bar-play", Icon::Play, "Play")
        } else {
            ("bar-pause", Icon::Pause, "Pause")
        };
        let playback = act_button(
            id,
            icon,
            Hint::Playback.tooltip(words),
            window,
            cx,
            |this, _window, cx| this.toggle_playback(cx),
        );

        let (id, icon, words) = if self.is_muted() {
            ("bar-unmute", Icon::VolumeOff, "Unmute")
        } else {
            ("bar-mute", Icon::Volume, "Mute")
        };
        let mute = act_button(
            id,
            icon,
            Hint::Mute.tooltip(words),
            window,
            cx,
            |this, window, cx| this.toggle_mute(window, cx),
        );

        // In a wrapper of its own, which is what carries the tooltip: the
        // slider is the library's, and takes none. gpui shows no tooltip
        // while something is being dragged, the thumb included.
        let slider = self.fit.slider.then(|| {
            div()
                .id("bar-volume")
                .flex_none()
                .w(px(theme::VOLUME_SLIDER))
                .when(window_hovered, |slider| {
                    slider.tooltip(controls::tip(Hint::Volume.tooltip("Volume")))
                })
                .child(Slider::new(&self.volume_slider).horizontal())
        });
        let figure = self.fit.figure.then(|| {
            div()
                .flex_none()
                .w(px(theme::VOLUME_FIGURE))
                .text_size(px(theme::TEXT_META))
                .text_right()
                .text_color(theme::text_muted())
                .child(SharedString::from(format!("{volume}%")))
        });

        // The pill says what is playing, and opens the menu to change it.
        // Words rather than an icon, so it is the first thing to go when
        // the bar runs out of room: More then carries it as its first row.
        let quality = self.fit.quality.then(|| {
            controls::pill("quality", self.qualities.playing.clone(), Variant::OnVideo).on_click(
                cx.listener(|this, _event, _window, cx| this.toggle_menu(Menu::Quality, cx)),
            )
        });

        let chat = match self.chat {
            ChatButton::Shown => act_button(
                "bar-chat-hide",
                Icon::Chat,
                Hint::Chat.tooltip("Hide chat"),
                window,
                cx,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::ToggleChat)),
            )
            .into_any_element(),
            ChatButton::Hidden => act_button(
                "bar-chat-show",
                Icon::ChatOff,
                Hint::Chat.tooltip("Show chat"),
                window,
                cx,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::ToggleChat)),
            )
            .into_any_element(),
            // Still, rather than gone: the cluster keeps its shape, and the
            // tooltip says why there is nothing to press. `C` says the same,
            // in a toast. A press on it still closes an open menu, as a press
            // anywhere else on the bar does: it is inside the menus' anchor,
            // so the anchor's dismiss never hears it (see `act_button`).
            ChatButton::Unavailable => div()
                .id("bar-chat-none")
                .flex_none()
                .when(window_hovered, |still| {
                    still.tooltip(controls::tip("No chat replay"))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, _window, cx| {
                        this.close_menu(cx);
                    }),
                )
                .child(controls::icon_waiting(Icon::ChatOff))
                .into_any_element(),
        };

        let (id, icon, words) = if window.is_fullscreen() {
            (
                "bar-exit-fullscreen",
                Icon::FullscreenExit,
                "Exit fullscreen",
            )
        } else {
            ("bar-fullscreen", Icon::Fullscreen, "Fullscreen")
        };
        let fullscreen = act_button(
            id,
            icon,
            Hint::Fullscreen.tooltip(words),
            window,
            cx,
            // The window's, as `F` is; see `RootView::on_toggle_fullscreen`.
            |_this, window, _cx| window.toggle_fullscreen(),
        );

        let more = menu_button(
            "bar-more",
            Icon::More,
            "More".to_string(),
            Menu::More,
            window,
            cx,
        );

        let buttons: [AnyElement; RIGHT_BUTTONS] = [
            chat,
            // phase 4: guide
            // phase 3: maximize in window, pop out
            fullscreen.into_any_element(),
            more.into_any_element(),
        ];

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .child(playback)
            .child(mute)
            .children(slider)
            .children(figure)
            .child(div().flex_1())
            .child(
                // The right-hand cluster, and the one anchor every menu opens
                // from. Every control that joins the bar's right-hand end
                // joins this cluster, so a press on one is never "elsewhere".
                div()
                    .relative()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    // A press anywhere else closes the menu, the way every
                    // menu does. On the anchor rather than the menu, so a
                    // press on the button is not "elsewhere": that would
                    // close the menu, and the click that followed would open
                    // it straight again. The menu itself hangs outside the
                    // anchor's bounds, so a press on one of its rows counts
                    // as elsewhere too, which is why rows act on the press;
                    // see `menu::menu_row`. What is inside the anchor closes
                    // the menu itself, or toggles it: `act_button`,
                    // `menu_button`, the pill and the still chat glyph. Only
                    // the narrow gaps between them close nothing.
                    .when(self.menu.is_some(), |anchor| {
                        anchor.on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                            this.close_menu(cx);
                        }))
                    })
                    .children(quality)
                    .children(buttons)
                    .when_some(self.menu, |anchor, which| {
                        anchor.child(self.menu_box(which, cx))
                    }),
            )
    }
}

/// An icon on the bar: an on-video [`controls::icon_button`], which lifts
/// under the pointer, and after the tooltip delay says `tip` — while the
/// pointer is in the window; see the module. `id` is keyed on whatever
/// state `tip` follows, for the same reason.
fn bar_icon(id: &'static str, icon: Icon, tip: String, window: &Window) -> Stateful<Div> {
    controls::icon_button(id, icon, Variant::OnVideo).when(window.is_window_hovered(), |button| {
        button.tooltip(controls::tip(tip))
    })
}

/// An icon that does something: closes whichever menu is open, then runs
/// `act`. A control in the right-hand cluster is inside the menus' anchor,
/// so the anchor's dismiss never hears its press, and the menu would stay
/// open over whatever the press changed. Elsewhere on the bar the dismiss
/// has closed it already, and this finds nothing to close.
fn act_button(
    id: &'static str,
    icon: Icon,
    tip: String,
    window: &Window,
    cx: &mut Context<VideoView>,
    act: impl Fn(&mut VideoView, &mut Window, &mut Context<VideoView>) + 'static,
) -> Stateful<Div> {
    bar_icon(id, icon, tip, window).on_click(cx.listener(move |this, _event, window, cx| {
        this.close_menu(cx);
        act(this, window, cx);
    }))
}

/// An icon that opens `which`, or closes it when it is the menu open. Closes
/// nothing first, unlike [`act_button`]: a second press on an open menu's
/// button would close it and then open it again, and a press while the
/// other menu is open swaps one for the other (`menu::toggled`). The press is
/// inside the anchor, so the dismiss does not hear it either.
fn menu_button(
    id: &'static str,
    icon: Icon,
    tip: String,
    which: Menu,
    window: &Window,
    cx: &mut Context<VideoView>,
) -> Stateful<Div> {
    bar_icon(id, icon, tip, window)
        .on_click(cx.listener(move |this, _event, _window, cx| this.toggle_menu(which, cx)))
}

/// What the bar has room for, beyond what it always keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fit {
    /// The volume figure beside the slider.
    pub(super) figure: bool,
    /// The volume slider. The speaker and `↑`/`↓` still set the level.
    pub(super) slider: bool,
    /// The quality pill. Without it, More's first row is the quality.
    pub(super) quality: bool,
}

impl Fit {
    /// Everything: what a bar not yet measured assumes.
    pub(super) const EVERYTHING: Fit = Fit {
        figure: true,
        slider: true,
        quality: true,
    };
}

/// The bar at each width it gives way at, widest first: each drops one more
/// thing, in the order they go. The figure first, because the slider shows
/// the same level; then the slider, since the speaker and the keys still
/// change it; then the quality pill, which More can carry.
const NARROWINGS: [Fit; 4] = [
    Fit::EVERYTHING,
    Fit {
        figure: false,
        ..Fit::EVERYTHING
    },
    Fit {
        figure: false,
        slider: false,
        quality: true,
    },
    Fit {
        figure: false,
        slider: false,
        quality: false,
    },
];

/// How wide the bar has to be to hold `fit`, with `right_buttons` icon
/// buttons in its right-hand cluster: its padding, play and volume, the
/// spacer between the two ends, the cluster's buttons, and whatever `fit`
/// adds, each with the gap before it. The sizes are the ones `button_row`
/// draws with, from `theme`; the quality pill's is an estimate
/// (`theme::QUALITY_PILL_ROOM`).
fn needs(fit: Fit, right_buttons: usize) -> f32 {
    let gap = theme::GAP_TIGHT;
    let shown = |kept: bool, width: f32| if kept { width + gap } else { 0.0 };
    let buttons = right_buttons as f32;
    2.0 * theme::PANEL_PAD
        // Play and volume.
        + 2.0 * theme::ICON_BUTTON
        + gap
        // Either side of the spacer.
        + 2.0 * gap
        + buttons * theme::ICON_BUTTON
        + (buttons - 1.0).max(0.0) * gap
        + shown(fit.slider, theme::VOLUME_SLIDER)
        + shown(fit.figure, theme::VOLUME_FIGURE)
        + shown(fit.quality, theme::QUALITY_PILL_ROOM)
}

/// What fits on a bar `width` logical pixels wide with `right_buttons` icon
/// buttons in its right-hand cluster: the widest of [`NARROWINGS`] that does,
/// or the narrowest when none does. Narrower than that the bar still holds
/// play, volume and the cluster, and runs past the pane's edge — a beside
/// pane at its narrowest, which is a known limit.
pub(super) fn fit(width: f32, right_buttons: usize) -> Fit {
    NARROWINGS
        .into_iter()
        .find(|fit| needs(*fit, right_buttons) <= width)
        .unwrap_or(NARROWINGS[NARROWINGS.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How far through giving way a bar is: an index into [`NARROWINGS`].
    fn rank(fit: Fit) -> usize {
        NARROWINGS
            .iter()
            .position(|narrowing| *narrowing == fit)
            .expect("fit only ever answers one of the narrowings")
    }

    /// Every whole width from nothing to a 4K window.
    fn widths() -> impl DoubleEndedIterator<Item = f32> {
        (0..=3840).map(|width| width as f32)
    }

    #[test]
    fn a_wide_bar_shows_everything() {
        assert_eq!(fit(1600.0, RIGHT_BUTTONS), Fit::EVERYTHING);
        assert_eq!(
            fit(needs(Fit::EVERYTHING, RIGHT_BUTTONS), RIGHT_BUTTONS),
            Fit::EVERYTHING,
            "a bar exactly wide enough holds it all"
        );
    }

    /// Narrowing gives things up one at a time and in order — the figure,
    /// then the slider — and never takes one back as it goes on narrowing.
    #[test]
    fn narrowing_drops_the_figure_then_the_slider() {
        let mut last = 0;
        for width in widths().rev() {
            let fit = fit(width, RIGHT_BUTTONS);
            assert!(
                rank(fit) >= last,
                "at {width}px the bar took something back"
            );
            last = rank(fit);
            assert!(
                !fit.figure || fit.slider,
                "at {width}px the figure stayed after the slider it labels went"
            );
        }
        let seen: Vec<Fit> = widths().map(|width| fit(width, RIGHT_BUTTONS)).collect();
        for narrowing in NARROWINGS {
            assert!(
                seen.contains(&narrowing),
                "no width gives {narrowing:?}, so a step is skipped"
            );
        }
    }

    /// The quality pill is the last thing to go: it folds only once the
    /// slider has gone, and with it into More.
    #[test]
    fn the_quality_pill_folds_after_the_slider() {
        for width in widths() {
            let fit = fit(width, RIGHT_BUTTONS);
            assert!(
                !fit.slider || fit.quality,
                "at {width}px the pill folded with the slider still up"
            );
        }
        assert!(
            widths().any(|width| {
                let fit = fit(width, RIGHT_BUTTONS);
                fit.quality && !fit.slider
            }),
            "the pill and the slider went at the same width"
        );
    }

    /// However narrow, nothing past the narrowest can go: `Fit` has no way
    /// to say play, volume or a right-hand button is not there. And the
    /// narrowest bar is counted as holding exactly those, inside its padding.
    #[test]
    fn play_volume_and_the_right_cluster_always_stay() {
        let narrowest = NARROWINGS[NARROWINGS.len() - 1];
        assert_eq!(fit(0.0, RIGHT_BUTTONS), narrowest);
        assert_eq!(
            narrowest,
            Fit {
                figure: false,
                slider: false,
                quality: false,
            }
        );
        let squares = 2.0 + RIGHT_BUTTONS as f32;
        assert!(
            needs(narrowest, RIGHT_BUTTONS)
                >= 2.0 * theme::PANEL_PAD + squares * theme::ICON_BUTTON
        );
    }

    /// A control added to the right-hand cluster — the guide, the pop-out —
    /// takes its room from what folds: at any width the bar gives up at
    /// least as much as it did, and somewhere strictly more.
    #[test]
    fn more_on_the_right_narrows_sooner() {
        for width in widths() {
            assert!(
                rank(fit(width, RIGHT_BUTTONS + 1)) >= rank(fit(width, RIGHT_BUTTONS)),
                "at {width}px another button gave the bar more room"
            );
        }
        assert!(widths().any(|width| fit(width, RIGHT_BUTTONS + 1) != fit(width, RIGHT_BUTTONS)));
    }
}
