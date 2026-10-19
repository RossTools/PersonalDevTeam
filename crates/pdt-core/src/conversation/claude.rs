//! Claude Code's `claude -p` stream-json protocol (notes: docs/superpowers/notes/phase-5-agent-protocols.md): stdout
//! frames → normalized events, and the lines PDT writes to stdin. Pure; the adapter in pdt-agents owns the process.

use super::{
    AgentEvent, Answer, DeltaField, MAX_TOOL_OUTPUT, MessagePhase, MessageSource, NoticeLevel, RateLimitWindow,
    RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus, cap_text,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What one stdout frame turns into.
#[derive(Debug, Clone, PartialEq)]
pub enum MapOut {
    /// A new or changed event (`item_id` identifies the row to update in place).
    Event { item_id: Option<String>, turn_id: Option<String>, event: AgentEvent },
    /// Streaming text for an item.
    Delta { item_id: String, field: DeltaField, text: String },
    /// `system/init` (sent at every turn's start).
    Init(ClaudeInit),
    /// The reply to one of PDT's own control requests (`pdt-<n>` ids).
    Reply { request_id: String, result: Result<Value, String> },
    /// A line PDT must write to stdin now (refusing a control request it does not handle).
    Send(Value),
}

/// What `system/init` reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClaudeInit {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub version: Option<String>,
    /// `none` when Claude uses the plan login.
    pub api_key_source: Option<String>,
    pub slash_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnswerError {
    #[error("this request is no longer open")]
    Closed,
    #[error("that answer does not fit this request")]
    WrongAnswer,
}

#[derive(Debug, Clone)]
struct ToolInfo {
    tool: String,
    title: String,
    input: Value,
    parent: Option<String>,
}

#[derive(Debug, Clone)]
struct OpenRequest {
    kind: RequestKind,
    tool: String,
    title: String,
    description: Option<String>,
    input: Value,
    tool_use_id: Option<String>,
    suggestions: Vec<Value>,
    turn_id: Option<String>,
}

/// The state of one `claude -p` process's stream.
#[derive(Debug, Default)]
pub struct ClaudeMapper {
    turn: Option<String>,
    interrupt_requested: bool,
    /// Text so far of each top-level message in the turn, by message id.
    messages: HashMap<String, String>,
    /// The last top-level agent message of the turn: it becomes the turn's final answer.
    last_message: Option<String>,
    tools: HashMap<String, ToolInfo>,
    requests: HashMap<String, OpenRequest>,
    declined: HashSet<String>,
    rate_limits: BTreeMap<String, RateLimitWindow>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    cost_usd: Option<f64>,
    model: Option<String>,
    permission_mode: Option<String>,
    next_control: u64,
}

pub fn message_item(id: &str) -> String {
    format!("m:{id}")
}
pub fn tool_item(id: &str) -> String {
    format!("t:{id}")
}
pub fn request_item(id: &str) -> String {
    format!("r:{id}")
}
pub const USAGE_ITEM: &str = "usage";

/// A user turn as a stdin line. `now` aborts what Claude is doing and runs it as a fresh turn.
pub fn user_line(session_id: &str, text: &str, now: bool) -> Value {
    let mut v = json!({
        "type": "user",
        "message": {"role": "user", "content": text},
        "parent_tool_use_id": null,
        "session_id": session_id,
    });
    if now {
        v["priority"] = json!("now");
    }
    v
}

/// The one-line subject of a tool call: the command, the file, the pattern, the URL.
pub fn tool_title(tool: &str, input: &Value) -> String {
    let field = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let raw = match tool {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let f = field("file_path");
            if f.is_empty() { field("notebook_path") } else { f }
        }
        "Grep" | "Glob" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Task" | "Agent" => field("description"),
        "Skill" => field("skill"),
        _ => String::new(),
    };
    let line = raw.lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(200).collect();
    if line.chars().count() > 200 || raw.lines().nth(1).is_some() {
        t.push('…');
    }
    t
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn str_field(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

/// `<tag>body</tag>` → body (Claude wraps local command output this way).
fn strip_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let rest = s.trim().strip_prefix(open.as_str())?;
    Some(rest.strip_suffix(close.as_str()).unwrap_or(rest).trim())
}

impl ClaudeMapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// The turn that is running, if any.
    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    pub fn open_requests(&self) -> usize {
        self.requests.len()
    }

    /// PDT asked Claude to interrupt: the turn's error result means "interrupted", not "failed".
    pub fn note_interrupt(&mut self) {
        self.interrupt_requested = true;
    }

    /// A control request line with a fresh `pdt-<n>` id.
    pub fn control_line(&mut self, subtype: &str, mut extra: Value) -> (String, Value) {
        self.next_control += 1;
        let id = format!("pdt-{}", self.next_control);
        if !extra.is_object() {
            extra = json!({});
        }
        extra["subtype"] = json!(subtype);
        (id.clone(), json!({"type": "control_request", "request_id": id, "request": extra}))
    }

    fn event(&self, item_id: Option<String>, event: AgentEvent) -> MapOut {
        MapOut::Event { item_id, turn_id: self.turn.clone(), event }
    }

    fn start_turn(&mut self, id: String, out: &mut Vec<MapOut>) {
        self.turn = Some(id.clone());
        out.push(MapOut::Event {
            item_id: Some(format!("turn:{id}")),
            turn_id: Some(id),
            event: AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None },
        });
    }

    fn ensure_turn(&mut self, fallback_id: &str, out: &mut Vec<MapOut>) {
        if self.turn.is_none() {
            self.start_turn(format!("x-{fallback_id}"), out);
        }
    }

    fn request_event(&self, id: &str, r: &OpenRequest, state: RequestState, answer: Option<Answer>) -> MapOut {
        MapOut::Event {
            item_id: Some(request_item(id)),
            turn_id: r.turn_id.clone(),
            event: AgentEvent::Request {
                id: id.to_string(),
                request_kind: r.kind,
                tool: r.tool.clone(),
                title: r.title.clone(),
                description: r.description.clone(),
                input: r.input.clone(),
                tool_call_id: r.tool_use_id.clone(),
                can_always: r.kind == RequestKind::Approval && !session_rules(&r.suggestions).is_empty(),
                state,
                answer,
            },
        }
    }

    fn usage_event(&self) -> MapOut {
        MapOut::Event {
            item_id: Some(USAGE_ITEM.into()),
            turn_id: None,
            event: AgentEvent::Usage {
                context_used: self.context_used,
                context_window: self.context_window,
                rate_limits: self.rate_limits.values().cloned().collect(),
                cost_usd: self.cost_usd,
            },
        }
    }

    /// Expires every open request (the turn ended, or the process stopped).
    pub fn expire_requests(&mut self) -> Vec<MapOut> {
        let mut ids: Vec<String> = self.requests.keys().cloned().collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                self.requests.remove(&id).map(|r| self.request_event(&id, &r, RequestState::Expired, None))
            })
            .collect()
    }

    /// The process ended: open requests expire and a running turn ends as interrupted (`reason`).
    pub fn on_exit(&mut self, reason: &str) -> Vec<MapOut> {
        let mut out = self.expire_requests();
        if let Some(id) = self.turn.take() {
            out.push(MapOut::Event {
                item_id: Some(format!("turn-end:{id}")),
                turn_id: Some(id),
                event: AgentEvent::Turn {
                    phase: TurnPhase::Ended,
                    status: Some(TurnStatus::Interrupted),
                    duration_ms: None,
                    reason: Some(reason.to_string()),
                },
            });
        }
        self.messages.clear();
        self.last_message = None;
        out
    }

    /// Maps one stdout line (a JSON frame). Lines that are not JSON objects map to nothing.
    pub fn on_line(&mut self, line: &str) -> Vec<MapOut> {
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(v) if v.is_object() => self.on_frame(&v),
            _ => Vec::new(),
        }
    }

    pub fn on_frame(&mut self, v: &Value) -> Vec<MapOut> {
        let mut out = Vec::new();
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "system" => self.on_system(v, &mut out),
            "user" => self.on_user(v, &mut out),
            "assistant" => self.on_assistant(v, &mut out),
            "stream_event" => self.on_stream(v, &mut out),
            "control_request" => self.on_control_request(v, &mut out),
            "control_response" => {
                let r = &v["response"];
                if let Some(id) = r.get("request_id").and_then(Value::as_str).filter(|id| id.starts_with("pdt-")) {
                    let result = if r.get("subtype").and_then(Value::as_str) == Some("success") {
                        Ok(r.get("response").cloned().unwrap_or(Value::Null))
                    } else {
                        Err(str_field(r, "error").unwrap_or_else(|| "the agent refused the request".into()))
                    };
                    out.push(MapOut::Reply { request_id: id.to_string(), result });
                }
            }
            "control_cancel_request" => {
                if let Some(id) = v.get("request_id").and_then(Value::as_str)
                    && let Some(r) = self.requests.remove(id)
                {
                    out.push(self.request_event(id, &r, RequestState::Expired, None));
                }
            }
            "rate_limit_event" => self.on_rate_limit(v, &mut out),
            "result" => self.on_result(v, &mut out),
            "keep_alive" => {}
            _ => out.push(self.event(None, AgentEvent::Unknown { raw: v.clone() })),
        }
        out
    }

    fn on_system(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        match v.get("subtype").and_then(Value::as_str).unwrap_or("") {
            "init" => {
                let init = ClaudeInit {
                    model: str_field(v, "model"),
                    permission_mode: str_field(v, "permissionMode"),
                    version: str_field(v, "claude_code_version"),
                    api_key_source: str_field(v, "apiKeySource"),
                    slash_commands: v
                        .get("slash_commands")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                        .unwrap_or_default(),
                };
                if let (Some(old), Some(new)) = (&self.model, &init.model)
                    && old != new
                {
                    let text = format!("Model changed to {new}");
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
                }
                if let (Some(old), Some(new)) = (&self.permission_mode, &init.permission_mode)
                    && old != new
                {
                    let text = format!("Permission mode changed to {new}");
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
                }
                self.model = init.model.clone().or(self.model.take());
                self.permission_mode = init.permission_mode.clone().or(self.permission_mode.take());
                out.push(MapOut::Init(init));
            }
            "compact_boundary" => {
                let before = v.pointer("/compact_metadata/pre_tokens").and_then(Value::as_i64);
                out.push(self.event(None, AgentEvent::Compaction { before, after: None }));
            }
            "api_retry" => {
                let attempt = v.get("attempt").and_then(Value::as_i64).unwrap_or(0);
                let text = format!("Claude is retrying a failed request (attempt {attempt})");
                out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None }));
            }
            // Progress Claude reports for its own bookkeeping; the tool calls carry what matters.
            "status" | "task_started" | "task_progress" | "task_notification" | "hook_started" | "hook_response"
            | "hook_progress" | "files_persisted" => {}
            _ => out.push(self.event(None, AgentEvent::Unknown { raw: v.clone() })),
        }
    }

    fn on_user(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let content = &v["message"]["content"];
        let replay = v.get("isReplay").and_then(Value::as_bool).unwrap_or(false);
        if replay {
            let uuid = str_field(v, "uuid").unwrap_or_default();
            let source = if self.turn.is_some() { MessageSource::Steer } else { MessageSource::Pdt };
            if self.turn.is_none() {
                self.start_turn(uuid.clone(), out);
            }
            let images = match content {
                Value::Array(b) => b
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("image"))
                    .map(|b| b.pointer("/source/media_type").and_then(Value::as_str).unwrap_or("image").to_string())
                    .collect(),
                _ => Vec::new(),
            };
            let item = (!uuid.is_empty()).then(|| format!("u:{uuid}"));
            out.push(self.event(item, AgentEvent::UserMessage { text: text_of(content), images, source }));
            return;
        }
        let Value::Array(blocks) = content else {
            if let Some(body) = content.as_str().and_then(|s| strip_tag(s, "local-command-stdout")) {
                let text = body.to_string();
                out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
            }
            return;
        };
        for b in blocks {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else { continue };
            let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let output = cap_text(&text_of(&b["content"]), MAX_TOOL_OUTPUT);
            let status = if self.declined.remove(id) {
                ToolStatus::Declined
            } else if is_error {
                ToolStatus::Failed
            } else {
                ToolStatus::Done
            };
            let info = self.tools.remove(id).unwrap_or_else(|| ToolInfo {
                tool: "tool".into(),
                title: String::new(),
                input: Value::Null,
                parent: None,
            });
            let exit_code = v.pointer("/tool_use_result/exitCode").and_then(Value::as_i64).map(|c| c as i32);
            out.push(self.event(
                Some(tool_item(id)),
                AgentEvent::ToolCall {
                    id: id.to_string(),
                    tool: info.tool,
                    title: info.title,
                    input: info.input,
                    status,
                    output: Some(output),
                    exit_code,
                    diff: None,
                    parent_id: info.parent,
                },
            ));
        }
    }

    fn on_assistant(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let msg = &v["message"];
        let msg_id = str_field(msg, "id").unwrap_or_default();
        let parent = str_field(v, "parent_tool_use_id");
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else { return };
        self.ensure_turn(&msg_id, out);
        for b in blocks {
            match b.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" if parent.is_none() => {
                    let text = b.get("text").and_then(Value::as_str).unwrap_or("");
                    let entry = self.messages.entry(msg_id.clone()).or_default();
                    if !entry.is_empty() {
                        entry.push_str("\n\n");
                    }
                    entry.push_str(text);
                    let markdown = entry.clone();
                    self.last_message = Some(msg_id.clone());
                    out.push(self.event(
                        Some(message_item(&msg_id)),
                        AgentEvent::AgentMessage { id: msg_id.clone(), markdown, phase: MessagePhase::Commentary },
                    ));
                }
                "thinking" if parent.is_none() => {
                    let text = b.get("thinking").and_then(Value::as_str).unwrap_or("").trim();
                    if !text.is_empty() {
                        let summary = text.to_string();
                        out.push(self.event(Some(format!("th:{msg_id}")), AgentEvent::Reasoning { summary }));
                    }
                }
                "tool_use" => {
                    let id = str_field(b, "id").unwrap_or_default();
                    let tool = str_field(b, "name").unwrap_or_else(|| "tool".into());
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    let title = tool_title(&tool, &input);
                    let info = ToolInfo {
                        tool: tool.clone(),
                        title: title.clone(),
                        input: input.clone(),
                        parent: parent.clone(),
                    };
                    self.tools.insert(id.clone(), info);
                    // A tool call ends the message before it: it was commentary.
                    if parent.is_none() {
                        self.last_message = None;
                    }
                    out.push(self.event(
                        Some(tool_item(&id)),
                        AgentEvent::ToolCall {
                            id,
                            tool,
                            title,
                            input,
                            status: ToolStatus::Running,
                            output: None,
                            exit_code: None,
                            diff: None,
                            parent_id: parent.clone(),
                        },
                    ));
                }
                _ => {}
            }
        }
    }

    fn on_stream(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
            return;
        }
        let e = &v["event"];
        if e.get("type").and_then(Value::as_str) != Some("content_block_delta")
            || e.pointer("/delta/type").and_then(Value::as_str) != Some("text_delta")
        {
            return;
        }
        let Some(id) = str_field(v, "api_message_id") else { return };
        let text = e.pointer("/delta/text").and_then(Value::as_str).unwrap_or("");
        if !text.is_empty() {
            out.push(MapOut::Delta { item_id: message_item(&id), field: DeltaField::Markdown, text: text.into() });
        }
    }

    fn on_control_request(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let id = str_field(v, "request_id").unwrap_or_default();
        let r = &v["request"];
        if r.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
            // Claude waits for an answer to every control request: refuse what PDT does not handle.
            let subtype = r.get("subtype").and_then(Value::as_str).unwrap_or("?");
            out.push(MapOut::Send(json!({
                "type": "control_response",
                "response": {"subtype": "error", "request_id": id, "error": format!("{subtype} is not supported")},
            })));
            return;
        }
        let tool = str_field(r, "tool_name").unwrap_or_else(|| "tool".into());
        let input = r.get("input").cloned().unwrap_or(Value::Null);
        let kind = match tool.as_str() {
            "AskUserQuestion" => RequestKind::Question,
            "ExitPlanMode" => RequestKind::Plan,
            _ => RequestKind::Approval,
        };
        let open = OpenRequest {
            kind,
            title: tool_title(&tool, &input),
            description: str_field(r, "description").or_else(|| str_field(&input, "description")),
            tool,
            input,
            tool_use_id: str_field(r, "tool_use_id"),
            suggestions: r.get("permission_suggestions").and_then(Value::as_array).cloned().unwrap_or_default(),
            turn_id: self.turn.clone(),
        };
        out.push(self.request_event(&id, &open, RequestState::Open, None));
        self.requests.insert(id, open);
    }

    fn on_rate_limit(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let info = &v["rate_limit_info"];
        if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
            for (name, w) in windows {
                let window = RateLimitWindow {
                    name: name.clone(),
                    utilization: w.get("utilization").and_then(Value::as_f64).unwrap_or(0.0),
                    resets_at: w.get("resetsAt").and_then(Value::as_i64).map(|s| s * 1000),
                };
                self.rate_limits.insert(name.clone(), window);
            }
        }
        out.push(self.usage_event());
        if info.get("status").and_then(Value::as_str) == Some("rejected") {
            let until = info.get("resetsAt").and_then(Value::as_i64).map(|s| s * 1000);
            let text = "Claude's usage limit is reached. Nothing is sent to it until you send again.".to_string();
            out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until }));
        }
    }

    fn on_result(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let fallback = str_field(v, "uuid").unwrap_or_default();
        self.ensure_turn(&fallback, out);
        out.extend(self.expire_requests());
        if let Some(id) = self.last_message.take()
            && let Some(markdown) = self.messages.get(&id).cloned()
        {
            out.push(
                self.event(
                    Some(message_item(&id)),
                    AgentEvent::AgentMessage { id, markdown, phase: MessagePhase::Final },
                ),
            );
        }
        let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false) || subtype != "success";
        let interrupted = is_error && (self.interrupt_requested || subtype == "error_during_execution");
        let status = match (is_error, interrupted) {
            (false, _) => TurnStatus::Completed,
            (true, true) => TurnStatus::Interrupted,
            (true, false) => TurnStatus::Failed,
        };
        let reason = match status {
            TurnStatus::Completed => None,
            TurnStatus::Interrupted => Some("Interrupted".to_string()),
            TurnStatus::Failed => Some(
                str_field(v, "result")
                    .or_else(|| {
                        v.get("errors")
                            .and_then(Value::as_array)
                            .map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("The turn failed ({subtype})")),
            ),
        };
        if status == TurnStatus::Failed {
            let text = reason.clone().unwrap_or_default();
            let rate_limited = v.get("api_error_status").and_then(Value::as_i64) == Some(429);
            if rate_limited {
                let text = "Claude is rate limited. Nothing is sent to it until you send again.".to_string();
                out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None }));
            }
            out.push(self.event(None, AgentEvent::Error { text, retryable: false }));
        }
        // Context in use: the last model call's whole input.
        let usage = v.pointer("/usage/iterations").and_then(Value::as_array).and_then(|a| a.last()).or(v.get("usage"));
        if let Some(u) = usage {
            let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
            let used = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
            if used > 0 {
                self.context_used = Some(used);
            }
        }
        if let Some(window) = v
            .get("modelUsage")
            .and_then(Value::as_object)
            .and_then(|m| m.values().filter_map(|u| u.get("contextWindow").and_then(Value::as_i64)).max())
        {
            self.context_window = Some(window);
        }
        if let Some(c) = v.get("total_cost_usd").and_then(Value::as_f64) {
            self.cost_usd = Some(c);
        }
        out.push(self.usage_event());
        let id = self.turn.take().unwrap_or_default();
        out.push(MapOut::Event {
            item_id: Some(format!("turn-end:{id}")),
            turn_id: Some(id),
            event: AgentEvent::Turn {
                phase: TurnPhase::Ended,
                status: Some(status),
                duration_ms: v.get("duration_ms").and_then(Value::as_i64),
                reason,
            },
        });
        self.interrupt_requested = false;
        self.messages.clear();
        self.tools.retain(|_, t| t.parent.is_some());
    }

    /// Builds the `control_response` line answering an open request, and the events it changes (the request, and a
    /// tool call the user declined).
    pub fn answer(&mut self, request_id: &str, answer: &Answer) -> Result<(Value, Vec<MapOut>), AnswerError> {
        let r = self.requests.get(request_id).ok_or(AnswerError::Closed)?;
        let deny = |message: &str| json!({"behavior": "deny", "message": message});
        let allow = |input: Value| json!({"behavior": "allow", "updatedInput": input});
        let (response, declined) = match (r.kind, answer) {
            (RequestKind::Question, Answer::Approve { .. } | Answer::Plan { .. })
            | (RequestKind::Approval, Answer::Question { .. } | Answer::Plan { .. })
            | (RequestKind::Plan, Answer::Question { .. }) => return Err(AnswerError::WrongAnswer),
            (_, Answer::Approve { always }) => {
                let mut resp = allow(r.input.clone());
                let rules = session_rules(&r.suggestions);
                if *always && !rules.is_empty() {
                    resp["updatedPermissions"] = Value::Array(rules);
                }
                (resp, false)
            }
            (_, Answer::Deny { message }) => {
                let m = message.as_deref().map(str::trim).filter(|m| !m.is_empty());
                (deny(m.unwrap_or("The user declined this.")), true)
            }
            (_, Answer::Question { answers }) => {
                let mut input = r.input.clone();
                if !input.is_object() {
                    input = json!({});
                }
                let joined: serde_json::Map<String, Value> =
                    answers.iter().map(|(q, a)| (q.clone(), Value::String(a.join(", ")))).collect();
                input["answers"] = Value::Object(joined);
                (allow(input), false)
            }
            (_, Answer::Plan { approve: true, .. }) => (allow(r.input.clone()), false),
            (_, Answer::Plan { approve: false, feedback }) => {
                let f = feedback.as_deref().map(str::trim).filter(|f| !f.is_empty());
                (deny(f.unwrap_or("Keep planning: the user wants changes before you start.")), true)
            }
        };
        let r = self.requests.remove(request_id).ok_or(AnswerError::Closed)?;
        if declined && let Some(t) = &r.tool_use_id {
            self.declined.insert(t.clone());
        }
        let line = json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response},
        });
        let ev = self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()));
        Ok((line, vec![ev]))
    }
}

/// Claude's permission suggestions turned into session-only rules: "Always allow" never writes settings files. For
/// file edits Claude suggests `setMode acceptEdits`, which switches the session to Accept edits.
fn session_rules(suggestions: &[Value]) -> Vec<Value> {
    suggestions
        .iter()
        .filter(|s| matches!(s.get("type").and_then(Value::as_str), Some("addRules" | "addDirectories" | "setMode")))
        .map(|s| {
            let mut s = s.clone();
            s["destination"] = json!("session");
            s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPROVAL: &str = include_str!("../../tests/fixtures/claude/approval.jsonl");

    fn events(out: &[MapOut]) -> Vec<(Option<String>, AgentEvent)> {
        out.iter()
            .filter_map(|o| match o {
                MapOut::Event { item_id, event, .. } => Some((item_id.clone(), event.clone())),
                _ => None,
            })
            .collect()
    }

    fn run(m: &mut ClaudeMapper, frames: &str) -> Vec<MapOut> {
        frames.lines().flat_map(|l| m.on_line(l)).collect()
    }

    #[test]
    fn a_recorded_turn_with_an_approval_maps_to_events() {
        let mut m = ClaudeMapper::new();
        let lines: Vec<&str> = APPROVAL.lines().collect();
        // Up to and including the permission request.
        let at = lines.iter().position(|l| l.contains("\"can_use_tool\"")).unwrap();
        let out: Vec<MapOut> = lines[..=at].iter().flat_map(|l| m.on_line(l)).collect();
        assert!(out.iter().any(|o| matches!(o, MapOut::Init(i) if i.api_key_source.as_deref() == Some("none"))));
        let ev = events(&out);
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(kinds, ["turn", "user_message", "tool_call", "request"], "no unknown rows for known frames");
        let AgentEvent::ToolCall { title, status, .. } = &ev[2].1 else { panic!() };
        assert_eq!((title.as_str(), *status), ("touch probe.txt", ToolStatus::Running));
        let (req_item, AgentEvent::Request { id, request_kind, can_always, state, tool_call_id, .. }) = &ev[3] else {
            panic!()
        };
        assert_eq!(*request_kind, RequestKind::Approval);
        assert_eq!(*state, RequestState::Open);
        assert!(can_always);
        assert_eq!(req_item.as_deref(), Some(format!("r:{id}").as_str()));
        assert_eq!(tool_call_id.as_deref(), Some("toolu_0132Po1bno8XHiGd91fEuvS9"));
        assert_eq!(m.open_requests(), 1);
        assert!(m.turn().is_some());

        let (line, changed) = m.answer(id, &Answer::Approve { always: true }).unwrap();
        assert_eq!(line["response"]["response"]["behavior"], "allow");
        let perms = line["response"]["response"]["updatedPermissions"].as_array().unwrap();
        assert!(!perms.is_empty());
        assert!(perms.iter().all(|p| p["destination"] == "session"), "never localSettings: {perms:?}");
        assert!(matches!(&events(&changed)[0].1, AgentEvent::Request { state: RequestState::Answered, .. }));
        assert_eq!(m.answer(id, &Answer::Approve { always: false }), Err(AnswerError::Closed));

        let rest: Vec<MapOut> = lines[at + 1..].iter().flat_map(|l| m.on_line(l)).collect();
        let ev = events(&rest);
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(kinds, ["usage", "tool_call", "agent_message", "agent_message", "usage", "turn"]);
        assert!(matches!(&ev[1].1, AgentEvent::ToolCall { status: ToolStatus::Done, .. }));
        assert!(
            matches!(&ev[3].1, AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } if markdown == "done")
        );
        assert_eq!(ev[2].0, ev[3].0, "the final answer updates the same row");
        let AgentEvent::Usage { context_used, context_window, rate_limits, cost_usd } = &ev[4].1 else { panic!() };
        assert!(context_used.unwrap() > 0 && context_window.is_some() && cost_usd.is_some());
        assert_eq!(rate_limits.len(), 2);
        assert!(matches!(
            &ev[5].1,
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert!(m.turn().is_none());
        assert!(
            rest.iter().any(|o| matches!(o, MapOut::Delta { field: DeltaField::Markdown, text, .. } if text == "done"))
        );
    }

    #[test]
    fn deny_declines_the_tool_and_sends_the_message() {
        let mut m = ClaudeMapper::new();
        run(
            &mut m,
            r#"{"type":"user","message":{"role":"user","content":"go"},"uuid":"u1","isReplay":true}
{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"rm -rf x\nmore"}}]},"parent_tool_use_id":null}
{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf x"},"tool_use_id":"t1","permission_suggestions":[]}}"#,
        );
        let (line, _) = m.answer("r1", &Answer::Deny { message: Some(" too risky ".into()) }).unwrap();
        assert_eq!(line["response"]["response"], json!({"behavior": "deny", "message": "too risky"}));
        let out = m.on_line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"denied","is_error":true}]}}"#);
        let AgentEvent::ToolCall { status, title, .. } = &events(&out)[0].1 else { panic!() };
        assert_eq!(*status, ToolStatus::Declined);
        assert_eq!(title, "rm -rf x…");
    }

    #[test]
    fn questions_and_plans_take_their_own_answers() {
        let mut m = ClaudeMapper::new();
        let out = run(
            &mut m,
            r#"{"type":"control_request","request_id":"q","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Which?","options":[{"label":"A"},{"label":"B"}]}]}}}
{"type":"control_request","request_id":"p","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","input":{"plan":"1. do it"}}}"#,
        );
        let ev = events(&out);
        assert!(matches!(&ev[0].1, AgentEvent::Request { request_kind: RequestKind::Question, can_always: false, .. }));
        assert!(matches!(&ev[1].1, AgentEvent::Request { request_kind: RequestKind::Plan, .. }));
        assert_eq!(m.answer("q", &Answer::Approve { always: false }), Err(AnswerError::WrongAnswer));
        let answers = BTreeMap::from([("Which?".to_string(), vec!["A".to_string(), "B".to_string()])]);
        let (line, _) = m.answer("q", &Answer::Question { answers }).unwrap();
        assert_eq!(line["response"]["response"]["updatedInput"]["answers"], json!({"Which?": "A, B"}));
        let (line, _) = m.answer("p", &Answer::Plan { approve: false, feedback: None }).unwrap();
        assert_eq!(line["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn interrupt_and_errors_end_the_turn() {
        let mut m = ClaudeMapper::new();
        run(
            &mut m,
            r#"{"type":"user","message":{"content":"go"},"uuid":"u1","isReplay":true}
{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}"#,
        );
        m.note_interrupt();
        let out = m.on_line(r#"{"type":"result","subtype":"error_during_execution","is_error":true,"duration_ms":10}"#);
        let ev = events(&out);
        assert!(matches!(&ev[0].1, AgentEvent::Request { state: RequestState::Expired, .. }), "{ev:?}");
        assert!(matches!(ev.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
        assert!(!ev.iter().any(|(_, e)| e.kind() == "error"));

        run(&mut m, r#"{"type":"user","message":{"content":"again"},"uuid":"u2","isReplay":true}"#);
        let out = m.on_line(r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"API Error: rate limited"}"#);
        let ev = events(&out);
        assert!(ev.iter().any(|(_, e)| matches!(e, AgentEvent::Notice { level: NoticeLevel::Warning, .. })));
        assert!(
            ev.iter().any(|(_, e)| matches!(e, AgentEvent::Error { text, .. } if text == "API Error: rate limited"))
        );
        assert!(matches!(ev.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Failed), .. }));
    }

    #[test]
    fn exit_mid_turn_interrupts_it_and_expires_requests() {
        let mut m = ClaudeMapper::new();
        run(
            &mut m,
            r#"{"type":"user","message":{"content":"go"},"uuid":"u1","isReplay":true}
{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{}}}"#,
        );
        let ev = events(&m.on_exit("Claude stopped"));
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].0.as_deref(), Some("turn-end:u1"));
        assert!(m.on_exit("again").is_empty());
    }

    #[test]
    fn steering_unknown_frames_and_unsupported_control_requests() {
        let mut m = ClaudeMapper::new();
        let out = run(
            &mut m,
            r#"{"type":"user","message":{"content":"one"},"uuid":"u1","isReplay":true}
{"type":"user","message":{"content":[{"type":"text","text":"two"},{"type":"image","source":{"media_type":"image/png"}}]},"uuid":"u2","isReplay":true}
{"type":"mystery","x":1}
{"type":"system","subtype":"status","status":"requesting"}
{"type":"control_request","request_id":"h1","request":{"subtype":"hook_callback"}}
not json"#,
        );
        let ev = events(&out);
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { source: MessageSource::Pdt, .. }));
        assert!(
            matches!(&ev[2].1, AgentEvent::UserMessage { source: MessageSource::Steer, text, images, .. } if text == "two" && images == &["image/png"])
        );
        assert!(matches!(&ev[3].1, AgentEvent::Unknown { .. }));
        assert_eq!(ev.len(), 4);
        assert!(out.iter().any(|o| matches!(o, MapOut::Send(v) if v["response"]["subtype"] == "error")));
    }

    #[test]
    fn init_changes_become_notices_and_replies_match_pdt_ids() {
        let mut m = ClaudeMapper::new();
        let first = m.on_line(r#"{"type":"system","subtype":"init","model":"opus","permissionMode":"default"}"#);
        assert_eq!(events(&first).len(), 0);
        let out = m.on_line(r#"{"type":"system","subtype":"init","model":"sonnet","permissionMode":"plan"}"#);
        assert_eq!(events(&out).len(), 2);
        let (id, line) = m.control_line("interrupt", json!({}));
        assert_eq!(line["request"]["subtype"], "interrupt");
        let out = m.on_line(&format!(
            r#"{{"type":"control_response","response":{{"subtype":"error","request_id":"{id}","error":"nope"}}}}"#
        ));
        assert_eq!(out, vec![MapOut::Reply { request_id: id, result: Err("nope".into()) }]);
        assert!(
            m.on_line(r#"{"type":"control_response","response":{"subtype":"success","request_id":"abc"}}"#).is_empty()
        );
    }

    #[test]
    fn titles_and_user_lines() {
        assert_eq!(tool_title("Edit", &json!({"file_path": "src/a.rs"})), "src/a.rs");
        assert_eq!(tool_title("Mystery", &json!({"x": 1})), "");
        let long = "x".repeat(300);
        assert_eq!(tool_title("Bash", &json!({"command": long})).chars().count(), 201);
        let l = user_line("s", "hi", true);
        assert_eq!((l["priority"].as_str(), l["message"]["content"].as_str()), (Some("now"), Some("hi")));
        assert!(user_line("s", "hi", false).get("priority").is_none());
    }
}
