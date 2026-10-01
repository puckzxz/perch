//! The control bar along the bottom of a playing picture: the seek row on a
//! recording, then the row of buttons every stream has.
//!
//! An `impl VideoView` of its own, so the bar can grow without the player
//! growing with it. It reads the player's fields directly, since a child
//! module sees its parent's private items. Its methods are private to this
//! file, though, so `control_bar`, the one `render` calls, is `pub(super)`.

use gpui::{div, prelude::*, px, Context, SharedString};
use gpui_component::slider::Slider;

use super::{Menu, VideoView};
use crate::controls;
use crate::seek_bar;
use crate::theme;

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

    pub(super) fn control_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            // Clicks on the bar stay on the bar. Hit-testing is flat, so
            // without this a click on `Pause` would also reach the pane
            // underneath, where a double-click now means fullscreen. The hover
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
            // than full-strength text: the resting labels and the volume
            // figure are `text_muted`, which only passes over a white frame
            // on this one.
            .bg(theme::video_chrome())
            .children(self.seek_row(cx))
            .child(self.button_row(cx))
    }

    /// Pause, mute, volume and quality: the row every stream has.
    fn button_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // The figure is the level chosen, beside the slider showing it; the
        // pill says what a press would do, which for a hushed pane is unmute.
        let volume = self.loudness.level();
        let muted = self.is_muted();
        let paused = self.stream.is_paused();

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            .child(
                controls::pill(
                    "pause",
                    if paused { "Play" } else { "Pause" },
                    controls::Variant::OnVideo,
                )
                .on_click(cx.listener(|this, _event, _window, cx| this.toggle_playback(cx))),
            )
            .child(
                controls::pill(
                    "mute",
                    if muted { "Unmute" } else { "Mute" },
                    controls::Variant::OnVideo,
                )
                .on_click(cx.listener(|this, _event, window, cx| this.toggle_mute(window, cx))),
            )
            .child(
                div()
                    .w(px(120.))
                    .child(Slider::new(&self.volume_slider).horizontal()),
            )
            .child(
                div()
                    .w(px(38.))
                    .text_size(px(theme::TEXT_META))
                    .text_right()
                    .text_color(theme::text_muted())
                    .child(SharedString::from(format!("{volume}%"))),
            )
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
                    // see `menu::menu_row`.
                    .when(self.menu.is_some(), |anchor| {
                        anchor.on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                            this.close_menu(cx);
                        }))
                    })
                    // A menu button toggles its own menu and closes nothing
                    // first: the press is inside the anchor, so the dismiss
                    // above does not hear it, and a close before the toggle
                    // would turn a second press on an open menu's button into
                    // a reopen.
                    .child(
                        controls::pill(
                            "quality",
                            self.qualities.playing.clone(),
                            controls::Variant::OnVideo,
                        )
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.toggle_menu(Menu::Quality, cx),
                        )),
                    )
                    .when_some(self.menu, |anchor, which| {
                        anchor.child(self.menu_box(which, cx))
                    }),
            )
    }
}
