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
//! `Choose quality for …` is its keyboard path. While a quality somebody
//! picked is under way it names that one instead and breathes, and only
//! then has a tooltip, `Switching to 480p30` ([`pill_face`]); the bar stays
//! up meanwhile and a moment after (`VideoView::sync_controls`). Three rules
//! hold for every icon, through the builders here ([`bar_icon`],
//! [`act_button`], [`menu_button`]), and the first two for the pill's
//! tooltip too:
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
//! figure, then the slider, then folds the quality pill into More, then the
//! maximize control after it, and never drops play, volume or a button of
//! the right-hand cluster.
//!
//! The maximize control gives the pane the whole watch page, chat and all,
//! and on the pane that has it shows every pane again (`stage`'s
//! `MaximizeButton`, mirrored as `VideoView::maximize`). It stands just
//! before the cluster, after the pill, and is not one of the cluster's
//! buttons: it folds, where they never drop, so a narrow pane's bar runs no
//! further past its edge for it than it did before there was one. With
//! fewer than two panes it is not drawn and takes no room.
//!
//! A pane popped into a window of its own has a bar of its own shape: the
//! seek row and the left-hand end as a pane has them, and at the right only
//! Bring back and Close. No quality, chat, fullscreen or More — the pane's
//! chat and its menus stay with its cell in the main window, and the window
//! is too small to want them.

use gpui::{
    div, prelude::*, px, AnyElement, Context, Div, MouseButton, SharedString, Stateful, Window,
};
use gpui_component::slider::Slider;

use super::{ChatButton, Menu, Switching, VideoEvent, VideoView};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::motion;
use crate::seek_bar;
use crate::stage::{MaximizeButton, Place};
use crate::theme;
use crate::watch::PaneAction;

/// The icon buttons in a pane's right-hand cluster: chat, fullscreen and
/// More. `button_row` lays them out as an array this long, so a control
/// cannot join the cluster without this count changing with it, and with it
/// what [`fit`] leaves room for. The quality pill is not one of them: it is
/// words, and it folds. Nor is the maximize control, which folds after it.
pub(super) const RIGHT_BUTTONS: usize = 3;

/// The same for a pop-out's cluster: Bring back and Close.
pub(super) const POP_OUT_BUTTONS: usize = 2;

/// The right-hand end of a bar, as [`fit`] makes room for it: how many icon
/// buttons it has, and whether the quality pill and the maximize control
/// stand before them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cluster {
    buttons: usize,
    pill: bool,
    maximize: bool,
}

/// A pane's, with nothing to maximize it over: the quality pill, then chat,
/// fullscreen and More.
const PANE: Cluster = Cluster {
    buttons: RIGHT_BUTTONS,
    pill: true,
    maximize: false,
};

/// A pop-out's: Bring back and Close, and no pill. Counting room for one
/// it never draws took the slider off a pop-out wide enough for it — the
/// width a vertical stream opens at among them. No maximize either: a pane
/// out of the main window is not on the page to fill it.
const POP_OUT: Cluster = Cluster {
    buttons: POP_OUT_BUTTONS,
    pill: false,
    maximize: false,
};

/// The right-hand end of the bar of a player in `place`, whose maximize
/// control offers `maximize`, for [`fit`]. A tile and a player offstage
/// draw no bar, and are counted as a pane, whose bar they would be.
pub(super) fn cluster(place: Place, maximize: MaximizeButton) -> Cluster {
    match place {
        Place::Pane | Place::Tile | Place::Offstage => Cluster {
            maximize: maximize != MaximizeButton::Hidden,
            ..PANE
        },
        Place::PopOut => POP_OUT,
    }
}

/// What the quality pill shows, with `playing` on screen and a pick
/// `switching` the pane, if one is under way: what plays, plainly, with no
/// tooltip — the pill is words, and says it already — or what the pick asked
/// for, breathing (`motion::waiting`, a state that always ends), with a
/// tooltip that says what is going on. Its id is keyed on the words the
/// tooltip follows, so one already up says no stale rendition after a second
/// pick or once the switch has ended (`controls::tip`).
pub(super) fn pill_face(playing: &SharedString, switching: Option<&Switching>) -> PillFace {
    match switching {
        None => PillFace {
            id: SharedString::from("quality"),
            label: playing.clone(),
            waiting: false,
            tip: None,
        },
        Some(switching) => PillFace {
            id: SharedString::from(format!("quality-switching-{}", switching.to)),
            label: switching.to.clone(),
            waiting: true,
            tip: Some(format!("Switching to {}", switching.to)),
        },
    }
}

/// The quality pill as it is drawn; see [`pill_face`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PillFace {
    pub(super) id: SharedString,
    pub(super) label: SharedString,
    /// Whether it breathes.
    pub(super) waiting: bool,
    pub(super) tip: Option<String>,
}

/// What the maximize control is while it offers `button`: its ids on the
/// bar and in More — keyed on what it offers, so a press that flips it
/// drops a tooltip still up with the old words (`controls::tip`) — its
/// icon, and its words. `None` while it is hidden. One answer for the bar
/// and More's row, so the two can never offer different things.
pub(super) fn maximize_control(button: MaximizeButton) -> Option<MaximizeControl> {
    match button {
        MaximizeButton::Hidden => None,
        MaximizeButton::Maximize => Some(MaximizeControl {
            bar_id: "bar-maximize",
            row_id: "more-maximize",
            icon: Icon::PaneMaximize,
            words: "Maximize",
        }),
        MaximizeButton::Restore => Some(MaximizeControl {
            bar_id: "bar-show-all",
            row_id: "more-show-all",
            icon: Icon::PaneGrid,
            words: "Show all panes",
        }),
    }
}

/// The maximize control as it is drawn; see [`maximize_control`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MaximizeControl {
    pub(super) bar_id: &'static str,
    pub(super) row_id: &'static str,
    pub(super) icon: Icon,
    pub(super) words: &'static str,
}

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

    /// Play, volume, then the right-hand cluster — a pane's quality, chat,
    /// fullscreen and More, or a pop-out's Bring back and Close: the row
    /// every stream has.
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
            Variant::OnVideo,
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
            Variant::OnVideo,
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

        let right = match self.place {
            Place::Pane | Place::Tile | Place::Offstage => {
                self.pane_cluster(window, cx).into_any_element()
            }
            Place::PopOut => pop_out_cluster(window, cx).into_any_element(),
        };

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
            .child(right)
    }

    /// A pane's right-hand cluster: the quality pill, the maximize control,
    /// chat, fullscreen and More, and the one anchor every menu opens from.
    fn pane_cluster(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_hovered = window.is_window_hovered();

        // The pill says what is playing, and opens the menu to change it.
        // Words rather than an icon, so it is the first thing to go when
        // the bar runs out of room: More then carries it as its first row.
        // While a pick is under way it names that instead, and breathes
        // (`pill_face`), in the same room (`theme::QUALITY_PILL_ROOM`).
        let quality = self.fit.quality.then(|| {
            let face = pill_face(&self.qualities.playing, self.switching.as_ref());
            let pill = controls::pill(face.id, face.label, Variant::OnVideo)
                // Breathing, in the lifted words a pointer would give it:
                // the resting ones only just read over a white frame, and at
                // the bottom of a breath would not
                // (`controls::a_breathing_on_video_label_still_reads`).
                .when(face.waiting, |pill| pill.text_color(theme::text()))
                .when_some(face.tip.filter(|_| window_hovered), |pill, tip| {
                    pill.tooltip(controls::tip(tip))
                })
                .on_click(
                    cx.listener(|this, _event, _window, cx| this.toggle_menu(Menu::Quality, cx)),
                );
            if face.waiting {
                motion::waiting("quality-switching", pill).into_any_element()
            } else {
                pill.into_any_element()
            }
        });

        let chat = match self.chat {
            ChatButton::Shown => act_button(
                "bar-chat-hide",
                Icon::Chat,
                Variant::OnVideo,
                Hint::Chat.tooltip("Hide chat"),
                window,
                cx,
                |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::ToggleChat)),
            )
            .into_any_element(),
            ChatButton::Hidden => act_button(
                "bar-chat-show",
                Icon::ChatOff,
                Variant::OnVideo,
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
            Variant::OnVideo,
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
        // With the pill folded into More, More is where a switch under way
        // is said (`menu::folded_quality`), so it breathes meanwhile: the
        // bar held up for the switch has something on it that says why.
        let more = if !self.fit.quality && self.switching.is_some() {
            motion::waiting("more-switching", more).into_any_element()
        } else {
            more.into_any_element()
        };

        // The pane given the watch page, or every pane back. After the pill
        // and before the buttons that never drop, since it folds into More
        // right after the pill does: the two that give way stand together,
        // and folding either moves none of the three. A press takes the
        // rest of its run, as a menu row's does (`run_guard`): the pane
        // grows, or the grid comes back, under the pointer, and the second
        // press of a double-click would land on whatever is there now — a
        // picture, whose double-click is fullscreen.
        let maximize = maximize_control(self.maximize)
            .filter(|_| self.fit.maximize)
            .map(|control| {
                act_button(
                    control.bar_id,
                    control.icon,
                    Variant::OnVideo,
                    Hint::Maximize.tooltip(control.words),
                    window,
                    cx,
                    |this, _window, cx| {
                        this.row_run = true;
                        cx.emit(VideoEvent::Pane(PaneAction::Maximize));
                    },
                )
            });

        let buttons: [AnyElement; RIGHT_BUTTONS] = [
            chat,
            // phase 4: guide. Pop out is not here: it is the header's and
            // More's, and this cluster never drops a button.
            fullscreen.into_any_element(),
            more,
        ];

        // The right-hand cluster, and the one anchor every menu opens from.
        // Every control that joins the bar's right-hand end joins this
        // cluster, so a press on one is never "elsewhere".
        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT))
            // A press anywhere else closes the menu, the way every menu does.
            // On the anchor rather than the menu, so a press on the button is
            // not "elsewhere": that would close the menu, and the click that
            // followed would open it straight again. The menu itself hangs
            // outside the anchor's bounds, so a press on one of its rows
            // counts as elsewhere too, which is why rows act on the press;
            // see `menu::menu_row`. What is inside the anchor closes the menu
            // itself, or toggles it: `act_button`, `menu_button`, the pill and
            // the still chat glyph. Only the narrow gaps between them close
            // nothing.
            .when(self.menu.is_some(), |anchor| {
                anchor.on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                    this.close_menu(cx);
                }))
            })
            .children(quality)
            .children(maximize)
            .children(buttons)
            .when_some(self.menu, |anchor, which| {
                anchor.child(self.menu_box(which, cx))
            })
    }
}

/// A pop-out's right-hand cluster: the way back, then the way out. Both go
/// up to the root as the pane's own (`VideoEvent::Pane`), which answers them
/// with the main window, the way it answers the pane's header. Close is red
/// as the header's × is, and names the same key: `Ctrl+W` closes the pane
/// in the pop-out too. The window's own close — Alt+F4, the taskbar — brings
/// the pane back instead, so nothing is lost to a reflex.
fn pop_out_cluster(window: &Window, cx: &mut Context<VideoView>) -> impl IntoElement {
    let buttons: [AnyElement; POP_OUT_BUTTONS] = [
        act_button(
            "bar-pop-in",
            Icon::PopIn,
            Variant::OnVideo,
            Hint::PopOut.tooltip("Bring back"),
            window,
            cx,
            |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::PopIn)),
        )
        .into_any_element(),
        act_button(
            "bar-close",
            Icon::Close,
            Variant::Destructive,
            Hint::Close.tooltip("Close"),
            window,
            cx,
            |_this, _window, cx| cx.emit(VideoEvent::Pane(PaneAction::Close)),
        )
        .into_any_element(),
    ];
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .children(buttons)
}

/// An icon on the bar: a [`controls::icon_button`] in `variant` — on-video,
/// which lifts under the pointer, or red for a close — that after the
/// tooltip delay says `tip`, while the pointer is in the window; see the
/// module. `id` is keyed on whatever state `tip` follows, for the same
/// reason.
fn bar_icon(
    id: &'static str,
    icon: Icon,
    variant: Variant,
    tip: String,
    window: &Window,
) -> Stateful<Div> {
    controls::icon_button(id, icon, variant).when(window.is_window_hovered(), |button| {
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
    variant: Variant,
    tip: String,
    window: &Window,
    cx: &mut Context<VideoView>,
    act: impl Fn(&mut VideoView, &mut Window, &mut Context<VideoView>) + 'static,
) -> Stateful<Div> {
    bar_icon(id, icon, variant, tip, window).on_click(cx.listener(
        move |this, _event, window, cx| {
            this.close_menu(cx);
            act(this, window, cx);
        },
    ))
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
    bar_icon(id, icon, Variant::OnVideo, tip, window)
        .on_click(cx.listener(move |this, _event, _window, cx| this.toggle_menu(which, cx)))
}

/// What the bar has room for, beyond what it always keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fit {
    /// The volume figure beside the slider.
    pub(super) figure: bool,
    /// The volume slider. The speaker and `↑`/`↓` still set the level.
    pub(super) slider: bool,
    /// The quality pill. Without it, More's first row is the quality. Read
    /// only by a pane's cluster: a pop-out has neither pill nor More, and
    /// [`fit`] counts no room for the pill there.
    pub(super) quality: bool,
    /// The maximize control, where the cluster has one. Without it, More
    /// carries it, after the quality. Read only while the control is
    /// offered: a cluster with none counts no room for it, so this says
    /// nothing there.
    pub(super) maximize: bool,
}

impl Fit {
    /// Everything: what a bar not yet measured assumes.
    pub(super) const EVERYTHING: Fit = Fit {
        figure: true,
        slider: true,
        quality: true,
        maximize: true,
    };
}

/// The bar at each width it gives way at, widest first: each drops one more
/// thing, in the order they go. The figure first, because the slider shows
/// the same level; then the slider, since the speaker and the keys still
/// change it; then the quality pill, which More can carry; then the
/// maximize control, which More carries after it, and `Z` does too.
const NARROWINGS: [Fit; 5] = [
    Fit::EVERYTHING,
    Fit {
        figure: false,
        ..Fit::EVERYTHING
    },
    Fit {
        figure: false,
        slider: false,
        ..Fit::EVERYTHING
    },
    Fit {
        figure: false,
        slider: false,
        quality: false,
        maximize: true,
    },
    Fit {
        figure: false,
        slider: false,
        quality: false,
        maximize: false,
    },
];

/// How wide the bar has to be to hold `fit`, with `cluster` at its
/// right-hand end: its padding, play and volume, the spacer between the two
/// ends, the cluster's buttons, and whatever `fit` adds, each with the gap
/// before it — the pill and the maximize control only where the cluster has
/// them. The sizes are the ones `button_row` draws with, from `theme`; the
/// quality pill's is an estimate (`theme::QUALITY_PILL_ROOM`).
fn needs(fit: Fit, cluster: Cluster) -> f32 {
    let gap = theme::GAP_TIGHT;
    let shown = |kept: bool, width: f32| if kept { width + gap } else { 0.0 };
    let buttons = cluster.buttons as f32;
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
        + shown(cluster.pill && fit.quality, theme::QUALITY_PILL_ROOM)
        + shown(cluster.maximize && fit.maximize, theme::ICON_BUTTON)
}

/// What fits on a bar `width` logical pixels wide with `cluster` at its
/// right-hand end: the widest of [`NARROWINGS`] that does, or the narrowest
/// when none does. Narrower than that the bar still holds play, volume and
/// the cluster, and runs past the pane's edge — a beside pane at its
/// narrowest, which is a known limit.
pub(super) fn fit(width: f32, cluster: Cluster) -> Fit {
    NARROWINGS
        .into_iter()
        .find(|fit| needs(*fit, cluster) <= width)
        .unwrap_or(NARROWINGS[NARROWINGS.len() - 1])
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// A pick switching a pane to `to`, from its rendition's row or, with
    /// `default`, from the settings' row.
    fn switching(to: &str, default: bool) -> Switching {
        Switching {
            to: SharedString::from(to.to_string()),
            default,
            since: Instant::now(),
        }
    }

    /// At rest the pill says what plays, plainly, with no tooltip; while a
    /// pick is under way it names what was picked, breathes, and says so in
    /// a tooltip, under an id keyed on those words.
    #[test]
    fn the_pill_names_a_pick_under_way() {
        let playing = SharedString::from("720p60");
        let rest = pill_face(&playing, None);
        assert_eq!(rest.label, playing);
        assert!(!rest.waiting);
        assert_eq!(rest.tip, None);

        let picked = switching("480p30", false);
        let busy = pill_face(&playing, Some(&picked));
        assert_eq!(busy.label.as_ref(), "480p30");
        assert!(busy.waiting);
        assert_eq!(busy.tip.as_deref(), Some("Switching to 480p30"));
        assert_ne!(busy.id, rest.id, "a tooltip up at rest would keep no words");

        // The settings' row, by what the settings pick now.
        let back = pill_face(&playing, Some(&switching("1080p60", true)));
        assert_eq!(back.label.as_ref(), "1080p60");
        assert_ne!(back.id, busy.id, "another pick drops the first one's words");
    }

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

    /// A pane's cluster with the maximize control in it: one of two panes
    /// or more on the watch page.
    const PANE_MAX: Cluster = Cluster {
        maximize: true,
        ..PANE
    };

    #[test]
    fn a_wide_bar_shows_everything() {
        assert_eq!(fit(1600.0, PANE), Fit::EVERYTHING);
        assert_eq!(
            fit(needs(Fit::EVERYTHING, PANE), PANE),
            Fit::EVERYTHING,
            "a bar exactly wide enough holds it all"
        );
    }

    /// Narrowing gives things up one at a time and in order — the figure,
    /// then the slider — and never takes one back as it goes on narrowing,
    /// with the maximize control in the bar or not.
    #[test]
    fn narrowing_drops_the_figure_then_the_slider() {
        for cluster in [PANE, PANE_MAX] {
            let mut last = 0;
            for width in widths().rev() {
                let fit = fit(width, cluster);
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
            let seen: Vec<Fit> = widths().map(|width| fit(width, cluster)).collect();
            for narrowing in NARROWINGS {
                assert!(
                    seen.contains(&narrowing),
                    "no width gives {narrowing:?} for {cluster:?}, so a step is skipped"
                );
            }
        }
    }

    /// The quality pill is the last thing to go: it folds only once the
    /// slider has gone, and with it into More.
    #[test]
    fn the_quality_pill_folds_after_the_slider() {
        for width in widths() {
            let fit = fit(width, PANE);
            assert!(
                !fit.slider || fit.quality,
                "at {width}px the pill folded with the slider still up"
            );
        }
        assert!(
            widths().any(|width| {
                let fit = fit(width, PANE);
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
        assert_eq!(fit(0.0, PANE), narrowest);
        assert_eq!(fit(0.0, PANE_MAX), narrowest);
        assert_eq!(
            narrowest,
            Fit {
                figure: false,
                slider: false,
                quality: false,
                maximize: false,
            }
        );
        let squares = 2.0 + PANE.buttons as f32;
        assert!(needs(narrowest, PANE) >= 2.0 * theme::PANEL_PAD + squares * theme::ICON_BUTTON);
    }

    /// The maximize control folds right after the quality pill: never while
    /// the pill is still up, and at some width the pill has gone and it has
    /// not. Folded, it takes no room, so the narrowest bar — a beside pane
    /// at its narrowest, which already runs past its edge — runs no further
    /// for there being one.
    #[test]
    fn the_maximize_button_folds_after_the_quality_pill() {
        for width in widths() {
            let fit = fit(width, PANE_MAX);
            assert!(
                fit.maximize || !fit.quality,
                "at {width}px the maximize folded with the pill still up"
            );
        }
        assert!(
            widths().any(|width| {
                let fit = fit(width, PANE_MAX);
                fit.maximize && !fit.quality
            }),
            "the maximize and the pill went at the same width"
        );
        let narrowest = NARROWINGS[NARROWINGS.len() - 1];
        assert_eq!(needs(narrowest, PANE_MAX), needs(narrowest, PANE));
        assert!(needs(Fit::EVERYTHING, PANE_MAX) > needs(Fit::EVERYTHING, PANE));
    }

    /// A pane with nothing to be maximized over — the only one — has no
    /// control, and gives up nothing for it: at every width its bar keeps
    /// what a bar without the control keeps, and keeps at least as much as
    /// one with it.
    #[test]
    fn a_hidden_maximize_takes_no_room() {
        assert_eq!(cluster(Place::Pane, MaximizeButton::Hidden), PANE);
        for button in [MaximizeButton::Maximize, MaximizeButton::Restore] {
            assert_eq!(cluster(Place::Pane, button), PANE_MAX);
        }
        for narrowing in NARROWINGS {
            assert_eq!(
                needs(narrowing, PANE),
                needs(
                    Fit {
                        maximize: false,
                        ..narrowing
                    },
                    PANE
                ),
                "{narrowing:?} counted room for a control that is not there"
            );
        }
        for width in widths() {
            assert!(
                rank(fit(width, PANE)) <= rank(fit(width, PANE_MAX)),
                "at {width}px the bar with no maximize gave up more"
            );
        }
        assert_eq!(maximize_control(MaximizeButton::Hidden), None);
    }

    /// The control offers what a press does, under an id keyed on it, with
    /// the same words on the bar and in More; and it is not the window's
    /// maximize, the mini player's way back, nor fullscreen.
    #[test]
    fn the_maximize_control_says_what_a_press_does() {
        let maximize = maximize_control(MaximizeButton::Maximize).expect("offered");
        let restore = maximize_control(MaximizeButton::Restore).expect("offered");
        assert_eq!(maximize.words, "Maximize");
        assert_eq!(restore.words, "Show all panes");
        assert_ne!(maximize.bar_id, restore.bar_id);
        assert_ne!(maximize.row_id, restore.row_id);
        for control in [maximize, restore] {
            for other in [
                Icon::Maximize,
                Icon::Restore,
                Icon::Expand,
                Icon::Fullscreen,
                Icon::FullscreenExit,
            ] {
                assert_ne!(control.icon.path(), other.path(), "{control:?}");
            }
        }
    }

    /// A pane's cluster with one more button in it.
    const ANOTHER: Cluster = Cluster {
        buttons: RIGHT_BUTTONS + 1,
        ..PANE
    };

    /// A control added to the right-hand cluster — the guide — takes its
    /// room from what folds: at any width the bar gives up at least as much
    /// as it did, and somewhere strictly more.
    #[test]
    fn more_on_the_right_narrows_sooner() {
        for width in widths() {
            assert!(
                rank(fit(width, ANOTHER)) >= rank(fit(width, PANE)),
                "at {width}px another button gave the bar more room"
            );
        }
        assert!(widths().any(|width| fit(width, ANOTHER) != fit(width, PANE)));
    }

    /// A pop-out at the least size its window may be is still a player: its
    /// bar holds play, the speaker, Bring back and Close inside the window's
    /// width.
    #[test]
    fn a_pop_out_bar_keeps_play_volume_and_its_two_buttons() {
        let narrowest = fit(theme::POP_OUT_MIN_WIDTH, POP_OUT);
        assert!(
            needs(narrowest, POP_OUT) <= theme::POP_OUT_MIN_WIDTH,
            "the smallest pop-out's bar runs past its edge"
        );
        for button in [
            MaximizeButton::Hidden,
            MaximizeButton::Maximize,
            MaximizeButton::Restore,
        ] {
            assert_eq!(cluster(Place::PopOut, button), POP_OUT);
        }
    }

    /// With no pill to make room for, a pop-out's slider and figure are up
    /// at every width that holds them beside its two buttons — including
    /// the width a vertical stream's pop-out opens at, which lost its
    /// slider to room kept for a pill it never draws — and at the width a
    /// widescreen one opens at, both are.
    #[test]
    fn a_pop_out_keeps_its_slider_wherever_it_fits() {
        let slider_only = Fit {
            figure: false,
            slider: true,
            quality: false,
            maximize: false,
        };
        for width in widths() {
            let fit = fit(width, POP_OUT);
            assert_eq!(
                fit.slider,
                width >= needs(slider_only, POP_OUT),
                "the slider at {width}px"
            );
            assert_eq!(
                fit.figure,
                width >= needs(Fit::EVERYTHING, POP_OUT),
                "the figure at {width}px"
            );
        }
        let display = gpui::Bounds {
            origin: gpui::point(px(0.0), px(0.0)),
            size: gpui::size(px(1920.0), px(1080.0)),
        };
        for aspect in [9.0 / 16.0, crate::layout::VIDEO_ASPECT, 4.0 / 3.0] {
            let opens = crate::layout::pop_out_bounds(display, aspect, &[], None);
            assert!(
                fit(f32::from(opens.size.width), POP_OUT).slider,
                "a {aspect} pop-out opens without its slider"
            );
        }
        assert_eq!(fit(theme::POP_OUT_WIDTH, POP_OUT), Fit::EVERYTHING);
    }
}
