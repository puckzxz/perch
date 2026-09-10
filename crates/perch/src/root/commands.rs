//! The palette, as the root drives it: what it can offer right now, moving
//! the selection, and running whichever row is chosen. The rows themselves
//! and the box they sit in are `crate::palette`.

use gpui::{App, Context, IntoElement, KeyDownEvent, Window};
use gpui_component::input::Input;

use super::RootView;
use crate::palette;
use crate::watch::Slot;

impl RootView {
    /// Everything the palette could run right now, in the order it shows them.
    ///
    /// Recomputed per render rather than held: it is a filter over lists this
    /// view already owns, and holding a copy would mean keeping it in step with
    /// a follows poll, a pane closing and every keystroke.
    pub(super) fn palette_entries(&self, cx: &App) -> Vec<palette::Entry> {
        let watching: Vec<String> = self.slots.iter().map(Slot::label).collect();
        palette::entries(
            self.palette_input.read(cx).value().as_ref(),
            &self.follows,
            &self.offline,
            &self.settings.recent,
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
            palette::Command::Videos {
                login,
                display_name,
                user_id,
            } => self.open_channel_page(login, display_name, user_id, cx),
            palette::Command::OpenVideo { id, start_secs } => {
                self.open_video_link(id, start_secs, cx)
            }
            palette::Command::GoBrowse => self.go_browse(cx),
            palette::Command::GoWatch => self.go_watch(cx),
            palette::Command::StopAll => self.stop_all(cx),
            palette::Command::ToggleSidebar => self.toggle_sidebar(window, cx),
            palette::Command::ToggleSettings => self.toggle_settings(window, cx),
            palette::Command::Refresh => self.refresh(cx),
        }
        cx.notify();
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
