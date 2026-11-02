//! PDT-367: "When an agent is stuck", end to end with `fake-agent` (it echoes what it is told and never submits an
//! outcome unless a message says `mcp: submit_outcome …`). Its default escalation, its inputs from Settings, a
//! customized workflow and the fallback when it ends without acting. With it off (PDT's own handling) see
//! agent_steps.rs.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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

/// Ticks the engine until `done` holds (up to 30 s).
async fn tick_until<F, Fut>(env: &Env, what: &str, done: F) -> Value
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        pdt_api::run_workflows(&env.state).await;
        if let Some(v) = done().await {
            return v;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn item(env: &Env, kind: &str) -> Value {
    tick_until(env, kind, || async { inbox(env).await.into_iter().find(|i| i["kind"] == kind) }).await
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

/// A review step whose agent only echoes `prompt`, with `extra` in its config.
fn review_doc(prompt: &str, extra: Value) -> Value {
    let mut cfg = json!({"persona": "Reviewer", "prompt": prompt,
        "outcomes": [{"id": "approved", "description": "it can merge"}, {"id": "changes_requested"}]});
    for (k, v) in extra.as_object().unwrap() {
        cfg[k] = v.clone();
    }
    json!({
        "name": "Review",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "review", "kind": "agent", "version": 1, "config": cfg},
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

async fn stuck_view(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/stuck-agents", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// The runs of "When an agent is stuck".
async fn stuck_runs(env: &Env) -> Vec<Value> {
    let id = stuck_view(env).await["workflowId"].as_str().unwrap().to_string();
    let (_, v) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={id}"), None).await;
    v.as_array().cloned().unwrap_or_default()
}

/// The default: nudged with the first message, nudged again quoting the goal and the last message, restarted in a
/// fresh session that carries the task and the earlier session's last words, then the Inbox.
#[tokio::test]
async fn a_stalled_agent_is_nudged_twice_restarted_then_asked() {
    let env = setup().await;
    let v = stuck_view(&env).await;
    assert_eq!((v["enabled"].as_bool(), v["customized"].as_bool()), (Some(true), Some(false)), "{v}");
    assert_eq!((v["values"]["nudges"].as_i64(), v["values"]["restart"].as_bool()), (Some(2), Some(true)));
    let t = ticket(&env, "Slow review").await;
    let wf = create_workflow(&env, review_doc("Have a look at a.txt.", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let stalled = item(&env, "stalled").await;
    let all = sessions(&env).await;
    assert_eq!(all.len(), 2, "a fresh session: {all:#?}");
    let (first, second) = (all[0]["id"].as_str().unwrap(), all[1]["id"].as_str().unwrap());
    assert_eq!(stalled["sessionId"].as_str(), Some(second), "the Inbox offers the fresh session");
    let said = tick_until(&env, "both nudges", || async {
        let said = agent_says(&env, first).await;
        (said.iter().filter(|m| m.contains("without an outcome")).count() >= 2).then(|| json!(said))
    })
    .await;
    let said: Vec<String> = serde_json::from_value(said).unwrap();
    let nudges: Vec<&String> = said.iter().filter(|m| m.contains("without an outcome")).collect();
    assert!(nudges[0].contains("You stopped without an outcome. Finish with submit_outcome"), "{said:#?}");
    assert!(nudges[0].contains("`approved` (it can merge)"), "the step's outcomes: {said:#?}");
    assert!(nudges[1].contains("You stopped again") && nudges[1].contains("Have a look at a.txt."), "{said:#?}");
    let fresh = tick_until(&env, "the fresh session's task", || async {
        let said = agent_says(&env, second).await;
        said.into_iter().find(|m| m.contains("This is a fresh session")).map(|m| json!(m))
    })
    .await;
    let fresh = fresh.as_str().unwrap();
    assert!(fresh.contains("Have a look at a.txt.\n\nTicket: "), "the task first: {fresh}");
    assert!(fresh.contains("Its last message:\n\n> "), "{fresh}");
    let (_, old) = call(&env.app, "GET", &format!("/api/sessions/{first}"), None).await;
    tick_until(&env, "the earlier session stopped", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{first}"), None).await;
        (s["status"] != "running").then_some(json!(null))
    })
    .await;
    assert_eq!(old["persona"]["name"], "Reviewer");
    // Two nudges, a restart and the Inbox: four runs of the workflow.
    let runs = stuck_runs(&env).await;
    assert_eq!(runs.len(), 4, "{runs:#?}");
    // The user takes over from the Inbox as before.
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", stalled["id"].as_str().unwrap()),
        Some(json!({"choice": "outcome:approved"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = tick_until(&env, "the run end", || async {
        let r = run(&env, &id).await;
        (r["status"] == "succeeded").then_some(r)
    })
    .await;
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(step["port"], "approved");
}

/// Inputs changed in Settings change the messages and the counts: one nudge with the user's text, no restart.
#[tokio::test]
async fn changed_inputs_change_the_messages_and_the_counts() {
    let env = setup().await;
    let (s, v) = call(
        &env.app,
        "PUT",
        "/api/stuck-agents",
        Some(json!({"values": {"nudges": 1, "firstNudge": "Wrap it up, please.", "restart": false}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["values"]["nudges"].as_i64(), v["customized"].as_bool()), (Some(1), Some(false)), "{v}");
    let (s, w) = call(&env.app, "GET", &format!("/api/workflows/{}", v["workflowId"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "nudge").unwrap()["config"]["message"],
        "Wrap it up, please."
    );
    let (s, e) = call(&env.app, "PUT", "/api/stuck-agents", Some(json!({"values": {"nope": 1}}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let t = ticket(&env, "Slow").await;
    let wf = create_workflow(&env, review_doc("Look.", json!({}))).await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let stalled = item(&env, "stalled").await;
    let all = sessions(&env).await;
    assert_eq!(all.len(), 1, "no restart: {all:#?}");
    let session = all[0]["id"].as_str().unwrap();
    assert_eq!(stalled["sessionId"].as_str(), Some(session));
    let said = agent_says(&env, session).await;
    let nudges: Vec<&String> = said.iter().filter(|m| m.contains("[PDT]")).collect();
    assert_eq!(nudges.len(), 1, "{said:#?}");
    assert!(nudges[0].contains("Wrap it up, please."), "{said:#?}");
}

/// A workflow changed beyond its inputs: Settings says so and can't change it, Reset to default brings it back. One
/// that ends without acting on a stuck step leaves it to PDT: one nudge, then the Inbox.
#[tokio::test]
async fn a_customized_workflow_that_does_nothing_falls_back_and_resets() {
    let env = setup().await;
    let v = stuck_view(&env).await;
    let id = v["workflowId"].as_str().unwrap().to_string();
    let doc = json!({
        "name": "When an agent is stuck",
        "nodes": [
            {"id": "stalled", "kind": "agent.stalled", "version": 1},
            {"id": "done", "kind": "end", "version": 1, "config": {"quiet": true}},
        ],
        "edges": [{"from": "stalled", "to": "done"}],
    });
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    let v = stuck_view(&env).await;
    assert_eq!(v["customized"], true, "{v}");
    let (s, e) = call(&env.app, "PUT", "/api/stuck-agents", Some(json!({"values": {"nudges": 3}}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("customized")), "{e}");
    let t = ticket(&env, "Quiet").await;
    let wf = create_workflow(&env, review_doc("Look.", json!({}))).await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let stalled = item(&env, "stalled").await;
    let session = stalled["sessionId"].as_str().unwrap();
    assert_eq!(sessions(&env).await.len(), 1);
    let said = agent_says(&env, session).await;
    assert_eq!(
        said.iter().filter(|m| m.contains("You ended your turn without `submit_outcome`")).count(),
        1,
        "PDT's own one nudge: {said:#?}"
    );
    assert!(stuck_runs(&env).await.len() >= 2, "the workflow ran and did nothing");
    let (s, v) = call(&env.app, "POST", "/api/stuck-agents/reset", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["customized"].as_bool(), v["values"]["nudges"].as_i64()), (Some(false), Some(2)), "{v}");
}

/// A step that times out is restarted once in a fresh session (its run goes back to it), then the Inbox asks.
#[tokio::test]
async fn a_timed_out_step_restarts_once_then_asks() {
    let env = setup().await;
    let t = ticket(&env, "Slow").await;
    let wf = create_workflow(&env, review_doc("Have a look.", json!({"maxTurns": 1}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let timed_out = item(&env, "timed_out").await;
    let r = run(&env, &id).await;
    let reviews: Vec<&Value> = r["steps"].as_array().unwrap().iter().filter(|s| s["nodeId"] == "review").collect();
    assert_eq!(reviews.len(), 2, "{r:#}");
    let (a, b) = (reviews[0]["output"]["sessionId"].as_str(), reviews[1]["output"]["sessionId"].as_str());
    assert!(a.is_some() && b.is_some() && a != b, "a fresh session: {r:#}");
    assert_eq!(timed_out["stepId"], reviews[1]["id"]);
    let said = agent_says(&env, b.unwrap()).await;
    assert!(said.iter().any(|m| m.contains("This is a fresh session") && m.contains("Have a look.")), "{said:#?}");
}

/// With "stop" at a loop limit, the step ends by `stopped` without asking.
#[tokio::test]
async fn a_loop_limit_can_stop_the_step() {
    let env = setup().await;
    let (s, v) = call(&env.app, "PUT", "/api/stuck-agents", Some(json!({"values": {"onLoopLimit": "stop"}}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut doc =
        review_doc(r#"mcp: submit_outcome {"outcome": "again", "summary": "Once more."}"#, json!({"loopLimit": 1}));
    doc["nodes"][1]["config"]["outcomes"] = json!([{"id": "approved"}, {"id": "changes_requested"}, {"id": "again"}]);
    doc["edges"].as_array_mut().unwrap().push(json!({"from": "review", "port": "again", "to": "review"}));
    let wf = create_workflow(&env, doc).await;
    let t = ticket(&env, "Loop").await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let r = tick_until(&env, "the run end", || async {
        let r = run(&env, &id).await;
        (!matches!(r["status"].as_str(), Some("running" | "waiting"))).then_some(r)
    })
    .await;
    let last = r["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!((last["port"].as_str(), last["output"]["by"].as_str()), (Some("stopped"), Some("workflow")), "{r:#}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "loop_limit"));
}
