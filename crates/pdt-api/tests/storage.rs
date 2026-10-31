//! Worktree storage (PDT-324): sizes measured in the background, cleaning build output, the automatic cleanup of
//! finished tickets' worktrees, and low disk space (an Inbox item; auto-start waits while it is critical).

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::storage::{DiskSpace, GB};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const DAY: i64 = 24 * 60 * 60 * 1000;

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    repo: PathBuf,
    /// What the disks have free.
    free: Arc<AtomicU64>,
    _tmp: tempfile::TempDir,
}

/// A project whose repository ignores `target` and `node_modules` and declares `target` and `dist` as its build
/// output.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".gitignore"), "/target\nnode_modules/\n").unwrap();
    std::fs::write(repo.join(".pdt/project.toml"), "[storage]\nbuild_output = [\"target\", \"dist\"]\n").unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    let free = Arc::new(AtomicU64::new(500 * GB));
    let seen = free.clone();
    state.disk_space = Arc::new(move |_| Some(DiskSpace { free: seen.load(Ordering::SeqCst), total: 1000 * GB }));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), repo, free, _tmp: tmp }
}

fn write(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// Fills a worktree with build output (declared and ignored), output that isn't ignored, and other ignored files.
fn fill(dir: &Path) {
    write(&dir.join("target/debug/app"), 300_000);
    write(&dir.join("dist/bundle.js"), 1000);
    write(&dir.join("node_modules/pkg/index.js"), 1000);
}

async fn worktree(env: &Env, branch: &str) -> (String, PathBuf) {
    let (s, w) =
        call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.project), Some(json!({"branch": branch})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    (w["id"].as_str().unwrap().into(), PathBuf::from(w["path"].as_str().unwrap()))
}

/// A ticket started (no session) and finished keeping its worktree: Done, with the worktree still there.
async fn finished_ticket(env: &Env, title: &str) -> (String, String, PathBuf) {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let ticket = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1;
    let wt = ticket["attempt"]["worktreeId"].as_str().unwrap().to_string();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{id}/finish"), Some(json!({"keepWorktree": true}))).await;
    assert_eq!((s, v["outcome"].as_str()), (StatusCode::OK, Some("kept")), "{v}");
    let path = storage(env).await["projects"][0]["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == wt.as_str())
        .map(|w| PathBuf::from(w["path"].as_str().unwrap()))
        .unwrap();
    (id, wt, path)
}

async fn storage(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/storage", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn worktree_of<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["projects"][0]["worktrees"].as_array().unwrap().iter().find(|w| w["id"] == id).unwrap()
}

/// GET /api/storage once every worktree has been measured since the refresh.
async fn measured(env: &Env) -> Value {
    let (s, _) = call(&env.app, "POST", "/api/storage/refresh", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let v = storage(env).await;
        let all = v["projects"][0]["worktrees"].as_array().unwrap().iter().all(|w| w["size"].is_object());
        if all && v["measuring"] == false {
            return v;
        }
        assert!(Instant::now() < deadline, "never measured: {v}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Tests never read the machine's own disk: a state that sets no probe has no disks, so nothing is ever low.
#[tokio::test]
async fn a_state_without_a_probe_sees_no_disks_and_never_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let app = router(AppState::new(store, tmp.path().join("data")), None);
    let (_, v) = call(&app, "GET", "/api/storage", None).await;
    assert_eq!((v["disks"].clone(), v["autoStartPaused"].clone()), (json!([]), json!(false)));
    let (_, inbox) = call(&app, "GET", "/api/inbox", None).await;
    assert!(!inbox["items"].as_array().unwrap().iter().any(|i| i["kind"] == "low_disk"), "{inbox}");
}

#[tokio::test]
async fn the_storage_view_measures_worktrees_and_their_build_output() {
    let env = setup().await;
    let (id, path) = worktree(&env, "feature").await;
    fill(&path);
    std::fs::write(path.join("a.txt"), "changed\n").unwrap();
    let v = measured(&env).await;
    let p = &v["projects"][0];
    assert_eq!(p["buildOutput"], json!({"paths": ["target", "dist"], "detected": false, "error": null}));
    let size = &worktree_of(&v, &id)["size"];
    assert!(size["build"].as_u64().unwrap() >= 300_000, "{size}");
    assert!(size["total"].as_u64().unwrap() > size["build"].as_u64().unwrap(), "{size}");
    assert_eq!(size["uncommitted"], true);
    assert_eq!(p["total"], size["total"]);
    assert_eq!(worktree_of(&v, &id)["ticket"], Value::Null);
    assert_eq!(v["disks"][0]["space"], json!({"free": 500 * GB, "total": 1000 * GB}));
    assert_eq!(v["disks"][0]["level"], "ok");
    assert_eq!((v["lowDiskGb"].as_u64(), v["criticalDiskGb"].as_u64()), (Some(20), Some(5)));
}

#[tokio::test]
async fn clean_removes_only_the_declared_build_output_git_ignores() {
    let env = setup().await;
    let (id, path) = worktree(&env, "feature").await;
    fill(&path);
    write(&path.join("src/new.rs"), 10);
    let (s, r) = call(&env.app, "POST", &format!("/api/worktrees/{id}/clean"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["removed"], json!(["target"]));
    assert_eq!(r["kept"], json!(["dist"]), "dist isn't ignored: it may be work");
    assert!(r["freed"].as_u64().unwrap() >= 300_000, "{r}");
    assert!(!path.join("target").exists());
    for kept in ["dist/bundle.js", "node_modules/pkg/index.js", "src/new.rs", "a.txt"] {
        assert!(path.join(kept).exists(), "{kept}");
    }
    let (s, v) = call(&env.app, "POST", "/api/worktrees/nope/clean", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
}

#[tokio::test]
async fn with_none_declared_build_output_is_detected() {
    let env = setup().await;
    std::fs::remove_file(env.repo.join(".pdt/project.toml")).unwrap();
    std::fs::write(env.repo.join("package.json"), "{}").unwrap();
    let (id, path) = worktree(&env, "feature").await;
    fill(&path);
    write(&path.join("web/node_modules/x/y.js"), 10);
    let v = storage(&env).await;
    let out = &v["projects"][0]["buildOutput"];
    assert_eq!(out["detected"], true);
    assert_eq!(out["paths"][0], "**/node_modules");
    let (_, r) = call(&env.app, "POST", &format!("/api/worktrees/{id}/clean"), None).await;
    let mut removed: Vec<&str> = r["removed"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    removed.sort();
    assert_eq!(removed, ["node_modules", "web/node_modules"], "{r}");
    assert!(path.join("target").exists(), "not detected for this project");
}

#[tokio::test]
async fn the_cleanup_cleans_finished_tickets_worktrees_after_the_delay_and_removes_them_when_asked() {
    let env = setup().await;
    let (ticket, wt, path) = finished_ticket(&env, "Done one").await;
    fill(&path);
    let v = storage(&env).await;
    assert_eq!(worktree_of(&v, &wt)["ticket"]["stage"], "done");
    assert_eq!(worktree_of(&v, &wt)["ticket"]["finished"], true);
    // A worktree with no ticket is never touched.
    let (_, other) = worktree(&env, "mine").await;
    fill(&other);

    let now =
        i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()).unwrap();
    pdt_api::storage::auto_cleanup_at(&env.state, now + DAY).await;
    assert!(path.join("target").exists(), "a day in: the default waits two");
    pdt_api::storage::auto_cleanup_at(&env.state, now + 2 * DAY + 1000).await;
    assert!(!path.join("target").exists(), "two days after Done");
    assert!(path.join("dist").exists() && other.join("target").exists());

    // Removing it later: refused while it has uncommitted changes, then done.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/storage", env.project),
        Some(json!({"cleanAfterDays": 2, "removeAfterDays": 5})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    std::fs::write(path.join("a.txt"), "changed\n").unwrap();
    pdt_api::storage::auto_cleanup_at(&env.state, now + 6 * DAY).await;
    assert!(path.is_dir(), "uncommitted work: kept");
    git(&path, &["checkout", "--", "a.txt"]);
    std::fs::remove_dir_all(path.join("dist")).unwrap();
    pdt_api::storage::auto_cleanup_at(&env.state, now + 6 * DAY).await;
    assert!(!path.exists(), "removed after five days");
    assert!(other.is_dir());
    let branches = std::process::Command::new("git").current_dir(&env.repo).args(["branch"]).output().unwrap();
    let t = call(&env.app, "GET", &format!("/api/tickets/{ticket}"), None).await.1;
    let branch = t["attempts"][0]["branch"].as_str().unwrap().to_string();
    assert!(String::from_utf8_lossy(&branches.stdout).contains(&branch), "its branch stays");

    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/storage", env.project),
        Some(json!({"cleanAfterDays": 400, "removeAfterDays": 0})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
}

#[tokio::test]
async fn clean_all_finished_skips_open_tickets_and_worktrees_in_use() {
    let env = setup().await;
    let (_, _, done) = finished_ticket(&env, "Done").await;
    fill(&done);
    let (_, open) = worktree(&env, "open").await;
    fill(&open);
    let (s, r) = call(&env.app, "POST", "/api/storage/clean-finished", None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!((r["cleaned"].as_u64(), r["skipped"].as_array().unwrap().len()), (Some(1), 0), "{r}");
    assert!(!done.join("target").exists());
    assert!(open.join("target").exists());
}

#[tokio::test]
async fn low_disk_space_warns_in_the_inbox_and_pauses_auto_start_when_critical() {
    let env = setup().await;
    let inbox = || async {
        let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
        v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "low_disk").cloned().collect::<Vec<_>>()
    };
    assert!(inbox().await.is_empty());

    env.free.store(12 * GB, Ordering::SeqCst);
    let items = inbox().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "low_disk");
    assert_eq!(items[0]["title"], "Disk space is low (12 GB free)");

    // Critical: auto-start waits (no countdown either), and says so.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/settings", env.project),
        Some(json!({"agentCredentials": null, "runOverrides": null, "autoStart": "todo", "startDelaySeconds": 0})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    env.free.store(3 * GB, Ordering::SeqCst);
    let items = inbox().await;
    assert_eq!(items[0]["id"], "low_disk:critical");
    assert!(items[0]["title"].as_str().unwrap().contains("auto-start is paused"));
    assert_eq!(storage(&env).await["autoStartPaused"], true);
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": "Next", "stage": "todo"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap();
    pdt_api::schedule_tickets(&env.state).await;
    let t = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1;
    assert_eq!((t["stage"].as_str(), &t["autoStartAt"]), (Some("todo"), &Value::Null), "{t}");

    // Space again: the warning goes, and auto-start takes the ticket.
    env.free.store(100 * GB, Ordering::SeqCst);
    assert!(inbox().await.is_empty());
    pdt_api::schedule_tickets(&env.state).await;
    let t = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1;
    assert_eq!(t["stage"], "in_progress", "{t}");
    for s in call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await.1.as_array().unwrap()
    {
        call(&env.app, "POST", &format!("/api/sessions/{}/kill", s["id"].as_str().unwrap()), None).await;
    }
}
