//! Tickets and conversations (structured agents spec §10, Phase 5d): Start in conversation mode, Make ticket's draft
//! and Start from a chat, with `fake-agent` as `claude` and `codex app-server`.

use crate::common;

use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    claude_dir: PathBuf,
}

async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.codex_program = FAKE.to_string();
    state.platform.login_path = None;
    let claude_dir = tmp.join("claude-config");
    state.platform.claude_config_dir = Some(claude_dir.clone());
    state.platform.codex_home = Some(tmp.join("codex-home"));
    let app = router(state.clone(), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, 201, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), claude_dir }
}

async fn ticket(env: &Env, title: &str) -> String {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(json!({"title": title}))).await;
    assert_eq!(s, 201, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn chat(env: &Env, agent: &str, worktree: Option<&str>) -> String {
    let mut body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": agent, "cols": 80, "rows": 24});
    if let Some(w) = worktree {
        body["worktreeId"] = json!(w);
    }
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["mode"], "conversation");
    v["id"].as_str().unwrap().to_string()
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0"), None).await;
    assert_eq!(s, 200, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

async fn wait_turns(env: &Env, id: &str, n: usize) -> Vec<Value> {
    for _ in 0..300 {
        let all = events(env, id).await;
        if all.iter().filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended").count() >= n {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {n} turns end: {:#?}", events(env, id).await);
}

async fn send(env: &Env, id: &str, text: &str) {
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, 202, "{v}");
}

/// Claude saves a conversation after its first turn (the stand-in doesn't).
fn saved(env: &Env, id: &str) {
    let proj = env.claude_dir.join("projects").join("-repo");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(format!("{id}.jsonl")), "{}\n").unwrap();
}

async fn draft(env: &Env, id: &str) -> (u16, Value) {
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/ticket-draft"), None).await;
    (s.as_u16(), v)
}

async fn kill(env: &Env, id: &str) {
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// The (kind, id) of each tab of the project's workspace, in order.
async fn project_tabs(env: &Env) -> Vec<(String, String)> {
    let (_, ws) = call(&env.app, "GET", "/api/workspaces", None).await;
    let ws = ws.as_array().unwrap().iter().find(|w| w["projectId"] == env.pid.as_str()).unwrap().clone();
    ws["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| match (t["sessionId"].as_str(), t["ticketId"].as_str()) {
            (Some(s), _) => ("session".to_string(), s.to_string()),
            (_, Some(t)) => ("ticket".to_string(), t.to_string()),
            _ => ("run".to_string(), t["runId"].as_str().unwrap_or("").to_string()),
        })
        .collect()
}

#[tokio::test]
async fn start_opens_a_conversation_whose_first_message_is_the_ticket() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = ticket(&env, "Fix the login").await;
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{id}/start"), Some(json!({"agent": "claude"}))).await;
    assert_eq!(s, 201, "{r}");
    let session = &r["session"];
    assert_eq!((&session["mode"], &session["permissionMode"]), (&json!("conversation"), &json!("ask")));
    assert_eq!(r["ticket"]["sessions"][0]["mode"], "conversation");
    let sid = session["id"].as_str().unwrap();
    let all = wait_turns(&env, sid, 1).await;
    let first = all.iter().find(|e| e["event"]["kind"] == "user_message").unwrap();
    let text = first["event"]["text"].as_str().unwrap();
    assert!(text.starts_with("PDT-1: Fix the login\n\n(Ticket PDT-1 in PDT."), "{text}");
    kill(&env, sid).await;

    // Bypass is the conversation's permission mode; a terminal is still there when asked for.
    let id2 = ticket(&env, "Second").await;
    let body = json!({"agent": "claude", "bypassPermissions": true, "newWorkspace": true});
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{id2}/start"), Some(body)).await;
    assert_eq!(s, 201, "{r}");
    assert_eq!((&r["session"]["permissionMode"], &r["session"]["bypassPermissions"]), (&json!("bypass"), &json!(true)));
    kill(&env, r["session"]["id"].as_str().unwrap()).await;
}

#[tokio::test]
async fn make_ticket_drafts_from_a_fork_and_leaves_the_chat_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = chat(&env, "claude", None).await;
    let (s, v) = draft(&env, &id).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("nothing_to_draft")), "{v}");

    send(&env, &id, "the login button does nothing").await;
    wait_turns(&env, &id, 1).await;
    saved(&env, &id);
    let before = events(&env, &id).await.len();
    let (s, d) = draft(&env, &id).await;
    assert_eq!(s, 200, "{d}");
    assert_eq!(d["title"], format!("Draft of {id}"));
    let desc = d["description"].as_str().unwrap();
    // A fork of the saved conversation, with no tools, never saved, shaped by the schema; the prompt on stdin.
    assert!(desc.starts_with(r#"resumed=true tools="" saved=false schema=true prompt=Draft a ticket"#), "{desc}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(events(&env, &id).await.len(), before, "the conversation shows nothing of it");

    // Terminals have no conversation to draft from.
    let body = json!({"projectId": env.pid, "kind": "shell", "cols": 80, "rows": 24});
    let (_, shell) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    let (s, v) = draft(&env, shell["id"].as_str().unwrap()).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("not_conversation")), "{v}");
    kill(&env, shell["id"].as_str().unwrap()).await;
    kill(&env, &id).await;
}

#[tokio::test]
async fn codex_drafts_from_an_ephemeral_read_only_fork() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = chat(&env, "codex", None).await;
    send(&env, &id, "hello").await;
    wait_turns(&env, &id, 1).await;
    let thread = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1["externalSessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let before = events(&env, &id).await.len();
    let (s, d) = draft(&env, &id).await;
    assert_eq!(s, 200, "{d}");
    assert_eq!(d["title"], format!("Draft of {thread}"));
    assert_eq!(d["description"], "ephemeral=true sandbox=read-only approval=never schema=true");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(events(&env, &id).await.len(), before, "the side turn never reaches the conversation");
    kill(&env, &id).await;
    pdt_api::shutdown_conversations(&env.state).await;
}

#[tokio::test]
async fn a_chat_in_a_worktree_starts_its_ticket_without_a_new_session() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (s, wt) =
        call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.pid), Some(json!({"branch": "chat-work"})))
            .await;
    assert_eq!(s, 201, "{wt}");
    let wid = wt["id"].as_str().unwrap();
    let id = chat(&env, "claude", Some(wid)).await;
    send(&env, &id, "let's fix it").await;
    wait_turns(&env, &id, 1).await;
    let before = events(&env, &id).await.len();
    let t = ticket(&env, "Fix it").await;
    let start = |t: String, body: Value| {
        let app = env.app.clone();
        async move { call(&app, "POST", &format!("/api/tickets/{t}/start"), Some(body)).await }
    };
    let (s, v) = start(t.clone(), json!({"agent": "claude", "fromSession": id, "branch": "x"})).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (400, Some("invalid_input")), "{v}");

    let (s, r) = start(t.clone(), json!({"agent": "claude", "fromSession": id})).await;
    assert_eq!(s, 201, "{r}");
    assert_eq!(r["session"]["id"], id.as_str(), "the chat itself, no new session");
    assert_eq!(r["session"]["ticket"]["key"], "PDT-1");
    assert_eq!((&r["attempt"]["branch"], &r["attempt"]["worktreeId"]), (&json!("chat-work"), &json!(wid)));
    assert_eq!(r["ticket"]["stage"], "in_progress");
    assert_eq!(r["ticket"]["sessions"].as_array().unwrap().len(), 1);
    // The ticket's tab goes just before the chat's.
    let tabs = project_tabs(&env).await;
    let at = tabs.iter().position(|x| x == &("session".to_string(), id.clone())).unwrap();
    assert!(at > 0 && tabs[at - 1] == ("ticket".to_string(), t.clone()), "{tabs:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(events(&env, &id).await.len(), before, "nothing is sent to the agent");

    // A chat works on one ticket; a chat in the main checkout can't adopt a worktree.
    let t2 = ticket(&env, "Other").await;
    let (s, v) = start(t2.clone(), json!({"agent": "claude", "fromSession": id})).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("chat_not_startable")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("PDT-1"), "{v}");
    let main = chat(&env, "claude", None).await;
    let (s, v) = start(t2, json!({"agent": "claude", "fromSession": main})).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("chat_not_startable")), "{v}");
    kill(&env, &main).await;
    kill(&env, &id).await;
}
