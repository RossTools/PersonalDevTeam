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
    let mut state = AppState::new(store, tmp.join("data"));
    // Codex launches read `<codex_home>/config.toml` for the user's notify: never the real ~/.codex.
    state.platform.codex_home = Some(tmp.join("codex-home"));
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
    assert_eq!(sess["title"], "P");
    assert_eq!(sess["worktreeId"], Value::Null);
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), &repo));
    let id = sess["id"].as_str().unwrap().to_string();

    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

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
async fn removing_a_running_session_stops_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24})))
            .await;
    let id = sess["id"].as_str().unwrap().to_string();
    assert!(state.ptys.is_running(&id));

    let (s, _) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!state.ptys.is_running(&id));
    let (s, _) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 0);

    let (s, _) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
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
    // Kill only signals; the process exits a moment later (seconds for a ConPTY shell on a loaded Windows runner).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut restarted = false;
    while std::time::Instant::now() < deadline {
        let (s, v) = call(&app, "POST", &uri, Some(json!({"cols": 80, "rows": 24}))).await;
        if s == StatusCode::OK {
            restarted = true;
            break;
        }
        assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "already_running"));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(restarted, "the killed process never exited");
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
        Some(json!({"projectId": pid, "kind": "agent_tui", "agent": "codex", "mode": "terminal", "cols": 80, "rows": 24})),
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
    assert_eq!(sess["title"], "w");
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

/// A stand-in agent program that records its arguments in `<dir>/<name>.args/<launch>/args` and then waits for input.
/// Each launch claims a folder of its own (`mkdir` is atomic) and renames its file into place, so launches at the same
/// moment never interleave (PDT's Codex instructions run to several KB, more than one write) and a reader never sees
/// half of one.
fn fake_agent(dir: &Path, name: &str) -> std::path::PathBuf {
    let args_dir = dir.join(format!("{name}.args"));
    std::fs::create_dir_all(&args_dir).unwrap();
    let (file, body) = if cfg!(windows) {
        // cmd has no process id, and two launches in the same second share %RANDOM%'s sequence: mkdir decides.
        (
            format!("{name}.cmd"),
            format!(
                "@echo off
:again
set n=%RANDOM%%RANDOM%
mkdir \"{d}\\%n%\" 2>nul || goto again
echo(%* > \"{d}\\%n%\\tmp\"
ren \"{d}\\%n%\\tmp\" args
set /p x=
",
                d = args_dir.display()
            ),
        )
    } else {
        (
            name.to_string(),
            format!(
                "#!/bin/sh
mkdir \"{d}/$$\"
printf '%s\\n' \"$*\" > \"{d}/$$/tmp\"
mv \"{d}/$$/tmp\" \"{d}/$$/args\"
read x
",
                d = args_dir.display()
            ),
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

fn forget_launches(dir: &Path, name: &str) {
    let args_dir = dir.join(format!("{name}.args"));
    let _ = std::fs::remove_dir_all(&args_dir);
    std::fs::create_dir_all(&args_dir).unwrap();
}

/// The fake agent's recorded launches as it received them, in no particular order.
fn raw_launches(dir: &Path, name: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join(format!("{name}.args"))) else { return Vec::new() };
    entries.flatten().filter_map(|e| std::fs::read_to_string(e.path().join("args")).ok()).collect()
}

/// The fake agent's recorded launches, sorted, one trimmed entry each (empty for a launch without arguments).
/// Waits for `want` of them.
async fn launches(dir: &Path, name: &str, want: usize) -> Vec<String> {
    let mut found = Vec::new();
    for _ in 0..100 {
        found = raw_launches(dir, name).iter().map(|l| without_hook_args(l)).collect();
        if found.len() >= want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    found.sort();
    found
}

/// A recorded launch without PDT's activity-hook and instructions arguments (tests/activity.rs and
/// `sessions_get_pdts_instructions` cover those): Codex's leading `-c developer_instructions=… -c notify=[…]` and
/// Claude's trailing `--settings <file> --append-system-prompt-file <file>`.
fn without_hook_args(line: &str) -> String {
    let mut rest = line.trim();
    if rest.starts_with("-c developer_instructions=")
        && let Some(at) = rest.find("-c notify=")
    {
        rest = &rest[at..];
    }
    if rest.starts_with("-c ")
        && let Some(at) = rest.find("codex-notify")
        && let Some(end) = rest[at..].find(']')
    {
        rest = rest[at + end + 1..].trim_start_matches('"').trim();
    }
    if let Some(at) = rest.find("--settings") {
        rest = rest[..at].trim();
    }
    rest.to_string()
}

/// The arguments of the first launch.
async fn recorded_args(dir: &Path, name: &str) -> String {
    launches(dir, name, 1).await.into_iter().next().unwrap_or_default()
}

/// Inserts a session row directly, as a previous daemon run would have left it. A Codex session was launched fresh
/// when it was created.
async fn seed(
    state: &AppState,
    pid: &str,
    id: &str,
    agent: Option<pdt_core::session::AgentKind>,
    cwd: &Path,
    status: pdt_core::session::SessionStatus,
    created_at: i64,
) {
    let fresh = (agent == Some(pdt_core::session::AgentKind::Codex)).then_some(created_at);
    seed_with(state, pid, id, agent, cwd, status, created_at, fresh).await;
}

#[allow(clippy::too_many_arguments)]
async fn seed_with(
    state: &AppState,
    pid: &str,
    id: &str,
    agent: Option<pdt_core::session::AgentKind>,
    cwd: &Path,
    status: pdt_core::session::SessionStatus,
    created_at: i64,
    fresh_started_at: Option<i64>,
) {
    use pdt_core::session::{Session, SessionKind, SessionStatus};
    let session = Session {
        id: id.into(),
        project_id: pid.into(),
        worktree_id: None,
        kind: if agent.is_some() { SessionKind::AgentTui } else { SessionKind::Shell },
        agent,
        bypass_permissions: false,
        title: id.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        status: SessionStatus::Running,
        exit_code: None,
        created_at,
        ended_at: None,
        external_session_id: None,
        fresh_started_at,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: None,
        ticket: None,
        mode: pdt_core::conversation::SessionMode::Terminal,
        model: None,
        permission_mode: None,
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

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// Writes a Codex rollout file for a conversation that began `offset_ms` after `base`, as Codex does after a turn.
fn rollout(codex_home: &Path, id: &str, cwd: &Path, base: i64, offset_ms: i64) {
    let began = jiff::Timestamp::from_millisecond(base + offset_ms).unwrap();
    let day = began.to_zoned(jiff::tz::TimeZone::UTC).strftime("%Y/%m/%d").to_string();
    let dir = codex_home.join("sessions").join(day);
    std::fs::create_dir_all(&dir).unwrap();
    let began = began.to_string();
    let meta = json!({"timestamp": began, "type": "session_meta",
        "payload": {"id": id, "timestamp": began, "cwd": cwd.to_string_lossy(), "instructions": "x".repeat(20_000)}});
    std::fs::write(dir.join(format!("rollout-2026-10-15T00-00-00-{id}.jsonl")), format!("{meta}\n{{}}\n")).unwrap();
}

async fn external_id(state: &AppState, id: &str) -> Option<String> {
    state.store.get_session(id).await.unwrap().unwrap().external_session_id
}

async fn fresh_at(state: &AppState, id: &str) -> Option<i64> {
    state.store.get_session(id).await.unwrap().unwrap().fresh_started_at
}

/// Router state with a fake `codex` that records its launches, and an empty Codex home.
async fn codex_setup(
    tmp: &Path,
    spacing_ms: u64,
) -> (axum::Router, AppState, String, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (_, mut state, pid, repo) = setup(tmp).await;
    let (bin, home) = (tmp.join("bin"), tmp.join("codex-home"));
    for d in [&bin, &home] {
        std::fs::create_dir_all(d).unwrap();
    }
    state.platform.codex_program = fake_agent(&bin, "fakecodex").to_string_lossy().into_owned();
    state.platform.codex_home = Some(home.clone());
    state.codex_spawn_spacing = std::time::Duration::from_millis(spacing_ms);
    let app = router(state.clone(), None);
    (app, state, pid, repo, bin, home)
}

#[tokio::test]
async fn auto_resume_gives_each_codex_session_its_own_conversation() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, bin, home) = codex_setup(tmp.path(), 5_000).await;
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let now = now_ms();
    // Two sessions in the same folder (the second written with a trailing separator), each with its own saved
    // conversation, plus one in another folder that never took a turn (no rollout file) and a shell.
    let (t_a, t_b, t_c) = (now - 30_000, now - 10_000, now - 20_000);
    let repo_slash = format!("{}{}", repo.display(), std::path::MAIN_SEPARATOR);
    seed(&state, &pid, "a", Some(Codex), &repo, Interrupted, t_a).await;
    seed(&state, &pid, "b", Some(Codex), Path::new(&repo_slash), Interrupted, t_b).await;
    seed(&state, &pid, "c", Some(Codex), &other, Interrupted, t_c).await;
    seed(&state, &pid, "shell", None, &repo, Interrupted, now - 40_000).await;
    rollout(&home, "conv-a", &repo, t_a, 1_000);
    rollout(&home, "conv-b", &repo, t_b, 1_000);

    assert_eq!(pdt_api::resume_interrupted(&state).await, 4);
    for id in ["a", "b", "c", "shell"] {
        wait_status(&app, id, "running").await;
        assert!(state.ptys.is_running(id), "{id}");
    }
    assert_eq!(external_id(&state, "a").await.as_deref(), Some("conv-a"));
    assert_eq!(external_id(&state, "b").await.as_deref(), Some("conv-b"));
    assert_eq!(external_id(&state, "c").await, None);
    // c never had a turn: it starts fresh. Resuming a bound conversation does not touch the launch time.
    assert_eq!(launches(&bin, "fakecodex", 3).await, ["", "resume conv-a", "resume conv-b"]);
    assert_eq!((fresh_at(&state, "a").await, fresh_at(&state, "b").await), (Some(t_a), Some(t_b)));
    assert!(fresh_at(&state, "c").await.unwrap() >= now, "a fresh launch records its time");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn restarting_an_exited_codex_session_resumes_its_own_conversation() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Finished;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, bin, home) = codex_setup(tmp.path(), 5_000).await;
    let now = now_ms();
    let (t_a, t_b) = (now - 30_000, now - 10_000);
    seed(&state, &pid, "a", Some(Codex), &repo, Finished, t_a).await;
    seed(&state, &pid, "b", Some(Codex), &repo, Finished, t_b).await;
    rollout(&home, "conv-a", &repo, t_a, 1_000);
    rollout(&home, "conv-b", &repo, t_b, 1_000);

    // The newer session is restarted first, then the older one: neither takes the other's conversation.
    let size = json!({"cols": 80, "rows": 24});
    let (s, v) = call(&app, "POST", "/api/sessions/b/restart", Some(size.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&app, "POST", "/api/sessions/a/restart", Some(size)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(launches(&bin, "fakecodex", 2).await, ["resume conv-a", "resume conv-b"]);
    assert_eq!(external_id(&state, "a").await.as_deref(), Some("conv-a"));
    assert_eq!(external_id(&state, "b").await.as_deref(), Some("conv-b"));
    assert_eq!(fresh_at(&state, "a").await, Some(t_a));
    state.ptys.shutdown().await;
}

async fn create_codex(app: &axum::Router, pid: &str) -> Value {
    let (s, sess) = call(
        app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "kind": "agent_tui", "agent": "codex", "mode": "terminal", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    sess
}

async fn stop(app: &axum::Router, id: &str) {
    call(app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    wait_status(app, id, "finished").await;
}

/// Waits for the exit recorder to finish its binding attempt (it runs just after the status write; up to 10 s for a
/// loaded Windows runner).
async fn settle_binding(state: &AppState, id: &str, expect_bound: bool) {
    for _ in 0..100 {
        if external_id(state, id).await.is_some() == expect_bound {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if !expect_bound {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

#[tokio::test]
async fn codex_conversation_is_captured_when_the_process_exits() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, _, home) = codex_setup(tmp.path(), 5_000).await;
    let sess = create_codex(&app, &pid).await;
    let id = sess["id"].as_str().unwrap();
    let fresh = sess["freshStartedAt"].as_i64().unwrap();
    rollout(&home, "conv-live", &repo, fresh, 500);
    stop(&app, id).await;
    settle_binding(&state, id, true).await;
    assert_eq!(external_id(&state, id).await.as_deref(), Some("conv-live"));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_codex_started_by_hand_later_is_not_claimed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, _, home) = codex_setup(tmp.path(), 5_000).await;
    let sess = create_codex(&app, &pid).await;
    let id = sess["id"].as_str().unwrap();
    // The session itself never took a turn; the user ran codex in a shell a minute later.
    rollout(&home, "by-hand", &repo, sess["freshStartedAt"].as_i64().unwrap(), 60_000);
    stop(&app, id).await;
    settle_binding(&state, id, false).await;
    assert_eq!(external_id(&state, id).await, None);
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn an_unbound_session_does_not_take_a_later_neighbours_conversation() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, bin, home) = codex_setup(tmp.path(), 100).await;
    let now = now_ms();
    let (t_a, t_b) = (now - 120_000, now - 60_000);
    seed(&state, &pid, "a", Some(Codex), &repo, Interrupted, t_a).await;
    seed(&state, &pid, "b", Some(Codex), &repo, Interrupted, t_b).await;
    rollout(&home, "conv-b", &repo, t_b, 1_000);

    assert_eq!(pdt_api::resume_interrupted(&state).await, 2);
    wait_status(&app, "a", "running").await;
    wait_status(&app, "b", "running").await;
    assert_eq!(external_id(&state, "a").await, None, "A never took a turn");
    assert_eq!(external_id(&state, "b").await.as_deref(), Some("conv-b"));
    assert_eq!(launches(&bin, "fakecodex", 2).await, ["", "resume conv-b"]);
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn fresh_codex_launches_in_one_folder_are_spaced_and_attributed_by_start_order() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, _, home) = codex_setup(tmp.path(), 2_000).await;
    // Two created at once: the second waits out the spacing instead of launching alongside the first.
    let (a, b) = tokio::join!(create_codex(&app, &pid), create_codex(&app, &pid));
    let (ida, idb) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
    let (fa, fb) = (a["freshStartedAt"].as_i64().unwrap(), b["freshStartedAt"].as_i64().unwrap());
    assert!((fa - fb).abs() >= 2_000, "launches {fa} and {fb} are too close");
    rollout(&home, "conv-a", &repo, fa, 200);
    rollout(&home, "conv-b", &repo, fb, 200);
    stop(&app, ida).await;
    stop(&app, idb).await;
    settle_binding(&state, ida, true).await;
    settle_binding(&state, idb, true).await;
    assert_eq!(external_id(&state, ida).await.as_deref(), Some("conv-a"));
    assert_eq!(external_id(&state, idb).await.as_deref(), Some("conv-b"));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn launches_too_close_together_are_ambiguous_and_never_bound() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, bin, home) = codex_setup(tmp.path(), 3_000).await;
    let now = now_ms();
    // Injected directly: two launches 2 s apart, which the spacing would normally prevent.
    let (t_a, t_b) = (now - 60_000, now - 58_000);
    seed(&state, &pid, "a", Some(Codex), &repo, Interrupted, t_a).await;
    seed(&state, &pid, "b", Some(Codex), &repo, Interrupted, t_b).await;
    rollout(&home, "conv-a", &repo, t_a, 500);
    rollout(&home, "conv-b", &repo, t_b, 500);

    assert_eq!(pdt_api::resume_interrupted(&state).await, 2);
    wait_status(&app, "a", "running").await;
    wait_status(&app, "b", "running").await;
    assert_eq!((external_id(&state, "a").await, external_id(&state, "b").await), (None, None));
    assert_eq!(launches(&bin, "fakecodex", 2).await, ["", ""], "both start fresh");
    // Starting them fresh spaces their new launches.
    let (fa, fb) = (fresh_at(&state, "a").await.unwrap(), fresh_at(&state, "b").await.unwrap());
    assert!((fa - fb).abs() >= 3_000, "{fa} {fb}");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_codex_session_with_no_recorded_launch_resumes_fresh() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, bin, home) = codex_setup(tmp.path(), 100).await;
    let now = now_ms();
    seed_with(&state, &pid, "old-row", Some(Codex), &repo, Interrupted, now - 30_000, None).await;
    rollout(&home, "conv-x", &repo, now - 30_000, 1_000);
    assert_eq!(pdt_api::resume_interrupted(&state).await, 1);
    wait_status(&app, "old-row", "running").await;
    assert_eq!(external_id(&state, "old-row").await, None);
    assert_eq!(launches(&bin, "fakecodex", 1).await, [""]);
    assert!(fresh_at(&state, "old-row").await.unwrap() >= now, "now it has a launch time to be matched by");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn startup_spaces_fresh_codex_launches_in_one_folder() {
    use pdt_core::session::AgentKind::Codex;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo, _, _) = codex_setup(tmp.path(), 1_000).await;
    let now = now_ms();
    seed(&state, &pid, "a", Some(Codex), &repo, Interrupted, now - 60_000).await;
    seed(&state, &pid, "b", Some(Codex), &repo, Interrupted, now - 30_000).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 2);
    wait_status(&app, "a", "running").await;
    wait_status(&app, "b", "running").await;
    let (fa, fb) = (fresh_at(&state, "a").await.unwrap(), fresh_at(&state, "b").await.unwrap());
    assert!((fa - fb).abs() >= 1_000, "{fa} {fb}");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_manual_restart_racing_auto_resume_starts_exactly_one_process() {
    use pdt_core::session::AgentKind::Claude;
    use pdt_core::session::SessionStatus::Interrupted;
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, pid, repo) = setup(tmp.path()).await;
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    state.platform.claude_program = fake_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let app = router(state.clone(), None);
    let size = json!({"cols": 80, "rows": 24});
    // Several rounds, since the interleaving varies: either the restart wins (auto-resume then skips the session)
    // or auto-resume wins (the restart gets "already running").
    for round in 0..6 {
        let id = format!("race-{round}");
        seed(&state, &pid, &id, Some(Claude), &repo, Interrupted, 1 + round).await;
        let plan = pdt_api::plan_resume_interrupted(&state).await;
        let uri = format!("/api/sessions/{id}/restart");
        let (resumed, (status, body)) = tokio::join!(plan.start(&state), call(&app, "POST", &uri, Some(size.clone())));
        match (status, resumed) {
            (StatusCode::OK, 0) | (StatusCode::CONFLICT, 1) => {}
            other => panic!("round {round}: {other:?} {body}"),
        }
        assert!(state.ptys.is_running(&id), "round {round}");
        wait_status(&app, &id, "running").await;
    }
    // One recorded launch per round, nothing started twice.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(launches(&bin, "fakeclaude", 6).await.len(), 6);
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

    forget_launches(&bin, "fakeclaude");
    seed(&state, &pid, "saved", Some(Claude), &repo, Interrupted, 2).await;
    assert_eq!(pdt_api::resume_interrupted(&state).await, 1);
    assert_eq!(recorded_args(&bin, "fakeclaude").await, "--resume saved");
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn sessions_get_pdts_instructions() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, pid, _repo, bin, home) = codex_setup(tmp.path(), 0).await;
    state.platform.claude_program = fake_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    std::fs::write(home.join("config.toml"), "developer_instructions = \"Be brief.\"\n").unwrap();
    let app = router(state.clone(), None);
    let settings = json!({"agentCredentials": null, "runOverrides": null, "agentInstructions": "Run make check."});
    let (s, v) = call(&app, "PUT", &format!("/api/projects/{pid}/settings"), Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, claude) = call(
        &app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "kind": "agent_tui", "agent": "claude", "mode": "terminal", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{claude}");
    let id = claude["id"].as_str().unwrap();
    let file = state.data_dir.join("sessions").join(id).join("instructions.md");
    let args = launches(&bin, "fakeclaude", 1).await;
    let raw = raw_launches(&bin, "fakeclaude").concat();
    assert!(raw.contains(&format!("--append-system-prompt-file {}", file.display())), "{raw} / {args:?}");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&format!("This session: `{id}`")) && text.ends_with("Run make check.\n"), "{text}");

    let codex = create_codex(&app, &pid).await;
    launches(&bin, "fakecodex", 1).await;
    let raw = raw_launches(&bin, "fakecodex").concat();
    if cfg!(windows) {
        // Through cmd.exe the agent is pointed at the file instead.
        assert!(raw.contains("developer_instructions='Read "), "{raw}");
    } else {
        assert!(raw.starts_with("-c developer_instructions=\"Be brief.\\n\\n# Running in"), "{raw}");
        assert!(raw.contains("Run make check."), "{raw}");
    }
    let codex_file = state.data_dir.join("sessions").join(codex["id"].as_str().unwrap()).join("instructions.md");
    assert!(std::fs::read_to_string(codex_file).unwrap().starts_with("Be brief.\n\n"), "the user's own come first");
    state.ptys.shutdown().await;
}
