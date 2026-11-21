use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use super::Process;

pub(super) struct Watchdog(Child);

pub(crate) fn spawn(command: &mut Command) -> io::Result<Process> {
    // Rust sets close-on-exec after creating a pipe on macOS. A concurrent
    // spawn can inherit the exec-status pipe's writer before that flag is
    // set and block the first spawn for the lifetime of the second child.
    static SPAWN_LOCK: Mutex<()> = Mutex::new(());
    let startup = SPAWN_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // The watchdog is the group leader, so the group ID stays reserved until
    // it signals its own group. Only Perch holds the pipe's write end open
    // across exec. The shell receives no Streamlink arguments.
    let watchdog = Watchdog(
        Command::new("/bin/sh")
            .args(["-c", "IFS= read -r _; kill -KILL 0"])
            .env_clear()
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| io::Error::other(format!("could not start the watchdog: {error}")))?,
    );

    let perch_pid = std::process::id();
    command.process_group(watchdog.0.id() as i32);
    // SAFETY: getppid is async-signal-safe and the error holds a raw errno.
    // This selects Rust's fork path, which joins the process group before the
    // hook and before exec closes inherited pipe descriptors. If Perch dies
    // before the join, the child either fails to join or detects reparenting.
    unsafe {
        command.pre_exec(move || {
            if libc::getppid() as u32 == perch_pid {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(libc::ESRCH))
            }
        });
    }
    let child = command.spawn();
    drop(startup);
    Ok(Process {
        child: child?,
        _watchdog: watchdog,
    })
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        // SAFETY: the unreaped watchdog reserves the ID of the group it
        // created. Signal that group before waiting so cancellation on the
        // UI thread does not depend on the watchdog being scheduled.
        unsafe { libc::killpg(self.0.id() as i32, libc::SIGKILL) };
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::job;

    const OWNER_FIXTURE: &str = "PERCH_STREAMLINK_OWNER_FIXTURE";
    const READY: &str = "streamlink fixture ready";

    #[derive(Debug, PartialEq, Eq)]
    enum Output {
        Ready,
        Closed,
    }

    #[test]
    fn a_guarded_probe_keeps_its_output_and_exit_status() {
        let mut probe = Command::new("/bin/sh");
        probe
            .args(["-c", "printf 'probe output'; exit 7"])
            .stdout(Stdio::piped());
        let mut process = job::spawn(&mut probe).unwrap();
        let mut output = String::new();
        process
            .child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();

        assert_eq!(process.wait().unwrap().code(), Some(7));
        assert_eq!(output, "probe output");
    }

    #[test]
    fn killing_streamlink_reaps_its_group() {
        let process = job::spawn(Command::new("/bin/cat").stdin(Stdio::piped())).unwrap();
        // SAFETY: the group is queried for the fixture we just launched.
        let group = unsafe { libc::getpgid(process.child.id() as i32) };
        process.kill();
        assert!(group > 0);
        // SAFETY: signal zero queries whether the group exists.
        assert_eq!(unsafe { libc::kill(-group, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn streamlink_and_its_child_exit_when_perch_is_killed() {
        if std::env::var_os(OWNER_FIXTURE).is_some() {
            // Keep stdout open in both processes. Closing the test-owned
            // stdin also lets the fixture exit if crash cleanup fails.
            let mut streamlink = Command::new("/bin/sh");
            streamlink.args([
                "-c",
                "exec 3<&0; (IFS= read -r _ <&3) & printf '\\n%s\\n' \"$1\"; wait",
                "streamlink-fixture",
                READY,
            ]);
            let _ = job::spawn(&mut streamlink).unwrap().wait();
            return;
        }

        let mut owner = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "job::macos::tests::streamlink_and_its_child_exit_when_perch_is_killed",
                "--nocapture",
            ])
            .env(OWNER_FIXTURE, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let release_fixture = owner.stdin.take().unwrap();
        let stdout = owner.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line == READY {
                    let _ = send.send(Output::Ready);
                }
            }
            let _ = send.send(Output::Closed);
        });

        let ready = output.recv_timeout(Duration::from_secs(5));
        owner.kill().unwrap();
        let status = owner.wait().unwrap();
        let closed = output.recv_timeout(Duration::from_secs(5));
        drop(release_fixture);
        reader.join().unwrap();

        assert_eq!(ready, Ok(Output::Ready), "streamlink fixture did not start");
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert_eq!(
            closed,
            Ok(Output::Closed),
            "streamlink group survived its owner"
        );
    }
}
