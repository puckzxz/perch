//! What a pane asks for, resolved: a press on one of its own controls, on
//! its player's bar or in its menus, a palette row about it, or a press on
//! the pane itself (`watch::PaneAction`), and what its player asks of the
//! root (`VideoEvent`).
//!
//! Both arrive with the pane's key and are looked up here, when they land,
//! rather than by a position read when the pane was drawn: a press can land
//! after the panes have moved, and a player's event long after that.

use gpui::{ClipboardItem, Context, Window};

use super::RootView;
use crate::video_view::{ChatButton, VideoEvent};
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
            PaneAction::ToggleChat => self.toggle_chat(index, cx),
            // The moment the pane is at, on a recording: More is where a
            // moment is offered, and the palette's row is More by name.
            PaneAction::OpenOnTwitch => cx.open_url(&self.slots[index].link(true)),
            PaneAction::CopyLink => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.slots[index].link(true)));
                self.toast("link copied", cx);
            }
        }
    }

    /// Show or hide the chat of the pane at `index`, and remember it for that
    /// channel: `C`, and the chat glyph on the pane's bar.
    ///
    /// Per pane rather than per app: the whole watch page is built on panes
    /// being independent, and the reason to hide chat — watching one stream for
    /// the game while reading another's chat — only makes sense if it is.
    ///
    /// The one place a pane's `chat_hidden` changes after it opens, so the one
    /// place that tells the pane's player, whose glyph mirrors it
    /// (`video_view::ChatButton`).
    pub(super) fn toggle_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        // A video whose chat cannot be replayed. Say so, rather than toggling
        // a pane that would come up empty. Its glyph is drawn still, so only
        // the key ever asks.
        if self.slots[index].chat.is_none() {
            self.toast("no chat replay for this video", cx);
            return;
        }
        let hidden = !self.slots[index].chat_hidden;
        self.slots[index].chat_hidden = hidden;
        if let Some(view) = self.slots[index].video().cloned() {
            view.update(cx, |view, cx| {
                view.set_chat(ChatButton::of(hidden, true), cx)
            });
        }

        let channel = self.slots[index].channel.clone();
        if self.settings.set_chat_hidden_for(&channel, hidden) {
            self.save_settings(cx);
        }
        cx.notify();
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
            // The bar's own: the same route as the pane's header and the
            // palette, by the key the player was subscribed with.
            VideoEvent::Pane(action) => self.on_pane_action(owner, action.clone(), window, cx),
        }
    }
}
