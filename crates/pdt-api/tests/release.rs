//! PDT-194 and PDT-214 end to end: the built-in "Cut a release" runs the project's release workflow, says the release
//! PR is open, waits for it to merge itself (a failed check goes to the Inbox) and tells the user when the tag's
//! release is published. GitHub is `FakeForge`; the waits are cut short.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState, QueueRemoval};
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

/// The titles of the notifications (all sections).
async fn notified(env: &Env) -> Vec<String> {
    let mut titles = vec![];
    for section in ["updates", "needs_you"] {
        let (_, v) = call(&env.app, "GET", &format!("/api/notifications?section={section}"), None).await;
        titles.extend(v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()));
    }
    titles
}

/// One CI run on the release branch at `sha`.
fn ci_run(id: u64, workflow: &str, sha: &str, state: CiState) -> CiRun {
    CiRun {
        id,
        attempt: 1,
        workflow: workflow.into(),
        state,
        url: format!("https://github.com/{REPO}/actions/runs/{id}"),
        head_sha: sha.into(),
        head_branch: "release".into(),
        created_at: format!("2026-10-28T12:00:{:02}Z", id % 60),
        failed_jobs: vec![],
    }
}

/// Starts Cut a release with a minor bump, through to waiting for PR #7 (open) to merge.
async fn release_pr_open(env: &Env) -> String {
    let wf = cut_release(env).await;
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()),
        Some(json!({"input": {"bump": "minor", "projectId": env.project}})),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    env.gh.set_pr("release", Some(pr(7, PrState::Open)));
    time_passes(env, &id).await;
    assert_eq!(waiting_at(env, &id).await, "wait_merge");
    id
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
    // Told, not asked: the PR merges itself, so nothing waits in the Inbox.
    assert!(notified(&env).await.contains(&"Release PR #7 is open".to_string()));
    assert!(inbox_titles(&env).await.is_empty(), "{:?}", inbox_titles(&env).await);

    // Still open, its checks running; then merged, but the tag's release isn't out yet; then it is.
    env.gh.set_runs("release", vec![ci_run(1, "ci", "abc", CiState::Running)]);
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
async fn a_failed_check_goes_to_the_inbox_and_the_release_waits_for_the_fix() {
    let env = setup().await;
    let id = release_pr_open(&env).await;

    // A bundle build fails on the PR's head: the Inbox says which, once, and it keeps waiting (not merged).
    env.gh
        .set_runs("release", vec![ci_run(1, "ci", "abc", CiState::Failed), ci_run(2, "lint", "abc", CiState::Passed)]);
    env.gh.set_jobs(1, &["bundle (macos-latest, app)"]);
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    let title = "Release PR #7 failed its checks".to_string();
    assert_eq!(inbox_titles(&env).await, std::slice::from_ref(&title));
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let text = inbox["items"][0]["body"]["text"].as_str().unwrap().to_string();
    assert!(text.contains("- ci: bundle (macos-latest, app)"), "{text}");
    assert!(text.contains("/actions/runs/1"), "{text}");
    assert!(notified(&env).await.contains(&title));
    time_passes(&env, &id).await;
    assert_eq!(inbox_titles(&env).await.len(), 1, "told once per failure");

    // A fix pushed to the release branch passes: it merges itself and is published.
    let mut fixed = pr(7, PrState::Open);
    fixed.head_sha = "def".into();
    env.gh.set_pr("release", Some(fixed.clone()));
    env.gh.set_runs("release", vec![ci_run(1, "ci", "abc", CiState::Failed), ci_run(3, "ci", "def", CiState::Passed)]);
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    // Then the merge queue's run fails and removes it (auto-merge off): that is news too.
    fixed.queue_removal = Some(QueueRemoval { id: "q1".into(), reason: "failed_checks".into(), by_queue: true });
    env.gh.set_pr("release", Some(fixed.clone()));
    let mut queued = ci_run(4, "ci", "q-sha", CiState::Failed);
    queued.head_branch = "gh-readonly-queue/main/pr-7-abcdef".into();
    *env.gh.queue.lock().unwrap() = vec![queued];
    env.gh.set_jobs(4, &["test (linux)"]);
    time_passes(&env, &id).await;
    assert_eq!(inbox_titles(&env).await.len(), 2, "{:?}", inbox_titles(&env).await);
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let texts: Vec<&str> =
        inbox["items"].as_array().unwrap().iter().map(|i| i["body"]["text"].as_str().unwrap()).collect();
    assert!(texts.iter().any(|t| t.contains("- ci: test (linux)") && t.contains("/actions/runs/4")), "{texts:?}");

    fixed.queue_removal = None;
    fixed.auto_merge = true;
    fixed.state = PrState::Merged;
    env.gh.set_pr("release", Some(fixed));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_release");
    env.gh.publish(REPO, release("v0.2.0"));
    time_passes(&env, &id).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(r["output"]["tag"], "v0.2.0");
}

#[tokio::test]
async fn a_failed_release_pr_closed_releases_nothing() {
    let env = setup().await;
    let id = release_pr_open(&env).await;
    env.gh.set_runs("release", vec![ci_run(1, "ci", "abc", CiState::Failed)]);
    time_passes(&env, &id).await;
    assert_eq!(inbox_titles(&env).await, ["Release PR #7 failed its checks".to_string()]);
    env.gh.set_pr("release", Some(pr(7, PrState::Closed)));
    time_passes(&env, &id).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(r["steps"].as_array().unwrap().last().unwrap()["nodeId"], "not_released", "{r}");
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
