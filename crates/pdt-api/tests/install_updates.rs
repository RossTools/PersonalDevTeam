//! Install updates (PDT-369): the nodes it is made of (waiting until PDT is idle, in a window, the check after
//! installing), what happens when an update fails, and Settings → Updates, which sets its inputs. The update loop and
//! installing what a check found are in `update.rs`.

use crate::common;

use axum::http::StatusCode;
use common::call;
use pdt_core::activity::Activity;
use pdt_core::handoff::{UpdateOutcome, UpdateResult, result_path};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Env {
    app: axum::Router,
    state: pdt_api::AppState,
    tmp: tempfile::TempDir,
}

/// A PDT under pdt-host running `current`, with no GitHub behind it (nothing here checks for updates).
async fn setup(current: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.update = Arc::new(pdt_api::UpdateService::new("http://127.0.0.1:9", "o/r", true, current));
    Env { app: pdt_api::router(state.clone(), None), state, tmp }
}

/// A workflow run by hand: `node` (a `kind` with `config`) leaving by any of `ports` to its end.
async fn one_step(env: &Env, kind: &str, config: Value, ports: &[&str]) -> String {
    let edges: Vec<Value> = std::iter::once(json!({"from": "t", "to": "n"}))
        .chain(ports.iter().map(|p| json!({"from": "n", "port": p, "to": "e"})))
        .collect();
    let doc = json!({"name": kind, "nodes": [
        {"id": "t", "kind": "manual", "version": 1, "config": {}},
        {"id": "n", "kind": kind, "version": 1, "config": config},
        {"id": "e", "kind": "end", "version": 1}], "edges": edges});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r["runId"].as_str().unwrap().to_string()
}

async fn run(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"].as_array().unwrap().iter().rev().find(|s| s["nodeId"] == node).unwrap_or_else(|| panic!("{run}"))
}

/// Ticks the engine until the run ended (5 s at most).
async fn finished(env: &Env, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let r = run(env, id).await;
        if !matches!(r["status"].as_str(), Some("running" | "waiting")) {
            return r;
        }
        assert!(Instant::now() < deadline, "the run never ended: {r}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Ticks the engine a few times; the run must still wait.
async fn still_waiting(env: &Env, id: &str) -> Value {
    for _ in 0..4 {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let r = run(env, id).await;
    assert_eq!(r["status"], "waiting", "{r}");
    r
}

/// A running Claude session that is working: PDT is busy.
async fn busy_session(env: &Env) {
    let repo = env.tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    crate::common::init_repo(&repo);
    let body = json!({"name": "P", "repoPath": repo.to_string_lossy()});
    let (_, p) = call(&env.app, "POST", "/api/projects", Some(body)).await;
    let session = Session {
        id: "busy".into(),
        project_id: p["id"].as_str().unwrap().into(),
        worktree_id: None,
        kind: SessionKind::AgentTui,
        agent: Some(AgentKind::Claude),
        bypass_permissions: false,
        title: "feature".into(),
        cwd: repo.to_string_lossy().into(),
        status: SessionStatus::Running,
        exit_code: None,
        created_at: 1,
        ended_at: None,
        external_session_id: None,
        fresh_started_at: None,
        activity: Some(Activity::Working),
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: None,
        ticket: None,
        mode: pdt_core::conversation::SessionMode::Terminal,
        model: None,
        permission_mode: None,
        persona: None,
        background: vec![],
    };
    env.state.store.insert_session(&session).await.unwrap();
}

fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// `HH:MM`, `d` minutes from now (this computer's time).
fn clock(d: i32) -> String {
    let t = jiff::Zoned::now();
    let m = (i32::from(t.hour()) * 60 + i32::from(t.minute()) + d).rem_euclid(24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

const WAIT_PORTS: &[&str] = &["idle", "timeout", "skipped"];

#[tokio::test]
async fn waiting_until_idle_goes_on_once_no_agent_works() {
    let env = setup("0.1.0").await;
    busy_session(&env).await;
    let id = one_step(&env, "host.wait_idle", json!({"checkEveryMs": 20}), WAIT_PORTS).await;
    let r = still_waiting(&env, &id).await;
    let waiting = step(&r, "n")["output"]["waiting"].as_str().unwrap().to_string();
    assert!(waiting.contains("Claude · feature is working"), "{waiting}");
    assert_eq!(step(&r, "n")["attempts"], 1, "looking again is no new attempt");

    env.state.store.end_session("busy", SessionStatus::Finished, Some(0), now()).await.unwrap();
    let r = finished(&env, &id).await;
    assert_eq!(step(&r, "n")["port"], "idle", "{r}");
}

#[tokio::test]
async fn still_busy_after_the_longest_wait_it_goes_on_by_timeout() {
    let env = setup("0.1.0").await;
    busy_session(&env).await;
    let id = one_step(&env, "host.wait_idle", json!({"checkEveryMs": 20, "maxWaitMs": 100}), WAIT_PORTS).await;
    let r = finished(&env, &id).await;
    let s = step(&r, "n");
    assert_eq!(s["port"], "timeout", "{r}");
    assert!(s["output"]["busy"][0].as_str().unwrap().contains("is working"), "{s}");
}

/// Time outside the window doesn't count towards installing anyway: the window opening on busy agents waits for them.
#[tokio::test]
async fn time_outside_the_window_does_not_count_towards_installing_anyway() {
    let env = setup("0.1.0").await;
    busy_session(&env).await;
    let later = json!({"on": true, "from": clock(120), "to": clock(180)});
    let config = json!({"checkEveryMs": 20, "maxWaitMs": 1, "window": later});
    let id = one_step(&env, "host.wait_idle", config, WAIT_PORTS).await;
    let r = still_waiting(&env, &id).await;
    let out = &step(&r, "n")["output"];
    assert!(out["waiting"].as_str().unwrap().starts_with("outside the window"), "{r}");
    assert_eq!(out["since"], Value::Null, "the clock doesn't run outside the window: {out}");
}

#[tokio::test]
async fn outside_its_window_it_waits_even_when_idle() {
    let env = setup("0.1.0").await;
    let later = json!({"on": true, "from": clock(120), "to": clock(180)});
    let id = one_step(&env, "host.wait_idle", json!({"checkEveryMs": 20, "window": later}), WAIT_PORTS).await;
    let r = still_waiting(&env, &id).await;
    assert!(step(&r, "n")["output"]["waiting"].as_str().unwrap().starts_with("outside the window"), "{r}");

    // A window that is open now (around midnight too: it wraps).
    let open = json!({"on": true, "from": clock(-10), "to": clock(60)});
    let id = one_step(&env, "host.wait_idle", json!({"checkEveryMs": 20, "window": open}), WAIT_PORTS).await;
    assert_eq!(step(&finished(&env, &id).await, "n")["port"], "idle");
    // A window that is off is no window.
    let off = json!({"on": false, "from": clock(120), "to": clock(180)});
    let id = one_step(&env, "host.wait_idle", json!({"window": off}), WAIT_PORTS).await;
    assert_eq!(step(&finished(&env, &id).await, "n")["port"], "idle");

    // Times that aren't times of day are refused.
    let bad = json!({"name": "bad", "nodes": [
        {"id": "t", "kind": "manual", "version": 1, "config": {}},
        {"id": "n", "kind": "host.wait_idle", "version": 1, "config": {"window": {"on": true, "from": "2am", "to": "06:00"}}}],
        "edges": [{"from": "t", "to": "n"}]});
    let (s, e) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": bad, "enabled": true}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("workflow_invalid")), "{e}");
}

#[tokio::test]
async fn an_update_that_is_installed_or_not_set_to_install_is_not_waited_for() {
    let env = setup("0.1.7").await;
    let id = one_step(&env, "host.wait_idle", json!({"update": "0.1.7"}), WAIT_PORTS).await;
    let r = finished(&env, &id).await;
    assert_eq!(step(&r, "n")["port"], "skipped");
    assert!(step(&r, "n")["output"]["reason"].as_str().unwrap().contains("installed"), "{r}");
    // Updates are set to notify only (PDT's default).
    let id = one_step(&env, "host.wait_idle", json!({"update": "0.1.8"}), WAIT_PORTS).await;
    let r = finished(&env, &id).await;
    assert!(step(&r, "n")["output"]["reason"].as_str().unwrap().contains("notify only"), "{r}");
}

#[tokio::test]
async fn the_check_after_installing_reports_what_is_wrong() {
    let env = setup("0.1.7").await;
    let ports = &["healthy", "unhealthy"];
    let id = one_step(&env, "host.check_health", json!({"settleMs": 0, "version": "0.1.7"}), ports).await;
    assert_eq!(step(&finished(&env, &id).await, "n")["port"], "healthy");

    // It lets PDT settle first.
    let id = one_step(&env, "host.check_health", json!({"settleMs": 60_000}), ports).await;
    still_waiting(&env, &id).await;

    // Another version runs, and a loop stopped.
    env.state.loops.register("watch", Duration::from_millis(1), Some(1));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let id = one_step(&env, "host.check_health", json!({"settleMs": 0, "version": "0.1.8"}), ports).await;
    let r = finished(&env, &id).await;
    let s = step(&r, "n");
    assert_eq!(s["port"], "unhealthy", "{r}");
    assert_eq!(s["output"]["error"], "PDT runs v0.1.7, not v0.1.8; its watch loop stopped", "{s}");
}

async fn install_updates(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/install-updates", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn inbox(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

fn write_result(env: &Env, outcome: UpdateOutcome, from: &str, to: &str) {
    let r = UpdateResult {
        outcome,
        from_version: from.into(),
        to_version: to.into(),
        message: "the new version did not start: rolled back".into(),
        log: None,
        at: 5,
    };
    let p = result_path(&env.state.data_dir);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_vec(&r).unwrap()).unwrap();
}

/// Ticks the engine until Install updates' runs all ended; them.
async fn runs_ended(env: &Env, wf: &str) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
        let list = r.as_array().unwrap().clone();
        if !list.is_empty() && list.iter().all(|r| !matches!(r["status"].as_str(), Some("running" | "waiting"))) {
            return list;
        }
        assert!(Instant::now() < deadline, "the runs never ended: {list:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_failed_update_goes_to_the_inbox() {
    let env = setup("0.1.5").await;
    let wf = install_updates(&env).await["workflowId"].as_str().unwrap().to_string();
    write_result(&env, UpdateOutcome::RolledBack, "0.1.5", "0.1.7");
    pdt_api::update::log_installed(&env.state).await;
    pdt_api::update::log_installed(&env.state).await;
    runs_ended(&env, &wf).await;
    let items = inbox(&env).await;
    let failed: Vec<&Value> = items.iter().filter(|i| i["title"] == "PDT couldn't update to v0.1.7").collect();
    assert_eq!(failed.len(), 1, "once: {items:?}");
    assert!(failed[0]["body"]["text"].as_str().unwrap().starts_with("the new version did not start"), "{items:?}");
}

#[tokio::test]
async fn a_failed_check_after_installing_goes_to_the_inbox() {
    let env = setup("0.1.7").await;
    let wf = install_updates(&env).await["workflowId"].as_str().unwrap().to_string();
    // No waiting a few minutes here: the check looks at once (a customized workflow, otherwise the built-in).
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let mut doc = w["doc"].clone();
    let health = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "health").unwrap();
    health["config"]["settleMs"] = json!(0);
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{wf}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    env.state.loops.register("watch", Duration::from_millis(1), Some(1));
    tokio::time::sleep(Duration::from_millis(20)).await;
    write_result(&env, UpdateOutcome::Installed, "0.1.5", "0.1.7");
    pdt_api::update::log_installed(&env.state).await;
    let r = runs_ended(&env, &wf).await;
    assert_eq!(r.len(), 1, "{r:?}");
    let items = inbox(&env).await;
    let item = items.iter().find(|i| i["title"] == "PDT v0.1.7 isn't healthy after updating").expect("in the Inbox");
    assert!(item["body"]["text"].as_str().unwrap().starts_with("its watch loop stopped"), "{item}");
}

#[tokio::test]
async fn settings_set_the_workflows_inputs_and_it_does_what_they_say() {
    let env = setup("0.1.0").await;
    let v = install_updates(&env).await;
    assert_eq!((v["enabled"].as_bool(), v["customized"].as_bool()), (Some(true), Some(false)), "{v}");
    assert_eq!(v["values"], v["defaults"]);
    assert_eq!((v["values"]["window"].as_bool(), v["values"]["installAnywayAfter"].as_u64()), (Some(false), Some(0)));
    let wf = v["workflowId"].as_str().unwrap().to_string();

    let body =
        json!({"values": {"window": true, "windowFrom": "01:30", "windowTo": "05:00", "installAnywayAfter": "4h"}});
    let (s, v) = call(&env.app, "PUT", "/api/install-updates", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["values"]["installAnywayAfter"], 4 * 3_600_000);
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{wf}/versions"), None).await;
    assert_eq!(versions[0]["note"], "inputs changed in Settings", "{versions}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let wait = w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "wait").unwrap().clone();
    assert_eq!(wait["config"]["window"], json!({"on": true, "from": "01:30", "to": "05:00"}), "{w}");
    assert_eq!(wait["config"]["maxWaitMs"], 4 * 3_600_000);

    for bad in [json!({"windowFrom": "1am"}), json!({"nope": 1}), json!({"installAnywayAfter": "soon"})] {
        let (s, e) = call(&env.app, "PUT", "/api/install-updates", Some(json!({"values": bad}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {e}");
    }

    // Stable releases only: a nightly build ends the run at once, a stable release waits (here: outside the window).
    call(
        &env.app,
        "PUT",
        "/api/install-updates",
        Some(json!({"values": {"stableOnly": true, "windowFrom": clock(120), "windowTo": clock(180)}})),
    )
    .await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"updatePolicy": "auto_when_idle"}))).await;
    let available = |version: &str, nightly: bool| {
        pdt_core::workflow::api::NewLogEvent::new(
            "update.available",
            format!("update:{version}"),
            "update",
            json!({"version": version, "current": "0.1.0", "tag": format!("v{version}"), "channel": "nightly",
               "nightly": nightly, "notesUrl": "", "notes": "", "publishedAt": null}),
        )
    };
    env.state.store.emit_event(&available("0.2.0-nightly.20261101", true), now()).await.unwrap();
    env.state.store.emit_event(&available("0.1.9", false), now()).await.unwrap();
    for _ in 0..4 {
        pdt_api::run_workflows(&env.state).await;
    }
    let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    let by = |v: &str| r.as_array().unwrap().iter().find(|r| r["trigger"]["version"] == v).unwrap().clone();
    assert_eq!(by("0.2.0-nightly.20261101")["status"], "quiet", "{r}");
    let stable = run(&env, by("0.1.9")["id"].as_str().unwrap()).await;
    assert_eq!(stable["status"], "waiting", "{stable}");
    assert!(step(&stable, "wait")["output"]["waiting"].as_str().unwrap().starts_with("outside the window"));
}

#[tokio::test]
async fn a_customized_or_disabled_workflow_is_named_and_reset_to_default() {
    let env = setup("0.1.0").await;
    let wf = install_updates(&env).await["workflowId"].as_str().unwrap().to_string();
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let mut doc = w["doc"].clone();
    let install = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "install").unwrap();
    install["config"]["force"] = json!(true);
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{wf}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(install_updates(&env).await["customized"], true);
    let (s, e) = call(&env.app, "PUT", "/api/install-updates", Some(json!({"values": {"window": true}}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("customized")), "{e}");

    let (s, _) = call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(install_updates(&env).await["enabled"], false);

    let (s, v) = call(&env.app, "POST", "/api/install-updates/reset", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["customized"].as_bool(), v["enabled"].as_bool()), (Some(false), Some(true)), "{v}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let install = w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "install").unwrap().clone();
    assert_eq!(install["config"]["force"], false, "the default again");
}
