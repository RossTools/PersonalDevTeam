//! PDT-158 end to end: secrets agents can use but never see. `fake-agent` stands in for Claude (it calls pdt-mcp tools
//! when a message says `mcp: <tool> <json>`, and echoes anything else); pdtd's API is served on a local port so the
//! agent can reach `/api/mcp`. Each test greps the database for the value at the end.

use crate::common;

use axum::http::StatusCode;
use common::wait::wait_until;
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
    session_in(env, "ask").await
}

/// [`session`] in a permission mode.
async fn session_in(env: &Env, mode: &str) -> String {
    // As the user: an agent-made ticket after the first waits as a proposal.
    let (s, t) = common::call_as_ui(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": "Publish"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{}/start", t["id"].as_str().unwrap()),
        Some(json!({"agent": "claude", "sessionMode": "conversation", "permissionMode": mode})),
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

/// Sends a call to an asking tool, answers the permission request it opens (`approve` or `deny`) and waits for the
/// agent's answer (PDT-404).
async fn say_answering(env: &Env, session: &str, text: &str, answer: &str) -> String {
    let before = agent_says(env, session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let deadline = Instant::now() + Duration::from_secs(15);
    let request = loop {
        let open = events(env, session)
            .await
            .into_iter()
            .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
            .map(|e| e["event"]["id"].as_str().unwrap().to_string());
        if let Some(id) = open {
            break id;
        }
        assert!(Instant::now() < deadline, "no permission request: {:?}", agent_says(env, session).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{session}/requests/{request}"),
        Some(json!({"answer": {"type": answer}})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    wait_said(env, session, before + 1).await
}

async fn say_allowed(env: &Env, session: &str, text: &str) -> String {
    say_answering(env, session, text, "approve").await
}

async fn wait_said(env: &Env, session: &str, n: usize) -> String {
    wait_until(&format!("the agent to say {n} things"), async || {
        // The engine's tick delivers the outbox (an Inbox answer's message).
        pdt_api::run_workflows(&env.state).await;
        let said = agent_says(env, session).await;
        if said.len() >= n { Ok(said[n - 1].clone()) } else { Err(said) }
    })
    .await
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
    let said = say_allowed(&env, &s, &format!("mcp: run_with_secrets {call_text}")).await;
    assert!(said.contains("[secret:NPM_TOKEN]") && !said.contains(VALUE), "{said}");
    assert!(said.contains("\"exitCode\": 0"), "{said}");
    // Encoded, it is replaced too.
    if cfg!(unix) {
        let call_text =
            json!({"command": "printf %s \"$TOK\" | base64", "env": {"TOK": "secret:NPM_TOKEN"}}).to_string();
        let said = say_allowed(&env, &s, &format!("mcp: run_with_secrets {call_text}")).await;
        assert!(said.contains("[secret:NPM_TOKEN]") && !said.contains("bnBtX1pxOHdYMi12ZXJ5LXNlY3JldA"), "{said}");
    }
    // Secrets go in env, never in the command line.
    let said = say_allowed(&env, &s, r#"mcp: run_with_secrets {"command": "echo {{secret.NPM_TOKEN}}"}"#).await;
    assert!(said.contains("failed") && said.contains("never in the command line"), "{said}");
    let said = say_allowed(&env, &s, r#"mcp: run_with_secrets {"command": "echo", "env": {"X": "secret:NOPE"}}"#).await;
    assert!(said.contains("no secret is called NOPE"), "{said}");

    // A file with it filled in; nothing of it comes back.
    let said = say_allowed(
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
    let runs = if cfg!(unix) { 2 } else { 1 };
    assert_eq!(how[0], "write_secret_file", "{uses}");
    assert_eq!(how[1..], vec!["run_with_secrets"; runs], "{uses}");
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

/// PDT-404: the tools run in PDT, outside the agent's sandbox, so the user allows each call (unless the session asks
/// before nothing anyway), each must use a secret, and files stay in the session's folder and out of `.git`.
#[tokio::test]
async fn secret_tools_ask_first_use_a_secret_and_stay_in_the_checkout() {
    let env = setup().await;
    let (st, v) = call(
        &env.app,
        "POST",
        "/api/secrets",
        Some(json!({"name": "TOKEN", "value": VALUE, "projects": [env.project]})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");

    // Asked, and denied: nothing runs.
    let s = session(&env).await;
    wait_said(&env, &s, 1).await;
    let cwd = |s: String| {
        let env = &env;
        async move {
            let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{s}"), None).await;
            std::path::PathBuf::from(v["cwd"].as_str().unwrap())
        }
    };
    let dir = cwd(s.clone()).await;
    let touch = if cfg!(windows) { "type nul > ran.txt" } else { "touch ran.txt" };
    let run = json!({"command": touch, "env": {"T": "secret:TOKEN"}}).to_string();
    let said = say_answering(&env, &s, &format!("mcp: run_with_secrets {run}"), "deny").await;
    assert!(said.contains("denied"), "{said}");
    assert!(!dir.join("ran.txt").exists());
    // Allowed, it runs.
    let said = say_allowed(&env, &s, &format!("mcp: run_with_secrets {run}")).await;
    assert!(said.contains("\"exitCode\": 0"), "{said}");
    assert!(dir.join("ran.txt").exists());

    // A session that bypasses permissions isn't asked, but the rules hold.
    let b = session_in(&env, "bypass").await;
    wait_said(&env, &b, 1).await;
    let dir = cwd(b.clone()).await;
    let no_secret = json!({"command": touch}).to_string();
    let said = say(&env, &b, &format!("mcp: run_with_secrets {no_secret}")).await;
    assert!(said.contains("failed") && said.contains("name a secret"), "{said}");
    assert!(!dir.join("ran.txt").exists());
    let outside = env.tmp.path().join("elsewhere");
    let out_run = json!({"command": touch, "env": {"T": "secret:TOKEN"}, "cwd": outside.to_string_lossy()});
    let said = say(&env, &b, &format!("mcp: run_with_secrets {out_run}")).await;
    assert!(said.contains("failed") && said.contains("outside"), "{said}");

    let write = |path: &str, template: &str| format!("mcp: {}", json!({"path": path, "template": template}));
    let write = |path: &str, template: &str| write(path, template).replacen("mcp: ", "mcp: write_secret_file ", 1);
    let said = say(&env, &b, &write("plain.txt", "no secret here")).await;
    assert!(said.contains("failed") && said.contains("name a secret"), "{said}");
    for bad in [".git/config", "sub/.git/hooks/pre-commit", "../escape.env"] {
        let said = say(&env, &b, &write(bad, "k={{secret.TOKEN}}")).await;
        assert!(said.contains("failed"), "{bad}: {said}");
    }
    let zshrc = env.tmp.path().join(".zshrc");
    let said = say(&env, &b, &write(&zshrc.to_string_lossy(), "k={{secret.TOKEN}}")).await;
    assert!(said.contains("failed") && said.contains("outside"), "{said}");
    assert!(!zshrc.exists() && !env.tmp.path().join("escape.env").exists());
    let config = std::fs::read_to_string(dir.join(".git").join("config")).unwrap_or_default();
    assert!(!config.contains(VALUE));

    // A symlink can't lead it out, as a folder or as the file.
    #[cfg(unix)]
    {
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("target.env"), dir.join("file.env")).unwrap();
        std::os::unix::fs::symlink(dir.join(".git"), dir.join("gitlink")).unwrap();
        for bad in ["link/a.env", "link/new/a.env", "file.env", "gitlink/config"] {
            let said = say(&env, &b, &write(bad, "k={{secret.TOKEN}}")).await;
            assert!(said.contains("failed"), "{bad}: {said}");
        }
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing written outside");
    }

    // Inside, it is written.
    let said = say(&env, &b, &write("conf/app.env", "k={{secret.TOKEN}}")).await;
    assert!(said.contains("\"bytes\""), "{said}");
    assert_eq!(std::fs::read_to_string(dir.join("conf").join("app.env")).unwrap(), format!("k={VALUE}"));
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

/// PDT-413: a connection's secret, kept by PDT rather than the user's secrets, is redacted like theirs.
#[tokio::test]
async fn a_connections_secret_is_redacted_too() {
    let env = setup().await;
    let (st, c) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "hook", "kind": "http"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap();
    let (st, _) = call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": VALUE}))).await;
    assert!(st.is_success());
    assert_eq!(env.state.store.redactor().string(&format!("got {VALUE}")), "got [secret:connection]");
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
    run_workflow_with(env, doc, json!({})).await
}

/// A local server that answers `POST /echo` with what it got: its JSON body and its `Authorization` header.
async fn echo_server() -> String {
    async fn handle(headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        axum::Json(json!({"body": body, "auth": auth}))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route("/echo", axum::routing::post(handle));
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

async fn run_workflow_with(env: &Env, doc: Value, input: Value) -> Value {
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
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

/// Only references the node's own config holds are filled in: one that comes in as data (here the run's trigger, as a
/// ticket's text, a webhook's payload or an earlier step's output would) is passed on as written, and one naming no
/// secret doesn't fail the step.
#[tokio::test]
async fn secret_references_that_come_in_as_data_are_neither_filled_nor_an_error() {
    let env = setup().await;
    let url = echo_server().await;
    let (st, s) =
        call(&env.app, "POST", "/api/secrets", Some(json!({"name": "API", "value": VALUE, "projects": [env.project]})))
            .await;
    assert_eq!(st, StatusCode::CREATED, "{s}");
    let data = json!({"text": "send {{secret.API}} and {{secret.NOPE}}", "obj": {"$secret": "API"}});
    let forward = |own: bool| {
        let mut config = json!({"url": format!("{url}/echo"), "method": "POST",
            "body": {"text": "{{trigger.text}}", "obj": "{{trigger.obj}}"}});
        if own {
            config["headers"] = json!({"Authorization": "Bearer {{secret.API}}"});
        }
        json!({
            "name": if own { "Forward with a secret" } else { "Forward" },
            "nodes": [
                {"id": "go", "kind": "manual", "version": 1},
                {"id": "call", "kind": "http.request", "version": 1, "config": config},
            ],
            "edges": [{"from": "go", "to": "call"}],
        })
    };
    for own in [false, true] {
        let r = run_workflow_with(&env, forward(own), data.clone()).await;
        assert_eq!(r["status"], "succeeded", "{r:#}");
        let got = &step(&r, "call")["output"]["body"];
        assert_eq!(got["body"]["text"], "send {{secret.API}} and {{secret.NOPE}}", "{got:#}");
        assert_eq!(got["body"]["obj"], json!({"$secret": "API"}), "{got:#}");
        let auth = if own { "Bearer [secret:API]" } else { "" };
        assert_eq!(got["auth"], auth, "only the node's own reference is filled (the echo is redacted)");
    }
    let (_, uses) = call(&env.app, "GET", &format!("/api/secrets/{}/uses", s["id"].as_str().unwrap()), None).await;
    assert_eq!(uses.as_array().unwrap().len(), 1, "only the run whose node names it used it: {uses}");
    assert!(!in_database(&env, VALUE));
}
