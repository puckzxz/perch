//! Pull decoded video frames out of libmpv as CPU-side BGRA buffers.
//!
//! libmpv does the hard parts - HLS ingest, demuxing, decoding, audio output and
//! A/V sync - and hands us finished frames through its software render API. This
//! crate deliberately exposes only raw bytes and dimensions, with no UI types in
//! its public API, so the same pipeline can feed any renderer.

pub mod ffi;

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use ffi::{Lib, MpvHandle, MpvRenderCtx, MpvRenderParam};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not load libmpv from {0}: {1}")]
    LoadLibrary(String, String),
    #[error("libmpv is missing symbol {0}: {1}")]
    MissingSymbol(String, String),
    #[error("could not find libmpv. Set MPV_DLL to its full path. Tried:\n{0}")]
    NotFound(String),
    #[error("mpv_create failed (out of memory, or libmpv is too old)")]
    CreateFailed,
    #[error("{context}: {message} (mpv error {code})")]
    Mpv {
        context: &'static str,
        message: String,
        code: c_int,
    },
    #[error("destination buffer holds {got} bytes, need {need} for {width}x{height} BGRA")]
    BufferTooSmall {
        got: usize,
        need: usize,
        width: u32,
        height: u32,
    },
    #[error("option name or value contained an interior NUL byte")]
    InteriorNul,
}

/// Startup options applied before `mpv_initialize`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Play the audio track. Off for frame-dumping, on for a real player.
    pub audio: bool,
    /// Allow hardware decoding. The software render path needs frames in system
    /// memory, so enabling this makes mpv copy them back - usually a net loss.
    pub hwdec: bool,
    /// Playback volume, 0-100. mpv's own default is 100, which is startlingly
    /// loud for a stream that opens on its own.
    pub volume: u8,
    /// Extra `mpv_set_option_string` pairs, applied last so they win.
    pub extra: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: false,
            hwdec: false,
            volume: 100,
            extra: Vec::new(),
        }
    }
}

/// Something that happened inside mpv since the last poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    FileLoaded,
    VideoReconfig,
    EndFile,
    Shutdown,
    Other(c_int),
}

#[derive(Default)]
struct FrameSignal {
    ready: Mutex<bool>,
    woken: Condvar,
}

/// A libmpv instance rendering to a CPU buffer.
pub struct Player {
    lib: Arc<Lib>,
    mpv: MpvHandle,
    render: MpvRenderCtx,
    signal: Arc<FrameSignal>,
    block_timing: AtomicBool,
}

// libmpv documents `mpv_handle` as thread-safe. The render context is not
// re-entrant, so `Player` moves between threads but is not shared across them.
unsafe impl Send for Player {}

impl Player {
    /// Load libmpv, start playback of `url`, and prepare software rendering.
    pub fn open(url: &str) -> Result<Self, Error> {
        Self::open_with(url, Config::default())
    }

    pub fn open_with(url: &str, config: Config) -> Result<Self, Error> {
        let lib = Arc::new(Lib::discover()?);
        Self::open_with_lib(lib, url, config)
    }

    pub fn open_with_lib(lib: Arc<Lib>, url: &str, config: Config) -> Result<Self, Error> {
        let mpv = unsafe { (lib.create)() };
        if mpv.is_null() {
            return Err(Error::CreateFailed);
        }

        // Built early so that any failure below still runs Drop and tears mpv down.
        let mut player = Self {
            lib,
            mpv,
            render: std::ptr::null_mut(),
            signal: Arc::new(FrameSignal::default()),
            block_timing: AtomicBool::new(true),
        };

        // `vo=libmpv` is mandatory: it tells mpv that a render context, not mpv's
        // own window, will present the video.
        player.set_option("vo", "libmpv")?;
        player.set_option("hwdec", if config.hwdec { "auto-copy" } else { "no" })?;
        if config.audio {
            player.set_option("volume", &config.volume.min(100).to_string())?;
        } else {
            player.set_option("ao", "null")?;
        }
        for (key, value) in &config.extra {
            player.set_option(key, value)?;
        }

        let rc = unsafe { (player.lib.initialize)(player.mpv) };
        player.check(rc, "mpv_initialize")?;

        player.create_render_context()?;
        player.command(&["loadfile", url])?;

        Ok(player)
    }

    fn create_render_context(&mut self) -> Result<(), Error> {
        const API_SW: &[u8] = b"sw\0";

        let mut params = [
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_API_TYPE,
                data: API_SW.as_ptr() as *mut c_void,
            },
            MpvRenderParam::TERMINATOR,
        ];

        let mut ctx: MpvRenderCtx = std::ptr::null_mut();
        let rc = unsafe { (self.lib.render_create)(&mut ctx, self.mpv, params.as_mut_ptr()) };
        self.check(rc, "mpv_render_context_create")?;
        self.render = ctx;

        // The Arc keeps the signal alive independently of `self` moving; the
        // pointer targets the heap allocation, which is stable. Drop frees the
        // render context first, so the callback can never outlive this Arc.
        unsafe {
            (self.lib.render_set_update_callback)(
                self.render,
                Some(on_mpv_update),
                Arc::as_ptr(&self.signal) as *mut c_void,
            );
        }
        Ok(())
    }

    /// Block until mpv reports a new frame, or `timeout` elapses.
    ///
    /// Returns `true` if a frame is waiting. Clears the flag, so each `true` maps
    /// to exactly one frame worth rendering.
    pub fn wait_for_frame(&self, timeout: Duration) -> bool {
        let mut ready = self.signal.ready.lock().unwrap();
        if !*ready {
            let (guard, _) = self
                .signal
                .woken
                .wait_timeout_while(ready, timeout, |ready| !*ready)
                .unwrap();
            ready = guard;
        }
        if *ready {
            *ready = false;
            drop(ready);
            // Confirm with mpv rather than trusting the wakeup: the callback also
            // fires for redraws that carry no new frame.
            let flags = unsafe { (self.lib.render_update)(self.render) };
            return flags & ffi::MPV_RENDER_UPDATE_FRAME != 0;
        }
        false
    }

    /// Control whether `render_bgra` blocks until the frame's display time.
    ///
    /// mpv defaults this on, and it is what keeps video timed to audio - but it
    /// makes `render_bgra` block for up to `video-timing-offset` (50 ms by
    /// default). That is fine on a dedicated render thread and fatal on a UI
    /// thread, where it would stall the whole frame loop.
    ///
    /// Turn it off only if you take over frame timing yourself; A/V sync drifts
    /// otherwise.
    pub fn set_block_for_target_time(&self, block: bool) {
        self.block_timing.store(block, Ordering::Relaxed);
    }

    /// Render the current frame into `dst` as BGRA at `width` x `height`.
    ///
    /// mpv scales to whatever size is asked for, so pass the size you will
    /// actually display - rendering a 1080p source into a 720p pane costs 3.7 MB
    /// per frame instead of 8.3 MB, on both the CPU convert and the GPU upload.
    pub fn render_bgra(&self, width: u32, height: u32, dst: &mut [u8]) -> Result<(), Error> {
        let need = width as usize * height as usize * 4;
        if dst.len() < need {
            return Err(Error::BufferTooSmall {
                got: dst.len(),
                need,
                width,
                height,
            });
        }

        // mpv writes b,g,r at increasing addresses and leaves the 4th byte as
        // uninitialised garbage - the format is "bgr0", not "bgra".
        const FORMAT: &[u8] = b"bgr0\0";

        let mut size = [width as c_int, height as c_int];
        let mut stride: usize = width as usize * 4;
        let mut block: c_int = self.block_timing.load(Ordering::Relaxed) as c_int;

        let mut params = [
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: &mut block as *mut c_int as *mut c_void,
            },
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_SW_FORMAT,
                data: FORMAT.as_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_SW_STRIDE,
                data: &mut stride as *mut usize as *mut c_void,
            },
            MpvRenderParam {
                type_: ffi::MPV_RENDER_PARAM_SW_POINTER,
                data: dst.as_mut_ptr() as *mut c_void,
            },
            MpvRenderParam::TERMINATOR,
        ];

        let rc = unsafe { (self.lib.render_render)(self.render, params.as_mut_ptr()) };
        self.check(rc, "mpv_render_context_render")?;

        // Turn the garbage byte into a real opaque alpha, or every pixel handed
        // to a GPU renderer is transparent.
        for pixel in dst[..need].chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        Ok(())
    }

    /// Drain pending mpv events without blocking.
    pub fn poll_events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        loop {
            let raw = unsafe { (self.lib.wait_event)(self.mpv, 0.0) };
            if raw.is_null() {
                break;
            }
            let id = unsafe { (*raw).event_id };
            let event = match id {
                ffi::MPV_EVENT_NONE => break,
                ffi::MPV_EVENT_FILE_LOADED => Event::FileLoaded,
                ffi::MPV_EVENT_VIDEO_RECONFIG => Event::VideoReconfig,
                ffi::MPV_EVENT_END_FILE => Event::EndFile,
                ffi::MPV_EVENT_SHUTDOWN => Event::Shutdown,
                other => Event::Other(other),
            };
            events.push(event);
        }
        events
    }

    /// Pause or resume.
    ///
    /// On a live stream, pausing means falling behind: mpv stops consuming
    /// while the source keeps producing. Callers should seek back to the live
    /// edge on resume rather than leaving the viewer silently in the past.
    pub fn set_paused(&self, paused: bool) -> Result<(), Error> {
        self.set_property("pause", if paused { "yes" } else { "no" })
    }

    /// Jump to the newest available point in a live stream.
    pub fn seek_to_live(&self) -> Result<(), Error> {
        // Equivalent to mpv's "seek 100 absolute-percent"; on a live stream the
        // end of the cache is the live edge.
        self.command(&["seek", "100", "absolute-percent"])
    }

    /// Change playback volume (0-100) while playing.
    pub fn set_volume(&self, percent: u8) -> Result<(), Error> {
        self.set_property("volume", &percent.min(100).to_string())
    }

    fn set_property(&self, name: &str, value: &str) -> Result<(), Error> {
        let c_name = CString::new(name).map_err(|_| Error::InteriorNul)?;
        let c_value = CString::new(value).map_err(|_| Error::InteriorNul)?;
        let rc =
            unsafe { (self.lib.set_property_string)(self.mpv, c_name.as_ptr(), c_value.as_ptr()) };
        self.check(rc, "mpv_set_property_string")
    }

    /// Read an mpv property as a string, e.g. `"width"`, `"video-codec"`.
    pub fn property(&self, name: &str) -> Option<String> {
        let c_name = CString::new(name).ok()?;
        let raw = unsafe { (self.lib.get_property_string)(self.mpv, c_name.as_ptr()) };
        if raw.is_null() {
            return None;
        }
        let value = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.lib.free)(raw as *mut c_void) };
        Some(value)
    }

    /// Path of the libmpv this player loaded.
    pub fn library_path(&self) -> &std::path::Path {
        &self.lib.path
    }

    fn set_option(&self, name: &str, value: &str) -> Result<(), Error> {
        let c_name = CString::new(name).map_err(|_| Error::InteriorNul)?;
        let c_value = CString::new(value).map_err(|_| Error::InteriorNul)?;
        let rc =
            unsafe { (self.lib.set_option_string)(self.mpv, c_name.as_ptr(), c_value.as_ptr()) };
        self.check(rc, "mpv_set_option_string")
    }

    fn command(&self, args: &[&str]) -> Result<(), Error> {
        let owned: Vec<CString> = args
            .iter()
            .map(|a| CString::new(*a).map_err(|_| Error::InteriorNul))
            .collect::<Result<_, _>>()?;
        let mut argv: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());

        let rc = unsafe { (self.lib.command)(self.mpv, argv.as_mut_ptr()) };
        self.check(rc, "mpv_command")
    }

    fn check(&self, code: c_int, context: &'static str) -> Result<(), Error> {
        if code >= 0 {
            return Ok(());
        }
        Err(Error::Mpv {
            context,
            message: self.lib.error_text(code),
            code,
        })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Order matters. Freeing the render context is what guarantees the update
        // callback has stopped firing; only then is it safe to let `signal` drop.
        if !self.render.is_null() {
            unsafe { (self.lib.render_free)(self.render) };
            self.render = std::ptr::null_mut();
        }
        if !self.mpv.is_null() {
            unsafe { (self.lib.terminate_destroy)(self.mpv) };
            self.mpv = std::ptr::null_mut();
        }
    }
}

/// Called by mpv from one of its internal threads. Must not call back into mpv
/// and must not block, so it only flips a flag and wakes the waiter.
unsafe extern "C" fn on_mpv_update(cb_ctx: *mut c_void) {
    if cb_ctx.is_null() {
        return;
    }
    let signal = unsafe { &*(cb_ctx as *const FrameSignal) };
    if let Ok(mut ready) = signal.ready.lock() {
        *ready = true;
        signal.woken.notify_all();
    }
}
