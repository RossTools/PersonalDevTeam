//! PDT-158 end to end: secrets agents can use but never see. `fake-agent` stands in for Claude (it calls pdt-mcp tools
//! when a message says `mcp: <tool> <json>`, and echoes anything else); pdtd's API is served on a local port so the
//! agent can reach `/api/mcp`. Each test greps the database for the value at the end.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const VALUE: &str = "npm_Zq8wX2-very-secret";

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    tmp: tempfile::TempDir,
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
    let mut state = AppState::new(store, tmp.path().join("data"));
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
    Env { app, state, project: p["id"].as_str().unwrap().into(), tmp }
}

/// Whether `value` is anywhere in the database's files (the WAL included).
fn in_database(env: &Env, value: &str) -> bool {
    std::fs::read_dir(env.tmp.path()).unwrap().flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.starts_with("pdt.db")
            && std::fs::read(e.path()).is_ok_and(|b| b.windows(value.len()).any(|w| w == value.as_bytes()))
    })
}

/// A conversation session on a new ticket; returns its id.
async fn session(env: &Env) -> String {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Publish"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{}/start", t["id"].as_str().unwrap()),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    started["session"]["id"].as_str().unwrap().to_string()
}

async fn events(env: &Env, session: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array().unwrap().clone()
}

async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    events(env, session)
        .await
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Sends `text` and waits for the agent's answer to it.
async fn say(env: &Env, session: &str, text: &str) -> String {
    let before = agent_says(env, session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    wait_said(env, session, before + 1).await
}

async fn wait_said(env: &Env, session: &str, n: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        // The engine's tick delivers the outbox (an Inbox answer's message).
        pdt_api::run_workflows(&env.state).await;
        let said = agent_says(env, session).await;
        if said.len() >= n {
            return said[n - 1].clone();
        }
        assert!(Instant::now() < deadline, "the agent said only {said:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn an_agent_asks_for_a_secret_and_uses_it_without_ever_seeing_it() {
    let env = setup().await;
    let s = session(&env).await;
    wait_said(&env, &s, 1).await;

    let said = say(&env, &s, r#"mcp: request_secret {"name": "NPM_TOKEN", "purpose": "Publish the package"}"#).await;
    assert!(said.contains("\"requested\": true"), "{said}");
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let item = inbox["items"].as_array().unwrap().iter().find(|i| i["kind"] == "secret").cloned().unwrap();
    assert_eq!((item["body"]["name"].as_str(), item["group"].as_str()), (Some("NPM_TOKEN"), Some("question")));
    // Too short to redact.
    let id = item["id"].as_str().unwrap();
    let (st, v) = call(&env.app, "POST", &format!("/api/inbox/{id}/answer"), Some(json!({"secret": "abc"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let n = agent_says(&env, &s).await.len();
    let (st, v) = call(&env.app, "POST", &format!("/api/inbox/{id}/answer"), Some(json!({"secret": VALUE}))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"name": "NPM_TOKEN", "stored": true}));
    // The agent hears it is stored (the fake echoes the message).
    let told = wait_said(&env, &s, n + 1).await;
    assert!(told.contains("The secret NPM_TOKEN is stored"), "{told}");

    let (_, list) = call(&env.app, "GET", "/api/secrets", None).await;
    let secret = list.as_array().unwrap()[0].clone();
    assert_eq!(secret["projects"], json!([env.project]), "the asking project's");
    assert_eq!(secret["description"], "Publish the package");
    assert!(!list.to_string().contains(VALUE));

    // Asked again, it is stored already.
    let said = say(&env, &s, r#"mcp: request_secret {"name": "npm_token", "purpose": "again"}"#).await;
    assert!(said.contains("\"stored\": true"), "{said}");
    let said = say(&env, &s, "mcp: list_secrets {}").await;
    assert!(said.contains("NPM_TOKEN"), "{said}");

    // A command that echoes it gets the placeholder back.
    let echo = if cfg!(windows) { "echo %TOK%" } else { "echo \"$TOK\"" };
    let call_text = json!({"command": echo, "env": {"TOK": "secret:NPM_TOKEN"}}).to_string();
    let said = say(&env, &s, &format!("mcp: run_with_secrets {call_text}")).await;
    assert!(said.contains("[secret:NPM_TOKEN]") && !said.contains(VALUE), "{said}");
    assert!(said.contains("\"exitCode\": 0"), "{said}");
    // Secrets go in env, never in the command line.
    let said = say(&env, &s, r#"mcp: run_with_secrets {"command": "echo {{secret.NPM_TOKEN}}"}"#).await;
    assert!(said.contains("failed") && said.contains("never in the command line"), "{said}");
    let said = say(&env, &s, r#"mcp: run_with_secrets {"command": "echo", "env": {"X": "secret:NOPE"}}"#).await;
    assert!(said.contains("no secret is called NOPE"), "{said}");

    // A file with it filled in; nothing of it comes back.
    let said = say(
        &env,
        &s,
        r#"mcp: write_secret_file {"path": ".npmrc", "template": "//registry/:_authToken={{secret.NPM_TOKEN}}\n"}"#,
    )
    .await;
    assert!(said.contains("\"bytes\"") && !said.contains(VALUE), "{said}");
    let (_, sess) = call(&env.app, "GET", &format!("/api/sessions/{s}"), None).await;
    let written = std::fs::read_to_string(std::path::Path::new(sess["cwd"].as_str().unwrap()).join(".npmrc")).unwrap();
    assert_eq!(written, format!("//registry/:_authToken={VALUE}\n"));

    // Pasted into the chat by accident: what is stored has the placeholder.
    let said = say(&env, &s, &format!("oops, it is {VALUE}")).await;
    assert_eq!(said, "echo: oops, it is [secret:NPM_TOKEN]");
    let all = serde_json::to_string(&events(&env, &s).await).unwrap();
    assert!(!all.contains(VALUE), "the transcript never holds it");
    assert!(all.contains("oops, it is [secret:NPM_TOKEN]"));

    // The audit list.
    let sid = secret["id"].as_str().unwrap();
    let (_, uses) = call(&env.app, "GET", &format!("/api/secrets/{sid}/uses"), None).await;
    let how: Vec<&str> = uses.as_array().unwrap().iter().map(|u| u["how"].as_str().unwrap()).collect();
    assert_eq!(how, ["write_secret_file", "run_with_secrets"], "{uses}");
    assert_eq!(uses[0]["sessionId"], json!(s));
    let (_, list) = call(&env.app, "GET", "/api/secrets", None).await;
    assert!(list[0]["lastUsedAt"].is_number() && list[0]["lastUsedBy"].is_string(), "{list}");

    // A comment holding it keeps the placeholder.
    let (_, t) = call(&env.app, "GET", &format!("/api/sessions/{s}"), None).await;
    let ticket = t["ticket"]["id"].as_str().unwrap().to_string();
    let (st, c) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/comments"),
        Some(json!({"body": format!("token {VALUE}")})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    assert!(!c.to_string().contains(VALUE), "{c}");

    assert!(!in_database(&env, VALUE), "the database never holds the value");
}

#[tokio::test]
async fn secrets_are_kept_listed_replaced_and_deleted_from_settings() {
    let env = setup().await;
    let (st, v) = call(&env.app, "POST", "/api/secrets", Some(json!({"name": "1bad", "value": VALUE}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) = call(&env.app, "POST", "/api/secrets", Some(json!({"name": "API", "value": "short"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, v) =
        call(&env.app, "POST", "/api/secrets", Some(json!({"name": "API", "value": VALUE, "projects": ["nope"]})))
            .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let (st, s) = call(
        &env.app,
        "POST",
        "/api/secrets",
        Some(json!({"name": "API", "value": VALUE, "description": "The API key", "projects": [env.project]})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{s}");
    assert!(!s.to_string().contains(VALUE));
    let (st, v) = call(&env.app, "POST", "/api/secrets", Some(json!({"name": "api", "value": VALUE}))).await;
    assert_eq!((st, v["code"].as_str()), (StatusCode::CONFLICT, Some("secret_exists")));
    let id = s["id"].as_str().unwrap();
    assert_eq!(env.state.secrets.get(&format!("secret:{id}")).unwrap().as_deref(), Some(VALUE), "in the keychain");

    let (st, p) = call(
        &env.app,
        "PATCH",
        &format!("/api/secrets/{id}"),
        Some(json!({"value": "replaced-value", "projects": []})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["projects"], json!([]));
    assert_eq!(env.state.secrets.get(&format!("secret:{id}")).unwrap().as_deref(), Some("replaced-value"));
    assert_eq!(env.state.store.redactor().string("x replaced-value"), "x [secret:API]");

    let (st, _) = call(&env.app, "DELETE", &format!("/api/secrets/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(env.state.secrets.get(&format!("secret:{id}")).unwrap(), None, "gone from the keychain");
    let (_, list) = call(&env.app, "GET", "/api/secrets", None).await;
    assert_eq!(list, json!([]));
}

/// A local server that wants `Authorization: Bearer <VALUE>` and `?key=<VALUE>`, and echoes the token back.
async fn api_server() -> String {
    async fn handle(
        headers: axum::http::HeaderMap,
        axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        if auth != format!("Bearer {VALUE}") || q.get("key").map(String::as_str) != Some(VALUE) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        axum::Json(json!({"ok": true, "youSent": auth})).into_response()
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route("/me", axum::routing::get(handle));
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

async fn run_workflow(env: &Env, doc: Value) -> Value {
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await;
        if !matches!(r["status"].as_str(), Some("running" | "waiting")) {
            return r;
        }
        assert!(Instant::now() < deadline, "the run never ended: {r}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn step(r: &Value, node: &str) -> Value {
    r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == node).cloned().unwrap()
}

#[tokio::test]
async fn a_workflow_request_authenticates_with_a_secret_and_stores_none() {
    let env = setup().await;
    let url = api_server().await;
    let (st, s) =
        call(&env.app, "POST", "/api/secrets", Some(json!({"name": "API", "value": VALUE, "projects": [env.project]})))
            .await;
    assert_eq!(st, StatusCode::CREATED, "{s}");
    let doc = json!({
        "name": "Call the API",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "call", "kind": "http.request", "version": 1, "config": {
                "url": format!("{url}/me"),
                "headers": {"Authorization": "Bearer {{secret.API}}"},
                "query": {"key": {"$secret": "API"}}}},
        ],
        "edges": [{"from": "go", "to": "call"}],
    });
    let r = run_workflow(&env, doc.clone()).await;
    assert_eq!(r["status"], "succeeded", "{r:#}");
    let call_step = step(&r, "call");
    assert_eq!(call_step["output"]["status"], 200, "{call_step:#}");
    assert_eq!(call_step["output"]["body"]["youSent"], "Bearer [secret:API]", "the echo is redacted");
    assert_eq!(
        call_step["input"]["headers"]["Authorization"], "Bearer {{secret.API}}",
        "the input keeps the reference"
    );
    assert_eq!(call_step["input"]["query"]["key"], json!({"$secret": "API"}));
    let (_, uses) = call(&env.app, "GET", &format!("/api/secrets/{}/uses", s["id"].as_str().unwrap()), None).await;
    assert_eq!(uses[0]["how"], "http.request", "{uses}");
    assert!(uses[0]["by"].as_str().unwrap().contains("Call the API"), "{uses}");

    // A secret can't go to an agent.
    let agent = json!({
        "name": "Leak",
        "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "a", "kind": "agent", "version": 1, "config": {"persona": "Coder",
                "prompt": "the key is {{secret.API}}", "outcomes": [{"id": "done"}]}},
        ],
        "edges": [{"from": "go", "to": "a"}],
    });
    let r = run_workflow(&env, agent).await;
    assert_eq!(r["status"], "failed", "{r:#}");
    assert!(step(&r, "a")["error"].as_str().unwrap().contains("secrets can't go in it"), "{r:#}");

    // Nor to a project it is not for.
    let (_, other) = call(
        &env.app,
        "PATCH",
        &format!("/api/secrets/{}", s["id"].as_str().unwrap()),
        Some(json!({"projects": [env.project]})),
    )
    .await;
    assert_eq!(other["projects"], json!([env.project]));
    let other_repo = env.tmp.path().join("other");
    std::fs::create_dir_all(&other_repo).unwrap();
    git(&other_repo, &["init", "-q", "-b", "main"]);
    git(&other_repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let (_, q) =
        call(&env.app, "POST", "/api/projects", Some(json!({"name": "Q", "repoPath": other_repo.to_string_lossy()})))
            .await;
    let (st, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": q["id"], "enabled": true})))
            .await;
    assert_eq!(st, StatusCode::CREATED, "{w}");
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", w["id"].as_str().unwrap()),
        Some(json!({"input": {}})),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    let r = loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await;
        if !matches!(r["status"].as_str(), Some("running" | "waiting")) {
            break r;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(r["status"], "failed", "{r:#}");
    assert!(step(&r, "call")["error"].as_str().unwrap().contains("may not be used here"), "{r:#}");

    assert!(!in_database(&env, VALUE), "the database never holds the value");
}
