//! Positioning inside a recording, for the render thread.
//!
//! A recording is not seeked; it is reopened. The player's HLS demuxer cannot
//! seek the fragmented-MP4 playlists Twitch keeps for most large channels —
//! `streamlink::playlist` says why in full — so to reach a moment the app
//! writes a playlist that starts at the segment holding it and tells the
//! player to load that file instead, with a start offset inside its first
//! segment. The player stays: its render context, its decoder, its audio
//! device. Only the file changes, and a second later the picture is where it
//! was asked to be, which is about what a seek cost anyway.
//!
//! Two things about the files. The player's demuxer keeps the playlist it is
//! reading open, and on Windows an open file cannot be renamed over, so every
//! reposition gets a new file and the old one is deleted once the new one has
//! loaded. And a broadcast still being recorded needs its local playlist to
//! grow: that is done by *appending* to the current file, which the demuxer
//! sees on its next re-read, and which never disturbs the part it has already
//! read. A keeper thread does the asking, once per segment.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mpv_frames::Player;
use streamlink::playlist::{self, Playlist};

use crate::seek_bar::Timeline;

/// Where the playlists a recording is played from are written. The temp
/// directory rather than the cache: they are worthless after the pane closes,
/// and a hard crash leaving a few behind costs kilobytes.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("perch").join("playlists")
}

/// Delete whatever an earlier run left in the scratch directory.
///
/// A pane closed a moment before the app is closed leaves its file behind:
/// the render thread that would delete it is still tearing the player down
/// when the process ends. Nothing of an earlier run is wanted, and this runs
/// once, before this run has written anything of its own.
pub fn sweep_scratch() {
    let Ok(entries) = fs::read_dir(scratch_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let _ = fs::remove_file(entry.path());
    }
}

/// What the demuxer may fetch from a playlist it read off disk. Left to
/// itself it refuses to follow a local file to the network — a sound rule for
/// a media player handed a stranger's file, and exactly what this file is
/// for. The list is quoted with mpv's length-prefix form because it has
/// commas in it and so does the option it goes into.
const PROTOCOLS: &str = "file,http,https,tcp,tls,crypto,data";

/// The recording's length as the seek bar sees it, published for the UI
/// thread and kept fresh by the keeper while the broadcast is still going.
#[derive(Clone)]
pub struct Extent {
    length_ms: Arc<AtomicU64>,
    updated: Arc<Mutex<Instant>>,
    growing: Arc<AtomicBool>,
}

impl Extent {
    pub fn new(playlist: &Playlist) -> Self {
        let extent = Self {
            length_ms: Arc::new(AtomicU64::new(0)),
            updated: Arc::new(Mutex::new(Instant::now())),
            growing: Arc::new(AtomicBool::new(false)),
        };
        extent.publish(playlist);
        extent
    }

    pub fn timeline(&self) -> Timeline {
        Timeline {
            length: self.length_ms.load(Ordering::Relaxed) as f64 / 1000.0,
            fetched_at: *self.updated.lock().unwrap(),
            growing: self.growing.load(Ordering::Relaxed),
        }
    }

    fn publish(&self, playlist: &Playlist) {
        self.length_ms.store(
            (playlist.length() * 1000.0).round() as u64,
            Ordering::Relaxed,
        );
        *self.updated.lock().unwrap() = Instant::now();
        self.growing.store(!playlist.ended, Ordering::Relaxed);
    }
}

/// What the render thread and the keeper thread share: the playlist as last
/// read, and the local file the player is reading right now.
struct Shared {
    playlist: Playlist,
    file: PathBuf,
    /// How many of the playlist's segments the file holds, counted from the
    /// playlist's start, and whether the end marker has been written to it.
    written: usize,
    ended_written: bool,
}

/// A recording being played, as the render thread holds it.
pub struct Recording {
    shared: Arc<Mutex<Shared>>,
    dir: PathBuf,
    label: String,
    /// Numbers the files, so a reposition never reuses a name the demuxer
    /// may still have open.
    serial: u32,
    /// Seconds of the recording before the current file's first segment:
    /// what the player's own position is relative to.
    base: f64,
    /// Between telling the player to load a new file and its saying it has:
    /// positions reported in that window belong to the old file.
    loading: bool,
    /// Files the demuxer may still hold. Deleted when they can be.
    stale: Vec<PathBuf>,
    extent: Extent,
}

impl Recording {
    /// Write the first playlist, starting at `start_at`, and say where the
    /// player should open: the file, and the offset inside it.
    pub fn open(
        playlist: Playlist,
        label: &str,
        start_at: f64,
        extent: Extent,
    ) -> std::io::Result<(Self, PathBuf, f64)> {
        let dir = scratch_dir();
        fs::create_dir_all(&dir)?;
        let (index, before) = playlist.segment_at(start_at);
        let path = dir.join(format!("{label}-1.m3u8"));
        fs::write(&path, playlist.render_from(index))?;
        let written = playlist.segments.len();
        let ended_written = playlist.ended;
        let recording = Self {
            shared: Arc::new(Mutex::new(Shared {
                playlist,
                file: path.clone(),
                written,
                ended_written,
            })),
            dir,
            label: label.to_string(),
            serial: 1,
            base: before,
            loading: false,
            stale: Vec::new(),
            extent,
        };
        Ok((recording, path, (start_at - before).max(0.0)))
    }

    /// The options the player needs to read a playlist off disk at all.
    ///
    /// `demuxer=lavf` is the important one. Given a local `.m3u8`, mpv reads
    /// it as one of its *own* playlists — a list of files to play in turn —
    /// and never hands it to the HLS demuxer, so the recording arrives as
    /// thousands of ten-second entries. Forcing ffmpeg's demuxer makes it
    /// HLS again. `live_start_index` is for a broadcast still being recorded,
    /// whose playlist has no end marker: without it the demuxer would start
    /// three segments from the end of whatever the file lists.
    pub fn mpv_options() -> Vec<(String, String)> {
        vec![
            ("demuxer".to_string(), "lavf".to_string()),
            (
                "demuxer-lavf-o".to_string(),
                format!(
                    "live_start_index=0,protocol_whitelist=%{}%{}",
                    PROTOCOLS.len(),
                    PROTOCOLS
                ),
            ),
        ]
    }

    /// Start asking Twitch for the playlist again, once per segment, for as
    /// long as the broadcast is still going. Does nothing for a recording
    /// that is complete.
    pub fn keep_growing(&self, stop: Arc<AtomicBool>) {
        if self.shared.lock().unwrap().playlist.ended {
            return;
        }
        let shared = self.shared.clone();
        let extent = self.extent.clone();
        // Not joined by anyone, like every other worker here: it tests `stop`
        // between sleeps and retires on its own.
        let spawned = std::thread::Builder::new()
            .name("playlist-keeper".into())
            .spawn(move || keep_growing(shared, extent, stop));
        if let Err(e) = spawned {
            eprintln!("video: could not start the playlist keeper: {e}");
        }
    }

    /// Where the recording is, given the player's position in the current
    /// file — or nothing, while the player is between files.
    pub fn position(&self, time_pos: f64) -> Option<f64> {
        (!self.loading).then_some(self.base + time_pos)
    }

    /// Reopen the player at `secs`.
    pub fn reposition(&mut self, player: &Player, secs: f64) -> Result<(), String> {
        let (path, within) = {
            let mut shared = self.shared.lock().unwrap();
            let (index, before) = shared.playlist.segment_at(secs);
            self.serial += 1;
            let path = self
                .dir
                .join(format!("{}-{}.m3u8", self.label, self.serial));
            fs::write(&path, shared.playlist.render_from(index))
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            let old = std::mem::replace(&mut shared.file, path.clone());
            shared.written = shared.playlist.segments.len();
            shared.ended_written = shared.playlist.ended;
            self.stale.push(old);
            self.base = before;
            (path, (secs - before).max(0.0))
        };
        player
            .load(&path.to_string_lossy(), within)
            .map_err(|e| e.to_string())?;
        self.loading = true;
        Ok(())
    }

    /// The player says it is now playing `path`. Only the file this recording
    /// last asked for counts: with two repositions in quick succession the
    /// first file may load and be replaced before the second lands, and a
    /// position read against the wrong base would put the bar a segment out.
    pub fn now_playing(&mut self, path: &str) {
        let current = self.shared.lock().unwrap().file.clone();
        if current.to_string_lossy() != path {
            return;
        }
        self.loading = false;
        self.stale.retain(|path| fs::remove_file(path).is_err());
    }

    /// Delete what was written. Called once the player has been torn down,
    /// which is when the demuxer lets go of the current file.
    pub fn finish(self) {
        let current = self.shared.lock().unwrap().file.clone();
        for path in self.stale.iter().chain(std::iter::once(&current)) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Sleep in slices so a closing pane does not wait out a whole segment.
fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    !stop.load(Ordering::Relaxed)
}

/// The keeper: ask for the playlist again, append whatever is new to the
/// file the player is reading, and stop once the broadcast has.
fn keep_growing(shared: Arc<Mutex<Shared>>, extent: Extent, stop: Arc<AtomicBool>) {
    loop {
        let (url, wait, ended) = {
            let shared = shared.lock().unwrap();
            (
                shared.playlist.url.clone(),
                shared.playlist.refresh_interval(),
                shared.playlist.ended,
            )
        };
        if ended || !sleep_unless_stopped(wait, &stop) {
            return;
        }
        // Outside the lock: a request is seconds on a bad day, and the render
        // thread must be able to reposition meanwhile.
        let fresh = match playlist::fetch(&url) {
            Ok(fresh) => fresh,
            Err(reason) => {
                eprintln!("video: could not refresh the playlist: {reason}");
                continue;
            }
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }

        let mut shared = shared.lock().unwrap();
        shared.playlist.absorb(fresh);
        let has_new = shared.written < shared.playlist.segments.len();
        let just_ended = shared.playlist.ended && !shared.ended_written;
        if has_new || just_ended {
            let tail = shared.playlist.tail(shared.written);
            let appended = OpenOptions::new()
                .append(true)
                .open(&shared.file)
                .and_then(|mut file| file.write_all(tail.as_bytes()));
            match appended {
                Ok(()) => {
                    shared.written = shared.playlist.segments.len();
                    shared.ended_written = shared.playlist.ended;
                }
                Err(e) => eprintln!("video: could not grow the playlist: {e}"),
            }
        }
        extent.publish(&shared.playlist);
    }
}
