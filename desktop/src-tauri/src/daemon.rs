use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME, Profile};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub enum Probe {
    /// Nothing is listening.
    Free,
    /// A Personal Dev Team daemon answered /api/health.
    Ours,
    /// Something else (or a daemon still starting up) holds the port.
    Foreign(String),
}

pub fn probe(port: u16) -> Probe {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
        return Probe::Free;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    if let Err(e) = s.write_all(req.as_bytes()) {
        return Probe::Foreign(e.to_string());
    }
    let mut buf = Vec::new();
    if let Err(e) = s.take(64 * 1024).read_to_end(&mut buf) {
        return Probe::Foreign(e.to_string());
    }
    classify_health(&buf)
}

pub fn classify_health(raw: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Probe::Foreign("it did not answer like a web server".into());
    };
    let status_line = head.lines().next().unwrap_or("");
    if !status_line.starts_with("HTTP/1.") || status_line.split_whitespace().nth(1) != Some("200") {
        return Probe::Foreign(format!("its health check answered {status_line:?}"));
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v["name"] == PRODUCT_NAME => Probe::Ours,
        Ok(v) => Probe::Foreign(format!("it identifies as {}", v["name"])),
        Err(_) => Probe::Foreign("its health check is not JSON".into()),
    }
}

/// Attaches to a running daemon, or spawns one and waits until it answers its health check.
pub fn ensure_daemon(
    port: u16,
    probe: impl Fn(u16) -> Probe,
    spawn: impl FnOnce() -> std::io::Result<()>,
    timeout: Duration,
) -> Result<(), String> {
    let mut last = probe(port);
    match last {
        Probe::Ours => return Ok(()),
        Probe::Free => spawn().map_err(|e| format!("failed to start {DAEMON_BIN}: {e}"))?,
        Probe::Foreign(_) => {}
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        last = probe(port);
        if last == Probe::Ours {
            return Ok(());
        }
    }
    Err(match last {
        Probe::Foreign(why) => format!("port {port} is in use by something that is not {PRODUCT_NAME} ({why})"),
        _ => format!("{DAEMON_BIN} did not start listening on port {port} within {timeout:?}"),
    })
}

/// Where the desktop app sends a spawned daemon's stdout/stderr (startup errors land here before pdtd's own log).
pub fn launch_log_path(profile: Profile) -> Option<PathBuf> {
    let data_dir = match std::env::var("PDT_DATA_DIR") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => dirs::data_local_dir()?.join(profile.data_dir_name()),
    };
    Some(data_dir.join("logs").join(format!("{DAEMON_BIN}-launch.log")))
}

pub fn with_log_tail(msg: String, profile: Profile) -> String {
    match launch_log_path(profile) {
        Some(p) => append_tail(msg, &p, 20),
        None => msg,
    }
}

fn append_tail(msg: String, log: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(log) else {
        return msg;
    };
    let all: Vec<&str> = text.lines().collect();
    let tail = all[all.len().saturating_sub(lines)..].join("\n");
    if tail.trim().is_empty() { msg } else { format!("{msg}\n\nLast lines of {}:\n{tail}", log.display()) }
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
    let log = launch_log_path(profile).and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::File::options().create(true).append(true).open(p).ok()
    });
    let (out, err) = match log.and_then(|f| Some((f.try_clone().ok()?, f))) {
        Some((a, b)) => (Stdio::from(a), Stdio::from(b)),
        None => (Stdio::null(), Stdio::null()),
    };
    // Pass the port explicitly so an inherited PDT_PORT can't make the window and the daemon disagree.
    cmd.env("PDT_PROFILE", if profile == Profile::Dev { "dev" } else { "stable" })
        .env("PDT_PORT", profile.port().to_string())
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
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
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    /// Serves `response` to every connection on a background thread; returns the port.
    fn fake_server(response: &'static str) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn classifies_health_responses() {
        let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"status\":\"ok\",\"name\":\"Personal Dev Team\",\"version\":\"0.1.0\"}";
        assert_eq!(classify_health(ok), Probe::Ours);
        assert!(
            matches!(classify_health(b"HTTP/1.1 200 OK\r\n\r\n{\"name\":\"other\"}"), Probe::Foreign(w) if w.contains("other"))
        );
        assert!(matches!(classify_health(b"HTTP/1.1 404 Not Found\r\n\r\n"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"SSH-2.0-OpenSSH"), Probe::Foreign(_)));
    }

    #[test]
    fn probes_real_sockets() {
        assert_eq!(probe(free_port()), Probe::Free);
        let ours = fake_server("HTTP/1.0 200 OK\r\n\r\n{\"name\":\"Personal Dev Team\"}");
        assert_eq!(probe(ours), Probe::Ours);
        let foreign = fake_server("HTTP/1.0 200 OK\r\n\r\n<html>hi</html>");
        assert!(matches!(probe(foreign), Probe::Foreign(_)));
    }

    #[test]
    fn attaches_without_spawning_when_ours() {
        let spawned = Cell::new(false);
        ensure_daemon(
            1,
            |_| Probe::Ours,
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(!spawned.get());
    }

    #[test]
    fn spawns_when_free_and_waits_for_health() {
        let calls = Cell::new(0);
        let probe = |_| {
            calls.set(calls.get() + 1);
            if calls.get() < 4 { Probe::Free } else { Probe::Ours }
        };
        let spawned = Cell::new(false);
        ensure_daemon(
            1,
            probe,
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(spawned.get());
    }

    #[test]
    fn foreign_port_fails_without_spawning() {
        let spawned = Cell::new(false);
        let err = ensure_daemon(
            9,
            |_| Probe::Foreign("it identifies as \"x\"".into()),
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(!spawned.get());
        assert!(err.contains("port 9 is in use by something that is not Personal Dev Team"), "{err}");
    }

    #[test]
    fn timeout_and_spawn_failure_messages() {
        let err = ensure_daemon(9, |_| Probe::Free, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
        let err = ensure_daemon(9, |_| Probe::Free, || Err(std::io::Error::other("nope")), Duration::from_millis(300))
            .unwrap_err();
        assert!(err.contains("failed to start pdtd"), "{err}");
    }

    #[test]
    fn log_tail_is_appended() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("pdtd-launch.log");
        std::fs::write(&log, (1..=30).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let msg = append_tail("boom".into(), &log, 20);
        assert!(msg.starts_with("boom"));
        assert!(msg.contains("line 30") && msg.contains("line 11") && !msg.contains("line 10\n"));
        assert_eq!(append_tail("boom".into(), &tmp.path().join("missing.log"), 20), "boom");
    }
}
