//! PDT-481 end to end: linking issues that are already in a tracker to tickets (GitHub here, as `FakeForge`), by
//! address or key, with their live title and state; plain links; the PM's `link_issue` and `create_ticket`'s
//! `external`; and Done closing only the issues it should: those PDT filed or imported, and a manual link only when
//! its "Close it when this ticket is done" is on.

use crate::common;
use axum::http::StatusCode;
use common::pm::*;
use common::{call, call_as_ui, git};
use pdt_core::github_issue::CloseReason;
use pdt_core::issue_link::{IssueState, LinkOrigin};
use pdt_store::StoredExternalLink;
use serde_json::{Value, json};

fn issue(repo: &str, n: u64, title: &str) -> pdt_forge::Issue {
    pdt_forge::Issue {
        number: n,
        title: title.into(),
        body: String::new(),
        url: format!("https://github.com/{repo}/issues/{n}"),
        author: "someone".into(),
        labels: vec![],
        created_at: "2026-11-01T00:00:00Z".into(),
    }
}

/// The PM's env with the project on GitHub (`o/r`), issues 1–3 there and issue 9 in `other/lib`.
async fn setup_github() -> Env {
    let env = setup().await;
    git(&env._tmp.path().join("repo"), &["remote", "add", "origin", "https://github.com/o/r.git"]);
    env.gh.set_issues("o/r", vec![issue("o/r", 1, "Crash on start"), issue("o/r", 2, "Slow"), issue("o/r", 3, "Typo")]);
    env.gh.set_issues("other/lib", vec![issue("other/lib", 9, "Upstream bug")]);
    env
}

async fn ticket(env: &Env, title: &str) -> String {
    let uri = format!("/api/projects/{}/tickets", env.project);
    let (s, t) = call(&env.app, "POST", &uri, Some(json!({"title": title, "stage": "todo"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn link(env: &Env, id: &str, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", &format!("/api/tickets/{id}/issues"), Some(body)).await
}

async fn move_ticket(env: &Env, id: &str, stage: &str) {
    let (s, v) = call_as_ui(&env.app, "PATCH", &format!("/api/tickets/{id}"), Some(json!({"stage": stage}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

fn closed(env: &Env, repo: &str, n: u64) -> Option<CloseReason> {
    env.gh.closed.lock().unwrap().get(&(repo.to_string(), n)).map(|(r, _)| *r)
}

#[tokio::test]
async fn issues_link_by_address_or_key_with_their_title_and_state() {
    let env = setup_github().await;
    let a = ticket(&env, "A").await;

    // By address, in another repository: its title and state come with it, and it is manual, left open by Done.
    let (s, t) = link(&env, &a, json!({"issue": "https://github.com/other/lib/issues/9#issuecomment-1"})).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let l = &t["external"][0];
    assert_eq!(
        (&l["system"], &l["key"], &l["title"], &l["state"]),
        (&json!("github"), &json!("other/lib#9"), &json!("Upstream bug"), &json!("open"))
    );
    assert_eq!((&l["origin"], &l["closeOnDone"]), (&json!("manual"), &json!(false)));

    // By key in the project's own repository (`#2`), and by `owner/name#n`; one already closed says so.
    env.gh.closed.lock().unwrap().insert(("o/r".into(), 3), (CloseReason::Completed, None));
    let (s, t) = link(&env, &a, json!({"issue": "#2"})).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (s, t) = link(&env, &a, json!({"issue": "o/r#3", "closeOnDone": true})).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let keys: Vec<_> =
        t["external"].as_array().unwrap().iter().map(|l| (l["key"].clone(), l["state"].clone())).collect();
    assert_eq!(
        keys,
        [(json!("o/r#2"), json!("open")), (json!("o/r#3"), json!("closed")), (json!("other/lib#9"), json!("open"))]
    );
    assert_eq!(t["external"][1]["closeOnDone"], true);

    // One ticket per issue; an issue GitHub doesn't have; an address no tracker knows.
    let b = ticket(&env, "B").await;
    let (s, v) = link(&env, &b, json!({"issue": "o/r#2"})).await;
    assert_eq!((s, &v["code"]), (StatusCode::CONFLICT, &json!("already_linked")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("PX-2"), "{v}");
    let (s, v) = link(&env, &b, json!({"issue": "o/r#77"})).await;
    assert_eq!((s, &v["code"]), (StatusCode::BAD_GATEWAY, &json!("tracker_error")), "{v}");
    let (s, v) = link(&env, &b, json!({"issue": "https://www.figma.com/file/abc"})).await;
    assert_eq!((s, &v["code"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("unrecognized")), "{v}");

    // Board lists carry the chips too.
    let (_, page) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let row = page["items"].as_array().unwrap().iter().find(|t| t["id"] == a.as_str()).unwrap().clone();
    assert_eq!(row["external"].as_array().unwrap().len(), 3);

    // The toggle, then unlinking (nothing changes on GitHub).
    let (s, t) = call(
        &env.app,
        "PATCH",
        &format!("/api/tickets/{a}/issues"),
        Some(json!({"system": "github", "key": "other/lib#9", "closeOnDone": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["external"][2]["closeOnDone"], true);
    let uri = format!("/api/tickets/{a}/issues?system=github&key=o%2Fr%232");
    let (s, t) = call(&env.app, "DELETE", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["external"].as_array().unwrap().len(), 2);
    let (s, _) = call(&env.app, "DELETE", &uri, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(closed(&env, "o/r", 2), None);
}

#[tokio::test]
async fn plain_links_are_added_and_removed() {
    let env = setup_github().await;
    let a = ticket(&env, "A").await;
    let uri = format!("/api/tickets/{a}/web-links");
    let (s, t) =
        call(&env.app, "POST", &uri, Some(json!({"url": "https://www.figma.com/file/abc", "label": "Figma"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (_, t) = call(&env.app, "POST", &uri, Some(json!({"url": "https://docs.example.com/x"}))).await;
    let labels: Vec<_> = t["webLinks"].as_array().unwrap().iter().map(|l| l["label"].clone()).collect();
    assert_eq!(labels, [json!("Figma"), json!("docs.example.com")]);
    assert_eq!(t["external"], json!([]), "kept apart from tracker issues");
    let (s, v) = call(&env.app, "POST", &uri, Some(json!({"url": "not an address"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let id = t["webLinks"][0]["id"].as_str().unwrap();
    let (s, t) = call(&env.app, "DELETE", &format!("{uri}/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["webLinks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn viewing_a_ticket_reads_its_issues_again() {
    let env = setup_github().await;
    let a = ticket(&env, "A").await;
    // An imported link has never been read.
    let l =
        StoredExternalLink::new(&a, "github", "o/r", "o/r#1", "https://github.com/o/r/issues/1", LinkOrigin::Imported);
    env.state.store.link_external(&l, pdt_core::ticket::Actor::System, 1).await.unwrap().unwrap();
    env.gh.closed.lock().unwrap().insert(("o/r".into(), 1), (CloseReason::Completed, None));
    let (s, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/issues/refresh"), None).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!((&t["external"][0]["title"], &t["external"][0]["state"]), (&json!("Crash on start"), &json!("closed")));
    // Read a moment ago: not again.
    env.gh.closed.lock().unwrap().clear();
    let (_, t) = call(&env.app, "POST", &format!("/api/tickets/{a}/issues/refresh"), None).await;
    assert_eq!(t["external"][0]["state"], "closed");
    let s = env.state.store.get_ticket(&a).await.unwrap().unwrap();
    assert_eq!(s.summary.external[0].state, Some(IssueState::Closed));
}

#[tokio::test]
async fn done_closes_created_and_imported_issues_and_manual_ones_only_when_asked() {
    let env = setup_github().await;
    // Manual, left as it is: Done leaves the issue open.
    let manual = ticket(&env, "Manual").await;
    link(&env, &manual, json!({"issue": "o/r#1"})).await;
    // Manual, turned on: Done closes it.
    let asked = ticket(&env, "Asked").await;
    link(&env, &asked, json!({"issue": "o/r#2", "closeOnDone": true})).await;
    // Imported: Done closes it, as before.
    let imported = ticket(&env, "Imported").await;
    let l = StoredExternalLink::new(
        &imported,
        "github",
        "o/r",
        "o/r#3",
        "https://github.com/o/r/issues/3",
        LinkOrigin::Imported,
    );
    env.state.store.link_external(&l, pdt_core::ticket::Actor::System, 1).await.unwrap().unwrap();
    // Filed from the ticket: Done closes it, as before.
    let filed = ticket(&env, "Filed").await;
    let uri = format!("/api/projects/{}/issues", env.project);
    let (s, v) = call_as_ui(&env.app, "POST", &uri, Some(json!({"title": "Filed", "ticketId": filed}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let filed_n = v["number"].as_u64().unwrap();
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{filed}"), None).await;
    assert_eq!((&t["external"][0]["origin"], &t["external"][0]["closeOnDone"]), (&json!("created"), &json!(true)));

    for id in [&manual, &asked, &imported, &filed] {
        move_ticket(&env, id, "done").await;
    }
    assert_eq!(closed(&env, "o/r", 1), None, "linked by hand: never closed by surprise");
    assert_eq!(closed(&env, "o/r", 2), Some(CloseReason::Completed));
    assert_eq!(closed(&env, "o/r", 3), Some(CloseReason::Completed));
    assert_eq!(closed(&env, "o/r", filed_n), Some(CloseReason::Completed));

    // A user's own workflows still see every link (`external`, `githubIssue`); only `closing` leaves the manual one
    // out, and that is what the Done built-ins look at.
    let moved = |id: &str| {
        let (app, id) = (env.app.clone(), id.to_string());
        async move {
            let (_, log) = call(&app, "GET", &format!("/api/events?type=ticket.stage_changed&ticket={id}"), None).await;
            log.as_array().unwrap().last().cloned().unwrap_or_else(|| panic!("{log}"))
        }
    };
    let m = moved(&manual).await;
    let d = &m["payload"];
    assert_eq!((&d["githubIssue"], &d["external"]["github"]), (&json!("o/r#1"), &json!("o/r#1")), "{m}");
    assert_eq!(d["closing"], json!({}), "{m}");
    let a = moved(&asked).await;
    let d = &a["payload"];
    assert_eq!(d["closing"]["github"], "o/r#2", "{a}");

    // A step a user's own workflow gives the ticket acts on its issue linked by hand.
    let doc = json!({"name": "Close it", "nodes": [
        {"id": "start", "kind": "manual", "version": 1},
        {"id": "act", "kind": "github.close", "version": 1, "config": {"ticket": manual, "reason": "not_planned"}},
        {"id": "end", "kind": "end", "version": 1, "config": {"quiet": true}}],
      "edges": [{"from": "start", "to": "act"}]});
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let uri = format!("/api/workflows/{}/run", w["id"].as_str().unwrap());
    let (s, r) = call(&env.app, "POST", &uri, Some(json!({"input": {}}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    assert_eq!(closed(&env, "o/r", 1), Some(CloseReason::NotPlanned));
}

/// A tool's answer as JSON (`mcp <tool>: <json>`).
fn answer(said: &str) -> Value {
    serde_json::from_str(said.split_once(": ").map_or(said, |(_, j)| j)).unwrap_or_else(|_| panic!("{said}"))
}

/// Calls a tool that asks, and approves the request.
async fn allowed(env: &Env, id: &str, tool: &str, args: Value) -> String {
    let n = agent_says(env, id).await.len();
    send(env, id, &format!("mcp: {tool} {args}")).await;
    let req = open_request(env, id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    wait_said(env, id, n + 1).await.pop().unwrap()
}

#[tokio::test]
async fn the_pm_links_and_unlinks_issues_with_the_users_yes() {
    let env = setup_github().await;
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;

    let said =
        allowed(&env, &id, "link_issue", json!({"ticket": "PX-1", "issue": "https://github.com/o/r/issues/1"})).await;
    assert!(said.starts_with("mcp link_issue:") && said.contains("Crash on start"), "{said}");
    assert_eq!(answer(&said)["issue"]["title"], "Crash on start", "the one linked: {said}");
    let said = allowed(
        &env,
        &id,
        "link_issue",
        json!({"ticket": "PX-1", "issue": "https://www.figma.com/file/abc", "label": "Figma"}),
    )
    .await;
    assert_eq!(answer(&said)["ticket"]["links"][0]["label"], "Figma", "a plain link: {said}");
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let px1 = t["items"].as_array().unwrap().iter().find(|t| t["key"] == "PX-1").unwrap().clone();
    assert_eq!((&px1["external"][0]["key"], &px1["external"][0]["closeOnDone"]), (&json!("o/r#1"), &json!(false)));

    let said = allowed(&env, &id, "unlink_issue", json!({"ticket": "PX-1", "issue": "o/r#1"})).await;
    assert_eq!(answer(&said)["ticket"]["issues"], json!([]), "{said}");
    assert_eq!(answer(&said)["issue"]["key"], "o/r#1", "the one removed: {said}");

    // create_ticket links what it is given as it makes the ticket.
    let said = allowed(
        &env,
        &id,
        "create_ticket",
        json!({"project": "PX", "title": "Upstream", "external": ["https://github.com/other/lib/issues/9"]}),
    )
    .await;
    assert!(said.contains("other/lib#9"), "{said}");
    // One no tracker can read stops it before anything is made.
    let n = ticket_titles(&env).await.len();
    let said =
        allowed(&env, &id, "create_ticket", json!({"project": "PX", "title": "Nope", "external": ["o/r#404"]})).await;
    assert!(said.contains("no issue"), "{said}");
    assert_eq!(ticket_titles(&env).await.len(), n);
}
