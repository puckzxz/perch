//! Where problems go when there is no console to print them to.
//!
//! The release build is a `windows_subsystem = "windows"` binary, so on Windows
//! it has no console at all: `eprintln!` and panic messages get written to a
//! handle that leads nowhere, and the first sign of trouble is an app that
//! silently does not work. A macOS app launched from Finder rather than a
//! terminal arrives at the same place by a different road.
//!
//! Rather than teach seven crates about logging, the *process's* stderr is
//! pointed at a file once at startup. Both kernels resolve stderr on every
//! write — a handle on Windows, file descriptor 2 on Unix — so this catches
//! output from the library crates and panic messages too, without a line of
//! change anywhere else. That behaviour was tested before this was written,
//! because the whole approach rests on it.

use std::fs::File;
use std::path::PathBuf;

/// Beside the image cache: reproducible, not worth roaming.
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

    redirect_stderr(File::create(&path)?)?;

    eprintln!(
        "{} {} started {}",
        crate::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().to_rfc3339()
    );
    Ok(path)
}

/// `STD_ERROR_HANDLE` from `winbase.h`.
#[cfg(windows)]
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

#[cfg(windows)]
extern "system" {
    /// Reassigns one of the process's standard handles.
    fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
}

/// Point stderr at `file`, consuming it because the process keeps it forever.
#[cfg(windows)]
fn redirect_stderr(file: File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    // SAFETY: the handle is valid for as long as the file is, and the file is
    // deliberately leaked below so it outlives every write.
    let ok = unsafe { SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle()) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // `SetStdHandle` stores the handle rather than duplicating it, so dropping
    // the file here would close the handle Windows had just been given.
    std::mem::forget(file);
    Ok(())
}

/// Point stderr at `file`, consuming it to match the Windows signature.
///
/// There is no std API for this. `Stderr` is a handle *Rust* owns, and
/// replacing it would only redirect Rust's own `eprintln!` — not the panic
/// runtime and not any C library that writes to fd 2 directly, which between
/// them are most of what this module exists to catch.
#[cfg(unix)]
fn redirect_stderr(file: File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: `file` is open across the call and `STDERR_FILENO` is a constant.
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // Unlike `SetStdHandle`, `dup2` *duplicates*: fd 2 is now its own reference
    // to the same open file, so the original closes normally on drop.
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The two things perch tells the outside world its version is: the header
    /// this module writes, and the `User-Agent` `twitch-chat` sends to the
    /// recent-messages service. They are one number now — every crate inherits
    /// the workspace's — and this holds them to it.
    ///
    /// Worth a test rather than a comment because the failure was invisible
    /// from either side. `env!("CARGO_PKG_VERSION")` expands to whichever crate
    /// is being compiled, so the header here was right through both releases
    /// while the identical-looking expression in `twitch-chat` sat at the 0.1.0
    /// that crate was still nominally on.
    #[test]
    fn the_chat_user_agent_names_this_release() {
        assert_eq!(
            twitch_chat::history::USER_AGENT,
            concat!("perch/", env!("CARGO_PKG_VERSION")),
            "chat is introducing itself as a different version than this build"
        );
    }
}
