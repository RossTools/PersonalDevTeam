//! Starts the real pdtd binary on a free port with a temp data dir. Never touches the stable instance.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start(port: u16, data: &Path) -> Daemon {
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
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let d = Daemon(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if http(port, "GET", "/api/health", None).0 == 200 {
            return d;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("pdtd did not become healthy");
}

fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return (0, serde_json::Value::Null) };
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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

fn wait_exit(d: &mut Daemon, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = d.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "pdtd did not exit");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn shutdown_interrupts_sessions_and_restart_resumes_only_those() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    let port = free_port();

    let mut d = start(port, &data);
    let body = serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string();
    let (status, project) = http(port, "POST", "/api/projects", Some(&body));
    assert_eq!(status, 201, "{project}");
    let body = serde_json::json!({"projectId": project["id"], "kind": "shell", "cols": 80, "rows": 24}).to_string();
    let (status, session) = http(port, "POST", "/api/sessions", Some(&body));
    assert_eq!(status, 201, "{session}");
    let id = session["id"].as_str().unwrap().to_string();
    // A session the user stopped must stay stopped across the restart.
    let (status, stopped) = http(port, "POST", "/api/sessions", Some(&body));
    assert_eq!(status, 201, "{stopped}");
    let stopped_id = stopped["id"].as_str().unwrap().to_string();
    assert_eq!(http(port, "POST", &format!("/api/sessions/{stopped_id}/kill"), None).0, 204);
    let deadline = Instant::now() + Duration::from_secs(10);
    while http(port, "GET", &format!("/api/sessions/{stopped_id}"), None).1["status"] != "finished" {
        assert!(Instant::now() < deadline, "stopped session never finished");
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    let status = wait_exit(&mut d, Duration::from_secs(20));
    assert!(status.success(), "graceful shutdown exits 0, got {status:?}");

    let logs: Vec<_> = std::fs::read_dir(data.join("logs")).unwrap().flatten().collect();
    assert!(logs.iter().any(|e| e.file_name().to_string_lossy().starts_with("pdtd")), "log file written");

    let _d = start(port, &data);
    // Sessions restart in the background after the daemon starts answering, so poll.
    let deadline = Instant::now() + Duration::from_secs(20);
    let s = loop {
        let (_, s) = http(port, "GET", &format!("/api/sessions/{id}"), None);
        if s["status"] == "running" || Instant::now() > deadline {
            break s;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!((s["status"].as_str(), &s["exitCode"]), (Some("running"), &serde_json::Value::Null), "auto-resumed");
    let (_, s) = http(port, "GET", &format!("/api/sessions/{stopped_id}"), None);
    assert_eq!(s["status"], "finished", "a stopped session is not resumed");
}
