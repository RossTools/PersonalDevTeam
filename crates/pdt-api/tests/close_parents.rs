//! Close finished parents (PDT-377): a parent closes when its last part does, with a comment; a part opening again
//! reopens it; a parent with work of its own stays open; and the board's Done window is a project setting.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
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
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Proj", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

async fn ticket(env: &Env, body: Value) -> String {
    let (s, v) = call_as_ui(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

/// Moves a ticket, then lets the workflows react.
async fn move_to(env: &Env, id: &str, stage: &str) {
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{id}"), Some(json!({"stage": stage}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
    }
}

async fn stage(env: &Env, id: &str) -> String {
    call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1["stage"].as_str().unwrap().into()
}

async fn comments(env: &Env, id: &str) -> Vec<String> {
    let (_, ev) = call(&env.app, "GET", &format!("/api/tickets/{id}/events"), None).await;
    ev["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "comment")
        .map(|e| e["body"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn the_last_part_closing_closes_the_parent_and_reopening_one_reopens_it() {
    let env = setup().await;
    let parent = ticket(&env, json!({"title": "Phase 5", "stage": "todo"})).await;
    let a = ticket(&env, json!({"title": "5a", "parent": parent, "stage": "todo"})).await;
    let b = ticket(&env, json!({"title": "5b", "parent": parent, "stage": "todo"})).await;

    move_to(&env, &a, "done").await;
    assert_eq!(stage(&env, &parent).await, "todo", "a part is still open");
    move_to(&env, &b, "wont_do").await;
    assert_eq!(stage(&env, &parent).await, "done");
    let said = comments(&env, &parent).await;
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("All parts closed: moved to Done when PROJ-3 closed"), "{said:?}");
    let (_, notes) = call(&env.app, "GET", "/api/notifications", None).await;
    let titles: Vec<&str> = notes["items"].as_array().unwrap().iter().filter_map(|n| n["title"].as_str()).collect();
    assert!(titles.contains(&"PROJ-1 closed: all its parts are closed"), "{titles:?}");

    move_to(&env, &a, "in_progress").await;
    assert_eq!(stage(&env, &parent).await, "in_progress", "a part opened again");
    assert!(comments(&env, &parent).await.iter().any(|c| c.starts_with("PROJ-2 opened again")));

    // Every part dropped: the parent is dropped too.
    move_to(&env, &a, "wont_do").await;
    assert_eq!(stage(&env, &parent).await, "wont_do");
}

#[tokio::test]
async fn a_parent_with_work_of_its_own_stays_open() {
    let env = setup().await;
    let parent = ticket(&env, json!({"title": "Phase 5", "stage": "todo"})).await;
    let a = ticket(&env, json!({"title": "5a", "parent": parent, "stage": "todo"})).await;
    env.state.store.insert_attempt("at1", &parent, None, "pdt/PROJ-1-a", "main", "abc123", 1).await.unwrap();
    move_to(&env, &a, "done").await;
    assert_eq!(stage(&env, &parent).await, "todo");
    assert!(comments(&env, &parent).await.is_empty());
    let (_, notes) = call(&env.app, "GET", "/api/notifications", None).await;
    let titles: Vec<&str> = notes["items"].as_array().unwrap().iter().filter_map(|n| n["title"].as_str()).collect();
    assert!(titles.contains(&"All parts of PROJ-1 are closed"), "{titles:?}");
}

#[tokio::test]
async fn turned_off_it_leaves_parents_alone() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let wf = list.as_array().unwrap().iter().find(|w| w["builtin"] == "close-parents").expect("installed").clone();
    assert_eq!(wf["enabled"], true, "on by default");
    let id = wf["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let parent = ticket(&env, json!({"title": "Phase 5", "stage": "todo"})).await;
    let a = ticket(&env, json!({"title": "5a", "parent": parent, "stage": "todo"})).await;
    move_to(&env, &a, "done").await;
    assert_eq!(stage(&env, &parent).await, "todo");
}

#[tokio::test]
async fn the_done_window_is_saved_per_project() {
    let env = setup().await;
    let url = format!("/api/projects/{}/board", env.pid);
    assert_eq!(call(&env.app, "GET", &url, None).await.1, json!({"doneWindowDays": 14}));
    let (s, v) = call(&env.app, "PUT", &url, Some(json!({"doneWindowDays": 3}))).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"doneWindowDays": 3})));
    assert_eq!(call(&env.app, "GET", &url, None).await.1, json!({"doneWindowDays": 3}));
    let (s, v) = call(&env.app, "PUT", &url, Some(json!({"doneWindowDays": 5}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");
    let (s, _) = call(&env.app, "GET", "/api/projects/nope/board", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
