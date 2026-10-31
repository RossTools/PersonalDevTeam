//! PDT-194, PDT-214, PDT-294 and PDT-301 end to end: "Cut a release", made from its template for a project, runs the
//! project's release workflow for the kind of release Run now asks for, says the release PR is open, waits for it to
//! merge (a failed check goes to the Inbox) and tells the user when the release is published; Report a missing scheduled release and PDT missing nightly (version 1 of its node). GitHub
//! is `FakeForge`; the waits are cut short.

mod common;
mod retired;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState, QueueRemoval, QueueRemovalReason};
use pdt_forge::testing::FakeForge;
use pdt_forge::{ChangeRequest, Release};
use pdt_store::Store;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

const REPO: &str = "example-org/thing";
/// The project's release workflow and the branch it opens its release PR from: nothing PDT's own.
const WORKFLOW: &str = "publish.yml";
const BRANCH: &str = "cut/release";

/// Run now's body for a stable release with these inputs (`bump`, `version`).
fn stable(inputs: Value) -> Value {
    let mut input = json!({"type": "stable"});
    input.as_object_mut().unwrap().extend(inputs.as_object().cloned().unwrap_or_default());
    json!({"input": input})
}

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    gh: Arc<FakeForge>,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    setup_on(REPO).await
}

/// A project whose repository is `repo` on GitHub.
async fn setup_on(gh_repo: &str) -> Env {
    setup_with(gh_repo, &[]).await
}

/// [`setup_on`] on a host that has these retired built-ins from before PDT-295 (the maintainer's, now their own).
async fn setup_with(gh_repo: &str, retired: &[&str]) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    git(&repo, &["remote", "add", "origin", &format!("https://github.com/{gh_repo}.git")]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    for id in retired {
        retired::install(&store, id, true).await;
    }
    let mut state = AppState::new(store, tmp.path().join("data"));
    let gh = Arc::new(FakeForge::default());
    state.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::testing::fake_github(&gh))]);
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

/// Cut a release made for the project from its template: [`WORKFLOW`], its release PR on [`BRANCH`].
async fn cut_release(env: &Env) -> Value {
    cut_release_with(env, json!({"workflow": WORKFLOW, "releaseBranch": BRANCH})).await
}

/// Cut a release made for the project with these params.
async fn cut_release_with(env: &Env, params: Value) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:cut-release/instantiate",
        Some(json!({"params": params, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w
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
        provider: "github_actions".into(),
        id: id.to_string(),
        attempt: 1,
        name: workflow.into(),
        state,
        url: format!("https://github.com/{REPO}/actions/runs/{id}"),
        head_sha: sha.into(),
        branch: BRANCH.into(),
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
        Some(stable(json!({"bump": "minor"}))),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    env.gh.set_pr(BRANCH, Some(pr(7, PrState::Open)));
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
        Some(stable(json!({"bump": "minor"}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;

    // The release workflow runs on main with the bump (no version given: none sent).
    let started = env.gh.ci.triggered.lock().unwrap().clone();
    let inputs = BTreeMap::from([("bump".to_string(), "minor".to_string())]);
    assert_eq!(started, [(REPO.to_string(), WORKFLOW.to_string(), "main".to_string(), inputs)]);
    assert_eq!(waiting_at(&env, &id).await, "wait_pr");

    // The last release's PR, merged, is not the new one: it keeps looking until one is open.
    env.gh.set_pr(BRANCH, Some(pr(6, PrState::Merged)));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_pr");
    env.gh.set_pr(BRANCH, Some(pr(7, PrState::Open)));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    // Told, not asked: the PR merges itself, so nothing waits in the Inbox.
    assert!(notified(&env).await.contains(&"Release PR #7 is open".to_string()));
    assert!(inbox_titles(&env).await.is_empty(), "{:?}", inbox_titles(&env).await);

    // Still open, its checks running; then merged, but the tag's release isn't out yet; then it is.
    env.gh.set_runs(BRANCH, vec![ci_run(1, "ci", "abc", CiState::Running)]);
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    env.gh.set_pr(BRANCH, Some(pr(7, PrState::Merged)));
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
    env.gh.set_runs(BRANCH, vec![ci_run(1, "ci", "abc", CiState::Failed), ci_run(2, "lint", "abc", CiState::Passed)]);
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
    env.gh.set_pr(BRANCH, Some(fixed.clone()));
    env.gh.set_runs(BRANCH, vec![ci_run(1, "ci", "abc", CiState::Failed), ci_run(3, "ci", "def", CiState::Passed)]);
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "wait_merge");
    // Then the merge queue's run fails and removes it (auto-merge off): that is news too.
    fixed.queue_removal =
        Some(QueueRemoval { id: "q1".into(), reason: QueueRemovalReason::FailedChecks, by_queue: true });
    env.gh.set_pr(BRANCH, Some(fixed.clone()));
    let mut queued = ci_run(4, "ci", "q-sha", CiState::Failed);
    queued.branch = "fake-queue/main/7/abcdef".into();
    *env.gh.ci.queue.lock().unwrap() = vec![queued];
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
    env.gh.set_pr(BRANCH, Some(fixed));
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
    env.gh.set_runs(BRANCH, vec![ci_run(1, "ci", "abc", CiState::Failed)]);
    time_passes(&env, &id).await;
    assert_eq!(inbox_titles(&env).await, ["Release PR #7 failed its checks".to_string()]);
    env.gh.set_pr(BRANCH, Some(pr(7, PrState::Closed)));
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
        Some(stable(json!({"version": "1.0.0"}))),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    let started = env.gh.ci.triggered.lock().unwrap().clone();
    assert_eq!(started[0].3, BTreeMap::from([("version".to_string(), "1.0.0".to_string())]));

    env.gh.set_pr(BRANCH, Some(pr(9, PrState::Open)));
    time_passes(&env, &id).await;
    env.gh.set_pr(BRANCH, Some(pr(9, PrState::Closed)));
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
        Some(json!({"workflowId": wf["id"], "event": {"type": "manual", "payload": stable(json!({"bump": "patch"}))["input"]}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let errors: Vec<_> = r["issues"].as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
    assert!(errors.is_empty(), "{r}");
    let run = &r["run"];
    assert_eq!(run["status"], "succeeded", "{r}");
    let nodes: Vec<&str> = run["steps"].as_array().unwrap().iter().map(|s| s["nodeId"].as_str().unwrap()).collect();
    assert!(nodes.contains(&"dispatch") && nodes.contains(&"released"), "{nodes:?}");
    assert!(env.gh.ci.triggered.lock().unwrap().is_empty());
    let calls = env.gh.calls.lock().unwrap().clone();
    assert!(calls.is_empty(), "a dry run calls nothing: {calls:?}");
}

#[tokio::test]
async fn an_open_release_pr_stops_a_new_release_before_the_action_runs() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    // An earlier release PR is still open: the Action would refuse, and that PR is not the bump asked for.
    env.gh.set_pr(BRANCH, Some(pr(5, PrState::Open)));
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()),
        Some(stable(json!({"bump": "minor"}))),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;

    assert!(env.gh.ci.triggered.lock().unwrap().is_empty(), "the release workflow is not started");
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let last = r["steps"].as_array().unwrap().last().unwrap();
    assert_eq!(last["nodeId"], "not_started", "{r}");
    assert!(inbox_titles(&env).await.contains(&"Release PR #5 is still open".to_string()));
}

/// PDT-301: Run now takes only the inputs Cut a release declares, and names them when it refuses; without a release
/// workflow there is no Cut a release to run.
#[tokio::test]
async fn run_now_refuses_input_the_form_does_not_ask_for() {
    let env = setup().await;
    let wf = cut_release(&env).await;
    let url = format!("/api/workflows/{}/run", wf["id"].as_str().unwrap());
    for (input, says) in [
        (json!({"workflow": WORKFLOW}), "there is no input workflow"),
        (json!({"type": "stable", "bump": "huge"}), "bump must be patch, minor or major"),
        (json!({"type": "final"}), "type must be prerelease or stable"),
    ] {
        let (s, r) = call(&env.app, "POST", &url, Some(json!({"input": input}))).await;
        assert_eq!(s, StatusCode::CONFLICT, "{r}");
        let msg = r["message"].as_str().unwrap();
        assert!(msg.contains(says) && msg.contains("type (prerelease or stable, required)"), "{msg}");
    }
    assert!(env.gh.ci.triggered.lock().unwrap().is_empty());
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:cut-release/instantiate",
        Some(json!({"params": {}, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");
    assert!(r["message"].as_str().unwrap().contains("Release workflow (workflow) is required"), "{r}");
}

/// PDT-301: the other kind of release (a beta here) is what Run now starts by default: the workflow on its branch with
/// that kind's inputs, no release PR stage, and a wait for a newer release matching its tag pattern.
#[tokio::test]
async fn the_other_kind_of_release_runs_by_default_and_waits_for_its_tag() {
    let env = setup().await;
    let wf = cut_release_with(
        &env,
        json!({"workflow": "beta.yml", "ref": "develop", "releaseBranch": BRANCH, "preName": "Beta",
               "preInputs": "{\"channel\": \"beta\", \"dry\": false}", "preTagPattern": "v*-beta.*"}),
    )
    .await;
    env.gh.publish(REPO, release("v0.2.0-beta.1"));
    // A release PR left open doesn't hold this up: it makes none.
    env.gh.set_pr(BRANCH, Some(pr(5, PrState::Open)));
    let (_, r) = call(&env.app, "POST", &format!("/api/workflows/{}/run", wf["id"].as_str().unwrap()), None).await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &id).await["trigger"], json!({"type": "prerelease"}), "the default filled in");

    let started = env.gh.ci.triggered.lock().unwrap().clone();
    let inputs =
        BTreeMap::from([("channel".to_string(), "beta".to_string()), ("dry".to_string(), "false".to_string())]);
    assert_eq!(started, [(REPO.to_string(), "beta.yml".to_string(), "develop".to_string(), inputs)]);
    assert_eq!(waiting_at(&env, &id).await, "pre_wait");

    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "pre_wait", "nothing newer yet");
    // A release that doesn't match the pattern is not the one.
    env.gh.publish(REPO, release("v0.2.0"));
    time_passes(&env, &id).await;
    assert_eq!(waiting_at(&env, &id).await, "pre_wait", "not a beta");
    env.gh.publish(REPO, release("v0.2.1-beta.1"));
    time_passes(&env, &id).await;

    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(r["output"]["tag"], "v0.2.1-beta.1");
    let titles = inbox_titles(&env).await;
    assert!(titles.contains(&"Release v0.2.1-beta.1 is published".to_string()), "{titles:?}");
    let calls = env.gh.calls.lock().unwrap().clone();
    assert!(!calls.iter().any(|c| c.starts_with("merge ")), "{calls:?}");
}

/// PDT-294: without a release PR branch, a stable release has no PR stage: the workflow publishes it.
#[tokio::test]
async fn a_stable_cut_without_a_release_branch_dry_runs_without_calling_github() {
    let env = setup().await;
    let wf = cut_release_with(&env, json!({"workflow": WORKFLOW})).await;
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/workflows/dry-run",
        Some(json!({"workflowId": wf["id"], "event": {"type": "manual", "payload": stable(json!({}))["input"]}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let errors: Vec<_> = r["issues"].as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
    assert!(errors.is_empty(), "{r}");
    let run = &r["run"];
    assert_eq!(run["status"], "succeeded", "{r}");
    let nodes: Vec<&str> = run["steps"].as_array().unwrap().iter().map(|s| s["nodeId"].as_str().unwrap()).collect();
    assert!(nodes.contains(&"direct_dispatch") && nodes.contains(&"direct_released"), "{nodes:?}");
    assert!(!nodes.contains(&"dispatch"), "not the release PR path: {nodes:?}");
    assert!(env.gh.calls.lock().unwrap().is_empty());
}

/// PDT-301: the Cut a release built-in a host has from before is gone when it was never turned on or run, and kept as
/// the user's own workflow (as it was) when it was on.
#[tokio::test]
async fn the_cut_release_builtin_is_retired() {
    let unused = setup().await;
    retired::install(&unused.state.store, retired::CUT_RELEASE, false).await;
    pdt_api::run_workflows(&unused.state).await;
    let (s, _) = call(&unused.app, "GET", &format!("/api/workflows/{}", retired::id(retired::CUT_RELEASE)), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "never used: removed");

    let on = setup().await;
    retired::install(&on.state.store, retired::CUT_RELEASE, true).await;
    let wf = kept(&on, retired::CUT_RELEASE).await;
    assert_eq!(wf["name"], "Cut a release");
    let (_, all) = call(&on.app, "GET", "/api/workflows", None).await;
    assert!(!all.as_array().unwrap().iter().any(|w| w["builtin"] == "cut-release"), "nothing installed");
}

/// A run of the release workflow on main (the scheduled nightly).
fn nightly_run(id: u64, state: CiState, at: &str) -> CiRun {
    CiRun { branch: "main".into(), created_at: at.into(), ..ci_run(id, "release", "def", state) }
}

/// The host's own copy of a retired built-in, once pdtd has kept it as the user's (PDT-295).
async fn kept(env: &Env, builtin: &str) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, wf) = call(&env.app, "GET", &format!("/api/workflows/{}", retired::id(builtin)), None).await;
    assert!(wf["builtin"].is_null(), "{wf}");
    assert_eq!(wf["enabled"], true, "{wf}");
    wf
}

/// PDT-283: PDT nightly release (the maintainer's own workflow since PDT-295) reads the scheduled nightly's run each
/// day and puts a failed one in the Inbox, once.
#[tokio::test]
async fn a_failed_nightly_goes_to_the_inbox_once() {
    let env = setup_with(pdt_core::brand::RELEASE_REPO, &[retired::NIGHTLY]).await;
    let wf = kept(&env, retired::NIGHTLY).await;
    let fire = |slot: i64| {
        let payload = json!({"workflowId": wf["id"], "nodeId": "daily", "slot": slot,
                             "slotIso": "2026-10-30T11:00:00Z", "date": "2026-10-30"});
        pdt_core::workflow::api::NewLogEvent::new(
            pdt_core::workflow::events::SCHEDULE_FIRED,
            format!("schedule:test:{slot}"),
            "scheduler",
            payload,
        )
    };

    // A passed run (a night with nothing new on main publishes nothing, and passes): nothing to say.
    env.gh.ci.set_runs("main", vec![nightly_run(40, CiState::Passed, "2026-10-29T08:00:00Z")]);
    env.state.store.emit_event(&fire(1), 1).await.unwrap().expect("logged");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    assert!(!inbox_titles(&env).await.iter().any(|t| t.contains("nightly")));

    // The next night's failed: the Inbox says so, with its failed jobs; an older run and other workflows don't count.
    env.gh.ci.set_runs(
        "main",
        vec![
            nightly_run(40, CiState::Passed, "2026-10-29T08:00:00Z"),
            nightly_run(41, CiState::Failed, "2026-10-30T08:00:00Z"),
            CiRun {
                branch: "main".into(),
                created_at: "2026-10-30T09:00:00Z".into(),
                ..ci_run(42, "pages", "def", CiState::Passed)
            },
        ],
    );
    env.gh.ci.set_jobs("41", &["bundle (macos-latest, app)"]);
    env.state.store.emit_event(&fire(2), 2).await.unwrap().expect("logged");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let titles = inbox_titles(&env).await;
    assert_eq!(titles.iter().filter(|t| *t == "The nightly release failed").count(), 1, "{titles:?}");
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    let item = v["items"].as_array().unwrap().iter().find(|i| i["title"] == "The nightly release failed").unwrap();
    let text = item.to_string();
    assert!(text.contains("release: bundle (macos-latest, app)") && text.contains("/actions/runs/41"), "{item}");

    // Looked at again (a catch-up), the same failed run is not news again.
    env.state.store.emit_event(&fire(3), 3).await.unwrap().expect("logged");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let titles = inbox_titles(&env).await;
    assert_eq!(titles.iter().filter(|t| *t == "The nightly release failed").count(), 1, "{titles:?}");
}

/// PDT missing nightly (the maintainer's own workflow since PDT-295), and a schedule firing for `date`'s 10:00 UTC
/// look.
async fn missing_nightly(env: &Env) -> (String, impl Fn(i64, &str) -> pdt_core::workflow::api::NewLogEvent) {
    let wf = kept(env, retired::NIGHTLY_MISSING).await;
    let id = wf["id"].as_str().unwrap().to_string();
    let wf_id = id.clone();
    let fire = move |slot: i64, date: &str| {
        let payload = json!({"workflowId": wf_id, "nodeId": "daily", "slot": slot,
                             "slotIso": format!("{date}T10:00:00Z"), "date": date});
        pdt_core::workflow::api::NewLogEvent::new(
            pdt_core::workflow::events::SCHEDULE_FIRED,
            format!("schedule:test:{slot}"),
            "scheduler",
            payload,
        )
    };
    (id, fire)
}

async fn fired(env: &Env, e: &pdt_core::workflow::api::NewLogEvent, slot: i64) {
    env.state.store.emit_event(e, slot).await.unwrap().expect("logged");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

const MISSING: &str = "Tonight's nightly didn't run";

async fn missing_items(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    v["items"].as_array().unwrap().iter().filter(|i| i["title"] == MISSING).cloned().collect()
}

/// PDT-288: a day main changed and no nightly is out by 10:00 UTC makes exactly one Inbox item, which closes once a
/// nightly for the day is published; a day main didn't change makes none, nor does a look before the due time.
#[tokio::test]
async fn a_missing_nightly_goes_to_the_inbox_once_and_clears_when_published() {
    let repo = pdt_core::brand::RELEASE_REPO;
    let env = setup_with(repo, &[retired::NIGHTLY_MISSING]).await;
    let (wf, fire) = missing_nightly(&env).await;
    env.gh.publish(repo, release("v0.2.0"));
    env.gh.publish(repo, release("v0.2.1-nightly.20261029"));

    // Nothing merged since the last nightly (a stable release since is older): nothing to say.
    env.gh.set_ahead(repo, "v0.2.1-nightly.20261029", "main", 0);
    env.gh.set_ahead(repo, "v0.2.0", "main", 5);
    fired(&env, &fire(1, "2026-10-30"), 1).await;
    assert!(missing_items(&env).await.is_empty());

    // Merged since, and no nightly for the day: one item, with the runs and Start it.
    env.gh.set_ahead(repo, "v0.2.1-nightly.20261029", "main", 2);
    fired(&env, &fire(2, "2026-10-30"), 2).await;
    let items = missing_items(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let text = items[0]["body"]["text"].as_str().unwrap();
    assert!(text.contains("2 commits since v0.2.1-nightly.20261029"), "{text}");
    assert!(text.contains(&format!("https://github.com/{repo}/actions/workflows/release.yml")), "{text}");
    let choices: Vec<&str> =
        items[0]["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["start", "ignore"]);

    // A catch-up the same day, and the watch's own next look, add none.
    fired(&env, &fire(3, "2026-10-30"), 3).await;
    let (_, list) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    let watching: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["status"] == "waiting")
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(watching.len(), 1, "{list}");
    time_passes(&env, &watching[0]).await;
    assert_eq!(missing_items(&env).await.len(), 1);
    assert_eq!(waiting_at(&env, &watching[0]).await, "watch");

    // The nightly is out (run by hand on GitHub): at the next look the item closes and the run ends quietly.
    env.gh.publish(repo, release("v0.2.1-nightly.20261030"));
    time_passes(&env, &watching[0]).await;
    assert!(missing_items(&env).await.is_empty());
    let r = run(&env, &watching[0]).await;
    assert_eq!(r["status"], "quiet", "{r}");

    // A look before the day's due time says nothing, whatever main did.
    env.gh.set_ahead(repo, "v0.2.1-nightly.20261030", "main", 3);
    fired(&env, &fire(4, "2099-01-01"), 4).await;
    assert!(missing_items(&env).await.is_empty());
}

/// PDT-288: Start it runs the release workflow on main with channel nightly.
#[tokio::test]
async fn start_it_runs_the_nightly() {
    let repo = pdt_core::brand::RELEASE_REPO;
    let env = setup_with(repo, &[retired::NIGHTLY_MISSING]).await;
    let (_, fire) = missing_nightly(&env).await;
    env.gh.publish(repo, release("v0.2.1-nightly.20261029"));
    env.gh.set_ahead(repo, "v0.2.1-nightly.20261029", "main", 1);
    fired(&env, &fire(1, "2026-10-30"), 1).await;
    let items = missing_items(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", items[0]["id"].as_str().unwrap()),
        Some(json!({"choice": "start"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let inputs = BTreeMap::from([("channel".to_string(), "nightly".to_string())]);
    let started = env.gh.ci.triggered.lock().unwrap().clone();
    assert_eq!(started, [(repo.to_string(), "release.yml".to_string(), "main".to_string(), inputs)]);
    assert!(missing_items(&env).await.is_empty());
    assert!(notified(&env).await.contains(&"Tonight's nightly is starting".to_string()));
}

/// PDT-294: Report a missing scheduled release, made from its template for any tag scheme (here `snapshot-{date}`,
/// reported whether or not the branch changed): a day with no release goes to the Inbox, and Start it runs the
/// workflow on the branch with the inputs given; a day whose release is out says nothing.
#[tokio::test]
async fn the_missing_release_template_reports_and_starts_any_scheduled_release() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let params = json!({"repo": REPO, "workflow": "snapshot.yml", "branch": "develop",
                        "tagPattern": "snapshot-{date}", "hour": 6, "onlyIfChanged": false,
                        "inputs": "{\"kind\": \"snapshot\"}", "title": "No snapshot today"});
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:missing-scheduled-release/instantiate",
        Some(json!({"params": params})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let fire = |slot: i64, date: &str| {
        let payload = json!({"workflowId": wf, "nodeId": "daily", "slot": slot,
                             "slotIso": format!("{date}T06:00:00Z"), "date": date});
        pdt_core::workflow::api::NewLogEvent::new(
            pdt_core::workflow::events::SCHEDULE_FIRED,
            format!("schedule:test:{slot}"),
            "scheduler",
            payload,
        )
    };
    let items = || async {
        let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
        v["items"].as_array().unwrap().iter().filter(|i| i["title"] == "No snapshot today").cloned().collect::<Vec<_>>()
    };

    // The day before's release is out, the day's isn't: missing.
    env.gh.publish(REPO, release("snapshot-2026-10-29"));
    fired(&env, &fire(1, "2026-10-30"), 1).await;
    let found = items().await;
    assert_eq!(found.len(), 1, "{found:?}");
    let text = found[0]["body"]["text"].as_str().unwrap();
    assert!(
        text.contains("No release matching `snapshot-{date}` for 2026-10-30 was published by 06:00 UTC."),
        "{text}"
    );
    assert!(text.contains(&format!("https://github.com/{REPO}/actions/workflows/snapshot.yml")), "{text}");
    assert!(!env.gh.calls.lock().unwrap().iter().any(|c| c.contains("ahead")), "the branch isn't compared");

    // Start it: the workflow runs on the branch with the template's inputs.
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/inbox/{}/answer", found[0]["id"].as_str().unwrap()),
        Some(json!({"choice": "start"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let inputs = BTreeMap::from([("kind".to_string(), "snapshot".to_string())]);
    let started = env.gh.ci.triggered.lock().unwrap().clone();
    assert_eq!(started, [(REPO.to_string(), "snapshot.yml".to_string(), "develop".to_string(), inputs)]);
    assert!(items().await.is_empty());
    assert!(notified(&env).await.contains(&"The release workflow is starting".to_string()));

    // The next day's (in the other date form) is out: nothing to say.
    env.gh.publish(REPO, release("snapshot-20261031"));
    fired(&env, &fire(2, "2026-10-31"), 2).await;
    assert!(items().await.is_empty());
}

/// PDT-301: a host-wide Cut a release has no project of its own: Run now asks for one (`needsProject`), and a start
/// without one is refused at once, saying so, instead of a run waiting on retries.
#[tokio::test]
async fn a_host_wide_cut_needs_a_project_to_run_in() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let (s, wf) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:cut-release/instantiate",
        Some(json!({"params": {"workflow": WORKFLOW, "releaseBranch": BRANCH}, "scope": "host"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{wf}");
    assert_eq!(wf["needsProject"], true, "{wf}");
    assert_eq!(cut_release(&env).await["needsProject"], false, "a project's own has one");

    let url = format!("/api/workflows/{}/run", wf["id"].as_str().unwrap());
    let (s, e) = call(&env.app, "POST", &url, Some(stable(json!({"bump": "minor"})))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert!(e["message"].as_str().unwrap().contains("needs a project to run in"), "{e}");
    let (_, runs) =
        call(&env.app, "GET", &format!("/api/workflow-runs?workflow={}", wf["id"].as_str().unwrap()), None).await;
    assert!(runs.as_array().unwrap().is_empty(), "no run started: {runs}");

    // With the project picked, it runs there.
    let mut body = stable(json!({"bump": "minor"}));
    body["input"]["projectId"] = json!(env.project);
    let (s, r) = call(&env.app, "POST", &url, Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let started = env.gh.ci.triggered.lock().unwrap().clone();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0].0, REPO);
}

/// PDT-301: a step that can't run for its setup (here a repository that isn't owner/name) fails the run at once, its
/// retries left unused: waiting won't fix it.
#[tokio::test]
async fn a_setup_error_fails_the_run_at_once() {
    let env = setup().await;
    let doc = json!({"name": "Read a release", "nodes": [
        {"id": "start", "kind": "manual", "version": 1},
        {"id": "read", "kind": "github.release", "version": 1, "retry": {"max": 5, "backoffMs": 60_000},
         "config": {"repo": "{{trigger.repo}}"}},
        {"id": "done", "kind": "end", "version": 1}],
        "edges": [{"from": "start", "to": "read"}, {"from": "read", "port": "found", "to": "done"},
                  {"from": "read", "port": "none", "to": "done"}]});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let (_, r) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{}/run", w["id"].as_str().unwrap()),
        Some(json!({"input": {"repo": "not a repo"}})),
    )
    .await;
    let id = r["runId"].as_str().unwrap().to_string();
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "failed", "{r}");
    assert_eq!(r["error"], "read: \"not a repo\" is not owner/name", "{r}");
    assert_eq!(r["steps"][1]["attempts"], 1, "not retried: {r}");
}
