//! PDT-501: a running session hears `notifications/tools/list_changed` when its pdt-mcp tools change (a plugin
//! turned on or off, its persona saved), on its `GET /api/mcp` stream or ahead of a POST's answer, once per change.

use crate::common::{self, call, call_with};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::sdk::{Integration, Tool};
use pdt_core::integration::ToolDecl;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionPersona, SessionStatus};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const NOTICE: &str = "notifications/tools/list_changed";

struct Fake;

impl Integration for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            decl: ToolDecl::new("fake_poke", "Poke the fake").pm(),
            title: "Poke the fake",
            description: "Pokes it.",
            schema: || json!({"type": "object"}),
        }]
    }
}

struct Env {
    app: axum::Router,
    bearer: String,
    persona: String,
    _tmp: tempfile::TempDir,
}

/// A session whose persona names `get_ticket` and the fake plugin's tool, with the plugin off.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    common::git(&repo, &["init", "-q", "-b", "main"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store.clone(), tmp.path());
    state.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(Fake)]));
    let app = pdt_api::router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    plugin(&app, true).await;
    let (s, persona) = call(
        &app,
        "POST",
        "/api/personas",
        Some(json!({"name": "Poker", "agent": "claude", "tools": ["get_ticket", "fake_poke"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{persona}");
    plugin(&app, false).await;
    let persona_id = persona["id"].as_str().unwrap().to_string();
    store
        .insert_session(&Session {
            id: "s1".into(),
            project_id: p["id"].as_str().unwrap().into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: "t".into(),
            cwd: repo.to_string_lossy().into(),
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
            persona: Some(SessionPersona { id: persona_id.clone(), name: "Poker".into(), version: 1, step_id: None }),
            background: vec![],
        })
        .await
        .unwrap();
    let bearer = format!("Bearer {}", pdt_api::mcp_token(&state, "s1").unwrap());
    Env { app, bearer, persona: persona_id, _tmp: tmp }
}

async fn plugin(app: &axum::Router, on: bool) {
    let enabled: Vec<&str> = if on { vec!["fake"] } else { vec![] };
    let (s, v) = call(app, "PUT", "/api/integrations", Some(json!({"enabled": enabled}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn tools(env: &Env) -> Vec<String> {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let (s, v) = call_with(&env.app, "POST", "/api/mcp", Some(rpc), &[("authorization", &env.bearer)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

/// The session's `GET /api/mcp` stream.
async fn listen(env: &Env) -> Body {
    let req = Request::get("/api/mcp")
        .header("authorization", &env.bearer)
        .header("accept", "text/event-stream")
        .body(Body::empty())
        .unwrap();
    let res = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    res.into_body()
}

/// The stream's next message, if one comes within `wait` (keep-alive comments skipped).
async fn next_message(body: &mut Body, wait: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let frame = tokio::time::timeout_at(deadline, body.frame()).await.ok()??.unwrap();
        let Ok(data) = frame.into_data() else { continue };
        let text = String::from_utf8_lossy(&data).to_string();
        if let Some(line) = text.lines().find_map(|l| l.strip_prefix("data:")) {
            return Some(serde_json::from_str(line.trim()).unwrap());
        }
    }
}

#[tokio::test]
async fn a_listening_session_hears_its_tools_changed_and_lists_the_new_ones() {
    let env = setup().await;
    let (s, init) = call_with(
        &env.app,
        "POST",
        "/api/mcp",
        Some(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})),
        &[("authorization", &env.bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{init}");
    assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true);
    assert_eq!(tools(&env).await, ["get_ticket"], "the plugin is off");

    let mut stream = listen(&env).await;
    assert_eq!(next_message(&mut stream, Duration::from_millis(300)).await, None, "nothing changed yet");

    // The plugin turned on: the session is told, and its next list has the plugin's tool.
    plugin(&env.app, true).await;
    let notice = next_message(&mut stream, Duration::from_secs(10)).await.expect("the notice");
    assert_eq!(notice, json!({"jsonrpc": "2.0", "method": NOTICE}));
    assert_eq!(tools(&env).await, ["get_ticket", "fake_poke"]);

    // A change that leaves its list as it is tells it nothing.
    plugin(&env.app, true).await;
    assert_eq!(next_message(&mut stream, Duration::from_millis(300)).await, None);

    // Its persona saved: it follows the persona's current version.
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/personas/{}", env.persona),
        Some(json!({"agent": "claude", "tools": ["fake_poke"], "note": "only poke"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let notice = next_message(&mut stream, Duration::from_secs(10)).await.expect("the notice");
    assert_eq!(notice["method"], NOTICE);
    assert_eq!(tools(&env).await, ["fake_poke"]);
}

#[tokio::test]
async fn a_post_after_a_change_is_answered_with_the_notice_first_once() {
    let env = setup().await;
    assert_eq!(tools(&env).await, ["get_ticket"]);
    plugin(&env.app, true).await;
    let ping = || async {
        let req = Request::post("/api/mcp")
            .header("authorization", &env.bearer)
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(Body::from(json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}).to_string()))
            .unwrap();
        let res = env.app.clone().oneshot(req).await.unwrap();
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        (ct, String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap())
    };
    let (ct, body) = ping().await;
    assert_eq!(ct, "text/event-stream");
    let messages: Vec<Value> =
        body.lines().filter_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect();
    assert_eq!(
        messages,
        [json!({"jsonrpc": "2.0", "method": NOTICE}), json!({"jsonrpc": "2.0", "id": 7, "result": {}})]
    );

    // Told once: the next call is answered plainly, before and after it lists them again.
    let (ct, body) = ping().await;
    assert_eq!((ct.as_str(), body.as_str()), ("application/json", r#"{"jsonrpc":"2.0","id":7,"result":{}}"#));
    assert_eq!(tools(&env).await, ["get_ticket", "fake_poke"]);
    assert_eq!(ping().await.0, "application/json");
}

#[tokio::test]
async fn the_stream_needs_the_token_and_an_event_stream_accept() {
    let env = setup().await;
    let get = |headers: Vec<(&'static str, String)>| {
        let mut req = Request::get("/api/mcp");
        for (k, v) in headers {
            req = req.header(k, v);
        }
        env.app.clone().oneshot(req.body(Body::empty()).unwrap())
    };
    let res = get(vec![("authorization", env.bearer.clone())]).await.unwrap();
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    let res = get(vec![("accept", "text/event-stream".into())]).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
