//! Phase 7c end to end: the Pipeline template drives a ticket from Refine to Done with `fake-agent` personas
//! submitting outcomes through pdt-mcp; project policies, auto-start, manual stages, Board stages and the host's cap
//! on agents running a turn.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState};
use pdt_github::PullRequest;
use pdt_github::testing::FakeGithub;
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
    setup_with(None).await
}

/// [`setup`], for a project on GitHub (its origin says so; nothing is fetched) answered by `gh`.
async fn setup_with(gh: Option<std::sync::Arc<FakeGithub>>) -> Env {
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
    if gh.is_some() {
        git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
        git(&repo, &["config", "http.proxy", "http://127.0.0.1:1"]);
    }
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    if let Some(gh) = gh {
        state.github = gh;
    }
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

async fn labelled(env: &Env, title: &str, labels: &[&str]) -> String {
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": title, "labels": labels})),
    )
    .await;
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
    commit_file(worktree, "b.txt");
}

fn commit_file(worktree: &Path, name: &str) {
    std::fs::write(worktree.join(name), "built\n").unwrap();
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
async fn every_step_session_on_a_ticket_opens_a_tab_and_belongs_to_the_ticket() {
    let env = setup().await;
    let t = ticket(&env, "Tabbed").await;
    pipeline(&env, json!({}), &[("refine", "ready"), ("code", "submitted"), ("review", "approved"), ("qa", "pass")])
        .await;
    settings(&env, json!({"autoStart": "todo"})).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    inbox_item(&env, "gate", &t).await;

    // The ticket lists every step's session, each with its persona and the ticket's key; only the Coder's is the
    // attempt's (PR notices go to it).
    let v = get_ticket(&env, &t).await;
    let attempt = v["attempts"][0]["id"].clone();
    let session = |title: &str| {
        v["sessions"].as_array().unwrap().iter().find(|s| s["title"] == title).unwrap_or_else(|| panic!("{title}: {v}"))
    };
    let [refiner, coder, reviewer, qa] =
        ["Refiner · PX-1", "Coder · PX-1", "Reviewer · PX-1", "QA · PX-1"].map(|title| session(title).clone());
    let tagged: Vec<(&Value, &Value)> =
        [&refiner, &coder, &reviewer, &qa].iter().map(|s| (&s["persona"], &s["attemptId"])).collect();
    assert_eq!(
        tagged,
        [
            (&json!("Refiner"), &Value::Null),
            (&json!("Coder"), &attempt),
            (&json!("Reviewer"), &Value::Null),
            (&json!("QA"), &Value::Null)
        ]
    );
    let (_, all) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
    let on_ticket = all.as_array().unwrap().iter().filter(|s| s["ticket"]["key"] == "PX-1").count();
    assert_eq!(on_ticket, 4, "{all}");

    // Auto-start made the ticket's workspace with its tab; the Coder's, Reviewer's and QA's sessions opened after it,
    // in that order. The Refiner ran before there was an attempt: its tab is in the project's workspace.
    let workspace = v["attempts"][0]["workspaceId"].as_str().expect("auto-start made a workspace").to_string();
    let (_, list) = call(&env.app, "GET", "/api/workspaces", None).await;
    let list = list.as_array().unwrap();
    let w = list.iter().find(|w| w["id"] == workspace).unwrap();
    let tabs: Vec<(&Value, &Value)> =
        w["tabs"].as_array().unwrap().iter().map(|tab| (&tab["ticketId"], &tab["sessionId"])).collect();
    assert_eq!(
        tabs,
        [
            (&json!(t), &Value::Null),
            (&Value::Null, &coder["id"]),
            (&Value::Null, &reviewer["id"]),
            (&Value::Null, &qa["id"])
        ],
        "{w}"
    );
    let project = list.iter().find(|w| w["projectId"] == env.project.as_str()).unwrap();
    let in_project: Vec<&Value> = project["tabs"].as_array().unwrap().iter().map(|tab| &tab["sessionId"]).collect();
    assert_eq!(in_project, [&refiner["id"]], "{project}");
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
        json!({"mergeBy": "user", "requireApproval": true, "approvalExemptLabels": [], "maxRunningAttempts": 1,
               "autoStart": "off", "pmActs": false})
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

#[tokio::test]
async fn rejecting_at_accept_with_a_note_comments_on_the_ticket_and_goes_back_to_code() {
    let env = setup().await;
    let t = ticket(&env, "All by hand").await;
    pipeline(&env, json!({"codeBy": "user", "reviewBy": "user", "qaBy": "user"}), &[]).await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    for outcome in ["submitted", "approved", "pass"] {
        let mine = inbox_item(&env, "manual", &t).await;
        answer(&env, &mine, json!({"choice": format!("outcome:{outcome}")})).await;
    }
    let accept = inbox_item(&env, "gate", &t).await;
    answer(&env, &accept, json!({"choice": "reject", "text": "The button is the wrong colour."})).await;
    // Back to Code (a manual step here), and the note is the user's comment, which reaches the coder.
    let again = inbox_item(&env, "manual", &t).await;
    assert_eq!(again["title"], "Your turn: Code");
    let events = timeline(&env, &t).await;
    let note = events["events"].as_array().unwrap().iter().find(|e| e["kind"] == "comment").cloned().unwrap();
    assert_eq!(
        (note["actor"].as_str(), note["body"].as_str(), note["data"]["choice"].as_str()),
        (Some("user"), Some("The button is the wrong colour."), Some("reject"))
    );
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "code");
}

/// What a session's agent said (`fake-agent` echoes each message it gets).
async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_rejected_tickets_coder_hears_why_once_without_the_whole_task_again() {
    let env = setup().await;
    let t = ticket(&env, "Colour the button").await;
    pipeline(&env, json!({"reviewBy": "user", "qaBy": "user"}), &[("code", "submitted")]).await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    for outcome in ["approved", "pass"] {
        let mine = inbox_item(&env, "manual", &t).await;
        answer(&env, &mine, json!({"choice": format!("outcome:{outcome}")})).await;
    }
    let accept = inbox_item(&env, "gate", &t).await;
    answer(&env, &accept, json!({"choice": "reject", "text": "The button is the wrong colour."})).await;

    // The Coder's session (the attempt's) gets the next pass: why it is back, once, and not the task again.
    let coder = get_ticket(&env, &t).await["sessions"][0]["id"].as_str().unwrap().to_string();
    let said = tick_until(&env, "the next pass reaching the coder", || async {
        let said = agent_says(&env, &coder).await;
        said.iter().any(|m| m.contains("back for another pass")).then_some(json!(said))
    })
    .await;
    let pass =
        said.as_array().unwrap().iter().map(|m| m.as_str().unwrap()).find(|m| m.contains("another pass")).unwrap();
    assert!(pass.contains("The user chose `reject`: The button is the wrong colour."), "{pass}");
    assert_eq!(pass.matches("wrong colour").count(), 1, "the note once, not also as a forwarded comment: {pass}");
    assert!(!pass.contains("Your role") && !pass.contains("Ticket:"), "{pass}");
    // Nor later: "Ticket comments to the agent" leaves a gate's note to its run.
    for _ in 0..20 {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let all = agent_says(&env, &coder).await.join("\n");
    assert_eq!(all.matches("wrong colour").count(), 1, "{all}");
}

/// Starts `t` with no session and commits a change on its branch; the attempt.
async fn start_with_a_commit(env: &Env, t: &str, file: &str) -> Value {
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    commit_file(&env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap()), file);
    started["attempt"].clone()
}

async fn until_done(env: &Env, t: &str) -> Value {
    tick_until(env, "the ticket done", || async {
        let v = get_ticket(env, t).await;
        (v["stage"] == "done").then_some(v)
    })
    .await
}

#[tokio::test]
async fn labelled_tickets_merge_once_qa_passes_and_the_rest_wait_at_accept() {
    let env = setup().await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": "automerge, trivial"}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    // The approval guard stays on for every ticket but the exempt ones.
    settings(&env, json!({"requireApproval": true, "approvalExemptLabels": ["AutoMerge", "trivial"]})).await;
    let (_, s) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(s["policy"]["approvalExemptLabels"], json!(["automerge", "trivial"]), "labels are normalised");

    // Labelled: no Inbox item, the run merges it and finishes the ticket.
    let auto = labelled(&env, "Fix a typo", &["automerge"]).await;
    start_with_a_commit(&env, &auto, "typo.txt").await;
    until_done(&env, &auto).await;
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != auto.as_str()), "no Inbox item for it");
    assert_eq!(outcomes(&timeline(&env, &auto).await), ["submitted", "approved", "pass"]);

    // Unlabelled: Accept waits in the Inbox, and an automated merge before it is refused for want of approval.
    let other = labelled(&env, "A real change", &["docs"]).await;
    let attempt = start_with_a_commit(&env, &other, "change.txt").await;
    let accept = inbox_item(&env, "gate", &other).await;
    assert_eq!(accept["group"], "accept");
    let id = attempt["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("approval_required")), "{v}");
    answer(&env, &accept, json!({"choice": "merge"})).await;
    until_done(&env, &other).await;
    let log = std::process::Command::new("git")
        .current_dir(&env.repo)
        .args(["ls-tree", "--name-only", "main"])
        .output()
        .unwrap();
    let files = String::from_utf8_lossy(&log.stdout);
    assert!(files.contains("typo.txt") && files.contains("change.txt"), "both merged into main: {files}");
}

#[tokio::test]
async fn without_the_exemption_a_labelled_tickets_merge_is_blocked_and_the_inbox_asks() {
    let env = setup().await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["automerge"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    settings(&env, json!({"requireApproval": true})).await;
    let t = labelled(&env, "Fix a typo", &["automerge"]).await;
    start_with_a_commit(&env, &t, "typo.txt").await;
    // Accept is skipped, but Merge refuses for want of approval: no agent can fix that, so the Inbox asks.
    let blocked = inbox_item(&env, "gate", &t).await;
    assert_eq!(blocked["group"], "gate");
    let title = blocked["title"].as_str().unwrap();
    assert!(title.starts_with("Merge blocked on ") && title.contains("approval"), "{title}");
    let choices: Vec<&str> =
        blocked["choices"].as_array().unwrap().iter().map(|c| c["title"].as_str().unwrap()).collect();
    assert_eq!(choices, ["Merge again", "Stop"]);
    let v = get_ticket(&env, &t).await;
    assert_eq!((v["pipelineStage"].as_str(), v["agent"]["state"].as_str()), (Some("merge_blocked"), Some("yours")));
    // Merge again is the user's approval: the merge goes through and the ticket finishes.
    answer(&env, &blocked, json!({"choice": "merge_again"})).await;
    let done = until_done(&env, &t).await;
    assert_eq!(done["pipelineStage"], Value::Null);
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "its item closed");
}

#[tokio::test]
async fn a_blocked_merge_closes_its_inbox_item_when_someone_merges_it() {
    let env = setup().await;
    pipeline(&env, json!({"accept": "auto"}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    settings(&env, json!({"mergeBy": "user"})).await;
    let t = ticket(&env, "Mine to merge").await;
    let attempt = start_with_a_commit(&env, &t, "mine.txt").await;
    let blocked = inbox_item(&env, "gate", &t).await;
    assert!(blocked["title"].as_str().unwrap().starts_with("Merge blocked"), "{blocked}");
    let id = attempt["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the run done", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
        runs.as_array().unwrap().iter().all(|r| r["status"] == "succeeded" || r["status"] == "quiet").then_some(runs)
    })
    .await;
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "the item closed with the merge");
}

#[tokio::test]
async fn a_merge_left_to_a_person_asks_again_after_its_time_limit() {
    let env = setup().await;
    pipeline(
        &env,
        json!({"accept": "auto", "merge": "person"}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = ticket(&env, "Waits for a person").await;
    start_with_a_commit(&env, &t, "w.txt").await;
    let wait = tick_until(&env, "the wait for the merge", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
        let r = runs.as_array().unwrap().iter().find(|r| r["status"] == "waiting").cloned()?;
        let (_, run) = call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await;
        run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["nodeId"] == "wait_merge" && s["status"] == "waiting")
            .cloned()
    })
    .await;
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "merge");
    let three_days = 3 * 24 * 60 * 60 * 1000;
    let wake = wait["wakeAt"].as_i64().expect("the wait has a time limit");
    assert!((wake - wait["startedAt"].as_i64().unwrap() - three_days).abs() < 60_000, "{wait}");
    // Its time is up: the Inbox asks (keep waiting, merge or stop), and Merge merges it.
    let mut step = env.state.store.get_step(wait["id"].as_str().unwrap()).await.unwrap().unwrap();
    step.wake_at = Some(1);
    env.state.store.put_step(&step).await.unwrap();
    env.state.store.wake_workflow_run(&step.run_id, 1).await.unwrap();
    let ask = inbox_item(&env, "gate", &t).await;
    assert_eq!(
        ask["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect::<Vec<_>>(),
        ["keep_waiting", "merge", "stop"]
    );
    answer(&env, &ask, json!({"choice": "merge"})).await;
    until_done(&env, &t).await;
}

/// A local merge that conflicts (main moved on): the Coder fixes it, a Reviewer checks the fix, and it merges, with
/// no person in the loop (PDT-124).
#[tokio::test]
async fn a_merge_refused_for_a_conflict_is_fixed_reviewed_and_merged() {
    let env = setup().await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass"), ("fix_review", "approved")],
    )
    .await;
    let t = labelled(&env, "Change a", &["agentic"]).await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let worktree = env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    std::fs::write(worktree.join("a.txt"), "branch\n").unwrap();
    git(&worktree, &["commit", "-qam", "branch"]);
    // Main moves on and conflicts.
    std::fs::write(env.repo.join("a.txt"), "main\n").unwrap();
    git(&env.repo, &["commit", "-qam", "main"]);

    let fix = tick_until(&env, "the fix step", || async {
        let steps = env.state.store.active_agent_steps().await.unwrap();
        steps.into_iter().find(|a| a.node_id == "fix").and_then(|a| a.session_id).map(Value::from)
    })
    .await;
    let fix = fix.as_str().unwrap().to_string();
    let coder = get_ticket(&env, &t).await["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str().unwrap().starts_with("Coder"))
        .unwrap()["id"]
        .clone();
    assert_eq!(coder, json!(fix), "the Coder's own session");
    let said = tick_until(&env, "the fix reaching the coder", || async {
        agent_says(&env, &fix).await.into_iter().find(|m| m.contains("merge was refused")).map(Value::from)
    })
    .await;
    assert!(said.as_str().unwrap().contains("the merge conflicts"), "{said}");
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "code");
    // The Coder merges main in and resolves the conflict, then submits; a fresh Reviewer approves and it merges.
    git(&worktree, &["merge", "-q", "main", "-X", "ours", "-m", "merge main"]);
    let text = r#"mcp: submit_outcome {"outcome": "submitted", "summary": "merged main in"}"#;
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{fix}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    until_done(&env, &t).await;
    assert_eq!(outcomes(&timeline(&env, &t).await), ["submitted", "approved", "pass", "submitted", "approved"]);
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "nobody was asked");
    assert_eq!(std::fs::read_to_string(env.repo.join("a.txt")).unwrap(), "branch\n", "merged into main");
}

fn pull_request(mergeable: Mergeable) -> PullRequest {
    PullRequest {
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "Change it".into(),
        state: PrState::Open,
        draft: false,
        head_sha: "abcdef1234".into(),
        base_branch: "main".into(),
        mergeable,
        behind: false,
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: None,
    }
}

/// The pull request case of PDT-123, through the Pipeline and PR notices: QA passes, the PR conflicts, so the merge is
/// refused (and not recorded as set to merge); the Coder fixes it and a Reviewer checks the fix; CI passing meanwhile
/// turns nothing on; then the Pipeline turns auto-merge on.
#[tokio::test]
async fn a_conflicting_pr_is_fixed_and_reviewed_before_auto_merge_goes_on() {
    conflicting_pr(false).await;
}

/// The same with the run stopped while the Reviewer looks at the fix: CI passing then turns nothing on either, as
/// nothing recorded the refused merge as set to merge (PR notices would turn auto-merge on for it).
#[tokio::test]
async fn a_conflict_fix_left_unreviewed_never_merges() {
    conflicting_pr(true).await;
}

async fn conflicting_pr(abandon: bool) {
    let gh = std::sync::Arc::new(FakeGithub::default());
    let env = setup_with(Some(gh.clone())).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change it", &["agentic"]).await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Conflicting)));
    pdt_api::refresh_prs(&env.state).await;
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();

    let step = |node: &'static str| {
        let env = &env;
        async move {
            tick_until(env, &format!("the {node} step"), || async {
                let steps = env.state.store.active_agent_steps().await.unwrap();
                steps.into_iter().find(|a| a.node_id == node).and_then(|a| a.session_id).map(Value::from)
            })
            .await
            .as_str()
            .unwrap()
            .to_string()
        }
    };
    let fix = step("fix").await;
    let requested = || async { env.state.store.get_attempt(&id).await.unwrap().unwrap().merge_requested };
    assert!(!requested().await, "a refused merge isn't set to merge");
    assert_eq!(merges(), 0);
    let said = tick_until(&env, "the fix reaching the coder", || async {
        agent_says(&env, &fix).await.into_iter().find(|m| m.contains("merge was refused")).map(Value::from)
    })
    .await;
    assert!(said.as_str().unwrap().contains("conflicts with its base"), "{said}");
    // Fixed and pushed: GitHub can merge it now.
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    pdt_api::refresh_prs(&env.state).await;
    let submit = |session: String, outcome: &'static str| {
        let env = &env;
        async move {
            let text = format!(r#"mcp: submit_outcome {{"outcome": "{outcome}", "summary": "{outcome}"}}"#);
            let (s, v) =
                call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
            assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        }
    };
    submit(fix.clone(), "submitted").await;
    let reviewer = step("fix_review").await;
    assert_ne!(reviewer, fix, "a fresh Reviewer");
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "review");
    if abandon {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
        let run = runs.as_array().unwrap().iter().find(|r| r["status"] == "waiting").unwrap()["id"].clone();
        let (s, v) =
            call(&env.app, "POST", &format!("/api/workflow-runs/{}/cancel", run.as_str().unwrap()), None).await;
        assert!(s.is_success(), "{v}");
        assert!(env.state.store.active_agent_steps().await.unwrap().is_empty(), "no agent on it now");
    }
    // CI passes while the Reviewer works (or after the run stopped): nothing merges unreviewed.
    gh.set_runs(
        &branch,
        vec![CiRun {
            id: 8,
            attempt: 1,
            workflow: "ci".into(),
            state: CiState::Passed,
            url: "https://github.com/o/r/actions/runs/8".into(),
            head_sha: "abcdef1234".into(),
            head_branch: branch.clone(),
            created_at: "2026-10-23T00:00:00Z".into(),
            failed_jobs: vec![],
        }],
    );
    pdt_api::refresh_prs(&env.state).await;
    for _ in 0..10 {
        pdt_api::run_workflows(&env.state).await;
    }
    assert_eq!(merges(), 0, "not before the review");
    if abandon {
        assert!(!requested().await);
        return;
    }
    submit(reviewer, "approved").await;
    tick_until(&env, "auto-merge on", || async { (merges() == 1).then_some(Value::Null) }).await;
    let info = env.state.store.get_attempt(&id).await.unwrap().unwrap();
    assert!(info.merge_requested && info.finish_on_merge, "the reviewed merge is set to merge");
    let v = get_ticket(&env, &t).await;
    assert_eq!((v["stage"].as_str(), v["pipelineStage"].as_str()), (Some("in_progress"), Some("merge")), "merging");
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "nobody was asked");
}
