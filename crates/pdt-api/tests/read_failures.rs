//! PDT-431: a pass of the engine that can't read the enabled workflows or the steps waiting for events leaves the new
//! events new, so they start their runs and resume their steps once reads work. PDT-524: a failed read isn't taken for
//! "not there": pdt-mcp answers 503, not a sign-in challenge, and the outbox keeps a message it can't check.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_with, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    db: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let db = tmp.path().join("pdt.db");
    let store = Store::open(&db).await.unwrap();
    let state = common::app_state(store, tmp.path());
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Proj", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().to_string(), db, _tmp: tmp }
}

/// Runs `sql` on its own connection to the store at `db`, as something outside PDT would.
async fn exec(db: &std::path::Path, sql: &str) {
    use sqlx::Connection;
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(db);
    let mut c = sqlx::SqliteConnection::connect_with(&opts).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string())).execute(&mut c).await.unwrap();
    c.close().await.unwrap();
}

async fn create(env: &Env, doc: Value) -> String {
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn ticket(env: &Env) -> String {
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    t["id"].as_str().unwrap().to_string()
}

async fn comment(env: &Env, ticket: &str) {
    let (s, c) = call(&env.app, "POST", &format!("/api/tickets/{ticket}/comments"), Some(json!({"body": "hi"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
}

async fn runs(env: &Env, wf: &str) -> Vec<Value> {
    call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await.1.as_array().unwrap().clone()
}

async fn new_comments(env: &Env) -> usize {
    let left = env.state.store.unprocessed_events(1000).await.unwrap();
    left.iter().filter(|e| e.kind == "ticket.commented").count()
}

#[tokio::test]
async fn an_event_starts_its_run_once_the_workflows_can_be_read() {
    let env = setup().await;
    let tid = ticket(&env).await;
    let wf = create(
        &env,
        json!({"name": "on comments", "nodes": [
        {"id": "t", "kind": "ticket.commented", "version": 1},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "e"}]}),
    )
    .await;
    // The workflows' documents can't be read.
    exec(&env.db, "ALTER TABLE workflow_versions RENAME TO test_hidden_versions").await;
    comment(&env, &tid).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(new_comments(&env).await, 1, "the event stays new");

    exec(&env.db, "ALTER TABLE test_hidden_versions RENAME TO workflow_versions").await;
    pdt_api::run_workflows(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "started once: {r:?}");
    assert_eq!(r[0]["status"], "succeeded");
    assert_eq!(new_comments(&env).await, 0);
}

#[tokio::test]
async fn a_waiting_step_resumes_once_the_waiting_steps_can_be_read() {
    let env = setup().await;
    let tid = ticket(&env).await;
    let wf = create(
        &env,
        json!({"name": "waits", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "w", "kind": "wait", "version": 1, "config": {"timeoutMs": 600000, "event": {"type": "ticket.commented",
            "where": [{"left": "{{event.ticketId}}", "op": "eq", "right": "{{trigger.ticket}}"}]}}},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "w"}, {"from": "w", "port": "event", "to": "e"}]}),
    )
    .await;
    let (s, r) =
        call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {"ticket": tid}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1["status"], "waiting");

    // The waiting steps can't be read.
    exec(&env.db, "ALTER TABLE workflow_steps RENAME TO test_hidden_steps").await;
    comment(&env, &tid).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(new_comments(&env).await, 1, "the event stays new");

    exec(&env.db, "ALTER TABLE test_hidden_steps RENAME TO workflow_steps").await;
    pdt_api::run_workflows(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(new_comments(&env).await, 0);
}

fn session(project: &str, cwd: &std::path::Path) -> pdt_core::session::Session {
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
    Session {
        id: "s1".into(),
        project_id: project.into(),
        worktree_id: None,
        kind: SessionKind::AgentTui,
        agent: Some(AgentKind::Claude),
        bypass_permissions: false,
        title: "t".into(),
        cwd: cwd.to_string_lossy().into(),
        status: SessionStatus::Running,
        exit_code: None,
        created_at: 1,
        ended_at: None,
        external_session_id: None,
        fresh_started_at: None,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: None,
        ticket: None,
        mode: pdt_core::conversation::SessionMode::Conversation,
        model: None,
        permission_mode: None,
        persona: None,
        background: vec![],
    }
}

#[tokio::test]
async fn pdt_mcp_answers_503_not_a_sign_in_challenge_when_it_cant_read_the_session() {
    let env = setup().await;
    env.state.store.insert_session(&session(&env.project, env._tmp.path())).await.unwrap();
    let bearer = format!("Bearer {}", pdt_api::mcp_token(&env.state, "s1").unwrap());
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let list = |auth: String| {
        let (app, rpc) = (env.app.clone(), rpc.clone());
        async move { call_with(&app, "POST", "/api/mcp", Some(rpc), &[("authorization", &auth)]).await }
    };

    exec(&env.db, "ALTER TABLE sessions RENAME TO test_hidden_sessions").await;
    let (s, v) = list(bearer.clone()).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("read_failed")), "{v}");

    exec(&env.db, "ALTER TABLE test_hidden_sessions RENAME TO sessions").await;
    let (s, v) = list(bearer.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = list("Bearer s1.forged".into()).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("unauthorized")), "{v}");
}

#[tokio::test]
async fn the_outbox_keeps_a_message_whose_step_it_cant_read() {
    let env = setup().await;
    let m = pdt_store::SessionMessage {
        id: "m1".into(),
        attempt_id: None,
        session_id: Some("s-gone".into()),
        ticket_id: None,
        dedupe: None,
        text: "hello".into(),
        timeline: Value::Null,
        run_id: None,
        step_id: Some("step-gone".into()),
        status: "waiting".into(),
        told_session: None,
        created_at: 1,
        resolved_at: None,
        by_user: false,
        pending_id: None,
        reopens: false,
    };
    env.state.store.queue_session_message(&m).await.unwrap();
    let status = || async { env.state.store.get_session_message("m1").await.unwrap().unwrap().status };

    exec(&env.db, "ALTER TABLE workflow_steps RENAME TO test_hidden_steps").await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(status().await, "waiting", "kept for the next pass");

    // Readable again: its step is gone, so it is dropped.
    exec(&env.db, "ALTER TABLE test_hidden_steps RENAME TO workflow_steps").await;
    pdt_api::run_workflows(&env.state).await;
    assert_ne!(status().await, "waiting");
}
