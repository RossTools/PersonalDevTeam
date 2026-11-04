//! Notifications (PDT-108): rows made from sessions' state and the Inbox, cleared when their cause goes away, the
//! per-kind settings and the stream.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::call;
use http_body_util::BodyExt;
use pdt_core::activity::Activity;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tower::ServiceExt;

async fn setup(tmp: &Path) -> (axum::Router, pdt_api::AppState) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    crate::common::init_repo(&repo);
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = common::app_state(store.clone(), tmp);
    let app = pdt_api::router(state.clone(), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    store
        .insert_session(&Session {
            id: "c1".into(),
            project_id: p["id"].as_str().unwrap().into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: "Fix the build".into(),
            cwd: repo.to_string_lossy().into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: Some(Activity::Idle),
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: None,
            ticket: None,
            mode: pdt_core::conversation::SessionMode::Terminal,
            model: None,
            permission_mode: None,
            persona: None,
            background: vec![],
        })
        .await
        .unwrap();
    (app, state)
}

async fn needs_you(app: &axum::Router) {
    let r = json!({"source": "claude", "event": "Notification", "notificationType": "permission_prompt"});
    assert_eq!(call(app, "POST", "/api/sessions/c1/activity", Some(r)).await.0, StatusCode::NO_CONTENT);
}

async fn list(app: &axum::Router, section: &str) -> Vec<Value> {
    let (_, v) = call(app, "GET", &format!("/api/notifications?section={section}"), None).await;
    v["items"].as_array().unwrap().clone()
}

#[tokio::test]
async fn a_waiting_session_notifies_until_it_is_viewed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    needs_you(&app).await;
    pdt_api::sync_notifications(&st).await;
    let rows = list(&app, "needs_you").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (&rows[0]["kind"], &rows[0]["show"], &rows[0]["title"], &rows[0]["dedupeKey"]),
        (&json!("session.needs_you"), &json!("desktop"), &json!("Fix the build"), &json!("session:c1:needs_you"))
    );
    assert_eq!(rows[0]["target"]["kind"], "session");
    let (_, inbox) = call(&app, "GET", "/api/inbox", None).await;
    assert_eq!(
        (&inbox["total"], &inbox["items"][0]["id"], &inbox["items"][0]["group"]),
        (&json!(1), &json!("session:c1"), &json!("session"))
    );
    pdt_api::sync_notifications(&st).await;
    assert_eq!(list(&app, "needs_you").await.len(), 1, "one row per cause");

    tokio::time::sleep(Duration::from_millis(5)).await;
    call(&app, "POST", "/api/sessions/c1/view", None).await;
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "needs_you").await.is_empty(), "viewed: cleared");
    assert_eq!(call(&app, "GET", "/api/inbox", None).await.1["total"], 0);

    // Turned down to the dot: no Inbox entry.
    let (s, _) = call(&app, "PATCH", "/api/notifications/settings", Some(json!({"session.needs_you": "dot"}))).await;
    assert_eq!(s, StatusCode::OK);
    call(&app, "POST", "/api/sessions/c1/activity", Some(json!({"source": "claude", "event": "UserPromptSubmit"})))
        .await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    needs_you(&app).await;
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "needs_you").await.is_empty());
    // Back to Inbox: the Inbox entry, still no desktop notification.
    call(&app, "PATCH", "/api/notifications/settings", Some(json!({"session.needs_you": "inbox"}))).await;
    pdt_api::sync_notifications(&st).await;
    let rows = list(&app, "needs_you").await;
    assert_eq!((rows.len(), &rows[0]["show"]), (1, &json!("inbox")));
}

#[tokio::test]
async fn a_turn_left_unseen_is_an_update_only_above_the_dot() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    let long_ago = jiff::Timestamp::now().as_millisecond() - 60_000;
    st.store.update_activity("c1", Some(Activity::Idle), Some(long_ago)).await.unwrap();
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "updates").await.is_empty(), "the dot by default");
    call(&app, "PATCH", "/api/notifications/settings", Some(json!({"session.turn_ended": "inbox"}))).await;
    pdt_api::sync_notifications(&st).await;
    let rows = list(&app, "updates").await;
    assert_eq!((rows.len(), &rows[0]["kind"]), (1, &json!("session.turn_ended")));
    let (_, v) = call(&app, "GET", "/api/notifications?section=updates", None).await;
    assert_eq!(v["unreadUpdates"], 1);
    let id = rows[0]["id"].as_str().unwrap();
    assert_eq!(call(&app, "POST", &format!("/api/notifications/{id}/read"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "GET", "/api/notifications", None).await.1["unreadUpdates"], 0);
    assert_eq!(call(&app, "POST", "/api/notifications/nope/read", None).await.0, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/api/notifications/read-all", Some(json!({"section": "updates"}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn cleared_updates_stay_gone_and_their_cause_does_not_raise_them_again() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    let long_ago = jiff::Timestamp::now().as_millisecond() - 60_000;
    st.store.update_activity("c1", Some(Activity::Idle), Some(long_ago)).await.unwrap();
    call(&app, "PATCH", "/api/notifications/settings", Some(json!({"session.turn_ended": "inbox"}))).await;
    pdt_api::sync_notifications(&st).await;
    let rows = list(&app, "updates").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let id = rows[0]["id"].as_str().unwrap();

    assert_eq!(call(&app, "POST", &format!("/api/notifications/{id}/clear"), None).await.0, StatusCode::NO_CONTENT);
    assert!(list(&app, "updates").await.is_empty());
    assert_eq!(call(&app, "GET", "/api/notifications", None).await.1["unreadUpdates"], 0);
    // The turn is still unseen: the next pass doesn't raise it again.
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "updates").await.is_empty(), "the same cause stays cleared");
    assert_eq!(call(&app, "POST", &format!("/api/notifications/{id}/clear"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "POST", "/api/notifications/nope/clear", None).await.0, StatusCode::NOT_FOUND);
    let nid = needs_you_row(&st).await;
    let (s, v) = call(&app, "POST", &format!("/api/notifications/{nid}/clear"), None).await;
    assert_eq!((s, &v["code"]), (StatusCode::CONFLICT, &json!("needs_you")), "Needs you keeps its own rules");

    // News of another cause shows; Clear all takes it.
    let merged = pdt_core::notification::NewNotification::new(
        pdt_core::notification::NotificationKind::PrMerged,
        "test:merged",
        "PDT-1 merged",
        pdt_core::notification::NotificationTarget::None,
    );
    st.store.put_notification(&merged, pdt_core::notification::Show::Inbox, 2).await.unwrap();
    assert_eq!(list(&app, "updates").await.len(), 1);
    assert_eq!(call(&app, "POST", "/api/notifications/clear-all", None).await.0, StatusCode::NO_CONTENT);
    assert!(list(&app, "updates").await.is_empty());
    let needs = st.store.get_notification(&nid).await.unwrap().unwrap();
    assert_eq!(needs.dismissed_at, None, "Clear all leaves Needs you");

    // After a restart (the store opened again) they are still gone.
    drop((app, st));
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let st = common::app_state(store, tmp.path());
    let app = pdt_api::router(st.clone(), None);
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "updates").await.is_empty());
}

/// A Needs-you row, made directly (the next sync clears it: nothing waits).
async fn needs_you_row(st: &pdt_api::AppState) -> String {
    let n = pdt_core::notification::NewNotification::new(
        pdt_core::notification::NotificationKind::SessionNeedsYou,
        "test:needs_you",
        "Waits",
        pdt_core::notification::NotificationTarget::None,
    );
    st.store.put_notification(&n, pdt_core::notification::Show::Inbox, 1).await.unwrap().0.id
}

#[tokio::test]
async fn settings_list_every_kind_and_keep_inbox_kinds_findable() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (_, kinds) = call(&app, "GET", "/api/notifications/settings", None).await;
    let kinds = kinds.as_array().unwrap();
    assert_eq!(kinds.len(), 22);
    let gate = kinds.iter().find(|k| k["kind"] == "inbox.gate").unwrap();
    assert_eq!((&gate["show"], &gate["allowed"]), (&json!("toast"), &json!(["inbox", "toast", "desktop"])));
    let (s, e) = call(&app, "PATCH", "/api/notifications/settings", Some(json!({"inbox.gate": "off"}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, _) = call(&app, "PATCH", "/api/notifications/settings", Some(json!({"nope": "off"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = call(&app, "PATCH", "/api/notifications/settings", Some(json!({"pr.merged": "desktop"}))).await;
    assert_eq!(v.as_array().unwrap().iter().find(|k| k["kind"] == "pr.merged").unwrap()["show"], "desktop");
    let (_, v) = call(&app, "PATCH", "/api/notifications/settings", Some(json!({"pr.merged": null}))).await;
    assert_eq!(v.as_array().unwrap().iter().find(|k| k["kind"] == "pr.merged").unwrap()["show"], "inbox");
}

/// Reads the stream until `want` shows up in what it sent.
async fn read_until(body: &mut Body, want: &str) -> String {
    let mut seen = String::new();
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while !seen.contains(want) {
            match body.frame().await {
                Some(Ok(f)) => {
                    if let Some(d) = f.data_ref() {
                        seen.push_str(&String::from_utf8_lossy(d));
                    }
                }
                _ => break,
            }
        }
    })
    .await;
    assert!(found.is_ok() && seen.contains(want), "never saw {want:?} in {seen:?}");
    seen
}

#[tokio::test]
async fn the_stream_sends_new_rows_and_what_a_reconnect_missed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    let req = Request::get("/api/notifications/stream").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = res.into_body();
    needs_you(&app).await;
    pdt_api::sync_notifications(&st).await;
    let got = read_until(&mut body, "session.needs_you").await;
    assert!(got.contains("event: created") && got.contains("id: 1"), "{got}");

    tokio::time::sleep(Duration::from_millis(5)).await;
    call(&app, "POST", "/api/sessions/c1/view", None).await;
    pdt_api::sync_notifications(&st).await;
    read_until(&mut body, "event: cleared").await;

    // A reconnect from before both changes gets the row as it is now.
    let req = Request::get("/api/notifications/stream").header("last-event-id", "0").body(Body::empty()).unwrap();
    let mut body = app.clone().oneshot(req).await.unwrap().into_body();
    let got = read_until(&mut body, "id: 2").await;
    assert!(got.contains("event: cleared"), "{got}");
}

#[tokio::test]
async fn an_open_inbox_item_notifies_until_it_is_answered() {
    use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    let item = InboxItem::new("g1", InboxKind::Gate, InboxGroup::Accept, "Accept the work?", json!({}), 1)
        .choices(vec![InboxChoice::new("yes", "Accept", true)]);
    st.store.open_inbox_item(&item).await.unwrap();
    pdt_api::sync_notifications(&st).await;
    let rows = list(&app, "needs_you").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (&rows[0]["kind"], &rows[0]["show"], &rows[0]["body"], &rows[0]["target"], &rows[0]["inboxItemId"]),
        (
            &json!("inbox.gate"),
            &json!("toast"),
            &json!("Ready to accept"),
            &json!({"kind": "inbox", "id": "g1"}),
            &json!("g1")
        )
    );
    st.store.end_inbox_item("g1", InboxStatus::Resolved, &json!({"choice": "yes"}), 2).await.unwrap();
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "needs_you").await.is_empty(), "answered: cleared");
}

/// `POST /api/inbox/{id}/dismiss` closes a question, a secret request, a stalled step or a notice without acting (and
/// its notification clears); anything else must be answered, and an item already closed can't be dismissed again.
#[tokio::test]
async fn dismiss_closes_the_kinds_that_can_wait_and_refuses_the_rest() {
    use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind, InboxStatus};
    let tmp = tempfile::tempdir().unwrap();
    let (app, st) = setup(tmp.path()).await;
    let items = [
        ("q1", InboxKind::Question, InboxGroup::Question),
        ("s1", InboxKind::Secret, InboxGroup::Question),
        ("st1", InboxKind::Stalled, InboxGroup::Stalled),
        ("n1", InboxKind::Notice, InboxGroup::Notice),
        ("g1", InboxKind::Gate, InboxGroup::Accept),
    ];
    for (id, kind, group) in items {
        st.store.open_inbox_item(&InboxItem::new(id, kind, group, "Over to you", json!({}), 1)).await.unwrap();
    }
    pdt_api::sync_notifications(&st).await;
    assert!(list(&app, "needs_you").await.iter().any(|r| r["inboxItemId"] == "q1"));

    for id in ["q1", "s1", "st1", "n1"] {
        let (s, v) = call(&app, "POST", &format!("/api/inbox/{id}/dismiss"), None).await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{id}: {v}");
        let i = st.store.get_inbox_item(id).await.unwrap().unwrap();
        assert_eq!((i.status, i.resolution), (InboxStatus::Closed, json!({"dismissed": true})), "{id}");
    }
    let (s, v) = call(&app, "POST", "/api/inbox/g1/dismiss", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("cannot_answer")), "{v}");
    assert_eq!(st.store.get_inbox_item("g1").await.unwrap().unwrap().status, InboxStatus::Open, "a gate is answered");
    let (s, v) = call(&app, "POST", "/api/inbox/q1/dismiss", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("cannot_answer")), "already closed: {v}");
    let (s, _) = call(&app, "POST", "/api/inbox/nope/dismiss", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    pdt_api::sync_notifications(&st).await;
    let left: Vec<Value> = list(&app, "needs_you").await.into_iter().map(|r| r["inboxItemId"].clone()).collect();
    assert!(!left.contains(&json!("q1")), "the dismissed question's notification cleared: {left:?}");
    let (_, inbox) = call(&app, "GET", "/api/inbox", None).await;
    let open: Vec<&str> = inbox["items"].as_array().unwrap().iter().filter_map(|i| i["id"].as_str()).collect();
    assert!(open.contains(&"g1") && !open.contains(&"q1") && !open.contains(&"n1"), "{open:?}");
}
