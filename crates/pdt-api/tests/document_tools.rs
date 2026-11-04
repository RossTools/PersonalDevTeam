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
