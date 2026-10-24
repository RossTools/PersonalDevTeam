//! Phase 7b end to end: pdt-mcp, personas, agent steps, the Inbox and Personal projects, with `fake-agent` standing
//! in for Claude (it calls pdt-mcp tools when a message says `mcp: <tool> <json>`). pdtd's API is served on a local
//! port so the agent can reach `/api/mcp`.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

/// A project with a repo, served on a free local port (the agent's pdt-mcp calls come over TCP).
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    state.conversations.background_grace = Duration::from_millis(500);
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
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title}))).await;
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

/// Ticks the engine until `done` holds (up to 20 s).
async fn tick_until<F, Fut>(env: &Env, what: &str, done: F) -> Value
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        if let Some(v) = done().await {
            return v;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn run_ended(env: &Env, id: &str) -> Value {
    tick_until(env, "the run end", || async {
        let r = run(env, id).await;
        (!matches!(r["status"].as_str(), Some("running" | "waiting"))).then_some(r)
    })
    .await
}

async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

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
            {"id": "ok", "kind": "end", "version": 1, "config": {"output": {"result": "{{steps.review.outcome}}"}}},
            {"id": "fix", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "go", "to": "review"},
            {"from": "review", "port": "approved", "to": "ok"},
            {"from": "review", "port": "changes_requested", "to": "fix"},
        ],
    })
}

#[tokio::test]
async fn personas_are_built_in_versioned_and_diffed() {
    let env = setup().await;
    let (_, list) = call(&env.app, "GET", "/api/personas", None).await;
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Coder", "PM", "QA", "Refiner", "Reviewer"]);
    let reviewer = list.as_array().unwrap().iter().find(|p| p["name"] == "Reviewer").unwrap().clone();
    assert_eq!((reviewer["permissionMode"].as_str(), reviewer["builtin"].as_str()), (None, Some("reviewer")));

    // PDT-110: the Personas page lists every pdt-mcp tool from here, with what it does and whether it asks first.
    let (s, tools) = call(&env.app, "GET", "/api/personas/tools", None).await;
    assert_eq!(s, StatusCode::OK, "{tools}");
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, pdt_core::persona::ALL_TOOLS);
    let link = tools.as_array().unwrap().iter().find(|t| t["name"] == "link_tickets").unwrap();
    assert_eq!((link["asks"].as_bool(), link["default"].as_bool()), (Some(true), Some(false)));

    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Triage", "role": "pm", "agent": "claude", "instructions": "Sort it.",
                    "tools": ["get_ticket", "submit_outcome"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let id = p["id"].as_str().unwrap().to_string();
    let (s, bad) =
        call(&env.app, "PUT", &format!("/api/personas/{id}"), Some(json!({"agent": "claude", "tools": ["rm"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    let (s, p2) = call(
        &env.app,
        "PUT",
        &format!("/api/personas/{id}"),
        Some(json!({"agent": "claude", "instructions": "Sort it well.", "permissionMode": "plan",
                    "tools": ["get_ticket", "submit_outcome"], "note": "better"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{p2}");
    assert_eq!(p2["version"], 2);
    let (_, versions) = call(&env.app, "GET", &format!("/api/personas/{id}/versions"), None).await;
    assert_eq!(versions.as_array().unwrap().len(), 2);
    let (_, d) = call(&env.app, "GET", &format!("/api/personas/{id}/diff"), None).await;
    assert_eq!(d["changed"], json!(["permissionMode", "instructions"]));
    assert!(d["unified"].as_str().unwrap().contains("+Sort it well."), "{d}");

    let (s, _) = call(&env.app, "DELETE", &format!("/api/personas/{}", reviewer["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "built-ins are archived, not deleted");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/personas/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pdt_mcp_needs_the_session_token_and_lists_its_tools() {
    let env = setup().await;
    let (s, _) =
        call(&env.app, "POST", "/api/mcp", Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let t = ticket(&env, "Tidy up").await;
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();
    let send = |text: &str| {
        let (app, session, text) = (env.app.clone(), session.clone(), text.to_string());
        async move {
            let (s, v) =
                call(&app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        }
    };
    let wait_said = |n: usize| {
        let (env, session) = (&env, session.clone());
        async move {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let said = agent_says(env, &session).await;
                if said.len() >= n {
                    return said;
                }
                assert!(Instant::now() < deadline, "the agent said only {said:?}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let first = wait_said(1).await.len();
    send("mcp-list").await;
    let said = wait_said(first + 1).await;
    assert_eq!(
        said.last().unwrap(),
        "mcp tools: get_ticket,submit_outcome,ask_user,add_ticket_comment,add_review_comment"
    );
    send("mcp: get_ticket {}").await;
    let said = wait_said(first + 2).await;
    assert!(said.last().unwrap().contains("\"title\": \"Tidy up\""), "{said:?}");
    send(r#"mcp: add_ticket_comment {"body": "Found a second bug."}"#).await;
    wait_said(first + 3).await;
    send(r#"mcp: add_review_comment {"file": "a.txt", "line": 2, "body": "Say two twice."}"#).await;
    let said = wait_said(first + 4).await;
    assert!(said.last().unwrap().starts_with("mcp add_review_comment:"), "{said:?}");
    send(r#"mcp: submit_outcome {"outcome": "done", "summary": "x"}"#).await;
    let said = wait_said(first + 5).await;
    assert!(said.last().unwrap().contains("no workflow step is waiting"), "{said:?}");
    send("mcp: catalog_search {}").await;
    let said = wait_said(first + 6).await;
    assert!(said.last().unwrap().contains("not one of this session's tools"), "{said:?}");

    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{t}/events"), None).await;
    let comment = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "comment").cloned().unwrap();
    assert_eq!((comment["actor"].as_str(), comment["body"].as_str()), (Some("agent"), Some("Found a second bug.")));
    let wt = started["attempt"]["worktreeId"].as_str().unwrap();
    let (_, comments) = call(&env.app, "GET", &format!("/api/worktrees/{wt}/comments"), None).await;
    let c = &comments.as_array().unwrap()[0];
    assert_eq!(
        (c["path"].as_str(), c["lineText"].as_str(), c["author"].as_str()),
        (Some("a.txt"), Some("two"), Some("agent"))
    );
}

#[tokio::test]
async fn an_agent_step_ends_with_the_outcome_its_persona_submits() {
    let env = setup().await;
    let t = ticket(&env, "Review me").await;
    // The project's mode is over the persona's and the host's.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/settings", env.project),
        Some(json!({"agentCredentials": null, "runOverrides": null, "agentPermissionMode": "accept_edits"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Looks right."}"#, json!({})),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    assert_eq!(r["output"]["result"], "approved");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!((step["port"].as_str(), step["output"]["summary"].as_str()), (Some("approved"), Some("Looks right.")));
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!((s["persona"]["name"].as_str(), s["mode"].as_str()), (Some("Reviewer"), Some("conversation")));
    assert_eq!(s["permissionMode"], "accept_edits", "the project's override");
    let instructions =
        std::fs::read_to_string(env.state.data_dir.join("sessions").join(&session).join("instructions.md")).unwrap();
    assert!(
        instructions.contains("# Your role: Reviewer") && instructions.contains("MCP server `pdt`"),
        "{instructions}"
    );
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{t}/events"), None).await;
    let outcome = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "outcome").cloned().unwrap();
    assert_eq!((outcome["data"]["outcome"].as_str(), outcome["actor"].as_str()), (Some("approved"), Some("agent")));
    let (_, log) = call(&env.app, "GET", &format!("/api/events?type=agent.outcome&ticket={t}"), None).await;
    assert_eq!(log.as_array().unwrap().len(), 1, "{log}");
    assert!(inbox(&env).await.is_empty());
}

#[tokio::test]
async fn questions_nudges_stalls_and_take_over_go_through_the_inbox() {
    let env = setup().await;
    let t = ticket(&env, "Unclear").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: ask_user {"questions": [{"question": "Which file?", "choices": ["a.txt"]}]}"#, json!({})),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let q =
        tick_until(&env, "the question", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "question") })
            .await;
    assert_eq!(
        (q["title"].as_str(), q["ticketTitle"].as_str(), q["personaName"].as_str()),
        (Some("Which file?"), Some("Unclear"), Some("Reviewer"))
    );
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", q["id"].as_str().unwrap()),
        Some(json!({"answers": ["a.txt"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The answer goes in, the agent echoes it (no outcome), PDT nudges once, the agent echoes again: stalled.
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    let session = stalled["sessionId"].as_str().unwrap().to_string();
    // The echo of the nudge can land in the session's events just after the stall shows (slow machines).
    let said = tick_until(&env, "the nudge's echo", || async {
        let said = agent_says(&env, &session).await;
        said.iter().any(|m| m.contains("You ended your turn without `submit_outcome`")).then(|| json!(said))
    })
    .await;
    let said: Vec<String> = serde_json::from_value(said).unwrap();
    assert!(said.iter().any(|m| m.contains("The user answered your questions") && m.contains("a.txt")), "{said:?}");
    let choices: Vec<&str> = stalled["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["nudge", "outcome:approved", "outcome:changes_requested"]);
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", stalled["id"].as_str().unwrap()),
        Some(json!({"choice": "outcome:changes_requested", "text": "Needs tests."})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = run_ended(&env, &id).await;
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!((step["port"].as_str(), step["output"]["by"].as_str()), (Some("changes_requested"), Some("user")));
    assert!(inbox(&env).await.is_empty());
}

/// PDT-95: an answer to a conversation that stopped after asking (parked, then interrupted by an update) reopens it
/// and goes in, rather than being recorded as told to nobody.
#[tokio::test]
async fn an_answer_reopens_the_stopped_conversation_that_asked() {
    let env = setup().await;
    let t = ticket(&env, "Unclear").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: ask_user {"questions": [{"question": "Which file?", "choices": ["a.txt"]}]}"#, json!({})),
    )
    .await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let q =
        tick_until(&env, "the question", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "question") })
            .await;
    let session = q["sessionId"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_ne!(v["status"], "running", "{v}");
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", q["id"].as_str().unwrap()),
        Some(json!({"answers": ["a.txt"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the answer in the conversation", || async {
        let said = agent_says(&env, &session).await;
        said.iter()
            .any(|m| m.contains("The user answered your questions") && m.contains("a.txt"))
            .then_some(json!(null))
    })
    .await;
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(v["status"], "running", "{v}");
}

/// PDT-93: removing the asking session closes its question (nothing can answer it) and stalls the step, whose item
/// offers no session to open; taking it over still ends the step.
#[tokio::test]
async fn a_removed_sessions_question_closes_and_its_step_stalls() {
    let env = setup().await;
    let t = ticket(&env, "Unclear").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: ask_user {"questions": [{"question": "Which file?", "choices": ["a.txt"]}]}"#, json!({})),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let q =
        tick_until(&env, "the question", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "question") })
            .await;
    let session = q["sessionId"].as_str().unwrap();
    let (s, v) = call(&env.app, "DELETE", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "question"), "the question closed");
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert!(stalled["sessionId"].is_null(), "{stalled}");
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", stalled["id"].as_str().unwrap()),
        Some(json!({"choice": "outcome:approved"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = run_ended(&env, &id).await;
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(step["port"].as_str(), Some("approved"));
}

#[tokio::test]
async fn gates_are_answered_from_the_inbox_and_loop_limits_ask() {
    let env = setup().await;
    let gate = create_workflow(&env, json!({
        "name": "Ship?",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Ship it?", "choices": ["ship", "hold"], "inbox": "accept"}},
            {"id": "done", "kind": "end", "version": 1},
        ],
        "edges": [{"from": "go", "to": "ask"}, {"from": "ask", "port": "ship", "to": "done"}],
    })).await;
    let id = run_now(&env, &gate, json!({})).await;
    let item =
        tick_until(&env, "the gate", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "gate") }).await;
    assert_eq!((item["group"].as_str(), item["choices"][0]["primary"].as_bool()), (Some("accept"), Some(true)));
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()),
        Some(json!({"choice": "nope"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()),
        Some(json!({"choice": "ship"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(run_ended(&env, &id).await["status"], "succeeded");

    // An agent that always asks for another pass: the second is allowed (loopLimit 1), the third asks the user.
    let mut doc =
        review_doc(r#"mcp: submit_outcome {"outcome": "again", "summary": "Once more."}"#, json!({"loopLimit": 1}));
    doc["nodes"][1]["config"]["outcomes"] = json!([{"id": "approved"}, {"id": "changes_requested"}, {"id": "again"}]);
    doc["edges"].as_array_mut().unwrap().push(json!({"from": "review", "port": "again", "to": "review"}));
    let wf = create_workflow(&env, doc).await;
    let t = ticket(&env, "Loop").await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let limit = tick_until(&env, "the loop limit", || async {
        inbox(&env).await.into_iter().find(|i| i["kind"] == "loop_limit")
    })
    .await;
    assert_eq!(limit["body"]["passes"].as_array().unwrap().len(), 2, "{limit:#}");
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", limit["id"].as_str().unwrap()),
        Some(json!({"choice": "stop"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = run_ended(&env, &id).await;
    let last = r["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(last["port"], "stopped");
}

/// A review step that times out after one turn without an outcome (its session only echoes `prompt`), and ends the
/// run by `gave_up`, as the Pipeline's timeout path does.
fn timing_out_doc(prompt: &str) -> Value {
    let mut doc = review_doc(prompt, json!({"maxTurns": 1}));
    doc["nodes"].as_array_mut().unwrap().push(json!({"id": "gave_up", "kind": "end", "version": 1}));
    doc["edges"].as_array_mut().unwrap().push(json!({"from": "review", "port": "timeout", "to": "gave_up"}));
    doc
}

/// Runs `timing_out_doc` until its review step times out: the run, the step and its Inbox item.
async fn timed_out_run(env: &Env, t: &str) -> (String, Value, Value) {
    let wf = create_workflow(env, timing_out_doc("Have a look.")).await;
    let id = run_now(env, &wf, json!({"ticketId": t})).await;
    let r = run_ended(env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(step["port"], "timeout", "{r:#}");
    assert_eq!(r["resumeFrom"], step["id"], "{r:#}");
    let item = tick_until(env, "the timed-out item", || async {
        inbox(env).await.into_iter().find(|i| i["kind"] == "timed_out")
    })
    .await;
    (id, step, item)
}

/// PDT-123: a step that times out tells the user in the Inbox, and Resume runs it again in the same run and session,
/// the user's note first.
#[tokio::test]
async fn a_timed_out_step_is_in_the_inbox_and_resumes_in_its_session() {
    let env = setup().await;
    let t = ticket(&env, "Slow").await;
    let (id, step, item) = timed_out_run(&env, &t).await;
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    assert_eq!(
        (item["group"].as_str(), item["stepId"].as_str(), item["runId"].as_str(), item["sessionId"].as_str()),
        (Some("stalled"), step["id"].as_str(), Some(id.as_str()), Some(session.as_str()))
    );
    assert_eq!(
        (item["ticketTitle"].as_str(), item["personaName"].as_str(), item["body"]["node"].as_str()),
        (Some("Slow"), Some("Reviewer"), Some("review"))
    );
    let choices: Vec<&str> = item["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["resume", "stop"]);
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()),
        Some(json!({"choice": "resume",
                    "text": r#"mcp: submit_outcome {"outcome": "approved", "summary": "Done after all."}"#})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = run_ended(&env, &id).await;
    assert_eq!((r["status"].as_str(), r["output"]["result"].as_str()), (Some("succeeded"), Some("approved")), "{r:#}");
    let reviews: Vec<&Value> = r["steps"].as_array().unwrap().iter().filter(|s| s["nodeId"] == "review").collect();
    assert_eq!(reviews.len(), 2, "{r:#}");
    assert_eq!(reviews[1]["output"]["sessionId"].as_str(), Some(session.as_str()), "the same session");
    assert!(r["resumeFrom"].is_null());
    let said = agent_says(&env, &session).await;
    assert!(said.iter().any(|m| m.starts_with("mcp submit_outcome:")), "{said:?}");
    assert!(inbox(&env).await.is_empty());
}

/// PDT-123: an outcome the agent submits after its step timed out, while the run hasn't moved on, continues the run
/// from that outcome's port; once it has moved on, there is no step to take one.
#[tokio::test]
async fn a_late_outcome_after_a_timeout_continues_the_run() {
    let env = setup().await;
    let t = ticket(&env, "Late").await;
    let (id, step, _) = timed_out_run(&env, &t).await;
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    let send = |text: &str| {
        let (app, session, text) = (env.app.clone(), session.clone(), text.to_string());
        async move {
            let (s, v) =
                call(&app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        }
    };
    send(r#"mcp: submit_outcome {"outcome": "changes_requested", "summary": "Finally done."}"#).await;
    let said = tick_until(&env, "the accepted outcome", || async {
        let said = agent_says(&env, &session).await;
        said.iter().any(|m| m.contains("hadn't moved on")).then(|| json!(said))
    })
    .await;
    assert!(!said.to_string().contains("no workflow step is waiting"), "{said}");
    let r = run_ended(&env, &id).await;
    let last = r["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(
        (last["port"].as_str(), last["output"]["summary"].as_str(), last["output"]["sessionId"].as_str()),
        (Some("changes_requested"), Some("Finally done."), Some(session.as_str())),
        "{r:#}"
    );
    assert!(r["steps"].as_array().unwrap().iter().any(|s| s["nodeId"] == "fix" && s["status"] == "succeeded"));
    assert!(inbox(&env).await.is_empty(), "the timed-out item is resolved");
    // The run has moved on: another outcome finds no step.
    send(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Again."}"#).await;
    tick_until(&env, "the refusal", || async {
        let said = agent_says(&env, &session).await;
        said.iter().any(|m| m.contains("no workflow step is waiting")).then_some(json!(null))
    })
    .await;
}

/// PDT-123: Stop leaves the step stopped; the run can still be resumed from Workflows → Runs, which sends the same
/// session on (here it times out again, and is back in the Inbox).
#[tokio::test]
async fn a_stopped_timeout_can_be_resumed_from_the_run() {
    let env = setup().await;
    let t = ticket(&env, "Stop").await;
    let (id, step, item) = timed_out_run(&env, &t).await;
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()),
        Some(json!({"choice": "stop"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(inbox(&env).await.is_empty());
    assert_eq!(run(&env, &id).await["status"], "succeeded");
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/resume"), None).await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::OK, Some("running")), "{r}");
    let again = tick_until(&env, "the second timeout", || async {
        inbox(&env).await.into_iter().find(|i| i["kind"] == "timed_out")
    })
    .await;
    assert_ne!(again["stepId"], step["id"]);
    assert_eq!(again["sessionId"].as_str(), Some(session.as_str()));
    let said = agent_says(&env, &session).await;
    assert!(said.iter().any(|m| m.contains("The user resumed your step after it stopped")), "{said:?}");
    // Resuming from the run answers the item too.
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/resume"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(inbox(&env).await.is_empty());
    let (s, v) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn a_personal_projects_tickets_run_in_a_folder() {
    let env = setup().await;
    let (s, p) =
        call(&env.app, "POST", "/api/projects", Some(json!({"name": "Briefs", "kind": "personal", "key": "BR"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!((p["kind"].as_str(), p["baseBranch"].as_str()), (Some("personal"), Some("")));
    let pid = p["id"].as_str().unwrap().to_string();
    let folder = Path::new(p["repoPath"].as_str().unwrap()).to_path_buf();
    assert!(folder.is_dir() && folder.starts_with(&env.state.data_dir));
    let (s, _) = call(&env.app, "POST", &format!("/api/projects/{pid}/worktrees"), Some(json!({"branch": "x"}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Weekly brief"}))).await;
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{}/start", t["id"].as_str().unwrap()),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    assert_eq!((started["attempt"]["branch"].as_str(), started["attempt"]["worktreeId"].as_str()), (Some(""), None));
    let cwd = started["session"]["cwd"].as_str().unwrap();
    assert_eq!(Path::new(cwd), folder.join("BR-1"));
    std::fs::write(folder.join("BR-1").join("brief.md"), "# Brief\n").unwrap();
    let (s, files) = call(&env.app, "GET", &format!("/api/projects/{pid}/files?path=BR-1"), None).await;
    assert_eq!(s, StatusCode::OK, "{files}");
    assert_eq!(files[0]["name"], "brief.md");
    pdt_api::check_merges(&env.state).await;
    let (_, t2) = call(&env.app, "GET", &format!("/api/tickets/{}", t["id"].as_str().unwrap()), None).await;
    assert_eq!(t2["stage"], "in_progress", "merge detection leaves it alone");
}

/// PDT-128: a step that hangs (an HTTP server that never answers) holds up neither other runs nor an agent's
/// `submit_outcome`, and cancelling its run doesn't wait for it.
#[tokio::test]
async fn a_hung_step_holds_up_neither_other_runs_nor_submit_outcome() {
    let env = setup().await;
    let slow = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let slow_url = format!("http://{}/hang", slow.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = vec![];
        while let Ok((sock, _)) = slow.accept().await {
            held.push(sock);
        }
    });
    let hang = create_workflow(
        &env,
        json!({
            "name": "Hang",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "get", "kind": "http.request", "version": 1, "timeoutMs": 120_000, "config": {"url": slow_url}},
            ],
            "edges": [{"from": "go", "to": "get"}],
        }),
    )
    .await;
    // As in pdtd: a tick goes on without a run still in a slow step.
    let mut st = env.state.clone();
    st.engine_tick_wait = Duration::from_millis(200);
    let hung = run_now(&env, &hang, json!({})).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pdt_api::run_workflows(&st).await;
        let r = run(&env, &hung).await;
        if r["steps"].as_array().unwrap().iter().any(|s| s["nodeId"] == "get" && s["status"] == "running") {
            break;
        }
        assert!(Instant::now() < deadline, "the request never started: {r:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let t = ticket(&env, "Review me").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Looks right."}"#, json!({})),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(30);
    let r = loop {
        pdt_api::run_workflows(&st).await;
        let r = run(&env, &id).await;
        if !matches!(r["status"].as_str(), Some("running" | "waiting")) {
            break r;
        }
        assert!(Instant::now() < deadline, "the review never ended while the other run hung: {r:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!((r["status"].as_str(), r["output"]["result"].as_str()), (Some("succeeded"), Some("approved")), "{r:#}");
    let r = run(&env, &hung).await;
    assert_eq!(r["status"], "running", "{r:#}");

    // Cancelling doesn't wait for the hung request, whose result, if it ever comes, goes nowhere.
    let cancel = tokio::time::timeout(
        Duration::from_secs(5),
        call(&env.app, "POST", &format!("/api/workflow-runs/{hung}/cancel"), None),
    )
    .await
    .expect("cancel waited for the hung step");
    assert_eq!(cancel.0, StatusCode::OK, "{}", cancel.1);
    let r = run(&env, &hung).await;
    assert_eq!(r["status"], "cancelled", "{r:#}");
    let get = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "get").unwrap().clone();
    assert_eq!(get["status"], "skipped", "{r:#}");
}

#[tokio::test]
async fn a_working_steps_removed_session_stalls_it() {
    let env = setup().await;
    let t = ticket(&env, "Long one").await;
    // Its turn stays open on a question for the user.
    let wf = create_workflow(&env, review_doc("ask: Which one?", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let step = tick_until(&env, "the step's session", || async {
        let r = run(&env, &id).await;
        let step = r["steps"].as_array()?.iter().find(|s| s["nodeId"] == "review")?.clone();
        (step["status"] == "waiting").then_some(step)
    })
    .await;
    let step_id = step["id"].as_str().unwrap();
    let binding = env.state.store.get_agent_step(step_id).await.unwrap().unwrap();
    let session = binding.session_id.clone().unwrap();
    let (s, v) = call(&env.app, "DELETE", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    // Past the grace a working step gets before its session counts as gone.
    let mut a = env.state.store.get_agent_step(step_id).await.unwrap().unwrap();
    assert!(a.session_id.is_none(), "removing the session cleared it");
    a.updated_at -= 60_000;
    env.state.store.put_agent_step(&a).await.unwrap();
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert_eq!(stalled["stepId"], step_id);
    assert!(stalled["body"]["why"].as_str().unwrap().contains("removed"), "{stalled}");
}

#[tokio::test]
async fn a_step_whose_session_failed_to_start_starts_one_on_retry() {
    let mut env = setup().await;
    let t = ticket(&env, "Retry me").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Second time lucky."}"#, json!({})),
    )
    .await;
    // The engine's state starts sessions: with no agent program they fail to start.
    env.state.platform.claude_program = env._tmp.path().join("no-such-claude").to_string_lossy().into_owned();
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "failed", "{r:#}");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    let a = env.state.store.get_agent_step(step["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(a.state, pdt_core::workflow::agent::AgentStepState::Done, "the failed step no longer works");
    assert!(a.session_id.is_none());

    env.state.platform.claude_program = FAKE.to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/retry"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(step["port"], "approved");
    assert!(step["output"]["sessionId"].is_string(), "{step}");
}

/// The agent step of a run and its session, once the step has one.
async fn step_session(env: &Env, run_id: &str) -> String {
    tick_until(env, "the step's session", || async {
        let r = run(env, run_id).await;
        let step = r["steps"].as_array()?.iter().find(|s| s["nodeId"] == "review")?.clone();
        step["wait"]["agent"]["session"].as_str().map(|s| json!(s))
    })
    .await
    .as_str()
    .unwrap()
    .to_string()
}

const NUDGE: &str = "You ended your turn without `submit_outcome`";

/// PDT-142: a turn that ends while a background task runs (a long pytest) is neither counted, nudged nor stalled; the
/// ticket says what it waits on. The turn Claude starts when the task finishes is evaluated as usual.
#[tokio::test]
async fn a_turn_ended_waiting_on_a_background_task_is_neither_nudged_nor_stalled() {
    let env = setup().await;
    let t = ticket(&env, "Slow tests").await;
    let wf = create_workflow(&env, review_doc("bg: 3000", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    let agent = tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        (v["agent"]["state"] == "background").then(|| v["agent"].clone())
    })
    .await;
    assert_eq!(agent["background"], json!(["Sleep in the background"]), "{agent}");
    // Ticking on while it runs: no nudge, no stall.
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(inbox(&env).await.is_empty());
    assert!(!agent_says(&env, &session).await.iter().any(|m| m.contains(NUDGE)));
    // The task's own turn ends without an outcome: that one is nudged, and the nudge's turn stalls.
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert_eq!((stalled["body"]["turns"].as_u64(), stalled["body"]["nudges"].as_u64()), (Some(2), Some(1)));
    let said = tick_until(&env, "the nudge's echo", || async {
        let said = agent_says(&env, &session).await;
        said.iter().any(|m| m.contains(NUDGE)).then(|| json!(said))
    })
    .await;
    let said: Vec<String> = serde_json::from_value(said).unwrap();
    let at = |what: &str| said.iter().position(|m| m.contains(what)).unwrap();
    assert!(at("The background task finished.") < at(NUDGE), "{said:?}");
}

/// PDT-142: a background task that ends without the agent taking it up leaves its turn ended without an outcome,
/// once the conversation stays quiet for the grace period.
#[tokio::test]
async fn a_background_task_that_ends_quietly_counts_as_the_turn_ending() {
    let env = setup().await;
    let t = ticket(&env, "Quiet task").await;
    let wf = create_workflow(&env, review_doc("bg-quiet: 1000", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        (v["agent"]["state"] == "background").then_some(Value::Null)
    })
    .await;
    assert!(inbox(&env).await.is_empty());
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert_eq!((stalled["body"]["turns"].as_u64(), stalled["body"]["nudges"].as_u64()), (Some(2), Some(1)));
    assert!(!agent_says(&env, &session).await.iter().any(|m| m.contains("The background task finished.")));
}

/// PDT-142: a hung background task still stalls the step after its cap, saying why; an outcome the stalled step
/// submits after all closes the Stalled item.
#[tokio::test]
async fn a_hung_background_task_stalls_at_the_cap_and_a_late_outcome_closes_the_stall() {
    let env = setup().await;
    let t = ticket(&env, "Hung task").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({"backgroundWaitMs": 1000}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    let why = stalled["body"]["why"].as_str().unwrap();
    assert!(why.starts_with("Waiting on background tasks for 0 min: Sleep in the background"), "{why}");
    assert_eq!((stalled["body"]["turns"].as_u64(), stalled["body"]["nudges"].as_u64()), (Some(0), Some(0)));
    assert!(!agent_says(&env, &session).await.iter().any(|m| m.contains(NUDGE)));
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    assert_eq!(v["agent"]["state"], "stalled", "{}", v["agent"]);

    let text = r#"mcp: submit_outcome {"outcome": "approved", "summary": "Done after all."}"#;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let r = run_ended(&env, &id).await;
    assert_eq!(r["output"]["result"], "approved", "{r:#}");
    assert!(inbox(&env).await.is_empty(), "the Stalled item closed");
}
