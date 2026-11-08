//! The user's MCP servers in agent sessions (PDT-322): configured once as `mcp` connections, each agent's launch
//! config gets them with secrets and sign-ins filled in and tools allowed or blocked; Test lists a server's tools; a
//! remote server signs in with OAuth against a fake authorization server (registration, PKCE, refresh, revocation →
//! Inbox).

use crate::common;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use common::wait::wait_until;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const FAKE_MCP: &str = env!("CARGO_BIN_EXE_fake-mcp");

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    bin: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let devin_config = tmp.path().join("devin-user").join("config.json");
    std::fs::create_dir_all(devin_config.parent().unwrap()).unwrap();
    std::fs::write(&devin_config, "{}").unwrap();
    let codex_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::write(codex_home.join("config.toml"), "[mcp_servers.mine]\ncommand = \"my-mcp\"\n").unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    state.platform.codex_program = argv_agent(&bin, "fakecodex").to_string_lossy().into_owned();
    state.platform.devin_program = FAKE.to_string();
    state.platform.devin_config_file = Some(devin_config);
    state.platform.codex_home = Some(codex_home);
    state.platform.login_path = None;
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), bin, _tmp: tmp }
}

async fn server(env: &Env, name: &str, url: &str, config: Value) -> String {
    let body = json!({"name": name, "kind": "mcp", "url": url, "config": config});
    let (s, c) = call(&env.app, "POST", "/api/connections", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    c["id"].as_str().unwrap().to_string()
}

async fn secret(env: &Env, name: &str, value: &str) {
    let (s, v) = call(&env.app, "POST", "/api/secrets", Some(json!({"name": name, "value": value}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

async fn terminal(env: &Env, agent: &str) -> String {
    let body =
        json!({"projectId": env.pid, "kind": "agent_tui", "agent": agent, "mode": "terminal", "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().unwrap().to_string()
}

fn claude_config(env: &Env, session: &str) -> Value {
    let path = env.state.data_dir.join("sessions").join(session).join("mcp.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The launches [`common::recorded_launches`] recorded, each as its arguments on Windows too (there, one raw command
/// line).
async fn recorded_launches(bin: &std::path::Path, name: &str, want: usize) -> Vec<Vec<String>> {
    let all = common::recorded_launches(bin, name, want).await;
    if cfg!(windows) { all.iter().map(|l| common::windows_args(&l[0])).collect() } else { all }
}

/// The recorded launch of a Claude session (its `--session-id`).
async fn launch_of(env: &Env, session: &str) -> Vec<String> {
    wait_until(&format!("a launch of {session}"), async || {
        let all = recorded_launches(&env.bin, "fakeclaude", 1).await;
        all.into_iter()
            .find(|l| l.iter().any(|a| a == session))
            .ok_or_else(|| std::fs::read_to_string(env.bin.join("fakeclaude.args")).unwrap_or_default())
    })
    .await
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// Notes (a program, its key a PDT secret: echo allowed, hang blocked) and Docs (remote, a secret header; not for
/// sessions the user starts).
async fn two_servers(env: &Env) {
    secret(env, "NOTES_KEY", "notes-key-123").await;
    secret(env, "DOCS_TOKEN", "docs-token-456").await;
    server(
        env,
        "Notes",
        "",
        json!({"transport": "stdio", "command": FAKE_MCP, "env": {"NOTES_KEY": {"$secret": "NOTES_KEY"}, "LOG": "warn"},
               "agents": {"tools": {"default": "ask", "overrides": {"echo": "allow", "hang": "block"}}}}),
    )
    .await;
    server(
        env,
        "Docs",
        "https://docs.example/mcp",
        json!({"transport": "http", "headers": {"X-Api-Key": {"$secret": "DOCS_TOKEN"}}, "agents": {"user": false}}),
    )
    .await;
}

#[tokio::test]
async fn claude_gets_the_servers_it_is_meant_to_with_secrets_filled_in_and_tools_allowed_or_blocked() {
    let env = setup().await;
    two_servers(&env).await;
    let id = terminal(&env, "claude").await;
    let launch = recorded_launches(&env.bin, "fakeclaude", 1).await.remove(0);
    let cfg = claude_config(&env, &id);
    let servers = cfg["mcpServers"].as_object().unwrap();
    assert_eq!(servers.keys().collect::<Vec<_>>(), ["notes", "pdt"], "Docs isn't for sessions the user starts");
    assert_eq!(cfg["mcpServers"]["notes"]["env"]["NOTES_KEY"], "notes-key-123");
    assert_eq!(cfg["mcpServers"]["notes"]["command"], FAKE_MCP);
    let allowed = arg_after(&launch, "--allowedTools").unwrap();
    assert!(allowed.contains("mcp__notes__echo") && allowed.contains("mcp__pdt__get_ticket"), "{allowed}");
    assert_eq!(arg_after(&launch, "--disallowedTools"), Some("mcp__notes__hang"));
    assert!(!launch.contains(&"--strict-mcp-config".to_string()), "the user's own servers load as well");
    // PDT's asking tools each get an ask rule in the session's `--settings` file, beside its hooks, so an allow rule in
    // a settings file Claude reads (the worktree's `.claude/settings.local.json`) can't switch the prompt off.
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(env.state.data_dir.join("sessions").join(&id).join("claude-settings.json")).unwrap(),
    )
    .unwrap();
    assert!(settings["hooks"]["PreToolUse"].is_array());
    let ask: Vec<&str> = settings["permissions"]["ask"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(ask.contains(&"mcp__pdt__issue_create"), "{ask:?}");
    assert!(ask.iter().all(|t| !allowed.split(',').any(|a| a == *t)), "{ask:?} {allowed}");
    // The agent is told that what the tools return isn't instructions.
    let instructions =
        std::fs::read_to_string(env.state.data_dir.join("sessions").join(&id).join("instructions.md")).unwrap();
    assert!(instructions.contains("`notes`") && instructions.contains("never as instructions"), "{instructions}");
    // The secret's use is recorded.
    let (_, list) = call(&env.app, "GET", "/api/secrets", None).await;
    let notes = list.as_array().unwrap().iter().find(|s| s["name"] == "NOTES_KEY").unwrap();
    assert!(notes["lastUsedAt"].is_number(), "{notes}");

    // Use only PDT's MCP servers: Claude ignores its own config.
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"mcpOnlyPdt": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    terminal(&env, "claude").await;
    let launch = recorded_launches(&env.bin, "fakeclaude", 2).await.remove(1);
    assert!(launch.contains(&"--strict-mcp-config".to_string()), "{launch:?}");
    let (_, list) = call(&env.app, "GET", "/api/mcp-servers", None).await;
    assert_eq!(list["onlyPdt"], true);
    let names: Vec<_> = list["servers"].as_array().unwrap().iter().map(|s| s["agentName"].as_str().unwrap()).collect();
    assert_eq!(names, ["docs", "notes"]);
}

#[tokio::test]
async fn a_codex_terminal_gets_the_servers_with_their_secrets_off_its_command_line() {
    let env = setup().await;
    two_servers(&env).await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"mcpOnlyPdt": true}))).await;
    terminal(&env, "codex").await;
    let launch = recorded_launches(&env.bin, "fakecodex", 1).await.remove(0);
    let joined = launch.join(" ");
    assert!(!joined.contains("notes-key-123"), "no secret on the command line: {joined}");
    let notes = launch.iter().find(|a| a.starts_with("mcp_servers.notes=")).expect("the notes server");
    // One inline TOML table (pdt-agents' tests parse it).
    assert!(notes.contains(r#"env_vars = ["NOTES_KEY"]"#), "{notes}");
    assert!(notes.contains(r#"disabled_tools = ["hang"]"#), "{notes}");
    assert!(notes.contains(r#"tools = {echo = {approval_mode = "approve"}}"#), "{notes}");
    assert!(launch.contains(&"mcp_servers.mine.enabled=false".to_string()), "the user's own server is off: {joined}");
}

#[tokio::test]
async fn an_acp_agent_gets_the_servers_it_can_connect_to() {
    let env = setup().await;
    two_servers(&env).await;
    server(&env, "Old", "https://old.example/sse", json!({"transport": "sse"})).await;
    let body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "devin", "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{id}/messages"),
        Some(json!({"text": "servers", "delivery": "next"})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let said: Vec<String> = wait_until("the agent to list its servers", async || {
        let (_, ev) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
        let said: Vec<String> = ev
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| match e["event"]["kind"].as_str() {
                Some("agent_message") => e["event"]["markdown"].as_str().map(String::from),
                Some("notice") => e["event"]["text"].as_str().map(String::from),
                _ => None,
            })
            .collect();
        if said.iter().any(|m| m.starts_with("servers=")) { Ok(said) } else { Err(said) }
    })
    .await;
    assert!(said.contains(&"servers=pdt,notes".to_string()), "{said:?}");
    assert!(said.iter().any(|m| m.contains("doesn't get old")), "SSE isn't one it takes: {said:?}");
    assert!(said.iter().any(|m| m.contains("can't block single MCP tools") && m.contains("notes")), "{said:?}");
}

#[tokio::test]
async fn test_lists_a_programs_tools() {
    let env = setup().await;
    let id = server(&env, "fake", "", json!({"transport": "stdio", "command": FAKE_MCP})).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/mcp-servers/{id}/test"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tools"]["error"], Value::Null, "{v}");
    let names: Vec<_> = v["tools"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["echo", "add", "fail", "whoami", "hang"]);
    let broken = server(&env, "broken", "", json!({"transport": "stdio", "command": "/no/such/mcp-program"})).await;
    let (_, v) = call(&env.app, "POST", &format!("/api/mcp-servers/{broken}/test"), None).await;
    assert!(v["tools"]["error"].as_str().unwrap().contains("could not start"), "{v}");
}

// ---- a remote server that signs in with OAuth ----

#[derive(Default)]
struct Fake {
    base: String,
    challenge: Option<String>,
    registered: u32,
    refreshes: u32,
    revoked: bool,
    /// The sign-in address its metadata names instead of its own `/authorize` (a hostile server's).
    authorize_at: Option<String>,
    /// The bearer tokens the MCP endpoint saw.
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Fake>>;

async fn prm(State(f): State<Shared>) -> axum::Json<Value> {
    let base = f.lock().unwrap().base.clone();
    axum::Json(
        json!({"resource": format!("{base}/mcp"), "authorization_servers": [base], "scopes_supported": ["search:read"]}),
    )
}

async fn as_meta(State(f): State<Shared>) -> axum::Json<Value> {
    let (base, at) = {
        let f = f.lock().unwrap();
        (f.base.clone(), f.authorize_at.clone())
    };
    let authorize = at.unwrap_or_else(|| format!("{base}/authorize"));
    axum::Json(json!({"issuer": base, "authorization_endpoint": authorize,
        "token_endpoint": format!("{base}/token"), "registration_endpoint": format!("{base}/register"),
        "code_challenge_methods_supported": ["S256"]}))
}

async fn register(State(f): State<Shared>, axum::Json(b): axum::Json<Value>) -> (StatusCode, axum::Json<Value>) {
    assert_eq!(b["token_endpoint_auth_method"], "none");
    f.lock().unwrap().registered += 1;
    (StatusCode::CREATED, axum::Json(json!({"client_id": "client-1", "redirect_uris": b["redirect_uris"]})))
}

async fn authorize(State(f): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Redirect {
    assert_eq!(q["client_id"], "client-1");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["scope"], "search:read");
    assert!(q["resource"].ends_with("/mcp"));
    f.lock().unwrap().challenge = Some(q["code_challenge"].clone());
    Redirect::to(&format!("{}?code=code-1&state={}", q["redirect_uri"], q["state"]))
}

async fn token(State(f): State<Shared>, Form(q): Form<HashMap<String, String>>) -> Response {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let mut f = f.lock().unwrap();
    match q["grant_type"].as_str() {
        "authorization_code" => {
            let want = f.challenge.clone().unwrap();
            let got =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(q["code_verifier"].as_bytes()));
            assert_eq!(got, want, "PKCE");
            assert_eq!(q["code"], "code-1");
            // Expires within PDT's refresh margin: the next use refreshes it.
            axum::Json(json!({"access_token": "tok-1", "refresh_token": "ref-1", "expires_in": 60,
                "token_type": "Bearer", "team": {"name": "Acme"}}))
            .into_response()
        }
        "refresh_token" if f.revoked => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant", "error_description": "token revoked"})),
        )
            .into_response(),
        "refresh_token" => {
            assert_eq!(q["refresh_token"], "ref-1");
            f.refreshes += 1;
            axum::Json(json!({"access_token": format!("tok-r{}", f.refreshes), "expires_in": 60})).into_response()
        }
        other => panic!("grant {other}"),
    }
}

/// A streamable HTTP MCP server answering in JSON, for a bearer token from the fake's token endpoint.
async fn mcp(State(f): State<Shared>, headers: HeaderMap, axum::Json(req): axum::Json<Value>) -> Response {
    let auth = headers.get("authorization").and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
    let base = f.lock().unwrap().base.clone();
    if !auth.starts_with("Bearer tok-") {
        let www = format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\"");
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", www)]).into_response();
    }
    f.lock().unwrap().seen.push(auth.trim_start_matches("Bearer ").to_string());
    let Some(id) = req.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
    let result = match req["method"].as_str() {
        Some("initialize") => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-slack", "version": "1"}}),
        Some("tools/list") => json!({"tools": [
            {"name": "search", "description": "Searches.", "inputSchema": {"type": "object"}},
            {"name": "post_message", "description": "Posts.", "inputSchema": {"type": "object"}}]}),
        _ => json!({}),
    };
    axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn fake_service() -> Shared {
    use axum::routing::{get, post};
    let f: Shared = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.lock().unwrap().base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/.well-known/oauth-protected-resource/mcp", get(prm))
        .route("/.well-known/oauth-authorization-server", get(as_meta))
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/mcp", post(mcp))
        .with_state(f.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    f
}

async fn view(env: &Env, id: &str) -> Value {
    let (_, list) = call(&env.app, "GET", "/api/mcp-servers", None).await;
    list["servers"].as_array().unwrap().iter().find(|s| s["connection"]["id"] == id).unwrap().clone()
}

#[tokio::test]
async fn a_remote_server_signs_in_once_in_pdt_and_every_session_gets_a_fresh_bearer_token() {
    let env = setup().await;
    let f = fake_service().await;
    let base = f.lock().unwrap().base.clone();
    let id = server(
        &env,
        "Slack",
        &format!("{base}/mcp"),
        json!({"transport": "http", "oauth": {},
               "agents": {"tools": {"default": "ask", "overrides": {"search": "allow", "post_message": "block"}}}}),
    )
    .await;
    assert_eq!(view(&env, &id).await["oauth"]["signedIn"], false);
    // Not signed in: sessions start without it.
    let s0 = terminal(&env, "claude").await;
    // Its launch done before the next one's (the fake records them in one file).
    launch_of(&env, &s0).await;
    assert!(claude_config(&env, &s0)["mcpServers"].get("slack").is_none());

    // Sign in: PDT registers itself and sends the browser to the service, which comes back to PDT's callback.
    let (s, start) = call(
        &env.app,
        "POST",
        &format!("/api/mcp-servers/{id}/sign-in"),
        Some(json!({"origin": "http://pdt.example:7420"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{start}");
    let url = start["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{base}/authorize?")), "{url}");
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let r = client.get(url).send().await.unwrap();
    let back = r.headers()["location"].to_str().unwrap().to_string();
    let path = back.strip_prefix("http://pdt.example:7420").expect("back to the address PDT was reached at");
    assert!(path.starts_with("/api/mcp-oauth/callback?"), "{path}");
    let (s, ct, page) = common::call_raw(&env.app, "GET", path, vec![]).await;
    let page = String::from_utf8(page).unwrap();
    assert_eq!(s, StatusCode::OK, "{page}");
    assert!(ct.unwrap().starts_with("text/html") && page.contains("signed in to Slack as Acme"), "{page}");
    assert_eq!(f.lock().unwrap().registered, 1);
    let v = view(&env, &id).await;
    assert_eq!((v["oauth"]["signedIn"].as_bool(), v["oauth"]["account"].as_str()), (Some(true), Some("Acme")));
    // The callback's state is used up.
    let (s, _, _) = common::call_raw(&env.app, "GET", path, vec![]).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // A new session refreshes the token that is about to expire, and gets it as a bearer header.
    let s1 = terminal(&env, "claude").await;
    let cfg = claude_config(&env, &s1);
    // (PDT's own listing after the sign-in may have refreshed it once already.)
    let bearer = cfg["mcpServers"]["slack"]["headers"]["Authorization"].as_str().unwrap_or_default();
    assert!(bearer.starts_with("Bearer tok-r"), "{cfg}");
    assert_eq!(cfg["mcpServers"]["slack"]["type"], "http");
    let launch = launch_of(&env, &s1).await;
    assert!(arg_after(&launch, "--allowedTools").unwrap().contains("mcp__slack__search"));
    assert_eq!(arg_after(&launch, "--disallowedTools"), Some("mcp__slack__post_message"));

    // Test connects with the token and lists the tools.
    let (_, probe) = call(&env.app, "POST", &format!("/api/mcp-servers/{id}/test"), None).await;
    assert_eq!(probe["tools"]["error"], Value::Null, "{probe}");
    let names: Vec<_> =
        probe["tools"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["search", "post_message"]);
    assert!(f.lock().unwrap().seen.iter().all(|t| t.starts_with("tok-r")), "a refreshed token");

    // Revoked: the refresh is refused, the Inbox says to sign in again, and new sessions go without it.
    f.lock().unwrap().revoked = true;
    let _ = terminal(&env, "claude").await;
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let item = inbox["items"].as_array().unwrap().iter().find(|i| i["kind"] == "mcp_sign_in").expect("an Inbox item");
    assert_eq!(item["title"], "Sign in to Slack again");
    assert!(item["body"]["text"].as_str().unwrap().contains("token revoked"), "{item}");
    let s3 = terminal(&env, "claude").await;
    assert!(claude_config(&env, &s3)["mcpServers"].get("slack").is_none());
    assert!(view(&env, &id).await["oauth"]["problem"].as_str().unwrap().contains("revoked"));

    // Signing out forgets it, and the Inbox item goes.
    let (s, _) = call(&env.app, "DELETE", &format!("/api/mcp-servers/{id}/sign-in"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(view(&env, &id).await["oauth"]["signedIn"], false);
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert!(!inbox["items"].as_array().unwrap().iter().any(|i| i["kind"] == "mcp_sign_in"));
}

#[tokio::test]
async fn a_sign_in_address_that_isnt_https_is_refused_before_the_browser_goes_there() {
    let env = setup().await;
    let f = fake_service().await;
    let base = f.lock().unwrap().base.clone();
    let id = server(&env, "Evil", &format!("{base}/mcp"), json!({"transport": "http", "oauth": {}})).await;
    for at in ["javascript:fetch('/api/sessions',{method:'POST'})//", "http://auth.example/authorize"] {
        f.lock().unwrap().authorize_at = Some(at.into());
        let (s, v) = call(&env.app, "POST", &format!("/api/mcp-servers/{id}/sign-in"), None).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
        assert!(v["message"].as_str().unwrap().contains("isn't https"), "{v}");
        assert!(v.get("url").is_none(), "{v}");
    }
    assert_eq!(f.lock().unwrap().registered, 0, "PDT didn't register with it");
}

/// A keychain that won't delete (locked, or a refused prompt).
struct KeepingKeychain(pdt_api::secrets::MemorySecretStore);
impl pdt_api::secrets::SecretStore for KeepingKeychain {
    fn get(&self, name: &str) -> Result<Option<String>, pdt_api::secrets::SecretError> {
        self.0.get(name)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), pdt_api::secrets::SecretError> {
        self.0.set(name, value)
    }
    fn delete(&self, _: &str) -> Result<(), pdt_api::secrets::SecretError> {
        Err(pdt_api::secrets::SecretError("the keychain is locked".into()))
    }
}

#[tokio::test]
async fn signing_out_says_so_when_the_keychain_keeps_the_tokens() {
    let env = setup().await;
    let id = server(&env, "Slack", "https://slack.example/mcp", json!({"transport": "http", "oauth": {}})).await;
    let mut state = env.state.clone();
    state.secrets = Arc::new(KeepingKeychain(Default::default()));
    let app = router(state, None);
    let (s, v) = call(&app, "DELETE", &format!("/api/mcp-servers/{id}/sign-in"), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["code"], "keychain_unavailable", "{v}");
    // Deleting the server stops there too: the connection stays, as its tokens do.
    let (s, v) = call(&app, "DELETE", &format!("/api/connections/{id}"), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(view(&env, &id).await["connection"]["id"], id.as_str());
}

#[tokio::test]
async fn a_server_for_one_project_or_turned_off_there_stays_out_of_other_sessions() {
    let env = setup().await;
    server(&env, "Here", "", json!({"transport": "stdio", "command": FAKE_MCP, "agents": {"projects": [env.pid]}}))
        .await;
    server(
        &env,
        "Elsewhere",
        "",
        json!({"transport": "stdio", "command": FAKE_MCP, "agents": {"projects": ["other"]}}),
    )
    .await;
    server(&env, "Off", "", json!({"transport": "stdio", "command": FAKE_MCP, "agents": {"offIn": [env.pid]}})).await;
    server(
        &env,
        "Nothing",
        "",
        json!({"transport": "stdio", "command": FAKE_MCP, "agents": {"tools": {"default": "block"}}}),
    )
    .await;
    let id = terminal(&env, "claude").await;
    let cfg = claude_config(&env, &id);
    assert_eq!(cfg["mcpServers"].as_object().unwrap().keys().collect::<Vec<_>>(), ["here", "pdt"]);
}
