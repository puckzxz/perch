//! What the user has said about how the app should be: the settings sheet
//! and what saving it changes, the divider drag that sizes video against
//! chat, how every chat is drawn, the rail folding away and what is pinned
//! to it. All of it ends up in `settings.json`.

use std::time::Duration;

use gpui::{
    canvas, prelude::*, App, Context, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent,
    Window,
};
use settings::SheetFields;

use super::{Resize, RootView};
use crate::browse::{Discovery, SignIn};
use crate::chat_display::ChatDisplay;
use crate::settings_view::{SettingsEvent, SettingsPanel};
use crate::watch::ResizeStart;
use crate::{layout, theme};

/// How long a run of changes is left to settle before the file is written.
/// A volume drag is a change per pixel; the file is written once, after.
const SAVE_SETTLE: Duration = Duration::from_millis(400);

/// How long a run of window resizes is left to settle before quality is
/// chosen again. A drag of the frame is dozens of resizes a second, and a
/// stream restart per one of them would be a pane that never plays.
const RESIZE_SETTLE: Duration = Duration::from_millis(750);

impl RootView {
    pub(super) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.take().is_some() {
            cx.notify();
            return;
        }

        // The sheet takes the keyboard from whatever had it, whichever way it
        // was opened. `Ctrl+,` does not stand aside for a text box, so it can
        // open the sheet with the cursor still in the title bar's search box,
        // which sits above the scrim. The veil there only stops the pointer,
        // so typing went on reaching the box behind the sheet. `Enter` ran a
        // search, which on the watch page left the page and, with the mini
        // player off, stopped every stream. `Esc` could not close the sheet
        // either, because its binding stands aside for a text box like every
        // other. The palette is closed rather than left under the sheet
        // without its cursor. There, `Esc` would only have closed and reopened
        // the sheet over it, since a binding is answered before the palette's
        // own key handler is asked. The other way round, `Ctrl+K` does nothing
        // while the sheet is up (see `keys::bindings`), so the two never stack.
        if self.palette_open {
            self.toggle_palette(window, cx);
        }
        // Nor under a chat's copy menu, which would draw over it.
        self.close_copy_menus(cx);
        self.focus.focus(window);

        // Only the fields it shows: whatever else the app writes while the
        // sheet is open is never the sheet's to hand back. See `SheetFields`.
        let panel = cx.new(|cx| {
            SettingsPanel::new(
                SheetFields::of(&self.settings),
                self.sign_in.summary(),
                window,
                cx,
            )
        });
        cx.subscribe_in(
            &panel,
            window,
            |this: &mut RootView, _, event, window, cx| {
                match event {
                    SettingsEvent::Dismissed => this.settings_panel = None,
                    SettingsEvent::Saved(updated) => {
                        let client_id_changed =
                            this.settings.credentials.client_id != updated.client_id;
                        // Turned off while streams are already parked on the
                        // browse page, the setting has to act now — otherwise
                        // it reads as broken until the next navigation.
                        let miniplayer_off = this.settings.miniplayer && !updated.miniplayer;
                        let stream_changed = this.settings.quality != updated.quality
                            || this.settings.credentials.auth_token != updated.auth_token;

                        // Only what the sheet owns, so whatever the app wrote
                        // while it was open — the channel a later launch
                        // opened — survives; see `adopt_sheet`.
                        this.settings.adopt_sheet(updated);
                        // A new client id invalidates any stored sign-in, so
                        // that case drops the tokens rather than keeping them.
                        let saved = if client_id_changed {
                            this.settings.save_forgetting_sign_in(&this.settings_path)
                        } else {
                            this.settings.save_preferences(&this.settings_path)
                        };
                        if let Err(e) = saved {
                            eprintln!("settings: could not save: {e}");
                            this.toast(format!("Could not save settings: {e}"), cx);
                        }
                        this.settings_panel = None;

                        // By the rule leaving the page with it off stops
                        // panes by: those in windows of their own play on.
                        if miniplayer_off {
                            this.retire_homeless(cx);
                        }

                        // Apply immediately rather than asking for a restart,
                        // which is the entire reason this panel exists.
                        if client_id_changed {
                            this.sign_in = SignIn::Connecting;
                            this.follows.clear();
                            this.offline.clear();
                            this.home_offline.clear();
                            this.known_live.clear();
                            this.avatars.clear();
                            this.follows_loaded = false;
                            // Browsing was fetched with the old app's token, so
                            // it goes with it — and so does the trail through
                            // it, whose places were read from what just went.
                            this.discovery = Discovery::default();
                            this.trail.clear();
                            let (service, pump) =
                                Self::spawn_twitch(this.settings_path.clone(), window, cx);
                            this.twitch = service;
                            this._twitch_pump = pump;
                            // And the panes' asks about past broadcasts, whose
                            // answers die with the old worker; the new one's
                            // sign-in asks again.
                            this.forget_asks();
                        }
                        if stream_changed {
                            // By key, not by channel: a recording's pane is
                            // keyed by the video, and looking it up by its
                            // channel found nothing, so a quality change
                            // here restarted every live pane and left a
                            // recording playing at the old one.
                            let keys: Vec<String> =
                                this.slots.iter().map(|slot| slot.key.clone()).collect();
                            for key in keys {
                                if let Some(index) = this.slot_index(&key) {
                                    this.slots[index].quality_override = None;
                                    this.restart_stream(index, window, cx);
                                }
                            }
                        }
                    }
                }
                cx.notify();
            },
        )
        .detach();

        self.settings_panel = Some(panel);
        cx.notify();
    }

    /// Remember where a divider drag began.
    pub(super) fn start_resize(
        &mut self,
        start: ResizeStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let video_share = self.effective_video_share(&start.key, window, cx);
        self.resize = Some(Resize {
            start,
            chat_width: self.settings.chat_width,
            video_share,
        });
    }

    /// What share of a stacked cell the video has right now, in the pane
    /// `key` names.
    ///
    /// A drag has to start from what is on screen, and until somebody has
    /// dragged one there is no stored share — only the box the layout derives
    /// from that pane's stream. Reading it back means the first pull moves from
    /// where the divider actually is rather than jumping to a default. The
    /// cell is the one grid's (`RootView::grid`), the page's own.
    pub(super) fn effective_video_share(&self, key: &str, window: &Window, cx: &App) -> f32 {
        if self.settings.video_share > 0.0 {
            return self.settings.video_share;
        }
        let grid = self.grid(window);
        if grid.cell_height <= 0.0 {
            return theme::VIDEO_SHARE_MIN;
        }
        // Through what the cell is drawn from, so the drag starts where the
        // page drew the divider: a pane in a window of its own has its
        // picture there and a 16:9 box here (`watch::pane`).
        let aspect = self
            .slot_index(key)
            .and_then(|index| self.video_in_main(&self.slots[index]))
            .and_then(|view| view.read(cx).source_aspect())
            .unwrap_or(layout::VIDEO_ASPECT);
        (layout::stacked_video_height(grid.cell_width, grid.cell_height, aspect, 0.0)
            / grid.cell_height)
            .clamp(theme::VIDEO_SHARE_MIN, theme::VIDEO_SHARE_MAX)
    }

    /// The listeners that follow a divider drag, as an element for the root
    /// to hold: a `canvas` that registers them with the window as it paints.
    ///
    /// Window-level rather than on the handle or the root. The pointer leaves
    /// a six-pixel handle on the first frame of any pull worth making, and an
    /// element's own `on_mouse_move` and `on_mouse_up` only hear the pointer
    /// over that element's unblocked hitbox — so the root's went deaf over
    /// the occluded title bar, over a toast, over anything that blocks the
    /// mouse, and a drag that wandered there lost its moves and could miss its
    /// release. The window hears every one: gpui captures the mouse on a
    /// press, so even a drag that leaves the window is still reported.
    ///
    /// A `canvas` inserts no hitbox, so this covers the window without being
    /// in the way of anything. Both handlers return at once when nothing is
    /// being dragged, which is most of the time.
    ///
    /// The release also ends a pane header's drag (`pane_move`), wherever it
    /// lands: gpui ends its own drag after every release (window.rs:3750-3760)
    /// and tells nobody, so a drag let go over no other pane would otherwise
    /// leave every pane offering itself. One let go on a pane swaps the two
    /// as well, from that pane's drop layer, which this runs ahead of — the
    /// drop carries its own pane, so it needs nothing kept here.
    pub(super) fn drag_listeners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moves = owner.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        moves
                            .update(cx, |this, cx| this.on_mouse_move(event, window, cx))
                            .ok();
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                        owner
                            .update(cx, |this, cx| this.on_mouse_up(event, window, cx))
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    /// Follow the pointer, if a divider is being dragged.
    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = &self.resize else {
            return;
        };

        if resize.start.portrait {
            let cell_height = self.grid(window).cell_height;
            if cell_height <= 0.0 {
                return;
            }
            let travelled = f32::from(event.position.y - resize.start.origin.y);
            self.settings.video_share = (resize.video_share + travelled / cell_height)
                .clamp(theme::VIDEO_SHARE_MIN, theme::VIDEO_SHARE_MAX);
        } else {
            // Chat is to the *right* of the video, so dragging left widens it.
            let travelled = f32::from(resize.start.origin.x - event.position.x);
            self.settings.chat_width =
                (resize.chat_width + travelled).clamp(theme::CHAT_WIDTH_MIN, theme::CHAT_WIDTH_MAX);
        }
        cx.notify();
    }

    /// Let go: of a pane's header, and of a divider, writing its size down.
    ///
    /// Saved here rather than on every move: a drag is hundreds of events and
    /// each save is a read-modify-write of the whole settings file.
    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pane_move.take().is_some() {
            cx.notify();
        }
        if self.resize.take().is_none() {
            return;
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Fold the follows rail away, or bring it back, and remember which.
    ///
    /// Not in fullscreen, where the rail is not drawn either way (see
    /// `layout::rail_shown`). There the press would change nothing on screen
    /// and only show up on leaving fullscreen, as a rail folded or back
    /// without anything having said so. `B` and the palette's row stand aside
    /// there, as `Ctrl+F` does for the search box that goes with the bar.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_fullscreen() {
            return;
        }
        self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
        self.save_settings(cx);
        // The panes just changed width, and with it perhaps their shape.
        self.sync_quality(window, cx);
        // Folded, the recommendations are not asked for; unfolded, whatever
        // came due meanwhile is.
        self.update_recommended();
        cx.notify();
    }

    /// Pin a channel to the top of the rail, or take it off, and write it
    /// down. The one place either happens, so a pin is saved however it came
    /// to be made, and a press that changes nothing writes nothing.
    pub(super) fn set_pinned(&mut self, login: String, pinned: bool, cx: &mut Context<Self>) {
        if self.settings.set_pinned(&login, pinned) {
            self.save_settings(cx);
            cx.notify();
        }
    }

    /// Draw every chat as `display` says, and remember it: a row of a pane's
    /// chat options menu (`watch::chat_menu`).
    ///
    /// Every chat's, whichever pane's menu it came from, since a text size is
    /// about the reader and the screen rather than about a channel. The one
    /// place the settings' chat options change, so the one place that tells
    /// the chats, each of which mirrors them (`ChatView::set_display`): the
    /// settings sheet does not own them (`SheetFields`), so saving it keeps
    /// them, and a chat made later reads them as it is made. Written at once
    /// rather than soon: it is one press, not a run of them like a drag.
    pub(super) fn set_chat_display(&mut self, display: ChatDisplay, cx: &mut Context<Self>) {
        if !display.store(&mut self.settings) {
            return;
        }
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        for chat in chats {
            chat.update(cx, |chat, cx| chat.set_display(display, cx));
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Write the preferences down once they have stopped changing.
    ///
    /// For the changes that come in runs: a volume drag is a change per
    /// pixel, and each save is a read-modify-write of the whole file, so a
    /// hundred of them in one drag was a hundred writes. The value is in
    /// memory already; only the file waits, and anything else that writes it
    /// meanwhile — the window closing, the sheet saving — writes the same
    /// memory, so nothing is lost by waiting.
    pub(super) fn save_settings_soon(&mut self, cx: &mut Context<Self>) {
        self.save_epoch += 1;
        let epoch = self.save_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_SETTLE).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                if this.save_epoch == epoch {
                    this.save_settings(cx);
                }
            });
        })
        .detach();
    }

    /// Something a pane is drawn in changed size: the main window, resized
    /// or gone fullscreen and back, or a pop-out, opened (`pop_out`) or
    /// grown or shrunk (`pop_out_moved`). Quality is chosen again once the
    /// run of changes has settled, the one settle for every window; see
    /// `RootView::sync_quality`.
    ///
    /// `window` is always the main one, whatever changed: a pop-out reaches
    /// the root only through `pop_out::to_root`, and each pane is measured
    /// by its own window regardless (`RootView::pane_height_for`). The wait
    /// is bound to the main window's handle and needs no frame of it, so a
    /// pop-out resized while the main window is minimised still has its
    /// quality chosen again. Returning early here while the main window is
    /// not on screen would leave every pop-out at the quality it opened
    /// with.
    pub(super) fn on_window_resized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resize_epoch += 1;
        let epoch = self.resize_epoch;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let _ = this.update_in(cx, |this: &mut RootView, window, cx| {
                if this.resize_epoch == epoch {
                    this.sync_quality(window, cx);
                }
            });
        })
        .detach();
    }

    /// Where the window was when it closed, written down for next time.
    pub(crate) fn remember_window(
        &mut self,
        placement: settings::WindowPlacement,
        cx: &mut Context<Self>,
    ) {
        self.settings.window = Some(placement);
        self.save_settings(cx);
    }
}
