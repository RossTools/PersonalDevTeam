//! Jira Cloud (Phase 7e-2, PDT-85) through the API: a project linked to Jira, the sensor's events, importing issues
//! (assigned ones by the built-in, any one by key, never twice), Done moving the issue, and the Jira actions. Jira is
//! a fake local server.

mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use common::{call, git};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const EMAIL: &str = "me@example.com";
const TOKEN: &str = "jira-token";
const ME: &str = "acc-me";

/// A fake Jira Cloud site: its issues, and every request it got.
#[derive(Default)]
struct FakeJira {
    issues: Vec<Value>,
    /// Method, path and query, JSON body.
    seen: Vec<(String, String, Value)>,
    /// A request without the connection's basic auth.
    unauthorized: bool,
    /// Every request is refused (a revoked token).
    revoked: bool,
    /// What `GET /issue/{key}/comment` answers (the newest comments of an issue a search cut short).
    newest_comments: Vec<Value>,
}

type Shared = Arc<Mutex<FakeJira>>;

fn basic() -> String {
    use std::io::Write;
    let mut raw = vec![];
    write!(raw, "{EMAIL}:{TOKEN}").unwrap();
    format!("Basic {}", base64(&raw))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

async fn jira_app(State(j): State<Shared>, req: Request<Body>) -> (StatusCode, axum::Json<Value>) {
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let path = req.uri().path().to_string();
    let authed = req.headers().get("authorization").and_then(|v| v.to_str().ok()) == Some(basic().as_str());
    let bytes = req.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut j = j.lock().unwrap();
    j.seen.push((method.clone(), uri, body.clone()));
    if !authed || j.revoked {
        j.unauthorized |= !authed;
        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"errorMessages": ["no"]})));
    }
    let parts: Vec<&str> = path.trim_start_matches("/rest/api/3/").split('/').collect();
    let find = |j: &FakeJira, key: &str| j.issues.iter().position(|i| i["key"] == key);
    let ok = |v: Value| (StatusCode::OK, axum::Json(v));
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["myself"]) => ok(json!({"accountId": ME})),
        ("POST", ["search", "jql"]) => ok(json!({"issues": j.issues, "isLast": true})),
        ("GET", ["issue", key]) => match find(&j, key) {
            Some(i) => ok(j.issues[i].clone()),
            None => (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["Issue does not exist"]}))),
        },
        ("GET", ["issue", _, "transitions"]) => ok(json!({"transitions": [
            {"id": "21", "name": "Start work", "to": {"name": "In Progress"}},
            {"id": "31", "name": "Done", "to": {"name": "Done"}}]})),
        ("POST", ["issue", key, "transitions"]) => {
            let to = if body["transition"]["id"] == "31" { "Done" } else { "In Progress" };
            if let Some(i) = find(&j, key) {
                j.issues[i]["fields"]["status"]["name"] = json!(to);
            }
            (StatusCode::NO_CONTENT, axum::Json(Value::Null))
        }
        ("GET", ["issue", _, "comment"]) => {
            ok(json!({"comments": j.newest_comments, "total": j.newest_comments.len()}))
        }
        ("POST", ["issue", _, "comment"]) => (StatusCode::CREATED, axum::Json(json!({"id": "900"}))),
        ("POST", ["issue"]) => (StatusCode::CREATED, axum::Json(json!({"id": "1", "key": "ABC-99"}))),
        _ => (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["no route"]}))),
    }
}

async fn fake_jira() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = axum::Router::new().fallback(jira_app).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), shared)
}

const T0: &str = "2026-10-22T09:00:00.000+0000";
const T1: &str = "2026-10-22T10:00:00.000+0000";
const T2: &str = "2026-10-22T10:30:00.000+0000";

fn ms(t: &str) -> i64 {
    pdt_core::jira::parse_time(t).unwrap()
}

/// An issue as REST v3 answers, created and updated at `created`.
fn issue(key: &str, summary: &str, assignee: Option<&str>, created: &str) -> Value {
    json!({"key": key, "fields": {
        "summary": summary,
        "description": {"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": format!("About {summary}")}]}]},
        "status": {"name": "To Do"},
        "assignee": assignee.map(|a| json!({"displayName": a, "accountId": a})),
        "reporter": {"displayName": "Ann"},
        "issuetype": {"name": "Story"},
        "priority": {"name": "Medium"},
        "labels": [],
        "created": created,
        "updated": created,
        "comment": {"comments": []}},
        "changelog": {"histories": []}})
}

struct Env {
    app: axum::Router,
    state: AppState,
    jira: Shared,
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
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (url, jira) = fake_jira().await;
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Work", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (s, c) = call(
        &app,
        "POST",
        "/api/connections",
        Some(json!({"name": "work-jira", "kind": "jira", "url": url, "config": {"email": EMAIL}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().unwrap();
    let (s, _) = call(&app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": TOKEN}))).await;
    assert_eq!(s, StatusCode::OK);
    Env { app, state, jira, project, _tmp: tmp }
}

async fn link(env: &Env) {
    link_project(env, &env.project).await;
}

async fn link_project(env: &Env, project: &str) {
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/projects/{project}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": null,
            "jira": {"connection": "work-jira", "scope": "project = ABC"}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["jira"], json!({"connection": "work-jira", "scope": "project = ABC", "done": "Done"}));
}

/// Sets the sensor's last look to `t`, as if it had polled then.
async fn polled_at(env: &Env, t: &str) {
    project_polled_at(env, &env.project, t).await;
}

async fn project_polled_at(env: &Env, project: &str, t: &str) {
    let conn = env.state.store.connection_by_name("work-jira").await.unwrap().unwrap();
    let v = json!({"since": ms(t), "connection": conn.id, "scope": "project = ABC"});
    env.state.store.put_host_value(&format!("sensor.jira.{project}"), &v).await.unwrap();
}

async fn since(env: &Env) -> i64 {
    let key = format!("sensor.jira.{}", env.project);
    env.state.store.get_host_value(&key).await.unwrap().unwrap()["since"].as_i64().unwrap()
}

/// The live notifications of a kind.
async fn notifications(env: &Env, kind: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/notifications?section=updates", None).await;
    v["items"].as_array().unwrap().iter().filter(|n| n["kind"] == kind && n["clearedAt"].is_null()).cloned().collect()
}

async fn events(env: &Env, kind: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/events?type={kind}"), None).await;
    v.as_array().unwrap().clone()
}

async fn tickets(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    v["items"].as_array().unwrap().clone()
}

#[tokio::test]
async fn the_first_look_only_records_where_jira_is() {
    let env = setup().await;
    env.jira.lock().unwrap().issues.push(issue("ABC-1", "Old", Some(ME), T0));
    pdt_api::refresh_jira(&env.state).await;
    assert!(env.jira.lock().unwrap().seen.is_empty(), "an unlinked project is not read");
    link(&env).await;
    pdt_api::refresh_jira(&env.state).await;
    assert!(events(&env, "jira.issue.created").await.is_empty(), "the baseline is silent");
    let key = format!("sensor.jira.{}", env.project);
    assert!(env.state.store.get_host_value(&key).await.unwrap().unwrap()["since"].as_i64().is_some());
    pdt_api::refresh_jira(&env.state).await;
    let seen = env.jira.lock().unwrap().seen.clone();
    let search = seen.iter().find(|(m, p, _)| m == "POST" && p.contains("search/jql")).expect("a search");
    assert!(search.2["jql"].as_str().unwrap().starts_with("(project = ABC) AND updated >= \"-"), "{}", search.2);
    assert_eq!(search.2["expand"], "changelog");
    assert!(!env.jira.lock().unwrap().unauthorized, "signed in with the email and token");
    assert!(events(&env, "jira.issue.created").await.is_empty(), "nothing new since the baseline");
}

#[tokio::test]
async fn assigned_issues_become_linked_tickets_once() {
    let env = setup().await;
    link(&env).await;
    polled_at(&env, T0).await;
    {
        let mut j = env.jira.lock().unwrap();
        j.issues.push(issue("ABC-1", "Fix login", Some(ME), T1));
        j.issues.push(issue("ABC-2", "Someone else's", Some("acc-other"), T1));
    }
    pdt_api::refresh_jira(&env.state).await;
    let created = events(&env, "jira.issue.created").await;
    assert_eq!(created.len(), 2, "{created:?}");
    let mine = created.iter().find(|e| e["payload"]["key"] == "ABC-1").unwrap();
    assert_eq!(mine["payload"]["assignedToMe"], true);
    assert_eq!(mine["payload"]["projectId"], env.project.as_str());
    assert_eq!(mine["payload"]["description"], "About Fix login");
    pdt_api::run_workflows(&env.state).await;
    let list = tickets(&env).await;
    assert_eq!(list.len(), 1, "only the issue assigned to me: {list:?}");
    assert_eq!(list[0]["title"], "Fix login");
    assert_eq!(list[0]["stage"], "draft");
    let url = list[0]["external"][0]["url"].as_str().unwrap();
    assert!(url.ends_with("/browse/ABC-1"));
    assert_eq!(list[0]["external"][0]["key"], "ABC-1");
    let id = list[0]["id"].as_str().unwrap().to_string();
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    assert_eq!(t["description"], "About Fix login");

    // Read again (and assigned again later): never a second ticket; the events now carry the ticket.
    pdt_api::refresh_jira(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(tickets(&env).await.len(), 1);
    {
        let mut j = env.jira.lock().unwrap();
        let i = &mut j.issues[0];
        i["fields"]["updated"] = json!(T2);
        i["changelog"]["histories"] = json!([{"id": "7", "created": T2, "items": [
            {"field": "assignee", "fromString": null, "toString": "Me"}]}]);
    }
    polled_at(&env, "2026-10-22T10:15:00.000+0000").await;
    pdt_api::refresh_jira(&env.state).await;
    let updated = events(&env, "jira.issue.updated").await;
    assert_eq!(updated.len(), 1, "{updated:?}");
    assert_eq!(updated[0]["payload"]["changed"], json!(["assignee"]));
    assert_eq!(updated[0]["payload"]["ticketId"], id.as_str());
    pdt_api::run_workflows(&env.state).await;
    assert_eq!(tickets(&env).await.len(), 1, "a linked issue is never imported again");

    // By key: the linked one exists; another (not assigned) is imported; bad and unknown keys are refused.
    let path = format!("/api/projects/{}/jira/import", env.project);
    let (s, r) = call(&env.app, "POST", &path, Some(json!({"key": "abc-1"}))).await;
    assert_eq!((s, r["existed"].clone(), r["ticket"]["id"].clone()), (StatusCode::OK, json!(true), json!(id)));
    let (s, r) = call(&env.app, "POST", &path, Some(json!({"key": "ABC-2"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!((r["existed"].clone(), r["ticket"]["external"][0]["key"].clone()), (json!(false), json!("ABC-2")));
    let (s, _) = call(&env.app, "POST", &path, Some(json!({"key": "not a key"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, r) = call(&env.app, "POST", &path, Some(json!({"key": "ABC-404"}))).await;
    assert_eq!((s, r["code"].clone()), (StatusCode::NOT_FOUND, json!("jira_issue_not_found")));
    assert_eq!(tickets(&env).await.len(), 2);
}

#[tokio::test]
async fn transitions_and_comments_are_events() {
    let env = setup().await;
    link(&env).await;
    polled_at(&env, T1).await;
    let mut i = issue("ABC-3", "Older", None, T0);
    i["fields"]["updated"] = json!(T2);
    i["changelog"]["histories"] = json!([
        {"id": "8", "created": T0, "items": [{"field": "priority", "fromString": "Low", "toString": "High"}]},
        {"id": "9", "created": T2, "items": [{"field": "status", "fromString": "To Do", "toString": "In Progress"}]}]);
    i["fields"]["comment"]["comments"] = json!([
        {"id": "50", "author": {"displayName": "Me", "accountId": ME}, "created": T2,
         "body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "On it"}]}]}}]);
    env.jira.lock().unwrap().issues.push(i);
    pdt_api::refresh_jira(&env.state).await;
    pdt_api::refresh_jira(&env.state).await;
    assert!(events(&env, "jira.issue.created").await.is_empty(), "created before the last look");
    let t = events(&env, "jira.issue.transitioned").await;
    assert_eq!(t.len(), 1, "once, however often it is read");
    assert_eq!(
        (t[0]["payload"]["from"].clone(), t[0]["payload"]["to"].clone()),
        (json!("To Do"), json!("In Progress"))
    );
    let c = events(&env, "jira.issue.commented").await;
    assert_eq!(c.len(), 1);
    assert_eq!((c[0]["payload"]["body"].clone(), c[0]["payload"]["byMe"].clone()), (json!("On it"), json!(true)));
    let u = events(&env, "jira.issue.updated").await;
    assert_eq!(u[0]["payload"]["changed"], json!(["status"]), "only what changed since the last look");
}

#[tokio::test]
async fn done_moves_the_linked_issue_and_nothing_else() {
    let env = setup().await;
    link(&env).await;
    env.jira.lock().unwrap().issues.push(issue("ABC-5", "Ship", Some(ME), T1));
    let (s, r) =
        call(&env.app, "POST", &format!("/api/projects/{}/jira/import", env.project), Some(json!({"key": "ABC-5"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let linked = r["ticket"]["id"].as_str().unwrap().to_string();
    let (_, plain) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Local"})))
            .await;
    let plain = plain["id"].as_str().unwrap().to_string();
    // Other moves stay in PDT.
    call(&env.app, "PATCH", &format!("/api/tickets/{linked}"), Some(json!({"stage": "todo"}))).await;
    for t in [&linked, &plain] {
        let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{t}"), Some(json!({"stage": "done"}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    pdt_api::run_workflows(&env.state).await;
    let moved: Vec<_> = env
        .jira
        .lock()
        .unwrap()
        .seen
        .iter()
        .filter(|(m, p, _)| m == "POST" && p.ends_with("/transitions"))
        .cloned()
        .collect();
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert!(moved[0].1.contains("ABC-5"));
    assert_eq!(moved[0].2["transition"]["id"], "31");
    assert_eq!(env.jira.lock().unwrap().issues[0]["fields"]["status"]["name"], "Done");
    let stage = events(&env, "ticket.stage_changed").await;
    let done: Vec<_> = stage.iter().filter(|e| e["payload"]["to"] == "done").collect();
    assert_eq!(done.len(), 2);
    assert!(done.iter().any(|e| e["payload"]["jiraKey"] == "ABC-5"));
    assert!(done.iter().any(|e| e["payload"]["jiraKey"].is_null()));
    let (_, runs) = call(&env.app, "GET", "/api/workflow-runs", None).await;
    let jira_runs: Vec<_> =
        runs.as_array().unwrap().iter().filter(|r| r["workflowName"] == "Done moves the Jira issue").collect();
    assert_eq!(jira_runs.len(), 1, "the unlinked ticket starts no run: {runs}");
}

fn manual_then(node: Value) -> Value {
    json!({"name": "Test", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
        "edges": [{"from": "go", "to": node["id"]}]})
}

async fn run_once(env: &Env, node: Value, dry: bool) -> Value {
    if dry {
        let (s, r) = call(
            &env.app,
            "POST",
            "/api/workflows/dry-run",
            Some(json!({"doc": manual_then(node), "event": {"type": "manual"}})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{r}");
        return r;
    }
    let (s, w) =
        call(&env.app, "POST", "/api/workflows", Some(json!({"doc": manual_then(node), "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap();
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    call(&env.app, "GET", &format!("/api/workflow-runs/{}", r["runId"].as_str().unwrap()), None).await.1
}

fn last_step(run: &Value) -> Value {
    run["steps"].as_array().unwrap().last().cloned().unwrap()
}

#[tokio::test]
async fn the_jira_actions() {
    let env = setup().await;
    env.jira.lock().unwrap().issues.push(issue("ABC-7", "Act", None, T1));
    // A comment goes as ADF.
    let run = run_once(
        &env,
        json!({"id": "c", "kind": "jira.comment", "version": 1,
            "config": {"connection": "work-jira", "key": "ABC-7", "body": "Done here\n\nSee PR"}}),
        false,
    )
    .await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let seen = env.jira.lock().unwrap().seen.clone();
    let (_, _, body) = seen.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/comment")).unwrap();
    assert_eq!(body["body"]["type"], "doc");
    assert_eq!(body["body"]["content"].as_array().unwrap().len(), 2);
    // Transition by status name; then one with no such transition leaves by `unavailable`.
    let run = run_once(
        &env,
        json!({"id": "t", "kind": "jira.transition", "version": 1,
            "config": {"connection": "work-jira", "key": "ABC-7", "to": "in progress"}}),
        false,
    )
    .await;
    let step = last_step(&run);
    assert_eq!((step["port"].clone(), step["output"]["transition"].clone()), (json!("out"), json!("Start work")));
    let run = run_once(
        &env,
        json!({"id": "t", "kind": "jira.transition", "version": 1,
            "config": {"connection": "work-jira", "key": "ABC-7", "to": "Won't fix"}}),
        false,
    )
    .await;
    let step = last_step(&run);
    assert_eq!(step["port"], "unavailable", "{run}");
    assert_eq!(step["output"]["available"], json!(["Start work", "Done"]));
    assert_eq!(run["status"], "quiet");
    // Create.
    let run = run_once(
        &env,
        json!({"id": "n", "kind": "jira.create", "version": 1, "config": {"connection": "work-jira",
            "project": "ABC", "summary": "From PDT", "description": "Body", "fields": {"labels": ["pdt"]}}}),
        false,
    )
    .await;
    assert_eq!(last_step(&run)["output"]["key"], "ABC-99", "{run}");
    let seen = env.jira.lock().unwrap().seen.clone();
    let (_, _, body) = seen.iter().find(|(m, p, _)| m == "POST" && p == "/rest/api/3/issue").unwrap();
    assert_eq!(body["fields"]["project"]["key"], "ABC");
    assert_eq!(body["fields"]["issuetype"]["name"], "Task");
    assert_eq!(body["fields"]["labels"], json!(["pdt"]));
    // Import into a project not linked to Jira, naming the connection.
    let run = run_once(
        &env,
        json!({"id": "i", "kind": "jira.import", "version": 1,
            "config": {"key": "ABC-7", "project": env.project, "connection": "work-jira"}}),
        false,
    )
    .await;
    assert_eq!(last_step(&run)["port"], "imported", "{run}");
    // Dry runs call nothing.
    let before = env.jira.lock().unwrap().seen.len();
    let r = run_once(
        &env,
        json!({"id": "c", "kind": "jira.comment", "version": 1,
            "config": {"connection": "work-jira", "key": "ABC-7", "body": "x"}}),
        true,
    )
    .await;
    assert!(r.to_string().contains("comment on ABC-7"), "{r}");
    assert_eq!(env.jira.lock().unwrap().seen.len(), before);
}

#[tokio::test]
async fn settings_check_the_connection() {
    let env = setup().await;
    call(&env.app, "POST", "/api/connections", Some(json!({"name": "hook", "kind": "webhook"}))).await;
    let path = format!("/api/projects/{}/settings", env.project);
    let put = |jira: Value| json!({"agentCredentials": null, "runOverrides": null, "jira": jira});
    let (s, _) = call(&env.app, "PUT", &path, Some(put(json!({"connection": "hook"})))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a webhook is not Jira");
    let (s, _) = call(&env.app, "PUT", &path, Some(put(json!({"connection": "nope"})))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(&env.app, "PUT", &path, Some(put(json!({"connection": "work-jira", "done": "Close"})))).await;
    assert_eq!((s, v["jira"]["done"].clone()), (StatusCode::OK, json!("Close")));
    // Left out: kept; "" unlinks.
    let (_, v) = call(&env.app, "PUT", &path, Some(json!({"agentCredentials": null, "runOverrides": null}))).await;
    assert_eq!(v["jira"]["connection"], "work-jira");
    let (_, v) = call(&env.app, "PUT", &path, Some(put(json!({"connection": ""})))).await;
    assert!(v["jira"].is_null());
    let (s, r) =
        call(&env.app, "POST", &format!("/api/projects/{}/jira/import", env.project), Some(json!({"key": "ABC-1"})))
            .await;
    assert_eq!((s, r["code"].clone()), (StatusCode::CONFLICT, json!("jira_not_linked")));
}

#[tokio::test]
async fn a_revoked_token_notifies_once_and_clears_when_it_works_again() {
    let env = setup().await;
    link(&env).await;
    polled_at(&env, T0).await;
    env.jira.lock().unwrap().revoked = true;
    pdt_api::refresh_jira(&env.state).await;
    pdt_api::refresh_jira(&env.state).await;
    let n = notifications(&env, "connection.failed").await;
    assert_eq!(n.len(), 1, "one per connection, however often it fails: {n:?}");
    assert_eq!(n[0]["title"], "work-jira is failing");
    assert!(n[0]["body"].as_str().unwrap().contains("401"), "{}", n[0]["body"]);
    assert_eq!(n[0]["target"], json!({"kind": "connections"}));
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    let c = &list.as_array().unwrap()[0];
    assert!(c["lastError"].as_str().unwrap().contains("401"), "{c}");
    assert!(c["lastErrorAt"].as_i64().is_some());
    assert_eq!(since(&env).await, ms(T0), "nothing read: the mark stays");

    env.jira.lock().unwrap().revoked = false;
    pdt_api::refresh_jira(&env.state).await;
    assert!(notifications(&env, "connection.failed").await.is_empty(), "cleared once it works");
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(list[0]["lastError"].is_null());
}

#[tokio::test]
async fn two_projects_with_overlapping_scopes_both_get_the_event() {
    let env = setup().await;
    let repo = env._tmp.path().join("repo2");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let (s, p) =
        call(&env.app, "POST", "/api/projects", Some(json!({"name": "Other", "repoPath": repo.to_string_lossy()})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let other = p["id"].as_str().unwrap().to_string();
    link(&env).await;
    link_project(&env, &other).await;
    polled_at(&env, T0).await;
    project_polled_at(&env, &other, T0).await;
    env.jira.lock().unwrap().issues.push(issue("ABC-1", "Shared", None, T1));
    pdt_api::refresh_jira(&env.state).await;
    let mut projects: Vec<String> = events(&env, "jira.issue.created")
        .await
        .iter()
        .map(|e| e["payload"]["projectId"].as_str().unwrap().to_string())
        .collect();
    projects.sort();
    let mut want = vec![env.project.clone(), other];
    want.sort();
    assert_eq!(projects, want);
}

#[tokio::test]
async fn an_event_that_could_not_be_logged_leaves_the_mark() {
    let env = setup().await;
    link(&env).await;
    polled_at(&env, T0).await;
    env.jira.lock().unwrap().issues.push(issue("ABC-1", "New", None, T1));
    let db = env._tmp.path().join("pdt.db");
    common::fail_events(&db, Some("jira.issue.created")).await;
    pdt_api::refresh_jira(&env.state).await;
    assert_eq!(since(&env).await, ms(T0), "the mark stays before what was not logged");
    assert!(notifications(&env, "connection.failed").await.is_empty(), "PDT's own failure, not the connection's");
    common::fail_events(&db, None).await;
    pdt_api::refresh_jira(&env.state).await;
    assert_eq!(events(&env, "jira.issue.created").await.len(), 1);
    assert!(since(&env).await > ms(T0));
}

#[tokio::test]
async fn comments_a_search_cut_short_are_read_from_the_issue() {
    let env = setup().await;
    link(&env).await;
    polled_at(&env, T1).await;
    let mut i = issue("ABC-7", "Busy", None, T0);
    i["fields"]["updated"] = json!(T2);
    // The search answers the oldest 100 of 150 comments.
    i["fields"]["comment"] = json!({"comments": [], "total": 150});
    let text = |t: &str| json!({"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": t}]}]});
    {
        let mut j = env.jira.lock().unwrap();
        j.issues.push(i);
        j.newest_comments = vec![
            json!({"id": "150", "author": {"displayName": "Ann", "accountId": "a"}, "created": T2, "body": text("Latest")}),
        ];
    }
    pdt_api::refresh_jira(&env.state).await;
    let c = events(&env, "jira.issue.commented").await;
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0]["payload"]["body"], "Latest");
}
