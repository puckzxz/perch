//! Which panes are heard: `Only this one` and `Hear all again`, from a
//! pane's right-click menu (`crate::channel_menu`), which reads what it
//! offers from the panes' hushes as it is drawn.
//!
//! Both go through Mute all's hush (`set_quiet_all`, `VideoView::set_hushed`)
//! and nothing else, so neither ever writes a level, saves one, or moves a
//! slider: a hushed pane keeps the level it was chosen at, and letting the
//! hush go plays it at that level again. What every menu offers is worked
//! out from the hushes alone (`loudness::hear_only`), never kept as a fact of
//! its own; see `loudness::HearOnly` for why.
//!
//! And how every pane sounds: the audio compressor, More's `Audio
//! compressor` row on any pane, one answer for all of them
//! (`toggle_compressor`, `crate::compressor`).

use gpui::Context;

use super::RootView;
use crate::compressor;
use crate::loudness::{self, HearOnly};

impl RootView {
    /// The pane menu's row, pressed on the pane `key` names: with one pane alone
    /// heard, every pane heard again; otherwise this pane heard and every
    /// other held silent.
    ///
    /// Decided from the panes as they are now, not from the words the row
    /// was drawn with, so a press can never do the opposite of what is
    /// heard. The pane chosen is let go of a hush of its own — Mute all's,
    /// say — since hearing it is the point; one muted by hand, at a level
    /// of 0, stays muted, as letting go of Mute all leaves it.
    pub(super) fn hear_only(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.hear_only_now() == HearOnly::Held {
            self.set_quiet_all(false, cx);
            return;
        }
        for slot in &mut self.slots {
            let quiet = slot.key != key;
            slot.quiet = quiet;
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_hushed(quiet, cx));
            }
        }
        cx.notify();
    }

    /// More's `Audio compressor`, pressed on any pane: on for every pane if
    /// it was off, off for every pane if it was on, and remembered. Every
    /// player picks it up between frames (`crate::compressor`), so there is
    /// nothing to tell each pane; the menu that was pressed closes with the
    /// press, and the next one drawn reads the flag.
    pub(super) fn toggle_compressor(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.compressor;
        self.settings.compressor = on;
        compressor::set(on);
        self.save_settings();
        self.toast(
            if on {
                "Audio compressor on"
            } else {
                "Audio compressor off"
            },
            cx,
        );
    }

    /// What a pane's menu offers about hearing one alone, from the panes'
    /// hushes now; see `loudness::hear_only`.
    pub(super) fn hear_only_now(&self) -> HearOnly {
        loudness::hear_only(self.slots.iter().map(|slot| slot.quiet))
    }
}
