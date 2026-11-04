//! Agent steps end to end: pdt-mcp, personas, agent steps, the Inbox and Personal projects, with `fake-agent` standing
//! in for Claude (it calls pdt-mcp tools when a message says `mcp: <tool> <json>`). pdtd's API is served on a local
//! port so the agent can reach `/api/mcp`.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, call_with, git};
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
    setup_with(|_| {}).await
}

/// `setup`, with changes to the state its router serves.
async fn setup_with(change: impl FnOnce(&mut AppState)) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    state.conversations.background_grace = Duration::from_millis(500);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    change(&mut state);
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    // These tests are PDT's own handling of stuck steps: "When an agent is stuck" off (its tests: stuck_agents.rs).
    common::stuck_agents_off(&app).await;
    Env { app, state, project: p["id"].as_str().unwrap().into(), _tmp: tmp }
}

/// A ticket the user makes (from PDT's web client).
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

/// A conversation's open request (PDT-490: an asking tool's card of PDT's own): its id, once it shows.
async fn open_card(env: &Env, session: &str) -> String {
    tick_until(env, "the card", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
        let all = v.as_array().cloned().unwrap_or_default();
        all.into_iter()
            .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
            .map(|e| e["event"]["id"].clone())
    })
    .await
    .as_str()
    .unwrap()
    .to_string()
}

/// Allows each request open in a conversation, as the user's clicks.
async fn allow_cards(env: &Env, session: &str) {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    for e in v.as_array().cloned().unwrap_or_default() {
        if e["event"]["kind"] == "request" && e["event"]["state"] == "open" {
            let url = format!("/api/sessions/{session}/requests/{}", e["event"]["id"].as_str().unwrap_or_default());
            call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
        }
    }
}

/// Answers a conversation's request as the user's click.
async fn answer_card(env: &Env, session: &str, request: &str, answer: Value) {
    let url = format!("/api/sessions/{session}/requests/{request}");
    let (s, v) = call_as_ui(&env.app, "POST", &url, Some(json!({"answer": answer}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
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
    assert_eq!(names, ["Coder", "PM", "PR Reviewer", "QA", "Refiner", "Reviewer", "Triage"]);
    let reviewer = list.as_array().unwrap().iter().find(|p| p["name"] == "Reviewer").unwrap().clone();
    assert_eq!((reviewer["permissionMode"].as_str(), reviewer["builtin"].as_str()), (None, Some("reviewer")));

    // PDT-110: the Personas page lists every pdt-mcp tool from here, with what it does and whether it asks first.
    let (s, tools) = call(&env.app, "GET", "/api/personas/tools", None).await;
    assert_eq!(s, StatusCode::OK, "{tools}");
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, env.state.registry().tool_table().names());
    let link = tools.as_array().unwrap().iter().find(|t| t["name"] == "link_tickets").unwrap();
    assert_eq!((link["asks"].as_bool(), link["default"].as_bool()), (Some(true), Some(false)));

    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Sorter", "role": "pm", "agent": "claude", "instructions": "Sort it.",
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
        "mcp tools: get_ticket,submit_outcome,ask_user,add_ticket_comment,add_review_comment,list_secrets,\
         request_secret,run_with_secrets,write_secret_file,change_request_view,change_request_comment,\
         change_request_open,change_request_merge,ci_runs,ci_failed_logs,issue_view,issue_comment,documents_list,\
         document_get,doc_read,doc_edit,sheet_read,sheet_edit,document_create,document_link,comments_list,\
         comment_reply,comment_resolve,document_open_beside,issue_create"
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

/// PDT-414: a public-posting tool (a forge comment) runs in the workflow step that expects it; the same call from a
/// session outside a step asks in the Inbox first.
#[tokio::test]
async fn public_posting_tools_run_in_a_step_and_ask_outside_one() {
    let gh = std::sync::Arc::new(pdt_forge::testing::FakeForge::default());
    let fake = gh.clone();
    let env = setup_with(move |st| {
        st.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::testing::fake_github(&fake))]);
    })
    .await;
    git(&env._tmp.path().join("repo"), &["remote", "add", "origin", "https://github.com/o/r.git"]);
    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Commenter", "role": "coder", "agent": "claude", "instructions": "Comment.",
                    "tools": ["get_ticket", "submit_outcome", "issue_comment"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let own = ticket(&env, "Mine").await;
    let wf = create_workflow(
        &env,
        json!({
            "name": "Comment",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "review", "kind": "agent", "version": 1, "config": {"persona": "Commenter",
                    "prompt": "Comment.", "outcomes": [{"id": "submitted"}]}},
                {"id": "done", "kind": "end", "version": 1},
            ],
            "edges": [{"from": "go", "to": "review"}, {"from": "review", "port": "submitted", "to": "done"}],
        }),
    )
    .await;
    let run_id = run_now(&env, &wf, json!({"ticketId": own})).await;
    let session = step_session(&env, &run_id).await;
    let say = |session: String, text: &'static str| {
        let env = &env;
        async move {
            let before = agent_says(env, &session).await.len();
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
            tick_until(env, "the agent's answer", || async {
                agent_says(env, &session)
                    .await
                    .into_iter()
                    .skip(before)
                    .find(|m| m.starts_with("mcp "))
                    .map(|m| json!(m))
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    let comment = r#"mcp: issue_comment {"number": 3, "body": "Looking"}"#;
    let said = say(session.clone(), comment).await;
    assert!(said.contains("issues/3#issuecomment-1"), "{said}");
    // A persona version PDT can't find gives the session no tools, never the default set.
    assert!(env.state.store.delete_persona(p["id"].as_str().unwrap()).await.unwrap());
    let before = agent_says(&env, &session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": "mcp-list"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let listed = tick_until(&env, "the tool list", || async {
        agent_says(&env, &session)
            .await
            .into_iter()
            .skip(before)
            .find(|m| m.starts_with("mcp tools:"))
            .map(|m| json!(m))
    })
    .await;
    assert_eq!(listed, json!("mcp tools: "), "fails closed");

    // A session the user started on a ticket, outside any step.
    let other = ticket(&env, "Other").await;
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{other}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation", "permissionMode": "bypass"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let plain = started["session"]["id"].as_str().unwrap().to_string();
    let answered = say(plain.clone(), comment);
    let asking = async {
        let card = open_card(&env, &plain).await;
        assert_eq!(gh.issue_comments.lock().unwrap().len(), 1, "nothing posted before the user's yes");
        let items = inbox(&env).await;
        assert!(items.iter().any(|i| i["kind"] == "approval" && i["body"]["tool"] == "mcp__pdt__issue_comment"));
        answer_card(&env, &plain, &card, json!({"type": "deny"})).await;
    };
    let (said, ()) = tokio::join!(answered, asking);
    assert!(said.contains("denied"), "{said}");
    assert_eq!(gh.issue_comments.lock().unwrap().len(), 1, "nothing posted after a Deny");
}

/// PDT-495: a turn's end that is stored late (a slow or failing write) expires only PDT's requests that were open when
/// it ended, never the card of the next turn's call that opened meanwhile.
#[tokio::test]
async fn a_turn_end_stored_late_leaves_the_next_turns_card_open() {
    let gh = std::sync::Arc::new(pdt_forge::testing::FakeForge::default());
    let fake = gh.clone();
    let env = setup_with(move |st| {
        st.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::testing::fake_github(&fake))]);
    })
    .await;
    git(&env._tmp.path().join("repo"), &["remote", "add", "origin", "https://github.com/o/r.git"]);
    let other = ticket(&env, "Other").await;
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{other}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation", "permissionMode": "bypass"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let plain = started["session"]["id"].as_str().unwrap().to_string();
    let idle = || async {
        (!agent_says(&env, &plain).await.is_empty() && !pdt_api::sdk::conversations::is_working(&env.state, &plain))
            .then_some(json!(true))
    };
    tick_until(&env, "the first turn's end", idle).await;

    // The disk fills: the next turn's end waits to be stored, with the agent idle.
    let db = env._tmp.path().join("pdt.db");
    common::disk_full(&db, "session_events", Some("1")).await;
    let send = |text: &str| {
        let (env, url) = (&env, format!("/api/sessions/{plain}/messages"));
        let body = json!({"text": text});
        async move {
            let (s, v) = call(&env.app, "POST", &url, Some(body)).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        }
    };
    send("hello").await;
    tick_until(&env, "the second turn's end", || async {
        (!pdt_api::sdk::conversations::is_working(&env.state, &plain)).then_some(json!(true))
    })
    .await;
    // The third turn's call asks; its card waits behind the unstored end.
    let n = agent_says(&env, &plain).await.len();
    send(r#"mcp: issue_comment {"number": 3, "body": "Looking"}"#).await;
    tick_until(&env, "the call asking", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{plain}"), None).await;
        (s["activity"] == "needs_you").then_some(json!(true))
    })
    .await;
    common::disk_full(&db, "session_events", None).await;

    let card = open_card(&env, &plain).await;
    answer_card(&env, &plain, &card, json!({"type": "deny"})).await;
    let said = tick_until(&env, "the call's answer", || async {
        agent_says(&env, &plain).await.into_iter().skip(n).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
    })
    .await;
    assert!(said.as_str().unwrap().contains("denied"), "{said}");
    assert!(gh.issue_comments.lock().unwrap().is_empty(), "nothing posted after a Deny");
}

/// PDT-534: letting the PM act is the PM's alone. A Coder in Claude's own permission prompt, in a project that lets the
/// PM act, still waits for the user: PDT doesn't answer its request for them.
#[tokio::test]
async fn another_personas_prompt_waits_for_the_user_where_the_pm_may_act() {
    let env = setup().await;
    let (_, list) = call(&env.app, "GET", "/api/personas", None).await;
    let mut coder = list.as_array().unwrap().iter().find(|p| p["name"] == "Coder").unwrap().clone();
    let mut tools: Vec<&str> = pdt_core::persona::DEFAULT_TOOLS.to_vec();
    tools.push("edit_ticket");
    coder["tools"] = json!(tools);
    let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", coder["id"].as_str().unwrap()), Some(coder)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mine = ticket(&env, "Mine").await;
    let other = ticket(&env, "Other").await;
    let (_, o) = call(&env.app, "GET", &format!("/api/tickets/{other}"), None).await;
    let other_key = o["key"].as_str().unwrap().to_string();
    let wf = create_workflow(&env, review_doc("Get going.", json!({"persona": "Coder"}))).await;
    let run_id = run_now(&env, &wf, json!({"ticketId": mine})).await;
    let session = step_session(&env, &run_id).await;
    let n = agent_says(&env, &session).await.len();
    let text = format!(r#"mcp: edit_ticket {{"ticket": "{other_key}", "title": "Renamed"}}"#);
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let card = open_card(&env, &session).await;
    // Long enough for PDT to have answered it, had it thought the Coder could act.
    for _ in 0..10 {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    let still =
        v.as_array().unwrap().iter().any(|e| e["event"]["id"] == card.as_str() && e["event"]["state"] == "open");
    assert!(still, "PDT answered the Coder's request: {v:#}");
    let (_, o) = call(&env.app, "GET", &format!("/api/tickets/{other}"), None).await;
    assert_eq!(o["title"], "Other", "nothing before the user's yes");
    answer_card(&env, &session, &card, json!({"type": "deny"})).await;
    let said = tick_until(&env, "the agent's answer", || async {
        agent_says(&env, &session).await.into_iter().skip(n).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
    })
    .await;
    assert!(said.as_str().unwrap().contains("denied"), "{said}");
}

/// PDT-163: a Coder working on one ticket edits and comments on another; both name the Coder, its session and the
/// ticket it works on, in the timeline and in `get_ticket`. Its edit asks even where the project lets the PM act.
#[tokio::test]
async fn an_agents_changes_to_another_ticket_name_its_persona_session_and_ticket() {
    let env = setup().await;
    let (_, list) = call(&env.app, "GET", "/api/personas", None).await;
    let mut coder = list.as_array().unwrap().iter().find(|p| p["name"] == "Coder").unwrap().clone();
    let mut tools: Vec<&str> = pdt_core::persona::DEFAULT_TOOLS.to_vec();
    tools.push("edit_ticket");
    coder["tools"] = json!(tools);
    // edit_ticket asks first; Bypass never shows Claude's prompt, so PDT asks in the conversation (PDT-414, PDT-490).
    coder["permissionMode"] = json!("bypass");
    let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", coder["id"].as_str().unwrap()), Some(coder)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mine = ticket(&env, "Mine").await;
    let other = ticket(&env, "Other").await;
    let (_, m) = call(&env.app, "GET", &format!("/api/tickets/{mine}"), None).await;
    let (_, o) = call(&env.app, "GET", &format!("/api/tickets/{other}"), None).await;
    let (mine_key, other_key) = (m["key"].as_str().unwrap().to_string(), o["key"].as_str().unwrap().to_string());
    let wf = create_workflow(&env, review_doc("Get going.", json!({"persona": "Coder"}))).await;
    let run_id = run_now(&env, &wf, json!({"ticketId": mine})).await;
    let session = step_session(&env, &run_id).await;
    let say = |text: String| {
        let (env, session) = (&env, session.clone());
        async move {
            let before = agent_says(env, &session).await.len();
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
            tick_until(env, "the agent's answer", || async {
                let said = agent_says(env, &session).await;
                said.iter().skip(before).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    let edit = format!(r#"mcp: edit_ticket {{"ticket": "{other_key}", "title": "Renamed", "stage": "ready"}}"#);
    let answered = say(edit.clone());
    let asking = async {
        let card = open_card(&env, &session).await;
        let approval = inbox(&env).await.into_iter().find(|i| i["kind"] == "approval").expect("in the Inbox too");
        assert_eq!(approval["body"]["tool"], "mcp__pdt__edit_ticket");
        let (_, o) = call(&env.app, "GET", &format!("/api/tickets/{other}"), None).await;
        assert_eq!(o["title"], "Other", "nothing before the user's yes");
        answer_card(&env, &session, &card, json!({"type": "deny"})).await;
    };
    let (said, ()) = tokio::join!(answered, asking);
    assert!(said.contains("denied"), "{said}");
    // Letting the PM act is the PM's: the Coder still asks there (PDT-534), and goes ahead on the user's yes.
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let answered = say(edit);
    let asking = async {
        let card = open_card(&env, &session).await;
        let (_, o) = call(&env.app, "GET", &format!("/api/tickets/{other}"), None).await;
        assert_eq!(o["title"], "Other", "nothing before the user's yes");
        answer_card(&env, &session, &card, json!({"type": "approve"})).await;
    };
    let (said, ()) = tokio::join!(answered, asking);
    assert!(said.contains("\"updated\": true"), "{said}");
    let said = say(format!(r#"mcp: add_ticket_comment {{"ticket": "{other_key}", "body": "Mine needs this."}}"#)).await;
    assert!(said.contains("\"commented\": true"), "{said}");

    let want = json!({"sessionId": session, "projectId": env.project, "persona": "Coder", "fromTicket": mine_key,
                      "fromTicketId": mine});
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{other}/events"), None).await;
    let items = events["items"].as_array().unwrap();
    for kind in ["edited", "moved", "comment"] {
        let e = items.iter().find(|e| e["kind"] == kind).unwrap_or_else(|| panic!("no {kind} in {events:#}"));
        assert_eq!((&e["actor"], &e["agent"]), (&json!("agent"), &want), "{kind}: {e:#}");
    }
    let edited = items.iter().find(|e| e["kind"] == "edited").unwrap();
    assert_eq!(edited["data"]["fields"], json!(["title"]));
    // The user's own changes name nobody.
    let created = items.iter().find(|e| e["kind"] == "created").unwrap();
    assert_eq!((&created["actor"], &created["agent"]), (&json!("user"), &Value::Null));

    let said = say(format!(r#"mcp: get_ticket {{"ticket": "{other_key}"}}"#)).await;
    let got: Value = serde_json::from_str(said.strip_prefix("mcp get_ticket: ").unwrap_or(&said)).unwrap_or_else(|e| {
        panic!("{e}: {said}");
    });
    let comment = &got["comments"][0];
    assert_eq!(
        (&comment["by"], &comment["sessionId"], &comment["fromTicket"], &comment["body"]),
        (&json!("Coder"), &json!(session), &json!(mine_key), &json!("Mine needs this."))
    );
    let history = got["history"].as_array().unwrap();
    let kinds: Vec<&str> = history.iter().map(|h| h["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["created", "edited", "moved"], "{got:#}");
    assert_eq!((&history[0]["by"], &history[0].get("sessionId")), (&json!("user"), &None));
    for h in &history[1..] {
        assert_eq!((&h["by"], &h["sessionId"], &h["fromTicket"]), (&json!("Coder"), &json!(session), &json!(mine_key)));
        assert!(h["data"].get("agent").is_none(), "{h}");
    }
    assert_eq!(history[1]["data"]["fields"], json!(["title"]));
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
    // PDT-183: a `new` step's session is done once its outcome is in.
    session_finished(&env, &session).await;
}

/// PDT-188: a message sent to a `new` step's session just as its run ends (before the stop) is taken: the stop waits
/// for the turn it starts, or the message reopens the session after it.
#[tokio::test]
async fn a_message_sent_as_a_steps_session_is_stopped_is_answered() {
    // The message goes in between the session found idle and its stop.
    let env = setup_with(|st| st.conversations.close_pause = Duration::from_millis(1500)).await;
    let t = ticket(&env, "Late message").await;
    let wf = create_workflow(
        &env,
        review_doc(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Fine."}"#, json!({})),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    // Once the outcome's turn is over (the close is pausing), a turn longer than a closing process's grace (5 s), so a
    // stop that doesn't wait for it kills it.
    tick_until(&env, "the outcome's turn ended", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        (s["activity"] == "idle").then_some(s)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s["status"], "running", "the close is still pausing");
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": "compact:7000"})))
            .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    tick_until(&env, "the answer", || async {
        agent_says(&env, &session).await.iter().any(|m| m == "Compacted and carried on.").then_some(json!(null))
    })
    .await;
    session_finished(&env, &session).await;
}

/// Ticks until a session is Finished.
async fn session_finished(env: &Env, id: &str) {
    tick_until(env, "the session finished", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
        (s["status"] == "finished").then_some(s)
    })
    .await;
}

/// PDT-183: an `own` step goes on in the session it had on an earlier pass of the run, never in the attempt's (the
/// Coder's), is parked between passes, and is stopped when the run ends; the attempt's session goes on.
#[tokio::test]
async fn an_own_step_keeps_one_session_across_passes_and_closes_it() {
    let env = setup().await;
    let t = ticket(&env, "Twice reviewed").await;
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let coder = started["session"]["id"].as_str().unwrap().to_string();
    let wf = create_workflow(
        &env,
        json!({
            "name": "Fix and review",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "fix", "kind": "agent", "version": 1, "config": {"persona": "Coder", "owner": "user",
                    "prompt": "Fix it.", "outcomes": [{"id": "submitted"}]}},
                {"id": "review", "kind": "agent", "version": 1, "config": {"persona": "Reviewer", "session": "own",
                    "nudges": 0,
                    "prompt": r#"mcp: submit_outcome {"outcome": "changes_requested", "summary": "Not yet."}"#,
                    "outcomes": [{"id": "approved"}, {"id": "changes_requested"}]}},
                {"id": "done", "kind": "end", "version": 1},
            ],
            "edges": [
                {"from": "go", "to": "fix"},
                {"from": "fix", "port": "submitted", "to": "review"},
                {"from": "review", "port": "changes_requested", "to": "fix"},
                {"from": "review", "port": "approved", "to": "done"},
            ],
        }),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let fixed = |n: usize| {
        let env = &env;
        async move {
            let item = tick_until(env, "the fix step", || async {
                inbox(env).await.into_iter().find(|i| i["kind"] == "manual")
            })
            .await;
            let (s, v) = call(
                &env.app,
                "POST",
                &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()),
                Some(json!({"choice": "outcome:submitted", "text": format!("Fix {n}.")})),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    };
    fixed(1).await;
    let reviewer = tick_until(&env, "the first review", || async {
        let r = run(&env, &id).await;
        let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review" && s["port"].is_string());
        step.map(|s| s["output"]["sessionId"].clone())
    })
    .await;
    let reviewer = reviewer.as_str().unwrap().to_string();
    assert_ne!(reviewer, coder, "never the attempt's session");
    // Between passes it is parked, not stopped.
    tick_until(&env, "the reviewer parked", || async {
        (!pdt_api::conversation_live(&env.state, &reviewer)).then_some(json!(null))
    })
    .await;
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{reviewer}"), None).await;
    assert_ne!(s["status"], "finished", "{s}");

    fixed(2).await;
    tick_until(&env, "the second pass in the same session", || async {
        let said = agent_says(&env, &reviewer).await;
        said.iter().any(|m| m.contains("back for another pass") && m.contains("Fix 2.")).then_some(json!(null))
    })
    .await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{reviewer}/messages"),
        Some(json!({"text": r#"mcp: submit_outcome {"outcome": "approved", "summary": "Good now."}"#})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    let reviews: Vec<&Value> = r["steps"].as_array().unwrap().iter().filter(|s| s["nodeId"] == "review").collect();
    assert_eq!(reviews.len(), 2, "{r:#}");
    assert!(reviews.iter().all(|s| s["output"]["sessionId"].as_str() == Some(reviewer.as_str())), "{r:#}");
    assert_eq!(reviews[1]["port"], "approved");
    // The run ended: the Reviewer's session is stopped, the Coder's goes on and never got the review.
    session_finished(&env, &reviewer).await;
    let (_, c) = call(&env.app, "GET", &format!("/api/sessions/{coder}"), None).await;
    assert_ne!(c["status"], "finished", "{c}");
    assert!(!agent_says(&env, &coder).await.iter().any(|m| m.contains("submit_outcome") || m.contains("Reviewer")));
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    let reviewers = v["sessions"].as_array().unwrap().iter().filter(|s| s["persona"] == "Reviewer").count();
    assert_eq!(reviewers, 1, "{v:#}");
}

/// PDT-183: a cancelled run stops its steps' sessions at once, even mid-turn.
#[tokio::test]
async fn a_cancelled_run_stops_its_steps_sessions() {
    let env = setup().await;
    let t = ticket(&env, "Cancel me").await;
    let wf = create_workflow(&env, review_doc("slow", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = tick_until(&env, "the review session", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        v["sessions"][0]["id"].as_str().map(|s| json!(s))
    })
    .await;
    let session = session.as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    session_finished(&env, &session).await;
}

/// PDT-152: a personal workflow's agent step, with no project or ticket, runs in the host project as the PM does, and
/// its dry run says so (and fails where the real run would).
#[tokio::test]
async fn an_agent_step_in_a_personal_workflow_runs_in_the_host_project() {
    let env = setup().await;
    let doc = json!({
        "name": "Sweeper",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "sweep", "kind": "agent", "version": 1, "config": {"persona": "PM",
                "prompt": r#"mcp: submit_outcome {"outcome": "nothing_ready", "summary": "None to import."}"#,
                "outcomes": [{"id": "imported"}, {"id": "nothing_ready"}, {"id": "needs_user"}]}},
            {"id": "done", "kind": "end", "version": 1, "config": {"output": {"result": "{{steps.sweep.outcome}}"}}},
        ],
        "edges": [{"from": "go", "to": "sweep"}, {"from": "sweep", "port": "nothing_ready", "to": "done"}],
    });
    let (s, dry) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": doc, "event": {"type": "manual"}}))).await;
    assert_eq!(s, StatusCode::OK, "{dry}");
    let step = dry["run"]["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "sweep").unwrap().clone();
    assert!(step["error"].is_null(), "{dry:#}");
    assert!(step["output"]["would"].as_str().unwrap().contains("in the host project"), "{dry:#}");
    let mut missing = doc.clone();
    missing["nodes"][1]["config"]["persona"] = json!("Nobody");
    let (_, dry) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": missing, "event": {"type": "manual"}})))
            .await;
    let step = dry["run"]["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "sweep").unwrap().clone();
    assert_eq!(step["error"], "no persona is called Nobody", "{dry:#}");

    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "scope": "host", "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert!(w["projectId"].is_null(), "{w}");
    let id = run_now(&env, w["id"].as_str().unwrap(), json!({})).await;
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    assert_eq!(r["output"]["result"], "nothing_ready");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "sweep").unwrap().clone();
    let session = step["output"]["sessionId"].as_str().unwrap().to_string();
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s["persona"]["name"], "PM", "{s}");
    assert!(s["ticket"].is_null() && s["attemptId"].is_null(), "{s}");
    let host = env.state.store.host_project().await.unwrap().unwrap();
    assert_eq!(s["projectId"].as_str(), Some(host.id.as_str()), "{s}");
    // The PM's work folder: the folder holding the projects' checkouts.
    let work = std::path::PathBuf::from(s["cwd"].as_str().unwrap());
    assert_eq!(work.canonicalize().unwrap(), env._tmp.path().canonicalize().unwrap(), "{s}");
    let instructions =
        std::fs::read_to_string(env.state.data_dir.join("sessions").join(&session).join("instructions.md")).unwrap();
    for want in ["project manager", "# Your role: PM", "`nothing_ready`", "`catalog_search`"] {
        assert!(instructions.contains(want), "missing {want:?} in\n{instructions}");
    }
    assert!(inbox(&env).await.is_empty());
}

#[tokio::test]
async fn questions_nudges_stalls_and_take_over_go_through_the_inbox() {
    let env = setup().await;
    let t = ticket(&env, "Unclear").await;
    let wf = create_workflow(
        &env,
        review_doc(
            r#"mcp: ask_user {"questions": [{"question": "Which file?", "context": "Two files **match**.", "choices": [{"label": "a.txt", "description": "The newer one."}, "b.txt"], "recommended": "a.txt", "why": "It is newer."}]}"#,
            json!({}),
        ),
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
    // PDT-476: the context, the described choices (a plain one as a label) and the recommendation are kept.
    assert_eq!(
        q["body"]["questions"][0],
        json!({"question": "Which file?", "context": "Two files **match**.",
               "choices": [{"label": "a.txt", "description": "The newer one."}, {"label": "b.txt"}],
               "recommended": "a.txt", "why": "It is newer."}),
        "{q}"
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
    // The run ends when submit_outcome takes the outcome, before the agent writes the tool's reply: wait for it.
    tick_until(&env, "the submit_outcome reply", || async {
        agent_says(&env, &session).await.iter().any(|m| m.starts_with("mcp submit_outcome:")).then_some(json!(null))
    })
    .await;
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
    // The run has moved on: another outcome finds no step. The run's end stops the step's session once its turn is over
    // (PDT-183); wait for that, so the message reopens it rather than landing in a turn the stop then cuts short.
    tick_until(&env, "the step's session stopped", || async {
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
        (s["status"] == "finished").then_some(s)
    })
    .await;
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
        call_as_ui(&env.app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Weekly brief"})))
            .await;
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
    // Its turn stays open while it works (a step's session bypasses permissions, so an `ask:` would be answered at once
    // and its turn end before the session is removed).
    let wf = create_workflow(&env, review_doc("slow", json!({}))).await;
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

/// An orderly pdtd restart (an update): the old pdtd stops its agents, background tasks and all, and a new one on the
/// same store recovers and serves on.
async fn restart(env: Env) -> Env {
    let Env { state: old, project, _tmp: tmp, .. } = env;
    old.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    old.store.mark_running_interrupted(1).await.unwrap();
    pdt_api::shutdown_conversations(&old).await;
    let mut state = common::app_state(old.store.clone(), tmp.path());
    state.platform = old.platform.clone();
    state.conversations.notice_quiet = Duration::ZERO;
    state.conversations.background_grace = Duration::from_millis(500);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    pdt_api::recover_conversations(&state).await;
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    Env { app, state, project, _tmp: tmp }
}

/// The messages a session's agent took (echoed).
async fn agent_took(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| e["event"]["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

const RESTARTED: &str = "[PDT] PDT restarted and your background tasks were stopped: Sleep in the background.";

/// PDT-311: a session waiting on a background task keeps the host busy, so an update when idle waits for it.
#[tokio::test]
async fn a_session_waiting_on_a_background_task_keeps_the_host_busy() {
    let env = setup().await;
    let t = ticket(&env, "Busy").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    step_session(&env, &id).await;
    tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        (v["agent"]["state"] == "background").then_some(Value::Null)
    })
    .await;
    let busy = pdt_api::update::busy_reasons(&env.state).await;
    assert!(busy.iter().any(|b| b.ends_with("is waiting on 1 background task")), "{busy:?}");
}

/// PDT-311: PDT restarts while a step waits on a background task. The task died with the agent, so the step's
/// conversation reopens to hear so (an automatic nudge) instead of the step stalling as stopped; its turn is then
/// judged as usual.
#[tokio::test]
async fn a_restart_while_a_step_waits_on_a_background_task_tells_the_agent() {
    let env = setup().await;
    let t = ticket(&env, "Restarted").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        (v["agent"]["state"] == "background").then_some(Value::Null)
    })
    .await;
    let env = restart(env).await;
    tick_until(&env, "the restart notice", || async {
        agent_took(&env, &session).await.iter().any(|m| m.trim().starts_with(RESTARTED)).then_some(Value::Null)
    })
    .await;
    // The notice was the step's one automatic nudge: its turn ending without an outcome stalls it, saying so.
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert_eq!(stalled["body"]["why"], "Its turn ended without an outcome.", "{stalled}");
    assert_eq!(stalled["body"]["nudges"].as_u64(), Some(1));
    assert_eq!(agent_took(&env, &session).await.iter().filter(|m| m.contains("PDT restarted")).count(), 1);
    call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
}

/// PDT-311: with no automatic nudge left, a step a restart left waiting on stopped background tasks stalls, saying so.
#[tokio::test]
async fn a_restart_stalls_a_waiting_step_with_no_nudges_left_saying_why() {
    let env = setup().await;
    let t = ticket(&env, "No nudges").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({"nudges": 0}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        (v["agent"]["state"] == "background").then_some(Value::Null)
    })
    .await;
    let env = restart(env).await;
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    assert_eq!(
        stalled["body"]["why"], "PDT restarted while it waited on background tasks: Sleep in the background.",
        "{stalled}"
    );
    // The user's Nudge reopens it, and the agent hears of the stopped task with it.
    let item = stalled["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/inbox/{item}/answer"), Some(json!({"choice": "nudge"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the restart notice", || async {
        agent_took(&env, &session).await.iter().any(|m| m.trim().starts_with(RESTARTED)).then_some(Value::Null)
    })
    .await;
    call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
}

/// Ticks the engine for `ms`.
async fn tick_for(env: &Env, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// PDT-142: the cap doesn't stall a step whose session is at work in a turn (a notice or the user's message started
/// it) while a long-lived background task runs.
#[tokio::test]
async fn the_background_cap_does_not_stall_a_turn_at_work() {
    let env = setup().await;
    let t = ticket(&env, "Dev server").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({"backgroundWaitMs": 1000}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let session = step_session(&env, &id).await;
    // No ticks from here until the turn runs: the wait starts at the turn's end, the cap only on a tick.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        if v["agent"]["state"] == "background" {
            break;
        }
        assert!(Instant::now() < deadline, "never waited on the task: {}", v["agent"]);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": "slow"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    tick_for(&env, 1500).await;
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    assert_eq!((v["agent"]["state"].as_str(), v["agent"]["busy"].as_bool()), (Some("working"), Some(true)));
    assert!(inbox(&env).await.iter().all(|i| i["kind"] != "stalled"), "stalled mid-turn");
}

/// PDT-142: a binding still working on a step that has ended (cut short some other way) is done, not stalled by the
/// cap.
#[tokio::test]
async fn the_background_cap_does_not_stall_a_step_that_has_ended() {
    let env = setup().await;
    let t = ticket(&env, "Ended").await;
    let wf = create_workflow(&env, review_doc("bg: 600000", json!({"backgroundWaitMs": 1000}))).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    step_session(&env, &id).await;
    let step_id = tick_until(&env, "the wait on the task", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
        let r = run(&env, &id).await;
        let step = r["steps"].as_array()?.iter().find(|s| s["nodeId"] == "review")?.clone();
        (v["agent"]["state"] == "background").then(|| step["id"].clone())
    })
    .await;
    let step_id = step_id.as_str().unwrap();
    // The step ends without its binding hearing of it.
    let mut step = env.state.store.get_step(step_id).await.unwrap().unwrap();
    step.status = pdt_core::workflow::api::StepStatus::Failed;
    env.state.store.put_step(&step).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    tick_for(&env, 500).await;
    assert!(inbox(&env).await.iter().all(|i| i["kind"] != "stalled"), "stalled an ended step");
    let a = env.state.store.get_agent_step(step_id).await.unwrap().unwrap();
    assert_eq!(a.state, pdt_core::workflow::agent::AgentStepState::Done);
}

/// PDT-144: a later pass's message that never reaches its session (the agent stopped listening, then was stopped)
/// stalls the step saying so and quoting it, and Resume with this message sends it again, reopening the conversation.
#[tokio::test]
async fn a_pass_message_that_never_arrives_stalls_the_step_and_can_be_sent_again() {
    let env = setup().await;
    let t = ticket(&env, "Twice").await;
    let wf = create_workflow(&env, json!({
        "name": "Code and accept",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "code", "kind": "agent", "version": 1, "config": {"persona": "Coder", "session": "reuse",
                "prompt": r#"mcp: submit_outcome {"outcome": "submitted", "summary": "Built."}"#,
                "outcomes": [{"id": "submitted"}]}},
            {"id": "accept", "kind": "gate", "version": 1, "config": {"question": "Accept?", "choices": ["accept", "reject"]}},
            {"id": "done", "kind": "end", "version": 1},
        ],
        "edges": [
            {"from": "go", "to": "code"},
            {"from": "code", "port": "submitted", "to": "accept"},
            {"from": "accept", "port": "accept", "to": "done"},
            {"from": "accept", "port": "reject", "to": "code"},
        ],
    }))
    .await;
    run_now(&env, &wf, json!({"ticketId": t})).await;
    let gate =
        tick_until(&env, "the gate", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "gate") }).await;
    let coder = gate_session(&env, &t).await;
    // The agent stops reading: the next pass is handed to it but never taken, and stopping it loses it.
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{coder}/messages"), Some(json!({"text": "deaf"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    tick_until(&env, "the agent going deaf", || async {
        agent_says(&env, &coder).await.iter().any(|m| m.contains("Not listening")).then_some(json!(null))
    })
    .await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", gate["id"].as_str().unwrap()),
        Some(json!({"choice": "reject", "text": "Wrong colour."})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the pass on its way", || async {
        let sending = env.state.store.sending_session_messages(None).await.unwrap();
        sending.iter().any(|m| m.text.contains("back for another pass")).then_some(json!(null))
    })
    .await;
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{coder}/kill"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let stalled =
        tick_until(&env, "the stall", || async { inbox(&env).await.into_iter().find(|i| i["kind"] == "stalled") })
            .await;
    let why = stalled["body"]["why"].as_str().unwrap();
    assert!(why.starts_with("Its message was never delivered: [PDT] Your step is back for another pass."), "{why}");
    assert!(why.contains("Wrong colour."), "{why}");
    let choices: Vec<&str> = stalled["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["resend", "nudge", "outcome:submitted"]);
    assert_eq!(stalled["choices"][0]["primary"], true);

    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", stalled["id"].as_str().unwrap()),
        Some(json!({"choice": "resend"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the pass reaching the reopened coder", || async {
        let said = agent_says(&env, &coder).await;
        said.iter().any(|m| m.contains("back for another pass") && m.contains("Wrong colour.")).then_some(json!(null))
    })
    .await;
    assert!(
        !inbox(&env).await.iter().any(
            |i| i["kind"] == "stalled" && i["body"]["why"].as_str().unwrap_or_default().contains("never delivered")
        )
    );
}

/// The session of a ticket's only agent step so far.
async fn gate_session(env: &Env, t: &str) -> String {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    v["sessions"][0]["id"].as_str().unwrap().to_string()
}

/// PDT-162: a Coder edits its own ticket and drafts it created; another ticket needs its exact key, and one in
/// progress or done that it didn't create keeps its title and description.
#[tokio::test]
async fn a_coder_edits_only_its_own_tickets_unless_it_names_another_by_key() {
    let env = setup().await;
    // A Bypass session's asking tools ask in its conversation (PDT-414), even where the project lets the PM act
    // (PDT-534): the user allows each (`allow_cards`).
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Its follow-ups go straight to the board (proposals: `ticket_proposals_wait_for_the_user`).
    let proposals = builtin_workflow(&env, "ticket-proposals").await;
    call(
        &env.app,
        "PATCH",
        &format!("/api/workflows/{}", proposals["id"].as_str().unwrap()),
        Some(json!({"enabled": false})),
    )
    .await;
    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Builder", "role": "coder", "agent": "claude", "instructions": "Build it.",
                    "permissionMode": "bypass",
                    "tools": ["get_ticket", "submit_outcome", "create_ticket", "edit_ticket"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (_, project) = call(&env.app, "GET", &format!("/api/projects/{}", env.project), None).await;
    let key = project["key"].as_str().unwrap().to_string();
    let own = ticket(&env, "Mine").await;
    let busy = ticket(&env, "Someone else's").await;
    let done = ticket(&env, "Finished").await;
    let todo = ticket(&env, "Up next").await;
    for (t, stage) in [(&busy, "in_progress"), (&done, "done")] {
        let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": stage}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let key_of = |t: &str| {
        let (app, t) = (env.app.clone(), t.to_string());
        async move { call(&app, "GET", &format!("/api/tickets/{t}"), None).await.1["key"].as_str().unwrap().to_string() }
    };
    let (busy_key, done_key, todo_key) = (key_of(&busy).await, key_of(&done).await, key_of(&todo).await);
    let wf = create_workflow(
        &env,
        json!({
            "name": "Build",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "code", "kind": "agent", "version": 1, "config": {"persona": "Builder",
                    "prompt": "Build it.", "outcomes": [{"id": "submitted"}]}},
                {"id": "done", "kind": "end", "version": 1},
            ],
            "edges": [{"from": "go", "to": "code"}, {"from": "code", "port": "submitted", "to": "done"}],
        }),
    )
    .await;
    run_now(&env, &wf, json!({"ticketId": own})).await;
    let coder = tick_until(&env, "the coder", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{own}"), None).await;
        v["sessions"][0]["id"].as_str().map(|s| json!(s))
    })
    .await;
    let coder = coder.as_str().unwrap().to_string();
    let ask = |text: String| {
        let (env, coder) = (&env, coder.clone());
        async move {
            let n = agent_says(env, &coder).await.len();
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{coder}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
            // Its answer to the tool call, past PDT's nudges to finish the step.
            tick_until(env, "the agent's answer", || async {
                allow_cards(env, &coder).await;
                agent_says(env, &coder).await.into_iter().skip(n).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    let title = |t: &str| {
        let (app, t) = (env.app.clone(), t.to_string());
        async move { call(&app, "GET", &format!("/api/tickets/{t}"), None).await.1["title"].as_str().unwrap().to_string() }
    };

    // Its own ticket, even by id.
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{own}", "title": "Mine, renamed"}}"#)).await;
    assert!(said.contains("\"updated\": true"), "{said}");
    // A draft it created, and still once work on it starts.
    let said = ask(format!(r#"mcp: create_ticket {{"project": "{key}", "title": "Follow-up"}}"#)).await;
    assert!(said.contains("\"created\": true"), "{said}");
    let follow = said.split("\"key\": \"").nth(1).unwrap().split('"').next().unwrap().to_string();
    // PDT-363: recorded as the workflow's agent, with its persona and the ticket it works on, not as the user.
    let own_key = key_of(&own).await;
    let (_, made) = call(&env.app, "GET", &format!("/api/tickets/{follow}"), None).await;
    assert_eq!(made["source"], "agent", "{made}");
    assert_eq!(
        (made["createdBy"]["persona"].as_str(), made["createdBy"]["fromTicket"].as_str()),
        (Some("Builder"), Some(own_key.as_str()))
    );
    assert_eq!(made["createdBy"]["sessionId"], coder.as_str());
    let made_id = made["id"].as_str().unwrap();
    let (_, log) = call(&env.app, "GET", &format!("/api/events?type=ticket.created&ticket={made_id}"), None).await;
    let e = &log[0]["payload"];
    assert_eq!(log.as_array().unwrap().len(), 1, "{log}");
    assert_eq!(
        (e["actor"].as_str(), e["persona"].as_str(), e["sessionId"].as_str()),
        (Some("agent"), Some("Builder"), Some(coder.as_str()))
    );
    assert_eq!((e["fromTicket"].as_str(), e["fromTicketId"].as_str()), (Some(own_key.as_str()), Some(own.as_str())));
    // Over the HTTP API: the session its header names; another curl is an agent's; PDT's web client is the user.
    let new_ticket = format!("/api/projects/{}/tickets", env.project);
    let (s, v) =
        call_with(&env.app, "POST", &new_ticket, Some(json!({"title": "Curled"})), &[("x-agent-session", &coder)])
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["source"].as_str(), v["createdBy"]["persona"].as_str()), (Some("agent"), Some("Builder")), "{v}");
    assert_eq!(v["createdBy"]["fromTicket"], own_key.as_str());
    let (_, v) = call(&env.app, "POST", &new_ticket, Some(json!({"title": "Anonymous"}))).await;
    assert_eq!((v["source"].as_str(), v["createdBy"].clone()), (Some("agent"), json!(null)), "{v}");
    let (_, log) =
        call(&env.app, "GET", &format!("/api/events?type=ticket.created&ticket={}", v["id"].as_str().unwrap()), None)
            .await;
    assert_eq!(log[0]["payload"]["actor"], "agent", "{log}");
    let (_, v) = call_as_ui(&env.app, "POST", &new_ticket, Some(json!({"title": "Clicked"}))).await;
    assert_eq!((v["source"].as_str(), v["createdBy"].clone()), (Some("user"), json!(null)), "{v}");
    let parts = format!("/api/tickets/{own}/parts");
    let (s, v) = call_with(
        &env.app,
        "POST",
        &parts,
        Some(json!({"parts": [{"title": "Part"}]})),
        &[("x-agent-session", &coder)],
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let part = v["items"][0]["id"].as_str().unwrap();
    let (_, log) = call(&env.app, "GET", &format!("/api/events?type=ticket.created&ticket={part}"), None).await;
    let e = &log[0]["payload"];
    assert_eq!(
        (e["actor"].as_str(), e["persona"].as_str(), e["viaParts"].as_bool()),
        (Some("agent"), Some("Builder"), Some(true)),
        "{e}"
    );
    let said =
        ask(format!(r#"mcp: edit_ticket {{"ticket": "{follow}", "description": "More.", "stage": "in_progress"}}"#))
            .await;
    assert!(said.contains("\"updated\": true"), "{said}");
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{follow}", "title": "Follow-up, renamed"}}"#)).await;
    assert!(said.contains("\"updated\": true"), "{said}");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets?project={}&q={follow}", env.project), None).await;
    assert_eq!(events["items"][0]["title"], "Follow-up, renamed", "{events}");
    // Another ticket nobody works on, by its key.
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{todo_key}", "title": "Up next, clearer"}}"#)).await;
    assert!(said.contains("\"updated\": true"), "{said}");

    // Refused: one in progress or done it didn't create, and any other ticket by id.
    for (t, k, word) in [(&busy, &busy_key, "in progress"), (&done, &done_key, "done")] {
        let before = title(t).await;
        let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{k}", "title": "Overwritten"}}"#)).await;
        assert!(said.contains(word) && said.contains("add_ticket_comment"), "{said}");
        let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{k}", "description": "Gone."}}"#)).await;
        assert!(said.contains("add_ticket_comment"), "{said}");
        assert_eq!(title(t).await, before);
    }
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{todo}", "priority": "high"}}"#)).await;
    assert!(said.contains("exact key") && said.contains("add_ticket_comment"), "{said}");
    // Fields other than the text are fine by key.
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{busy_key}", "priority": "high"}}"#)).await;
    assert!(said.contains("\"updated\": true"), "{said}");
}

async fn builtin_workflow(env: &Env, id: &str) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, all) = call(&env.app, "GET", "/api/workflows", None).await;
    all.as_array().unwrap().iter().find(|w| w["builtin"] == id).cloned().unwrap_or_else(|| panic!("no {id}: {all}"))
}

/// PDT-366: a ticket an agent makes is a proposal: held in Draft without `agentic`, off the lists, in the Inbox with
/// who proposed it, from which ticket and why. Only the user's click decides it; the agent hears the decision in its
/// session while it runs, else on the ticket it came from. The user's tickets and Make into parts' skip it.
#[tokio::test]
async fn ticket_proposals_wait_for_the_user() {
    let env = setup().await;
    // A Bypass session's asking tools ask in its conversation (PDT-414), even where the project lets the PM act
    // (PDT-534): the user allows each (`allow_cards`).
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(builtin_workflow(&env, "ticket-proposals").await["enabled"], true, "on from the start");
    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Builder", "role": "coder", "agent": "claude", "instructions": "Build it.",
                    "permissionMode": "bypass",
                    "tools": ["get_ticket", "submit_outcome", "create_ticket", "edit_ticket"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (_, project) = call(&env.app, "GET", &format!("/api/projects/{}", env.project), None).await;
    let key = project["key"].as_str().unwrap().to_string();
    let own = ticket(&env, "Mine").await;
    let wf = create_workflow(
        &env,
        json!({
            "name": "Build",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "code", "kind": "agent", "version": 1, "config": {"persona": "Builder",
                    "prompt": "Build it.", "outcomes": [{"id": "submitted"}]}},
                {"id": "done", "kind": "end", "version": 1},
            ],
            "edges": [{"from": "go", "to": "code"}, {"from": "code", "port": "submitted", "to": "done"}],
        }),
    )
    .await;
    run_now(&env, &wf, json!({"ticketId": own})).await;
    let coder = tick_until(&env, "the coder", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{own}"), None).await;
        v["sessions"][0]["id"].as_str().map(|s| json!(s))
    })
    .await;
    let coder = coder.as_str().unwrap().to_string();
    let ask = |text: String| {
        let (env, coder) = (&env, coder.clone());
        async move {
            let n = agent_says(env, &coder).await.len();
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{coder}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
            tick_until(env, "the agent's answer", || async {
                allow_cards(env, &coder).await;
                agent_says(env, &coder).await.into_iter().skip(n).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    let proposal_of = |t: String| {
        let env = &env;
        async move {
            tick_until(env, "the proposal", || async {
                inbox(env).await.into_iter().find(|i| i["kind"] == "ticket_proposal" && i["ticketId"] == t.as_str())
            })
            .await
        }
    };
    let listed = || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
        v["items"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    let own_key =
        call(&env.app, "GET", &format!("/api/tickets/{own}"), None).await.1["key"].as_str().unwrap().to_string();

    // Made in To do with agentic, it is held in Draft without it, and the agent is told.
    let said = ask(format!(
        r#"mcp: create_ticket {{"project": "{key}", "title": "Flaky test", "stage": "todo", "labels": ["agentic", "bug"], "reason": "It failed twice in CI."}}"#
    ))
    .await;
    assert!(said.contains("\"proposal\": true") && said.contains("Inbox"), "{said}");
    let made = said.split("\"id\": \"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let made_key = said.split("\"key\": \"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{made}"), None).await;
    assert_eq!((t["stage"].as_str(), t["labels"].clone()), (Some("draft"), json!(["bug"])));
    let item = proposal_of(made.clone()).await;
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{made}"), None).await;
    assert_eq!(t["proposed"], true, "{t}");
    assert!(!listed().await.contains(&made), "off the board and lists");
    assert_eq!(item["group"], "ticket_proposal");
    assert_eq!(item["title"], "Flaky test");
    let b = &item["body"];
    assert_eq!(
        (b["reason"].as_str(), b["persona"].as_str()),
        (Some("It failed twice in CI."), Some("Builder")),
        "{item}"
    );
    assert_eq!((b["fromTicket"].as_str(), b["fromTicketId"].as_str()), (Some(own_key.as_str()), Some(own.as_str())));
    assert_eq!(item["sessionId"], coder.as_str());
    let choices: Vec<&str> = item["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["accept_todo", "accept_backlog", "duplicate", "reject"]);

    // The agent can't decide it: not by moving it, nor by answering the item.
    let said = ask(format!(r#"mcp: edit_ticket {{"ticket": "{made_key}", "stage": "todo"}}"#)).await;
    assert!(said.contains("ticket proposal"), "{said}");
    let answer = format!("/api/inbox/{}/answer", item["id"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &answer, Some(json!({"choice": "accept_todo"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["message"].as_str().unwrap().contains("only the user"), "{v}");

    // Accept to Todo with the user's edits and the agentic box: the agent hears it in its session.
    let body = json!({"choice": "accept_todo", "proposal": {"title": "Fix the flaky test", "priority": "high", "agentic": true}});
    let (s, v) = call_as_ui(&env.app, "POST", &answer, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{made}"), None).await;
    assert_eq!(
        (
            t["stage"].as_str(),
            t["title"].as_str(),
            t["priority"].as_str(),
            t["labels"].clone(),
            t["proposed"].as_bool()
        ),
        (Some("todo"), Some("Fix the flaky test"), Some("high"), json!(["agentic", "bug"]), Some(false)),
        "{t}"
    );
    assert!(listed().await.contains(&made));
    let heard = tick_until(&env, "the agent hearing it", || async {
        agent_took(&env, &coder)
            .await
            .into_iter()
            .find(|m| m.contains("accepted your ticket proposal"))
            .map(|m| json!(m))
    })
    .await;
    assert!(heard.as_str().unwrap().contains("to To do, labelled agentic"), "{heard}");
    let (s, _) = call_as_ui(&env.app, "POST", &answer, Some(json!({"choice": "reject"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "answered once");

    // Its session gone, the decision goes on the ticket it came from; curl with its header makes proposals too.
    call(&env.app, "POST", &format!("/api/sessions/{coder}/kill"), None).await;
    let new_ticket = format!("/api/projects/{}/tickets", env.project);
    let curl = |title: &str| {
        let (env, coder, new_ticket) = (&env, coder.clone(), new_ticket.clone());
        let body = json!({"title": title, "stage": "ready", "reason": "Found it."});
        async move {
            let (s, v) = call_with(&env.app, "POST", &new_ticket, Some(body), &[("x-agent-session", &coder)]).await;
            assert_eq!((s, v["stage"].as_str()), (StatusCode::CREATED, Some("draft")), "{v}");
            v["id"].as_str().unwrap().to_string()
        }
    };
    let rejected = curl("Rename everything").await;
    // Straight away, before the engine has put it in the Inbox: held, and an agent's curl can't move it on, label it
    // agentic or start it.
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{rejected}"), None).await;
    assert_eq!(t["proposed"], true, "held from its making: {t}");
    assert!(!inbox(&env).await.iter().any(|i| i["ticketId"] == rejected.as_str()), "no item yet");
    assert!(listed().await.contains(&rejected), "listed until it is in the Inbox, so it is never out of reach");
    let path = format!("/api/tickets/{rejected}");
    for body in [json!({"stage": "todo"}), json!({"stage": "ready"}), json!({"labels": ["agentic"]})] {
        let (s, v) = call(&env.app, "PATCH", &path, Some(body)).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("ticket_proposal")), "{v}");
    }
    let bulk = json!({"ids": [rejected], "action": "stage", "value": "todo"});
    let (s, v) = call(&env.app, "POST", "/api/tickets/bulk", Some(bulk)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("ticket_proposal")), "{v}");
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{rejected}/start"), Some(json!({"agent": "claude"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("ticket_proposal")), "{v}");
    let (_, t) = call(&env.app, "GET", &path, None).await;
    assert_eq!((t["stage"].as_str(), t["labels"].clone()), (Some("draft"), json!([])), "{t}");
    let item = proposal_of(rejected.clone()).await;
    assert!(!listed().await.contains(&rejected), "in the Inbox: off the lists");
    assert_eq!(item["body"]["reason"], "Found it.");
    let answer = format!("/api/inbox/{}/answer", item["id"].as_str().unwrap());
    let (s, v) =
        call_as_ui(&env.app, "POST", &answer, Some(json!({"choice": "reject", "text": "Not worth it."}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{rejected}"), None).await;
    assert_eq!(t["stage"], "wont_do");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{own}/events"), None).await;
    let text = events.to_string();
    assert!(text.contains("rejected your ticket proposal") && text.contains("Their reason: Not worth it."), "{events}");

    // Duplicate of…: Won't do, linked to the ticket it repeats.
    let dup = curl("Fix the flaky test again").await;
    let item = proposal_of(dup.clone()).await;
    let answer = format!("/api/inbox/{}/answer", item["id"].as_str().unwrap());
    let (s, v) = call_as_ui(&env.app, "POST", &answer, Some(json!({"choice": "duplicate"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let body = json!({"choice": "duplicate", "proposal": {"duplicateOf": made}});
    let (s, v) = call_as_ui(&env.app, "POST", &answer, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{dup}"), None).await;
    assert_eq!(t["stage"], "wont_do");
    assert_eq!(
        (t["links"][0]["kind"].as_str(), t["links"][0]["id"].as_str()),
        (Some("relates"), Some(made.as_str())),
        "{t}"
    );

    // The user's own tickets and Make into parts' parts are no proposals.
    let (_, v) = call_as_ui(&env.app, "POST", &new_ticket, Some(json!({"title": "Clicked", "stage": "todo"}))).await;
    assert_eq!((v["stage"].as_str(), v["proposed"].as_bool()), (Some("todo"), Some(false)), "{v}");
    let parts = format!("/api/tickets/{own}/parts");
    let body = json!({"parts": [{"title": "Part"}]});
    let (s, v) = call_with(&env.app, "POST", &parts, Some(body), &[("x-agent-session", &coder)]).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let part = v["items"][0]["id"].as_str().unwrap().to_string();
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
    }
    assert!(!inbox(&env).await.iter().any(|i| i["ticketId"] == part.as_str()), "parts skip it");
    assert!(listed().await.contains(&part));
}

/// PDT-475: the PM proposes decisions on many ticket proposals in one `inbox_decide` call. Nothing changes until the
/// user confirms: the call asks in the PM's conversation with a card of its rows, also in the Inbox (PDT-502), even
/// where the project lets the PM act. An agent can't allow it, a Deny changes nothing, and the user's Allow with rows
/// unticked (here from the Inbox) resolves only the rows they kept, as their own decisions with the PM kept as the
/// proposer; the waiting call returns what it did in the same turn (PDT-492). Another persona given the tool is refused.
#[tokio::test]
async fn the_pm_decides_ticket_proposals_in_a_batch_the_user_confirms() {
    let env = setup().await;
    let (s, p) = call(
        &env.app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Sneaky", "role": "coder", "agent": "claude", "instructions": "Build it.",
                    "permissionMode": "bypass", "tools": ["get_ticket", "submit_outcome", "inbox_decide"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let own = ticket(&env, "Mine").await;
    let wf = create_workflow(
        &env,
        json!({
            "name": "Build",
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "code", "kind": "agent", "version": 1, "config": {"persona": "Sneaky",
                    "prompt": "Build it.", "outcomes": [{"id": "submitted"}]}},
                {"id": "done", "kind": "end", "version": 1},
            ],
            "edges": [{"from": "go", "to": "code"}, {"from": "code", "port": "submitted", "to": "done"}],
        }),
    )
    .await;
    run_now(&env, &wf, json!({"ticketId": own})).await;
    let coder = tick_until(&env, "the coder", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{own}"), None).await;
        v["sessions"][0]["id"].as_str().map(|s| json!(s))
    })
    .await;
    let coder = coder.as_str().unwrap().to_string();
    let said_to = |session: String, text: String| {
        let env = &env;
        async move {
            let n = agent_says(env, &session).await.len();
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
            tick_until(env, "the agent's answer", || async {
                agent_says(env, &session).await.into_iter().skip(n).find(|m| m.starts_with("mcp ")).map(|m| json!(m))
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    // Three proposals from the coder's session.
    let mut made = vec![];
    for title in ["Flaky test", "Same flaky test", "Rename everything"] {
        let url = format!("/api/projects/{}/tickets", env.project);
        let body = json!({"title": title, "stage": "todo", "reason": "Found it."});
        let (s, v) = call_with(&env.app, "POST", &url, Some(body), &[("x-agent-session", &coder)]).await;
        assert_eq!((s, v["stage"].as_str()), (StatusCode::CREATED, Some("draft")), "{v}");
        made.push((v["id"].as_str().unwrap().to_string(), v["key"].as_str().unwrap().to_string()));
    }
    for (id, _) in &made {
        tick_until(&env, "the proposal", || async {
            inbox(&env).await.into_iter().find(|i| i["kind"] == "ticket_proposal" && i["ticketId"] == id.as_str())
        })
        .await;
    }
    let (a, b, c) = (&made[0], &made[1], &made[2]);
    let call_ = json!({"items": [
        {"item": a.1, "decision": "accept_todo", "agentic": true, "priority": "high", "note": "Worth fixing."},
        {"item": b.1, "decision": "duplicate", "duplicateOf": a.1},
        {"item": c.1, "decision": "reject", "note": "Not worth it."},
    ]});

    // Another persona given the tool can't use it.
    let said = said_to(coder.clone(), format!("mcp: inbox_decide {call_}")).await;
    assert!(said.contains("failed") && said.contains("project manager's tool"), "{said}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "confirm"), "nothing asked");

    // Mistakes are refused before anything is asked.
    let (s, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({"text": "hi"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let pm = v["id"].as_str().unwrap().to_string();
    let said =
        said_to(pm.clone(), r#"mcp: inbox_decide {"items": [{"item": "NOPE-1", "decision": "accept_todo"}]}"#.into())
            .await;
    assert!(said.contains("failed"), "{said}");
    let bad = json!({"items": [{"item": a.1, "decision": "accept_todo"}, {"item": a.1, "decision": "accept_backlog"}]});
    let said = said_to(pm.clone(), format!("mcp: inbox_decide {bad}")).await;
    assert!(said.contains("failed") && said.contains("twice"), "{said}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "confirm"), "nothing asked");

    // The PM's call asks in its conversation with a rows card (PDT-502) and waits for the user in the same turn, with
    // no prompt of Claude's and despite the project letting the PM act.
    let stage = |id: String| {
        let env = &env;
        async move { call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1 }
    };
    let open_card = || async {
        tick_until(&env, "the card", || async {
            let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{pm}/events?after=0"), None).await;
            v.as_array()
                .unwrap()
                .iter()
                .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
                .map(|e| e["event"].clone())
        })
        .await
    };
    let card_state = |rid: String| {
        let env = &env;
        let pm = pm.clone();
        async move {
            let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{pm}/events?after=0"), None).await;
            v.as_array().unwrap().iter().find(|e| e["event"]["id"] == rid.as_str()).unwrap()["event"]["state"].clone()
        }
    };
    let answer_url = |session: &str, rid: &str| format!("/api/sessions/{session}/requests/{rid}");
    let text = format!("mcp: inbox_decide {call_}");

    // Deny in the conversation: nothing changes, and the call hears it.
    let n = agent_says(&env, &pm).await.len();
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{pm}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let card = open_card().await;
    assert_eq!(card["tool"], "mcp__pdt__inbox_decide", "{card}");
    assert_eq!(card["canAlways"], false, "never always: {card}");
    let rows = card["rows"].as_array().unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r["ticketKey"].as_str().unwrap()).collect();
    assert_eq!(keys, [a.1.as_str(), b.1.as_str(), c.1.as_str()]);
    assert_eq!(
        (rows[0]["decision"].as_str(), rows[0]["ticketTitle"].as_str()),
        (Some("accept_todo"), Some("Flaky test"))
    );
    assert_eq!(rows[2]["note"], "Not worth it.");
    let items = inbox(&env).await;
    assert!(!items.iter().any(|i| i["kind"] == "confirm"), "no Inbox confirmation of its own: {items:#?}");
    let item = items.iter().find(|i| i["kind"] == "approval").expect("the card is in the Inbox too");
    assert_eq!(item["body"]["rows"], card["rows"], "the Inbox mirrors the card's rows");
    for (id, _) in &made {
        assert_eq!(stage(id.clone()).await["stage"], "draft", "nothing before the user's yes");
    }
    assert!(!agent_says(&env, &pm).await.iter().skip(n).any(|m| m.starts_with("mcp ")), "the call is waiting");
    let rid = card["id"].as_str().unwrap().to_string();
    let (s, v) = call_as_ui(&env.app, "POST", &answer_url(&pm, &rid), Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let heard = tick_until(&env, "the PM's call returning", || async {
        agent_says(&env, &pm).await.into_iter().skip(n).find(|m| m.starts_with("mcp inbox_decide")).map(|m| json!(m))
    })
    .await;
    let heard = heard.as_str().unwrap();
    assert!(heard.starts_with("mcp inbox_decide failed:") && heard.contains("denied"), "{heard}");
    for (id, _) in &made {
        assert_eq!(stage(id.clone()).await["stage"], "draft", "a Deny changes nothing");
    }
    assert_eq!(card_state(rid).await, "answered");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "approval"), "answered in both");

    // Answered from the Inbox: the user unticks the duplicate and allows the rest.
    let n = agent_says(&env, &pm).await.len();
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{pm}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let rid = open_card().await["id"].as_str().unwrap().to_string();
    let item = inbox(&env).await.into_iter().find(|i| i["kind"] == "approval").expect("in the Inbox");
    assert_eq!(item["body"]["requestId"].as_str(), Some(rid.as_str()));
    let url = answer_url(item["sessionId"].as_str().unwrap(), &rid);
    // An agent can't allow it, and the rows kept must be the card's.
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}, "rows": [0, 2]}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    for rows in [json!([]), json!([0, 3])] {
        let (s, v) =
            call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}, "rows": rows}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{rows}: {v}");
    }
    assert_eq!(card_state(rid.clone()).await, "open", "a wrong pick leaves the card open");
    let (s, v) = call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}, "rows": [0, 2]}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    // The PM's waiting call goes on and returns what it did.
    let heard = tick_until(&env, "the PM's call returning", || async {
        agent_says(&env, &pm).await.into_iter().skip(n).find(|m| m.starts_with("mcp inbox_decide")).map(|m| json!(m))
    })
    .await;
    let heard = heard.as_str().unwrap();
    assert!(heard.contains("\"unticked\"") && heard.contains(b.1.as_str()) && heard.contains("\"decided\""), "{heard}");
    assert!(!agent_took(&env, &pm).await.iter().any(|m| m.contains("[PDT] The user allowed")), "no message of its own");
    assert_eq!(card_state(rid).await, "answered", "the card is answered too");
    let t = stage(a.0.clone()).await;
    assert_eq!(
        (t["stage"].as_str(), t["priority"].as_str(), t["labels"].clone(), t["proposed"].as_bool()),
        (Some("todo"), Some("high"), json!(["agentic"]), Some(false)),
        "{t}"
    );
    assert_eq!(stage(c.0.clone()).await["stage"], "wont_do");
    assert_eq!(stage(b.0.clone()).await["stage"], "draft", "unticked: left as it was");
    let left: Vec<Value> = inbox(&env).await.into_iter().filter(|i| i["kind"] == "ticket_proposal").collect();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["ticketId"], b.0.as_str());

    // The decisions are the user's, proposed by the PM: on the ticket's history and the item's resolution.
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{}/events", a.0), None).await;
    let moved = events["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "moved" && e["data"]["to"] == "todo")
        .unwrap_or_else(|| panic!("no move: {events}"))
        .clone();
    assert_eq!(moved["actor"], "user", "{moved}");
    assert_eq!(
        (moved["proposedBy"]["persona"].as_str(), moved["proposedBy"]["sessionId"].as_str()),
        (Some("PM"), Some(pm.as_str())),
        "{moved}"
    );
    assert!(moved["agent"].is_null(), "not an agent's move: {moved}");
    let proposal = env.state.store.open_inbox_items_of(pdt_core::inbox::InboxKind::TicketProposal).await.unwrap();
    assert_eq!(proposal.len(), 1, "only the unticked one is open");
    // The proposing agent hears the user's decision as usual.
    let heard = tick_until(&env, "the coder hearing it", || async {
        agent_took(&env, &coder)
            .await
            .into_iter()
            .find(|m| m.contains("rejected your ticket proposal"))
            .map(|m| json!(m))
    })
    .await;
    assert!(heard.as_str().unwrap().contains("Their reason: Not worth it."), "{heard}");
}

/// A switched-off agent (PDT-270): Settings lists the agents still on and won't switch off the default; no new
/// session starts on it; a step whose persona uses it fails (no fallback), and validation warns ahead of time.
#[tokio::test]
async fn a_switched_off_agent_is_refused_everywhere_until_it_is_back_on() {
    let env = setup().await;
    let put = |body: Value| call(&env.app, "PUT", "/api/host/settings", Some(body));
    let (s, v) = put(json!({"disabledAgents": ["claude"]})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Claude is the default agent"), "{v}");
    let (s, v) = put(json!({"defaultAgent": "codex", "disabledAgents": ["claude"]})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let on: Vec<&str> = v["enabledAgents"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert!(!on.contains(&"claude") && on.contains(&"codex"), "{v}");

    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "claude", "mode": "conversation",
                      "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body.clone())).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("agent_off")), "{v}");
    assert_eq!(v["message"], "Claude is switched off in Settings → Agents");

    let doc = review_doc(r#"mcp: submit_outcome {"outcome": "approved", "summary": "Fine."}"#, json!({}));
    let (s, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let warning = v.as_array().unwrap().iter().find(|i| i["code"] == "agent_off").expect("a warning");
    assert_eq!(warning["severity"], "warning");
    assert!(warning["message"].as_str().unwrap().contains("Claude is switched off"), "{warning}");

    let t = ticket(&env, "Off").await;
    let wf = create_workflow(&env, doc.clone()).await;
    let id = run_now(&env, &wf, json!({"ticketId": t})).await;
    let r = run_ended(&env, &id).await;
    assert_eq!(r["status"], "failed", "{r:#}");
    let step = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "review").unwrap().clone();
    assert_eq!(step["error"], "Reviewer: Claude is switched off in Settings → Agents", "{step:#}");
    let (_, sessions) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
    assert_eq!(sessions.as_array().map(Vec::len), Some(0), "nothing started on another agent: {sessions}");

    // Back on: everything is.
    let (s, v) = put(json!({"disabledAgents": []})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert!(!v.as_array().unwrap().iter().any(|i| i["code"] == "agent_off"), "{v}");
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}
