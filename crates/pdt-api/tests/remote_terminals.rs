//! Remote terminals (PDT-633): an owner (`studio`) and a remote instance (`win-box`) on loopback, paired for one
//! project. A shell or an agent's terminal started from the owner runs on the remote instance, in the remote
//! project's checkout or a ticket's worktree there and nowhere else, and the owner's browser terminal is bridged to it:
//! typing, resize, reconnect with the scrollback (gzipped too), exit, restart and removal.

use crate::common::wait::wait_until;
use crate::common::{call, call_with};
use crate::remote_attempts::{Pair, paired, sessions, ticket, worktrees};
use axum::http::StatusCode;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(30);

/// Typed text contains "4000+2000+21"; only the shell's evaluated output contains "6021".
const PROBE: &str = if cfg!(windows) { "echo (4000+2000+21)\r" } else { "echo $((4000+2000+21))\r" };
const PROBE2: &str = if cfg!(windows) { "echo (3000+1)\r" } else { "echo $((3000+1))\r" };

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The owner's terminal WebSocket for `id`, as its browser opens it.
async fn terminal(p: &Pair, id: &str, query: &str) -> Ws {
    let url = format!("ws://127.0.0.1:{}/api/sessions/{id}/pty{query}", p.owner.state.listen_port);
    connect_async(url).await.unwrap().0
}

async fn read_until(ws: &mut Ws, needle: &str) -> String {
    let mut out = String::new();
    tokio::time::timeout(WAIT, async {
        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Binary(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Message::Text(t) => out.push_str(t.as_str()),
                _ => {}
            }
            if out.contains(needle) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}; got {out:?}"));
    out
}

/// The binary frames before the first text frame, and that frame.
async fn replay(ws: &mut Ws) -> (Vec<Vec<u8>>, String) {
    let mut frames = vec![];
    tokio::time::timeout(WAIT, async {
        loop {
            match ws.next().await.expect("open").expect("a frame") {
                Message::Binary(b) => frames.push(b.to_vec()),
                Message::Text(t) => return (frames, t.to_string()),
                _ => {}
            }
        }
    })
    .await
    .expect("a replay")
}

async fn session(p: &Pair, id: &str) -> Value {
    call(&p.owner.app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_on_the_remote_instance_is_usable_from_the_owner() {
    let p = paired().await;
    let owner = &p.owner;
    // "New shell on win-box": in the remote project's checkout there.
    let body = json!({"projectId": p.project, "kind": "shell", "cols": 80, "rows": 24, "instanceId": p.instance_id});
    let (s, shell) = call(&owner.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{shell}");
    let id = shell["id"].as_str().unwrap().to_string();
    assert_eq!(shell["instance"]["name"], "win-box", "{shell}");
    assert_eq!(shell["cwd"], p.checkout.to_string_lossy().as_ref(), "{shell}");
    assert!(p.remote.state.ptys.is_running(&id), "the PTY runs on win-box");
    assert!(!owner.state.ptys.is_running(&id), "and not on the owner");

    // Typing and output through the bridge, and a keystroke's round trip.
    let mut ws = terminal(&p, &id, "").await;
    ws.send(Message::Text(r#"{"type":"resize","cols":100,"rows":30}"#.into())).await.unwrap();
    ws.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "6021").await;
    let sent = Instant::now();
    ws.send(Message::Binary(PROBE2.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "3001").await;
    let round_trip = sent.elapsed();
    println!("a command's round trip through the owner to the remote instance: {round_trip:?}");
    assert!(round_trip < Duration::from_secs(5), "{round_trip:?}");
    #[cfg(unix)]
    {
        // The resize reached the PTY there.
        ws.send(Message::Binary(b"stty size\r".to_vec().into())).await.unwrap();
        read_until(&mut ws, "30 100").await;
    }

    // Reconnecting gets the scrollback the remote kept, plain or gzipped.
    drop(ws);
    let mut again = terminal(&p, &id, "").await;
    let (frames, ready) = replay(&mut again).await;
    assert_eq!(ready, r#"{"type":"ready"}"#);
    assert!(String::from_utf8_lossy(&frames.concat()).contains("6021"));
    let mut gz = terminal(&p, &id, "?replay=gzip").await;
    let (frames, _) = replay(&mut gz).await;
    assert_eq!(frames.len(), 1, "one gzip frame");
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&frames[0][..]), &mut text).unwrap();
    assert!(text.contains("6021"), "{text:?}");
    drop(gz);

    // It stays the owner's: the remote instance runs it for the owner and lists it read-only, never as its own.
    let rows = p.remote.state.store.list_exec_sessions(None).await.unwrap();
    assert_eq!((rows.len(), rows[0].agent.as_str()), (1, "shell"), "{rows:?}");
    let (_, listed) = call(&p.remote.app, "GET", "/api/remote-projects", None).await;
    assert_eq!(listed[0]["running"][0]["live"], true, "{listed}");
    let (_, own) = call(&p.remote.app, "GET", "/api/projects", None).await;
    assert!(own.as_array().unwrap().is_empty(), "{own}");

    // Stopped from the owner: the exit reaches its terminal, and the session ends.
    let (s, v) = call(&owner.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert!(s.is_success(), "{v}");
    read_until(&mut again, "\"type\":\"exit\"").await;
    wait_until("the session ended", async || session(&p, &id).await["status"] == "finished").await;

    // Restarted: it runs there again.
    let (s, v) =
        call(&owner.app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "running");
    let mut ws = terminal(&p, &id, "").await;
    ws.send(Message::Binary(PROBE2.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "3001").await;

    // Closing its tab removes it, and stops it there.
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    wait_until("the PTY there stopped", async || !p.remote.state.ptys.is_running(&id)).await;
    p.remote.state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_agent_on_the_remote_instance_is_usable_from_the_owner() {
    let p = paired().await;
    let owner = &p.owner;
    let t = ticket(&p, "Build the client", &["windows"]).await;
    let start = json!({"agent": "claude", "sessionMode": "terminal"});
    let (s, started) = call(&owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let id = started["session"]["id"].as_str().unwrap().to_string();
    let branch = started["attempt"]["branch"].as_str().unwrap();
    let path = p.checkout.join(".worktrees").join(branch);
    let s = session(&p, &id).await;
    assert_eq!((s["mode"].as_str(), s["instance"]["name"].as_str()), (Some("terminal"), Some("win-box")), "{s}");
    assert_eq!(s["cwd"], path.to_string_lossy().as_ref());
    assert_eq!(s["activity"], "idle");
    assert!(p.remote.state.ptys.is_running(&id));

    // Typing reaches the agent's terminal there.
    let mut ws = terminal(&p, &id, "").await;
    ws.send(Message::Binary(b"hello-remote\r".to_vec().into())).await.unwrap();
    read_until(&mut ws, "hello-remote").await;

    // Its hooks report to win-box, which passes them on: the owner's session shows the agent's activity.
    let hook = |event: &str| json!({"source": "claude", "event": event});
    let url = format!("/api/sessions/{id}/activity");
    let (s, v) = call(&p.remote.app, "POST", &url, Some(hook("UserPromptSubmit"))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(session(&p, &id).await["activity"], "working");
    let (s, _) = call(&p.remote.app, "POST", &url, Some(hook("Stop"))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(session(&p, &id).await["activity"], "idle");

    // Under its ticket on the owner; the agent leaving ends the session there.
    assert!(sessions(&p).await.iter().any(|s| s["id"] == id.as_str() && s["ticket"]["id"] == t.as_str()));
    assert!(worktrees(&p).await.iter().any(|w| w["branch"] == branch && w["instance"]["name"] == "win-box"));
    ws.send(Message::Binary(b"/exit\r".to_vec().into())).await.unwrap();
    read_until(&mut ws, "\"type\":\"exit\"").await;
    wait_until("the session ended", async || session(&p, &id).await["status"] == "finished").await;
    p.remote.state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_shell_opens_only_in_the_remote_projects_folders() {
    let p = paired().await;
    let owner = &p.owner;
    let shell = |extra: Value| {
        let mut body = json!({"projectId": p.project, "kind": "shell", "cols": 80, "rows": 24});
        body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        body
    };
    // Only on an instance that joined the project, and a worktree's shell where the worktree is.
    let (s, v) = call(&owner.app, "POST", "/api/sessions", Some(shell(json!({"instanceId": "elsewhere"})))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, wt) =
        call(&owner.app, "POST", &format!("/api/projects/{}/worktrees", p.project), Some(json!({"branch": "here"})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{wt}");
    let (s, v) = call(
        &owner.app,
        "POST",
        "/api/sessions",
        Some(shell(json!({"instanceId": p.instance_id, "worktreeId": wt["id"]}))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // The remote instance takes a folder from no one: the owner names a branch, and a branch that would lead out of
    // the checkout, or that has no worktree there, is refused.
    let body = shell(json!({"instanceId": p.instance_id}));
    let (s, row) = call(&owner.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{row}");
    let (_, v) = call(&owner.app, "GET", &format!("/api/projects/{}/remote-instances", p.project), None).await;
    let link = v["instances"][0]["linkId"].as_str().unwrap().to_string();
    let raw = owner.state.secrets.get(&format!("project-link:{link}")).unwrap().unwrap();
    let bytes: [u8; 32] = base64::engine::general_purpose::STANDARD.decode(raw).unwrap().try_into().unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&bytes);
    let open = |branch: &str| {
        let mut session = row.clone();
        session["id"] = json!(uuid::Uuid::now_v7().to_string());
        session["cwd"] = json!(std::env::temp_dir().to_string_lossy());
        json!({
            "session": session, "ticketKey": null, "branch": branch, "base": "main", "cols": 80, "rows": 24,
            "fresh": true, "prompt": null, "model": null, "credentials": "subscription", "instructions": null, "mcp": null,
        })
    };
    let path = "/api/link/exec/pty/open";
    for (branch, why) in [("../../outside", StatusCode::BAD_REQUEST), ("nope", StatusCode::CONFLICT)] {
        let body = open(branch);
        let bytes = serde_json::to_vec(&body).unwrap();
        let headers = pdt_api::link::sign_headers(&key, path, &link, &p.project, &bytes, 1, now_ms());
        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let (s, v) = call_with(&p.remote.app, "POST", path, Some(body), &h).await;
        assert_eq!(s, why, "{branch}: {v}");
    }
    // The folder the row names is ignored: it runs in the checkout.
    let rows = p.remote.state.store.list_exec_sessions(None).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].cwd, p.checkout.to_string_lossy());
    // Unsigned, nothing runs.
    let (s, _) = call(&p.remote.app, "POST", path, Some(open(""))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    p.remote.state.ptys.shutdown().await;
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A Codex rollout file for a conversation that began at `began_ms` in `cwd`, as Codex writes after a turn.
fn rollout(codex_home: &std::path::Path, id: &str, cwd: &std::path::Path, began_ms: i64) {
    let began = jiff::Timestamp::from_millisecond(began_ms).unwrap();
    let day = began.to_zoned(jiff::tz::TimeZone::UTC).strftime("%Y/%m/%d").to_string();
    let dir = codex_home.join("sessions").join(day);
    std::fs::create_dir_all(&dir).unwrap();
    let began = began.to_string();
    let meta = json!({"timestamp": began, "type": "session_meta",
        "payload": {"id": id, "timestamp": began, "cwd": cwd.to_string_lossy()}});
    std::fs::write(dir.join(format!("rollout-2026-11-23T00-00-00-{id}.jsonl")), format!("{meta}\n{{}}\n")).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_codex_terminal_keeps_its_conversation_for_restarts() {
    let p = paired().await;
    // win-box's own Codex home (its Codex is `fake-agent`), where the conversation is saved.
    let home = p.remote.state.platform.codex_home.clone().unwrap();
    let remote = &p.remote;
    let body = json!({"projectId": p.project, "kind": "agent_tui", "agent": "codex", "mode": "terminal",
        "cols": 80, "rows": 24, "instanceId": p.instance_id});
    let (s, row) = call(&p.owner.app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{row}");
    let id = row["id"].as_str().unwrap().to_string();
    assert!(remote.state.ptys.is_running(&id));
    // It took a turn: Codex saved the conversation there, in the checkout.
    rollout(&home, "conv-remote", &p.checkout, now_ms() + 300);

    // When it ends, win-box finds it in its own Codex files and the owner's row gets it.
    let (s, v) = call(&p.owner.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert!(s.is_success(), "{v}");
    wait_until("the conversation recorded", async || {
        let s = session(&p, &id).await;
        (s["externalSessionId"] == "conv-remote" && s["status"] == "finished").then_some(()).ok_or(s)
    })
    .await;
    remote.state.ptys.shutdown().await;
}
