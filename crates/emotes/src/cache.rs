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
//!
//! Not everything is immutable, though. A channel's preview lives at a *stable*
//! URL whose picture Twitch replaces every few minutes, so caching one by URL
//! forever pins whatever was there the first time you looked — which is how the
//! browse page ended up showing day-old thumbnails. Those go through
//! [`ImageCache::get_or_request_fresh`] instead, which refetches once a copy is
//! stale, and they live in a subdirectory that is emptied at startup so nothing
//! survives into the next run.
//!
//! A refresh lands at a *new* filename rather than overwriting the old one.
//! GPUI decodes an image once per path and caches it there, so replacing the
//! bytes underneath would leave the stale picture on screen — the one thing this
//! is trying to fix. The previous file is deleted as the new one arrives, so a
//! refreshing image costs one file, not one per refresh.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc as futures_mpsc;

/// Downloads run on a small pool: emote-heavy messages arrive in bursts, but a
/// chat pane never needs more than a handful of images in flight.
const WORKERS: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(15);

/// How long to wait before trying a URL again after a failure. Without it a
/// broken URL is retried on every repaint, which is sixty times a second.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Where volatile images live, relative to the cache directory.
const VOLATILE_DIR: &str = "live";

/// Whether a cached image can be trusted to stay the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifetime {
    /// Immutable at its URL — emotes, box art. Kept, and reused across runs.
    Permanent,
    /// Stable URL, changing picture. Refetched when stale, never carried into
    /// the next run.
    Volatile,
}

/// One image the cache knows about.
#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    /// When these bytes arrived. Only meaningful for volatile entries; a
    /// permanent one is never asked how old it is.
    fetched: Instant,
}

/// A download for a worker to run.
struct Job {
    url: String,
    lifetime: Lifetime,
    /// The file this replaces, deleted once the new one is safely on disk.
    replaces: Option<PathBuf>,
}

pub struct ImageCache {
    dir: PathBuf,
    volatile_dir: PathBuf,
    /// url -> what we have, for everything known to be present.
    ready: Arc<Mutex<HashMap<String, Entry>>>,
    /// URLs currently being downloaded. Lookups happen on every repaint, so
    /// without this a single miss would queue the same job sixty times a second.
    inflight: Arc<Mutex<HashSet<String>>>,
    /// When a URL was last attempted, so a failure backs off instead of
    /// spinning. Replaces the old "never retry" set: a transient network blip
    /// should not disable an emote for the rest of the session.
    attempted: Arc<Mutex<HashMap<String, Instant>>>,
    queue: mpsc::Sender<Job>,
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

        // Yesterday's previews are worse than none: emptying this is what stops
        // a restart from resurrecting them.
        let volatile_dir = dir.join(VOLATILE_DIR);
        let _ = std::fs::remove_dir_all(&volatile_dir);
        std::fs::create_dir_all(&volatile_dir)?;

        let on_disk = scan_existing(&dir);
        let ready: Arc<Mutex<HashMap<String, Entry>>> = Arc::new(Mutex::new(HashMap::new()));
        let inflight = Arc::new(Mutex::new(HashSet::new()));
        let attempted = Arc::new(Mutex::new(HashMap::new()));
        let (queue, rx) = mpsc::channel::<Job>();
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
            let inflight = inflight.clone();
            let dir = dir.clone();
            let volatile_dir = volatile_dir.clone();
            let agent = agent.clone();
            let notify = notify.clone();

            workers.push(
                std::thread::Builder::new()
                    .name(format!("image-cache-{index}"))
                    .spawn(move || loop {
                        let Ok(job) = ({
                            let guard = rx.lock().unwrap();
                            guard.recv()
                        }) else {
                            return; // sender dropped; cache is going away
                        };

                        let volatile = job.lifetime == Lifetime::Volatile;
                        let target = if volatile { &volatile_dir } else { &dir };

                        match download(&agent, target, &job.url, volatile) {
                            Ok(path) => {
                                ready.lock().unwrap().insert(
                                    job.url.clone(),
                                    Entry {
                                        path,
                                        fetched: Instant::now(),
                                    },
                                );
                                // Deleted only once the replacement is indexed,
                                // or a failure would leave nothing to draw.
                                if let Some(old) = job.replaces {
                                    let _ = std::fs::remove_file(old);
                                }
                                let _ = notify.unbounded_send(());
                            }
                            Err(e) => eprintln!("image cache: {}: {e}", job.url),
                        }
                        inflight.lock().unwrap().remove(&job.url);
                    })
                    .expect("failed to spawn image cache worker"),
            );
        }

        // Seed from disk so a restart does not re-download everything. Only
        // permanent images are here; the volatile directory was just emptied.
        {
            let mut ready = ready.lock().unwrap();
            for (stem, path) in on_disk.iter() {
                ready.insert(
                    format!("stem:{stem}"),
                    Entry {
                        path: path.clone(),
                        fetched: Instant::now(),
                    },
                );
            }
        }

        Ok((
            Self {
                dir,
                volatile_dir,
                ready,
                inflight,
                attempted,
                queue,
                _workers: workers,
            },
            notify_rx,
        ))
    }

    /// Path to `url` if it is already local, otherwise queue it and return None.
    ///
    /// For images that never change at their address: emotes and box art.
    pub fn get_or_request(&self, url: &str) -> Option<PathBuf> {
        if let Some(entry) = self.lookup(url) {
            return Some(entry.path);
        }
        self.enqueue(url, Lifetime::Permanent, None);
        None
    }

    /// Path to `url`, refetching it if what we hold is older than `max_age`.
    ///
    /// For images whose address is stable but whose content is not — a
    /// channel's live preview being the case this exists for. The stale copy is
    /// still returned while the new one downloads: showing the previous frame
    /// for a moment beats punching a hole in the grid and reflowing it.
    pub fn get_or_request_fresh(&self, url: &str, max_age: Duration) -> Option<PathBuf> {
        // Deliberately not `lookup`: that promotes files left by earlier runs,
        // and a preview from a previous session is exactly what must not be
        // shown. Only what this run fetched counts.
        let known = self.ready.lock().unwrap().get(url).cloned();
        match known {
            Some(entry) if entry.fetched.elapsed() <= max_age => Some(entry.path),
            Some(entry) => {
                self.enqueue(url, Lifetime::Volatile, Some(entry.path.clone()));
                Some(entry.path)
            }
            None => {
                self.enqueue(url, Lifetime::Volatile, None);
                None
            }
        }
    }

    /// What we hold for `url`, promoting a file left by a previous run.
    ///
    /// Permanent images only — see `get_or_request_fresh` for why.
    fn lookup(&self, url: &str) -> Option<Entry> {
        // One lock scope for the whole lookup. Taking the guard twice in a
        // single `if let` would deadlock: the scrutinee's temporary lives to the
        // end of the block, and std's Mutex is not reentrant.
        let mut ready = self.ready.lock().unwrap();
        if let Some(entry) = ready.get(url) {
            return Some(entry.clone());
        }
        // Files from a previous run are indexed by hash, not by URL.
        let stem = format!("stem:{}", hash_url(url));
        let entry = ready.get(&stem).cloned()?;
        ready.insert(url.to_string(), entry.clone());
        Some(entry)
    }

    /// Queue a download, unless one is running or one just failed.
    fn enqueue(&self, url: &str, lifetime: Lifetime, replaces: Option<PathBuf>) {
        {
            let mut attempted = self.attempted.lock().unwrap();
            if let Some(last) = attempted.get(url) {
                if last.elapsed() < RETRY_AFTER {
                    return;
                }
            }
            attempted.insert(url.to_string(), Instant::now());
        }
        // Lookups run on every repaint, so this is what stops a single miss
        // queueing the same job sixty times a second.
        if !self.inflight.lock().unwrap().insert(url.to_string()) {
            return;
        }
        let _ = self.queue.send(Job {
            url: url.to_string(),
            lifetime,
            replaces,
        });
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where volatile images land. Emptied on startup.
    pub fn volatile_dir(&self) -> &Path {
        &self.volatile_dir
    }
}

fn download(agent: &ureq::Agent, dir: &Path, url: &str, unique: bool) -> Result<PathBuf, String> {
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

    // A volatile image needs a filename nobody has drawn yet: GPUI caches a
    // decoded image against its path, so reusing the name would keep the old
    // picture on screen no matter what the bytes say.
    let stem = if unique {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!("{}-{}", hash_url(url), NEXT.fetch_add(1, Ordering::Relaxed))
    } else {
        hash_url(url)
    };
    let path = dir.join(format!("{stem}.{extension}"));

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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("nativetwitch-cache-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The bug this exists to prevent: a channel preview from a previous run
    /// being shown as if it were live. Permanent images must survive the same
    /// restart, or every emote would be refetched on every launch.
    #[test]
    fn a_restart_discards_previews_but_keeps_emotes() {
        let dir = scratch("restart");
        std::fs::create_dir_all(dir.join(VOLATILE_DIR)).unwrap();
        let preview = dir.join(VOLATILE_DIR).join("yesterday.jpg");
        let emote = dir.join("emote.png");
        std::fs::write(&preview, b"stale").unwrap();
        std::fs::write(&emote, b"forever").unwrap();

        let (cache, _ready) = ImageCache::new(dir.clone()).unwrap();

        assert!(!preview.exists(), "a preview survived the restart");
        assert!(emote.exists(), "an emote was thrown away");
        assert!(cache.volatile_dir().is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh copy is handed back without a refetch, and a stale one is still
    /// handed back — the grid keeps the old picture rather than a hole while
    /// the replacement downloads.
    #[test]
    fn staleness_decides_refetching_not_what_is_returned() {
        let dir = scratch("staleness");
        let (cache, _ready) = ImageCache::new(dir.clone()).unwrap();

        let url = "https://example.invalid/preview.jpg";
        let path = dir.join("pretend.jpg");
        std::fs::write(&path, b"pretend").unwrap();
        cache.ready.lock().unwrap().insert(
            url.to_string(),
            Entry {
                path: path.clone(),
                fetched: Instant::now(),
            },
        );

        assert_eq!(
            cache.get_or_request_fresh(url, Duration::from_secs(300)),
            Some(path.clone()),
            "a fresh copy should come straight back"
        );
        assert_eq!(
            cache.get_or_request_fresh(url, Duration::ZERO),
            Some(path),
            "a stale copy should still be shown while it refreshes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Lookups happen on every repaint. Without the in-flight guard one miss
    /// would queue the same download sixty times a second.
    #[test]
    fn a_miss_is_queued_once_not_once_per_repaint() {
        let dir = scratch("queueing");
        let (cache, _ready) = ImageCache::new(dir.clone()).unwrap();

        let url = "https://example.invalid/never-resolves.png";
        for _ in 0..50 {
            assert_eq!(cache.get_or_request(url), None);
        }
        assert_eq!(
            cache.attempted.lock().unwrap().len(),
            1,
            "the same URL was attempted more than once"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
