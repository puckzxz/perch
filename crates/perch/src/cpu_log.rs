//! A record of where perch's CPU went, so an intermittent spike can be read
//! afterwards instead of caught live.
//!
//! "perch used 12%" is not a fact anyone can act on. "11 of those 12 points
//! were in threads named `worker`" is, because that names libmpv's decode pool
//! rather than the UI. Windows keeps the name Rust gives a thread, and libmpv
//! names its own, so the process can ask itself which of its subsystems is busy
//! without a profiler attached:
//!
//!   main            the UI thread - layout, paint, everything in `render`
//!   mpv-render      perch's own render loop in `video.rs`
//!   vo demux core   libmpv's presentation, demuxer and playback core
//!   worker          libmpv's decode pool, usually the largest single consumer
//!   ao-wasapi       audio output
//!   image-cache     thumbnail and emote downloads
//!   cpu-log         this module, left in the output so its own cost is visible
//!   (unnamed)       the runtime, the D3D driver, and Windows' own pools
//!
//! Two of the columns exist because of how the number is usually *seen*.
//! `child_pct` is streamlink and the python under it: Task Manager's process
//! list folds a parent's children into its row, so a stream left playing as a
//! browse thumbnail shows up as perch using CPU when none of it is perch's.
//! And `renders_per_s` separates work from spinning - a static page repainting
//! sixty times a second is a bug, and in a CPU number alone it looks exactly
//! like a page that is legitimately busy.
//!
//! Each row also carries what the app was doing: which page, how many panes,
//! how many playing, whether the window was even in front. The same CPU figure
//! means different things in each, and an occluded window repaints less. The UI
//! publishes all of it through atomics, so `render` pays a few relaxed stores
//! and nothing else.
//!
//! Opt-in: it runs only with `PERCH_CPU_LOG=1` in the environment. It ran
//! unconditionally for a while, on the argument that the problem worth
//! diagnosing is the one that already happened — and that is how the
//! hardware-decode measurement was made. But it is nine hundred lines of
//! hand-transcribed Win32 with a file in every user's AppData to show for it,
//! which is the wrong thing to have on for people who will never read the file.
//! Off, every hook below is one relaxed atomic access and nothing else.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
#[cfg(windows)]
use std::time::Duration;

/// How often to write a row. Long enough to be free, short enough that a spike
/// lasting a few seconds still lands in two or three rows.
#[cfg(windows)]
const INTERVAL: Duration = Duration::from_secs(5);

/// How often to look at the process total between rows.
///
/// A row is an average, and an average over five seconds hides the thing this
/// was built to catch: a spike one second long arrives as a fifth of itself.
/// `GetProcessTimes` is two syscalls, so the peak comes almost free - only the
/// expensive per-thread pass stays on the row cadence.
#[cfg(windows)]
const SUB_INTERVAL: Duration = Duration::from_secs(1);

/// Thread buckets quieter than this are left out of the row, so a line is the
/// handful of things that were actually running rather than 170 zeroes.
#[cfg(windows)]
const FLOOR_PERCENT: f64 = 0.05;

/// Rotate once past this, so a log left on for weeks cannot grow without end.
///
/// Generous on purpose. A row is around 110 bytes idle and 260 with a stream
/// playing, so this is roughly a month of continuous use - long enough that
/// rotation is a housekeeping detail rather than something that can lose the
/// day you were trying to capture.
#[cfg(windows)]
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// How many samples between rescans for threads and children that did not exist
/// yet.
///
/// The rescan is the expensive half of this module and the reason for the
/// number. `CreateToolhelp32Snapshot` has no per-process mode for threads - it
/// snapshots every thread on the machine and leaves the caller to filter - so
/// doing it every sample cost more CPU than the idle app it was measuring:
/// 1.5% of a core, against `main`'s 0.3%. Between rescans the loop reuses open
/// handles, which is one syscall per thread and nothing else. The price is that
/// something created just after a rescan goes unattributed for up to a minute,
/// which is acceptable: perch's threads are made at startup and libmpv's when a
/// stream opens, and both are followed by a long time running.
#[cfg(windows)]
const RESCAN_EVERY: u32 = 12;

/// The columns, in one place.
///
/// One constant rather than a literal at the write and another at the check,
/// because the whole point of the check is that the two cannot drift.
#[cfg(windows)]
const HEADER: &str = "time,uptime_s,cores,cpu_pct_1core,cpu_peak_pct_1core,child_pct,renders_per_s,drops,page,panes,playing,hwdec,render,focused,visible,priv_mb,ws_mb,threads,handles,cache_ready,cache_inflight,by_thread";

/// Whether the sampler is running. Set once by [`start`], read by every hook,
/// so that with the log off the hooks cost their callers one relaxed load.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the log is on. Callers with something to publish that costs more
/// than a store — the image cache's two counts each need a lock — check this
/// first and skip the work.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// What the UI was doing, published for the sampler thread.
///
/// Separate atomics rather than a lock: the sampler tolerates reading a page
/// from one frame and a pane count from the next, and `render` must not be able
/// to block on a diagnostic under any circumstances.
static PAGE: AtomicU8 = AtomicU8::new(0);
static PANES: AtomicU8 = AtomicU8::new(0);
static PLAYING: AtomicU8 = AtomicU8::new(0);
static RENDERS: AtomicU64 = AtomicU64::new(0);
static CACHE_READY: AtomicU64 = AtomicU64::new(0);
static CACHE_INFLIGHT: AtomicU64 = AtomicU64::new(0);
/// Bit 0: some stream decoded in software. Bit 1: some stream used the GPU.
///
/// Worth a column of its own because `hwdec=auto-copy` is a request, not a
/// guarantee: it falls back to software per codec and per driver, silently. A
/// row where the CPU doubled and this went 2 -> 1 explains itself.
static HWDEC: AtomicU8 = AtomicU8::new(0);
/// The size the video is actually being rendered at, packed `(w << 32) | h`.
///
/// The single biggest thing that moves the CPU between two otherwise identical
/// sessions, because mpv scales to whatever the pane asks for.
static RENDER_SIZE: AtomicU64 = AtomicU64::new(0);
/// Frames mpv reports having dropped, cumulative.
///
/// The difference between "the CPU was busy" and "the CPU was busy and the
/// picture suffered for it", which is the only version a viewer can see.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Which page is on screen. An enum rather than a bare number so a third page
/// cannot be added without this being looked at.
#[derive(Clone, Copy)]
pub enum Page {
    Browse = 0,
    Watch = 1,
}

/// Called once per render. A handful of relaxed stores; safe every frame.
///
/// The render counter is what makes the rest legible: without it, a quiet row
/// and a row spent repainting a static page at 60fps read the same.
pub fn note_frame(page: Page, panes: usize, playing: usize, ready: usize, inflight: usize) {
    PAGE.store(page as u8, Ordering::Relaxed);
    PANES.store(panes.min(u8::MAX as usize) as u8, Ordering::Relaxed);
    PLAYING.store(playing.min(u8::MAX as usize) as u8, Ordering::Relaxed);
    RENDERS.fetch_add(1, Ordering::Relaxed);
    CACHE_READY.store(ready as u64, Ordering::Relaxed);
    CACHE_INFLIGHT.store(inflight as u64, Ordering::Relaxed);
}

/// Called by a stream once mpv says which decoder it settled on.
pub fn note_hwdec(active: &str) {
    // mpv names the method it chose (`d3d11va-copy`, `nvdec-copy`, ...) and
    // says "no" when it decided against all of them.
    // Or-ed, not stored: with four panes the last writer would otherwise
    // decide for all of them, and "one of these fell back to software" is
    // exactly the case worth seeing.
    let bit = if active.is_empty() || active == "no" {
        1
    } else {
        2
    };
    HWDEC.fetch_or(bit, Ordering::Relaxed);
}

/// Called by a stream's render thread with the size it just rendered at.
pub fn note_render_size(width: u32, height: u32) {
    // The largest pane wins, and the sampler drains it per row. A plain store
    // would be whichever of up to four panes wrote last, which is a number
    // about no particular pane.
    RENDER_SIZE.fetch_max(((width as u64) << 32) | height as u64, Ordering::Relaxed);
}

/// Called by a stream's render thread with mpv's cumulative drop count.
pub fn note_dropped(since_last: u64) {
    // Added, not stored. Each pane has its own mpv and its own cumulative
    // counter starting at zero; storing them into one slot and differencing it
    // subtracts one stream's total from another's, which invents drop storms
    // out of nothing and hides real ones. The caller sends its own delta.
    DROPPED.fetch_add(since_last, Ordering::Relaxed);
}

#[cfg(windows)]
fn hwdec_name() -> &'static str {
    match HWDEC.load(Ordering::Relaxed) {
        1 => "software",
        2 => "hardware",
        3 => "mixed",
        _ => "-",
    }
}

#[cfg(windows)]
fn render_size() -> String {
    // Drained, so a row with nothing playing reports "-" rather than the size
    // of a pane that closed an hour ago.
    let packed = RENDER_SIZE.swap(0, Ordering::Relaxed);
    if packed == 0 {
        return "-".into();
    }
    format!("{}x{}", packed >> 32, packed as u32)
}

#[cfg(windows)]
fn page_name() -> &'static str {
    if PAGE.load(Ordering::Relaxed) == Page::Watch as u8 {
        "watch"
    } else {
        "browse"
    }
}

/// Beside the stderr log, for the same reasons.
#[cfg(windows)]
pub fn log_path() -> PathBuf {
    crate::diagnostics::log_path().with_extension("cpu.csv")
}

#[cfg(windows)]
fn enabled() -> bool {
    matches!(
        std::env::var("PERCH_CPU_LOG").as_deref(),
        Ok("1") | Ok("on") | Ok("true")
    )
}

#[cfg(windows)]
pub use windows_impl::start;

/// No sampler off Windows.
///
/// Not a gap worth filling on principle: everything below is Win32, the spike
/// this exists for is a Windows one, and macOS has Instruments, which is better
/// than anything this module could be. `note_frame` stays unconditional so
/// there is nothing to keep in step.
#[cfg(not(windows))]
pub fn start() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
mod windows_impl {
    use super::*;

    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::io::{BufRead, Write};

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        /// FILETIME counts 100-nanosecond ticks, in two halves because it
        /// predates a 64-bit integer being something an API could return.
        fn seconds(self) -> f64 {
            (((self.high as u64) << 32) | self.low as u64) as f64 / 1e7
        }
    }

    #[repr(C)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_pri: i32,
        delta_pri: i32,
        flags: u32,
    }

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        pri_class_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[repr(C)]
    #[derive(Default)]
    struct MemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_nonpaged_pool: usize,
        quota_nonpaged_pool: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        /// The number Task Manager does *not* show, and the one that mattered:
        /// committed private bytes, which ran to 44 GB against a 5.6 GB working
        /// set the day this file was written.
        private_usage: usize,
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    /// DWMWA_CLOAKED from dwmapi.h: set when the compositor is not drawing the
    /// window at all - another virtual desktop, and a few shell states besides.
    const DWMWA_CLOAKED: u32 = 14;
    /// What `GetExitCode*` reports for something still running.
    const STILL_ACTIVE: u32 = 259;

    extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> u32;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn GetProcessHandleCount(process: Handle, count: *mut u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> i32;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn OpenThread(access: u32, inherit: i32, tid: u32) -> Handle;
        fn GetThreadTimes(
            thread: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn GetThreadDescription(thread: Handle, description: *mut *mut u16) -> i32;
        fn GetExitCodeThread(thread: Handle, code: *mut u32) -> i32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn FlushFileBuffers(handle: Handle) -> i32;
        fn LocalFree(mem: Handle) -> Handle;
        fn K32GetProcessMemoryInfo(process: Handle, counters: *mut MemoryCounters, cb: u32) -> i32;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmGetWindowAttribute(
            window: Handle,
            attribute: u32,
            value: *mut c_void,
            size: u32,
        ) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> Handle;
        fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
        fn EnumWindows(
            callback: Option<unsafe extern "system" fn(Handle, isize) -> i32>,
            lparam: isize,
        ) -> i32;
        fn IsWindowVisible(window: Handle) -> i32;
        fn IsIconic(window: Handle) -> i32;
    }

    /// Kernel + user seconds for a process handle.
    fn process_seconds(handle: Handle) -> Option<f64> {
        let (mut c, mut e, mut k, mut u) = Default::default();
        // SAFETY: the four out-params are live for the call, and `handle` is
        // either the current-process pseudo handle or one from `OpenProcess`
        // that has not been closed.
        let ok = unsafe { GetProcessTimes(handle, &mut c, &mut e, &mut k, &mut u) };
        (ok != 0).then(|| k.seconds() + u.seconds())
    }

    /// Kernel + user seconds for an already-open thread handle.
    ///
    /// `None` means the thread has exited, which is the caller's cue to close
    /// the handle and forget it.
    fn thread_seconds(handle: Handle) -> Option<f64> {
        let (mut c, mut e, mut k, mut u) = Default::default();
        // SAFETY: `handle` came from `OpenThread` and has not been closed.
        let ok = unsafe { GetThreadTimes(handle, &mut c, &mut e, &mut k, &mut u) };
        (ok != 0).then(|| k.seconds() + u.seconds())
    }

    fn memory_mb() -> (f64, f64) {
        let mut counters = MemoryCounters {
            cb: std::mem::size_of::<MemoryCounters>() as u32,
            ..Default::default()
        };
        // SAFETY: `counters` is live and `cb` describes its real size.
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        if ok == 0 {
            return (0.0, 0.0);
        }
        const MB: f64 = 1024.0 * 1024.0;
        (
            counters.private_usage as f64 / MB,
            counters.working_set as f64 / MB,
        )
    }

    fn handle_count() -> u32 {
        let mut count = 0;
        // SAFETY: `count` is live for the call.
        unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
        count
    }

    /// Whether a handle still refers to a running thread.
    ///
    /// `GetThreadTimes` is emphatically not this test, which is what the first
    /// version of this module got wrong. Holding a handle is what keeps the
    /// kernel object alive, so it keeps returning success on a corpse - frozen
    /// counters, and an exit time that MSDN calls undefined while the thread
    /// runs, so it cannot be tested either. The reaping below therefore never
    /// fired, and the sampler accumulated a handle for every thread libmpv has
    /// ever created: a handle leak, in the diagnostic watching for leaks, with
    /// `threads` and `handles` climbing forever to report it.
    fn thread_alive(handle: Handle) -> bool {
        let mut code = 0u32;
        // SAFETY: `handle` came from `OpenThread` and has not been closed.
        let ok = unsafe { GetExitCodeThread(handle, &mut code) };
        ok != 0 && code == STILL_ACTIVE
    }

    /// The same, for a child process.
    fn process_alive(handle: Handle) -> bool {
        let mut code = 0u32;
        // SAFETY: `handle` came from `OpenProcess` and has not been closed.
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        ok != 0 && code == STILL_ACTIVE
    }

    /// Whether perch's window has focus.
    ///
    /// `GetForegroundWindow` is "the window the user is working with", so this
    /// is focus and nothing else: a maximised window on a second monitor is
    /// fully visible and still reports false. Useful for reading intent - was
    /// anyone actually driving it - and useless for predicting repaints, which
    /// is what `is_visible` is for.
    fn is_focused(pid: u32) -> bool {
        let mut owner = 0;
        // SAFETY: a null window yields pid 0, which cannot match ours.
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut owner) };
        owner == pid
    }

    /// Whether the window is actually on screen somewhere.
    ///
    /// This is the one that predicts CPU. DWM stops painting a window that is
    /// minimised or cloaked - the latter covering another virtual desktop, and
    /// a few shell states besides - and an unfocused but visible window paints
    /// exactly as hard as a focused one. True occlusion by another window is
    /// not detectable without the compositor's cooperation, so this is a floor
    /// rather than a guarantee: false means definitely not painting, true means
    /// probably painting.
    fn is_visible(pid: u32) -> bool {
        let mut found: Handle = std::ptr::null_mut();
        let mut search = (pid, &mut found as *mut Handle);
        // SAFETY: the callback only writes through the pointer in `search`,
        // which outlives the call.
        unsafe { EnumWindows(Some(top_level_of), &mut search as *mut _ as isize) };
        if found.is_null() {
            return false;
        }
        // SAFETY: `found` came from the enumeration and is a live window for
        // the duration of the call.
        unsafe {
            if IsIconic(found) != 0 {
                return false;
            }
            let mut cloaked: u32 = 0;
            let ok = DwmGetWindowAttribute(
                found,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            );
            // A failure here is not evidence of anything; assume visible.
            ok != 0 || cloaked == 0
        }
    }

    /// Finds the process's first visible top-level window.
    ///
    /// SAFETY: called only by `EnumWindows` above, with an `lparam` that is the
    /// `(pid, *mut Handle)` pair it was given.
    unsafe extern "system" fn top_level_of(window: Handle, lparam: isize) -> i32 {
        let search = &mut *(lparam as *mut (u32, *mut Handle));
        let mut owner = 0;
        GetWindowThreadProcessId(window, &mut owner);
        // `IsWindowVisible` stays true for a minimised window, so it is only
        // filtering out the hidden helper windows every GUI process has.
        if owner == search.0 && IsWindowVisible(window) != 0 {
            *search.1 = window;
            return 0;
        }
        1
    }

    /// Push a row all the way to disk, metadata included.
    ///
    /// `File::flush` is a no-op on Windows - `File` has no userspace buffer, so
    /// there is nothing for it to do - and without this the directory entry
    /// keeps the size the file had when it was opened. The bytes are in the
    /// cache and a reader gets them, but Explorer shows 0 KB for a log being
    /// written to all day, and the size only appears when the handle closes.
    /// That is the opposite of what a diagnostic should look like while it is
    /// doing its job. One forced flush per five seconds is nothing.
    fn flush_to_disk(file: &std::fs::File) {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: the handle belongs to `file`, which outlives the call.
        unsafe { FlushFileBuffers(file.as_raw_handle() as Handle) };
    }

    /// The name Rust or libmpv gave the thread, folded to a subsystem.
    ///
    /// Pools are collapsed - libmpv's are all called `worker` already, and
    /// perch's four downloaders become one `image-cache` - because which of the
    /// four was busy is never the question.
    fn thread_name(tid: u32) -> String {
        // SAFETY: a failed open returns null, which is checked; the handle is
        // closed on every path that opened one.
        let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid) };
        if handle.is_null() {
            return "(unnamed)".into();
        }
        let mut raw: *mut u16 = std::ptr::null_mut();
        let mut name = String::new();
        // SAFETY: `raw` is written only on success, and freed with `LocalFree`
        // as `GetThreadDescription` documents.
        unsafe {
            if GetThreadDescription(handle, &mut raw) >= 0 && !raw.is_null() {
                let mut len = 0;
                while *raw.add(len) != 0 {
                    len += 1;
                }
                name = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
                LocalFree(raw as Handle);
            }
            CloseHandle(handle);
        }
        if name.is_empty() {
            return "(unnamed)".into();
        }
        if name.starts_with("image-cache-") {
            return "image-cache".into();
        }
        // `lua/stats`, `ao/wasapi`: the slash and comma would need quoting.
        name.replace(['/', '\\', ',', '"'], "-")
    }

    /// Thread ids belonging to `pid`, for the periodic rescan.
    fn thread_ids(pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        // SAFETY: the snapshot is closed on every path that made one.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return out;
        }
        // SAFETY: zeroed is a valid THREADENTRY32; `size` is set as the API
        // requires, and iteration stops when `Thread32Next` reports no more.
        unsafe {
            let mut entry: ThreadEntry32 = std::mem::zeroed();
            entry.size = std::mem::size_of::<ThreadEntry32>() as u32;
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.owner_process_id == pid {
                    out.push(entry.thread_id);
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        out
    }

    /// Every process descended from `root`, at any depth.
    ///
    /// Two levels in practice - perch spawns `streamlink.exe`, which spawns the
    /// `python.exe` that does the fetching - but walked generally rather than
    /// assuming that shape.
    fn descendants(root: u32) -> Vec<u32> {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        // SAFETY: the snapshot is closed on every path that made one.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Vec::new();
        }
        // SAFETY: as `thread_ids` above.
        unsafe {
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                children
                    .entry(entry.parent_process_id)
                    .or_default()
                    .push(entry.process_id);
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }

        // Breadth-first with a seen set: Windows recycles pids, and a stale
        // parent id pointing back up the tree would otherwise spin here.
        let mut out = Vec::new();
        let mut seen = HashSet::from([root]);
        let mut queue = vec![root];
        while let Some(pid) = queue.pop() {
            for &child in children.get(&pid).into_iter().flatten() {
                if seen.insert(child) {
                    out.push(child);
                    queue.push(child);
                }
            }
        }
        out
    }

    /// What the sampler keeps between ticks for one thread.
    struct Thread {
        handle: Handle,
        name: String,
        last: f64,
    }

    /// And for one descendant process.
    struct Child {
        handle: Handle,
        last: f64,
    }

    /// Start sampling, if asked to. Returns where the rows are going, for the
    /// startup line.
    pub fn start() -> Option<PathBuf> {
        if !enabled() {
            return None;
        }
        let path = log_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        // Appended to, not rotated per run - unlike the stderr log, and
        // deliberately. What this is for is a long stretch of ordinary use, and
        // restarting the app in the middle of one is ordinary; rotating per run
        // would put the morning in `.csv.old` and throw it away on the next
        // restart. A restart stays legible because `uptime_s` returns to one
        // interval.
        //
        // Rotation happens on size, and on the columns changing. That second
        // one is not hypothetical bookkeeping: appending today's rows under
        // yesterday's header does not fail loudly, it fails silently, because
        // `csv.DictReader` and its kin happily zip 21 values onto 16 names and
        // hand back a row where `playing` reads "watch". A day of unattended
        // sampling would be on disk and quietly wrong.
        let stale = match std::fs::File::open(&path) {
            Ok(existing) => {
                let mut first = String::new();
                std::io::BufReader::new(existing)
                    .read_line(&mut first)
                    .is_err()
                    || first.trim_end() != HEADER
            }
            // Absent is not stale; it is simply new.
            Err(_) => false,
        };
        let too_big = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES);
        if stale || too_big {
            // A distinct name per rotation, so a size rotation and a schema
            // rotation cannot silently eat one another.
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let _ = std::fs::rename(&path, path.with_extension(format!("{stamp}.csv")));
        }

        let fresh = !path.exists();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        if fresh {
            writeln!(file, "{HEADER}").ok()?;
        }

        std::thread::Builder::new()
            .name("cpu-log".into())
            .spawn(move || sample_forever(file))
            .ok()?;
        ACTIVE.store(true, Ordering::Relaxed);

        Some(path)
    }

    fn sample_forever(mut file: std::fs::File) {
        // SAFETY: no arguments, no failure mode.
        let pid = unsafe { GetCurrentProcessId() };
        // SAFETY: a pseudo handle, valid for the life of the process and not to
        // be closed.
        let me = unsafe { GetCurrentProcess() };
        let started = std::time::Instant::now();
        // So a reader can turn "% of one core" into "% of this machine"
        // without having to know which machine it was.
        let cores = std::thread::available_parallelism().map_or(0, |n| n.get());

        let mut threads: HashMap<u32, Thread> = HashMap::new();
        let mut kids: HashMap<u32, Child> = HashMap::new();
        let mut last_process = process_seconds(me).unwrap_or(0.0);
        let mut last_renders = RENDERS.load(Ordering::Relaxed);
        let mut last_at = std::time::Instant::now();
        let mut tick = 0u32;
        let mut unaccounted = false;
        let mut last_dropped = DROPPED.load(Ordering::Relaxed);

        loop {
            // Discover what is new: on a cadence, and immediately after any
            // sample that could not account for the CPU it measured.
            //
            // The cadence alone is not enough, and the log said so twice.
            // Opening a stream creates dozens of libmpv threads at once, and
            // until the next rescan none of them is attributed: one sample read
            // 203% of a core with only 118 points named, the missing 85 being a
            // decode pool the sampler had not met. Triggering on the app's own
            // "a stream started" signal was the obvious fix and it did not
            // work - mpv builds its pool a moment *after* the pane reports
            // playing, so the rescan fired just too early and the next twelve
            // samples were still short.
            //
            // Unattributed time is the signal that actually means what it needs
            // to mean, whatever the cause: threads that appeared, threads from
            // something other than a stream, anything. It costs one bad sample
            // and then corrects itself.
            if tick % RESCAN_EVERY == 0 || unaccounted {
                for tid in thread_ids(pid) {
                    if threads.contains_key(&tid) {
                        continue;
                    }
                    // SAFETY: a failed open returns null, which is checked.
                    let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid) };
                    if handle.is_null() {
                        continue;
                    }
                    match thread_seconds(handle) {
                        Some(last) => {
                            let name = thread_name(tid);
                            threads.insert(tid, Thread { handle, name, last });
                        }
                        // SAFETY: opened just above, closed exactly once.
                        None => unsafe {
                            CloseHandle(handle);
                        },
                    }
                }
                for child_pid in descendants(pid) {
                    if kids.contains_key(&child_pid) {
                        continue;
                    }
                    // SAFETY: a failed open returns null, which is checked.
                    let handle =
                        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child_pid) };
                    if handle.is_null() {
                        continue;
                    }
                    match process_seconds(handle) {
                        Some(last) => {
                            kids.insert(child_pid, Child { handle, last });
                        }
                        // SAFETY: opened just above, closed exactly once.
                        None => unsafe {
                            CloseHandle(handle);
                        },
                    }
                }
            }
            // Cleared here, set below by the sample that could not account
            // for itself.
            unaccounted = false;

            // Sleep in short steps, watching the process total on each, so
            // `cpu_peak_pct` reports the worst second rather than the mean of
            // five. See `SUB_INTERVAL`.
            let mut peak: f64 = 0.0;
            let mut sub_last = last_process;
            let mut sub_at = last_at;
            let steps = (INTERVAL.as_secs_f64() / SUB_INTERVAL.as_secs_f64()).round() as u32;
            for _ in 0..steps.max(1) {
                std::thread::sleep(SUB_INTERVAL);
                let at = std::time::Instant::now();
                let seconds = at.duration_since(sub_at).as_secs_f64();
                let total = process_seconds(me).unwrap_or(sub_last);
                if seconds > 0.0 {
                    peak = peak.max(((total - sub_last) / seconds) * 100.0);
                }
                sub_last = total;
                sub_at = at;
            }

            tick = tick.wrapping_add(1);

            let now = std::time::Instant::now();
            let elapsed = now.duration_since(last_at).as_secs_f64();
            last_at = now;
            if elapsed <= 0.0 {
                continue;
            }

            let process = process_seconds(me).unwrap_or(last_process);
            let cpu = ((process - last_process) / elapsed) * 100.0;
            last_process = process;

            let renders = RENDERS.load(Ordering::Relaxed);
            let renders_per_second = renders.saturating_sub(last_renders) as f64 / elapsed;
            last_renders = renders;

            // Threads, and the ones that have gone. A dead thread's handle is
            // closed here rather than left to accumulate over a day.
            let mut by_name: HashMap<String, f64> = HashMap::new();
            let mut gone = Vec::new();
            for (&tid, thread) in threads.iter_mut() {
                if !thread_alive(thread.handle) {
                    gone.push(tid);
                    continue;
                }
                let Some(seconds) = thread_seconds(thread.handle) else {
                    gone.push(tid);
                    continue;
                };
                let delta = seconds - thread.last;
                thread.last = seconds;
                if delta > 0.0 {
                    *by_name.entry(thread.name.clone()).or_insert(0.0) += delta;
                }
            }
            for tid in gone {
                if let Some(thread) = threads.remove(&tid) {
                    // SAFETY: opened in the rescan, closed exactly once here.
                    unsafe { CloseHandle(thread.handle) };
                }
            }

            let mut child_seconds = 0.0;
            let mut dead = Vec::new();
            for (&child_pid, child) in kids.iter_mut() {
                if !process_alive(child.handle) {
                    dead.push(child_pid);
                    continue;
                }
                let Some(seconds) = process_seconds(child.handle) else {
                    dead.push(child_pid);
                    continue;
                };
                let delta = seconds - child.last;
                child.last = seconds;
                if delta > 0.0 {
                    child_seconds += delta;
                }
            }
            for child_pid in dead {
                if let Some(child) = kids.remove(&child_pid) {
                    // SAFETY: opened in the rescan, closed exactly once here.
                    unsafe { CloseHandle(child.handle) };
                }
            }

            let mut ranked: Vec<(String, f64)> = by_name
                .into_iter()
                .map(|(name, seconds)| (name, (seconds / elapsed) * 100.0))
                .filter(|(_, percent)| *percent >= FLOOR_PERCENT)
                .collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            let breakdown = ranked
                .iter()
                .map(|(name, percent)| format!("{name}={percent:.1}"))
                .collect::<Vec<_>>()
                .join(" ");

            // Anything much over a fifth of the process unexplained means the
            // thread list is stale. The floor keeps an idle app, where a single
            // rounding-sized bucket is most of a tiny total, from rescanning
            // every five seconds for no reason.
            let named: f64 = ranked.iter().map(|(_, percent)| percent).sum();
            unaccounted = cpu > 5.0 && named < cpu * 0.8;

            let (private_mb, working_set_mb) = memory_mb();

            let panes = PANES.load(Ordering::Relaxed);
            let playing = PLAYING.load(Ordering::Relaxed);

            let dropped = DROPPED.load(Ordering::Relaxed);
            let drops = dropped.saturating_sub(last_dropped);
            last_dropped = dropped;

            let line = format!(
                "{},{:.0},{},{:.1},{:.1},{:.1},{:.1},{},{},{},{},{},{},{},{},{:.0},{:.0},{},{},{},{},\"{}\"",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                started.elapsed().as_secs_f64(),
                cores,
                cpu,
                peak,
                (child_seconds / elapsed) * 100.0,
                renders_per_second,
                drops,
                page_name(),
                panes,
                playing,
                hwdec_name(),
                render_size(),
                is_focused(pid),
                is_visible(pid),
                private_mb,
                working_set_mb,
                threads.len(),
                handle_count(),
                CACHE_READY.load(Ordering::Relaxed),
                CACHE_INFLIGHT.load(Ordering::Relaxed),
                breakdown,
            );
            // Flushed per row: the run worth reading is often one that ended by
            // being killed, and a buffered tail is exactly the part that
            // explains why.
            // A failed write is not a reason to stop sampling for the rest of
            // the day: a transient error - a scanner, a full disk that empties -
            // would otherwise silently end the log at the moment it got
            // interesting, and nothing would say so.
            if writeln!(file, "{line}").is_ok() {
                flush_to_disk(&file);
            }
        }
    }
}
