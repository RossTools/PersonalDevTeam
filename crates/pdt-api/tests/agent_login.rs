//! PDT-604: an agent that isn't logged in. `fake-agent` copied into the test's folder, with `fake-agent-logged-out`
//! beside it, answers every message like Claude with an expired login and says so to `auth status`; removing the file
//! logs it in. Agent steps aren't nudged: they wait, one Inbox item counts them, new steps wait before starting, and
//! once the agent is logged in they all go on and the item closes.

use crate::common;
use common::conversation;

use axum::http::StatusCode;
use common::wait::wait_until;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    /// Logged out while it exists.
    marker: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    // The test's own copy of the stand-in, so its logged-out file is this test's alone.
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join(format!("fake-agent{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-agent"), &fake).unwrap();
    let marker = bin.join("fake-agent-logged-out");
    std::fs::write(&marker, "").unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.claude_program = fake.to_string_lossy().into_owned();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), marker, _tmp: tmp }
}

async fn ticket(env: &Env, title: &str) -> String {
    let (s, t) =
        call_as_ui(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// A review step whose agent hands in `approved` at once (when it is logged in).
async fn review_workflow(env: &Env) -> String {
    let prompt = r#"mcp: submit_outcome {"outcome": "approved", "summary": "Looks good."}"#;
    let doc = json!({
        "name": "Review",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "review", "kind": "agent", "version": 1, "config": {"persona": "Reviewer", "prompt": prompt,
                "outcomes": [{"id": "approved"}, {"id": "changes_requested"}]}},
            {"id": "ok", "kind": "end", "version": 1},
            {"id": "fix", "kind": "end", "version": 1},
            {"id": "gave_up", "kind": "end", "version": 1},
            {"id": "halted", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "go", "to": "review"},
            {"from": "review", "port": "approved", "to": "ok"},
            {"from": "review", "port": "changes_requested", "to": "fix"},
            {"from": "review", "port": "timeout", "to": "gave_up"},
            {"from": "review", "port": "stopped", "to": "halted"},
        ],
    });
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn run_now(env: &Env, wf: &str, ticket: &str) -> String {
    let (s, r) =
        call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {"ticketId": ticket}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r["runId"].as_str().unwrap().to_string()
}

async fn inbox(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

/// Ticks the engine until `done` holds.
async fn tick_until<F, Fut>(env: &Env, what: &str, done: F) -> Value
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    wait_until(what, async || {
        pdt_api::run_workflows(&env.state).await;
        done().await
    })
    .await
}

/// The agent-login item once it counts `n` waiting steps.
async fn login_item(env: &Env, n: u64) -> Value {
    tick_until(env, &format!("the login item with {n} steps"), || async {
        inbox(env).await.into_iter().find(|i| i["kind"] == "agent_login" && i["body"]["waiting"] == n)
    })
    .await
}

async fn sessions(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

/// What PDT sent a session (its user messages).
async fn told(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| e["event"]["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn ticket_agent(env: &Env, id: &str) -> Value {
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    t["agent"].clone()
}

#[tokio::test]
async fn steps_wait_for_the_login_and_go_on_after_it() {
    let env = setup().await;
    let (first_t, second_t) = (ticket(&env, "One").await, ticket(&env, "Two").await);
    let wf = review_workflow(&env).await;
    let first = run_now(&env, &wf, &first_t).await;
    let item = login_item(&env, 1).await;
    assert_eq!(item["title"], "Claude isn't logged in: 1 step is waiting", "{item:#}");
    assert!(item["body"]["hint"].as_str().unwrap().contains("/login"), "{item:#}");
    assert!(item["body"]["detail"].as_str().unwrap().contains("Failed to authenticate"), "{item:#}");
    assert_eq!(item["choices"][0]["id"], "check_again");
    let all = sessions(&env).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    let session = all[0]["id"].as_str().unwrap().to_string();
    // The conversation shows how to log in under the error.
    let (_, events) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    assert!(
        events.as_array().unwrap().iter().any(|e| e["event"]["kind"] == "notice"
            && e["event"]["text"].as_str().unwrap_or_default().contains("isn't logged in")),
        "{events:#}"
    );
    // A second step for the same agent waits before its session starts, under the same item.
    let second = run_now(&env, &wf, &second_t).await;
    let item = login_item(&env, 2).await;
    assert_eq!(item["title"], "Claude isn't logged in: 2 steps are waiting");
    assert_eq!(sessions(&env).await.len(), 1, "no session for a step that waits to start");
    for t in [&first_t, &second_t] {
        let a = ticket_agent(&env, t).await;
        assert_eq!((a["state"].as_str(), a["background"][0].as_str()), (Some("logged_out"), Some("Claude")), "{a:#}");
    }
    // Not nudged, stalled or listed as a session needing you.
    tokio::time::sleep(Duration::from_millis(300)).await;
    pdt_api::run_workflows(&env.state).await;
    let items = inbox(&env).await;
    let kinds: Vec<&str> = items.iter().filter_map(|i| i["kind"].as_str()).collect();
    assert_eq!(kinds, ["agent_login"], "{items:#?}");
    assert_eq!(told(&env, &session).await.len(), 1, "no nudge");
    // Check again while still logged out says so.
    let id = item["id"].as_str().unwrap().to_string();
    let (s, e) =
        call(&env.app, "POST", &format!("/api/inbox/{id}/answer"), Some(json!({"choice": "check_again"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert!(e["message"].as_str().unwrap().contains("still isn't logged in"), "{e}");
    // Logged in: the next check takes both steps up, and they finish.
    std::fs::remove_file(&env.marker).unwrap();
    pdt_api::watch_logins(&env.state).await;
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "agent_login"), "the item closed");
    let closed = env.state.store.get_inbox_item(&id).await.unwrap().unwrap();
    assert_eq!(closed.resolution["resumed"], 2, "{closed:#?}");
    for run in [&first, &second] {
        tick_until(&env, "the run's end", || async {
            let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{run}"), None).await;
            (r["status"] == "succeeded").then_some(r)
        })
        .await;
    }
    let again = told(&env, &session).await;
    assert!(again.last().unwrap().contains("wasn't logged in"), "the task again, saying why: {again:#?}");
}

/// A started ticket whose step waits to start for the login: the Board says why, and the idle net doesn't call the
/// ticket stalled meanwhile (the agent's item has no ticket).
#[tokio::test]
async fn a_step_waiting_to_start_is_not_an_idle_ticket() {
    let env = setup().await;
    let wf = review_workflow(&env).await;
    run_now(&env, &wf, &ticket(&env, "One").await).await;
    login_item(&env, 1).await;
    let started = ticket(&env, "Started").await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{started}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    run_now(&env, &wf, &started).await;
    login_item(&env, 2).await;
    assert_eq!(ticket_agent(&env, &started).await["state"], "logged_out");
    let now = 1_000_000_000_000;
    pdt_api::check_idle_at(&env.state, now).await;
    pdt_api::check_idle_at(&env.state, now + 60 * 60_000).await;
    let (_, idle) = call(&env.app, "GET", "/api/events?type=ticket.idle", None).await;
    assert_eq!(idle.as_array().unwrap().len(), 0, "{idle:#}");
}

/// A step left waiting with no open item (held just as the item closed) goes on at the watch's next pass.
#[tokio::test]
async fn a_step_waiting_with_no_item_goes_on() {
    let env = setup().await;
    let wf = review_workflow(&env).await;
    let run = run_now(&env, &wf, &ticket(&env, "One").await).await;
    let item = login_item(&env, 1).await;
    let id = item["id"].as_str().unwrap();
    let resolution = json!({"by": "test"});
    assert!(env.state.store.end_inbox_item(id, pdt_core::inbox::InboxStatus::Closed, &resolution, 1).await.unwrap());
    std::fs::remove_file(&env.marker).unwrap();
    pdt_api::watch_logins(&env.state).await;
    tick_until(&env, "the run's end", || async {
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{run}"), None).await;
        (r["status"] == "succeeded").then_some(r)
    })
    .await;
}

/// A user's own conversation (no step) whose first turn fails with `text`.
async fn failed_turn(env: &Env, text: &str) -> String {
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24,
                      "initialPrompt": format!("fail: {text}")});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    conversation::wait_turns(&env.app, &id, 1).await;
    id
}

fn login_notices(all: &[Value]) -> usize {
    all.iter()
        .filter(|e| {
            e["event"]["kind"] == "notice"
                && e["event"]["text"].as_str().unwrap_or_default().contains("isn't logged in")
        })
        .count()
}

/// PDT-714: words that only might mean a refused login ("failed to authenticate", from an MCP server here) while the
/// agent's login check says it is logged in: the turn's error stands, the conversation needs you, no login hint.
#[tokio::test]
async fn a_weak_login_phrase_the_check_rejects_is_an_error() {
    let env = setup().await;
    std::fs::remove_file(&env.marker).unwrap();
    let id = failed_turn(&env, "The GitHub MCP server failed to authenticate.").await;
    conversation::wait_session(&env.app, &id, "activity", "needs_you").await;
    // Another turn after it: the check that rejected it has long finished.
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(json!({"text": "hi"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let all = conversation::wait_turns(&env.app, &id, 2).await;
    assert_eq!(login_notices(&all), 0, "{all:#?}");
    assert!(all.iter().any(|e| e["event"]["kind"] == "error"), "{all:#?}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "agent_login"), "no login item");
}

/// The same words while the login check says logged out: how to log in shows under the error, and the conversation
/// doesn't need you for it.
#[tokio::test]
async fn a_weak_login_phrase_the_check_confirms_shows_how_to_log_in() {
    let env = setup().await;
    let id = failed_turn(&env, "Not logged in · Please run /login").await;
    conversation::wait_events(&env.app, &id, "the login notice", |all| login_notices(all) == 1).await;
    conversation::wait_session(&env.app, &id, "activity", "idle").await;
}
