//! Where problems go when there is no console to print them to.
//!
//! The release build is a `windows_subsystem = "windows"` binary, so it has no
//! console at all: `eprintln!` and panic messages get written to a handle that
//! leads nowhere, and the first sign of trouble is an app that silently does
//! not work.
//!
//! Rather than teach seven crates about logging, the *process's* stderr handle
//! is pointed at a file once at startup. Windows resolves that handle on every
//! write, so this catches output from the library crates and panic messages
//! too, without a line of change anywhere else. That behaviour was tested
//! before this was written, because the whole approach rests on it.

use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;

/// `STD_ERROR_HANDLE` from `winbase.h`.
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

extern "system" {
    /// Reassigns one of the process's standard handles.
    fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
}

/// Beside the image cache, in local app data: reproducible, not worth roaming.
pub fn log_path() -> PathBuf {
    crate::image_cache_dir()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{}.log", crate::APP_NAME))
}

/// Send everything written to stderr to the log file from here on.
///
/// The previous run is kept beside it as `.log.old`, because the interesting
/// case is usually "it broke, and I restarted it before thinking to look".
pub fn capture_stderr() -> std::io::Result<PathBuf> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::rename(&path, path.with_extension("log.old"));

    let file = File::create(&path)?;
    // SAFETY: the handle is valid for as long as the file is, and the file is
    // deliberately leaked below so it outlives every write.
    let ok = unsafe { SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle()) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Windows now owns this handle for the life of the process.
    std::mem::forget(file);

    eprintln!(
        "{} {} started {}",
        crate::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().to_rfc3339()
    );
    Ok(path)
}
