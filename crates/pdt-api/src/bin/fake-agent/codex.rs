//! A stand-in for `codex app-server --listen ws://… --ws-auth capability-token --ws-token-file F`, for tests. Frame
//! shapes follow the recorded run (crates/pdt-core/tests/fixtures/codex). What a turn's text does:
//! - `run: <cmd>` — a command that asks for approval (`accept`, `acceptForSession`, `decline`, `cancel`).
//! - `slow` — streams for ~20 s unless interrupted; `turn/steer` messages join it.
//! - `who` — says the thread, whether it was resumed, the turn's model, approval policy and sandbox, and whether the
//!   thread got developer instructions.
//! - `where` — says the thread's working folder, its `web_search` config and whether the turn's sandbox has the
//!   network.
//! - `exit` — the server exits 3.
//! - `mcp: <tool> <json arguments>` — calls that pdt-mcp tool (from the thread's `mcp_servers.pdt` config) without
//!   asking, as Codex does with `default_tools_approval_mode: approve`, and says `mcp <tool>: <text>` (`mcp <tool>
//!   failed: …` for a tool error).
//! - anything else — `echo: <text>` (plus `saw N images` when the input has `localImage`s).
//!
//! `thread/fork` makes a fork that is never saved; a turn on it answers with a ticket draft naming the thread forked
//! and what the fork and turn were given (`ephemeral`, sandbox, approval policy, an `outputSchema`).
//!
//! `thread/compact/start` and `review/start` run a short turn (a compaction item; a review answer), and
//! `account/read` reports a ChatGPT account.
//!
//! Threads that took a turn are saved in `<token dir>/fake-threads.json`, so a restarted server resumes them (and
//! refuses threads that never took one, as Codex does). The server's URL is written to `<token dir>/fake-url`.
//!
//! `fake-agent resume <thread> --remote <url> --remote-auth-token-env <VAR>` plays the TUI: it prints what it was
//! given and waits for its terminal to close.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// The fake TUI.
pub fn tui(args: &[String]) {
    let thread = args.get(2).cloned().unwrap_or_default();
    let url = flag(args, "--remote").unwrap_or_default();
    let var = flag(args, "--remote-auth-token-env").unwrap_or_default();
    let has_token = std::env::var(&var).is_ok_and(|t| !t.is_empty());
    println!("fake codex tui thread={thread} remote={url} token={}", if has_token { "set" } else { "missing" });
    let mut buf = [0u8; 256];
    while let Ok(n) = std::io::stdin().read(&mut buf) {
        if n == 0 {
            break;
        }
    }
}

#[derive(Default)]
struct Store {
    path: PathBuf,
    /// Saved threads (took a turn): id → turns.
    saved: HashMap<String, Vec<Value>>,
    /// Threads started in this process (saved or not).
    loaded: HashMap<String, Vec<Value>>,
    /// Forks (`thread/fork`): id → the thread forked and the fork's params. Never saved.
    forks: HashMap<String, (String, Value)>,
}

impl Store {
    fn load(path: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self { path, saved, loaded: HashMap::new(), forks: HashMap::new() }
    }
    fn save(&self) {
        let _ = std::fs::write(&self.path, serde_json::to_string(&self.saved).unwrap_or_default());
    }
}

struct Server {
    token: String,
    store: Mutex<Store>,
    next: AtomicU64,
}

/// A running turn: its id, its interrupt and where steered messages go.
type RunningTurn = (String, Arc<tokio::sync::Notify>, mpsc::UnboundedSender<String>);

struct Conn {
    tx: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_req: AtomicU64,
    /// Running turns by thread: (turn id, interrupt, steer).
    turns: Mutex<HashMap<String, RunningTurn>>,
    resumed: Mutex<std::collections::HashSet<String>>,
    /// Threads started or resumed with `developerInstructions`.
    instructed: Mutex<std::collections::HashSet<String>>,
    /// Each thread's last `thread/start` or `thread/resume` params.
    opened: Mutex<HashMap<String, Value>>,
}

impl Conn {
    fn send(&self, v: Value) {
        let _ = self.tx.send(v.to_string());
    }
    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }
    async fn ask(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_req.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        rx.await.ok()
    }
}

pub fn serve(args: &[String]) {
    let listen = flag(args, "--listen").unwrap_or_default();
    let token_file = PathBuf::from(flag(args, "--ws-token-file").unwrap_or_default());
    let token = std::fs::read_to_string(&token_file).unwrap_or_default().trim().to_string();
    let dir = token_file.parent().map(PathBuf::from).unwrap_or_default();
    let addr = listen.trim_start_matches("ws://").to_string();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
        let _ = std::fs::write(dir.join("fake-url"), &listen);
        let server = Arc::new(Server {
            token,
            store: Mutex::new(Store::load(dir.join("fake-threads.json"))),
            next: AtomicU64::new(1),
        });
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let server = server.clone();
            tokio::spawn(async move { connection(server, tcp).await });
        }
    });
}

#[allow(clippy::result_large_err)] // tungstenite's handshake callback signature
async fn connection(server: Arc<Server>, tcp: tokio::net::TcpStream) {
    let want = format!("Bearer {}", server.token);
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let ok = req.headers().get("Authorization").and_then(|v| v.to_str().ok()) == Some(want.as_str());
        if ok {
            Ok(resp)
        } else {
            let mut e = ErrorResponse::new(Some("unauthorized".into()));
            *e.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED;
            Err(e)
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(tcp, check).await else { return };
    let (mut write, mut read) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if write.send(Message::Text(m.into())).await.is_err() {
                break;
            }
        }
    });
    let conn = Arc::new(Conn {
        tx,
        pending: Mutex::new(HashMap::new()),
        next_req: AtomicU64::new(0),
        turns: Mutex::new(HashMap::new()),
        resumed: Mutex::new(Default::default()),
        instructed: Mutex::new(Default::default()),
        opened: Mutex::new(Default::default()),
    });
    while let Some(Ok(msg)) = read.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let method = v.get("method").and_then(Value::as_str).map(String::from);
        let id = v.get("id").cloned();
        match (method, id) {
            (None, Some(id)) => {
                if let Some(tx) = id.as_u64().and_then(|i| conn.pending.lock().unwrap().remove(&i)) {
                    let _ = tx.send(v.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            (Some(method), Some(id)) => {
                let p = v.get("params").cloned().unwrap_or(Value::Null);
                match handle(&server, &conn, &method, &p).await {
                    Ok(r) => conn.send(json!({"jsonrpc": "2.0", "id": id, "result": r})),
                    Err(m) => conn.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600, "message": m}})),
                }
            }
            _ => {}
        }
    }
}

async fn handle(server: &Arc<Server>, conn: &Arc<Conn>, method: &str, p: &Value) -> Result<Value, String> {
    let thread = p.get("threadId").and_then(Value::as_str).unwrap_or("").to_string();
    match method {
        "initialize" => Ok(json!({"userAgent": "fake-codex/0.163.0", "platformFamily": "unix"})),
        "model/list" => {
            Ok(json!({"data": [{"id": "gpt-fake", "displayName": "GPT Fake", "description": "", "hidden": false},
                                          {"id": "hidden-one", "displayName": "Hidden", "description": "", "hidden": true}],
                                 "nextCursor": null}))
        }
        "thread/start" => {
            let n = server.next.fetch_add(1, Ordering::SeqCst);
            let id = format!("thr-{}-{n}", super::run_tag());
            server.store.lock().unwrap().loaded.insert(id.clone(), Vec::new());
            if p.get("developerInstructions").and_then(Value::as_str).is_some_and(|t| t.contains("Running in")) {
                conn.instructed.lock().unwrap().insert(id.clone());
            }
            conn.opened.lock().unwrap().insert(id.clone(), p.clone());
            conn.notify("thread/started", json!({"thread": {"id": id}}));
            Ok(json!({"thread": {"id": id, "status": {"type": "idle"}}}))
        }
        "thread/resume" => {
            let mut st = server.store.lock().unwrap();
            if !st.loaded.contains_key(&thread) {
                match st.saved.get(&thread).cloned() {
                    Some(turns) => {
                        st.loaded.insert(thread.clone(), turns);
                    }
                    None => return Err(format!("thread/resume failed: no rollout found for thread id {thread}")),
                }
            }
            conn.resumed.lock().unwrap().insert(thread.clone());
            conn.opened.lock().unwrap().insert(thread.clone(), p.clone());
            if p.get("developerInstructions").and_then(Value::as_str).is_some_and(|t| t.contains("Running in")) {
                conn.instructed.lock().unwrap().insert(thread.clone());
            }
            Ok(json!({"thread": {"id": thread}}))
        }
        "thread/turns/list" => {
            let st = server.store.lock().unwrap();
            let turns = st.loaded.get(&thread).or(st.saved.get(&thread)).cloned().unwrap_or_default();
            Ok(json!({"data": turns, "nextCursor": null, "backwardsCursor": null}))
        }
        "thread/fork" => {
            let mut st = server.store.lock().unwrap();
            if !st.loaded.contains_key(&thread) && !st.saved.contains_key(&thread) {
                return Err(format!("no rollout found for thread id {thread}"));
            }
            if p["ephemeral"] == true && p["excludeTurns"] != true {
                return Err("ephemeral paginated thread/fork requires `excludeTurns: true`".into());
            }
            let n = server.next.fetch_add(1, Ordering::SeqCst);
            let id = format!("fork-{}-{n}", super::run_tag());
            st.forks.insert(id.clone(), (thread.clone(), p.clone()));
            Ok(json!({"thread": {"id": id}}))
        }
        "thread/unsubscribe" => Ok(json!({})),
        "account/read" => {
            Ok(json!({"account": {"type": "chatgpt", "email": null, "planType": "pro"}, "requiresOpenaiAuth": true}))
        }
        "thread/compact/start" | "review/start" => {
            let n = server.next.fetch_add(1, Ordering::SeqCst);
            let turn = format!("turn-{n}");
            let item = if method == "review/start" {
                let target = p.pointer("/target/type").and_then(Value::as_str).unwrap_or("?");
                json!({"type": "agentMessage", "id": format!("msg-{n}"), "text": format!("reviewed: {target}"),
                       "phase": "final_answer"})
            } else {
                json!({"type": "contextCompaction", "id": format!("compact-{n}")})
            };
            let conn = conn.clone();
            tokio::spawn(async move {
                let note = |m: &str, mut v: Value| {
                    v["threadId"] = json!(thread);
                    conn.notify(m, v);
                };
                note("turn/started", json!({"turn": {"id": turn, "items": [], "status": "inProgress"}}));
                note("item/completed", json!({"turnId": turn, "item": item}));
                note(
                    "turn/completed",
                    json!({"turn": {"id": turn, "status": "completed", "durationMs": 1, "items": []}}),
                );
            });
            Ok(json!({}))
        }
        "turn/start" if server.store.lock().unwrap().forks.contains_key(&thread) => {
            // A side turn on a fork: a draft that says what the fork and the turn were given.
            let (parent, fork) = server.store.lock().unwrap().forks.get(&thread).cloned().unwrap();
            let n = server.next.fetch_add(1, Ordering::SeqCst);
            let turn = format!("turn-{n}");
            let description = format!(
                "ephemeral={} sandbox={} approval={} schema={}",
                fork["ephemeral"],
                fork["sandbox"].as_str().unwrap_or("?"),
                p["approvalPolicy"].as_str().unwrap_or("?"),
                p.get("outputSchema").is_some_and(|s| s.is_object())
            );
            let answer = json!({"title": format!("Draft of {parent}"), "description": description}).to_string();
            let item =
                json!({"type": "agentMessage", "id": format!("msg-{n}"), "text": answer, "phase": "final_answer"});
            let conn = conn.clone();
            let result = json!({"turn": {"id": turn, "items": [], "status": "inProgress"}});
            tokio::spawn(async move {
                let note = |m: &str, mut v: Value| {
                    v["threadId"] = json!(thread);
                    conn.notify(m, v);
                };
                note("turn/started", json!({"turn": {"id": turn, "items": [], "status": "inProgress"}}));
                note("item/completed", json!({"turnId": turn, "item": item}));
                note("turn/completed", json!({"turn": {"id": turn, "status": "completed", "items": []}}));
            });
            Ok(result)
        }
        "turn/start" => {
            if !server.store.lock().unwrap().loaded.contains_key(&thread) {
                return Err("thread not loaded".into());
            }
            let n = server.next.fetch_add(1, Ordering::SeqCst);
            let turn = format!("turn-{n}");
            let text = p.pointer("/input/0/text").and_then(Value::as_str).unwrap_or("").to_string();
            let (steer_tx, steer_rx) = mpsc::unbounded_channel();
            let interrupt = Arc::new(tokio::sync::Notify::new());
            conn.turns.lock().unwrap().insert(thread.clone(), (turn.clone(), interrupt.clone(), steer_tx));
            let (server, conn, p) = (server.clone(), conn.clone(), p.clone());
            let t = turn.clone();
            tokio::spawn(async move { run_turn(server, conn, thread, t, text, p, interrupt, steer_rx).await });
            Ok(json!({"turn": {"id": turn, "status": "inProgress"}}))
        }
        "turn/steer" => {
            let expected = p.get("expectedTurnId").and_then(Value::as_str).unwrap_or("");
            let text = p.pointer("/input/0/text").and_then(Value::as_str).unwrap_or("").to_string();
            match conn.turns.lock().unwrap().get(&thread) {
                Some((t, _, steer)) if t == expected => {
                    let _ = steer.send(text);
                    Ok(json!({"turnId": t}))
                }
                _ => Err("no running turn to steer".into()),
            }
        }
        "turn/interrupt" => {
            if let Some((_, interrupt, _)) = conn.turns.lock().unwrap().get(&thread) {
                interrupt.notify_one();
            }
            Ok(json!({}))
        }
        other => Err(format!("{other} is not supported by the fake")),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    server: Arc<Server>,
    conn: Arc<Conn>,
    thread: String,
    turn: String,
    text: String,
    params: Value,
    interrupt: Arc<tokio::sync::Notify>,
    mut steer: mpsc::UnboundedReceiver<String>,
) {
    let started = Instant::now();
    let n = server.next.fetch_add(1, Ordering::SeqCst);
    let tid = thread.clone();
    let note = |m: &str, mut p: Value| {
        p["threadId"] = json!(tid);
        conn.notify(m, p);
    };
    let mut items: Vec<Value> = Vec::new();
    let user = json!({"type": "userMessage", "id": format!("user-{n}"), "content": [{"type": "text", "text": text}]});
    note("thread/status/changed", json!({"status": {"type": "active", "activeFlags": []}}));
    note("turn/started", json!({"turn": {"id": turn, "items": [], "status": "inProgress"}}));
    note("item/started", json!({"turnId": turn, "item": user}));
    note("item/completed", json!({"turnId": turn, "item": user}));
    items.push(user);
    // The thread is saved once it takes a turn.
    {
        let mut st = server.store.lock().unwrap();
        st.saved.entry(thread.clone()).or_default();
        st.save();
    }
    let say = |text: &str, items: &mut Vec<Value>| {
        let id = format!("msg-{}", server.next.fetch_add(1, Ordering::SeqCst));
        let item = json!({"type": "agentMessage", "id": id, "text": text, "phase": "final_answer"});
        note(
            "item/started",
            json!({"turnId": turn, "item": {"type": "agentMessage", "id": id, "text": "", "phase": "final_answer"}}),
        );
        let cut = (0..=text.len() / 2).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        for part in [&text[..cut], &text[cut..]] {
            if !part.is_empty() {
                note("item/agentMessage/delta", json!({"turnId": turn, "itemId": id, "delta": part}));
            }
        }
        note("item/completed", json!({"turnId": turn, "item": item}));
        items.push(item);
    };
    let mut status = "completed";
    let t = text.trim();
    if let Some(cmd) = t.strip_prefix("run:") {
        let cmd = cmd.trim();
        let id = format!("exec-{n}");
        let item = json!({"type": "commandExecution", "id": id, "command": format!("/bin/zsh -lc '{cmd}'"),
                          "commandActions": [{"type": "unknown", "command": cmd}], "cwd": "/", "status": "inProgress"});
        note("item/started", json!({"turnId": turn, "item": item}));
        note("thread/status/changed", json!({"status": {"type": "active", "activeFlags": ["waitingOnApproval"]}}));
        let req = json!({"threadId": thread, "turnId": turn, "itemId": id, "command": format!("/bin/zsh -lc '{cmd}'"),
                         "cwd": "/", "commandActions": [{"type": "unknown", "command": cmd}],
                         "availableDecisions": ["accept", "acceptForSession", "decline", "cancel"]});
        let answer = tokio::select! {
            a = conn.ask("item/commandExecution/requestApproval", req) => a,
            _ = interrupt.notified() => None,
        };
        let decision =
            answer.as_ref().and_then(|a| a.get("decision")).and_then(Value::as_str).unwrap_or("cancel").to_string();
        if answer.is_some() {
            note(
                "serverRequest/resolved",
                json!({"requestId": conn.next_req.load(Ordering::SeqCst).saturating_sub(1)}),
            );
        }
        match decision.as_str() {
            "accept" | "acceptForSession" => {
                let mut done = item.clone();
                done["status"] = json!("completed");
                done["aggregatedOutput"] = json!(format!("ran {cmd}"));
                done["exitCode"] = json!(0);
                note(
                    "item/commandExecution/outputDelta",
                    json!({"turnId": turn, "itemId": id, "delta": format!("ran {cmd}")}),
                );
                note("item/completed", json!({"turnId": turn, "item": done}));
                items.push(done);
                say(&format!("Ran it (decision={decision})."), &mut items);
            }
            "decline" => {
                let mut done = item.clone();
                done["status"] = json!("declined");
                note("item/completed", json!({"turnId": turn, "item": done}));
                items.push(done);
                say("Not running it.", &mut items);
            }
            _ => status = "interrupted",
        }
    } else if t == "slow" {
        let id = format!("msg-{n}");
        let end = Instant::now() + Duration::from_secs(20);
        let mut steered = Vec::new();
        while Instant::now() < end && status == "completed" {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    note("item/agentMessage/delta", json!({"turnId": turn, "itemId": id, "delta": "."}));
                }
                _ = interrupt.notified() => status = "interrupted",
                Some(s) = steer.recv() => {
                    let u = json!({"type": "userMessage", "id": format!("user-steer-{}", server.next.fetch_add(1, Ordering::SeqCst)),
                                   "content": [{"type": "text", "text": s}]});
                    note("item/completed", json!({"turnId": turn, "item": u}));
                    items.push(u);
                    steered.push(s);
                    if steered.len() == 1 {
                        // A steer ends the slow part early, so tests need not wait.
                        break;
                    }
                }
            }
        }
        if status == "completed" {
            say(&format!("Finished the slow job. steered={steered:?}"), &mut items);
        }
    } else if let Some(call) = t.strip_prefix("mcp:") {
        let opened = conn.opened.lock().unwrap().get(&thread).cloned().unwrap_or_default();
        let pdt = &opened["config"]["mcp_servers.pdt"];
        let server = pdt["url"].as_str().zip(pdt.pointer("/http_headers/Authorization").and_then(Value::as_str));
        let reply = match server.map(|(u, a)| (u.to_string(), a.to_string())) {
            None => "mcp error: no pdt-mcp server".to_string(),
            Some(server) => {
                let call = call.trim().to_string();
                let (tool, args) = call.split_once(' ').unwrap_or((&call, "{}"));
                let (tool, args) = (tool.to_string(), serde_json::from_str(args.trim()).unwrap_or(json!({})));
                let name = tool.clone();
                let r = tokio::task::spawn_blocking(move || {
                    crate::rpc(&server, "tools/call", json!({"name": name, "arguments": args}))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                match r {
                    Ok(r) => format!(
                        "mcp {tool}{}: {}",
                        if r["isError"] == true { " failed" } else { "" },
                        r["content"][0]["text"].as_str().unwrap_or("")
                    ),
                    Err(e) => format!("mcp error: {e}"),
                }
            }
        };
        say(&reply, &mut items);
    } else if t == "exit" {
        eprintln!("fake codex: crashing as asked");
        // Let the frames sent so far reach the client first.
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::process::exit(3);
    } else if t == "where" {
        let opened = conn.opened.lock().unwrap().get(&thread).cloned().unwrap_or_default();
        let s = format!(
            "cwd={} web_search={} network={}",
            opened["cwd"].as_str().unwrap_or("?"),
            opened.pointer("/config/web_search").and_then(Value::as_str).unwrap_or("default"),
            params.pointer("/sandboxPolicy/networkAccess").and_then(Value::as_bool).unwrap_or(false),
        );
        say(&s, &mut items);
    } else if t == "who" {
        let resumed = conn.resumed.lock().unwrap().contains(&thread);
        let instructions = conn.instructed.lock().unwrap().contains(&thread);
        let s = format!(
            "thread {thread} resumed={resumed} model={} approval={} sandbox={} instructions={instructions}",
            params.get("model").and_then(Value::as_str).unwrap_or("default"),
            params.get("approvalPolicy").and_then(Value::as_str).unwrap_or("?"),
            params.pointer("/sandboxPolicy/type").and_then(Value::as_str).unwrap_or("?"),
        );
        say(&s, &mut items);
    } else {
        say(&format!("echo: {t}"), &mut items);
        let images = params
            .get("input")
            .and_then(Value::as_array)
            .map_or(0, |a| a.iter().filter(|i| i["type"] == "localImage" && i["path"].is_string()).count());
        if images > 0 {
            say(&format!("saw {images} images"), &mut items);
        }
    }
    // Messages steered into the turn are taken before it ends. The turn stops taking steers first, so one that comes
    // later is refused (the client then starts a new turn) rather than accepted and lost.
    conn.turns.lock().unwrap().remove(&thread);
    if status == "completed" {
        while let Ok(s) = steer.try_recv() {
            let u = json!({"type": "userMessage", "id": format!("user-steer-{}", server.next.fetch_add(1, Ordering::SeqCst)),
                           "content": [{"type": "text", "text": s}]});
            note("item/completed", json!({"turnId": turn, "item": u}));
            items.push(u);
            say(&format!("echo: {s}"), &mut items);
        }
    }
    note(
        "thread/tokenUsage/updated",
        json!({"turnId": turn, "tokenUsage": {"last": {"totalTokens": 1000}, "modelContextWindow": 400000}}),
    );
    let duration = started.elapsed().as_millis() as u64;
    let done = json!({"id": turn, "items": items, "status": status, "durationMs": duration});
    {
        let mut st = server.store.lock().unwrap();
        st.loaded.entry(thread.clone()).or_default().push(done.clone());
        st.saved.entry(thread.clone()).or_default().push(done);
        st.save();
    }
    note("thread/status/changed", json!({"status": {"type": "idle"}}));
    note("turn/completed", json!({"turn": {"id": turn, "status": status, "durationMs": duration, "items": []}}));
}
