//! PDT-323: writes that fail as on a full disk. A run whose start could not be written starts once writes work, and
//! once only; a step that failed to write is retried without using up a retry; a step's result that could not be
//! written is written without running the step again; and the Inbox says PDT can't save changes while it can't.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    db: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let db = tmp.path().join("pdt.db");
    let store = Store::open(&db).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Proj", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().to_string(), db, _tmp: tmp }
}

/// Makes inserts into `table` (those `when` picks) fail as SQLite does when the disk is full: the statement fails and
/// SQLite rolls the whole transaction back itself. None: they work again.
async fn disk_full(db: &std::path::Path, table: &str, when: Option<&str>) {
    use sqlx::Connection;
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(db);
    let mut c = sqlx::SqliteConnection::connect_with(&opts).await.unwrap();
    let name = format!("test_full_{table}");
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP TRIGGER IF EXISTS {name}"))).execute(&mut c).await.unwrap();
    if let Some(when) = when {
        let sql = format!(
            "CREATE TRIGGER {name} BEFORE INSERT ON {table} WHEN {when} \
             BEGIN SELECT RAISE(ROLLBACK, 'database or disk is full'); END"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql)).execute(&mut c).await.unwrap();
    }
    c.close().await.unwrap();
}

async fn create(env: &Env, doc: Value) -> String {
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn ticket(env: &Env) -> String {
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    t["id"].as_str().unwrap().to_string()
}

async fn runs(env: &Env, wf: &str) -> Vec<Value> {
    call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await.1.as_array().unwrap().clone()
}

async fn run(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

async fn storage_item(env: &Env) -> Option<Value> {
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    inbox["items"].as_array().unwrap().iter().find(|i| i["kind"] == "storage").cloned()
}

/// Past the first backoff, so the engine tries writing again.
async fn after_backoff() {
    tokio::time::sleep(Duration::from_millis(u64::try_from(pdt_store::write_backoff_ms(0)).unwrap() + 100)).await;
}

#[tokio::test]
async fn a_run_whose_start_could_not_be_written_starts_once_writes_work() {
    let env = setup().await;
    let tid = ticket(&env).await;
    let wf = create(
        &env,
        json!({"name": "on comments", "nodes": [
        {"id": "t", "kind": "ticket.commented", "version": 1},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "e"}]}),
    )
    .await;
    disk_full(&env.db, "workflow_runs", Some("1")).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/tickets/{tid}/comments"), Some(json!({"body": "hi"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    pdt_api::run_workflows(&env.state).await;
    assert!(runs(&env, &wf).await.is_empty());
    let item = storage_item(&env).await.expect("the Inbox says PDT can't save changes");
    assert_eq!(item["title"], "PDT can't save changes: the disk is full", "{item}");
    assert_eq!(item["body"]["diskFull"], true);
    assert!(item["body"]["freeBytes"].as_u64().is_some(), "{item}");

    disk_full(&env.db, "workflow_runs", None).await;
    // Still backing off.
    pdt_api::run_workflows(&env.state).await;
    assert!(runs(&env, &wf).await.is_empty());
    after_backoff().await;
    pdt_api::run_workflows(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "started once: {r:?}");
    assert_eq!(r[0]["status"], "succeeded");
    assert_eq!(storage_item(&env).await, None, "it goes once writes work");
}

#[tokio::test]
async fn a_step_that_could_not_write_is_retried_without_using_up_a_retry() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "keeps", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "keep", "kind": "state.set", "version": 1, "config": {"key": "k", "value": "v"}}],
        "edges": [{"from": "t", "to": "keep"}]}),
    )
    .await;
    disk_full(&env.db, "workflow_state", Some("1")).await;
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    let keep = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "keep").unwrap().clone();
    assert_eq!((r["status"].as_str(), keep["status"].as_str()), (Some("waiting"), Some("waiting")), "{r}");
    assert_eq!(keep["attempts"], 0, "{keep}");
    assert!(storage_item(&env).await.is_some());

    disk_full(&env.db, "workflow_state", None).await;
    after_backoff().await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "quiet", "{r}");
    let keep = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "keep").unwrap();
    assert_eq!(keep["attempts"], 1, "{keep}");
    assert_eq!(storage_item(&env).await, None);
}

#[tokio::test]
async fn a_result_that_could_not_be_written_is_written_without_running_the_step_again() {
    let env = setup().await;
    let tid = ticket(&env).await;
    let wf = create(
        &env,
        json!({"name": "commenter", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "c", "kind": "ticket.comment", "version": 1, "config": {"ticket": "{{trigger.ticket}}", "body": "hello"}},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "c"}, {"from": "c", "to": "e"}]}),
    )
    .await;
    // The comment is written; the step's end and the next step are not.
    disk_full(&env.db, "workflow_steps", Some("NEW.node_id = 'e'")).await;
    let (_, r) =
        call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {"ticket": tid}}))).await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "running", "{r}");
    assert!(storage_item(&env).await.is_some());

    disk_full(&env.db, "workflow_steps", None).await;
    after_backoff().await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{tid}/events"), None).await;
    let comments = events["items"].as_array().unwrap().iter().filter(|e| e["kind"] == "comment").count();
    assert_eq!(comments, 1, "commented once: {events}");
    // New writes work on every connection.
    for _ in 0..10 {
        let (s, _) =
            call(&env.app, "POST", &format!("/api/tickets/{tid}/comments"), Some(json!({"body": "more"}))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
}
