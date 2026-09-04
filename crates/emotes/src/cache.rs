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
//!
//! Deleting the file is only half of it. What the UI decoded from that path is
//! keyed on the path rather than on the file, so it outlives the deletion and
//! would be held for the rest of the session - the same picture kept twice
//! over, once per refresh. Every superseded path therefore goes on a list that
//! [`ImageCache::take_retired`] drains, and releasing whatever was decoded from
//! it is the caller's half of the bargain.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

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

/// Extension of a download in progress. It is renamed onto the real name once
/// complete, so one left on disk is the debris of an interrupted run.
const PART_EXTENSION: &str = "part";

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
    /// Files a refresh has superseded, waiting for the caller to release what
    /// it decoded from them. See [`ImageCache::take_retired`].
    ///
    /// A path lands here only once its replacement is in `ready`, never when the
    /// refresh is queued: until the new bytes exist, the old path is exactly
    /// what the UI is still drawing.
    retired: Arc<Mutex<Vec<PathBuf>>>,
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

/// How much of the permanent cache to keep. Emotes and box art are a few KB
/// each, so this is thousands of images — reached only by an install that has
/// been running for a very long time, which is exactly the case that had no
/// bound at all before.
const BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// Index whatever is already cached from previous runs, keyed by URL hash,
/// deleting anything that should not survive into this one.
///
/// Indexing and pruning are one pass on purpose. `read_dir` already walks every
/// entry, and on both platforms the size and mtime come out of that same walk,
/// so bounding the directory costs a stat per file rather than a second scan —
/// and it is what keeps this scan bounded in the first place. Nothing here ever
/// deleted a permanent entry before, so an install accumulated every emote,
/// avatar and box art it had ever seen, and `ImageCache::new` grew slower with
/// every run.
///
/// Two things go: `.part` files, which are only ever left behind by a download
/// interrupted mid-write, and the oldest entries once the total passes
/// [`BUDGET_BYTES`]. Pruning happens *before* the survivors are indexed, so the
/// returned map can never name a file this function has just removed.
fn scan_and_prune(dir: &Path) -> HashMap<String, PathBuf> {
    prune_to(dir, BUDGET_BYTES)
}

/// [`scan_and_prune`] against an explicit budget, so a test does not have to
/// write a quarter of a gigabyte to reach the interesting branch.
fn prune_to(dir: &Path, budget: u64) -> HashMap<String, PathBuf> {
    let mut found = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };

    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    let mut total: u64 = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        // The volatile subdirectory is emptied by the caller; skip it and
        // anything else that is not a plain file.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == PART_EXTENSION) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let size = meta.len();
        total += size;
        files.push((
            path,
            size,
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        ));
    }

    if total > budget {
        // Oldest first. Least-recently-*modified* rather than least-recently-
        // used: nothing here records reads, and for content that never changes
        // at its address, age is the closest honest proxy.
        files.sort_by_key(|(_, _, modified)| *modified);
        let mut index = 0;
        while total > budget && index < files.len() {
            let (path, size, _) = &files[index];
            if std::fs::remove_file(path).is_ok() {
                total -= size;
            }
            index += 1;
        }
        files.drain(..index);
    }

    for (path, _, _) in files {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            found.insert(stem.to_string(), path.clone());
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

        let on_disk = scan_and_prune(&dir);
        let ready: Arc<Mutex<HashMap<String, Entry>>> = Arc::new(Mutex::new(HashMap::new()));
        let inflight = Arc::new(Mutex::new(HashSet::new()));
        let attempted = Arc::new(Mutex::new(HashMap::new()));
        let retired = Arc::new(Mutex::new(Vec::new()));
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
            let retired = retired.clone();

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
                                // or a failure would leave nothing to draw - and
                                // retired in the same breath, for the same
                                // reason: `ready` already names the new file, so
                                // nothing can ask for this one again.
                                retire(&retired, job.replaces);
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
                retired,
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

    /// How many images this run has fetched and indexed.
    ///
    /// For the CPU log, which wants to know whether a busy stretch was the
    /// cache filling up. Grows with distinct URLs seen and never shrinks, so a
    /// number that keeps climbing after the page has settled is a question
    /// worth asking.
    pub fn ready_len(&self) -> usize {
        self.ready.lock().unwrap().len()
    }

    /// How many downloads are in flight right now.
    ///
    /// Should return to zero whenever the app is idle. One that does not is a
    /// job that never completed and never released its slot.
    pub fn inflight_len(&self) -> usize {
        self.inflight.lock().unwrap().len()
    }

    /// Take the paths that refreshes have superseded since the last call.
    ///
    /// Drains: each path is reported exactly once, so this is cheap enough to
    /// call every frame and harmless on a frame that draws none of these images.
    ///
    /// The caller is expected to release whatever it decoded from each path
    /// *before* it next asks this cache for anything. That ordering is what
    /// makes it safe: a path appears here only once `ready` names its
    /// replacement, so a lookup after the drain can only return the new file.
    pub fn take_retired(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.retired.lock().unwrap())
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

/// A replacement has landed: the file it supersedes goes from disk, and its
/// path goes on the retired list.
///
/// Retired whether or not the unlink succeeded, and deliberately so. The delete
/// is best-effort - a scanner holding a handle open is enough to fail it on
/// Windows - but the decoded copy has to be released either way, and `ready`
/// already names the replacement, so nothing can ask for this path again.
/// Retiring without deleting costs one stale file until the volatile directory
/// is emptied at startup; not retiring costs the image for the rest of the
/// session, which is the whole bug.
fn retire(retired: &Mutex<Vec<PathBuf>>, replaced: Option<PathBuf>) {
    let Some(old) = replaced else { return };
    let _ = std::fs::remove_file(&old);
    retired.lock().unwrap().push(old);
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
    let temp = path.with_extension(format!("{extension}.{PART_EXTENSION}"));
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

    /// A superseded file goes from disk *and* on to the retired list. Deleting
    /// it alone is what left the decoded copy behind.
    #[test]
    fn a_replaced_file_is_deleted_and_reported() {
        let dir = scratch("retire");
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("abcdef0123456789-0.jpg");
        std::fs::write(&old, b"yesterday's preview").unwrap();

        let retired = Mutex::new(Vec::new());
        retire(&retired, Some(old.clone()));

        assert!(!old.exists(), "a superseded file was left on disk");
        assert_eq!(retired.lock().unwrap().as_slice(), &[old]);

        // A first fetch replaces nothing, and must retire nothing.
        retire(&retired, None);
        assert_eq!(retired.lock().unwrap().len(), 1);

        // The unlink is best-effort; the retirement is not. Releasing the
        // decoded copy matters whether or not the file went.
        let missing = dir.join("never-existed.jpg");
        retire(&retired, Some(missing));
        assert_eq!(
            retired.lock().unwrap().len(),
            2,
            "a failed unlink skipped the retirement"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Draining is what makes it safe to call every frame: reporting a path
    /// twice would ask the UI to free one image two times.
    #[test]
    fn a_retired_path_is_reported_once() {
        let dir = scratch("retire-drains");
        let (cache, _ready) = ImageCache::new(dir.clone()).unwrap();
        assert!(cache.take_retired().is_empty());

        cache.retired.lock().unwrap().push(dir.join("x.jpg"));
        assert_eq!(cache.take_retired().len(), 1);
        assert!(
            cache.take_retired().is_empty(),
            "a retired path was reported twice"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("perch-cache-tests").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A download that is interrupted mid-write leaves its temp file behind.
    /// Nothing used to remove those, so they accumulated forever and were also
    /// indexed as if they were images.
    #[test]
    fn a_scan_deletes_the_debris_of_an_interrupted_download() {
        let dir = scratch("part-files");
        std::fs::create_dir_all(&dir).unwrap();
        let debris = dir.join("abc.png.part");
        let real = dir.join("abc.png");
        std::fs::write(&debris, b"half a download").unwrap();
        std::fs::write(&real, b"a whole one").unwrap();

        let index = scan_and_prune(&dir);

        assert!(!debris.exists(), "a .part file survived the scan");
        assert!(real.exists(), "a finished download was deleted");
        assert!(
            !index.values().any(|path| path == &debris),
            "a .part file was indexed as an image"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Under budget, nothing is touched — which is the case every real install
    /// is in, and the one a size cap must not disturb.
    #[test]
    fn a_cache_under_budget_keeps_everything() {
        let dir = scratch("under-budget");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.png", "b.png", "c.png"] {
            std::fs::write(dir.join(name), b"small").unwrap();
        }

        let index = scan_and_prune(&dir);

        assert_eq!(index.len(), 3);
        for name in ["a.png", "b.png", "c.png"] {
            assert!(dir.join(name).exists(), "{name} was pruned under budget");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Over budget, the oldest go first and the index never names a file that
    /// was just deleted — which is why pruning happens before indexing.
    #[test]
    fn going_over_budget_drops_the_oldest_and_indexes_only_survivors() {
        let dir = scratch("over-budget");
        std::fs::create_dir_all(&dir).unwrap();

        // Written oldest-first with distinct mtimes, since the sort key is the
        // modification time and a same-millisecond tie would make this flaky.
        let mut written = Vec::new();
        for name in ["old.png", "middle.png", "new.png"] {
            let path = dir.join(name);
            std::fs::write(&path, vec![0u8; 8192]).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            written.push(path);
        }

        // 16KB of the 24KB present, so exactly one file has to go.
        let index = prune_to(&dir, 16 * 1024);

        assert!(!written[0].exists(), "the oldest file was kept");
        assert!(written[1].exists(), "too much was pruned");
        assert!(written[2].exists(), "the newest file was pruned");
        assert_eq!(index.len(), 2);
        assert!(
            index.values().all(|path| path.exists()),
            "the index named a file that had just been deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
