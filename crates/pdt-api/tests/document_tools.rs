//! PDT-317 end to end: the document tools of agents and the PM against a fake provider (list, read, edit docs and
//! sheets, charts, comments fenced as outside input, create and link), asking the user before writes unless the
//! project lets the PM act, and the documents open beside a conversation, opened by the user or by the agent's tool
//! (`fake-agent` stands in for Claude: it calls pdt-mcp tools when a message says `mcp: <tool> <json>`, asking first
//! for tools `--allowedTools` leaves out).

use crate::common;
use axum::http::StatusCode;
use common::call;
use common::pm::*;
use pdt_api::documents::DocumentProviders;
use pdt_api::documents::testing::FakeProvider;
use pdt_core::document::DocumentKind;
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup_docs() -> (Env, Arc<FakeProvider>) {
    let fake = Arc::new(FakeProvider::new("fake", "Fake"));
    let given = fake.clone();
    let env = setup_with(move |st| st.documents = DocumentProviders::new(vec![given])).await;
    (env, fake)
}

/// A tool's answer as JSON (`mcp <tool>: <json>`); an error's text as a string.
fn answer(said: &str) -> Value {
    let json = said.split_once(": ").map_or(said, |(_, j)| j);
    serde_json::from_str(json).unwrap_or_else(|_| json!(said))
}

/// Calls a tool that asks, and approves the request.
async fn allowed(env: &Env, id: &str, tool: &str, args: Value) -> Value {
    let n = agent_says(env, id).await.len();
    send(env, id, &format!("mcp: {tool} {args}")).await;
    let req = open_request(env, id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    answer(&wait_said(env, id, n + 1).await.pop().unwrap())
}

async fn tool(env: &Env, id: &str, tool: &str, args: Value) -> Value {
    answer(&ask(env, id, &format!("mcp: {tool} {args}")).await)
}

/// The text in a block of outside text (`<untrusted id="…">\n…\n</untrusted id="…">`), or panics.
fn inner(v: &Value) -> String {
    let s = v.as_str().unwrap_or_else(|| panic!("not a string: {v}"));
    assert!(s.starts_with("<untrusted id=\""), "not fenced: {s}");
    let open = s.split_once('\n').unwrap_or_else(|| panic!("{s}")).1;
    open.rsplit_once('\n').unwrap_or_else(|| panic!("{s}")).0.to_string()
}

async fn beside(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/documents"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn tab_titles(b: &Value) -> Vec<String> {
    b["tabs"].as_array().unwrap().iter().map(|t| t["document"]["title"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn the_pm_makes_a_sheet_with_formulas_and_a_chart_and_opens_it_beside_the_chat() {
    let (env, _) = setup_docs().await;
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;

    // Making it asks first: nothing before the yes.
    let n = agent_says(&env, &id).await.len();
    let create = json!({"title": "Budget", "kind": "sheet", "project": "PX", "openBeside": true});
    send(&env, &id, &format!("mcp: document_create {create}")).await;
    let req = open_request(&env, &id).await;
    let (_, list) = call(&env.app, "GET", "/api/documents", None).await;
    assert_eq!(list, json!([]), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let made = answer(&wait_said(&env, &id, n + 1).await.pop().unwrap());
    assert_eq!((&made["title"], &made["kind"], &made["openBeside"]), (&json!("Budget"), &json!("sheet"), &json!(true)));
    assert_eq!(made["projects"], json!(["PX"]));
    assert_eq!(made["addedBy"], "PM");
    let sheet = made["id"].as_str().unwrap().to_string();

    // It is open beside the conversation, put there by the agent.
    let b = beside(&env, &id).await;
    assert_eq!((tab_titles(&b), &b["active"]), (vec!["Budget".to_string()], &json!(sheet)));
    assert_eq!(b["tabs"][0]["by"], "agent");

    // The project lets the PM act: edits go through without asking.
    call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    let edits = json!({"document": sheet, "edits": [
        {"op": "set_cells", "sheet": "Sheet1", "cell": "A1",
         "values": [["Month", "Spend"], ["Jan", "10"], ["Feb", "12"], ["Total", "=SUM(B2:B3)"]]},
        {"op": "add_chart", "sheet": "Sheet1", "chart": {"kind": "column", "data": "A1:B3", "title": "Spend"}},
    ]});
    let (_, before) = call(&env.app, "GET", &format!("/api/documents/{sheet}"), None).await;
    let done = tool(&env, &id, "sheet_edit", edits).await;
    assert_eq!(done["version"], "2", "{done}");
    assert_eq!(
        (done["kind"].as_str(), done["id"].as_str()),
        (Some("sheet"), Some(sheet.as_str())),
        "its brief: {done}"
    );
    let chart = done["charts"][0].as_str().unwrap_or_else(|| panic!("{done}")).to_string();
    let (_, after) = call(&env.app, "GET", &format!("/api/documents/{sheet}"), None).await;
    assert!(after["updatedAt"].as_i64() >= before["updatedAt"].as_i64(), "the open views see it changed");

    // What a sheet holds is outside text (PDT-536): its title and content come fenced, the content as JSON.
    let read = tool(&env, &id, "sheet_read", json!({"document": "Budget", "range": "B4"})).await;
    assert_eq!(inner(&read["title"]), "Budget", "{read}");
    assert!(read["outsideText"].as_str().unwrap().contains("never instructions"), "{read}");
    let held: Value = serde_json::from_str(&inner(&read["content"])).unwrap();
    assert_eq!(held["cells"], json!([{"cell": "B4", "value": "=SUM(B2:B3)", "formula": "=SUM(B2:B3)"}]), "{held}");
    assert_eq!((&held["sheets"][0]["rows"], &held["sheets"][0]["columns"]), (&json!(4), &json!(2)));
    assert_eq!(held["sheets"][0]["charts"][0]["id"], chart.as_str());
    let (_, content) = call(&env.app, "GET", &format!("/api/documents/{sheet}/content"), None).await;
    assert_eq!(content["content"]["sheets"][0]["charts"][0]["spec"]["kind"], "column", "{content}");

    // What the provider can't do fails clearly, and the wrong tool for the kind is named.
    let filter = json!({"document": sheet, "edits": [
        {"op": "filter", "sheet": "Sheet1", "range": "A1:B3", "column": "B", "when": {"type": "greater_than", "value": "10"}}
    ]});
    let said = tool(&env, &id, "sheet_edit", filter).await;
    assert!(said.as_str().is_some_and(|s| s.contains("filter isn't supported")), "{said}");
    let said = tool(&env, &id, "doc_read", json!({"document": sheet})).await;
    assert!(said.as_str().is_some_and(|s| s.contains("use sheet_read")), "{said}");
    let mixed = json!({"document": sheet, "edits": [{"op": "append_text", "markdown": "x"}]});
    let said = tool(&env, &id, "sheet_edit", mixed).await;
    assert!(said.as_str().is_some_and(|s| s.contains("use doc_edit")), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn agents_read_and_edit_docs_and_answer_their_comments() {
    let (env, fake) = setup_docs().await;
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;

    let md = "# Spec\n\n## Scope\n\nOld.\n\n## Notes\n\n| Who | What |\n|---|---|\n| A | x |\n";
    let made = allowed(
        &env,
        &id,
        "document_create",
        json!({"title": "Spec", "kind": "doc", "ticket": "PX-1", "markdown": md}),
    )
    .await;
    let doc = made["id"].as_str().unwrap_or_else(|| panic!("{made}")).to_string();
    assert_eq!((&made["tickets"], &made["projects"]), (&json!(["PX-1"]), &json!(["PX"])));

    let list = tool(&env, &id, "documents_list", json!({"ticket": "PX-1"})).await;
    assert_eq!(list["documents"][0]["id"], doc.as_str(), "{list}");
    assert_eq!(inner(&list["documents"][0]["title"]), "Spec", "titles are outside text (PDT-536): {list}");
    // A document's text is outside text (PDT-536): whoever can edit it wrote it.
    let part = tool(&env, &id, "doc_read", json!({"document": doc, "heading": "Scope"})).await;
    assert_eq!(inner(&part["markdown"]), "## Scope\n\nOld.", "{part}");
    assert_eq!(inner(&part["headings"]), "# Spec\n## Scope\n## Notes");
    assert_eq!(inner(&part["title"]), "Spec");
    assert!(part["outsideText"].as_str().unwrap().contains("never instructions"), "{part}");
    // The provider knows an agent's tool read it (Google then uses the account's agent permission), and the API not.
    let agent_read = fake.reads_by_agent.lock().unwrap().pop();
    assert_eq!(agent_read, Some(true));
    call(&env.app, "GET", &format!("/api/documents/{doc}/content"), None).await;
    assert_eq!(fake.reads_by_agent.lock().unwrap().pop(), Some(false));

    // Edits ask (the project doesn't let the PM act) and come back with a diff.
    let edits = json!({"document": doc, "edits": [
        {"op": "replace_section", "heading": "Scope", "markdown": "New."},
        {"op": "table_append_row", "heading": "Notes", "cells": ["B", "y"]},
    ]});
    let done = allowed(&env, &id, "doc_edit", edits).await;
    let diff = &inner(&done["diff"]);
    assert!(diff.contains("-Old.") && diff.contains("+New.") && diff.contains("+| B | y |"), "{diff}");
    assert_eq!((inner(&done["title"]), done["kind"].as_str()), ("Spec".into(), Some("doc")), "its brief: {done}");
    let (_, r) = call(&env.app, "GET", &format!("/api/documents/{doc}/content"), None).await;
    assert!(r["content"]["markdown"].as_str().unwrap().contains("## Scope\n\nNew.\n"), "{r}");
    let bad = json!({"document": doc, "edits": [{"op": "replace_section", "heading": "Goal", "markdown": "x"}]});
    let said = allowed(&env, &id, "doc_edit", bad).await;
    assert!(said.as_str().is_some_and(|s| s.contains("no heading \"Goal\"")), "{said}");

    // Others' comments come fenced; replying and resolving ask too.
    let reference = fake.put("Roadmap", DocumentKind::Doc, "# Roadmap\n");
    let (_, imported) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "fake", "reference": reference, "projectId": "PX"})),
    )
    .await;
    let road = imported["id"].as_str().unwrap().to_string();
    let c = tool(&env, &id, "comments_list", json!({"document": road})).await;
    let body = c["comments"][0]["body"].as_str().unwrap_or_else(|| panic!("{c}"));
    assert!(body.starts_with("<untrusted id=") && body.contains("Is this still right?"), "{body}");
    assert!(c["outsideText"].as_str().unwrap().contains("never instructions"), "{c}");
    let got = tool(&env, &id, "document_get", json!({"document": "Roadmap"})).await;
    assert!(got["comments"][0]["body"].as_str().unwrap().starts_with("<untrusted"), "{got}");
    assert_eq!(got["capabilities"]["comments"], true);
    let r = allowed(&env, &id, "comment_reply", json!({"document": road, "comment": "c1", "body": "Yes."})).await;
    assert_eq!(r["replied"], true, "{r}");
    let r = allowed(&env, &id, "comment_resolve", json!({"document": road, "comment": "c1"})).await;
    assert_eq!(r["resolved"], true, "{r}");
    let c = tool(&env, &id, "comments_list", json!({"document": road})).await;
    assert_eq!(c["comments"], json!([]), "resolved ones are left out");

    // Linking, and opening beside by tool, which brings an open tab forward.
    let l = allowed(&env, &id, "document_link", json!({"document": road, "ticket": "PX-1"})).await;
    assert_eq!(l["tickets"], json!(["PX-1"]), "{l}");
    let o = tool(&env, &id, "document_open_beside", json!({"document": doc})).await;
    assert_eq!(o["tabs"].as_array().unwrap().iter().map(inner).collect::<Vec<_>>(), ["Spec"], "{o}");
    tool(&env, &id, "document_open_beside", json!({"document": road})).await;
    let o = tool(&env, &id, "document_open_beside", json!({"document": doc})).await;
    assert_eq!(o["tabs"].as_array().unwrap().iter().map(inner).collect::<Vec<_>>(), ["Spec", "Roadmap"], "{o}");
    assert_eq!(beside(&env, &id).await["active"], doc.as_str());
    let said = tool(&env, &id, "doc_read", json!({"document": "Nope"})).await;
    assert!(said.as_str().is_some_and(|s| s.contains("no document is called Nope")), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn the_user_opens_several_documents_in_tabs_beside_a_conversation() {
    let (env, _) = setup_docs().await;
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;
    let mut ids = vec![];
    for title in ["One", "Two", "Three"] {
        let (s, d) =
            call(&env.app, "POST", "/api/documents", Some(json!({"title": title, "kind": "doc", "by": "user"}))).await;
        assert_eq!(s, StatusCode::CREATED, "{d}");
        ids.push(d["id"].as_str().unwrap().to_string());
    }
    let url = format!("/api/sessions/{id}/documents");
    for d in &ids {
        let (s, b) = call(&env.app, "POST", &url, Some(json!({"documentId": d, "by": "user"}))).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let b = beside(&env, &id).await;
    assert_eq!(tab_titles(&b), ["One", "Two", "Three"]);
    assert_eq!((&b["active"], &b["tabs"][0]["by"]), (&json!(ids[2]), &json!("user")));

    let (_, b) = call(&env.app, "PUT", &url, Some(json!({"documentIds": [ids[2], ids[0]]}))).await;
    assert_eq!(tab_titles(&b), ["Three", "One", "Two"]);
    let (_, b) = call(&env.app, "DELETE", &format!("{url}/{}", ids[2]), None).await;
    assert_eq!(tab_titles(&b), ["One", "Two"]);
    assert_eq!(b["active"], ids[1].as_str(), "the newest focused of the rest shows");

    let (s, _) = call(&env.app, "POST", &url, Some(json!({"documentId": "nope"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", "/api/sessions/nope/documents", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// An agent's conversation on PX-1 in Bypass, where PDT asks for its asking tools itself, as for a Coder's step: not
/// the PM, so the project never lets it act without asking.
async fn start_ticket_agent(env: &Env) -> String {
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let t = list["items"].as_array().unwrap().iter().find(|t| t["key"] == "PX-1").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let body = json!({"agent": "claude", "sessionMode": "conversation", "permissionMode": "bypass"});
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{t}/start"), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["session"]["id"].as_str().unwrap().to_string();
    wait_said(env, &id, 1).await;
    id
}

/// The open approval in the Inbox for `session`.
async fn approval(env: &Env, session: &str) -> Value {
    common::wait::wait_until("an approval in the Inbox", async || {
        let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
        let items = v["items"].as_array().cloned().unwrap_or_default();
        items.into_iter().find(|i| i["kind"] == "approval" && i["sessionId"] == session).ok_or(v)
    })
    .await
}

/// Answers an approval as the user's click in PDT's web.
async fn answer_as_user(env: &Env, item: &Value, a: Value) {
    let (session, request) = (item["sessionId"].as_str().unwrap(), item["body"]["requestId"].as_str().unwrap());
    let url = format!("/api/sessions/{session}/requests/{request}");
    let (s, v) = common::call_as_ui(&env.app, "POST", &url, Some(json!({"answer": a}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
}

async fn ticket_docs(env: &Env) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", "/api/documents?ticket=PX-1", None).await;
    v.as_array().unwrap_or_else(|| panic!("{v}")).iter().filter_map(|d| d["title"].as_str().map(String::from)).collect()
}

/// PDT-613: a ticket's agent's `document_create` for its own ticket asks the user in the Inbox (and its conversation), and
/// waits: Claude's idle timeout doesn't cut the wait short. The user's Allow creates and links the document; a Deny
/// tells the agent no. An agent can't allow it.
#[tokio::test]
async fn an_agents_document_create_waits_for_the_users_answer_in_the_inbox() {
    let (env, _fake) = setup_docs().await;
    let id = start_ticket_agent(&env).await;
    // Claude would give up on a call that waits for the user before PDT does (PDT-613).
    let idle = ask(&env, &id, "mcp-idle").await;
    let ms: u128 =
        idle.strip_prefix("mcp idle timeout ").and_then(|m| m.parse().ok()).unwrap_or_else(|| panic!("{idle}"));
    assert!(ms > env.state.ask_wait.as_millis(), "{idle}, PDT waits {:?}", env.state.ask_wait);

    let create = |title: &str| {
        let args = json!({"title": title, "kind": "doc", "ticket": "PX-1", "markdown": format!("# {title}\n")});
        format!("mcp: document_create {args}")
    };
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("Spike notes")).await;
    let item = approval(&env, &id).await;
    assert_eq!(item["ticketKey"], "PX-1", "{item}");
    assert_eq!(item["body"]["input"]["title"], "Spike notes", "{item}");
    assert!(ticket_docs(&env).await.is_empty(), "nothing before the yes");
    // An agent's curl can't say yes.
    let url = format!("/api/sessions/{id}/requests/{}", item["body"]["requestId"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    answer_as_user(&env, &item, json!({"type": "approve"})).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    let made = answer(&said);
    assert_eq!(made["tickets"], json!(["PX-1"]), "created and linked: {said}");
    assert_eq!(ticket_docs(&env).await, ["Spike notes"]);

    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("Not wanted")).await;
    let item = approval(&env, &id).await;
    answer_as_user(&env, &item, json!({"type": "deny", "message": "Comment instead"})).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(
        said.contains("denied this document_create call") && said.contains("Comment instead"),
        "a clear refusal: {said}"
    );
    assert_eq!(ticket_docs(&env).await, ["Spike notes"]);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-613: a ticket's agent's `document_create` the user hasn't answered within PDT's wait (shortened here) returns, telling
/// the agent the request waits in PDT's Inbox; the user's later Allow creates the document and tells the agent.
#[tokio::test]
async fn an_agents_unanswered_document_create_says_it_waits_in_the_inbox() {
    let fake = Arc::new(FakeProvider::new("fake", "Fake"));
    let given = fake.clone();
    let env = setup_with(move |st| {
        st.documents = DocumentProviders::new(vec![given]);
        st.ask_wait = std::time::Duration::from_millis(300);
    })
    .await;
    let id = start_ticket_agent(&env).await;
    let n = agent_says(&env, &id).await.len();
    let args = json!({"title": "Late", "kind": "doc", "ticket": "PX-1", "markdown": "# Late\n"});
    send(&env, &id, &format!("mcp: document_create {args}")).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(
        said.contains("Still waiting for the user's answer") && said.contains("PDT's Inbox"),
        "names where the user answers: {said}"
    );
    let item = approval(&env, &id).await;
    assert!(ticket_docs(&env).await.is_empty());
    let n = agent_says(&env, &id).await.len();
    answer_as_user(&env, &item, json!({"type": "approve"})).await;
    common::wait::wait_until("the agent to hear it ran", async || {
        pdt_api::run_workflows(&env.state).await;
        let said = agent_says(&env, &id).await;
        said[n..].iter().find(|m| m.contains("The user allowed your document_create")).cloned().ok_or(said)
    })
    .await;
    assert_eq!(ticket_docs(&env).await, ["Late"]);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// Sets what the project lets agents do to documents without asking, as the user in PDT's web (PDT-618).
async fn agent_documents(env: &Env, level: &str) {
    let url = format!("/api/projects/{}/settings", env.project);
    let (s, v) = common::call_as_ui(&env.app, "PUT", &url, Some(json!({"agentDocuments": level}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["policy"]["agentDocuments"], level);
}

/// A second ticket, PX-2, and two documents the agent's ticket isn't linked to: one on PX-2, one on the project only.
/// Returns their ids.
async fn others(env: &Env) -> (String, String) {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Other"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let make = |body: Value| async move {
        let (s, d) = call(&env.app, "POST", "/api/documents", Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{d}");
        d["id"].as_str().unwrap().to_string()
    };
    let other = make(json!({"title": "Theirs", "kind": "doc", "ticketId": "PX-2", "markdown": "# Theirs\n"})).await;
    let plan = make(json!({"title": "Plan", "kind": "doc", "projectId": "PX", "markdown": "# Plan\n\nOld.\n"})).await;
    (other, plan)
}

/// Calls a tool that must ask: its approval comes (in the Inbox, or as the conversation's request), the user denies
/// it, and the agent hears no.
async fn asks(env: &Env, id: &str, tool: &str, args: Value) {
    let n = agent_says(env, id).await.len();
    send(env, id, &format!("mcp: {tool} {args}")).await;
    let req = open_request(env, id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, v) = common::call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let said = wait_said(env, id, n + 1).await.pop().unwrap();
    assert!(said.contains("denied"), "{tool} asked: {said}");
}

/// No request opened in the session (every call so far went through without asking).
async fn never_asked(env: &Env, id: &str) {
    let asked: Vec<Value> = events(env, id).await.into_iter().filter(|e| e["event"]["kind"] == "request").collect();
    assert!(asked.is_empty(), "{asked:?}");
}

async fn changes(env: &Env, doc: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/changes"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

/// The `document` events in PX-1's history (oldest first).
async fn ticket_changes(env: &Env) -> Vec<Value> {
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let t = list["items"].as_array().unwrap().iter().find(|t| t["key"] == "PX-1").unwrap()["id"].clone();
    let (_, v) = call(&env.app, "GET", &format!("/api/tickets/{}/events?limit=200", t.as_str().unwrap()), None).await;
    let items = v.get("items").unwrap_or(&v).as_array().unwrap_or_else(|| panic!("{v}")).clone();
    items.into_iter().filter(|e| e["kind"] == "document").collect()
}

/// PDT-618: only the user sets what agents may do to documents unasked; an agent's API call can't.
#[tokio::test]
async fn only_the_user_lets_agents_change_documents_unasked() {
    let (env, _) = setup_docs().await;
    let url = format!("/api/projects/{}/settings", env.project);
    let (_, v) = call(&env.app, "GET", &url, None).await;
    assert_eq!(v["policy"]["agentDocuments"], "off", "off by default");
    let (s, v) = call(&env.app, "PUT", &url, Some(json!({"agentDocuments": "project"}))).await;
    assert_eq!((s, &v["code"]), (StatusCode::FORBIDDEN, &json!("not_the_user")), "{v}");
    agent_documents(&env, "own_ticket").await;
    // Resending the stored level (another section's save) is no change, so it goes through.
    let (s, v) = call(&env.app, "PUT", &url, Some(json!({"agentDocuments": "own_ticket", "pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// PDT-618, "Own ticket": a ticket's agent (PDT asks for it: Bypass) creates, edits and links its ticket's documents and
/// sheets with no prompt, each change recorded on the document and the ticket; another ticket's or a project-only
/// document, and comment replies, still ask.
#[tokio::test]
async fn own_ticket_lets_an_agent_change_its_tickets_documents_unasked() {
    let (env, fake) = setup_docs().await;
    let (theirs, plan) = others(&env).await;
    let id = start_ticket_agent(&env).await;
    let t = tool(&env, &id, "get_ticket", json!({})).await;
    assert!(t.get("documentsWithoutAsking").is_none(), "off: nothing to say");
    agent_documents(&env, "own_ticket").await;
    // The agent learns what it may do without asking.
    let t = tool(&env, &id, "get_ticket", json!({})).await;
    assert_eq!(t["documentsWithoutAsking"], "own_ticket", "{t}");
    assert!(t["documentsWithoutAskingNote"].as_str().unwrap().contains("linked to your own ticket"));

    let made =
        tool(&env, &id, "document_create", json!({"title": "Notes", "kind": "doc", "markdown": "# Notes\n\nOld.\n"}))
            .await;
    assert_eq!(made["tickets"], json!(["PX-1"]), "its own ticket: {made}");
    let notes = made["id"].as_str().unwrap().to_string();
    let edits = json!({"document": notes, "edits": [{"op": "replace_string", "find": "Old.", "replace": "New."}]});
    let done = tool(&env, &id, "doc_edit", edits).await;
    assert!(inner(&done["diff"]).contains("+New."), "{done}");
    let sheet = tool(&env, &id, "document_create", json!({"title": "Costs", "kind": "sheet", "ticket": "PX-1"})).await;
    let sheet = sheet["id"].as_str().unwrap_or_else(|| panic!("{sheet}")).to_string();
    let cells =
        json!({"document": sheet, "edits": [{"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [["1"]]}]});
    assert_eq!(tool(&env, &id, "sheet_edit", cells).await["id"], sheet.as_str());
    let linked = tool(&env, &id, "document_link", json!({"document": notes, "ticket": "PX-1"})).await;
    assert_eq!(linked["tickets"], json!(["PX-1"]), "{linked}");
    never_asked(&env, &id).await;

    // Each is recorded: on the document with the agent and its diff, and in its ticket's history.
    let c = changes(&env, &notes).await;
    let tools: Vec<&str> = c.iter().map(|c| c["tool"].as_str().unwrap()).collect();
    assert_eq!(tools, ["document_link", "doc_edit", "document_create"], "newest first: {c:?}");
    assert!(c[1]["diff"].as_str().unwrap().contains("-Old.\n+New."), "{c:?}");
    assert_eq!((&c[1]["agent"]["fromTicket"], &c[1]["summary"]), (&json!("PX-1"), &json!("Edited (1 edit)")));
    assert_eq!(c[1]["version"], done["version"]);
    let s = changes(&env, &sheet).await;
    assert!(s[0]["summary"].as_str().unwrap().contains("set_cells") && s[0]["version"].is_string(), "{s:?}");
    let on_ticket = ticket_changes(&env).await;
    assert_eq!(on_ticket.len(), 5, "{on_ticket:?}");
    let edit = on_ticket.iter().find(|e| e["data"]["tool"] == "doc_edit").unwrap();
    assert_eq!((&edit["data"]["documentId"], &edit["data"]["title"]), (&json!(notes), &json!("Notes")));
    assert!(
        edit["body"].as_str().unwrap().starts_with("```diff\n") && edit["body"].as_str().unwrap().contains("+New.")
    );
    assert_eq!(edit["actor"], "agent");

    // Beyond its ticket it asks: another ticket's document, a project-only one, a new one elsewhere, a link elsewhere.
    let other_edit = json!({"op": "append_text", "markdown": "x"});
    asks(&env, &id, "doc_edit", json!({"document": theirs, "edits": [other_edit]})).await;
    asks(&env, &id, "doc_edit", json!({"document": plan, "edits": [other_edit]})).await;
    asks(&env, &id, "document_create", json!({"title": "Elsewhere", "kind": "doc", "ticket": "PX-2"})).await;
    asks(&env, &id, "document_create", json!({"title": "Project", "kind": "doc", "project": "PX"})).await;
    asks(&env, &id, "document_link", json!({"document": notes, "ticket": "PX-2"})).await;
    asks(&env, &id, "document_link", json!({"document": plan, "ticket": "PX-1"})).await;
    let reference = fake.put("Roadmap", DocumentKind::Doc, "# Roadmap\n");
    let body = json!({"provider": "fake", "reference": reference, "ticketId": "PX-1"});
    let (_, road) = call(&env.app, "POST", "/api/documents/import", Some(body)).await;
    let road = road["id"].as_str().unwrap_or_else(|| panic!("{road}")).to_string();
    asks(&env, &id, "comment_reply", json!({"document": road, "comment": "c1", "body": "Yes."})).await;
    assert_eq!(changes(&env, &plan).await, Vec::<Value>::new(), "nothing recorded for what it asked");
    assert_eq!(ticket_changes(&env).await.len(), 5);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-618, "Project": an agent whose Claude asks through its permission prompt gets PDT's yes at once for any
/// document linked to the project, and for new ones on the project or its own ticket; another ticket's new document
/// and comment resolves still ask. Off again, its own ticket's document asks too.
#[tokio::test]
async fn project_lets_an_agent_change_the_projects_documents_unasked() {
    let (env, fake) = setup_docs().await;
    let (theirs, plan) = others(&env).await;
    let (_, list) = call(&env.app, "GET", &format!("/api/tickets?project={}", env.project), None).await;
    let t = list["items"].as_array().unwrap().iter().find(|t| t["key"] == "PX-1").unwrap()["id"].clone();
    let body = json!({"agent": "claude", "sessionMode": "conversation", "permissionMode": "ask"});
    let (s, v) = call(&env.app, "POST", &format!("/api/tickets/{}/start", t.as_str().unwrap()), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["session"]["id"].as_str().unwrap().to_string();
    wait_said(&env, &id, 1).await;
    agent_documents(&env, "project").await;

    let edits = json!({"document": plan, "edits": [{"op": "replace_section", "heading": "Plan", "markdown": "New."}]});
    let done = tool(&env, &id, "doc_edit", edits).await;
    assert!(inner(&done["diff"]).contains("+New."), "{done}");
    let edits = json!({"document": theirs, "edits": [{"op": "append_text", "markdown": "More."}]});
    tool(&env, &id, "doc_edit", edits).await;
    let made = tool(&env, &id, "document_create", json!({"title": "Overview", "kind": "doc", "project": "PX"})).await;
    assert_eq!(made["projects"], json!(["PX"]), "{made}");
    let linked = tool(&env, &id, "document_link", json!({"document": plan, "ticket": "PX-1"})).await;
    assert_eq!(linked["tickets"], json!(["PX-1"]), "{linked}");
    // Claude asked each time, and PDT said yes for the user.
    let requests: Vec<Value> =
        events(&env, &id).await.into_iter().filter(|e| e["event"]["kind"] == "request").collect();
    assert_eq!(requests.len(), 4, "{requests:?}");
    assert!(requests.iter().all(|r| r["event"]["state"] != "open"), "{requests:?}");
    let c = changes(&env, &plan).await;
    assert_eq!(c.iter().map(|c| c["tool"].as_str().unwrap()).collect::<Vec<_>>(), ["document_link", "doc_edit"]);
    assert_eq!(c[0]["summary"], "Linked to PX-1");
    assert_eq!(ticket_changes(&env).await.len(), 4, "on the agent's own ticket, whichever document");

    asks(&env, &id, "document_create", json!({"title": "Theirs too", "kind": "doc", "ticket": "PX-2"})).await;
    let reference = fake.put("Roadmap", DocumentKind::Doc, "# Roadmap\n");
    let body = json!({"provider": "fake", "reference": reference, "projectId": "PX"});
    let (_, road) = call(&env.app, "POST", "/api/documents/import", Some(body)).await;
    let road = road["id"].as_str().unwrap_or_else(|| panic!("{road}")).to_string();
    asks(&env, &id, "comment_resolve", json!({"document": road, "comment": "c1"})).await;

    agent_documents(&env, "off").await;
    let edits = json!({"document": made["id"], "edits": [{"op": "append_text", "markdown": "x"}]});
    asks(&env, &id, "doc_edit", edits).await;
    assert_eq!(ticket_changes(&env).await.len(), 4);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-709: before an agent's unasked change to a local sheet PDT keeps what it was, charts too, and Restore puts that
/// back; a provider with its own history keeps no copy.
#[tokio::test]
async fn a_local_sheets_unasked_change_restores_as_it_was_before() {
    let env = setup().await;
    let id = start_ticket_agent(&env).await;
    agent_documents(&env, "own_ticket").await;
    let made = tool(&env, &id, "document_create", json!({"title": "Costs", "kind": "sheet"})).await;
    let sheet = made["id"].as_str().unwrap_or_else(|| panic!("{made}")).to_string();
    assert_eq!(made["provider"], "local", "{made}");
    let first = json!({"document": sheet, "edits": [
        {"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [["Rent"], ["12"]]},
        {"op": "add_chart", "sheet": "Sheet1", "chart": {"kind": "bar", "data": "A1:B2"}},
    ]});
    let first = tool(&env, &id, "sheet_edit", first).await;
    let chart = first["charts"][0].as_str().unwrap_or_else(|| panic!("{first}")).to_string();
    let second = json!({"document": sheet, "edits": [
        {"op": "set_cells", "sheet": "Sheet1", "cell": "A2", "values": [["99"]]},
        {"op": "delete_chart", "id": chart},
    ]});
    tool(&env, &id, "sheet_edit", second).await;
    never_asked(&env, &id).await;

    let c = changes(&env, &sheet).await;
    let restorable: Vec<bool> = c.iter().map(|c| c["restorable"].as_bool().unwrap()).collect();
    assert_eq!(restorable, [true, true, false], "the edits keep a copy, the create has nothing before it: {c:?}");
    let url = format!("/api/documents/{sheet}/changes/{}/restore", c[0]["id"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &url, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, read) = call(&env.app, "GET", &format!("/api/documents/{sheet}/content"), None).await;
    assert_eq!(read["content"]["version"], v["version"], "{read}");
    let sheet1 = &read["content"]["sheets"][0];
    assert_eq!(sheet1["rows"][1][0]["value"], "12", "{read}");
    assert_eq!(sheet1["charts"][0]["id"], chart.as_str(), "its chart is back: {read}");

    // A change that kept nothing has nothing to restore.
    let url = format!("/api/documents/{sheet}/changes/{}/restore", c[2]["id"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &url, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-709: a provider with its own history (the fake here) keeps no copy, so its changes have no Restore.
#[tokio::test]
async fn another_providers_unasked_change_keeps_no_copy() {
    let (env, _) = setup_docs().await;
    let id = start_ticket_agent(&env).await;
    agent_documents(&env, "own_ticket").await;
    let made = tool(&env, &id, "document_create", json!({"title": "Notes", "kind": "doc", "markdown": "Old.\n"})).await;
    let notes = made["id"].as_str().unwrap().to_string();
    let edits = json!({"document": notes, "edits": [{"op": "replace_string", "find": "Old.", "replace": "New."}]});
    tool(&env, &id, "doc_edit", edits).await;
    let c = changes(&env, &notes).await;
    assert_eq!(c[0]["tool"], "doc_edit");
    assert_eq!(c[0]["restorable"], false, "{c:?}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
