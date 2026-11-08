//! PDT-81 end to end: the PM's sessions in the host project, its tools, asking before acting (and the project policy
//! that lets it act), notes, the journal (checkpoints, daily updates, closing), the briefing, with
//! `fake-agent` standing in
//! for Claude (it calls pdt-mcp tools when a message says `mcp: <tool> <json>`, asking first for tools
//! `--allowedTools` leaves out).

use crate::common;

use axum::http::StatusCode;
use common::pm::*;
use common::wait::wait_until;
use common::{call, git};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[tokio::test]
async fn the_pm_answers_from_pdt_and_creates_tickets_after_a_yes() {
    let env = setup().await;
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm, json!({"current": null, "sessions": []}));

    let id = start_pm(&env, "who").await;
    let said = wait_said(&env, &id, 1).await;
    assert!(said[0].contains("instructions=true") && said[0].contains("mode=default"), "{said:?}");
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm["current"]["id"], id.as_str());
    assert_eq!(pm["current"]["persona"]["name"], "PM");
    // The host project is PDT's own: never listed.
    let (_, projects) = call(&env.app, "GET", "/api/projects", None).await;
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_ne!(pm["current"]["projectId"], env.project.as_str());

    let text = instructions(&env, &id);
    let in_repo = format!("- P (`PX`) in `{}`: To do 1", projects[0]["repoPath"].as_str().unwrap());
    for want in ["project manager", "# Briefing", &in_repo, "`create_ticket`", "## Your role"] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }

    let said = ask(&env, &id, r#"mcp: search_tickets {"query": "login"}"#).await;
    assert!(said.contains("Fix the login") && said.contains("\"key\": \"PX-1\""), "{said}");
    let said = ask(&env, &id, "mcp: get_overview {}").await;
    assert!(said.contains("\"openTickets\"") && said.contains("\"To do\": 1"), "{said}");
    let said = ask(&env, &id, "mcp: get_ticket {\"ticket\": \"PX-1\"}").await;
    assert!(said.contains("Fix the login"), "{said}");

    // Creating a ticket waits for the user's yes.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Add a logout"}"#).await;
    let req = open_request(&env, &id).await;
    assert_eq!(ticket_titles(&env).await, ["Fix the login"], "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.starts_with("mcp create_ticket:") && said.contains("\"key\": \"PX-2\""), "{said}");
    assert_eq!(ticket_titles(&env).await, ["Add a logout", "Fix the login"]);
    let (_, t) = call(&env.app, "GET", "/api/tickets?project=&q=PX-2", None).await;
    assert_eq!(t["items"][0]["stage"], "draft");

    // A no leaves it undone.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Not this"}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(wait_said(&env, &id, n + 1).await.pop().unwrap(), "mcp create_ticket denied");
    assert_eq!(ticket_titles(&env).await.len(), 2);

    // A project that lets the PM act: PDT says yes for the user.
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["policy"]["pmActs"], true);
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "stage": "ready"}"#).await;
    assert!(said.contains("\"updated\": true") && said.contains("\"stage\": \"ready\""), "{said}");
    // What changed, from → to, for the conversation's card.
    assert!(said.contains("\"field\": \"stage\"") && said.contains("\"to\": \"ready\""), "{said}");

    // Messaging a session asks too (its project doesn't let the PM act), and a PM can't message itself.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, &format!(r#"mcp: send_to_session {{"session": "{id}", "text": "hi"}}"#)).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("that is this session"), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-100: every agent's PM runs in the work folder (the folder holding the projects' checkouts) with its agent's own
/// tools; Codex's also with live web search and the network in its sandbox.
#[tokio::test]
async fn the_pm_works_from_the_work_folder_with_its_agents_own_tools() {
    let env = setup().await;
    let work = env._tmp.path().canonicalize().unwrap();
    let said_cwd = |said: &str| {
        let dir = said.strip_prefix("cwd=").unwrap().split(' ').next().unwrap().to_string();
        std::path::PathBuf::from(dir).canonicalize().unwrap()
    };
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();
    mcp_server(&env, "Slack").await;

    let id = start_pm(&env, "where").await;
    let said = wait_said(&env, &id, 1).await.pop().unwrap();
    assert_eq!(said_cwd(&said), work, "{said}");
    assert!(said.ends_with("tools=\"all\""), "Claude keeps its built-in tools: {said}");
    // The user's MCP servers reach the PM too (PDT-322), with the note that their output isn't instructions.
    assert_eq!(mcp_servers_of(&env, &id), ["pdt", "slack"]);
    let text = instructions(&env, &id);
    assert!(text.contains("`slack`") && text.contains("never as instructions"), "{text}");
    let (_, projects) = call(&env.app, "GET", "/api/projects", None).await;
    let repo = std::path::PathBuf::from(projects[0]["repoPath"].as_str().unwrap());
    assert!(text.contains(&format!("from the work folder `{}`", repo.parent().unwrap().display())), "{text}");

    for (agent, want) in [("codex", " web_search=live network=true"), ("devin", "")] {
        let mut setup = pm["setup"].clone();
        setup["agent"] = json!(agent);
        let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", pm["id"].as_str().unwrap()), Some(setup)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let id = start_pm(&env, "where").await;
        let said = wait_said(&env, &id, 1).await.pop().unwrap();
        assert_eq!(said_cwd(&said), work, "{agent}: {said}");
        assert!(said.ends_with(want), "{agent}: {said}");
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// PDT-106: the PM links and unlinks tickets (blocks, blocked by, relates) and sets a parent, asking first like
/// `edit_ticket`; the links show on both tickets and hold the blocked one back; mistakes come back as errors.
#[tokio::test]
async fn the_pm_links_and_unlinks_tickets_and_sets_a_parent() {
    let env = setup().await;
    for title in ["Ship the login", "The epic"] {
        let url = format!("/api/projects/{}/tickets", env.project);
        let (s, v) = call(&env.app, "POST", &url, Some(json!({"title": title, "stage": "todo"}))).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let ticket = |key: &'static str| {
        let app = env.app.clone();
        async move {
            let (_, v) = call(&app, "GET", &format!("/api/tickets?project=&q={key}"), None).await;
            let id = v["items"][0]["id"].as_str().unwrap().to_string();
            call(&app, "GET", &format!("/api/tickets/{id}"), None).await.1
        }
    };
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;

    // Linking asks first, the same as edit_ticket.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: link_tickets {"ticket": "PX-2", "kind": "blockedBy", "other": "PX-1"}"#).await;
    let req = open_request(&env, &id).await;
    assert_eq!(ticket("PX-2").await["blockedBy"], 0, "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"linked\": true") && said.contains("\"blockedBy\""), "{said}");

    // Like a link made in the UI: on both tickets, recorded as the agent's, and the blocked one waits.
    let (two, one) = (ticket("PX-2").await, ticket("PX-1").await);
    assert_eq!((two["blockedBy"].as_i64(), two["clearToStart"].as_bool()), (Some(1), Some(false)), "{two}");
    assert_eq!(two["links"][0]["key"], "PX-1");
    assert_eq!(
        (one["links"][0]["key"].as_str(), one["links"][0]["direction"].as_str()),
        (Some("PX-2"), Some("outgoing"))
    );
    let (_, ev) = call(&env.app, "GET", &format!("/api/tickets/{}/events", two["id"].as_str().unwrap()), None).await;
    let linked = ev["items"].as_array().unwrap().iter().find(|e| e["kind"] == "linked").unwrap().clone();
    assert_eq!(linked["actor"], "agent", "{linked}");

    // The rest act without asking once the project lets the PM act.
    let (s, _) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK);
    for (call_, want) in [
        (r#"{"ticket": "PX-1", "kind": "blocks", "other": "PX-1"}"#, "can't link to itself"),
        (r#"{"ticket": "PX-1", "kind": "blockedBy", "other": "PX-2"}"#, "would close a cycle"),
        (r#"{"ticket": "PX-1", "kind": "blocks", "other": "PX-99"}"#, "there is no ticket PX-99"),
        (r#"{"ticket": "PX-1", "kind": "blocks", "other": "NOPE-1"}"#, "no project has the key NOPE"),
    ] {
        let said = ask(&env, &id, &format!("mcp: link_tickets {call_}")).await;
        assert!(said.contains(want) && !said.contains("\"linked\""), "{call_}: {said}");
    }
    assert_eq!(ticket("PX-1").await["links"].as_array().unwrap().len(), 1, "the refused links were not made");

    let said = ask(&env, &id, r#"mcp: unlink_tickets {"ticket": "PX-2", "kind": "blockedBy", "other": "PX-1"}"#).await;
    assert!(said.contains("\"unlinked\": true"), "{said}");
    let two = ticket("PX-2").await;
    assert_eq!((two["blockedBy"].as_i64(), two["links"].as_array().unwrap().len()), (Some(0), 0), "{two}");
    let said = ask(&env, &id, r#"mcp: unlink_tickets {"ticket": "PX-2", "kind": "blockedBy", "other": "PX-1"}"#).await;
    assert!(said.contains("there is no such link"), "{said}");

    // relates, either way round.
    let said = ask(&env, &id, r#"mcp: link_tickets {"ticket": "PX-1", "kind": "relates", "other": "PX-2"}"#).await;
    assert!(said.contains("\"relates\""), "{said}");
    assert_eq!(ticket("PX-2").await["links"][0]["kind"], "relates");
    let said = ask(&env, &id, r#"mcp: unlink_tickets {"ticket": "PX-2", "kind": "relates", "other": "PX-1"}"#).await;
    assert!(said.contains("\"unlinked\": true"), "{said}");
    assert!(ticket("PX-1").await["links"].as_array().unwrap().is_empty());

    // A parent, by key, and detached again.
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "parent": "PX-3"}"#).await;
    assert!(said.contains("\"parent\": \"PX-3\""), "{said}");
    assert_eq!(ticket("PX-3").await["partList"][0]["key"], "PX-2");
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "parent": ""}"#).await;
    assert!(said.contains("\"parent\": null"), "{said}");
    assert!(ticket("PX-2").await["parent"].is_null());
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "parent": "PX-2"}"#).await;
    assert!(!said.contains("\"updated\""), "a ticket can't be its own parent: {said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn notes_and_the_journal_carry_into_the_next_session() {
    let env = setup().await;
    let (s, n) = call(
        &env.app,
        "POST",
        "/api/pm/notes",
        Some(json!({"title": "Work hours", "body": "The user works 9 to 5.", "pinned": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    let (_, found) = call(&env.app, "GET", "/api/pm/notes?q=work", None).await;
    assert_eq!(found[0]["title"], "Work hours");

    let first = start_pm(&env, "hello").await;
    wait_said(&env, &first, 1).await;
    let said = ask(
        &env,
        &first,
        r#"mcp: save_note {"project": "PX", "title": "Deploys", "body": "Deploy keys are in the vault.", "pinned": true}"#,
    )
    .await;
    assert!(said.contains("\"saved\": true"), "{said}");
    assert!(said.contains("\"title\": \"Deploys\"") && said.contains("\"project\": \"PX\""), "{said}");
    let said = ask(&env, &first, r#"mcp: search_notes {"query": "vault"}"#).await;
    assert!(said.contains("Deploys"), "{said}");
    let said = ask(&env, &first, r#"mcp: write_journal {"body": "We planned the PM's notes."}"#).await;
    assert!(said.contains("\"written\": true"), "{said}");

    close_pm(&env, &first).await;
    let marked = pdt_says(&env, &first).await;
    assert!(marked.len() == 1 && marked[0].0 == "journal", "the journal request shows collapsed: {marked:?}");
    let second = start_pm(&env, "hello again").await;
    wait_said(&env, &second, 1).await;
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm["current"]["id"], second.as_str());
    let old = &pm["sessions"][1];
    assert_eq!((old["session"]["id"].as_str(), old["rotation"].as_str()), (Some(first.as_str()), Some("done")));
    assert_eq!((old["reason"].as_str(), old["session"]["status"].as_str()), (Some("user"), Some("finished")));
    assert_eq!(old["journal"], "We planned the PM's notes.");
    let text = instructions(&env, &second);
    for want in ["We planned the PM's notes.", "host-wide: Work hours", "· PX: Deploys", "## Since the last PM session"]
    {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    call(&env.app, "POST", &format!("/api/sessions/{second}/kill"), None).await;
}

/// The PM conversation `id` as `GET /api/pm/sessions` lists it.
async fn pm_info(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/pm/sessions", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().iter().find(|i| i["session"]["id"] == id).cloned().unwrap_or(Value::Null)
}

/// Closes a PM conversation and waits until it has ended (its journal turn over, its agent stopped).
async fn close_pm(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "DELETE", &format!("/api/pm/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    wait_until("the conversation to end", async || {
        let info = pm_info(env, id).await;
        // The row is marked done just before a stand-in journal is written; the session finishes once its agent has
        // exited, which can come a moment later.
        if info["open"] == false && !info["journal"].is_null() && info["session"]["status"] != "running" {
            return Ok(info);
        }
        pdt_api::run_workflows(&env.state).await;
        Err(info)
    })
    .await
}

async fn user_says(env: &Env, session: &str) -> Vec<String> {
    events(env, session)
        .await
        .iter()
        .filter(|e| e["event"]["kind"] == "user_message")
        .map(|e| e["event"]["text"].as_str().unwrap().to_string())
        .collect()
}

/// PDT's own messages to the conversation (PDT-211): which each is (`pdt`) and its text, in order.
async fn pdt_says(env: &Env, session: &str) -> Vec<(String, String)> {
    events(env, session)
        .await
        .iter()
        .map(|e| &e["event"])
        .filter(|e| e["kind"] == "user_message" && e["pdt"].is_string())
        .map(|e| {
            assert_eq!(e["source"], "notice", "{e}");
            (e["pdt"].as_str().unwrap().to_string(), e["text"].as_str().unwrap().to_string())
        })
        .collect()
}

#[tokio::test]
async fn several_pm_conversations_run_at_once_each_with_its_own_transcript() {
    let env = setup().await;
    let cb = start_pm(&env, "Plan the CB cutover").await;
    wait_said(&env, &cb, 1).await;
    let pdt = start_pm(&env, "A PDT bug\nThe board flickers.").await;
    wait_said(&env, &pdt, 1).await;

    // A new one ends none of the others; each is titled from its first message and keeps its own transcript.
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    assert_eq!(pm["current"]["id"], pdt.as_str(), "old clients get the newest open conversation");
    let (a, b) = (pm_info(&env, &cb).await, pm_info(&env, &pdt).await);
    assert_eq!((a["open"].as_bool(), a["title"].as_str()), (Some(true), Some("Plan the CB cutover")));
    assert_eq!((b["open"].as_bool(), b["title"].as_str()), (Some(true), Some("A PDT bug")));
    assert_eq!(a["unseen"], a["session"]["unseen"]);
    assert!(a["lastActivity"].as_i64().unwrap() >= a["session"]["createdAt"].as_i64().unwrap());
    assert_eq!(user_says(&env, &cb).await, ["Plan the CB cutover"]);
    assert_eq!(user_says(&env, &pdt).await, ["A PDT bug\nThe board flickers."]);
    ask(&env, &cb, "Back to the cutover").await;
    assert_eq!(user_says(&env, &pdt).await.len(), 1, "a message goes to its own conversation only");

    // Each briefing lists the other open conversations.
    let text = instructions(&env, &pdt);
    assert!(text.contains("## Other open PM conversations") && text.contains("- Plan the CB cutover ("), "{text}");
    let third = start_pm(&env, "Something else").await;
    wait_said(&env, &third, 1).await;
    let text = instructions(&env, &third);
    for want in ["- Plan the CB cutover (", "- A PDT bug ("] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    assert!(!text.contains("- Something else ("), "not itself: {text}");
    let (_, list) = call(&env.app, "GET", "/api/pm/sessions", None).await;
    assert_eq!(list.as_array().unwrap().iter().filter(|i| i["open"] == true).count(), 3);
    // The most recently active first.
    assert_eq!(list[0]["session"]["id"], third.as_str());

    // Renaming.
    let url = format!("/api/pm/sessions/{pdt}");
    let (s, v) = call(&env.app, "PATCH", &url, Some(json!({"title": "Board flicker"}))).await;
    assert_eq!((s, v["title"].as_str()), (StatusCode::OK, Some("Board flicker")));
    assert_eq!(pm_info(&env, &pdt).await["title"], "Board flicker");
    let (s, _) = call(&env.app, "PATCH", &url, Some(json!({"title": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "PATCH", "/api/pm/sessions/nope", Some(json!({"title": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Closing one asks it for its journal and ends it; the others stay open.
    let closed = close_pm(&env, &third).await;
    assert_eq!((closed["rotation"].as_str(), closed["reason"].as_str()), (Some("done"), Some("user")));
    assert_eq!(closed["journal"], "(no journal)");
    assert!(agent_says(&env, &third).await.iter().any(|m| m.contains("The user closed this PM conversation")));
    let (_, pm) = call(&env.app, "GET", "/api/pm", None).await;
    let open: Vec<&str> = pm["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["open"] == true)
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert_eq!(open.len(), 2, "{pm}");
    assert!(open.contains(&"Board flicker") && open.contains(&"Plan the CB cutover"));
    assert_eq!(pm["current"]["id"], pdt.as_str(), "the newest open one");
    assert_eq!(pm["sessions"][2]["session"]["id"], third.as_str(), "ended ones last");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/sessions/{third}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "closing an ended one does nothing");

    // One started without a message is named by the first one it gets; closing it ends it at once.
    let (s, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({}))).await;
    assert_eq!((s, v["title"].as_str()), (StatusCode::CREATED, Some("New conversation")));
    let blank = v["id"].as_str().unwrap().to_string();
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/sessions/{blank}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let info = pm_info(&env, &blank).await;
    assert_eq!((info["open"].as_bool(), info["journal"].is_null()), (Some(false), true), "no journal: it said nothing");
    let (_, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({"title": "Named"}))).await;
    let named = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["title"], "Named");
    let (_, v) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({}))).await;
    let fresh = v["id"].as_str().unwrap().to_string();
    ask(&env, &fresh, "Release notes for 1.4").await;
    assert_eq!(pm_info(&env, &fresh).await["title"], "Release notes for 1.4");
    ask(&env, &named, "Hi").await;
    assert_eq!(pm_info(&env, &named).await["title"], "Named", "a given title stays");
    for id in [&cb, &pdt, &named, &fresh] {
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// The conversation's events of `kind` with their text (`markdown` or `text`), in order.
async fn said_in_order(env: &Env, session: &str) -> Vec<(String, String)> {
    events(env, session)
        .await
        .iter()
        .filter_map(|e| {
            let ev = &e["event"];
            let text = ev["markdown"].as_str().or(ev["text"].as_str()).unwrap_or("");
            match ev["kind"].as_str()? {
                k @ ("user_message" | "agent_message" | "compaction") => Some((k.to_string(), text.to_string())),
                "turn" if ev["phase"] == "ended" => Some(("turn_ended".into(), String::new())),
                _ => None,
            }
        })
        .collect()
}

#[tokio::test]
async fn a_compacted_conversation_writes_a_checkpoint_after_its_turn_and_carries_on() {
    let env = setup().await;
    let id = start_pm(&env, "hello").await;
    wait_said(&env, &id, 1).await;
    // The agent compacts mid-turn and works on: the checkpoint request waits for the turn to end (the engine ticks
    // all along), then goes in as a turn of its own.
    send(&env, &id, "compact: 1500").await;
    let said = wait_said(&env, &id, 3).await;
    assert_eq!(said[1], "Compacted and carried on.");
    assert!(said[2].contains("[PDT] Your context was just compacted") && said[2].contains("carries on"), "{said:?}");
    let order = said_in_order(&env, &id).await;
    let at = |want: &dyn Fn(&(String, String)) -> bool| order.iter().position(want).unwrap();
    let compaction = at(&|(k, _)| k == "compaction");
    let carried_on = at(&|(k, t)| k == "agent_message" && t == "Compacted and carried on.");
    let request = at(&|(k, t)| k == "user_message" && t.contains("Your context was just compacted"));
    let turn_end = compaction + order[compaction..].iter().position(|(k, _)| k == "turn_ended").unwrap();
    assert!(compaction < carried_on && carried_on < turn_end && turn_end < request, "{order:?}");
    let marked = pdt_says(&env, &id).await;
    assert_eq!(marked.len(), 1, "{marked:?}");
    assert!(marked[0].0 == "checkpoint" && marked[0].1.contains("Your context was just compacted"), "{marked:?}");

    // Each checkpoint replaces the conversation's entry; it stays open, and the next conversation is briefed with it.
    let said = ask(&env, &id, r#"mcp: write_journal {"body": "Planning the CB cutover."}"#).await;
    assert!(said.contains("\"written\": true"), "{said}");
    let said = ask(&env, &id, r#"mcp: write_journal {"body": "CB cutover set for Friday."}"#).await;
    assert!(said.contains("\"written\": true"), "{said}");
    let info = pm_info(&env, &id).await;
    assert_eq!((info["open"].as_bool(), info["rotation"].as_str()), (Some(true), Some("none")));
    assert_eq!(info["journal"], "CB cutover set for Friday.");
    let (_, journal) = call(&env.app, "GET", "/api/pm/notes?kind=journal", None).await;
    assert_eq!(journal.as_array().unwrap().len(), 1, "{journal}");
    let next = start_pm(&env, "What's next?").await;
    wait_said(&env, &next, 1).await;
    let text = instructions(&env, &next);
    assert!(text.contains("CB cutover set for Friday.") && !text.contains("Planning the CB cutover."), "{text}");
    for s in [&id, &next] {
        call(&env.app, "POST", &format!("/api/sessions/{s}/kill"), None).await;
    }
}

/// The PM daily checkpoint built-in (PDT-372), installed and on.
async fn daily_checkpoint(env: &Env) -> Value {
    pdt_api::run_workflows(&env.state).await;
    let (_, list) = call(&env.app, "GET", "/api/workflows", None).await;
    let w = list.as_array().unwrap().iter().find(|w| w["builtin"] == "pm-daily-checkpoint").cloned();
    let w = w.unwrap_or_else(|| panic!("no PM daily checkpoint in {list}"));
    assert_eq!(w["enabled"], true, "{w}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{}", w["id"].as_str().unwrap()), None).await;
    w
}

/// Fires the PM daily checkpoint's schedule (its `slot`) and runs the engine until the run is over.
async fn fire_daily(env: &Env, wf: &Value, slot: i64) {
    let payload = json!({"workflowId": wf["id"], "nodeId": "hourly", "slot": slot,
                         "slotIso": "2026-11-01T09:00:00Z", "date": "2026-11-01"});
    let e = pdt_core::workflow::api::NewLogEvent::new(
        pdt_core::workflow::events::SCHEDULE_FIRED,
        format!("schedule:test:{slot}"),
        "scheduler",
        payload,
    );
    env.state.store.emit_event(&e, slot).await.unwrap().expect("logged");
    for _ in 0..3 {
        pdt_api::run_workflows(&env.state).await;
    }
}

/// Makes a PM conversation's last update `ago` ms old.
async fn updated_ago(env: &Env, id: &str, ago: i64) {
    let row = env.state.store.get_pm_session(id).await.unwrap().unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    assert!(env.state.store.set_pm_caught_up(id, row.caught_up_at, now - ago).await.unwrap());
}

/// Ticks the engine for a while (the outbox delivers what is waiting).
async fn settle(env: &Env) {
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        pdt_api::run_workflows(&env.state).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

const HOUR: i64 = 3_600_000;

#[tokio::test]
async fn a_conversation_in_use_gets_a_daily_update_and_carries_on() {
    let env = setup().await;
    let wf = daily_checkpoint(&env).await;
    let id = start_pm(&env, "hello").await;
    wait_said(&env, &id, 1).await;

    // Less than a day since its briefing: the schedule's run sends nothing, and ending a turn sends nothing either.
    fire_daily(&env, &wf, 1).await;
    settle(&env).await;
    assert_eq!(agent_says(&env, &id).await.len(), 1);

    // A day on, nothing happened yet: only a checkpoint request.
    updated_ago(&env, &id, 24 * HOUR).await;
    fire_daily(&env, &wf, 2).await;
    let said = wait_said(&env, &id, 2).await;
    for want in ["[PDT] Nothing that needs you happened", "`write_journal`", "carries on"] {
        assert!(said[1].contains(want), "missing {want:?} in\n{}", said[1]);
    }
    assert_eq!(pdt_says(&env, &id).await.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["checkpoint"]);
    // Answering it asks for no other, and the conversation stays open.
    fire_daily(&env, &wf, 3).await;
    settle(&env).await;
    assert_eq!(agent_says(&env, &id).await.len(), 2);
    let info = pm_info(&env, &id).await;
    assert_eq!((info["open"].as_bool(), info["rotation"].as_str()), (Some(true), Some("none")));

    // Another conversation writes its journal; the user writes here, and a day later the update brings it.
    let other = start_pm(&env, "CB cutover").await;
    wait_said(&env, &other, 1).await;
    ask(&env, &other, r#"mcp: write_journal {"body": "Cut over Friday.\n\n## Still open\n- DNS"}"#).await;
    ask(&env, &id, "again").await;
    updated_ago(&env, &id, 24 * HOUR).await;
    fire_daily(&env, &wf, 4).await;
    let said = wait_said(&env, &id, 4).await;
    for want in ["[PDT] Since the last update (", "## Tickets", "## Inbox", "### CB cutover (", "Still open:\n- DNS"] {
        assert!(said[3].contains(want), "missing {want:?} in\n{}", said[3]);
    }
    assert!(!said[3].contains("Cut over Friday."), "only what is still open: {}", said[3]);
    let marked = pdt_says(&env, &id).await;
    assert_eq!(marked.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["checkpoint", "update"], "{marked:?}");
    assert_eq!(pm_info(&env, &id).await["open"], true);
    // The other conversation was only just briefed: nothing for it.
    assert!(pdt_says(&env, &other).await.is_empty());
    for s in [&id, &other] {
        call(&env.app, "POST", &format!("/api/sessions/{s}/kill"), None).await;
    }
}

/// PDT-372: the user changes the PM daily checkpoint (a weekday time, half a day, every open conversation) and it
/// runs as changed, kept as theirs; turned off, no update goes out.
#[tokio::test]
async fn the_daily_checkpoint_runs_as_the_user_changed_it() {
    let env = setup().await;
    let wf = daily_checkpoint(&env).await;
    let wf_id = wf["id"].as_str().unwrap().to_string();
    let id = start_pm(&env, "hello").await;
    wait_said(&env, &id, 1).await;

    let mut doc = wf["doc"].clone();
    for n in doc["nodes"].as_array_mut().unwrap() {
        match n["id"].as_str().unwrap() {
            "hourly" => n["config"] = json!({"cron": "0 9 * * 1-5", "timezone": "UTC", "catchUp": true}),
            "update" => n["config"] = json!({"after": "12h", "onlyInUse": false}),
            _ => {}
        }
    }
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/workflows/{wf_id}"), Some(json!({"doc": doc, "note": "mine"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, saved) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}"), None).await;
    assert_eq!(saved["doc"]["nodes"][0]["config"]["cron"], "0 9 * * 1-5", "{saved}");

    // Half a day on (the default waits a day): it goes out.
    updated_ago(&env, &id, 13 * HOUR).await;
    fire_daily(&env, &saved, 10).await;
    wait_said(&env, &id, 2).await;
    let kinds = pdt_says(&env, &id).await.into_iter().map(|m| m.0).collect::<Vec<_>>();
    assert_eq!(kinds.len(), 1, "{kinds:?}");

    // Turned off: nothing more.
    let (s, v) = call(&env.app, "PATCH", &format!("/api/workflows/{wf_id}"), Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    updated_ago(&env, &id, 13 * HOUR).await;
    fire_daily(&env, &saved, 11).await;
    settle(&env).await;
    assert_eq!(pdt_says(&env, &id).await.len(), 1);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// The PM daily checkpoint as Settings → This host → PM shows it (PDT-384).
async fn checkpoint_settings(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/pm-checkpoint", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

fn node(doc: &Value, id: &str) -> Value {
    doc["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap_or_else(|| panic!("no {id}"))
}

/// PDT-384: Settings sets the daily checkpoint's time, days and chat channel as the built-in's inputs; the workflow
/// then runs at that time on those days, and sends an update half a day on.
#[tokio::test]
async fn settings_set_when_the_daily_checkpoint_runs() {
    let env = setup().await;
    let v = checkpoint_settings(&env).await;
    assert_eq!((v["enabled"].as_bool(), v["customized"].as_bool()), (Some(true), Some(false)), "{v}");
    assert_eq!(v["values"], v["defaults"]);
    assert_eq!((v["values"]["atTime"].as_bool(), v["values"]["days"].as_array().map(Vec::len)), (Some(false), Some(7)));
    let wf_id = v["workflowId"].as_str().unwrap().to_string();
    let id = start_pm(&env, "hello").await;
    wait_said(&env, &id, 1).await;

    let body = json!({"values": {"atTime": true, "time": "07:45", "days": ["mon", "tue", "wed", "thu", "fri"]}});
    let (s, v) = call(&env.app, "PUT", "/api/pm-checkpoint", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["values"]["time"].as_str(), v["customized"].as_bool()), (Some("07:45"), Some(false)), "{v}");
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}/versions"), None).await;
    assert_eq!(versions[0]["note"], "inputs changed in Settings", "{versions}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}"), None).await;
    assert_eq!(node(&w["doc"], "hourly")["config"], json!({"cron": "45 7 * * mon,tue,wed,thu,fri", "catchUp": true}));
    assert_eq!(node(&w["doc"], "update")["config"]["after"], "12h", "{w}");

    // At its time, half a day on (each hour waits a day): it goes out.
    updated_ago(&env, &id, 13 * HOUR).await;
    fire_daily(&env, &w, 20).await;
    wait_said(&env, &id, 2).await;
    assert_eq!(pdt_says(&env, &id).await.len(), 1);

    // Back to each hour, on every day.
    let body = json!({"values": {"atTime": false, "days": "mon,tue,wed,thu,fri,sat,sun"}});
    let (s, v) = call(&env.app, "PUT", "/api/pm-checkpoint", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}"), None).await;
    assert_eq!(node(&w["doc"], "hourly")["config"], json!({"every": "1h", "catchUp": true}));

    for bad in [json!({"time": "7am"}), json!({"days": []}), json!({"days": ["someday"]}), json!({"nope": 1})] {
        let (s, e) = call(&env.app, "PUT", "/api/pm-checkpoint", Some(json!({"values": bad}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {e}");
    }
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-384: a daily checkpoint changed in the workflow editor, or turned off, is only named by Settings; Reset to
/// default puts back the built-in, on.
#[tokio::test]
async fn a_customized_daily_checkpoint_is_reset_to_default() {
    let env = setup().await;
    let wf_id = checkpoint_settings(&env).await["workflowId"].as_str().unwrap().to_string();
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}"), None).await;
    let mut doc = w["doc"].clone();
    let update = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "update").unwrap();
    update["config"]["onlyInUse"] = json!(false);
    let (s, w) = call(&env.app, "PUT", &format!("/api/workflows/{wf_id}"), Some(json!({"doc": doc}))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(checkpoint_settings(&env).await["customized"], true);
    let (s, e) = call(&env.app, "PUT", "/api/pm-checkpoint", Some(json!({"values": {"atTime": true}}))).await;
    assert_eq!((s, e["code"].as_str()), (StatusCode::CONFLICT, Some("customized")), "{e}");

    let (s, _) = call(&env.app, "PATCH", &format!("/api/workflows/{wf_id}"), Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(checkpoint_settings(&env).await["enabled"], false);

    let (s, v) = call(&env.app, "POST", "/api/pm-checkpoint/reset", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["customized"].as_bool(), v["enabled"].as_bool()), (Some(false), Some(true)), "{v}");
    let (_, w) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}"), None).await;
    assert_eq!(node(&w["doc"], "update")["config"]["onlyInUse"], true, "the default again");
    let (_, versions) = call(&env.app, "GET", &format!("/api/workflows/{wf_id}/versions"), None).await;
    assert_eq!(versions[0]["note"], "reset to default", "{versions}");
}

#[tokio::test]
async fn notes_are_checked_and_edited() {
    let env = setup().await;
    let (s, v) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "x", "project": "NOPE"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (_, n) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "Style", "project": "PX"}))).await;
    assert_eq!(n["projectId"], env.project.as_str());
    let id = n["id"].as_str().unwrap();
    let (s, n) =
        call(&env.app, "PATCH", &format!("/api/pm/notes/{id}"), Some(json!({"project": "", "pinned": true}))).await;
    assert_eq!(s, StatusCode::OK, "{n}");
    assert_eq!((n["projectId"].clone(), n["pinned"].clone()), (Value::Null, json!(true)));
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_users_own_notes_are_found_by_the_pm_but_never_briefed_or_written_by_it() {
    let env = setup().await;
    let (s, v) = call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "x", "kind": "journal"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "only PM conversations write journal entries: {v}");
    let (s, mine) = call(
        &env.app,
        "POST",
        "/api/pm/notes",
        Some(json!({"title": "Dark mode idea", "body": "A toggle in Settings.", "kind": "user", "pinned": true,
                    "project": "PX"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{mine}");
    assert_eq!(mine["kind"], "user");
    let id = mine["id"].as_str().unwrap().to_string();
    call(&env.app, "POST", "/api/pm/notes", Some(json!({"title": "The PM's own", "pinned": true}))).await;

    let (_, listed) = call(&env.app, "GET", "/api/pm/notes?kind=user", None).await;
    let titles: Vec<&str> = listed.as_array().unwrap().iter().map(|n| n["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Dark mode idea"]);
    let (s, n) = call(&env.app, "GET", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!((s, n["title"].as_str()), (StatusCode::OK, Some("Dark mode idea")));
    let (s, n) = call(&env.app, "PATCH", &format!("/api/pm/notes/{id}"), Some(json!({"body": "Follow the OS."}))).await;
    assert_eq!((s, n["kind"].as_str(), n["body"].as_str()), (StatusCode::OK, Some("user"), Some("Follow the OS.")));

    let pm = start_pm(&env, "hello").await;
    wait_said(&env, &pm, 1).await;
    let text = instructions(&env, &pm);
    assert!(text.contains("The PM's own"), "{text}");
    assert!(!text.contains("Dark mode idea"), "the user's notes are never briefed, even pinned:\n{text}");

    let said = ask(&env, &pm, r#"mcp: search_notes {"query": "dark mode"}"#).await;
    assert!(said.contains("Dark mode idea") && said.contains("\"writtenBy\": \"the user\""), "{said}");
    let said = ask(&env, &pm, &format!(r#"mcp: save_note {{"id": "{id}", "title": "Mine now"}}"#)).await;
    assert!(said.contains("the user's own") && !said.contains("\"saved\""), "{said}");
    let said = ask(&env, &pm, &format!(r#"mcp: delete_note {{"id": "{id}"}}"#)).await;
    assert!(said.contains("the user's own") && !said.contains("\"deleted\""), "{said}");
    let said = ask(&env, &pm, r#"mcp: save_note {"title": "Made by the PM"}"#).await;
    assert!(said.contains("\"saved\": true"), "{said}");
    let made = said.split("\"id\": \"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_else(|| panic!("{said}"));
    let said = ask(&env, &pm, &format!(r#"mcp: delete_note {{"id": "{made}"}}"#)).await;
    assert!(said.contains("\"deleted\": true") && said.contains("\"title\": \"Made by the PM\""), "{said}");
    let (_, listed) = call(&env.app, "GET", "/api/pm/notes?kind=user", None).await;
    assert_eq!((listed[0]["title"].as_str(), listed.as_array().unwrap().len()), (Some("Dark mode idea"), 1));
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;

    let (s, _) = call(&env.app, "DELETE", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "the user deletes their own");
    let (s, _) = call(&env.app, "GET", &format!("/api/pm/notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

fn issue(n: u64, title: &str, body: &str) -> pdt_forge::Issue {
    pdt_forge::Issue {
        number: n,
        title: title.into(),
        body: body.into(),
        url: format!("https://github.com/o/r/issues/{n}"),
        author: "al".into(),
        labels: vec!["bug".into()],
        created_at: "2026-10-20T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn the_triage_template_has_triage_draft_an_external_ticket_for_a_new_issue() {
    let env = setup().await;
    // The test talks to the Triage session it holds: "When an agent is stuck" would nudge and restart it meanwhile.
    common::stuck_agents_off(&env.app).await;
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:issue-triage/instantiate",
        Some(json!({"params": {"repo": "o/r", "project": "PX"}, "scope": "project", "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let wf = w["id"].as_str().unwrap().to_string();
    call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": true}))).await;
    // The first look is the baseline; the next new issue starts a run.
    env.gh.set_issues("o/r", vec![issue(1, "Old", "")]);
    pdt_api::refresh_issues(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    env.gh.set_issues("o/r", vec![issue(2, "Login fails on Safari", "It spins forever."), issue(1, "Old", "")]);
    pdt_api::refresh_issues(&env.state).await;

    mcp_server(&env, "Slack").await;
    // The locked-down Triage persona's step session starts with the issue and where its ticket goes (PDT-201).
    let session = wait_until("a triage session", async || {
        pdt_api::run_workflows(&env.state).await;
        let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/sessions", env.project), None).await;
        let found = list.as_array().unwrap().iter().find(|s| s["persona"]["name"] == "Triage");
        found.map(|s| s["id"].as_str().unwrap().to_string()).ok_or(list)
    })
    .await;
    wait_said(&env, &session, 1).await;
    let first = events(&env, &session).await;
    let prompt = first.iter().find(|e| e["event"]["kind"] == "user_message").unwrap()["event"]["text"].clone();
    let prompt = prompt.as_str().unwrap();
    assert!(prompt.contains("#2 by") && prompt.contains("It spins forever."), "{prompt}");
    // The title and text are fenced (PDT-202).
    assert!(prompt.starts_with("Text between <untrusted id=\""), "{prompt}");
    assert!(prompt.contains("\">\nLogin fails on Safari\n</untrusted id=\""), "{prompt}");
    assert!(prompt.contains("project PX") && prompt.contains("submit_outcome"), "{prompt}");

    // Locked down: Claude keeps only its read tools.
    let n = agent_says(&env, &session).await.len();
    send(&env, &session, "where").await;
    let said = tool_answer(&env, &session, n, "tools=").await;
    assert!(said.contains("tools=\"Read,Grep,Glob\""), "{said}");
    // Nor any of the user's MCP servers (PDT-322).
    assert_eq!(mcp_servers_of(&env, &session), ["pdt"]);

    // Asking for To do or the agentic label is refused, even with the user's yes to the call.
    for call_text in [
        r#"mcp: create_ticket {"project": "PX", "title": "Login fails on Safari", "stage": "todo"}"#,
        r#"mcp: create_ticket {"project": "PX", "title": "Login fails on Safari", "labels": ["agentic"]}"#,
    ] {
        let n = agent_says(&env, &session).await.len();
        send(&env, &session, call_text).await;
        approve_open(&env, &session).await;
        let said = tool_answer(&env, &session, n, "mcp create_ticket").await;
        assert!(said.contains("failed") && said.contains("Drafts"), "{said}");
    }
    // A run_with_secrets is not among its tools, and refused anyway.
    let n = agent_says(&env, &session).await.len();
    send(&env, &session, r#"mcp: run_with_secrets {"command": "true", "env": {}}"#).await;
    assert!(tool_answer(&env, &session, n, "mcp run_with_secrets").await.contains("failed"));

    // Drafting the ticket asks first; then the outcome ends the run by `drafted`.
    let n = agent_says(&env, &session).await.len();
    send(&env, &session, r#"mcp: create_ticket {"project": "PX", "title": "Login fails on Safari"}"#).await;
    approve_open(&env, &session).await;
    let said = tool_answer(&env, &session, n, "mcp create_ticket").await;
    assert!(said.contains("\"created\": true"), "{said}");
    assert_eq!(ticket_titles(&env).await, ["Fix the login", "Login fails on Safari"]);
    let (_, t) = call(&env.app, "GET", "/api/tickets?project=&q=PX-2", None).await;
    let t = &t["items"][0];
    assert_eq!(
        (t["stage"].as_str(), t["source"].as_str(), t["admitted"].as_bool()),
        (Some("draft"), Some("external"), Some(false))
    );
    send(&env, &session, r#"mcp: submit_outcome {"outcome": "drafted", "summary": "Drafted PX-2."}"#).await;
    let r = wait_until("the run to succeed", async || {
        pdt_api::run_workflows(&env.state).await;
        let (_, runs) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
        let r = runs[0].clone();
        if r["status"] == "succeeded" { Ok(r) } else { Err(r) }
    })
    .await;
    assert_eq!(r["output"]["summary"], "Drafted PX-2.", "{r}");
    call(&env.app, "POST", &format!("/api/sessions/{session}/kill"), None).await;
}

/// Approves the session's open request.
/// A local MCP server (any program: sessions only name it) every agent gets.
async fn mcp_server(env: &Env, name: &str) {
    let body =
        json!({"name": name, "kind": "mcp", "url": "", "config": {"transport": "stdio", "command": "slack-mcp"}});
    let (s, v) = call(&env.app, "POST", "/api/connections", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

/// The MCP servers in a Claude session's `--mcp-config` file.
fn mcp_servers_of(env: &Env, session: &str) -> Vec<String> {
    let path = env.state.data_dir.join("sessions").join(session).join("mcp.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["mcpServers"].as_object().unwrap().keys().cloned().collect()
}

async fn approve_open(env: &Env, session: &str) {
    approve_open_except(env, session, &[]).await
}

/// [`approve_open`] for a request that isn't one of `answered`.
async fn approve_open_except(env: &Env, session: &str, answered: &[String]) {
    let req = open_request_except(env, session, answered).await;
    let url = format!("/api/sessions/{session}/requests/{req}");
    let (s, _) = common::call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

/// The first thing the agent said after its `n`th message that contains `what` (a nudge after a step's turn can come
/// in between).
async fn tool_answer(env: &Env, session: &str, n: usize, what: &str) -> String {
    wait_until(&format!("the agent to say {what}"), async || {
        let said = agent_says(env, session).await;
        if let Some(m) = said[n..].iter().find(|m| m.contains(what)) {
            return Ok(m.clone());
        }
        pdt_api::run_workflows(&env.state).await;
        Err(said)
    })
    .await
}

/// PDT-201: the PM lets an external ticket in only with the user's yes to that very call, even in a project that lets
/// it act without asking.
#[tokio::test]
async fn the_pm_moves_an_external_ticket_to_to_do_only_after_the_users_yes() {
    let env = setup().await;
    let (s, _) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK);
    let t = pdt_core::ticket::NewTicket { title: "From an issue".into(), ..Default::default() }.validate().unwrap();
    let t = pdt_core::ticket::ValidNewTicket { source: pdt_core::ticket::TicketSource::External, ..t };
    let made = env.state.store.create_ticket("ext", &env.project, &t, 1).await.unwrap();
    assert_eq!(made.summary.key, "PX-2");
    let stage = || async { call(&env.app, "GET", "/api/tickets/ext", None).await.1 };

    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    // Anything short of the board: PDT answers for the user as the project says.
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "stage": "ready", "priority": "high"}"#).await;
    assert!(said.contains("\"updated\": true"), "{said}");

    // To do: the user is asked all the same, and a no leaves it.
    let to_do = r#"mcp: edit_ticket {"ticket": "PX-2", "stage": "todo"}"#;
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, to_do).await;
    let denied = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{denied}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(wait_said(&env, &id, n + 1).await.pop().unwrap(), "mcp edit_ticket denied");
    assert_eq!(stage().await["stage"], "ready");

    // A yes that isn't the user's click (an agent's curl, without the UI token) grants nothing (PDT-204).
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, to_do).await;
    let curled = open_request_except(&env, &id, std::slice::from_ref(&denied)).await;
    let url = format!("/api/sessions/{id}/requests/{curled}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("only the user"), "{said}");
    assert_eq!(stage().await["stage"], "ready");

    // A yes lets it in.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, to_do).await;
    approve_open_except(&env, &id, &[denied, curled]).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"updated\": true") && said.contains("\"stage\": \"todo\""), "{said}");
    let v = stage().await;
    assert_eq!((v["stage"].as_str(), v["admitted"].as_bool()), (Some("todo"), Some(true)), "{v}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn the_pm_reads_the_users_todos_but_cannot_change_them() {
    let env = setup().await;
    for body in [
        json!({"title": "Buy milk", "body": "Oat, **not** soy.", "dueOn": "2026-10-26", "recurrence": {"rule": {"every": "week", "interval": 1, "days": [1]}, "from": "due"}}),
        json!({"title": "Review the login fix", "ticket": "PX-1"}),
    ] {
        let (s, v) = call(&env.app, "POST", "/api/todos", Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let pm = start_pm(&env, "hello").await;
    wait_said(&env, &pm, 1).await;
    let said = ask(&env, &pm, r#"mcp: list_todos {}"#).await;
    assert!(said.contains("Buy milk") && said.contains("Review the login fix"), "{said}");
    assert!(said.contains("\"ticket\": \"PX-1\""), "{said}");
    assert!(said.contains("\"every\": \"week\""), "the rule comes with it: {said}");
    assert!(said.contains("Oat, **not** soy."), "and the body: {said}");
    let said = ask(&env, &pm, r#"mcp: list_todos {"project": "PX"}"#).await;
    assert!(!said.contains("Buy milk") && said.contains("Review the login fix"), "{said}");
    let said = ask(&env, &pm, r#"mcp: list_todos {"done": true}"#).await;
    assert!(said.contains("\"todos\": []"), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{pm}/kill"), None).await;
}

#[tokio::test]
async fn the_pm_reports_a_bug_after_a_yes() {
    // Filed as the GitHub login (the fake forge), not with a token.
    let (base, _) = common::issues::FakeIssues::serve().await;
    let env = setup_with(|st| {
        st.update = std::sync::Arc::new(pdt_api::UpdateService::new(&base, "o/r", false, "0.1.0-local"));
    })
    .await;
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: report_bug {"title": "The board flickers in repo", "description": "On every poll of repo/a.txt.", "includeDiagnostics": true}"#).await;
    let req = open_request(&env, &id).await;
    assert!(env.gh.created_issues.lock().unwrap().is_empty(), "nothing before the yes");
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.starts_with("mcp report_bug:") && said.contains("\"kind\": \"issue\""), "{said}");
    let (repo, issue) = env.gh.created_issues.lock().unwrap()[0].clone();
    // What the agent wrote is redacted of the project's names (its folder is `repo`), as the log is.
    assert_eq!((repo.as_str(), issue.title.as_str()), ("o/r", "The board flickers in [name]"));
    assert_eq!(issue.labels, ["bug", "from-pdt"]);
    assert!(issue.body.contains("PDT version:"), "{}", issue.body);
    assert!(issue.body.contains("On every poll of [name]/a.txt."), "{}", issue.body);
}

/// PDT-256, PDT-490: a PM on an agent that can't ask in its own prompt (Codex) asks in its conversation, with a card
/// that is also the Inbox's approval item, and its call waits: only the user's Allow runs it, in the same turn; a Deny
/// tells it no.
#[tokio::test]
async fn a_codex_pm_reports_a_bug_after_the_users_allow() {
    let (base, _) = common::issues::FakeIssues::serve().await;
    let env = setup_with(|st| {
        st.update = std::sync::Arc::new(pdt_api::UpdateService::new(&base, "o/r", false, "0.1.0-local"));
    })
    .await;
    let filed = || env.gh.created_issues.lock().unwrap().clone();
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();
    let setup = json!({"note": "", "agent": "codex", "model": null, "instructions": pm["instructions"],
                       "permissionMode": pm["permissionMode"], "tools": pm["tools"]});
    let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", pm["id"].as_str().unwrap()), Some(setup)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let approval = || async {
        let (_, v) = call(&env.app, "GET", "/api/inbox", None).await;
        v["items"].as_array().unwrap().iter().find(|i| i["kind"] == "approval").cloned()
    };

    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: report_bug {"title": "The board flickers", "description": "On every poll."}"#).await;
    let req = open_request(&env, &id).await;
    assert!(filed().is_empty(), "nothing before the yes");
    let item = approval().await.expect("the card is in the Inbox too");
    assert_eq!((item["group"].as_str(), item["sessionId"].as_str()), (Some("approval"), Some(id.as_str())), "{item}");
    assert_eq!(item["body"]["requestId"], req.as_str());
    assert_eq!(item["body"]["tool"], "mcp__pdt__report_bug");
    assert_eq!(item["body"]["input"]["title"], "The board flickers");
    let url = format!("/api/sessions/{id}/requests/{req}");

    // An Allow that isn't the user's click (an agent's curl, without the UI token) runs nothing.
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(filed().is_empty());

    // The user's Allow runs the call as given, and the PM reads its result.
    let (s, v) = common::call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let said = tool_answer(&env, &id, n, "mcp report_bug").await;
    assert!(said.contains("\"kind\": \"issue\"") || said.contains("\"kind\":\"issue\""), "{said}");
    assert_eq!(filed()[0].1.title, "The board flickers");
    assert!(approval().await.is_none(), "answered");
    let (s, _) = common::call_as_ui(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a second click runs nothing");
    assert_eq!(filed().len(), 1);

    // A Deny files nothing, and the PM hears it.
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: report_bug {"title": "Not this one", "description": "No."}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    let (s, v) = call(&env.app, "POST", &url, Some(json!({"answer": {"type": "deny"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let said = tool_answer(&env, &id, n, "mcp report_bug").await;
    assert!(said.contains("denied"), "{said}");
    assert_eq!(filed().len(), 1);
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-159: a ticket the PM creates in To do and links as blocked right after waits out the start delay, so the link
/// holds it back; `stop_ticket` asks first, then cancels the ticket's work, stops its session, abandons its attempt
/// and moves it back to To do.
#[tokio::test]
async fn the_pm_stops_a_tickets_work_after_asking_and_its_links_hold_auto_start_back() {
    let env = setup().await;
    let settings = |body: Value| {
        let url = format!("/api/projects/{}/settings", env.project);
        let app = env.app.clone();
        async move {
            let (s, v) = call(&app, "PUT", &url, Some(body)).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    };
    settings(json!({"pmActs": true, "autoStart": "todo", "maxRunningAttempts": 5, "startDelaySeconds": 1})).await;
    let id = start_pm(&env, "hello").await;
    wait_said(&env, &id, 1).await;
    let said =
        ask(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "After the login", "stage": "todo"}"#).await;
    assert!(said.contains("\"key\": \"PX-2\""), "{said}");
    let said = ask(&env, &id, r#"mcp: link_tickets {"ticket": "PX-2", "kind": "blockedBy", "other": "PX-1"}"#).await;
    assert!(said.starts_with("mcp link_tickets:"), "{said}");
    // The delay is over (without waiting it out): PX-2 could start now, but for its link.
    settings(json!({"pmActs": true, "autoStart": "todo", "maxRunningAttempts": 5, "startDelaySeconds": 0})).await;
    for _ in 0..3 {
        common::pick_up(&env.state).await;
    }
    let stage = |key: &'static str| {
        let app = env.app.clone();
        async move {
            let (_, v) = call(&app, "GET", &format!("/api/tickets?project=&q={key}"), None).await;
            v["items"][0].clone()
        }
    };
    assert_eq!(stage("PX-1").await["stage"], "in_progress", "the blocker started");
    assert_eq!(stage("PX-2").await["stage"], "todo", "the link held it back");

    // Stopping asks (the project no longer lets the PM act), then stops PX-1's session and abandons its attempt.
    settings(json!({"pmActs": false, "autoStart": "off"})).await;
    let t = stage("PX-1").await;
    let (_, full) = call(&env.app, "GET", &format!("/api/tickets/{}", t["id"].as_str().unwrap()), None).await;
    let session = full["sessions"][0]["id"].as_str().expect("Start made a session").to_string();
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_eq!(s["status"], "running");
    let n = agent_says(&env, &id).await.len();
    send(&env, &id, r#"mcp: stop_ticket {"ticket": "PX-1"}"#).await;
    let req = open_request(&env, &id).await;
    let url = format!("/api/sessions/{id}/requests/{req}");
    call(&env.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    let said = wait_said(&env, &id, n + 1).await.pop().unwrap();
    assert!(said.contains("\"stopped\": true") && said.contains("\"sessionsStopped\": 1"), "{said}");
    let t = stage("PX-1").await;
    assert_eq!((&t["stage"], &t["attempt"]["state"]), (&json!("todo"), &json!("abandoned")), "{t}");
    let (_, ev) = call(&env.app, "GET", &format!("/api/tickets/{}/events", t["id"].as_str().unwrap()), None).await;
    let notes: Vec<&str> = ev["items"].as_array().unwrap().iter().filter_map(|e| e["body"].as_str()).collect();
    assert!(notes.contains(&"Work stopped by the PM."), "{ev}");
    let (_, s) = call(&env.app, "GET", &format!("/api/sessions/{session}"), None).await;
    assert_ne!(s["status"], "running");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-300: the user pastes a screenshot into the PM chat; the PM puts it on a new ticket (`pasted:1`) and a text file
/// on an existing one by path. Each asks as the tool always does, nothing more.
#[tokio::test]
async fn the_pm_attaches_a_pasted_image_and_a_file_to_tickets() {
    let env = setup().await;
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = start_pm(&env, "who").await;
    wait_said(&env, &id, 1).await;
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRfake".to_vec();
    let (s, _, _) = common::call_raw(&env.app, "POST", &format!("/api/sessions/{id}/attachments"), png.clone()).await;
    assert_eq!(s, StatusCode::CREATED);

    let said =
        ask(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "Site bug", "files": ["pasted:1"]}"#).await;
    assert!(said.contains("\"created\": true") && said.contains("\"mediaType\": \"image/png\""), "{said}");
    let (_, list) = call(&env.app, "GET", "/api/tickets/PX-2/attachments", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["actor"], "agent");
    assert_eq!(list[0]["agent"]["persona"], "PM");

    let log = env._tmp.path().join("server.log");
    std::fs::write(&log, "panic at the disco\n").unwrap();
    let call_json = json!({"ticket": "PX-1", "files": [log.to_string_lossy()]});
    let said = ask(&env, &id, &format!("mcp: edit_ticket {call_json}")).await;
    assert!(said.contains("\"updated\": true") && said.contains("server.log"), "{said}");
    let (_, list) = call(&env.app, "GET", "/api/tickets/PX-1/attachments", None).await;
    assert_eq!((&list[0]["name"], &list[0]["mediaType"]), (&json!("server.log"), &json!("text/plain")));

    let missing = env._tmp.path().join("no-such-file.png");
    let call_json = json!({"ticket": "PX-1", "files": [missing.to_string_lossy()]});
    let said = ask(&env, &id, &format!("mcp: edit_ticket {call_json}")).await;
    assert!(said.contains("can't read"), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-301: a project's Cut a release, made from the template under any name, is the release workflow the PM's
/// briefing names for it once it is on.
#[tokio::test]
async fn the_briefing_names_the_project_s_release_workflow() {
    let env = setup().await;
    let (s, w) = call(
        &env.app,
        "POST",
        "/api/workflow-templates/builtin:cut-release/instantiate",
        Some(json!({"params": {"workflow": "publish.yml"}, "name": "Ship it", "scope": "project",
                    "projectId": env.project})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let first = start_pm(&env, "Anything to release?").await;
    wait_said(&env, &first, 1).await;
    assert!(!instructions(&env, &first).contains("released with the workflow"), "off: not named");
    call(&env.app, "PATCH", &format!("/api/workflows/{}", w["id"].as_str().unwrap()), Some(json!({"enabled": true})))
        .await;
    let on = start_pm(&env, "Release it").await;
    wait_said(&env, &on, 1).await;
    let text = instructions(&env, &on);
    assert!(text.contains("released with the workflow `Ship it`"), "{text}");
}

/// Resets a PM conversation, ticking the engine (which delivers the journal request) until the reset answers; the
/// fresh conversation.
async fn reset_pm(env: &Env, id: &str) -> Value {
    let (app, url) = (env.app.clone(), format!("/api/pm/sessions/{id}/reset"));
    let reset = tokio::spawn(async move { call(&app, "POST", &url, None).await });
    wait_until("the reset to answer", async || {
        if reset.is_finished() {
            return true;
        }
        pdt_api::run_workflows(&env.state).await;
        false
    })
    .await;
    let (s, v) = reset.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// PDT-308: Reset ends a conversation with its journal and starts a fresh one with its title, settings and place in
/// the list, whose briefing leads with that journal; the old one stays readable.
#[tokio::test]
async fn a_reset_conversation_carries_on_fresh_with_its_name_settings_and_journal() {
    let env = setup().await;
    let first = start_pm(&env, "Plan the CB cutover").await;
    wait_said(&env, &first, 1).await;
    let said =
        ask(&env, &first, r#"mcp: write_journal {"body": "We were planning the cutover. Still open: DNS."}"#).await;
    assert!(said.contains("\"written\": true"), "{said}");
    let (s, _) = call(&env.app, "PATCH", &format!("/api/pm/sessions/{first}"), Some(json!({"title": "Cutover"}))).await;
    assert_eq!(s, StatusCode::OK);
    // Another conversation, more recently active, and the PM persona since switched to Codex.
    let other = start_pm(&env, "Something newer").await;
    wait_said(&env, &other, 1).await;
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();
    let mut setup = pm["setup"].clone();
    setup["agent"] = json!("codex");
    let (s, v) = call(&env.app, "PUT", &format!("/api/personas/{}", pm["id"].as_str().unwrap()), Some(setup)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let old = pm_info(&env, &first).await;
    let order = |list: &Value| -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .filter(|i| i["open"] == true)
            .map(|i| i["title"].as_str().unwrap().into())
            .collect()
    };
    let (_, before) = call(&env.app, "GET", "/api/pm/sessions", None).await;
    assert_eq!(order(&before), ["Something newer", "Cutover"]);

    let fresh = reset_pm(&env, &first).await;
    let id = fresh["id"].as_str().unwrap().to_string();
    assert_ne!(id, first);
    // The same title, agent, model, permission mode and persona, the persona at its current version (PDT-500).
    assert_eq!(fresh["title"], "Cutover");
    for key in ["agent", "model", "permissionMode", "bypassPermissions", "cwd"] {
        assert_eq!(fresh[key], old["session"][key], "{key}");
    }
    assert_eq!(fresh["agent"], "claude", "not the persona's agent now");
    assert_eq!(fresh["persona"]["id"], old["session"]["persona"]["id"]);
    assert_eq!(fresh["persona"]["version"], v["version"], "{v}");

    // The old one asked for its journal, ended with reason `reset` and stays readable, unread by nobody.
    assert!(agent_says(&env, &first).await.iter().any(|m| m.contains("The user is resetting this PM conversation")));
    assert_eq!(pdt_says(&env, &first).await.last().unwrap().0, "journal", "the request shows collapsed");
    let ended = pm_info(&env, &first).await;
    assert_eq!((ended["open"].as_bool(), ended["rotation"].as_str()), (Some(false), Some("done")));
    assert_eq!((ended["reason"].as_str(), ended["session"]["status"].as_str()), (Some("reset"), Some("finished")));
    assert_eq!(ended["session"]["unseen"], false, "the journal turn is no reply");
    assert_eq!(ended["journal"], "We were planning the cutover. Still open: DNS.");
    assert!(user_says(&env, &first).await.contains(&"Plan the CB cutover".to_string()), "the transcript is kept");

    // The fresh one knows where it came from, sits where the old one was, and starts from its journal first.
    let info = pm_info(&env, &id).await;
    assert_eq!((info["open"].as_bool(), info["resetFrom"].as_str()), (Some(true), Some(first.as_str())));
    let (_, after) = call(&env.app, "GET", "/api/pm/sessions", None).await;
    assert_eq!(order(&after), ["Something newer", "Cutover"], "no jump in the list");
    assert_eq!(info["lastActivity"], old["lastActivity"]);
    ask(&env, &id, "Where were we?").await;
    let text = instructions(&env, &id);
    let own = text.find("## This conversation before it was reset").expect(&text);
    assert!(text[own..].contains("We were planning the cutover. Still open: DNS."), "{text}");
    assert!(own < text.find("## Projects").unwrap(), "{text}");
    assert!(!text.contains("- Cutover ("), "not listed as another open conversation: {text}");
    assert_eq!(pm_info(&env, &id).await["title"], "Cutover", "its first message doesn't rename it");

    // Resetting it again chains back one by one; resetting an ended one is refused.
    let (s, _) = call(&env.app, "POST", &format!("/api/pm/sessions/{first}/reset"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let again = reset_pm(&env, &id).await;
    let third = again["id"].as_str().unwrap().to_string();
    assert_eq!(pm_info(&env, &third).await["resetFrom"], id.as_str());
    assert_eq!(pm_info(&env, &id).await["resetFrom"], first.as_str());
    let (s, _) = call(&env.app, "POST", "/api/pm/sessions/nope/reset", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    for id in [&other, &third] {
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// PDT-500: a reset conversation runs the PM persona's current version, so a tool saved after the old one started is
/// in its `tools/list`.
#[tokio::test]
async fn a_reset_conversation_gets_the_pm_persona_s_newest_tools() {
    let env = setup().await;
    let (_, personas) = call(&env.app, "GET", "/api/personas", None).await;
    let pm = personas.as_array().unwrap().iter().find(|p| p["builtin"] == "pm").unwrap().clone();
    let url = format!("/api/personas/{}", pm["id"].as_str().unwrap());
    let tools: Vec<Value> = pm["tools"].as_array().unwrap().clone();
    assert!(tools.contains(&json!("watch")), "{tools:?}");
    let save = |tools: Vec<Value>| {
        let setup = json!({"note": "", "agent": pm["agent"], "model": pm["model"], "instructions": pm["instructions"],
                           "permissionMode": pm["permissionMode"], "tools": tools});
        let (app, url) = (env.app.clone(), url.clone());
        async move {
            let (s, v) = call(&app, "PUT", &url, Some(setup)).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            v
        }
    };
    save(tools.iter().filter(|t| *t != "watch").cloned().collect()).await;
    let first = start_pm(&env, "hello").await;
    wait_said(&env, &first, 1).await;
    let has_watch = |said: &str| said.trim_start_matches("mcp tools:").split(',').any(|t| t == "watch");
    let said = ask(&env, &first, "mcp-list").await;
    assert!(said.starts_with("mcp tools:") && !has_watch(&said), "{said}");

    let newer = save(tools).await;
    let fresh = reset_pm(&env, &first).await;
    assert_eq!(fresh["persona"]["version"], newer["version"], "{fresh}");
    let id = fresh["id"].as_str().unwrap().to_string();
    let said = ask(&env, &id, "mcp-list").await;
    assert!(has_watch(&said), "{said}");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

/// PDT-308: a conversation that writes no journal in time ends with `(no journal)` before the fresh one starts; one
/// that never said anything is reset at once.
#[tokio::test]
async fn a_reset_records_no_journal_after_its_wait() {
    let env = setup_with(|st| st.pm_reset_wait_ms = 300).await;
    let first = start_pm(&env, "hello").await;
    wait_said(&env, &first, 1).await;
    // Nothing ticks the engine: the journal request is never delivered.
    let (s, fresh) = call(&env.app, "POST", &format!("/api/pm/sessions/{first}/reset"), None).await;
    assert_eq!(s, StatusCode::OK, "{fresh}");
    let ended = pm_info(&env, &first).await;
    assert_eq!((ended["reason"].as_str(), ended["journal"].as_str()), (Some("reset"), Some("(no journal)")));
    let id = fresh["id"].as_str().unwrap().to_string();
    let text = std::fs::read_to_string(env.state.data_dir.join("sessions").join(&id).join("briefing.md")).unwrap();
    assert!(text.contains("## This conversation before it was reset") && text.contains("(no journal)"), "{text}");
    pdt_api::run_workflows(&env.state).await;
    assert!(
        !agent_says(&env, &first).await.iter().any(|m| m.contains("resetting")),
        "the dropped request is never sent"
    );

    // One that never said anything has nothing to write down.
    let (_, blank) = call(&env.app, "POST", "/api/pm/sessions", Some(json!({"title": "Blank"}))).await;
    let blank = blank["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "POST", &format!("/api/pm/sessions/{blank}/reset"), None).await;
    assert_eq!((s, v["title"].as_str()), (StatusCode::OK, Some("Blank")));
    let info = pm_info(&env, &blank).await;
    assert_eq!((info["reason"].as_str(), info["journal"].is_null()), (Some("reset"), true));
    for id in [&id, v["id"].as_str().unwrap()] {
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// PDT-308: a reset carries on when its caller goes away mid-way (a closed page, a dropped connection), and one that
/// never finished (pdtd stopped mid-way) is listed as ended, not as resetting forever.
#[tokio::test]
async fn an_abandoned_reset_still_finishes_and_an_unfinished_one_counts_as_ended() {
    let env = setup().await;
    let first = start_pm(&env, "hello").await;
    wait_said(&env, &first, 1).await;
    let (app, url) = (env.app.clone(), format!("/api/pm/sessions/{first}/reset"));
    let request = tokio::spawn(async move { call(&app, "POST", &url, None).await });
    wait_until("the reset to start", async || pm_info(&env, &first).await["rotation"] == "journal").await;
    let resetting = pm_info(&env, &first).await;
    assert_eq!((resetting["open"].as_bool(), resetting["reason"].as_str()), (Some(true), Some("reset")));
    request.abort();
    let fresh = wait_until("the abandoned reset to finish", async || {
        let (_, list) = call(&env.app, "GET", "/api/pm/sessions", None).await;
        if let Some(i) = list.as_array().unwrap().iter().find(|i| i["resetFrom"] == first.as_str()) {
            return Ok(i.clone());
        }
        pdt_api::run_workflows(&env.state).await;
        Err(list)
    })
    .await;
    assert_eq!((fresh["open"].as_bool(), fresh["title"].as_str()), (Some(true), Some("hello")));
    // The fresh conversation is listed before the reset ends the old one.
    wait_until("the abandoned reset to end the old conversation", async || {
        pm_info(&env, &first).await["open"] == false
    })
    .await;

    // A reset cut short after its conversation ended, with no fresh one: ended, not "resetting".
    let other = start_pm(&env, "again").await;
    wait_said(&env, &other, 1).await;
    let ended = pdt_core::pm::Rotation::Done;
    let reset = Some(pdt_core::pm::RotateReason::Reset);
    assert!(env.state.store.set_pm_rotation(&other, pdt_core::pm::Rotation::None, ended, reset, 1).await.unwrap());
    let info = pm_info(&env, &other).await;
    assert_eq!((info["open"].as_bool(), info["reason"].as_str()), (Some(false), Some("reset")));
    for id in [fresh["session"]["id"].as_str().unwrap(), &other] {
        call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    }
}

/// PDT-329: the PM tags a ticket's areas when it drafts or edits one, from the project's areas (`get_ticket` lists
/// them); a name the project doesn't have is refused with the ones it has.
#[tokio::test]
async fn the_pm_tags_tickets_with_the_projects_areas() {
    let env = setup().await;
    let (_, projects) = call(&env.app, "GET", "/api/projects", None).await;
    let repo = std::path::PathBuf::from(projects[0]["repoPath"].as_str().unwrap());
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    std::fs::write(
        repo.join(".pdt/project.toml"),
        "[[area]]\nname = \"api\"\npaths = [\"api/**\"]\n\n[[area]]\nname = \"web\"\npaths = [\"web/**\"]\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "areas", "-q"]);
    let (s, v) =
        call(&env.app, "PUT", &format!("/api/projects/{}/settings", env.project), Some(json!({"pmActs": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = start_pm(&env, "hi").await;
    wait_said(&env, &id, 1).await;

    let said = ask(&env, &id, "mcp: get_ticket {\"ticket\": \"PX-1\"}").await;
    assert!(said.contains("\"projectAreas\"") && said.contains("api/**") && said.contains("\"areasNote\""), "{said}");
    let said =
        ask(&env, &id, r#"mcp: create_ticket {"project": "PX", "title": "An endpoint", "areas": ["api"]}"#).await;
    assert!(said.contains("\"key\": \"PX-2\""), "{said}");
    let areas = |key: &'static str| {
        let env = &env;
        async move {
            let (_, t) = call(&env.app, "GET", &format!("/api/tickets?project=&q={key}"), None).await;
            t["items"][0]["areas"].clone()
        }
    };
    assert_eq!(areas("PX-2").await, json!(["api"]));
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "areas": ["api", "web"]}"#).await;
    assert!(said.contains("\"updated\": true"), "{said}");
    assert_eq!(areas("PX-2").await, json!(["api", "web"]));
    let said = ask(&env, &id, r#"mcp: edit_ticket {"ticket": "PX-2", "areas": ["apo"]}"#).await;
    assert!(said.contains("not areas of the project: apo") && said.contains("api, web"), "{said}");
    assert_eq!(areas("PX-2").await, json!(["api", "web"]), "unchanged");
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}
