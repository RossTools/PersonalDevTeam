//! The PM's end-to-end helpers (PDT-81), shared by pdt-api's `pm` tests and the integration crates' tests of their
//! PM tools: a project served on a free port with `fake-agent` as every agent, starting a PM conversation, and
//! reading what it said and asked.

use super::wait::wait_until;
use super::{call, git};
use axum::http::StatusCode;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

pub struct Env {
    pub app: axum::Router,
    pub state: AppState,
    pub project: String,
    pub gh: std::sync::Arc<pdt_forge::testing::FakeForge>,
    pub _tmp: tempfile::TempDir,
}

/// A project `P` (key `PX`) with one ticket, served on a free local port (the agent's pdt-mcp calls come over TCP).
pub async fn setup() -> Env {
    setup_with(|_| {}).await
}

/// `setup`, with `tweak` applied to the state before it is served.
pub async fn setup_with(tweak: impl FnOnce(&mut AppState)) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = super::app_state(store, tmp.path());
    let fake = super::fake_agent();
    state.platform.claude_program = fake.clone();
    state.platform.codex_program = fake.clone();
    state.platform.devin_program = fake;
    let devin_config = tmp.path().join("devin-user").join("config.json");
    std::fs::create_dir_all(devin_config.parent().unwrap()).unwrap();
    std::fs::write(&devin_config, "{}").unwrap();
    state.platform.devin_config_file = Some(devin_config);
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let gh = std::sync::Arc::new(pdt_forge::testing::FakeForge::default());
    state.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::testing::fake_github(&gh))]);
    tweak(&mut state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "P", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (s, t) = call(
        &app,
        "POST",
        &format!("/api/projects/{project}/tickets"),
        Some(json!({"title": "Fix the login", "stage": "todo"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    Env { app, state, project, gh, _tmp: tmp }
}

pub async fn start_pm(env: &Env, text: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

pub async fn send(env: &Env, session: &str, text: &str) {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

pub async fn events(env: &Env, session: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array().unwrap().clone()
}

pub async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    events(env, session)
        .await
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Waits until the agent has said `n` things (ticking the engine, which delivers outbox messages); returns them.
pub async fn wait_said(env: &Env, session: &str, n: usize) -> Vec<String> {
    wait_until(&format!("the agent to say {n} things"), async || {
        let said = agent_says(env, session).await;
        if said.len() >= n {
            return Ok(said);
        }
        pdt_api::run_workflows(&env.state).await;
        Err(said)
    })
    .await
}

/// Sends a message and returns what the agent said to it.
pub async fn ask(env: &Env, session: &str, text: &str) -> String {
    let n = agent_says(env, session).await.len();
    send(env, session, text).await;
    wait_said(env, session, n + 1).await.pop().unwrap()
}

pub async fn open_request(env: &Env, session: &str) -> String {
    open_request_except(env, session, &[]).await
}

/// The open request that isn't one of `answered`: a request just answered can still read as open for a moment (a slow
/// runner), and answering it again would miss the new one.
pub async fn open_request_except(env: &Env, session: &str, answered: &[String]) -> String {
    wait_until("an open request", async || {
        let all = events(env, session).await;
        let fresh = |e: &&Value| {
            e["event"]["kind"] == "request"
                && e["event"]["state"] == "open"
                && !answered.iter().any(|a| e["event"]["id"] == a.as_str())
        };
        all.iter().find(fresh).map(|e| e["event"]["id"].as_str().unwrap().to_string())
    })
    .await
}

pub async fn ticket_titles(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let mut t: Vec<String> =
        v["items"].as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().into()).collect();
    t.sort();
    t
}

pub fn instructions(env: &Env, session: &str) -> String {
    std::fs::read_to_string(env.state.data_dir.join("sessions").join(session).join("instructions.md")).unwrap()
}
