//! `fake-agent [--config <file>] acp`: a stand-in for `devin acp` (Agent Client Protocol, JSON-RPC on stdio), frame
//! shapes copied from recorded Devin runs (crates/pdt-core/tests/fixtures/devin). What a prompt does:
//! - `run: <cmd>` — a shell tool call that asks permission (`session/request_permission`), then runs or is rejected.
//! - `ask: <question>` — asks with `elicitation/create` (options A and B), then says the answer.
//! - `edit` — edits `f.txt` in the folder (a `diff` tool call).
//! - `slow` — streams text for ~20 s unless cancelled; prompts sent meanwhile join the turn.
//! - `exit` — writes to stderr and exits 3.
//! - `who` — says the session id, whether it was loaded, its mode and model, and whether `--config` carries PDT's
//!   instructions hook.
//! - `where` — says the session's working folder.
//! - `servers` — says the names of the session's `mcpServers` (`servers=pdt,slack`; it takes HTTP ones, not SSE).
//! - `mcp: <tool> <json arguments>` (its first line) — asks permission for that pdt-mcp tool (from the session's `mcpServers`), then
//!   calls it and says `mcp <tool>: <the result's text>` (or `mcp <tool> denied`, `mcp error: …`).
//! - a prompt with images — says `saw N images (<media types>)`.
//! - anything else — `echo: <text>`.
//!
//! `--plain` makes it a plain ACP agent: no `_meta` or error `data` (Devin's `cognition.ai/*` fields) and no
//! session-only permission option. `--modes` makes `session/new` and `session/load` advertise its modes
//! (`availableModes`); `--partial-modes` advertises only `build` (none of PDT's) and `plan`, starting in `build`, as
//! OpenCode does. A prompt's last text block is what it does; text blocks before it are context (PDT's
//! instructions for an ACP agent, which `who` then reports).
//!
//! `fake-agent agent --no-leader stdio` stands in for Grok Build (PDT-236; shapes from crates/pdt-core/tests/fixtures/
//! grok): tool names in `_meta["x.ai/tool"]` (pdt-mcp's tools through `use_tool` as `pdt__<tool>`), Grok's option ids
//! (`allow-once`, `reject-once`, and `allow-edits-session` on an edit), `_meta.toolNotRun` for a refusal, no modes,
//! and `edit` asks before it edits.
//!
//! A session is saved at its first prompt (a marker file in the temp folder), as Devin does: loading one never
//! prompted fails with -32016. Loading replays its history first, as Devin does.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// `--plain`: no Devin fields.
static PLAIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--modes`: advertise `availableModes`.
static MODES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--partial-modes`: advertise only `build` and `plan`.
static PARTIAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `agent --no-leader stdio`: Grok's fields instead of Devin's.
static GROK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A Devin-shaped message in Grok's terms.
fn grok(v: &mut Value) {
    if let Some(u) = v.pointer_mut("/params/update").and_then(Value::as_object_mut)
        && let Some(meta) = u.remove("_meta")
    {
        if let Some(name) = meta["cognition.ai/inferenceToolName"].as_str() {
            let tool = match name.strip_prefix("mcp__pdt__") {
                Some(t) => {
                    let input = u.get("rawInput").cloned().unwrap_or_default();
                    u.insert("rawInput".into(), json!({"tool_name": format!("pdt__{t}"), "tool_input": input}));
                    "use_tool".to_string()
                }
                None => name.to_string(),
            };
            u.insert("_meta".into(), json!({"x.ai/tool": {"name": tool}}));
        }
        if meta["cognition.ai/rejected"] == true {
            u.insert("_meta".into(), json!({"toolNotRun": "rejected"}));
        }
    }
    if let Some(e) = v.get_mut("error").and_then(Value::as_object_mut) {
        e.remove("data");
    }
    if let Some(o) = v.pointer_mut("/params/options").and_then(Value::as_array_mut) {
        o.retain(|o| o["optionId"] == "allow_once" || o["optionId"] == "reject_once" || o["optionId"] == "allow_edits");
        for o in o.iter_mut() {
            let id = match o["optionId"].as_str() {
                Some("allow_edits") => "allow-edits-session",
                Some(id) => &id.replace('_', "-"),
                None => continue,
            };
            o["optionId"] = json!(id);
        }
    }
}

fn out(mut v: Value) {
    if GROK.load(std::sync::atomic::Ordering::Relaxed) {
        grok(&mut v);
    }
    if PLAIN.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(u) = v.pointer_mut("/params/update").and_then(Value::as_object_mut) {
            u.remove("_meta");
        }
        if let Some(e) = v.get_mut("error").and_then(Value::as_object_mut) {
            e.remove("data");
        }
        if let Some(o) = v.pointer_mut("/params/options").and_then(Value::as_array_mut) {
            o.retain(|o| o["optionId"] != "allow_session");
        }
    }
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

/// A session's saved history: one `{"text", "atMs"}` line per prompt (tests add lines to stand in for turns taken in
/// Devin's terminal). It exists once the session was prompted.
fn saved_marker(id: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("fake-acp-saved-{id}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// ms since the epoch → RFC 3339 UTC, as Devin stamps messages (Howard Hinnant's civil-from-days).
fn rfc3339(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, sec, milli) = (rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{sec:02}.{milli:03}000+00:00")
}

/// pdt-mcp from `session/new`'s or `session/load`'s `mcpServers`: its URL and Authorization header.
fn mcp_server(p: &Value) -> Option<(String, String)> {
    let s = p["mcpServers"].as_array()?.iter().find(|s| s["name"] == "pdt")?;
    let auth = s["headers"].as_array()?.iter().find(|h| h["name"] == "Authorization")?;
    Some((s["url"].as_str()?.to_string(), auth["value"].as_str()?.to_string()))
}

fn server_names(p: &Value) -> Vec<String> {
    p["mcpServers"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str().map(String::from)).collect()
}

struct Acp {
    rx: mpsc::Receiver<Option<Value>>,
    queue: Vec<Value>,
    session: String,
    loaded: bool,
    mode: String,
    model: String,
    hook: bool,
    /// The `cwd` of `session/new` or `session/load`.
    cwd: String,
    /// pdt-mcp's URL and Authorization header, from the session's `mcpServers`.
    mcp: Option<(String, String)>,
    /// The names of the session's `mcpServers`.
    servers: Vec<String>,
    next: u64,
    cancelled: bool,
    /// Prompts sent while a turn ran: answered with it.
    joined: Vec<Value>,
}

impl Acp {
    fn recv(&mut self) -> Option<Value> {
        self.rx.recv().ok().flatten()
    }

    fn update(&self, u: Value) {
        out(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.session, "update": u}}));
    }

    fn config(&self) -> Value {
        json!([
            {"id": "mode", "name": "Session Mode", "category": "mode", "type": "select", "currentValue": self.mode,
             "options": [{"value": "accept-edits", "name": "Code"}, {"value": "plan", "name": "Plan"},
                         {"value": "bypass", "name": "Bypass Permissions"}]},
            {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": self.model,
             "options": [{"value": "fake-1", "name": "Fake One"}, {"value": "fake-2", "name": "Fake Two"}]}
        ])
    }

    /// `session/new`'s or `session/load`'s answer.
    fn opened(&self, mut r: Value) -> Value {
        if MODES.load(std::sync::atomic::Ordering::Relaxed) {
            r["modes"] = json!({"currentModeId": self.mode, "availableModes": [
                {"id": "ask", "name": "Ask"}, {"id": "accept-edits", "name": "Code"},
                {"id": "plan", "name": "Plan"}, {"id": "bypass", "name": "Bypass Permissions"}]});
        }
        if PARTIAL.load(std::sync::atomic::Ordering::Relaxed) {
            r["modes"] = json!({"currentModeId": self.mode, "availableModes": [
                {"id": "build", "name": "Build"}, {"id": "plan", "name": "Plan"}]});
        }
        r
    }

    fn announce(&self) {
        self.update(json!({"sessionUpdate": "config_option_update", "configOptions": self.config()}));
        self.update(json!({"sessionUpdate": "current_mode_update", "currentModeId": self.mode}));
        self.update(json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "compact", "description": "Force conversation compaction"}]}));
    }

    fn say(&self, text: &str) {
        for part in text.split_inclusive(' ') {
            self.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": part}}));
        }
    }

    /// Waits for the answer to an agent request, keeping other messages for later (a cancel is noted).
    fn wait_answer(&mut self, id: &str) -> Option<Value> {
        loop {
            let v = self.recv()?;
            if v["id"] == id && v.get("method").is_none() {
                return Some(v);
            }
            self.other(v);
        }
    }

    /// A message that arrived mid-turn.
    fn other(&mut self, v: Value) {
        match v["method"].as_str() {
            Some("session/cancel") => self.cancelled = true,
            Some("session/prompt") => self.joined.push(v["id"].clone()),
            _ => self.queue.push(v),
        }
    }

    fn tool(&self, id: &str, title: &str, kind: &str, name: &str, input: Value, content: Value) {
        self.update(json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": title, "kind": kind,
            "content": content, "rawInput": input, "_meta": {"cognition.ai/inferenceToolName": name}}));
    }

    fn prompt(&mut self, id: Value, p: &Value) {
        let blocks = p["prompt"].as_array().cloned().unwrap_or_default();
        let texts: Vec<&str> =
            blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
        let text = texts.last().copied().unwrap_or("").to_string();
        if texts.len() > 1 {
            self.hook = true;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(saved_marker(&self.session)) {
            let _ = writeln!(f, "{}", json!({"text": text, "atMs": now_ms()}));
        }
        let images: Vec<String> = blocks
            .iter()
            .filter(|b| b["type"] == "image")
            .filter_map(|b| b["mimeType"].as_str().map(String::from))
            .collect();
        self.cancelled = false;
        self.joined.clear();
        self.update(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Thinking."}}));
        self.next += 1;
        let call = format!("call_{}_{}", self.next, super::run_tag());
        if !images.is_empty() {
            self.say(&format!("saw {} images ({})", images.len(), images.join(", ")));
        } else if let Some(cmd) = text.strip_prefix("run: ") {
            self.tool(&call, &format!("Ran {cmd}"), "execute", "exec", json!({"command": cmd}), json!([]));
            let rid = format!("perm-{}", self.next);
            out(json!({"jsonrpc": "2.0", "id": rid, "method": "session/request_permission", "params": {
                "sessionId": self.session, "toolCall": {"toolCallId": call}, "options": [
                    {"optionId": "allow_once", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "allow_session", "name": "Yes, allow (this session)", "kind": "allow_always"},
                    {"optionId": "allow_always", "name": "Yes, always allow in `repo`", "kind": "allow_always"},
                    {"optionId": "reject_once", "name": "Reject", "kind": "reject_once"}]}}));
            let Some(a) = self.wait_answer(&rid) else { return };
            let picked = a["result"]["outcome"]["optionId"].as_str().unwrap_or("cancelled").to_string();
            if picked.starts_with("allow") {
                self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": format!("ran {cmd}")}}],
                    "_meta": {"terminal_exit": {"exit_code": 0}}}));
                self.say(&format!("ran it ({picked})"));
            } else {
                self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed",
                    "_meta": {"cognition.ai/rejected": picked.starts_with("reject")}}));
                self.say(&format!("not run ({picked})"));
            }
        } else if let Some(q) = text.strip_prefix("ask: ") {
            let rid = format!("ask-{}", self.next);
            out(json!({"jsonrpc": "2.0", "id": rid, "method": "elicitation/create", "params": {
                "mode": "form", "sessionId": self.session, "message": q, "requestedSchema": {"type": "object",
                "properties": {"q0": {"type": "string", "title": "Pick", "description": q,
                    "oneOf": [{"const": "A", "title": "The first"}, {"const": "B", "title": "The second"}]}},
                "required": ["q0"]}}}));
            let Some(a) = self.wait_answer(&rid) else { return };
            match a["result"]["action"].as_str() {
                Some("accept") => {
                    self.say(&format!("you chose {}", a["result"]["content"]["q0"].as_str().unwrap_or("?")))
                }
                other => self.say(&format!("no answer ({})", other.unwrap_or("?"))),
            }
        } else if text == "edit" {
            let path = std::env::current_dir().unwrap_or_default().join("f.txt");
            let path = path.to_string_lossy().into_owned();
            self.tool(
                &call,
                "Edit file",
                "edit",
                "edit",
                json!({"file_path": path}),
                json!([{"type": "diff", "path": path, "oldText": "a\n", "newText": "b\n"}]),
            );
            if GROK.load(std::sync::atomic::Ordering::Relaxed) {
                let rid = format!("perm-{}", self.next);
                out(json!({"jsonrpc": "2.0", "id": rid, "method": "session/request_permission", "params": {
                    "sessionId": self.session, "toolCall": {"toolCallId": call, "kind": "edit"}, "options": [
                        {"optionId": "allow_edits", "name": "Yes, allow all edits during this session",
                         "kind": "allow_always"},
                        {"optionId": "allow_once", "name": "Yes", "kind": "allow_once"},
                        {"optionId": "reject_once", "name": "No", "kind": "reject_once"}]}}));
                let Some(a) = self.wait_answer(&rid) else { return };
                let picked = a["result"]["outcome"]["optionId"].as_str().unwrap_or("cancelled").to_string();
                if !picked.starts_with("allow") {
                    self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed",
                        "_meta": {"cognition.ai/rejected": true}}));
                    self.say(&format!("not edited ({picked})"));
                    return self.end_turn(id);
                }
            }
            self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed"}));
            self.say("edited");
        } else if text == "slow" {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(20) && !self.cancelled {
                self.say("tick ");
                if let Ok(Some(v)) = self.rx.recv_timeout(Duration::from_millis(100)) {
                    self.other(v);
                }
            }
        } else if text == "exit" {
            eprintln!("fake devin: exiting");
            std::process::exit(3);
        } else if let Some(wanted) = text.lines().next().and_then(|l| l.strip_prefix("mcp:")) {
            let wanted = wanted.trim();
            let (tool, args) = wanted.split_once(' ').unwrap_or((wanted, "{}"));
            let args: Value = serde_json::from_str(args.trim()).unwrap_or(json!({}));
            let name = format!("mcp__pdt__{tool}");
            self.tool(&call, &name, "other", &name, args.clone(), json!([]));
            let rid = format!("perm-{}", self.next);
            out(json!({"jsonrpc": "2.0", "id": rid, "method": "session/request_permission", "params": {
                "sessionId": self.session, "toolCall": {"toolCallId": call}, "options": [
                    {"optionId": "allow_once", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "reject_once", "name": "Reject", "kind": "reject_once"}]}}));
            let Some(a) = self.wait_answer(&rid) else { return };
            let reply = if !a["result"]["outcome"]["optionId"].as_str().is_some_and(|o| o.starts_with("allow")) {
                format!("mcp {tool} denied")
            } else {
                match &self.mcp {
                    None => "mcp error: no pdt-mcp server".to_string(),
                    Some(server) => match super::rpc(server, "tools/call", json!({"name": tool, "arguments": args})) {
                        Ok(r) => format!(
                            "mcp {tool}{}: {}",
                            if r["isError"] == true { " failed" } else { "" },
                            r["content"][0]["text"].as_str().unwrap_or("")
                        ),
                        Err(e) => format!("mcp error: {e}"),
                    },
                }
            };
            self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call,
                "status": "completed"}));
            self.say(&reply);
        } else if text == "where" {
            self.say(&format!("cwd={}", self.cwd));
        } else if text == "servers" {
            self.say(&format!("servers={}", self.servers.join(",")));
        } else if text == "who" {
            self.say(&format!(
                "session={} loaded={} mode={} model={} instructions={}",
                self.session, self.loaded, self.mode, self.model, self.hook
            ));
        } else {
            self.say(&format!("echo: {text}"));
        }
        self.end_turn(id);
    }

    fn end_turn(&mut self, id: Value) {
        self.update(json!({"sessionUpdate": "usage_update", "used": 1200, "size": 200000}));
        let stop = if self.cancelled { "cancelled" } else { "end_turn" };
        for pid in std::iter::once(id).chain(std::mem::take(&mut self.joined)) {
            out(json!({"jsonrpc": "2.0", "id": pid, "result": {"stopReason": stop}}));
        }
    }
}

pub fn serve(args: &[String]) {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    PLAIN.store(args.iter().any(|a| a == "--plain"), std::sync::atomic::Ordering::Relaxed);
    GROK.store(args.get(1).is_some_and(|a| a == "agent"), std::sync::atomic::Ordering::Relaxed);
    MODES.store(args.iter().any(|a| a == "--modes"), std::sync::atomic::Ordering::Relaxed);
    let partial = args.iter().any(|a| a == "--partial-modes");
    PARTIAL.store(partial, std::sync::atomic::Ordering::Relaxed);
    let hook = flag("--config")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .is_some_and(|t| t.contains("hook devin --instructions"));
    let mut log = std::env::var_os("FAKE_AGENT_LOG")
        .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "argv {}", serde_json::to_string(&args[1..]).unwrap_or_default());
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Some(f) = log.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(Some(v));
            }
        }
        let _ = tx.send(None);
    });
    let mut a = Acp {
        rx,
        queue: Vec::new(),
        session: String::new(),
        loaded: false,
        mode: if partial { "build" } else { "accept-edits" }.into(),
        model: "fake-1".into(),
        hook,
        cwd: String::new(),
        mcp: None,
        servers: Vec::new(),
        next: 0,
        cancelled: false,
        joined: Vec::new(),
    };
    loop {
        let v = if a.queue.is_empty() {
            match a.recv() {
                Some(v) => v,
                None => return,
            }
        } else {
            a.queue.remove(0)
        };
        let id = v["id"].clone();
        let p = v["params"].clone();
        let ok = |r: Value| out(json!({"jsonrpc": "2.0", "id": id, "result": r}));
        match v["method"].as_str().unwrap_or("") {
            "initialize" => ok(json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true,
                "promptCapabilities": {"image": true}, "mcpCapabilities": {"http": true, "sse": false}}, "agentInfo": {"name": "fake", "version": "0.0.0-fake"}})),
            // Sign in (PDT-239): `FAKE_ACP_AUTH` names a file the method goes in.
            "authenticate" => {
                if let Ok(path) = std::env::var("FAKE_ACP_AUTH") {
                    let _ = std::fs::write(path, p["methodId"].as_str().unwrap_or(""));
                }
                ok(json!({}));
            }
            "session/new" if std::env::var("FAKE_ACP_AUTH").is_ok_and(|f| !std::path::Path::new(&f).exists()) => {
                out(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "Authentication required",
                    "data": {"message": "No authentication method selected."}}}));
            }
            "session/new" => {
                a.cwd = p["cwd"].as_str().unwrap_or("?").to_string();
                a.mcp = mcp_server(&p);
                a.servers = server_names(&p);
                a.session = format!("fake-{}-{}", super::run_tag(), a.next);
                a.announce();
                ok(a.opened(json!({"sessionId": a.session, "configOptions": a.config()})));
            }
            "session/load" => {
                let sid = p["sessionId"].as_str().unwrap_or("").to_string();
                if !saved_marker(&sid).exists() {
                    out(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32016, "message": "Session not found",
                        "data": {"cognition.ai/errorKind": "session_not_found", "cognition.ai/retryable": false}}}));
                    continue;
                }
                a.session = sid;
                a.loaded = true;
                a.cwd = p["cwd"].as_str().unwrap_or("?").to_string();
                a.mcp = mcp_server(&p);
                a.servers = server_names(&p);
                // The whole conversation again, as Devin replays it (the answers are stand-ins).
                let saved = std::fs::read_to_string(saved_marker(&a.session)).unwrap_or_default();
                for (n, line) in saved.lines().enumerate() {
                    let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
                    let text = e["text"].as_str().unwrap_or("");
                    let at = rfc3339(e["atMs"].as_i64().unwrap_or(0));
                    a.update(json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": text},
                        "_meta": {"cognition.ai/clientMessageId": format!("msg-{n}"), "cognition.ai/timestamp": at}}));
                    a.say(&format!("old answer to {text}"));
                }
                a.announce();
                ok(a.opened(json!({"configOptions": a.config()})));
            }
            "session/set_mode" => {
                a.mode = p["modeId"].as_str().unwrap_or("").to_string();
                a.update(json!({"sessionUpdate": "current_mode_update", "currentModeId": a.mode}));
                ok(json!({}));
            }
            "session/set_config_option" => {
                if p["configId"] == "model" {
                    a.model = p["value"].as_str().unwrap_or("").to_string();
                }
                a.update(json!({"sessionUpdate": "config_option_update", "configOptions": a.config()}));
                ok(json!({"configOptions": a.config()}));
            }
            "session/prompt" => a.prompt(id, &p),
            "session/cancel" => {}
            _ if v.get("method").is_some() && !id.is_null() => {
                out(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "unknown"}}));
            }
            _ => {}
        }
    }
}
