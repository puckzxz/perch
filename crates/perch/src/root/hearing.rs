//! Which panes are heard: `Only this one` and `Hear all again`, from a
//! pane's More menu, and the mirror of it every player's menu keeps.
//!
//! Both go through Mute all's hush (`set_quiet_all`, `VideoView::set_hushed`)
//! and nothing else, so neither ever writes a level, saves one, or moves a
//! slider: a hushed pane keeps the level it was chosen at, and letting the
//! hush go plays it at that level again. What every menu offers is worked
//! out from the hushes alone (`loudness::hear_only`), never kept as a fact of
//! its own; see `loudness::HearOnly` for why.

use gpui::Context;

use super::RootView;
use crate::loudness::{self, HearOnly};

impl RootView {
    /// More's row, pressed on the pane `key` names: with one pane alone
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
        self.sync_hear_only(cx);
        cx.notify();
    }

    /// What every pane's More offers about hearing one alone, from the
    /// panes' hushes now; see `loudness::hear_only`.
    pub(super) fn hear_only_now(&self) -> HearOnly {
        loudness::hear_only(self.slots.iter().map(|slot| slot.quiet))
    }

    /// Tell every player what More offers now (`VideoView::hear_only`), the
    /// one writer of that mirror after a player is made. Called after every
    /// change of a pane's hush — Mute all and its undoing, a level somebody
    /// set, which ends a pane's hush, and the two rows here — and from
    /// `restage`, which every change of which panes there are ends in.
    pub(super) fn sync_hear_only(&mut self, cx: &mut Context<Self>) {
        let offer = self.hear_only_now();
        for slot in &self.slots {
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_hear_only(offer, cx));
            }
        }
    }
}
