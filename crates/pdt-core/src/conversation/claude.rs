//! Claude Code's `claude -p` stream-json protocol: stdout
//! frames → normalized events, and the lines PDT writes to stdin. Pure; the adapter in pdt-agents owns the process.

use super::{
    AgentEvent, Answer, BackgroundTask, DeltaField, MAX_TOOL_OUTPUT, MessagePhase, MessageSource, NoticeLevel,
    PermissionMode, RateLimitWindow, RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus, cap_text,
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
    /// The background tasks now running (`background_tasks_changed`): all of them, none when empty.
    BackgroundTasks(Vec<BackgroundTask>),
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
    /// Commands that work only in the TUI.
    pub terminal_commands: Vec<String>,
    /// Claude's id for the conversation this turn belongs to. It changes after `/clear` (the new id is this one,
    /// not `conversation_reset`'s `new_conversation_id`, which names no transcript).
    pub session_id: Option<String>,
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
    /// The session's folder: diffs name files relative to it, and a turn's diff keeps only files inside it.
    cwd: Option<String>,
    /// The files the running turn changed, in the order first changed.
    turn_files: Vec<FileDiff>,
    /// Slash commands PDT sent that Claude has not shown yet (local ones, like `/context`, are never echoed).
    sent_commands: std::collections::VecDeque<String>,
    shown_commands: u64,
}

/// One file's part of a diff: `--- a/path` / `+++ b/path` (or `/dev/null` for a new file), then hunks.
#[derive(Debug, Clone, PartialEq)]
struct FileDiff {
    path: String,
    created: bool,
    hunks: String,
}

impl FileDiff {
    fn render(&self) -> String {
        let side = |prefix: &str| {
            if is_absolute(&self.path) { self.path.clone() } else { format!("{prefix}{}", self.path) }
        };
        let old = if self.created { "/dev/null".to_string() } else { side("a/") };
        format!("--- {old}\n+++ {}\n{}", side("b/"), self.hunks)
    }
}

fn is_absolute(p: &str) -> bool {
    p.starts_with('/') || p.starts_with('\\') || p.as_bytes().get(1) == Some(&b':')
}

/// The tools whose calls change files.
fn edits_files(tool: &str) -> bool {
    matches!(tool, "Edit" | "MultiEdit" | "Write")
}

/// `-` lines then `+` lines, for a change known only by its text (no line numbers).
fn text_hunk(old: &str, new: &str) -> String {
    let mut h = String::from("@@ @@\n");
    for l in old.lines() {
        h.push('-');
        h.push_str(l);
        h.push('\n');
    }
    for l in new.lines() {
        h.push('+');
        h.push_str(l);
        h.push('\n');
    }
    h
}

/// The change a file tool's input asks for (no line numbers: the agent has not read the file for us).
fn input_hunks(tool: &str, input: &Value) -> Option<String> {
    let field = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match tool {
        "Edit" => Some(text_hunk(&field(input, "old_string"), &field(input, "new_string"))),
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .map(|edits| edits.iter().map(|e| text_hunk(&field(e, "old_string"), &field(e, "new_string"))).collect()),
        "Write" => input.get("content").and_then(Value::as_str).map(|c| text_hunk("", c)),
        _ => None,
    }
}

/// The change a file tool made, from its structured result: hunks with line numbers, or a whole new file.
fn result_hunks(result: &Value) -> Option<(bool, String)> {
    if let Some(patch) = result.get("structuredPatch").and_then(Value::as_array).filter(|p| !p.is_empty()) {
        let n = |h: &Value, k: &str| h.get(k).and_then(Value::as_i64).unwrap_or(0);
        let mut out = String::new();
        for h in patch {
            out.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                n(h, "oldStart"),
                n(h, "oldLines"),
                n(h, "newStart"),
                n(h, "newLines")
            ));
            for l in h.get("lines").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                out.push_str(l);
                out.push('\n');
            }
        }
        return Some((false, out));
    }
    if result.get("type").and_then(Value::as_str) == Some("create") {
        let content = result.get("content").and_then(Value::as_str).unwrap_or("");
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Some((true, String::new()));
        }
        let mut out = format!("@@ -0,0 +1,{} @@\n", lines.len());
        for l in lines {
            out.push('+');
            out.push_str(l);
            out.push('\n');
        }
        return Some((true, out));
    }
    None
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

/// A user turn as a stdin line, with images as `(media type, base64 data)` before the text. `now` aborts what Claude
/// is doing and runs it as a fresh turn.
pub fn user_line(session_id: &str, text: &str, images: &[(String, String)], now: bool) -> Value {
    let content = if images.is_empty() {
        json!(text)
    } else {
        let mut blocks: Vec<Value> = images
            .iter()
            .map(|(media, data)| json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": data}}))
            .collect();
        if !text.trim().is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
        Value::Array(blocks)
    };
    let mut v = json!({
        "type": "user",
        "message": {"role": "user", "content": content},
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
    let pdt = tool.strip_prefix(&format!("mcp__{}__", crate::mcp::SERVER_NAME));
    let raw = match (tool, pdt) {
        // PDT's asking tools (PDT-81): what the user says yes to.
        (_, Some("create_ticket")) => format!("New ticket in {}: {}", field("project"), field("title")),
        (_, Some("edit_ticket")) => {
            let changes: Vec<&str> = ["title", "description", "stage", "priority", "labels"]
                .into_iter()
                .filter(|k| input.get(*k).is_some_and(|v| !v.is_null()))
                .collect();
            format!("Edit {}: {}", field("ticket"), changes.join(", "))
        }
        (_, Some("stop_ticket")) => {
            let removes = input.get("removeWorktree").and_then(Value::as_bool) == Some(true);
            let to = input.get("stage").and_then(Value::as_str).map(|s| format!(", moving it to {s}"));
            let gone = if removes { ", removing its worktree" } else { "" };
            format!("Stop {}'s work{}{gone}", field("ticket"), to.unwrap_or_default())
        }
        (_, Some("send_to_session")) => format!("To session {}: {}", field("session"), field("text")),
        ("Bash", _) => field("command"),
        ("Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit", _) => {
            let f = field("file_path");
            if f.is_empty() { field("notebook_path") } else { f }
        }
        ("Grep" | "Glob", _) => field("pattern"),
        ("WebFetch", _) => field("url"),
        ("WebSearch", _) => field("query"),
        ("Task" | "Agent", _) => field("description"),
        ("Skill", _) => field("skill"),
        _ => String::new(),
    };
    title_line(&raw)
}

/// A tool call's title from its subject: the first line, at most 200 characters (`…` when cut).
pub fn title_line(raw: &str) -> String {
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
/// A finished background task's report, which Claude sends itself as a user message (`origin.kind`
/// `task-notification`, `<task-notification>…<summary>…</summary>…`): its summary, to show as a notice rather than as
/// something the user said.
fn task_notification(entry: &Value, text: &str) -> Option<String> {
    let tagged = text.trim_start().starts_with("<task-notification>");
    if !tagged && entry.pointer("/origin/kind").and_then(Value::as_str) != Some("task-notification") {
        return None;
    }
    let summary = text
        .split_once("<summary>")
        .and_then(|(_, rest)| rest.split_once("</summary>"))
        .map(|(summary, _)| unescape_xml(summary.trim()))
        .filter(|s| !s.is_empty());
    Some(summary.unwrap_or_else(|| "A background task finished".into()))
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

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

    /// PDT sent a slash command: if Claude never echoes it, it is shown when its turn starts.
    pub fn note_sent_command(&mut self, text: &str) {
        self.sent_commands.push_back(text.trim().to_string());
    }

    /// A mapper for a session working in `cwd`.
    pub fn with_cwd(cwd: impl Into<String>) -> Self {
        Self { cwd: Some(cwd.into()), ..Self::default() }
    }

    /// `path` relative to the session's folder when inside it, else as given.
    fn shown_path(&self, path: &str) -> String {
        self.inside(path).unwrap_or_else(|| path.to_string())
    }

    /// The path relative to the folder, when `path` is inside it (always, with no folder known).
    fn inside(&self, path: &str) -> Option<String> {
        let Some(cwd) = &self.cwd else { return Some(path.to_string()) };
        let cwd = cwd.trim_end_matches(['/', '\\']);
        let rest = path.strip_prefix(cwd)?;
        let rest = rest.strip_prefix(['/', '\\'])?;
        (!rest.is_empty()).then(|| rest.replace('\\', "/"))
    }

    /// The diff of a file tool's call: what it did (`result`), else what it asked for.
    fn tool_diff(&self, tool: &str, input: &Value, result: Option<&Value>) -> Option<FileDiff> {
        if !edits_files(tool) {
            return None;
        }
        let path = input.get("file_path").and_then(Value::as_str)?;
        let (created, hunks) = match result.and_then(result_hunks) {
            Some(r) => r,
            None => (false, input_hunks(tool, input)?),
        };
        Some(FileDiff { path: self.shown_path(path), created, hunks })
    }

    /// Adds a finished edit to the turn's diff, when the file is inside the folder.
    fn note_turn_edit(&mut self, input: &Value, d: &FileDiff) {
        let Some(path) = input.get("file_path").and_then(Value::as_str).and_then(|p| self.inside(p)) else { return };
        match self.turn_files.iter_mut().find(|f| f.path == path) {
            Some(f) => f.hunks.push_str(&d.hunks),
            None => self.turn_files.push(FileDiff { path, created: d.created, hunks: d.hunks.clone() }),
        }
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

    /// Starts a turn that began without an echoed prompt: a local slash command's, which then shows as sent.
    fn ensure_turn(&mut self, fallback_id: &str, out: &mut Vec<MapOut>) {
        if self.turn.is_none() {
            self.start_turn(format!("x-{fallback_id}"), out);
            if let Some(cmd) = self.sent_commands.pop_front() {
                self.shown_commands += 1;
                let item = format!("u:cmd-{fallback_id}-{}", self.shown_commands);
                out.push(self.event(
                    Some(item),
                    AgentEvent::UserMessage { text: cmd, images: vec![], source: MessageSource::Pdt, pdt: None },
                ));
            }
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
                can_always: r.kind == RequestKind::Approval
                    && !session_rules(&r.suggestions).is_empty()
                    && !pdt_asks(&r.tool),
                diff: self.tool_diff(&r.tool, &r.input, None).map(|d| d.render()),
                rows: None,
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
        if let Some(diff) = self.turn_diff_event() {
            out.push(diff);
        }
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

    /// The running turn's diff (and forgets it), when the turn changed files in the folder.
    fn turn_diff_event(&mut self) -> Option<MapOut> {
        let files = std::mem::take(&mut self.turn_files);
        let turn = self.turn.clone()?;
        if files.is_empty() {
            return None;
        }
        let diff = files.iter().map(FileDiff::render).collect::<String>();
        Some(MapOut::Event {
            item_id: Some(format!("diff:{turn}")),
            turn_id: Some(turn),
            event: AgentEvent::TurnDiff { diff },
        })
    }

    /// The turn's last top-level message becomes its answer (once).
    fn finalize_message(&mut self, out: &mut Vec<MapOut>) {
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
    }

    /// Ends the running turn: its last message becomes the answer, then its diff, then the turn's end.
    fn close_turn(
        &mut self,
        status: TurnStatus,
        duration_ms: Option<i64>,
        reason: Option<String>,
        out: &mut Vec<MapOut>,
    ) {
        self.finalize_message(out);
        out.extend(self.turn_diff_event());
        let id = self.turn.take().unwrap_or_default();
        out.push(MapOut::Event {
            item_id: Some(format!("turn-end:{id}")),
            turn_id: Some(id),
            event: AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(status), duration_ms, reason },
        });
        self.interrupt_requested = false;
        self.messages.clear();
        self.tools.retain(|_, t| t.parent.is_some());
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
            // `/clear` (or another trigger): what follows is a new conversation; the next init names its id.
            "conversation_reset" => {
                let trigger = str_field(v, "trigger").unwrap_or_else(|| "clear".into());
                out.push(self.event(None, AgentEvent::ConversationReset { trigger }));
            }
            // A running tool's heartbeat; its row already shows it running.
            "keep_alive" | "tool_progress" => {}
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
                    terminal_commands: v
                        .get("terminal_slash_commands")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                        .unwrap_or_default(),
                    session_id: str_field(v, "session_id"),
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
            "status" => {
                // A mode change (e.g. leaving plan mode) shows here at once; the next turn's init repeats it.
                if let Some(mode) = str_field(v, "permissionMode")
                    && self.permission_mode.as_deref() != Some(mode.as_str())
                {
                    let name =
                        PermissionMode::from_claude(&mode).map_or_else(|| mode.clone(), |m| m.label().to_string());
                    let text = format!("Permission mode: {name}");
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
                    self.permission_mode = Some(mode.clone());
                    out.push(MapOut::Init(ClaudeInit { permission_mode: Some(mode), ..ClaudeInit::default() }));
                }
            }
            // e.g. `gh pr create` or a push to a pull request's branch.
            "code_change_published" => {
                let action = str_field(v, "action").unwrap_or_else(|| "published".into());
                let id = str_field(v, "identifier").unwrap_or_default();
                let target = match str_field(v, "repo") {
                    Some(repo) if !id.is_empty() => format!("{repo}#{id}"),
                    Some(repo) => repo,
                    None => str_field(v, "url").unwrap_or_else(|| format!("#{id}")),
                };
                let text = match action.as_str() {
                    "pushed" => format!("Pushed to {target}"),
                    "created" => format!("Created {target}"),
                    other => format!("Code change {other}: {target}"),
                };
                out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
            }
            // Progress Claude reports for its own bookkeeping; the tool calls carry what matters.
            "background_tasks_changed" => {
                let tasks = v.get("tasks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                let tasks = tasks
                    .iter()
                    .filter_map(|t| {
                        Some(BackgroundTask {
                            id: str_field(t, "task_id")?,
                            description: str_field(t, "description").unwrap_or_default(),
                            kind: str_field(t, "task_type").unwrap_or_default(),
                        })
                    })
                    .collect();
                out.push(MapOut::BackgroundTasks(tasks));
            }
            // Each task's own progress; `background_tasks_changed` says which still run.
            "task_started" | "task_progress" | "task_notification" | "task_updated" | "vcs_state_changed"
            | "hook_started" | "hook_response" | "hook_progress" | "files_persisted" | "thinking_tokens" => {}
            _ => out.push(self.event(None, AgentEvent::Unknown { raw: v.clone() })),
        }
    }

    fn on_user(&mut self, v: &Value, out: &mut Vec<MapOut>) {
        let content = &v["message"]["content"];
        let replay = v.get("isReplay").and_then(Value::as_bool).unwrap_or(false);
        if replay {
            let uuid = str_field(v, "uuid").unwrap_or_default();
            let item = (!uuid.is_empty()).then(|| format!("u:{uuid}"));
            // A background task finished: Claude takes its report as the next message, starting a turn when idle.
            if let Some(text) = task_notification(v, &text_of(content)) {
                if self.turn.is_none() {
                    self.start_turn(uuid, out);
                }
                out.push(self.event(item, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
                return;
            }
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
            // A custom command comes back expanded (`<command-name>/x</command-name>…`): show what was typed.
            let raw = text_of(content);
            let text = command_text(raw.trim()).unwrap_or(raw);
            if let Some(i) = self.sent_commands.iter().position(|c| *c == text.trim()) {
                self.sent_commands.remove(i);
            }
            out.push(self.event(item, AgentEvent::UserMessage { text, images, source, pdt: None }));
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
            let result = (status == ToolStatus::Done).then(|| v.get("tool_use_result")).flatten();
            let diff = self.tool_diff(&info.tool, &info.input, result);
            // Sub-agents' edits change files too: they count toward the turn's diff.
            if let Some(d) = diff.as_ref().filter(|_| status == ToolStatus::Done) {
                self.note_turn_edit(&info.input, d);
            }
            let diff = diff.map(|d| d.render());
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
                    diff,
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
                    let diff = self.tool_diff(&tool, &input, None).map(|d| d.render());
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
                            diff,
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
                    resets_at: w.get("resetsAt").and_then(Value::as_i64).map(|s| s.saturating_mul(1000)),
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
        self.finalize_message(out);
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
        self.close_turn(status, v.get("duration_ms").and_then(Value::as_i64), reason, out);
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
                if *always && !rules.is_empty() && !pdt_asks(&r.tool) {
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
            (_, Answer::Plan { approve: true, mode, .. }) => {
                let mut resp = allow(r.input.clone());
                if let Some(m) = mode {
                    resp["updatedPermissions"] =
                        json!([{"type": "setMode", "mode": m.claude(), "destination": "session"}]);
                }
                (resp, false)
            }
            (_, Answer::Plan { approve: false, feedback, .. }) => {
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("the transcript no longer has the entry PDT's log ends at")]
    AnchorMissing,
}

/// The uuid of a transcript's last conversation entry: where PDT's log ends when the TUI takes over.
pub fn last_uuid(transcript: &str) -> Option<String> {
    transcript
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|e| matches!(e.get("type").and_then(Value::as_str), Some("user" | "assistant" | "system")))
        .and_then(|e| str_field(&e, "uuid"))
}

/// `<command-name>/model</command-name>…<command-args>opus</command-args>` → `/model opus`.
fn command_text(s: &str) -> Option<String> {
    let between = |open: &str, close: &str| {
        let start = s.find(open)? + open.len();
        let end = s[start..].find(close)? + start;
        Some(s[start..end].trim().to_string())
    };
    let name = between("<command-name>", "</command-name>")?;
    let args = between("<command-args>", "</command-args>").unwrap_or_default();
    Some(if args.is_empty() { name } else { format!("{name} {args}") })
}

/// Events for the turns a session's transcript (Claude's `<id>.jsonl`) has after the entry `after` (all of them
/// without one): what was typed and done in the TUI. Same mapping as the live stream; turns end at the
/// next prompt, an interrupt, or the TUI's `turn_duration`. Usage is left out (the transcript has no totals).
pub fn history_events(transcript: &str, after: Option<&str>, cwd: Option<&str>) -> Result<Vec<MapOut>, HistoryError> {
    let entries: Vec<Value> =
        transcript.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect();
    let start = match after {
        None => 0,
        Some(a) => {
            entries
                .iter()
                .position(|e| e.get("uuid").and_then(Value::as_str) == Some(a))
                .ok_or(HistoryError::AnchorMissing)?
                + 1
        }
    };
    let mut m = match cwd {
        Some(c) => ClaudeMapper::with_cwd(c),
        None => ClaudeMapper::new(),
    };
    let mut out = Vec::new();
    for e in &entries[start..] {
        if e.get("isSidechain").and_then(Value::as_bool) == Some(true)
            || e.get("isMeta").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        match e.get("type").and_then(Value::as_str).unwrap_or("") {
            "assistant" => out
                .extend(m.on_frame(&json!({"type": "assistant", "message": e["message"], "parent_tool_use_id": null}))),
            "user" => m.history_user(e, &mut out),
            "system" if e.get("subtype").and_then(Value::as_str) == Some("turn_duration") && m.turn.is_some() => {
                m.close_turn(TurnStatus::Completed, e.get("durationMs").and_then(Value::as_i64), None, &mut out);
            }
            _ => {}
        }
    }
    if m.turn.is_some() {
        m.close_turn(TurnStatus::Completed, None, None, &mut out);
    }
    out.retain(|o| matches!(o, MapOut::Event { event, .. } if event.kind() != "usage"));
    Ok(out)
}

impl ClaudeMapper {
    /// One `user` transcript entry: a prompt (a new turn), a tool result, an interrupt or a local command.
    fn history_user(&mut self, e: &Value, out: &mut Vec<MapOut>) {
        let content = &e["message"]["content"];
        let is_result = content
            .as_array()
            .is_some_and(|b| b.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")));
        if is_result {
            let frame = json!({"type": "user", "message": e["message"], "tool_use_result": e.get("toolUseResult")});
            out.extend(self.on_frame(&frame));
            return;
        }
        let text = text_of(content);
        let trimmed = text.trim();
        if trimmed.starts_with("[Request interrupted by user") {
            if self.turn.is_some() {
                self.close_turn(TurnStatus::Interrupted, None, Some("Interrupted".into()), out);
            }
            return;
        }
        if let Some(body) = strip_tag(trimmed, "local-command-stdout") {
            let text = body.to_string();
            out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
            return;
        }
        let uuid = str_field(e, "uuid").unwrap_or_default();
        if let Some(text) = task_notification(e, trimmed) {
            if self.turn.is_some() {
                self.close_turn(TurnStatus::Completed, None, None, out);
            }
            self.start_turn(uuid.clone(), out);
            let item = (!uuid.is_empty()).then(|| format!("u:{uuid}"));
            out.push(self.event(item, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
            return;
        }
        let prompt = command_text(trimmed).unwrap_or_else(|| trimmed.to_string());
        if prompt.is_empty() {
            return;
        }
        if self.turn.is_some() {
            self.close_turn(TurnStatus::Completed, None, None, out);
        }
        self.start_turn(uuid.clone(), out);
        let images = match content {
            Value::Array(b) => b
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("image"))
                .map(|b| b.pointer("/source/media_type").and_then(Value::as_str).unwrap_or("image").to_string())
                .collect(),
            _ => Vec::new(),
        };
        let item = (!uuid.is_empty()).then(|| format!("u:{uuid}"));
        out.push(
            self.event(
                item,
                AgentEvent::UserMessage { text: prompt, images, source: MessageSource::Terminal, pdt: None },
            ),
        );
    }
}

/// One of PDT's own tools: Claude only asks for those PDT gives an ask rule, which wins over any allow rule, so "Always
/// allow" would change nothing (PDT-535).
fn pdt_asks(tool: &str) -> bool {
    tool.strip_prefix("mcp__")
        .and_then(|t| t.strip_prefix(crate::mcp::SERVER_NAME))
        .is_some_and(|t| t.starts_with("__"))
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
    fn clear_shows_a_divider_and_the_next_init_names_the_new_conversation() {
        let mut m = ClaudeMapper::new();
        let out = run(&mut m, include_str!("../../tests/fixtures/claude/clear.jsonl"));
        let ev = events(&out);
        assert!(!ev.iter().any(|(_, e)| e.kind() == "unknown"), "{ev:?}");
        assert_eq!(
            ev.iter().filter(|(_, e)| *e == AgentEvent::ConversationReset { trigger: "clear".into() }).count(),
            1
        );
        let ids: Vec<String> = out
            .iter()
            .filter_map(|o| match o {
                MapOut::Init(i) => i.session_id.clone(),
                _ => None,
            })
            .collect();
        // Not `new_conversation_id` (170715ef…): the transcript and later frames use the init's id.
        assert_eq!(
            ids,
            [
                "144cb04b-8d3f-491e-8ea9-cf6ed4f159bf",
                "90c29dfc-42bf-4be0-816c-6b0ceddf0d62",
                "90c29dfc-42bf-4be0-816c-6b0ceddf0d62"
            ]
        );
        let mut texts: Vec<&str> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::AgentMessage { markdown, .. } => Some(markdown.as_str()),
                _ => None,
            })
            .collect();
        texts.dedup(); // a message row is written again when it is final
        assert_eq!(texts, ["one", "two"]);
    }

    #[test]
    fn a_reset_keeps_its_trigger() {
        let mut m = ClaudeMapper::new();
        let ev = events(&m.on_line(r#"{"type":"conversation_reset","trigger":"resume","new_conversation_id":"n"}"#));
        assert_eq!(ev[0].1, AgentEvent::ConversationReset { trigger: "resume".into() });
    }

    #[test]
    fn pdt_s_asking_tools_ask_every_time() {
        assert!(pdt_asks("mcp__pdt__issue_create"));
        assert!(!pdt_asks("mcp__pdtx__issue_create") && !pdt_asks("mcp__slack__post") && !pdt_asks("Bash"));
        let mut m = ClaudeMapper::new();
        let line = r#"{"type":"control_request","request_id":"q1","request":{"subtype":"can_use_tool","tool_name":"mcp__pdt__issue_create","input":{"title":"x"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"mcp__pdt__issue_create"}],"behavior":"allow","destination":"localSettings"}]}}"#;
        let ev = events(&m.on_line(line));
        let Some(AgentEvent::Request { id, can_always, .. }) =
            ev.iter().map(|(_, e)| e).find(|e| e.kind() == "request")
        else {
            panic!("{ev:?}")
        };
        assert!(!can_always, "an allow rule can't beat PDT's ask rule");
        let (line, _) = m.answer(id, &Answer::Approve { always: true }).unwrap();
        assert_eq!(line["response"]["response"]["behavior"], "allow");
        assert!(line["response"]["response"].get("updatedPermissions").is_none());
    }

    #[test]
    fn a_far_off_rate_limit_reset_saturates() {
        let mut m = ClaudeMapper::new();
        let line = json!({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {
            "five_hour": {"utilization": 0.5, "resetsAt": i64::MAX},
            "seven_day": {"utilization": 0.1, "resetsAt": i64::MIN},
        }}});
        m.on_line(&line.to_string());
        assert_eq!(m.rate_limits["five_hour"].resets_at, Some(i64::MAX));
        assert_eq!(m.rate_limits["seven_day"].resets_at, Some(i64::MIN));
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
        let (line, _) = m.answer("p", &Answer::Plan { approve: false, feedback: None, mode: None }).unwrap();
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
{"type":"system","subtype":"thinking_tokens","estimated_tokens":222,"estimated_tokens_delta":172}
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
    fn progress_frames_are_quiet_and_a_published_change_is_a_notice() {
        let mut m = ClaudeMapper::new();
        let out = run(
            &mut m,
            r#"{"type":"tool_progress","elapsed_time_seconds":30,"heartbeat":true,"tool_name":"Bash","tool_use_id":"t1"}
{"type":"system","subtype":"background_tasks_changed","tasks":[{"description":"Wait for CI","task_id":"b1","task_type":"local_bash"}]}
{"type":"system","subtype":"task_updated","task_id":"b1","patch":{"end_time":1792463355379,"status":"completed"}}
{"type":"system","subtype":"vcs_state_changed","kind":"push","cwd":"/w"}
{"type":"system","subtype":"code_change_published","action":"pushed","identifier":"69","provider":"github","repo":"o/r","url":"https://github.com/o/r/pull/69"}
{"type":"system","subtype":"code_change_published","action":"merged","url":"https://example.com/x"}"#,
        );
        let texts: Vec<_> = events(&out)
            .into_iter()
            .map(|(_, e)| match e {
                AgentEvent::Notice { text, .. } => text,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["Pushed to o/r#69", "Code change merged: https://example.com/x"]);
    }

    #[test]
    fn background_tasks_are_reported_and_their_reports_are_notices() {
        let mut m = ClaudeMapper::new();
        let tasks = |out: &[MapOut]| -> Vec<Vec<BackgroundTask>> {
            out.iter()
                .filter_map(|o| match o {
                    MapOut::BackgroundTasks(t) => Some(t.clone()),
                    _ => None,
                })
                .collect()
        };
        let out = m.on_line(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","task_type":"local_bash","description":"Wait for CI"}]}"#,
        );
        let ci = BackgroundTask { id: "b1".into(), description: "Wait for CI".into(), kind: "local_bash".into() };
        assert_eq!(tasks(&out), [vec![ci]]);
        run(&mut m, r#"{"type":"result","subtype":"success","is_error":false}"#);

        // The task finishes after the turn ended: Claude reports it to itself, which starts a turn.
        let out = run(
            &mut m,
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}
{"type":"system","subtype":"init","model":"opus"}
{"type":"user","message":{"role":"user","content":"<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>Background command \"gh run watch 1 &gt; /dev/null\" completed (exit code 0)</summary>\n</task-notification>"},"uuid":"n1","isReplay":true,"origin":{"kind":"task-notification"}}
{"type":"assistant","message":{"id":"a1","content":[{"type":"text","text":"CI passed."}]},"parent_tool_use_id":null}"#,
        );
        assert_eq!(tasks(&out), [vec![]]);
        let ev = events(&out);
        assert_eq!(ev[0].1.kind(), "turn");
        assert_eq!(
            ev[1],
            (
                Some("u:n1".into()),
                AgentEvent::Notice {
                    level: NoticeLevel::Info,
                    text: r#"Background command "gh run watch 1 > /dev/null" completed (exit code 0)"#.into(),
                    until: None,
                }
            )
        );
        assert!(!ev.iter().any(|(_, e)| e.kind() == "user_message"), "{ev:?}");

        // In a transcript (the TUI's history) it is a notice too.
        let transcript = r#"{"type":"user","uuid":"n2","origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification><task-id>b2</task-id><status>killed</status></task-notification>"}}"#;
        let ev = events(&history_events(transcript, None, None).unwrap());
        assert!(
            ev.iter()
                .any(|(_, e)| matches!(e, AgentEvent::Notice { text, .. } if text == "A background task finished"))
        );
        assert!(!ev.iter().any(|(_, e)| e.kind() == "user_message"), "{ev:?}");
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

    const PLAN: &str = include_str!("../../tests/fixtures/claude/plan.jsonl");
    const TRANSCRIPT: &str = include_str!("../../tests/fixtures/claude/transcript.jsonl");

    fn tool_calls(ev: &[(Option<String>, AgentEvent)], tool: &str) -> Vec<AgentEvent> {
        ev.iter()
            .filter(|(_, e)| matches!(e, AgentEvent::ToolCall { tool: t, .. } if t == tool))
            .map(|(_, e)| e.clone())
            .collect()
    }

    fn diff_of(e: &AgentEvent) -> &str {
        match e {
            AgentEvent::ToolCall { diff, .. } | AgentEvent::Request { diff, .. } => diff.as_deref().unwrap_or(""),
            AgentEvent::TurnDiff { diff } => diff,
            _ => "",
        }
    }

    #[test]
    fn a_recorded_plan_turn_answers_the_question_and_plan_and_diffs_its_edits() {
        let mut m = ClaudeMapper::with_cwd("/work/probe");
        let mut out = Vec::new();
        let mut answered = Vec::new();
        for line in PLAN.lines() {
            let mapped = m.on_line(line);
            for (_, e) in events(&mapped) {
                let AgentEvent::Request { id, request_kind, state: RequestState::Open, input, .. } = e else {
                    continue;
                };
                let answer = match request_kind {
                    RequestKind::Question => {
                        let q = input["questions"][0]["question"].as_str().unwrap().to_string();
                        Answer::Question { answers: BTreeMap::from([(q, vec!["pear".to_string()])]) }
                    }
                    RequestKind::Plan => {
                        Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::AcceptEdits) }
                    }
                    RequestKind::Approval => Answer::Approve { always: false },
                };
                let (line, _) = m.answer(&id, &answer).unwrap();
                answered.push((request_kind, line));
            }
            out.extend(mapped);
        }
        let kinds: Vec<RequestKind> = answered.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, [RequestKind::Question, RequestKind::Plan], "edits ran without asking in Accept edits");
        let plan = &answered[1].1["response"]["response"];
        assert_eq!(
            plan["updatedPermissions"],
            json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}])
        );
        assert!(
            out.iter().any(|o| matches!(o, MapOut::Init(i) if i.permission_mode.as_deref() == Some("acceptEdits"))),
            "system/status reports the new mode at once"
        );

        let ev = events(&out);
        assert!(!ev.iter().any(|(_, e)| e.kind() == "unknown"), "{ev:#?}");
        let edit = tool_calls(&ev, "Edit");
        let done = edit.iter().find(|e| matches!(e, AgentEvent::ToolCall { status: ToolStatus::Done, .. })).unwrap();
        let d = diff_of(done);
        assert!(
            d.starts_with("--- a/notes.txt\n+++ b/notes.txt\n@@ -1,3 +1,3 @@\n line one\n-line two\n+pear\n"),
            "{d}"
        );
        let running =
            edit.iter().find(|e| matches!(e, AgentEvent::ToolCall { status: ToolStatus::Running, .. })).unwrap();
        assert!(diff_of(running).contains("-line two\n+pear"), "a running edit shows its input: {}", diff_of(running));

        let writes = tool_calls(&ev, "Write");
        let fruit = writes.iter().rev().find(|e| diff_of(e).contains("fruit.txt")).unwrap();
        assert!(
            diff_of(fruit).starts_with("--- /dev/null\n+++ b/fruit.txt\n@@ -0,0 +1,1 @@\n+pear"),
            "{}",
            diff_of(fruit)
        );

        let turn_diffs: Vec<&AgentEvent> = ev.iter().map(|(_, e)| e).filter(|e| e.kind() == "turn_diff").collect();
        assert_eq!(turn_diffs.len(), 1);
        let td = diff_of(turn_diffs[0]);
        assert!(td.contains("+++ b/notes.txt") && td.contains("+++ b/fruit.txt"), "{td}");
        assert!(!td.contains("plans/"), "files outside the folder (the plan file) stay out: {td}");
        let pos = |k: &str| ev.iter().rposition(|(_, e)| e.kind() == k).unwrap();
        assert!(pos("turn_diff") < pos("turn"), "the diff comes before the turn ends");
    }

    #[test]
    fn edit_requests_carry_the_change_and_plans_can_switch_mode() {
        let mut m = ClaudeMapper::with_cwd("/w");
        let out = run(
            &mut m,
            r#"{"type":"control_request","request_id":"e","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"/w/src/a.rs","old_string":"let a = 1;","new_string":"let a = 2;\nlet b = 3;"},"tool_use_id":"t1"}}
{"type":"control_request","request_id":"w","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/elsewhere/b.md","content":"hi\n"}}}
{"type":"control_request","request_id":"p","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","input":{"plan":"x"}}}"#,
        );
        let ev = events(&out);
        assert_eq!(diff_of(&ev[0].1), "--- a/src/a.rs\n+++ b/src/a.rs\n@@ @@\n-let a = 1;\n+let a = 2;\n+let b = 3;\n");
        assert_eq!(diff_of(&ev[1].1), "--- /elsewhere/b.md\n+++ /elsewhere/b.md\n@@ @@\n+hi\n");
        assert_eq!(diff_of(&ev[2].1), "");
        let (line, _) = m.answer("p", &Answer::Plan { approve: true, feedback: None, mode: None }).unwrap();
        assert!(line["response"]["response"].get("updatedPermissions").is_none());
        let out =
            m.on_line(r#"{"type":"system","subtype":"status","status":null,"permissionMode":"bypassPermissions"}"#);
        assert!(
            events(&out)
                .iter()
                .any(|(_, e)| matches!(e, AgentEvent::Notice { text, .. } if text == "Permission mode: Bypass"))
        );
        assert!(m.on_line(r#"{"type":"system","subtype":"status","permissionMode":"bypassPermissions"}"#).is_empty());
    }

    #[test]
    fn history_from_a_transcript_becomes_terminal_turns() {
        let out = history_events(TRANSCRIPT, None, Some("/work/probe")).unwrap();
        let ev = events(&out);
        assert!(out.iter().all(|o| matches!(o, MapOut::Event { .. })), "only events: {out:?}");
        assert!(!ev.iter().any(|(_, e)| matches!(e.kind(), "usage" | "unknown")), "{ev:#?}");
        assert!(matches!(&ev[0].1, AgentEvent::Turn { phase: TurnPhase::Started, .. }));
        assert!(
            matches!(&ev[1].1, AgentEvent::UserMessage { source: MessageSource::Terminal, text, .. } if text.starts_with("Use AskUserQuestion"))
        );
        let edit = tool_calls(&ev, "Edit");
        assert!(diff_of(edit.last().unwrap()).contains("@@ -1,3 +1,3 @@"));
        assert!(matches!(
            &ev.last().unwrap().1,
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert!(ev.iter().any(|(_, e)| matches!(e, AgentEvent::AgentMessage { phase: MessagePhase::Final, markdown, .. } if markdown.contains("pear"))));
        assert!(ev.iter().any(|(_, e)| e.kind() == "turn_diff"));

        // After an anchor: only what follows it (here the Write call, its result and the answer).
        let last = last_uuid(TRANSCRIPT).unwrap();
        assert!(history_events(TRANSCRIPT, Some(&last), None).unwrap().is_empty());
        let lines: Vec<&str> = TRANSCRIPT.lines().collect();
        let anchor = lines
            .iter()
            .rev()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["type"] == "user")
            .nth(1)
            .and_then(|v| v["uuid"].as_str().map(String::from))
            .unwrap();
        let tail = events(&history_events(TRANSCRIPT, Some(&anchor), None).unwrap());
        let kinds: Vec<&str> = tail.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(
            kinds,
            ["turn", "tool_call", "tool_call", "agent_message", "agent_message", "turn_diff", "turn"],
            "{tail:#?}"
        );
        assert_eq!(history_events(TRANSCRIPT, Some("nope"), None), Err(HistoryError::AnchorMissing));
    }

    #[test]
    fn history_handles_commands_interrupts_and_turn_durations() {
        let t = r#"{"type":"user","uuid":"a","message":{"role":"user","content":"first"}}
{"type":"assistant","uuid":"b","message":{"id":"m1","content":[{"type":"text","text":"working"}]}}
{"type":"user","uuid":"c","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}
{"type":"user","uuid":"d","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x</local-command-caveat>"}}
{"type":"user","uuid":"e","message":{"role":"user","content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args>opus</command-args>"}}
{"type":"user","uuid":"f","message":{"role":"user","content":"<local-command-stdout>Set model to opus</local-command-stdout>"}}
{"type":"user","uuid":"g","message":{"role":"user","content":"second"}}
{"type":"assistant","uuid":"h","isSidechain":true,"message":{"id":"side","content":[{"type":"text","text":"sub"}]}}
{"type":"assistant","uuid":"i","message":{"id":"m2","content":[{"type":"text","text":"done"}]}}
{"type":"system","uuid":"j","subtype":"turn_duration","durationMs":1500}"#;
        let ev = events(&history_events(t, None, None).unwrap());
        let summary: Vec<String> = ev
            .iter()
            .map(|(_, e)| match e {
                AgentEvent::Turn { phase: TurnPhase::Started, .. } => "start".into(),
                AgentEvent::Turn { status, duration_ms, .. } => format!("end {status:?} {duration_ms:?}"),
                AgentEvent::UserMessage { text, .. } => format!("user {text}"),
                AgentEvent::AgentMessage { markdown, phase, .. } => format!("{phase:?} {markdown}"),
                AgentEvent::Notice { text, .. } => format!("notice {text}"),
                e => e.kind().to_string(),
            })
            .collect();
        assert_eq!(
            summary,
            [
                "start",
                "user first",
                "Commentary working",
                "Final working",
                "end Some(Interrupted) None",
                "start",
                "user /model opus",
                "notice Set model to opus",
                "end Some(Completed) None",
                "start",
                "user second",
                "Commentary done",
                "Final done",
                "end Some(Completed) Some(1500)",
            ]
        );
    }

    #[test]
    fn slash_commands_show_as_sent_whether_or_not_claude_echoes_them() {
        let mut m = ClaudeMapper::new();
        // A custom command comes back expanded.
        m.note_sent_command("/hi there");
        let ev = events(&m.on_line(
            r#"{"type":"user","message":{"role":"user","content":"<command-message>hi</command-message>\n<command-name>/hi</command-name>\n<command-args>there</command-args>"},"uuid":"u1","isReplay":true}"#,
        ));
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { text, .. } if text == "/hi there"), "{ev:?}");
        run(&mut m, r#"{"type":"result","subtype":"success","is_error":false}"#);
        // A local command (`/context`) is never echoed: its turn starts with it.
        m.note_sent_command("/context");
        let ev = events(&m.on_line(
            r#"{"type":"assistant","message":{"id":"m9","content":[{"type":"text","text":"Context usage"}]},"parent_tool_use_id":null}"#,
        ));
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(kinds, ["turn", "user_message", "agent_message"]);
        assert!(
            matches!(&ev[1].1, AgentEvent::UserMessage { text, source: MessageSource::Pdt, .. } if text == "/context")
        );
        assert!(ev[1].0.is_some(), "it has its own row");
        run(&mut m, r#"{"type":"result","subtype":"success","is_error":false}"#);
        // Nothing is left over for the next turn.
        let ev =
            events(&m.on_line(r#"{"type":"assistant","message":{"id":"m10","content":[{"type":"text","text":"x"}]}}"#));
        assert!(!ev.iter().any(|(_, e)| e.kind() == "user_message"));
    }

    #[test]
    fn messages_with_images_send_image_blocks_first() {
        let images = vec![("image/png".to_string(), "AAAA".to_string())];
        let l = user_line("s", "what is this?", &images, false);
        assert_eq!(
            l["message"]["content"],
            json!([{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}},
                   {"type": "text", "text": "what is this?"}])
        );
        let l = user_line("s", "  ", &images, false);
        assert_eq!(l["message"]["content"].as_array().unwrap().len(), 1, "no empty text block");
    }

    #[test]
    fn titles_and_user_lines() {
        assert_eq!(tool_title("Edit", &json!({"file_path": "src/a.rs"})), "src/a.rs");
        assert_eq!(tool_title("Mystery", &json!({"x": 1})), "");
        let t = tool_title(
            "mcp__pdt__create_ticket",
            &json!({"project": "PDT", "title": "Add a logout", "description": "x"}),
        );
        assert_eq!(t, "New ticket in PDT: Add a logout");
        let t = tool_title("mcp__pdt__edit_ticket", &json!({"ticket": "PDT-4", "stage": "ready", "title": null}));
        assert_eq!(t, "Edit PDT-4: stage");
        let t = tool_title("mcp__pdt__stop_ticket", &json!({"ticket": "PDT-4"}));
        assert_eq!(t, "Stop PDT-4's work");
        let t =
            tool_title("mcp__pdt__stop_ticket", &json!({"ticket": "PDT-4", "stage": "ready", "removeWorktree": true}));
        assert_eq!(t, "Stop PDT-4's work, moving it to ready, removing its worktree");
        let long = "x".repeat(300);
        assert_eq!(tool_title("Bash", &json!({"command": long})).chars().count(), 201);
        let l = user_line("s", "hi", &[], true);
        assert_eq!((l["priority"].as_str(), l["message"]["content"].as_str()), (Some("now"), Some("hi")));
        assert!(user_line("s", "hi", &[], false).get("priority").is_none());
    }
}
