//! One perch per settings file.
//!
//! The app keeps its Twitch session in the settings file, and Twitch's refresh
//! tokens are single-use (see `twitch`). Two copies on one file each spent the
//! same token, and whichever came second was signed out, with nothing on
//! screen to say why. And starting perch while it ran was a second window
//! rather than the first one coming back.
//!
//! So the first copy claims the settings directory and listens, and a later
//! launch hands its arguments over — the running window opens them and comes
//! to the front — and exits. The claim is the operating system's, never a
//! marker file: a named pipe on Windows, which exists exactly as long as a
//! process holds it, and `flock` on Unix, which the kernel lets go of with
//! the process. A crash leaves nothing behind to clear.
//!
//! A handover waits to be answered. The running copy can be on its way out —
//! the window closed, the process still taking its players down — and a
//! launch handed to it then would vanish. It answers only once the arguments
//! are on their way to a window that still exists; unanswered, the launch
//! tries again, and once the old copy lets go it becomes the first itself.
//!
//! What crosses is the arguments as typed, and `launch` reads them on arrival,
//! so a launch handed over means what it would have meant at startup.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

/// What starting up found.
pub enum Claim {
    /// This is the first copy. It runs, and serves later launches with
    /// [`Listener::serve`].
    First(Listener),
    /// The running copy has this launch's arguments; this one exits.
    Handed,
    /// A copy holds the settings and never answered. This one exits too: a
    /// second running anyway is the race for the sign-in all over again.
    Unanswered,
    /// The operating system would not say. This one runs unguarded, the way
    /// perch always did, with the reason for the log.
    Alone(io::Error),
}

/// How many times a launch tries to hand over or to take over, and the wait
/// between tries: three seconds in all, for a copy still starting to begin
/// listening or one closing to let go.
const ATTEMPTS: u32 = 30;
const RETRY: Duration = Duration::from_millis(100);

/// How long one handover waits to be answered. A running copy answers at
/// once — its listener is up before its window — so this is only the bound on
/// one that has stopped answering altogether.
const ANSWER_WAIT: Duration = Duration::from_secs(5);

/// Claim the settings in `dir` for this process, or hand `args` to the
/// process that has them.
pub fn claim(dir: &Path, args: &[String]) -> Claim {
    for _ in 0..ATTEMPTS {
        match platform::listen(dir) {
            Ok(Some(listener)) => return Claim::First(Listener(listener)),
            Ok(None) => {}
            Err(e) => return Claim::Alone(e),
        }
        match platform::connect(dir) {
            Ok(Some(stream)) => match answered(stream, args) {
                Some(true) => return Claim::Handed,
                // Turned away: that copy is closing. Try again.
                Some(false) => {}
                None => return Claim::Unanswered,
            },
            // Gone between the two calls, or not listening yet.
            Ok(None) => {}
            Err(e) => return Claim::Alone(e),
        }
        thread::sleep(RETRY);
    }
    Claim::Unanswered
}

/// Hand `args` over on `stream` and wait for the answer: whether it came, or
/// `None` once [`ANSWER_WAIT`] has passed with neither an answer nor a refusal.
fn answered(stream: platform::Stream, args: &[String]) -> Option<bool> {
    let message = encode(args);
    let (tx, rx) = mpsc::channel();
    // On a thread of its own because a read on a Windows pipe has no timeout.
    // One left waiting goes with the process, which is about to exit.
    thread::spawn(move || {
        let _ = tx.send(hand_over(stream, &message));
    });
    rx.recv_timeout(ANSWER_WAIT).ok()
}

fn hand_over(mut stream: impl Read + Write, message: &[u8]) -> bool {
    if stream.write_all(message).is_err() {
        return false;
    }
    let mut answer = [0u8; 1];
    matches!(stream.read(&mut answer), Ok(1) if answer[0] == TAKEN)
}

/// The first copy's ear for later launches.
pub struct Listener(platform::Listener);

impl Listener {
    /// Listen for later launches on a thread of its own, giving each one's
    /// arguments to `deliver`, which says whether there is still a window to
    /// take them. Once there is not, the next caller is turned away
    /// unanswered and the claim let go, so a launch that arrives while this
    /// copy closes becomes the next one.
    pub fn serve(self, deliver: impl Fn(Vec<String>) -> bool + Send + Sync + 'static) {
        let deliver = Arc::new(deliver);
        let closing = Arc::new(AtomicBool::new(false));
        let mut listener = self.0;
        let spawned = thread::Builder::new()
            .name("instance".into())
            .spawn(move || loop {
                let stream = match listener.accept() {
                    Ok(Some(stream)) => stream,
                    Ok(None) => continue,
                    Err(e) => {
                        eprintln!("instance: stopped listening for later launches: {e}");
                        return;
                    }
                };
                if closing.load(Ordering::SeqCst) {
                    return;
                }
                let deliver = deliver.clone();
                let closing = closing.clone();
                // Every caller on a thread of its own: one that connects and
                // says nothing must not keep the next one waiting.
                thread::spawn(move || answer(stream, &*deliver, &closing));
            });
        if let Err(e) = spawned {
            eprintln!("instance: could not listen for later launches: {e}");
        }
    }
}

/// Read one launch off `stream`, deliver it, and answer — or, with no window
/// left to deliver to, mark this copy as closing and hang up unanswered. The
/// mark comes first: the caller tries again the moment the line drops, and
/// must find the listener already letting go.
fn answer(
    mut stream: impl Read + Write,
    deliver: &dyn Fn(Vec<String>) -> bool,
    closing: &AtomicBool,
) {
    // Not a perch, or one that gave up waiting: nothing to do.
    let Ok(Some(args)) = read_message(&mut stream) else {
        return;
    };
    if deliver(args) {
        let _ = stream.write_all(&[TAKEN]);
    } else {
        closing.store(true, Ordering::SeqCst);
    }
}

/// The first bytes of a handover, a name and a version, so that anything
/// else that finds the pipe or the socket is ignored rather than read as a
/// launch.
const MAGIC: [u8; 8] = *b"perch\0\x01\0";

/// The answer: the launch is on its way to the window.
const TAKEN: u8 = 1;

/// More than any command line runs to. A caller claiming more is not a perch.
const MAX_MESSAGE: usize = 64 * 1024;

/// A launch as it crosses: [`MAGIC`], the length of what follows, and the
/// arguments, each ended by a NUL — which no argument can hold, on either
/// platform.
fn encode(args: &[String]) -> Vec<u8> {
    let body: Vec<u8> = args
        .iter()
        .flat_map(|arg| arg.bytes().chain(std::iter::once(0)))
        .collect();
    let mut message = MAGIC.to_vec();
    message.extend_from_slice(&(body.len() as u32).to_le_bytes());
    message.extend_from_slice(&body);
    message
}

/// The arguments [`encode`] wrote, or `None` for something that is not a
/// launch at all.
fn read_message(reader: &mut impl Read) -> io::Result<Option<Vec<String>>> {
    let mut magic = [0u8; MAGIC.len()];
    reader.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Ok(None);
    }
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Ok(None);
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let Ok(text) = String::from_utf8(body) else {
        return Ok(None);
    };
    Ok(Some(
        text.split_terminator('\0').map(String::from).collect(),
    ))
}

/// Bring the window to the front, out of the taskbar or the Dock if it was
/// minimised. See the Windows half for why it is not simply
/// `Window::activate_window` there.
#[cfg(windows)]
pub fn bring_forward(window: &gpui::Window, _cx: &mut gpui::App) {
    windows::bring_forward(window);
}

#[cfg(not(windows))]
pub fn bring_forward(window: &gpui::Window, cx: &mut gpui::App) {
    cx.activate(true);
    window.activate_window();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    /// A directory of the test's own, so the claims of tests running at once
    /// — and of a perch the developer has open — never meet.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("perch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_launch_reads_back_as_the_arguments_it_was_made_of() {
        for list in [
            Vec::new(),
            args(&["xqc"]),
            args(&["https://www.twitch.tv/videos/1?t=1h", "--volume", "20"]),
            args(&[""]),
            args(&["naïve", "日本語"]),
        ] {
            let message = encode(&list);
            assert_eq!(read_message(&mut message.as_slice()).unwrap(), Some(list));
        }
    }

    #[test]
    fn anything_that_is_not_a_perch_is_ignored() {
        let stranger = b"GET / HTTP/1.1\r\n\r\n";
        assert_eq!(read_message(&mut &stranger[..]).unwrap(), None);

        let mut boastful = MAGIC.to_vec();
        boastful.extend_from_slice(&(MAX_MESSAGE as u32 + 1).to_le_bytes());
        assert_eq!(read_message(&mut boastful.as_slice()).unwrap(), None);

        assert!(read_message(&mut &MAGIC[..4]).is_err(), "cut short");
    }

    /// The whole round, through the operating system: the first claims, a
    /// second hands its arguments over and is answered, and the first has
    /// them.
    #[test]
    fn a_second_launch_hands_its_arguments_to_the_first() {
        let dir = scratch("handover");
        let Claim::First(listener) = claim(&dir, &[]) else {
            panic!("nothing else holds a fresh directory");
        };
        let (tx, rx) = mpsc::channel();
        listener.serve(move |args| tx.send(args).is_ok());

        let launch = args(&["xqc", "--volume", "20"]);
        assert!(matches!(claim(&dir, &launch), Claim::Handed));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), launch);

        // A bare launch is handed over too: it is the window coming forward.
        assert!(matches!(claim(&dir, &[]), Claim::Handed));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), args(&[]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A first copy whose window has gone turns the launch away and lets go,
    /// and the launch becomes the first itself rather than vanishing.
    #[test]
    fn a_launch_handed_to_a_closing_perch_takes_over() {
        let dir = scratch("closing");
        let Claim::First(listener) = claim(&dir, &[]) else {
            panic!("nothing else holds a fresh directory");
        };
        listener.serve(|_| false);

        assert!(matches!(claim(&dir, &args(&["xqc"])), Claim::First(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
