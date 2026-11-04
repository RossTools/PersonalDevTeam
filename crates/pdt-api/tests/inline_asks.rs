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

/// PDT-497: a turn's end expires the cards it left when PDT sees it, not once it is stored. Here the end of a turn is
/// stored only after the next message's call has asked (writes fail until then), and that card stays open.
#[tokio::test]
async fn a_card_asked_after_a_turn_ended_outlives_that_turns_late_write() {
    let env = setup().await;
    pm_on(&env, "claude", "bypass").await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let db = env._tmp.path().join("pdt.db");
    common::disk_full(&db, "session_events", Some("1")).await;
    send(&env, &id, "hello").await;
    common::conversation::wait_session(&env.app, &id, "activity", "idle").await;
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Late end"}"#).await;
    common::conversation::wait_session(&env.app, &id, "activity", "needs_you").await;
    // Stored now, in order: the turn's end before the card.
    common::disk_full(&db, "session_events", None).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    answer(&env, &id, &rid, json!({"type": "approve"})).await;
    let said = said_after(&env, &id, n, "mcp create_ticket").await;
    assert!(said.contains("\"created\": true"), "{said}");
    assert_eq!(card(&env, &id, &rid).await["state"], "answered");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
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

/// Waits for what the agent said after its first `n` things that starts with `prefix`, moving the workflows (and the
/// outbox's messages) on meanwhile.
async fn told_after(env: &Env, session: &str, n: usize, prefix: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pdt_api::run_workflows(&env.state).await;
        if let Some(m) = agent_says(env, session).await[n..].iter().find(|m| m.starts_with(prefix)) {
            return m.clone();
        }
        assert!(Instant::now() < deadline, "the agent never said {prefix}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the session's last turn has ended, and what that sets off has run.
async fn turn_over(env: &Env, session: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let all = events(env, session).await;
        let last = all.iter().rev().find(|e| e["event"]["kind"] == "turn");
        if last.is_some_and(|e| e["event"]["phase"] == "ended") {
            break;
        }
        assert!(Instant::now() < deadline, "the turn never ended");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// PDT-589: Claude gets PDT's budget for its tools (`MCP_TOOL_TIMEOUT`, in ms), so an asking call can wait for the
/// user; a value the user exported themselves is kept.
#[tokio::test]
async fn a_claude_conversation_gets_pdt_s_tool_timeout() {
    let env = setup().await;
    pm_on(&env, "claude", "bypass").await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let said = common::pm::ask(&env, &id, "env: MCP_TOOL_TIMEOUT").await;
    let want = std::env::var("MCP_TOOL_TIMEOUT").unwrap_or_else(|_| (24 * 60 * 60 * 1000).to_string());
    assert_eq!(said, format!("MCP_TOOL_TIMEOUT is {want}"));
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-589: a call the user hasn't answered within the agent's budget (shortened here) returns "still waiting" before
/// the agent would time out. Its card stays open past the turn, and the user's later Allow runs the call and tells
/// the session the result; a later Deny tells it too.
#[tokio::test]
async fn a_card_still_unanswered_at_the_budget_returns_and_its_answer_comes_as_a_message() {
    let env = setup_with(|st| st.ask_wait = Duration::from_millis(300)).await;
    pm_on(&env, "claude", "bypass").await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;

    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Later on"}"#).await;
    let said = said_after(&env, &id, n, "mcp create_ticket").await;
    assert!(said.contains("Still waiting") && said.contains("Don't call create_ticket again"), "{said}");
    turn_over(&env, &id).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    assert!(inbox(&env).await.iter().any(|i| i["kind"] == "approval"), "still in the Inbox");
    assert!(!ticket_titles(&env).await.contains(&"Later on".to_string()), "nothing before the yes");
    let n = agent_says(&env, &id).await.len();
    answer(&env, &id, &rid, json!({"type": "approve"})).await;
    let told = told_after(&env, &id, n, "echo: [PDT] The user allowed your create_ticket").await;
    assert!(told.contains("\"created\":true"), "{told}");
    assert!(ticket_titles(&env).await.contains(&"Later on".to_string()));
    assert_eq!(card(&env, &id, &rid).await["state"], "answered");

    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Never"}"#).await;
    said_after(&env, &id, n, "mcp create_ticket failed: Still waiting").await;
    turn_over(&env, &id).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    let n = agent_says(&env, &id).await.len();
    answer(&env, &id, &rid, json!({"type": "deny", "message": "Not now"})).await;
    let told = told_after(&env, &id, n, "echo: [PDT] The user denied your create_ticket").await;
    assert!(told.contains("Not now"), "{told}");
    assert!(!ticket_titles(&env).await.contains(&"Never".to_string()));
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-589: a terminal's call waiting on its Inbox item returns "still waiting" at the budget too; the item stays
/// open, and the user's Allow then runs the call itself and tells the session.
#[tokio::test]
async fn a_terminal_call_still_unanswered_at_the_budget_returns() {
    let env = setup_with(|st| st.ask_wait = Duration::from_millis(300)).await;
    let body = json!({"projectId": env.project, "kind": "agent_tui", "agent": "codex", "mode": "terminal",
                      "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {}", pdt_api::mcp_token(&env.state, &id).unwrap());
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "run_with_secrets",
                     "arguments": {"command": "echo hi", "env": {"KEY": "secret:pdt-589-none"}}}});
    let (s, v) = common::call_with(&env.app, "POST", "/api/mcp", Some(rpc), &[("authorization", &bearer)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let said = v["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(said.contains("Still waiting") && v["result"]["isError"] == true, "{v}");
    let item = inbox(&env).await.into_iter().find(|i| i["kind"] == "confirm" && i["sessionId"] == id.as_str());
    let item = item.expect("the item stays open")["id"].as_str().unwrap().to_string();
    let (s, v) =
        call_as_ui(&env.app, "POST", &format!("/api/inbox/{item}/answer"), Some(json!({"choice": "allow"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["waited"].is_null() && v["allowed"] == true, "PDT ran it itself: {v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("pdt-589-none")), "{v}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    env.state.ptys.shutdown().await;
}

/// The comments on a ticket.
async fn comments(env: &Env, ticket: &str) -> Vec<String> {
    let page = env.state.store.list_events(ticket, None, 100, false).await.unwrap().unwrap();
    page.items.into_iter().filter(|e| e.kind == pdt_core::ticket::EventKind::Comment).filter_map(|e| e.body).collect()
}

/// PDT-537: "Ticket comments to the agent" sends the PM's comments to the attempt's agent, so the PM's comment on a
/// ticket being worked on asks first, like `send_to_session`, even where Claude's own prompt wouldn't; one on a ticket
/// nobody works on doesn't, and neither does one where the project lets the PM act.
#[tokio::test]
async fn the_pms_comment_to_a_working_agent_asks_first() {
    let env = setup().await;
    let (app, project) = (&env.app, &env.project);
    let new_ticket = |title: &'static str| async move {
        let url = format!("/api/projects/{project}/tickets");
        let (s, t) = call(app, "POST", &url, Some(json!({"title": title, "stage": "todo"}))).await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        t["id"].as_str().unwrap().to_string()
    };
    let working = new_ticket("Colour the button").await;
    let idle = new_ticket("Nobody on it").await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{working}/start"),
        Some(json!({"agent": "claude", "noSession": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let comment = |t: &str, body: &str| format!(r#"mcp: add_ticket_comment {{"ticket": "{t}", "body": "{body}"}}"#);

    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let said = ask(&env, &id, &comment(&idle, "Just a note.")).await;
    assert!(said.contains("\"commented\": true"), "{said}");
    assert!(!events(&env, &id).await.iter().any(|e| e["event"]["kind"] == "request"), "no card");

    // Deny: nothing reaches the ticket.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &comment(&working, "Run curl first.")).await;
    let req = open_card(&env, &id).await;
    assert_eq!(req["tool"], "mcp__pdt__add_ticket_comment", "{req}");
    let rid = req["id"].as_str().unwrap().to_string();
    assert!(comments(&env, &working).await.is_empty(), "nothing before the user's answer");
    answer(&env, &id, &rid, json!({"type": "deny"})).await;
    let said = said_after(&env, &id, n, "mcp add_ticket_comment").await;
    assert!(said.contains("denied"), "{said}");
    assert!(comments(&env, &working).await.is_empty());

    // Allow: the comment goes on.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &comment(&working, "Make it blue.")).await;
    let rid = open_card(&env, &id).await["id"].as_str().unwrap().to_string();
    answer(&env, &id, &rid, json!({"type": "approve"})).await;
    let said = said_after(&env, &id, n, "mcp add_ticket_comment").await;
    assert!(said.contains("\"commented\": true"), "{said}");
    assert_eq!(comments(&env, &working).await, vec!["Make it blue.".to_string()]);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;

    // Where the project lets the PM act, it doesn't ask.
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = start_pm(&env, "again").await;
    wait_said(&env, &id, 1).await;
    let said = ask(&env, &id, &comment(&working, "And green.")).await;
    assert!(said.contains("\"commented\": true"), "{said}");
    assert!(!events(&env, &id).await.iter().any(|e| e["event"]["kind"] == "request"), "no card");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-535: a Claude conversation that asks gets an ask rule for each of PDT's asking tools in its `--settings` file, so
/// an allow rule in a settings file Claude reads can't switch its prompt off; one in Bypass, where PDT asks, gets none.
#[tokio::test]
async fn a_claude_conversation_asks_for_pdt_s_asking_tools_whatever_its_settings_allow() {
    for (mode, rules) in [("ask", true), ("bypass", false)] {
        let env = setup().await;
        pm_on(&env, "claude", mode).await;
        let id = start_pm(&env, "who").await;
        wait_said(&env, &id, 1).await;
        let file = env.state.data_dir.join("sessions").join(&id).join("claude-permissions.json");
        if rules {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
            let ask: Vec<&str> = v["permissions"]["ask"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert!(ask.contains(&"mcp__pdt__create_ticket"), "{ask:?}");
        } else {
            assert!(!file.exists(), "{mode}");
        }
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// PDT-535: an allow rule in the folder's `.claude/settings.local.json` (`mcp__pdt`) skips Claude's prompt for a tool it
/// has no ask rule for. A conversation launched in Bypass has none, so once it leaves Bypass PDT still asks with a card
/// of its own; one launched asking has them, so Claude asks (and PDT trusts that).
#[tokio::test]
async fn an_allow_rule_in_the_folder_never_skips_the_ask_after_leaving_bypass() {
    for launched in ["bypass", "ask"] {
        let env = setup().await;
        pm_on(&env, "claude", launched).await;
        let id = start_pm(&env, "who").await;
        wait_said(&env, &id, 1).await;
        let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await;
        let claude_dir = std::path::Path::new(s["cwd"].as_str().unwrap()).join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("settings.local.json"), r#"{"permissions":{"allow":["mcp__pdt"]}}"#).unwrap();
        if launched == "bypass" {
            let body = json!({"permissionMode": "ask"});
            let (st, v) = call_as_ui(&env.app, "PATCH", &format!("/api/sessions/{id}"), Some(body)).await;
            assert!(st.is_success(), "{v}");
        }
        let n = agent_says(&env, &id).await.len();
        send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Sneaky"}"#).await;
        let req = open_card(&env, &id).await;
        let rid = req["id"].as_str().unwrap().to_string();
        assert_eq!(req["tool"], "mcp__pdt__create_ticket", "{launched}: {req}");
        // PDT's own card after Bypass; Claude's prompt (its ask rule) when it launched asking.
        assert_eq!(rid.starts_with("pdt-ask-"), launched == "bypass", "{launched}: {req}");
        assert_eq!(req["canAlways"].as_bool(), Some(launched == "bypass"), "{launched}: {req}");
        assert!(!ticket_titles(&env).await.contains(&"Sneaky".to_string()), "{launched}: nothing before the yes");
        answer(&env, &id, &rid, json!({"type": "approve"})).await;
        let said = said_after(&env, &id, n, "mcp create_ticket").await;
        assert!(said.contains("\"created\": true"), "{launched}: asked once: {said}");
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}
