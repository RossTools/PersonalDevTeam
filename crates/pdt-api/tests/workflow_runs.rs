//! PDT-292 end to end: the template "Report failed workflow runs" watches one GitHub Actions workflow, puts each failed
//! run (however it was started) in the Inbox once with its failed jobs and Re-run failed jobs, and clears the item once
//! a later attempt passes. GitHub is `FakeForge`'s CI; the looks are forced instead of waited for.

use crate::common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::CiState;
use pdt_forge::testing::FakeForge;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;

const REPO: &str = "example-org/thing";
const PATH: &str = ".github/workflows/release.yml";

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    gh: Arc<FakeForge>,
    db: std::path::PathBuf,
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
    git(&repo, &["remote", "add", "origin", &format!("https://github.com/{REPO}.git")]);
    let db = tmp.path().join("pdt.db");
    let gh = Arc::new(FakeForge::default());
    let state = state_on(&db, tmp.path(), &gh).await;
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
    Env { app, state, project, gh, db, tmp }
}

/// A pdtd on the data in `db`, with `gh` as GitHub.
async fn state_on(db: &std::path::Path, dir: &std::path::Path, gh: &Arc<FakeForge>) -> AppState {
    let store = Store::open(db).await.unwrap();
    let mut state = common::app_state(store, dir);
    state.forges = pdt_forge::ForgeRegistry::new(vec![Arc::new(pdt_forge::testing::fake_github(gh))]);
    state
}

/// The template instantiated for the project with `params` (its workflow `release.yml` unless given) and enabled.
async fn watch(env: &Env, params: Value) -> String {
    let mut params = params;
    if params.get("workflow").is_none() {
        params["workflow"] = json!("release.yml");
    }
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:failed-workflow-runs/instantiate",
        Some(json!({"params": params, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!(w["enabled"], false, "a template's workflow starts off");
    let id = w["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    id
}

/// A run of the release workflow, its attempt `attempt` ending at `at`.
fn run(
    env: &Env,
    id: &str,
    attempt: u32,
    event: &str,
    state: CiState,
    at: &str,
) -> pdt_core::workflow_runs::WorkflowRun {
    env.gh.ci.workflow_run(id, attempt, PATH, "main", event, state, at)
}

/// The watch's interval has passed: pdtd looks at the runs, and the engine runs what that started.
async fn look(state: &AppState, wf: &str) {
    let key = "_pdt.workflow_runs.failed";
    if let Some(mut c) = state.store.workflow_state(wf).await.unwrap().get(key).cloned() {
        c["lookedAt"] = json!(0);
        state.store.set_workflow_state(wf, key, &c, false, 1).await.unwrap();
    }
    pdt_api::refresh_workflow_runs(state).await;
    for _ in 0..3 {
        pdt_api::run_workflows(state).await;
    }
}

/// The open items about failed runs.
async fn items(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    v["items"].as_array().unwrap().iter().filter(|i| i["body"]["runId"].is_string()).cloned().collect()
}

async fn runs_of(env: &Env, wf: &str) -> Vec<Value> {
    let (_, list) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    list.as_array().unwrap().clone()
}

/// Cuts the watch's wait short (its next look at the run), and lets the engine go on.
async fn time_passes(env: &Env, run_id: &str) {
    let store = &env.state.store;
    let mut step = store
        .list_steps(run_id)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.status == pdt_core::workflow::api::StepStatus::Waiting)
        .expect("a waiting step");
    step.wake_at = Some(1);
    store.put_step(&step).await.unwrap();
    store.wake_workflow_run(run_id, 1).await.unwrap();
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

async fn answer(env: &Env, item: &Value, choice: &str) {
    let path = format!("/api/inbox/{}/answer", item["id"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &path, Some(json!({"choice": choice}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

/// Failed scheduled and manual runs each make one item, with what failed and Re-run failed jobs; runs that passed or
/// were cancelled make none, and the first look reports nothing that already happened.
#[tokio::test]
async fn failed_scheduled_and_manual_runs_each_raise_one_item() {
    let env = setup().await;
    // Before the watch: an old failure, not news.
    env.gh.ci.put_workflow_run(run(&env, "10", 1, "schedule", CiState::Failed, "2026-10-29T08:30:00Z"));
    let wf = watch(&env, json!({})).await;
    look(&env.state, &wf).await;
    assert!(items(&env).await.is_empty(), "the first look only records where the runs are");

    let mut scheduled = run(&env, "11", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z");
    scheduled.run.branch = "v1.2.0".into();
    scheduled.title = "Release v1.2.0".into();
    env.gh.ci.put_workflow_run(scheduled);
    env.gh.ci.set_jobs("11", &["bundle (macos-latest)", "publish"]);
    env.gh.ci.put_workflow_run(run(&env, "12", 1, "workflow_dispatch", CiState::Failed, "2026-10-30T09:00:00Z"));
    env.gh.ci.put_workflow_run(run(&env, "13", 1, "push", CiState::Passed, "2026-10-30T09:10:00Z"));
    env.gh.ci.put_workflow_run(run(&env, "14", 1, "schedule", CiState::Cancelled, "2026-10-30T09:20:00Z"));
    // Another workflow's failure is not this watch's.
    env.gh.ci.put_workflow_run(env.gh.ci.workflow_run(
        "15",
        1,
        ".github/workflows/ci.yml",
        "main",
        "push",
        CiState::Failed,
        "2026-10-30T09:30:00Z",
    ));
    look(&env.state, &wf).await;
    let mut found = items(&env).await;
    found.sort_by_key(|i| i["body"]["runId"].as_str().unwrap().to_string());
    let ids: Vec<&str> = found.iter().map(|i| i["body"]["runId"].as_str().unwrap()).collect();
    assert_eq!(ids, ["11", "12"], "{found:?}");
    assert_eq!(found[0]["title"], "release failed on v1.2.0");
    let text = found[0]["body"]["text"].as_str().unwrap();
    for part in [
        "[Release v1.2.0](https://ci.example/github_actions/runs/11)",
        "Started by its schedule",
        "Version: `v1.2.0`",
        "- bundle (macos-latest)\n- publish",
    ] {
        assert!(text.contains(part), "{part:?} in {text}");
    }
    assert!(found[1]["body"]["text"].as_str().unwrap().contains("Started by hand"));
    let choices: Vec<&str> =
        found[0]["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["rerun", "dismiss"]);

    // The next looks, and the watches' own, add none.
    look(&env.state, &wf).await;
    for r in runs_of(&env, &wf).await.iter().filter(|r| r["status"] == "waiting") {
        time_passes(&env, r["id"].as_str().unwrap()).await;
    }
    assert_eq!(items(&env).await.len(), 2);
    assert_eq!(runs_of(&env, &wf).await.len(), 2, "one run per failed run");

    // Dismissed, it is gone.
    answer(&env, &found[1], "dismiss").await;
    assert_eq!(items(&env).await.len(), 1);
}

/// Re-run failed jobs re-runs them through the forge; a re-run that passes leaves nothing, one that fails again makes
/// one item for the new attempt; and a re-run started on GitHub that passes clears the item by itself.
#[tokio::test]
async fn a_successful_re_run_clears_the_item() {
    let env = setup().await;
    let wf = watch(&env, json!({"events": ["schedule", "workflow_dispatch"]})).await;
    look(&env.state, &wf).await;
    env.gh.ci.put_workflow_run(run(&env, "20", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    // Not an event the watch takes.
    env.gh.ci.put_workflow_run(run(&env, "21", 1, "push", CiState::Failed, "2026-10-30T08:41:00Z"));
    look(&env.state, &wf).await;
    let found = items(&env).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["body"]["runId"], "20");

    // Re-run from the Inbox: the forge re-runs the failed jobs; the watch waits for the new attempt.
    answer(&env, &found[0], "rerun").await;
    assert_eq!(*env.gh.ci.retried.lock().unwrap(), [("20".to_string(), true)]);
    assert!(items(&env).await.is_empty());
    let watching: Vec<String> = runs_of(&env, &wf)
        .await
        .iter()
        .filter(|r| r["status"] == "waiting")
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(watching.len(), 1);
    let id = &watching[0];
    time_passes(&env, id).await;
    assert!(items(&env).await.is_empty(), "the re-run attempt has not ended yet");

    // It fails again: one item, for attempt 2 (the watch's or the trigger's, never both).
    env.gh.ci.put_workflow_run(run(&env, "20", 2, "schedule", CiState::Failed, "2026-10-30T09:40:00Z"));
    look(&env.state, &wf).await;
    time_passes(&env, id).await;
    let found = items(&env).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["body"]["attempt"], 2);
    assert!(found[0]["body"]["text"].as_str().unwrap().contains("(attempt 2)"));

    // Re-run on GitHub, and it passes: the item goes away by itself.
    env.gh.ci.put_workflow_run(run(&env, "20", 3, "schedule", CiState::Passed, "2026-10-30T10:40:00Z"));
    look(&env.state, &wf).await;
    for r in runs_of(&env, &wf).await.iter().filter(|r| r["status"] == "waiting") {
        time_passes(&env, r["id"].as_str().unwrap()).await;
    }
    assert!(items(&env).await.is_empty());
    assert!(runs_of(&env, &wf).await.iter().all(|r| r["status"] != "waiting" && r["status"] != "failed"));
}

/// A re-run the forge refuses (a login without the Actions write permission) says so in the Inbox, with the run.
#[tokio::test]
async fn a_refused_re_run_says_so() {
    let env = setup().await;
    let wf = watch(&env, json!({})).await;
    look(&env.state, &wf).await;
    env.gh.ci.put_workflow_run(run(&env, "30", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    look(&env.state, &wf).await;
    let found = items(&env).await;
    assert_eq!(found.len(), 1);
    env.gh.ci.capabilities.lock().unwrap().retry = false;
    answer(&env, &found[0], "rerun").await;
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    let notice = v["items"].as_array().unwrap().iter().find(|i| i["title"] == "Couldn't re-run release").cloned();
    let notice = notice.unwrap_or_else(|| panic!("{v}"));
    assert!(notice["body"]["text"].as_str().unwrap().contains("https://ci.example/github_actions/runs/30"));
}

/// Where the watch read to is kept: a pdtd started again on the same data reports neither a run it reported nor one
/// from before, and still reports the next.
#[tokio::test]
async fn the_cursor_survives_a_restart() {
    let env = setup().await;
    let wf = watch(&env, json!({})).await;
    env.gh.ci.put_workflow_run(run(&env, "40", 1, "schedule", CiState::Failed, "2026-10-29T08:40:00Z"));
    look(&env.state, &wf).await;
    env.gh.ci.put_workflow_run(run(&env, "41", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    look(&env.state, &wf).await;
    assert_eq!(items(&env).await.len(), 1);

    let again = state_on(&env.db, env.tmp.path(), &env.gh).await;
    look(&again, &wf).await;
    assert_eq!(runs_of(&env, &wf).await.len(), 1, "nothing repeated");
    env.gh.ci.put_workflow_run(run(&env, "42", 1, "workflow_dispatch", CiState::Failed, "2026-10-30T09:40:00Z"));
    look(&again, &wf).await;
    let ids: Vec<String> = items(&env).await.iter().map(|i| i["body"]["runId"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.contains(&"42".to_string()));
}

/// The watch looks no more often than its interval.
#[tokio::test]
async fn the_watch_waits_for_its_interval() {
    let env = setup().await;
    let wf = watch(&env, json!({"every": "1h"})).await;
    look(&env.state, &wf).await;
    env.gh.ci.put_workflow_run(run(&env, "50", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    pdt_api::refresh_workflow_runs(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert!(items(&env).await.is_empty(), "not due yet");
    look(&env.state, &wf).await;
    assert_eq!(items(&env).await.len(), 1);
}

// ---- the nodes on their own (PDT-446) ----

/// A project workflow of one `node` after a manual trigger, run once: the run as the API gives it.
async fn run_node(env: &Env, node: Value) -> Value {
    let doc = json!({"name": "One node", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
                     "edges": [{"from": "go", "to": "it"}]});
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "projectId": env.project, "enabled": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await.1
}

fn node_step(run: &Value) -> &Value {
    run["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == "it").unwrap_or_else(|| panic!("{run}"))
}

fn watch_node(config: Value) -> Value {
    json!({"id": "it", "kind": "github.workflow.watch", "version": 1, "config": config})
}

/// The watch leaves quietly, with no Inbox item, for a run that isn't there or was cancelled; waits (no item) while
/// the run goes or while only the attempt already re-run has failed; and stops looking after `watchFor`.
#[tokio::test]
async fn the_watch_reports_only_a_failed_attempt_it_has_not_seen() {
    let env = setup().await;
    let r = run_node(&env, watch_node(json!({"runId": 900}))).await;
    assert_eq!((node_step(&r)["port"].as_str(), &node_step(&r)["output"]["quiet"]), (Some("none"), &json!(true)));

    env.gh.ci.put_workflow_run(run(&env, "901", 1, "schedule", CiState::Cancelled, "2026-10-30T08:40:00Z"));
    let r = run_node(&env, watch_node(json!({"runId": "901"}))).await;
    assert_eq!(node_step(&r)["port"], "cancelled", "{r}");

    env.gh.ci.put_workflow_run(run(&env, "902", 2, "schedule", CiState::Passed, "2026-10-30T08:40:00Z"));
    let r = run_node(&env, watch_node(json!({"runId": "902"}))).await;
    assert_eq!(
        (node_step(&r)["port"].as_str(), node_step(&r)["output"]["attempt"].as_u64()),
        (Some("passed"), Some(2))
    );

    // Attempt 1 is the one re-run (`after`): its failure is old news; the watch waits for attempt 2.
    env.gh.ci.put_workflow_run(run(&env, "903", 1, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    let r = run_node(&env, watch_node(json!({"runId": "903", "after": 1, "title": "Release broke"}))).await;
    assert_eq!(node_step(&r)["status"], "waiting", "{r}");
    assert!(items(&env).await.is_empty());
    let rid = r["id"].as_str().unwrap().to_string();
    env.gh.ci.put_workflow_run(run(&env, "903", 2, "schedule", CiState::Failed, "2026-10-30T09:40:00Z"));
    time_passes(&env, &rid).await;
    let found = items(&env).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0]["title"].as_str(), found[0]["body"]["attempt"].as_u64()), (Some("Release broke"), Some(2)));

    // A run still going: it waits, until `watchFor` has passed with nothing to report.
    env.gh.ci.put_workflow_run(run(&env, "904", 1, "schedule", CiState::Running, "2026-10-30T08:40:00Z"));
    let r = run_node(&env, watch_node(json!({"runId": "904", "watchFor": "1h"}))).await;
    assert_eq!(node_step(&r)["status"], "waiting", "{r}");
    let rid = r["id"].as_str().unwrap().to_string();
    let mut step = env.state.store.list_steps(&rid).await.unwrap().into_iter().find(|s| s.node_id == "it").unwrap();
    step.output["since"] = json!(1);
    env.state.store.put_step(&step).await.unwrap();
    time_passes(&env, &rid).await;
    let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs/{rid}"), None).await;
    assert_eq!(node_step(&r)["port"], "expired", "{r}");
    assert_eq!(items(&env).await.len(), 1, "only 903's");
}

/// The re-run node re-runs the failed jobs, or every job with `allJobs`, and says `none` for a run that isn't there.
#[tokio::test]
async fn the_rerun_node_re_runs_failed_or_all_jobs() {
    let env = setup().await;
    env.gh.ci.put_workflow_run(run(&env, "910", 3, "schedule", CiState::Failed, "2026-10-30T08:40:00Z"));
    let rerun = |config: Value| json!({"id": "it", "kind": "github.workflow.rerun", "version": 1, "config": config});
    let r = run_node(&env, rerun(json!({"runId": "910"}))).await;
    let step = node_step(&r);
    assert_eq!(step["port"], "out", "{r}");
    assert_eq!((step["output"]["runId"].as_str(), step["output"]["attempt"].as_u64()), (Some("910"), Some(3)));
    assert_eq!(step["output"]["url"], "https://ci.example/github_actions/runs/910");
    let r = run_node(&env, rerun(json!({"runId": 910, "allJobs": true}))).await;
    assert_eq!(node_step(&r)["port"], "out", "{r}");
    assert_eq!(*env.gh.ci.retried.lock().unwrap(), [("910".to_string(), true), ("910".to_string(), false)]);

    let r = run_node(&env, rerun(json!({"runId": "911"}))).await;
    assert_eq!(node_step(&r)["port"], "none", "{r}");
    assert_eq!(env.gh.ci.retried.lock().unwrap().len(), 2, "nothing re-run");

    // A forge that can't re-run fails the step (its error port), and re-runs nothing.
    env.gh.ci.capabilities.lock().unwrap().retry = false;
    let r = run_node(&env, rerun(json!({"runId": "910"}))).await;
    assert_eq!(node_step(&r)["status"], "failed", "{r}");
    assert_eq!(env.gh.ci.retried.lock().unwrap().len(), 2);
}
