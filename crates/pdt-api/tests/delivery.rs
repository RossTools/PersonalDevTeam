//! Honest delivery of messages to agents (PDT-129): a conversation's message is told only once the agent echoes it,
//! one it never takes is given up on and reported, a restart reports what was on its way, and a terminal stuck at
//! `unknown` gets its messages resolved untold (with a notification) once the outbox's deadline passes. `fake-agent`
//! stands in for Claude.

use crate::common;

use common::call;
use pdt_api::{AppState, router};
use pdt_store::{SessionMessage, Store};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
}

async fn setup(tmp: &Path, claude: &str) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    common::git(&repo, &["init", "-q", "-b", "main"]);
    common::git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.claude_program = claude.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.codex_home = Some(tmp.join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, 201, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into() }
}

async fn new_session(env: &Env, extra: Value) -> String {
    let mut body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    v["id"].as_str().unwrap().to_string()
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0"), None).await;
    v.as_array().unwrap().clone()
}

async fn wait_turns(env: &Env, id: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ended = events(env, id)
            .await
            .iter()
            .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
            .count();
        if ended >= n {
            return;
        }
        assert!(Instant::now() < deadline, "never saw {n} turns end: {:#?}", events(env, id).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// PDT's own message for a session, as `session.send` or a nudge queues it.
async fn queue(env: &Env, session: &str, text: &str) -> String {
    let m = SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(session.into()),
        ticket_id: None,
        dedupe: None,
        text: text.into(),
        timeline: Value::Null,
        run_id: None,
        step_id: None,
        status: "waiting".into(),
        told_session: None,
        created_at: 1,
        resolved_at: None,
        by_user: false,
        pending_id: None,
        reopens: false,
    };
    env.state.store.queue_session_message(&m).await.unwrap().id
}

async fn status(env: &Env, id: &str) -> String {
    env.state.store.get_session_message(id).await.unwrap().unwrap().status
}

/// Ticks the engine (which delivers) until the message's status is not `from` (up to 20 s); returns it.
async fn tick_past(env: &Env, id: &str, from: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let s = status(env, id).await;
        if s != from {
            return s;
        }
        assert!(Instant::now() < deadline, "the message stayed {from}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn undelivered(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/notifications?section=updates", None).await;
    v["items"].as_array().unwrap().iter().filter(|n| n["kind"] == "session.undelivered").cloned().collect()
}

async fn wait_undelivered(env: &Env) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(n) = undelivered(env).await.into_iter().next() {
            return n;
        }
        assert!(Instant::now() < deadline, "no session.undelivered notification");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_conversation_message_is_told_once_the_agent_echoes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), FAKE).await;
    let id = new_session(&env, json!({"initialPrompt": "first"})).await;
    wait_turns(&env, &id, 1).await;
    let m = queue(&env, &id, "PDT says hello").await;
    let s = tick_past(&env, &m, "waiting").await;
    // The echo may already be in; it never goes straight from waiting to told without it.
    if s == "sending" {
        assert_eq!(tick_past(&env, &m, "sending").await, "told");
    } else {
        assert_eq!(s, "told");
    }
    let all = events(&env, &id).await;
    assert!(
        all.iter().any(|e| e["event"]["kind"] == "user_message"
            && e["event"]["text"] == "PDT says hello"
            && e["event"]["source"] == "notice"),
        "the agent took it: {all:#?}"
    );
    assert!(env.state.store.sending_session_messages(None).await.unwrap().is_empty(), "nothing left on its way");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_message_the_agent_never_takes_is_reported_not_told() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = setup(tmp.path(), FAKE).await;
    let id = new_session(&env, json!({"initialPrompt": "deaf"})).await;
    wait_turns(&env, &id, 1).await;
    let m = queue(&env, &id, "are you listening").await;
    assert_eq!(tick_past(&env, &m, "waiting").await, "sending", "handed over, not told");
    tokio::time::sleep(Duration::from_millis(300)).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(status(&env, &m).await, "sending", "no echo, so not told");
    assert_eq!(pdt_api::expire_pending_messages(&env.state).await, 0, "not given up on yet");

    env.state.conversations.pending_timeout = Duration::ZERO;
    assert_eq!(pdt_api::expire_pending_messages(&env.state).await, 1);
    let n = wait_undelivered(&env).await;
    assert_eq!(status(&env, &m).await, "untold");
    assert_eq!(n["sessionId"], id.as_str());
    let body = n["body"].as_str().unwrap();
    assert!(body.contains("did not take it") && body.contains("are you listening"), "{body}");
    // No longer held busy by it.
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s["activity"], "idle", "{s}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_restart_reports_the_users_message_still_on_its_way() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), FAKE).await;
    let id = new_session(&env, json!({"initialPrompt": "deaf"})).await;
    wait_turns(&env, &id, 1).await;
    // The user's own message: accepted (202), never echoed.
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": "after the restart?"})))
            .await;
    assert_eq!(s, 202, "{v}");
    let sending = env.state.store.sending_session_messages(Some(v["id"].as_str().unwrap())).await.unwrap();
    assert_eq!(sending.len(), 1, "it has a row on its way");
    assert!(sending[0].by_user);

    // pdtd stops; the next one reports what it never delivered.
    env.state.store.mark_running_interrupted(1).await.unwrap();
    pdt_api::shutdown_conversations(&env.state).await;
    let mut fresh = AppState::new(env.state.store.clone(), tmp.path().join("data"));
    fresh.platform = env.state.platform.clone();
    pdt_api::recover_conversations(&fresh).await;
    assert_eq!(status(&env, &sending[0].id).await, "untold");
    let n = wait_undelivered(&env).await;
    let body = n["body"].as_str().unwrap();
    assert!(body.contains("PDT restarted") && body.contains("after the restart?"), "{body}");
}

/// A stand-in for Claude's TUI: answers every line it reads (and sends no hooks: the test posts them).
fn fake_tui(dir: &Path) -> std::path::PathBuf {
    let (file, body) = if cfg!(windows) {
        ("claude.cmd", "@echo off\r\n:loop\r\nset \"x=\"\r\nset /p x=\r\necho got %x%\r\ngoto loop\r\n")
    } else {
        ("claude", "#!/bin/sh\nwhile read x; do echo \"got $x\"; done\n")
    };
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_stuck_at_unknown_has_its_message_reported_by_the_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let mut env = setup(tmp.path(), &fake_tui(&bin).to_string_lossy()).await;
    env.state.activity_timing = pdt_core::activity::Timing { quiet_after_ms: 200, hook_grace_ms: 60_000 };
    env.state.activity_tick = Duration::from_millis(50);
    env.state.outbox_deadline = Duration::from_millis(400);
    tokio::spawn(pdt_api::run_activity_ticker(env.state.clone()));
    let id = new_session(&env, json!({"mode": "terminal"})).await;
    // Working, then quiet with no turn-end hook: unknown.
    let r = json!({"source": "claude", "event": "UserPromptSubmit"});
    assert_eq!(call(&env.app, "POST", &format!("/api/sessions/{id}/activity"), Some(r)).await.0, 204);
    let deadline = Instant::now() + Duration::from_secs(20);
    while call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1["activity"] != "unknown" {
        assert!(Instant::now() < deadline, "never unknown");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let m = queue(&env, &id, "CI failed on your PR").await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(status(&env, &m).await, "waiting", "not typed into a terminal PDT can't read");
    assert!(undelivered(&env).await.is_empty());
    assert_eq!(tick_past(&env, &m, "waiting").await, "untold", "given up on by the deadline");
    let n = wait_undelivered(&env).await;
    let body = n["body"].as_str().unwrap();
    assert!(body.contains("could not tell whether it was idle") && body.contains("CI failed on your PR"), "{body}");
    assert_eq!(n["target"], json!({"kind": "session", "id": id, "projectId": env.pid}));
    env.state.ptys.shutdown().await;
}

/// A `session.send` step run again (a restart, a retry) while its message is on its way to a conversation waits for
/// the echo, rather than taking `untold` (PDT-129).
#[tokio::test]
async fn a_rerun_send_step_waits_for_its_message_on_its_way() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), FAKE).await;
    let id = new_session(&env, json!({"initialPrompt": "deaf"})).await;
    wait_turns(&env, &id, 1).await;
    let doc = json!({"name": "Tell", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        {"id": "tell", "kind": "session.send", "version": 1, "config": {"session": id, "text": "hello from a step"}},
    ], "edges": [{"from": "go", "to": "tell"}]});
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.pid, "enabled": true})))
            .await;
    assert_eq!(s, 201, "{w}");
    let (s, r) =
        call(&env.app, "POST", &format!("/api/workflows/{}/run", w["id"].as_str().unwrap()), Some(json!({}))).await;
    assert_eq!(s, 200, "{r}");
    let run = r["runId"].as_str().unwrap().to_string();
    let step = || async {
        let steps = env.state.store.list_steps(&run).await.unwrap();
        steps.into_iter().find(|s| s.node_id == "tell").unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let message = loop {
        pdt_api::run_workflows(&env.state).await;
        let sending = env.state.store.sending_session_messages(None).await.unwrap();
        if let Some(m) = sending.into_iter().find(|m| m.step_id.is_some()) {
            break m;
        }
        assert!(Instant::now() < deadline, "the step's message never went out");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    // Run the step again, as a restart would.
    let mut s = step().await;
    assert_eq!(s.status, pdt_core::workflow::api::StepStatus::Waiting);
    s.status = pdt_core::workflow::api::StepStatus::Pending;
    env.state.store.put_step_and_wake(&s).await.unwrap();
    pdt_api::run_workflows(&env.state).await;
    let s = step().await;
    assert_eq!(s.status, pdt_core::workflow::api::StepStatus::Waiting, "{:?}", s.port);
    assert_eq!(status(&env, &message.id).await, "sending");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
