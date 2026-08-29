//! Tie streamlink children to the lifetime of this process, without needing
//! this process to cooperate.
//!
//! [`crate::StreamSupervisor::drop`] kills the child it owns, and for every
//! ordinary teardown — closing a pane, switching quality, going back to browse
//! — that is the whole story. It is not the whole story when perch itself goes
//! away without running destructors: a panic that aborts, a crash inside a
//! driver, the window's quit path unwinding somewhere `Drop` never reaches, or
//! the user ending the process from Task Manager. Nothing runs `Drop` on any of
//! those routes, and `Child`'s own `Drop` deliberately does not kill, so
//! streamlink is left behind serving a stream nobody is watching — holding its
//! loopback port and the Twitch credential on its command line — until someone
//! notices it in a process list.
//!
//! That is not a race to be tightened. No arrangement of flags and slots inside
//! this crate can help, because the code that would run them is what failed to
//! run. The only fix is to hand the job to something that outlives us, which on
//! Windows is the kernel: a Job Object holding every child, with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set. Process exit closes handles however
//! the process died, so the last handle to the job going away is exactly the
//! event "perch is gone, by any route" — and the kernel kills what is left.
//!
//! Children are assigned after spawn rather than through `CREATE_SUSPENDED`,
//! which would need the child's *thread* handle to resume and `std`'s `Command`
//! does not expose one. That leaves a sliver in which the shim could spawn its
//! own child before being assigned, so the grandchild would sit outside the job
//! — but killing the shim takes its `python.exe` worker with it, which is
//! measured behaviour of the Streamlink launcher, not an assumption. The chain
//! holds either way.
//!
//! There is no equivalent on macOS. Job objects have no counterpart, and
//! `PR_SET_PDEATHSIG` is Linux-only, so [`track`] is a documented no-op there
//! and teardown still rests on `Drop`. Ordinary quits are covered; a hard crash
//! on macOS can still leave streamlink behind.

/// Put `child` under this process's lifetime, so that it cannot outlive perch
/// even if perch dies without running destructors.
///
/// Best effort by design: a failure here means the supervisor's own `Drop` is
/// the only teardown, which is what the behaviour was before this existed.
/// Losing the safety net is not worth refusing to play a stream over.
#[cfg(windows)]
pub fn track(child: &std::process::Child) {
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

#[cfg(not(windows))]
pub fn track(_child: &std::process::Child) {}

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
