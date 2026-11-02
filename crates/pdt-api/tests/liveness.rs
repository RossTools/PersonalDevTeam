//! Background loops' liveness (PDT-127): `/api/health` reports each loop, and a stale loop is a notification. (A
//! sensor's calls ending within their timeout, and the poll going on past a hung one, are tested with an
//! integration's check, in its crate.)

use crate::common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
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
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = common::app_state(store, tmp.path());
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Work", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, _tmp: tmp }
}

async fn loop_notifications(app: &axum::Router) -> Vec<Value> {
    let (_, v) = call(app, "GET", "/api/notifications?section=updates", None).await;
    let items = v["items"].as_array().cloned().unwrap_or_else(|| v.as_array().cloned().unwrap_or_default());
    items.into_iter().filter(|n| n["kind"] == "host.loop_stale" && n["clearedAt"].is_null()).collect()
}

#[tokio::test]
async fn health_reports_each_loop_and_a_stale_one_is_a_notification() {
    let env = setup().await;
    env.state.loops.register("poll", Duration::from_secs(60), Some(30));
    env.state.loops.register("engine", Duration::from_secs(2), None);
    env.state.loops.passed("engine", pdt_api_now(), false);
    let (s, h) = call(&env.app, "GET", "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    let loops = h["loops"].as_array().unwrap();
    let names: Vec<&str> = loops.iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["engine", "poll"]);
    assert!(loops[0]["lastPassAt"].is_i64() && loops[0]["stale"] == false, "{h}");
    assert_eq!((loops[1]["everyMs"].clone(), loops[1]["lastPassAt"].clone()), (json!(60_000), Value::Null));

    tokio::time::sleep(Duration::from_millis(60)).await;
    let (_, h) = call(&env.app, "GET", "/api/health", None).await;
    assert_eq!(h["status"], "ok");
    assert_eq!(h["loops"][1]["stale"], true, "{h}");
    pdt_api::sync_notifications(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    let rows = loop_notifications(&env.app).await;
    assert_eq!(rows.len(), 1, "one row, deduped: {rows:?}");
    assert_eq!(rows[0]["title"], "PDT's poll loop stopped");
    assert_eq!(rows[0]["severity"], "error");

    // A pass clears it.
    env.state.loops.passed("poll", pdt_api_now(), false);
    pdt_api::sync_notifications(&env.state).await;
    assert!(loop_notifications(&env.app).await.is_empty());
}

fn pdt_api_now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
