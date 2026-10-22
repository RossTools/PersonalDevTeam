//! Phase 7e's triggers and actions through the API: schedules, webhooks, GitHub issues, `command`, `http.request`,
//! `discord.post` and `notify`, and the Hermes templates. External services are local fake servers and `FakeGithub`.

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

/// One request a fake server got: method, path and query, JSON body, Authorization.
type Request1 = (String, String, Value, Option<String>);

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<Request1>>>);

impl Seen {
    fn all(&self) -> Vec<Request1> {
        self.0.lock().unwrap().clone()
    }
}

/// A server on a port the OS picks: records each request (method, path and query, JSON body, Authorization) and
/// answers what `answer` says for its path.
async fn fake_server(answer: fn(&str, usize) -> (u16, Value)) -> (String, Seen) {
    let seen = Seen::default();
    let s2 = seen.clone();
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let seen = s2.clone();
        async move {
            let method = req.method().to_string();
            let path = req.uri().to_string();
            let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).map(String::from);
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
async fn notify_records_a_notification_event() {
    let env = setup().await;
    let wf = create(
        &env,
        manual_then(json!({"id": "tell", "kind": "notify", "version": 1, "config": {
            "title": "Build {{trigger.n}} broke", "body": "see logs", "level": "error", "link": "PDT-1"}})),
    )
    .await;
    let r = run_now(&env, &wf, json!({"n": 7})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, events) = call(&env.app, "GET", "/api/events?type=pdt.notified", None).await;
    let e = &events.as_array().unwrap()[0];
    assert_eq!(
        (&e["payload"]["title"], &e["payload"]["level"], &e["payload"]["link"], &e["payload"]["workflowId"]),
        (&json!("Build 7 broke"), &json!("error"), &json!("PDT-1"), &json!(wf))
    );
    // Not something a workflow can start on.
    let (_, cat) = call(&env.app, "GET", "/api/workflows/catalog", None).await;
    assert!(!cat.as_array().unwrap().iter().any(|k| k["id"] == "pdt.notified"));
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
