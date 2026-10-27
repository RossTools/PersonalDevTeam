//! PDT-154 end to end: bug reports from other PDT instances (issues labelled `from-pdt` on the release repository)
//! come in through the built-in "PDT bug triage" (the PM's step, played by `fake-agent`), and a linked ticket that
//! finishes closes its issue (the built-in "Done closes the GitHub issue"). GitHub is `FakeForge`.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_core::brand::RELEASE_REPO;
use pdt_core::github_issue::CloseReason;
use pdt_forge::testing::{FakeForge, fake_issue_id};
use pdt_store::Store;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    gh: Arc<FakeForge>,
    _tmp: tempfile::TempDir,
}

/// A project `P` (key `PX`) whose repository is the release repository, served on a free local port (the agent's
/// pdt-mcp calls come over TCP).
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    git(&repo, &["remote", "add", "origin", &format!("https://github.com/{RELEASE_REPO}.git")]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.codex_program = FAKE.to_string();
    state.platform.devin_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let gh = Arc::new(FakeForge::default());
    state.github = gh.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "P", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    Env { app, state, project, gh, _tmp: tmp }
}

fn issue(n: u64, title: &str, labels: &[&str]) -> pdt_forge::Issue {
    pdt_forge::Issue {
        number: n,
        title: title.into(),
        body: format!("Steps for #{n}."),
        url: format!("https://github.com/{RELEASE_REPO}/issues/{n}"),
        author: "someone".into(),
        labels: labels.iter().map(|l| l.to_string()).collect(),
        created_at: "2026-10-25T00:00:00Z".into(),
    }
}

async fn builtin(env: &Env, id: &str) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, all) = call(&env.app, "GET", "/api/workflows", None).await;
    all.as_array().unwrap().iter().find(|w| w["builtin"] == id).cloned().unwrap_or_else(|| panic!("no {id}: {all}"))
}

async fn runs(env: &Env, wf: &str) -> Vec<Value> {
    let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    r.as_array().unwrap().clone()
}

async fn tickets(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    v["items"].as_array().unwrap().clone()
}

/// Opens `new` on the release repository (on top of the issues there) and has pdtd look.
async fn look(env: &Env, new: pdt_forge::Issue) {
    let mut all = env.gh.issues.lock().unwrap().get(RELEASE_REPO).cloned().unwrap_or_default();
    all.insert(0, new);
    env.gh.set_issues(RELEASE_REPO, all);
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
}

/// The next PM session a triage step started (one not in `seen`).
async fn triage_session(env: &Env, seen: &mut HashSet<String>) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
        let found = list.as_array().unwrap().iter().find_map(|s| {
            let id = s["id"].as_str().unwrap().to_string();
            (s["persona"]["name"] == "PM" && !seen.contains(&id)).then_some(id)
        });
        if let Some(id) = found {
            seen.insert(id.clone());
            return id;
        }
        assert!(Instant::now() < deadline, "no triage session");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn first_prompt(env: &Env, session: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
        if let Some(e) = v.as_array().unwrap().iter().find(|e| e["event"]["kind"] == "user_message") {
            return e["event"]["text"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "the session got no prompt");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The agent submits `outcome` (with `data`), and the run it belongs to ends; that run.
async fn submit(env: &Env, wf: &str, session: &str, outcome: &str, summary: &str, data: Value) -> Value {
    first_prompt(env, session).await;
    let before: HashSet<String> = runs(env, wf)
        .await
        .iter()
        .filter(|r| r["status"] == "succeeded")
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    let args = json!({"outcome": outcome, "summary": summary, "data": data});
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/sessions/{session}/messages"),
        Some(json!({"text": format!("mcp: submit_outcome {args}")})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        let all = runs(env, wf).await;
        if let Some(r) = all.iter().find(|r| r["status"] == "succeeded" && !before.contains(r["id"].as_str().unwrap()))
        {
            call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
            let id = r["id"].as_str().unwrap();
            return call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
        }
        let failed: Vec<_> = all.iter().filter(|r| r["status"] == "failed").collect();
        assert!(failed.is_empty(), "a run failed: {failed:?}");
        assert!(Instant::now() < deadline, "the run never ended: {all:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn move_ticket(env: &Env, id: &str, stage: &str) {
    let (s, v) = call(&env.app, "PATCH", &format!("/api/tickets/{id}"), Some(json!({"stage": stage}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

#[tokio::test]
async fn bug_reports_are_triaged_into_tickets_duplicates_and_rejections() {
    let env = setup().await;
    let triage = builtin(&env, "pdt-bug-triage").await;
    assert_eq!(triage["enabled"], true, "enabled where a project is the release repository");
    let wf = triage["id"].as_str().unwrap().to_string();
    // The first look is the baseline.
    look(&env, issue(1, "Old", &["from-pdt"])).await;
    let mut seen = HashSet::new();

    // A report labelled from-pdt: the PM triages it and accepts it.
    look(&env, issue(2, "Crash on start", &["from-pdt"])).await;
    let session = triage_session(&env, &mut seen).await;
    let prompt = first_prompt(&env, &session).await;
    assert!(prompt.contains("#2 on") && prompt.contains("\"Crash on start\"") && prompt.contains("Steps for #2."));
    assert!(prompt.contains("`accept`") && prompt.contains("duplicateOf"), "{prompt}");
    let data = json!({"priority": "high", "labels": ["bug", "crash"]});
    let run = submit(&env, &wf, &session, "accept", "Panics in main.rs when the config is empty.", data).await;
    let t = tickets(&env).await;
    assert_eq!(t.len(), 1, "{t:?}");
    let ticket = &t[0];
    assert_eq!((ticket["title"].as_str(), ticket["stage"].as_str()), (Some("Crash on start"), Some("draft")));
    assert_eq!((ticket["priority"].as_str(), ticket["labels"].clone()), (Some("high"), json!(["bug", "crash"])));
    let key = format!("{}#2", RELEASE_REPO.to_ascii_lowercase());
    assert_eq!(ticket["external"], json!([{"system": "github", "key": key, "url": issue(2, "", &[]).url}]));
    let (id, tkey) = (ticket["id"].as_str().unwrap().to_string(), ticket["key"].as_str().unwrap().to_string());
    let comments = env.gh.comments_on(RELEASE_REPO, 2);
    assert!(comments.len() == 1 && comments[0].contains(&format!("tracked as {tkey}")), "{comments:?}");
    let (_, full) = call(&env.app, "GET", &format!("/api/tickets/{id}"), None).await;
    assert!(full["description"].as_str().unwrap().contains("Reported by @someone"), "{full}");
    let (_, timeline) = call(&env.app, "GET", &format!("/api/tickets/{id}/events"), None).await;
    assert!(timeline.to_string().contains("Panics in main.rs"), "the triage notes: {timeline}");
    assert_eq!(run["steps"].as_array().unwrap().last().unwrap()["nodeId"], "imported");

    // An issue without the label is left alone.
    look(&env, issue(3, "A question", &["question"])).await;
    assert_eq!(runs(&env, &wf).await.len(), 1);

    // A second report of the same bug: closed at once as a GitHub duplicate of #2, the ticket notes it, no ticket.
    look(&env, issue(4, "Crashes when starting", &["from-pdt"])).await;
    let session = triage_session(&env, &mut seen).await;
    let data = json!({"ticket": tkey, "duplicateOf": 2});
    submit(&env, &wf, &session, "duplicate", "Same panic as #2.", data).await;
    assert_eq!(tickets(&env).await.len(), 1, "no ticket for a duplicate");
    let closed = env.gh.closed.lock().unwrap().get(&(RELEASE_REPO.to_string(), 4)).cloned();
    assert_eq!(closed, Some((CloseReason::Duplicate, Some(fake_issue_id(2)))));
    let c = env.gh.comments_on(RELEASE_REPO, 4);
    assert!(c.len() == 1 && c[0].contains("duplicate of #2") && c[0].contains(&tkey), "{c:?}");
    let (_, timeline) = call(&env.app, "GET", &format!("/api/tickets/{id}/events"), None).await;
    assert!(timeline.to_string().contains("Another report of this bug"), "{timeline}");

    // A duplicate of a ticket with no issue to point at: closed as not planned, naming the ticket.
    look(&env, issue(5, "Also crashes", &["from-pdt"])).await;
    let session = triage_session(&env, &mut seen).await;
    submit(&env, &wf, &session, "duplicate", "Same bug.", json!({"ticket": tkey})).await;
    let closed = env.gh.closed.lock().unwrap().get(&(RELEASE_REPO.to_string(), 5)).cloned();
    assert_eq!(closed, Some((CloseReason::NotPlanned, None)));
    assert!(env.gh.comments_on(RELEASE_REPO, 5)[0].contains(&format!("already tracked as {tkey}")));

    // Rejected: closed as not planned with the reason, and a notice in the Inbox.
    look(&env, issue(6, "Make it purple", &["from-pdt"])).await;
    let session = triage_session(&env, &mut seen).await;
    submit(&env, &wf, &session, "reject", "It works as intended.", json!({})).await;
    let closed = env.gh.closed.lock().unwrap().get(&(RELEASE_REPO.to_string(), 6)).cloned();
    assert_eq!(closed, Some((CloseReason::NotPlanned, None)));
    assert!(env.gh.comments_on(RELEASE_REPO, 6)[0].contains("It works as intended."));
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    let notice = inbox.to_string();
    assert!(notice.contains("Triage closed") && notice.contains("#6 as not planned"), "{inbox}");
    assert_eq!(tickets(&env).await.len(), 1);

    // Done closes the issue as completed with a comment, once: moving it again changes nothing on GitHub.
    move_ticket(&env, &id, "done").await;
    let done_wf = builtin(&env, "github-issue-done").await;
    let done_runs = runs(&env, done_wf["id"].as_str().unwrap()).await;
    let closed = env.gh.closed.lock().unwrap().get(&(RELEASE_REPO.to_string(), 2)).cloned();
    assert_eq!(closed, Some((CloseReason::Completed, None)), "{done_runs:?}");
    let c = env.gh.comments_on(RELEASE_REPO, 2);
    assert!(c.len() == 2 && c[1].contains(&format!("Fixed in {tkey}")), "{c:?}");
    env.gh.closed.lock().unwrap().remove(&(RELEASE_REPO.to_string(), 2));
    move_ticket(&env, &id, "todo").await;
    move_ticket(&env, &id, "done").await;
    assert_eq!(env.gh.comments_on(RELEASE_REPO, 2).len(), 2, "once per ticket");
    assert!(!env.gh.closed.lock().unwrap().contains_key(&(RELEASE_REPO.to_string(), 2)));
}

/// A workflow that runs one action on Run now's input.
async fn action(env: &Env, node: Value) -> String {
    let doc = json!({"name": "One action", "nodes": [
        {"id": "start", "kind": "manual", "version": 1},
        node,
        {"id": "end", "kind": "end", "version": 1, "config": {"quiet": true}}],
      "edges": [{"from": "start", "to": "act"}]});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

async fn run_now(env: &Env, wf: &str, input: Value) -> Value {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let id = r["runId"].as_str().unwrap();
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"].as_array().unwrap().iter().find(|s| s["nodeId"] == node).unwrap_or_else(|| panic!("{run}"))
}

#[tokio::test]
async fn an_issue_is_imported_once_and_wont_do_closes_it_as_not_planned() {
    let env = setup().await;
    let slow = pdt_forge::Issue { url: "https://github.com/o/r/issues/9".into(), ..issue(9, "Slow board", &[]) };
    env.gh.set_issues("o/r", vec![slow]);
    let import = action(
        &env,
        json!({"id": "act", "kind": "github.import", "version": 1, "config": {
            "repo": "o/r", "number": "{{trigger.n}}", "project": env.project, "priority": "p0", "labels": "{{trigger.labels}}"}}),
    )
    .await;
    let r = run_now(&env, &import, json!({"n": 9})).await;
    let s = step(&r, "act");
    assert_eq!(s["port"], "imported", "{r}");
    let ticket = s["output"]["ticketId"].as_str().unwrap().to_string();
    assert_eq!(s["output"]["issue"], "o/r#9");
    let t = tickets(&env).await;
    assert_eq!(t[0]["priority"], "none", "a priority PDT doesn't know is left out");

    // Again: nothing new, by `exists`.
    let r = run_now(&env, &import, json!({"n": 9})).await;
    assert_eq!(step(&r, "act")["port"], "exists", "{r}");
    assert_eq!(step(&r, "act")["output"]["ticketId"], ticket.as_str());
    assert_eq!(tickets(&env).await.len(), 1);

    // An issue that isn't there fails the step.
    let r = run_now(&env, &import, json!({"n": 10})).await;
    assert_eq!(r["status"], "failed", "{r}");

    // Won't do closes it as not planned, with a comment.
    move_ticket(&env, &ticket, "wont_do").await;
    let closed = env.gh.closed.lock().unwrap().get(&("o/r".to_string(), 9)).cloned();
    assert_eq!(closed, Some((CloseReason::NotPlanned, None)));
    assert!(env.gh.comments_on("o/r", 9)[0].contains("won't be done"));

    // Closing an issue closed already changes nothing; labels are added.
    let close = action(
        &env,
        json!({"id": "act", "kind": "github.close", "version": 1, "config": {
            "ticket": ticket, "reason": "completed", "comment": "again", "labels": ["triaged"]}}),
    )
    .await;
    let r = run_now(&env, &close, json!({})).await;
    assert_eq!(step(&r, "act")["output"]["alreadyClosed"], true, "{r}");
    assert_eq!(env.gh.comments_on("o/r", 9).len(), 1);
    env.gh.closed.lock().unwrap().clear();
    let r = run_now(&env, &close, json!({})).await;
    assert_eq!(step(&r, "act")["output"]["reason"], "completed", "{r}");
    let labels = env.gh.added_labels.lock().unwrap().clone();
    assert_eq!(labels, [("o/r".to_string(), 9, vec!["triaged".to_string()])]);

    // A ticket linked to no issue leaves by `unlinked`.
    let (_, other) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Local"})))
            .await;
    let comment = action(
        &env,
        json!({"id": "act", "kind": "github.comment", "version": 1, "config": {
            "ticket": other["key"], "body": "hi"}}),
    )
    .await;
    let r = run_now(&env, &comment, json!({})).await;
    assert_eq!(step(&r, "act")["port"], "unlinked", "{r}");
}

/// PDT-176: on a forge without issues the triage reads nothing and the issue actions fail, saying why.
#[tokio::test]
async fn a_forge_without_issues_reads_and_changes_none() {
    let env = setup().await;
    *env.gh.capabilities.lock().unwrap() = pdt_forge::Capabilities::NONE;
    assert_eq!(builtin(&env, "pdt-bug-triage").await["enabled"], true);
    look(&env, issue(1, "Old", &["from-pdt"])).await;
    look(&env, issue(2, "Crash on start", &["from-pdt"])).await;
    assert!(env.gh.calls.lock().unwrap().is_empty(), "{:?}", env.gh.calls.lock().unwrap());
    let (_, logged) = call(&env.app, "GET", "/api/events?type=github.issue.opened", None).await;
    assert!(logged.as_array().unwrap().is_empty(), "{logged}");

    env.gh.set_issues("o/r", vec![issue(9, "Slow board", &[])]);
    let import = action(
        &env,
        json!({"id": "act", "kind": "github.import", "version": 1, "config": {
            "repo": "o/r", "number": 9, "project": env.project}}),
    )
    .await;
    let r = run_now(&env, &import, json!({})).await;
    assert_eq!(r["status"], "failed", "{r}");
    assert!(r.to_string().contains("has no issues"), "{r}");
    assert!(tickets(&env).await.is_empty());
}

#[tokio::test]
async fn the_triage_workflow_validates_and_dry_runs() {
    let env = setup().await;
    pdt_api::run_workflows(&env.state).await;
    for id in ["pdt-bug-triage", "github-issue-done"] {
        let w = builtin(&env, id).await;
        let wid = w["id"].as_str().unwrap();
        let (_, d) = call(&env.app, "GET", &format!("/api/workflows/{wid}"), None).await;
        let (s, v) = call(&env.app, "POST", "/api/workflows/validate", Some(json!({"doc": d["doc"]}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let errors: Vec<_> = v.as_array().unwrap().iter().filter(|i| i["severity"] == "error").collect();
        assert!(errors.is_empty(), "{id}: {errors:?}");
    }
    let wid = builtin(&env, "pdt-bug-triage").await["id"].as_str().unwrap().to_string();
    let event = json!({"type": "github.issue.opened", "payload": {
        "projectId": env.project, "repo": RELEASE_REPO, "number": 7, "title": "Crash", "body": "b",
        "url": "https://github.com/x/7", "author": "al", "labels": ["from-pdt"], "createdAt": "t"}});
    let (s, d) =
        call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"workflowId": wid, "event": event}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let text = d.to_string();
    assert!(text.contains(&format!("import {RELEASE_REPO}#7 as a ticket")), "{d}");
}
