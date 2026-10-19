//! A stand-in for `claude -p` stream-json, for tests (spec §12): frame shapes copied from recorded runs. Never
//! shipped. What a user message does:
//! - `run: <cmd>` — asks to run `<cmd>` with Bash (a `can_use_tool` request), then reports it ran or was denied.
//! - `ask: <question>` — asks an AskUserQuestion with options A and B, then says the answer.
//! - `slow` — streams text for ~20 s unless interrupted (messages sent meanwhile wait for the next turn).
//! - `exit` — writes to stderr and exits 3.
//! - `env: NAME` — says whether NAME is set.
//! - `who` — says the session id, whether it was resumed, its model and mode, and whether it got PDT's instructions.
//! - `/local` — like Claude's local commands (`/context`): answers without echoing the message first.
//! - a message with images — says `saw N images (<media types>)`.
//! - anything else — streams `echo: <text>` back.
//!
//! Started without `-p` (Claude's TUI, in a terminal) it reads lines and leaves on `/exit`.
//!
//! `FAKE_AGENT_LOG` (a file path), when set, gets every stdin line and the argv.
//!
//! `fake-agent app-server …` plays `codex app-server` instead (see `codex.rs`), and `fake-agent resume …` the Codex
//! TUI attached to it.

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
    log: Option<std::fs::File>,
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
        } else if text == "exit" {
            eprintln!("fake-agent: crashing as asked");
            std::process::exit(3);
        } else if let Some(name) = text.strip_prefix("env:") {
            let name = name.trim();
            let set = std::env::var_os(name).is_some();
            self.say(&format!("{name} is {}", if set { "set" } else { "unset" }));
        } else if text == "who" {
            let s = format!(
                "session {} resumed={} model={} mode={} instructions={}",
                self.session, self.resumed, self.model, self.mode, self.instructions
            );
            self.say(&s);
        } else {
            self.say(&format!("echo: {text}"));
        }
        self.result("success", started);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("app-server") => return codex::serve(&args),
        Some("resume") => return codex::tui(&args),
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
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
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
            .is_some_and(|t| t.contains("Running in")),
        msg: 0,
        queue: Vec::new(),
        rx,
        log,
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
        }
    }
}
