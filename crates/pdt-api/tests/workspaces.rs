mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;

async fn setup(tmp: &Path) -> (axum::Router, AppState, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    (app, state, p["id"].as_str().unwrap().to_string())
}

async fn workspaces(app: &axum::Router) -> Vec<Value> {
    let (s, v) = call(app, "GET", "/api/workspaces", None).await;
    assert_eq!(s, StatusCode::OK);
    v.as_array().unwrap().clone()
}

fn tab_sessions(w: &Value) -> Vec<String> {
    w["tabs"].as_array().unwrap().iter().map(|t| t["sessionId"].as_str().unwrap().to_string()).collect()
}

async fn shell(app: &axum::Router, pid: &str, tab: Option<Value>) -> (StatusCode, Value) {
    let mut body = json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24});
    if let Some(t) = tab {
        body["tab"] = t;
    }
    call(app, "POST", "/api/sessions", Some(body)).await
}

#[tokio::test]
async fn a_new_project_has_its_own_workspace_that_cannot_be_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid) = setup(tmp.path()).await;
    let list = workspaces(&app).await;
    assert_eq!(list.len(), 1);
    assert_eq!((list[0]["name"].as_str(), list[0]["projectId"].as_str()), (Some("P"), Some(pid.as_str())));
    assert_eq!(list[0]["tabs"], json!([]));
    let id = list[0]["id"].as_str().unwrap();
    let (s, v) = call(&app, "DELETE", &format!("/api/workspaces/{id}"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("project_workspace")));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn sessions_open_tabs_in_their_project_workspace_or_where_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid) = setup(tmp.path()).await;
    let home = workspaces(&app).await[0]["id"].as_str().unwrap().to_string();
    let (_, a) = shell(&app, &pid, None).await;
    let (_, b) = shell(&app, &pid, None).await;
    let (s, c) = shell(&app, &pid, Some(json!({"workspaceId": home, "position": 0}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let ids: Vec<String> = [&c, &a, &b].iter().map(|v| v["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(tab_sessions(&workspaces(&app).await[0]), ids);

    let (s, v) = shell(&app, &pid, Some(json!({"workspaceId": "nope"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 3, "a bad placement creates nothing");

    // Removing a session removes its tab.
    let id = &ids[0];
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    for _ in 0..100 {
        if call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await.0 == StatusCode::NO_CONTENT {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(tab_sessions(&workspaces(&app).await[0]), ids[1..]);
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn free_form_workspaces_and_moving_tabs() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid) = setup(tmp.path()).await;
    let (_, a) = shell(&app, &pid, None).await;
    let (_, b) = shell(&app, &pid, None).await;
    let (a, b) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());

    let (s, v) = call(&app, "POST", "/api/workspaces", Some(json!({"name": "  "}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, f) = call(&app, "POST", "/api/workspaces", Some(json!({"name": " Release "}))).await;
    assert_eq!(s, StatusCode::CREATED, "{f}");
    assert_eq!(
        (f["name"].as_str(), f["projectId"].clone(), f["position"].as_u64()),
        (Some("Release"), Value::Null, Some(1))
    );
    let fid = f["id"].as_str().unwrap();

    // Opening an item that has a tab moves the tab.
    let (s, t) = call(&app, "POST", &format!("/api/workspaces/{fid}/tabs"), Some(json!({"sessionId": b}))).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    let list = workspaces(&app).await;
    assert_eq!((tab_sessions(&list[0]), tab_sessions(&list[1])), (vec![a.to_string()], vec![b.to_string()]));

    for (body, code) in [
        (json!({"sessionId": a, "runId": "r"}), "invalid_input"),
        (json!({}), "invalid_input"),
        (json!({"sessionId": "missing"}), "invalid_input"),
    ] {
        let (s, v) = call(&app, "POST", &format!("/api/workspaces/{fid}/tabs"), Some(body.clone())).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some(code)), "{body}");
    }
    let (s, _) = call(&app, "POST", "/api/workspaces/nope/tabs", Some(json!({"sessionId": a}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Move a tab back by PATCH, reorder, close.
    let tid = t["id"].as_str().unwrap();
    let home = list[0]["id"].as_str().unwrap();
    let (s, moved) =
        call(&app, "PATCH", &format!("/api/tabs/{tid}"), Some(json!({"workspaceId": home, "position": 0}))).await;
    assert_eq!((s, moved["position"].as_u64()), (StatusCode::OK, Some(0)), "{moved}");
    assert_eq!(tab_sessions(&workspaces(&app).await[0]), [b, a]);
    let (s, _) = call(&app, "PATCH", &format!("/api/tabs/{tid}"), Some(json!({"workspaceId": "nope"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "DELETE", &format!("/api/tabs/{tid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "DELETE", &format!("/api/tabs/{tid}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(tab_sessions(&workspaces(&app).await[0]), [a]);

    // Rename and reorder workspaces; deleting a free-form one sends its tabs home.
    let (s, w) =
        call(&app, "PATCH", &format!("/api/workspaces/{fid}"), Some(json!({"name": "Ship", "position": 0}))).await;
    assert_eq!((s, w["name"].as_str(), w["position"].as_u64()), (StatusCode::OK, Some("Ship"), Some(0)));
    call(&app, "POST", &format!("/api/workspaces/{fid}/tabs"), Some(json!({"sessionId": a}))).await;
    let (s, _) = call(&app, "DELETE", &format!("/api/workspaces/{fid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let list = workspaces(&app).await;
    assert_eq!(list.len(), 1);
    assert_eq!(tab_sessions(&list[0]), [a]);
    let (s, _) = call(&app, "PATCH", "/api/workspaces/nope", Some(json!({"name": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    state.ptys.shutdown().await;
}
