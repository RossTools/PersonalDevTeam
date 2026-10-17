mod common;
#[path = "../../../test-support/port_blocks.rs"]
mod port_blocks;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_tailscale::testing::RecordingTailscale;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// A web server in node (CI has node for pnpm): answers every request with the run's branch.
const CONFIG: &str = r#"
[[run]]
name = "web"
setup = "node -e \"require('fs').writeFileSync(require('path').join(process.env.PDT_RUN_DATA, 'setup.txt'), String(process.env.PORT))\""
command = "node -e \"require('http').createServer((q, s) => s.end(process.env.PDT_RUN_BRANCH)).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true
data = true

[[run]]
name = "webhost"
command = "node -e \"require('http').createServer((q, s) => s.end(process.env.SEEN_HOST)).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true
env = { SEEN_HOST = "$PDT_RUN_HOST" }

[[run]]
name = "broken-setup"
setup = "node -e \"process.exit(3)\""
command = "node -e \"setTimeout(() => {}, 60000)\""

[[run]]
name = "web6"
command = "node -e \"require('http').createServer((q, s) => s.end('six')).listen(Number(process.env.PORT), '::1')\""
web = true

[[run]]
name = "idle"
command = "node -e \"setTimeout(() => {}, 120000)\""

[[test]]
name = "ok"
command = "node -e \"process.exit(0)\""

[[test]]
name = "bad"
command = "node -e \"process.exit(1)\""

[[test]]
name = "shim"
command = '.\failing-shim'
"#;

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

/// A block of 40 run ports in 30000–39999, below the OS ephemeral ranges (Windows and macOS start at 49152) and apart
/// from pdtd's lifecycle tests (20000–29999), so a port the OS hands to some connection never lands in it. Each call
/// claims a block no other test holds (`port_blocks`).
fn free_range() -> (u16, u16) {
    let start = port_blocks::claim(30000, 250, 40);
    (start, start + 39)
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".pdt").join("project.toml"), CONFIG).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (start, end) = free_range();
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"runPorts": {"start": start, "end": end}}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), repo, _tmp: tmp }
}

async fn start(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", &format!("/api/projects/{}/runs", env.pid), Some(body)).await
}

async fn wait_state(app: &axum::Router, id: &str, want: &[&str]) -> Value {
    for _ in 0..600 {
        let (_, r) = call(app, "GET", &format!("/api/runs/{id}"), None).await;
        if want.contains(&r["state"].as_str().unwrap_or("")) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, r) = call(app, "GET", &format!("/api/runs/{id}"), None).await;
    panic!("run {id} never reached {want:?}: {r}");
}

async fn http_get(port: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf
}

#[tokio::test]
async fn web_run_becomes_ready_and_serves_its_branch() {
    let env = setup().await;
    let (s, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["branch"], "main");
    let id = r["id"].as_str().unwrap();
    let r = wait_state(&env.app, id, &["ready", "failed"]).await;
    assert_eq!(r["state"], "ready", "{r}");
    let port = r["port"].as_u64().unwrap() as u16;
    assert_eq!(r["url"], format!("http://127.0.0.1:{port}/"));
    assert!(http_get(port).await.ends_with("main"));
    let data = PathBuf::from(r["dataDir"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(data.join("setup.txt")).unwrap(), port.to_string(), "setup saw $PORT");
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/runs", env.pid), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, active) = call(&env.app, "GET", "/api/runs", None).await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, id, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn stopping_a_web_run_frees_its_port() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    let port = r["port"].as_u64().unwrap() as u16;
    let (s, _) = call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let r = wait_state(&env.app, &id, &["stopped", "failed"]).await;
    assert_eq!(r["state"], "stopped", "{r}");
    let mut freed = false;
    for _ in 0..100 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(freed, "the node server (a grandchild of the shell) still holds port {port}");
    let (s, v) = call(&env.app, "POST", &format!("/api/runs/{id}/restart"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    assert_eq!(r["state"], "ready", "restarts after a stop: {r}");
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn failing_setup_fails_the_run() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "broken-setup", "kind": "run"})).await;
    let r = wait_state(&env.app, r["id"].as_str().unwrap(), &["failed", "ready"]).await;
    assert_eq!((r["state"].as_str(), r["exitCode"].as_i64()), (Some("failed"), Some(3)), "{r}");
    assert!(r["error"].as_str().unwrap().contains("setup"));
    assert_eq!(r["port"], Value::Null, "not a web run");
}

#[tokio::test]
async fn tests_pass_and_fail() {
    let env = setup().await;
    let (_, ok) = start(&env, json!({"name": "ok", "kind": "test"})).await;
    let (_, bad) = start(&env, json!({"name": "bad", "kind": "test"})).await;
    let ok = wait_state(&env.app, ok["id"].as_str().unwrap(), &["passed", "failed"]).await;
    let bad = wait_state(&env.app, bad["id"].as_str().unwrap(), &["passed", "failed"]).await;
    assert_eq!(ok["state"], "passed");
    assert_eq!((bad["state"].as_str(), bad["exitCode"].as_i64()), (Some("failed"), Some(1)));
    let (s, v) = start(&env, json!({"name": "nope", "kind": "test"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_run")));
}

/// A step that runs a `.cmd` shim (like pnpm.cmd) keeps the shim's exit code (ruling F22). This shim's code is lost
/// when the step's batch file hands control to it without `call`.
#[cfg(windows)]
#[tokio::test]
async fn a_cmd_shim_keeps_its_exit_code() {
    let env = setup().await;
    std::fs::write(
        env.repo.join("failing-shim.cmd"),
        "@echo off\r\nsetlocal\r\nnode -e \"process.exit(3)\"\r\nendlocal\r\n",
    )
    .unwrap();
    let (_, r) = start(&env, json!({"name": "shim", "kind": "test"})).await;
    let r = wait_state(&env.app, r["id"].as_str().unwrap(), &["passed", "failed"]).await;
    assert_eq!((r["state"].as_str(), r["exitCode"].as_i64()), (Some("failed"), Some(3)), "{r}");
}

#[tokio::test]
async fn the_run_limit_asks_before_stopping_another_run() {
    let env = setup().await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 1}))).await;
    let (_, first) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let first = first["id"].as_str().unwrap().to_string();
    wait_state(&env.app, &first, &["ready"]).await;
    let (s, v) = start(&env, json!({"name": "broken-setup", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("too_many_runs")), "{v}");
    let (s, v) = start(&env, json!({"name": "ok", "kind": "test"})).await;
    assert_eq!(s, StatusCode::CREATED, "tests do not count: {v}");
    let (s, v) = start(&env, json!({"name": "broken-setup", "kind": "run", "stopRunId": first})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    wait_state(&env.app, &first, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn restarting_a_run_counts_toward_the_limit() {
    let env = setup().await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 1}))).await;
    let (_, a) = start(&env, json!({"name": "idle", "kind": "run"})).await;
    let a = a["id"].as_str().unwrap().to_string();
    call(&env.app, "POST", &format!("/api/runs/{a}/stop"), None).await;
    wait_state(&env.app, &a, &["stopped"]).await;
    let (_, b) = start(&env, json!({"name": "idle", "kind": "run"})).await;
    let b = b["id"].as_str().unwrap().to_string();
    let restart = format!("/api/runs/{a}/restart");
    let (s, v) = call(&env.app, "POST", &restart, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("too_many_runs")), "{v}");
    let (s, v) = call(&env.app, "POST", &restart, Some(json!({"stopRunId": "not-active"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("too_many_runs")), "{v}");
    let (s, v) = call(&env.app, "POST", &restart, Some(json!({"stopRunId": b}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_state(&env.app, &b, &["stopped"]).await;
    let r = wait_state(&env.app, &a, &["starting", "ready", "running"]).await;
    assert!(r["error"].is_null(), "{r}");
    call(&env.app, "POST", &format!("/api/runs/{a}/stop"), None).await;
    env.state.ptys.shutdown().await;
}

fn recording() -> Arc<RecordingTailscale> {
    Arc::new(RecordingTailscale::new(Ok(pdt_tailscale::TsStatus {
        dns_name: "box.t.ts.net".into(),
        self_login: None,
        https_enabled: true,
    })))
}

#[tokio::test]
async fn remote_runs_get_https_links_and_mappings() {
    let mut env = setup().await;
    let fake = recording();
    env.state.tailscale = fake.clone();
    *env.state.remote.lock().unwrap() = pdt_api::RemoteStatus {
        enabled: true,
        active: true,
        dns_name: Some("box.t.ts.net".into()),
        ..Default::default()
    };
    env.app = router(env.state.clone(), None);
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    let port = r["port"].as_u64().unwrap();
    assert_eq!(r["url"], format!("https://box.t.ts.net:{port}/"));
    assert!(fake.calls().contains(&format!("serve --bg --https={port} http://127.0.0.1:{port}")), "{:?}", fake.calls());
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    assert_eq!(fake.calls().last().unwrap(), &format!("serve --https={port} off"));
    env.state.ptys.shutdown().await;
}

/// Review Focus 5, with the controller's ruling: a run the previous pdtd left on a port that is now taken moves to a
/// free one. Applying remote access at startup maps nothing for it (the old port may now be another instance's),
/// the resumed run maps its new port, and the old port is never unserved.
#[tokio::test]
async fn a_resumed_run_that_moves_port_maps_only_its_new_port() {
    let mut env = setup().await;
    let fake = recording();
    env.state.tailscale = fake.clone();
    env.app = router(env.state.clone(), None);
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let old = squatter.local_addr().unwrap().port();
    let now = 1;
    let left_behind = pdt_core::run::Run {
        id: "left-behind".into(),
        project_id: env.pid.clone(),
        worktree_id: None,
        branch: "main".into(),
        name: "web".into(),
        kind: pdt_core::run::RunKind::Run,
        state: pdt_core::run::RunState::Ready,
        port: Some(old),
        url: Some(format!("https://box.t.ts.net:{old}/")),
        data_dir: Some(env._tmp.path().join("data").join("old-run").to_string_lossy().into_owned()),
        cwd: env.repo.to_string_lossy().into_owned(),
        exit_code: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    env.state.store.insert_run(&left_behind).await.unwrap();
    let (s, v) = call(
        &env.app,
        "PUT",
        "/api/host/settings",
        Some(json!({"remoteAccess": true, "ownerLogin": "me@example.com"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // pdtd's startup order: apply remote access, then resume.
    pdt_api::apply_remote_access(&env.state).await;
    assert!(fake.calls().iter().all(|c| !c.contains(&format!("--https={old}"))), "{:?}", fake.calls());
    assert_eq!(pdt_api::resume_runs(&env.state).await, 1);
    // The row still says "ready" from the previous daemon, so wait for the new server itself.
    let (_, r) = call(&env.app, "GET", "/api/runs/left-behind", None).await;
    let new = r["port"].as_u64().unwrap() as u16;
    assert_ne!(new, old, "{r}");
    assert_eq!(r["url"], format!("https://box.t.ts.net:{new}/"));
    for _ in 0..600 {
        if tokio::net::TcpStream::connect(("127.0.0.1", new)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    wait_state(&env.app, "left-behind", &["ready"]).await;
    let calls = fake.calls();
    assert!(calls.iter().all(|c| !c.contains(&format!("--https={old}"))), "the old port is left alone: {calls:?}");
    assert!(calls.contains(&format!("serve --bg --https={new} http://127.0.0.1:{new}")), "{calls:?}");
    call(&env.app, "POST", "/api/runs/left-behind/stop", None).await;
    wait_state(&env.app, "left-behind", &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

/// Turning remote access on maps the running web runs and gives them tailnet links; turning it off removes those
/// mappings and gives back loopback links.
#[tokio::test]
async fn toggling_remote_access_updates_running_web_runs() {
    let mut env = setup().await;
    let fake = recording();
    env.state.tailscale = fake.clone();
    env.app = router(env.state.clone(), None);
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready"]).await;
    let port = r["port"].as_u64().unwrap();
    assert_eq!(r["url"], format!("http://127.0.0.1:{port}/"));
    let on = json!({"remoteAccess": true, "ownerLogin": "me@example.com"});
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(on)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(fake.calls().contains(&format!("serve --bg --https={port} http://127.0.0.1:{port}")), "{:?}", fake.calls());
    let (_, r) = call(&env.app, "GET", &format!("/api/runs/{id}"), None).await;
    assert_eq!(r["url"], format!("https://box.t.ts.net:{port}/"));
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(fake.calls().contains(&format!("serve --https={port} off")), "{:?}", fake.calls());
    let (_, r) = call(&env.app, "GET", &format!("/api/runs/{id}"), None).await;
    assert_eq!(r["url"], format!("http://127.0.0.1:{port}/"));
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

/// Waits until the web server on `port` answers with a body ending in `want`.
async fn wait_body(port: u16, want: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let get = async || -> std::io::Result<String> {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").await?;
        let mut buf = String::new();
        s.read_to_string(&mut buf).await?;
        Ok(buf)
    };
    for _ in 0..600 {
        if let Ok(Ok(body)) = tokio::time::timeout(Duration::from_secs(2), get()).await
            && body.ends_with(want)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("port {port} never answered {want}: {:?}", get().await);
}

/// Final review #9: a web run whose definition uses `$PDT_RUN_HOST` (a dev PDT's allowlist) restarts on its port when
/// remote access changes the host; a run that does not use it keeps running untouched.
#[tokio::test]
async fn runs_that_use_the_run_host_restart_when_remote_access_changes_it() {
    let mut env = setup().await;
    env.state.tailscale = recording();
    env.app = router(env.state.clone(), None);
    let (_, r) = start(&env, json!({"name": "webhost", "kind": "run"})).await;
    let host_run = r["id"].as_str().unwrap().to_string();
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let plain_run = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &host_run, &["ready"]).await;
    let port = r["port"].as_u64().unwrap() as u16;
    let plain = wait_state(&env.app, &plain_run, &["ready"]).await;
    wait_body(port, "127.0.0.1").await;
    let setup_txt = PathBuf::from(plain["dataDir"].as_str().unwrap()).join("setup.txt");
    std::fs::remove_file(&setup_txt).unwrap();

    let on = json!({"remoteAccess": true, "ownerLogin": "me@example.com"});
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(on)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_body(port, "box.t.ts.net").await;
    let r = wait_state(&env.app, &host_run, &["ready"]).await;
    assert_eq!(
        (r["port"].as_u64(), r["url"].as_str()),
        (Some(port as u64), Some(format!("https://box.t.ts.net:{port}/").as_str()))
    );

    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_body(port, "127.0.0.1").await;
    let r = wait_state(&env.app, &host_run, &["ready"]).await;
    assert_eq!(r["url"], format!("http://127.0.0.1:{port}/"));

    let (_, p) = call(&env.app, "GET", &format!("/api/runs/{plain_run}"), None).await;
    assert_eq!(p["state"], "ready", "{p}");
    assert!(!setup_txt.exists(), "the run that does not use the host was not restarted");
    for id in [&host_run, &plain_run] {
        call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
        wait_state(&env.app, id, &["stopped"]).await;
    }
    env.state.ptys.shutdown().await;
}

/// Starts are serialized: two at once, with room for one more run, start exactly one; and two web runs started
/// at once never get the same port.
#[tokio::test]
async fn concurrent_starts_respect_the_limit_and_never_share_a_port() {
    let env = setup().await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 2}))).await;
    let (s, v) = start(&env, json!({"name": "idle", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let web = json!({"name": "web", "kind": "run"});
    let ((s1, a), (s2, b)) = tokio::join!(start(&env, web.clone()), start(&env, web.clone()));
    let mut codes = [s1, s2];
    codes.sort();
    assert_eq!(codes, [StatusCode::CREATED, StatusCode::CONFLICT], "{a} {b}");
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 8}))).await;
    let ((s3, c), (s4, d)) = tokio::join!(start(&env, web.clone()), start(&env, web.clone()));
    assert_eq!((s3, s4), (StatusCode::CREATED, StatusCode::CREATED), "{c} {d}");
    let ports: std::collections::BTreeSet<_> = [&a, &b, &c, &d].iter().filter_map(|r| r["port"].as_u64()).collect();
    assert_eq!(ports.len(), 3, "three web runs, three ports: {a} {b} {c} {d}");
    env.state.ptys.shutdown().await;
}

/// A stop that does not finish in time leaves the run alone instead of starting another over it.
#[tokio::test]
async fn a_stop_that_times_out_blocks_the_restart() {
    let mut env = setup().await;
    env.state.run_stop_wait = Duration::ZERO;
    env.app = router(env.state.clone(), None);
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 2}))).await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (_, r) = start(&env, json!({"name": "idle", "kind": "run"})).await;
        ids.push(r["id"].as_str().unwrap().to_string());
    }
    for id in &ids {
        wait_state(&env.app, id, &["ready"]).await;
    }
    let (s, v) = start(&env, json!({"name": "idle", "kind": "run", "stopRunId": ids[0]})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("run_stopping")), "{v}");
    let (s, v) = call(&env.app, "POST", &format!("/api/runs/{}/restart", ids[1]), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("run_stopping")), "{v}");
    let (_, runs) = call(&env.app, "GET", &format!("/api/projects/{}/runs", env.pid), None).await;
    assert_eq!(runs.as_array().unwrap().len(), 2, "nothing new was started: {runs}");
    env.state.ptys.shutdown().await;
}

/// A web server that listens only on IPv6 loopback still counts as ready.
#[tokio::test]
async fn readiness_also_probes_ipv6_loopback() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "web6", "kind": "run"})).await;
    let r = wait_state(&env.app, r["id"].as_str().unwrap(), &["ready", "failed"]).await;
    assert_eq!(r["state"], "ready", "{r}");
    env.state.ptys.shutdown().await;
}

/// With remote access off, a web run runs no tailscale command at all (ruling F1).
#[tokio::test]
async fn local_runs_make_no_tailscale_calls() {
    let mut env = setup().await;
    let fake = recording();
    env.state.tailscale = fake.clone();
    env.app = router(env.state.clone(), None);
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    wait_state(&env.app, &id, &["ready"]).await;
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    assert_eq!((fake.calls(), fake.status_calls()), (Vec::<String>::new(), 0));
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_branch_without_a_worktree_gets_one() {
    let env = setup().await;
    git(&env.repo, &["branch", "feature/x"]);
    let (_, branches) = call(&env.app, "GET", &format!("/api/projects/{}/branches", env.pid), None).await;
    assert!(branches.as_array().unwrap().iter().any(|b| b["name"] == "feature/x"), "{branches}");
    let (s, r) = start(&env, json!({"branch": "feature/x", "name": "ok", "kind": "test"})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["branch"], "feature/x");
    assert!(r["worktreeId"].is_string());
    let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert_eq!(wts.as_array().unwrap().len(), 1);
    wait_state(&env.app, r["id"].as_str().unwrap(), &["passed"]).await;
    let (s, r2) = start(&env, json!({"branch": "feature/x", "name": "ok", "kind": "test"})).await;
    assert_eq!((s, &r2["worktreeId"]), (StatusCode::CREATED, &r["worktreeId"]), "reuses the worktree");
    let (s, r3) = start(&env, json!({"branch": "main", "name": "ok", "kind": "test"})).await;
    assert_eq!((s, &r3["worktreeId"]), (StatusCode::CREATED, &Value::Null), "the main checkout's branch");
}

/// A start that is refused creates no worktree; a branch whose worktree folder another branch already has is a
/// conflict, not a server error.
#[tokio::test]
async fn refused_starts_leave_no_worktree_behind() {
    let env = setup().await;
    for b in ["feature/z", "feature/x", "feature-x"] {
        git(&env.repo, &["branch", b]);
    }
    let worktrees = || async {
        let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
        wts.as_array().unwrap().len()
    };
    let (s, v) = start(&env, json!({"branch": "feature/z", "name": "nope", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_run")), "{v}");
    let (s, v) = start(&env, json!({"branch": "no-such-branch", "name": "idle", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_ref")), "{v}");
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 1}))).await;
    let (_, first) = start(&env, json!({"name": "idle", "kind": "run"})).await;
    let (s, v) = start(&env, json!({"branch": "feature/z", "name": "idle", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("too_many_runs")), "{v}");
    assert_eq!(worktrees().await, 0);
    assert!(!env.repo.join(".worktrees").join("feature-z").exists());
    call(&env.app, "POST", &format!("/api/runs/{}/stop", first["id"].as_str().unwrap()), None).await;
    let (s, v) = start(&env, json!({"branch": "feature-x", "name": "ok", "kind": "test"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = start(&env, json!({"branch": "feature/x", "name": "ok", "kind": "test"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("worktree_path_taken")), "{v}");
    assert_eq!(worktrees().await, 1);
    env.state.ptys.shutdown().await;
}

/// The sweep goes by the run's folder and branch, so it also stops a run whose worktree record is gone.
#[tokio::test]
async fn the_sweep_needs_no_worktree_record() {
    let env = setup().await;
    let elsewhere = env._tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let mut run = pdt_core::run::Run {
        id: "orphan".into(),
        project_id: env.pid.clone(),
        worktree_id: None,
        branch: "deleted-branch".into(),
        name: "idle".into(),
        kind: pdt_core::run::RunKind::Run,
        state: pdt_core::run::RunState::Ready,
        port: None,
        url: None,
        data_dir: None,
        cwd: elsewhere.to_string_lossy().into_owned(),
        exit_code: None,
        error: None,
        created_at: 1,
        updated_at: 1,
    };
    env.state.store.insert_run(&run).await.unwrap();
    run.id = "missing-folder".into();
    run.branch = "main".into();
    run.cwd = env._tmp.path().join("no-such-folder").to_string_lossy().into_owned();
    env.state.store.insert_run(&run).await.unwrap();
    pdt_api::sweep_gone_runs(&env.state).await;
    for id in ["orphan", "missing-folder"] {
        let r = wait_state(&env.app, id, &["stopped"]).await;
        assert!(r["error"].as_str().unwrap_or("").contains("gone"), "{r}");
    }
}

#[tokio::test]
async fn the_sweep_stops_runs_whose_worktree_is_gone() {
    let env = setup().await;
    git(&env.repo, &["branch", "feature/y"]);
    let (_, kept) = start(&env, json!({"name": "idle", "kind": "run"})).await;
    let (_, gone) = start(&env, json!({"branch": "feature/y", "name": "idle", "kind": "run"})).await;
    let (kept, gone) = (kept["id"].as_str().unwrap().to_string(), gone["id"].as_str().unwrap().to_string());
    wait_state(&env.app, &gone, &["ready"]).await;
    wait_state(&env.app, &kept, &["ready"]).await;
    pdt_api::sweep_gone_runs(&env.state).await;
    let (_, r) = call(&env.app, "GET", &format!("/api/runs/{gone}"), None).await;
    assert_eq!(r["state"], "ready", "nothing is gone yet");
    // The branch ref disappears (git refuses `branch -D` for a checked-out branch, and Windows refuses to delete a
    // folder a process works in, so the ref is removed directly).
    git(&env.repo, &["update-ref", "-d", "refs/heads/feature/y"]);
    pdt_api::sweep_gone_runs(&env.state).await;
    let r = wait_state(&env.app, &gone, &["stopped"]).await;
    assert!(r["error"].as_str().unwrap_or("").contains("gone"), "{r}");
    let (_, r) = call(&env.app, "GET", &format!("/api/runs/{kept}"), None).await;
    assert_eq!(r["state"], "ready", "a main-checkout run is never swept: {r}");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn run_terminals_are_websockets_per_step() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "ok", "kind": "test"})).await;
    let id = r["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "GET", &format!("/api/runs/{id}/pty/command"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("websocket_required")));
    let (s, _) = call(&env.app, "GET", &format!("/api/runs/{id}/pty/nope"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = call(&env.app, "GET", "/api/runs/nope/pty/command", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_run")));
}

#[tokio::test]
async fn data_reset_and_removal_need_a_stopped_run() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready"]).await;
    let data = PathBuf::from(r["dataDir"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &format!("/api/runs/{id}/reset-data"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("run_active")));
    let (s, _) = call(&env.app, "DELETE", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/runs/{id}/reset-data"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(data.is_dir() && std::fs::read_dir(&data).unwrap().next().is_none(), "emptied, still there");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&env.app, "GET", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
