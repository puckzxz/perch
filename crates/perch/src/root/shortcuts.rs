//! What each key does. The bindings themselves are in `crate::keys`; these
//! are the handlers the root installs for them, one per action, each a line
//! or two that names the method doing the work.

use gpui::{Context, Window};
use settings::Settings;

use super::{Page, RootView};
use crate::browse::{Action, Place};
use crate::{keys, stage};

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

    /// Show or hide the active pane's chat; see `toggle_chat`, which the
    /// chat glyph on a pane's bar reaches too.
    pub(super) fn on_toggle_chat(
        &mut self,
        _: &keys::ToggleChat,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.active_slot() {
            self.toggle_chat(index, cx);
        }
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

    /// `Esc` on the watch page: one step out of it, taking back the last
    /// thing that was opened first. A pane's header being dragged goes
    /// first, let go where it was with nothing moved — the button is still
    /// down, and the release then lands on nothing. Then a menu open over a
    /// pane — leaving the page with the menu still up was two steps taken
    /// for one press — then a pane given the whole page shows every pane
    /// again (`show_all_panes`), and only then is the page left.
    pub(super) fn on_go_browse(
        &mut self,
        _: &keys::GoBrowse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only a drag of a pane. The volume slider drags too, and `Esc`
        // midway through that is no reason to let go of it.
        if self.pane_move.take().is_some() {
            cx.stop_active_drag(window);
            cx.notify();
            return;
        }
        let videos: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.video().cloned())
            .collect();
        // A pane's chat options menu is one of those menus too.
        let mut closed = self.chat_menu.take().is_some();
        if closed {
            cx.notify();
        }
        for view in videos {
            closed |= view.update(cx, |view, cx| view.close_menu(cx));
        }
        if closed || self.show_all_panes(window, cx) {
            return;
        }
        self.go_browse(cx);
    }

    /// `Z`: the active pane given the whole watch page, chat and all, or
    /// every pane shown again from the pane that has it; see
    /// `toggle_maximize`. Nothing for a lone pane, or one in a window of its
    /// own.
    pub(super) fn on_toggle_maximize(
        &mut self,
        _: &keys::ToggleMaximize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self
            .active_slot()
            .map(|index| self.slots[index].key.clone())
        {
            self.toggle_maximize(&key, window, cx);
        }
    }

    pub(super) fn on_toggle_settings(
        &mut self,
        _: &keys::ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }

    /// Put the cursor in the title bar's search box — on either page, since
    /// the bar is over both.
    ///
    /// Not in fullscreen, where the bar is not drawn. The box would take focus
    /// without being on screen, and focus held by an element that is not
    /// rendered is lost on the next frame and handed back to the root, so the
    /// press would do nothing but blink focus away and back.
    pub(super) fn on_focus_search(
        &mut self,
        _: &keys::FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.is_fullscreen() {
            return;
        }
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

    /// Point the keyboard at pane `index`, counting from the top left, and
    /// show which that is; see `reveal_header`. With a pane maximized, the
    /// chosen one takes the page (`choose`).
    pub(super) fn on_activate_pane(
        &mut self,
        action: &keys::ActivatePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.slots.get(action.index).map(|slot| slot.key.clone()) {
            self.choose(&key, window, cx);
            self.reveal_active(&key, cx);
        }
    }

    pub(super) fn on_next_pane(
        &mut self,
        _: &keys::NextPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active(1, window, cx);
    }

    pub(super) fn on_previous_pane(
        &mut self,
        _: &keys::PreviousPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active(-1, window, cx);
    }

    /// Move the active pane along the grid, wrapping at either end, and show
    /// which it landed on — taking the page with it, while a pane has it, so
    /// `Tab` walks the panes one whole page at a time.
    fn step_active(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.slots.len();
        let Some(current) = self.active_slot() else {
            return;
        };
        let next = (current as isize + delta).rem_euclid(count as isize) as usize;
        let key = self.slots[next].key.clone();
        self.choose(&key, window, cx);
        self.reveal_active(&key, cx);
    }

    /// `Shift+←`: the active pane swapped with the one before it in the
    /// order; see `move_active`.
    pub(super) fn on_move_pane_earlier(
        &mut self,
        _: &keys::MovePaneEarlier,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active(-1, cx);
    }

    /// `Shift+→`: the same with the one after it.
    pub(super) fn on_move_pane_later(
        &mut self,
        _: &keys::MovePaneLater,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_active(1, cx);
    }

    /// Swap the active pane with its neighbour `delta` places along the
    /// order, as dropping its header there would (`move_pane`), keeping it
    /// the active pane. Not past either end, which is not a wrap
    /// (`stage::moved`); and only where a header could be dragged, with two
    /// panes or more on the page, so not while one is maximized and the
    /// move would be to somewhere nobody can see. Nor while a header is
    /// being dragged: that is a move already, and this one would end it.
    fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.cells().len() < 2 || self.pane_move.is_some() {
            return;
        }
        let Some(index) = self.active_slot() else {
            return;
        };
        let Some(neighbour) = stage::moved(index, delta, self.slots.len()) else {
            return;
        };
        let from = self.slots[index].key.clone();
        let onto = self.slots[neighbour].key.clone();
        self.move_pane(&from, &onto, cx);
    }

    /// Reveal the header of the pane a pane key just chose, with more than
    /// one pane on screen. With one, which pane the keys talk to is the
    /// answer to a question nobody asked — the reason a lone pane's header
    /// is never underlined either.
    fn reveal_active(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.slots.len() > 1 {
            self.reveal_header(key, cx);
        }
    }

    /// One step out on the browse page: a channel's page, a search or a
    /// category closes first, since it took the page over; with none open,
    /// back to whatever is playing. `Esc` on the watch page goes the other
    /// way, which makes the key a toggle between the two pages when nothing
    /// else is in the way.
    ///
    /// Out rather than back: it goes to what is under the takeover, however
    /// you got to it, where `Alt+←` goes to wherever you were before. Both
    /// are steps on the trail, so `Alt+←` also takes an `Esc` back.
    pub(super) fn on_step_out(
        &mut self,
        _: &keys::StepOut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let leave = match self.discovery.place() {
            Place::Channel(_) => Some(Action::CloseChannel),
            Place::Search(_) => Some(Action::CloseSearch),
            Place::Category(_) => Some(Action::CloseCategory),
            Place::Tab(_) => None,
        };
        match leave {
            Some(action) => self.on_browse_action(action, window, cx),
            None => self.go_watch(cx),
        }
    }

    /// `P`. On the watch page, the active pane into a window of its own, or
    /// back from one. On the browse page, which has no active pane, the mini
    /// player: every pane it shows out into windows of their own, as its
    /// bar's `Pop out` does — or, with nothing left in it to pop out, every
    /// popped pane back. Which of the two is the bar's own answer
    /// (`mini_pop_out_offered`), so the key pops out exactly when the bar
    /// offers to. Nothing where the pop-out is not offered, and nothing for
    /// a pane with no player to move; see `pop_out`.
    pub(super) fn on_toggle_pop_out(
        &mut self,
        _: &keys::TogglePopOut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page == Page::Browse {
            if self.mini_pop_out_offered() {
                self.pop_out_all_shown(window, cx);
            } else {
                self.pop_in_all(window, cx);
            }
            return;
        }
        let Some(key) = self
            .active_slot()
            .map(|index| self.slots[index].key.clone())
        else {
            return;
        };
        if self.stage.is_popped(&key) {
            self.pop_in(&key, window, cx);
        } else {
            self.pop_out(&key, window, cx);
        }
    }

    /// Back along the trail; see `navigation`.
    pub(super) fn on_navigate_back(
        &mut self,
        _: &keys::NavigateBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_back(window, cx);
    }

    /// Forward along the trail, after going back.
    pub(super) fn on_navigate_forward(
        &mut self,
        _: &keys::NavigateForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_forward(window, cx);
    }
}
