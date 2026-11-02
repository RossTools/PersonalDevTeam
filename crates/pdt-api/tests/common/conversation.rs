//! A conversation session's events and state, read through the API, and waits on them.

use super::call;
use super::wait::wait_until;
use serde_json::Value;

/// The session's events, oldest first.
pub async fn events(app: &axum::Router, id: &str) -> Vec<Value> {
    let (s, v) = call(app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
    assert_eq!(s, 200, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

/// The statuses of the turns that ended.
pub fn ended(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        .map(|e| e["event"]["status"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// What the agent said, in order.
pub fn said(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Waits until `f` holds for the session's events, and returns them.
pub async fn wait_events(app: &axum::Router, id: &str, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    wait_until(what, async || {
        let all = events(app, id).await;
        if f(&all) { Ok(all) } else { Err(all) }
    })
    .await
}

/// Waits until `n` turns have ended, and returns the events.
pub async fn wait_turns(app: &axum::Router, id: &str, n: usize) -> Vec<Value> {
    wait_events(app, id, &format!("{n} turns to end"), |all| ended(all).len() >= n).await
}

/// The session, as GET /api/sessions/{id} gives it.
pub async fn session(app: &axum::Router, id: &str) -> Value {
    call(app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

/// Waits until the session's `field` (`activity`, `status`) is `want`, and returns the session.
pub async fn wait_session(app: &axum::Router, id: &str, field: &str, want: impl Into<Value>) -> Value {
    let want = want.into();
    wait_until(&format!("session {id}'s {field} to be {want}"), async || {
        let s = session(app, id).await;
        if s[field] == want { Ok(s) } else { Err(s) }
    })
    .await
}
