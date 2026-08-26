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
//! window does not leave streamlink running.

pub mod quality;

use std::io::{BufRead, BufReader};
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
    Failed { reason: String },
}

/// A running streamlink process. Dropping it kills the child.
pub struct StreamSupervisor {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Locate the streamlink executable.
///
/// `STREAMLINK_PATH` wins so a user with an unusual install is never stuck.
pub fn find_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("STREAMLINK_PATH") {
        let path = PathBuf::from(explicit);
        return runs(&path).then_some(path);
    }

    let mut candidates = vec![PathBuf::from("streamlink")];
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\Streamlink\bin\streamlink.exe"));
        candidates.push(PathBuf::from(
            r"C:\Program Files (x86)\Streamlink\bin\streamlink.exe",
        ));
    } else {
        candidates.push(PathBuf::from("/usr/bin/streamlink"));
        candidates.push(PathBuf::from("/usr/local/bin/streamlink"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/streamlink"));
    }
    candidates.into_iter().find(runs)
}

/// Build a command that does not open a console window.
///
/// streamlink is a console-subsystem program, so Windows hands every one we
/// spawn its own console — and the app itself is windowed, so those are the
/// only console windows a user ever sees. One of them sticks around for as long
/// as the stream plays. `CREATE_NO_WINDOW` suppresses them without touching the
/// pipes: stdout is still captured exactly as before.
fn command(binary: impl AsRef<std::ffi::OsStr>) -> Command {
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
fn runs(path: &PathBuf) -> bool {
    command(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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

    // A full account credential, so it is never logged and never echoed.
    if let Some(token) = options.token() {
        args.push("--twitch-api-header".into());
        args.push(format!("Authorization=OAuth {token}"));
    }

    args.push(format!("twitch.tv/{channel}"));
    args
}

/// Which qualities the channel currently offers, newest info from Twitch.
fn list_qualities(
    binary: &PathBuf,
    channel: &str,
    options: &StreamOptions,
) -> Result<Vec<String>, String> {
    let mut args = vec!["--json".to_string()];
    args.extend(twitch_args(channel, options));

    let output = command(binary)
        .args(&args)
        .output()
        .map_err(|e| format!("could not run streamlink: {e}"))?;

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
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
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));

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
        // Killing the child also closes its stdout, which ends the reader loop.
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    channel: String,
    pane_height: u32,
    options: StreamOptions,
    tx: mpsc::UnboundedSender<StreamEvent>,
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    let Some(binary) = find_binary() else {
        let _ = tx.unbounded_send(StreamEvent::Failed {
            reason: "streamlink not found. Install it, or set STREAMLINK_PATH.".into(),
        });
        return;
    };

    let _ = tx.unbounded_send(StreamEvent::Resolving);

    let available = match list_qualities(&binary, &channel, &options) {
        Ok(list) => list,
        Err(reason) => {
            let _ = tx.unbounded_send(StreamEvent::Failed { reason });
            return;
        }
    };
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
        list.sort_by(|a, b| b.0.cmp(&a.0));
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
    *child_slot.lock().unwrap() = Some(process);

    // streamlink announces the server on stdout, so watch for that rather than
    // probing the port: a probe connection would be a real client, and this
    // also tells us when the channel turns out to be offline.
    let url = format!("http://127.0.0.1:{port}/");
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if stop.load(Ordering::Relaxed) {
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
