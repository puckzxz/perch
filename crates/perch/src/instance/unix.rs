//! The claim on Unix: a lock on a file, and a socket beside it.
//!
//! `flock` lasts as long as the file stays open, and the kernel closes it
//! with the process however that ends, so the lock is the claim. The socket
//! is only the ear, and a crash leaves its file behind; the next first copy
//! clears it before listening, which only a holder of the lock can be doing.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// One end of a handover.
pub type Stream = UnixStream;

/// The first copy's claim: the locked file, and the socket it listens on.
pub struct Listener {
    socket: UnixListener,
    path: PathBuf,
    /// Held, never read: closing it is what lets the claim go.
    _lock: File,
}

fn socket_path(dir: &Path) -> PathBuf {
    dir.join("instance.sock")
}

/// Claim `dir`: a listener if this is the first copy, `None` if another is.
pub fn listen(dir: &Path) -> io::Result<Option<Listener>> {
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("instance.lock"))?;
    // SAFETY: a descriptor that stays open across the call.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::EWOULDBLOCK) => Ok(None),
            _ => Err(e),
        };
    }
    let path = socket_path(dir);
    // A socket a crash left behind. Holding the lock, nothing else can be
    // listening on it.
    let _ = fs::remove_file(&path);
    let socket = UnixListener::bind(&path)?;
    Ok(Some(Listener {
        socket,
        path,
        _lock: lock,
    }))
}

/// Reach the copy that has `dir`, or `None` if it is not listening yet — or
/// was, before a crash the next first copy will clear up after.
pub fn connect(dir: &Path) -> io::Result<Option<UnixStream>> {
    match UnixStream::connect(socket_path(dir)) {
        Ok(stream) => Ok(Some(stream)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

impl Listener {
    /// Wait for the next caller. `None` for one that hung up before it
    /// could be read.
    pub fn accept(&mut self) -> io::Result<Option<UnixStream>> {
        match self.socket.accept() {
            Ok((stream, _)) => Ok(Some(stream)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for Listener {
    /// Tidy the socket away while the lock is still held — the fields drop
    /// after this — so no first copy can have bound a new one in its place.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
