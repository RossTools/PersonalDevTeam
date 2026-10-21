//! Phase 7c end to end: the Pipeline template drives a ticket from Refine to Done with `fake-agent` personas
//! submitting outcomes through pdt-mcp; project policies, auto-start, manual stages, Board stages and the host's cap
//! on agents running a turn.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    // A local merge commits as the user: CI runners have no global identity.
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
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
    Env { app, state, project: p["id"].as_str().unwrap().into(), repo, _tmp: tmp }
}

async fn ticket(env: &Env, title: &str) -> String {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn get_ticket(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1
}

async fn settings(env: &Env, body: Value) {
    let mut b = json!({"agentCredentials": null, "runOverrides": null});
    for (k, v) in body.as_object().unwrap() {
        b[k] = v.clone();
    }
    let (s, v) = call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(b)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// The Pipeline instantiated for the project with `params`, every agent step's prompt replaced by the pdt-mcp call
/// its fake persona makes (`outcomes`: node → outcome), and enabled.
async fn pipeline(env: &Env, params: Value, outcomes: &[(&str, &str)]) -> String {
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:pipeline/instantiate",
        Some(json!({"params": params, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!(w["enabled"], false, "a template's workflow starts off");
    let id = w["id"].as_str().unwrap().to_string();
    let mut doc = w["doc"].clone();
    for n in doc["nodes"].as_array_mut().unwrap() {
        if let Some((_, outcome)) = outcomes.iter().find(|(node, _)| n["id"] == *node) {
            n["config"]["prompt"] = json!(format!(
                r#"mcp: submit_outcome {{"outcome": "{outcome}", "summary": "{} says {outcome}"}}"#,
                n["id"].as_str().unwrap()
            ));
        }
    }
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc, "note": "fake"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&env.app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    id
}

/// Ticks the engine (and the scheduler) until `done` holds (up to 30 s).
async fn tick_until<F, Fut>(env: &Env, what: &str, done: F) -> Value
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        pdt_api::run_workflows(&env.state).await;
        pdt_api::schedule_tickets(&env.state).await;
        if let Some(v) = done().await {
            return v;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn inbox(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

async fn inbox_item(env: &Env, kind: &str, ticket: &str) -> Value {
    tick_until(env, &format!("a {kind} item"), || async {
        inbox(env).await.into_iter().find(|i| i["kind"] == kind && i["ticketId"] == ticket)
    })
    .await
}

async fn answer(env: &Env, item: &Value, body: Value) {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/inbox/{}/answer", item["id"].as_str().unwrap()), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

fn outcomes(t: &Value) -> Vec<String> {
    t["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "outcome")
        .map(|e| e["data"]["outcome"].as_str().unwrap().to_string())
        .collect()
}

async fn timeline(env: &Env, id: &str) -> Value {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{id}/events?limit=200"), None).await;
    json!({"events": v["items"]})
}

/// Commits a change on the attempt's branch (what a real Coder would have done).
fn commit_in(worktree: &Path) {
    std::fs::write(worktree.join("b.txt"), "built\n").unwrap();
    git(worktree, &["add", "."]);
    git(worktree, &["commit", "-m", "build", "-q"]);
}

#[tokio::test]
async fn a_draft_goes_through_refine_code_review_and_qa_and_merges_once_accepted() {
    let env = setup().await;
    let t = ticket(&env, "Add b.txt").await;
    // Refine needs an enabled workflow that listens.
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_refine_workflow")), "{v}");
    pipeline(&env, json!({}), &[("refine", "ready"), ("code", "submitted"), ("review", "approved"), ("qa", "pass")])
        .await;
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 2})).await;

    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["pipelineStage"], "refine");

    // Refined: To do (the template's default), out of the pipeline stage; then auto-start takes it.
    tick_until(&env, "the ticket started", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "in_progress").then_some(v)
    })
    .await;
    let started = timeline(&env, &t).await;
    let start = started["events"].as_array().unwrap().iter().find(|e| e["kind"] == "started").unwrap().clone();
    assert_eq!(start["data"]["auto"], true, "{start}");

    // Code → Review → QA, each a persona's session; then Accept waits in the Inbox, and the card shows it is mine.
    let accept = inbox_item(&env, "gate", &t).await;
    assert_eq!(accept["group"], "accept");
    let v = get_ticket(&env, &t).await;
    assert_eq!(v["pipelineStage"], "accept");
    assert_eq!(v["agent"]["state"], "yours", "{}", v["agent"]);
    let (_, list) =
        call(&env.app, "GET", &format!("/api/tickets?project={}&stage=in_progress", env.project), None).await;
    assert_eq!(list["items"][0]["agent"]["state"], "yours");
    assert_eq!(outcomes(&timeline(&env, &t).await), ["ready", "submitted", "approved", "pass"]);
    let personas: Vec<String> =
        v["sessions"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap().to_string()).collect();
    assert!(personas.iter().any(|p| p.starts_with("Coder")), "the Coder's session is the attempt's: {personas:?}");

    // Accept: the user's approval is recorded and the run merges the branch locally, then finishes the ticket.
    let worktree = env.repo.join(".worktrees").join(v["attempts"][0]["branch"].as_str().unwrap());
    commit_in(&worktree);
    answer(&env, &accept, json!({"choice": "merge"})).await;
    let done = tick_until(&env, "the ticket done", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "done").then_some(v)
    })
    .await;
    assert_eq!(done["pipelineStage"], Value::Null);
    let events = timeline(&env, &t).await;
    let review = events["events"].as_array().unwrap().iter().find(|e| e["kind"] == "review").unwrap().clone();
    assert_eq!((review["actor"].as_str(), review["data"]["workflowName"].as_str()), (Some("user"), Some("Pipeline")));
    let log =
        std::process::Command::new("git").current_dir(&env.repo).args(["log", "--oneline", "main"]).output().unwrap();
    assert!(String::from_utf8_lossy(&log.stdout).contains("Merge"), "merged into main");
}

#[tokio::test]
async fn a_strict_project_is_other_parameters_and_policies() {
    let env = setup().await;
    let t = ticket(&env, "Strict work").await;
    pipeline(&env, json!({"reviewBy": "user", "merge": "person"}), &[("code", "submitted"), ("qa", "pass")]).await;
    settings(&env, json!({"mergeBy": "user", "requireApproval": true, "maxRunningAttempts": 1})).await;
    let (_, s) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(
        s["policy"],
        json!({"mergeBy": "user", "requireApproval": true, "maxRunningAttempts": 1, "autoStart": "off"})
    );

    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    // The cap: a second ticket is a finding Start reports (force starts anyway).
    let other = ticket(&env, "Another").await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{other}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("start_blocked")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("cap of 1 running tickets"), "{v}");

    // Review is the user's: a Your turn item with the outcomes.
    let mine = inbox_item(&env, "manual", &t).await;
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "review");
    assert_eq!(get_ticket(&env, &t).await["agent"]["state"], "yours");
    answer(&env, &mine, json!({"choice": "outcome:approved", "text": "Looks right."})).await;
    let accept = inbox_item(&env, "gate", &t).await;

    // Before the user accepts, an automated merge is refused by the policy.
    let attempt = started["attempt"]["id"].as_str().unwrap();
    let worktree = env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    commit_in(&worktree);
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/merge"), Some(json!({}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("merge_not_allowed")), "{v}");
    answer(&env, &accept, json!({"choice": "merge"})).await;
    // merge: person → the run waits for someone to merge.
    tick_until(&env, "the run waiting for the merge", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
        let r = runs.as_array().unwrap().iter().find(|r| r["status"] == "waiting").cloned()?;
        let (_, run) = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await;
        run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["nodeId"] == "wait_merge" && s["status"] == "waiting")
            .then_some(run)
    })
    .await;
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the run done", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
        runs.as_array().unwrap().iter().all(|r| r["status"] == "succeeded" || r["status"] == "quiet").then_some(runs)
    })
    .await;
    let events = timeline(&env, &t).await;
    let outs: Vec<(String, String)> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "outcome")
        .map(|e| (e["data"]["outcome"].as_str().unwrap().into(), e["actor"].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        outs,
        [("submitted".into(), "agent".into()), ("approved".into(), "user".into()), ("pass".into(), "agent".into())]
    );
    // External: nobody merges in PDT.
    settings(&env, json!({"mergeBy": "external"})).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("merge_not_allowed")), "{v}");
}

#[tokio::test]
async fn agent_steps_wait_for_a_slot_under_the_host_cap() {
    let env = setup().await;
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxAgentTurns": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // A busy conversation takes the one slot.
    let busy = ticket(&env, "Busy").await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{busy}/start"), Some(json!({"agent": "claude"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": "slow"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");

    let t = ticket(&env, "Waits").await;
    pipeline(&env, json!({}), &[("refine", "ready")]).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let queued = tick_until(&env, "the step waiting for a slot", || async {
        let v = get_ticket(&env, &t).await;
        (v["agent"]["state"] == "queued").then_some(v)
    })
    .await;
    assert_eq!(queued["agent"]["persona"], "Refiner");
    // The slot frees: the step runs and the ticket is refined.
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{session}/interrupt"), None).await;
    assert!(s.is_success(), "{v}");
    tick_until(&env, "the ticket refined", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "todo").then_some(v)
    })
    .await;
}
