//! The workflow engine through the API (Phase 7a): catalog, documents and versions, runs (waits, gates, retries, error
//! paths, sub-workflows, run keys, state), dry runs, the event log, connections, and ticket comments to the agent.

use crate::common;

use axum::http::StatusCode;
use common::{call, git};
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

/// A stand-in agent that keeps reading input (a multi-line message is several lines; one `read` would exit on the
/// first and fail the Enter that follows).
fn reading_agent(dir: &std::path::Path) -> std::path::PathBuf {
    let (file, body) = if cfg!(windows) {
        ("reader.cmd", "@echo off\r\n:again\r\nset /p x=\r\ngoto again\r\n")
    } else {
        ("reader", "#!/bin/sh\nwhile read x; do :; done\n")
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
    state.platform.claude_program = reading_agent(&bin).to_string_lossy().into_owned();
    state.typing_quiet = std::time::Duration::ZERO;
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
    assert_eq!(builtins.len(), 8, "no integration's built-ins until it is enabled: {list}");
    // PDT's own development's are not built-ins (PDT-295): neither installed nor offered.
    assert!(builtins.contains(&("github-issue-done", true)) && !builtins.iter().any(|b| b.0 == "pdt-bug-triage"));
    let (_, templates) = call(&env.app, "GET", "/api/workflow-templates", None).await;
    assert!(!templates.to_string().contains("builtin:pdt-bug-triage"), "{templates}");
    let (s, _) = call(&env.app, "GET", "/api/workflow-templates/builtin:pdt-bug-triage", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(builtins.contains(&("late-commits", true)) && !builtins.iter().any(|b| b.0 == "cut-release"));
    // Cut a release is a template since PDT-301, made per project.
    assert!(templates.to_string().contains("builtin:cut-release"), "{templates}");
    assert!(builtins.contains(&("pr-notices", true)) && builtins.contains(&("base-moved", false)));
    // Auto-start is a built-in (PDT-327), on from the start; each project's settings decide what it does.
    assert!(builtins.contains(&("pick-up-tickets", true)));
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

/// PDT-301: a Run now trigger's declared inputs: a start gets their defaults, a field hidden by its showIf is left
/// out, and unknown or invalid input is refused naming the fields; bad declarations don't validate.
#[tokio::test]
async fn run_now_checks_the_declared_inputs() {
    let env = setup().await;
    let doc = json!({"name": "deploy", "nodes": [
        {"id": "t", "kind": "manual", "version": 1, "config": {"runKey": "{{trigger.env}}-{{trigger.day}}", "inputs": [
            {"id": "env", "label": "Environment", "type": "choice", "default": "staging", "required": true,
             "choices": [{"value": "staging"}, {"value": "production", "label": "Production"}]},
            {"id": "day", "type": "number", "default": 1},
            {"id": "why", "type": "text", "required": true, "showIf": {"field": "env", "equals": "production"}}],
         "confirm": {"text": "Deploy to {{trigger.env}}?", "showIf": {"field": "env", "equals": "production"}}}},
        {"id": "done", "kind": "end", "version": 1, "config": {"output": "{{trigger}}"}}],
        "edges": [{"from": "t", "to": "done"}]});
    let id = create(&env, doc.clone()).await;
    let r1 = run_now(&env, &id, Value::Null).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &r1).await["trigger"], json!({"env": "staging", "day": 1}), "the defaults");

    let url = format!("/api/workflows/{id}/run");
    for (input, says) in [
        (json!({"environment": "staging"}), "there is no input environment"),
        (json!({"env": "prod"}), "env must be staging or production"),
        (json!({"env": "production"}), "why (why) is required"),
        (json!({"day": "soon"}), "day must be a number"),
    ] {
        let (s, e) = call(&env.app, "POST", &url, Some(json!({"input": input}))).await;
        assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("cannot_run")), "{e}");
        let msg = e["message"].as_str().unwrap();
        assert!(msg.contains(says), "{msg}");
        assert!(
            msg.contains(
                "env (staging or production, required), day (a number), why (text, required, when env is production)"
            ),
            "{msg}"
        );
    }
    let r2 = run_now(&env, &id, json!({"env": "production", "why": "fix", "day": "2", "projectId": env.project})).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &r2).await;
    assert_eq!(r["trigger"], json!({"env": "production", "day": 2, "why": "fix", "projectId": env.project}));
    assert_eq!(r["runKey"], "production-2", "the key reads the checked input");

    // A declaration that makes no sense is an error in validation.
    let mut bad = doc.clone();
    bad["nodes"][0]["config"]["inputs"][2]["showIf"]["field"] = json!("later");
    let (_, issues) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": bad}))).await;
    assert!(issues.to_string().contains("showIf names later, which is no earlier field"), "{issues}");
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
    // Resume is for a run that stopped at an agent step (PDT-123), not a failed action.
    assert!(run(&env, &id).await["resumeFrom"].is_null());
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{id}/resume"), None).await;
    assert_eq!((s, r["code"].as_str()), (StatusCode::CONFLICT, Some("cannot_resume")), "{r}");
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
    assert_eq!(run["steps"][1]["nodeId"], "asked", "never set to merge: just the notice");
    assert_eq!(run["steps"][2]["nodeId"], "tell");
    assert_eq!(run["steps"][2]["input"]["text"], "[PDT] fix it");
    assert!(run["steps"][2]["output"]["would"].as_str().unwrap().contains("[PDT] fix it"), "{run}");
    assert!(env.state.store.waiting_session_messages().await.unwrap().is_empty(), "nothing was queued");

    // A closed ticket's merge doesn't tell the agent.
    let event =
        json!({"type": "attempt.merged", "payload": {"via": "closed", "notice": {"key": "merged", "text": "t"}}});
    let (_, r) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": id, "event": event}))).await;
    assert_eq!(r["run"]["triggers"][0]["matched"], false, "{r}");
    assert!(r["run"]["status"].is_null());
}

/// A PR notice nobody was told does not spend its key (PDT-129): the timeline's `pr` events count only when told.
#[tokio::test]
async fn an_untold_pr_notice_does_not_spend_its_key() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-notices").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let tell = async |key: &str| {
        let event = json!({"type": "pr.conflict", "payload": {"attemptId": "a1", "ticketId": ticket,
                           "notice": {"key": key, "text": "[PDT] fix it"}}});
        let (s, r) =
            call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": id, "event": event}))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        let steps = r["run"]["steps"].as_array().unwrap().clone();
        steps.iter().find(|s| s["nodeId"] == "tell").unwrap()["port"].as_str().unwrap().to_string()
    };
    for (key, told) in [("conflict:untold", false), ("conflict:told", true)] {
        let data = json!({"attemptId": "a1", "notice": "conflict", "key": key, "told": told, "sessionId": null});
        env.state
            .store
            .add_ticket_event(
                &ticket,
                pdt_core::ticket::EventKind::Pr,
                pdt_core::ticket::Actor::System,
                None,
                Some(data),
                1,
            )
            .await
            .unwrap();
    }
    assert_eq!(tell("conflict:untold").await, "sent", "nobody was told: it goes again");
    assert_eq!(tell("conflict:told").await, "skipped", "told once is enough");
}

/// PDT-104: a pull request that was set to merge gets an agent on its failure (unless one is on it already), a CI pass
/// with auto-merge off merges it again, and after 3 tries a comment instead.
#[tokio::test]
async fn a_dry_run_covers_each_case_of_a_pr_set_to_merge() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let id = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pr-notices").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let dry = async |kind: &str, extra: Value| {
        let mut payload = json!({"attemptId": "a1", "ticketId": "t1", "pr": 12,
                                 "notice": {"key": "k", "text": "[PDT] it failed"}});
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        let event = json!({"type": kind, "payload": payload});
        let (s, r) =
            call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": id, "event": event}))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        r["run"].clone()
    };
    let path = |run: &Value| -> Vec<String> {
        run["steps"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|s| format!("{}:{}", s["nodeId"].as_str().unwrap(), s["port"].as_str().unwrap_or("-")))
            .collect()
    };

    // Never asked to merge: a notice, as before.
    let run = dry("ci.run_failed", json!({})).await;
    assert_eq!(path(&run), ["ci:out", "asked:false", "tell:sent"], "{run}");

    // Set to merge: CI failed, a conflict, the queue dropped it, the base moved: the Coder fixes it, then it merges.
    let fix = ["count:out", "over:false", "fix:submitted", "remerge:queued", "merging:-"];
    for (kind, node) in
        [("ci.run_failed", "ci"), ("pr.conflict", "conflict"), ("pr.queue_left", "queue"), ("pr.behind", "behind")]
    {
        let run = dry(kind, json!({"mergeRequested": true})).await;
        let p = path(&run);
        assert_eq!(p[..3], [format!("{node}:out"), "asked:true".into(), "busy:no".into()], "{kind}: {run}");
        assert_eq!(p[3..], fix, "{kind}: {run}");
        let step = run["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "fix").unwrap();
        let prompt = step["input"]["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("[PDT] it failed") && prompt.contains("set to merge"), "{prompt}");
        assert!(step["output"]["would"].as_str().unwrap().contains("Coder (reused session)"), "{step}");
    }
    // An agent is on it already: it hears the news instead of a second step.
    let run = dry("pr.conflict", json!({"mergeRequested": true, "agentWorking": true})).await;
    assert_eq!(path(&run), ["conflict:out", "asked:true", "busy:yes", "tell:sent"], "{run}");

    // CI passes with auto-merge off: merge again. With it on, in the queue, or an agent still on it: nothing.
    let run = dry("ci.run_passed", json!({"mergeRequested": true, "notice": null})).await;
    assert_eq!(path(&run), ["passed:out", "remerge:queued", "merging:-"], "{run}");
    for extra in [json!({"autoMerge": true}), json!({"inMergeQueue": true}), json!({"agentWorking": true}), json!({})] {
        let mut e = json!({"mergeRequested": true});
        for (k, v) in extra.as_object().unwrap() {
            e[k] = v.clone();
        }
        if extra.as_object().unwrap().is_empty() {
            e["mergeRequested"] = json!(false);
        }
        let run = dry("ci.run_passed", e).await;
        assert!(run["status"].is_null(), "{extra}: {run}");
    }

    // Out of tries (3 so far): a comment the first time over, then only the notice.
    let key = "tries.a1";
    env.state.store.set_workflow_state(&id, key, &json!(3), false, 1).await.unwrap();
    let run = dry("ci.run_failed", json!({"mergeRequested": true})).await;
    assert_eq!(path(&run)[3..], ["count:out", "over:true", "first_over:true", "limit_note:out", "tell:sent"], "{run}");
    env.state.store.set_workflow_state(&id, key, &json!(4), false, 1).await.unwrap();
    let run = dry("ci.run_failed", json!({"mergeRequested": true})).await;
    assert_eq!(path(&run)[3..], ["count:out", "over:true", "first_over:false", "tell:sent"], "{run}");
    assert_eq!(env.state.store.workflow_state(&id).await.unwrap()[key], json!(4), "a dry run counts nothing");
}

#[tokio::test]
async fn connections_keep_their_secret_in_the_keychain() {
    let env = setup().await;
    let (s, c) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "hook", "kind": "http"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap();
    let (s, e) = call(&env.app, "POST", "/api/connections", Some(json!({"name": "hook"}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("connection_exists")));
    let (s, c) =
        call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": "https://hook"}))).await;
    assert_eq!((s, c["hasSecret"].as_bool()), (StatusCode::OK, Some(true)));
    assert!(!c.to_string().contains("https://hook"), "never returned");
    assert_eq!(env.state.secrets.get(&format!("connection:{id}")).unwrap().as_deref(), Some("https://hook"));
    let (_, c) =
        call(&env.app, "PATCH", &format!("/api/connections/{id}"), Some(json!({"url": "https://example.com"}))).await;
    assert_eq!(c["url"], "https://example.com");
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
    let runs = async || {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={ticket}"), None).await;
        runs.as_array().unwrap().iter().filter(|r| r["workflowName"] == "Ticket comments to the agent").count()
    };
    // An agent's own comment (no `by`: any API caller but PDT's UI) isn't sent back to it.
    let (s, c) =
        call(&env.app, "POST", &format!("/api/tickets/{ticket}/comments"), Some(json!({"body": "Done the API."})))
            .await;
    assert_eq!((s, c["actor"].as_str()), (StatusCode::CREATED, Some("agent")), "{c}");
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(runs().await, 0);
    let (s, _) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/comments"),
        Some(json!({"body": "Also update the docs.", "by": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(runs().await, 1);

    let (_, events) = call(&env.app, "GET", &format!("/api/events?ticket={ticket}"), None).await;
    let kinds: Vec<&str> = events.as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"ticket.commented") && kinds.contains(&"attempt.started"), "{kinds:?}");
    // The message waits until the agent is ready (idle, nobody typing), which can take a few ticks.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?ticket={ticket}"), None).await;
        let r = runs
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["workflowName"] == "Ticket comments to the agent")
            .cloned()
            .unwrap();
        if r["status"] != "waiting" || std::time::Instant::now() > deadline {
            break r["status"].clone();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        pdt_api::run_workflows(&env.state).await;
    };
    assert_eq!(status, "succeeded");
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
    assert_eq!((t["stage"].as_str(), t["attempt"]["state"].as_str()), (Some("done"), Some("merged")), "{t}");
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

    // PDT-97: a finish whose caller goes away once the ticket is Done (an agent finishing its own ticket is stopped
    // by it) still removes the worktree.
    let (c, _, c_path) = start("Self").await;
    let (app, path) = (env.app.clone(), format!("/api/tickets/{c}/finish"));
    let req = tokio::spawn(async move { call(&app, "POST", &path, Some(json!({"stopRunning": true}))).await });
    for _ in 0..200 {
        let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{c}"), None).await;
        if t["stage"] == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    req.abort();
    for _ in 0..200 {
        if !c_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!c_path.exists(), "the worktree is removed after the request is dropped");

    // A Done ticket whose worktree is still there (kept, or left by an older daemon) can still be finished.
    let r = finish(&a, json!({"stopRunning": true})).await;
    assert_eq!((r["outcome"].as_str(), r["worktreeRemoved"].as_bool()), (Some("finished"), Some(true)), "{r}");
    assert!(!a_path.exists());
    env.state.ptys.shutdown().await;
}

/// Finishing a ticket ends its active attempt (else it stays active with no worktree: counted against the cap,
/// polled, and a later Start refused) and cancels its runs still going.
#[tokio::test]
async fn finishing_a_ticket_ends_its_attempt_and_cancels_its_runs() {
    let env = setup().await;
    let wf = create(&env, gate_doc("asks about the ticket")).await;
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Finish me"})))
            .await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, st) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{st}");
    let waiting = run_now(&env, &wf, json!({"ticketId": ticket})).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &waiting).await["status"], "waiting");

    let (_, r) = call(&env.app, "POST", &format!("/api/tickets/{ticket}/finish"), Some(json!({}))).await;
    assert_eq!(r["outcome"], "finished", "{r}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{ticket}"), None).await;
    assert_eq!((t["stage"].as_str(), t["attempt"]["state"].as_str()), (Some("done"), Some("merged")), "{t}");
    let r = run(&env, &waiting).await;
    assert_eq!(r["status"], "cancelled", "{r}");
    assert!(r["error"].as_str().unwrap().contains("finished"), "{r}");
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(run(&env, &waiting).await["status"], "cancelled", "the engine leaves it ended");

    // Back to Ready, it starts again.
    call(&env.app, "PATCH", &format!("/api/tickets/{ticket}"), Some(json!({"stage": "ready"}))).await;
    let (s, st) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{st}");
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

fn gate_doc(name: &str) -> Value {
    json!({"name": name, "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "g", "kind": "gate", "version": 1, "config": {"question": "q", "choices": ["yes", "no"]}}],
        "edges": [{"from": "t", "to": "g"}]})
}

#[tokio::test]
async fn check_gives_issues_and_each_nodes_ports() {
    let env = setup().await;
    let (s, c) = call(&env.app, "POST", "/api/workflows/check", Some(json!({"doc": gate_doc("w")}))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let ids: Vec<&str> = c["ports"]["g"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["yes", "no", "unanswered", "timeout"]);
    assert_eq!(c["ports"]["t"][0]["title"], "Then");
    assert!(c["issues"].as_array().unwrap().iter().all(|i| i["severity"] == "warning"), "{c}");
}

#[tokio::test]
async fn an_agents_change_is_proposed_until_the_user_accepts_it() {
    let env = setup().await;
    let id = create(&env, gate_doc("mine")).await;
    let put = |doc: Value, propose: bool| {
        let path = format!("/api/workflows/{id}");
        let app = env.app.clone();
        async move { call(&app, "PUT", &path, Some(json!({"doc": doc, "note": "n", "propose": propose}))).await }
    };
    let (s, w) = put(gate_doc("agent's"), true).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!((w["version"].as_u64(), w["proposed"].as_u64(), w["name"].as_str()), (Some(1), Some(2), Some("mine")));
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{id}/versions"), None).await;
    assert_eq!((versions[0]["status"].as_str(), versions[0]["author"].as_str()), (Some("proposed"), Some("agent")));
    let (s, _) = call(&env.app, "POST", &format!("/api/workflows/{id}/versions/1/accept"), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "version 1 is not a proposal");
    let (s, w) = call(&env.app, "POST", &format!("/api/workflows/{id}/versions/2/accept"), None).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!((w["version"].as_u64(), w["proposed"].as_u64(), w["name"].as_str()), (Some(2), None, Some("agent's")));
    put(gate_doc("again"), true).await;
    let (s, w) = call(&env.app, "POST", &format!("/api/workflows/{id}/versions/3/discard"), None).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!((w["version"].as_u64(), w["proposed"].as_u64()), (Some(2), None));
    let (s, _) = put(json!({"name": "", "nodes": []}), true).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "a proposal is validated too");
}

#[tokio::test]
async fn a_proposal_waits_in_the_inbox_until_it_is_accepted_discarded_or_replaced() {
    let env = setup().await;
    let id = create(&env, gate_doc("mine")).await;
    let propose = |name: &'static str| {
        let path = format!("/api/workflows/{id}");
        let app = env.app.clone();
        // `by` is the agent's, set by workflow_save: a request can't name who proposed it.
        let body = json!({"doc": gate_doc(name), "note": "tidier", "propose": true, "by": "Someone else"});
        async move { call(&app, "PUT", &path, Some(body)).await }
    };
    let proposals = || {
        let app = env.app.clone();
        async move {
            let (_, list) = call(&app, "GET", "/api/inbox", None).await;
            list["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "proposal").cloned().collect::<Vec<_>>()
        }
    };
    assert!(proposals().await.is_empty());

    propose("agent's").await;
    let items = proposals().await;
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(item["id"], format!("proposal:{id}:2"));
    assert_eq!(item["title"], "An agent proposed version 2 of mine");
    assert_eq!(item["group"], "proposal");
    assert_eq!(
        (item["body"]["workflowId"].as_str(), item["body"]["note"].as_str()),
        (Some(id.as_str()), Some("tidier"))
    );
    assert_eq!(item["body"]["summary"], "name, description or settings changed");
    let choices: Vec<_> = item["choices"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(choices, ["discard", "accept"]);

    // A newer proposal replaces it.
    propose("newer").await;
    let items = proposals().await;
    assert_eq!(items.iter().map(|i| i["id"].as_str().unwrap()).collect::<Vec<_>>(), [format!("proposal:{id}:3")]);
    let (s, _) =
        call(&env.app, "POST", &format!("/api/inbox/proposal:{id}:2/answer"), Some(json!({"choice": "accept"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "the replaced one can't be accepted");
    let (s, _) =
        call(&env.app, "POST", &format!("/api/inbox/proposal:{id}:3/answer"), Some(json!({"choice": "nope"}))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, r) =
        call(&env.app, "POST", &format!("/api/inbox/proposal:{id}:3/answer"), Some(json!({"choice": "accept"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(proposals().await.is_empty(), "accepted: it goes");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!((w["version"].as_u64(), w["name"].as_str()), (Some(3), Some("newer")));

    propose("again").await;
    assert_eq!(proposals().await.len(), 1);
    let (s, r) =
        call(&env.app, "POST", &format!("/api/inbox/proposal:{id}:4/answer"), Some(json!({"choice": "discard"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(proposals().await.is_empty(), "discarded: it goes");
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{id}/versions"), None).await;
    assert_eq!((versions[0]["status"].as_str(), versions[0]["proposedBy"].as_str()), (Some("discarded"), None));
}

#[tokio::test]
async fn templates_are_listed_saved_and_instantiated_disabled() {
    let env = setup().await;
    let (_, list) = call(&env.app, "GET", "/api/workflow-templates", None).await;
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"builtin:blank") && ids.contains(&"builtin:ask-finish"), "{ids:?}");
    let (s, _) = call(&env.app, "DELETE", "/api/workflow-templates/builtin:blank", None).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // A built-in with a typed param: the boolean stays a boolean.
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:ask-finish/instantiate",
        Some(
            json!({"params": {"deleteBranch": true}, "name": "Ask here", "scope": "project", "projectId": env.project}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!(
        (w["name"].as_str(), w["enabled"].as_bool(), w["scope"].as_str()),
        (Some("Ask here"), Some(false), Some("project"))
    );
    let finish = w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "finish").unwrap().clone();
    assert_eq!(finish["config"]["deleteBranch"], json!(true));
    assert_eq!(finish["config"]["ticket"], json!("{{trigger.ticketId}}"), "other references are left alone");

    // The user's own: checked with placeholders, then instantiated with a required param.
    let doc = json!({"name": "Say {{params.word}}", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "e", "kind": "end", "version": 1, "config": {"quiet": "{{params.quiet}}"}}],
        "edges": [{"from": "t", "to": "e"}]});
    let tpl = json!({"name": "Mine", "params": [
        {"id": "word", "title": "Word", "type": "string", "required": true},
        {"id": "quiet", "title": "Quiet", "type": "boolean", "default": true}], "doc": doc});
    let (s, t) = call(&env.app, "POST", "/api/workflow-templates", Some(tpl.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let tid = t["id"].as_str().unwrap().to_string();
    let inst = format!("/api/workflow-templates/{tid}/instantiate");
    let (s, e) = call(&env.app, "POST", &inst, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (s, w) = call(&env.app, "POST", &inst, Some(json!({"params": {"word": "hi"}}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!((w["name"].as_str(), w["doc"]["nodes"][1]["config"]["quiet"].as_bool()), (Some("Say hi"), Some(true)));
    let mut bad = tpl.clone();
    bad["doc"]["nodes"][1]["kind"] = json!("nope");
    let (s, _) = call(&env.app, "PUT", &format!("/api/workflow-templates/{tid}"), Some(bad)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let mut renamed = tpl;
    renamed["name"] = json!("Mine 2");
    let (s, t) = call(&env.app, "PUT", &format!("/api/workflow-templates/{tid}"), Some(renamed)).await;
    assert_eq!((s, t["name"].as_str()), (StatusCode::OK, Some("Mine 2")));
    let (s, _) = call(&env.app, "DELETE", &format!("/api/workflow-templates/{tid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

/// The Pipeline as PDT shipped it before PDT-124: a refused merge waited for the merge, with no time limit.
fn older_pipeline(doc: &Value) -> Value {
    let gone = ["fixable", "fix", "fix_review", "blocked", "not_merged"];
    let mut d = doc.clone();
    d["nodes"].as_array_mut().unwrap().retain(|n| !gone.contains(&n["id"].as_str().unwrap()));
    for n in d["nodes"].as_array_mut().unwrap() {
        if n["id"] == "wait_merge" {
            let c = n["config"].as_object_mut().unwrap();
            c.remove("timeoutMs");
            c.remove("pipelineStage");
        }
        if n["id"] == "merge" {
            n["config"].as_object_mut().unwrap().remove("pipelineStage");
        }
    }
    let edges = d["edges"].as_array_mut().unwrap();
    edges.retain(|e| {
        !gone.contains(&e["from"].as_str().unwrap())
            && !gone.contains(&e["to"].as_str().unwrap())
            && !(e["from"] == "wait_merge" && e["port"] == "timeout")
    });
    edges.push(json!({"from": "merge", "port": "refused", "to": "wait_merge"}));
    edges.push(json!({"from": "wait_merge", "port": "timeout", "to": "stopped"}));
    d
}

/// PDT-124: a workflow made from PDT's Pipeline template, never changed since, follows the template's newer versions
/// with the params it was made with (one made before workflows recorded their template too); one the user changed
/// stays theirs.
#[tokio::test]
async fn untouched_pipelines_follow_the_newer_template_and_edited_ones_stay() {
    use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
    let env = setup().await;
    let params = json!({"coder": "Builder", "accept": "auto", "autoAcceptLabels": ["agentic"], "loopLimit": 5});
    let instantiate = || async {
        let body = json!({"params": params, "scope": "project", "projectId": env.project});
        let (s, w) = call(&env.app, "POST", "/api/workflow-templates/builtin:pipeline/instantiate", Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{w}");
        w
    };
    let newest = instantiate().await;
    let old = older_pipeline(&newest["doc"]);
    assert_ne!(old, newest["doc"]);
    let old_doc: pdt_core::workflow::doc::WorkflowDoc = serde_json::from_value(old.clone()).unwrap();
    let store = &env.state.store;

    // Made before PDT recorded templates: version 1, as instantiating wrote it.
    let legacy = |id: &str| Workflow {
        id: id.into(),
        name: "Pipeline".into(),
        description: String::new(),
        scope: Scope::Project,
        project_id: Some(env.project.clone()),
        enabled: true,
        builtin: None,
        source: Source::Host,
        repo_file: None,
        version: 1,
        proposed: None,
        created_at: 1,
        updated_at: 1,
    };
    store.insert_workflow(&legacy("old"), &old_doc, Author::User, "from the template Pipeline").await.unwrap();
    // The same, then changed by the user.
    store.insert_workflow(&legacy("edited"), &old_doc, Author::User, "from the template Pipeline").await.unwrap();
    let mut mine = old_doc.clone();
    mine.description = "Mine now".into();
    store.save_workflow_version("edited", &mine, Author::User, "mine", 2).await.unwrap();
    // Made recently (its template recorded), then given an older template by PDT.
    let recorded = instantiate().await["id"].as_str().unwrap().to_string();
    store.save_workflow_version(&recorded, &old_doc, Author::Builtin, "updated by PDT", 3).await.unwrap();

    // pdtd starts again.
    let restart = || async {
        let store = Store::open(&env._tmp.path().join("pdt.db")).await.unwrap();
        let state = AppState::new(store, env._tmp.path().join("data"));
        pdt_api::run_workflows(&state).await;
        router(state, None)
    };
    let app = restart().await;
    let get =
        |app: axum::Router, id: String| async move { call(&app, "GET", &format!("/api/workflows/{id}"), None).await.1 };
    let followed = get(app.clone(), "old".into()).await;
    assert_eq!(
        (followed["version"].as_u64(), &followed["doc"]),
        (Some(2), &newest["doc"]),
        "the newer template, its params"
    );
    assert_eq!(
        followed["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "fix").unwrap()["config"]["persona"],
        "Builder"
    );
    let (_, versions) = call(&app, "GET", "/api/workflows/old/versions", None).await;
    assert_eq!(versions[0]["author"], "builtin", "{versions}");
    let again = get(app.clone(), recorded.clone()).await;
    assert_eq!((again["version"].as_u64(), &again["doc"]), (Some(3), &newest["doc"]));
    let edited = get(app.clone(), "edited".into()).await;
    assert_eq!(
        (edited["version"].as_u64(), edited["doc"]["description"].as_str()),
        (Some(2), Some("Mine now")),
        "left alone"
    );

    // Once is enough: another start writes nothing.
    let app = restart().await;
    assert_eq!(get(app.clone(), "old".into()).await["version"], 2);
    assert_eq!(get(app, recorded).await["version"], 3);
}

/// Runs the engine until the run stops moving (at most a few ticks).
async fn settle(env: &Env, id: &str) -> Value {
    for _ in 0..5 {
        pdt_api::run_workflows(&env.state).await;
    }
    run(env, id).await
}

#[tokio::test]
async fn a_wait_takes_an_event_that_came_before_it_started_waiting() {
    let env = setup().await;
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    let tid = t["id"].as_str().unwrap().to_string();
    let wf = create(
        &env,
        json!({"name": "looks back", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "qa", "kind": "gate", "version": 1, "config": {"question": "QA done?", "choices": ["yes"]}},
        {"id": "w", "kind": "wait", "version": 1, "config": {"event": {"type": "ticket.commented",
            "where": [{"left": "{{event.ticketId}}", "op": "eq", "right": "{{trigger.ticket}}"}]}}},
        {"id": "again", "kind": "wait", "version": 1, "config": {"timeoutMs": 600000, "event": {"type": "ticket.commented",
            "where": [{"left": "{{event.ticketId}}", "op": "eq", "right": "{{trigger.ticket}}"}]}}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "{{steps.again.body}}"}}],
        "edges": [{"from": "t", "to": "qa"}, {"from": "qa", "port": "yes", "to": "w"},
                  {"from": "w", "port": "event", "to": "again"}, {"from": "again", "port": "event", "to": "e"}]}),
    )
    .await;
    // Before the run: not for it.
    call(&env.app, "POST", &format!("/api/tickets/{tid}/comments"), Some(json!({"body": "too early"}))).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let id = run_now(&env, &wf, json!({"ticket": tid})).await;
    pdt_api::run_workflows(&env.state).await;
    // While the run is at the gate, the comment comes and is processed with nobody waiting for it.
    call(&env.app, "POST", &format!("/api/tickets/{tid}/comments"), Some(json!({"body": "during QA"}))).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &id).await;
    let gate = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "qa").unwrap()["id"].as_str().unwrap();
    let (s, _) =
        call(&env.app, "POST", &format!("/api/workflow-runs/{id}/steps/{gate}/answer"), Some(json!({"choice": "yes"})))
            .await;
    assert_eq!(s, StatusCode::OK);
    // The first wait takes it at once; the second, a loop's next wait, doesn't take it again.
    let r = settle(&env, &id).await;
    assert_eq!(r["status"], "waiting", "{r}");
    let w = r["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == "w").unwrap();
    assert_eq!((w["port"].as_str(), w["output"]["body"].as_str()), (Some("event"), Some("during QA")), "{r}");
    call(&env.app, "POST", &format!("/api/tickets/{tid}/comments"), Some(json!({"body": "next"}))).await;
    let r = settle(&env, &id).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("next")), "{r}");
}

#[tokio::test]
async fn a_step_that_ended_without_its_next_steps_queued_is_recovered() {
    let env = setup().await;
    let wf = create(
        &env,
        json!({"name": "crashy", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "ask", "kind": "gate", "version": 1, "config": {"question": "Go?", "choices": ["go"]}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "went"}}],
        "edges": [{"from": "t", "to": "ask"}, {"from": "ask", "port": "go", "to": "e"}]}),
    )
    .await;
    let id = run_now(&env, &wf, json!({})).await;
    pdt_api::run_workflows(&env.state).await;
    // What a PDT before single-write completion could leave: the gate written as answered, then a crash before its
    // next step was queued and its run woken (a waiting run with no wake time).
    let store = &env.state.store;
    let mut gate = store.list_steps(&id).await.unwrap().into_iter().find(|s| s.node_id == "ask").unwrap();
    gate.status = pdt_core::workflow::api::StepStatus::Succeeded;
    gate.port = Some("go".into());
    gate.ended_at = Some(gate.started_at.unwrap() + 1);
    store.put_step(&gate).await.unwrap();
    let mut r = store.get_workflow_run(&id).await.unwrap().unwrap();
    assert_eq!(r.status, pdt_core::workflow::api::RunStatus::Waiting);
    r.wake_at = None;
    store.update_workflow_run(&r).await.unwrap();

    let r = settle(&env, &id).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("went")), "{r}");
    assert_eq!(ports(&r), ["t:out", "ask:go", "e:-"]);
}

#[tokio::test]
async fn deleting_a_workflow_ends_its_runs_and_closes_their_inbox_items() {
    let env = setup().await;
    let child = create(&env, gate_doc("child")).await;
    let parent = create(
        &env,
        json!({"name": "parent", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "call", "kind": "workflow.call", "version": 1, "config": {"workflow": child}},
        {"id": "e", "kind": "end", "version": 1, "config": {"output": "the child is gone"}}],
        "edges": [{"from": "t", "to": "call"}, {"from": "call", "port": "failed", "to": "e"}]}),
    )
    .await;
    let id = run_now(&env, &parent, json!({})).await;
    let r = settle(&env, &id).await;
    assert_eq!(r["status"], "waiting", "{r}");
    let open = |v: &Value| v["items"].as_array().map_or(0, |i| i.len());
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(open(&inbox), 1, "the child's gate: {inbox}");

    let (s, _) = call(&env.app, "DELETE", &format!("/api/workflows/{child}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    assert_eq!(open(&inbox), 0, "{inbox}");
    let r = settle(&env, &id).await;
    assert_eq!((r["status"].as_str(), &r["output"]), (Some("succeeded"), &json!("the child is gone")), "{r}");
}

#[tokio::test]
async fn a_gate_is_titled_by_its_first_line_and_has_no_pull_request_of_its_own() {
    let env = setup().await;
    let question = "**Ship [v1.2](https://example.com/notes)?**\n\nThe notes:\n- one\n- two";
    let id = create(
        &env,
        json!({"name": "ask", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "g", "kind": "gate", "version": 1, "config": {"question": question, "choices": ["yes"]}}],
        "edges": [{"from": "t", "to": "g"}]}),
    )
    .await;
    let run = run_now(&env, &id, json!({})).await;
    assert_eq!(settle(&env, &run).await["status"], "waiting");
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let gate = &inbox["items"][0];
    assert_eq!(gate["title"], "Ship v1.2?", "{gate}");
    assert_eq!(gate["body"]["question"], question, "{gate}");
    assert!(gate["changeRequest"].is_null(), "{gate}");
}

#[tokio::test]
async fn the_runtime_limit_ends_a_waiting_run_and_counts_from_a_retry() {
    // Long enough that a tick on a busy machine stays well inside it: the test waits it out once, not tick by tick.
    const LIMIT: i64 = 3000;
    let env = setup().await;
    let settings = json!({"maxSteps": 200, "maxRuntimeMs": LIMIT});
    let mut doc = gate_doc("slow");
    doc["settings"] = settings.clone();
    let slow = create(&env, doc).await;
    // A run that fails at once, to be retried after its limit.
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    let doc = json!({"name": "mover", "settings": settings, "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "move", "kind": "ticket.move", "version": 1, "config": {"ticket": "{{state.ticket}}", "stage": "todo"}}],
        "edges": [{"from": "t", "to": "move"}]});
    let mover = create(&env, doc).await;
    let waits = run_now(&env, &slow, json!({})).await;
    let fails = run_now(&env, &mover, json!({})).await;
    pdt_api::run_workflows(&env.state).await;
    let r = run(&env, &waits).await;
    assert_eq!(r["status"], "waiting", "{r}");
    assert_eq!(
        r["wakeAt"].as_i64().unwrap(),
        r["startedAt"].as_i64().unwrap() + LIMIT + 1,
        "it wakes at its limit: {r}"
    );
    assert_eq!(run(&env, &fails).await["status"], "failed");

    tokio::time::sleep(Duration::from_millis(u64::try_from(LIMIT).unwrap() + 100)).await;
    let r = settle(&env, &waits).await;
    assert_eq!(r["status"], "failed", "{r}");
    assert!(r["error"].as_str().unwrap().contains("took longer"), "{r}");

    // Past its limit too, a retried run gets its time again.
    env.state.store.set_workflow_state(&mover, "ticket", &t["id"], false, 1).await.unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflow-runs/{fails}/retry"), None).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let r = settle(&env, &fails).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert!(r["resumedAt"].as_i64().is_some());
}

#[tokio::test]
async fn a_comment_step_run_again_after_a_restart_comments_once() {
    let env = setup().await;
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "x"}))).await;
    let tid = t["id"].as_str().unwrap().to_string();
    let wf = create(
        &env,
        json!({"name": "commenter", "nodes": [
        {"id": "t", "kind": "manual", "version": 1},
        {"id": "c", "kind": "ticket.comment", "version": 1, "config": {"ticket": "{{trigger.ticket}}", "body": "hello"}}],
        "edges": [{"from": "t", "to": "c"}]}),
    )
    .await;
    let id = run_now(&env, &wf, json!({"ticket": tid})).await;
    assert_eq!(settle(&env, &id).await["status"], "succeeded");
    // As if PDT stopped after the comment was written but before its step was: the step is found running.
    let store = &env.state.store;
    let mut c = store.list_steps(&id).await.unwrap().into_iter().find(|s| s.node_id == "c").unwrap();
    c.status = pdt_core::workflow::api::StepStatus::Running;
    c.port = None;
    c.ended_at = None;
    store.put_step(&c).await.unwrap();
    let mut r = store.get_workflow_run(&id).await.unwrap().unwrap();
    r.status = pdt_core::workflow::api::RunStatus::Running;
    r.ended_at = None;
    store.update_workflow_run(&r).await.unwrap();
    assert_eq!(settle(&env, &id).await["status"], "succeeded");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{tid}/events"), None).await;
    let comments = events["items"].as_array().unwrap().iter().filter(|e| e["kind"] == "comment").count();
    assert_eq!(comments, 1, "{events}");
}

/// A Draft ticket made from outside input (PDT-201), as an issue import makes it.
async fn external_ticket(env: &Env, title: &str) -> String {
    let t = pdt_core::ticket::NewTicket { title: title.into(), ..Default::default() }.validate().unwrap();
    let t = pdt_core::ticket::ValidNewTicket { source: pdt_core::ticket::TicketSource::External, ..t };
    let id = uuid::Uuid::now_v7().to_string();
    env.state.store.create_ticket(&id, &env.project, &t, 1).await.unwrap();
    id
}

#[tokio::test]
async fn a_workflow_never_puts_an_external_ticket_in_to_do() {
    let env = setup().await;
    let t = external_ticket(&env, "From an issue").await;
    let mover = |stage: &str, instead: Option<&str>| {
        let mut config = json!({"ticket": "{{trigger.ticket}}", "stage": stage});
        if let Some(i) = instead {
            config["externalStage"] = json!(i);
        }
        json!({"name": format!("to {stage}"), "nodes": [
            {"id": "t", "kind": "manual", "version": 1},
            {"id": "move", "kind": "ticket.move", "version": 1, "config": config}],
            "edges": [{"from": "t", "to": "move"}]})
    };
    // To do and In progress are refused, with why.
    for stage in ["todo", "in_progress"] {
        let wf = create(&env, mover(stage, None)).await;
        let r = settle(&env, &run_now(&env, &wf, json!({"ticket": t})).await).await;
        assert_eq!(r["status"], "failed", "{r}");
        assert!(r.to_string().contains("only the user"), "{r}");
    }
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    assert_eq!(
        (v["stage"].as_str(), v["source"].as_str(), v["admitted"].as_bool()),
        (Some("draft"), Some("external"), Some(false))
    );
    // With somewhere else to go (the Pipeline's afterRefine), it goes there instead.
    let wf = create(&env, mover("todo", Some("ready"))).await;
    let r = settle(&env, &run_now(&env, &wf, json!({"ticket": t})).await).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{t}"), None).await;
    assert_eq!(v["stage"], "ready");

    // Making one in To do from outside input is refused too; a workflow's own tickets are `workflow`'s.
    let (_, made) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Mine"}))).await;
    assert_eq!(made["source"], "user");
    let wf = create(
        &env,
        json!({"name": "maker", "nodes": [
            {"id": "t", "kind": "manual", "version": 1},
            {"id": "make", "kind": "ticket.create", "version": 1,
             "config": {"project": env.project, "title": "Follow-up", "stage": "todo"}}],
            "edges": [{"from": "t", "to": "make"}]}),
    )
    .await;
    let r = settle(&env, &run_now(&env, &wf, json!({})).await).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}&q=Follow-up", env.project), None).await;
    assert_eq!(
        (list["items"][0]["source"].as_str(), list["items"][0]["stage"].as_str()),
        (Some("workflow"), Some("todo"))
    );

    // The user moving it to To do lets it in: from then on workflows move it as any other.
    let (s, v) =
        common::call_as_ui(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "todo"}))).await;
    assert_eq!((s, v["admitted"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "ready"}))).await;
    let wf = create(&env, mover("todo", None)).await;
    let r = settle(&env, &run_now(&env, &wf, json!({"ticket": t})).await).await;
    assert_eq!(r["status"], "succeeded", "{r}");
}
