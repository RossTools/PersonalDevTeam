//! PDT-319: watching a ticket or a workflow run. A PM conversation (`fake-agent` as Claude) watches with its `watch`
//! tool and is told through the outbox; the user watches from the API and is told by `watch.event` notifications.

use crate::common;

use axum::http::StatusCode;
use common::call;
use common::pm::*;
use pdt_core::ticket::{Actor, EventKind};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// A PDT whose watches tell at once (no waiting for a burst).
async fn env() -> Env {
    setup_with(|s| s.watch_wait_ms = (0, 0)).await
}

async fn ticket_id(env: &Env) -> String {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    v["items"][0]["id"].as_str().unwrap().to_string()
}

async fn outcome(env: &Env, ticket: &str, outcome: &str, summary: &str) {
    let data = json!({"outcome": outcome, "summary": summary, "persona": "Coder", "by": "agent"});
    env.state.store.add_ticket_event(ticket, EventKind::Outcome, Actor::System, None, Some(data), now()).await.unwrap();
}

async fn comment(env: &Env, ticket: &str, body: &str) {
    env.state.store.add_ticket_event(ticket, EventKind::Comment, Actor::User, Some(body), None, now()).await.unwrap();
}

async fn moved(env: &Env, ticket: &str, to: &str) {
    let data = json!({"from": "in_progress", "to": to});
    env.state.store.add_ticket_event(ticket, EventKind::Moved, Actor::User, None, Some(data), now()).await.unwrap();
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// What PDT told the conversation, as the agent echoed it.
async fn told(env: &Env, session: &str) -> Vec<String> {
    agent_says(env, session)
        .await
        .into_iter()
        .filter_map(|s| s.strip_prefix("echo: [PDT] ").map(String::from))
        .collect()
}

/// Ticks until the conversation was told `n` things.
async fn wait_told(env: &Env, session: &str, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let t = told(env, session).await;
        if t.len() >= n {
            return t;
        }
        if Instant::now() >= deadline {
            let waiting = env.state.store.waiting_session_messages().await.unwrap();
            let watches = env.state.store.active_watches().await.unwrap();
            panic!(
                "told only {t:?}; said {:?}; waiting {waiting:?}; watches {watches:?}",
                agent_says(env, session).await
            );
        }
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A few engine ticks, for what should not happen.
async fn settle(env: &Env) {
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn watches_of(env: &Env, ticket: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/watches?ticket={ticket}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

async fn watch_notifications(app: &axum::Router) -> Vec<Value> {
    let (_, v) = call(app, "GET", "/api/notifications", None).await;
    v["items"].as_array().unwrap().iter().filter(|n| n["kind"] == "watch.event").cloned().collect()
}

#[tokio::test]
async fn the_pm_hears_when_a_watched_tickets_agent_reports_and_the_ticket_finishes() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let pm = start_pm(&env, "who").await;
    wait_said(&env, &pm, 1).await;
    let said = ask(&env, &pm, r#"mcp: watch {"ticket": "PX-1", "events": ["finished", "outcome", "comment"]}"#).await;
    assert!(said.starts_with("mcp watch:") && said.contains("\"targetLabel\": \"PX-1\""), "{said}");
    let w = watches_of(&env, &t).await;
    assert_eq!(w.len(), 1);
    assert_eq!(w[0]["sessionId"], pm.as_str());
    assert!(w[0]["watcher"].as_str().unwrap().starts_with("PM “"), "{}", w[0]);
    settle(&env).await;
    assert!(told(&env, &pm).await.is_empty(), "what happened before the watch is no news");

    outcome(&env, &t, "submitted", "Built the page.").await;
    let t1 = wait_told(&env, &pm, 1).await;
    assert_eq!(t1[0], "Watched PX-1 “Fix the login”: the Coder reported `submitted`: Built the page.");
    settle(&env).await;
    assert_eq!(told(&env, &pm).await.len(), 1, "each event once");

    comment(&env, &t, "Looks good").await;
    assert_eq!(wait_told(&env, &pm, 2).await[1], "Watched PX-1 “Fix the login”: the user commented: Looks good.");

    moved(&env, &t, "done").await;
    let t3 = wait_told(&env, &pm, 3).await;
    assert_eq!(t3[2], "Watched PX-1 “Fix the login”: moved to Done. The watch has ended.");
    assert!(watches_of(&env, &t).await.is_empty(), "once: the watch ended");
    assert_eq!(ask(&env, &pm, "mcp: list_watches {}").await, "mcp list_watches: {\n  \"watches\": []\n}");
    comment(&env, &t, "After the end").await;
    settle(&env).await;
    assert_eq!(told(&env, &pm).await.len(), 3, "nothing after the end");
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;
}

#[tokio::test]
async fn a_burst_is_told_in_one_message_and_delivery_waits_for_the_pm_to_be_idle() {
    // Real waits: a burst coalesces.
    let env = setup().await;
    let t = ticket_id(&env).await;
    let pm = start_pm(&env, "who").await;
    wait_said(&env, &pm, 1).await;
    ask(&env, &pm, r#"mcp: watch {"ticket": "PX-1", "events": ["outcome", "comment"], "once": false}"#).await;

    outcome(&env, &t, "changes_requested", "Fix the tests").await;
    comment(&env, &t, "One more thing").await;
    let start = now();
    pdt_api::tick_watches_at(&env.state, start).await;
    let waiting = |env: &Env| {
        let st = env.state.clone();
        let pm = pm.clone();
        async move {
            st.store
                .waiting_session_messages()
                .await
                .unwrap()
                .into_iter()
                .filter(|m| m.session_id.as_deref() == Some(pm.as_str()) && m.text.starts_with("[PDT] Watched"))
                .collect::<Vec<_>>()
        }
    };
    assert!(waiting(&env).await.is_empty(), "a fresh burst waits for more");

    // The PM is busy with a long turn: the message waits for it.
    send(&env, &pm, "slow").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    pdt_api::tick_watches_at(&env.state, start + 16_000).await;
    let queued = waiting(&env).await;
    assert_eq!(queued.len(), 1, "one message for the burst");
    assert_eq!(
        queued[0].text,
        "[PDT] Watched PX-1 “Fix the login”:\n- the Coder reported `changes_requested`: Fix the tests\n- the user \
         commented: One more thing"
    );
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(waiting(&env).await.len(), 1, "not into a running turn");
    assert!(told(&env, &pm).await.is_empty());
    call(&env.app, "POST", &format!("/api/sessions/{pm}/interrupt"), None).await;
    let t1 = wait_told(&env, &pm, 1).await;
    assert!(t1[0].contains("changes_requested") && t1[0].contains("One more thing"), "{t1:?}");
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;
}

#[tokio::test]
async fn a_closed_conversations_watch_tells_the_user_and_ends() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let pm = start_pm(&env, "who").await;
    wait_said(&env, &pm, 1).await;
    ask(&env, &pm, r#"mcp: watch {"ticket": "PX-1", "events": ["outcome"]}"#).await;
    // Closed (its journal written), before anything happened.
    use pdt_core::pm::Rotation;
    assert!(env.state.store.set_pm_rotation(&pm, Rotation::None, Rotation::Done, None, now()).await.unwrap());
    outcome(&env, &t, "submitted", "Done it").await;
    // One tick: the note is made and, the conversation being closed, goes to the user.
    pdt_api::tick_watches(&env.state).await;
    let n = watch_notifications(&env.app).await;
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0]["title"], "PX-1 “Fix the login”: the Coder reported `submitted`: Done it");
    assert!(n[0]["body"].as_str().unwrap().contains("was closed, so you're told instead"), "{}", n[0]);
    assert_eq!(n[0]["target"]["kind"], "ticket");
    assert!(watches_of(&env, &t).await.is_empty(), "the watch ended");
    settle(&env).await;
    assert!(told(&env, &pm).await.is_empty());
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;
}

#[tokio::test]
async fn news_during_a_closing_conversations_journal_turn_goes_to_the_user() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let pm = start_pm(&env, "who").await;
    wait_said(&env, &pm, 1).await;
    ask(&env, &pm, r#"mcp: watch {"ticket": "PX-1", "events": ["outcome"]}"#).await;
    // The user closed it: it is writing its journal (rotation `journal`) when the news comes.
    use pdt_core::pm::{RotateReason, Rotation};
    let closing =
        env.state.store.set_pm_rotation(&pm, Rotation::None, Rotation::Journal, Some(RotateReason::User), now());
    assert!(closing.await.unwrap());
    outcome(&env, &t, "submitted", "During the journal").await;
    pdt_api::tick_watches(&env.state).await;
    let n = watch_notifications(&env.app).await;
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(n[0]["body"].as_str().unwrap().contains("was closed, so you're told instead"), "{}", n[0]);
    let queued = env.state.store.waiting_session_messages().await.unwrap();
    assert!(queued.iter().all(|m| !m.text.starts_with("[PDT] Watched")), "nothing into the closing conversation");
    assert!(watches_of(&env, &t).await.is_empty(), "the watch ended");
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;
}

#[tokio::test]
async fn a_once_watch_of_a_finished_target_is_refused() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let (s, _) = call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "done"}))).await;
    assert!(s.is_success());
    let (s, v) = call(&env.app, "POST", "/api/watches", Some(json!({"ticket": t}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("already_finished")), "{v}");
    let (s, v) = call(&env.app, "POST", "/api/watches", Some(json!({"ticket": t, "once": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[tokio::test]
async fn watches_follow_a_reset_conversation() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let old = start_pm(&env, "who").await;
    wait_said(&env, &old, 1).await;
    ask(&env, &old, r#"mcp: watch {"ticket": "PX-1", "events": ["outcome"]}"#).await;
    // Reset, ticking the engine (and the watches) while it waits for the journal.
    let (app, url) = (env.app.clone(), format!("/api/pm/sessions/{old}/reset"));
    let reset = tokio::spawn(async move { call(&app, "POST", &url, None).await });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !reset.is_finished() {
        assert!(Instant::now() < deadline, "the reset never answered");
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, v) = reset.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{v}");
    let fresh = v["id"].as_str().unwrap().to_string();
    let w = watches_of(&env, &t).await;
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["sessionId"], fresh.as_str(), "the watch carried over");

    outcome(&env, &t, "submitted", "Carried over").await;
    let t1 = wait_told(&env, &fresh, 1).await;
    assert!(t1[0].ends_with("the Coder reported `submitted`: Carried over."), "{t1:?}");
    assert!(watch_notifications(&env.app).await.is_empty(), "not the user: the fresh conversation");

    // Moved by hand too (anything else that puts a conversation in another's place).
    let other = start_pm(&env, "who").await;
    wait_said(&env, &other, 1).await;
    pdt_api::follow_watches(&env.state, &fresh, &other).await;
    assert_eq!(watches_of(&env, &t).await[0]["sessionId"], other.as_str());
    for id in [old, fresh, other] {
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

#[tokio::test]
async fn the_user_watches_a_ticket_and_a_restart_catches_up_once() {
    let env = env().await;
    let t = ticket_id(&env).await;
    let (s, w) = call(&env.app, "POST", "/api/watches", Some(json!({"ticket": "PX-1"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!((w["watcher"].as_str(), w["once"].as_bool()), (Some("you"), Some(true)));
    assert_eq!(w["events"], json!(["finished", "failed", "needs_you"]), "the defaults");
    let (s, again) = call(&env.app, "POST", "/api/watches", Some(json!({"ticket": t, "once": false}))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(again["id"], w["id"], "one watch per watcher");
    let (s, _) = call(&env.app, "POST", "/api/watches", Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // An outcome isn't one of its events; Done is.
    outcome(&env, &t, "submitted", "x").await;
    moved(&env, &t, "done").await;
    // PDT was down: a second daemon on the same database looks.
    let store = pdt_store::Store::open(&env._tmp.path().join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, env._tmp.path().join("data2"));
    state.watch_wait_ms = (0, 0);
    let app = pdt_api::router(state.clone(), None);
    pdt_api::tick_watches(&state).await;
    pdt_api::tick_watches(&state).await;
    let n = watch_notifications(&app).await;
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0]["title"], "PX-1 “Fix the login”: moved to Done");
    assert_eq!(watches_of(&env, &t).await.len(), 1, "once: false keeps watching");
    pdt_api::tick_watches(&env.state).await;
    assert_eq!(watch_notifications(&env.app).await.len(), 1, "told once, by either daemon");

    let id = w["id"].as_str().unwrap();
    let (s, _) = call(&env.app, "DELETE", &format!("/api/watches/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(watches_of(&env, &t).await.is_empty());
    let (s, _) = call(&env.app, "DELETE", &format!("/api/watches/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_watched_run_tells_when_it_waits_on_you_and_when_it_ends() {
    let mut env = common::workflows::setup(vec![]).await;
    env.state.watch_wait_ms = (0, 0);
    let wf = common::workflows::create(
        &env,
        json!({"name": "Ship", "nodes": [
            {"id": "t", "kind": "manual", "version": 1},
            {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Go?", "choices": ["yes", "no"]}}],
            "edges": [{"from": "t", "to": "ask"}]}),
    )
    .await;
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let run = r["runId"].as_str().unwrap().to_string();
    let (s, w) = call(&env.app, "POST", "/api/watches", Some(json!({"run": run}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!(w["targetLabel"], "Ship");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let n = watch_notifications(&env.app).await;
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0]["title"], "the run of “Ship”: waits on you: Go?");
    assert_eq!(n[0]["target"]["kind"], "workflow");

    let (s, v) = call(&env.app, "POST", &format!("/api/workflow-runs/{run}/cancel"), None).await;
    assert!(s.is_success(), "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let n = watch_notifications(&env.app).await;
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n.iter().any(|n| n["title"] == "the run of “Ship”: was cancelled"), "{n:?}");
    let (_, left) = call(&env.app, "GET", &format!("/api/watches?run={run}"), None).await;
    assert_eq!(left, json!([]), "the run finished: the watch ended");
}
