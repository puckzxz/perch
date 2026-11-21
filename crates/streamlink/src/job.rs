//! Spawn streamlink so crash cleanup can run after Perch exits. Windows uses
//! a kill-on-close job object. On macOS a watchdog receives EOF on Perch's
//! pipe and kills the private process group, including Streamlink's children.

use std::io;
use std::process::{Child, ExitStatus};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::spawn;

pub(crate) struct Process {
    pub(crate) child: Child,
    #[cfg(target_os = "macos")]
    _watchdog: macos::Watchdog,
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn spawn(command: &mut std::process::Command) -> io::Result<Process> {
    let child = command.spawn()?;
    #[cfg(windows)]
    track(&child);
    Ok(Process { child })
}

impl Process {
    pub(crate) fn wait(mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub(crate) fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Closing the job when Perch exits kills every assigned process.
///
/// Assignment is best effort after spawn. `Command` gives us a process
/// handle, while assigning a suspended child would need its thread handle
/// to resume it. Killing the Streamlink launcher also stops its `python.exe`
/// worker, which covers a worker spawned before assignment.
#[cfg(windows)]
fn track(child: &Child) {
    use std::os::windows::io::AsRawHandle;

    let Some(job) = job() else { return };
    // SAFETY: `job` is a live job object handle, and `as_raw_handle` borrows a
    // process handle that `child` keeps open for as long as this borrow lives.
    unsafe {
        windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
            job,
            child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
        );
    }
}

/// The one job every streamlink child is assigned to, created on first use.
///
/// `HANDLE` is a raw pointer and so is neither `Send` nor `Sync`; it is parked
/// as a `usize` to keep the static shareable, and only ever handed straight
/// back to the API. `None` means the job could not be set up and callers should
/// carry on without it.
///
/// Never closed. The handle is meant to be released by process exit and nothing
/// else — that release *is* the teardown signal, so closing it early would kill
/// the streams it is supposed to be protecting.
#[cfg(windows)]
fn job() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<Option<usize>> = OnceLock::new();

    (*JOB.get_or_init(|| {
        // SAFETY: an unnamed job with default security, then one limit set on
        // it before anything is assigned. Every pointer below is to a local
        // that outlives the call.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }

            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 {
                // Without the limit the job would hold children and never kill
                // them, which is worse than not having one: it would look like
                // the leak was fixed.
                CloseHandle(job);
                return None;
            }

            Some(job as usize)
        }
    }))
    .map(|handle| handle as HANDLE)
}

#[cfg(all(test, windows))]
mod tests {
    /// Everything here rests on the job existing *and* accepting the
    /// kill-on-close limit. Both failures are silent by design — [`track`]
    /// swallows them so a stream still plays — and either one looks exactly
    /// like the leak being fixed right up until perch next dies badly.
    #[test]
    fn the_job_exists_and_took_its_limit() {
        assert!(
            super::job().is_some(),
            "no job object: streamlink children would outlive a hard exit again"
        );
    }
}
