//! A disk-backed image cache.
//!
//! GPUI's `img` element takes a path, so every remote image has to land on disk
//! before it can be drawn. That is not a workaround — emotes are immutable and
//! endlessly repeated, so caching them is what you would build anyway. The same
//! cache serves avatars and stream thumbnails later.
//!
//! Lookups never block. `get_or_request` returns a path if the file is already
//! local and otherwise queues a download and returns `None`, so the UI draws a
//! placeholder and repaints when the ready channel fires.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc as futures_mpsc;

/// Downloads run on a small pool: emote-heavy messages arrive in bursts, but a
/// chat pane never needs more than a handful of images in flight.
const WORKERS: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(15);

pub struct ImageCache {
    dir: PathBuf,
    /// url -> local path, for everything known to be present.
    ready: Arc<Mutex<HashMap<String, PathBuf>>>,
    /// URLs already queued, downloaded, or permanently failed. Failures stay in
    /// here so a broken URL is not retried on every repaint.
    seen: Arc<Mutex<HashSet<String>>>,
    queue: mpsc::Sender<String>,
    _workers: Vec<std::thread::JoinHandle<()>>,
}

/// FNV-1a. Only needs to be stable and collision-resistant enough for filenames.
fn hash_url(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Identify the format from the bytes themselves. Some CDNs answer without a
/// usable content-type, and the file's own magic number never lies.
fn extension_from_magic(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

fn extension_for(content_type: Option<&str>) -> &'static str {
    match content_type.unwrap_or("") {
        t if t.contains("gif") => "gif",
        t if t.contains("png") => "png",
        t if t.contains("webp") => "webp",
        t if t.contains("jpeg") || t.contains("jpg") => "jpg",
        _ => "img",
    }
}

/// Index whatever is already cached from previous runs, keyed by URL hash.
fn scan_existing(dir: &Path) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            found.insert(stem.to_string(), path);
        }
    }
    found
}

impl ImageCache {
    /// Open (creating if needed) a cache in `dir`.
    ///
    /// The returned channel fires whenever a download completes, so callers can
    /// repaint. It carries no payload: the receiver re-queries the cache.
    pub fn new(dir: PathBuf) -> std::io::Result<(Self, futures_mpsc::UnboundedReceiver<()>)> {
        std::fs::create_dir_all(&dir)?;

        let on_disk = scan_existing(&dir);
        let ready: Arc<Mutex<HashMap<String, PathBuf>>> = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let (queue, rx) = mpsc::channel::<String>();
        let (notify, notify_rx) = futures_mpsc::unbounded();

        // One receiver shared by the pool; workers take whichever job is next.
        let rx = Arc::new(Mutex::new(rx));
        let on_disk = Arc::new(on_disk);

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();

        let mut workers = Vec::new();
        for index in 0..WORKERS {
            let rx = rx.clone();
            let ready = ready.clone();
            let dir = dir.clone();
            let agent = agent.clone();
            let notify = notify.clone();

            workers.push(
                std::thread::Builder::new()
                    .name(format!("image-cache-{index}"))
                    .spawn(move || loop {
                        let Ok(url) = ({
                            let guard = rx.lock().unwrap();
                            guard.recv()
                        }) else {
                            return; // sender dropped; cache is going away
                        };

                        match download(&agent, &dir, &url) {
                            Ok(path) => {
                                ready.lock().unwrap().insert(url, path);
                                let _ = notify.unbounded_send(());
                            }
                            Err(e) => eprintln!("image cache: {url}: {e}"),
                        }
                    })
                    .expect("failed to spawn image cache worker"),
            );
        }

        // Seed from disk so a restart does not re-download everything.
        {
            let mut ready = ready.lock().unwrap();
            for (stem, path) in on_disk.iter() {
                ready.insert(format!("stem:{stem}"), path.clone());
            }
        }

        Ok((
            Self {
                dir,
                ready,
                seen,
                queue,
                _workers: workers,
            },
            notify_rx,
        ))
    }

    /// Path to `url` if it is already local, otherwise queue it and return None.
    pub fn get_or_request(&self, url: &str) -> Option<PathBuf> {
        // One lock scope for the whole lookup. Taking the guard twice in a
        // single `if let` would deadlock: the scrutinee's temporary lives to the
        // end of the block, and std's Mutex is not reentrant.
        {
            let mut ready = self.ready.lock().unwrap();
            if let Some(path) = ready.get(url) {
                return Some(path.clone());
            }
            // Files from a previous run are indexed by hash, not by URL.
            let stem = format!("stem:{}", hash_url(url));
            if let Some(path) = ready.get(&stem).cloned() {
                ready.insert(url.to_string(), path.clone());
                return Some(path);
            }
        }

        if self.seen.lock().unwrap().insert(url.to_string()) {
            let _ = self.queue.send(url.to_string());
        }
        None
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn download(agent: &ureq::Agent, dir: &Path, url: &str) -> Result<PathBuf, String> {
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("request failed: {e}"))?;

    let declared = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("read failed: {e}"))?;
    if bytes.is_empty() {
        return Err("empty response".into());
    }

    // Bytes first, header second: the header is advisory, the magic number is not.
    let extension =
        extension_from_magic(&bytes).unwrap_or_else(|| extension_for(declared.as_deref()));

    let path = dir.join(format!("{}.{extension}", hash_url(url)));

    // Write to a temp name first so a crash mid-download cannot leave a
    // truncated file that looks cached forever after.
    let temp = path.with_extension(format!("{extension}.part"));
    std::fs::write(&temp, &bytes).map_err(|e| format!("write failed: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("rename failed: {e}"))?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_filename_safe() {
        let a = hash_url("https://example.test/a");
        assert_eq!(a, hash_url("https://example.test/a"));
        assert_ne!(a, hash_url("https://example.test/b"));
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn identifies_formats_from_magic_bytes() {
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(extension_from_magic(&png), Some("png"));

        assert_eq!(extension_from_magic(b"GIF89a....."), Some("gif"));

        let jpg = [0xFFu8, 0xD8, 0xFF, 0xE0];
        assert_eq!(extension_from_magic(&jpg), Some("jpg"));

        // RIFF, four size bytes, then WEBP.
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(extension_from_magic(&webp), Some("webp"));

        assert_eq!(extension_from_magic(b"nonsense"), None);
        assert_eq!(extension_from_magic(b""), None);
    }

    #[test]
    fn maps_content_types_to_extensions() {
        assert_eq!(extension_for(Some("image/gif")), "gif");
        assert_eq!(extension_for(Some("image/png")), "png");
        assert_eq!(extension_for(Some("image/webp")), "webp");
        assert_eq!(extension_for(None), "img");
    }
}
