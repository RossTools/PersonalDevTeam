//! Codex conversations end to end, with `fake-agent app-server` standing in for `codex app-server` (plan 5b).

mod common;

use common::{call, git};
use futures_util::{SinkExt, StreamExt};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
}

async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.codex_program = FAKE.to_string();
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.codex_home = Some(tmp.join("codex-home"));
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, 201, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into() }
}

async fn new_codex(env: &Env, extra: Value) -> String {
    let mut body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "codex", "cols": 80, "rows": 24});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["mode"], "conversation", "Codex opens as a conversation by default");
    v["id"].as_str().unwrap().to_string()
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
    assert_eq!(s, 200, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

async fn wait_for(env: &Env, id: &str, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    for _ in 0..400 {
        let all = events(env, id).await;
        if f(&all) {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {what}: {:#?}", events(env, id).await);
}

fn ended(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        .map(|e| e["event"]["status"].as_str().unwrap().to_string())
        .collect()
}

async fn wait_turns(env: &Env, id: &str, n: usize) -> Vec<Value> {
    wait_for(env, id, &format!("{n} turns"), |a| ended(a).len() >= n).await
}

fn said(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

fn open_request(all: &[Value]) -> Option<String> {
    all.iter()
        .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
        .map(|e| e["event"]["id"].as_str().unwrap().to_string())
}

async fn send(env: &Env, id: &str, text: &str, delivery: &str) {
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{id}/messages"),
        Some(json!({"text": text, "delivery": delivery})),
    )
    .await;
    assert_eq!(s, 202, "{v}");
}

async fn answer(env: &Env, id: &str, req: &str, a: Value) -> (u16, Value) {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/requests/{req}"), Some(json!({"answer": a}))).await;
    (s.as_u16(), v)
}

async fn session(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

async fn stop(env: &Env, id: &str) {
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_codex_conversation_streams_and_keeps_its_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({"initialPrompt": "hello"})).await;
    let all = wait_turns(&env, &id, 1).await;
    let kinds: Vec<&str> = all.iter().map(|e| e["event"]["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["turn", "user_message", "agent_message", "usage", "turn"], "{all:#?}");
    assert_eq!(all[1]["event"]["source"], "pdt");
    assert_eq!(said(&all), ["echo: hello"]);
    let s = session(&env, &id).await;
    assert!(s["externalSessionId"].as_str().unwrap().starts_with("thr-"), "{s}");
    assert_eq!(s["activity"], "idle");
    let (_, caps) = call(&env.app, "GET", "/api/agents/codex/capabilities", None).await;
    assert_eq!(caps["models"].as_array().unwrap().len(), 1, "hidden models are left out: {caps}");
    assert_eq!(caps["models"][0]["value"], "gpt-fake");

    // Settings ride along on the next turn.
    let (st, v) = call(
        &env.app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        Some(json!({"permissionMode": "plan", "model": "gpt-fake"})),
    )
    .await;
    assert_eq!(st.as_u16(), 200, "{v}");
    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 2).await;
    let who = said(&all).pop().unwrap();
    assert!(
        who.contains("model=gpt-fake approval=on-request sandbox=readOnly") && who.contains("resumed=false"),
        "{who}"
    );
    stop(&env, &id).await;
}

#[tokio::test]
async fn approvals_always_allow_for_the_session_and_deny_notes_follow() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({"initialPrompt": "run: touch a"})).await;
    let all = wait_for(&env, &id, "a request", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    let r = all.iter().find(|e| e["event"]["kind"] == "request").unwrap();
    assert_eq!((r["event"]["title"].as_str(), r["event"]["canAlways"].as_bool()), (Some("touch a"), Some(true)));
    assert_eq!(session(&env, &id).await["activity"], "needs_you");
    assert_eq!(answer(&env, &id, &req, json!({"type": "approve", "always": true})).await.0, 204);
    let (s, v) = answer(&env, &id, &req, json!({"type": "deny"})).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("request_closed")));
    let all = wait_turns(&env, &id, 1).await;
    assert!(said(&all).last().unwrap().contains("decision=acceptForSession"), "{:?}", said(&all));
    let tool = all.iter().find(|e| e["event"]["kind"] == "tool_call").unwrap();
    assert_eq!(
        (tool["event"]["status"].as_str(), tool["event"]["output"].as_str()),
        (Some("done"), Some("ran touch a"))
    );

    // Deny with a note: Codex's decline has no message, so the note follows as the next message (steered into the
    // turn that is still running).
    send(&env, &id, "run: rm -rf /", "next").await;
    let all = wait_for(&env, &id, "a second request", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!(answer(&env, &id, &req, json!({"type": "deny", "message": "too risky"})).await.0, 204);
    let all = wait_for(&env, &id, "the note's answer", |a| said(a).contains(&"echo: too risky".to_string())).await;
    assert!(said(&all).contains(&"Not running it.".to_string()), "{:?}", said(&all));
    assert_eq!(ended(&all).len(), 2, "the note joined the running turn");
    let declined = all.iter().filter(|e| e["event"]["kind"] == "tool_call").nth(1).unwrap();
    assert_eq!(declined["event"]["status"], "declined");
    stop(&env, &id).await;
}

#[tokio::test]
async fn messages_steer_a_running_turn_send_now_restarts_it_and_interrupt_expires_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({"initialPrompt": "slow"})).await;
    wait_for(&env, &id, "the slow turn", |a| a.iter().any(|e| e["event"]["kind"] == "user_message")).await;
    send(&env, &id, "also this", "next").await;
    let all = wait_turns(&env, &id, 1).await;
    assert!(said(&all)[0].contains("steered=[\"also this\"]"), "{:?}", said(&all));
    let steer = all.iter().filter(|e| e["event"]["kind"] == "user_message").nth(1).unwrap();
    assert_eq!(steer["event"]["source"], "pdt", "a steer PDT sent is PDT's");

    send(&env, &id, "slow", "next").await;
    wait_for(&env, &id, "the second slow turn", |a| {
        a.iter().filter(|e| e["event"]["kind"] == "user_message").count() >= 3
    })
    .await;
    send(&env, &id, "urgent", "now").await;
    let all = wait_turns(&env, &id, 3).await;
    assert_eq!(ended(&all), ["completed", "interrupted", "completed"]);
    assert_eq!(said(&all).last().unwrap(), "echo: urgent");

    send(&env, &id, "run: make", "next").await;
    wait_for(&env, &id, "a request", |a| open_request(a).is_some()).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert_eq!(s.as_u16(), 204);
    let all = wait_turns(&env, &id, 4).await;
    assert_eq!(ended(&all).last().unwrap(), "interrupted");
    assert!(open_request(&all).is_none());
    assert!(all.iter().any(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "expired"));
    stop(&env, &id).await;
}

/// Connects to PDT's app-server as another client would (the TUI), using the token PDT wrote.
async fn second_client(
    env: &Env,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let dir = env.state.data_dir.join("codex");
    let url = std::fs::read_to_string(dir.join("fake-url")).unwrap();
    let token = std::fs::read_to_string(dir.join("token")).unwrap();
    let mut req = url.as_str().into_client_request().unwrap();
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "tui", "version": "0"}}});
    ws.send(Message::Text(init.to_string().into())).await.unwrap();
    ws
}

async fn rpc_until(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    msg: Value,
    until: &str,
) {
    ws.send(Message::Text(msg.to_string().into())).await.unwrap();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
        let v: Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["method"] == until || (until == "result" && v.get("result").is_some() && v["id"] == msg["id"]) {
            return;
        }
    }
}

#[tokio::test]
async fn turns_taken_elsewhere_are_imported_when_the_conversation_reopens() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = setup(tmp.path()).await;
    env.state.conversations.idle_park = Duration::ZERO;
    env.app = router(env.state.clone(), None);
    let id = new_codex(&env, json!({"initialPrompt": "first"})).await;
    wait_turns(&env, &id, 1).await;
    pdt_api::park_idle_conversations(&env.state).await;
    let thread = session(&env, &id).await["externalSessionId"].as_str().unwrap().to_string();

    // Another client (as the TUI would) takes a turn while PDT is not listening.
    let mut tui = second_client(&env).await;
    rpc_until(
        &mut tui,
        json!({"jsonrpc": "2.0", "id": 2, "method": "thread/resume", "params": {"threadId": thread}}),
        "result",
    )
    .await;
    let start = json!({"jsonrpc": "2.0", "id": 3, "method": "turn/start",
                       "params": {"threadId": thread, "input": [{"type": "text", "text": "from tui"}]}});
    rpc_until(&mut tui, start, "turn/completed").await;

    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 3).await;
    let users: Vec<(&str, &str)> = all
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| (e["event"]["text"].as_str().unwrap(), e["event"]["source"].as_str().unwrap()))
        .collect();
    assert_eq!(users, [("first", "pdt"), ("from tui", "terminal"), ("who", "pdt")], "{all:#?}");
    let s = said(&all);
    assert_eq!(s[1], "echo: from tui");
    assert!(s[2].contains("resumed=true"), "{s:?}");
    stop(&env, &id).await;
}

#[tokio::test]
async fn a_crashed_server_restarts_and_resumes_the_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({"initialPrompt": "first"})).await;
    wait_turns(&env, &id, 1).await;
    let thread = session(&env, &id).await["externalSessionId"].clone();
    send(&env, &id, "exit", "next").await;
    let all = wait_for(&env, &id, "an error", |a| a.iter().any(|e| e["event"]["kind"] == "error")).await;
    let err = all.iter().find(|e| e["event"]["kind"] == "error").unwrap();
    assert!(err["event"]["text"].as_str().unwrap().contains("crashing as asked"), "{err}");
    assert_eq!(ended(&all).last().unwrap(), "interrupted");
    send(&env, &id, "who", "next").await;
    let all =
        wait_for(&env, &id, "the answer after the restart", |a| said(a).iter().any(|s| s.contains("resumed=true")))
            .await;
    assert_eq!(session(&env, &id).await["externalSessionId"], thread, "the same thread");
    assert!(said(&all).last().unwrap().contains("resumed=true"));
    stop(&env, &id).await;
}

#[tokio::test]
async fn a_thread_that_never_took_a_turn_is_replaced_after_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({})).await;
    let before = loop {
        if let Some(t) = session(&env, &id).await["externalSessionId"].as_str() {
            break t.to_string();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    pdt_api::shutdown_conversations(&env.state).await;
    send(&env, &id, "hello", "next").await;
    let all = wait_turns(&env, &id, 1).await;
    assert!(all.iter().any(|e| e["event"]["kind"] == "notice"), "the user is told: {all:#?}");
    let after = session(&env, &id).await["externalSessionId"].as_str().unwrap().to_string();
    assert_ne!(before, after);
    assert_eq!(said(&all), ["echo: hello"]);
    stop(&env, &id).await;
}

#[tokio::test]
async fn the_live_terminal_opens_on_the_thread_after_the_first_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_codex(&env, json!({})).await;
    let url = format!("/api/sessions/{id}/terminal");
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"cols": 100, "rows": 30}))).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("no_turn_yet")));
    send(&env, &id, "hi", "next").await;
    wait_turns(&env, &id, 1).await;
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"cols": 100, "rows": 30}))).await;
    assert_eq!(s.as_u16(), 204, "{v}");
    assert!(env.state.ptys.is_running(&id));
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"cols": 100, "rows": 30}))).await;
    assert_eq!(s.as_u16(), 204, "opening it again is fine");
    // The fake TUI prints what it was started with.
    let mut seen = String::new();
    for _ in 0..100 {
        let bytes = env.state.ptys.attach(&id).map(|a| a.scrollback).unwrap_or_default();
        seen = String::from_utf8_lossy(&bytes).into_owned();
        if seen.contains("fake codex tui") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let thread = session(&env, &id).await["externalSessionId"].as_str().unwrap().to_string();
    assert!(
        seen.contains(&format!("thread={thread}"))
            && seen.contains("remote=ws://127.0.0.1:")
            && seen.contains("token=set"),
        "{seen}"
    );
    let (s, _) = call(&env.app, "DELETE", &url, None).await;
    assert_eq!(s.as_u16(), 204);
    for _ in 0..100 {
        if !env.state.ptys.is_running(&id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!env.state.ptys.is_running(&id));
    assert_eq!(session(&env, &id).await["status"], "running", "closing the TUI leaves the conversation");
    // Claude has no live terminal yet.
    let (_, c) = call(
        &env.app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24})),
    )
    .await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{}/terminal", c["id"].as_str().unwrap()),
        Some(json!({"cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (409, Some("terminal_unsupported")));
    stop(&env, c["id"].as_str().unwrap()).await;
    stop(&env, &id).await;
    pdt_api::shutdown_conversations(&env.state).await;
}
