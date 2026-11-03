//! PDT-368: the Notification rules workflow. Its defaults are today's levels, Settings → Notifications changes its
//! inputs (and only names a customized one), levels changed before carry over, an Inbox item left unanswered is
//! escalated, and quiet hours hold toasts back until one summary. Posting to a chat service is in pdt-int-discord's
//! tests.

use crate::common;

use axum::http::StatusCode;
use common::call;
use common::workflows::{Env, runs, setup};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use serde_json::{Value, json};
use std::time::Duration;

async fn view(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/notification-rules", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn put(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "PUT", "/api/notification-rules", Some(body)).await
}

fn kind<'a>(v: &'a Value, k: &str) -> &'a Value {
    v["kinds"].as_array().unwrap().iter().find(|x| x["kind"] == k).unwrap_or_else(|| panic!("{k} in {v}"))
}

fn gate(id: &str) -> InboxItem {
    InboxItem::new(
        id,
        InboxKind::Gate,
        InboxGroup::Gate,
        format!("Go on with {id}?"),
        json!({}),
        jiff::Timestamp::now().as_millisecond(),
    )
    .choices(vec![InboxChoice::new("yes", "Yes", true)])
}

/// The live rows, by dedupe key.
async fn rows(env: &Env) -> Vec<Value> {
    let (_, n) = call(&env.app, "GET", "/api/notifications?limit=100", None).await;
    n["items"].as_array().unwrap().iter().filter(|r| r["clearedAt"].is_null()).cloned().collect()
}

async fn row(env: &Env, key: &str) -> Option<Value> {
    rows(env).await.into_iter().find(|r| r["dedupeKey"] == key)
}

#[tokio::test]
async fn by_default_nothing_changes_and_settings_set_the_workflows_inputs() {
    let env = setup(vec![]).await;
    let v = view(&env).await;
    assert_eq!((v["enabled"].as_bool(), v["customized"].as_bool()), (Some(true), Some(false)), "{v}");
    assert_eq!(v["name"], "Notification rules");
    assert!(v["kinds"].as_array().unwrap().iter().all(|k| k["show"] == k["defaultShow"]), "today's levels: {v}");
    assert_eq!(v["values"], v["defaults"]);
    assert_eq!((v["values"]["escalate"].as_bool(), v["values"]["quietHours"].as_bool()), (Some(false), Some(false)));
    assert_eq!(v["values"]["escalateAfter"], 2 * 60 * 60 * 1000);
    assert_eq!(v["channels"], json!([]), "no plugin posts notifications here");
    let wf = v["workflowId"].as_str().unwrap().to_string();

    // A gate notifies as a toast, as it always has, and no rule runs for it.
    env.state.store.open_inbox_item(&gate("g1")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    assert_eq!(row(&env, "inbox:g1").await.unwrap()["show"], "toast");
    assert!(runs(&env, &wf).await.is_empty(), "the rules that are off start no runs");

    // A level set in Settings is the workflow's input, saved as a version that still follows PDT's.
    let (s, v) = put(&env, json!({"levels": {"inbox.gate": "desktop"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(kind(&v, "inbox.gate")["show"], "desktop");
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{wf}/versions"), None).await;
    assert_eq!(versions[0]["note"], "inputs changed in Settings", "{versions}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let levels = w["doc"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "levels").unwrap().clone();
    assert_eq!(levels["config"]["levels"], json!({"inbox.gate": "desktop"}), "{w}");
    env.state.store.open_inbox_item(&gate("g2")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    assert_eq!(row(&env, "inbox:g2").await.unwrap()["show"], "desktop");
    // The old route sets the same input.
    let (s, kinds) = call(&env.app, "PATCH", "/api/notifications/settings", Some(json!({"pr.merged": "toast"}))).await;
    assert_eq!(s, StatusCode::OK, "{kinds}");
    assert_eq!(kind(&view(&env).await, "pr.merged")["show"], "toast");

    // Levels a kind may not take, and inputs there aren't, are refused.
    let (s, e) = put(&env, json!({"levels": {"inbox.gate": "off"}})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")), "{e}");
    assert_eq!(put(&env, json!({"values": {"nope": 1}})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(put(&env, json!({"values": {"quietFrom": "late"}})).await.0, StatusCode::BAD_REQUEST);

    // Turned off, every kind is at PDT's default.
    let (s, _) = call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    env.state.store.open_inbox_item(&gate("g3")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    assert_eq!(row(&env, "inbox:g3").await.unwrap()["show"], "toast");
}

#[tokio::test]
async fn a_customized_workflow_is_named_and_reset_to_default() {
    let env = setup(vec![]).await;
    put(&env, json!({"levels": {"pr.merged": "desktop"}, "values": {"escalate": true}})).await;
    let v = view(&env).await;
    let wf = v["workflowId"].as_str().unwrap().to_string();
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf}"), None).await;
    let mut doc = w["doc"].clone();
    let escalate = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "escalate").unwrap();
    escalate["config"]["title"] = json!("Hey: {{trigger.title}}");
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{wf}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    let v = view(&env).await;
    assert_eq!(v["customized"], true, "{v}");
    assert_eq!(kind(&v, "pr.merged")["show"], "desktop", "the customized workflow's levels");
    let (s, e) = put(&env, json!({"values": {"escalate": false}})).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("customized")), "{e}");
    let (s, v) = call(&env.app, "POST", "/api/notification-rules/reset", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["customized"].as_bool(), v["values"]["escalate"].as_bool()), (Some(false), Some(false)), "{v}");
    assert_eq!(kind(&v, "pr.merged")["show"], "inbox", "every level back at its default");
}

#[tokio::test]
async fn levels_changed_before_carry_over() {
    let env = setup(vec![]).await;
    let old = json!({"pr.merged": "desktop", "session.needs_you": "inbox"});
    env.state.store.put_host_value("notifications.show", &old).await.unwrap();
    let v = view(&env).await;
    assert_eq!(
        (kind(&v, "pr.merged")["show"].as_str(), kind(&v, "session.needs_you")["show"].as_str()),
        (Some("desktop"), Some("inbox"))
    );
    assert_eq!(v["customized"], false);
}

#[tokio::test]
async fn an_inbox_item_left_unanswered_is_escalated() {
    let env = setup(vec![]).await;
    let (s, v) = put(&env, json!({"values": {"escalate": true, "escalateAfter": 300}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let wf = v["workflowId"].as_str().unwrap().to_string();
    env.state.store.open_inbox_item(&gate("late")).await.unwrap();
    env.state.store.open_inbox_item(&gate("handled")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
    let waiting = runs(&env, &wf).await;
    assert_eq!(waiting.len(), 2, "one per item: {waiting:?}");
    assert!(waiting.iter().all(|r| r["status"] == "waiting"), "{waiting:?}");
    // One is dealt with before the time is up (not by an answer: closed by its run).
    let at = jiff::Timestamp::now().as_millisecond();
    env.state.store.end_inbox_item("handled", InboxStatus::Closed, &json!({}), at).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    for _ in 0..4 {
        pdt_api::run_workflows(&env.state).await;
    }
    let done = runs(&env, &wf).await;
    assert!(done.iter().all(|r| r["status"] == "quiet"), "ended, saying nothing of their own: {done:?}");
    let escalated: Vec<Value> = rows(&env).await.into_iter().filter(|r| r["kind"] == "workflow.notify").collect();
    assert_eq!(escalated.len(), 1, "only the one still open: {escalated:?}");
    let e = &escalated[0];
    assert_eq!(
        (&e["title"], &e["show"], &e["severity"], &e["target"]),
        (
            &json!("Still waiting: Go on with late?"),
            &json!("desktop"),
            &json!("warning"),
            &json!({"kind": "inbox", "id": "late"})
        )
    );
}

/// The minute past local midnight `d` minutes from now, as `HH:MM`.
fn clock(d: i32) -> String {
    let t = jiff::Zoned::now();
    let m = (i32::from(t.hour()) * 60 + i32::from(t.minute()) + d).rem_euclid(24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

#[tokio::test]
async fn quiet_hours_hold_toasts_back_until_one_summary() {
    let env = setup(vec![]).await;
    let (s, v) = put(&env, json!({"values": {"quietHours": true, "quietFrom": clock(-1), "quietTo": clock(3)}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    env.state.store.open_inbox_item(&gate("q1")).await.unwrap();
    env.state.store.open_inbox_item(&gate("q2")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    for id in ["q1", "q2"] {
        assert_eq!(row(&env, &format!("inbox:{id}")).await.unwrap()["show"], "inbox", "held in the Inbox");
    }
    assert!(rows(&env).await.iter().all(|r| r["kind"] != "notifications.quiet_summary"), "not while quiet");

    // Quiet hours end (here: turned off): one summary at the level held back.
    put(&env, json!({"values": {"quietHours": false}})).await;
    pdt_api::sync_notifications(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    let summaries: Vec<Value> =
        rows(&env).await.into_iter().filter(|r| r["kind"] == "notifications.quiet_summary").collect();
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    assert_eq!(
        (&summaries[0]["title"], &summaries[0]["body"], &summaries[0]["show"]),
        (&json!("2 notifications during quiet hours"), &json!("Gate × 2"), &json!("toast"))
    );
    env.state.store.open_inbox_item(&gate("q3")).await.unwrap();
    pdt_api::sync_notifications(&env.state).await;
    assert_eq!(row(&env, "inbox:q3").await.unwrap()["show"], "toast", "no longer quiet");
}
