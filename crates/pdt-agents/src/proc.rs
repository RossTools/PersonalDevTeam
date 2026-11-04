//! Ending a process and everything it started: on unix a polite signal to its process group, then SIGKILL once the
//! grace runs out (sooner when the group empties); on Windows `taskkill /T /F`.

use std::time::Duration;

/// How long a process group gets to act on SIGTERM (or a terminal's SIGHUP) before it is killed outright.
pub const KILL_GRACE: Duration = Duration::from_secs(3);

/// How often a signalled group is checked for having emptied.
#[cfg(unix)]
const CHECK_EVERY: Duration = Duration::from_millis(100);

/// Ends the process group `pid` leads (unix: SIGTERM, then SIGKILL after `grace`) or the process tree under `pid`
/// (Windows). Returns at once: the waiting happens on a thread. The process must lead its own group (`setsid` or
/// `process_group(0)`), or this signals nothing.
pub fn terminate_group(pid: u32, grace: Duration) {
    #[cfg(unix)]
    if let Some(pgid) = pgid(pid) {
        signal_group(pgid, libc::SIGTERM, grace, || {});
    }
    #[cfg(windows)]
    {
        let _ = grace;
        std::thread::spawn(move || crate::pty::kill_tree(pid));
    }
}

/// Kills the process group `pid` leads (unix: SIGKILL) or the process tree under `pid` (Windows) outright. Returns at
/// once: on Windows `taskkill` starts on a thread of its own, so this is safe to call from async code and `Drop`.
pub fn kill_group(pid: u32) {
    #[cfg(unix)]
    if let Some(pgid) = pgid(pid) {
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    std::thread::spawn(move || crate::pty::kill_tree(pid));
}

/// `pid` as a process group id to signal: never 0 or 1 (`kill(-1)` signals every process the user owns).
#[cfg(unix)]
pub(crate) fn pgid(pid: u32) -> Option<libc::pid_t> {
    libc::pid_t::try_from(pid).ok().filter(|p| *p > 1)
}

/// Sends `signal` to group `pgid` before it returns, then on a thread waits up to `grace` for the group to empty,
/// SIGKILLs it if it hasn't, and runs `then` (also when the group was already gone).
#[cfg(unix)]
pub(crate) fn signal_group(
    pgid: libc::pid_t,
    signal: libc::c_int,
    grace: Duration,
    then: impl FnOnce() + Send + 'static,
) {
    let sent = unsafe { libc::kill(-pgid, signal) } == 0;
    std::thread::spawn(move || {
        if sent {
            let deadline = std::time::Instant::now() + grace;
            // Signal 0 checks without sending anything.
            while unsafe { libc::kill(-pgid, 0) } == 0 {
                if std::time::Instant::now() >= deadline {
                    unsafe {
                        libc::kill(-pgid, libc::SIGKILL);
                    }
                    break;
                }
                std::thread::sleep(CHECK_EVERY);
            }
        }
        then();
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::time::Instant;

    fn group(script: &str) -> std::process::Child {
        std::process::Command::new("/bin/sh").args(["-c", script]).process_group(0).spawn().unwrap()
    }

    fn wait_gone(pgid: libc::pid_t, within: Duration) -> Duration {
        let start = Instant::now();
        while unsafe { libc::kill(-pgid, 0) } == 0 {
            assert!(start.elapsed() < within, "the group outlived {within:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        start.elapsed()
    }

    #[test]
    fn a_group_that_heeds_sigterm_ends_before_the_grace() {
        let mut child = group("sleep 30 & wait");
        let pgid = pgid(child.id()).unwrap();
        terminate_group(child.id(), Duration::from_secs(10));
        let _ = child.wait();
        assert!(wait_gone(pgid, Duration::from_secs(5)) < Duration::from_secs(5));
    }

    #[test]
    fn a_group_that_ignores_sigterm_is_killed_after_the_grace() {
        let mut child = group("trap '' TERM; sleep 30 & wait");
        let pgid = pgid(child.id()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        terminate_group(child.id(), Duration::from_millis(300));
        let _ = child.wait();
        wait_gone(pgid, Duration::from_secs(5));
        assert!(start.elapsed() >= Duration::from_millis(300), "killed before the grace ran out");
    }

    #[test]
    fn then_runs_when_the_group_is_already_gone() {
        let mut child = group("true");
        let _ = child.wait();
        let (tx, rx) = std::sync::mpsc::channel();
        signal_group(pgid(child.id()).unwrap(), libc::SIGTERM, Duration::from_secs(10), move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(1)).expect("then ran at once");
    }

    #[test]
    fn never_signals_every_process() {
        assert_eq!(pgid(0), None);
        assert_eq!(pgid(1), None);
        assert_eq!(pgid(u32::MAX), None);
    }
}
