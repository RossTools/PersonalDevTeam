//! Grok Build as the built-in ACP profile `acp:grok` (PDT-236) end to end, with `fake-agent agent --no-leader stdio`
//! (Grok's frames) installed as `grok` on the login path: found by the checks, a persona on it running a workflow
//! agent step whose pdt-mcp call runs without asking, and PDT keeping each permission mode by answering Grok's
//! permission requests (Grok has no modes of its own over ACP), then resuming it.

use crate::common;

use axum::http::StatusCode;
use common::conversation::{self, said};
use common::wait::wait_until;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    /// Where the stand-in `grok` is.
    bin: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

/// A project with a repo, served on a free local port (the agent's pdt-mcp calls come over TCP), with the fake as the
/// only `grok` on the login path.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(FAKE, bin.join(format!("grok{}", std::env::consts::EXE_SUFFIX))).unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.login_path = Some(bin.to_string_lossy().into_owned());
    state.conversations.notice_quiet = Duration::ZERO;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), bin, _tmp: tmp }
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    conversation::events(&env.app, id).await
}

/// Ticks the workflow engine until `f` holds for the session's events.
async fn wait_for(env: &Env, id: &str, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    wait_until(what, async || {
        pdt_api::run_workflows(&env.state).await;
        let all = events(env, id).await;
        if f(&all) { Ok(all) } else { Err(all) }
    })
    .await
}

fn ended(all: &[Value]) -> usize {
    all.iter().filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended").count()
}

fn open_request(all: &[Value]) -> Option<Value> {
    all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open").map(|e| e["event"].clone())
}

/// The requests the user was asked, in any state (an answered one keeps its row).
fn requests(all: &[Value]) -> usize {
    all.iter().filter(|e| e["event"]["kind"] == "request").count()
}

#[tokio::test]
async fn grok_is_built_in_and_a_persona_on_it_runs_an_agent_step_whose_pdt_tools_never_ask() {
    let env = setup().await;
    let (s, v) = call(&env.app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let grok = v["acpAgents"].as_array().unwrap().iter().find(|a| a["agent"] == "acp:grok").cloned();
    let grok = grok.unwrap_or_else(|| panic!("Grok ships with PDT: {v}"));
    assert_eq!((grok["label"].as_str(), grok["builtin"].as_bool()), (Some("Grok"), Some(true)));
    assert_eq!(grok["args"], json!(["agent", "--no-leader", "stdio"]));
    assert!(grok["found"].as_str().unwrap().starts_with(env.bin.to_str().unwrap()), "{grok}");
    let (_, checks) = call(&env.app, "POST", "/api/host/checks/refresh", None).await;
    assert_eq!(checks["acp"]["acp:grok"]["found"], true, "{checks}");

    let persona = json!({"name": "Grok coder", "role": "coder", "agent": "acp:grok",
                         "instructions": "Build it.", "tools": ["get_ticket", "submit_outcome"]});
    let (s, v) = call(&env.app, "POST", "/api/personas", Some(persona)).await;
    assert_eq!((s, v["agent"].as_str()), (StatusCode::CREATED, Some("acp:grok")), "{v}");
    // Ask mode: anything else Grok runs would ask, but PDT's own tools (`pdt__…`) don't.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/settings", env.project),
        Some(json!({"agentCredentials": null, "runOverrides": null, "agentPermissionMode": "ask"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Build"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let doc = json!({
        "name": "Code",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "code", "kind": "agent", "version": 1, "config": {"persona": "Grok coder",
                "prompt": r#"mcp: submit_outcome {"outcome": "submitted", "summary": "Built."}"#,
                "outcomes": [{"id": "submitted"}]}},
            {"id": "ok", "kind": "end", "version": 1, "config": {"output": {"result": "{{steps.code.outcome}}"}}},
        ],
        "edges": [{"from": "go", "to": "code"}, {"from": "code", "port": "submitted", "to": "ok"}],
    });
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let (s, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", w["id"].as_str().unwrap()),
        Some(json!({"input": {"ticketId": t["id"]}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let run_id = r["runId"].as_str().unwrap().to_string();

    // The step's session is a conversation with Grok; the run ends on the outcome its pdt-mcp call submits.
    let mut session = None;
    let r = wait_until("the run to end", async || {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{run_id}"), None).await;
        let step = r["steps"].as_array().and_then(|a| a.iter().find(|s| s["nodeId"] == "code").cloned());
        if let Some(id) = step.as_ref().and_then(|s| s["wait"]["agent"]["session"].as_str()) {
            session = Some(id.to_string());
        }
        if matches!(r["status"].as_str(), Some("running" | "waiting")) { Err(r) } else { Ok(r) }
    })
    .await;
    assert_eq!((r["status"].as_str(), r["output"]["result"].as_str()), (Some("succeeded"), Some("submitted")), "{r:#}");
    let session = session.expect("the step started a session");
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!((s["agent"].as_str(), s["mode"].as_str()), (Some("acp:grok"), Some("conversation")), "{s}");
    let all = wait_for(&env, &session, "the agent saying it submitted", |a| {
        said(a).iter().any(|m| m.starts_with("mcp submit_outcome:"))
    })
    .await;
    assert_eq!(requests(&all), 0, "pdt-mcp's tools ran without asking: {all:#?}");
}

#[tokio::test]
async fn pdt_keeps_each_mode_for_grok_and_resumes_its_session() {
    let env = setup().await;
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "acp:grok", "cols": 80, "rows": 24,
                      "permissionMode": "ask", "initialPrompt": "edit"});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!((s, v["mode"].as_str()), (StatusCode::CREATED, Some("conversation")), "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let answer = |req: &Value, a: Value| {
        let uri = format!("/api/sessions/{id}/requests/{}", req["id"].as_str().unwrap());
        let app = env.app.clone();
        async move {
            let (s, v) = call(&app, "POST", &uri, Some(json!({"answer": a}))).await;
            assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
        }
    };
    let send = |text: &'static str| {
        let uri = format!("/api/sessions/{id}/messages");
        let app = env.app.clone();
        async move {
            let (s, v) = call(&app, "POST", &uri, Some(json!({"text": text, "delivery": "next"}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        }
    };
    let mode = |m: &'static str| {
        let uri = format!("/api/sessions/{id}");
        let app = env.app.clone();
        async move {
            let (s, v) = call(&app, "PATCH", &uri, Some(json!({"permissionMode": m}))).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    };

    // Ask: an edit asks, and "always" is Grok's edits-for-this-session option.
    let all = wait_for(&env, &id, "the edit's request", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!((req["tool"].as_str(), req["title"].as_str()), (Some("edit"), Some("f.txt")), "{req}");
    assert_eq!(req["canAlways"], true);
    answer(&req, json!({"type": "approve", "always": true})).await;
    let all = wait_for(&env, &id, "the first turn", |a| ended(a) == 1).await;
    assert_eq!(said(&all), ["edited"]);
    let (_, caps) = call(&env.app, "GET", "/api/agents/acp:grok/capabilities", None).await;
    assert_eq!(caps["permissionModes"].as_array().map(Vec::len), Some(4), "PDT keeps all four: {caps}");

    // Accept edits: the edit runs without asking; a command still asks (no session option for it).
    mode("accept_edits").await;
    send("edit").await;
    let all = wait_for(&env, &id, "the second turn", |a| ended(a) == 2).await;
    assert_eq!(said(&all).last().unwrap(), "edited");
    send("run: touch x").await;
    let all = wait_for(&env, &id, "the command's request", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!((req["title"].as_str(), req["canAlways"].as_bool()), (Some("touch x"), Some(false)), "{req}");
    answer(&req, json!({"type": "deny"})).await;
    let all = wait_for(&env, &id, "the third turn", |a| ended(a) == 3).await;
    assert_eq!(said(&all).last().unwrap(), "not run (reject-once)");
    assert!(all.iter().any(|e| e["event"]["kind"] == "tool_call" && e["event"]["status"] == "declined"));

    // Plan: PDT refuses the edit; Bypass: PDT allows the command.
    mode("plan").await;
    send("edit").await;
    let all = wait_for(&env, &id, "the fourth turn", |a| ended(a) == 4).await;
    assert_eq!(said(&all).last().unwrap(), "not edited (reject-once)");
    mode("bypass").await;
    send("run: rm y").await;
    let all = wait_for(&env, &id, "the fifth turn", |a| ended(a) == 5).await;
    assert_eq!(said(&all).last().unwrap(), "ran it (allow-once)");
    assert_eq!(requests(&all), 2, "nothing else asked");

    // PDT's instructions came in front of the first prompt; stopped, Grok resumes its saved session.
    send("who").await;
    let all = wait_for(&env, &id, "the sixth turn", |a| ended(a) == 6).await;
    let who = said(&all).last().unwrap().clone();
    assert!(who.contains("loaded=false") && who.ends_with("instructions=true"), "{who}");
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
    let grok_session = s["externalSessionId"].as_str().unwrap().to_string();
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    send("who").await;
    let all = wait_for(&env, &id, "the resumed turn", |a| ended(a) == 7).await;
    let who = said(&all).last().unwrap().clone();
    assert!(who.starts_with(&format!("session={grok_session} loaded=true")), "{who}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
