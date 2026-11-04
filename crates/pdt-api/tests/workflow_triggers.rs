//! Workflow triggers on the Inbox, sessions and workflow runs (PDT-364): each event fires once with its payload, a
//! workflow never reacts to its own runs unless its trigger says so, and many turns in a row are all matched.

use crate::common;

use axum::http::StatusCode;
use common::wait::{still_after, wait_until};
use common::workflows::{Env, create, runs, setup};
use common::{call, call_as_ui};
use pdt_core::workflow::api::LogEvent;
use serde_json::{Value, json};
use std::time::Duration;

async fn events(env: &Env, kind: &str) -> Vec<LogEvent> {
    let f = pdt_store::EventFilter { kind: Some(kind.into()), limit: 1000, ..Default::default() };
    env.state.store.list_log_events(&f).await.unwrap()
}

/// Runs the engine until nothing is running or waiting to run (at most `ticks` times).
async fn settle(env: &Env, ticks: usize) {
    for _ in 0..ticks {
        pdt_api::run_workflows(&env.state).await;
    }
}

/// A workflow started by `trigger` (its config `config`) that ends at once.
fn on(name: &str, trigger: &str, config: Value) -> Value {
    json!({"name": name, "nodes": [
        {"id": "t", "kind": trigger, "version": 1, "config": config},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "e"}]})
}

/// A workflow whose step fails (it moves a ticket that doesn't exist), started by `trigger`.
fn failing(name: &str, trigger: &str, config: Value) -> Value {
    json!({"name": name, "nodes": [
        {"id": "t", "kind": trigger, "version": 1, "config": config},
        {"id": "move", "kind": "ticket.move", "version": 1, "config": {"ticket": "nope", "stage": "done"}}],
        "edges": [{"from": "t", "to": "move"}]})
}

async fn start(env: &Env, wf: &str) -> String {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r["runId"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_failed_run_is_reported_once_and_the_reporter_never_reports_itself() {
    let env = setup(vec![]).await;
    let fails = create(&env, failing("fails", "manual", Value::Null)).await;
    // The reporter fails too: its own failure must not start it again.
    let reporter = create(&env, failing("reporter", "run.failed", json!({}))).await;
    let id = start(&env, &fails).await;
    settle(&env, 6).await;

    let failed = events(&env, "run.failed").await;
    let of = |wf: &str| failed.iter().filter(|e| e.payload["workflowId"] == wf).count();
    assert_eq!((of(&fails), of(&reporter)), (1, 1), "each failure once: {failed:?}");
    let e = failed.iter().find(|e| e.payload["workflowId"] == fails).unwrap();
    let p = &e.payload;
    assert_eq!(
        (&p["workflowName"], &p["runId"], &p["status"], &p["failedNode"], &p["failedKind"], &p["triggerType"]),
        (&json!("fails"), &json!(id), &json!("failed"), &json!("move"), &json!("ticket.move"), &json!("manual"))
    );
    assert!(p["error"].as_str().unwrap().starts_with("move:"), "{p}");
    assert!(p["failedError"].as_str().unwrap().contains("no such ticket"), "{p}");
    assert!(p["durationMs"].as_i64().unwrap() >= 0 && p["chain"] == 0, "{p}");
    assert_eq!(p["endNode"], "move");

    let reported = runs(&env, &reporter).await;
    assert_eq!(reported.len(), 1, "the reporter ran for `fails` only: {reported:?}");
    assert_eq!(reported[0]["trigger"]["workflowName"], "fails");
    assert!(events(&env, "run.finished").await.iter().all(|e| e.payload["workflowId"] != fails));
}

#[tokio::test]
async fn a_finished_run_says_where_it_ended_and_include_own_stops_after_a_short_chain() {
    let env = setup(vec![]).await;
    let ok = create(&env, on("ok", "manual", Value::Null)).await;
    let only = json!({"where": [{"left": "{{trigger.workflowName}}", "op": "in", "right": ["ok", "echo"]}]});
    let watcher = create(&env, on("watcher", "run.finished", only.clone())).await;
    let mut echo_config = only;
    echo_config["includeOwn"] = json!(true);
    let echo = create(&env, on("echo", "run.finished", echo_config)).await;
    start(&env, &ok).await;
    settle(&env, 12).await;

    let finished = events(&env, "run.finished").await;
    let e = finished.iter().find(|e| e.payload["workflowId"] == ok).unwrap();
    assert_eq!(
        (&e.payload["status"], &e.payload["endNode"], &e.payload["endKind"], &e.payload["failedNode"]),
        (&json!("succeeded"), &json!("e"), &json!("end"), &Value::Null)
    );
    // The watcher saw `ok` and `echo`'s runs, never its own.
    let watched = runs(&env, &watcher).await;
    assert!(watched.iter().all(|r| r["trigger"]["workflowId"] != watcher), "{watched:?}");
    assert!(watched.iter().any(|r| r["trigger"]["workflowId"] == ok));
    // Echo reacts to its own runs too, but a chain of runs started by run events stops at three.
    let echoed = runs(&env, &echo).await;
    let chains: Vec<u64> = echoed.iter().filter_map(|r| r["trigger"]["chain"].as_u64()).collect();
    let mut sorted = chains.clone();
    sorted.sort();
    assert!(sorted.contains(&0) && sorted.iter().all(|c| *c < 3), "{chains:?}");
    assert!(!finished.iter().any(|e| e.payload["chain"].as_u64() >= Some(3)), "{finished:?}");
    // Nothing is left to start.
    let before = runs(&env, &echo).await.len();
    settle(&env, 3).await;
    assert_eq!(runs(&env, &echo).await.len(), before);
}

#[tokio::test]
async fn an_inbox_item_fires_once_when_it_opens_and_once_when_it_is_answered() {
    let env = setup(vec![]).await;
    let asks = create(
        &env,
        json!({"name": "asks", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Go?", "choices": ["yes", "no"]}},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "ask"}, {"from": "ask", "port": "yes", "to": "e"}]}),
    )
    .await;
    let opened = create(&env, on("opened", "inbox.created", json!({}))).await;
    let answered = create(&env, on("answered", "inbox.answered", json!({}))).await;
    // The gate's own workflow would see its item only with includeOwn.
    let run_id = start(&env, &asks).await;
    settle(&env, 2).await;
    pdt_api::sync_notifications(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    settle(&env, 2).await;

    let created = events(&env, "inbox.created").await;
    assert_eq!(created.len(), 1, "once, however many passes: {created:?}");
    let p = &created[0].payload;
    assert_eq!(
        (&p["kind"], &p["group"], &p["title"], &p["runId"], &p["workflowId"]),
        (&json!("gate"), &json!("gate"), &json!("Go?"), &json!(run_id), &json!(asks))
    );
    assert!(p["ageMs"].as_i64().unwrap() >= 0 && p["itemId"].is_string(), "{p}");
    assert_eq!(runs(&env, &opened).await.len(), 1);

    let item = p["itemId"].as_str().unwrap().to_string();
    let (s, r) =
        call_as_ui(&env.app, "POST", &format!("/api/inbox/{item}/answer"), Some(json!({"choice": "yes"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    settle(&env, 3).await;
    let done = events(&env, "inbox.answered").await;
    assert_eq!(done.len(), 1, "{done:?}");
    let p = &done[0].payload;
    assert_eq!(
        (&p["itemId"], &p["choice"], &p["by"], &p["kind"]),
        (&json!(item), &json!("yes"), &json!("user"), &json!("gate"))
    );
    assert_eq!(runs(&env, &answered).await.len(), 1);
}

#[tokio::test]
async fn a_workflow_sees_its_own_inbox_items_only_with_include_own() {
    let env = setup(vec![]).await;
    let gate = |name: &str, include_own: bool| {
        json!({"name": name, "nodes": [
            {"id": "t", "kind": "manual", "version": 1},
            {"id": "ask", "kind": "gate", "version": 1, "config": {"question": name, "choices": ["yes"]}},
            {"id": "seen", "kind": "inbox.created", "version": 1, "config": {"includeOwn": include_own,
                "where": [{"left": "{{trigger.title}}", "op": "eq", "right": name}]}},
            {"id": "e", "kind": "end", "version": 1}],
            "edges": [{"from": "t", "to": "ask"}, {"from": "seen", "to": "e"}]})
    };
    let mine = create(&env, gate("mine", false)).await;
    let all = create(&env, gate("all", true)).await;
    start(&env, &mine).await;
    start(&env, &all).await;
    settle(&env, 2).await;
    pdt_api::sync_notifications(&env.state).await;
    settle(&env, 2).await;
    assert_eq!(runs(&env, &mine).await.len(), 1, "only the run Run now started");
    assert_eq!(runs(&env, &all).await.len(), 2, "and the one its own item started");
}

/// An agent session in a project, at `activity`.
async fn agent_session(env: &Env, id: &str) -> String {
    let repo = env.tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    crate::common::init_repo(&repo);
    let (_, p) =
        call(&env.app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    let project = p["id"].as_str().unwrap_or_default().to_string();
    let project = if project.is_empty() {
        let (_, all) = call(&env.app, "GET", "/api/projects", None).await;
        all[0]["id"].as_str().unwrap().to_string()
    } else {
        project
    };
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
    env.state
        .store
        .insert_session(&Session {
            id: id.into(),
            project_id: project.clone(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: "Fix the build".into(),
            cwd: repo.to_string_lossy().into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: Some(pdt_core::activity::Activity::Idle),
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
        })
        .await
        .unwrap();
    project
}

async fn hook(env: &Env, session: &str, event: &str, extra: Value) {
    let mut r = json!({"source": "claude", "event": event});
    if let (Some(r), Some(extra)) = (r.as_object_mut(), extra.as_object()) {
        r.extend(extra.clone());
    }
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{session}/activity"), Some(r)).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
}

/// Waits for `n` events of `kind` (session events are logged off the request's task).
async fn wait_for(env: &Env, kind: &str, n: usize) -> Vec<LogEvent> {
    wait_until(&format!("{n} {kind}"), async || {
        let e = events(env, kind).await;
        if e.len() >= n { Ok(e) } else { Err(e) }
    })
    .await
}

#[tokio::test]
async fn a_session_needing_the_user_and_ending_a_turn_fires_once_each() {
    let env = setup(vec![]).await;
    let project = agent_session(&env, "c1").await;
    hook(&env, "c1", "UserPromptSubmit", json!({})).await;
    hook(&env, "c1", "Notification", json!({"notificationType": "permission_prompt"})).await;
    hook(&env, "c1", "Notification", json!({"notificationType": "permission_prompt"})).await;
    let needs = wait_for(&env, "session.needs_you", 1).await;
    hook(&env, "c1", "Stop", json!({})).await;
    let ended = wait_for(&env, "session.turn_ended", 1).await;
    // A second prompt while waiting is the same wait.
    still_after("a second session.needs_you or session.turn_ended", Duration::from_millis(100), async || {
        events(&env, "session.needs_you").await.len() != 1 || events(&env, "session.turn_ended").await.len() != 1
    })
    .await;
    for e in [&needs[0], &ended[0]] {
        let p = &e.payload;
        assert_eq!(
            (&p["sessionId"], &p["title"], &p["projectId"], &p["agent"], &p["mode"], &p["workflowStep"]),
            (
                &json!("c1"),
                &json!("Fix the build"),
                &json!(project),
                &json!("claude"),
                &json!("terminal"),
                &json!(false)
            )
        );
        assert!(p["lastMessage"].is_string() && p["workflowId"].is_null(), "{p}");
        assert_eq!(e.session_id.as_deref(), Some("c1"));
    }
    // Idle again with nothing going on: no turn ended.
    hook(&env, "c1", "Stop", json!({})).await;
    still_after("a second session.turn_ended", Duration::from_millis(100), async || {
        events(&env, "session.turn_ended").await.len() != 1
    })
    .await;
}

#[tokio::test]
async fn many_turns_are_each_logged_and_matched() {
    let env = setup(vec![]).await;
    agent_session(&env, "c1").await;
    let wf = create(&env, on("turns", "session.turn_ended", json!({}))).await;
    const TURNS: usize = 300;
    for _ in 0..TURNS {
        hook(&env, "c1", "UserPromptSubmit", json!({})).await;
        hook(&env, "c1", "Stop", json!({})).await;
    }
    let logged = wait_for(&env, "session.turn_ended", TURNS).await;
    assert_eq!(logged.len(), TURNS, "each turn once");
    // The engine takes 200 new events a tick: two ticks match them all, each starting one run.
    let started = std::time::Instant::now();
    settle(&env, 4).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "took {:?}", started.elapsed());
    let (_, all) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}&limit=1000"), None).await;
    assert_eq!(all.as_array().unwrap().len(), TURNS);
    // Every turn was matched (what is left are the runs' own `run.finished` events, logged as they ended).
    let left = env.state.store.unprocessed_events(1000).await.unwrap();
    assert!(left.iter().all(|e| e.kind != "session.turn_ended"), "{left:?}");
}

/// An agent that prints its last words and exits with code 3.
fn exiting_agent(dir: &std::path::Path) -> std::path::PathBuf {
    let (file, body) = if cfg!(windows) {
        ("exits.cmd", "@echo off\r\necho All done: the build is green.\r\nexit /b 3\r\n")
    } else {
        ("exits", "#!/bin/sh\necho 'All done: the build is green.'\nsleep 0.2\nexit 3\n")
    };
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[tokio::test]
async fn a_session_exiting_says_its_code_and_whether_pdt_stopped_it() {
    let mut env = setup(vec![]).await;
    let bin = env.tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    env.state.platform.claude_program = exiting_agent(&bin).to_string_lossy().into_owned();
    env.state.platform.claude_config_dir = Some(env.tmp.path().join("claude-config"));
    env.app = pdt_api::router(env.state.clone(), None);
    let project = agent_session(&env, "unused").await;
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": project, "kind": "agent_tui", "agent": "claude", "mode": "terminal", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let exited = wait_for(&env, "session.exited", 1).await;
    let p = &exited[0].payload;
    assert_eq!((&p["sessionId"], &p["exitCode"], &p["stoppedByPdt"]), (&json!(id), &json!(3), &json!(false)), "{p}");
    assert!(p["lastMessage"].as_str().unwrap().contains("the build is green"), "{p}");
    env.state.ptys.shutdown().await;
}
