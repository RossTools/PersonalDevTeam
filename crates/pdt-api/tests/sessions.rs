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
    assert_eq!(sess["title"], "Shell · P");
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
async fn restart_right_after_kill_is_not_overwritten_by_old_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    let id = sess["id"].as_str().unwrap().to_string();
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    let uri = format!("/api/sessions/{id}/restart");
    let mut restarted = false;
    for _ in 0..100 {
        let (s, v) = call(&app, "POST", &uri, Some(json!({"cols": 80, "rows": 24}))).await;
        if s == StatusCode::OK {
            restarted = true;
            break;
        }
        assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "already_running"));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(restarted);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (_, now) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(now["status"], "running");
    assert!(state.ptys.is_running(&id));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn concurrent_restarts_start_exactly_one_run() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    let id = sess["id"].as_str().unwrap().to_string();
    let uri = format!("/api/sessions/{id}/restart");
    let size = json!({"cols": 80, "rows": 24});
    // Several rounds, since the interleaving of the two requests varies.
    for round in 0..5 {
        call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
        wait_status(&app, &id, "finished").await;
        let ((s1, v1), (s2, v2)) =
            tokio::join!(call(&app, "POST", &uri, Some(size.clone())), call(&app, "POST", &uri, Some(size.clone())));
        let mut statuses = [s1, s2];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT], "round {round}: {v1} {v2}");
        let loser = if s1 == StatusCode::CONFLICT { &v1 } else { &v2 };
        assert_eq!(loser["code"], "already_running", "round {round}");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let (_, now) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
        assert_eq!(now["status"], "running", "round {round}");
        assert!(state.ptys.is_running(&id), "round {round}");
    }
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
    assert_eq!(sess["title"], "Shell · w");
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), Path::new(wt["path"].as_str().unwrap())));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn rename_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    let uri = format!("/api/sessions/{}", sess["id"].as_str().unwrap());
    let (s, v) = call(&app, "PATCH", &uri, Some(json!({"title": "  Build server  "}))).await;
    assert_eq!((s, v["title"].as_str().unwrap()), (StatusCode::OK, "Build server"), "{v}");
    let (_, got) = call(&app, "GET", &uri, None).await;
    assert_eq!(got["title"], "Build server");
    for bad in ["", "   ", &"x".repeat(101)] {
        let (s, v) = call(&app, "PATCH", &uri, Some(json!({"title": bad}))).await;
        assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::BAD_REQUEST, "invalid_title"));
    }
    let (s, _) = call(&app, "PATCH", "/api/sessions/nope", Some(json!({"title": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    state.ptys.shutdown().await;
}

fn same_dir(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}

/// A stand-in agent program that records its arguments to `<dir>/<name>.args` and then waits for input.
fn fake_agent(dir: &Path, name: &str) -> std::path::PathBuf {
    let args_file = dir.join(format!("{name}.args"));
    let (file, body) = if cfg!(windows) {
        (format!("{name}.cmd"), format!("@echo off\r\necho %* > \"{}\"\r\nset /p x=\r\n", args_file.display()))
    } else {
        (name.to_string(), format!("#!/bin/sh\necho \"$@\" > \"{}\"\nread x\n", args_file.display()))
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

async fn recorded_args(dir: &Path, name: &str) -> String {
    let file = dir.join(format!("{name}.args"));
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.trim().is_empty()
        {
            return text.trim().to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("{name} never recorded its arguments");
}

/// Inserts a session row directly, as a previous daemon run would have left it.
async fn seed(
    state: &AppState,
    pid: &str,
    id: &str,
    agent: Option<pdt_core::session::AgentKind>,
    cwd: &Path,
    status: pdt_core::session::SessionStatus,
    created_at: i64,
) {
    use pdt_core::session::{Session, SessionKind, SessionStatus};
    let session = Session {
        id: id.into(),
        project_id: pid.into(),
        worktree_id: None,
        kind: if agent.is_some() { SessionKind::AgentTui } else { SessionKind::Shell },
        agent,
        title: id.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        status: SessionStatus::Running,
        exit_code: None,
        created_at,
        ended_at: None,
    };
    state.store.insert_session(&session).await.unwrap();
    match status {
        SessionStatus::Running => {}
        SessionStatus::Finished => {
            state.store.end_session(id, status, Some(0), 1).await.unwrap();
        }
        SessionStatus::Interrupted => {
            state.store.end_session(id, status, None, 1).await.unwrap();
        }
    }
}

#[tokio::test]
async fn resume_interrupted_restarts_only_interrupted_sessions() {
    use pdt_core::session::SessionStatus::{Finished, Interrupted};
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo) = setup(tmp.path()).await;
    seed(&state, &pid, "int-a", None, &repo, Interrupted, 1).await;
    seed(&state, &pid, "int-b", None, &repo, Interrupted, 2).await;
    seed(&state, &pid, "done", None, &repo, Finished, 3).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 2);
    for id in ["int-a", "int-b"] {
        assert_eq!(wait_status(&app, id, "running").await["exitCode"], Value::Null);
        assert!(state.ptys.is_running(id));
    }
    let (_, done) = call(&app, "GET", "/api/sessions/done", None).await;
    assert_eq!(done["status"], "finished");
    assert!(!state.ptys.is_running("done"));
    assert_eq!(pdt_api::resume_interrupted(&state).await, 0, "nothing left to resume");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn resume_interrupted_skips_a_session_that_cannot_start() {
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo) = setup(tmp.path()).await;
    let gone = tmp.path().join("deleted worktree");
    seed(&state, &pid, "bad", None, &gone, Interrupted, 1).await;
    seed(&state, &pid, "good", None, &repo, Interrupted, 2).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 1);
    wait_status(&app, "good", "running").await;
    let (_, bad) = call(&app, "GET", "/api/sessions/bad", None).await;
    assert_eq!(bad["status"], "interrupted");
    assert!(!state.ptys.is_running("bad"));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn resume_interrupted_resumes_one_codex_session_per_folder() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, mut state, pid, repo) = setup(tmp.path()).await;
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    state.platform.codex_program = fake_agent(&bin, "fakecodex").to_string_lossy().into_owned();
    seed(&state, &pid, "old", Some(Codex), &repo, Interrupted, 1).await;
    seed(&state, &pid, "new", Some(Codex), &repo, Interrupted, 3).await;
    seed(&state, &pid, "elsewhere", Some(Codex), &other, Interrupted, 2).await;
    seed(&state, &pid, "shell", None, &repo, Interrupted, 0).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 3);
    for id in ["new", "elsewhere", "shell"] {
        wait_status(&app, id, "running").await;
    }
    let (_, old) = call(&app, "GET", "/api/sessions/old", None).await;
    assert_eq!(old["status"], "interrupted", "an older Codex session in the same folder is left alone");
    assert!(!state.ptys.is_running("old"));
    assert!(recorded_args(&bin, "fakecodex").await.starts_with("resume --last"));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn claude_resume_falls_back_to_a_fresh_conversation_when_none_was_saved() {
    use pdt_core::session::AgentKind::Claude;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (_app, mut state, pid, repo) = setup(tmp.path()).await;
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let config = tmp.path().join("claude-config");
    std::fs::create_dir_all(config.join("projects").join("some-folder")).unwrap();
    std::fs::write(config.join("projects").join("some-folder").join("saved.jsonl"), "{}\n").unwrap();
    state.platform.claude_program = fake_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    state.platform.claude_config_dir = Some(config);

    seed(&state, &pid, "unsaved", Some(Claude), &repo, Interrupted, 1).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 1);
    assert_eq!(recorded_args(&bin, "fakeclaude").await, "--session-id unsaved");
    state.ptys.shutdown().await;

    std::fs::remove_file(bin.join("fakeclaude.args")).unwrap();
    seed(&state, &pid, "saved", Some(Claude), &repo, Interrupted, 2).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 1);
    assert_eq!(recorded_args(&bin, "fakeclaude").await, "--resume saved");
    state.ptys.shutdown().await;
}
