//! A stand-in for `claude -p` stream-json, for tests (spec §12): frame shapes copied from recorded runs. Never
//! shipped. What a user message does:
//! - `run: <cmd>` — asks to run `<cmd>` with Bash (a `can_use_tool` request), then reports it ran or was denied.
//! - `ask: <question>` — asks an AskUserQuestion with options A and B, then says the answer.
//! - `slow` — streams text for ~20 s unless interrupted (messages sent meanwhile wait for the next turn).
//! - `exit` — writes to stderr and exits 3.
//! - `compact: <ms>` (its first line) — compacts its context (a `compact_boundary`), then works on for `<ms>` before
//!   ending the turn.
//! - `deaf` — ends its turn, then never reads another message (it stays alive; nothing sent to it is echoed).
//! - `env: NAME` — says whether NAME is set.
//! - `bg: <ms>` (its first line) — starts a background task and ends the turn; when the task finishes `<ms>` later it reports it to
//!   itself like Claude (a replayed `task-notification` message), which starts a turn that says so. Closing stdin
//!   first ends it with the process.
//! - `bg-quiet: <ms>` — like `bg:`, but the finished task only drops off the list: no turn takes it up.
//! - `who` — says the session id, whether it was resumed, its model and mode, whether it got PDT's instructions, and
//!   the `session_id` the message carried.
//! - `where` — says its working folder and its built-in tools (`--tools`; `all` without it).
//! - `/clear` — like Claude's: `conversation_reset` (whose `new_conversation_id` names no transcript), then an init
//!   and result under a new session id, which later turns use.
//! - `/local` — like Claude's local commands (`/context`): answers without echoing the message first.
//! - `mcp: <tool> <json arguments>` (its first line) — calls that pdt-mcp tool (from `--mcp-config`) and says
//!   `mcp <tool>: <the result's text>`, or `mcp error: …`. Like Claude, it first asks to use a listed tool that
//!   `--allowedTools` leaves out (unless bypassing permissions), and says `mcp <tool> denied` when refused.
//! - `mcp-list` — says the pdt-mcp tools it was given.
//! - a message with images — says `saw N images (<media types>)`.
//! - anything else — streams `echo: <text>` back.
//!
//! Started without `-p` (Claude's TUI, in a terminal) it reads lines and leaves on `/exit`.
//!
//! With `--fork-session` (Make ticket's side request) it reads the prompt from stdin and prints one `json` result
//! whose `structured_output` is a draft naming the session and what it was given.
//!
//! `FAKE_AGENT_LOG` (a file path), when set, gets every stdin line and the argv.
//!
//! `fake-agent app-server …` plays `codex app-server` instead (see `codex.rs`), `fake-agent resume …` the Codex
//! TUI attached to it, and `fake-agent [--config <file>] acp` Devin's ACP server (see `acp.rs`).

mod acp;
mod codex;

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Agent {
    session: String,
    resumed: bool,
    model: String,
    mode: String,
    /// PDT's instructions arrived (`--append-system-prompt-file` names a readable file).
    instructions: bool,
    msg: u64,
    queue: Vec<Value>,
    rx: mpsc::Receiver<Option<Value>>,
    /// Feeds `rx`: background tasks report through it.
    tx: mpsc::Sender<Option<Value>>,
    log: Option<std::fs::File>,
    /// pdt-mcp's URL and Authorization header, from `--mcp-config`.
    mcp: Option<(String, String)>,
    /// `--allowedTools`.
    allowed: Vec<String>,
    /// `--tools` (None: all of Claude's built-in tools).
    tools: Option<String>,
}

/// pdt-mcp from Claude's `--mcp-config` file.
fn mcp_server(file: Option<String>) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file?).ok()?).ok()?;
    let s = &v["mcpServers"]["pdt"];
    Some((s["url"].as_str()?.to_string(), s["headers"]["Authorization"].as_str()?.to_string()))
}

/// One JSON-RPC call over plain HTTP/1.1 (pdt-mcp answers with JSON). Err: what went wrong.
fn rpc(server: &(String, String), method: &str, params: Value) -> Result<Value, String> {
    use std::io::Read;
    let rest = server.0.strip_prefix("http://").ok_or("not an http URL")?;
    let (host, path) = rest.split_once('/').ok_or("no path")?;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let mut s = std::net::TcpStream::connect(host).map_err(|e| e.to_string())?;
    let req = format!(
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: {}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        server.1,
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    let (head, body) = raw.split_once("\r\n\r\n").ok_or("no body")?;
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(format!("HTTP {status}"));
    }
    let v: Value = serde_json::from_str(body.trim()).map_err(|e| e.to_string())?;
    if let Some(e) = v.get("error") {
        return Err(e["message"].as_str().unwrap_or("error").to_string());
    }
    Ok(v["result"].clone())
}

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

impl Agent {
    fn next_id(&mut self, p: &str) -> String {
        self.msg += 1;
        format!("{p}_{}_{}", self.msg, std::process::id())
    }

    fn init(&self) {
        out(json!({"type":"system","subtype":"init","cwd":std::env::current_dir().unwrap_or_default(),
            "session_id":self.session,"tools":["Bash","AskUserQuestion"],"model":self.model,
            "permissionMode":self.mode,"slash_commands":["compact"],"apiKeySource":"none",
            "claude_code_version":"2.1.302-fake","uuid":"init"}));
    }

    /// Handles a control request; returns false for messages that are not control requests.
    fn control(&mut self, v: &Value) -> Option<bool> {
        if v["type"] != "control_request" {
            return None;
        }
        let id = v["request_id"].clone();
        let req = &v["request"];
        let ok = |resp: Value| json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":resp}});
        let mut interrupt = false;
        match req["subtype"].as_str().unwrap_or("") {
            "initialize" => out(ok(json!({
                "commands":[{"name":"compact","description":"Compact the conversation","argumentHint":""}],
                "models":[{"value":"default","displayName":"Default"},{"value":"opus","displayName":"Opus"}],
                "account":{"subscriptionType":"fake"},"current_permission_mode":self.mode}))),
            "interrupt" => {
                interrupt = true;
                out(ok(json!({})));
            }
            "set_model" => {
                self.model = req["model"].as_str().unwrap_or("default").to_string();
                out(ok(json!({})));
            }
            "set_permission_mode" => {
                self.mode = req["mode"].as_str().unwrap_or("default").to_string();
                out(ok(json!({})));
            }
            other => out(json!({"type":"control_response","response":{"subtype":"error","request_id":id,
                "error":format!("Unsupported control request subtype: {other}")}})),
        }
        Some(interrupt)
    }

    /// Waits for the next stdin message. None: stdin closed.
    fn recv(&mut self) -> Option<Value> {
        let v = self.rx.recv().ok()??;
        if let Some(f) = self.log.as_mut() {
            let _ = writeln!(f, "{v}");
        }
        Some(v)
    }

    /// Waits for the answer to a control request we sent, handling other control requests meanwhile and queueing
    /// user messages. None: interrupted or stdin closed.
    fn await_response(&mut self, request_id: &str) -> Option<Value> {
        loop {
            let v = self.recv()?;
            if v["type"] == "control_response" && v["response"]["request_id"] == request_id {
                return Some(v["response"]["response"].clone());
            }
            match self.control(&v) {
                Some(true) => return None,
                Some(false) => {}
                None => self.queue.push(v),
            }
        }
    }

    fn replay(&mut self, v: &Value) {
        let mut r = v.clone();
        r["uuid"] = json!(self.next_id("u"));
        r["isReplay"] = json!(true);
        r["session_id"] = json!(self.session);
        out(r);
    }

    fn say(&mut self, text: &str) {
        let id = self.next_id("msg");
        let mid = text.len() / 2;
        let cut = (0..=mid).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        for part in [&text[..cut], &text[cut..]] {
            if part.is_empty() {
                continue;
            }
            out(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,
                "delta":{"type":"text_delta","text":part}},"parent_tool_use_id":null,"api_message_id":id}));
        }
        out(json!({"type":"assistant","message":{"id":id,"model":self.model,"role":"assistant",
            "content":[{"type":"text","text":text}]},"parent_tool_use_id":null,"session_id":self.session}));
    }

    /// A background task finished: Claude tells itself, which starts a turn.
    fn task_done(&mut self, v: &Value) {
        let started = Instant::now();
        let task = v["task"].as_str().unwrap_or("");
        out(json!({"type":"system","subtype":"task_updated","task_id":task,"patch":{"status":"completed"}}));
        out(json!({"type":"system","subtype":"background_tasks_changed","tasks":[],"session_id":self.session}));
        if v["quiet"] == true {
            return;
        }
        self.init();
        let note = format!(
            "<task-notification>\n<task-id>{task}</task-id>\n<status>completed</status>\n<summary>\
            Background command &quot;sleep&quot; completed (exit code 0)</summary>\n</task-notification>"
        );
        out(json!({"type":"user","message":{"role":"user","content":note},"session_id":self.session,
            "parent_tool_use_id":null,"uuid":self.next_id("u"),"isReplay":true,"origin":{"kind":"task-notification"}}));
        self.say("The background task finished.");
        self.result("success", started);
    }

    fn result(&self, subtype: &str, started: Instant) {
        let is_error = subtype != "success";
        out(json!({"type":"result","subtype":subtype,"is_error":is_error,
            "duration_ms":started.elapsed().as_millis() as u64,"total_cost_usd":0.01,
            "usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":0,"output_tokens":5},
            "modelUsage":{"fake":{"contextWindow":200000}},"session_id":self.session,"result":"ok"}));
    }

    fn tool(&mut self, name: &str, input: Value) -> Result<(String, Value), ()> {
        let tool_id = self.next_id("toolu");
        let msg = self.next_id("msg");
        out(json!({"type":"assistant","message":{"id":msg,"content":[{"type":"tool_use","id":tool_id,"name":name,
            "input":input}]},"parent_tool_use_id":null,"session_id":self.session}));
        if self.mode == "bypassPermissions" {
            return Ok((tool_id, json!({"behavior":"allow","updatedInput":input})));
        }
        let req = self.next_id("req");
        out(json!({"type":"control_request","request_id":req,"request":{"subtype":"can_use_tool","tool_name":name,
            "display_name":name,"input":input,"tool_use_id":tool_id,
            "permission_suggestions":[{"type":"addRules","rules":[{"toolName":name,"ruleContent":"*"}],
                "behavior":"allow","destination":"localSettings"}]}}));
        match self.await_response(&req) {
            Some(r) => Ok((tool_id, r)),
            None => {
                // Claude cancels the open prompt when interrupted.
                out(json!({"type":"control_cancel_request","request_id":req}));
                Err(())
            }
        }
    }

    fn tool_result(&self, tool_id: &str, text: &str, is_error: bool) {
        out(json!({"type":"user","message":{"role":"user","content":[{"tool_use_id":tool_id,"type":"tool_result",
            "content":text,"is_error":is_error}]},"parent_tool_use_id":null,"session_id":self.session}));
    }

    fn turn(&mut self, v: Value) {
        let started = Instant::now();
        if v["message"]["content"].as_str().map(str::trim) == Some("/clear") {
            let uuid = |n: u128| {
                let h = format!("{n:032x}");
                format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
            };
            let nanos =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
            let (reset_id, new_session) = (uuid(nanos ^ 0xaaaa), uuid(nanos));
            out(json!({"type":"conversation_reset","trigger":"clear","new_conversation_id":reset_id,
                "uuid":reset_id,"session_id":self.session}));
            self.session = new_session;
            self.init();
            out(json!({"type":"result","subtype":"success","is_error":false,"duration_ms":5,"num_turns":0,
                "session_id":self.session,"result":"","local_command":"clear"}));
            return;
        }
        self.init();
        let images: Vec<String> = v["message"]["content"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|b| b["type"] == "image" && b["source"]["data"].as_str().is_some_and(|d| !d.is_empty()))
                    .map(|b| b["source"]["media_type"].as_str().unwrap_or("?").to_string())
                    .collect()
            })
            .unwrap_or_default();
        if v["message"]["content"].as_str().map(str::trim) == Some("/local") {
            self.say("local command output");
            return self.result("success", started);
        }
        self.replay(&v);
        if !images.is_empty() {
            self.say(&format!("saw {} images ({})", images.len(), images.join(", ")));
            return self.result("success", started);
        }
        let text = v["message"]["content"].as_str().map(String::from).unwrap_or_else(|| {
            v["message"]["content"]
                .as_array()
                .and_then(|a| a.iter().find_map(|b| b["text"].as_str()))
                .unwrap_or("")
                .to_string()
        });
        let text = text.trim();
        if let Some(cmd) = text.strip_prefix("run:") {
            let cmd = cmd.trim();
            let Ok((tool_id, resp)) = self.tool("Bash", json!({"command": cmd, "description": "Run it"})) else {
                return self.result("error_during_execution", started);
            };
            if resp["behavior"] == "allow" {
                let always = resp.get("updatedPermissions").map(|p| p.to_string()).unwrap_or_default();
                self.tool_result(&tool_id, &format!("ran {cmd}"), false);
                self.say(&format!(
                    "Ran it.{}",
                    if always.is_empty() { String::new() } else { format!(" rules={always}") }
                ));
            } else {
                let why = resp["message"].as_str().unwrap_or("").to_string();
                self.tool_result(&tool_id, &format!("The user doesn't want to proceed: {why}"), true);
                self.say(&format!("Not running it ({why})."));
            }
        } else if let Some(q) = text.strip_prefix("ask:") {
            let input = json!({"questions":[{"question":q.trim(),"header":"Q","multiSelect":false,
                "options":[{"label":"A","description":"a"},{"label":"B","description":"b"}]}]});
            let Ok((tool_id, resp)) = self.tool("AskUserQuestion", input) else {
                return self.result("error_during_execution", started);
            };
            let answers = resp["updatedInput"]["answers"].to_string();
            self.tool_result(&tool_id, &answers, false);
            self.say(&format!("You answered {answers}"));
        } else if text == "slow" {
            let id = self.next_id("msg");
            let end = Instant::now() + Duration::from_secs(20);
            while Instant::now() < end {
                out(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,
                    "delta":{"type":"text_delta","text":"."}},"parent_tool_use_id":null,"api_message_id":id}));
                match self.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(m)) => {
                        if let Some(f) = self.log.as_mut() {
                            let _ = writeln!(f, "{m}");
                        }
                        match self.control(&m) {
                            Some(true) => {
                                out(json!({"type":"user","message":{"role":"user","content":[{"type":"text",
                                    "text":"[Request interrupted by user]"}]},"session_id":self.session}));
                                return self.result("error_during_execution", started);
                            }
                            Some(false) => {}
                            None if m["priority"] == "now" => {
                                self.result("error_during_execution", started);
                                return self.turn(m);
                            }
                            None => self.queue.push(m),
                        }
                    }
                    Ok(None) => std::process::exit(0),
                    Err(_) => {}
                }
            }
            self.say("Finished the slow job.");
        } else if let Some(call) = text.lines().next().and_then(|l| l.strip_prefix("mcp:")) {
            let call = call.trim();
            let (tool, args) = call.split_once(' ').unwrap_or((call, "{}"));
            let full = format!("mcp__pdt__{tool}");
            // Claude asks before a tool the server lists that `--allowedTools` leaves out.
            let listed = self.mcp.as_ref().is_some_and(|server| {
                rpc(server, "tools/list", json!({}))
                    .is_ok_and(|r| r["tools"].as_array().is_some_and(|a| a.iter().any(|t| t["name"] == tool)))
            });
            if listed && self.mode != "bypassPermissions" && !self.allowed.contains(&full) {
                let input: Value = serde_json::from_str(args.trim()).unwrap_or(json!({}));
                let Ok((tool_id, resp)) = self.tool(&full, input) else {
                    return self.result("error_during_execution", started);
                };
                if resp["behavior"] != "allow" {
                    self.tool_result(&tool_id, "denied", true);
                    self.say(&format!("mcp {tool} denied"));
                    return self.result("success", started);
                }
                self.tool_result(&tool_id, "allowed", false);
            }
            let reply = match &self.mcp {
                None => "mcp error: no pdt-mcp server".to_string(),
                Some(server) => {
                    let args: Value = serde_json::from_str(args.trim()).unwrap_or(json!({}));
                    match rpc(server, "tools/call", json!({"name": tool, "arguments": args})) {
                        Ok(r) => format!(
                            "mcp {tool}{}: {}",
                            if r["isError"] == true { " failed" } else { "" },
                            r["content"][0]["text"].as_str().unwrap_or("")
                        ),
                        Err(e) => format!("mcp error: {e}"),
                    }
                }
            };
            self.say(&reply);
        } else if text == "mcp-list" {
            let reply = match &self.mcp {
                None => "mcp error: no pdt-mcp server".to_string(),
                Some(server) => match rpc(server, "tools/list", json!({})) {
                    Ok(r) => {
                        let names: Vec<&str> = r["tools"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|t| t["name"].as_str()).collect())
                            .unwrap_or_default();
                        format!("mcp tools: {}", names.join(","))
                    }
                    Err(e) => format!("mcp error: {e}"),
                },
            };
            self.say(&reply);
        } else if let Some((quiet, ms)) = text.lines().next().and_then(|l| {
            l.strip_prefix("bg-quiet:").map(|ms| (true, ms)).or_else(|| l.strip_prefix("bg:").map(|ms| (false, ms)))
        }) {
            let task = self.next_id("task");
            out(json!({"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":task,
                "task_type":"local_bash","description":"Sleep in the background"}],"session_id":self.session}));
            let (tx, ms) = (self.tx.clone(), ms.trim().parse().unwrap_or(1000));
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                let _ = tx.send(Some(json!({"type":"fake_task_done","task":task,"quiet":quiet})));
            });
            self.say("Started it in the background.");
        } else if let Some(ms) = text.lines().next().and_then(|l| l.strip_prefix("compact:")) {
            // Claude compacting its context mid-turn, then working on for a while (messages sent meanwhile wait).
            out(json!({"type":"system","subtype":"compact_boundary","session_id":self.session,
                "compact_metadata":{"trigger":"auto","pre_tokens":150000}}));
            std::thread::sleep(Duration::from_millis(ms.trim().parse().unwrap_or(500)));
            self.say("Compacted and carried on.");
        } else if text == "deaf" {
            self.say("Not listening any more.");
            self.result("success", started);
            // Alive, but never reads another message (nothing sent now is echoed).
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        } else if text == "exit" {
            eprintln!("fake-agent: crashing as asked");
            std::process::exit(3);
        } else if let Some(name) = text.strip_prefix("env:") {
            let name = name.trim();
            let set = std::env::var_os(name).is_some();
            self.say(&format!("{name} is {}", if set { "set" } else { "unset" }));
        } else if text == "where" {
            let cwd = std::env::current_dir().unwrap_or_default();
            self.say(&format!("cwd={} tools={:?}", cwd.display(), self.tools.as_deref().unwrap_or("all")));
        } else if text == "who" {
            let s = format!(
                "session {} resumed={} model={} mode={} instructions={} msg_session={}",
                self.session,
                self.resumed,
                self.model,
                self.mode,
                self.instructions,
                v["session_id"].as_str().unwrap_or("")
            );
            self.say(&s);
        } else {
            self.say(&format!("echo: {text}"));
        }
        self.result("success", started);
    }
}

/// A side request: one `--output-format json` result, nothing saved.
fn draft(args: &[String], session: &str, resumed: bool) {
    let mut prompt = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut prompt);
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let has = |name: &str| args.iter().any(|a| a == name);
    let schema = flag("--json-schema").and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let description = format!(
        "resumed={resumed} tools={:?} saved={} schema={} prompt={}",
        flag("--tools").unwrap_or_default(),
        !has("--no-session-persistence"),
        schema.is_some(),
        prompt.split_whitespace().take(3).collect::<Vec<_>>().join(" ")
    );
    out(json!({"type":"result","subtype":"success","is_error":false,"session_id":"fork","result":"",
        "structured_output":{"title":format!("Draft of {session}"),"description":description}}));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("app-server") => return codex::serve(&args),
        Some("resume") => return codex::tui(&args),
        _ if args.iter().any(|a| a == "acp") => return acp::serve(&args),
        _ => {}
    }
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let mut log = std::env::var_os("FAKE_AGENT_LOG")
        .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "argv {}", serde_json::to_string(&args[1..]).unwrap_or_default());
    }
    let (resumed, session) = match (flag("--resume"), flag("--session-id")) {
        (Some(s), _) => (true, s),
        (None, Some(s)) => (false, s),
        _ => (false, "none".into()),
    };
    if args.iter().any(|a| a == "--fork-session") {
        return draft(&args, &session, resumed);
    }
    let (tx, rx) = mpsc::channel();
    let stdin_tx = tx.clone();
    std::thread::spawn(move || {
        let tx = stdin_tx;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            // Typed into the TUI stand-in (no `-p`): leave like Claude's TUI does.
            if line.trim() == "/exit" {
                break;
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(Some(v));
            }
        }
        let _ = tx.send(None);
    });
    let mut agent = Agent {
        session,
        resumed,
        model: flag("--model").unwrap_or_else(|| "default".into()),
        mode: flag("--permission-mode").unwrap_or_else(|| "default".into()),
        instructions: flag("--append-system-prompt-file")
            .and_then(|f| std::fs::read_to_string(f).ok())
            .is_some_and(|t| t.contains("Running in") || t.contains("# Briefing")),
        msg: 0,
        queue: Vec::new(),
        rx,
        tx,
        log,
        mcp: mcp_server(flag("--mcp-config")),
        allowed: flag("--allowedTools").map(|t| t.split(',').map(String::from).collect()).unwrap_or_default(),
        tools: flag("--tools"),
    };
    loop {
        let v = if agent.queue.is_empty() {
            match agent.recv() {
                Some(v) => v,
                None => return,
            }
        } else {
            agent.queue.remove(0)
        };
        if agent.control(&v).is_some() {
            continue;
        }
        if v["type"] == "user" {
            agent.turn(v);
        } else if v["type"] == "fake_task_done" {
            agent.task_done(&v);
        }
    }
}
