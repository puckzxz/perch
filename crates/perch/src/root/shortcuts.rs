//! What each key does. The bindings themselves are in `crate::keys`; these
//! are the handlers the root installs for them, one per action, each a line
//! or two that names the method doing the work.

use gpui::{Context, Window};
use settings::Settings;

use super::RootView;
use crate::browse::Action;
use crate::keys;

impl RootView {
    pub(super) fn on_toggle_playback(
        &mut self,
        _: &keys::TogglePlayback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.toggle_playback(cx));
        }
    }

    pub(super) fn on_toggle_mute(
        &mut self,
        _: &keys::ToggleMute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.toggle_mute(window, cx));
        }
    }

    pub(super) fn on_volume_up(
        &mut self,
        _: &keys::VolumeUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_volume(keys::VOLUME_STEP, window, cx);
    }

    pub(super) fn on_volume_down(
        &mut self,
        _: &keys::VolumeDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_volume(-keys::VOLUME_STEP, window, cx);
    }

    pub(super) fn nudge_volume(&mut self, delta: i16, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.nudge_volume(delta, window, cx));
        }
    }

    pub(super) fn on_seek_back(
        &mut self,
        _: &keys::SeekBack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seek_by(-keys::SEEK_STEP, cx);
    }

    pub(super) fn on_seek_forward(
        &mut self,
        _: &keys::SeekForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seek_by(keys::SEEK_STEP, cx);
    }

    /// Skip the active pane's recording. A live pane ignores it: there is
    /// nowhere to go.
    pub(super) fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        if let Some(view) = self.active_video() {
            view.update(cx, |video, cx| video.seek_by(delta, cx));
        }
    }

    /// Show or hide the active pane's chat, and remember it for that channel.
    ///
    /// Per pane rather than per app: the whole watch page is built on panes
    /// being independent, and the reason to hide chat — watching one stream for
    /// the game while reading another's chat — only makes sense if it is.
    pub(super) fn on_toggle_chat(
        &mut self,
        _: &keys::ToggleChat,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_slot() else {
            return;
        };
        // A video whose chat cannot be replayed. Say so, rather than toggling
        // a pane that would come up empty.
        if self.slots[index].chat.is_none() {
            self.toast("no chat replay for this video", cx);
            return;
        }
        let hidden = !self.slots[index].chat_hidden;
        self.slots[index].chat_hidden = hidden;

        let channel = self.slots[index].channel.clone();
        if self.settings.set_chat_hidden_for(&channel, hidden) {
            self.save_settings(cx);
        }
        cx.notify();
    }

    pub(super) fn on_toggle_sidebar(
        &mut self,
        _: &keys::ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_sidebar(window, cx);
    }

    pub(super) fn on_close_pane(
        &mut self,
        _: &keys::ClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.active_slot() {
            self.close_slot(index, window, cx);
        }
    }

    pub(super) fn on_go_browse(
        &mut self,
        _: &keys::GoBrowse,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_browse(cx);
    }

    pub(super) fn on_toggle_settings(
        &mut self,
        _: &keys::ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }

    pub(super) fn on_focus_search(
        &mut self,
        _: &keys::FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    pub(super) fn on_toggle_palette(
        &mut self,
        _: &keys::TogglePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_palette(window, cx);
    }

    pub(super) fn on_toggle_fullscreen(
        &mut self,
        _: &keys::ToggleFullscreen,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    /// Put the video and chat back to the sizes they are derived at.
    pub(super) fn on_reset_layout(
        &mut self,
        _: &keys::ResetLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let defaults = Settings::default();
        self.settings.chat_width = defaults.chat_width;
        self.settings.video_share = defaults.video_share;
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn on_refresh(
        &mut self,
        _: &keys::Refresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh(cx);
    }

    /// Point the keyboard at pane `index`, counting from the top left.
    pub(super) fn on_activate_pane(
        &mut self,
        action: &keys::ActivatePane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(slot) = self.slots.get(action.index) {
            self.active = Some(slot.key.clone());
            cx.notify();
        }
    }

    pub(super) fn on_next_pane(
        &mut self,
        _: &keys::NextPane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active(1, cx);
    }

    pub(super) fn on_previous_pane(
        &mut self,
        _: &keys::PreviousPane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active(-1, cx);
    }

    /// Move the active pane along the grid, wrapping at either end.
    fn step_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.slots.len();
        let Some(current) = self.active_slot() else {
            return;
        };
        let next = (current as isize + delta).rem_euclid(count as isize) as usize;
        self.active = Some(self.slots[next].key.clone());
        cx.notify();
    }

    /// One step back on the browse page: a channel's page, a search or a
    /// category closes first, since it took the page over; with none open,
    /// back to whatever is playing. `Esc` on the watch page goes the other
    /// way, which makes the key a toggle between the two pages when nothing
    /// else is in the way.
    pub(super) fn on_back(&mut self, _: &keys::Back, window: &mut Window, cx: &mut Context<Self>) {
        let leave = if self.discovery.channel.is_some() {
            Some(Action::CloseChannel)
        } else if self.discovery.search.is_some() {
            Some(Action::CloseSearch)
        } else if self.discovery.open.is_some() {
            Some(Action::CloseCategory)
        } else {
            None
        };
        match leave {
            Some(action) => self.on_browse_action(action, window, cx),
            None => self.go_watch(cx),
        }
    }
}
