//! The claim on Windows: a named pipe.
//!
//! A pipe exists exactly as long as some process holds an instance of it, so
//! the claim goes with the process however that ends. Creating the first
//! instance with `FILE_FLAG_FIRST_PIPE_INSTANCE` fails once anybody has one,
//! which makes the test and the claim one call, with no gap between them for
//! a second launch to slip through.
//!
//! The name is made from the settings directory. Pipes are machine-wide, and
//! a name that was only "perch" would hand one user's launch to another
//! user's window.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
/// Byte mode, blocking: `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`,
/// which are all zero.
const PIPE_BYTES_BLOCKING: u32 = 0;
/// Nobody from another machine; a pipe answers over the network otherwise.
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const PIPE_BUFFER: u32 = 4096;
/// The caller may learn who the pipe's owner is and nothing more: without
/// it, whoever created a pipe of this name first could act as this user.
const SECURITY_IDENTIFICATION: u32 = 1 << 16;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_PIPE_CONNECTED: i32 = 535;
/// How long to wait for a pipe busy with somebody else's launch.
const BUSY_WAIT_MS: u32 = 500;
const SW_RESTORE: i32 = 9;

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer: u32,
        in_buffer: u32,
        default_timeout: u32,
        security: *mut c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn AllowSetForegroundWindow(pid: u32) -> i32;
    fn IsIconic(window: Handle) -> i32;
    fn ShowWindow(window: Handle, command: i32) -> i32;
    fn SetForegroundWindow(window: Handle) -> i32;
}

/// One end of a handover.
pub type Stream = File;

/// The first copy's claim: the instance of the pipe waiting for the next
/// caller, and the name to make the one after it under.
pub struct Listener {
    pipe: OwnedHandle,
    name: Vec<u16>,
}

/// The settings directory as a word: FNV-1a over its path, case folded the
/// way the file system folds it. Not std's hasher, which is free to change
/// between releases — a perch built tomorrow has to find the one running
/// today.
fn key(dir: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in dir.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn pipe_path(dir: &Path) -> String {
    format!(r"\\.\pipe\perch-{}", key(dir))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Make an instance of the pipe: the first one, which is the claim, or
/// another for the next caller.
fn create(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let open_mode = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: `name` is NUL-terminated and outlives the call. No security
    // attributes is the default descriptor — the creator's — and a handle no
    // child inherits, so a streamlink this perch starts cannot hold the claim
    // after it has gone.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_BYTES_BLOCKING | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            ptr::null_mut(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(pipe) })
}

/// Claim `dir`: a listener if this is the first copy, `None` if another is.
pub fn listen(dir: &Path) -> io::Result<Option<Listener>> {
    let name = wide(&pipe_path(dir));
    match create(&name, true) {
        Ok(pipe) => Ok(Some(Listener { pipe, name })),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reach the copy that has `dir`, or `None` if nobody is listening right
/// now — gone, or busy with another launch for a moment.
pub fn connect(dir: &Path) -> io::Result<Option<File>> {
    let path = pipe_path(dir);
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(&path);
    match opened {
        Ok(pipe) => {
            give_way(&pipe);
            Ok(Some(pipe))
        }
        Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => Ok(None),
        Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
            let name = wide(&path);
            // SAFETY: `name` is NUL-terminated and outlives the call.
            unsafe { WaitNamedPipeW(name.as_ptr(), BUSY_WAIT_MS) };
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Hand this launch's right to the foreground — Windows gives it to a
/// process the user has just started — to the copy at the other end of
/// `pipe`, so that its window can come forward. See [`bring_forward`].
fn give_way(pipe: &File) {
    let mut pid = 0;
    // SAFETY: a live pipe handle, and an out-param that outlives the call.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } != 0 {
        // SAFETY: a plain call on a process id.
        unsafe { AllowSetForegroundWindow(pid) };
    }
}

impl Listener {
    /// Wait for the next caller. `None` for one that hung up before it
    /// could be read.
    pub fn accept(&mut self) -> io::Result<Option<File>> {
        // SAFETY: a live pipe handle; the call blocks until a caller comes.
        let connected = unsafe { ConnectNamedPipe(self.pipe.as_raw_handle(), ptr::null_mut()) }
            != 0
            || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED);
        // The next instance before this one is handed off: while this copy
        // runs, the name must never be free for a second to take.
        let next = create(&self.name, false)?;
        let caller = std::mem::replace(&mut self.pipe, next);
        Ok(connected.then(|| File::from(caller)))
    }
}

/// Bring `window` to the front, restoring it first if it was minimised.
///
/// Not `Window::activate_window`, which on Windows earns the foreground by
/// pressing Alt through `SendInput` — a real keystroke, into whatever had the
/// keyboard. The launch that handed over gave its own right to the
/// foreground to this process before saying anything ([`give_way`]), so
/// asking is enough.
pub fn bring_forward(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // Through the trait: gpui's own `Window::window_handle` is its id for the
    // window, not the platform's.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as Handle;
    // SAFETY: the window's own handle, alive while `window` is borrowed.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pipe per settings directory, however its case is written, and a
    /// name pinned to the arithmetic: two builds of perch on one machine
    /// have to agree on it.
    #[test]
    fn the_pipe_is_named_for_the_settings_directory() {
        let mine = Path::new(r"C:\Users\someone\AppData\Roaming\perch");
        assert_eq!(
            pipe_path(mine),
            pipe_path(Path::new(r"c:\users\SOMEONE\appdata\roaming\perch"))
        );
        assert_ne!(
            pipe_path(mine),
            pipe_path(Path::new(r"C:\Users\other\AppData\Roaming\perch"))
        );
        assert_eq!(pipe_path(mine), r"\\.\pipe\perch-298b7ff925010b98");
    }
}
