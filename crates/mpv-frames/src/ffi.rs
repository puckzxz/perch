//! Raw libmpv bindings, resolved at runtime from `libmpv-2.dll` / `libmpv.so.2`.
//!
//! We load dynamically rather than link, because Windows has no official libmpv
//! development package: the DLL ships inside player distributions (mpv.net, Plex)
//! without an import library or headers. Dynamic loading also lets the app point
//! at whichever copy the user already has.
//!
//! Signatures are transcribed from mpv's `client.h` and `render.h`. They are not
//! machine-checked, so any edit here must be diffed against the upstream headers.

use std::ffi::{c_char, c_double, c_int, c_void};
use std::path::PathBuf;

use libloading::{Library, Symbol};

// ── render.h: mpv_render_param_type ──────────────────────────────────
pub const MPV_RENDER_PARAM_INVALID: c_int = 0;
pub const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const MPV_RENDER_PARAM_SW_SIZE: c_int = 17;
pub const MPV_RENDER_PARAM_SW_FORMAT: c_int = 18;
pub const MPV_RENDER_PARAM_SW_STRIDE: c_int = 19;
pub const MPV_RENDER_PARAM_SW_POINTER: c_int = 20;

/// Returned by `mpv_render_context_update`: a new frame is ready to render.
pub const MPV_RENDER_UPDATE_FRAME: u64 = 1;

// ── client.h: mpv_event_id (only the ones we act on) ─────────────────
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MpvRenderParam {
    pub type_: c_int,
    pub data: *mut c_void,
}

impl MpvRenderParam {
    pub const TERMINATOR: Self = Self {
        type_: MPV_RENDER_PARAM_INVALID,
        data: std::ptr::null_mut(),
    };
}

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

pub type MpvHandle = *mut c_void;
pub type MpvRenderCtx = *mut c_void;
pub type MpvRenderUpdateFn = unsafe extern "C" fn(cb_ctx: *mut c_void);

type FnCreate = unsafe extern "C" fn() -> MpvHandle;
type FnInitialize = unsafe extern "C" fn(MpvHandle) -> c_int;
type FnTerminateDestroy = unsafe extern "C" fn(MpvHandle);
type FnSetOptionString = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnCommand = unsafe extern "C" fn(MpvHandle, *mut *const c_char) -> c_int;
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnWaitEvent = unsafe extern "C" fn(MpvHandle, c_double) -> *mut MpvEvent;
type FnGetPropertyString = unsafe extern "C" fn(MpvHandle, *const c_char) -> *mut c_char;
type FnSetPropertyString = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnFree = unsafe extern "C" fn(*mut c_void);

type FnRenderCreate =
    unsafe extern "C" fn(*mut MpvRenderCtx, MpvHandle, *mut MpvRenderParam) -> c_int;
type FnRenderRender = unsafe extern "C" fn(MpvRenderCtx, *mut MpvRenderParam) -> c_int;
type FnRenderSetUpdateCb =
    unsafe extern "C" fn(MpvRenderCtx, Option<MpvRenderUpdateFn>, *mut c_void);
type FnRenderUpdate = unsafe extern "C" fn(MpvRenderCtx) -> u64;
type FnRenderFree = unsafe extern "C" fn(MpvRenderCtx);

/// Every libmpv entry point we use, plus the `Library` that owns them.
///
/// The `Library` field is never read but must outlive the pointers: dropping it
/// unloads the module and every function pointer below dangles.
pub struct Lib {
    _library: Library,
    pub path: PathBuf,

    pub create: FnCreate,
    pub initialize: FnInitialize,
    pub terminate_destroy: FnTerminateDestroy,
    pub set_option_string: FnSetOptionString,
    pub command: FnCommand,
    pub error_string: FnErrorString,
    pub wait_event: FnWaitEvent,
    pub get_property_string: FnGetPropertyString,
    pub set_property_string: FnSetPropertyString,
    pub free: FnFree,

    pub render_create: FnRenderCreate,
    pub render_render: FnRenderRender,
    pub render_set_update_callback: FnRenderSetUpdateCb,
    pub render_update: FnRenderUpdate,
    pub render_free: FnRenderFree,
}

// The loaded module is process-global and libmpv's own handles are internally
// synchronised; the function pointers themselves are plain code addresses.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

unsafe fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T, crate::Error> {
    let mut c_name = name.as_bytes().to_vec();
    c_name.push(0);
    let symbol: Symbol<T> = unsafe { lib.get(&c_name) }
        .map_err(|e| crate::Error::MissingSymbol(name.to_string(), e.to_string()))?;
    Ok(*symbol)
}

impl Lib {
    /// Load libmpv from `path`.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, crate::Error> {
        let path = path.into();
        let library = unsafe { Library::new(&path) }
            .map_err(|e| crate::Error::LoadLibrary(path.display().to_string(), e.to_string()))?;

        // SAFETY: each signature is transcribed from the upstream header and the
        // symbol table was verified to contain every name below.
        unsafe {
            Ok(Self {
                create: sym(&library, "mpv_create")?,
                initialize: sym(&library, "mpv_initialize")?,
                terminate_destroy: sym(&library, "mpv_terminate_destroy")?,
                set_option_string: sym(&library, "mpv_set_option_string")?,
                command: sym(&library, "mpv_command")?,
                error_string: sym(&library, "mpv_error_string")?,
                wait_event: sym(&library, "mpv_wait_event")?,
                get_property_string: sym(&library, "mpv_get_property_string")?,
                set_property_string: sym(&library, "mpv_set_property_string")?,
                free: sym(&library, "mpv_free")?,

                render_create: sym(&library, "mpv_render_context_create")?,
                render_render: sym(&library, "mpv_render_context_render")?,
                render_set_update_callback: sym(
                    &library,
                    "mpv_render_context_set_update_callback",
                )?,
                render_update: sym(&library, "mpv_render_context_update")?,
                render_free: sym(&library, "mpv_render_context_free")?,

                path,
                _library: library,
            })
        }
    }

    /// Load from `MPV_DLL` if set, otherwise try the usual install locations.
    pub fn discover() -> Result<Self, crate::Error> {
        let mut tried = Vec::new();

        if let Some(explicit) = std::env::var_os("MPV_DLL") {
            return Self::load(PathBuf::from(explicit));
        }

        for candidate in Self::candidates() {
            match Self::load(&candidate) {
                Ok(lib) => return Ok(lib),
                Err(e) => tried.push(format!("  {} — {e}", candidate.display())),
            }
        }

        // On Windows and macOS the candidates are filtered to files that
        // exist, so the list is empty when there is no libmpv anywhere rather
        // than full of not-found lines. Saying where we looked beats saying
        // nothing.
        if tried.is_empty() {
            tried.push(format!("  {NOTHING_FOUND}"));
        }

        Err(crate::Error::NotFound(tried.join("\n")))
    }

    /// Where to look, in order.
    ///
    /// On Windows and macOS every entry is an absolute path that exists, for
    /// reasons that differ by platform but land in the same place: see
    /// [`windows_candidates`] for why a bare filename is not acceptable there,
    /// and [`macos_candidates`] for why dyld cannot be relied on to find it
    /// here. Only on other Unixes are the bare sonames handed to the dynamic
    /// linker, because there a packaged libmpv really is on the loader's path
    /// and that path — unlike Windows' — does not include the current
    /// directory.
    fn candidates() -> Vec<PathBuf> {
        #[cfg(windows)]
        {
            windows_candidates()
        }
        #[cfg(target_os = "macos")]
        {
            macos_candidates()
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            ["libmpv.so.2", "libmpv.so"]
                .iter()
                .map(PathBuf::from)
                .collect()
        }
    }

    /// Human-readable text for a libmpv negative error code.
    pub fn error_text(&self, code: c_int) -> String {
        let ptr = unsafe { (self.error_string)(code) };
        if ptr.is_null() {
            return format!("unknown mpv error {code}");
        }
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Absolute paths to try on Windows, in order, filtered to those that exist.
///
/// The two fallbacks used to be the bare names `libmpv-2.dll` and `mpv-2.dll`,
/// handed straight to `Library::new` — which is `LoadLibraryExW(name, 0, 0)`,
/// and with no path separator that means the standard DLL search order,
/// including the process's current directory. No Windows machine ships a system
/// copy of libmpv (this module's opening paragraph is about exactly that), so
/// nothing legitimate ever wins that search — but a DLL sitting beside a
/// downloaded executable run out of an ordinary Downloads folder would be
/// loaded into this process.
///
/// The directories are searched by hand instead, which removes the current
/// directory while keeping every case that actually worked before: a copy next
/// to the binary, and a copy on `PATH`. Filtering on existence also keeps the
/// `NotFound` error readable, since `PATH` alone is usually dozens of entries.
///
/// `MPV_DLL` still overrides all of this — an explicit path is the user saying
/// where it is, which is not the same as the OS guessing.
#[cfg(windows)]
fn windows_candidates() -> Vec<PathBuf> {
    const NAMES: [&str; 2] = ["libmpv-2.dll", "mpv-2.dll"];

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(beside_us) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        dirs.push(beside_us);
    }
    dirs.push(PathBuf::from(r"C:\Program Files\mpv.net"));
    dirs.push(PathBuf::from(r"C:\Program Files\Plex\Plex"));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }

    existing(&dirs, &NAMES)
}

/// Absolute paths to try on macOS, in order, filtered to those that exist.
///
/// The two fallbacks used to be the bare sonames `libmpv.2.dylib` and
/// `libmpv.dylib` handed straight to `Library::new`, i.e. to dlopen, whose
/// fallback search is `/usr/local/lib` and then `/usr/lib`. Homebrew on Apple
/// Silicon installs to `/opt/homebrew/lib`, which is in neither — so the one
/// thing every Mac user will actually have done, `brew install mpv`, was the
/// one case that could not work. `DYLD_FALLBACK_LIBRARY_PATH` is not a way
/// out of that: SIP strips every `DYLD_*` variable from a protected process.
///
/// So the directories are searched by hand, as on Windows and for the mirror
/// image of the reason — there the loader's search included somewhere it
/// should not be trusted from, here it excludes the one place the library
/// actually is.
///
/// `MPV_DLL` still overrides all of this — an explicit path is the user saying
/// where it is, which is not the same as the OS guessing.
#[cfg(target_os = "macos")]
fn macos_candidates() -> Vec<PathBuf> {
    const NAMES: [&str; 2] = ["libmpv.2.dylib", "libmpv.dylib"];

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(beside_us) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        // Inside an .app the executable is in `Contents/MacOS` and a bundled
        // dylib belongs in `Contents/Frameworks`, which is the same "shipped
        // alongside" case that `beside_us` covers for a loose binary.
        dirs.push(beside_us.join("../Frameworks"));
        dirs.push(beside_us);
    }
    dirs.push(PathBuf::from("/opt/homebrew/lib")); // Homebrew, Apple Silicon
    dirs.push(PathBuf::from("/usr/local/lib")); // Homebrew, Intel
    dirs.push(PathBuf::from("/opt/local/lib")); // MacPorts

    existing(&dirs, &NAMES)
}

/// Join every name onto every directory, in that order, keeping what exists.
///
/// Filtering on existence is what keeps the `NotFound` error readable: `PATH`
/// alone is usually dozens of entries, and a screenful of paths that were never
/// going to hold a libmpv says nothing about what went wrong.
#[cfg(any(windows, target_os = "macos"))]
fn existing(dirs: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// What to say when the search turned up nothing at all.
///
/// Named directories rather than a general apology: the user is about to go
/// looking, and the list is the useful half of the message.
#[cfg(windows)]
const NOTHING_FOUND: &str = "nothing named libmpv-2.dll beside the executable, in the usual \
     player install directories, or on PATH. Set MPV_DLL to point at one.";
#[cfg(target_os = "macos")]
const NOTHING_FOUND: &str = "no libmpv.2.dylib beside the executable, in /opt/homebrew/lib, \
     /usr/local/lib or /opt/local/lib. `brew install mpv` puts one in the first of those; \
     otherwise set MPV_DLL to point at one.";
#[cfg(not(any(windows, target_os = "macos")))]
const NOTHING_FOUND: &str = "no libmpv.so.2 on the dynamic linker's search path. Install your \
     distribution's libmpv package, or set MPV_DLL to point at one.";
