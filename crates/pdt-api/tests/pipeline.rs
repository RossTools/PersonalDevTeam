//! The Pipeline end to end: the Pipeline template drives a ticket from Refine to Done with `fake-agent` personas
//! submitting outcomes through pdt-mcp; project policies, auto-start, manual stages, Board stages and the host's cap
//! on agents running a turn.

use crate::common;

use axum::http::StatusCode;
use common::{call, call_as_ui, git, mark_pushed};
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
    let mut state = common::app_state(store, tmp.path());
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    if let Some(gh) = gh {
        state.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::testing::fake_github(&gh))]);
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
    // The fake agents here hand in outcomes only when a test tells them to, from the session it holds: "When an agent
    // is stuck" would nudge and restart them meanwhile (its tests: stuck_agents.rs).
    common::stuck_agents_off(&app).await;
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

    fn ci_provider(
        &self,
        _st: &pdt_api::sdk::AppState,
        _project_id: &str,
        settings: &pdt_store::ProjectSettings,
    ) -> Option<std::sync::Arc<dyn pdt_forge::CiProvider>> {
        let caps = pdt_forge::CiCapabilities { branches: true, ..pdt_forge::CiCapabilities::NONE };
        let ci = pdt_forge::testing::FakeCi::new("fakeci", "Fake CI", caps, pdt_forge::CiConfig::Known(true));
        settings.integrations.contains_key("fakeci").then(|| std::sync::Arc::new(ci) as _)
    }
}

async fn ticket(env: &Env, title: &str) -> String {
    let (s, t) =
        call_as_ui(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn labelled(env: &Env, title: &str, labels: &[&str]) -> String {
    let (s, t) = call_as_ui(
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
        common::pick_up(&env.state).await;
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

    // Refined: To do (the template's default), out of the pipeline stage; then auto-start takes it. The change is
    // committed on its branch at once (what a real Coder would do), so it has something to merge (PDT-354).
    let v = tick_until(&env, "the ticket started", || async {
        let v = get_ticket(&env, &t).await;
        (v["stage"] == "in_progress").then_some(v)
    })
    .await;
    let worktree = env.repo.join(".worktrees").join(v["attempts"][0]["branch"].as_str().unwrap());
    commit_in(&worktree);
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
    let (s, t) = call_as_ui(
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
        common::pick_up(&env.state).await;
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

    // What an outsider wrote, its title too, comes fenced (PDT-415).
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{session}/messages"),
        Some(json!({"text": "mcp: get_ticket {}"})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let got = tick_until(&env, "get_ticket answered", || async {
        agent_says(&env, &session).await.into_iter().find(|m| m.starts_with("mcp get_ticket")).map(Value::String)
    })
    .await;
    let got = got.as_str().unwrap();
    assert!(got.contains("\"title\": \"<untrusted id=\\\"") && got.contains("\\nFrom an issue\\n"), "{got}");
    assert!(got.contains("its title and description"), "{got}");

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
    common::pick_up(&env.state).await;
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
    common::pick_up(&env.state).await;
    assert_eq!(get_ticket(&env, &c).await["stage"], "todo", "still blocked by B");

    // 0: at once.
    settings(&env, json!({"startDelaySeconds": 0})).await;
    let d = todo(&env, "D").await;
    assert_eq!(get_ticket(&env, &d).await["autoStartAt"], Value::Null);
    common::pick_up(&env.state).await;
    assert_eq!(get_ticket(&env, &d).await["stage"], "in_progress");
}

async fn todo_at(env: &Env, title: &str, priority: &str) -> String {
    let (s, t) = call_as_ui(
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
    // The tickets in the order they started (the event log's order).
    let started_in_order = || async {
        let (_, v) = call(&env.app, "GET", "/api/events?type=attempt.started&limit=100", None).await;
        let mut ids: Vec<String> =
            v.as_array().unwrap().iter().map(|e| e["ticketId"].as_str().unwrap().to_string()).collect();
        ids.reverse();
        ids
    };
    // A new To do ticket goes to the top: the medium ones rank above the high one, `medium` first. Since PDT-327 the
    // pickup starts every ticket it may in one pass (the old loop started one per project every 10 s), in the same
    // order: the high-priority ticket first, then rank within a priority.
    let high = todo_at(&env, "High", "high").await;
    let medium_later = todo_at(&env, "Medium later", "medium").await;
    let medium = todo_at(&env, "Medium", "medium").await;
    common::pick_up(&env.state).await;
    assert_eq!(started_in_order().await, [high.clone(), medium.clone(), medium_later.clone()]);

    // By rank: today's order, whatever the priority.
    settings(&env, json!({"autoStartOrder": "rank"})).await;
    let (_, v) = call(&env.app, "GET", &format!("/api/projects/{}/settings", env.project), None).await;
    assert_eq!(v["policy"]["autoStartOrder"], "rank");
    let urgent = todo_at(&env, "Urgent", "urgent").await;
    let low = todo_at(&env, "Low", "low").await;
    common::pick_up(&env.state).await;
    assert_eq!(started_in_order().await[3..], [low, urgent], "the first in rank first");
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
               "requireGreenCi": true, "triageApproval": "inbox", "resolveConflicts": "merge_time", "hotAreas": [],
               "untaggedAlongsideHot": true})
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

    // Review is the user's: a Your turn item with the outcomes. The change is on the branch by then.
    let mine = inbox_item(&env, "manual", &t).await;
    let attempt = started["attempt"]["id"].as_str().unwrap();
    let worktree = env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    commit_in(&worktree);
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "review");
    assert_eq!(get_ticket(&env, &t).await["agent"]["state"], "yours");
    answer(&env, &mine, json!({"choice": "outcome:approved", "text": "Looks right."})).await;
    let accept = inbox_item(&env, "gate", &t).await;

    // Before the user accepts, an automated merge is refused by the policy.
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

/// PDT-341: "Ticket comments to the agent" sends the PM's comments as well as the user's, each once, labelled, with
/// the local paths of the files they attach; the ticket's own agents' comments and a workflow's notes stay on the
/// ticket.
#[tokio::test]
async fn the_pms_and_the_users_comments_reach_the_coder_with_their_attachments() {
    use pdt_core::ticket::{Actor, AgentBy, By};
    let env = setup().await;
    let t = ticket(&env, "Colour the button").await;
    pipeline(&env, json!({"reviewBy": "user", "qaBy": "user"}), &[("code", "submitted")]).await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    inbox_item(&env, "manual", &t).await;
    let coder = get_ticket(&env, &t).await["sessions"][0]["id"].as_str().unwrap().to_string();
    let (s, _, shot) = common::call_raw(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/attachments?name=shot.png"),
        b"\x89PNG\r\n\x1a\nrest".to_vec(),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let shot: Value = serde_json::from_slice(&shot).unwrap();
    let agent = |persona: &str| {
        By::agent(AgentBy {
            session_id: format!("{persona}-session"),
            project_id: Some(env.project.clone()),
            persona: Some(persona.into()),
            from_ticket: None,
            from_ticket_id: None,
        })
    };
    let store = &env.state.store;
    let body =
        format!("The cache is wrong.\n\n![shot.png](/api/tickets/{t}/attachments/{})", shot["id"].as_str().unwrap());
    store.add_comment_by(&t, &body, agent("PM"), None, 1).await.unwrap();
    store.add_comment_by(&t, "Coder: done the cache.", agent("Coder"), None, 2).await.unwrap();
    store.add_comment_by(&t, "PM via a workflow.", agent("PM"), Some(json!({"workflowId": "w1"})), 3).await.unwrap();
    store
        .add_comment_by(&t, "A note from a workflow.", Actor::System, Some(json!({"workflowId": "w1"})), 4)
        .await
        .unwrap();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{t}/comments"),
        Some(json!({"body": "Make it blue.", "by": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    // Messages waiting for the same turn may arrive together.
    let told = || async { agent_says(&env, &coder).await.join("\n") };
    tick_until(&env, "both comments reaching the coder", || async {
        (told().await.matches("New comment on").count() >= 2).then_some(json!(null))
    })
    .await;
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
    }
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
    let runs = runs.as_array().unwrap().iter().filter(|r| r["workflowName"] == "Ticket comments to the agent").count();
    let told = told().await;
    assert_eq!((runs, told.matches("New comment on").count()), (2, 2), "each once, none of the others: {told}");
    for not in ["Coder: done", "PM via a workflow", "A note from a workflow"] {
        assert!(!told.contains(not), "{told}");
    }
    let (pm, user) = told.split_once("from the user:").unwrap_or_else(|| panic!("{told}"));
    assert!(pm.contains("from the PM:\n\nThe cache is wrong."), "{told}");
    let path = env
        .state
        .data_dir
        .join("tickets")
        .join(&t)
        .join("attachments")
        .join(format!("{}.png", shot["id"].as_str().unwrap()));
    assert!(pm.contains(&format!("- shot.png: {}", path.display())), "{told}");
    assert!(path.is_file());
    assert!(user.contains("Make it blue.") && !user.contains("Attached"), "{told}");
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

/// Starts `t` with no session and no commits on its branch (a ticket that needs no change); the attempt.
async fn start_without_changes(env: &Env, t: &str) -> Value {
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    started["attempt"].clone()
}

/// The attempt finished without a merge (PDT-354): nothing merged into main, its sessions stopped, its worktree and
/// branch gone. `main`: how many commits main has then (finishing pulls it).
async fn finished_without_a_merge(env: &Env, t: &str, attempt: &Value, main: usize) {
    let done = until_done(env, t).await;
    assert_eq!(done["pipelineStage"], Value::Null);
    let branch = attempt["branch"].as_str().unwrap();
    assert!(!branch_exists(env, branch), "the branch is deleted");
    assert!(!env.repo.join(".worktrees").join(branch).exists(), "the worktree is removed");
    let ours = done["attempts"].as_array().unwrap().iter().find(|a| a["id"] == attempt["id"]).unwrap().clone();
    assert_ne!(ours["state"], "active", "{ours}");
    assert_eq!(ours["pr"], Value::Null, "no pull request: {ours}");
    let (_, sessions) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
    let running: Vec<&Value> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["attemptId"] == attempt["id"] && s["status"] == "running")
        .collect();
    assert!(running.is_empty(), "its sessions stopped: {running:?}");
    let log = std::process::Command::new("git").current_dir(&env.repo).args(["rev-list", "--count", "main"]).output();
    assert_eq!(String::from_utf8_lossy(&log.unwrap().stdout).trim(), main.to_string(), "nothing merged into main");
}

/// PDT-354: an approved attempt whose branch is the same as its base has nothing to merge. An `agentic` ticket goes
/// straight to Done; it never sits at Merge blocked.
#[tokio::test]
async fn an_agentic_ticket_with_no_changes_finishes_without_a_merge() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let t = labelled(&env, "Triage only", &["agentic"]).await;
    let attempt = start_without_changes(&env, &t).await;
    finished_without_a_merge(&env, &t, &attempt, 1).await;
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "no Inbox item for it");
    assert_eq!(outcomes(&timeline(&env, &t).await), ["submitted", "approved", "pass"]);
}

/// PDT-354: on a project whose local `main` isn't pulled, Start branches from `origin/main` (ahead of it); origin's
/// newer commits are not the branch's own, so it still has nothing to merge.
#[tokio::test]
async fn a_ticket_with_no_changes_started_from_a_newer_origin_finishes_without_a_merge() {
    let env = setup().await;
    // origin/main is one commit ahead of the local main.
    let remote = env._tmp.path().join("remote.git");
    git(env._tmp.path(), &["init", "-q", "--bare", &remote.to_string_lossy()]);
    git(&env.repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
    std::fs::write(env.repo.join("merged.txt"), "merged on the forge\n").unwrap();
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-m", "merged elsewhere", "-q"]);
    git(&env.repo, &["push", "-q", "origin", "main"]);
    git(&env.repo, &["reset", "-q", "--hard", "HEAD~1"]);
    pipeline(&env, json!({}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let t = labelled(&env, "Triage only", &["agentic"]).await;
    let attempt = start_without_changes(&env, &t).await;
    let wt = env.repo.join(".worktrees").join(attempt["branch"].as_str().unwrap());
    assert!(wt.join("merged.txt").exists(), "the branch starts at origin/main");
    git(&wt, &["push", "-q", "-u", "origin", attempt["branch"].as_str().unwrap()]);
    finished_without_a_merge(&env, &t, &attempt, 2).await;
}

/// PDT-354: a ticket that isn't accepted automatically waits at Accept, saying it has no changes; accepted, it moves to
/// Done without a merge. Commits that came meanwhile merge as any other change.
#[tokio::test]
async fn a_ticket_with_no_changes_waits_at_accept_then_finishes_without_a_merge() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let t = ticket(&env, "Answer a question").await;
    let attempt = start_without_changes(&env, &t).await;
    let accept = inbox_item(&env, "gate", &t).await;
    assert_eq!(accept["group"], "accept");
    let title = accept["title"].as_str().unwrap();
    assert!(title.contains("passed with no changes to merge") && title.contains("code says submitted"), "{title}");
    assert_eq!(get_ticket(&env, &t).await["pipelineStage"], "accept");
    answer(&env, &accept, json!({"choice": "done"})).await;
    finished_without_a_merge(&env, &t, &attempt, 1).await;

    let late = ticket(&env, "Changed while it waited").await;
    let attempt = start_without_changes(&env, &late).await;
    let accept = inbox_item(&env, "gate", &late).await;
    commit_file(&env.repo.join(".worktrees").join(attempt["branch"].as_str().unwrap()), "late.txt");
    answer(&env, &accept, json!({"choice": "done"})).await;
    until_done(&env, &late).await;
    let log = std::process::Command::new("git")
        .current_dir(&env.repo)
        .args(["ls-tree", "--name-only", "main"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&log.stdout).contains("late.txt"), "the late commit merged");
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
    // The Pipeline's merge wait, a day here instead of three (PDT-371).
    pipeline(
        &env,
        json!({"accept": "auto", "merge": "person", "mergeWait": "1d"}),
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
    let one_day = 24 * 60 * 60 * 1000;
    let wake = wait["wakeAt"].as_i64().expect("the wait has a time limit");
    assert!((wake - wait["startedAt"].as_i64().unwrap() - one_day).abs() < 60_000, "{wait}");
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
/// The `merge_update` events on the ticket's timeline: how each update went (PDT-328).
async fn merge_updates(env: &Env, t: &str) -> Vec<Value> {
    timeline(env, t).await["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "merge_update")
        .map(|e| e["data"].clone())
        .collect()
}

fn hows(updates: &[Value]) -> Vec<&str> {
    updates.iter().map(|u| u["how"].as_str().unwrap()).collect()
}

/// PDT-328: at merge time PDT merges main in itself; a real conflict goes to the Coder once, with the files, the hunks
/// and what merged into main since, and the merge left in progress. Once the Coder pushes, PDT finds the branch up to
/// date and merges it, and the timeline says the agent resolved it.
#[tokio::test]
async fn a_real_conflict_at_merge_time_goes_to_the_coder_once_with_its_hunks() {
    let env = setup().await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
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
    git(&env.repo, &["commit", "-qam", "PX-9: Main's change"]);

    let resolve = step_session(&env, "resolve").await;
    let coder = get_ticket(&env, &t).await["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"].as_str().unwrap().starts_with("Coder"))
        .unwrap()["id"]
        .clone();
    assert_eq!(coder, json!(resolve), "the Coder's own session");
    let said = tick_until(&env, "the conflicts reaching the coder", || async {
        agent_says(&env, &resolve).await.into_iter().find(|m| m.contains("conflicts")).map(Value::from)
    })
    .await;
    let said = said.as_str().unwrap();
    for want in ["- a.txt", "<<<<<<<", "- PX-9: Main's change", "merged `main`"] {
        assert!(said.contains(want), "{want} in {said}");
    }
    let v = get_ticket(&env, &t).await;
    assert_eq!(
        (v["pipelineStage"].as_str(), v["attempts"][0]["mergeUpdate"].as_str()),
        (Some("merge"), Some("resolving"))
    );
    assert!(
        worktree.join(".git").exists() && std::fs::read_to_string(worktree.join("a.txt")).unwrap().contains("<<<<<<<")
    );
    assert_eq!(hows(&merge_updates(&env, &t).await), ["conflict"]);
    assert_eq!(merge_updates(&env, &t).await[0]["files"], json!(["a.txt"]));

    // The Coder resolves the conflict in the merge PDT left in progress, commits and submits.
    std::fs::write(worktree.join("a.txt"), "branch\n").unwrap();
    git(&worktree, &["add", "a.txt"]);
    git(&worktree, &["commit", "-q", "--no-edit"]);
    mark_pushed(&worktree);
    submit_in(&env, &resolve, "submitted").await;
    let done = until_done(&env, &t).await;
    assert_eq!(outcomes(&timeline(&env, &t).await), ["submitted", "approved", "pass", "submitted"]);
    assert_eq!(hows(&merge_updates(&env, &t).await), ["conflict", "agent"]);
    assert_eq!(done["attempts"][0]["mergeUpdate"], Value::Null);
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "nobody was asked");
    // Git on Windows may check the merged file out with CRLF.
    let merged = std::fs::read_to_string(env.repo.join("a.txt")).unwrap().replace("\r\n", "\n");
    assert_eq!(merged, "branch\n", "merged into main");
}

/// PDT-328: main moved without conflicting. PDT merges it in and pushes the branch once, with no agent, then merges.
#[tokio::test]
async fn a_clean_update_at_merge_time_needs_no_agent_and_pushes_once() {
    let env = setup().await;
    // A plain git remote: the branch was pushed, and main moved there.
    let remote = env._tmp.path().join("origin.git");
    git(env._tmp.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(&env.repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&env.repo, &["push", "-q", "origin", "main"]);
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Add b", &["agentic"]).await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let worktree = env.repo.join(".worktrees").join(&branch);
    std::fs::write(worktree.join("b.txt"), "built\n").unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-qm", "build"]);
    git(&worktree, &["push", "-q", "origin", &branch]);
    let other = env._tmp.path().join("other");
    git(env._tmp.path(), &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    git(&other, &["config", "user.name", "t"]);
    git(&other, &["config", "user.email", "t@t"]);
    std::fs::write(other.join("m.txt"), "main\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-qm", "PX-8: Main's change"]);
    git(&other, &["push", "-q", "origin", "main"]);

    until_done(&env, &t).await;
    assert_eq!(outcomes(&timeline(&env, &t).await), ["submitted", "approved", "pass"], "no agent after QA");
    let updates = merge_updates(&env, &t).await;
    assert_eq!(hows(&updates), ["clean"], "{updates:?}");
    assert_eq!(updates[0]["base"], "origin/main");
    // The update was pushed: the remote branch has main's change.
    let rev = |r: &str| {
        let out = std::process::Command::new("git").current_dir(&remote).args(["rev-parse", r]).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let (pushed, main_head) = (rev(&format!("refs/heads/{branch}")), rev("main"));
    let r = std::process::Command::new("git")
        .current_dir(&remote)
        .args(["merge-base", "--is-ancestor", &main_head, &pushed])
        .status()
        .unwrap();
    assert!(r.success(), "the pushed branch has main");
    assert!(env.repo.join("m.txt").exists() && env.repo.join("b.txt").exists(), "merged into main");
}

/// PDT-328: only a generated file conflicts. PDT takes main's side, runs the project's regeneration command and
/// commits, with no agent.
#[tokio::test]
async fn a_conflict_only_in_generated_files_is_regenerated_without_an_agent() {
    let env = setup().await;
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    std::fs::write(
        env.repo.join(".pdt/project.toml"),
        "[[merge.regenerate]]\npaths = [\"gen/**\"]\nrun = \"cd gen && echo regenerated> out.txt\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(env.repo.join("gen")).unwrap();
    std::fs::write(env.repo.join("gen/out.txt"), "one\n").unwrap();
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-qm", "regeneration rules"]);
    common::approve_commands(&env.app, &env.project, None).await;
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change gen", &["agentic"]).await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let worktree = env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    std::fs::write(worktree.join("gen/out.txt"), "branch\n").unwrap();
    std::fs::write(worktree.join("b.txt"), "built\n").unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-qm", "build"]);
    std::fs::write(env.repo.join("gen/out.txt"), "main\n").unwrap();
    git(&env.repo, &["commit", "-qam", "main regenerates"]);

    until_done(&env, &t).await;
    assert_eq!(outcomes(&timeline(&env, &t).await), ["submitted", "approved", "pass"], "no agent after QA");
    let updates = merge_updates(&env, &t).await;
    assert_eq!(hows(&updates), ["regenerated"], "{updates:?}");
    assert_eq!(updates[0]["regenerated"], json!(["gen/out.txt"]));
    let generated = std::fs::read_to_string(env.repo.join("gen/out.txt")).unwrap();
    assert_eq!(generated.trim(), "regenerated", "merged into main as regenerated");
    assert!(env.repo.join("b.txt").exists());
}

/// PDT-405: a regeneration rule runs only from the base branch's committed file, and only once the user approved its
/// command. Unapproved, the files are a conflict for the agent and the Inbox says the rule waits; a rule the branch adds
/// to its own file is never run.
#[tokio::test]
async fn a_regeneration_rule_runs_only_from_the_base_and_once_approved() {
    let env = setup().await;
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    std::fs::write(
        env.repo.join(".pdt/project.toml"),
        "[[merge.regenerate]]\npaths = [\"gen/**\"]\nrun = \"cd gen && echo regenerated> out.txt\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(env.repo.join("gen")).unwrap();
    std::fs::write(env.repo.join("gen/out.txt"), "one\n").unwrap();
    std::fs::write(env.repo.join("a.txt"), "one\n").unwrap();
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-qm", "regeneration rules"]);
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"]}),
        &[("code", "submitted"), ("review", "approved"), ("qa", "pass")],
    )
    .await;
    let t = labelled(&env, "Change gen", &["agentic"]).await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let worktree = env.repo.join(".worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    // The branch adds a rule of its own for a.txt, which would write a marker if it ran.
    std::fs::write(
        worktree.join(".pdt/project.toml"),
        "[[merge.regenerate]]\npaths = [\"gen/**\"]\nrun = \"cd gen && echo regenerated> out.txt\"\n\n\
         [[merge.regenerate]]\npaths = [\"a.txt\"]\nrun = \"echo ran> marker.txt\"\n",
    )
    .unwrap();
    std::fs::write(worktree.join("gen/out.txt"), "branch\n").unwrap();
    std::fs::write(worktree.join("a.txt"), "branch\n").unwrap();
    git(&worktree, &["commit", "-qam", "build"]);
    std::fs::write(env.repo.join("gen/out.txt"), "main\n").unwrap();
    std::fs::write(env.repo.join("a.txt"), "main\n").unwrap();
    git(&env.repo, &["commit", "-qam", "main moves"]);

    let updates = tick_until(&env, "a merge update", || async {
        let u = merge_updates(&env, &t).await;
        (!u.is_empty()).then(|| Value::from(u))
    })
    .await;
    let updates = updates.as_array().unwrap();
    assert_eq!(hows(updates), ["conflict"], "{updates:?}");
    let mut files: Vec<&str> = updates[0]["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    files.sort();
    assert_eq!(files, ["a.txt", "gen/out.txt"], "the unapproved rule's files are the agent's");
    assert!(!worktree.join("marker.txt").exists(), "the branch's own rule never ran");
    assert!(
        !worktree.join("gen/out.txt").exists()
            || !std::fs::read_to_string(worktree.join("gen/out.txt")).unwrap().contains("regenerated")
    );
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert!(inbox.to_string().contains("cd gen && echo regenerated> out.txt"), "the Inbox says it waits: {inbox}");
    assert!(!inbox.to_string().contains("marker.txt"), "the branch's rule is not offered: {inbox}");
}

/// PDT-328: while the ticket is in code (review, QA or accept), a conflict is recorded and shown, not sent to its
/// agent; a project that resolves conflicts immediately tells the agent at once.
#[tokio::test]
async fn a_conflict_before_the_merge_step_waits_for_merge_time_unless_the_project_says_immediately() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    // The Coder never submits: the ticket stays in code.
    pipeline(&env, json!({}), &[]).await;
    let t = ticket(&env, "Long work").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    let coder = step_session(&env, "code").await;
    let notices = |run: &Value| -> Vec<String> {
        run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| format!("{}:{}", s["nodeId"].as_str().unwrap(), s["port"].as_str().unwrap_or("-")))
            .collect()
    };
    let notice_run = |head: &'static str| {
        let env = &env;
        let t = t.clone();
        async move {
            tick_until(env, "PR notices on the conflict", || async {
                runs_of(env, &t, "PR notices")
                    .await
                    .into_iter()
                    .find(|r| r["trigger"]["headSha"] == head && r["status"] != "running" && r["status"] != "waiting")
            })
            .await
        }
    };
    let mut conflicting = pull_request(Mergeable::Conflicting);
    gh.set_pr(&branch, Some(conflicting.clone()));
    pdt_api::refresh_prs(&env.state).await;
    let run = notice_run("abcdef1234").await;
    assert_eq!(notices(&run), ["conflict:out", "defer:deferred", "later:-"], "{run}");
    let v = get_ticket(&env, &t).await;
    assert_eq!(
        (v["pipelineStage"].as_str(), v["attempts"][0]["mergeUpdate"].as_str()),
        (Some("code"), Some("deferred"))
    );
    let updates = merge_updates(&env, &t).await;
    assert_eq!((hows(&updates), updates[0]["pr"].as_u64()), (vec!["deferred"], Some(12)));
    let pr_events = timeline(&env, &t).await["events"].as_array().unwrap().iter().filter(|e| e["kind"] == "pr").count();
    assert_eq!(pr_events, 0, "the agent wasn't told");
    assert!(agent_says(&env, &coder).await.iter().all(|m| !m.contains("conflicts with main")));

    // No longer conflicting: the ticket shows nothing.
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(get_ticket(&env, &t).await["attempts"][0]["mergeUpdate"], Value::Null);

    // Immediately: the agent hears of the next conflict at once.
    settings(&env, json!({"resolveConflicts": "immediately"})).await;
    conflicting.head_sha = "fedcba9876".into();
    gh.set_pr(&branch, Some(conflicting));
    pdt_api::refresh_prs(&env.state).await;
    let run = notice_run("fedcba9876").await;
    assert_eq!(notices(&run)[..3], ["conflict:out", "defer:now", "asked:false"], "{run}");
    let told = timeline(&env, &t).await["events"].as_array().unwrap().iter().filter(|e| e["kind"] == "pr").count();
    assert_eq!(told, 1, "the agent was told");
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
            provider: "github_actions".into(),
            id: "8".into(),
            attempt: 1,
            name: "ci".into(),
            state: CiState::Passed,
            url: "https://github.com/o/r/actions/runs/8".into(),
            head_sha: "abcdef1234".into(),
            branch: branch.clone(),
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
        assert_eq!(nodes[..4], ["conflict", "defer", "asked", "busy"], "{run}");
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
        provider: "github_actions".into(),
        id: id.to_string(),
        attempt: 1,
        name: "ci".into(),
        state,
        url: format!("https://github.com/o/r/actions/runs/{id}"),
        head_sha: "abcdef1234".into(),
        branch: branch.into(),
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
    // The Pipeline's CI wait, two hours here instead of six (PDT-371).
    pipeline(
        &env,
        json!({"autoAcceptLabels": ["agentic"], "ciWait": "2h"}),
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
    let wake = waiting["wakeAt"].as_i64().expect("the wait has a time limit") - waiting["startedAt"].as_i64().unwrap();
    assert!((wake - 2 * 60 * 60 * 1000).abs() < 60_000, "{waiting}");
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
    // It names the project's CI providers, not one forge's CI.
    let why = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|e| e["body"].as_str().filter(|b| b.contains("without waiting for CI")).map(str::to_string));
    assert!(why.as_deref().is_some_and(|b| b.contains("the repository has no CI set up (")), "{why:?}");
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

/// PDT-242: `[ci] providers` picks the project's CI: listing GitHub Actions alone leaves the CI integration out of the
/// gate, and listing it too makes the merge wait for it.
#[tokio::test]
async fn ci_providers_listed_in_the_project_file_are_the_ones_the_merge_waits_for() {
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
    std::fs::create_dir_all(env.repo.join(".pdt")).unwrap();
    let toml = env.repo.join(".pdt/project.toml");
    std::fs::write(&toml, "[ci]\nproviders = [\"github_actions\"]\n").unwrap();
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    std::fs::write(&toml, "[ci]\nproviders = [\"fakeci\", \"github_actions\"]\n").unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ci_pending")), "both listed: {v}");
    assert!(v["message"].as_str().unwrap().contains("Fake CI hasn't reported"), "{v}");
    std::fs::write(&toml, "[ci]\nproviders = [\"github_actions\"]\n").unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!(s, StatusCode::OK, "Actions alone, and it passed: {v}");
    assert!(gh.calls.lock().unwrap().iter().any(|c| c.starts_with("merge o/r 12")));
}

/// PDT-176: on a forge that doesn't hold merges for required checks, an agent's merge waits for green CI even where the
/// project doesn't ask it to; on one without merge queues, a repository's queue setting is ignored and the merge names
/// its strategy. A person's merge doesn't wait.
#[tokio::test]
async fn a_forge_without_required_checks_or_queues_still_merges_safely() {
    let caps = pdt_forge::Capabilities { required_checks: false, merge_queue: false, ..pdt_forge::Capabilities::ALL };
    let gh = std::sync::Arc::new(FakeForge::with(caps));
    *gh.settings.lock().unwrap() =
        Some(MergeSettings { squash: true, merge: false, rebase: false, queue: true, auto_merge: true });
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
    assert!(!gh.ci.calls.lock().unwrap().iter().any(|c| c.starts_with("queue")), "no merge-queue runs read");
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
        common::pick_up(&env.state).await;
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

/// PDT-159: Abandon stops the attempt's work too (and keeps a worktree with changes, PDT-339), and
/// `POST /tickets/{id}/stop` with `removeWorktree` also removes the worktree, so the next start begins clean.
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
    let path = env.repo.join(".worktrees").join(get_ticket(&env, &t).await["attempts"][0]["branch"].as_str().unwrap());
    std::fs::write(path.join("left-over.txt"), "x").unwrap();
    let attempt = get_ticket(&env, &t).await["attempts"][0]["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/abandon"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["attempts"][0]["state"], "abandoned");
    assert_eq!(v["stage"], "in_progress", "Abandon leaves the stage alone");
    assert!(runs_of(&env, &t, "Pipeline").await.iter().all(|r| r["status"] == "cancelled"));
    assert_ne!(session_status(&env, &coder).await, "running");

    assert!(path.exists(), "the worktree with changes is kept");

    // Stop: an in-progress ticket goes back to To do; the worktree the abandoned attempt left goes, changes and all.
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

/// A GitHub repository with neither a merge queue nor auto-merge allowed (PDT-297).
fn no_auto_merge() -> std::sync::Arc<FakeForge> {
    let gh = std::sync::Arc::new(FakeForge::default());
    *gh.settings.lock().unwrap() =
        Some(MergeSettings { squash: true, merge: false, rebase: false, queue: false, auto_merge: false });
    gh
}

async fn events_of(env: &Env, kind: &str) -> Vec<Value> {
    call(&env.app, "GET", &format!("/api/events?type={kind}"), None).await.1.as_array().unwrap().clone()
}

/// PDT-297: on a repository without auto-merge, the user's Merge of a pull request whose CI passed merges it at once,
/// and the attempt ends merged.
#[tokio::test]
async fn without_auto_merge_a_green_pull_request_merges_at_once() {
    let gh = no_auto_merge();
    let env = setup_with(Some(gh.clone())).await;
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, p) = call(&env.app, "GET", &format!("/api/attempts/{id}/merge"), None).await;
    assert_eq!((s, &p["pr"]["autoMerge"], &p["pr"]["queue"]), (StatusCode::OK, &json!(false), &json!(false)), "{p}");

    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("merged")), "{r}");
    assert_eq!(
        gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).collect::<Vec<_>>(),
        ["merge o/r 12 squash"]
    );
    assert_eq!(get_ticket(&env, &t).await["attempt"]["state"], "merged");
}

/// PDT-297: on a repository without auto-merge, the user's Merge while CI runs sets the pull request to merge (no
/// call to the forge yet), and PDT merges it once CI passes, even where only people may merge (the request was a
/// person's). No queue notice and no "turned off by hand" along the way.
#[tokio::test]
async fn without_auto_merge_pdt_merges_a_pull_request_once_its_ci_passes() {
    let gh = no_auto_merge();
    let env = setup_with(Some(gh.clone())).await;
    settings(&env, json!({"mergeBy": "user"})).await;
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    let merges = || gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();

    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{r}");
    assert_eq!(merges(), 0, "nothing merges while CI runs");
    let v = get_ticket(&env, &t).await;
    assert_eq!(v["attempt"]["pr"]["mergeHeld"], true, "{v}");
    let (_, comments) = call(&env.app, "GET", &format!("/api/tickets/{t}/events"), None).await;
    assert!(
        comments.to_string().contains("merges PR #12 once its CI passes: o/r doesn't allow auto-merge"),
        "{comments}"
    );
    let (_, p) = call(&env.app, "GET", &format!("/api/attempts/{id}/merge"), None).await;
    assert_eq!(p["problem"], "pr_queued", "already set to merge: {p}");

    // Still running: nothing. Green: PDT merges it.
    pdt_api::refresh_prs(&env.state).await;
    assert_eq!(merges(), 0);
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    tick_until(&env, "PDT merging the pull request", || async {
        pdt_api::refresh_prs(&env.state).await;
        (get_ticket(&env, &t).await["attempt"]["state"] == "merged").then_some(Value::Null)
    })
    .await;
    assert_eq!(merges(), 1, "merged once");
    assert!(events_of(&env, "pr.queue_left").await.is_empty(), "no merge queue to leave");
    assert!(env.state.store.get_attempt(&id).await.unwrap().unwrap().merge_requested, "never taken as turned off");
}

/// PDT-297: a workflow's merge (the Pipeline's `attempt.merge`) on a repository without auto-merge waits for CI, then
/// merges the pull request itself.
#[tokio::test]
async fn without_auto_merge_a_workflow_merge_waits_for_ci_then_merges() {
    let gh = no_auto_merge();
    let env = setup_with(Some(gh.clone())).await;
    settings(&env, json!({"requireGreenCi": false})).await;
    let t = ticket(&env, "Change it").await;
    let attempt = start_with_a_commit(&env, &t, "c.txt").await;
    let (id, branch) = (attempt["id"].as_str().unwrap().to_string(), attempt["branch"].as_str().unwrap().to_string());
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));
    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Running)]);
    pdt_api::refresh_prs(&env.state).await;
    // Even without requireGreenCi: the repository can't hold the merge, so it waits.
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ci_pending")), "{v}");
    assert!(!gh.calls.lock().unwrap().iter().any(|c| c.starts_with("merge o/r 12")));

    gh.set_runs(&branch, vec![ci_run(1, &branch, CiState::Passed)]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, r) = call(&env.app, "POST", &format!("/api/attempts/{id}/merge"), Some(json!({"by": "agent"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("merged")), "{r}");
    assert_eq!(get_ticket(&env, &t).await["attempt"]["state"], "merged");
}

/// The ticket's timeline comments by PDT itself.
async fn system_notes(env: &Env, t: &str) -> Vec<String> {
    let events = timeline(env, t).await;
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "comment" && e["actor"] == "system")
        .map(|e| e["body"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn has_branch(repo: &Path, branch: &str) -> bool {
    let out = std::process::Command::new("git").current_dir(repo).args(["branch", "--list", branch]).output().unwrap();
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

async fn move_to(env: &Env, t: &str, stage: &str) -> Value {
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": stage}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// PDT-339: Won't do on a ticket at work stops its sessions, cancels its Pipeline run, closes its open pull request
/// with a comment, and, with nothing to lose, removes the worktree, the branch here and on origin, and the workspace.
#[tokio::test]
async fn wont_do_stops_the_work_closes_the_pr_and_removes_the_worktree_and_branches() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    // Pushes go to a local remote; the fetch URL stays GitHub's, so the forge is the fake.
    let remote = env._tmp.path().join("remote.git");
    git(env._tmp.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
    git(&env.repo, &["config", "remote.origin.pushurl", &remote.to_string_lossy()]);
    pipeline(&env, json!({}), &[]).await;
    let t = ticket(&env, "Not wanted").await;
    let (s, started) =
        call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let worktree = env.repo.join(".worktrees").join(&branch);
    let coder = step_session(&env, "code").await;
    std::fs::write(worktree.join("w.txt"), "w\n").unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-q", "-m", "work"]);
    git(&worktree, &["push", "-q", "-u", "origin", &branch]);
    gh.set_pr(&branch, Some(pull_request(Mergeable::Mergeable)));

    let v = move_to(&env, &t, "wont_do").await;
    assert_eq!(v["attempts"][0]["state"], "abandoned", "{v}");
    assert!(runs_of(&env, &t, "Pipeline").await.iter().all(|r| r["status"] == "cancelled"));
    assert_ne!(session_status(&env, &coder).await, "running");
    assert_eq!(*gh.closed_changes.lock().unwrap(), [("o/r".to_string(), 12)]);
    let said = gh.change_comments_added.lock().unwrap().clone();
    assert!(said.iter().any(|(n, _, body)| *n == 12 && body.contains("Won't do")), "{said:?}");
    assert!(!worktree.exists(), "the worktree is removed");
    assert!(!has_branch(&env.repo, &branch), "the local branch is deleted");
    assert!(!has_branch(&remote, &branch), "the branch on origin is deleted");
    let (_, w) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.project), None).await;
    assert_eq!(w.as_array().unwrap().len(), 0, "{w}");
    let notes = system_notes(&env, &t).await.join("\n");
    assert!(notes.contains("Stopped 1 session") && notes.contains("1 workflow run"), "{notes}");
    assert!(notes.contains("Closed pull request #12.") && notes.contains("Removed the worktree"), "{notes}");
    assert!(notes.contains("its copy on origin"), "{notes}");
    let (_, ws) = call(&env.app, "GET", "/api/workspaces", None).await;
    assert!(
        ws.as_array().unwrap().iter().all(|w| w["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["ticketId"] != t.as_str())),
        "no tab shows the ticket: {ws}"
    );
}

/// PDT-594: a branch named from the project's template (no `pdt-` prefix, a `/`) is still PDT's: Won't do deletes it
/// here and on origin. A branch the user named is left on origin.
#[tokio::test]
async fn wont_do_deletes_a_templated_branch_from_origin_but_not_one_the_user_named() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    let remote = env._tmp.path().join("remote.git");
    git(env._tmp.path(), &["init", "-q", "--bare", "-b", "main", &remote.to_string_lossy()]);
    git(&env.repo, &["config", "remote.origin.pushurl", &remote.to_string_lossy()]);
    let uri = format!("/api/projects/{}/branch-names", env.project);
    let (s, v) = call(&env.app, "PUT", &uri, Some(json!({"template": "{type}/{key}-{slug}"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    for (title, given) in [("Not wanted", None), ("Mine", Some("feature/mine"))] {
        let t = ticket(&env, title).await;
        let mut body = json!({"agent": "claude", "noSession": true});
        if let Some(b) = given {
            body["branch"] = json!(b);
        }
        let (s, started) = call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{started}");
        let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
        if given.is_none() {
            assert!(branch.starts_with("feat/PX-1-"), "{branch}");
        }
        let worktree = env.repo.join(".worktrees").join(branch.replace('/', "-"));
        std::fs::write(worktree.join("w.txt"), "w\n").unwrap();
        git(&worktree, &["add", "."]);
        git(&worktree, &["commit", "-q", "-m", "work"]);
        git(&worktree, &["push", "-q", "-u", "origin", &branch]);

        move_to(&env, &t, "wont_do").await;
        assert!(!worktree.exists(), "the worktree is removed");
        assert!(!has_branch(&env.repo, &branch), "the local branch is deleted");
        assert_eq!(has_branch(&remote, &branch), given.is_some(), "{branch} on origin");
    }
}

/// PDT-339: Won't do keeps a worktree with uncommitted changes (and says so), and leaves a merged pull request alone;
/// Clean up then removes it anyway.
#[tokio::test]
async fn wont_do_keeps_a_dirty_worktree_until_clean_up_and_leaves_a_merged_pr() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    let t = ticket(&env, "Half done").await;
    let attempt = start_with_a_commit(&env, &t, "half.txt").await;
    let branch = attempt["branch"].as_str().unwrap().to_string();
    let worktree = env.repo.join(".worktrees").join(&branch);
    std::fs::write(worktree.join("half.txt"), "changed\n").unwrap();
    let mut merged = pull_request(Mergeable::Mergeable);
    merged.state = PrState::Merged;
    gh.set_pr(&branch, Some(merged));

    move_to(&env, &t, "wont_do").await;
    assert!(gh.closed_changes.lock().unwrap().is_empty(), "a merged pull request is left alone");
    assert!(worktree.exists() && has_branch(&env.repo, &branch));
    let notes = system_notes(&env, &t).await.join("\n");
    assert!(notes.contains("Kept the worktree and branch") && notes.contains("uncommitted changes"), "{notes}");

    let id = attempt["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/clean-up"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!worktree.exists() && !has_branch(&env.repo, &branch));
    assert_eq!(v["stage"], "wont_do");
}

/// PDT-339: the project's Won't do setting: keep, or keep for some days (then the automatic cleanup removes it).
#[tokio::test]
async fn the_projects_wont_do_setting_keeps_the_worktree_or_leaves_it_to_the_cleanup() {
    let env = setup().await;
    let put = |body: Value| {
        let app = env.app.clone();
        let p = env.project.clone();
        async move {
            let (s, v) = call(&app, "PUT", &format!("/api/projects/{p}/storage"), Some(body)).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    };
    put(json!({"cleanAfterDays": 0, "removeAfterDays": 0, "wontDo": "keep"})).await;
    let kept = ticket(&env, "Keep it").await;
    let a = start_with_a_commit(&env, &kept, "k.txt").await;
    let kept_wt = env.repo.join(".worktrees").join(a["branch"].as_str().unwrap());
    move_to(&env, &kept, "wont_do").await;
    assert!(kept_wt.exists());
    assert!(system_notes(&env, &kept).await.iter().any(|n| n.contains("as the project says")));
    put(json!({"cleanAfterDays": 0, "removeAfterDays": 1, "wontDo": "keep"})).await;
    let far = i64::try_from(std::time::UNIX_EPOCH.elapsed().unwrap().as_millis()).unwrap() + 30 * 24 * 60 * 60 * 1000;
    pdt_api::auto_cleanup_at(&env.state, far).await;
    assert!(kept_wt.exists(), "a kept one stays");

    put(json!({"cleanAfterDays": 0, "removeAfterDays": 0, "wontDo": "later", "wontDoAfterDays": 2})).await;
    let later = ticket(&env, "Later").await;
    let a = start_with_a_commit(&env, &later, "l.txt").await;
    let later_wt = env.repo.join(".worktrees").join(a["branch"].as_str().unwrap());
    move_to(&env, &later, "wont_do").await;
    assert!(later_wt.exists());
    assert!(system_notes(&env, &later).await.iter().any(|n| n.contains("for 2 days")));
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{}/storage", env.project),
        Some(json!({"wontDo": "later", "wontDoAfterDays": 999})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let day = 24 * 60 * 60 * 1000;
    let now = i64::try_from(std::time::UNIX_EPOCH.elapsed().unwrap().as_millis()).unwrap();
    pdt_api::auto_cleanup_at(&env.state, now + day).await;
    assert!(later_wt.exists(), "not yet");
    pdt_api::auto_cleanup_at(&env.state, now + 2 * day).await;
    assert!(!later_wt.exists(), "removed once its days passed");
}

/// PDT-339: a ticket that isn't accepted automatically, whose branch merges while it waits at Accept: the Accept step
/// ends (its Inbox item closes), its sessions stop, and Finish merged tickets leaves the one merged banner.
#[tokio::test]
async fn a_merge_while_waiting_at_accept_ends_the_pipeline_and_stops_the_sessions() {
    let env = setup().await;
    pipeline(&env, json!({}), &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let t = ticket(&env, "Merged meanwhile").await;
    let attempt = start_with_a_commit(&env, &t, "m.txt").await;
    let accept = inbox_item(&env, "gate", &t).await;
    assert_eq!(accept["group"], "accept");
    let branch = attempt["branch"].as_str().unwrap();
    // Merge detection sees the branch's own commits first, as its loop does.
    call(&env.app, "POST", &format!("/api/tickets/{t}/check-merge"), None).await;
    git(&env.repo, &["merge", "-q", "--no-ff", "-m", "merge", branch]);
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/check-merge"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["attempts"][0]["state"], "merged", "{v}");
    tick_until(&env, "the Pipeline done", || async {
        let runs = runs_of(&env, &t, "Pipeline").await;
        runs.iter().all(|r| r["status"] == "succeeded").then_some(Value::Null)
    })
    .await;
    assert!(inbox(&env).await.iter().all(|i| i["ticketId"] != t.as_str()), "no Inbox item waits for it");
    let v = get_ticket(&env, &t).await;
    assert_eq!(v["pipelineStage"], Value::Null, "{v}");
    for s in v["sessions"].as_array().unwrap() {
        assert_ne!(session_status(&env, s["id"].as_str().unwrap()).await, "running", "{s}");
    }
    assert_eq!(v["stage"], "in_progress", "the merged banner finishes it");
}

/// PDT-339: Abandon closes the attempt's open pull request too, saying the attempt was abandoned (the ticket keeps its
/// stage), and the timeline counts the sessions and workflow runs it stopped.
#[tokio::test]
async fn abandon_closes_the_pr_saying_the_attempt_was_abandoned() {
    let gh = std::sync::Arc::new(FakeForge::default());
    let env = setup_with(Some(gh.clone())).await;
    pipeline(&env, json!({}), &[]).await;
    let t = ticket(&env, "Give up").await;
    let attempt = start_with_a_commit(&env, &t, "g.txt").await;
    step_session(&env, "code").await;
    gh.set_pr(attempt["branch"].as_str().unwrap(), Some(pull_request(Mergeable::Mergeable)));
    let id = attempt["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/attempts/{id}/abandon"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["stage"], "in_progress");
    assert_eq!(*gh.closed_changes.lock().unwrap(), [("o/r".to_string(), 12)]);
    let said = gh.change_comments_added.lock().unwrap().clone();
    assert!(said.iter().any(|(n, _, b)| *n == 12 && b == "Closed: the attempt was abandoned in PDT."), "{said:?}");
    let notes = system_notes(&env, &t).await.join("\n");
    assert!(notes.contains("Stopped 1 session") && notes.contains("1 workflow run"), "{notes}");
    assert!(notes.contains("Closed pull request #12."), "{notes}");
}
