//! PDT-194 end to end: the built-in "Cut a release" runs the project's release workflow, puts the release PR in the
//! Inbox, waits for its merge and tells the user when the tag's release is published. GitHub is `FakeForge`; the
//! waits are cut short.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{Mergeable, PrState};
use pdt_forge::testing::FakeForge;
use pdt_forge::{ChangeRequest, Release};
use pdt_store::Store;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

const REPO: &str = "example-org/thing";

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    gh: Arc<FakeForge>,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    git(&repo, &["remote", "add", "origin", &format!("https://github.com/{REPO}.git")]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    let gh = Arc::new(FakeForge::default());
    state.github = gh.clone();
    let app = router(state.clone(), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "Thing", "key": "TH", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    Env { app, state, project, gh, _tmp: tmp }
}

fn release(tag: &str) -> Release {
    Release {
        tag: tag.into(),
        name: tag.into(),
        url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
        published_at: Some("2026-10-28T12:00:00Z".into()),
        notes: "### Added\n- things".into(),
    }
}

fn pr(number: u32, state: PrState) -> ChangeRequest {
    ChangeRequest {
        number,
        url: format!("https://github.com/{REPO}/pull/{number}"),
        title: "Release v0.2.0".into(),
        state,
        draft: false,
        head_sha: "abc".into(),
        base_branch: "main".into(),
        mergeable: Mergeable::Mergeable,
        behind: false,
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: None,
    }
}

async fn cut_release(env: &Env) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, all) = call(&env.app, "GET", "/api/workflows", None).await;
    all.as_array().unwrap().iter().find(|w| w["builtin"] == "cut-release").cloned().expect("installed")
}

async fn run(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

/// The node the run waits at now.
async fn waiting_at(env: &Env, id: &str) -> String {
    let r = run(env, id).await;
    let s = r["steps"].as_array().unwrap().iter().rev().find(|s| s["status"] == "waiting");
    s.map(|s| s["nodeId"].as_str().unwrap().to_string()).unwrap_or_else(|| panic!("not waiting: {r}"))
}

/// Cuts the run's current wait short, and lets the engine go on to the next one.
async fn time_passes(env: &Env, id: &str) {
    let store = &env.state.store;
    let mut step = store
        .list_steps(id)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.status == pdt_core::workflow::api::StepStatus::Waiting)
        .expect("a waiting step");
    step.wake_at = Some(1);
    store.put_step(&step).await.unwrap();
    store.wake_workflow_run(id, 1).await.unwrap();
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

async fn inbox_titles(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap_or_default().to_string()).collect()
}

#[tokio::test]
async fn cut_a_release_from_the_action_to_the_published_release() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    assert_eq!(wf["enabled"], false, "off by default");
    env.gh.publish(REPO, release("v0.1.840"));

    let (s, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()),
        Some(json!({"input": {"bump": "minor", "projectId": env.project}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;

    // The release workflow runs on main with the bump (no version given: none sent).
    let started = env.gh.workflow_runs.lock().unwrap().clone();
    let inputs = BTreeMap::from([("bump".to_string(), "minor".to_string())]);
    assert_eq!(started, [(REPO.to_string(), "release.yml".to_string(), "main".to_string(), inputs)]);
    assert_eq!(waiting_at(&env, &id).await, "wait_pr");

    // The last release's PR, merged, is not the new one: it keeps looking until one is open.
    env.gh.set_pr("release", Some(pr(6, PrState::Merged)));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_pr");
    env.gh.set_pr("release", Some(pr(7, PrState::Open)));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    assert!(inbox_titles(&env).await.contains(&"Release PR #7 is ready for review".to_string()));

    // Still open; then merged, but the tag's release isn't out yet; then it is.
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    env.gh.set_pr("release", Some(pr(7, PrState::Merged)));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_release");
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_release");
    env.gh.publish(REPO, release("v0.2.0"));
    time_passes(&env, &id).await;

    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(r["output"]["tag"], "v0.2.0");
    assert!(inbox_titles(&env).await.contains(&"Release v0.2.0 is published".to_string()));
    let calls = env.gh.calls.lock().unwrap().clone();
    assert!(!calls.iter().any(|c| c.starts_with("merge ")), "PDT never merges the release PR: {calls:?}");
}

#[tokio::test]
async fn a_release_pr_closed_unmerged_releases_nothing() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()),
        Some(json!({"input": {"version": "1.0.0", "projectId": env.project}})),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    let started = env.gh.workflow_runs.lock().unwrap().clone();
    assert_eq!(started[0].3, BTreeMap::from([("version".to_string(), "1.0.0".to_string())]));

    env.gh.set_pr("release", Some(pr(9, PrState::Open)));
    time_passes(&env, &id).await;
    env.gh.set_pr("release", Some(pr(9, PrState::Closed)));
    time_passes(&env, &id).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let last = r["steps"].as_array().unwrap().last().unwrap();
    assert_eq!(last["nodeId"], "not_released", "{r}");
}

#[tokio::test]
async fn the_builtin_dry_runs_without_calling_github() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/workflows/dry-run",
        Some(json!({"workflowId": wf["id"], "event": {"type": "manual", "payload": {"bump": "patch", "projectId": env.project}}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let errors: Vec<_> = r["issues"].as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
    assert!(errors.is_empty(), "{r}");
    let run = &r["run"];
    assert_eq!(run["status"], "succeeded", "{r}");
    let nodes: Vec<&str> = run["steps"].as_array().unwrap().iter().map(|s| s["nodeId"].as_str().unwrap()).collect();
    assert!(nodes.contains(&"dispatch") && nodes.contains(&"released"), "{nodes:?}");
    assert!(env.gh.workflow_runs.lock().unwrap().is_empty());
    let calls = env.gh.calls.lock().unwrap().clone();
    assert!(calls.is_empty(), "a dry run calls nothing: {calls:?}");
}

#[tokio::test]
async fn an_open_release_pr_stops_a_new_release_before_the_action_runs() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    // An earlier release PR is still open: the Action would refuse, and that PR is not the bump asked for.
    env.gh.set_pr("release", Some(pr(5, PrState::Open)));
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()),
        Some(json!({"input": {"bump": "minor", "projectId": env.project}})),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;

    assert!(env.gh.workflow_runs.lock().unwrap().is_empty(), "the release workflow is not started");
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let last = r["steps"].as_array().unwrap().last().unwrap();
    assert_eq!(last["nodeId"], "not_started", "{r}");
    assert!(inbox_titles(&env).await.contains(&"Release PR #5 is still open".to_string()));
}
