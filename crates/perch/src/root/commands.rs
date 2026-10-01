//! The palette, as the root drives it: what it can offer right now, moving
//! the selection, and running whichever row is chosen. The rows themselves
//! and the box they sit in are `crate::palette`.

use gpui::{App, Context, IntoElement, KeyDownEvent, Window};
use gpui_component::input::Input;

use super::{Page, RootView};
use crate::channel_page;
use crate::history_page;
use crate::palette;
use crate::video_view::Menu;
use crate::watch::PaneAction;

impl RootView {
    /// Everything the palette could run right now, in the order it shows them.
    ///
    /// Recomputed per render rather than held: it is a filter over lists this
    /// view already owns, and holding a copy would mean keeping it in step with
    /// a follows poll, a pane closing and every keystroke.
    pub(super) fn palette_entries(&self, cx: &App) -> Vec<palette::Entry> {
        let watching: Vec<palette::OpenPane> = self
            .slots
            .iter()
            .map(|slot| palette::OpenPane {
                login: slot.is_live().then(|| slot.channel.clone()),
                // A recording says which kind it is, so two panes on one
                // channel read as two things.
                title: match slot.recording() {
                    Some(video) => format!(
                        "{} ({})",
                        self.display_name(slot),
                        channel_page::kind_tag(video.kind)
                    ),
                    None => self.display_name(slot),
                },
                // A picture, not just a player: one still waiting for its
                // first frame draws no bar, so it has no menu to offer
                // (`VideoView::open_menu`).
                playing: slot.video().is_some_and(|view| view.read(cx).has_picture()),
            })
            .collect();
        palette::entries(
            self.palette_input.read(cx).value().as_ref(),
            &self.follows,
            &self.offline,
            &self.settings.recent,
            &self.history.videos,
            &watching,
            self.can_add(),
        )
    }

    pub(super) fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        self.palette_selected = 0;

        if self.palette_open {
            // Opened on a query from last time, the first thing you type lands
            // in the middle of it.
            self.palette_input
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.palette_input
                .update(cx, |state, cx| state.focus(window, cx));
        } else {
            // Focus has to come back to the root or every shortcut stops
            // working; see the `focus` field.
            self.focus.focus(window);
        }
        cx.notify();
    }

    /// Move the selection, wrapping at both ends.
    pub(super) fn move_palette_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.palette_entries(cx).len();
        if count == 0 {
            return;
        }
        let next = self.palette_selected as isize + delta;
        self.palette_selected = next.rem_euclid(count as isize) as usize;
        cx.notify();
    }

    /// Arrow keys and Escape, handled as key events rather than as bindings.
    ///
    /// A binding cannot win here: the palette's own text field is focused, its
    /// context is deeper than this view's, and the keymap deliberately stands
    /// aside for a focused input — which is the behaviour that keeps typing
    /// working everywhere else. Reading the event on the way past costs nothing
    /// and answers to nobody's precedence rules.
    pub(super) fn on_palette_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.palette_open {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.toggle_palette(window, cx),
            "up" => self.move_palette_selection(-1, cx),
            "down" => self.move_palette_selection(1, cx),
            _ => {}
        }
    }

    pub(super) fn run_selected_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.palette_entries(cx);
        let Some(entry) = entries.get(self.palette_selected).cloned() else {
            return;
        };
        self.run_command(entry.command, window, cx);
    }

    pub(super) fn run_command(
        &mut self,
        command: palette::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Closed first, whatever the command turns out to be: every one of them
        // changes what is on screen, and a palette still sitting over the
        // result is a palette you have to dismiss before you can see it.
        if self.palette_open {
            self.toggle_palette(window, cx);
        }

        match command {
            palette::Command::Watch(channel) => self.open_channel(channel, true, window, cx),
            palette::Command::Add(channel) => self.open_channel(channel, false, window, cx),
            palette::Command::Close(index) => self.close_slot(index, window, cx),
            palette::Command::ChooseQuality(index) => self.choose_quality(index, cx),
            // More's rows by name, down the route More takes, by the pane's
            // key: the same link, the same toast.
            palette::Command::CopyLink(index) => {
                self.pane_command(index, PaneAction::CopyLink, window, cx)
            }
            palette::Command::OpenOnTwitch(index) => {
                self.pane_command(index, PaneAction::OpenOnTwitch, window, cx)
            }
            palette::Command::Videos {
                login,
                display_name,
                user_id,
            } => self.open_channel_page(login, display_name, user_id, window, cx),
            palette::Command::OpenVideo { id, start_secs } => {
                self.open_video_link(id, start_secs, cx)
            }
            // From what the history kept of it, so no lookup and no wait; it
            // opens where it was left, as a recording always does.
            palette::Command::Resume(id) => {
                if let Some(video) = self
                    .history
                    .get(&id)
                    .map(|watched| history_page::video(watched, chrono::Utc::now()))
                {
                    self.open_video(video, true, window, cx);
                }
            }
            palette::Command::ShowTab(tab) => self.go_to_tab(tab, window, cx),
            palette::Command::GoWatch => self.go_watch(cx),
            palette::Command::StopAll => self.stop_all(cx),
            palette::Command::ToggleSidebar => self.toggle_sidebar(window, cx),
            palette::Command::ToggleSettings => self.toggle_settings(window, cx),
            palette::Command::Refresh => self.refresh(cx),
        }
        cx.notify();
    }

    /// What a pane's own control would ask, for the pane at `index` in the
    /// list the palette was drawn from. The palette holds positions; the
    /// route takes keys, so the key is read here, as the row runs.
    fn pane_command(
        &mut self,
        index: usize,
        action: PaneAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.slots.get(index).map(|slot| slot.key.clone()) {
            self.on_pane_action(&key, action, window, cx);
        }
    }

    /// Open the quality menu of the pane at `index`, from the palette.
    ///
    /// The keyboard's way to it, and so also the way posted input can reach
    /// the bar: an open menu holds the bar up wherever the pointer is. From
    /// the browse page it goes back to watching first, with that pane the
    /// one the keys talk to, as a press on its tile would; on the watch page
    /// that pane takes the keys. Any other pane's menu closes, since a menu
    /// opened from here comes with no press elsewhere to dismiss the last.
    /// Nothing at all for a pane with no picture yet, which the palette
    /// offers no row for either: the menu would not open, so neither does
    /// the page change for it.
    fn choose_quality(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        let Some(view) = slot
            .video()
            .filter(|view| view.read(cx).has_picture())
            .cloned()
        else {
            return;
        };
        let key = slot.key.clone();
        if self.page == Page::Watch {
            self.active = Some(key);
        } else {
            self.go_watch_pane(key, cx);
        }
        let others: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.video().cloned())
            .filter(|other| *other != view)
            .collect();
        for other in others {
            other.update(cx, |other, cx| {
                other.close_menu(cx);
            });
        }
        view.update(cx, |video, cx| video.open_menu(Menu::Quality, cx));
    }

    /// The palette, when it is open.
    pub(super) fn palette_sheet(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.palette_open {
            return None;
        }
        let entries = self.palette_entries(cx);
        // Clamped here rather than where it moves: the list shrinks under the
        // cursor as you type, and a selection past the end would run nothing.
        let selected = self.palette_selected.min(entries.len().saturating_sub(1));

        Some(palette::sheet(
            Input::new(&self.palette_input),
            &entries,
            selected,
            move |this: &mut RootView, index, window, cx| {
                this.palette_selected = index;
                this.run_selected_command(window, cx);
            },
            cx,
        ))
    }
}
