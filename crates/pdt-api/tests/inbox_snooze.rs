//! Snoozing Inbox items (PDT-641): until a ticket is done, or until a time; the item is out of the list and the count
//! meanwhile, listed under `snoozed`, and back by itself.

use crate::common;

use axum::http::StatusCode;
use common::call;
use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind};
use serde_json::{Value, json};

async fn setup(tmp: &std::path::Path) -> (axum::Router, pdt_api::AppState, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    common::init_repo(&repo);
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = common::app_state(store.clone(), tmp);
    let app = pdt_api::router(state.clone(), None);
    let (_, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "P", "key": "PX", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    let project = p["id"].as_str().unwrap().to_string();
    for id in ["g1", "g2"] {
        let item =
            InboxItem::new(id, InboxKind::Gate, InboxGroup::Gate, "Merge it?", json!({}), 1).project(project.clone());
        store.open_inbox_item(&item).await.unwrap();
    }
    (app, state, project)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The ids listed, those snoozed, and the count.
async fn inbox(app: &axum::Router) -> (Vec<String>, Vec<Value>, u64) {
    let (s, v) = call(app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ids = v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect();
    (ids, v["snoozed"].as_array().unwrap().clone(), v["total"].as_u64().unwrap())
}

#[tokio::test]
async fn an_item_snoozed_until_a_ticket_is_done_comes_back_when_it_is() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _st, project) = setup(tmp.path()).await;
    let (_, t) =
        call(&app, "POST", &format!("/api/projects/{project}/tickets"), Some(json!({"title": "Fix main"}))).await;
    let key = t["key"].as_str().unwrap();

    let (s, v) = call(&app, "POST", "/api/inbox/g1/snooze", Some(json!({"ticket": key}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (ids, snoozed, total) = inbox(&app).await;
    assert_eq!((ids, total), (vec!["g2".to_string()], 1), "out of the list and the count");
    assert_eq!(snoozed.len(), 1);
    assert_eq!((snoozed[0]["id"].as_str(), snoozed[0]["snooze"]["ticketKey"].as_str()), (Some("g1"), Some(key)));

    let (s, v) =
        call(&app, "PATCH", &format!("/api/tickets/{}", t["id"].as_str().unwrap()), Some(json!({"stage": "done"})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (ids, snoozed, total) = inbox(&app).await;
    assert_eq!((ids.len(), snoozed.len(), total), (2, 0, 2), "back once the ticket is done: {ids:?}");

    // A closed ticket can't be waited for; an unknown one neither.
    for (ticket, why) in [(key, "already closed"), ("PX-99", "no ticket")] {
        let (s, v) = call(&app, "POST", "/api/inbox/g1/snooze", Some(json!({"ticket": ticket}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        assert!(v["message"].as_str().unwrap().contains(why), "{v}");
    }
}

#[tokio::test]
async fn an_item_snoozed_for_a_while_comes_back_at_its_time_or_when_unsnoozed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st, _) = setup(tmp.path()).await;
    let week = now_ms() + 7 * 24 * 3_600_000;
    let (s, v) = call(&app, "POST", "/api/inbox/g1/snooze", Some(json!({"until": week}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    // Any item, stored or not (an approval's id).
    let (s, _) = call(&app, "POST", "/api/inbox/approval:c1:r1/snooze", Some(json!({"until": week}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (ids, snoozed, _) = inbox(&app).await;
    assert_eq!(ids, ["g2"]);
    assert_eq!(snoozed[0]["snooze"]["until"], json!(week));

    let (s, _) = call(&app, "DELETE", "/api/inbox/g1/snooze", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(inbox(&app).await.0.len(), 2, "Unsnooze brings it back");

    // Its time come (as if a minute had passed), it is back by itself.
    let soon = pdt_core::inbox::InboxSnooze {
        item_id: "g2".into(),
        until: Some(now_ms() - 1),
        ticket_id: None,
        ticket_key: None,
        created_at: 1,
    };
    st.store.snooze_inbox_item(&soon).await.unwrap();
    assert_eq!(inbox(&app).await.0.len(), 2);
    assert!(st.store.inbox_snoozes().await.unwrap().iter().all(|s| s.item_id != "g2"), "its snooze is gone");

    for bad in [json!({}), json!({"until": 5})] {
        let (s, v) = call(&app, "POST", "/api/inbox/g2/snooze", Some(bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    }
}

#[tokio::test]
async fn an_item_its_own_ticket_waits_on_cannot_be_snoozed_until_that_ticket_is_done() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st, project) = setup(tmp.path()).await;
    let ticket = |title: &'static str| {
        let app = app.clone();
        let project = project.clone();
        async move {
            let (_, t) =
                call(&app, "POST", &format!("/api/projects/{project}/tickets"), Some(json!({"title": title}))).await;
            (t["id"].as_str().unwrap().to_string(), t["key"].as_str().unwrap().to_string())
        }
    };
    let (own, own_key) = ticket("Own").await;
    let (_, other_key) = ticket("Other").await;
    let open = |id: &str, kind: InboxKind, group: InboxGroup, body: Value| {
        InboxItem::new(id, kind, group, "Which?", body, 1).project(project.clone()).ticket(own.clone())
    };
    st.store.open_inbox_item(&open("q", InboxKind::Question, InboxGroup::Question, json!({}))).await.unwrap();
    st.store.open_inbox_item(&open("gate", InboxKind::Gate, InboxGroup::Gate, json!({}))).await.unwrap();
    st.store
        .open_inbox_item(&open("aside", InboxKind::Question, InboxGroup::Question, json!({"blocking": false})))
        .await
        .unwrap();

    // Listed with the ticket waiting on them, the non-blocking question not.
    let (s, v) = call(&app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let waits =
        |id: &str| v["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap()["ticketWaits"].clone();
    assert_eq!((waits("q"), waits("gate"), waits("aside")), (json!(true), json!(true), Value::Null));

    for id in ["q", "gate"] {
        let (s, v) = call(&app, "POST", &format!("/api/inbox/{id}/snooze"), Some(json!({"ticket": own_key}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        assert!(v["message"].as_str().unwrap().contains("is waiting on this"), "{v}");
        let (s, v) = call(&app, "POST", &format!("/api/inbox/{id}/snooze"), Some(json!({"ticket": other_key}))).await;
        assert_eq!(s, StatusCode::NO_CONTENT, "another ticket is fine: {v}");
    }
    let (s, v) = call(&app, "POST", "/api/inbox/aside/snooze", Some(json!({"ticket": own_key}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "a non-blocking question can wait for its ticket: {v}");

    // Snoozed, they still say their ticket waits on them.
    let (_, snoozed, _) = inbox(&app).await;
    let waiting: Vec<_> = snoozed.iter().filter(|i| i["ticketWaits"] == true).map(|i| i["id"].clone()).collect();
    assert_eq!(waiting.len(), 2, "{snoozed:?}");
}
