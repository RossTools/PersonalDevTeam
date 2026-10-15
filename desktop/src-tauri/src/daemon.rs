use pdt_core::brand::{DAEMON_BIN, Profile};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn is_listening(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(200)).is_ok()
}

/// Attaches to a running daemon, or spawns one and waits for it to listen.
pub fn ensure_daemon(port: u16, spawn: impl FnOnce() -> std::io::Result<()>, timeout: Duration) -> Result<(), String> {
    if is_listening(port) {
        return Ok(());
    }
    spawn().map_err(|e| format!("failed to start {DAEMON_BIN}: {e}"))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_listening(port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{DAEMON_BIN} did not start listening on port {port} within {timeout:?}"))
}

fn pdtd_path() -> PathBuf {
    let file = format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

/// Starts pdtd detached from this window; it keeps running after the window closes.
pub fn spawn_pdtd(profile: Profile) -> std::io::Result<()> {
    let mut cmd = Command::new(pdtd_path());
    // Pass the port explicitly so an inherited PDT_PORT can't make the window and the daemon disagree.
    cmd.env("PDT_PROFILE", if profile == Profile::Dev { "dev" } else { "stable" })
        .env("PDT_PORT", profile.port().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        // Own process group, so Ctrl-C in the launching terminal doesn't kill pdtd.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(|_| ())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::net::TcpListener;

    #[test]
    fn does_not_spawn_when_already_listening() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let spawned = Cell::new(false);
        ensure_daemon(
            port,
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_millis(500),
        )
        .unwrap();
        assert!(!spawned.get());
    }

    #[test]
    fn spawns_and_waits_until_listening() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let spawn = || {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                let l = TcpListener::bind(("127.0.0.1", port)).unwrap();
                std::thread::sleep(Duration::from_secs(2));
                drop(l);
            });
            Ok(())
        };
        ensure_daemon(port, spawn, Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn times_out_with_message() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = ensure_daemon(port, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
    }

    #[test]
    fn spawn_failure_is_reported() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = ensure_daemon(port, || Err(std::io::Error::other("nope")), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("failed to start pdtd"), "{err}");
    }
}
