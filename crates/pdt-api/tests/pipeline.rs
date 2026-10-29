//! Phase 7c end to end: the Pipeline template drives a ticket from Refine to Done with `fake-agent` personas
//! submitting outcomes through pdt-mcp; project policies, auto-start, manual stages, Board stages and the host's cap
//! on agents running a turn.

mod common;

use axum::http::StatusCode;
use common::{call, git, mark_pushed};
use pdt_api::{AppState, router};
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState};
use pdt_forge::testing::FakeForge;
use pdt_forge::{ChangeRequest, MergeSettings};
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

/// [`setup`], for a project on GitHub (its origin says so; nothing is fetched) answered by a `FakeForge`.
async fn setup_with(gh: Option<std::sync::Arc<FakeForge>>) -> Env {
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
    state.integrations = std::sync::Arc::new(pdt_api::Integrations::new(vec![std::sync::Arc::new(FakeCi)]));
    state.integrations.enable(&["fakeci".into()]);
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

/// A CI integration (PDT-173): a project linked to it has CI from it.
struct FakeCi;

impl pdt_api::Integration for FakeCi {
    fn id(&self) -> &'static str {
        "fakeci"
    }

    fn project_settings(&self) -> Option<pdt_core::integration::ProjectSettingsSchema> {
        Some(pdt_core::integration::ProjectSettingsSchema {
            integration: "fakeci".into(),
            label: "Fake CI".into(),
            connection_kind: "http".into(),
            ..Default::default()
        })
    }

    fn project_ci(&self, settings: &pdt_store::ProjectSettings) -> Option<&'static str> {
        settings.integrations.contains_key("fakeci").then_some("Fake CI")
    }
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
    mark_pushed(worktree);
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
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 2, "startDelaySeconds": 0})).await;

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

async fn todo(env: &Env, title: &str) -> String {
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": title, "stage": "todo"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// A Draft ticket made from outside input (PDT-201), as an issue import makes it.
async fn external(env: &Env, title: &str) -> String {
    let t = pdt_core::ticket::NewTicket { title: title.into(), ..Default::default() }.validate().unwrap();
    let t = pdt_core::ticket::ValidNewTicket { source: pdt_core::ticket::TicketSource::External, ..t };
    let id = uuid::Uuid::now_v7().to_string();
    env.state.store.create_ticket(&id, &env.project, &t, 1).await.unwrap();
    id
}

/// PDT-201: a refined external ticket waits in Ready (whatever afterRefine says) until the user moves it to To do.
#[tokio::test]
async fn a_refined_external_ticket_waits_in_ready_for_the_user() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("refine", "ready")]).await;
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 2, "startDelaySeconds": 0})).await;
    let t = external(&env, "From an issue").await;
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let v = tick_until(&env, "the ticket refined", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "ready" && v["pipelineStage"].is_null()).then_some(v)
    })
    .await;
    assert_eq!(v["admitted"], false);
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
        pdt_api::schedule_tickets(&env.state).await;
    }
    assert_eq!(get_ticket(&env, &t).await["stage"], "ready", "auto-start never takes it");

    // The user moves it to To do: let in, auto-start takes it.
    let (s, v) =
        common::call_as_ui(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "todo"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tick_until(&env, "the ticket started", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "in_progress").then_some(v)
    })
    .await;
}

/// PDT-201: a session on an external ticket the user hasn't let in gets no secrets.
#[tokio::test]
async fn a_session_on_an_external_ticket_gets_no_secrets_until_it_is_let_in() {
    let env = setup().await;
    // A Refine step run by the Coder persona, whose tools include the secret ones.
    let wf = pipeline(&env, json!({"refiner": "Coder"}), &[]).await;
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let mut doc = w["doc"].clone();
    let run = r#"mcp: run_with_secrets {"command": "echo hi", "env": {}}"#;
    for n in doc["nodes"].as_array_mut().unwrap() {
        if n["id"] == "refine" {
            n["config"]["prompt"] = json!(run);
        }
    }
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/workflows/{wf}"), Some(json!({"doc": doc, "note": "secrets"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let t = external(&env, "From an issue").await;
    call(&env.app, "POST", &format!("/api/tickets/{t}/refine"), Some(json!({"by": "user"}))).await;
    let answer = |n: usize| {
        let env = &env;
        let t = t.clone();
        async move {
            tick_until(env, "run_with_secrets answered", || async {
                let v = get_ticket(env, &t).await;
                let s = v["sessions"][0]["id"].as_str()?.to_string();
                agent_says(env, &s)
                    .await
                    .into_iter()
                    .filter(|m| m.starts_with("mcp run_with_secrets"))
                    .nth(n)
                    .map(|m| json!([s, m]))
            })
            .await
        }
    };
    let said = answer(0).await;
    let (session, said) = (said[0].as_str().unwrap().to_string(), said[1].as_str().unwrap().to_string());
    assert!(said.contains("failed") && said.contains("no secrets"), "{said}");

    // Let in: the same call runs.
    common::call_as_ui(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "todo"}))).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": run}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let said = answer(1).await;
    assert!(!said[1].as_str().unwrap().contains("no secrets"), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
}

#[tokio::test]
async fn auto_start_waits_until_a_ticket_has_been_quiet_for_the_start_delay() {
    let env = setup().await;
    let (_, v) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(v["policy"]["startDelaySeconds"], 120, "two minutes unless the project says otherwise");
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 5})).await;

    // Just created: auto-start leaves it, and the ticket (and the list) says when it will take it.
    let a = todo(&env, "A").await;
    pdt_api::schedule_tickets(&env.state).await;
    let v = get_ticket(&env, &a).await;
    assert_eq!(v["stage"], "todo");
    let at = v["autoStartAt"].as_i64().expect("waiting out the delay");
    assert_eq!(at, v["updatedAt"].as_i64().unwrap() + 120_000);
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}&stage=todo", env.project), None).await;
    assert_eq!(list["items"][0]["autoStartAt"], at);

    // A link on either side starts the quiet period again.
    let b = todo(&env, "B").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{b}/links"), Some(json!({"to": a, "kind": "relates"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(get_ticket(&env, &a).await["autoStartAt"].as_i64().unwrap() > at, "the link restarted A's delay");

    // A person's Start is never delayed.
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{a}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(get_ticket(&env, &a).await["autoStartAt"], Value::Null);

    // A blocker linked inside the delay keeps the ticket from starting once it is over.
    settings(&env, json!({"startDelaySeconds": 1})).await;
    let c = todo(&env, "C").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{b}/links"), Some(json!({"to": c, "kind": "blocks"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(get_ticket(&env, &c).await["autoStartAt"], Value::Null, "blocked: auto-start won't take it");
    tick_until(&env, "B started", || async {
        let v = get_ticket(&env, &b).await;
        (v["stage"] == "in_progress").then_some(v)
    })
    .await;
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(get_ticket(&env, &c).await["stage"], "todo", "still blocked by B");

    // 0: at once.
    settings(&env, json!({"startDelaySeconds": 0})).await;
    let d = todo(&env, "D").await;
    assert_eq!(get_ticket(&env, &d).await["autoStartAt"], Value::Null);
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(get_ticket(&env, &d).await["stage"], "in_progress");
}

async fn todo_at(env: &Env, title: &str, priority: &str) -> String {
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": title, "stage": "todo", "priority": priority})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// PDT-218: auto-start takes the most urgent To do ticket first (rank within a priority), unless the project keeps to
/// backlog rank.
#[tokio::test]
async fn auto_start_takes_the_most_urgent_ticket_first_unless_the_project_says_rank() {
    let env = setup().await;
    let (_, v) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(v["policy"]["autoStartOrder"], "priority", "most urgent first unless the project says otherwise");
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 10, "startDelaySeconds": 0})).await;
    // No host cap on agents running a turn: each start's session keeps working.
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxAgentTurns": 0}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let stage = |id: String| {
        let env = &env;
        async move { get_ticket(env, &id).await["stage"].as_str().unwrap().to_string() }
    };
    // A new To do ticket goes to the top: the medium ones rank above the high one, `medium` first.
    let high = todo_at(&env, "High", "high").await;
    let medium_later = todo_at(&env, "Medium later", "medium").await;
    let medium = todo_at(&env, "Medium", "medium").await;
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(stage(high.clone()).await, "in_progress", "the high-priority ticket first");
    assert_eq!(stage(medium.clone()).await, "todo");
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(stage(medium.clone()).await, "in_progress", "then rank within a priority");
    assert_eq!(stage(medium_later.clone()).await, "todo");
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(stage(medium_later).await, "in_progress");

    // By rank: today's order, whatever the priority.
    settings(&env, json!({"autoStartOrder": "rank"})).await;
    let (_, v) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(v["policy"]["autoStartOrder"], "rank");
    let urgent = todo_at(&env, "Urgent", "urgent").await;
    let low = todo_at(&env, "Low", "low").await;
    pdt_api::schedule_tickets(&env.state).await;
    assert_eq!(stage(low).await, "in_progress", "the first in rank");
    assert_eq!(stage(urgent).await, "todo");
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"autoStartOrder": "x"})))
            .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[tokio::test]
async fn every_step_session_on_a_ticket_opens_a_tab_and_belongs_to_the_ticket() {
    let env = setup().await;
    // On GitHub (the default forge, which runs nothing in tests), so its sessions are told how to read CI.
    git(&env.repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
    git(&env.repo, &["config", "http.proxy", "http://127.0.0.1:1"]);
    // Its branch is "pushed" already, so the Coder's `submitted` passes (it commits nothing).
    git(&env.repo, &["update-ref", "refs/remotes/origin/pdt-PX-1-tabbed", "HEAD"]);
    let t = ticket(&env, "Tabbed").await;
    pipeline(&env, json!({}), &[("refine", "ready"), ("code", "submitted"), ("review", "approved"), ("qa", "pass")])
        .await;
    settings(&env, json!({"autoStart": "todo", "startDelaySeconds": 0})).await;
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
    // PDT-165: the Reviewer and QA have no attempt of their own, but read its CI like the Coder, with the forge tools
    // the GitHub forge names (PDT-176, PDT-177).
    let branch = v["attempts"][0]["branch"].as_str().unwrap();
    let instructions = |s: &Value| {
        let id = s["id"].as_str().unwrap();
        std::fs::read_to_string(env.state.data_dir.join("sessions").join(id).join("instructions.md")).unwrap()
    };
    for s in [&coder, &reviewer, &qa] {
        let text = instructions(s);
        assert!(text.contains(&format!("`ci_runs` (the runs on {branch} and")), "{}: {text}", s["title"]);
    }
    assert!(!instructions(&refiner).contains("`ci_runs` (the runs"), "no attempt yet");
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
               "autoStart": "off", "autoStartOrder": "priority", "pmActs": false, "startDelaySeconds": 120,
               "requireGreenCi": true, "triageApproval": "inbox"})
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

/// pdtd stopping (an update): every running session is interrupted and the conversations' processes close.
async fn restart(env: &Env) {
    env.state.store.mark_running_interrupted(1).await.unwrap();
    pdt_api::shutdown_conversations(&env.state).await;
}

async fn session_status(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1["status"].clone()
}

/// PDT-144: a restart interrupted the Coder's conversation; rejecting at Accept reopens it with why it is back,
/// rather than telling nobody and stalling the step.
#[tokio::test]
async fn a_rejected_tickets_coder_interrupted_by_a_restart_reopens_and_hears_why() {
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
    let coder = get_ticket(&env, &t).await["sessions"][0]["id"].as_str().unwrap().to_string();
    restart(&env).await;
    assert_eq!(session_status(&env, &coder).await, "interrupted");
    answer(&env, &accept, json!({"choice": "reject", "text": "The button is the wrong colour."})).await;

    let said = tick_until(&env, "the next pass reaching the interrupted coder", || async {
        let said = agent_says(&env, &coder).await;
        said.iter().any(|m| m.contains("back for another pass")).then_some(json!(said))
    })
    .await;
    assert!(said.to_string().contains("The user chose `reject`: The button is the wrong colour."), "{said}");
    assert_eq!(session_status(&env, &coder).await, "running", "it reopened");
    assert_eq!(get_ticket(&env, &t).await["sessions"].as_array().unwrap().len(), 1, "no new session");
    // (fake-agent only echoes the pass, so its turn ends without an outcome: that stalls it, in time, as usual.)
    for _ in 0..10 {
        pdt_api::run_workflows(&env.state).await;
    }
    let lost = |i: &Value| {
        let why = i["body"]["why"].as_str().unwrap_or_default();
        why.contains("stopped before") || why.contains("never delivered")
    };
    assert!(!inbox(&env).await.iter().any(lost), "{:#?}", inbox(&env).await);
}

/// PDT-144: "Ticket comments to the agent" reopens the attempt's conversation a restart interrupted.
#[tokio::test]
async fn a_comment_reaches_the_coder_a_restart_interrupted() {
    let env = setup().await;
    let t = ticket(&env, "Colour the button").await;
    pipeline(&env, json!({"reviewBy": "user", "qaBy": "user"}), &[("code", "submitted")]).await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    inbox_item(&env, "manual", &t).await;
    let coder = get_ticket(&env, &t).await["sessions"][0]["id"].as_str().unwrap().to_string();
    restart(&env).await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/comments"),
        Some(json!({"body": "Make it blue.", "by": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    tick_until(&env, "the comment reaching the interrupted coder", || async {
        let said = agent_says(&env, &coder).await;
        said.iter().any(|m| m.contains("New comment on") && m.contains("Make it blue.")).then_some(json!(null))
    })
    .await;
    assert_eq!(session_status(&env, &coder).await, "running");
}

/// PDT-144: a terminal a restart interrupted can't take the step's message, so the step starts a session of its own
/// with the whole task.
#[tokio::test]
async fn a_step_on_an_attempt_whose_terminal_was_interrupted_gets_a_new_session() {
    let env = setup().await;
    let t = ticket(&env, "Colour the button").await;
    pipeline(&env, json!({"reviewBy": "user", "qaBy": "user"}), &[("code", "submitted")]).await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let terminal = v["session"]["id"].as_str().unwrap().to_string();
    // The restart comes before the engine's next tick picks the start up.
    restart(&env).await;
    env.state.ptys.shutdown().await;
    assert_eq!(session_status(&env, &terminal).await, "interrupted");
    // The Coder's own conversation submits, and review is next.
    inbox_item(&env, "manual", &t).await;
    let sessions = get_ticket(&env, &t).await["sessions"].as_array().unwrap().clone();
    let coder = sessions.iter().find(|x| x["id"] != terminal.as_str()).expect("a new session");
    assert_eq!(coder["mode"], "conversation", "{coder}");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "stalled"));
}

/// PDT-144: the Coder's session was handed to its terminal and a restart stopped that: rejecting at Accept starts a new
/// conversation with the whole task and why it is back.
#[tokio::test]
async fn a_rejected_tickets_coder_whose_terminal_stopped_gets_a_new_session_with_the_task_and_why() {
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
    let first = get_ticket(&env, &t).await["sessions"][0]["id"].as_str().unwrap().to_string();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{first}/mode"), Some(json!({"mode": "terminal"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "terminal", "{v}");
    restart(&env).await;
    env.state.ptys.shutdown().await;
    assert_eq!(session_status(&env, &first).await, "interrupted");
    answer(&env, &accept, json!({"choice": "reject", "text": "The button is the wrong colour."})).await;

    let coder = tick_until(&env, "a new Coder session", || async {
        let sessions = get_ticket(&env, &t).await["sessions"].as_array().unwrap().clone();
        sessions.into_iter().find(|x| x["id"] != first.as_str())
    })
    .await;
    assert_eq!(coder["mode"], "conversation", "{coder}");
    let id = coder["id"].as_str().unwrap().to_string();
    let said = tick_until(&env, "the new Coder's first message", || async {
        let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0"), None).await;
        v.as_array().unwrap().iter().find(|e| e["event"]["kind"] == "user_message").map(|e| e["event"]["text"].clone())
    })
    .await;
    let said = said.as_str().unwrap();
    assert!(said.contains("Ticket: ") && said.contains("Colour the button"), "the whole task: {said}");
    assert!(
        said.contains("This step is back for another pass. The user chose `reject`: The button is the wrong colour."),
        "{said}"
    );
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

/// By default the `agentic` label skips Accept; every other ticket still waits there (PDT-200).
#[tokio::test]
async fn a_default_pipeline_accepts_agentic_tickets_and_the_rest_wait_at_accept() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let agentic = labelled(&env, "Agentic", &["agentic"]).await;
    start_with_a_commit(&env, &agentic, "agentic.txt").await;
    until_done(&env, &agentic).await;
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != agentic.as_str()), "no Inbox item for it");

    let other = ticket(&env, "Unlabelled").await;
    start_with_a_commit(&env, &other, "other.txt").await;
    let accept = inbox_item(&env, "gate", &other).await;
    assert_eq!(accept["group"], "accept");
}

/// A Pipeline that saved an empty list keeps it: `agentic` tickets wait at Accept too (PDT-200).
#[tokio::test]
async fn an_explicit_empty_list_accepts_no_label_automatically() {
    let env = setup().await;
    pipeline(&env, json!({"autoAcceptLabels": []}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")])
        .await;
    let t = labelled(&env, "Agentic", &["agentic"]).await;
    start_with_a_commit(&env, &t, "agentic.txt").await;
    let accept = inbox_item(&env, "gate", &t).await;
    assert_eq!(accept["group"], "accept");
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
    // Git on Windows may check the merged file out with CRLF.
    let merged = std::fs::read_to_string(env.repo.join("a.txt")).unwrap().replace("\r\n", "\n");
    assert_eq!(merged, "branch\n", "merged into main");
}

fn pull_request(mergeable: Mergeable) -> ChangeRequest {
    ChangeRequest {
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
/// nothing recorded the refused merge as set to merge (PR notices would turn auto-merge on for it). A later conflict
/// still gets PR notices' fix step, as the merge was refused (PDT-139).
#[tokio::test]
async fn a_conflict_fix_left_unreviewed_never_merges() {
    conflicting_pr(true).await;
}

async fn conflicting_pr(abandon: bool) {
    let gh = std::sync::Arc::new(FakeForge::default());
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
        // The refused merge still counts (PDT-139): when the PR conflicts again, PR notices puts the Coder on it.
        assert!(env.state.store.get_attempt(&id).await.unwrap().unwrap().merge_refused);
        let mut again = pull_request(Mergeable::Conflicting);
        again.head_sha = "fedcba9876".into();
        gh.set_pr(&branch, Some(again));
        pdt_api::refresh_prs(&env.state).await;
        let run = tick_until(&env, "PR notices fixing the new conflict", || async {
            let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
            for r in runs.as_array().unwrap().iter().filter(|r| r["workflowName"] == "PR notices") {
                let (_, run) =
                    call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await;
                let fixing = run["steps"].as_array().unwrap().iter().any(|s| s["nodeId"] == "fix");
                if fixing && run["trigger"]["headSha"] == "fedcba9876" {
                    return Some(run);
                }
            }
            None
        })
        .await;
        let nodes: Vec<&str> = run["steps"].as_array().unwrap().iter().map(|s| s["nodeId"].as_str().unwrap()).collect();
        assert_eq!(nodes[..3], ["conflict", "asked", "busy"], "{run}");
        assert_eq!(run["trigger"]["mergeRefused"], true);
        assert_eq!(merges(), 0, "nothing merges before the fix");
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

fn ci_run(id: u64, branch: &str, state: CiState) -> CiRun {
    CiRun {
        id,
        attempt: 1,
        workflow: "ci".into(),
        state,
        url: format!("https://github.com/o/r/actions/runs/{id}"),
        head_sha: "abcdef1234".into(),
        head_branch: branch.into(),
        created_at: format!("2026-10-25T00:00:{id:02}Z"),
        failed_jobs: vec![],
    }
}

/// The session of the agent step at `node`, once it runs.
async fn step_session(env: &Env, node: &'static str) -> String {
    tick_until(env, &format!("the {node} step"), || async {
        let steps = env.state.store.active_agent_steps().await.unwrap();
        steps.into_iter().find(|a| a.node_id == node).and_then(|a| a.session_id).map(Value::from)
    })
    .await
    .as_str()
    .unwrap()
    .to_string()
}

async fn submit_in(env: &Env, session: &str, outcome: &str) {
    let text = format!(r#"mcp: submit_outcome {{"outcome": "{outcome}", "summary": "{outcome}"}}"#);
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

async fn runs_of(env: &Env, t: &str, workflow: &str) -> Vec<Value> {
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
    let mut out = vec![];
    for r in runs.as_array().unwrap().iter().filter(|r| r["workflowName"] == workflow) {
        out.push(call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["id"].as_str().unwrap()), None).await.1);
    }
    out
}

/// PDT-151: a Pipeline never merges a pull request whose CI is pending or red, whatever GitHub would allow. Pending:
/// the merge waits for CI; red: the Coder fixes it (PR notices leaves the failure to the waiting merge) and a
/// Reviewer checks the fix; green: it merges.
#[tokio::test]
async fn a_pipeline_merge_waits_for_pending_ci_and_sends_red_ci_back_to_the_coder() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change it", &["agentic"]).await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();

    // Pending: the merge waits for CI.
    let waiting = tick_until(&env, "the merge waiting for CI", || async {
        let runs = runs_of(&env, &t, "Pipeline").await;
        let steps = runs.first()?["steps"].as_array()?.clone();
        steps.into_iter().find(|s| s["nodeId"] == "wait_ci" && s["status"] == "waiting")
    })
    .await;
    assert_eq!(waiting["wait"]["event"]["also"], json!(["ci.run_failed"]), "{waiting}");
    let merge = runs_of(&env, &t, "Pipeline").await[0]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["nodeId"] == "merge")
        .unwrap()
        .clone();
    assert_eq!(merge["output"]["code"], "ci_pending", "{merge}");
    assert_eq!(merges(), 0, "not while CI runs");
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "merge");
    assert!(env.state.store.attempt_merge_waits_for_ci(attempt["id"].as_str().unwrap()).await.unwrap());

    // Red: the waiting merge takes it to the Coder; PR notices doesn't also tell it.
    let mut red = ci_run(2, &branch, CiState::Failed);
    red.failed_jobs = vec!["test (linux)".into()];
    gh.set_runs(&branch, vec![red]);
    gh.set_jobs(2, &["test (linux)"]);
    pdt_api::refresh_prs(&env.state).await;
    let fix = step_session(&env, "fix").await;
    let said = tick_until(&env, "the fix reaching the coder", || async {
        agent_says(&env, &fix).await.into_iter().find(|m| m.contains("merge was refused")).map(Value::from)
    })
    .await;
    assert!(said.as_str().unwrap().contains("CI failed on PR #12 at abcdef1 (test (linux))"), "{said}");
    assert_eq!(merges(), 0);
    assert!(runs_of(&env, &t, "PR notices").await.is_empty(), "the waiting merge handles the failure");

    // Fixed, reviewed, green: it merges.
    gh.set_runs(&branch, vec![ci_run(3, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    submit_in(&env, &fix, "submitted").await;
    let reviewer = step_session(&env, "fix_review").await;
    submit_in(&env, &reviewer, "approved").await;
    tick_until(&env, "auto-merge on", || async { (merges() == 1).then_some(Value::Null) }).await;
}

/// PDT-151: a repository with no CI at all merges as before, and the ticket says so.
#[tokio::test]
async fn a_repository_without_ci_merges_and_says_so() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change it", &["agentic"]).await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    pdt_api::refresh_prs(&env.state).await;
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();
    tick_until(&env, "auto-merge on", || async { (merges() == 1).then_some(Value::Null) }).await;
    let events = timeline(&env, &t).await;
    let said =
        events["events"].as_array().unwrap().iter().any(|e| {
            e["kind"] == "comment" && e["body"].as_str().is_some_and(|b| b.contains("without waiting for CI"))
        });
    assert!(said, "{events}");
}

/// PDT-151: with CI turned off for the project, a merge doesn't wait for it.
#[tokio::test]
async fn a_project_can_merge_without_waiting_for_ci() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    settings(&env, json!({"requireGreenCi": false})).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change it", &["agentic"]).await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();
    tick_until(&env, "auto-merge on", || async { (merges() == 1).then_some(Value::Null) }).await;
}

/// A started ticket whose PR merged at its first commit, with a second commit pushed after: its gate item.
async fn merged_then_pushed(env: &Env, gh: &FakeForge, title: &str) -> (String, String, Value) {
    let t = ticket(env, title).await;
    let attempt = start_with_a_commit(env, &t, &format!("{title}.txt")).await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    let worktree = env.repo.join(".worktrees").join(&branch);
    let mut pr = pull_request(Mergeable::Mergeable);
    pr.state = PrState::Merged;
    let head = std::process::Command::new("git").current_dir(&worktree).args(["rev-parse", "HEAD"]).output().unwrap();
    pr.head_sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    gh.set_pr(&branch, Some(pr));
    commit_file(&worktree, &format!("{title}-late.txt"));
    pdt_api::refresh_prs(&env.state).await;
    pdt_api::check_merges(&env.state).await;
    let item = inbox_item(env, "gate", &t).await;
    (t, branch, item)
}

fn branch_exists(env: &Env, branch: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(&env.repo)
        .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// PDT-151: commits pushed after the PR merged keep the attempt open; the Inbox says so and asks what becomes of them.
/// Open follow-up makes a ticket that cherry-picks them and finishes this one (keeping the branch); Discard and finish
/// asks again, then finishes and deletes the branch; Keep open leaves it as it is.
#[tokio::test]
async fn commits_pushed_after_a_merge_ask_in_the_inbox() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;

    let (t, branch, item) = merged_then_pushed(&env, &gh, "follow").await;
    let v = get_ticket(&env, &t).await;
    assert_eq!(v["attempts"][0]["state"], "active", "not merged: a commit isn't in the PR");
    let question = item["body"]["question"].as_str().unwrap_or_default().to_string();
    let key = v["key"].as_str().unwrap();
    assert!(question.starts_with(&format!("{key}: 1 commit pushed after PR #12 merged: ")), "{item}");
    assert!(question.contains("build"), "{question}");
    // The agent is told too (with no session here, it is recorded as untold).
    let run = &runs_of(&env, &t, "Commits pushed after a merge").await[0];
    let tell = run["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "tell").unwrap();
    assert!(tell["input"]["text"].as_str().unwrap().contains("before you pushed 1 commit"), "{tell}");
    // Checking again doesn't ask again.
    pdt_api::check_merges(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(inbox(&env).await.iter().filter(|i| i["ticketId"] == t.as_str()).count(), 1);

    answer(&env, &item, json!({"choice": "open_follow_up"})).await;
    let done = until_done(&env, &t).await;
    assert_eq!(done["attempts"][0]["state"], "merged");
    assert!(branch_exists(&env, &branch), "the follow-up cherry-picks from it");
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}&stage=todo", env.project), None).await;
    let follow =
        list["items"].as_array().unwrap().iter().find(|i| i["title"].as_str().unwrap().starts_with("Follow-up to"));
    let follow = get_ticket(&env, follow.expect("a follow-up ticket")["id"].as_str().unwrap()).await;
    assert!(follow["description"].as_str().unwrap().contains("git cherry-pick"), "{follow}");

    let (t, branch, item) = merged_then_pushed(&env, &gh, "drop").await;
    answer(&env, &item, json!({"choice": "discard_and_finish"})).await;
    let confirm = tick_until(&env, "the confirmation", || async {
        inbox(&env).await.into_iter().find(|i| i["ticketId"] == t.as_str() && i["id"] != item["id"])
    })
    .await;
    assert!(confirm["body"]["question"].as_str().unwrap().starts_with("Discard "), "{confirm}");
    assert!(branch_exists(&env, &branch));
    answer(&env, &confirm, json!({"choice": "discard"})).await;
    until_done(&env, &t).await;
    assert!(!branch_exists(&env, &branch), "the commits are dropped");

    let (t, _, item) = merged_then_pushed(&env, &gh, "keep").await;
    answer(&env, &item, json!({"choice": "keep_open"})).await;
    tick_until(&env, "the item closed", || async {
        inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()).then_some(Value::Null)
    })
    .await;
    let v = get_ticket(&env, &t).await;
    assert_eq!((v["stage"].as_str(), v["attempts"][0]["state"].as_str()), (Some("in_progress"), Some("active")));
}

/// PDT-151: a CI event logged before the wait began (between the merge's read and the wait, so not marked
/// `mergeWaiting`) still ends the wait: it is on the head the merge was refused at.
#[tokio::test]
async fn a_ci_event_logged_before_the_wait_still_ends_it() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"], "codeBy": "user"}),
        &[("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change it", &["agentic"]).await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    // The run has started (it looks back only to its start) and waits at Code, the user's.
    let code = inbox_item(&env, "manual", &t).await;
    // CI passed and was logged (no merge waiting then); a re-run is going when the merge reads it.
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let (_, logged) = call(&env.app, "GET", "/api/events?type=ci.run_passed", None).await;
    let early = logged.as_array().unwrap()[0].clone();
    assert_eq!(early["payload"]["mergeWaiting"], false, "{early}");
    let mut rerun = ci_run(1, &branch, CiState::Running);
    rerun.attempt = 2;
    gh.set_runs(&branch, vec![rerun]);
    answer(&env, &code, json!({"choice": "outcome:submitted"})).await;
    let took = tick_until(&env, "the wait taking the earlier event", || async {
        let runs = runs_of(&env, &t, "Pipeline").await;
        let steps = runs.first()?["steps"].as_array()?.clone();
        steps.into_iter().find(|s| s["nodeId"] == "wait_ci" && s["port"] == "event")
    })
    .await;
    assert_eq!(took["output"]["eventId"], early["id"], "{took}");
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();
    assert_eq!(merges(), 0, "the merge read CI again: still running");
}

/// PDT-151: in a project with a CI integration, green GitHub Actions alone don't pass while it hasn't
/// reported on the head; and a read from GitHub that fails is an error, not a wait.
#[tokio::test]
async fn a_ci_integration_not_yet_on_the_head_and_an_unreadable_github_hold_the_merge() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "ci", "kind": "http", "url": "http://127.0.0.1:1"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    settings(&env, json!({"integrations": {"fakeci": {"connection": "ci"}}})).await;
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ci_pending")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Fake CI hasn't reported"), "{v}");

    *gh.error.lock().unwrap() = Some("gh: signed out".into());
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ci_unreadable")), "{v}");
    assert!(!gh.calls.lock().unwrap().iter().any(|c| c.starts_with("merge o/r 12")), "nothing merged");
}

/// PDT-176: on a forge that doesn't hold merges for required checks, an agent's merge waits for green CI even where the
/// project doesn't ask it to; on one without merge queues, a repository's queue setting is ignored and the merge names
/// its strategy. A person's merge doesn't wait.
#[tokio::test]
async fn a_forge_without_required_checks_or_queues_still_merges_safely() {
    let caps = pdt_forge::Capabilities { required_checks: false, merge_queue: false, ..pdt_forge::Capabilities::ALL };
    let gh = std::sync::Arc::new(FakeForge::with(caps));
    *gh.settings.lock().unwrap() = Some(MergeSettings { squash: true, merge: false, rebase: false, queue: true });
    let env = setup_with(Some(gh.clone())).await;
    settings(&env, json!({"requireGreenCi": false})).await;
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, v) = call(&env.app, "GET", &format!("/api/attempts/{id}/merge"), None).await;
    assert_eq!((s, &v["pr"]["queue"]), (StatusCode::OK, &json!(false)), "no merge queue on this forge: {v}");
    let merges =
        || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).cloned().collect::<Vec<_>>();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ci_pending")), "{v}");
    assert!(merges().is_empty(), "nothing merged while CI runs");

    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(merges(), ["merge o/r 12 squash"], "the strategy is named: no queue picks it");
    assert!(!gh.calls.lock().unwrap().iter().any(|c| c.starts_with("queue")), "no merge-queue runs read");
}

/// PDT-159: moving an in-progress ticket back to To do stops its work: the Pipeline's run is cancelled, the Coder's
/// session stops and the attempt is abandoned. An abandoned attempt doesn't hold the ticket back: once it is clear,
/// auto-start takes it again.
#[tokio::test]
async fn moving_a_started_ticket_back_to_todo_stops_its_work_and_auto_start_takes_it_again() {
    let env = setup().await;
    // The Coder never submits: it is still at work when the ticket moves.
    pipeline(&env, json!({}), &[("review", "approved"), ("qa", "pass")]).await;
    settings(&env, json!({"autoStart": "todo", "startDelaySeconds": 0})).await;
    let t = todo(&env, "Started by mistake").await;
    let coder = step_session(&env, "code").await;
    assert_eq!(session_status(&env, &coder).await, "running");
    assert_eq!(get_ticket(&env, &t).await["stage"], "in_progress");

    // Blocked first, so auto-start leaves it until the blocker closes.
    let blocker = ticket(&env, "Blocker").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{blocker}/links"), Some(json!({"to": t, "kind": "blocks"})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "todo"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["stage"], "todo");
    assert_eq!(v["attempts"][0]["state"], "abandoned", "{v}");
    let runs = runs_of(&env, &t, "Pipeline").await;
    assert!(!runs.is_empty() && runs.iter().all(|r| r["status"] == "cancelled"), "{runs:?}");
    assert_ne!(session_status(&env, &coder).await, "running");
    assert!(env.state.store.active_agent_steps().await.unwrap().iter().all(|a| a.ticket_id.as_deref() != Some(&t)));
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
        pdt_api::schedule_tickets(&env.state).await;
    }
    assert_eq!(get_ticket(&env, &t).await["stage"], "todo", "still blocked");

    // Clear: auto-start takes it again, with a new attempt.
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{blocker}"), Some(json!({"stage": "done"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let v = tick_until(&env, "the ticket started again", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "in_progress").then_some(v)
    })
    .await;
    let states: Vec<&Value> = v["attempts"].as_array().unwrap().iter().map(|a| &a["state"]).collect();
    assert!(states.contains(&&json!("active")) && states.contains(&&json!("abandoned")), "{v}");
}

/// PDT-159: Abandon stops the attempt's work too, and `POST /tickets/{id}/stop` with `removeWorktree` also removes
/// the worktree, so the next start begins clean.
#[tokio::test]
async fn abandon_and_stop_end_the_run_and_sessions_and_stop_can_remove_the_worktree() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("review", "approved"), ("qa", "pass")]).await;
    let t = ticket(&env, "Abandoned").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let coder = step_session(&env, "code").await;
    let attempt = get_ticket(&env, &t).await["attempts"][0]["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/abandon"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["attempts"][0]["state"], "abandoned");
    assert_eq!(v["stage"], "in_progress", "Abandon leaves the stage alone");
    assert!(runs_of(&env, &t, "Pipeline").await.iter().all(|r| r["status"] == "cancelled"));
    assert_ne!(session_status(&env, &coder).await, "running");

    // Stop: an in-progress ticket goes back to To do; the worktree the abandoned attempt left goes, changes and all.
    let wt = v["attempts"][0]["worktreeId"].as_str().unwrap().to_string();
    let (_, w) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.project), None).await;
    let path =
        w.as_array().unwrap().iter().find(|x| x["id"] == wt.as_str()).unwrap()["path"].as_str().unwrap().to_string();
    std::fs::write(Path::new(&path).join("left-over.txt"), "x").unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{t}/stop"), Some(json!({"removeWorktree": true}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["stage"].as_str(), r["abandoned"].is_null(), r["worktreeRemoved"].as_bool()),
        (Some("todo"), true, Some(true)),
        "{r}"
    );
    assert!(!Path::new(&path).exists());
    let (s, r) = call(&env.app, "POST", &format!("/api/tickets/{t}/stop"), Some(json!({"stage": "done"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{r}");
}

/// PDT-161: a CI-failed notice that comes while a QA-fail pass waits for the Coder goes in with that pass's message,
/// after QA's findings, not ahead of it as a message of its own.
#[tokio::test]
async fn a_ci_failure_during_a_qa_fail_pass_comes_after_qas_findings_in_its_message() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    // Another conversation, to hold the host's one slot later.
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "claude", "mode": "conversation",
                      "cols": 80, "rows": 24});
    let (s, created) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    let busy = created["id"].as_str().unwrap().to_string();
    pipeline(&env, json!({"qaBy": "user"}), &[("code", "submitted"), ("review", "approved")]).await;
    let t = ticket(&env, "Make it fast").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    let qa = inbox_item(&env, "manual", &t).await;
    let sessions = get_ticket(&env, &t).await["sessions"].as_array().unwrap().clone();
    let coder = sessions.iter().find(|s| s["persona"] == "Coder").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(qa["title"], "Your turn: QA");

    // The Coder can't take a turn yet: the one slot is busy. QA fails, then CI does.
    let (s, v) = call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxAgentTurns": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{busy}/messages"), Some(json!({"text": "slow"}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    tick_until(&env, "the other session working", || async {
        let s = call(&env.app, "GET", &format!("/api/sessions/{busy}"), None).await.1;
        (s["activity"] == "working").then_some(s)
    })
    .await;
    answer(&env, &qa, json!({"choice": "outcome:fail", "text": "p95 is 900 ms; the target is 200 ms."})).await;
    let mut red = ci_run(1, &branch, CiState::Failed);
    red.failed_jobs = vec!["test (linux)".into()];
    gh.set_runs(&branch, vec![red]);
    gh.set_jobs(1, &["test (linux)"]);
    pdt_api::refresh_prs(&env.state).await;
    tick_until(&env, "the notice waiting for the coder", || async {
        let waiting = env.state.store.waiting_session_messages().await.unwrap();
        let pass = waiting.iter().any(|m| m.dedupe.as_deref().is_some_and(|d| d.starts_with("agent:")));
        let notice = waiting.iter().any(|m| m.text.contains("CI failed"));
        (pass && notice).then_some(Value::Null)
    })
    .await;
    assert!(!agent_says(&env, &coder).await.iter().any(|m| m.contains("CI failed")));

    // The slot frees: one message, QA's findings first, the notice after them as part of the pass.
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{busy}/interrupt"), None).await;
    assert!(s.is_success(), "{v}");
    let said = tick_until(&env, "the pass reaching the coder", || async {
        agent_says(&env, &coder).await.into_iter().find(|m| m.contains("another pass")).map(Value::from)
    })
    .await;
    let said = said.as_str().unwrap();
    let findings = said.find("The user answered `fail`: p95 is 900 ms").unwrap_or_else(|| panic!("{said}"));
    let also = said.find("Also for this pass").unwrap_or_else(|| panic!("{said}"));
    let ci = said.find("CI failed on PR #12").unwrap_or_else(|| panic!("{said}"));
    assert!(findings < also && also < ci, "{said}");
    let all = agent_says(&env, &coder).await;
    assert_eq!(all.iter().filter(|m| m.contains("CI failed")).count(), 1, "{all:?}");
}

/// The mcp result a session's agent said for its latest `submit_outcome`.
async fn submitted_reply(env: &Env, session: &str, before: usize) -> String {
    tick_until(env, "the submit_outcome reply", || async {
        let said = agent_says(env, session).await;
        said.into_iter()
            .filter(|m| m.starts_with("mcp submit_outcome") || m.starts_with("mcp error"))
            .nth(before)
            .map(Value::from)
    })
    .await
    .as_str()
    .unwrap()
    .to_string()
}

/// PDT-161: `submitted` is refused while the worktree has uncommitted changes to tracked files or commits its upstream
/// lacks, and taken once it is clean and pushed. Untracked files don't count.
#[tokio::test]
async fn submitted_needs_a_clean_pushed_worktree() {
    let env = setup().await;
    let remote = env._tmp.path().join("remote.git");
    git(env._tmp.path(), &["init", "-q", "--bare", &remote.to_string_lossy()]);
    git(&env.repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
    git(&env.repo, &["push", "-q", "origin", "main"]);
    pipeline(&env, json!({"reviewBy": "user", "qaBy": "user"}), &[]).await;
    let t = ticket(&env, "Push it").await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let wt = env.repo.join(".worktrees").join(&branch);
    let coder = step_session(&env, "code").await;

    std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
    std::fs::write(wt.join("scratch.txt"), "untracked\n").unwrap();
    submit_in(&env, &coder, "submitted").await;
    let r = submitted_reply(&env, &coder, 0).await;
    assert!(
        r.contains("`submitted` is refused") && r.contains("uncommitted changes to tracked files (`a.txt`)"),
        "{r}"
    );
    assert!(!r.contains("scratch.txt"), "{r}");

    // Committed, but the branch was never pushed.
    git(&wt, &["commit", "-q", "-am", "change"]);
    submit_in(&env, &coder, "submitted").await;
    let r = submitted_reply(&env, &coder, 1).await;
    assert!(r.contains("still has a branch that isn't pushed (it is not on `origin`)"), "{r}");

    git(&wt, &["push", "-q", "-u", "origin", &branch]);
    std::fs::write(wt.join("a.txt"), "again\n").unwrap();
    git(&wt, &["commit", "-q", "-am", "again"]);
    submit_in(&env, &coder, "submitted").await;
    let r = submitted_reply(&env, &coder, 2).await;
    assert!(r.contains(&format!("1 commit not pushed to `origin/{branch}`")), "{r}");
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "code");

    git(&wt, &["push", "-q"]);
    submit_in(&env, &coder, "submitted").await;
    let r = submitted_reply(&env, &coder, 3).await;
    assert!(r.starts_with("mcp submit_outcome:"), "{r}");
    let review = inbox_item(&env, "manual", &t).await;
    assert_eq!(review["title"], "Your turn: Review");
}
