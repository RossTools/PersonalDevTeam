mod common;

use axum::http::StatusCode;
use common::call;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;

/// A router with one project (no git needed: tickets never touch the checkout) and that project's id.
async fn setup(tmp: &Path) -> (axum::Router, AppState, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    common::git(&repo, &["init", "-q", "-b", "main"]);
    common::git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.join("data"));
    let app = router(state.clone(), None);
    let body = json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    (app, state, p["id"].as_str().unwrap().to_string())
}

async fn create(app: &axum::Router, pid: &str, body: Value) -> Value {
    let (s, v) = call(app, "POST", &format!("/api/projects/{pid}/tickets"), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

async fn list(app: &axum::Router, query: &str) -> Value {
    let (s, v) = call(app, "GET", &format!("/api/tickets?{query}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn keys(page: &Value) -> Vec<String> {
    page["items"].as_array().unwrap().iter().map(|t| t["key"].as_str().unwrap().to_string()).collect()
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn tickets_are_created_listed_edited_and_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _st, pid) = setup(tmp.path()).await;
    let a = create(&app, &pid, json!({"title": "  Fix login ", "labels": ["Bug", "bug"], "priority": "high"})).await;
    assert_eq!(a["key"], "PDT-1");
    assert_eq!(a["title"], "Fix login");
    assert_eq!(a["stage"], "draft");
    assert_eq!(a["labels"], json!(["bug"]));
    assert_eq!(a["description"], "");
    assert_eq!(a["blockedBy"], 0);
    assert_eq!(a["overlap"], "none");
    let b = create(&app, &pid, json!({"title": "Write docs", "description": "All of them", "stage": "ready"})).await;

    let page = list(&app, &format!("project={pid}")).await;
    assert_eq!(page["total"], 2);
    assert_eq!(keys(&page), ["PDT-2", "PDT-1"]);
    assert_eq!(page["nextCursor"], Value::Null);
    assert_eq!(keys(&list(&app, &format!("project={pid}&q=doc")).await), ["PDT-2"]);
    assert_eq!(keys(&list(&app, &format!("project={pid}&q=pdt-1")).await), ["PDT-1"]);
    assert_eq!(keys(&list(&app, &format!("project={pid}&stage=draft,ready&priority=high")).await), ["PDT-1"]);
    assert_eq!(keys(&list(&app, &format!("project={pid}&label=bug&sort=created")).await), ["PDT-1"]);
    let page = list(&app, &format!("project={pid}&limit=1&sort=priority")).await;
    assert_eq!(keys(&page), ["PDT-1"]);
    let cursor = page["nextCursor"].as_str().unwrap().to_string();
    let next = list(&app, &format!("project={pid}&limit=1&sort=priority&cursor={}", urlencode(&cursor))).await;
    assert_eq!(keys(&next), ["PDT-2"]);

    for bad in ["stage=doing", "sort=nope", "limit=0", "limit=x", "cursor=junk"] {
        let (s, v) = call(&app, "GET", &format!("/api/tickets?{bad}"), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{bad}: {v}");
    }

    let (s, v) = call(&app, "GET", &format!("/api/tickets/{}", id(&b)), None).await;
    assert_eq!((s, v["description"].as_str()), (StatusCode::OK, Some("All of them")));
    let (s, v) = call(
        &app,
        "PATCH",
        &format!("/api/tickets/{}", id(&a)),
        Some(json!({"stage": "done", "title": "Fixed login", "labels": []})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["stage"].as_str(), v["title"].as_str()), (Some("done"), Some("Fixed login")));
    assert!(v["closedAt"].is_number());
    assert_eq!(v["labels"], json!([]));

    let (s, v) = call(&app, "GET", &format!("/api/projects/{pid}/labels"), None).await;
    assert_eq!((s, v), (StatusCode::OK, json!([])));
    let (s, _) = call(&app, "GET", "/api/projects/nope/labels", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, v) = call(&app, "DELETE", &format!("/api/tickets/{}", id(&a)), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = call(&app, "GET", &format!("/api/tickets/{}", id(&a)), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    assert_eq!(create(&app, &pid, json!({"title": "Next"})).await["key"], "PDT-3", "numbers are never reused");
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn bad_input_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _st, pid) = setup(tmp.path()).await;
    for body in [
        json!({"title": "  "}),
        json!({"title": "x".repeat(201)}),
        json!({"title": "ok", "labels": ["a,b"]}),
        json!({"title": "ok", "stage": "doing"}),
        json!({"title": "ok", "before": "nope"}),
        json!({}),
    ] {
        let (s, v) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(body.clone())).await;
        assert_eq!(s.as_u16() / 100, 4, "{body}: {v}");
        assert_eq!(v["code"], "invalid_input", "{body}: {v}");
    }
    let (s, v) = call(&app, "POST", "/api/projects/nope/tickets", Some(json!({"title": "x"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
    for (method, path, body) in [
        ("PATCH", "/api/tickets/nope", Some(json!({"title": "x"}))),
        ("DELETE", "/api/tickets/nope", None),
        ("POST", "/api/tickets/nope/rank", Some(json!({"to": "top"}))),
        ("POST", "/api/tickets/nope/comments", Some(json!({"body": "hi"}))),
        ("GET", "/api/tickets/nope/events", None),
        ("POST", "/api/tickets/nope/links", Some(json!({"to": "x", "kind": "blocks"}))),
    ] {
        let (s, v) = call(&app, method, path, body).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} {path}: {v}");
    }
    let a = create(&app, &pid, json!({"title": "A"})).await;
    let (s, v) = call(&app, "POST", &format!("/api/tickets/{}/comments", id(&a)), Some(json!({"body": " "}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, v) = call(&app, "POST", &format!("/api/tickets/{}/rank", id(&a)), Some(json!({}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
}

#[tokio::test]
async fn ranks_move_and_bulk_actions_apply_to_all_or_none() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _st, pid) = setup(tmp.path()).await;
    let c = create(&app, &pid, json!({"title": "C"})).await;
    let b = create(&app, &pid, json!({"title": "B"})).await;
    let a = create(&app, &pid, json!({"title": "A"})).await;
    let order = |app: axum::Router, pid: String| async move {
        let page = list(&app, &format!("project={pid}")).await;
        page["items"].as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(order(app.clone(), pid.clone()).await, ["A", "B", "C"]);
    let (s, v) = call(&app, "POST", &format!("/api/tickets/{}/rank", id(&a)), Some(json!({"after": id(&c)}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(order(app.clone(), pid.clone()).await, ["B", "C", "A"]);
    call(&app, "POST", &format!("/api/tickets/{}/rank", id(&a)), Some(json!({"before": id(&b)}))).await;
    assert_eq!(order(app.clone(), pid.clone()).await, ["A", "B", "C"]);
    call(&app, "POST", &format!("/api/tickets/{}/rank", id(&a)), Some(json!({"to": "bottom"}))).await;
    assert_eq!(order(app.clone(), pid.clone()).await, ["B", "C", "A"]);

    let bulk = |body: Value| {
        let app = app.clone();
        async move { call(&app, "POST", "/api/tickets/bulk", Some(body)).await }
    };
    let (s, v) = bulk(json!({"ids": [id(&a), "nope"], "action": "stage", "value": "ready"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = bulk(json!({"ids": [id(&a)], "action": "stage", "value": "doing"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, v) = bulk(json!({"ids": [id(&a), id(&b)], "action": "stage", "value": "ready"})).await;
    assert_eq!((s, v["count"].as_u64()), (StatusCode::OK, Some(2)));
    let (_, v) = bulk(json!({"ids": [id(&a), id(&c)], "action": "addLabel", "value": "UI"})).await;
    assert_eq!(v["count"], 2);
    let (_, v) = bulk(json!({"ids": [id(&a)], "action": "priority", "value": "urgent"})).await;
    assert_eq!(v["count"], 1);
    let (_, v) = bulk(json!({"ids": [id(&a), id(&c)], "action": "top"})).await;
    assert_eq!(v["count"], 2);
    assert_eq!(order(app.clone(), pid.clone()).await, ["C", "A", "B"]);
    let (s, v) = call(&app, "GET", &format!("/api/projects/{pid}/labels"), None).await;
    assert_eq!((s, v), (StatusCode::OK, json!(["ui"])));
    let page = list(&app, &format!("project={pid}&stage=ready")).await;
    assert_eq!(page["total"], 2);
    let (_, v) = bulk(json!({"ids": [id(&a), id(&b)], "action": "delete"})).await;
    assert_eq!(v["count"], 2);
    assert_eq!(order(app.clone(), pid.clone()).await, ["C"]);
}

#[tokio::test]
async fn links_comments_and_the_timeline() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st, pid) = setup(tmp.path()).await;
    let a = create(&app, &pid, json!({"title": "A"})).await;
    let b = create(&app, &pid, json!({"title": "B"})).await;
    let c = create(&app, &pid, json!({"title": "C"})).await;
    let link = |from: &Value, to: &Value, kind: &str, method: &str| {
        let (app, path, body) =
            (app.clone(), format!("/api/tickets/{}/links", id(from)), json!({"to": id(to), "kind": kind}));
        let method = method.to_string();
        async move { call(&app, &method, &path, Some(body)).await }
    };
    let (s, v) = link(&a, &b, "blocks", "POST").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["links"],
        json!([{"kind": "blocks", "direction": "outgoing", "id": id(&b), "key": "PDT-2", "title": "B", "stage": "draft"}])
    );
    link(&b, &c, "blocks", "POST").await;
    let (s, v) = link(&c, &a, "blocks", "POST").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("link_cycle")));
    assert!(v["message"].as_str().unwrap().contains("PDT-3 → PDT-1 → PDT-2 → PDT-3"), "{v}");
    let (s, v) = link(&a, &a, "relates", "POST").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));

    let page = list(&app, &format!("project={pid}&blocked=true")).await;
    assert_eq!(keys(&page), ["PDT-3", "PDT-2"]);
    let (s, v) = link(&b, &c, "blocks", "DELETE").await;
    assert_eq!((s, v["links"].as_array().map(Vec::len)), (StatusCode::OK, Some(1)));
    let (s, v) = link(&b, &c, "blocks", "DELETE").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));

    let comment =
        async |body: Value| call(&app, "POST", &format!("/api/tickets/{}/comments", id(&a)), Some(body)).await;
    let (s, v) = comment(json!({"body": "Looks good", "by": "user"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        (v["kind"].as_str(), v["body"].as_str(), v["actor"].as_str()),
        (Some("comment"), Some("Looks good"), Some("user"))
    );
    // Without `by` (an agent's curl) it is the agent's; only PDT writes system comments.
    let (s, v) = comment(json!({"body": "Done"})).await;
    assert_eq!((s, v["actor"].as_str()), (StatusCode::CREATED, Some("agent")), "{v}");
    let (s, v) = comment(json!({"body": "x", "by": "system"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{v}");
    let (s, v) = call(&app, "GET", &format!("/api/tickets/{}/events", id(&a)), None).await;
    assert_eq!(s, StatusCode::OK);
    let kinds: Vec<&str> = v["items"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["created", "linked", "comment", "comment"]);
    let (_, v) = call(&app, "GET", &format!("/api/tickets/{}/events?limit=1&newestFirst=true", id(&a)), None).await;
    assert_eq!(v["items"][0]["kind"], "comment");
    let cursor = urlencode(v["nextCursor"].as_str().unwrap());
    let (_, v) =
        call(&app, "GET", &format!("/api/tickets/{}/events?limit=5&newestFirst=true&cursor={cursor}", id(&a)), None)
            .await;
    let kinds: Vec<&str> = v["items"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["comment", "linked", "created"]);
    assert_eq!(v["nextCursor"], Value::Null);

    // A ticket with an active attempt can't be deleted.
    sqlx_insert_attempt(&st, &id(&a)).await;
    let (s, v) = call(&app, "DELETE", &format!("/api/tickets/{}", id(&a)), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ticket_active")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("PDT-1"));
    let (s, v) =
        call(&app, "POST", "/api/tickets/bulk", Some(json!({"ids": [id(&a), id(&b)], "action": "delete"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("ticket_active")));
    assert_eq!(list(&app, &format!("project={pid}")).await["total"], 3);
}

/// Attempts arrive with Start (Phase 4b); the store can already record one.
async fn sqlx_insert_attempt(st: &AppState, ticket_id: &str) {
    st.store.insert_attempt("at1", ticket_id, None, "pdt/PDT-1-a", "main", "abc123", 1).await.unwrap();
}

#[tokio::test]
async fn tickets_open_in_tabs_and_deleting_one_closes_its_tab() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _st, pid) = setup(tmp.path()).await;
    let a = create(&app, &pid, json!({"title": "A"})).await;
    let (_, ws) = call(&app, "GET", "/api/workspaces", None).await;
    let home = ws[0]["id"].as_str().unwrap().to_string();
    let (s, tab) = call(&app, "POST", &format!("/api/workspaces/{home}/tabs"), Some(json!({"ticketId": id(&a)}))).await;
    assert_eq!(s, StatusCode::OK, "{tab}");
    assert_eq!((tab["ticketId"].as_str(), tab["sessionId"].as_str()), (Some(id(&a).as_str()), None));
    let (s, v) = call(&app, "POST", &format!("/api/workspaces/{home}/tabs"), Some(json!({"ticketId": "nope"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, v) = call(
        &app,
        "POST",
        &format!("/api/workspaces/{home}/tabs"),
        Some(json!({"ticketId": id(&a), "sessionId": "x"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (_, ws) = call(&app, "GET", "/api/workspaces", None).await;
    assert_eq!(ws[0]["tabs"][0]["ticketId"], json!(id(&a)));

    call(&app, "DELETE", &format!("/api/tickets/{}", id(&a)), None).await;
    let (_, ws) = call(&app, "GET", "/api/workspaces", None).await;
    assert_eq!(ws[0]["tabs"], json!([]));
}

/// PDT-204: an external ticket is let in over the HTTP API only from PDT's web client (its UI token): an agent's curl,
/// which otherwise counts as the user, is refused.
#[tokio::test]
async fn only_the_web_client_lets_an_external_ticket_in() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st, pid) = setup(tmp.path()).await;
    let t = pdt_core::ticket::NewTicket { title: "From an issue".into(), ..Default::default() }.validate().unwrap();
    let t = pdt_core::ticket::ValidNewTicket { source: pdt_core::ticket::TicketSource::External, ..t };
    st.store.create_ticket("ext", &pid, &t, 1).await.unwrap();
    let refused = |(s, v): (StatusCode, Value)| {
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("untrusted_ticket")), "{v}");
        assert!(v["message"].as_str().unwrap().contains("PDT-1"), "{v}");
    };

    // A curl: To do, In progress, the agentic label (one at a time or in bulk) and Start are refused.
    for patch in [json!({"stage": "todo"}), json!({"stage": "in_progress"}), json!({"labels": ["agentic"]})] {
        refused(call(&app, "PATCH", "/api/tickets/ext", Some(patch)).await);
    }
    for (action, value) in [("stage", "todo"), ("addLabel", "agentic")] {
        let body = json!({"ids": ["ext"], "action": action, "value": value});
        refused(call(&app, "POST", "/api/tickets/bulk", Some(body)).await);
    }
    refused(call(&app, "POST", "/api/tickets/ext/start", Some(json!({"agent": "claude"}))).await);
    // Anything short of that goes through.
    let (s, v) = call(&app, "PATCH", "/api/tickets/ext", Some(json!({"stage": "ready", "labels": ["bug"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = call(&app, "GET", "/api/tickets/ext", None).await;
    assert_eq!((v["stage"].as_str(), v["admitted"].as_bool()), (Some("ready"), Some(false)), "{v}");

    // The token goes only to a browser's same-origin fetch; one from an earlier start is stale.
    let (s, v) = call(&app, "GET", "/api/ui/token", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_the_ui")), "{v}");
    let (s, v) = common::call_with(
        &app,
        "PATCH",
        "/api/tickets/ext",
        Some(json!({"stage": "todo"})),
        &[("x-ui-token", "from-before")],
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("stale_ui_token")), "{v}");

    // The web client: let in.
    let (s, v) = common::call_as_ui(&app, "PATCH", "/api/tickets/ext", Some(json!({"stage": "todo"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["stage"].as_str(), v["admitted"].as_bool()), (Some("todo"), Some(true)), "{v}");
    // Once let in, the gate is open to anyone.
    let (s, v) = call(&app, "PATCH", "/api/tickets/ext", Some(json!({"labels": ["agentic"]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}
