use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// Router plus a registered project whose repo has one commit on main. Returns (app, state, project id, repo path).
async fn setup(tmp: &Path) -> (axum::Router, AppState, String, std::path::PathBuf) {
    let repo = tmp.join("my repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store);
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED);
    (app, state, p["id"].as_str().unwrap().to_string(), repo)
}

async fn wait_status(app: &axum::Router, id: &str, want: &str) -> Value {
    for _ in 0..100 {
        let (_, s) = call(app, "GET", &format!("/api/sessions/{id}"), None).await;
        if s["status"] == want {
            return s;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("session {id} never became {want}");
}

#[tokio::test]
async fn shell_session_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo) = setup(tmp.path()).await;
    let (s, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    assert_eq!(sess["status"], "running");
    assert_eq!(sess["title"], "Shell");
    assert_eq!(sess["worktreeId"], Value::Null);
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), &repo));
    let id = sess["id"].as_str().unwrap().to_string();

    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (s, v) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "session_running"));

    let (s, _) = call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let ended = wait_status(&app, &id, "finished").await;
    assert!(ended["endedAt"].is_number());
    let (s, v) = call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "not_running"));

    let (s, _) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn restart_finished_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    let id = sess["id"].as_str().unwrap().to_string();
    let (s, v) =
        call(&app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "already_running"));
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    wait_status(&app, &id, "finished").await;
    let (s, v) =
        call(&app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str().unwrap(), &v["exitCode"]), ("running", &Value::Null));
    assert!(state.ptys.is_running(&id));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn missing_program_is_400_and_leaves_no_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, pid, _) = setup(tmp.path()).await;
    state.platform.codex_program = "pdt-no-such-codex-xyz".into();
    let app = router(state, None);
    let (s, v) = call(
        &app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "kind": "agent_tui", "agent": "codex", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::BAD_REQUEST, "program_not_found"));
    assert!(v["message"].as_str().unwrap().contains("pdt-no-such-codex-xyz"));
    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert!(list.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_session_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, pid, _) = setup(tmp.path()).await;
    for (body, status, code) in [
        (
            json!({"projectId": pid, "kind": "agent_tui", "cols": 80, "rows": 24}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"projectId": pid, "kind": "shell", "agent": "claude", "cols": 80, "rows": 24}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (json!({"projectId": pid, "kind": "shell", "cols": 0, "rows": 24}), StatusCode::BAD_REQUEST, "invalid_input"),
        (
            json!({"projectId": pid, "kind": "teleporter", "cols": 80, "rows": 24}),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
        ),
        (json!({"projectId": "nope", "kind": "shell", "cols": 80, "rows": 24}), StatusCode::NOT_FOUND, "not_found"),
        (
            json!({"projectId": pid, "worktreeId": "nope", "kind": "shell", "cols": 80, "rows": 24}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
    ] {
        let (s, v) = call(&app, "POST", "/api/sessions", Some(body.clone())).await;
        assert_eq!((s, v["code"].as_str().unwrap_or("")), (status, code), "{body}");
    }
}

#[tokio::test]
async fn session_runs_in_its_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, wt) = call(&app, "POST", &format!("/api/projects/{pid}/worktrees"), Some(json!({"branch": "w"}))).await;
    let wid = wt["id"].as_str().unwrap();
    let (s, sess) = call(
        &app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "worktreeId": wid, "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(sess["worktreeId"], wid);
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), Path::new(wt["path"].as_str().unwrap())));
    state.ptys.shutdown().await;
}

fn same_dir(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}
