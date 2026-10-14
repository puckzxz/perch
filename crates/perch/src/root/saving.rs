//! Writing the settings and the history down, off the UI thread.
//!
//! Every save is flushed to the disk before it lands (see
//! `settings::writer`), and a flush can take long enough to see, so the
//! root never writes a file itself: each save is a copy of what it holds,
//! handed to the one writer thread, which writes them in the order they
//! were made. A save that fails comes back here to be said. The one save
//! the UI waits for is the last, as the window closes ([`finish_saving`]).
//!
//! [`finish_saving`]: RootView::finish_saving

use std::path::PathBuf;

use gpui::{Context, Task};
use settings::writer::{Failure, Kind, Save, Writer};

use super::RootView;

/// What a failed save is called: its line in the log, and its toast when it
/// gets one.
fn failure_words(kind: Kind) -> (&'static str, Option<&'static str>) {
    match kind {
        Kind::Preferences | Kind::ForgettingSignIn => {
            ("settings: could not save", Some("Could not save settings"))
        }
        Kind::SignOut => (
            "settings: could not forget the sign-in",
            Some("Could not forget the sign-in"),
        ),
        // The log only: this is written every fifteen seconds while a
        // recording plays, and a disk that refuses once will refuse again,
        // which would be a toast every fifteen seconds for as long as it
        // did.
        Kind::History => ("history: could not save", None),
    }
}

impl RootView {
    /// The writer every save of the root's goes through, and the task that
    /// says on screen a save that failed.
    ///
    /// A toast rather than only a log line: a release build's stderr is a
    /// file nobody is watching, and a save that fails silently is a
    /// preference that comes back on the next launch with no explanation.
    /// The line in the log is written on the writer's thread, so it is
    /// there even for a save that failed as the window closed, when no
    /// toast can be.
    pub(super) fn start_saving(
        settings_path: PathBuf,
        history_path: PathBuf,
        cx: &mut Context<Self>,
    ) -> (Writer, Task<()>) {
        let (said, mut toasts) = futures::channel::mpsc::unbounded::<String>();
        let writer = Writer::start(settings_path, history_path, move |failure: Failure| {
            let (logged, toast) = failure_words(failure.kind);
            eprintln!("{logged}: {}", failure.error);
            if let Some(toast) = toast {
                let _ = said.unbounded_send(format!("{toast}: {}", failure.error));
            }
        });
        let pump = cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(text) = toasts.next().await {
                if this
                    .update(cx, |this: &mut RootView, cx| this.toast(text, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        (writer, pump)
    }

    /// Write the preferences down: a copy, to the writer, which keeps the
    /// sign-in the file has (`Settings::save_preferences`).
    pub(super) fn save_settings(&mut self) {
        self.saves.save(Save::Preferences(self.settings.clone()));
    }

    /// Wait for every save handed to the writer to land. The window is
    /// closing, and the process ends with it without waiting for the
    /// writer's thread, so the last saves — where the window was, where
    /// each recording had got to — would otherwise be lost. The one place
    /// the UI thread waits on the disk, at the one moment nothing is left
    /// on screen to stop.
    pub(crate) fn finish_saving(&self) {
        self.saves.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every failure is logged, and only the history's goes without a
    /// toast: each of the others is something the user did, and would
    /// otherwise come back on the next launch without a word.
    #[test]
    fn every_failed_save_is_logged_and_only_the_history_is_not_said() {
        for kind in [
            Kind::Preferences,
            Kind::ForgettingSignIn,
            Kind::SignOut,
            Kind::History,
        ] {
            let (logged, toast) = failure_words(kind);
            assert!(!logged.is_empty());
            assert_eq!(toast.is_none(), kind == Kind::History, "{kind:?}");
            if let Some(toast) = toast {
                assert!(toast.starts_with(char::is_uppercase), "{toast}");
            }
        }
    }
}
