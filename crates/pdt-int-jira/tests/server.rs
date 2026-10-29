//! Jira Server / Data Center (PDT-258) through the API: a connection that signs in with a personal access token (a
//! bearer token, REST v2) and never shows it, JQL pages, transitions with their screens' required fields in the same
//! request, creating, editing, assigning and commenting with custom fields by mapped name, boards, active sprints and
//! backlog sections, epics through the Epic Name and Epic Link fields, and the PM using the same actions. Jira is a
//! fake local server.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use common::call;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const PAT: &str = "server-pat-secret";
const ME: &str = "jdoe";
const POINTS: &str = "customfield_10016";
const SEVERITY: &str = "customfield_10200";
const EPIC_NAME: &str = "customfield_10011";
const EPIC_LINK: &str = "customfield_10014";
const SPRINT: &str = "customfield_10020";

/// A fake Jira Data Center: its issues, and every request it got.
#[derive(Default)]
struct FakeServer {
    issues: Vec<Value>,
    /// Method, path and query, JSON body.
    seen: Vec<(String, String, Value)>,
    /// A request sent anything but the bearer token.
    unauthorized: bool,
    next_key: u32,
}

type Shared = Arc<Mutex<FakeServer>>;

fn issue(key: &str, summary: &str, sprint: Option<Value>) -> Value {
    json!({"key": key, "fields": {
        "summary": summary, "description": format!("About {summary}"),
        "status": {"name": "Open"}, "assignee": {"name": ME, "displayName": "Jane Doe"},
        "reporter": {"name": "ann", "displayName": "Ann"}, "issuetype": {"name": "Story"},
        "priority": {"name": "Major"}, "labels": [], "components": [{"name": "Web"}],
        "created": "2026-10-22T09:00:00.000+0000", "updated": "2026-10-22T09:00:00.000+0000",
        "comment": {"comments": [{"id": "5", "author": {"name": "ann", "displayName": "Ann"},
            "body": "Plain *wiki* text", "created": "2026-10-22T09:30:00.000+0000"}], "total": 1},
        SPRINT: sprint.map(|s| json!([s])), EPIC_LINK: null, POINTS: 3}})
}

fn field(name: &str, kind: &str, required: bool) -> Value {
    json!({"name": name, "required": required, "schema": {"type": kind}})
}

async fn server_app(State(j): State<Shared>, req: Request<Body>) -> (StatusCode, axum::Json<Value>) {
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let path = req.uri().path().trim_start_matches("/jira").to_string();
    let authed = req.headers().get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {PAT}"));
    let bytes = req.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut j = j.lock().unwrap();
    j.seen.push((method.clone(), uri.clone(), body.clone()));
    if !authed {
        j.unauthorized = true;
        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"message": "Client must be authenticated"})));
    }
    let ok = |v: Value| (StatusCode::OK, axum::Json(v));
    let none = || (StatusCode::NO_CONTENT, axum::Json(Value::Null));
    let find = |j: &FakeServer, key: &str| j.issues.iter().position(|i| i["key"] == key);
    let (api, rest) = if let Some(r) = path.strip_prefix("/rest/api/2/") {
        ("api", r)
    } else if let Some(r) = path.strip_prefix("/rest/agile/1.0/") {
        ("agile", r)
    } else {
        return (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["no route"]})));
    };
    let parts: Vec<&str> = rest.split('/').collect();
    match (api, method.as_str(), parts.as_slice()) {
        ("api", "GET", ["myself"]) => ok(json!({"name": ME, "key": "JIRAUSER10", "displayName": "Jane Doe"})),
        ("api", "GET", ["serverInfo"]) => ok(json!({"version": "9.12.0", "deploymentType": "Server"})),
        ("api", "GET", ["field"]) => ok(json!([
            {"id": EPIC_NAME, "schema": {"custom": "com.pyxis.greenhopper.jira:gh-epic-label"}},
            {"id": EPIC_LINK, "schema": {"custom": "com.pyxis.greenhopper.jira:gh-epic-link"}},
            {"id": SPRINT, "schema": {"custom": "com.pyxis.greenhopper.jira:gh-sprint"}}])),
        ("api", "POST", ["search"]) => {
            let start = body["startAt"].as_u64().unwrap_or(0) as usize;
            let max = body["maxResults"].as_u64().unwrap_or(50) as usize;
            let page: Vec<Value> = j.issues.iter().skip(start).take(max).cloned().collect();
            ok(json!({"startAt": start, "maxResults": max, "total": j.issues.len(), "issues": page}))
        }
        ("api", "GET", ["issue", "createmeta", _, "issuetypes"]) => ok(json!({"values": [
            {"id": "10001", "name": "Story"}, {"id": "10000", "name": "Epic"}]})),
        ("api", "GET", ["issue", "createmeta", _, "issuetypes", id]) => {
            let mut values = vec![
                json!({"fieldId": "summary", "name": "Summary", "required": true, "schema": {"type": "string"}}),
                json!({"fieldId": "issuetype", "name": "Issue Type", "required": true, "schema": {"type": "issuetype"}}),
                json!({"fieldId": "project", "name": "Project", "required": true, "schema": {"type": "project"}}),
                json!({"fieldId": "reporter", "name": "Reporter", "required": true, "hasDefaultValue": true,
                    "schema": {"type": "user"}}),
                json!({"fieldId": "priority", "name": "Priority", "required": false, "schema": {"type": "priority"},
                    "allowedValues": [{"id": "2", "name": "Critical"}, {"id": "3", "name": "Major"}]}),
                json!({"fieldId": SEVERITY, "name": "Severity", "required": true, "schema": {"type": "option",
                    "custom": "com.atlassian.jira.plugin.system.customfieldtypes:select"},
                    "allowedValues": [{"id": "10300", "value": "Sev 1"}, {"id": "10301", "value": "Sev 2"}]}),
            ];
            if *id == "10000" {
                values.push(json!({"fieldId": EPIC_NAME, "name": "Epic Name", "required": true,
                    "schema": {"type": "string", "custom": "com.pyxis.greenhopper.jira:gh-epic-label"}}));
            } else {
                values.push(json!({"fieldId": EPIC_LINK, "name": "Epic Link", "required": false,
                    "schema": {"type": "any", "custom": "com.pyxis.greenhopper.jira:gh-epic-link"}}));
            }
            ok(json!({"values": values}))
        }
        ("api", "POST", ["issue"]) => {
            j.next_key += 1;
            let key = format!("OPS-{}", 100 + j.next_key);
            let mut i = issue(&key, body["fields"]["summary"].as_str().unwrap_or_default(), None);
            if let Some(e) = body["fields"][EPIC_LINK].as_str() {
                i["fields"][EPIC_LINK] = json!(e);
            }
            j.issues.push(i);
            (StatusCode::CREATED, axum::Json(json!({"id": "1", "key": key})))
        }
        ("api", "GET", ["issue", key, "editmeta"]) => match find(&j, key) {
            Some(_) => ok(json!({"fields": {
                "summary": field("Summary", "string", true),
                "priority": {"name": "Priority", "required": false, "schema": {"type": "priority"},
                    "allowedValues": [{"id": "2", "name": "Critical"}, {"id": "3", "name": "Major"}]},
                POINTS: {"name": "Story Points", "required": false, "schema": {"type": "number"}},
                SEVERITY: {"name": "Severity", "required": false, "schema": {"type": "option"},
                    "allowedValues": [{"id": "10300", "value": "Sev 1"}, {"id": "10301", "value": "Sev 2"}]},
                EPIC_LINK: {"name": "Epic Link", "required": false,
                    "schema": {"type": "any", "custom": "com.pyxis.greenhopper.jira:gh-epic-link"}}}})),
            None => (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["Issue Does Not Exist"]}))),
        },
        ("api", "GET", ["issue", _, "transitions"]) => ok(json!({"transitions": [
            {"id": "11", "name": "Start Progress", "to": {"name": "In Progress"}, "fields": {}},
            {"id": "51", "name": "Resolve Issue", "to": {"name": "Resolved"}, "fields": {
                "resolution": {"name": "Resolution", "required": true, "schema": {"type": "resolution"},
                    "allowedValues": [{"id": "1", "name": "Fixed"}, {"id": "2", "name": "Won't Fix"}]},
                POINTS: {"name": "Story Points", "required": true, "schema": {"type": "number"}},
                "assignee": {"name": "Assignee", "required": true, "hasDefaultValue": true,
                    "schema": {"type": "user"}}}}]})),
        ("api", "POST", ["issue", key, "transitions"]) => {
            let to = if body["transition"]["id"] == "51" { "Resolved" } else { "In Progress" };
            if let Some(i) = find(&j, key) {
                j.issues[i]["fields"]["status"]["name"] = json!(to);
            }
            none()
        }
        ("api", "GET", ["issue", key]) => match find(&j, key) {
            Some(i) => ok(j.issues[i].clone()),
            None => (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["Issue Does Not Exist"]}))),
        },
        ("api", "PUT", ["issue", key]) => {
            if let Some(i) = find(&j, key) {
                for (k, v) in body["fields"].as_object().cloned().unwrap_or_default() {
                    j.issues[i]["fields"][k] = v;
                }
            }
            none()
        }
        ("api", "PUT", ["issue", key, "assignee"]) => {
            if let Some(i) = find(&j, key) {
                j.issues[i]["fields"]["assignee"] = body.clone();
            }
            none()
        }
        ("api", "GET", ["user", "search"]) => ok(json!([{"name": "bsmith", "displayName": "Bob Smith"}])),
        ("api", "POST", ["issue", _, "comment"]) => (StatusCode::CREATED, axum::Json(json!({"id": "777"}))),
        ("agile", "GET", ["board"]) => ok(json!({"isLast": true, "values": [
            {"id": 7, "name": "Ops board", "type": "scrum", "location": {"projectKey": "OPS"}}]})),
        ("agile", "GET", ["board", "7", "sprint"]) => ok(json!({"isLast": true, "values": [
            {"id": 70, "name": "Sprint 7", "state": "active"},
            {"id": 71, "name": "Backlog: Ready", "state": "future"},
            {"id": 72, "name": "Sprint 8", "state": "future"}]})),
        ("agile", "GET", ["board", "7", "issue"]) => {
            let list: Vec<Value> = j
                .issues
                .iter()
                .map(|i| {
                    let mut i = i.clone();
                    // The Agile API's own `sprint`: the issue's open one.
                    let open = i["fields"][SPRINT].as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
                    i["fields"]["sprint"] = open;
                    i
                })
                .collect();
            ok(json!({"startAt": 0, "total": list.len(), "issues": list}))
        }
        ("agile", "POST", ["sprint", id, "issue"]) => {
            let sprint = match *id {
                "70" => json!({"id": 70, "name": "Sprint 7", "state": "active"}),
                "71" => json!({"id": 71, "name": "Backlog: Ready", "state": "future"}),
                _ => json!({"id": 72, "name": "Sprint 8", "state": "future"}),
            };
            for k in body["issues"].as_array().cloned().unwrap_or_default() {
                if let Some(i) = find(&j, k.as_str().unwrap()) {
                    j.issues[i]["fields"][SPRINT] = json!([sprint]);
                }
            }
            none()
        }
        ("agile", "POST", ["backlog", "issue"]) => {
            for k in body["issues"].as_array().cloned().unwrap_or_default() {
                if let Some(i) = find(&j, k.as_str().unwrap()) {
                    j.issues[i]["fields"][SPRINT] = Value::Null;
                }
            }
            none()
        }
        _ => (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["no route"]}))),
    }
}

async fn fake_server() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = axum::Router::new().fallback(server_app).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/jira"), shared)
}

struct Env {
    app: axum::Router,
    state: pdt_api::AppState,
    jira: Shared,
    project: String,
    conn_id: String,
    _tmp: tempfile::TempDir,
}

fn config() -> Value {
    json!({"deployment": "server", "fields": {"points": POINTS, "severity": SEVERITY},
        "backlogSections": ["Backlog: *"]})
}

/// Adds the `ops-jira` Server connection (with `secret`) to an app.
async fn connect(app: &axum::Router, url: &str, secret: &str) -> String {
    let (s, c) = call(
        app,
        "POST",
        "/api/connections",
        Some(json!({"name": "ops-jira", "kind": "jira", "url": url, "config": config()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap().to_string();
    let (s, _) = call(app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": secret}))).await;
    assert_eq!(s, StatusCode::OK);
    id
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    common::git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    common::git(&repo, &["add", "-A"]);
    common::git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, tmp.path().join("data"));
    state.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_jira::Jira)]));
    state.integrations.enable(&["jira".into()]);
    let app = pdt_api::router(state.clone(), None);
    let (url, jira) = fake_server().await;
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Ops", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let conn_id = connect(&app, &url, PAT).await;
    Env { app, state, jira, project, conn_id, _tmp: tmp }
}

fn manual_then(node: Value) -> Value {
    json!({"name": "Test", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
        "edges": [{"from": "go", "to": node["id"]}]})
}

/// Runs a one-step workflow; its run.
async fn run(env: &Env, kind: &str, config: Value) -> Value {
    let node = json!({"id": "s", "kind": kind, "version": 1, "config": config});
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": manual_then(node), "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await.1
}

fn step(run: &Value) -> Value {
    run["steps"].as_array().unwrap().last().cloned().unwrap()
}

/// A step that succeeded; its output.
async fn out(env: &Env, kind: &str, config: Value) -> Value {
    let r = run(env, kind, config).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    step(&r)["output"].clone()
}

fn seen(env: &Env) -> Vec<(String, String, Value)> {
    env.jira.lock().unwrap().seen.clone()
}

fn sent(env: &Env, method: &str, path: &str) -> Value {
    seen(env)
        .into_iter()
        .rev()
        .find(|(m, p, _)| m == method && p.split('?').next() == Some(path))
        .unwrap_or_else(|| panic!("no {method} {path}"))
        .2
}

#[tokio::test]
async fn a_server_connection_signs_in_with_its_token_and_never_shows_it() {
    let env = setup().await;
    let o = out(&env, "jira.check", json!({"connection": "ops-jira"})).await;
    assert_eq!(
        (o["deployment"].as_str(), o["userId"].as_str(), o["version"].as_str()),
        (Some("server"), Some(ME), Some("9.12.0"))
    );
    assert!(seen(&env).iter().all(|(_, p, _)| p.starts_with("/jira/rest/api/2/")), "REST v2 under the base path");
    assert!(!env.jira.lock().unwrap().unauthorized);
    // The token stays in the keychain: not in the connection, its export, or a run.
    let (_, c) = call(&env.app, "GET", &format!("/api/connections/{}", env.conn_id), None).await;
    assert!(!c.to_string().contains(PAT), "{c}");
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(!list.to_string().contains(PAT));
    // A wrong token: the step fails saying what to fix, without the token.
    call(&env.app, "PUT", &format!("/api/connections/{}/secret", env.conn_id), Some(json!({"secret": "nope"}))).await;
    let r = run(&env, "jira.check", json!({"connection": "ops-jira"})).await;
    assert_eq!(r["status"], "failed", "{r}");
    let text = r.to_string();
    assert!(text.contains("personal access token") && !text.contains("nope") && !text.contains(PAT), "{text}");
    // A deployment PDT doesn't know is refused.
    let (s, e) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "bad", "kind": "jira", "url": "https://x", "config": {"deployment": "mainframe"}})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
}

#[tokio::test]
async fn search_reads_page_after_page_with_placement() {
    let env = setup().await;
    {
        let mut j = env.jira.lock().unwrap();
        j.issues.push(issue("OPS-1", "One", Some(json!({"id": 70, "name": "Sprint 7", "state": "ACTIVE"}))));
        // Server's older form of the Sprint field.
        j.issues.push(issue(
            "OPS-2",
            "Two",
            Some(json!("com.atlassian.greenhopper.service.sprint.Sprint@3f[id=71,rapidViewId=7,state=FUTURE,name=Backlog: Ready]")),
        ));
        j.issues.push(issue("OPS-3", "Three", None));
    }
    let o = out(&env, "jira.search", json!({"connection": "ops-jira", "jql": "project = OPS", "limit": 2})).await;
    assert_eq!(
        (o["issues"].as_array().unwrap().len(), o["next"].clone(), o["total"].clone()),
        (2, json!("2"), json!(3))
    );
    assert_eq!(o["issues"][0]["placement"], json!({"area": "sprint", "sprint": "Sprint 7", "status": "Open"}));
    assert_eq!(o["issues"][1]["placement"]["area"], "section");
    assert_eq!(o["issues"][1]["sprint"]["backlogSection"], true);
    let body = sent(&env, "POST", "/jira/rest/api/2/search");
    assert_eq!((body["startAt"].clone(), body["maxResults"].clone()), (json!(0), json!(2)));
    assert!(body["fields"].as_array().unwrap().contains(&json!(SPRINT)), "the Sprint field, found by its type");
    let o =
        out(&env, "jira.search", json!({"connection": "ops-jira", "jql": "project = OPS", "limit": 2, "page": "2"}))
            .await;
    assert_eq!((o["issues"][0]["key"].clone(), o["next"].clone()), (json!("OPS-3"), Value::Null));
    assert_eq!(o["issues"][0]["placement"]["area"], "backlog");
    // One issue in full: plain-text description and comments, mapped fields by name.
    let o = out(&env, "jira.issue", json!({"connection": "ops-jira", "key": "ops-1"})).await;
    assert_eq!(
        (o["description"].as_str(), o["comments"][0]["body"].as_str()),
        (Some("About One"), Some("Plain *wiki* text"))
    );
    assert_eq!((o["fields"]["points"].clone(), o["components"].clone()), (json!(3), json!(["Web"])));
}

#[tokio::test]
async fn assigned_issues_are_imported_from_a_server_once() {
    let env = setup().await;
    env.jira.lock().unwrap().issues.push(issue("OPS-9", "Import me", None));
    let o = out(&env, "jira.import", json!({"connection": "ops-jira", "key": "OPS-9", "project": env.project})).await;
    let ticket = o["ticketId"].as_str().unwrap().to_string();
    let r = run(&env, "jira.import", json!({"connection": "ops-jira", "key": "OPS-9", "project": env.project})).await;
    assert_eq!((step(&r)["port"].clone(), step(&r)["output"]["ticketId"].clone()), (json!("exists"), json!(ticket)));
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{ticket}"), None).await;
    assert_eq!(t["description"], "About Import me");
    assert!(t.to_string().contains("/jira/browse/OPS-9"), "linked to its source: {t}");
}

#[tokio::test]
async fn a_transition_sends_its_required_fields_in_the_same_request() {
    let env = setup().await;
    env.jira.lock().unwrap().issues.push(issue("OPS-1", "Fix", None));
    let o = out(&env, "jira.transitions", json!({"connection": "ops-jira", "key": "OPS-1"})).await;
    let resolve = o["transitions"].as_array().unwrap().iter().find(|t| t["id"] == "51").unwrap().clone();
    assert!(resolve["fields"].as_array().unwrap().iter().any(|f| f["id"] == "resolution" && f["required"] == true));
    // Missing values: the step fails naming what to give, and nothing moves.
    let r = run(&env, "jira.transition", json!({"connection": "ops-jira", "key": "OPS-1", "to": "Resolved"})).await;
    assert_eq!(r["status"], "failed", "{r}");
    let text = r.to_string();
    assert!(text.contains("Resolution [resolution] (one of Fixed, Won't Fix)"), "{text}");
    assert!(text.contains("Story Points [customfield_10016] (a number)") && !text.contains("Assignee"), "{text}");
    assert!(!seen(&env).iter().any(|(m, p, _)| m == "POST" && p.ends_with("/transitions")));
    // Given (a choice by name, a number as text, by mapped name): one request with the transition and its fields.
    let o = out(
        &env,
        "jira.transition",
        json!({"connection": "ops-jira", "key": "OPS-1", "to": "resolve issue",
            "fields": {"Resolution": "fixed", "points": "5"}}),
    )
    .await;
    assert_eq!((o["transition"].as_str(), o["status"].as_str()), (Some("Resolve Issue"), Some("Resolved")));
    let body = sent(&env, "POST", "/jira/rest/api/2/issue/OPS-1/transitions");
    assert_eq!(body, json!({"transition": {"id": "51"}, "fields": {"resolution": {"id": "1"}, POINTS: 5}}));
    // A bad choice says which ones there are.
    let r = run(
        &env,
        "jira.transition",
        json!({"connection": "ops-jira", "key": "OPS-1", "to": "51", "fields": {"resolution": "Nope", "points": 1}}),
    )
    .await;
    assert!(r.to_string().contains("Resolution has no option \\\"Nope\\\" (it takes: Fixed, Won't Fix)"), "{r}");
}

#[tokio::test]
async fn issues_and_epics_are_created_edited_assigned_and_commented_on() {
    let env = setup().await;
    // The create screen's required Severity is named when left out.
    let r = run(
        &env,
        "jira.create",
        json!({"connection": "ops-jira", "project": "OPS", "issueType": "Story",
        "summary": "No severity"}),
    )
    .await;
    assert!(r.to_string().contains("Severity [customfield_10200] (one of Sev 1, Sev 2)"), "{r}");
    // An epic: its Epic Name (found by type) is the summary.
    let o = out(
        &env,
        "jira.create",
        json!({"connection": "ops-jira", "project": "OPS", "issueType": "Epic",
        "summary": "Billing", "fields": {"severity": "Sev 2"}}),
    )
    .await;
    let epic = o["key"].as_str().unwrap().to_string();
    let body = sent(&env, "POST", "/jira/rest/api/2/issue");
    assert_eq!(
        (body["fields"][EPIC_NAME].clone(), body["fields"][SEVERITY].clone()),
        (json!("Billing"), json!({"id": "10301"}))
    );
    // A story in it, with the standard fields.
    let o = out(
        &env,
        "jira.create",
        json!({"connection": "ops-jira", "project": "OPS", "summary": "Invoices",
        "issueType": "Story", "description": "Line one\n\nLine two", "priority": "critical", "labels": ["billing"],
        "components": ["Web"], "assignee": "me", "epic": epic, "fields": {"Severity": "Sev 1"}}),
    )
    .await;
    let story = o["key"].as_str().unwrap().to_string();
    assert!(o["url"].as_str().unwrap().ends_with(&format!("/jira/browse/{story}")));
    let f = sent(&env, "POST", "/jira/rest/api/2/issue")["fields"].clone();
    assert_eq!(f["description"], "Line one\n\nLine two", "plain text on Server");
    assert_eq!((f["priority"].clone(), f["assignee"].clone()), (json!({"id": "2"}), json!({"name": ME})));
    assert_eq!((f["labels"].clone(), f["components"].clone()), (json!(["billing"]), json!([{"name": "Web"}])));
    assert_eq!((f[EPIC_LINK].clone(), f["project"].clone()), (json!(epic), json!({"key": "OPS"})));
    // Edit (a number and a choice by mapped name) and assign by name.
    let o = out(
        &env,
        "jira.edit",
        json!({"connection": "ops-jira", "key": story, "summary": "Invoices v2",
        "fields": {"points": "8", "severity": "sev 2"}, "assignee": "Bob Smith"}),
    )
    .await;
    assert_eq!(o["changed"], json!([POINTS, SEVERITY, "summary", "assignee"]));
    let body = sent(&env, "PUT", &format!("/jira/rest/api/2/issue/{story}"));
    assert_eq!(body["fields"], json!({"summary": "Invoices v2", POINTS: 8, SEVERITY: {"id": "10301"}}));
    assert_eq!(sent(&env, "PUT", &format!("/jira/rest/api/2/issue/{story}/assignee")), json!({"name": "bsmith"}));
    // Comment: plain text.
    let o = out(&env, "jira.comment", json!({"connection": "ops-jira", "key": story, "body": "Shipped"})).await;
    assert_eq!(o["commentId"], "777");
    assert_eq!(sent(&env, "POST", &format!("/jira/rest/api/2/issue/{story}/comment")), json!({"body": "Shipped"}));
    // Epic membership through the Epic Link field: out, then in again.
    out(&env, "jira.epic", json!({"connection": "ops-jira", "keys": [story]})).await;
    assert_eq!(sent(&env, "PUT", &format!("/jira/rest/api/2/issue/{story}"))["fields"], json!({EPIC_LINK: null}));
    let o =
        out(&env, "jira.epic", json!({"connection": "ops-jira", "keys": [story.to_lowercase()], "epic": epic})).await;
    assert_eq!(o, json!({"keys": [story], "epic": epic}));
    let o = out(&env, "jira.search", json!({"connection": "ops-jira", "jql": "x"})).await;
    let mine = o["issues"].as_array().unwrap().iter().find(|i| i["key"] == story.as_str()).unwrap().clone();
    assert_eq!(mine["epic"], json!(epic));
}

#[tokio::test]
async fn sprints_backlog_sections_and_the_board() {
    let env = setup().await;
    {
        let mut j = env.jira.lock().unwrap();
        j.issues.push(issue("OPS-1", "One", None));
        j.issues.push(issue("OPS-2", "Two", None));
        j.issues.push(issue("OPS-3", "Three", None));
    }
    let o = out(&env, "jira.boards", json!({"connection": "ops-jira", "project": "OPS"})).await;
    assert_eq!(o["boards"], json!([{"id": 7, "name": "Ops board", "type": "scrum", "project": "OPS"}]));
    // The active sprint is found, and the backlog section isn't one.
    let o = out(&env, "jira.sprints", json!({"connection": "ops-jira", "board": "Ops board"})).await;
    assert_eq!(o["active"][0]["name"], "Sprint 7");
    assert_eq!(o["active"].as_array().unwrap().len(), 1);
    assert_eq!(o["sections"][0]["name"], "Backlog: Ready");
    // Into the active sprint, into a section by name, and back to the backlog.
    let o = out(
        &env,
        "jira.sprint",
        json!({"connection": "ops-jira", "keys": ["OPS-1", "OPS-2"], "sprint": "active",
        "board": "7"}),
    )
    .await;
    assert_eq!(o["sprint"]["id"], 70);
    assert_eq!(sent(&env, "POST", "/jira/rest/agile/1.0/sprint/70/issue"), json!({"issues": ["OPS-1", "OPS-2"]}));
    out(
        &env,
        "jira.sprint",
        json!({"connection": "ops-jira", "keys": ["OPS-3"], "sprint": "backlog: ready",
        "board": "Ops board"}),
    )
    .await;
    out(&env, "jira.sprint", json!({"connection": "ops-jira", "keys": ["OPS-2"], "backlog": true})).await;
    assert_eq!(sent(&env, "POST", "/jira/rest/agile/1.0/backlog/issue"), json!({"issues": ["OPS-2"]}));
    // Placement by sprint membership and status: the sprint first, then the section, then the backlog.
    let o = out(&env, "jira.board", json!({"connection": "ops-jira", "board": "7"})).await;
    let places: Vec<(String, String)> = o["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["key"].as_str().unwrap().into(), i["placement"]["area"].as_str().unwrap().into()))
        .collect();
    let want: Vec<(String, String)> =
        [("OPS-1", "sprint"), ("OPS-3", "section"), ("OPS-2", "backlog")].map(|(a, b)| (a.into(), b.into())).to_vec();
    assert_eq!(places, want);
    // A sprint name without a board, or both a sprint and the backlog, is refused before anything is sent.
    let r = run(&env, "jira.sprint", json!({"connection": "ops-jira", "keys": ["OPS-1"], "sprint": "Sprint 8"})).await;
    assert!(r.to_string().contains("name the board"), "{r}");
    let (s, e) = call(
        &env.app,
        "POST",
        "/api/workflows",
        Some(json!({"doc": manual_then(json!({"id": "s", "kind": "jira.sprint", "version": 1,
            "config": {"connection": "ops-jira", "keys": ["OPS-1"], "sprint": "1", "backlog": true}}))})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
}

/// The PM's Jira tools are the same actions: reading asks nothing; changing waits for a yes unless the connection
/// lets the PM act.
#[tokio::test]
async fn the_pm_searches_freely_and_moves_issues_after_a_yes() {
    use common::pm::*;
    let pm = setup_with(None, |st| {
        st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_jira::Jira)]));
        st.integrations.enable(&["jira".into()]);
    })
    .await;
    let (url, jira) = fake_server().await;
    let conn_id = connect(&pm.app, &url, PAT).await;
    jira.lock().unwrap().issues.push(issue("OPS-1", "Fix the outage", None));

    let id = start_pm(&pm, "hi").await;
    wait_said(&pm, &id, 1).await;
    assert!(instructions(&pm, &id).contains("`jira_transition`"));
    let said = ask(&pm, &id, r#"mcp: jira_search {"jql": "project = OPS"}"#).await;
    assert!(said.contains("\"key\": \"OPS-1\"") && said.contains("\"area\": \"backlog\""), "{said}");
    assert!(!said.contains(PAT));

    let n = agent_says(&pm, &id).await.len();
    send(&pm, &id, r#"mcp: jira_transition {"key": "OPS-1", "to": "In Progress"}"#).await;
    let req = open_request(&pm, &id).await;
    let title = events(&pm, &id)
        .await
        .iter()
        .find(|e| e["event"]["kind"] == "request")
        .map(|e| e["event"]["title"].as_str().unwrap_or_default().to_string())
        .unwrap();
    assert_eq!(title, "Jira: move OPS-1 to In Progress");
    assert_eq!(jira.lock().unwrap().issues[0]["fields"]["status"]["name"], "Open", "nothing before the yes");
    call(&pm.app, "POST", &format!("/api/sessions/{id}/requests/{req}"), Some(json!({"answer": {"type": "approve"}})))
        .await;
    let said = wait_said(&pm, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"status\": \"In Progress\""), "{said}");

    // The connection lets the PM act: a missing required field comes back as what to ask the user for.
    let mut cfg = config();
    cfg["pmActs"] = json!(true);
    call(&pm.app, "PATCH", &format!("/api/connections/{conn_id}"), Some(json!({"config": cfg}))).await;
    let said = ask(&pm, &id, r#"mcp: jira_transition {"key": "OPS-1", "to": "Resolved"}"#).await;
    assert!(said.contains("Resolution [resolution]"), "{said}");
    let said = ask(
        &pm,
        &id,
        r#"mcp: jira_transition {"key": "OPS-1", "to": "Resolved", "fields": {"resolution": "Fixed", "points": 2}}"#,
    )
    .await;
    assert!(said.contains("\"status\": \"Resolved\""), "{said}");
    call(&pm.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
