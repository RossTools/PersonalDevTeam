//! Buildkite CI (Phase 7e-2, PDT-85) through the API: a started ticket's builds become its CI (with or without a
//! pull request), `ci.*` events with `provider: buildkite` once per build state, and the retry and trigger actions.
//! Buildkite is a fake local server; GitHub is `FakeForge`.

mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use common::{argv_agent, call, git};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState};
use pdt_forge::ChangeRequest;
use pdt_forge::testing::FakeForge;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const TOKEN: &str = "bk-token";

#[derive(Default)]
struct FakeBuildkite {
    /// The builds of pipeline `app`.
    builds: Vec<Value>,
    /// Method, path and query, JSON body.
    seen: Vec<(String, String, Value)>,
    unauthorized: bool,
    /// Every request is refused (a revoked token).
    revoked: bool,
}

type Shared = Arc<Mutex<FakeBuildkite>>;

async fn bk_app(State(b): State<Shared>, req: Request<Body>) -> (StatusCode, axum::Json<Value>) {
    let method = req.method().to_string();
    let uri = req.uri().clone();
    let authed = req.headers().get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {TOKEN}"));
    let bytes = req.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut b = b.lock().unwrap();
    b.seen.push((method.clone(), uri.to_string(), body.clone()));
    if !authed || b.revoked {
        b.unauthorized |= !authed;
        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"message": "no"})));
    }
    let query: std::collections::HashMap<String, String> = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let path = uri.path().trim_start_matches("/v2/organizations/acme/pipelines/").to_string();
    let parts: Vec<&str> = path.split('/').collect();
    let ok = |v: Value| (StatusCode::OK, axum::Json(v));
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["app", "builds"]) => {
            let commit = query.get("commit").cloned().unwrap_or_default();
            ok(json!(b.builds.iter().filter(|x| x["commit"] == commit.as_str()).cloned().collect::<Vec<_>>()))
        }
        ("GET", ["app", "builds", n]) => match b.builds.iter().find(|x| x["number"].as_u64() == n.parse().ok()) {
            Some(x) => ok(x.clone()),
            None => (StatusCode::NOT_FOUND, axum::Json(json!({"message": "No build found"}))),
        },
        ("PUT", ["app", "builds", _, "jobs", _, "retry"]) => ok(json!({"id": "retried"})),
        ("PUT", ["app", "builds", n, "rebuild"]) => {
            let n: u64 = n.parse().unwrap();
            ok(json!({"number": n + 100, "web_url": format!("https://buildkite.com/acme/app/builds/{}", n + 100)}))
        }
        ("POST", ["app", "builds"]) => (
            StatusCode::CREATED,
            axum::Json(json!({"number": 77, "web_url": "https://buildkite.com/acme/app/builds/77"})),
        ),
        _ => (StatusCode::NOT_FOUND, axum::Json(json!({"message": "no route"}))),
    }
}

async fn fake_buildkite() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = axum::Router::new().fallback(bk_app).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), shared)
}

fn build(number: u64, state: &str, commit: &str, retries: u64) -> Value {
    json!({"number": number, "state": state, "commit": commit, "branch": "b",
        "web_url": format!("https://buildkite.com/acme/app/builds/{number}"),
        "created_at": format!("2026-10-22T10:00:{number:02}Z"),
        "jobs": [
            {"id": "job-test", "type": "script", "name": "test", "retries_count": retries, "retryable": true,
             "state": if state == "failed" { "failed" } else { "passed" }},
            {"id": "job-build", "type": "script", "name": "build", "state": "passed"}]})
}

struct Env {
    app: axum::Router,
    state: AppState,
    gh: Arc<FakeForge>,
    bk: Shared,
    ticket: String,
    head: String,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    git(&repo, &["config", "http.proxy", "http://127.0.0.1:1"]);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let gh = Arc::new(FakeForge::default());
    state.github = gh.clone();
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Work", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let pid = p["id"].as_str().unwrap().to_string();
    let (url, bk) = fake_buildkite().await;
    let (s, c) = call(
        &app,
        "POST",
        "/api/connections",
        Some(json!({"name": "bk", "kind": "buildkite", "url": url, "config": {"org": "acme"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let cid = c["id"].as_str().unwrap();
    call(&app, "PUT", &format!("/api/connections/{cid}/secret"), Some(json!({"secret": TOKEN}))).await;
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": null,
            "buildkite": {"connection": "bk", "pipelines": ["app", " app "]}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["buildkite"], json!({"connection": "bk", "pipelines": ["app"]}));
    let (_, t) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Ship it"}))).await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let out = std::process::Command::new("git").arg("-C").arg(&repo).args(["rev-parse", "main"]).output().unwrap();
    let head = String::from_utf8(out.stdout).unwrap().trim().to_string();
    Env { app, state, gh, bk, ticket, head, _tmp: tmp }
}

async fn attempt(env: &Env) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await.1["attempt"].clone()
}

async fn events(env: &Env, kind: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/events?type={kind}"), None).await;
    v.as_array().unwrap().clone()
}

#[tokio::test]
async fn a_branch_without_a_pr_gets_its_builds_once_per_state() {
    let env = setup().await;
    env.bk.lock().unwrap().builds = vec![build(1, "failed", &env.head, 0)];
    pdt_api::refresh_buildkite(&env.state).await;
    pdt_api::refresh_buildkite(&env.state).await;
    assert!(!env.bk.lock().unwrap().unauthorized, "sent the token");
    let seen = env.bk.lock().unwrap().seen.clone();
    assert!(seen[0].1.contains("branch=pdt-") && seen[0].1.contains(&format!("commit={}", env.head)), "{seen:?}");
    let a = attempt(&env).await;
    assert!(a["pr"].is_null());
    assert_eq!(a["ci"]["state"], "failed", "{a}");
    assert_eq!(a["ci"]["runs"][0]["failedJobs"], json!(["test"]));
    let failed = events(&env, "ci.run_failed").await;
    assert_eq!(failed.len(), 1, "once: {failed:?}");
    let p = &failed[0]["payload"];
    assert_eq!((p["provider"].clone(), p["pr"].clone()), (json!("buildkite"), Value::Null));
    assert!(p["notice"]["text"].as_str().unwrap().starts_with("[PDT] Buildkite CI failed on pdt-"), "{p}");
    // The built-in PR notices tell the agent.
    pdt_api::run_workflows(&env.state).await;
    let (_, runs) = call(&env.app, "GET", "/api/workflow-runs", None).await;
    assert!(runs.as_array().unwrap().iter().any(|r| r["workflowName"] == "PR notices"), "{runs}");

    // A retry that fails again is news; passing then is too.
    env.bk.lock().unwrap().builds = vec![build(1, "failed", &env.head, 1)];
    pdt_api::refresh_buildkite(&env.state).await;
    assert_eq!(events(&env, "ci.run_failed").await.len(), 2);
    env.bk.lock().unwrap().builds = vec![build(1, "failed", &env.head, 1), build(2, "passed", &env.head, 0)];
    pdt_api::refresh_buildkite(&env.state).await;
    pdt_api::refresh_buildkite(&env.state).await;
    let passed = events(&env, "ci.run_passed").await;
    assert_eq!(passed.len(), 1);
    assert_eq!(passed[0]["payload"]["provider"], "buildkite");
    assert_eq!(attempt(&env).await["ci"]["state"], "passed", "the newest build of the pipeline");
}

#[tokio::test]
async fn a_revoked_token_notifies_once_until_it_works_again() {
    let env = setup().await;
    env.bk.lock().unwrap().builds = vec![build(1, "running", &env.head, 0)];
    env.bk.lock().unwrap().revoked = true;
    let live = || async {
        let (_, v) = call(&env.app, "GET", "/api/notifications?section=updates", None).await;
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["kind"] == "connection.failed" && n["clearedAt"].is_null())
            .cloned()
            .collect::<Vec<_>>()
    };
    for _ in 0..3 {
        pdt_api::refresh_buildkite(&env.state).await;
    }
    let n = live().await;
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0]["title"], "bk is failing");
    assert!(n[0]["body"].as_str().unwrap().contains("401"));
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(list[0]["lastError"].as_str().unwrap().contains("401"), "{list}");

    env.bk.lock().unwrap().revoked = false;
    pdt_api::refresh_buildkite(&env.state).await;
    assert!(live().await.is_empty());
    assert_eq!(attempt(&env).await["ci"]["state"], "running");
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(list[0]["lastError"].is_null());
}

#[tokio::test]
async fn a_pr_shows_both_providers() {
    let env = setup().await;
    let branch = attempt(&env).await["branch"].as_str().unwrap().to_string();
    env.gh.set_pr(
        &branch,
        Some(ChangeRequest {
            number: 12,
            url: "https://github.com/o/r/pull/12".into(),
            title: "Ship it".into(),
            state: PrState::Open,
            draft: false,
            head_sha: env.head.clone(),
            base_branch: "main".into(),
            mergeable: Mergeable::Mergeable,
            behind: false,
            auto_merge: false,
            in_merge_queue: false,
            queue_position: None,
            queue_removal: None,
            created_at: None,
        }),
    );
    env.gh.set_runs(
        &branch,
        vec![CiRun {
            id: 5,
            attempt: 1,
            workflow: "ci".into(),
            state: CiState::Passed,
            url: "https://github.com/o/r/actions/runs/5".into(),
            head_sha: env.head.clone(),
            head_branch: branch.clone(),
            created_at: "2026-10-22T00:00:00Z".into(),
            failed_jobs: vec![],
        }],
    );
    env.bk.lock().unwrap().builds = vec![build(3, "failed", &env.head, 0)];
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::refresh_buildkite(&env.state).await;
    let a = attempt(&env).await;
    assert!(a["ci"].is_null(), "with a PR, Buildkite is in its CI");
    assert_eq!(a["pr"]["ci"]["state"], "failed", "{a}");
    assert_eq!(a["pr"]["ci"]["runs"].as_array().unwrap().len(), 2);
    assert_eq!(a["pr"]["ci"]["url"], "https://buildkite.com/acme/app/builds/3");
    let failed = events(&env, "ci.run_failed").await;
    assert_eq!(failed.len(), 1, "GitHub passed; only Buildkite failed");
    assert_eq!(failed[0]["payload"]["pr"], 12);
    assert!(failed[0]["payload"]["notice"]["text"].as_str().unwrap().contains("PR #12"));
}

fn manual_then(node: Value) -> Value {
    json!({"name": "Test", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
        "edges": [{"from": "go", "to": node["id"]}]})
}

async fn run_once(env: &Env, node: Value) -> Value {
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": manual_then(node), "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let run = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await.1;
    assert_eq!(run["status"], "succeeded", "{run}");
    run["steps"].as_array().unwrap().last().cloned().unwrap()["output"].clone()
}

#[tokio::test]
async fn retry_and_trigger() {
    let env = setup().await;
    env.bk.lock().unwrap().builds = vec![build(4, "failed", "c", 0), build(5, "passed", "c", 0)];
    let out = run_once(
        &env,
        json!({"id": "r", "kind": "buildkite.retry", "version": 1,
            "config": {"connection": "bk", "build": "https://buildkite.com/acme/app/builds/4"}}),
    )
    .await;
    assert_eq!(out["retried"], json!(["test"]), "{out}");
    assert!(
        env.bk.lock().unwrap().seen.iter().any(|(m, p, _)| m == "PUT" && p.ends_with("/builds/4/jobs/job-test/retry"))
    );
    let out = run_once(
        &env,
        json!({"id": "r", "kind": "buildkite.retry", "version": 1,
            "config": {"connection": "bk", "build": 5, "pipeline": "app"}}),
    )
    .await;
    assert_eq!((out["rebuilt"].clone(), out["number"].clone()), (json!(true), json!(105)), "nothing failed: rebuilt");
    let out = run_once(
        &env,
        json!({"id": "t", "kind": "buildkite.trigger", "version": 1,
            "config": {"connection": "bk", "pipeline": "app", "branch": "main", "env": {"DEPLOY": "1"}}}),
    )
    .await;
    assert_eq!(out, json!({"number": 77, "url": "https://buildkite.com/acme/app/builds/77"}));
    let seen = env.bk.lock().unwrap().seen.clone();
    let (_, _, body) = seen.iter().find(|(m, _, _)| m == "POST").unwrap();
    assert_eq!((body["commit"].clone(), body["env"]["DEPLOY"].clone()), (json!("HEAD"), json!("1")));
    assert_eq!(body["message"], "Started by Test");
    // Config checks.
    let (_, v) = call(
        &env.app,
        "POST",
        "/api/workflows/validate",
        Some(json!({"doc": manual_then(json!({"id": "r", "kind": "buildkite.retry", "version": 1,
            "config": {"connection": "bk", "build": 5}}))})),
    )
    .await;
    assert!(v.to_string().contains("needs its pipeline"), "{v}");
}
