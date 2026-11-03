//! PDT-490: an asking tool's call from an agent that can't ask in its own prompt (Codex, Claude in Bypass) asks in the
//! conversation itself, and the call waits for the answer: Allow runs it in the same turn, Deny tells the agent no.
//! The same request is the Inbox's approval item, so answering either answers both.

use crate::common;

use axum::http::StatusCode;
use common::pm::*;
use common::{call, call_as_ui};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Makes the PM run on `agent` in `mode`.
async fn pm_on(env: &Env, agent: &str, mode: &str) {
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();
    let setup = json!({"note": "", "agent": agent, "model": null, "instructions": pm["instructions"],
                       "permissionMode": mode, "tools": pm["tools"]});
    let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", pm["id"].as_str().unwrap()), Some(setup)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// The conversation's open request: its event.
async fn open_card(env: &Env, session: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let all = events(env, session).await;
        if let Some(e) = all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open") {
            return e["event"].clone();
        }
        assert!(Instant::now() < deadline, "no open request");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The request's event as it is now.
async fn card(env: &Env, session: &str, id: &str) -> Value {
    let all = events(env, session).await;
    all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["id"] == id).unwrap()["event"].clone()
}

async fn inbox(env: &Env) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
    v["items"].as_array().unwrap().clone()
}

/// Answers a request as the user's click in PDT's web.
async fn answer(env: &Env, session: &str, request: &str, a: Value) {
    let url = format!("/api/sessions/{session}/requests/{request}");
    let (s, v) = call_as_ui(&env.app, "POST", &url, Some(json!({"answer": a}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
}

/// What the agent said after its first `n` things that starts with `prefix`.
async fn said_after(env: &Env, session: &str, n: usize, prefix: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(m) = agent_says(env, session).await[n..].iter().find(|m| m.starts_with(prefix)) {
            return m.clone();
        }
        assert!(Instant::now() < deadline, "the agent never said {prefix}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The PM on `agent` in `mode` creates a ticket after the user's Allow in the conversation, hears a Deny, and its
/// card is the Inbox's approval item, answered there too; "Always allow" stops asking for the session.
async fn the_pm_asks_in_its_conversation(agent: &str, mode: &str) {
    let env = setup().await;
    pm_on(&env, agent, mode).await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let create = |title: &str| format!(r#"mcp: create_ticket {{"project": "PX", "title": "{title}"}}"#);

    // Allow in the conversation: the call runs in the same turn.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("Inline one")).await;
    let req = open_card(&env, &id).await;
    assert_eq!(req["tool"], "mcp__pdt__create_ticket", "{req}");
    assert_eq!((req["requestKind"].as_str(), req["canAlways"].as_bool()), (Some("approval"), Some(true)), "{req}");
    assert_eq!(req["input"]["title"], "Inline one");
    let rid = req["id"].as_str().unwrap().to_string();
    // A yes that isn't the user's click (an agent's curl, without the UI token) runs nothing.
    let url = format!("/api/sessions/{id}/requests/{rid}");
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(!ticket_titles(&env).await.contains(&"Inline one".to_string()), "nothing before the yes");
    let items = inbox(&env).await;
    assert!(!items.iter().any(|i| i["kind"] == "confirm"), "no Inbox confirmation of its own: {items:#?}");
    let item = items.iter().find(|i| i["kind"] == "approval").expect("the card is in the Inbox too");
    assert_eq!(
        (item["sessionId"].as_str(), item["body"]["requestId"].as_str()),
        (Some(id.as_str()), Some(rid.as_str()))
    );
    answer(&env, &id, &rid, json!({"type": "approve"})).await;
    let said = said_after(&env, &id, n, "mcp create_ticket").await;
    assert!(said.contains("\"created\": true"), "{said}");
    assert!(!said.contains("End your turn"), "{said}");
    assert!(ticket_titles(&env).await.contains(&"Inline one".to_string()));
    assert_eq!(card(&env, &id, &rid).await["state"], "answered");
    assert!(!inbox(&env).await.iter().any(|i| i["kind"] == "approval"), "answered in both");

    // Answered from the Inbox: its item answers the same request.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("From the Inbox")).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    let item = inbox(&env).await.into_iter().find(|i| i["kind"] == "approval").expect("in the Inbox");
    let from_inbox = item["body"]["requestId"].as_str().unwrap().to_string();
    assert_eq!(from_inbox, rid);
    answer(&env, item["sessionId"].as_str().unwrap(), &from_inbox, json!({"type": "approve"})).await;
    let said = said_after(&env, &id, n, "mcp create_ticket").await;
    assert!(said.contains("\"created\": true"), "{said}");
    assert_eq!(card(&env, &id, &rid).await["state"], "answered", "the card is answered too");
    let url = format!("/api/sessions/{id}/requests/{rid}");
    let (s, v) = call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a second answer runs nothing: {v}");

    // Deny: nothing is made, and the agent hears why.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("Not this")).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    answer(&env, &id, &rid, json!({"type": "deny", "message": "Not now"})).await;
    let said = said_after(&env, &id, n, "mcp create_ticket").await;
    assert!(
        said.starts_with("mcp create_ticket failed:") && said.contains("denied") && said.contains("Not now"),
        "{said}"
    );
    assert!(!ticket_titles(&env).await.contains(&"Not this".to_string()));

    // Always allow in this session: the next call runs without asking.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &create("Always one")).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    answer(&env, &id, &rid, json!({"type": "approve", "always": true})).await;
    said_after(&env, &id, n, "mcp create_ticket").await;
    let said = ask(&env, &id, &create("Always two")).await;
    assert!(said.contains("\"created\": true"), "asked nothing: {said}");
    let titles = ticket_titles(&env).await;
    assert!(titles.contains(&"Always one".to_string()) && titles.contains(&"Always two".to_string()), "{titles:?}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_claude_pm_in_bypass_asks_in_its_conversation() {
    the_pm_asks_in_its_conversation("claude", "bypass").await;
}

#[tokio::test]
async fn a_codex_pm_asks_in_its_conversation() {
    the_pm_asks_in_its_conversation("codex", "ask").await;
}

/// Where the project lets the PM act, nothing is asked.
#[tokio::test]
async fn a_pm_that_may_act_asks_nothing() {
    let env = setup().await;
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    pm_on(&env, "codex", "ask").await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let said = ask(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Straight in"}"#).await;
    assert!(said.contains("\"created\": true"), "{said}");
    assert!(!events(&env, &id).await.iter().any(|e| e["event"]["kind"] == "request"), "no card");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-492: a terminal session has no conversation to show a card in, so its call waits on its Inbox item instead:
/// the agent never ends its turn to wait. Deny and Allow both come back to the same call.
#[tokio::test]
async fn a_terminal_call_waits_on_its_inbox_item() {
    let env = setup().await;
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "codex", "mode": "terminal",
                      "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {}", pdt_api::mcp_token(&env.state, &id).unwrap());
    let run = |n: u32| {
        let (app, bearer) = (env.app.clone(), bearer.clone());
        let rpc = json!({"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "run_with_secrets",
                         "arguments": {"command": "echo hi", "env": {"KEY": "secret:pdt-492-none"}}}});
        tokio::spawn(async move {
            let (s, v) = common::call_with(&app, "POST", "/api/mcp", Some(rpc), &[("authorization", &bearer)]).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            v["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
        })
    };
    let waiting_item = || async {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let found = inbox(&env).await.into_iter().find(|i| i["kind"] == "confirm" && i["sessionId"] == id.as_str());
            if let Some(i) = found {
                return i["id"].as_str().unwrap().to_string();
            }
            assert!(Instant::now() < deadline, "no Inbox item");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };

    // Deny: the waiting call hears it.
    let call_ = run(1);
    let item = waiting_item().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!call_.is_finished(), "the call waits for the answer");
    let (s, v) =
        call_as_ui(&env.app, "POST", &format!("/api/inbox/{item}/answer"), Some(json!({"choice": "deny"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let said = call_.await.unwrap();
    assert!(said.contains("denied") && !said.contains("End your turn"), "{said}");

    // Allow: the same call goes on and runs (here it finds no such secret).
    let call_ = run(2);
    let item = waiting_item().await;
    let (s, v) =
        call_as_ui(&env.app, "POST", &format!("/api/inbox/{item}/answer"), Some(json!({"choice": "allow"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["waited"], true, "{v}");
    let said = call_.await.unwrap();
    assert!(said.contains("pdt-492-none") && !said.contains("Inbox"), "it ran: {said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    env.state.ptys.shutdown().await;
}
