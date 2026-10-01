//! What a pane asks for, resolved: a press on one of its own controls, on
//! its player's bar or in its menus, a palette row about it, or a press on
//! the pane itself (`watch::PaneAction`), and what its player asks of the
//! root (`VideoEvent`).
//!
//! Both arrive with the pane's key and are looked up here, when they land,
//! rather than by a position read when the pane was drawn: a press can land
//! after the panes have moved, and a player's event long after that.
//!
//! Also the one answer the root gives a pane on its own account: bringing
//! its header up over the picture for a moment when a key has just made it
//! the active pane (`reveal_header`).

use gpui::{ClipboardItem, Context, Window};

use super::RootView;
use crate::theme;
use crate::video_view::{ChatButton, VideoEvent};
use crate::watch::{placement, PaneAction, Placement};

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
    /// channel: `C`, the chat glyph on the pane's bar, and `Show chat` on the
    /// header over a pane with no picture, which has no bar. Hiding it brings
    /// the header up where it went for a moment (`reveal_header`).
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
        // Hiding chat takes the header off the panel and over the picture,
        // where it only shows under the pointer; showing where it went says
        // that it went somewhere, rather than away.
        if hidden {
            let key = self.slots[index].key.clone();
            self.reveal_header(&key, cx);
        }

        let channel = self.slots[index].channel.clone();
        if self.settings.set_chat_hidden_for(&channel, hidden) {
            self.save_settings(cx);
        }
        cx.notify();
    }

    /// Bring the header of the pane `key` names up over its picture for a
    /// moment ([`theme::HEADER_REVEAL`]), with its underline if it is marked
    /// as the active pane: after a key made it the pane the keys talk to, or
    /// hid its chat.
    ///
    /// Only a header that lives over the picture. One above chat is always
    /// on screen, underline and all, so it needs no reveal; that pane still
    /// takes the reveal off any other, since one pane at a time is revealed.
    /// Taken down by a timer — the toasts' pattern — unless a later reveal
    /// has started since, whose own timer is the one that counts.
    ///
    /// For a pane key, and for chat going away. The pointer cannot ask which
    /// pane the keys talk to — pointing at a pane is what makes it active —
    /// and pause, mute and volume get no reveal at all; see `keys`. Hiding
    /// chat from the bar's glyph reveals too, as `C` does: the pointer is on
    /// the pane then, so the band is up for it anyway, and the reveal only
    /// keeps it up for the rest of the moment if the pointer goes sooner.
    pub(super) fn reveal_header(&mut self, key: &str, cx: &mut Context<Self>) {
        let over_picture = self
            .slot_index(key)
            .is_some_and(|index| placement(&self.slots[index]) == Placement::OverPicture);
        for slot in &mut self.slots {
            slot.revealed = over_picture && slot.key == key;
        }
        cx.notify();
        if !over_picture {
            return;
        }

        self.reveal_epoch += 1;
        let epoch = self.reveal_epoch;
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(theme::HEADER_REVEAL).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                if !reveal_is_current(this.reveal_epoch, epoch) {
                    return;
                }
                if let Some(index) = this.slot_index(&key) {
                    this.slots[index].revealed = false;
                    cx.notify();
                }
            });
        })
        .detach();
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

/// Whether the reveal a timer was started for is still the newest, numbered
/// by `RootView::reveal_epoch`. A pane key pressed twice inside the reveal —
/// `2`, then `3` straight after — leaves the first timer running, and it must
/// not take down a header the second press brought up a moment ago.
fn reveal_is_current(latest: u64, timer: u64) -> bool {
    latest == timer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two reveals inside one `HEADER_REVEAL`: the first one's timer fires
    /// while the second is still up, and leaves it; the second one's timer
    /// takes it down.
    #[test]
    fn a_later_reveal_outlives_an_earlier_timer() {
        let (first, second) = (1, 2);
        let latest = second;
        assert!(!reveal_is_current(latest, first));
        assert!(reveal_is_current(latest, second));
    }
}
