//! Conversation sessions end to end, with `fake-agent` standing in for `claude -p` (structured agents spec §12).

mod common;

use common::{call, git, park_once_idle};
use futures_util::StreamExt;
use pdt_api::{AppState, router, with_host_guard};
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
    state.platform.login_path = None;
    let claude_dir = tmp.join("claude-config");
    state.platform.claude_config_dir = Some(claude_dir.clone());
    state.platform.codex_home = Some(tmp.join("codex-home"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, 201, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), claude_dir }
}

async fn new_conversation(env: &Env, extra: Value) -> Value {
    let mut body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    v
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0"), None).await;
    assert_eq!(s, 200, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

/// Waits until some event matches.
async fn wait_for(env: &Env, id: &str, what: &str, f: impl Fn(&Value) -> bool) -> Vec<Value> {
    for _ in 0..300 {
        let all = events(env, id).await;
        if all.iter().any(|e| f(&e["event"])) {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {what}: {:#?}", events(env, id).await);
}

fn turns_ended(all: &[Value]) -> usize {
    all.iter().filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended").count()
}

async fn wait_turns(env: &Env, id: &str, n: usize) -> Vec<Value> {
    for _ in 0..300 {
        let all = events(env, id).await;
        if turns_ended(&all) >= n {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {n} turns end: {:#?}", events(env, id).await);
}

fn agent_text(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

async fn send(env: &Env, id: &str, text: &str) -> Value {
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, 202, "{v}");
    v
}

async fn session(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

async fn wait_activity(env: &Env, id: &str, want: &str) -> Value {
    for _ in 0..300 {
        let s = session(env, id).await;
        if s["activity"] == want {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("activity never became {want}: {}", session(env, id).await);
}

fn open_request(all: &[Value]) -> Option<String> {
    all.iter()
        .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
        .map(|e| e["event"]["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn a_new_claude_tab_is_a_conversation_that_streams_replies() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let s = new_conversation(&env, json!({"initialPrompt": "hello there"})).await;
    assert_eq!((s["mode"].as_str(), s["permissionMode"].as_str()), (Some("conversation"), Some("ask")));
    let id = s["id"].as_str().unwrap();
    let all = wait_turns(&env, id, 1).await;
    let kinds: Vec<&str> = all.iter().map(|e| e["event"]["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["turn", "user_message", "agent_message", "usage", "turn"], "{all:#?}");
    assert_eq!(all[1]["event"]["text"], "hello there");
    assert_eq!(all[2]["event"]["phase"], "final");
    assert_eq!(agent_text(&all), ["echo: hello there"]);
    wait_activity(&env, id, "idle").await;
    let s = session(&env, id).await;
    assert!(s["unseen"].as_bool().unwrap(), "a finished turn wants attention");

    send(&env, id, "who").await;
    let all = wait_turns(&env, id, 2).await;
    assert!(agent_text(&all)[1].contains("resumed=false"), "the process stays up between turns");
    let (_, caps) = call(&env.app, "GET", "/api/agents/claude/capabilities", None).await;
    assert_eq!(caps["models"][1]["value"], "opus", "{caps}");

    // Bad input.
    let (st, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": "  "}))).await;
    assert_eq!((st.as_u16(), v["code"].as_str()), (400, Some("invalid_message")));
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn approvals_are_answered_once_and_always_allow_stays_in_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let s = new_conversation(&env, json!({})).await;
    let id = s["id"].as_str().unwrap();
    send(&env, id, "run: touch a").await;
    let all = wait_for(&env, id, "an open request", |e| e["kind"] == "request" && e["state"] == "open").await;
    let req = open_request(&all).unwrap();
    let req_ev = all.iter().find(|e| e["event"]["kind"] == "request").unwrap();
    assert_eq!(req_ev["event"]["canAlways"], true);
    assert_eq!(req_ev["event"]["title"], "touch a");
    wait_activity(&env, id, "needs_you").await;

    let url = format!("/api/sessions/{id}/requests/{req}");
    let (st, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "question", "answers": {}}}))).await;
    assert_eq!((st.as_u16(), v["code"].as_str()), (400, Some("wrong_answer")));
    let (st, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve", "always": true}}))).await;
    assert_eq!(st.as_u16(), 204);
    let (st, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!((st.as_u16(), v["code"].as_str()), (409, Some("request_closed")), "the first answer wins");
    let all = wait_turns(&env, id, 1).await;
    let said = agent_text(&all).join(" ");
    assert!(said.contains("Ran it."), "{said}");
    assert!(said.contains("\"destination\":\"session\"") && !said.contains("localSettings"), "{said}");
    let tool = all.iter().find(|e| e["event"]["kind"] == "tool_call").unwrap();
    assert_eq!(
        (tool["event"]["status"].as_str(), tool["event"]["output"].as_str()),
        (Some("done"), Some("ran touch a"))
    );
    let req_row = all.iter().find(|e| e["event"]["kind"] == "request").unwrap();
    assert_eq!(req_row["event"]["state"], "answered");
    assert_eq!(req_row["event"]["answer"]["type"], "approve");

    // Deny with a reason: the tool call is declined and the agent hears why.
    send(&env, id, "run: rm -rf /").await;
    let all = wait_for(&env, id, "a second request", |e| e["kind"] == "request" && e["state"] == "open").await;
    let req = open_request(&all).unwrap();
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (st, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny", "message": "too risky"}}))).await;
    assert_eq!(st.as_u16(), 204);
    let all = wait_turns(&env, id, 2).await;
    assert!(agent_text(&all).last().unwrap().contains("too risky"));
    let declined = all.iter().filter(|e| e["event"]["kind"] == "tool_call").nth(1).unwrap();
    assert_eq!(declined["event"]["status"], "declined");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn questions_take_answers_and_interrupt_expires_open_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_conversation(&env, json!({})).await["id"].as_str().unwrap().to_string();
    send(&env, &id, "ask: Which one?").await;
    let all = wait_for(&env, &id, "a question", |e| e["kind"] == "request" && e["state"] == "open").await;
    let q = all.iter().find(|e| e["event"]["kind"] == "request").unwrap();
    assert_eq!(q["event"]["requestKind"], "question");
    let req = q["event"]["id"].as_str().unwrap();
    let answer = json!({"answer": {"type": "question", "answers": {"Which one?": ["B"]}}});
    let (st, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/requests/{req}"), Some(answer)).await;
    assert_eq!(st.as_u16(), 204);
    let all = wait_turns(&env, &id, 1).await;
    assert!(agent_text(&all).last().unwrap().contains("\"Which one?\":\"B\""), "{:?}", agent_text(&all));

    // An open request when the user interrupts: it expires and the turn ends interrupted.
    send(&env, &id, "run: sleep 100").await;
    wait_for(&env, &id, "a request", |e| e["kind"] == "request" && e["state"] == "open").await;
    let (st, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert_eq!(st.as_u16(), 204);
    let all = wait_turns(&env, &id, 2).await;
    let last_turn = all.iter().rev().find(|e| e["event"]["kind"] == "turn").unwrap();
    assert_eq!(last_turn["event"]["status"], "interrupted", "{all:#?}");
    assert!(open_request(&all).is_none());
    assert!(all.iter().any(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "expired"));
    wait_activity(&env, &id, "idle").await;
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn messages_queue_while_working_and_send_now_interrupts() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_conversation(&env, json!({})).await["id"].as_str().unwrap().to_string();
    send(&env, &id, "slow").await;
    wait_for(&env, &id, "the slow turn", |e| e["kind"] == "user_message").await;
    wait_activity(&env, &id, "working").await;
    // Queued: taken as the next turn once the slow one ends (interrupted below).
    let pending = send(&env, &id, "after").await;
    assert_eq!(pending["delivery"], "next");
    let (st, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert_eq!(st.as_u16(), 204);
    let all = wait_turns(&env, &id, 2).await;
    let users: Vec<&str> = all
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| e["event"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(users, ["slow", "after"]);
    assert!(agent_text(&all).contains(&"echo: after".to_string()));

    // Send now: the running turn is cut short and the message runs at once.
    send(&env, &id, "slow").await;
    wait_turns(&env, &id, 2).await;
    wait_for(&env, &id, "the second slow turn", |_| true).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (st, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{id}/messages"),
        Some(json!({"text": "urgent", "delivery": "now"})),
    )
    .await;
    assert_eq!(st.as_u16(), 202, "{v}");
    let all = wait_turns(&env, &id, 4).await;
    assert!(agent_text(&all).contains(&"echo: urgent".to_string()), "{all:#?}");
    let ended: Vec<&str> = all
        .iter()
        .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        .map(|e| e["event"]["status"].as_str().unwrap())
        .collect();
    assert_eq!(ended, ["interrupted", "completed", "interrupted", "completed"]);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_crash_is_reported_and_the_next_message_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_conversation(&env, json!({"initialPrompt": "first"})).await["id"].as_str().unwrap().to_string();
    wait_turns(&env, &id, 1).await;
    // Claude saved the conversation after its first turn.
    let proj = env.claude_dir.join("projects").join("-repo");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(format!("{id}.jsonl")), "{}\n").unwrap();
    send(&env, &id, "exit").await;
    let all = wait_for(&env, &id, "an error", |e| e["kind"] == "error").await;
    let err = all.iter().find(|e| e["event"]["kind"] == "error").unwrap();
    let text = err["event"]["text"].as_str().unwrap();
    assert!(text.contains("exit code 3") && text.contains("crashing as asked"), "{text}");
    let last_turn = all.iter().rev().find(|e| e["event"]["kind"] == "turn").unwrap();
    assert_eq!(
        (last_turn["event"]["phase"].as_str(), last_turn["event"]["status"].as_str()),
        (Some("ended"), Some("interrupted"))
    );
    wait_activity(&env, &id, "needs_you").await;
    send(&env, &id, "who").await;
    let all = wait_turns(&env, &id, 3).await;
    assert!(agent_text(&all).last().unwrap().contains("resumed=true"), "{:?}", agent_text(&all));
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn idle_conversations_park_and_settings_apply_on_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = setup(tmp.path()).await;
    env.state.conversations.idle_park = Duration::ZERO;
    env.app = router(env.state.clone(), None);
    let s = new_conversation(&env, json!({"permissionMode": "plan", "model": "opus", "initialPrompt": "who"})).await;
    let id = s["id"].as_str().unwrap().to_string();
    let all = wait_turns(&env, &id, 1).await;
    assert!(agent_text(&all)[0].contains("model=opus mode=plan"), "{:?}", agent_text(&all));
    let (st, v) = call(
        &env.app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        Some(json!({"permissionMode": "accept_edits", "model": null})),
    )
    .await;
    assert_eq!(st.as_u16(), 200, "{v}");
    assert_eq!((v["permissionMode"].as_str(), &v["model"]), (Some("accept_edits"), &Value::Null));
    park_once_idle(&env.state).await;
    let s = session(&env, &id).await;
    assert_eq!(s["status"], "running", "a parked conversation is still open");
    let proj = env.claude_dir.join("projects").join("x");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(format!("{id}.jsonl")), "{}\n").unwrap();
    send(&env, &id, "who").await;
    let all = wait_turns(&env, &id, 2).await;
    let said = agent_text(&all).last().unwrap().clone();
    assert!(
        said.contains("resumed=true") && said.contains("mode=acceptEdits") && said.contains("model=default"),
        "{said}"
    );

    // Stop, then send: it reopens.
    let (st, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(st.as_u16(), 204);
    assert_eq!(session(&env, &id).await["status"], "finished");
    let (st, v) = call(&env.app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(st.as_u16(), 204, "{v}");
    let (st, _) = call(&env.app, "GET", &format!("/api/sessions/{id}/events"), None).await;
    assert_eq!(st.as_u16(), 404);
}

#[tokio::test]
async fn a_shutdown_ends_the_turn_and_the_next_pdtd_reopens_on_send() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_conversation(&env, json!({})).await["id"].as_str().unwrap().to_string();
    send(&env, &id, "run: make").await;
    wait_for(&env, &id, "a request", |e| e["kind"] == "request" && e["state"] == "open").await;
    // An orderly pdtd shutdown: the row says interrupted, the process is closed (its open request expires and its
    // turn ends), and the next pdtd recovers what is left.
    env.state.store.mark_running_interrupted(1).await.unwrap();
    pdt_api::shutdown_conversations(&env.state).await;
    let all = events(&env, &id).await;
    assert!(open_request(&all).is_none());
    let mut fresh = AppState::new(env.state.store.clone(), tmp.path().join("data"));
    fresh.platform = env.state.platform.clone();
    pdt_api::recover_conversations(&fresh).await;
    let app = router(fresh.clone(), None);
    let (_, v) = call(&app, "GET", &format!("/api/sessions/{id}/events"), None).await;
    let all = v.as_array().unwrap();
    assert!(all.iter().all(|e| e["event"]["kind"] != "request" || e["event"]["state"] != "open"));
    assert_eq!(turns_ended(all), 1);
    // Sending to the interrupted conversation reopens it.
    let (st, v) = call(&app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": "back"}))).await;
    assert_eq!(st.as_u16(), 202, "{v}");
    for _ in 0..200 {
        let (_, s) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
        if s["status"] == "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn recovery_closes_a_turn_left_running_by_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_conversation(&env, json!({})).await["id"].as_str().unwrap().to_string();
    pdt_api::shutdown_conversations(&env.state).await;
    // Rows a crashed pdtd would leave: a started turn and an open request.
    let st = &env.state.store;
    use pdt_core::conversation::*;
    let turn = AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None };
    st.upsert_session_event(
        &id,
        pdt_store::NewEvent { ts: 1, item_id: Some("turn:u9"), turn_id: Some("u9"), event: &turn, raw: None },
    )
    .await
    .unwrap();
    let req = AgentEvent::Request {
        id: "r9".into(),
        request_kind: RequestKind::Approval,
        tool: "Bash".into(),
        title: "x".into(),
        description: None,
        input: json!({}),
        tool_call_id: None,
        can_always: false,
        state: RequestState::Open,
        answer: None,
    };
    st.upsert_session_event(
        &id,
        pdt_store::NewEvent { ts: 1, item_id: Some("r:r9"), turn_id: Some("u9"), event: &req, raw: None },
    )
    .await
    .unwrap();
    st.mark_running_interrupted(2).await.unwrap();
    pdt_api::recover_conversations(&env.state).await;
    let all = events(&env, &id).await;
    let req = all.iter().find(|e| e["event"]["kind"] == "request").unwrap();
    assert_eq!(req["event"]["state"], "expired");
    let end = all.iter().find(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended").unwrap();
    assert_eq!(
        (end["event"]["status"].as_str(), end["event"]["reason"].as_str()),
        (Some("interrupted"), Some("PDT restarted"))
    );
    assert!(all.iter().any(|e| e["event"]["kind"] == "notice"));
    // Interrupted conversations are never auto-resumed as terminals.
    assert!(pdt_api::plan_resume_interrupted(&env.state).await.is_empty());
}

#[tokio::test]
async fn api_keys_are_scrubbed_from_the_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    // SAFETY: tests in this binary that read the environment don't depend on this variable.
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test") };
    let id = new_conversation(&env, json!({"initialPrompt": "env: ANTHROPIC_API_KEY"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let all = wait_turns(&env, &id, 1).await;
    assert_eq!(agent_text(&all), ["ANTHROPIC_API_KEY is unset"]);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn the_events_socket_replays_then_streams_live() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = with_host_guard(router(env.state.clone(), None), vec![format!("127.0.0.1:{port}")]);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let id = new_conversation(&env, json!({"initialPrompt": "one"})).await["id"].as_str().unwrap().to_string();
    wait_turns(&env, &id, 1).await;
    let url = format!("ws://127.0.0.1:{port}/api/sessions/{id}/events?after=0");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut frames = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
        let v: Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        let ready = v["type"] == "ready";
        frames.push(v);
        if ready {
            break;
        }
    }
    let types: Vec<&str> = frames.iter().map(|f| f["type"].as_str().unwrap()).collect();
    assert_eq!(types.iter().filter(|t| **t == "event").count(), 5, "{frames:#?}");
    assert_eq!(&types[types.len() - 2..], ["state", "ready"]);
    assert_eq!(frames[frames.len() - 2]["state"]["live"], true);
    send(&env, &id, "two").await;
    let mut saw_delta = false;
    let mut saw_final = false;
    while !saw_final {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
        let v: Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
        if v["type"] == "delta" {
            saw_delta = true;
        }
        if v["type"] == "event"
            && v["event"]["event"]["kind"] == "agent_message"
            && v["event"]["event"]["phase"] == "final"
        {
            assert_eq!(v["event"]["event"]["markdown"], "echo: two");
            saw_final = true;
        }
    }
    assert!(saw_delta, "text streams before the message is stored");
    // A terminal session has no events.
    let (_, shell) = call(
        &env.app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": env.pid, "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    let (st, v) = call(&env.app, "GET", &format!("/api/sessions/{}/events", shell["id"].as_str().unwrap()), None).await;
    assert_eq!((st.as_u16(), v["code"].as_str()), (409, Some("not_conversation")));
    call(&env.app, "POST", &format!("/api/sessions/{}/kill", shell["id"].as_str().unwrap()), None).await;
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
