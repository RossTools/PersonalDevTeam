//! Helpers shared by the pdtd integration tests (`mod common;`). They start the real pdtd binary on a free port with
//! a temp data dir, and never touch the stable instance.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "../../../../test-support/port_blocks.rs"]
pub mod port_blocks;
#[path = "../../../../test-support/wait.rs"]
pub mod wait;

/// A running pdtd and, once it answered, the port it serves.
pub struct Daemon(pub Child, Option<u16>);

impl Drop for Daemon {
    /// Asks pdtd to shut down first, so it stops its sessions' process trees: killing pdtd alone leaves them running
    /// on Windows, holding the test's output pipes (a nextest LEAK). Kills it if it does not exit in time.
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait()
            && let Some(port) = self.1
        {
            let _ = http(port, "POST", "/api/shutdown", None);
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline && matches!(self.0.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Makes `dir` (which exists) a repository on `main` with one empty commit: PDT refuses a repository with no commits.
pub fn init_repo(dir: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        assert!(
            std::process::Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success(),
            "git {args:?}"
        );
    }
}

/// A port for a test's pdtd that no other test holds, below every OS's ephemeral range (Linux hands out 32768 and up
/// for outgoing connections, Windows and macOS 49152 and up). A port from `bind(0)` was free only at that moment: under
/// nextest another test's connection could take it before pdtd bound it (or between a shutdown and a restart).
pub fn free_port() -> u16 {
    port_blocks::claim(30500, 500, 1)
}

pub fn start(port: u16, data: &Path) -> Daemon {
    start_with(port, data, &[])
}

/// Starts pdtd with every inherited `PDT_*` variable removed, plus `extra_env`.
pub fn start_with(port: u16, data: &Path, extra_env: &[(&str, &str)]) -> Daemon {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pdtd"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let child = cmd
        .env("PDT_PROFILE", "dev")
        .env("PDT_PORT", port.to_string())
        .env("PDT_DATA_DIR", data)
        .env("PDT_SECRET_STORE", "memory")
        // Never the machine's Tailscale (it serves the stable instance): any call would find no CLI.
        .env("PDT_TAILSCALE_BIN", data.join("no-such-tailscale"))
        // Nor GitHub: the forge has no network.
        .env("PDT_GITHUB_OFFLINE", "1")
        // Never the machine's agents either (pdtd's startup check runs `claude auth status`, which can read the real
        // keychain); a test that needs one passes a fake in `extra_env`, which overrides these.
        .env("PDT_CLAUDE_PROGRAM", data.join("no-such-claude"))
        .env("PDT_CODEX_PROGRAM", data.join("no-such-codex"))
        .env("PDT_DEVIN_PROGRAM", data.join("no-such-devin"))
        .envs(extra_env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file(data))
        .spawn()
        .unwrap();
    let mut d = Daemon(child, None);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if http(port, "GET", "/api/health", None).0 == 200 {
            d.1 = Some(port);
            return d;
        }
        if let Ok(Some(status)) = d.0.try_wait() {
            panic!("pdtd exited while starting ({status})\n{}", diagnose(&mut d, port, data));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("pdtd did not become healthy\n{}", diagnose(&mut d, port, data));
}

/// pdtd's stderr, kept in the data dir's parent so `diagnose` can show it.
fn log_file(data: &Path) -> Stdio {
    let path = data.with_extension("stderr.log");
    match std::fs::File::create(&path) {
        Ok(f) => Stdio::from(f),
        Err(_) => Stdio::null(),
    }
}

/// What a failing test should print about its daemon: alive or not, health, and its logs.
pub fn diagnose(d: &mut Daemon, port: u16, data: &Path) -> String {
    let alive = match d.0.try_wait() {
        Ok(None) => "running".to_string(),
        Ok(Some(s)) => format!("exited: {s}"),
        Err(e) => format!("unknown: {e}"),
    };
    let t = Instant::now();
    let health = http(port, "GET", "/api/health", None);
    let mut out = format!("pdtd {alive}; health {:?} in {:?}\n", health, t.elapsed());
    let mut logs: Vec<_> =
        std::fs::read_dir(data.join("logs")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    logs.push(data.with_extension("stderr.log"));
    for p in logs {
        if let Ok(text) = std::fs::read_to_string(&p) {
            let tail: Vec<&str> = text.lines().rev().take(80).collect();
            out.push_str(&format!("--- {}\n", p.display()));
            for l in tail.into_iter().rev() {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    out
}

pub fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    http_with(port, method, path, body, "")
}

/// Approves, as the user in PDT's web app (the UI token), every command the project's main checkout configures, so
/// its runs start (PDT-405).
pub fn approve_commands(port: u16, project_id: &str) {
    let (s, t) = http_with(port, "GET", "/api/ui/token", None, "Sec-Fetch-Site: same-origin\r\n");
    assert_eq!(s, 200, "{t}");
    let token = t["token"].as_str().unwrap().to_string();
    let (s, c) = http(port, "GET", &format!("/api/projects/{project_id}/commands"), None);
    assert_eq!(s, 200, "{c}");
    let body = serde_json::json!({ "fingerprints": c["untrusted"] }).to_string();
    let path = format!("/api/projects/{project_id}/commands/trust");
    let (s, r) = http_with(port, "POST", &path, Some(&body), &format!("X-UI-Token: {token}\r\n"));
    assert_eq!(s, 204, "{r}");
}

/// [`http`] with extra header lines (each ending in `\r\n`).
pub fn http_with(port: u16, method: &str, path: &str, body: Option<&str>, headers: &str) -> (u16, serde_json::Value) {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return (0, serde_json::Value::Null) };
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    if s.write_all(req.as_bytes()).is_err() {
        return (0, serde_json::Value::Null);
    }
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let Some((head, body)) = text.split_once("\r\n\r\n") else { return (0, serde_json::Value::Null) };
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(body).unwrap_or(serde_json::Value::Null))
}

pub fn wait_exit(d: &mut Daemon, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = d.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "pdtd did not exit");
        std::thread::sleep(Duration::from_millis(100));
    }
}
