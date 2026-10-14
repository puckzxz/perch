//! Saving off the UI thread, in the order the saves were made.
//!
//! Every write of `settings.json` and `history.json` is flushed to the disk
//! before its rename (`write_atomically`), which is what keeps a power cut
//! from emptying either file; the settings file holds the sign-in, and every
//! save of it writes the tokens back, so none of its saves may go unflushed.
//! A flush waits for the disk, though, and on a busy or spinning one long
//! enough to see. Most saves are the app's UI's — a preference, a pin, a
//! recording's place every fifteen seconds while one plays — and made on
//! the UI thread each of them stopped the window for as long as the disk
//! took. So the app hands each save, as a copy of what it holds, to a
//! [`Writer`]: one thread that writes them one at a time in the order they
//! were handed over, so the newest copy is the last one written and the one
//! left on disk. The sign-in worker writes its tokens on its own thread
//! already and does not come through here; the file's lock (`file_lock`)
//! orders it against this thread as it did against the UI.
//!
//! Two things wait for the writer rather than race it. Whatever reads the
//! settings file back must not read it before a save it depends on has
//! landed: the sign-in worker reads the client id and the tokens as it
//! starts, and a new client id or a Sign out handed over just before it has
//! to be on disk first ([`Writer::landed`]). And the app's last saves, as
//! its window closes, have to land before the process ends, which does not
//! wait for this thread ([`Writer::flush`]).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::history::History;
use crate::{Error, Settings};

/// One save, holding the copy it writes.
#[derive(Debug, Clone)]
pub enum Save {
    /// [`Settings::save_preferences`] of this copy: everything but the
    /// sign-in, which is kept as the file has it.
    Preferences(Settings),
    /// [`Settings::save_forgetting_sign_in`] of this copy: a new client id,
    /// whose old tokens mean nothing.
    ForgettingSignIn(Settings),
    /// [`Settings::sign_out`]: the tokens forgotten, and nothing else
    /// changed.
    SignOut,
    /// [`History::save`] of this copy.
    History(History),
}

/// Which save one was, for the words a failure of it is said in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Preferences,
    ForgettingSignIn,
    SignOut,
    History,
}

/// A save that did not land, and why.
#[derive(Debug)]
pub struct Failure {
    pub kind: Kind,
    pub error: Error,
}

impl Save {
    pub fn kind(&self) -> Kind {
        match self {
            Save::Preferences(_) => Kind::Preferences,
            Save::ForgettingSignIn(_) => Kind::ForgettingSignIn,
            Save::SignOut => Kind::SignOut,
            Save::History(_) => Kind::History,
        }
    }

    fn write(&self, settings_path: &Path, history_path: &Path) -> Result<(), Error> {
        match self {
            Save::Preferences(settings) => settings.save_preferences(settings_path),
            Save::ForgettingSignIn(settings) => settings.save_forgetting_sign_in(settings_path),
            Save::SignOut => Settings::sign_out(settings_path),
            Save::History(history) => history.save(history_path),
        }
    }
}

/// What the writer's thread is handed: a save, or a word to send back once
/// everything handed over before it is done. The save is boxed: a copy of
/// the settings is a few hundred bytes, and the word one pointer.
enum Job {
    Save(Box<Save>),
    Landed(Sender<()>),
}

/// The one thread the app's saves are written on; see the module.
///
/// Dropping it lets the thread write whatever it still holds and end. It is
/// not joined, since it is dropped on the UI thread and a join there would
/// wait on the disk this exists to keep the UI from waiting on; the way out
/// waits with [`flush`](Self::flush) instead.
pub struct Writer {
    jobs: Sender<Job>,
}

impl Writer {
    /// Start the thread, writing the settings to `settings_path` and the
    /// history to `history_path`. `failed` hears each save that did not
    /// land, on the writer's thread, so whatever it says is said even when
    /// the save was one of the last.
    pub fn start(
        settings_path: PathBuf,
        history_path: PathBuf,
        mut failed: impl FnMut(Failure) + Send + 'static,
    ) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || {
                // Ends once the `Writer` is dropped and everything it handed
                // over has been taken: a std channel hands over what was
                // queued before its sender went.
                for job in queue {
                    match job {
                        Job::Save(save) => {
                            if let Err(error) = save.write(&settings_path, &history_path) {
                                failed(Failure {
                                    kind: save.kind(),
                                    error,
                                });
                            }
                        }
                        Job::Landed(tell) => {
                            let _ = tell.send(());
                        }
                    }
                }
            })
            .expect("failed to spawn the settings writer");
        Self { jobs }
    }

    /// Hand `save` over, to be written after everything handed over before
    /// it. Returns at once.
    pub fn save(&self, save: Save) {
        // Fails only once the thread has gone, which only a panic brings
        // about; it is in the crash file then.
        let _ = self.jobs.send(Job::Save(Box::new(save)));
    }

    /// Word that everything handed over until now has been written, or has
    /// failed, for whoever has to read the file after it. Returns at once;
    /// the waiting is the [`Landed`]'s.
    pub fn landed(&self) -> Landed {
        let (tell, told) = mpsc::channel();
        // A writer that has gone drops `tell` here, which `Landed` takes as
        // landed: nothing more is coming from it.
        let _ = self.jobs.send(Job::Landed(tell));
        Landed(told)
    }

    /// Wait until everything handed over until now has been written. For
    /// the way out only, where the process would otherwise end before the
    /// last saves land; anywhere else it is the wait on the disk this
    /// thread exists to keep off the caller.
    pub fn flush(&self) {
        self.landed().wait();
    }
}

/// How often [`Landed::wait_unless`] asks whether to give up.
const LOOK_EVERY: Duration = Duration::from_millis(100);

/// The word from [`Writer::landed`], to wait for.
pub struct Landed(Receiver<()>);

impl Landed {
    /// Wait for it.
    pub fn wait(self) {
        let _ = self.0.recv();
    }

    /// Wait for it, giving up as soon as `stopped` says to, which is asked
    /// every [`LOOK_EVERY`]: for a thread that may be told to stop while it
    /// waits. Returns whether the saves landed.
    pub fn wait_unless(self, stopped: impl Fn() -> bool) -> bool {
        loop {
            match self.0.recv_timeout(LOOK_EVERY) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return true,
                Err(RecvTimeoutError::Timeout) => {
                    if stopped() {
                        return false;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Watched;
    use crate::OAuthTokens;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("perch-writer-tests")
            .join(format!("{name}.json"))
    }

    /// A writer whose failures land in the receiver it comes with.
    fn writer(name: &str) -> (Writer, PathBuf, PathBuf, Receiver<Failure>) {
        let settings = temp_file(&format!("{name}-settings"));
        let history = temp_file(&format!("{name}-history"));
        let _ = std::fs::remove_file(&settings);
        let _ = std::fs::remove_file(&history);
        let (tell, failures) = mpsc::channel();
        let writer = Writer::start(settings.clone(), history.clone(), move |failure| {
            let _ = tell.send(failure);
        });
        (writer, settings, history, failures)
    }

    /// The newest copy handed over is the one on disk: a run of saves,
    /// each from a newer copy than the last, ends at the last of them.
    #[test]
    fn saves_land_in_the_order_they_were_made() {
        let (writer, settings_path, history_path, failures) = writer("in-order");
        let mut settings = Settings::default();
        let mut history = History::default();
        for round in 1..=30u8 {
            settings.volume = round;
            writer.save(Save::Preferences(settings.clone()));
            history.opened(Watched {
                id: round.to_string(),
                ..Watched::default()
            });
            writer.save(Save::History(history.clone()));
        }
        writer.flush();

        assert_eq!(Settings::load(&settings_path).unwrap().volume, 30);
        let stored = History::load(&history_path).unwrap();
        assert_eq!(stored.videos.first().map(|w| w.id.as_str()), Some("30"));
        assert!(failures.try_recv().is_err(), "a save failed");
        let _ = std::fs::remove_file(&settings_path);
        let _ = std::fs::remove_file(&history_path);
    }

    /// What the sign-in worker waits for as it starts: a Sign out, or a
    /// new client id, handed over before it asked is on disk once it is
    /// told, so it never starts with the tokens just forgotten.
    #[test]
    fn what_was_handed_over_is_on_disk_once_it_has_landed() {
        let (writer, path, _, _) = writer("landed");
        let mut signed_in = Settings::default();
        signed_in.credentials.client_id = Some("old-app".into());
        signed_in.credentials.oauth = Some(OAuthTokens {
            refresh_token: "old".into(),
            ..OAuthTokens::default()
        });
        signed_in.save(&path).unwrap();

        writer.save(Save::SignOut);
        assert!(writer.landed().wait_unless(|| false));
        assert_eq!(Settings::load(&path).unwrap().credentials.oauth, None);

        let mut new_app = signed_in.clone();
        new_app.credentials.client_id = Some("new-app".into());
        writer.save(Save::ForgettingSignIn(new_app));
        writer.landed().wait();
        let stored = Settings::load(&path).unwrap();
        assert_eq!(stored.credentials.client_id.as_deref(), Some("new-app"));
        assert_eq!(stored.credentials.oauth, None);
        let _ = std::fs::remove_file(&path);
    }

    /// A worker stopped while it waits does not wait on: a save held up
    /// behind the file's lock keeps the word from coming, and being told
    /// to stop ends the wait anyway.
    #[test]
    fn a_wait_gives_up_when_told_to() {
        let (writer, path, _, _) = writer("gives-up");
        let held = crate::file_lock();
        writer.save(Save::Preferences(Settings::default()));
        assert!(!writer.landed().wait_unless(|| true), "it waited on");
        drop(held);
        assert!(writer.landed().wait_unless(|| false));
        let _ = std::fs::remove_file(&path);
    }

    /// A save that cannot land is said, as the kind of save it was; the
    /// ones after it still land.
    #[test]
    fn a_failed_save_is_reported_as_what_it_was() {
        // A file where the settings' directory would be.
        let blocker = temp_file("not-a-directory");
        std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        std::fs::write(&blocker, "").unwrap();
        let history_path = temp_file("after-a-failure-history");
        let _ = std::fs::remove_file(&history_path);
        let (tell, failures) = mpsc::channel();
        let writer = Writer::start(
            blocker.join("settings.json"),
            history_path.clone(),
            move |failure| {
                let _ = tell.send(failure);
            },
        );

        writer.save(Save::SignOut);
        writer.save(Save::History(History::default()));
        writer.flush();

        let failure = failures.try_recv().expect("the failure was not said");
        assert_eq!(failure.kind, Kind::SignOut);
        assert!(failures.try_recv().is_err(), "the history save failed too");
        assert!(History::load(&history_path).is_ok());
        let _ = std::fs::remove_file(&blocker);
        let _ = std::fs::remove_file(&history_path);
    }

    /// Dropping the writer does not drop what was handed to it.
    #[test]
    fn a_dropped_writer_still_writes_what_it_holds() {
        let (writer, path, _, _) = writer("dropped");
        let settings = Settings {
            volume: 77,
            ..Settings::default()
        };
        writer.save(Save::Preferences(settings));
        let landed = writer.landed();
        drop(writer);
        landed.wait();
        assert_eq!(Settings::load(&path).unwrap().volume, 77);
        let _ = std::fs::remove_file(&path);
    }
}
