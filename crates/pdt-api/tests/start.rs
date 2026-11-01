//! Ticket Start, attempts and merge detection (tickets spec §5, §6), with stand-in agents.

use crate::common;

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
    repo: PathBuf,
    bin: PathBuf,
}

/// A project "Personal Dev Team" (key PDT) whose repo has one commit on main, with stand-in `claude` and `codex`.
async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("my repo");
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
    state.codex_spawn_spacing = std::time::Duration::from_millis(10);
    let app = router(state.clone(), None);
    let body = json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().to_string(), repo, bin }
}

async fn ticket(env: &Env, body: Value) -> String {
    let (s, v) = call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.pid), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

/// Starts a ticket; a terminal unless the body says (these tests read the agent's argv).
async fn start(env: &Env, id: &str, body: Value) -> (StatusCode, Value) {
    let mut body = body;
    if body.get("sessionMode").is_none() && body.get("fromSession").is_none() {
        body["sessionMode"] = json!("terminal");
    }
    call(&env.app, "POST", &format!("/api/tickets/{id}/start"), Some(body)).await
}

async fn started(env: &Env, id: &str, body: Value) -> Value {
    let (s, v) = start(env, id, body).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn get_ticket(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn event_kinds(env: &Env, id: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{id}/events"), None).await;
    v["items"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap().to_string()).collect()
}

/// [`event_kinds`] but comments: what PDT notes after an attempt ends (what it stopped and cleaned up, PDT-339).
async fn event_kinds_but_comments(env: &Env, id: &str) -> Vec<String> {
    event_kinds(env, id).await.into_iter().filter(|k| k != "comment").collect()
}

/// The event log's types for a ticket (what workflows see), oldest first.
async fn logged(env: &Env, id: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/events?ticket={id}"), None).await;
    let mut kinds: Vec<String> =
        v.as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap().to_string()).collect();
    kinds.reverse();
    kinds
}

async fn workspaces(env: &Env) -> Vec<Value> {
    call(&env.app, "GET", "/api/workspaces", None).await.1.as_array().unwrap().clone()
}

/// The (kind, id) of each tab in a workspace, in order.
fn tabs(ws: &Value) -> Vec<(&'static str, String)> {
    ws["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            if let Some(s) = t["sessionId"].as_str() {
                ("session", s.to_string())
            } else if let Some(r) = t["runId"].as_str() {
                ("run", r.to_string())
            } else {
                ("ticket", t["ticketId"].as_str().unwrap().to_string())
            }
        })
        .collect()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

/// The prompt the agent received: its last argument (on Windows, where the stand-in is a `.cmd` shim, the short
/// prompt naming the prompt file, checked separately).
fn last_arg(launch: &[String]) -> &str {
    launch.last().map(String::as_str).unwrap_or("")
}

#[tokio::test]
async fn start_makes_the_branch_attempt_session_and_tabs() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id =
        ticket(&env, json!({"title": "Fix the login", "description": "Steps & notes:\n1. open", "stage": "ready"}))
            .await;
    assert_eq!(get_ticket(&env, &id).await["defaultBranch"], "pdt-PDT-1-fix-the-login");

    let r = started(&env, &id, json!({"agent": "claude", "cols": 100, "rows": 40})).await;
    let (t, attempt, session) = (&r["ticket"], &r["attempt"], &r["session"]);
    assert_eq!(t["stage"], "in_progress");
    assert_eq!((s(&attempt["branch"]), s(&attempt["state"])), ("pdt-PDT-1-fix-the-login", "active"));
    assert_eq!(attempt["baseBranch"], "main");
    assert_eq!(attempt["lastHead"], Value::Null);
    assert_eq!(s(&session["title"]), "Fix the login");
    assert_eq!(session["attemptId"], attempt["id"]);
    assert_eq!(session["ticket"], json!({"id": id, "key": "PDT-1"}));
    assert_eq!(session["status"], "running");
    assert_eq!(t["attempt"]["id"], attempt["id"]);
    assert_eq!(t["sessions"][0]["id"], session["id"]);
    assert_eq!(t["sessions"][0]["attemptId"], attempt["id"]);
    let wt = env.repo.join(".worktrees").join("pdt-PDT-1-fix-the-login");
    assert!(wt.join("a.txt").is_file(), "the worktree is checked out");
    assert_eq!(attempt["worktreeId"], session["worktreeId"]);

    // The agent got the ticket as its first prompt, after its flags (and after --settings, PDT's instructions and
    // pdt-mcp for Claude).
    let launch = recorded_launches(&env.bin, "fakeclaude", 1).await.remove(0);
    let sid = s(&session["id"]).to_string();
    if cfg!(windows) {
        let file = env.state.data_dir.join("sessions").join(&sid).join("prompt.md");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("PDT-1: Fix the login\n\nSteps & notes:"), "{text}");
        assert!(launch[0].contains("PDT-1"), "{launch:?}");
    } else {
        assert_eq!(launch[..2], ["--session-id".to_string(), sid.clone()]);
        assert_eq!(launch[launch.len() - 9], "--settings");
        assert_eq!(launch[launch.len() - 7], "--append-system-prompt-file");
        assert_eq!(launch[launch.len() - 5], "--mcp-config");
        assert_eq!(launch[launch.len() - 3], "--allowedTools");
        assert_eq!(
            last_arg(&launch),
            format!(
                "PDT-1: Fix the login\n\nSteps & notes:\n1. open\n\n(Ticket PDT-1 in PDT. Work in this worktree on \
                 branch pdt-PDT-1-fix-the-login. {})",
                pdt_core::start::ONE_TICKET
            )
        );
    }

    // Timeline: moved to In progress, started, the session.
    assert_eq!(event_kinds(&env, &id).await, ["created", "moved", "started", "session"]);
    // The sessions list carries the ticket.
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.pid), None).await;
    assert_eq!(list[0]["ticket"]["key"], "PDT-1");
    // Tabs: the ticket, then the session, at the end of the project's workspace.
    let ws = workspaces(&env).await;
    assert_eq!(tabs(&ws[0]), [("ticket", id.clone()), ("session", sid.clone())]);

    // One active attempt at a time.
    let (st, v) = start(&env, &id, json!({"agent": "codex"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "ticket_active"), "{v}");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn codex_gets_the_prompt_after_its_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = ticket(&env, json!({"title": "Add -v flag"})).await;
    // The ticket's comments are in the prompt too.
    let (st, _) =
        call(&env.app, "POST", &format!("/api/tickets/{id}/comments"), Some(json!({"body": "Use clap."}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let r = started(&env, &id, json!({"agent": "codex", "bypassPermissions": true, "branch": "feature/v"})).await;
    assert_eq!(r["attempt"]["branch"], "feature/v");
    assert_eq!(r["session"]["title"], "Add -v flag");
    assert_eq!(r["session"]["bypassPermissions"], true);
    let launch = recorded_launches(&env.bin, "fakecodex", 1).await.remove(0);
    if !cfg!(windows) {
        let n = launch.len();
        assert_eq!(launch[n - 2], "--dangerously-bypass-approvals-and-sandbox");
        assert_eq!(
            launch[n - 1],
            format!(
                "PDT-1: Add -v flag\n\nComments, oldest first:\n\n> Use clap.\n\n(Ticket PDT-1 in PDT. Work in this \
                 worktree on branch feature/v. {})",
                pdt_core::start::ONE_TICKET
            )
        );
    }
    env.state.ptys.shutdown().await;
}

/// The instructions file of a started ticket's session.
fn instructions(env: &Env, r: &Value) -> String {
    let file = env.state.data_dir.join("sessions").join(s(&r["session"]["id"])).join("instructions.md");
    std::fs::read_to_string(file).unwrap()
}

#[tokio::test]
async fn a_started_session_is_told_which_pdt_messages_to_expect() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    pdt_api::run_workflows(&env.state).await;
    let a = ticket(&env, json!({"title": "A"})).await;
    let text = instructions(&env, &started(&env, &a, json!({"agent": "claude"})).await);
    assert!(text.contains("This session works on ticket PDT-1"), "{text}");
    assert!(text.contains("when CI fails on its pull request, the pull request conflicts with `main`"), "{text}");
    assert!(text.contains("the user comments on the ticket") && text.contains("- Merged:"), "{text}");
    assert!(text.contains("don't poll or watch CI in a shell"), "{text}");
    assert!(text.contains("moves while it waits"), "PR notices handles a PR behind its base (PDT-104): {text}");

    // With the workflows that message the agent turned off, nothing is promised.
    let (_, list) = call(&env.app, "GET", &format!("/api/workflows?project={}", env.pid), None).await;
    for w in list.as_array().unwrap() {
        if matches!(w["builtin"].as_str(), Some("pr-notices" | "comments-to-agent")) {
            let path = format!("/api/workflows/{}", s(&w["id"]));
            let (st, v) = call(&env.app, "PATCH", &path, Some(json!({"enabled": false}))).await;
            assert_eq!(st, StatusCode::OK, "{v}");
        }
    }
    let b = ticket(&env, json!({"title": "B"})).await;
    let text = instructions(&env, &started(&env, &b, json!({"agent": "claude"})).await);
    assert!(text.contains("This session works on ticket PDT-2") && text.contains("Don't move the ticket"), "{text}");
    assert!(!text.contains("Messages from PDT") && !text.contains("gh run watch"), "{text}");

    // A session that is not on a ticket gets neither.
    let body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "mode": "terminal", "cols": 80, "rows": 24});
    let (st, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    let text = instructions(&env, &json!({"session": v}));
    assert!(!text.contains("This session's ticket"), "{text}");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn blockers_refuse_a_check_start_and_force_records_it() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let a = ticket(&env, json!({"title": "A"})).await;
    let b = ticket(&env, json!({"title": "B"})).await;
    let (st, v) =
        call(&env.app, "POST", &format!("/api/tickets/{a}/links"), Some(json!({"to": b, "kind": "blocks"}))).await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let (st, v) = start(&env, &b, json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "start_blocked"), "{v}");
    assert!(s(&v["message"]).contains("PDT-1"), "{v}");
    assert!(!env.repo.join(".worktrees").join("pdt-PDT-2-b").exists(), "nothing was made");
    assert_eq!(get_ticket(&env, &b).await["stage"], "draft");

    let r = started(&env, &b, json!({"agent": "claude", "mode": "force"})).await;
    assert_eq!(r["ticket"]["stage"], "in_progress");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{b}/events"), None).await;
    let started_event = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "started").unwrap().clone();
    assert_eq!(started_event["data"]["forced"], true);
    assert_eq!(started_event["data"]["blockers"], json!(["PDT-1"]));

    // A done blocker no longer blocks.
    let (st, _) = call(&env.app, "PATCH", &format!("/api/tickets/{a}"), Some(json!({"stage": "done"}))).await;
    assert_eq!(st, StatusCode::OK);
    let c = ticket(&env, json!({"title": "C"})).await;
    call(&env.app, "POST", &format!("/api/tickets/{a}/links"), Some(json!({"to": c, "kind": "blocks"}))).await;
    started(&env, &c, json!({"agent": "claude"})).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn closed_unknown_and_invalid_starts_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let done = ticket(&env, json!({"title": "Done", "stage": "done"})).await;
    let wont = ticket(&env, json!({"title": "Nope", "stage": "wont_do"})).await;
    for id in [&done, &wont] {
        let (st, v) = start(&env, id, json!({"agent": "claude"})).await;
        assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "ticket_closed"), "{v}");
    }
    let open = ticket(&env, json!({"title": "Open"})).await;
    for body in [
        json!({"agent": "claude", "branch": "a b"}),
        json!({"agent": "claude", "cols": 5}),
        json!({"agent": "gemini"}),
        json!({"agent": "claude", "mode": "wait"}),
        json!({"agent": "claude", "tab": {"workspaceId": "nope"}}),
    ] {
        let (st, v) = start(&env, &open, body.clone()).await;
        assert_eq!(st.as_u16() / 100, 4, "{body} {v}");
        assert_eq!(s(&v["code"]), "invalid_input", "{body} {v}");
    }
    let (st, v) = start(&env, "nope", json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::NOT_FOUND, "not_found"));

    // A folder in the worktree's place that is not that branch's worktree.
    std::fs::create_dir_all(env.repo.join(".worktrees").join("pdt-PDT-3-open")).unwrap();
    let (st, v) = start(&env, &open, json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "worktree_path_taken"), "{v}");
    assert_eq!(get_ticket(&env, &open).await["stage"], "draft");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn tabs_open_in_a_new_workspace_or_where_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let a = ticket(&env, json!({"title": "A"})).await;
    let r = started(&env, &a, json!({"agent": "claude", "newWorkspace": true})).await;
    let sid = s(&r["session"]["id"]).to_string();
    let ws = workspaces(&env).await;
    let mine = ws.iter().find(|w| w["name"] == "PDT-1").expect("a workspace named after the ticket");
    assert_eq!(mine["projectId"], Value::Null);
    assert_eq!(tabs(mine), [("ticket", a.clone()), ("session", sid)]);
    assert!(tabs(&ws[0]).is_empty(), "nothing in the project's workspace");
    assert_eq!(r["attempt"]["workspaceId"], mine["id"], "the attempt remembers its workspace");
    assert_eq!(r["ticket"]["attempt"]["workspaceId"], mine["id"]);

    // At a placement: the ticket's tab there, the session's right after it.
    let (_, free) = call(&env.app, "POST", "/api/workspaces", Some(json!({"name": "Free"}))).await;
    let free_id = s(&free["id"]).to_string();
    let other = ticket(&env, json!({"title": "Other"})).await;
    call(&env.app, "POST", &format!("/api/workspaces/{free_id}/tabs"), Some(json!({"ticketId": other}))).await;
    let b = ticket(&env, json!({"title": "B"})).await;
    let r = started(&env, &b, json!({"agent": "claude", "tab": {"workspaceId": free_id, "position": 0}})).await;
    let sid = s(&r["session"]["id"]).to_string();
    let ws = workspaces(&env).await;
    let free = ws.iter().find(|w| w["id"] == free_id.as_str()).unwrap();
    assert_eq!(tabs(free), [("ticket", b.clone()), ("session", sid), ("ticket", other)]);
    assert_eq!(r["attempt"]["workspaceId"], Value::Null, "an existing workspace is not the attempt's");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_start_without_a_session_makes_the_worktree_attempt_and_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = ticket(&env, json!({"title": "Quiet", "stage": "ready"})).await;
    let r = started(&env, &id, json!({"agent": "claude", "noSession": true, "newWorkspace": true})).await;
    assert_eq!(r["session"], Value::Null);
    assert_eq!(r["ticket"]["stage"], "in_progress");
    assert_eq!(r["attempt"]["state"], "active");
    assert_eq!(r["ticket"]["sessions"], json!([]));
    assert!(env.repo.join(".worktrees").join("pdt-PDT-1-quiet").join("a.txt").is_file(), "the worktree is checked out");
    let ws = workspaces(&env).await;
    let mine = ws.iter().find(|w| w["name"] == "PDT-1").expect("a workspace named after the ticket");
    assert_eq!(tabs(mine), [("ticket", id.clone())]);
    assert_eq!(r["attempt"]["workspaceId"], mine["id"]);
    assert_eq!(event_kinds(&env, &id).await, ["created", "moved", "started"]);
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.pid), None).await;
    assert_eq!(list, json!([]), "no agent was launched");
}

#[tokio::test]
async fn a_failed_session_rolls_the_start_back_and_keeps_the_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = setup(tmp.path()).await;
    let good = env.state.platform.claude_program.clone();
    env.state.platform.claude_program = "pdt-no-such-claude-xyz".into();
    env.app = router(env.state.clone(), None);
    let id = ticket(&env, json!({"title": "Retry me", "stage": "ready"})).await;
    let (st, v) = start(&env, &id, json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::BAD_REQUEST, "program_not_found"), "{v}");
    let t = get_ticket(&env, &id).await;
    assert_eq!(t["stage"], "ready");
    assert_eq!(t["attempts"], json!([]));
    assert_eq!(event_kinds(&env, &id).await, ["created"]);
    let wt = env.repo.join(".worktrees").join("pdt-PDT-1-retry-me");
    assert!(wt.is_dir(), "the worktree is kept");
    // No workflow ever saw it: an `attempt.started` for an attempt that is gone fails the Pipeline's Coder step.
    assert!(!logged(&env, &id).await.contains(&"attempt.started".to_string()), "{:?}", logged(&env, &id).await);
    pdt_api::run_workflows(&env.state).await;
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={id}"), None).await;
    assert_eq!(runs, json!([]));

    env.state.platform.claude_program = good;
    env.app = router(env.state.clone(), None);
    let r = started(&env, &id, json!({"agent": "claude"})).await;
    assert_eq!(r["attempt"]["state"], "active");
    let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert_eq!(wts.as_array().unwrap().len(), 1, "the kept worktree is reused: {wts}");
    assert_eq!(logged(&env, &id).await, ["ticket.stage_changed", "attempt.started"], "logged once it started");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn abandon_ends_the_attempt_and_removes_a_worktree_with_nothing_to_lose() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = ticket(&env, json!({"title": "Give up"})).await;
    let r = started(&env, &id, json!({"agent": "claude"})).await;
    let aid = s(&r["attempt"]["id"]).to_string();
    let (st, t) = call(&env.app, "POST", &format!("/api/attempts/{aid}/abandon"), None).await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(t["attempts"][0]["state"], "abandoned");
    assert!(t["attempts"][0]["endedAt"].is_number());
    assert!(event_kinds(&env, &id).await.iter().any(|k| k == "abandoned"));
    assert!(!env.repo.join(".worktrees").join("pdt-PDT-1-give-up").exists(), "nothing to lose: removed (PDT-339)");
    // Again: nothing changes.
    let (st, _) = call(&env.app, "POST", &format!("/api/attempts/{aid}/abandon"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(event_kinds(&env, &id).await.iter().filter(|k| *k == "abandoned").count(), 1);
    let (st, _) = call(&env.app, "POST", "/api/attempts/nope/abandon", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // An active attempt's worktree is not removed from under it.
    let r = started(&env, &id, json!({"agent": "claude", "noSession": true})).await;
    let wt = s(&r["attempt"]["worktreeId"]).to_string();
    let (st, v) = call(&env.app, "DELETE", &format!("/api/worktrees/{wt}"), None).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "worktree_attempt"), "{v}");
    assert!(env.repo.join(".worktrees").join("pdt-PDT-1-give-up").is_dir());
    let aid = s(&r["attempt"]["id"]).to_string();
    std::fs::write(env.repo.join(".worktrees").join("pdt-PDT-1-give-up").join("wip.txt"), "wip").unwrap();
    call(&env.app, "POST", &format!("/api/attempts/{aid}/abandon"), None).await;
    assert!(env.repo.join(".worktrees").join("pdt-PDT-1-give-up").is_dir(), "uncommitted work: kept");
    // A new attempt can start, on the same worktree.
    let r = started(&env, &id, json!({"agent": "claude"})).await;
    assert_eq!(r["ticket"]["attempts"].as_array().unwrap().len(), 3);
    env.state.ptys.shutdown().await;
}

/// Commits a change in the attempt's worktree; returns the worktree.
fn work_in(env: &Env, folder: &str, file: &str) -> PathBuf {
    let wt = env.repo.join(".worktrees").join(folder);
    std::fs::write(wt.join(file), "work\n").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    wt
}

#[tokio::test]
async fn merges_are_detected_on_demand_and_by_the_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let a = ticket(&env, json!({"title": "Merge me"})).await;
    started(&env, &a, json!({"agent": "claude"})).await;
    work_in(&env, "pdt-PDT-1-merge-me", "m.txt");

    // Not merged yet: still active, and the head is remembered.
    let (st, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/check-merge"), None).await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(t["attempts"][0]["state"], "active");
    assert!(t["attempts"][0]["lastHead"].is_string());

    git(&env.repo, &["merge", "-q", "--no-ff", "-m", "merge", "pdt-PDT-1-merge-me"]);
    let (_, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/check-merge"), None).await;
    assert_eq!(t["attempts"][0]["state"], "merged");
    assert_eq!(t["stage"], "in_progress", "the ticket never moves to Done by itself");
    assert_eq!(event_kinds_but_comments(&env, &a).await.last().map(String::as_str), Some("merged"));

    // The 60 s loop: a squash merge, then the branch deleted.
    let b = ticket(&env, json!({"title": "Squash me"})).await;
    started(&env, &b, json!({"agent": "claude"})).await;
    work_in(&env, "pdt-PDT-2-squash-me", "s.txt");
    pdt_api::check_merges(&env.state).await;
    assert_eq!(get_ticket(&env, &b).await["attempts"][0]["state"], "active");
    git(&env.repo, &["merge", "-q", "--squash", "pdt-PDT-2-squash-me"]);
    git(&env.repo, &["commit", "-q", "-m", "squashed"]);
    // Free the branch so it can be deleted. Detached rather than removed: on Windows the agent still running in the
    // folder keeps it from being deleted.
    git(&env.repo.join(".worktrees/pdt-PDT-2-squash-me"), &["checkout", "-q", "--detach"]);
    git(&env.repo, &["branch", "-q", "-D", "pdt-PDT-2-squash-me"]);
    pdt_api::check_merges(&env.state).await;
    assert_eq!(get_ticket(&env, &b).await["attempts"][0]["state"], "merged");
    let (_, check) = call(&env.app, "POST", "/api/tickets/nope/check-merge", None).await;
    assert_eq!(check["code"], "not_found");
    env.state.ptys.shutdown().await;
}

/// PDT-45: Start branches from `origin/main` when it is ahead of the local main, and a branch that only caught up
/// with main (fast-forwarded, no commits of its own) is not called merged; once it has its own work, a squash is.
#[tokio::test]
async fn a_branch_that_only_caught_up_with_main_is_not_merged() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let upstream = tmp.path().join("upstream");
    git(tmp.path(), &["clone", "-q", &env.repo.to_string_lossy(), &upstream.to_string_lossy()]);
    git(&env.repo, &["remote", "add", "origin", &upstream.to_string_lossy()]);
    let commit_upstream = |file: &str| {
        std::fs::write(upstream.join(file), "up\n").unwrap();
        git(&upstream, &["add", "-A"]);
        git(&upstream, &["commit", "-q", "-m", file]);
        pdt_git::rev_parse(&upstream, "HEAD").unwrap()
    };
    let ahead = commit_upstream("u1.txt");

    let a = ticket(&env, json!({"title": "Catch up"})).await;
    started(&env, &a, json!({"agent": "claude"})).await;
    let wt = env.repo.join(".worktrees/pdt-PDT-1-catch-up");
    assert_eq!(pdt_git::rev_parse(&wt, "HEAD"), Some(ahead), "the branch starts from origin/main, which was ahead");
    assert_ne!(pdt_git::rev_parse(&env.repo, "main"), pdt_git::rev_parse(&wt, "HEAD"), "local main untouched");
    let upstream_branch = std::process::Command::new("git")
        .current_dir(&wt)
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .output()
        .unwrap();
    assert!(!upstream_branch.status.success(), "the branch does not track origin/main");

    // origin/main moves on and the branch fast-forwards to it: nothing of its own, so not merged.
    commit_upstream("u2.txt");
    git(&env.repo, &["fetch", "-q", "origin"]);
    git(&wt, &["merge", "-q", "--ff-only", "origin/main"]);
    let (_, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/check-merge"), None).await;
    assert_eq!(t["attempts"][0]["state"], "active", "{t}");
    pdt_api::check_merges(&env.state).await;
    assert_eq!(get_ticket(&env, &a).await["attempts"][0]["state"], "active");

    // Its own work, squash-merged into main: merged.
    work_in(&env, "pdt-PDT-1-catch-up", "w.txt");
    let (_, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/check-merge"), None).await;
    assert_eq!(t["attempts"][0]["state"], "active");
    git(&env.repo, &["merge", "-q", "--ff-only", "origin/main"]);
    git(&env.repo, &["merge", "-q", "--squash", "pdt-PDT-1-catch-up"]);
    git(&env.repo, &["commit", "-q", "-m", "squashed"]);
    let (_, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/check-merge"), None).await;
    assert_eq!(t["attempts"][0]["state"], "merged");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn closing_a_ticket_ends_its_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    // Unmerged work: abandoned.
    let a = ticket(&env, json!({"title": "Drop"})).await;
    started(&env, &a, json!({"agent": "claude"})).await;
    // A close the PATCH refuses ends nothing.
    let bad = json!({"stage": "done", "waiting": true});
    let (st, v) = call(&env.app, "PATCH", &format!("/api/tickets/{a}"), Some(bad)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let t = get_ticket(&env, &a).await;
    assert_eq!((s(&t["stage"]), s(&t["attempts"][0]["state"])), ("in_progress", "active"));
    let (st, t) = call(&env.app, "PATCH", &format!("/api/tickets/{a}"), Some(json!({"stage": "wont_do"}))).await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!((s(&t["stage"]), s(&t["attempts"][0]["state"])), ("wont_do", "abandoned"));
    let kinds = event_kinds_but_comments(&env, &a).await;
    assert_eq!(kinds[kinds.len() - 2..], ["moved".to_string(), "abandoned".to_string()]);

    // Merged work: merged.
    let b = ticket(&env, json!({"title": "Ship"})).await;
    started(&env, &b, json!({"agent": "claude"})).await;
    work_in(&env, "pdt-PDT-2-ship", "ship.txt");
    git(&env.repo, &["merge", "-q", "--ff-only", "pdt-PDT-2-ship"]);
    let (_, t) = call(&env.app, "PATCH", &format!("/api/tickets/{b}"), Some(json!({"stage": "done"}))).await;
    assert_eq!((s(&t["stage"]), s(&t["attempts"][0]["state"])), ("done", "merged"));

    // Done says the work landed, even when git cannot see it: never abandoned. Bulk close too.
    let c = ticket(&env, json!({"title": "Bulk"})).await;
    started(&env, &c, json!({"agent": "claude"})).await;
    work_in(&env, "pdt-PDT-3-bulk", "bulk.txt");
    let (st, v) =
        call(&env.app, "POST", "/api/tickets/bulk", Some(json!({"ids": [c], "action": "stage", "value": "done"})))
            .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(get_ticket(&env, &c).await["attempts"][0]["state"], "merged");

    // The board's Done column: recently closed tickets.
    let (_, page) =
        call(&env.app, "GET", &format!("/api/tickets?project={}&stage=done&closedAfter=1", env.pid), None).await;
    assert_eq!(page["total"], 2);
    let (_, page) =
        call(&env.app, "GET", &format!("/api/tickets?project={}&closedAfter=99999999999999", env.pid), None).await;
    assert_eq!(page["total"], 0, "every ticket here is closed, none that recently");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn new_sessions_take_an_initial_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "mode": "terminal", "cols": 80,
                      "rows": 24, "initialPrompt": "Say hi\nthen stop"});
    let (st, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!((&v["attemptId"], &v["ticket"]), (&Value::Null, &Value::Null));
    let id = s(&v["id"]).to_string();
    let launch = recorded_launches(&env.bin, "fakeclaude", 1).await.remove(0);
    if cfg!(windows) {
        let file = env.state.data_dir.join("sessions").join(s(&v["id"])).join("prompt.md");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "Say hi\nthen stop");
        assert!(launch[0].contains("and follow it."), "{launch:?}");
    } else {
        assert_eq!(last_arg(&launch), "Say hi\nthen stop");
    }
    let shell = json!({"projectId": env.pid, "kind": "shell", "cols": 80, "rows": 24, "initialPrompt": "ls"});
    let (st, v) = call(&env.app, "POST", "/api/sessions", Some(shell)).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::BAD_REQUEST, "invalid_input"));
    let blank = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24,
                       "initialPrompt": "  "});
    let (st, _) = call(&env.app, "POST", "/api/sessions", Some(blank)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // A restart never repeats the prompt.
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    for _ in 0..100 {
        if call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1["status"] == "finished" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let (st, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let launches = recorded_launches(&env.bin, "fakeclaude", 2).await;
    assert_eq!(launches.len(), 2);
    assert!(!launches[1].iter().any(|a| a.contains("Say hi") || a.contains("follow it")), "{launches:?}");
    env.state.ptys.shutdown().await;
}

// Areas and overlap (tickets spec §7).

const AREAS: &str = r#"
[[area]]
name = "migrations"
paths = ["db/**"]
exclusive = true

[[area]]
name = "web"
paths = ["web/**"]
"#;

fn configure_areas(env: &Env) {
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    std::fs::write(env.repo.join(".pdt/project.toml"), AREAS).unwrap();
    git(&env.repo, &["add", "-A"]);
    git(&env.repo, &["commit", "-q", "-m", "areas"]);
}

async fn list(env: &Env, query: &str) -> Vec<Value> {
    let (st, v) = call(&env.app, "GET", &format!("/api/tickets?project={}{query}", env.pid), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

fn by_key<'a>(items: &'a [Value], key: &str) -> &'a Value {
    items.iter().find(|t| t["key"] == key).unwrap()
}

fn keys(items: &[Value]) -> Vec<String> {
    let mut k: Vec<String> = items.iter().map(|t| s(&t["key"]).to_string()).collect();
    k.sort();
    k
}

#[tokio::test]
async fn areas_make_planned_overlap_and_start_checks_them() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let (st, v) = call(&env.app, "GET", &format!("/api/projects/{}/areas", env.pid), None).await;
    assert_eq!((st, v), (StatusCode::OK, json!({"areas": [], "error": null})));
    configure_areas(&env);
    let (_, v) = call(&env.app, "GET", &format!("/api/projects/{}/areas", env.pid), None).await;
    assert_eq!(v["areas"][0], json!({"name": "migrations", "paths": ["db/**"], "exclusive": true}));
    assert_eq!(v["areas"][1]["exclusive"], false);

    let a = ticket(&env, json!({"title": "A", "areas": ["migrations", "web"]})).await;
    let b = ticket(&env, json!({"title": "B", "areas": ["migrations"], "stage": "ready"})).await;
    let c = ticket(&env, json!({"title": "C", "areas": ["web"]})).await;
    let d = ticket(&env, json!({"title": "D", "areas": ["gone"]})).await;
    assert!(list(&env, "").await.iter().all(|t| t["overlap"] == "none"), "nothing is active");
    started(&env, &a, json!({"agent": "claude"})).await;

    let items = list(&env, "").await;
    assert_eq!(by_key(&items, "PDT-2")["overlap"], "conflict");
    assert_eq!(by_key(&items, "PDT-2")["overlapWith"], json!(["PDT-1"]));
    assert_eq!(by_key(&items, "PDT-3")["overlap"], "overlap");
    assert_eq!(by_key(&items, "PDT-4")["overlap"], "none", "areas not in the config are ignored");
    assert_eq!(by_key(&items, "PDT-1")["overlap"], "none", "not-started tickets hold no areas");
    assert_eq!(keys(&list(&env, "&conflicts=true").await), ["PDT-2", "PDT-3"]);
    assert_eq!(keys(&list(&env, "&conflicts=false").await), ["PDT-1", "PDT-4"]);
    let t = get_ticket(&env, &b).await;
    assert_eq!(
        t["overlaps"],
        json!([{"ticketId": a, "key": "PDT-1", "title": "A", "stage": "in_progress", "level": "conflict",
                "areas": ["migrations"], "files": [], "filesTotal": 0}])
    );

    // Start refuses with the reasons; Wait instead.
    let (st, v) = start(&env, &b, json!({"agent": "claude"})).await;
    assert_eq!((st, s(&v["code"])), (StatusCode::CONFLICT, "start_blocked"), "{v}");
    assert_eq!(s(&v["message"]), "PDT-2 conflicts with PDT-1 (migrations)");
    let (st, t) = call(&env.app, "PATCH", &format!("/api/tickets/{b}"), Some(json!({"waiting": true}))).await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!((t["waiting"].as_bool(), t["clearToStart"].as_bool()), (Some(true), Some(false)));
    assert_eq!(event_kinds(&env, &b).await.last().map(String::as_str), Some("waiting"));

    // Force starts anyway and records it; the two active tickets now overlap each other.
    let r = started(&env, &c, json!({"agent": "claude", "mode": "force"})).await;
    assert_eq!(r["ticket"]["overlapWith"], json!(["PDT-1"]));
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{c}/events"), None).await;
    let ev = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "started").unwrap().clone();
    assert_eq!(ev["data"]["forced"], true);
    assert_eq!(ev["data"]["overlaps"], json!([{"key": "PDT-1", "level": "overlap", "areas": ["web"]}]));
    assert_eq!(get_ticket(&env, &a).await["overlapWith"], json!(["PDT-3"]));
    let (_, v) = start(&env, &d, json!({"agent": "claude"})).await;
    assert!(v["session"].is_object(), "no findings: starts at once {v}");

    // When the conflicting attempt ends, the waiting ticket is clear to start.
    let attempt = get_ticket(&env, &a).await["attempt"]["id"].as_str().unwrap().to_string();
    call(&env.app, "POST", &format!("/api/attempts/{attempt}/abandon"), None).await;
    let t = get_ticket(&env, &b).await;
    assert_eq!((t["overlap"].as_str(), t["clearToStart"].as_bool()), (Some("none"), Some(true)));
    // Starting it clears waiting.
    let r = started(&env, &b, json!({"agent": "claude"})).await;
    assert_eq!((r["ticket"]["waiting"].as_bool(), r["ticket"]["clearToStart"].as_bool()), (Some(false), Some(false)));
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn files_changed_by_two_attempts_overlap_and_are_recorded_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    configure_areas(&env);
    let a = ticket(&env, json!({"title": "A"})).await;
    let b = ticket(&env, json!({"title": "B"})).await;
    started(&env, &a, json!({"agent": "claude"})).await;
    started(&env, &b, json!({"agent": "claude"})).await;
    let wa = env.repo.join(".worktrees").join("pdt-PDT-1-a");
    let wb = env.repo.join(".worktrees").join("pdt-PDT-2-b");
    pdt_api::refresh_overlaps(&env.state).await;
    assert_eq!(get_ticket(&env, &a).await["overlap"], "none");

    // A commits a.txt; B edits it without committing: an overlap.
    work_in(&env, "pdt-PDT-1-a", "a.txt");
    std::fs::write(wb.join("a.txt"), "mine\n").unwrap();
    pdt_api::refresh_overlaps(&env.state).await;
    let t = get_ticket(&env, &a).await;
    assert_eq!(t["overlap"], "overlap");
    assert_eq!((t["overlaps"][0]["key"].as_str(), &t["overlaps"][0]["files"]), (Some("PDT-2"), &json!(["a.txt"])));
    assert_eq!(get_ticket(&env, &b).await["overlapWith"], json!(["PDT-1"]));
    let overlap_events = |id: String| {
        let app = env.app.clone();
        async move {
            let (_, v) = call(&app, "GET", &format!("/api/tickets/{id}/events"), None).await;
            v["items"].as_array().unwrap().iter().filter(|e| e["kind"] == "overlap").cloned().collect::<Vec<_>>()
        }
    };
    let ev = overlap_events(a.clone()).await;
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["data"], json!({"ticketId": b, "key": "PDT-2", "files": ["a.txt"], "level": "overlap"}));
    pdt_api::refresh_overlaps(&env.state).await;
    assert_eq!(overlap_events(a.clone()).await.len(), 1, "the same files are not recorded again");
    assert_eq!(overlap_events(b.clone()).await.len(), 1);

    // Both touch a file in the exclusive area (one untracked): a conflict, recorded as a new file set.
    std::fs::create_dir_all(wa.join("db")).unwrap();
    std::fs::create_dir_all(wb.join("db")).unwrap();
    std::fs::write(wa.join("db/0002.sql"), "a\n").unwrap();
    std::fs::write(wb.join("db/0002.sql"), "b\n").unwrap();
    pdt_api::refresh_overlaps(&env.state).await;
    let t = get_ticket(&env, &b).await;
    assert_eq!(t["overlap"], "conflict");
    assert_eq!(t["overlaps"][0]["files"], json!(["a.txt", "db/0002.sql"]));
    assert_eq!(t["overlaps"][0]["filesTotal"], 2);
    assert_eq!(overlap_events(b.clone()).await.len(), 2);
    assert_eq!(keys(&list(&env, "&conflicts=true").await), ["PDT-1", "PDT-2"]);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn repeated_checks_and_commits_with_the_same_overlap_log_one_event_and_a_clear_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let a = ticket(&env, json!({"title": "A"})).await;
    let b = ticket(&env, json!({"title": "B"})).await;
    started(&env, &a, json!({"agent": "claude"})).await;
    started(&env, &b, json!({"agent": "claude"})).await;
    let wb = env.repo.join(".worktrees").join("pdt-PDT-2-b");
    let logged = |kind: &'static str| {
        let app = env.app.clone();
        async move {
            let (_, v) = call(&app, "GET", &format!("/api/events?type={kind}"), None).await;
            v.as_array().unwrap().iter().map(|e| e["ticketId"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        }
    };
    let found = || logged("ticket.overlap_found");
    let check = || pdt_api::refresh_overlaps(&env.state);

    work_in(&env, "pdt-PDT-1-a", "a.txt");
    work_in(&env, "pdt-PDT-1-a", "b.txt");
    std::fs::write(wb.join("a.txt"), "mine\n").unwrap();
    std::fs::write(wb.join("b.txt"), "mine\n").unwrap();
    check().await;
    assert_eq!(found().await.len(), 2, "one on each ticket");

    // Polls, a commit (a push changes nothing that is shared), fewer files and a file coming back: no news.
    check().await;
    git(&wb, &["add", "-A"]);
    git(&wb, &["commit", "-q", "-m", "b's work"]);
    check().await;
    git(&wb, &["rm", "-q", "b.txt"]);
    check().await;
    assert_eq!(get_ticket(&env, &a).await["overlaps"][0]["files"], json!(["a.txt"]));
    git(&wb, &["checkout", "-q", "HEAD", "--", "b.txt"]);
    check().await;
    check().await;
    assert_eq!(found().await.len(), 2, "the same overlap is logged once");

    // A newly shared file is news, on both tickets.
    work_in(&env, "pdt-PDT-1-a", "c.txt");
    std::fs::write(wb.join("c.txt"), "mine\n").unwrap();
    check().await;
    check().await;
    assert_eq!(found().await.len(), 4);

    // B puts every shared file back: the overlap clears, once.
    git(&wb, &["reset", "-q", "--hard", "main"]);
    git(&wb, &["clean", "-qfd"]);
    check().await;
    check().await;
    assert_eq!(get_ticket(&env, &a).await["overlap"], "none");
    let mut cleared = logged("ticket.overlap_cleared").await;
    cleared.sort();
    let mut both = vec![a.clone(), b.clone()];
    both.sort();
    assert_eq!(cleared, both);
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{a}/events"), None).await;
    let clears: Vec<&Value> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "overlap" && e["data"]["cleared"] == true)
        .collect();
    assert_eq!(clears.len(), 1);
    assert_eq!(
        clears[0]["data"],
        json!({"ticketId": b, "key": "PDT-2", "files": [], "level": "none", "cleared": true})
    );

    // Overlapping again is news again.
    std::fs::write(wb.join("a.txt"), "again\n").unwrap();
    check().await;
    assert_eq!(found().await.len(), 6);
    env.state.ptys.shutdown().await;
}
