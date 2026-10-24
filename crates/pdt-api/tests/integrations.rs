//! Phase 7e's triggers and actions through the API: schedules, webhooks, GitHub issues, `command`, `http.request`,
//! `discord.post` and `notify`, the Hermes templates, Radarr, Sonarr, Plex, Scryer and the brief archive. External services are local fake servers and `FakeGithub`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::call;
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_github::testing::FakeGithub;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

struct Env {
    app: axum::Router,
    state: AppState,
    gh: Arc<FakeGithub>,
    tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    let gh = Arc::new(FakeGithub::default());
    state.github = gh.clone();
    let app = router(state.clone(), None);
    Env { app, state, gh, tmp }
}

async fn create(env: &Env, doc: Value) -> String {
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn run_now(env: &Env, wf: &str, input: Value) -> Value {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let id = r["runId"].as_str().unwrap();
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

async fn runs(env: &Env, wf: &str) -> Vec<Value> {
    let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    r.as_array().unwrap().clone()
}

async fn connection(env: &Env, name: &str, kind: &str, url: &str, config: Value, secret: Option<&str>) {
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": name, "kind": kind, "url": url, "config": config})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    if let Some(secret) = secret {
        let id = c["id"].as_str().unwrap();
        let (s, _) =
            call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": secret}))).await;
        assert_eq!(s, StatusCode::OK);
    }
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|s| s["nodeId"] == node)
        .unwrap_or_else(|| panic!("{node} in {run}"))
}

fn manual_then(node: Value) -> Value {
    json!({"name": "Test", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
        "edges": [{"from": "go", "to": node["id"]}]})
}

// ---- a fake HTTP service ----

/// One request a fake server got: method, path and query, JSON body, its key (Authorization, else X-Api-Key, else
/// X-Plex-Token).
type Request1 = (String, String, Value, Option<String>);

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<Request1>>>);

impl Seen {
    fn all(&self) -> Vec<Request1> {
        self.0.lock().unwrap().clone()
    }
}

/// A server on a port the OS picks: records each request (method, path and query, JSON body, key) and
/// answers what `answer` says for its path.
async fn fake_server(answer: fn(&str, usize) -> (u16, Value)) -> (String, Seen) {
    let seen = Seen::default();
    let s2 = seen.clone();
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let seen = s2.clone();
        async move {
            let method = req.method().to_string();
            let path = req.uri().to_string();
            let auth = ["authorization", "x-api-key", "x-plex-token"]
                .iter()
                .find_map(|h| req.headers().get(*h).and_then(|v| v.to_str().ok()).map(String::from));
            let bytes = req.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let n = {
                let mut all = seen.0.lock().unwrap();
                all.push((method, path.clone(), body, auth));
                all.len()
            };
            let (status, v) = answer(&path, n);
            (StatusCode::from_u16(status).unwrap(), axum::Json(v))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

// ---- command ----

fn fake_command() -> &'static str {
    env!("CARGO_BIN_EXE_fake-command")
}

#[tokio::test]
async fn a_command_gets_json_on_stdin_and_gives_json_back() {
    let env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "run", "kind": "command", "version": 1, "config": {
            "program": fake_command(), "args": ["echo", "{{trigger.word}}"],
            "input": {"n": "{{trigger.n}}"}, "env": {"FAKE_ENV": "set"}}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"n": 3, "word": "hi"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let out = &step(&r, "run")["output"];
    assert_eq!(out["exitCode"], 0);
    assert_eq!(out["json"]["got"], json!({"n": 3}), "a whole reference keeps its type");
    assert_eq!(out["json"]["args"], json!(["hi"]));
    assert_eq!(out["json"]["env"], "set");
    let cwd = out["json"]["cwd"].as_str().unwrap();
    assert!(cwd.contains("workflow-scratch"), "{cwd}");
    assert!(!std::path::Path::new(cwd).exists(), "the scratch folder goes with the run");
}

#[tokio::test]
async fn a_failing_command_fails_the_step_or_takes_its_error_port() {
    let env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "run", "kind": "command", "version": 1,
            "config": {"program": fake_command(), "args": ["fail"]}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "failed");
    assert!(r["error"].as_str().unwrap().contains("exited with 3: boom"), "{r}");

    // Not JSON, said to be JSON: an error; as text: fine.
    let text = |parse: &str| {
        manual_then(json!({"id": "run", "kind": "command", "version": 1,
            "config": {"program": fake_command(), "args": ["text"], "parse": parse}}))
    };
    let wf = create(&env, text("json")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("not JSON"), "{r}");
    let wf = create(&env, text("text")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "run")["output"]["stdout"].as_str().map(str::trim), Some("hello"));

    // `{"quiet": true}`: nothing to report.
    let wf = create(
        &env,
        manual_then(json!({"id": "run", "kind": "command", "version": 1,
            "config": {"program": fake_command(), "args": ["quiet"]}})),
    )
    .await;
    assert_eq!(run_now(&env, &wf, json!({})).await["status"], "quiet");
}

#[tokio::test]
async fn a_command_past_its_time_limit_is_stopped() {
    let env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "run", "kind": "command", "version": 1, "timeoutMs": 300,
            "config": {"program": fake_command(), "args": ["sleep", "20000"]}})),
    )
    .await;
    let started = std::time::Instant::now();
    let r = run_now(&env, &wf, json!({})).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert_eq!(r["status"], "failed");
    assert!(r["error"].as_str().unwrap().contains("timed out"), "{r}");
}

#[tokio::test]
async fn a_dry_run_never_runs_the_command() {
    let env = setup().await;
    let marker = env.tmp.path().join("ran");
    let doc = manual_then(json!({"id": "run", "kind": "command", "version": 1,
        "config": {"program": fake_command(), "args": ["echo"], "cwd": marker.to_string_lossy()}}));
    let (s, d) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": doc, "event": {"type": "manual"}}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let would = d["run"]["steps"][1]["output"]["would"].as_str().unwrap();
    assert!(would.starts_with("run ") && would.contains("echo"), "{would}");
}

#[tokio::test]
async fn a_command_cwd_must_be_a_mode_or_a_full_path() {
    let env = setup().await;
    let doc = manual_then(json!({"id": "run", "kind": "command", "version": 1,
        "config": {"program": "x", "cwd": "relative/dir"}}));
    let (s, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.to_string().contains("not scratch, project, attempt or a full path"), "{v}");
}

// ---- HTTP and Discord ----

#[tokio::test]
async fn an_http_request_uses_its_connection_and_checks_the_status() {
    let (url, seen) = fake_server(|path, _| {
        if path.starts_with("/api/v3/movie") {
            (200, json!({"movies": [1, 2]}))
        } else {
            (404, json!({"error": "nope"}))
        }
    })
    .await;
    let env = setup().await;
    connection(&env, "radarr", "http", &format!("{url}/api"), json!({"auth": "bearer"}), Some("tok")).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "get", "kind": "http.request", "version": 1, "config": {
            "connection": "radarr", "url": "/v3/movie", "query": {"term": "{{trigger.term}}"}}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"term": "dune"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let out = &step(&r, "get")["output"];
    assert_eq!((out["status"].as_u64(), &out["body"]), (Some(200), &json!({"movies": [1, 2]})));
    let (method, path, _, auth) = seen.all().pop().unwrap();
    assert_eq!(
        (method.as_str(), path.as_str(), auth.as_deref()),
        ("GET", "/api/v3/movie?term=dune", Some("Bearer tok"))
    );

    // A status outside `expect` is an error; `any` takes it.
    let missing = |expect: Value| {
        manual_then(json!({"id": "get", "kind": "http.request", "version": 1,
            "config": {"connection": "radarr", "url": "/nope", "method": "POST", "body": {"a": 1}, "expect": expect}}))
    };
    let wf = create(&env, missing(json!("2xx"))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("answered 404"), "{r}");
    let wf = create(&env, missing(json!("any"))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "get")["output"]["status"], 404);
    assert_eq!(seen.all().last().unwrap().2, json!({"a": 1}));

    // The secret never goes to another address.
    let wf = create(
        &env,
        manual_then(json!({"id": "get", "kind": "http.request", "version": 1,
            "config": {"connection": "radarr", "url": format!("{url}/elsewhere/v3/movie"), "expect": "any"}})),
    )
    .await;
    run_now(&env, &wf, json!({})).await;
    assert_eq!(seen.all().last().unwrap().3, None);
}

#[tokio::test]
async fn discord_posts_split_long_messages_and_wait_out_a_rate_limit() {
    let (url, seen) =
        fake_server(
            |_, n| {
                if n == 1 { (429, json!({"retry_after": 0.05})) } else { (200, json!({"id": format!("m{n}")})) }
            },
        )
        .await;
    let env = setup().await;
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&format!("{url}/api/webhooks/1/abc"))).await;
    let long = "line of news\n".repeat(200);
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1, "config": {
            "connection": "alerts", "content": "{{trigger.text}}", "username": "PDT"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"text": long})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let all = seen.all();
    assert_eq!(all.len(), 3, "a rate-limited try, then two messages: {all:?}");
    assert!(all.iter().all(|(m, p, _, _)| m == "POST" && p.starts_with("/api/webhooks/1/abc?wait=true")));
    assert!(all.iter().all(|(_, _, b, _)| b["content"].as_str().unwrap().chars().count() <= 2000));
    assert_eq!(all[1].2["username"], "PDT");
    assert_eq!(step(&r, "post")["output"]["messageIds"], json!(["m2", "m3"]));

    // A connection of another kind is refused.
    connection(&env, "plain", "http", &url, json!({}), Some("x")).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1,
            "config": {"connection": "plain", "content": "hi"}})),
    )
    .await;
    assert!(run_now(&env, &wf, json!({})).await["error"].as_str().unwrap().contains("not discord_webhook"));
}

/// A server that never answers request number `hang` (nor records it: as if PDT stopped before sending it), and
/// answers every other with `{"id": "m<n>"}`.
async fn hanging_server(hang: usize) -> (String, Seen) {
    let seen = Seen::default();
    let s2 = seen.clone();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let (seen, count) = (s2.clone(), count.clone());
        async move {
            let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == hang {
                std::future::pending::<()>().await;
            }
            let method = req.method().to_string();
            let path = req.uri().to_string();
            let bytes = req.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            seen.0.lock().unwrap().push((method, path, body, None));
            axum::Json(json!({"id": format!("m{n}")}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

/// Starts a run of `wf` and runs the engine once, with ticks that wait at most a moment for a slow step.
async fn start_slow(env: &mut Env, wf: &str, input: Value) -> String {
    env.state.engine_tick_wait = std::time::Duration::from_millis(300);
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    r["runId"].as_str().unwrap().to_string()
}

/// A second daemon on the same database and keychain: the first one stopped.
async fn restarted(env: &Env) -> (AppState, axum::Router) {
    let store = Store::open(&env.tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, env.tmp.path().join("data"));
    state.secrets = env.state.secrets.clone();
    let app = router(state.clone(), None);
    (state, app)
}

#[tokio::test]
async fn a_restart_mid_discord_post_posts_each_piece_once() {
    let (url, seen) = hanging_server(2).await;
    let mut env = setup().await;
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&format!("{url}/api/webhooks/1/abc"))).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1, "retry": {"max": 1},
            "config": {"connection": "alerts", "content": "{{trigger.text}}"}})),
    )
    .await;
    let long = format!("{}\n{}", "a".repeat(1500), "b".repeat(1500));
    let id = start_slow(&mut env, &wf, json!({"text": long})).await;
    // The first piece went out and is recorded; the daemon stops while the second is on its way.
    let r = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(step(&r, "post")["status"], "running", "{r}");
    assert_eq!(step(&r, "post")["output"]["messageIds"], json!(["m1"]));

    let (state, app) = restarted(&env).await;
    pdt_api::run_workflows(&state).await;
    let r = call(&app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(r["status"], "succeeded", "{r}");
    let post = step(&r, "post");
    assert_eq!(post["output"]["messageIds"], json!(["m1", "m3"]));
    assert_eq!(post["attempts"], 1, "the restart doesn't use up a retry");
    let contents: Vec<String> =
        seen.all().iter().map(|(_, _, b, _)| b["content"].as_str().unwrap().chars().take(1).collect()).collect();
    assert_eq!(contents, ["a", "b"], "each piece once");
}

#[tokio::test]
async fn a_post_cut_short_by_a_restart_is_not_sent_again() {
    let (url, seen) = hanging_server(1).await;
    let mut env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "req", "kind": "http.request", "version": 1, "retry": {"max": 3},
            "config": {"method": "POST", "url": format!("{url}/hook"), "body": {"x": 1}}})),
    )
    .await;
    let id = start_slow(&mut env, &wf, json!({})).await;
    let (state, app) = restarted(&env).await;
    pdt_api::run_workflows(&state).await;
    let r = call(&app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(r["status"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("Retry the run"), "{r}");
    assert!(seen.all().is_empty(), "not sent again");
    // The user's Retry sends it once more.
    let (s, _) = call(&app, "POST", &format!("/api/workflow-runs/{id}/retry"), None).await;
    assert_eq!(s, StatusCode::OK);
    pdt_api::run_workflows(&state).await;
    let r = call(&app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(seen.all().len(), 1);
}

/// A local address nothing listens on (the OS picked it, then it was let go).
async fn closed_address() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// What `tracing` writes while the test's thread runs, at every level.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn capture(&self) -> tracing::subscriber::DefaultGuard {
        let log = self.clone();
        let sub = tracing_subscriber::fmt().with_max_level(tracing::Level::TRACE).with_writer(move || log.clone());
        tracing::subscriber::set_default(sub.finish())
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn a_failed_request_never_shows_the_connection_secret() {
    let log = Log::default();
    let _guard = log.capture();
    let env = setup().await;
    let closed = closed_address().await;

    // A Discord webhook's URL is its secret.
    let hook = format!("{closed}/api/webhooks/987654321012/s3cret-hook-token");
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&hook)).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "post", "kind": "discord.post", "version": 1,
            "config": {"connection": "alerts", "content": "hi"}})),
    )
    .await;
    // A callsite other tests' threads registered as this one set its subscriber may have cached "never": look again.
    tracing::callsite::rebuild_interest_cache();
    let discord = run_now(&env, &wf, json!({})).await;

    // A key sent in the query.
    connection(&env, "svc", "http", &closed, json!({"auth": "query", "param": "apikey"}), Some("s3cret-query-key"))
        .await;
    let wf = create(
        &env,
        manual_then(json!({"id": "get", "kind": "http.request", "version": 1,
            "config": {"connection": "svc", "url": "/items"}})),
    )
    .await;
    tracing::callsite::rebuild_interest_cache();
    let http = run_now(&env, &wf, json!({})).await;

    for (r, node) in [(&discord, "post"), (&http, "get")] {
        assert_eq!(r["status"], "failed", "{r}");
        let error = step(r, node)["error"].as_str().unwrap();
        assert!(!error.is_empty() && r["error"].as_str().is_some(), "{r}");
    }
    assert!(step(&discord, "post")["error"].as_str().unwrap().starts_with("Discord: "), "{discord}");
    assert!(step(&http, "get")["error"].as_str().unwrap().contains("/items"), "{http}");
    let log = log.text();
    assert!(log.contains("a workflow step failed"), "{log}");
    for secret in ["987654321012", "s3cret-hook-token", "s3cret-query-key"] {
        for (what, text) in [("discord run", discord.to_string()), ("http run", http.to_string()), ("log", log.clone())]
        {
            assert!(!text.contains(secret), "{secret} in the {what}: {text}");
        }
    }
}

#[tokio::test]
async fn a_discord_channel_is_made_only_when_missing() {
    // Requests in order: the first run lists the channels; the second lists them, makes the category, then the
    // channel.
    let (url, seen) = fake_server(|_, n| match n {
        1 | 2 => (200, json!([{"id": "1", "name": "alerts", "type": 0}])),
        3 => (201, json!({"id": "5", "name": "Bots", "type": 4})),
        _ => (201, json!({"id": "6", "name": "ai-briefs", "type": 0, "parent_id": "5"})),
    })
    .await;
    let env = setup().await;
    connection(&env, "home", "discord_bot", &url, json!({"guildId": "42"}), Some("tok")).await;
    let ensure = |name: &str| {
        manual_then(json!({"id": "ch", "kind": "discord.channel.ensure", "version": 1,
            "config": {"connection": "home", "name": name, "category": "Bots"}}))
    };
    let wf = create(&env, ensure("#alerts")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "ch")["output"], json!({"channelId": "1", "name": "alerts", "created": false}), "{r}");
    let wf = create(&env, ensure("AI Briefs")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "ch")["output"], json!({"channelId": "6", "name": "ai-briefs", "created": true}), "{r}");
    let all = seen.all();
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(all.iter().all(|r| r.1 == "/guilds/42/channels" && r.3.as_deref() == Some("Bot tok")), "{all:?}");
    assert_eq!((all[2].2["name"].as_str(), all[2].2["type"].as_u64()), (Some("Bots"), Some(4)));
    assert_eq!(all[3].2, json!({"name": "ai-briefs", "type": 0, "parent_id": "5"}));
}

// ---- notify ----

#[tokio::test]
async fn notify_makes_a_workflow_notification() {
    let env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "tell", "kind": "notify", "version": 1, "config": {
            "title": "Build {{trigger.n}} broke", "body": "see logs", "level": "error", "link": "PDT-1"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"n": 7})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, list) = call(&env.app, "GET", "/api/notifications?section=updates", None).await;
    let n = &list["items"][0];
    assert_eq!(
        (&n["kind"], &n["title"], &n["severity"], &n["show"], &n["target"]),
        (
            &json!("workflow.notify"),
            &json!("Build 7 broke"),
            &json!("error"),
            &json!("toast"),
            &json!({"kind": "ticket_key", "key": "PDT-1"})
        )
    );
    assert!(n["body"].as_str().unwrap().starts_with("see logs · From "), "{n}");
    assert_eq!(list["unreadUpdates"], 1);
    // Turned off: the step makes nothing.
    let (s, _) = call(&env.app, "PATCH", "/api/notifications/settings", Some(json!({"workflow.notify": "off"}))).await;
    assert_eq!(s, StatusCode::OK, "patch answers with the settings");
    let r = run_now(&env, &wf, json!({"n": 8})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, list) = call(&env.app, "GET", "/api/notifications?section=updates", None).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
}

// ---- schedule ----

#[tokio::test]
async fn a_schedule_takes_a_baseline_then_fires_once_per_slot() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "Every minute", "nodes": [
            {"id": "tick", "kind": "schedule", "version": 1, "config": {"every": "1m", "runKey": "t:{{trigger.slot}}"}},
            {"id": "end", "kind": "end", "version": 1}],
          "edges": [{"from": "tick", "to": "end"}]}),
    )
    .await;
    pdt_api::run_workflows(&env.state).await;
    assert!(runs(&env, &wf).await.is_empty(), "a new schedule waits for its next slot");
    let (_, st) = call(&env.app, "GET", &format!("/api/workflows/{wf}/state"), None).await;
    let mark = st["_pdt.schedule.tick"].clone();
    let slot = mark["slot"].as_i64().unwrap();

    // Pretend the previous slot was the last one handled (as if a minute went by).
    let back = json!({"slot": slot - 60_000, "at": mark["at"].as_i64().unwrap()});
    env.state.store.set_workflow_state(&wf, "_pdt.schedule.tick", &back, false, 0).await.unwrap();
    pdt_api::run_workflows(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "one run for the slot however often the engine ticks: {r:?}");
    assert_eq!(r[0]["trigger"]["slot"], json!(slot));
    assert!(r[0]["trigger"]["date"].as_str().unwrap().len() == 10);

    // State keys of PDT's own can't be written by a workflow.
    let bad = json!({"name": "x", "nodes": [{"id": "go", "kind": "manual", "version": 1},
        {"id": "s", "kind": "state.set", "version": 1, "config": {"key": "_pdt.schedule.tick", "value": 1}}],
        "edges": [{"from": "go", "to": "s"}]});
    let (s, _) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": bad}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_schedule_needs_one_valid_schedule() {
    let env = setup().await;
    for (config, why) in [
        (json!({}), "give every"),
        (json!({"every": "10s"}), "shorter than a minute"),
        (json!({"cron": "0 7 * * *", "every": "1h"}), "not both"),
        (json!({"cron": "0 7 * *"}), "5-field"),
        (json!({"cron": "0 7 * * *", "timezone": "Nowhere/Land"}), "not a time zone"),
    ] {
        let doc = json!({"name": "x", "nodes": [{"id": "t", "kind": "schedule", "version": 1, "config": config}],
            "edges": []});
        let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
        assert!(v.to_string().contains(why), "{why}: {v}");
    }
}

// ---- webhook ----

async fn post_hook(env: &Env, name: &str, headers: &[(&str, &str)], body: &str) -> (StatusCode, Value) {
    let mut req = Request::builder().method("POST").uri(format!("/api/hooks/{name}?from=test"));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = env.app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_webhook_needs_its_secret_and_starts_its_workflows() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "On deploy", "nodes": [
            {"id": "hook", "kind": "webhook", "version": 1, "config": {"hook": "deploys",
             "where": [{"left": "{{trigger.body.ok}}", "op": "truthy"}]}},
            {"id": "end", "kind": "end", "version": 1}],
          "edges": [{"from": "hook", "to": "end"}]}),
    )
    .await;
    let other = create(
        &env,
        json!({"name": "Other hook", "nodes": [
            {"id": "hook", "kind": "webhook", "version": 1, "config": {"hook": "other"}},
            {"id": "end", "kind": "end", "version": 1}],
          "edges": [{"from": "hook", "to": "end"}]}),
    )
    .await;
    let json_ct = ("content-type", "application/json");
    assert_eq!(post_hook(&env, "deploys", &[], "{}").await.0, StatusCode::NOT_FOUND, "no such hook yet");
    connection(&env, "deploys", "webhook", "", json!({}), None).await;
    let (s, e) = post_hook(&env, "deploys", &[("authorization", "Bearer s3cret")], "{}").await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("webhook_no_secret")));
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    let id =
        list.as_array().unwrap().iter().find(|c| c["name"] == "deploys").unwrap()["id"].as_str().unwrap().to_string();
    call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": "s3cret"}))).await;

    let (s, e) = post_hook(&env, "deploys", &[("authorization", "Bearer wrong"), json_ct], "{\"ok\":true}").await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("bad_signature")));
    // From another machine on the tailnet (proxied, no user identity): the secret is enough.
    let good =
        [("authorization", "Bearer s3cret"), json_ct, ("x-forwarded-for", "100.64.0.9"), ("x-github-delivery", "d1")];
    let (s, e) = post_hook(&env, "deploys", &good, "{\"ok\":true}").await;
    assert_eq!(s, StatusCode::ACCEPTED, "{e}");
    let (s, e) = post_hook(&env, "deploys", &good, "{\"ok\":true}").await;
    assert_eq!((s, &e), (StatusCode::OK, &json!({"duplicate": true})), "the same delivery again");
    // Never over Funnel.
    let funnel =
        [("authorization", "Bearer s3cret"), ("tailscale-funnel-request", "?1"), ("x-forwarded-for", "1.2.3.4")];
    assert_eq!(post_hook(&env, "deploys", &funnel, "{}").await.0, StatusCode::FORBIDDEN);
    // A body the trigger's condition skips.
    let (s, _) = post_hook(&env, "deploys", &[("authorization", "Bearer s3cret"), json_ct], "{\"ok\":false}").await;
    assert_eq!(s, StatusCode::ACCEPTED);

    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "{r:?}");
    let t = &r[0]["trigger"];
    assert_eq!(
        (&t["hook"], &t["body"], &t["query"]["from"]),
        (&json!("deploys"), &json!({"ok": true}), &json!("test"))
    );
    assert_eq!(t["headers"]["x-github-delivery"], "d1");
    assert!(t["headers"].get("authorization").is_none());
    assert!(runs(&env, &other).await.is_empty(), "another hook's workflow");
}

// ---- GitHub issues ----

fn issue(n: u64, title: &str) -> pdt_github::Issue {
    pdt_github::Issue {
        number: n,
        title: title.into(),
        body: String::new(),
        url: format!("https://github.com/o/r/issues/{n}"),
        author: "al".into(),
        labels: vec!["bug".into()],
        created_at: "2026-10-20T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn new_issues_in_a_watched_repo_start_runs_after_a_silent_first_look() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "Issues", "nodes": [
            {"id": "opened", "kind": "github.issue.opened", "version": 1, "config": {"repos": ["o/r"]}},
            {"id": "end", "kind": "end", "version": 1}],
          "edges": [{"from": "opened", "to": "end"}]}),
    )
    .await;
    env.gh.set_issues("o/r", vec![issue(4, "Old"), issue(3, "Older")]);
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert!(runs(&env, &wf).await.is_empty(), "the first look is the baseline");

    env.gh.set_issues("o/r", vec![issue(6, "Newest"), issue(5, "New"), issue(4, "Old")]);
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    let mut titles: Vec<&str> = r.iter().map(|r| r["trigger"]["title"].as_str().unwrap()).collect();
    titles.sort();
    assert_eq!(titles, ["New", "Newest"]);
    assert_eq!(r[0]["trigger"]["repo"], "o/r");

    // Nothing watches a repo when no enabled workflow names it.
    call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": false}))).await;
    let before = env.gh.calls.lock().unwrap().len();
    pdt_api::refresh_issues(&env.state).await;
    assert_eq!(env.gh.calls.lock().unwrap().len(), before);
}

#[tokio::test]
async fn an_issue_that_could_not_be_logged_is_logged_on_the_next_look() {
    let env = setup().await;
    create(
        &env,
        json!({"name": "Issues", "nodes": [
            {"id": "opened", "kind": "github.issue.opened", "version": 1, "config": {"repos": ["o/r"]}},
            {"id": "end", "kind": "end", "version": 1}],
          "edges": [{"from": "opened", "to": "end"}]}),
    )
    .await;
    let mark = || async {
        env.state.store.get_host_value("sensor.github_issues.o/r").await.unwrap().unwrap()["newest"].as_u64()
    };
    env.gh.set_issues("o/r", vec![issue(4, "Old")]);
    pdt_api::refresh_issues(&env.state).await;
    assert_eq!(mark().await, Some(4));

    // gh failing: nothing moves.
    *env.gh.error.lock().unwrap() = Some("HTTP 401: Bad credentials".into());
    env.gh.set_issues("o/r", vec![issue(5, "New"), issue(4, "Old")]);
    pdt_api::refresh_issues(&env.state).await;
    assert_eq!(mark().await, Some(4));
    *env.gh.error.lock().unwrap() = None;

    // The store failing: the mark stays before the issue it could not log.
    let db = env.tmp.path().join("pdt.db");
    common::fail_events(&db, Some("github.issue.opened")).await;
    pdt_api::refresh_issues(&env.state).await;
    assert_eq!(mark().await, Some(4), "#5 was not logged");
    common::fail_events(&db, None).await;
    pdt_api::refresh_issues(&env.state).await;
    assert_eq!(mark().await, Some(5));
    let (_, ev) = call(&env.app, "GET", "/api/events?type=github.issue.opened", None).await;
    let numbers: Vec<u64> = ev.as_array().unwrap().iter().map(|e| e["payload"]["number"].as_u64().unwrap()).collect();
    assert_eq!(numbers, [5]);
}

// ---- templates ----

#[tokio::test]
async fn the_hermes_templates_instantiate_and_dry_run() {
    let env = setup().await;
    let inst = |id: &str, params: Value| {
        let app = env.app.clone();
        let id = id.to_string();
        async move {
            let (s, w) = call(
                &app,
                "POST",
                &format!("/api/workflow-templates/builtin:{id}/instantiate"),
                Some(json!({"params": params, "scope": "personal"})),
            )
            .await;
            assert_eq!(s, StatusCode::CREATED, "{id}: {w}");
            assert_eq!(w["enabled"], false);
            w
        }
    };
    let w = inst("github-issues-discord", json!({"repo": "o/r", "discord": "alerts"})).await;
    let wid = w["id"].as_str().unwrap();
    let (_, d) = call(
        &env.app,
        "POST",
        "/api/workflows/dry-run",
        Some(json!({"workflowId": wid, "event": {"type": "github.issue.opened",
            "payload": {"repo": "o/r", "number": 9, "title": "Crash", "author": "al", "url": "https://i/9"}}})),
    )
    .await;
    let would = d["run"]["steps"][1]["output"]["would"].as_str().unwrap();
    assert!(would.contains("New issue in o/r #9: **Crash** by al"), "{would}");

    let w = inst("health-watchdog", json!({"url": "http://127.0.0.1:1/health", "discord": "alerts"})).await;
    assert_eq!(w["doc"]["nodes"][0]["config"]["every"], "1m");
    let w = inst("brief-follow-ups", json!({"script": "/opt/briefs/watchlist.py", "discord": "alerts"})).await;
    assert_eq!(w["doc"]["nodes"][1]["config"]["args"], json!(["/opt/briefs/watchlist.py"]));
    let w = inst("email-processing", json!({"script": "/opt/mail/process.py", "discord": "alerts"})).await;
    assert_eq!(w["doc"]["nodes"][0]["config"]["every"], "15m");
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:email-processing/instantiate",
        Some(json!({"params": {"discord": "alerts"}})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the script is required");
}

#[tokio::test]
async fn the_health_watchdog_posts_only_on_a_change() {
    let (discord, posts) = fake_server(|_, n| (200, json!({"id": format!("m{n}")}))).await;
    // The service: up, up, down, down, up.
    let (svc, _) = fake_server(|_, n| if matches!(n, 3 | 4) { (503, json!({})) } else { (200, json!({})) }).await;
    let env = setup().await;
    connection(&env, "alerts", "discord_webhook", "", json!({}), Some(&format!("{discord}/hook"))).await;
    let (_, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:health-watchdog/instantiate",
        Some(json!({"params": {"name": "NAS", "url": format!("{svc}/health"), "discord": "alerts"}, "scope": "personal"})),
    )
    .await;
    let wid = w["id"].as_str().unwrap().to_string();
    // Run its graph from a manual trigger at the schedule's place, five times.
    let mut doc = w["doc"].clone();
    doc["nodes"][0] = json!({"id": "tick", "kind": "manual", "version": 1});
    let (s, v) = call(&env.app, "PUT", &format!("/api/workflows/{wid}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut statuses = vec![];
    for _ in 0..5 {
        statuses.push(run_now(&env, &wid, json!({})).await["status"].as_str().unwrap().to_string());
    }
    assert_eq!(statuses, ["quiet", "quiet", "succeeded", "quiet", "succeeded"]);
    let texts: Vec<String> = posts.all().iter().map(|p| p.2["content"].as_str().unwrap().to_string()).collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts[0].contains("NAS is down: 503"), "{}", texts[0]);
    assert!(texts[1].contains("NAS recovered"), "{}", texts[1]);
}

// ---- Radarr, Sonarr, Plex (7e-3) ----

fn radarr_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/api/v3/movie/lookup/tmdb?tmdbId=693134") => {
            (200, json!({"title": "Dune: Part Two", "year": 2024, "tmdbId": 693134, "images": []}))
        }
        p if p.starts_with("/api/v3/movie/lookup?term=dune") => (
            200,
            json!([{"title": "Dune", "year": 2021, "tmdbId": 438631, "id": 5, "overview": "Spice."},
                {"title": "Dune: Part Two", "year": 2024, "tmdbId": 693134}]),
        ),
        // In the library already.
        "/api/v3/movie?tmdbId=438631" => (200, json!([{"id": 5, "title": "Dune", "year": 2021, "tmdbId": 438631}])),
        // An older Radarr that ignores the filter and lists the whole library.
        p if p.starts_with("/api/v3/movie?tmdbId=") => (200, json!([{"id": 9, "title": "Other", "tmdbId": 1}])),
        "/api/v3/movie" => (201, json!({"id": 10, "title": "Dune: Part Two", "year": 2024, "tmdbId": 693134})),
        "/api/v3/qualityprofile" => (200, json!([{"id": 1, "name": "Any"}, {"id": 4, "name": "HD-1080p"}])),
        "/api/v3/rootfolder" => (200, json!([{"id": 1, "path": "/movies"}])),
        p if p.starts_with("/api/v3/queue?") => (
            200,
            json!({"page": 1, "records": [{"title": "Dune.Part.Two.2160p", "size": 200.0, "sizeleft": 50.0,
                "timeleft": "00:05:00", "estimatedCompletionTime": "2026-10-22T10:00:00Z",
                "trackedDownloadState": "downloading", "movie": {"title": "Dune: Part Two", "year": 2024}}]}),
        ),
        _ => (404, json!({"message": "not found"})),
    }
}

#[tokio::test]
async fn radarr_searches_adds_once_and_reads_its_queue() {
    let (url, seen) = fake_server(radarr_answer).await;
    let env = setup().await;
    connection(&env, "films", "radarr", &url, json!({}), Some("radarr-key")).await;

    let wf = create(
        &env,
        manual_then(json!({"id": "find", "kind": "radarr.search", "version": 1,
            "config": {"connection": "films", "term": "{{trigger.term}}"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"term": "dune"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let results = &step(&r, "find")["output"]["results"];
    assert_eq!(results[0]["tmdbId"], 438631);
    assert_eq!((&results[0]["inLibrary"], &results[1]["inLibrary"]), (&json!(true), &json!(false)));
    assert!(seen.all().iter().all(|(_, _, _, key)| key.as_deref() == Some("radarr-key")), "the API key goes with each");

    // A new film: looked up, given the named profile and the first root folder, searched for.
    let add = |id: Value| {
        manual_then(json!({"id": "add", "kind": "radarr.add", "version": 1,
            "config": {"connection": "films", "tmdbId": id, "qualityProfile": "hd-1080p"}}))
    };
    let wf = create(&env, add(json!("{{trigger.id}}"))).await;
    let r = run_now(&env, &wf, json!({"id": 693134})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "add")["port"], "added");
    assert_eq!(step(&r, "add")["output"]["id"], 10);
    let (method, _, body, _) = seen.all().into_iter().find(|(m, _, _, _)| m == "POST").unwrap();
    assert_eq!(method, "POST");
    assert_eq!(
        (&body["tmdbId"], &body["qualityProfileId"], &body["rootFolderPath"]),
        (&json!(693134), &json!(4), &json!("/movies"))
    );
    assert_eq!((&body["monitored"], &body["addOptions"]["searchForMovie"]), (&json!(true), &json!(true)));
    assert_eq!(body["minimumAvailability"], "released");

    // One already there takes `exists` and adds nothing.
    let posts = seen.all().iter().filter(|(m, _, _, _)| m == "POST").count();
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!((step(&r, "add")["port"].as_str(), &step(&r, "add")["output"]["id"]), (Some("exists"), &json!(5)));
    assert_eq!(seen.all().iter().filter(|(m, _, _, _)| m == "POST").count(), posts);

    // An unknown profile is an error; a bad id fails validation.
    let wf = create(
        &env,
        manual_then(json!({"id": "add", "kind": "radarr.add", "version": 1,
            "config": {"connection": "films", "tmdbId": 693134, "qualityProfile": "4K"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no quality profile \"4K\""), "{r}");
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": add(json!("dune"))}))).await;
    assert!(v.to_string().contains("is not an id"), "{v}");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "radarr.queue", "version": 1, "config": {"connection": "films"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let item = &step(&r, "q")["output"]["items"][0];
    assert_eq!(
        (&item["title"], &item["progress"], &item["status"]),
        (&json!("Dune: Part Two (2024)"), &json!(75.0), &json!("downloading"))
    );
}

fn sonarr_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/api/v3/series/lookup?term=tvdb%3A371980") => {
            (200, json!([{"title": "Severance", "year": 2022, "tvdbId": 371980, "seasons": []}]))
        }
        p if p.starts_with("/api/v3/series/lookup?term=") => {
            (200, json!([{"title": "Severance", "year": 2022, "tvdbId": 371980}]))
        }
        p if p.starts_with("/api/v3/series?tvdbId=") => (200, json!([])),
        "/api/v3/series" => (201, json!({"id": 3, "title": "Severance", "year": 2022, "tvdbId": 371980})),
        "/api/v3/qualityprofile" => (200, json!([{"id": 6, "name": "WEB-1080p"}])),
        p if p.starts_with("/api/v3/queue?") => (
            200,
            json!({"records": [{"title": "Severance.S02E03", "size": 10.0, "sizeleft": 10.0, "status": "queued",
                "series": {"title": "Severance"}, "episode": {"seasonNumber": 2, "episodeNumber": 3}}]}),
        ),
        _ => (404, json!({"message": "not found"})),
    }
}

#[tokio::test]
async fn sonarr_adds_a_series_with_the_connections_defaults() {
    let (url, seen) = fake_server(sonarr_answer).await;
    let env = setup().await;
    connection(&env, "tv", "sonarr", &url, json!({"rootFolder": "/data/tv", "qualityProfile": 6}), Some("sonarr-key"))
        .await;
    let wf = create(
        &env,
        manual_then(json!({"id": "add", "kind": "sonarr.add", "version": 1,
            "config": {"connection": "tv", "tvdbId": 371980, "monitor": "future", "search": false}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!((r["status"].as_str(), step(&r, "add")["port"].as_str()), (Some("succeeded"), Some("added")), "{r}");
    assert_eq!(step(&r, "add")["output"]["tvdbId"], 371980);
    let (_, _, body, key) = seen.all().into_iter().find(|(m, _, _, _)| m == "POST").unwrap();
    assert_eq!(key.as_deref(), Some("sonarr-key"));
    assert_eq!((&body["qualityProfileId"], &body["rootFolderPath"]), (&json!(6), &json!("/data/tv")));
    assert_eq!(body["addOptions"], json!({"monitor": "future", "searchForMissingEpisodes": false}));
    assert!(!seen.all().iter().any(|(_, p, _, _)| p == "/api/v3/rootfolder"), "the connection's folder is used");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "sonarr.queue", "version": 1, "config": {"connection": "tv"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "q")["output"]["items"][0]["title"], "Severance S02E03");

    // A monitor option Sonarr doesn't have fails validation; a Radarr kind refuses a Sonarr connection.
    let doc = manual_then(json!({"id": "add", "kind": "sonarr.add", "version": 1,
        "config": {"connection": "tv", "tvdbId": 1, "monitor": "sometimes"}}));
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert!(v.to_string().contains("monitor \\\"sometimes\\\""), "{v}");
    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "radarr.queue", "version": 1, "config": {"connection": "tv"}})),
    )
    .await;
    assert!(
        run_now(&env, &wf, json!({})).await["error"].as_str().unwrap().contains("is a sonarr connection, not radarr")
    );
}

fn plex_answer(path: &str, _: usize) -> (u16, Value) {
    match path {
        p if p.starts_with("/hub/search?query=dune") => (
            200,
            json!({"MediaContainer": {"Hub": [{"type": "movie", "Metadata": [
                {"title": "Dune", "type": "movie", "year": 2021, "ratingKey": "12", "librarySectionTitle": "Films"}]}]}}),
        ),
        "/library/sections" => (
            200,
            json!({"MediaContainer": {"Directory": [{"key": "1", "title": "Films"}, {"key": "2", "title": "TV"}]}}),
        ),
        p if p.starts_with("/library/sections/2/recentlyAdded?") => (
            200,
            json!({"MediaContainer": {"Metadata": [
                {"title": "Pilot", "type": "episode", "ratingKey": "50", "grandparentTitle": "Severance", "addedAt": 1},
                {"title": "Two", "type": "episode", "ratingKey": "51", "grandparentTitle": "Severance", "addedAt": 2}]}}),
        ),
        "/library/sections/all/refresh" => (200, Value::Null),
        _ => (404, json!({})),
    }
}

#[tokio::test]
async fn plex_searches_lists_whats_new_and_refreshes() {
    let (url, seen) = fake_server(plex_answer).await;
    let env = setup().await;
    connection(&env, "home", "plex", &url, json!({}), Some("plex-token")).await;
    let doc = json!({"name": "Plex", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        {"id": "find", "kind": "plex.search", "version": 1, "config": {"connection": "home", "query": "dune"}},
        {"id": "new", "kind": "plex.recently_added", "version": 1,
            "config": {"connection": "home", "section": "tv", "limit": 1}},
        {"id": "scan", "kind": "plex.refresh", "version": 1, "config": {"connection": "home"}}],
        "edges": [{"from": "go", "to": "find"}, {"from": "find", "to": "new"}, {"from": "new", "to": "scan"}]});
    let wf = create(&env, doc).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "find")["output"]["items"][0]["ratingKey"], "12");
    let new = step(&r, "new")["output"]["items"].as_array().unwrap().clone();
    assert_eq!((new.len(), &new[0]["show"]), (1, &json!("Severance")));
    assert_eq!(step(&r, "scan")["output"]["section"], "all");
    let all = seen.all();
    assert!(all.iter().all(|(_, _, _, key)| key.as_deref() == Some("plex-token")));
    assert!(all.iter().any(|(_, p, _, _)| p.contains("X-Plex-Container-Size=1")), "{all:?}");
}

#[tokio::test]
async fn media_dry_runs_call_nothing() {
    let (url, seen) = fake_server(radarr_answer).await;
    let env = setup().await;
    connection(&env, "films", "radarr", &url, json!({}), Some("k")).await;
    for node in [
        json!({"id": "a", "kind": "radarr.add", "version": 1, "config": {"connection": "films", "tmdbId": 693134}}),
        json!({"id": "a", "kind": "radarr.search", "version": 1, "config": {"connection": "films", "term": "x"}}),
        json!({"id": "a", "kind": "plex.refresh", "version": 1, "config": {"connection": "films"}}),
        json!({"id": "a", "kind": "scryer.add", "version": 1,
               "config": {"connection": "films", "facet": "movie", "tmdbId": 693134}}),
        json!({"id": "a", "kind": "scryer.library", "version": 1, "config": {"connection": "films"}}),
        json!({"id": "a", "kind": "scryer.query", "version": 1,
               "config": {"connection": "films", "query": "query { indexers { id } }"}}),
        json!({"id": "a", "kind": "scryer.mutate", "version": 1,
               "config": {"connection": "films", "mutation": "deleteIndexerConfig", "args": {"id": "1"}}}),
    ] {
        let doc = manual_then(node);
        let (s, d) =
            call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": doc, "event": {"type": "manual"}})))
                .await;
        assert_eq!(s, StatusCode::OK, "{d}");
        assert!(d["run"]["steps"][1]["output"]["would"].is_string(), "{d}");
    }
    assert!(seen.all().is_empty());
}

// ---- Scryer (PDT-147) ----

#[tokio::test]
async fn scryer_searches_adds_once_and_reads_its_queue_and_library() {
    use common::scryer::{FakeScryer, KEY};
    let (url, scryer) = FakeScryer::serve().await;
    let env = setup().await;
    connection(&env, "media", "scryer", &url, json!({}), Some(KEY)).await;

    // A series search says which result the library has.
    let wf = create(
        &env,
        manual_then(json!({"id": "find", "kind": "scryer.search", "version": 1,
            "config": {"connection": "media", "term": "{{trigger.term}}", "facet": "series"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"term": "severance"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let results = &step(&r, "find")["output"]["results"];
    assert_eq!((&results[0]["tvdbId"], &results[0]["tmdbId"]), (&json!("371980"), &json!(95396)));
    assert_eq!((&results[0]["inLibrary"], &results[0]["titleId"]), (&json!(true), &json!("t1")));
    assert_eq!(results[1]["inLibrary"], false);
    let seen = scryer.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|(path, _, _, auth)| path == "/scryer/graphql" && *auth == format!("Bearer {KEY}")));

    // A new film by its TMDB id: named by Scryer's metadata, put in the named library, monitored.
    let add = |config: Value| manual_then(json!({"id": "add", "kind": "scryer.add", "version": 1, "config": config}));
    let wf = create(
        &env,
        add(json!({"connection": "media", "facet": "movie", "tmdbId": "{{trigger.id}}", "library": "kids"})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "add")["port"], "added");
    assert_eq!(
        (&step(&r, "add")["output"]["name"], &step(&r, "add")["output"]["library"]),
        (&json!("Dune"), &json!("Kids"))
    );
    let input = scryer.title("Dune").unwrap()["input"].clone();
    assert_eq!(
        (&input["facet"], &input["libraryId"], &input["monitored"]),
        (&json!("MOVIE"), &json!("L2"), &json!(true))
    );
    assert_eq!((&input["tmdbId"], &input["imdbId"], &input["smgId"]), (&json!(438631), &json!("tt1160419"), &json!(7)));
    assert_eq!(input["options"], json!({"monitorType": "ALL_EPISODES"}));

    // Again: it is there now, so `exists` and nothing added.
    let r = run_now(&env, &wf, json!({"id": 438631})).await;
    assert_eq!(step(&r, "add")["port"], "exists", "{r}");
    assert_eq!(scryer.ops().iter().filter(|o| *o == "PdtAddTitle").count(), 1);

    // An id Scryer doesn't know, or a library it doesn't have, fails; a missing id fails validation.
    let r = run_now(&env, &wf, json!({"id": 1})).await;
    assert!(r["error"].as_str().unwrap().contains("no metadata for that id"), "{r}");
    let wf =
        create(&env, add(json!({"connection": "media", "facet": "movie", "tmdbId": 693134, "library": "4K"}))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no movie library \"4K\" (libraries: Films, Kids)"), "{r}");
    let doc = add(json!({"connection": "media", "facet": "series"}));
    let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    assert!(v.to_string().contains("tvdbId, tmdbId or imdbId"), "{v}");

    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "scryer.queue", "version": 1, "config": {"connection": "media"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let out = &step(&r, "q")["output"];
    assert_eq!(
        (&out["total"], &out["items"][0]["status"], &out["items"][0]["secondsLeft"]),
        (&json!(3), &json!("downloading"), &json!(600))
    );
    assert_eq!(
        (&out["items"][1]["status"], &out["items"][1]["attention"]),
        (&json!("importBlocked"), &json!("no matching file"))
    );

    let wf = create(
        &env,
        manual_then(json!({"id": "lib", "kind": "scryer.library", "version": 1,
            "config": {"connection": "media", "facet": "movie", "monitored": true}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    let out = &step(&r, "lib")["output"];
    assert_eq!((&out["total"], &out["titles"][0]["name"], &out["hasMore"]), (&json!(1), &json!("Dune"), &json!(false)));

    // A wrong key says so.
    connection(&env, "bad", "scryer", &url, json!({}), Some("ska_wrong")).await;
    let wf = create(
        &env,
        manual_then(json!({"id": "q", "kind": "scryer.queue", "version": 1, "config": {"connection": "bad"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("(the API key is wrong)"), "{r}");
}

/// Any query or mutation (PDT-150), checked against the bundled schema when Scryer doesn't answer introspection, with
/// a connection's secret filled in and credentials redacted from the output.
#[tokio::test]
async fn scryer_runs_any_query_or_mutation_checked_against_its_schema() {
    use common::scryer::{FakeScryer, KEY};
    let (url, scryer) = FakeScryer::serve().await;
    scryer.introspection.store(false, std::sync::atomic::Ordering::SeqCst);
    let env = setup().await;
    connection(&env, "media", "scryer", &url, json!({}), Some(KEY)).await;
    connection(&env, "sab", "http", "http://nas:8080", json!({}), Some("sab-pass-456")).await;

    let query = |q: &str| {
        manual_then(
            json!({"id": "q", "kind": "scryer.query", "version": 1, "config": {"connection": "media", "query": q}}),
        )
    };
    let wf = create(&env, query("query Settings { generalSettings { historyRetentionDays } }")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "q")["output"]["data"]["generalSettings"]["historyRetentionDays"], 30);
    // The bundled schema doesn't have what only a newer Scryer has: nothing is sent.
    let n = scryer.seen.lock().unwrap().len();
    let wf = create(&env, query("query { fakeVersion }")).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("fakeVersion"), "{r}");
    let ops = scryer.ops();
    assert!(ops[n..].iter().all(|o| o == "PdtIntrospection"), "only the schema was asked for: {ops:?}");

    // A mutation with a password by reference.
    let wf = create(
        &env,
        manual_then(json!({"id": "m", "kind": "scryer.mutate", "version": 1, "config": {"connection": "media",
            "mutation": "createDownloadClientConfig", "args": {"input": {"name": "{{trigger.name}}",
            "clientType": "sabnzbd", "config": [{"key": "password", "secretValue": {"$secret": "sab"}}]}},
            "select": "id name config { key defaultValue }"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"name": "SABnzbd"})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let out = &step(&r, "m")["output"];
    assert_eq!((&out["mutation"], &out["result"]["name"]), (&json!("createDownloadClientConfig"), &json!("SABnzbd")));
    assert_eq!(out["result"]["config"][0]["defaultValue"], "[redacted]", "{out}");
    assert_eq!(scryer.clients.lock().unwrap()[0]["input"]["config"][0]["secretValue"], "sab-pass-456");
    assert!(!r.to_string().contains("sab-pass-456"), "{r}");

    // A reference to a connection that isn't there fails before anything is sent.
    let wf = create(
        &env,
        manual_then(json!({"id": "m", "kind": "scryer.mutate", "version": 1, "config": {"connection": "media",
            "mutation": "createIndexerConfig", "args": {"input": {"name": "x", "providerType": "newznab",
            "config": [{"key": "apiKey", "secretValue": {"$secret": "nope"}}]}}}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("no connection is called nope"), "{r}");
    assert!(scryer.indexers.lock().unwrap().is_empty());

    // Validation: a query node runs no mutations, and signing in is never sent.
    for (node, says) in [
        (
            json!({"kind": "scryer.query", "config": {"connection": "media", "query": "mutation { triggerRssSync { __typename } }"}}),
            "queries only",
        ),
        (json!({"kind": "scryer.query", "config": {"connection": "media", "query": "query { "}}), "doesn't parse"),
        (
            json!({"kind": "scryer.mutate", "config": {"connection": "media", "mutation": "loginWithPlex"}}),
            "never sends",
        ),
    ] {
        let mut node = node;
        node["id"] = json!("x");
        node["version"] = json!(1);
        let (_, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": manual_then(node)}))).await;
        assert!(v.to_string().contains(says), "{v}");
    }
}

// ---- brief archive (7e-3) ----

#[tokio::test]
async fn a_brief_is_archived_by_day_and_read_back() {
    let env = setup().await;
    let archive = create(
        &env,
        manual_then(json!({"id": "keep", "kind": "brief.archive", "version": 1, "config": {
            "brief": "films", "title": "{{trigger.title}}", "body": "Today's news.", "date": "{{trigger.date}}",
            "items": "{{trigger.items}}"}})),
    )
    .await;
    for (date, title) in [("2026-10-20", "Monday"), ("2026-10-21", "Tuesday"), ("2026-10-21", "Tuesday, later")] {
        let items = json!([{"title": format!("{title} story"), "link": "https://x/1", "note": "a note"}]);
        let r = run_now(&env, &archive, json!({"date": date, "title": title, "items": items})).await;
        assert_eq!(r["status"], "succeeded", "{r}");
    }
    let dir = env.state.data_dir.join("briefs").join("films");
    let day = std::fs::read_to_string(dir.join("2026-10-21.md")).unwrap();
    assert!(
        day.starts_with(
            "# films — 2026-10-21\n\n## Tuesday\n\nToday's news.\n\n- [Tuesday story](https://x/1) — a note\n"
        ),
        "{day}"
    );
    assert!(day.contains("## Tuesday, later"));
    assert_eq!(std::fs::read_to_string(dir.join("index.jsonl")).unwrap().lines().count(), 3);

    let read = |config: Value| manual_then(json!({"id": "back", "kind": "brief.read", "version": 1, "config": config}));
    let wf = create(&env, read(json!({"brief": "films", "last": 2}))).await;
    let r = run_now(&env, &wf, json!({})).await;
    let out = &step(&r, "back")["output"];
    let titles: Vec<_> = out["entries"].as_array().unwrap().iter().map(|e| e["title"].clone()).collect();
    assert_eq!(titles, [json!("Tuesday"), json!("Tuesday, later")]);
    assert_eq!(out["entries"][0]["items"], json!([{"title": "Tuesday story", "link": "https://x/1"}]));
    assert_eq!(out["days"].as_array().unwrap().len(), 1);
    assert_eq!(out["days"][0]["markdown"], json!(day));

    let wf = create(&env, read(json!({"brief": "films", "since": "2026-10-20"}))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "back")["output"]["days"].as_array().unwrap().len(), 2);
    let wf = create(&env, read(json!({"brief": "nothing-yet"}))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "back")["output"], json!({"entries": [], "days": []}));

    // A name that would leave the folder is refused, before and when run.
    let (_, v) =
        call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": read(json!({"brief": "../x"}))}))).await;
    assert!(v.to_string().contains("not a brief name"), "{v}");
    let r = run_now(&env, &archive, json!({"date": "2026-10-22", "title": "x", "items": "not a list"})).await;
    assert_eq!(r["status"], "failed");
}

// ---- MCP tools as actions (7e-3) ----

fn fake_mcp() -> &'static str {
    env!("CARGO_BIN_EXE_fake-mcp")
}

async fn connection_id(env: &Env, name: &str) -> String {
    let (_, all) = call(&env.app, "GET", "/api/connections", None).await;
    all.as_array().unwrap().iter().find(|c| c["name"] == name).unwrap()["id"].as_str().unwrap().to_string()
}

fn tool_node(id: &str, server: &str, tool: &str, arguments: Value) -> Value {
    json!({"id": id, "kind": "mcp.tool", "version": 1, "config": {"server": server, "tool": tool, "arguments": arguments}})
}

#[tokio::test]
async fn a_stdio_mcp_servers_tools_are_listed_and_called() {
    let env = setup().await;
    connection(
        &env,
        "fake",
        "mcp",
        "",
        json!({"transport": "stdio", "command": fake_mcp(), "secretEnv": "FAKE_MCP_SECRET"}),
        Some("s3cret"),
    )
    .await;
    let id = connection_id(&env, "fake").await;
    let (s, listed) = call(&env.app, "POST", &format!("/api/connections/{id}/tools"), None).await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    assert_eq!(listed["error"], Value::Null, "{listed}");
    let names: Vec<_> = listed["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["echo", "add", "fail", "whoami", "hang"]);
    let (_, cached) = call(&env.app, "GET", &format!("/api/connections/{id}/tools"), None).await;
    assert_eq!(cached["tools"], listed["tools"]);

    // Each tool is in the catalog, as a preset of mcp.tool.
    let (_, catalog) = call(&env.app, "GET", "/api/workflows/catalog", None).await;
    let echo = catalog.as_array().unwrap().iter().find(|k| k["id"] == "mcp:fake/echo").expect("the echo tool");
    assert_eq!((&echo["title"], &echo["category"]), (&json!("Echo (fake)"), &json!("action")));
    assert_eq!(
        echo["preset"],
        json!({"kind": "mcp.tool", "config": {"server": "fake", "tool": "echo", "arguments": {}}})
    );
    assert_eq!(echo["configSchema"]["properties"]["arguments"]["required"], json!(["text"]));

    let doc = json!({"name": "MCP", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        tool_node("echo", "fake", "echo", json!({"text": "{{trigger.word}}", "times": 2})),
        tool_node("add", "fake", "add", json!({"a": 1, "b": "{{trigger.n}}"})),
        tool_node("who", "fake", "whoami", json!({})),
        tool_node("again", "fake", "whoami", json!({}))],
        "edges": [{"from": "go", "to": "echo"}, {"from": "echo", "to": "add"}, {"from": "add", "to": "who"},
            {"from": "who", "to": "again"}]});
    let wf = create(&env, doc).await;
    let r = run_now(&env, &wf, json!({"word": "hi", "n": 2})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "echo")["output"], "hihi", "text content");
    assert_eq!(step(&r, "add")["output"], json!({"sum": 3.0}), "structured content");
    let who = &step(&r, "who")["output"];
    assert_eq!(who["secret"], "s3cret", "the secret is in the env var the connection names");
    assert_eq!(step(&r, "again")["output"]["pid"], who["pid"], "one process serves the calls");

    // A changed connection starts the server again.
    let (s, _) = call(
        &env.app,
        "PATCH",
        &format!("/api/connections/{id}"),
        Some(json!({"config": {"transport": "stdio", "command": fake_mcp(), "secretEnv": "FAKE_MCP_SECRET",
            "env": {"X": "1"}}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let r = run_now(&env, &wf, json!({"word": "hi", "n": 2})).await;
    assert_ne!(step(&r, "who")["output"]["pid"], who["pid"]);

    // A tool error fails the step.
    let wf = create(&env, manual_then(tool_node("x", "fake", "fail", json!({})))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "failed");
    assert!(r["error"].as_str().unwrap().contains("it broke"), "{r}");

    // Validation checks arguments against the cached schema, and that the tool exists.
    let doc = json!({"name": "x", "nodes": [{"id": "go", "kind": "manual", "version": 1},
        tool_node("a", "fake", "echo", json!({"times": "two"})), tool_node("b", "fake", "nope", json!({}))],
        "edges": [{"from": "go", "to": "a"}, {"from": "a", "to": "b"}]});
    let (_, issues) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": doc}))).await;
    let messages: Vec<_> = issues
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["code"] == "mcp_tool")
        .map(|i| i["message"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        messages,
        [
            "a: echo arguments: text is required",
            "a: echo arguments: times should be integer",
            "b: the MCP server fake has no tool nope"
        ]
    );

    // A dry run calls nothing.
    let (_, d) = call(
        &env.app,
        "POST",
        "/api/workflows/dry-run",
        Some(json!({"doc": manual_then(tool_node("x", "fake", "fail", json!({}))), "event": {"type": "manual"}})),
    )
    .await;
    assert!(d["run"]["steps"][1]["output"]["would"].as_str().unwrap().starts_with("call fail on fake"), "{d}");

    // Deleting the connection drops its tools from the catalog.
    let (s, _) = call(&env.app, "DELETE", &format!("/api/connections/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, catalog) = call(&env.app, "GET", "/api/workflows/catalog", None).await;
    assert!(!catalog.to_string().contains("mcp:fake/"));
    pdt_api::shutdown_mcp(&env.state).await;
}

#[tokio::test]
async fn a_wedged_mcp_server_is_started_again_after_a_call_times_out() {
    let env = setup().await;
    connection(&env, "fake", "mcp", "", json!({"transport": "stdio", "command": fake_mcp()}), None).await;
    let mut hang = tool_node("hang", "fake", "hang", json!({}));
    hang["timeoutMs"] = json!(2000);
    let doc = json!({"name": "Hang", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        tool_node("who", "fake", "whoami", json!({})), hang],
        "edges": [{"from": "go", "to": "who"}, {"from": "who", "to": "hang"}]});
    let wf = create(&env, doc).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "failed", "{r}");
    let error = step(&r, "hang")["error"].as_str().unwrap().to_string();
    assert!(error.contains("did not answer"), "the call's own timeout, naming the server: {error}");
    let first = step(&r, "who")["output"]["pid"].clone();

    let doc = json!({"name": "After", "nodes": [
        {"id": "go", "kind": "manual", "version": 1}, tool_node("who", "fake", "whoami", json!({}))],
        "edges": [{"from": "go", "to": "who"}]});
    let wf = create(&env, doc).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_ne!(step(&r, "who")["output"]["pid"], first, "a new server answered");
    pdt_api::shutdown_mcp(&env.state).await;
}

#[tokio::test]
async fn an_mcp_server_that_does_not_start_says_why() {
    let env = setup().await;
    let missing = env.tmp.path().join("no-such-server");
    connection(&env, "broken", "mcp", "", json!({"command": missing.to_string_lossy()}), None).await;
    let id = connection_id(&env, "broken").await;
    let (_, listed) = call(&env.app, "POST", &format!("/api/connections/{id}/tools"), None).await;
    assert!(listed["error"].as_str().unwrap().contains("could not start"), "{listed}");
    assert_eq!(listed["tools"], json!([]));

    let wf = create(&env, manual_then(tool_node("x", "broken", "anything", json!({})))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert!(r["error"].as_str().unwrap().contains("could not start"), "{r}");

    // A connection of another kind is refused.
    connection(&env, "plain", "http", "http://127.0.0.1:9", json!({}), None).await;
    let plain = connection_id(&env, "plain").await;
    let (s, _) = call(&env.app, "GET", &format!("/api/connections/{plain}/tools"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let wf = create(&env, manual_then(tool_node("x", "plain", "anything", json!({})))).await;
    assert!(run_now(&env, &wf, json!({})).await["error"].as_str().unwrap().contains("not mcp"));
}

/// A streamable-HTTP MCP server answering with plain JSON: `initialize`, `tools/list` (one tool, `add`) and
/// `tools/call`; it wants `Authorization: Bearer tok`.
async fn fake_http_mcp() -> String {
    async fn handle(headers: axum::http::HeaderMap, body: String) -> axum::response::Response {
        use axum::response::IntoResponse;
        if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let Some(id) = msg.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
        let result = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake-http", "version": "1"}}),
            "tools/list" => json!({"tools": [{"name": "add", "description": "Adds.",
                "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}}}]}),
            "tools/call" => {
                let a = &msg["params"]["arguments"];
                let sum = a["a"].as_f64().unwrap_or(0.0) + a["b"].as_f64().unwrap_or(0.0);
                json!({"content": [{"type": "text", "text": format!("{{\"sum\": {sum}}}")}]})
            }
            _ => json!({}),
        };
        axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::post(handle)
            .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
            .delete(|| async { StatusCode::METHOD_NOT_ALLOWED }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/mcp")
}

#[tokio::test]
async fn an_http_mcp_server_gets_the_secret_as_a_bearer_token() {
    let url = fake_http_mcp().await;
    let env = setup().await;
    connection(&env, "remote", "mcp", &url, json!({"transport": "http"}), Some("tok")).await;
    let wf = create(&env, manual_then(tool_node("sum", "remote", "add", json!({"a": 2, "b": 3})))).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "sum")["output"], json!({"sum": 5}), "JSON text is parsed");

    connection(&env, "wrong", "mcp", &url, json!({"transport": "http"}), Some("nope")).await;
    let id = connection_id(&env, "wrong").await;
    let (_, listed) = call(&env.app, "POST", &format!("/api/connections/{id}/tools"), None).await;
    assert!(listed["error"].is_string(), "{listed}");
    pdt_api::shutdown_mcp(&env.state).await;
}
