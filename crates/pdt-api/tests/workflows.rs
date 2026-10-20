//! The workflow engine through the API (Phase 7a): catalog, documents and versions, runs (waits, gates, retries, error
//! paths, sub-workflows, run keys, state), dry runs, the event log, connections, and ticket comments to the agent.

mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Proj", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

async fn create(env: &Env, doc: Value) -> String {
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn run_now(env: &Env, wf: &str, input: Value) -> String {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r["runId"].as_str().unwrap().to_string()
}

async fn run(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

fn ports(run: &Value) -> Vec<String> {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| format!("{}:{}", s["nodeId"].as_str().unwrap(), s["port"].as_str().unwrap_or("-")))
        .collect()
}

#[tokio::test]
async fn the_catalog_and_the_builtins_are_served() {
    let env = setup().await;
    let (s, cat) = call(&env.app, "GET", "/api/workflows/catalog", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = cat.as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap()).collect();
    for want in ["ci.run_failed", "attempt.merged", "manual", "gate", "wait", "session.send", "ticket.finish"] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    let send = cat.as_array().unwrap().iter().find(|k| k["id"] == "session.send").unwrap();
    assert!(send["configSchema"]["properties"]["text"].is_object(), "{send}");

    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", &format!("/api/workflows?project={}", env.project), None).await;
    let builtins: Vec<(&str, bool)> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["builtin"].as_str().unwrap(), w["enabled"].as_bool().unwrap()))
        .collect();
    assert_eq!(builtins.len(), 4, "{list}");
    assert!(builtins.contains(&("pr-notices", true)) && builtins.contains(&("base-moved", false)));
    let id = list[0]["id"].as_str().unwrap();
    let (s, e) = call(&env.app, "DELETE", &format!("/api/workflows/{id}"), None).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("builtin_workflow")));
    let (s, w) = call(&env.app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": false}))).await;
    assert_eq!((s, w["enabled"].as_bool()), (StatusCode::OK, Some(false)));
}

#[tokio::test]
async fn documents_are_validated_and_versioned() {
    let env = setup().await;
    let bad = json!({"name": "bad", "nodes": [{"id": "x", "kind": "nope", "version": 1}]});
    let (s, e) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": bad}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("workflow_invalid")), "{e}");
    let (_, issues) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": bad}))).await;
    let codes: Vec<&str> = issues.as_array().unwrap().iter().map(|i| i["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"unknown_kind") && codes.contains(&"no_trigger"), "{codes:?}");

    let doc = json!({"name": "one", "nodes": [{"id": "t", "kind": "manual", "version": 1},
        {"id": "e", "kind": "end", "version": 1, "config": {"quiet": true}}], "edges": [{"from": "t", "to": "e"}]});
    let id = create(&env, doc.clone()).await;
    let mut doc2 = doc.clone();
    doc2["name"] = json!("two");
    doc2["nodes"][1]["config"]["quiet"] = json!(false);
    let (s, w) =
        call(&env.app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc2, "note": "loud"}))).await;
    assert_eq!((s, w["version"].as_u64(), w["name"].as_str()), (StatusCode::OK, Some(2), Some("two")), "{w}");
    let (_, vs) = call(&env.app, "GET", &format!("/api/workflows/{id}/versions"), None).await;
    assert_eq!(vs.as_array().unwrap().len(), 2);
    assert_eq!(vs[0]["note"], "loud");
    let (_, v1) = call(&env.app, "GET", &format!("/api/workflows/{id}/versions/1"), None).await;
    assert_eq!(v1["doc"]["name"], "one");
    let (_, d) = call(&env.app, "GET", &format!("/api/workflows/{id}/diff"), None).await;
    assert_eq!(d["nodesChanged"], json!(["e"]), "{d}");
    assert!(d["metaChanged"].as_bool().unwrap());
    assert!(d["unified"].as_str().unwrap().contains("-  \"name\": \"one\""));
}

#[tokio::test]
async fn a_manual_run_branches_remembers_state_and_honours_its_key() {
    let env = setup().await;
    let doc = json!({"name": "branchy", "nodes": [
        {"id": "t", "kind": "manual", "version": 1, "config": {"runKey": "day:{{trigger.day}}"}},
        {"id": "big", "kind": "condition", "version": 1, "config": {"conditions": [{"left": "{{trigger.n}}", "op": "gt", "right": 3}]}},
        {"id": "kind", "kind": "switch", "version": 1, "config": {"value": "{{trigger.color}}", "cases": [{"port": "red", "equals": "red"}]}},
        {"id": "keep", "kind": "state.set", "version": 1, "config": {"key": "first", "value": "{{trigger.n}}", "ifMissing": true}},
        {"id": "done", "kind": "end", "version": 1, "config": {"output": {"n": "{{trigger.n}}", "first": "{{state.first}}"}}}],
        "edges": [{"from": "t", "to": "big"}, {"from": "big", "port": "true", "to": "kind"},
                  {"from": "kind", "port": "red", "to": "keep"}, {"from": "keep", "to": "done"}]});
    let id = create(&env, doc).await;
    let r1 = run_now(&env, &id, json!({"n": 5, "color": "red", "day": 1})).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &r1).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(ports(&r), ["t:out", "big:true", "kind:red", "keep:out", "done:-"]);
    assert_eq!(r["output"], json!({"n": 5, "first": 5}));

    let (_, again) = call(
        &env.app,
        "POST",
        &format!("/api/workflows/{id}/run"),
        Some(json!({"input": {"n": 9, "color": "red", "day": 1}})),
    )
    .await;
    assert!(again["runId"].is_null(), "same run key: skipped");

    let r2 = run_now(&env, &id, json!({"n": 7, "color": "red", "day": 2})).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &r2).await["output"], json!({"n": 7, "first": 5}), "the baseline stays");
    let (_, state) = call(&env.app, "GET", &format!("/api/workflows/{id}/state"), None).await;
    assert_eq!(state, json!({"first": 5}));

    let r3 = run_now(&env, &id, json!({"n": 1, "day": 3})).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &r3).await;
    assert_eq!((r["status"].as_str(), ports(&r)), (Some("quiet"), vec!["t:out".to_string(), "big:false".to_string()]));
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={id}"), None).await;
    assert_eq!(runs.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn failures_retry_take_the_error_path_or_fail_and_can_be_retried() {
    let env = setup().await;
    let failing = |error_edge: bool| {
        let mut edges = vec![json!({"from": "t", "to": "move"})];
        if error_edge {
            edges.push(json!({"from": "move", "port": "error", "to": "oops"}));
        }
        json!({"name": format!("fails {error_edge}"), "nodes": [
            {"id": "t", "kind": "manual", "version": 1},
            {"id": "move", "kind": "ticket.move", "version": 1, "retry": {"max": 1, "backoffMs": 0},
             "config": {"ticket": "{{trigger.ticket}}", "stage": "done"}},
            {"id": "oops", "kind": "end", "version": 1, "config": {"output": "{{steps.move.error}}"}}],
            "edges": edges})
    };
    let plain = create(&env, failing(false)).await;
    let id = run_now(&env, &plain, json!({"ticket": "nope"})).await;
    pdt_api::run_workflows(&env.state).await;
    let mut r = run(&env, &id).await;
    for _ in 0..20 {
        if r["status"] != "running" && r["status"] != "waiting" {
            break;
        }
        pdt_api::run_workflows(&env.state).await;
        r = run(&env, &id).await;
    }
    assert_eq!(r["status"], "failed", "{r}");
    assert_eq!(r["steps"][1]["attempts"], 2, "one retry");
    assert!(r["error"].as_str().unwrap().starts_with("move:"), "{r}");

    let with_edge = create(&env, failing(true)).await;
    let id = run_now(&env, &with_edge, json!({"ticket": "nope"})).await;
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
    }
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(ports(&r).last().unwrap(), "oops:-");
    assert!(r["output"].as_str().unwrap().contains("no such ticket"), "{r}");
}

#[tokio::test]
async fn retry_resumes_a_failed_run_from_its_failed_step() {
    let env = setup().await;
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    let tid = t["id"].as_str().unwrap().to_string();
    let doc = json!({"name": "mover", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "move", "kind": "ticket.move", "version": 1, "config": {"ticket": "{{state.ticket}}", "stage": "todo"}}],
        "edges": [{"from": "t", "to": "move"}]});
    let wf = create(&env, doc).await;
    let id = run_now(&env, &wf, json!({})).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &id).await["status"], "failed");
    // Give the step what it needs, then retry it.
    env.state.store.set_workflow_state(&wf, "ticket", &json!(tid), false, 1).await.unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/retry"), None).await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::OK, Some("running")), "{r}");
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{tid}"), None).await;
    assert_eq!(t["stage"], "todo");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{tid}/events"), None).await;
    let moved = events["items"].as_array().unwrap().iter().find(|e| e["kind"] == "moved").unwrap();
    assert_eq!((moved["actor"].as_str(), moved["data"]["runId"].as_str()), (Some("system"), Some(id.as_str())));
}

#[tokio::test]
async fn gates_wait_for_an_answer_and_timers_and_sub_workflows_resume() {
    let env = setup().await;
    let child = create(
        &env,
        json!({"name": "child", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": {"doubled": "{{trigger.n}}{{trigger.n}}"}}}],
        "edges": [{"from": "t", "to": "e"}]}),
    )
    .await;
    let parent = create(&env, json!({"name": "parent", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "nap", "kind": "wait", "version": 1, "config": {"ms": 0}},
        {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Go?", "choices": ["yes", "no"],
            "ahead": [{"when": [{"left": "{{trigger.pre}}", "op": "truthy"}], "choice": "yes"}]}},
        {"id": "call", "kind": "workflow.call", "version": 1, "config": {"workflow": child, "input": {"n": "{{trigger.n}}"}}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "{{steps.call.doubled}}"}}],
        "edges": [{"from": "t", "to": "nap"}, {"from": "nap", "port": "done", "to": "ask"},
                  {"from": "ask", "port": "yes", "to": "call"}, {"from": "call", "port": "done", "to": "e"}]}))
    .await;

    let id = run_now(&env, &parent, json!({"n": 4})).await;
    pdt_api::run_workflows(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "waiting", "{r}");
    let gate = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "ask").unwrap().clone();
    assert_eq!((gate["status"].as_str(), gate["wait"]["answer"]["question"].as_str()), (Some("waiting"), Some("Go?")));
    let gid = gate["id"].as_str().unwrap();
    let (s, e) = call(
        &env.app,
        "POST",
        &format!("/api/workflow-runs/{id}/steps/{gid}/answer"),
        Some(json!({"choice": "maybe"})),
    )
    .await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("cannot_answer")));
    let (s, _) =
        call(&env.app, "POST", &format!("/api/workflow-runs/{id}/steps/{gid}/answer"), Some(json!({"choice": "yes"})))
            .await;
    assert_eq!(s, StatusCode::OK);
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let r = run(&env, &id).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("44")), "{r}");
    let (_, kids) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={child}"), None).await;
    assert_eq!(kids[0]["parentRunId"], json!(id));

    // Answered ahead of time: no wait at the gate.
    let id = run_now(&env, &parent, json!({"n": 1, "pre": true})).await;
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let gate = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "ask").unwrap();
    assert_eq!(gate["output"]["answeredAhead"], true);

    // A waiting run can be cancelled.
    let id = run_now(&env, &parent, json!({"n": 1})).await;
    pdt_api::run_workflows(&env.state).await;
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/cancel"), None).await;
    assert_eq!((s, r["status"].as_str()), (StatusCode::OK, Some("cancelled")), "{r}");
    assert!(r["steps"].as_array().unwrap().iter().any(|s| s["status"] == "skipped"));
}

#[tokio::test]
async fn a_dry_run_says_what_the_pr_notices_would_do() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-notices").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let event = json!({"type": "pr.conflict", "payload": {"attemptId": "a1", "ticketId": "t1", "notice": {"key": "conflict:h", "text": "[PDT] fix it"}}});
    let (s, r) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": id, "event": event}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let run = &r["run"];
    assert_eq!(run["triggers"][0], json!({"node": "conflict", "matched": true, "reason": null}));
    assert_eq!(run["steps"][1]["nodeId"], "tell");
    assert_eq!(run["steps"][1]["input"]["text"], "[PDT] fix it");
    assert!(run["steps"][1]["output"]["would"].as_str().unwrap().contains("[PDT] fix it"), "{run}");
    assert!(env.state.store.waiting_session_messages().await.unwrap().is_empty(), "nothing was queued");

    // A closed ticket's merge doesn't tell the agent.
    let event =
        json!({"type": "attempt.merged", "payload": {"via": "closed", "notice": {"key": "merged", "text": "t"}}});
    let (_, r) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": id, "event": event}))).await;
    assert_eq!(r["run"]["triggers"][0]["matched"], false, "{r}");
    assert!(r["run"]["status"].is_null());
}

#[tokio::test]
async fn connections_keep_their_secret_in_the_keychain() {
    let env = setup().await;
    let (s, c) =
        call(&env.app, "POST", "/api/connections", Some(json!({"name": "discord", "kind": "discord_webhook"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap();
    let (s, e) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "discord"}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("connection_exists")));
    let (s, c) =
        call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": "https://hook"}))).await;
    assert_eq!((s, c["hasSecret"].as_bool()), (StatusCode::OK, Some(true)));
    assert!(!c.to_string().contains("https://hook"), "never returned");
    assert_eq!(env.state.secrets.get(&format!("connection:{id}")).unwrap().as_deref(), Some("https://hook"));
    let (_, c) =
        call(&env.app, "PATCH", &format!("/api/connections/{id}"), Some(json!({"url": "https://discord.com"}))).await;
    assert_eq!(c["url"], "https://discord.com");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/connections/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(env.state.secrets.get(&format!("connection:{id}")).unwrap(), None);
}

#[tokio::test]
async fn a_comment_on_a_started_ticket_goes_to_its_agent_and_events_are_logged() {
    let env = setup().await;
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Ship it"})))
            .await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/comments"),
        Some(json!({"body": "Also update the docs."})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    pdt_api::run_workflows(&env.state).await;

    let (_, events) = call(&env.app, "GET", &format!("/api/events?ticket={ticket}"), None).await;
    let kinds: Vec<&str> = events.as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"ticket.commented") && kinds.contains(&"attempt.started"), "{kinds:?}");
    let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={ticket}"), None).await;
    let r = runs.as_array().unwrap().iter().find(|r| r["workflowName"] == "Ticket comments to the agent").unwrap();
    assert_eq!(r["status"], "succeeded", "{runs}");
    if cfg!(unix) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let screen = || String::from_utf8_lossy(&env.state.ptys.attach(&session).unwrap().scrollback).into_owned();
        while !screen().contains("Also update the docs.") && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(screen().contains("New comment on"), "{}", screen());
    }
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn workflows_in_the_repo_are_read_disabled_and_follow_the_file() {
    let env = setup().await;
    let (_, p) = call(&env.app, "GET", &format!("/api/projects/{}", env.project), None).await;
    let dir = std::path::PathBuf::from(p["repoPath"].as_str().unwrap()).join(".pdt").join("workflows");
    std::fs::create_dir_all(&dir).unwrap();
    let doc = json!({"name": "repo flow", "nodes": [{"id": "t", "kind": "manual", "version": 1}]});
    std::fs::write(dir.join("flow.json"), doc.to_string()).unwrap();
    std::fs::write(dir.join("broken.json"), "{").unwrap();
    pdt_api::maintain_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", &format!("/api/workflows?project={}", env.project), None).await;
    let w = list.as_array().unwrap().iter().find(|w| w["source"] == "repo").unwrap().clone();
    assert_eq!(
        (w["name"].as_str(), w["enabled"].as_bool(), w["repoFile"].as_str()),
        (Some("repo flow"), Some(false), Some(".pdt/workflows/flow.json"))
    );
    let id = w["id"].as_str().unwrap();
    let (s, e) = call(&env.app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("workflow_in_repo")));
    let mut changed = doc.clone();
    changed["description"] = json!("now with words");
    std::fs::write(dir.join("flow.json"), changed.to_string()).unwrap();
    pdt_api::maintain_workflows(&env.state).await;
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!((w["version"].as_u64(), w["description"].as_str()), (Some(2), Some("now with words")));
    std::fs::remove_file(dir.join("flow.json")).unwrap();
    pdt_api::maintain_workflows(&env.state).await;
    let (s, _) = call(&env.app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_run_resumes_in_a_new_daemon_after_a_restart() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "slow", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Go?", "choices": ["go"]}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "went"}}],
        "edges": [{"from": "t", "to": "ask"}, {"from": "ask", "port": "go", "to": "e"}]}),
    )
    .await;
    let id = run_now(&env, &wf, json!({})).await;
    pdt_api::run_workflows(&env.state).await;
    let gate = run(&env, &id).await["steps"][1]["id"].as_str().unwrap().to_string();

    // A second daemon on the same database (the first one is gone).
    let store = Store::open(&env._tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, env._tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, _) =
        call(&app, "POST", &format!("/api/workflow-runs/{id}/steps/{gate}/answer"), Some(json!({"choice": "go"})))
            .await;
    assert_eq!(s, StatusCode::OK);
    pdt_api::run_workflows(&state).await;
    let (_, r) = call(&app, "GET", &format!("/api/workflow-runs/{id}"), None).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("went")), "{r}");
}

#[tokio::test]
async fn finish_checks_first_then_cleans_up_or_only_moves_to_done() {
    let env = setup().await;
    let start = async |title: &str| {
        let (_, t) =
            call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title})))
                .await;
        let id = t["id"].as_str().unwrap().to_string();
        let (s, st) = call(
            &env.app,
            "POST",
            &format!("/api/tickets/{id}/start"),
            Some(json!({"agent": "claude", "sessionMode": "terminal"})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{st}");
        let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
        let wt = t["attempt"]["worktreeId"].as_str().unwrap().to_string();
        let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.project), None).await;
        let path = wts.as_array().unwrap().iter().find(|w| w["id"] == wt.as_str()).unwrap()["path"]
            .as_str()
            .unwrap()
            .to_string();
        (id, st["session"]["id"].as_str().unwrap().to_string(), std::path::PathBuf::from(path))
    };
    let finish =
        async |id: &str, body: Value| call(&env.app, "POST", &format!("/api/tickets/{id}/finish"), Some(body)).await.1;

    let (a, _, a_path) = start("Keep").await;
    let r = finish(&a, json!({})).await;
    assert_eq!((r["outcome"].as_str(), r["blocked"]["sessions"].as_u64()), (Some("blocked"), Some(1)), "{r}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{a}"), None).await;
    assert_eq!(t["stage"], "in_progress", "a blocked finish changes nothing");
    let r = finish(&a, json!({"keepWorktree": true})).await;
    assert_eq!(r["outcome"], "kept", "{r}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{a}"), None).await;
    assert_eq!(t["stage"], "done");
    assert!(a_path.exists());

    let (b, session, b_path) = start("Go").await;
    std::fs::write(b_path.join("scratch.txt"), "x\n").unwrap();
    let r = finish(&b, json!({"stopRunning": true})).await;
    assert_eq!(
        (r["outcome"].as_str(), r["blocked"]["dirty"].as_bool(), r["blocked"]["sessions"].as_u64()),
        (Some("blocked"), Some(true), Some(0)),
        "{r}"
    );
    let r = finish(&b, json!({"stopRunning": true, "discardChanges": true})).await;
    assert_eq!((r["outcome"].as_str(), r["worktreeRemoved"].as_bool()), (Some("finished"), Some(true)), "{r}");
    assert!(!b_path.exists());
    let (s, _) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{b}"), None).await;
    assert_eq!(t["stage"], "done");
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_wait_resumes_when_its_event_comes() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "waits", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "w", "kind": "wait", "version": 1, "config": {"timeoutMs": 600000, "event": {"type": "ticket.commented",
            "where": [{"left": "{{event.ticketId}}", "op": "eq", "right": "{{trigger.ticket}}"}]}}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "{{steps.w.body}}"}}],
        "edges": [{"from": "t", "to": "w"}, {"from": "w", "port": "event", "to": "e"}]}),
    )
    .await;
    let ticket = async |title: &str| {
        let (_, t) =
            call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title})))
                .await;
        t["id"].as_str().unwrap().to_string()
    };
    let (mine, other) = (ticket("mine").await, ticket("other").await);
    let id = run_now(&env, &wf, json!({"ticket": mine})).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!(r["status"], "waiting", "{r}");
    assert!(r["wakeAt"].as_i64().is_some(), "its timeout");
    call(&env.app, "POST", &format!("/api/tickets/{other}/comments"), Some(json!({"body": "not this one"}))).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &id).await["status"], "waiting");
    call(&env.app, "POST", &format!("/api/tickets/{mine}/comments"), Some(json!({"body": "this one"}))).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("this one")), "{r}");
}
