//! What a pane asks for, resolved: a press on one of its own controls, or on
//! the pane itself (`watch::PaneAction`), and what its player asks of the
//! root (`VideoEvent`).
//!
//! Both arrive with the pane's key and are looked up here, when they land,
//! rather than by a position read when the pane was drawn: a press can land
//! after the panes have moved, and a player's event long after that.

use gpui::{Context, Window};

use super::RootView;
use crate::video_view::VideoEvent;
use crate::watch::PaneAction;

impl RootView {
    /// A pane's control, or a press on the pane, for the pane `key` names.
    /// Nothing happens for a pane that has gone since.
    ///
    /// Every action takes the keys back for the root first. A press on a pane
    /// is a press on the watch page, and the page's shortcuts are the root's,
    /// so whatever had the keys before — the title bar's search box — should
    /// not keep them. Today the root's own `track_focus` hears most of these
    /// presses too, but it only hears what reaches it: anything drawn over a
    /// pane that blocks the pointer hides the press from it. Doing it here,
    /// for every action, means a control added later cannot forget to.
    pub(super) fn on_pane_action(
        &mut self,
        key: &str,
        action: PaneAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window);
        let Some(index) = self.slot_index(key) else {
            return;
        };
        match action {
            PaneAction::Close => self.close_slot(index, window, cx),
            PaneAction::Retry => self.retry_stream(key, window, cx),
            PaneAction::Activate => {
                // Only a change repaints: most presses land in the pane that
                // already has the keys.
                if self.active.as_deref() != Some(key) {
                    self.active = Some(key.to_string());
                    cx.notify();
                }
            }
        }
    }

    /// What the player in the pane `owner` names asked for. Looked up by key
    /// when it arrives, like a pane's own controls: the pane may have moved,
    /// or gone, since the player was made.
    pub(super) fn on_video_event(
        &mut self,
        owner: &str,
        event: &VideoEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            VideoEvent::VolumeChanged(volume) => {
                // Remembered against the channel rather than globally, so
                // coming back to a streamer finds them where you left them.
                // The channel, not the pane's key: for a recording that is
                // `vod:<id>`, and a level kept there was never read back — the
                // pane opens at its *channel's* level. A slider drag emits a
                // change per pixel, so the write waits for the run to end.
                //
                // A level somebody chose is also the end of Mute all for this
                // pane: the player has already let its hush go.
                let index = self.slot_index(owner);
                if let Some(index) = index {
                    self.slots[index].quiet = false;
                }
                let channel = index.map(|index| self.slots[index].channel.clone());
                if let Some(channel) = channel {
                    if self.settings.set_volume_for(&channel, *volume) {
                        self.save_settings_soon(cx);
                    }
                }
                cx.notify();
            }
            VideoEvent::QualityRequested(name) => {
                if let Some(index) = self.slot_index(owner) {
                    self.request_quality(index, name.clone(), window, cx);
                }
            }
            VideoEvent::Stopped(reason) => self.stream_stopped(owner, reason.clone(), cx),
        }
    }
}
