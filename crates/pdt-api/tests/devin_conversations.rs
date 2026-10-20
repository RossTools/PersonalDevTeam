//! Devin conversations end to end (PDT-55), with `fake-agent acp` standing in for `devin acp`.

mod common;

use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
}

async fn setup(tmp: &Path) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let user_config = tmp.join("devin-user").join("config.json");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "{ \"devin\": { \"org_id\": \"org-1\" } }").unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.platform.devin_program = FAKE.to_string();
    state.platform.devin_config_file = Some(user_config);
    state.platform.login_path = None;
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, 201, "{p}");
    Env { app, state, pid: p["id"].as_str().unwrap().into() }
}

async fn new_devin(env: &Env, extra: Value) -> String {
    let mut body = json!({"projectId": env.pid, "kind": "agent_tui", "agent": "devin", "cols": 80, "rows": 24});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (s, v) = call(&env.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["mode"], "conversation", "Devin opens as a conversation by default");
    v["id"].as_str().unwrap().to_string()
}

async fn events(env: &Env, id: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
    assert_eq!(s, 200, "{v}");
    let mut all = v.as_array().unwrap().clone();
    all.sort_by_key(|e| e["pos"].as_i64());
    all
}

async fn wait_for(env: &Env, id: &str, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    for _ in 0..400 {
        let all = events(env, id).await;
        if f(&all) {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never saw {what}: {:#?}", events(env, id).await);
}

fn ended(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        .map(|e| e["event"]["status"].as_str().unwrap().to_string())
        .collect()
}

async fn wait_turns(env: &Env, id: &str, n: usize) -> Vec<Value> {
    wait_for(env, id, &format!("{n} turns"), |a| ended(a).len() >= n).await
}

fn said(all: &[Value]) -> Vec<String> {
    all.iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

fn open_request(all: &[Value]) -> Option<Value> {
    all.iter().find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open").map(|e| e["event"].clone())
}

async fn send(env: &Env, id: &str, text: &str, delivery: &str) {
    let body = json!({"text": text, "delivery": delivery});
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/messages"), Some(body)).await;
    assert_eq!(s, 202, "{v}");
}

async fn answer(env: &Env, id: &str, req: &str, a: Value) {
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/requests/{req}"), Some(json!({"answer": a}))).await;
    assert_eq!(s, 204, "{v}");
}

async fn session(env: &Env, id: &str) -> Value {
    call(&env.app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

/// Activity is recorded just after the event that changes it, so it is waited for (20 s: a loaded Windows runner).
async fn wait_activity(env: &Env, id: &str, want: &str) {
    for _ in 0..800 {
        if session(env, id).await["activity"] == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("activity never became {want}: {}", session(env, id).await);
}

async fn stop(env: &Env, id: &str) {
    call(&env.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
}

#[tokio::test]
async fn a_devin_conversation_gets_pdts_instructions_and_reports_its_session_models_and_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "who"})).await;
    let all = wait_turns(&env, &id, 1).await;
    let kinds: Vec<&str> = all.iter().map(|e| e["event"]["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["turn", "user_message", "reasoning", "usage", "agent_message", "turn"], "{all:#?}");
    assert_eq!(ended(&all), ["completed"]);
    let s = session(&env, &id).await;
    let devin_id = s["externalSessionId"].as_str().expect("Devin's session id is kept").to_string();
    assert!(devin_id.starts_with("fake-"), "{s}");
    // Accept edits is Devin's "Code" mode; the --config file carries PDT's hook with the instructions.
    assert_eq!(
        said(&all),
        [format!("session={devin_id} loaded=false mode=accept-edits model=fake-1 instructions=true")]
    );
    assert_eq!(s["permissionMode"], "accept_edits");
    let (st, caps) = call(&env.app, "GET", "/api/agents/devin/capabilities", None).await;
    assert_eq!(st, 200);
    assert_eq!(caps["conversation"], true);
    assert_eq!(caps["permissionModes"], json!(["accept_edits", "plan", "bypass"]));
    assert_eq!(caps["models"][1]["value"], "fake-2");
    assert_eq!(caps["commands"][0]["name"], "compact");
    // The user's own config is kept in PDT's copy.
    let config = env.state.data_dir.join("sessions").join(&id).join("devin-config.json");
    let written: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(written["devin"]["org_id"], "org-1");

    // A model change applies live; Make ticket is not offered for Devin.
    let (st, v) = call(&env.app, "PATCH", &format!("/api/sessions/{id}"), Some(json!({"model": "fake-2"}))).await;
    assert_eq!(st, 200, "{v}");
    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 2).await;
    assert!(said(&all)[1].contains("model=fake-2"), "{:?}", said(&all));
    let (st, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/ticket-draft"), None).await;
    assert_eq!((st.as_u16(), v["code"].as_str()), (409, Some("draft_unsupported")), "{v}");
    stop(&env, &id).await;
}

#[tokio::test]
async fn approvals_questions_and_edits_round_trip_in_devins_terms() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "run: touch x"})).await;
    let all = wait_for(&env, &id, "an approval", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!((req["requestKind"].as_str(), req["title"].as_str()), (Some("approval"), Some("touch x")));
    assert_eq!(req["canAlways"], true, "Devin's session-only option");
    wait_activity(&env, &id, "needs_you").await;
    answer(&env, &id, req["id"].as_str().unwrap(), json!({"type": "approve", "always": true})).await;
    let all = wait_turns(&env, &id, 1).await;
    assert_eq!(said(&all).last().unwrap(), "ran it (allow_session)", "never a rule saved to Devin's settings");

    // A denial with a note: Devin's reject has no message, so the note follows as a message, taken into the turn.
    send(&env, &id, "run: rm y", "next").await;
    let all = wait_for(&env, &id, "a second approval", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    answer(&env, &id, req["id"].as_str().unwrap(), json!({"type": "deny", "message": "not that"})).await;
    let all = wait_turns(&env, &id, 2).await;
    let note = all.iter().find(|e| e["event"]["kind"] == "user_message" && e["event"]["text"] == "not that");
    assert_eq!(note.expect("the note")["event"]["source"], "steer");
    assert!(said(&all).iter().any(|m| m == "not run (reject_once)"), "{:?}", said(&all));
    let declined = all.iter().any(|e| e["event"]["kind"] == "tool_call" && e["event"]["status"] == "declined");
    assert!(declined);

    send(&env, &id, "ask: Which one?", "next").await;
    let all = wait_for(&env, &id, "a question", |a| open_request(a).is_some()).await;
    let req = open_request(&all).unwrap();
    assert_eq!(req["requestKind"], "question");
    assert_eq!(req["input"]["questions"][0]["options"][1]["label"], "B");
    let a = json!({"type": "question", "answers": {"Which one?": ["B"]}});
    answer(&env, &id, req["id"].as_str().unwrap(), a).await;
    let all = wait_turns(&env, &id, 3).await;
    assert_eq!(said(&all).last().unwrap(), "you chose B");

    send(&env, &id, "edit", "next").await;
    let all = wait_turns(&env, &id, 4).await;
    let edit = all.iter().rev().find(|e| e["event"]["kind"] == "tool_call").unwrap();
    assert_eq!(edit["event"]["title"], "f.txt");
    assert_eq!(edit["event"]["diff"], "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n");
    let diff = all.iter().find(|e| e["event"]["kind"] == "turn_diff").expect("the turn's changes");
    assert!(diff["event"]["diff"].as_str().unwrap().contains("+b"));
    stop(&env, &id).await;
}

#[tokio::test]
async fn messages_sent_mid_turn_join_it_and_interrupt_ends_it() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "slow"})).await;
    wait_for(&env, &id, "the slow turn", |a| a.iter().any(|e| e["event"]["kind"] == "turn")).await;
    send(&env, &id, "also this", "next").await;
    wait_for(&env, &id, "the steered message", |a| {
        a.iter().any(|e| e["event"]["kind"] == "user_message" && e["event"]["source"] == "steer")
    })
    .await;
    let (s, _) = call(&env.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert_eq!(s, 204);
    let all = wait_turns(&env, &id, 1).await;
    assert_eq!(ended(&all), ["interrupted"], "one turn, ended by the interrupt");
    stop(&env, &id).await;
}

#[tokio::test]
async fn a_stopped_conversation_resumes_its_devin_session_or_starts_a_new_one_when_it_was_never_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "hi"})).await;
    wait_turns(&env, &id, 1).await;
    let first = session(&env, &id).await["externalSessionId"].clone();
    stop(&env, &id).await;
    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 2).await;
    assert!(said(&all)[1].contains("loaded=true"), "{:?}", said(&all));
    assert!(!said(&all).iter().any(|m| m.contains("old answer")), "the replayed history is not logged twice");
    assert_eq!(session(&env, &id).await["externalSessionId"], first);
    stop(&env, &id).await;

    // Never prompted: Devin never saved it, so the next message starts a new session (with a notice).
    let id = new_devin(&env, json!({})).await;
    for _ in 0..100 {
        if session(&env, &id).await["externalSessionId"].is_string() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let unsaved = session(&env, &id).await["externalSessionId"].clone();
    assert!(unsaved.is_string());
    stop(&env, &id).await;
    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 1).await;
    assert!(said(&all)[0].contains("loaded=false"), "{:?}", said(&all));
    assert!(all.iter().any(|e| e["event"]["kind"] == "notice"), "{all:#?}");
    assert_ne!(session(&env, &id).await["externalSessionId"], unsaved);
    stop(&env, &id).await;
}

#[tokio::test]
async fn a_devin_that_exits_ends_the_turn_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "exit"})).await;
    let all = wait_turns(&env, &id, 1).await;
    assert_eq!(ended(&all), ["interrupted"]);
    let err = wait_for(&env, &id, "the error", |a| a.iter().any(|e| e["event"]["kind"] == "error")).await;
    let text =
        err.iter().find(|e| e["event"]["kind"] == "error").unwrap()["event"]["text"].as_str().unwrap().to_string();
    assert!(text.starts_with("Devin stopped unexpectedly (exit code 3)"), "{text}");
    assert!(text.contains("fake devin: exiting"), "{text}");
}

#[tokio::test]
async fn switching_to_the_terminal_and_back_imports_the_turns_taken_there() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let id = new_devin(&env, json!({"initialPrompt": "hi"})).await;
    wait_turns(&env, &id, 1).await;
    let devin_id = session(&env, &id).await["externalSessionId"].as_str().unwrap().to_string();

    let body = json!({"mode": "terminal", "cols": 80, "rows": 24});
    let (s, v) = call(&env.app, "POST", &format!("/api/sessions/{id}/mode"), Some(body)).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["mode"].as_str(), v["status"].as_str()), (Some("terminal"), Some("running")));
    assert!(env.state.ptys.is_running(&id), "Devin's own terminal runs the session");
    // A turn taken in the terminal lands in Devin's saved session (the stand-in's history file).
    tokio::time::sleep(Duration::from_millis(20)).await;
    let saved = std::env::temp_dir().join(format!("fake-acp-saved-{devin_id}"));
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let mut history = std::fs::read_to_string(&saved).unwrap();
    history.push_str(&format!("{}\n", json!({"text": "typed in the terminal", "atMs": at})));
    std::fs::write(&saved, history).unwrap();

    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{id}/mode"), Some(json!({"mode": "conversation"}))).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["mode"], "conversation");
    assert!(!env.state.ptys.is_running(&id), "the terminal quit (Devin locks a session to one process)");
    let all = wait_turns(&env, &id, 2).await;
    let typed =
        all.iter().find(|e| e["event"]["kind"] == "user_message" && e["event"]["text"] == "typed in the terminal");
    assert_eq!(typed.expect("the terminal's turn")["event"]["source"], "terminal");
    let answers = said(&all);
    assert_eq!(answers.last().unwrap(), "old answer to typed in the terminal");
    assert!(!answers.iter().any(|m| m == "old answer to hi"), "turns PDT logged are not imported again: {answers:?}");
    assert!(all.iter().any(|e| e["event"]["kind"] == "notice"), "the switch is noted");

    send(&env, &id, "who", "next").await;
    let all = wait_turns(&env, &id, 3).await;
    assert!(said(&all).last().unwrap().contains(&format!("session={devin_id} loaded=true")), "{:?}", said(&all));
    stop(&env, &id).await;
}
