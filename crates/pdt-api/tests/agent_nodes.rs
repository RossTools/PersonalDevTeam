//! PDT-498: `agent.nudge`, `agent.restart` and `agent.inbox` on their own, each wired straight to a trigger in a
//! customized "When an agent is stuck" (stuck_agents.rs covers the built-in's default chain). `fake-agent` echoes
//! what it is told and never submits an outcome unless a message says `mcp: submit_outcome …`.

use crate::common;

use axum::http::StatusCode;
use common::wait::wait_until;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.claude_program = FAKE.to_string();
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
    Env { app, state, project: p["id"].as_str().unwrap().into(), _tmp: tmp }
}

async fn ticket(env: &Env, title: &str) -> String {
    let (s, t) =
        call_as_ui(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn create_workflow(env: &Env, doc: Value) -> String {
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn run_now(env: &Env, wf: &str, input: Value) -> String {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r["runId"].as_str().unwrap().to_string()
}

async fn run(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
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

/// What the agent of a session said (`fake-agent` echoes each message it gets).
async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// The project's sessions, oldest first.
async fn sessions(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

/// A review step whose agent only echoes its prompt.
fn review_doc(prompt: &str) -> Value {
    json!({
        "name": "Review",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "review", "kind": "agent", "version": 1, "config": {"persona": "Reviewer", "prompt": prompt,
                "outcomes": [{"id": "approved", "description": "it can merge"}, {"id": "changes_requested"}]}},
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
    })
}

/// "When an agent is stuck" replaced by: a stalled step goes straight to `act` (an `agent.*` kind with `config`),
/// each of its `ports` to the end.
async fn on_stalled(env: &Env, kind: &str, config: Value, ports: &[&str]) -> String {
    let (_, v) = call(&env.app, "GET", "/api/stuck-agents", None).await;
    let id = v["workflowId"].as_str().unwrap().to_string();
    let mut doc = json!({
        "name": "When an agent is stuck",
        "nodes": [
            {"id": "stalled", "kind": "agent.stalled", "version": 1},
            {"id": "act", "kind": kind, "version": 1, "config": config},
            {"id": "done", "kind": "end", "version": 1, "config": {"quiet": true}},
        ],
    });
    let mut edges = vec![json!({"from": "stalled", "to": "act"})];
    edges.extend(ports.iter().map(|p| json!({"from": "act", "port": p, "to": "done"})));
    doc["edges"] = json!(edges);
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    id
}

/// The `act` step of the stuck workflow's first run that has one ended.
async fn act_step(env: &Env, stuck_wf: &str) -> Value {
    tick_until(env, "the act step", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={stuck_wf}"), None).await;
        for r in v.as_array().cloned().unwrap_or_default() {
            let r = run(env, r["id"].as_str().unwrap()).await;
            if let Some(s) =
                r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "act" && s["port"].is_string())
            {
                return Some(s.clone());
            }
        }
        None
    })
    .await
}

/// `agent.nudge` sends the stalled step's session the configured message with the step's outcomes; the step works
/// again (no Inbox item) and the node ends by `nudged`.
#[tokio::test]
async fn nudge_sends_the_message_with_the_outcomes() {
    let env = setup().await;
    let stuck = on_stalled(
        &env,
        "agent.nudge",
        json!({"step": "{{trigger.stepId}}", "message": "Pick an outcome, please."}),
        &["nudged", "no_session", "skipped"],
    )
    .await;
    let t = ticket(&env, "Nudged").await;
    let wf = create_workflow(&env, review_doc("Have a look.")).await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let step = act_step(&env, &stuck).await;
    assert_eq!(step["port"], "nudged", "{step:#}");
    assert_eq!(step["output"]["nudges"], 1, "{step:#}");
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    let said = tick_until(&env, "the nudge", || async {
        let said = agent_says(&env, &session).await;
        said.into_iter().find(|m| m.contains("Pick an outcome, please.")).map(|m| json!(m))
    })
    .await;
    assert!(said.as_str().unwrap().contains("`approved` (it can merge)"), "the step's outcomes: {said}");
    assert_eq!(sessions(&env).await.len(), 1, "no fresh session");
}

/// `agent.restart` alone: the stalled step goes on in a fresh session that gets its task again; the earlier one stops.
#[tokio::test]
async fn restart_moves_the_step_to_a_fresh_session() {
    let env = setup().await;
    let stuck =
        on_stalled(&env, "agent.restart", json!({"step": "{{trigger.stepId}}"}), &["restarted", "skipped"]).await;
    let t = ticket(&env, "Restarted").await;
    let wf = create_workflow(&env, review_doc("Read a.txt.")).await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let step = act_step(&env, &stuck).await;
    assert_eq!(step["port"], "restarted", "{step:#}");
    let fresh = step["output"]["sessionId"].as_str().unwrap().to_string();
    let all = sessions(&env).await;
    assert_eq!(all.len(), 2, "{all:#?}");
    let first = all.iter().find(|s| s["id"] != fresh.as_str()).unwrap()["id"].as_str().unwrap().to_string();
    let said = tick_until(&env, "the fresh session's task", || async {
        let said = agent_says(&env, &fresh).await;
        said.into_iter().find(|m| m.contains("This is a fresh session")).map(|m| json!(m))
    })
    .await;
    assert!(said.as_str().unwrap().contains("Read a.txt."), "{said}");
    tick_until(&env, "the earlier session stopped", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{first}"), None).await;
        (s["status"] != "running").then_some(json!(null))
    })
    .await;
}

/// `agent.inbox` alone: the stalled step goes to the Inbox at once, without PDT's own nudge first.
#[tokio::test]
async fn inbox_asks_without_nudging() {
    let env = setup().await;
    let stuck = on_stalled(&env, "agent.inbox", json!({"step": "{{trigger.stepId}}"}), &["asked", "skipped"]).await;
    let t = ticket(&env, "Asked").await;
    let wf = create_workflow(&env, review_doc("Look.")).await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let step = act_step(&env, &stuck).await;
    assert_eq!(step["port"], "asked", "{step:#}");
    let items = inbox(&env).await;
    let stalled = items.iter().find(|i| i["kind"] == "stalled").unwrap_or_else(|| panic!("{items:#?}"));
    let session = stalled["sessionId"].as_str().unwrap();
    let said = agent_says(&env, session).await;
    assert!(!said.iter().any(|m| m.contains("[PDT]")), "not nudged: {said:#?}");
}

/// On a step that isn't stuck (or doesn't exist), each node ends by `skipped` and does nothing.
#[tokio::test]
async fn each_node_skips_a_step_that_is_not_stuck() {
    let env = setup().await;
    let doc = json!({
        "name": "Not stuck",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "nudge", "kind": "agent.nudge", "version": 1,
             "config": {"step": "{{trigger.stepId}}", "message": "Hello?"}},
            {"id": "restart", "kind": "agent.restart", "version": 1, "config": {"step": "{{trigger.stepId}}"}},
            {"id": "ask", "kind": "agent.inbox", "version": 1, "config": {"step": "{{trigger.stepId}}"}},
            {"id": "done", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "go", "to": "nudge"},
            {"from": "nudge", "port": "skipped", "to": "restart"},
            {"from": "restart", "port": "skipped", "to": "ask"},
            {"from": "ask", "port": "skipped", "to": "done"},
        ],
    });
    let wf = create_workflow(&env, doc).await;
    let id = run_now(&env, &wf, json!({"stepId": "no-such-step"})).await;
    let r = tick_until(&env, "the run end", || async {
        let r = run(&env, &id).await;
        (!matches!(r["status"].as_str(), Some("running" | "waiting"))).then_some(r)
    })
    .await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    for node in ["nudge", "restart", "ask"] {
        let s = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == node).unwrap();
        assert_eq!(s["port"], "skipped", "{node}: {r:#}");
    }
    assert!(inbox(&env).await.is_empty());
}

/// A nudge with no message, or any of them with no step, doesn't pass the check.
#[tokio::test]
async fn the_nodes_need_a_step_and_a_message() {
    let env = setup().await;
    for (kind, config) in [
        ("agent.nudge", json!({"step": "{{trigger.stepId}}", "message": "  "})),
        ("agent.nudge", json!({"step": "", "message": "Hi"})),
        ("agent.restart", json!({"step": " "})),
        ("agent.inbox", json!({"step": ""})),
    ] {
        let doc = json!({
            "name": "Bad",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "act", "kind": kind, "version": 1, "config": config},
            ],
            "edges": [{"from": "go", "to": "act"}],
        });
        let (s, c) = call(&env.app, "POST", "/api/workflows/check", Some(json!({"doc": doc}))).await;
        assert_eq!(s, StatusCode::OK, "{c}");
        let issues = c["issues"].as_array().unwrap();
        assert!(issues.iter().any(|i| i["severity"] == "error" && i["node"] == "act"), "{kind} {config}: {c:#}");
    }
}
