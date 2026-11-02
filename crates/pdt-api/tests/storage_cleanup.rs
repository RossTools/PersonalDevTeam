//! Clean up storage (PDT-370): the built-in that acts on low disk space, Settings › Storage, which sets its inputs,
//! and its nodes (`storage.clean`, `storage.measure`, `pickup.pause`, `pickup.resume`). Measuring and the hourly
//! cleanup's rules are in `storage.rs`.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git};
use pdt_api::AppState;
use pdt_core::storage::{DiskSpace, GB};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    free: Arc<AtomicU64>,
    _tmp: tempfile::TempDir,
}

/// A project whose repository ignores and declares `target` as its build output, on a disk the test moves (500 GB
/// free), auto-start on with no delay.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".gitignore"), "/target\n").unwrap();
    std::fs::write(repo.join(".pdt/project.toml"), "[storage]\nbuild_output = [\"target\"]\n").unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    let free = Arc::new(AtomicU64::new(500 * GB));
    let seen = free.clone();
    state.disk_space = Arc::new(move |_| Some(DiskSpace { free: seen.load(Ordering::SeqCst), total: 1000 * GB }));
    let app = pdt_api::router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let settings = json!({"agentCredentials": null, "runOverrides": null, "autoStart": "todo", "startDelaySeconds": 0});
    let (s, v) = call(&app, "PUT", &format!("/api/projects/{project}/settings"), Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The built-ins are installed.
    pdt_api::run_workflows(&state).await;
    Env { app, state, project, free, _tmp: tmp }
}

/// The disk at `gb` free: the storage loop's look, then the engine acting on what it logged.
async fn disk_at(env: &Env, gb: u64) {
    env.free.store(gb * GB, Ordering::SeqCst);
    pdt_api::storage::watch_disks(&env.state).await;
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

async fn get(env: &Env, path: &str) -> Value {
    let (s, v) = call(&env.app, "GET", path, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn settings(env: &Env) -> Value {
    get(env, "/api/storage/cleanup").await
}

async fn put(env: &Env, values: Value) -> (StatusCode, Value) {
    call(&env.app, "PUT", "/api/storage/cleanup", Some(json!({"values": values}))).await
}

async fn todo_ticket(env: &Env, title: &str) -> String {
    let body = json!({"title": title, "stage": "todo"});
    let (s, t) =
        call_as_ui(&env.state, &env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// A Done ticket whose worktree was kept, with build output in it: (worktree id, its path).
async fn finished_ticket(env: &Env, title: &str) -> (String, PathBuf) {
    let body = json!({"title": title});
    let (_, t) =
        call_as_ui(&env.state, &env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(body)).await;
    let id = t["id"].as_str().unwrap().to_string();
    let body = json!({"agent": "claude", "noSession": true});
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{id}/start"), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let wt = get(env, &format!("/api/tickets/{id}")).await["attempt"]["worktreeId"].as_str().unwrap().to_string();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{id}/finish"), Some(json!({"keepWorktree": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, w) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.project), None).await;
    let path =
        PathBuf::from(w.as_array().unwrap().iter().find(|w| w["id"] == wt.as_str()).unwrap()["path"].as_str().unwrap());
    std::fs::create_dir_all(path.join("target/debug")).unwrap();
    std::fs::write(path.join("target/debug/app"), vec![b'x'; 100_000]).unwrap();
    (wt, path)
}

/// A workflow run by hand: `node` (a `kind` with `config`) leaving by any of `ports` to its end. Its run, ended.
async fn one_step(env: &Env, kind: &str, config: Value, ports: &[&str]) -> Value {
    let edges: Vec<Value> = std::iter::once(json!({"from": "t", "to": "n"}))
        .chain(ports.iter().map(|p| json!({"from": "n", "port": p, "to": "e"})))
        .collect();
    let doc = json!({"name": kind, "nodes": [
        {"id": "t", "kind": "manual", "version": 1, "config": {}},
        {"id": "n", "kind": kind, "version": 1, "config": config},
        {"id": "e", "kind": "end", "version": 1}], "edges": edges});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let run = get(env, &format!("/api/workflow-runs/{id}")).await;
        if !matches!(run["status"].as_str(), Some("running" | "waiting")) {
            return run;
        }
        assert!(Instant::now() < deadline, "never ended: {run}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == node).unwrap_or_else(|| panic!("{run}"))
}

/// With nothing changed it is today's policy: low at 20 GB, critical at 5; critical pauses pickup with a reason on
/// the Board and cleans every finished worktree's build output at once; space again resumes it.
#[tokio::test]
async fn by_default_a_full_disk_pauses_pickup_and_cleans_finished_worktrees() {
    let env = setup().await;
    let v = settings(&env).await;
    assert_eq!((v["enabled"].clone(), v["customized"].clone()), (json!(true), json!(false)), "{v}");
    assert_eq!(v["values"], json!({"lowGb": 20, "criticalGb": 5, "pausePickup": true, "cleanNow": true}));
    let (_, done) = finished_ticket(&env, "Done").await;

    // Low: the Inbox says so, nothing pauses or is cleaned before its delay.
    disk_at(&env, 12).await;
    assert_eq!(get(&env, "/api/pickup/pause").await, Value::Null);
    assert!(done.join("target").exists());

    // Almost full: paused, with why, and the finished worktree cleaned now.
    disk_at(&env, 3).await;
    let pause = get(&env, "/api/pickup/pause").await;
    assert_eq!(pause["reason"], "the disk is almost full (3.0 GB free)", "{pause}");
    assert!(!done.join("target").exists(), "cleaned at once");
    let ticket = todo_ticket(&env, "Next").await;
    let t = get(&env, &format!("/api/tickets/{ticket}")).await;
    assert_eq!(
        (t["pickupPaused"].as_str(), &t["autoStartAt"]),
        (Some("the disk is almost full (3.0 GB free)"), &Value::Null)
    );
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let row = list["items"].as_array().unwrap().iter().find(|r| r["id"] == ticket.as_str()).unwrap();
    assert_eq!(row["pickupPaused"], "the disk is almost full (3.0 GB free)", "on the Board too");

    // Space again: pickup resumes.
    disk_at(&env, 400).await;
    assert_eq!(get(&env, "/api/pickup/pause").await, Value::Null);
    assert_eq!(get(&env, &format!("/api/tickets/{ticket}")).await["pickupPaused"], Value::Null);
}

/// Settings writes the inputs as a new version, and the thresholds and actions follow them.
#[tokio::test]
async fn settings_change_the_thresholds_and_what_happens() {
    let env = setup().await;
    let (s, v) = put(&env, json!({"lowGb": 50, "criticalGb": 45, "cleanNow": false})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["values"]["lowGb"], 50);
    let wf = v["workflowId"].as_str().unwrap().to_string();
    let versions = get(&env, &format!("/api/workflows/{wf}/versions")).await;
    assert_eq!(versions[0]["note"], "inputs changed in Settings", "{versions}");
    let storage = get(&env, "/api/storage").await;
    assert_eq!((storage["lowDiskGb"].as_u64(), storage["criticalDiskGb"].as_u64()), (Some(50), Some(45)));

    // 44 GB free is fine by the defaults, and almost full now.
    let (_, done) = finished_ticket(&env, "Done").await;
    disk_at(&env, 44).await;
    let f = pdt_store::EventFilter { kind: Some("disk.low".into()), limit: 10, ..Default::default() };
    let low = env.state.store.list_log_events(&f).await.unwrap();
    assert_eq!(low.len(), 1, "{low:?}");
    assert_eq!((&low[0].payload["level"], &low[0].payload["criticalGb"]), (&json!("critical"), &json!(45)));
    assert!(get(&env, "/api/pickup/pause").await["reason"].as_str().unwrap().contains("almost full"));
    assert!(done.join("target").exists(), "cleanNow is off");
    disk_at(&env, 400).await;

    // Pausing off: a full disk only warns, and the Inbox doesn't say auto-start waits.
    let (s, _) = put(&env, json!({"pausePickup": false})).await;
    assert_eq!(s, StatusCode::OK);
    disk_at(&env, 3).await;
    assert_eq!(get(&env, "/api/pickup/pause").await, Value::Null);
    let inbox = get(&env, "/api/inbox").await;
    let item = inbox["items"].as_array().unwrap().iter().find(|i| i["kind"] == "low_disk").unwrap().clone();
    assert_eq!(item["title"], "The disk is almost full (3.0 GB free)");

    for bad in [json!({"lowGb": -1}), json!({"criticalGb": 2.5}), json!({"lowGb": 20000}), json!({"nope": 1})] {
        let (s, e) = put(&env, bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {e}");
    }
}

/// A workflow changed beyond its inputs is only named by Settings: a PUT is refused, Reset to default restores it.
#[tokio::test]
async fn a_customized_cleanup_is_reset_to_default() {
    let env = setup().await;
    let wf = settings(&env).await["workflowId"].as_str().unwrap().to_string();
    let w = get(&env, &format!("/api/workflows/{wf}")).await;
    let mut doc = w["doc"].clone();
    let hourly = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "hourly").unwrap();
    hourly["config"]["every"] = json!("6h");
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{wf}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(settings(&env).await["customized"], true);
    let (s, e) = put(&env, json!({"lowGb": 30})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("customized")), "{e}");

    let (s, v) = call(&env.app, "POST", "/api/storage/cleanup/reset", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["customized"].as_bool(), v["enabled"].as_bool()), (Some(false), Some(true)), "{v}");
    let w = get(&env, &format!("/api/workflows/{wf}")).await;
    let hourly = w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "hourly").unwrap().clone();
    assert_eq!(hourly["config"]["every"], "1h");
}

/// `pickup.pause` and `pickup.resume`: a workflow resumes only its own pause unless `any`; the user resumes any.
#[tokio::test]
async fn pickup_pauses_and_resumes_with_a_reason() {
    let env = setup().await;
    let run = one_step(&env, "pickup.pause", json!({"reason": "release freeze"}), &["out"]).await;
    assert_eq!(step(&run, "n")["output"]["reason"], "release freeze", "{run}");
    let ticket = todo_ticket(&env, "Waits").await;
    common::pick_up(&env.state).await;
    let t = get(&env, &format!("/api/tickets/{ticket}")).await;
    assert_eq!((t["stage"].as_str(), t["pickupPaused"].as_str()), (Some("todo"), Some("release freeze")), "{t}");

    let run = one_step(&env, "pickup.resume", json!({}), &["resumed", "not_paused"]).await;
    assert_eq!(step(&run, "n")["port"], "not_paused", "another workflow's pause stays");
    assert_eq!(get(&env, "/api/pickup/pause").await["reason"], "release freeze");
    let run = one_step(&env, "pickup.resume", json!({"any": true}), &["resumed", "not_paused"]).await;
    assert_eq!(step(&run, "n")["port"], "resumed");
    assert_eq!(get(&env, "/api/pickup/pause").await, Value::Null);

    one_step(&env, "pickup.pause", json!({"reason": "again"}), &["out"]).await;
    let (s, _) = call(&env.app, "DELETE", "/api/pickup/pause", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    common::pick_up(&env.state).await;
    let t = get(&env, &format!("/api/tickets/{ticket}")).await;
    assert_eq!(t["stage"], "in_progress", "{t}");
    for s in get(&env, &format!("/api/projects/{}/sessions", env.project)).await.as_array().unwrap() {
        call(&env.app, "POST", &format!("/api/sessions/{}/kill", s["id"].as_str().unwrap()), None).await;
    }

    let (s, e) = call(
        &env.app,
        "POST",
        "/api/workflows",
        Some(json!({"doc": {"name": "x", "nodes": [{"id": "n", "kind": "pickup.pause", "version": 1, "config": {"reason": " "}}], "edges": []}})),
    )
    .await;
    assert!(s == StatusCode::BAD_REQUEST || e.to_string().contains("reason is empty"), "{s} {e}");
}

/// `storage.clean` by stage and age, removing whole worktrees; `storage.measure` by its own thresholds.
#[tokio::test]
async fn the_storage_nodes_clean_by_stage_and_age_and_measure() {
    let env = setup().await;
    let (wt, path) = finished_ticket(&env, "Done").await;
    let run = one_step(&env, "storage.clean", json!({"which": "finished", "stages": ["wont_do"]}), &["out"]).await;
    assert_eq!(step(&run, "n")["output"]["cleaned"], 0, "only Won't do");
    let run = one_step(&env, "storage.clean", json!({"which": "finished", "olderThanDays": 1}), &["out"]).await;
    assert_eq!(step(&run, "n")["output"]["cleaned"], 0, "closed just now");
    let run = one_step(&env, "storage.clean", json!({"which": "finished", "what": "worktree"}), &["out"]).await;
    assert_eq!(step(&run, "n")["output"]["removed"], 1, "{run}");
    assert!(!path.exists());
    let (_, w) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.project), None).await;
    assert!(!w.as_array().unwrap().iter().any(|w| w["id"] == wt.as_str()));

    let ports = ["ok", "low", "critical"];
    let run = one_step(&env, "storage.measure", json!({"lowGb": 600, "criticalGb": 10}), &ports).await;
    assert_eq!(step(&run, "n")["port"], "low", "500 GB free is under 600");
    assert_eq!(step(&run, "n")["output"]["freeText"], "500 GB");
    let run = one_step(&env, "storage.measure", json!({}), &ports).await;
    assert_eq!(step(&run, "n")["port"], "ok", "Clean up storage's thresholds");
}
