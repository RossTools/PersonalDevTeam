//! Ticket pickup as a workflow (PDT-327): the pickup watch's triggers, the built-in "Pick up tickets", two pickup runs
//! at once, a start left half-done, and dry runs.

use crate::common;
use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

/// A project with a workflow on `attempt.started`, so starts hand over to it and make no agent session.
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = env!("CARGO_BIN_EXE_fake-agent").to_string();
    state.platform.login_path = None;
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let env = Env { app, state, project: p["id"].as_str().unwrap().into(), _tmp: tmp };
    workflow(
        &env,
        json!({"name": "Takes over", "nodes": [
            {"id": "started", "kind": "attempt.started", "version": 1},
            {"id": "end", "kind": "end", "version": 1}],
         "edges": [{"from": "started", "to": "end"}]}),
    )
    .await;
    env
}

/// An enabled workflow of the project.
async fn workflow(env: &Env, doc: Value) -> String {
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflows",
        Some(json!({"doc": doc, "scope": "project", "projectId": env.project, "enabled": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn settings(env: &Env, body: Value) {
    let mut b = json!({"agentCredentials": null, "runOverrides": null});
    for (k, v) in body.as_object().unwrap() {
        b[k] = v.clone();
    }
    let (s, v) = call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(b)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn ticket(env: &Env, title: &str, stage: &str) -> String {
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": title, "stage": stage})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn get_ticket(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await.1
}

async fn stage(env: &Env, id: &str) -> String {
    get_ticket(env, id).await["stage"].as_str().unwrap().to_string()
}

async fn move_to(env: &Env, id: &str, stage: &str) {
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{id}"), Some(json!({"stage": stage}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn events(env: &Env, kind: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/events?type={kind}&limit=500"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

async fn builtin(env: &Env) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    list.as_array().unwrap().iter().find(|w| w["builtin"] == "pick-up-tickets").expect("Pick up tickets").clone()
}

#[tokio::test]
async fn pick_up_tickets_runs_when_a_ticket_can_start_is_unblocked_or_room_frees_up() {
    let env = setup().await;
    let pick = builtin(&env).await;
    assert_eq!(pick["enabled"], true, "on from the start");
    assert_eq!(pick["scope"], "host");

    // Auto-start off: nothing is told, nothing starts.
    let a = ticket(&env, "A", "todo").await;
    common::pick_up(&env.state).await;
    assert_eq!(stage(&env, &a).await, "todo");
    assert!(events(&env, "ticket.startable").await.is_empty());

    // On: A can start, and does; B waits at the cap of one.
    settings(&env, json!({"autoStart": "todo", "startDelaySeconds": 0})).await;
    common::pick_up(&env.state).await;
    assert_eq!(stage(&env, &a).await, "in_progress");
    let told = events(&env, "ticket.startable").await;
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(
        (told[0]["payload"]["ticket"]["id"].as_str(), told[0]["projectId"].as_str()),
        (Some(a.as_str()), Some(env.project.as_str()))
    );
    let b = ticket(&env, "B", "todo").await;
    common::pick_up(&env.state).await;
    common::pick_up(&env.state).await;
    assert_eq!(stage(&env, &b).await, "todo", "at the cap");
    assert_eq!(events(&env, "ticket.startable").await.len(), 2, "B told once");

    // A finishes: room for one more, and B starts.
    move_to(&env, &a, "done").await;
    common::pick_up(&env.state).await;
    let freed = events(&env, "slot.freed").await;
    assert_eq!(freed[0]["payload"]["reason"], "attempt", "{freed:?}");
    assert_eq!(stage(&env, &b).await, "in_progress");

    // A raised cap is room too.
    let c = ticket(&env, "C", "todo").await;
    common::pick_up(&env.state).await;
    assert_eq!(stage(&env, &c).await, "todo");
    settings(&env, json!({"maxRunningAttempts": 2})).await;
    common::pick_up(&env.state).await;
    assert_eq!(events(&env, "slot.freed").await[0]["payload"]["reason"], "cap");
    assert_eq!(stage(&env, &c).await, "in_progress");

    // Blocked: never told until its blocker closes, then it starts.
    settings(&env, json!({"maxRunningAttempts": 5})).await;
    let blocker = ticket(&env, "Blocker", "draft").await;
    let d = ticket(&env, "D", "todo").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{blocker}/links"), Some(json!({"to": d, "kind": "blocks"})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    common::pick_up(&env.state).await;
    assert_eq!(stage(&env, &d).await, "todo", "blocked");
    let told_d = |list: Vec<Value>| list.iter().filter(|e| e["payload"]["ticket"]["id"] == d).count();
    assert_eq!(told_d(events(&env, "ticket.startable").await), 0);
    move_to(&env, &blocker, "done").await;
    common::pick_up(&env.state).await;
    assert_eq!(told_d(events(&env, "ticket.startable").await), 1);
    assert_eq!(stage(&env, &d).await, "in_progress");

    // The runs that found nothing to start are quiet; the ones that started a ticket aren't.
    let (_, runs) =
        call(&env.app, "GET", &format!("/api/workflow-runs?workflow={}&limit=100", pick["id"].as_str().unwrap()), None)
            .await;
    let statuses: Vec<&str> = runs.as_array().unwrap().iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses.iter().filter(|s| **s == "succeeded").count(), 4, "{statuses:?}");
    assert!(statuses.iter().all(|s| ["succeeded", "quiet"].contains(s)), "{statuses:?}");
}

/// The built-in's steps as a Run now workflow, so two runs can go at once.
fn pickup_by_hand() -> Value {
    json!({"name": "Pick up by hand", "nodes": [
        {"id": "go", "kind": "manual", "version": 1},
        {"id": "candidates", "kind": "tickets.candidates", "version": 1},
        {"id": "choose", "kind": "tickets.choose", "version": 1, "config": {"candidates": "{{steps.candidates}}"}},
        {"id": "start", "kind": "ticket.start", "version": 1, "config": {"ticket": "{{steps.choose.tickets}}"}},
        {"id": "end", "kind": "end", "version": 1, "config": {"quiet": true}}],
     "edges": [
        {"from": "go", "to": "candidates"},
        {"from": "candidates", "to": "choose"},
        {"from": "choose", "port": "chosen", "to": "start"},
        {"from": "choose", "port": "none", "to": "end"},
        {"from": "start", "port": "started", "to": "end"},
        {"from": "start", "port": "skipped", "to": "end"},
        {"from": "start", "port": "failed", "to": "end"}]})
}

async fn active_attempts(env: &Env, ids: &[&str]) -> usize {
    let mut n = 0;
    for id in ids {
        let t = get_ticket(env, id).await;
        n += t["attempts"].as_array().unwrap().iter().filter(|a| a["state"] == "active").count();
    }
    n
}

#[tokio::test]
async fn two_pickup_runs_at_once_never_start_a_ticket_twice_nor_go_over_the_cap() {
    let env = setup().await;
    settings(&env, json!({"autoStart": "todo", "startDelaySeconds": 0})).await;
    let wf = workflow(&env, pickup_by_hand()).await;
    let x = ticket(&env, "X", "todo").await;
    let y = ticket(&env, "Y", "todo").await;
    let run_twice = || async {
        for _ in 0..2 {
            let (s, v) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({}))).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
        // Both runs advance side by side in one engine tick.
        pdt_api::run_workflows(&env.state).await;
    };
    run_twice().await;
    assert_eq!(active_attempts(&env, &[&x, &y]).await, 1, "a cap of one");

    // Room for both: each starts once.
    settings(&env, json!({"maxRunningAttempts": 3})).await;
    run_twice().await;
    assert_eq!(active_attempts(&env, &[&x, &y]).await, 2);
    for id in [&x, &y] {
        assert_eq!(get_ticket(&env, id).await["attempts"].as_array().unwrap().len(), 1, "started once");
    }
}

#[tokio::test]
async fn pick_up_steps_can_narrow_to_a_label_and_their_own_cap() {
    let env = setup().await;
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 5, "startDelaySeconds": 0})).await;
    let mut doc = pickup_by_hand();
    doc["nodes"][1]["config"] = json!({"labels": ["agentic"]});
    doc["nodes"][2]["config"]["cap"] = json!(1);
    let wf = workflow(&env, doc).await;
    let plain = ticket(&env, "Plain", "todo").await;
    let (s, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": "Agentic", "stage": "todo", "labels": ["agentic"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let agentic = t["id"].as_str().unwrap().to_string();
    for _ in 0..2 {
        call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({}))).await;
        pdt_api::run_workflows(&env.state).await;
    }
    assert_eq!(stage(&env, &agentic).await, "in_progress");
    assert_eq!(stage(&env, &plain).await, "todo", "not agentic");
}

#[tokio::test]
async fn a_start_left_half_done_is_announced_so_its_workflow_takes_over() {
    let env = setup().await;
    let t = ticket(&env, "Half done", "todo").await;
    // An attempt made three minutes ago whose start was never announced (pdtd stopped before Start logged it).
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let made = |id: &'static str, ticket: String, at: i64| {
        let store = env.state.store.clone();
        async move { store.start_attempt(id, &ticket, None, "b", "main", "", json!({}), at).await.unwrap() }
    };
    made("half", t.clone(), now - 3 * 60 * 1000).await;
    let fresh = ticket(&env, "Starting now", "todo").await;
    made("fresh", fresh.clone(), now).await;
    assert!(events(&env, "attempt.started").await.is_empty());

    pdt_api::watch_pickup(&env.state).await;
    let started = events(&env, "attempt.started").await;
    assert_eq!(started.len(), 1, "only the old one: a Start still making its session announces it itself");
    assert_eq!(started[0]["ticketId"], t);
    assert!(!events(&env, "ticket.stage_changed").await.is_empty(), "its move to In progress too");
    pdt_api::run_workflows(&env.state).await;
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={t}"), None).await;
    assert!(runs.as_array().unwrap().iter().any(|r| r["workflowName"] == "Takes over"), "{runs}");
    // Announced once.
    pdt_api::watch_pickup(&env.state).await;
    assert_eq!(events(&env, "attempt.started").await.len(), 1);
}

#[tokio::test]
async fn a_dry_run_of_pick_up_tickets_says_what_it_would_start_and_starts_nothing() {
    let env = setup().await;
    settings(&env, json!({"autoStart": "todo", "startDelaySeconds": 0})).await;
    let t = ticket(&env, "Next", "todo").await;
    let pick = builtin(&env).await;
    let key = get_ticket(&env, &t).await["key"].as_str().unwrap().to_string();
    let event = json!({"type": "ticket.startable", "payload": {"projectId": env.project,
        "ticket": {"id": t, "key": key, "title": "Next", "stage": "todo", "priority": "none", "labels": []}}});
    let (s, v) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": pick["id"], "event": event}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let steps = v["run"]["steps"].as_array().unwrap();
    let step = |id: &str| steps.iter().find(|s| s["nodeId"] == id).unwrap_or_else(|| panic!("no {id}: {v}")).clone();
    assert_eq!(step("candidates")["output"]["count"], 1, "{v}");
    assert_eq!(step("choose")["port"], "chosen");
    assert_eq!(step("start")["output"]["would"], json!([format!("start {key}")]));
    assert_eq!(stage(&env, &t).await, "todo");
}

/// The docs' "Ready tickets only at night": a step's cap above the project's running cap holds; Start doesn't refuse
/// for the project's cap, and the ticket isn't passed over as failed.
#[tokio::test]
async fn a_steps_cap_above_the_projects_running_cap_holds() {
    let env = setup().await;
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 1, "startDelaySeconds": 0})).await;
    let mut doc = pickup_by_hand();
    doc["nodes"][1]["config"] = json!({"stages": ["ready"]});
    doc["nodes"][2]["config"]["cap"] = json!(3);
    let wf = workflow(&env, doc).await;
    let x = ticket(&env, "X", "ready").await;
    let y = ticket(&env, "Y", "ready").await;
    let z = ticket(&env, "Z", "ready").await;
    let mut ports = vec![];
    for _ in 0..4 {
        let (_, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({}))).await;
        pdt_api::run_workflows(&env.state).await;
        let (_, run) =
            call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await;
        let start = run["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "start").cloned();
        ports.push(start.map(|s| s["port"].clone()).unwrap_or(Value::Null));
    }
    assert_eq!(active_attempts(&env, &[&x, &y, &z]).await, 3, "three, over the project's cap of one");
    assert_eq!(ports[..3], [json!("started"), json!("started"), json!("started")], "{ports:?}");
    assert_eq!(ports[3], Value::Null, "the fourth chose none: at the step's cap");
    // Start's own check still holds the project's cap for a person's Start.
    let w = ticket(&env, "W", "todo").await;
    let (s, v) =
        call(&env.app, "POST", &format!("/api/tickets/{w}/start"), Some(json!({"agent": "claude", "noSession": true})))
            .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("start_blocked")), "{v}");
}

/// What another pickup run started since a pick was chosen counts: `ticket.start` checks the pick's project, area and
/// label caps again under its lock, and skips it (not a failure) when one is full.
#[tokio::test]
async fn a_stale_pick_is_checked_again_and_skipped_when_a_cap_filled_since() {
    let env = setup().await;
    settings(&env, json!({"autoStart": "todo", "maxRunningAttempts": 5, "startDelaySeconds": 0})).await;
    let (_, t) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/tickets", env.project),
        Some(json!({"title": "In flight", "stage": "todo", "labels": ["agentic"]})),
    )
    .await;
    let busy = t["id"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{busy}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let next = ticket(&env, "Next", "todo").await;
    let start_with = |pick: Value| {
        json!({"name": "Start a pick", "nodes": [
            {"id": "go", "kind": "manual", "version": 1},
            {"id": "start", "kind": "ticket.start", "version": 1, "config": {"ticket": [pick]}}],
         "edges": [{"from": "go", "to": "start"}]})
    };
    let run = |doc: Value| {
        let env = &env;
        async move {
            let wf = workflow(env, doc).await;
            let (_, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({}))).await;
            pdt_api::run_workflows(&env.state).await;
            let (_, run) =
                call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await;
            run["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "start").unwrap().clone()
        }
    };
    // Chosen under a cap of one, before `busy` started: the project is at it now.
    let step = run(start_with(json!({"id": next, "cap": 1}))).await;
    assert_eq!(step["port"], "skipped", "{step}");
    assert_eq!(step["output"]["skipped"][0]["reason"], "the project is at its cap (1 of 1 in flight)");
    // A label cap the ticket in flight fills (the pick has the label too).
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{next}"), Some(json!({"labels": ["agentic"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let step = run(start_with(json!({"id": next, "cap": 5, "labelCaps": {"agentic": 1}}))).await;
    assert_eq!(step["output"]["skipped"][0]["reason"], "label agentic is at its cap of 1", "{step}");
    let step = run(start_with(json!({"id": next, "cap": 5, "areaCaps": {}, "labelCaps": {"agentic": 2}}))).await;
    assert_eq!(step["port"], "started", "{step}");
    assert_eq!(stage(&env, &next).await, "in_progress");
}
