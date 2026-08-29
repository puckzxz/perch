//! Run streamlink as a headless byte source.
//!
//! streamlink is the piece that actually understands Twitch — resolving a
//! channel to HLS, and filtering pre-roll and mid-roll ads. That filtering only
//! happens inside streamlink's own HLS pipeline, so resolving a URL with
//! `--stream-url` and playing it elsewhere brings the ads back. Instead we run
//! streamlink with `--player-external-http`, which serves the filtered stream
//! over loopback and starts no player of its own.
//!
//! The supervisor owns the child process and kills it on drop, so closing the
//! window does not leave streamlink running. That covers every teardown perch
//! runs itself; [`job`] covers the ones it does not get to run, where the
//! process dies without destructors and would otherwise strand streamlink.

mod job;
pub mod quality;

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc;
pub use quality::Quality;

/// Progress of one channel's stream.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Asking Twitch which qualities exist.
    Resolving,
    /// Serving on loopback and ready to play.
    Ready {
        url: String,
        quality: String,
        /// Everything this channel offers right now, so the UI can present a
        /// switcher without asking Twitch again.
        available: Vec<String>,
    },
    /// The channel is not broadcasting.
    Offline,
    Failed {
        reason: String,
    },
}

/// A running streamlink process. Dropping it kills the child.
pub struct StreamSupervisor {
    stop: Arc<AtomicBool>,
    child: ChildSlot,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Where the worker parks whichever streamlink process it is currently running,
/// so [`StreamSupervisor::drop`] can kill it.
///
/// *Every* child goes here, not just the one serving the stream. It used to
/// hold only the serving process, which meant that during the resolve phase —
/// the `--version` probes and a full Twitch HLS lookup, the slowest part of
/// opening a channel — the slot was `None` and a teardown's `kill()` had
/// nothing to name.
type ChildSlot = Arc<Mutex<Option<Child>>>;

/// Locate the streamlink executable.
///
/// `STREAMLINK_PATH` wins so a user with an unusual install is never stuck.
fn find_binary(slot: &ChildSlot) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("STREAMLINK_PATH") {
        let path = PathBuf::from(explicit);
        return runs(&path, slot).then_some(path);
    }

    let mut candidates = vec![PathBuf::from("streamlink")];
    if cfg!(windows) {
        candidates.push(PathBuf::from(
            r"C:\Program Files\Streamlink\bin\streamlink.exe",
        ));
        candidates.push(PathBuf::from(
            r"C:\Program Files (x86)\Streamlink\bin\streamlink.exe",
        ));
    } else {
        candidates.push(PathBuf::from("/usr/bin/streamlink"));
        candidates.push(PathBuf::from("/usr/local/bin/streamlink"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/streamlink"));
    }
    candidates.into_iter().find(|path| runs(path, slot))
}

/// Build a command that does not open a console window.
///
/// streamlink is a console-subsystem program, so Windows hands every one we
/// spawn its own console — and the app itself is windowed, so those are the
/// only console windows a user ever sees. One of them sticks around for as long
/// as the stream plays. `CREATE_NO_WINDOW` suppresses them without touching the
/// pipes: stdout is still captured exactly as before.
fn command(binary: impl AsRef<std::ffi::OsStr>) -> Command {
    // The `mut` is the Windows branch's; everywhere else that block is compiled
    // out and the binding is never written to, which is an `unused_mut` warning
    // — and a warning is a CI failure on the macOS leg of the matrix.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW, from processthreadsapi.h.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// A bare name relies on PATH lookup, which is exactly what we want to test.
fn runs(path: &PathBuf, slot: &ChildSlot) -> bool {
    let mut probe = command(path);
    probe.arg("--version");
    matches!(run_tracked(probe, slot), Ok((_, true)))
}

/// Run a command to completion with its handle parked in `slot`, so a teardown
/// can kill it, and return `(stdout, exited cleanly)`.
///
/// `.status()` and `.output()` are the obvious calls and both are wrong here:
/// each owns the `Child` internally, so nothing outside can ever kill it. That
/// is what made closing a pane mid-resolve hang — `Drop` set its flag, found an
/// empty slot, and then waited on a process it had no way to interrupt.
///
/// The pipe comes out of the child before the handle goes into the slot, so the
/// read below holds no lock: taking one for the length of a Twitch lookup would
/// block the very teardown this exists to serve.
fn run_tracked(mut command: Command, slot: &ChildSlot) -> std::io::Result<(Vec<u8>, bool)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let stdout = child.stdout.take();
    // Before the handle goes into the slot, so a child is under the job for as
    // much of its life as this can manage.
    job::track(&child);
    *slot.lock().unwrap() = Some(child);

    let mut captured = Vec::new();
    if let Some(mut stdout) = stdout {
        let _ = stdout.read_to_end(&mut captured);
    }

    // Taken out from under the lock *before* waiting, on its own line rather
    // than as a temporary inside the `map`. A guard created in a `let`
    // initializer lives to the end of the statement, so the one-liner form held
    // the lock across `wait()` — and `Drop`'s first act after setting `stop` is
    // to take that same lock, on the UI thread. That is the freeze this
    // function exists to prevent, two lines under the comment promising it does.
    let child = slot.lock().unwrap().take();

    // A kill closes the pipe, which is what ends the read above — and it also
    // takes the handle, so by here there may be nothing left to reap.
    let status = child.map(|mut child| child.wait());
    Ok((
        captured,
        matches!(status, Some(Ok(status)) if status.success()),
    ))
}

/// Kill and reap whatever is parked in `slot`, if anything still is.
fn kill_parked(slot: &ChildSlot) {
    // Out from under the lock before the wait, for the reason `run_tracked`
    // spells out.
    let child = slot.lock().unwrap().take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Ask the OS for an unused port by binding and immediately releasing it.
///
/// There is a small race before streamlink binds, but the alternative is
/// hard-coding a port and colliding with a second window.
fn free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// How to open one channel.
#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    /// Explicit streamlink quality name. `None` picks one from the pane size,
    /// which is usually cheaper - see [`quality`].
    pub quality: Option<String>,
    /// The twitch.tv `auth-token` cookie. Unlocks subscriber-only qualities and
    /// suppresses ads. Comes from settings; `TWITCH_AUTH_TOKEN` is a fallback so
    /// it can be supplied without writing it to disk.
    pub auth_token: Option<String>,
}

impl StreamOptions {
    fn token(&self) -> Option<String> {
        self.auth_token
            .clone()
            .or_else(|| std::env::var("TWITCH_AUTH_TOKEN").ok())
            .filter(|token| !token.is_empty())
    }
}

/// Shared arguments for both the quality probe and the serving run.
fn twitch_args(channel: &str, options: &StreamOptions) -> Vec<String> {
    let mut args = vec![
        // Twitch's higher tiers are HEVC/AV1 via Enhanced Broadcasting, and
        // streamlink filters to h264 unless told otherwise, which silently hides
        // 1440p and 4K from the quality list.
        "--twitch-supported-codecs".into(),
        "h264,h265,av1".into(),
    ];

    // A full account credential, and it goes on the child's command line.
    //
    // That is worth stating plainly rather than leaving to be discovered. On
    // Windows a process's command line lives in its PEB and is readable by any
    // process running as the same user, and it is picked up by Process
    // Explorer, by EDR agents, and by Event 4688 where command-line auditing is
    // switched on. The at-rest copy in `settings.json` needs the same access to
    // read, so this adds no new attacker — but unlike a file in AppData, a
    // command line is the kind of thing that gets shipped off the machine to a
    // log server.
    //
    // streamlink's `--config` would take the header out of argv, at the cost of
    // writing the credential to a second file and having to delete it. Left as
    // argv on purpose; the README says so under "The two Twitch tokens".
    if let Some(token) = options.token() {
        args.push("--twitch-api-header".into());
        args.push(format!("Authorization=OAuth {token}"));
    }

    args.push(format!("twitch.tv/{channel}"));
    args
}

/// Which qualities the channel currently offers, newest info from Twitch.
///
/// This is the slow half of opening a channel — a full Twitch HLS resolve, with
/// no timeout of its own — so the child goes in `slot` and a teardown can end it.
fn list_qualities(
    binary: &PathBuf,
    channel: &str,
    options: &StreamOptions,
    slot: &ChildSlot,
) -> Result<Vec<String>, String> {
    let mut args = vec!["--json".to_string()];
    args.extend(twitch_args(channel, options));

    let mut probe = command(binary);
    probe.args(&args);
    let (stdout, _) =
        run_tracked(probe, slot).map_err(|e| format!("could not run streamlink: {e}"))?;

    let json: serde_json::Value = serde_json::from_slice(&stdout)
        .map_err(|e| format!("streamlink returned unreadable JSON: {e}"))?;

    if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
        // streamlink words the offline case as "no playable streams".
        if error.contains("No playable streams") {
            return Ok(Vec::new());
        }
        return Err(error.to_string());
    }

    Ok(json
        .get("streams")
        .and_then(|s| s.as_object())
        .map(|streams| streams.keys().cloned().collect())
        .unwrap_or_default())
}

impl StreamSupervisor {
    /// Resolve `channel`, pick a quality suited to a pane `pane_height` tall,
    /// and serve it on loopback.
    pub fn start(
        channel: String,
        pane_height: u32,
        options: StreamOptions,
    ) -> (Self, mpsc::UnboundedReceiver<StreamEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let child: ChildSlot = Arc::new(Mutex::new(None));

        let thread = std::thread::Builder::new()
            .name("streamlink".into())
            .spawn({
                let stop = stop.clone();
                let child = child.clone();
                move || run(channel, pane_height, options, tx, stop, child)
            })
            .expect("failed to spawn streamlink supervisor");

        (
            Self {
                stop,
                child,
                thread: Some(thread),
            },
            rx,
        )
    }
}

impl Drop for StreamSupervisor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Killing the child also closes its stdout, which ends whichever read
        // the worker is in — the resolve probes as well as the serving loop,
        // now that every child is parked in the slot. Waiting here is bounded:
        // the process is already dying.
        kill_parked(&self.child);
        // Deliberately *not* joined. Every drop site is on the UI thread —
        // closing a pane, going back to browse, changing quality — and the
        // worker can be somewhere the kill above does not reach, most obviously
        // `find_binary` when streamlink is missing. Joining there froze the
        // window for as long as that took, which is exactly the moment the user
        // has asked for something to go away.
        //
        // Nothing here needs ordered teardown: the worker writes to an
        // unbounded channel whose receiver going away is not an error, and it
        // checks `stop` between phases, so it retires on its own shortly after.
        drop(self.thread.take());
    }
}

fn run(
    channel: String,
    pane_height: u32,
    options: StreamOptions,
    tx: mpsc::UnboundedSender<StreamEvent>,
    stop: Arc<AtomicBool>,
    child_slot: ChildSlot,
) {
    let Some(binary) = find_binary(&child_slot) else {
        let _ = tx.unbounded_send(StreamEvent::Failed {
            reason: "streamlink not found. Install it, or set STREAMLINK_PATH.".into(),
        });
        return;
    };
    // `stop` is checked between every phase, not only inside the serving loop.
    // Teardown no longer joins this thread, so nothing outside is waiting for
    // it — but a pane closed mid-resolve should not go on to start a stream
    // nobody is watching.
    if stop.load(Ordering::Relaxed) {
        return;
    }

    let _ = tx.unbounded_send(StreamEvent::Resolving);

    let available = match list_qualities(&binary, &channel, &options, &child_slot) {
        Ok(list) => list,
        Err(reason) => {
            let _ = tx.unbounded_send(StreamEvent::Failed { reason });
            return;
        }
    };
    if stop.load(Ordering::Relaxed) {
        return;
    }
    if available.is_empty() {
        let _ = tx.unbounded_send(StreamEvent::Offline);
        return;
    }

    // Only real resolutions; "best"/"worst"/"audio_only" are aliases and
    // would just be duplicates in a menu.
    let playable: Vec<String> = {
        let mut list: Vec<(u32, String)> = available
            .iter()
            .filter_map(|name| quality::parse_quality(name).map(|q| (q.height, name.clone())))
            .collect();
        list.sort_by_key(|(height, _)| std::cmp::Reverse(*height));
        list.into_iter().map(|(_, name)| name).collect()
    };

    let resolved = match &options.quality {
        Some(preference) => quality::select_named(&available, preference, pane_height),
        None => quality::select(&available, pane_height),
    };
    let Some(chosen) = resolved else {
        let _ = tx.unbounded_send(StreamEvent::Failed {
            reason: format!("no video qualities offered (got {available:?})"),
        });
        return;
    };

    let port = match free_port() {
        Ok(port) => port,
        Err(e) => {
            let _ = tx.unbounded_send(StreamEvent::Failed {
                reason: format!("could not reserve a port: {e}"),
            });
            return;
        }
    };

    let mut args = vec![
        "--player-external-http".to_string(),
        "--player-external-http-port".to_string(),
        port.to_string(),
        // Loopback only: the default binds every interface, which would put the
        // stream on the local network.
        "--player-external-http-interface".to_string(),
        "127.0.0.1".to_string(),
        "--twitch-low-latency".to_string(),
    ];
    args.extend(twitch_args(&channel, &options));
    args.push(chosen.name.clone());

    let spawned = command(&binary)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut process = match spawned {
        Ok(process) => process,
        Err(e) => {
            let _ = tx.unbounded_send(StreamEvent::Failed {
                reason: format!("could not start streamlink: {e}"),
            });
            return;
        }
    };

    let stdout = process.stdout.take();
    let stderr = process.stderr.take();
    // This is the one that matters: the long-lived serving process, the one
    // found still running hours after perch was gone.
    job::track(&process);
    *child_slot.lock().unwrap() = Some(process);

    // The one window `Drop` cannot cover, closed here.
    //
    // Between the `stop` check above and the park on the line before this, the
    // slot is empty. A `Drop` landing in there sets the flag, finds nothing to
    // kill, and — since it no longer joins — returns. This thread would then
    // spawn streamlink anyway and leave it running: `Child`'s own `Drop` does
    // not kill, so returning from here would orphan the process for the life of
    // the session. Re-reading the flag *after* parking is what makes the pair
    // safe in either order.
    if stop.load(Ordering::Relaxed) {
        kill_parked(&child_slot);
        return;
    }

    // streamlink announces the server on stdout, so watch for that rather than
    // probing the port: a probe connection would be a real client, and this
    // also tells us when the channel turns out to be offline.
    let url = format!("http://127.0.0.1:{port}/");
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if stop.load(Ordering::Relaxed) {
                kill_parked(&child_slot);
                return;
            }
            if line.contains("Starting server, access with one of") {
                let _ = tx.unbounded_send(StreamEvent::Ready {
                    url: url.clone(),
                    quality: chosen.name.clone(),
                    available: playable.clone(),
                });
            }
        }
    }

    if stop.load(Ordering::Relaxed) {
        kill_parked(&child_slot);
        return;
    }

    // stdout closed: the child exited. Its stderr says why.
    let reason = stderr
        .map(|stderr| {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .filter(|line| line.contains("error"))
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "streamlink exited".to_string());

    if reason.contains("No playable streams") {
        let _ = tx.unbounded_send(StreamEvent::Offline);
    } else {
        let _ = tx.unbounded_send(StreamEvent::Failed { reason });
    }
}
