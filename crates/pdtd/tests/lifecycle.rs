//! Starts the real pdtd binary on a free port with a temp data dir. Never touches the stable instance.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::common;
use common::*;

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
    wait_run_since(port, id, want, 0)
}

/// Waits until the run is `want` with `updatedAt` at or after `since` (Unix ms): a row read back after a restart is
/// `ready` from before the shutdown until the resume changes it.
fn wait_run_since(port: u16, id: &str, want: &str, since: i64) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, r) = http(port, "GET", &format!("/api/runs/{id}"), None);
        if r["state"] == want && r["updatedAt"].as_i64().is_some_and(|t| t >= since) {
            return r;
        }
        assert!(Instant::now() < deadline, "run never became {want} after {since}: {r}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A block of 40 run ports in 20000–29999, below every OS's ephemeral range (Linux starts at 32768), so a
/// port another test's connection was just handed by the OS never lands in it. No two tests share a block.
fn run_port_base() -> u16 {
    port_blocks::claim(20000, 250, 40)
}

/// A project whose repo has RUN_CONFIG, its own run port range (`run_port_base`), and a started web run.
fn project_with_web_run(port: u16, tmp: &Path) -> (String, u16) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    std::fs::write(repo.join(".pdt").join("project.toml"), RUN_CONFIG).unwrap();
    let (_, settings) = http(port, "GET", "/api/host/settings", None);
    assert_eq!(settings["settings"]["runPorts"], serde_json::json!({"start": 8500, "end": 8599}), "dev defaults");
    let base = run_port_base();
    let body = serde_json::json!({"runPorts": {"start": base, "end": base + 39}}).to_string();
    assert_eq!(http(port, "PUT", "/api/host/settings", Some(&body)).0, 200);
    let body = serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string();
    let (_, p) = http(port, "POST", "/api/projects", Some(&body));
    approve_commands(port, p["id"].as_str().unwrap());
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
    let restarted_at = now_ms();
    let mut d = start(port, &data);
    let r = wait_run_since(port, &id, "ready", restarted_at);
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
    let restarted_at = now_ms();
    let mut d = start(port, &data);
    let r = wait_run_since(port, &id, "ready", restarted_at);
    assert_ne!(r["port"].as_u64(), Some(run_port as u64));
    shut_down(port, &mut d);
}

/// Final review #1: a dev instance never updates, even under pdt-host (PDT_SUPERVISED=1).
#[test]
fn a_dev_pdtd_never_updates_even_when_supervised() {
    let tmp = tempfile::tempdir().unwrap();
    let port = free_port();
    let _d = start_with(port, &tmp.path().join("data"), &[("PDT_SUPERVISED", "1")]);
    let (_, v) = http(port, "GET", "/api/host/update", None);
    assert_eq!(v["state"], "disabled", "{v}");
    assert_eq!(v["disabledReason"], "updates apply to the installed stable app", "{v}");
    let (status, v) = http(port, "POST", "/api/host/update/install", Some(r#"{"force":true}"#));
    assert_eq!((status, v["code"].as_str()), (409, Some("updates_disabled")), "{v}");
}

/// Final review #11: SIGHUP shuts pdtd down gracefully, like SIGTERM: it exits 0 rather than dying of the signal.
#[cfg(unix)]
#[test]
fn sighup_shuts_down_gracefully() {
    let tmp = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut d = start(port, &tmp.path().join("data"));
    let status = Command::new("kill").args(["-HUP", &d.0.id().to_string()]).status().unwrap();
    assert!(status.success());
    let exit = wait_exit(&mut d, Duration::from_secs(20));
    assert!(exit.success(), "a graceful shutdown exits 0, not by the signal: {exit:?}");
}

#[test]
fn demo_seed_fills_an_empty_dev_database_once() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let seed = [("PDT_SEED", "demo")];

    let mut d = start_with(port, &data, &seed);
    let (status, projects) = http(port, "GET", "/api/projects", None);
    assert_eq!(status, 200, "{projects}");
    let names: Vec<_> = projects.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Sample App", "Sample Library"], "{}", diagnose(&mut d, port, &data));
    let app = projects[0]["id"].as_str().unwrap().to_string();
    let (_, sessions) = http(port, "GET", &format!("/api/projects/{app}/sessions"), None);
    assert_eq!(sessions.as_array().map(Vec::len), Some(5), "{sessions}");
    assert!(sessions.as_array().unwrap().iter().all(|s| s["status"] == "finished"), "{sessions}");
    let (_, runs) = http(port, "GET", &format!("/api/projects/{app}/runs"), None);
    assert_eq!(runs.as_array().map(Vec::len), Some(3), "{runs}");
    let (_, config) = http(port, "GET", &format!("/api/projects/{app}/run-config"), None);
    assert_eq!(config["error"], serde_json::Value::Null, "{config}");
    assert_eq!(config["effective"]["runs"][0]["name"], "web", "{config}");

    // Restarting with the seed still set keeps what is there instead of adding a second copy.
    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    assert!(wait_exit(&mut d, Duration::from_secs(20)).success());
    let _d = start_with(port, &data, &seed);
    let (_, projects) = http(port, "GET", "/api/projects", None);
    assert_eq!(projects.as_array().map(Vec::len), Some(2), "{projects}");
}

#[test]
fn without_the_seed_a_dev_database_starts_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let _d = start(port, &data);
    let (_, projects) = http(port, "GET", "/api/projects", None);
    assert_eq!(projects, serde_json::json!([]));
}

/// PDT-179: an upgrade rehearsal (`PDT_REHEARSAL`) migrates and serves the copy, with the built-in workflows
/// installed, but resumes nothing: a session the last run left is still interrupted.
#[test]
fn a_rehearsal_serves_the_copy_and_resumes_nothing() {
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
    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    assert!(wait_exit(&mut d, Duration::from_secs(20)).success());

    let _d = start_with(port, &data, &[("PDT_REHEARSAL", "1")]);
    let (_, workflows) = http(port, "GET", "/api/workflows", None);
    assert!(workflows.as_array().unwrap().iter().any(|w| !w["builtin"].is_null()), "built-ins installed: {workflows}");
    wait::still_after_blocking("the session resuming", Duration::from_secs(2), || {
        http(port, "GET", &format!("/api/sessions/{id}"), None).1["status"] != "interrupted"
    });
}
