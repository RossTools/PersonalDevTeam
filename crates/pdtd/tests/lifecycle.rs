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

const RUN_CONFIG: &str = r#"
[[run]]
name = "web"
command = "node -e \"require('http').createServer((q, s) => s.end('ok')).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true
"#;

fn wait_run(port: u16, id: &str, want: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, r) = http(port, "GET", &format!("/api/runs/{id}"), None);
        if r["state"] == want {
            return r;
        }
        assert!(Instant::now() < deadline, "run never became {want}: {r}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A project whose repo has RUN_CONFIG, a run port range starting at a free port, and a started web run.
fn project_with_web_run(port: u16, tmp: &Path) -> (String, u16) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    std::fs::write(repo.join(".pdt").join("project.toml"), RUN_CONFIG).unwrap();
    let (_, settings) = http(port, "GET", "/api/host/settings", None);
    assert_eq!(settings["settings"]["runPorts"], serde_json::json!({"start": 8500, "end": 8599}), "dev defaults");
    let base = free_port().min(65000);
    let body = serde_json::json!({"runPorts": {"start": base, "end": base + 39}}).to_string();
    assert_eq!(http(port, "PUT", "/api/host/settings", Some(&body)).0, 200);
    let body = serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string();
    let (_, p) = http(port, "POST", "/api/projects", Some(&body));
    let body = serde_json::json!({"name": "web", "kind": "run"}).to_string();
    let (s, r) = http(port, "POST", &format!("/api/projects/{}/runs", p["id"].as_str().unwrap()), Some(&body));
    assert_eq!(s, 201, "{r}");
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_run(port, &id, "ready");
    (id, r["port"].as_u64().unwrap() as u16)
}

/// Shuts the daemon down gracefully, which also ends the runs' servers.
fn shut_down(port: u16, d: &mut Daemon) {
    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    wait_exit(d, Duration::from_secs(20));
}

#[test]
fn active_runs_resume_on_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let mut d = start(port, &data);
    let (id, run_port) = project_with_web_run(port, tmp.path());
    shut_down(port, &mut d);
    let mut d = start(port, &data);
    let r = wait_run(port, &id, "ready");
    assert_eq!(r["port"].as_u64(), Some(run_port as u64), "same port when it is still free");
    shut_down(port, &mut d);
}

#[test]
fn resume_moves_to_a_free_port_when_the_old_one_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let mut d = start(port, &data);
    let (id, run_port) = project_with_web_run(port, tmp.path());
    shut_down(port, &mut d);
    let mut squatter = None;
    for _ in 0..100 {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", run_port)) {
            squatter = Some(l);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _squatter = squatter.expect("the old run's port was freed by the shutdown");
    let mut d = start(port, &data);
    let r = wait_run(port, &id, "ready");
    assert_ne!(r["port"].as_u64(), Some(run_port as u64));
    shut_down(port, &mut d);
}
