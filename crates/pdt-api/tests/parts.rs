//! Ticket parts (PDT-38): parents are started through their next part, parts carry their parent in the prompt and
//! share its workspace, and parents roll up and close after their parts. Stand-in agents.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git, recorded_launches};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    bin: PathBuf,
}

async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.codex_home = Some(tmp.join("codex-home"));
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    state.platform.codex_program = argv_agent(&bin, "fakecodex").to_string_lossy().into_owned();
    let app = router(state.clone(), None);
    let body = json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), bin }
}

async fn ticket(env: &Env, body: Value) -> String {
    let (s, v) = call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn start(env: &Env, id: &str, body: Value) -> (StatusCode, Value) {
    let mut body = body;
    body["sessionMode"] = json!("terminal");
    call(&env.app, "POST", &format!("/api/tickets/{id}/start"), Some(body)).await
}

async fn get(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn patch(env: &Env, id: &str, body: Value) -> (StatusCode, Value) {
    call(&env.app, "PATCH", &format!("/api/tickets/{id}"), Some(body)).await
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

/// The (kind, id) of each tab in a workspace, in order.
fn tabs(ws: &Value) -> Vec<(&'static str, String)> {
    ws["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| match (t["sessionId"].as_str(), t["ticketId"].as_str()) {
            (Some(s), _) => ("session", s.to_string()),
            (_, Some(t)) => ("ticket", t.to_string()),
            _ => ("run", String::new()),
        })
        .collect()
}

async fn workspace(env: &Env, name: &str) -> Option<Value> {
    let (_, list) = call(&env.app, "GET", "/api/workspaces", None).await;
    list.as_array().unwrap().iter().find(|w| w["name"] == name).cloned()
}

#[tokio::test]
async fn a_parent_starts_through_its_next_part_and_rolls_up() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let parent = ticket(&env, json!({"title": "Phase 5", "description": "Conversations.", "stage": "todo"})).await;
    let a = ticket(&env, json!({"title": "5a", "parent": parent, "stage": "ready"})).await;
    let b = ticket(&env, json!({"title": "5b", "parent": parent, "blockedByPrevious": true})).await;

    let p = get(&env, &parent).await;
    assert_eq!(p["partList"].as_array().unwrap().iter().map(|t| s(&t["id"])).collect::<Vec<_>>(), [&a, &b]);
    assert_eq!(p["parts"]["total"], 2);
    assert_eq!(p["parts"]["segments"], json!(["backlog", "backlog"]));
    assert_eq!(get(&env, &b).await["parent"], json!({"id": parent, "key": "PDT-1", "title": "Phase 5"}));

    // A parent is never started itself.
    let (st, v) = start(&env, &parent, json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "ticket_has_parts"), "{v}");

    // Its next part: 5a (5b is blocked by it).
    let (st, r) = start(&env, &parent, json!({"agent": "claude", "next": true})).await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    assert_eq!((s(&r["ticket"]["id"]), s(&r["ticket"]["key"])), (a.as_str(), "PDT-2"));
    let p = get(&env, &parent).await;
    assert_eq!(p["stage"], "in_progress", "the parent follows its first part");
    assert_eq!(p["parts"]["segments"], json!(["progress", "backlog"]));
    let (_, ev) = call(&env.app, "GET", &format!("/api/tickets/{parent}/events"), None).await;
    let moved = ev["items"].as_array().unwrap().iter().find(|e| e["kind"] == "moved" && e["actor"] == "system");
    assert!(moved.is_some(), "{ev}");

    // The part's first prompt names its parent and the other parts.
    let launch = recorded_launches(&env.bin, "fakeclaude", 1).await.remove(0);
    let prompt = if cfg!(windows) {
        let sid = s(&r["session"]["id"]);
        std::fs::read_to_string(env.state.data_dir.join("sessions").join(sid).join("prompt.md")).unwrap()
    } else {
        launch.last().unwrap().clone()
    };
    assert!(prompt.starts_with("PDT-2: 5a\n\nPart of PDT-1: Phase 5\n\nConversations."), "{prompt}");
    assert!(prompt.contains("The other parts:\n- PDT-3 5b: Not started"), "{prompt}");

    // Nothing else can start: 5a is running and 5b waits for it.
    let (st, v) = start(&env, &parent, json!({"agent": "claude", "next": true})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "no_next_part"), "{v}");
    assert!(s(&v["message"]).contains("blocked: PDT-3"), "{v}");

    // The backlog finds parents and parts.
    let (_, page) = call(&env.app, "GET", &format!("/api/tickets?project={}&hasParts=true", env.pid), None).await;
    assert_eq!(page["items"][0]["id"], parent.as_str());
    assert_eq!(page["items"][0]["parts"]["progress"], 1);
    let (_, page) = call(&env.app, "GET", &format!("/api/tickets?parent={parent}"), None).await;
    assert_eq!(page["total"], 2);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn parts_share_their_parents_workspace_which_goes_with_the_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let parent = ticket(&env, json!({"title": "Phase 5"})).await;
    let a = ticket(&env, json!({"title": "5a", "parent": parent})).await;
    let b = ticket(&env, json!({"title": "5b", "parent": parent})).await;
    let ra = start(&env, &a, json!({"agent": "claude", "newWorkspace": true})).await.1;
    let rb = start(&env, &b, json!({"agent": "claude", "newWorkspace": true})).await.1;
    let (sa, sb) = (s(&ra["session"]["id"]).to_string(), s(&rb["session"]["id"]).to_string());
    assert_eq!(ra["attempt"]["workspaceId"], Value::Null, "the workspace is the parent's, not the attempt's");

    let ws = workspace(&env, "PDT-1").await.expect("one workspace named after the parent");
    assert_eq!(
        tabs(&ws),
        [
            ("ticket", parent.clone()),
            ("ticket", a.clone()),
            ("session", sa.clone()),
            ("ticket", b.clone()),
            ("session", sb.clone()),
        ]
    );
    assert!(workspace(&env, "PDT-2").await.is_none() && workspace(&env, "PDT-3").await.is_none());
    // The sidebar shows the parent, its newest active part's attempt and the roll-up.
    assert_eq!(ws["ticket"]["id"], parent.as_str());
    assert_eq!(ws["ticket"]["attemptId"], rb["attempt"]["id"]);
    assert_eq!(ws["ticket"]["parts"]["progress"], 2);

    // The parent can't close while a part is open; closing the parts leaves the workspace.
    let (st, v) = patch(&env, &parent, json!({"stage": "done"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "parts_open"), "{v}");
    assert!(s(&v["message"]).contains("PDT-2, PDT-3"), "{v}");
    for id in [&a, &b] {
        assert_eq!(patch(&env, id, json!({"stage": "done"})).await.0, StatusCode::OK);
    }
    assert!(workspace(&env, "PDT-1").await.is_some());

    // With its sessions' tabs still there the workspace stays when the parent closes; with only ticket tabs it goes.
    let (st, v) = patch(&env, &parent, json!({"stage": "done"})).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ws = workspace(&env, "PDT-1").await.expect("kept: it still has session tabs");
    for t in ws["tabs"].as_array().unwrap().iter().filter(|t| t["sessionId"].is_string()) {
        let (st, v) = call(&env.app, "DELETE", &format!("/api/tabs/{}", s(&t["id"])), None).await;
        assert!(st.is_success(), "{v}");
    }
    assert_eq!(patch(&env, &parent, json!({"stage": "in_progress"})).await.0, StatusCode::OK);
    assert_eq!(patch(&env, &parent, json!({"stage": "done"})).await.0, StatusCode::OK);
    assert!(workspace(&env, "PDT-1").await.is_none(), "only ticket tabs were left");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn parts_are_made_in_order_and_the_parent_rules_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let parent = ticket(&env, json!({"title": "Phase 5"})).await;
    let first = ticket(&env, json!({"title": "5a", "parent": parent})).await;
    let body =
        json!({"parts": [{"title": "5b"}, {"title": "5c", "description": "the rest"}], "blockedByPrevious": true});
    let (st, v) = call(&env.app, "POST", &format!("/api/tickets/{parent}/parts"), Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let made: Vec<&str> = v["items"].as_array().unwrap().iter().map(|t| s(&t["key"])).collect();
    assert_eq!(made, ["PDT-3", "PDT-4"]);
    assert!(v["items"].as_array().unwrap().iter().all(|t| t["blockedBy"] == 1));
    let keys: Vec<String> =
        get(&env, &parent).await["partList"].as_array().unwrap().iter().map(|t| s(&t["key"]).to_string()).collect();
    assert_eq!(keys, ["PDT-2", "PDT-3", "PDT-4"]);

    // A part can't have parts; a parent can't be deleted.
    let other = ticket(&env, json!({"title": "Other"})).await;
    let (st, v) = patch(&env, &other, json!({"parent": first})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "invalid_parent"), "{v}");
    let (st, v) = call(&env.app, "DELETE", &format!("/api/tickets/{parent}"), None).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "ticket_has_parts"), "{v}");
    let (st, v) = call(&env.app, "POST", &format!("/api/tickets/{parent}/parts"), Some(json!({"parts": []}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    // Detaching is a blank parent.
    let (st, v) = patch(&env, &first, json!({"parent": ""})).await;
    assert_eq!((st, &v["parent"]), (StatusCode::OK, &Value::Null), "{v}");
}
