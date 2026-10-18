mod common;

use axum::http::StatusCode;
use common::call;
use futures_util::SinkExt;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

async fn setup(tmp: &Path) -> (axum::Router, pdt_api::AppState) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(
        std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success()
    );
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = pdt_api::AppState::new(store.clone(), tmp.join("data"));
    let app = pdt_api::router(state.clone(), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    for (id, kind, agent) in [("c1", SessionKind::AgentTui, Some(AgentKind::Claude)), ("sh", SessionKind::Shell, None)]
    {
        store
            .insert_session(&Session {
                id: id.into(),
                project_id: p["id"].as_str().unwrap().into(),
                worktree_id: None,
                kind,
                agent,
                bypass_permissions: false,
                title: "t".into(),
                cwd: repo.to_string_lossy().into(),
                status: SessionStatus::Running,
                exit_code: None,
                created_at: 1,
                ended_at: None,
                external_session_id: None,
                fresh_started_at: None,
                activity: agent.map(|_| pdt_core::activity::Activity::Idle),
                attention_at: None,
                last_viewed_at: None,
                unseen: false,
            })
            .await
            .unwrap();
    }
    (app, state)
}

async fn report(app: &axum::Router, id: &str, body: Value) -> StatusCode {
    call(app, "POST", &format!("/api/sessions/{id}/activity"), Some(body)).await.0
}

async fn session(app: &axum::Router, id: &str) -> Value {
    call(app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

#[tokio::test]
async fn hook_reports_drive_activity_and_unseen() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    assert_eq!(
        report(&app, "c1", json!({"source": "claude", "event": "UserPromptSubmit"})).await,
        StatusCode::NO_CONTENT
    );
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("working"), Some(false)));
    report(&app, "c1", json!({"source": "claude", "event": "Stop"})).await;
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("idle"), Some(true)));
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(call(&app, "POST", "/api/sessions/c1/view", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(session(&app, "c1").await["unseen"], false, "seen on every device");
    tokio::time::sleep(Duration::from_millis(5)).await;
    report(&app, "c1", json!({"source": "claude", "event": "Notification", "notificationType": "permission_prompt"}))
        .await;
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("needs_you"), Some(true)));
    report(&app, "c1", json!({"source": "claude", "event": "Notification", "notificationType": "auth_success"})).await;
    assert_eq!(session(&app, "c1").await["activity"], "needs_you", "ignored notification types change nothing");
    assert_eq!(report(&app, "c1", json!({"source": "claude", "event": "FutureEvent"})).await, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn reports_need_an_agent_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (s, v) = call(&app, "POST", "/api/sessions/sh/activity", Some(json!({"source": "codex", "event": "x"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_an_agent")));
    assert_eq!(report(&app, "nope", json!({"source": "codex", "event": "x"})).await, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "POST", "/api/sessions/nope/view", None).await.0, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/api/sessions/c1/activity", Some(json!({"source": "nobody"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_ended_session_keeps_no_activity() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    state.store.end_session("c1", SessionStatus::Finished, Some(0), 5).await.unwrap();
    assert_eq!(session(&app, "c1").await["activity"], Value::Null, "ending a session clears its activity");
    assert_eq!(report(&app, "c1", json!({"source": "claude", "event": "Stop"})).await, StatusCode::NO_CONTENT);
    assert_eq!(session(&app, "c1").await["activity"], Value::Null, "a late hook does not bring it back");
}

/// A stand-in for `codex` that records its arguments, shows the approval prompt (two of its lines), then: waits for
/// an answer, prints one prompt line alone (ordinary output), waits for another line, and asks for approval again,
/// this time only through its window title (as Codex does: `[ ! ] Action Required`).
fn fake_codex(dir: &Path) -> std::path::PathBuf {
    let (file, body) = if cfg!(windows) {
        (
            "codex.cmd",
            "@echo off\r\necho(%*> \"%~dp0codex-args.txt\"\r\necho Would you like to run the following command?\r\n\
             echo   1. Yes, proceed (y)\r\nset /p x=\r\necho The docs say: Yes, proceed\r\nset /p x=\r\n\
             title [ ! ] Action Required - t\r\necho waiting\r\nset /p x=\r\n",
        )
    } else {
        (
            "codex",
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/codex-args.txt\"\n\
             printf 'Would you like to run the following command?\\n  1. Yes, proceed (y)\\n'\nread x\n\
             printf 'The docs say: Yes, proceed\\n'\nread x\n\
             printf '\\033]0;[ ! ] Action Required - t\\007waiting\\n'\nread x\n",
        )
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

async fn wait_for_activity(app: &axum::Router, id: &str, want: Value) {
    for _ in 0..200 {
        if session(app, id).await["activity"] == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("activity never became {want}: {}", session(app, id).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn codex_activity_follows_its_approval_screen_and_input() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state) = setup(tmp.path()).await;
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    state.platform.codex_program = fake_codex(&bin).to_string_lossy().into_owned();
    let codex_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::write(codex_home.join("config.toml"), "notify = [\"my-notifier\", \"turn-ended\"]\n").unwrap();
    state.platform.codex_home = Some(codex_home);
    state.hook_program = "/x/pdtd".into();
    let app = pdt_api::router(state.clone(), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });

    let (_, projects) = call(&app, "GET", "/api/projects", None).await;
    let body = json!({"projectId": projects[0]["id"], "kind": "agent_tui", "agent": "codex", "cols": 80, "rows": 24});
    let (s, created) = call(&app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();

    wait_for_activity(&app, &id, json!("needs_you")).await;
    assert_eq!(session(&app, &id).await["unseen"], true);
    let args = std::fs::read_to_string(bin.join("codex-args.txt")).unwrap();
    for part in ["notify=", "/x/pdtd", "codex-notify", "--chain", "my-notifier", "turn-ended"] {
        assert!(args.contains(part), "{part} missing from {args}");
    }

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty")).await.unwrap();
    ws.send(Message::Binary(b"y\r".to_vec().into())).await.unwrap();
    wait_for_activity(&app, &id, json!("working")).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(session(&app, &id).await["activity"], "working", "one prompt line in ordinary output is not the prompt");
    ws.send(Message::Binary(b"\r".to_vec().into())).await.unwrap();
    wait_for_activity(&app, &id, json!("needs_you")).await;

    let (s, _) = call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    for _ in 0..100 {
        if session(&app, &id).await["status"] != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    wait_for_activity(&app, &id, Value::Null).await;
    state.ptys.shutdown().await;
}

/// A stand-in for `claude` that answers every line it reads with `got <line>` (and sends no hooks: tests post them).
fn fake_claude(dir: &Path) -> std::path::PathBuf {
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

/// Serves a router whose Claude is the fake, with short activity thresholds and the ticker running.
/// Returns (app, state, port, project id).
/// Serves a fake Claude whose output goes quiet (working → unknown) after `quiet_after_ms` without output.
async fn serve_claude(tmp: &Path, quiet_after_ms: i64) -> (axum::Router, pdt_api::AppState, u16, String) {
    let (_, mut state) = setup(tmp).await;
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    state.platform.claude_program = fake_claude(&bin).to_string_lossy().into_owned();
    state.activity_timing = pdt_core::activity::Timing { quiet_after_ms, hook_grace_ms: 400 };
    state.activity_tick = Duration::from_millis(50);
    tokio::spawn(pdt_api::run_activity_ticker(state.clone()));
    let app = pdt_api::router(state.clone(), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let (_, projects) = call(&app, "GET", "/api/projects", None).await;
    (app, state, port, projects[0]["id"].as_str().unwrap().to_string())
}

async fn create_claude(app: &axum::Router, pid: &str) -> String {
    let body = json!({"projectId": pid, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24});
    let (s, created) = call(app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    created["id"].as_str().unwrap().to_string()
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn terminal(port: u16, id: &str) -> Ws {
    tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty")).await.unwrap().0
}

async fn type_in(ws: &mut Ws, text: &str) {
    ws.send(Message::Binary(text.as_bytes().to_vec().into())).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_hooks_that_never_fire_make_the_session_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, port, pid) = serve_claude(tmp.path(), 2_000).await;
    let id = create_claude(&app, &pid).await;
    assert_eq!(session(&app, &id).await["activity"], "idle");
    let mut ws = terminal(port, &id).await;
    type_in(&mut ws, "hello\r").await;
    // The fake answers (output) but no hook ever comes: after the grace period the session is unknown.
    wait_for_activity(&app, &id, json!("unknown")).await;
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_with_working_hooks_stays_known_until_it_goes_quiet() {
    let tmp = tempfile::tempdir().unwrap();
    // Output every 500 ms must keep it working: a quiet threshold well above that leaves room for a slow CI machine
    // (final review #5), and `wait_for_activity` (20 s) still sees it go quiet.
    let (app, state, port, pid) = serve_claude(tmp.path(), 6_000).await;
    let id = create_claude(&app, &pid).await;
    let mut ws = terminal(port, &id).await;
    // SessionStart changes nothing, but shows the hooks work.
    assert_eq!(report(&app, &id, json!({"source": "claude", "event": "SessionStart"})).await, StatusCode::NO_CONTENT);
    assert_eq!(session(&app, &id).await["activity"], "idle");
    // An empty Enter (or a slash command) prints but sends no hook: not a sign of broken hooks.
    type_in(&mut ws, "\r").await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(session(&app, &id).await["activity"], "idle", "an Enter without a hook keeps it idle");

    // A permission prompt, answered with a keypress: working again, and it stays so while output flows.
    report(&app, &id, json!({"source": "claude", "event": "PermissionRequest"})).await;
    assert_eq!(session(&app, &id).await["activity"], "needs_you");
    type_in(&mut ws, "1").await;
    wait_for_activity(&app, &id, json!("working")).await;
    for _ in 0..3 {
        type_in(&mut ws, "\r").await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(session(&app, &id).await["activity"], "working", "a long command that keeps printing");
    }
    // No output for longer than the quiet threshold while working: unknown.
    wait_for_activity(&app, &id, json!("unknown")).await;
    report(&app, &id, json!({"source": "claude", "event": "Stop"})).await;
    assert_eq!(session(&app, &id).await["activity"], "idle");
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_interrupt_returns_to_idle_without_a_stop_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, port, pid) = serve_claude(tmp.path(), 60_000).await;
    let id = create_claude(&app, &pid).await;
    let mut ws = terminal(port, &id).await;
    report(&app, &id, json!({"source": "claude", "event": "UserPromptSubmit"})).await;
    // The fake echoes each line as output. Quoting the interruption message during ordinary work is not a stop.
    type_in(&mut ws, "Interrupted · What should Claude do instead?\r").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(session(&app, &id).await["activity"], "working");
    // Use a separate terminal for cancellation so a delayed echo of the quoted example cannot serve as its ack.
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    let id = create_claude(&app, &pid).await;
    let mut ws = terminal(port, &id).await;
    report(&app, &id, json!({"source": "claude", "event": "UserPromptSubmit"})).await;
    // Esc requests cancellation; a dialog can consume it, so wait for Claude's interruption acknowledgement.
    type_in(&mut ws, "\x1b").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(session(&app, &id).await["activity"], "working");
    type_in(&mut ws, "Interrupted · What should Claude do instead?\r").await;
    wait_for_activity(&app, &id, json!("idle")).await;
    // A tool finishing while cancellation settles must not leave the session stuck working again.
    report(&app, &id, json!({"source": "claude", "event": "PostToolUse"})).await;
    report(&app, &id, json!({"source": "claude", "event": "PermissionRequest"})).await;
    assert_eq!(session(&app, &id).await["activity"], "idle");
    report(&app, &id, json!({"source": "claude", "event": "PreToolUse"})).await;
    assert_eq!(session(&app, &id).await["activity"], "working", "new tool use proves work resumed");
    report(&app, &id, json!({"source": "claude", "event": "Stop"})).await;
    report(&app, &id, json!({"source": "claude", "event": "UserPromptSubmit"})).await;
    assert_eq!(session(&app, &id).await["activity"], "working", "a new prompt starts normally");
    let id = create_claude(&app, &pid).await;
    let mut ws = terminal(port, &id).await;
    report(&app, &id, json!({"source": "claude", "event": "PermissionRequest"})).await;
    type_in(&mut ws, "\x1b").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(session(&app, &id).await["activity"], "needs_you", "Esc alone can decline a permission");
    report(&app, &id, json!({"source": "claude", "event": "PostToolUse"})).await;
    assert_eq!(session(&app, &id).await["activity"], "working", "a declined permission can continue the turn");
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_session_deletes_its_hook_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _, pid) = serve_claude(tmp.path(), 2_000).await;
    let id = create_claude(&app, &pid).await;
    let dir = tmp.path().join("data").join("sessions").join(&id);
    assert!(dir.join("claude-settings.json").is_file());
    assert_eq!(call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await.0, StatusCode::NO_CONTENT);
    let mut removed = StatusCode::CONFLICT;
    for _ in 0..100 {
        removed = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await.0;
        if removed != StatusCode::CONFLICT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(removed, StatusCode::NO_CONTENT);
    assert!(!dir.exists(), "the session's data folder is removed with it");
    state.ptys.shutdown().await;
}
