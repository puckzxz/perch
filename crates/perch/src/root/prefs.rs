//! What the user has said about how the app should be: the settings sheet
//! and what saving it changes, the divider drag that sizes video against
//! chat, and the rail folding away. All of it ends up in `settings.json`.

use std::time::Duration;

use gpui::{prelude::*, App, Context, MouseMoveEvent, MouseUpEvent, Window};

use super::{Page, Resize, RootView};
use crate::browse::{Discovery, SignIn};
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

        let panel = cx.new(|cx| {
            SettingsPanel::new(self.settings.clone(), self.sign_in.summary(), window, cx)
        });
        cx.subscribe_in(
            &panel,
            window,
            |this: &mut RootView, _, event, window, cx| {
                match event {
                    SettingsEvent::Dismissed => this.settings_panel = None,
                    SettingsEvent::Saved(updated) => {
                        let client_id_changed =
                            this.settings.credentials.client_id != updated.credentials.client_id;
                        // Turned off while streams are already parked on the
                        // browse page, the setting has to act now — otherwise
                        // it reads as broken until the next navigation.
                        let miniplayer_off = this.settings.miniplayer && !updated.miniplayer;
                        let stream_changed = this.settings.quality != updated.quality
                            || this.settings.credentials.auth_token
                                != updated.credentials.auth_token;

                        this.settings = (**updated).clone();
                        // A new client id invalidates any stored sign-in, so
                        // that case drops the tokens rather than keeping them.
                        let saved = if client_id_changed {
                            this.settings.save_forgetting_sign_in(&this.settings_path)
                        } else {
                            this.settings.save_preferences(&this.settings_path)
                        };
                        if let Err(e) = saved {
                            eprintln!("settings: could not save: {e}");
                            this.toast(format!("could not save settings: {e}"), cx);
                        }
                        this.settings_panel = None;

                        if miniplayer_off && this.page == Page::Browse {
                            this.slots.clear();
                        }

                        // Apply immediately rather than asking for a restart,
                        // which is the entire reason this panel exists.
                        if client_id_changed {
                            this.sign_in = SignIn::Connecting;
                            this.follows.clear();
                            this.offline.clear();
                            this.known_live.clear();
                            this.avatars.clear();
                            this.follows_loaded = false;
                            // Browsing was fetched with the old app's token, so
                            // it goes with it.
                            this.discovery = Discovery::default();
                            let (service, pump) =
                                Self::spawn_twitch(this.settings_path.clone(), window, cx);
                            this.twitch = service;
                            this._twitch_pump = pump;
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
        self.resize = Some(Resize {
            start,
            chat_width: self.settings.chat_width,
            video_share: self.effective_video_share(start.index, window, cx),
        });
    }

    /// What share of a stacked cell the video has right now, in pane `index`.
    ///
    /// A drag has to start from what is on screen, and until somebody has
    /// dragged one there is no stored share — only the box the layout derives
    /// from that pane's stream. Reading it back means the first pull moves from
    /// where the divider actually is rather than jumping to a default.
    pub(super) fn effective_video_share(&self, index: usize, window: &Window, cx: &App) -> f32 {
        if self.settings.video_share > 0.0 {
            return self.settings.video_share;
        }
        let body = self.body(window);
        let (rows, cols) = layout::grid_shape(self.slots.len().max(1), body.aspect());
        let cell_height = layout::cell_extent(body.height, rows);
        if cell_height <= 0.0 {
            return theme::VIDEO_SHARE_MIN;
        }
        let cell_width = layout::cell_extent(body.width, cols);
        let aspect = self
            .slots
            .get(index)
            .and_then(|slot| slot.video())
            .and_then(|view| view.read(cx).source_aspect())
            .unwrap_or(layout::VIDEO_ASPECT);
        (layout::stacked_video_height(cell_width, cell_height, aspect, 0.0) / cell_height)
            .clamp(theme::VIDEO_SHARE_MIN, theme::VIDEO_SHARE_MAX)
    }

    /// Follow the pointer, if a divider is being dragged.
    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = &self.resize else {
            return;
        };

        if resize.start.portrait {
            let body = self.body(window);
            let rows = layout::grid_shape(self.slots.len().max(1), body.aspect()).0;
            let cell_height = layout::cell_extent(body.height, rows);
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

    /// Let go, and write the size down.
    ///
    /// Saved here rather than on every move: a drag is hundreds of events and
    /// each save is a read-modify-write of the whole settings file.
    pub(super) fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resize.take().is_none() {
            return;
        }
        self.save_settings(cx);
        cx.notify();
    }

    /// Fold the follows rail away, or bring it back, and remember which.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
        self.save_settings(cx);
        // The panes just changed width, and with it perhaps their shape.
        self.sync_quality(window, cx);
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

    /// The window changed size, or went fullscreen and back. Quality is
    /// chosen again once the run of changes has settled; see
    /// `RootView::sync_quality`.
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
