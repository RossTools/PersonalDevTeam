//! ACP agents the user adds (PDT-234) end to end, with `fake-agent --plain acp` as the agent: added in Settings →
//! Agents, found by the checks, picked for a persona, running a workflow agent step (prompt, permission request,
//! outcome), and a conversation of its own (cancel, resume).

mod common;

use axum::http::StatusCode;
use common::{call, git};
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

/// A project with a repo, served on a free local port (the agent's pdt-mcp calls come over TCP), and the fake added
/// as the ACP agent `acp:fake`.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.login_path = None;
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

async fn add_fake(env: &Env) -> Value {
    let agents = json!([{"id": "fake", "label": "Fake ACP", "command": FAKE, "args": ["--plain", "acp"],
                         "env": [{"name": "FAKE_ACP", "value": "1"}]}]);
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"acpAgents": agents}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

/// Ticks the workflow engine until `f` holds for the session's events (up to 20 s).
async fn wait_for(env: &Env, id: &str, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let all = events(env, id).await;
        if f(&all) {
            return all;
        }
        assert!(Instant::now() < deadline, "never saw {what}: {all:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn said(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

fn ended(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        .map(|e| e["event"]["status"].as_str().unwrap().to_string())
        .collect()
}

fn open_request(all: &[Value]) -> Option<Value> {
    all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open").map(|e| e["event"].clone())
}

#[tokio::test]
async fn an_acp_agent_is_added_checked_and_refused_where_it_cannot_go() {
    let env = setup().await;
    // Unknown until added: no session, persona or default agent names it.
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "acp:fake", "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body.clone())).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_agent")), "{v}");
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"defaultAgent": "acp:fake"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let (s, v) = call(&env.app, "GET", "/api/agents/acp:fake/capabilities", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");

    let v = add_fake(&env).await;
    let (s, v2) = call(&env.app, "GET", "/api/agents/acp:fake/capabilities", None).await;
    assert_eq!((s, v2["permissionModes"].as_array().map(Vec::len)), (StatusCode::OK, Some(4)), "{v2}");
    // The built-in profiles (OhMyPi, Grok), then the user's.
    let agents = v["acpAgents"].as_array().unwrap();
    let builtins: Vec<&str> =
        agents.iter().filter(|a| a["builtin"] == true).filter_map(|a| a["agent"].as_str()).collect();
    assert_eq!(builtins, ["acp:omp", "acp:grok"], "{v}");
    let agents: Vec<&Value> = agents.iter().filter(|a| a["builtin"] == false).collect();
    assert_eq!(agents.len(), 1, "{v}");
    assert_eq!((agents[0]["agent"].as_str(), agents[0]["label"].as_str()), (Some("acp:fake"), Some("Fake ACP")));
    assert!(agents[0]["found"].as_str().is_some(), "{v}");
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"defaultAgent": "acp:fake"}))).await;
    assert_eq!((s, v["settings"]["defaultAgent"].as_str()), (StatusCode::OK, Some("acp:fake")), "{v}");
    let (_, checks) = call(&env.app, "POST", "/api/host/checks/refresh", None).await;
    assert_eq!(checks["acp"]["acp:fake"]["found"], true, "{checks}");

    // Conversations only: no terminal, no switching to one, no Make ticket.
    let mut term = body.clone();
    term["mode"] = json!("terminal");
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(term)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/mode"), Some(json!({"mode": "terminal"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/ticket-draft"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("draft_unsupported")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Fake ACP"), "{v}");

    // Removing it puts the default back on Claude.
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"acpAgents": []}))).await;
    assert_eq!((s, v["settings"]["defaultAgent"].as_str()), (StatusCode::OK, Some("claude")), "{v}");
}

#[tokio::test]
async fn a_persona_on_an_acp_agent_runs_an_agent_step_through_a_permission_request() {
    let env = setup().await;
    let persona = json!({"name": "Fake reviewer", "role": "reviewer", "agent": "acp:fake",
                         "instructions": "Review it.", "tools": ["get_ticket", "submit_outcome"]});
    let (s, v) = call(&env.app, "POST", "/api/personas", Some(persona.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an agent the host doesn't have: {v}");
    add_fake(&env).await;
    let (s, v) = call(&env.app, "POST", "/api/personas", Some(persona)).await;
    assert_eq!((s, v["agent"].as_str()), (StatusCode::CREATED, Some("acp:fake")), "{v}");

    // The project asks before tools, so the agent's pdt-mcp call is a permission request PDT passes on.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/settings", env.project),
        Some(json!({"agentCredentials": null, "runOverrides": null, "agentPermissionMode": "ask"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Review"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let ticket = t["id"].as_str().unwrap().to_string();
    let doc = json!({
        "name": "Review",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "review", "kind": "agent", "version": 1, "config": {"persona": "Fake reviewer",
                "prompt": r#"mcp: submit_outcome {"outcome": "approved", "summary": "Fine."}"#,
                "outcomes": [{"id": "approved"}, {"id": "changes_requested"}]}},
            {"id": "ok", "kind": "end", "version": 1, "config": {"output": {"result": "{{steps.review.outcome}}"}}},
            {"id": "fix", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "go", "to": "review"},
            {"from": "review", "port": "approved", "to": "ok"},
            {"from": "review", "port": "changes_requested", "to": "fix"},
        ],
    });
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let (s, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", w["id"].as_str().unwrap()),
        Some(json!({"input": {"ticketId": ticket}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let run_id = r["runId"].as_str().unwrap().to_string();

    // The step's session is a conversation with the ACP agent.
    let deadline = Instant::now() + Duration::from_secs(20);
    let session = loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{run_id}"), None).await;
        let step = r["steps"].as_array().and_then(|a| a.iter().find(|s| s["nodeId"] == "review").cloned());
        if let Some(id) = step.as_ref().and_then(|s| s["wait"]["agent"]["session"].as_str()) {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "the step never started a session: {r:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!((s["agent"].as_str(), s["mode"].as_str()), (Some("acp:fake"), Some("conversation")), "{s}");
    assert_eq!(s["permissionMode"], "ask");

    let all = wait_for(&env, &session, "the permission request", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!(req["tool"], "other", "named by its ACP kind: {req}");
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{session}/requests/{}", req["id"].as_str().unwrap()),
        Some(json!({"answer": {"type": "approve"}})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");

    let deadline = Instant::now() + Duration::from_secs(20);
    let r = loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{run_id}"), None).await;
        if !matches!(r["status"].as_str(), Some("running" | "waiting")) {
            break r;
        }
        assert!(Instant::now() < deadline, "the run never ended: {r:#}\n{:#?}", said(&events(&env, &session).await));
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!((r["status"].as_str(), r["output"]["result"].as_str()), (Some("succeeded"), Some("approved")), "{r:#}");
    // The outcome can end the run before the agent's message about it is recorded.
    wait_for(&env, &session, "the agent saying it submitted", |a| {
        said(a).iter().any(|m| m.starts_with("mcp submit_outcome:"))
    })
    .await;
}

#[tokio::test]
async fn an_acp_conversation_is_cancelled_stopped_and_resumed() {
    let env = setup().await;
    add_fake(&env).await;
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "acp:fake", "cols": 80, "rows": 24,
                      "initialPrompt": "who"});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["mode"].as_str(), v["permissionMode"].as_str()), (Some("conversation"), Some("ask")));
    let id = v["id"].as_str().unwrap().to_string();
    let all = wait_for(&env, &id, "the first turn", |a| ended(a).len() == 1).await;
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    let agent_session = s["externalSessionId"].as_str().expect("the agent's session id is kept").to_string();
    // PDT's instructions came in front of the first prompt, which PDT logs as the user wrote it.
    assert_eq!(
        said(&all),
        [format!("session={agent_session} loaded=false mode=accept-edits model=fake-1 instructions=true")]
    );
    let user: Vec<&str> = all
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .filter_map(|e| e["event"]["text"].as_str())
        .collect();
    assert_eq!(user, ["who"]);

    // Cancel: a slow turn interrupted ends cancelled.
    let send = |text: &str| json!({"text": text, "delivery": "next"});
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(send("slow"))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    wait_for(&env, &id, "the slow turn", |a| a.iter().filter(|e| e["event"]["kind"] == "turn").count() >= 3).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let all = wait_for(&env, &id, "the cancelled turn", |a| ended(a).len() == 2).await;
    assert_eq!(ended(&all)[1], "interrupted", "{all:#?}");

    // Resume: stopped, the next message starts the agent again on its saved session.
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(send("who"))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let all = wait_for(&env, &id, "the resumed turn", |a| ended(a).len() == 3).await;
    let last = said(&all).last().cloned().unwrap();
    assert!(last.starts_with(&format!("session={agent_session} loaded=true")), "{last}");
}
