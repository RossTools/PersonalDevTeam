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

/// A stand-in for `codex` that records its arguments, shows the approval prompt, then: waits for an answer, waits
/// for another line, and shows the prompt again.
fn fake_codex(dir: &Path) -> std::path::PathBuf {
    let (file, body) = if cfg!(windows) {
        (
            "codex.cmd",
            "@echo off\r\necho(%*> \"%~dp0codex-args.txt\"\r\necho Would you like to run the following command?\r\n\
             echo   1. Yes, proceed (y)\r\nset /p x=\r\nset /p x=\r\necho Would you like to run the following command?\r\n\
             echo   1. Yes, proceed (y)\r\nset /p x=\r\n",
        )
    } else {
        (
            "codex",
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/codex-args.txt\"\n\
             printf 'Would you like to run the following command?\\n  1. Yes, proceed (y)\\n'\nread x\nread x\n\
             printf 'Would you like to run the following command?\\n  1. Yes, proceed (y)\\n'\nread x\n",
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
