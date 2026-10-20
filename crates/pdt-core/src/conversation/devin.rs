//! Devin's `devin acp`, an Agent Client Protocol server (JSON-RPC 2.0 on stdio; notes:
//! docs/superpowers/notes/devin-cli.md): `session/update` notifications and the agent's requests
//! (`session/request_permission`, `elicitation/create`) → normalized events, answers → JSON-RPC results. Pure; the
//! adapter in pdt-agents owns the process.
//!
//! ACP has no message or turn ids: a turn is one `session/prompt` call (plus any prompts sent while it runs, which
//! Devin takes into it and answers together), and an agent message is a run of `agent_message_chunk`s that a tool
//! call, a thought or the turn's end closes.

use super::{
    AgentCommandInfo, AgentEvent, AgentModel, Answer, DeltaField, MAX_TOOL_OUTPUT, MessagePhase, MessageSource,
    PermissionMode, RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus, cap_text,
};
use crate::conversation::claude::AnswerError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// What one message from Devin turns into.
#[derive(Debug, Clone, PartialEq)]
// Short-lived (mapped, then stored or sent at once): boxing the event would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum DevinOut {
    Event {
        item_id: Option<String>,
        turn_id: Option<String>,
        event: AgentEvent,
    },
    Delta {
        item_id: String,
        field: DeltaField,
        text: String,
    },
    /// The session's mode or model, as Devin names them, when it reports either.
    Settings {
        mode: Option<String>,
        model: Option<String>,
    },
    /// The models the session may switch to.
    Models(Vec<AgentModel>),
    /// The slash commands Devin offers.
    Commands(Vec<AgentCommandInfo>),
    /// Answer an agent request now (one PDT does not handle).
    RespondError {
        id: Value,
        message: String,
    },
}

/// An answer to send for an open request.
#[derive(Debug, Clone, PartialEq)]
pub struct DevinAnswer {
    /// The agent request's JSON-RPC id.
    pub rpc_id: Value,
    pub result: Value,
    /// The changed request event.
    pub events: Vec<DevinOut>,
    /// Devin's reject carries no message: the user's note goes to it as the next message.
    pub follow_up: Option<String>,
}

/// The PDT permission modes Devin has. Devin has no mode that asks before edits: its "Code" mode (`accept-edits`)
/// makes edits in the folder without asking and asks before commands it does not judge safe.
pub const DEVIN_MODES: [PermissionMode; 3] =
    [PermissionMode::AcceptEdits, PermissionMode::Plan, PermissionMode::Bypass];

/// Devin's session mode for a PDT permission mode (Ask has no equivalent; Code comes closest).
pub fn devin_mode(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask | PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::Plan => "plan",
        PermissionMode::Bypass => "bypass",
    }
}

/// The PDT mode for Devin's; None for modes PDT does not offer (`smart`, and `ask`, which only answers questions).
pub fn from_devin_mode(mode: &str) -> Option<PermissionMode> {
    match mode {
        "accept-edits" => Some(PermissionMode::AcceptEdits),
        "plan" => Some(PermissionMode::Plan),
        "bypass" => Some(PermissionMode::Bypass),
        _ => None,
    }
}

/// `session/load` and `session/prompt` fail with this when Devin has no saved session by that id (it saves one only
/// at its first prompt).
pub const SESSION_NOT_FOUND: i64 = -32016;

#[derive(Debug, Clone, PartialEq)]
struct Tool {
    tool: String,
    title: String,
    input: Value,
    status: ToolStatus,
    output: Option<String>,
    exit_code: Option<i32>,
    diff: Option<String>,
    /// The file changes the call carries (absolute path, old text, new text).
    edits: Vec<(String, Option<String>, String)>,
    exit_plan: bool,
    turn_id: Option<String>,
}

#[derive(Debug, Clone)]
struct OpenRequest {
    rpc_id: Value,
    kind: RequestKind,
    tool: String,
    title: String,
    input: Value,
    tool_call_id: Option<String>,
    diff: Option<String>,
    /// optionId by kind (`allow_once`, `reject_once`, …), and Devin's session-only "always" option.
    options: Vec<(String, String)>,
    /// Question text → the form field that answers it, and whether it takes several.
    fields: BTreeMap<String, (String, bool)>,
    turn_id: Option<String>,
}

impl OpenRequest {
    fn option(&self, id_or_kind: &str) -> Option<String> {
        self.options.iter().find(|(id, kind)| id == id_or_kind || kind == id_or_kind).map(|(id, _)| id.clone())
    }
    /// Only Devin's "this session" option: the others save a rule to Devin's settings (never from PDT).
    fn can_always(&self) -> bool {
        self.options.iter().any(|(id, _)| id == "allow_session")
    }
}

/// The state of one `devin acp` session's stream.
#[derive(Debug, Default)]
pub struct DevinMapper {
    /// The session's folder (and its canonical form: Devin reports `/private/tmp/…` for `/tmp/…`).
    roots: Vec<String>,
    /// Makes item and turn ids unique across the processes of one session.
    prefix: String,
    next: u64,
    turn: Option<String>,
    /// `session/prompt` calls not answered yet: the turn ends with the last.
    prompts: usize,
    interrupt_requested: bool,
    /// The agent message being streamed: (item id, text so far).
    message: Option<(String, String)>,
    thought: Option<(String, String)>,
    tools: HashMap<String, Tool>,
    requests: HashMap<String, OpenRequest>,
    /// The plan Devin last wrote (plan mode): what an exit-plan request asks to approve.
    plan: Option<String>,
    /// The files the running turn changed inside the folder, in the order first changed: (path, created, hunks).
    turn_files: Vec<(String, bool, String)>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    /// `session/load` is replaying the saved conversation, which PDT has logged already.
    loading: bool,
    /// While loading: replayed turns whose prompt is newer than this (ms) were taken in Devin's terminal and are
    /// imported.
    import_after: Option<i64>,
    /// The replayed turn being imported (its prompt's `clientMessageId`).
    importing: Option<String>,
}

/// An RFC 3339 time (`2026-10-19T23:53:11.255937+00:00`, as Devin stamps messages) in ms since the epoch.
pub fn iso_ms(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { t.get(r)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let frac = t.get(start..i)?;
        ms = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    }
    let offset = match b.get(i) {
        Some(b'Z' | b'z') | None => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let o = num(i + 1..i + 3)? * 60 + num(i + 4..i + 6)?;
            if sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi - offset) * 60_000 + sec * 1000 + ms)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

fn meta<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.get("_meta").and_then(|m| m.get(k))
}

fn one_line(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(200).collect();
    if line.chars().count() > 200 || raw.lines().nth(1).is_some() {
        t.push('…');
    }
    t
}

/// The hunks turning `old` into `new` (a new file when `old` is None), three lines of context.
fn hunks(old: Option<&str>, new: &str) -> String {
    let old = old.unwrap_or("");
    similar::TextDiff::from_lines(old, new).unified_diff().context_radius(3).missing_newline_hint(false).to_string()
}

/// A content block's text (`{type: text}` only).
fn block_text(b: &Value) -> Option<&str> {
    (b.get("type").and_then(Value::as_str) == Some("text")).then(|| b.get("text").and_then(Value::as_str)).flatten()
}

impl DevinMapper {
    /// A mapper for a session working in `cwd` (plus its canonical path, when that differs). `prefix` keeps ids from
    /// this process apart from an earlier one's.
    pub fn new(cwd: impl Into<String>, canonical: Option<String>, prefix: impl Into<String>) -> Self {
        let mut roots = vec![cwd.into()];
        roots.extend(canonical.filter(|c| !roots.contains(c)));
        Self { roots, prefix: prefix.into(), ..Self::default() }
    }

    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    pub fn open_requests(&self) -> usize {
        self.requests.len()
    }

    /// PDT asked to stop the turn: it ends as interrupted.
    pub fn note_interrupt(&mut self) {
        if self.turn.is_some() {
            self.interrupt_requested = true;
        }
    }

    /// `session/load` starts: the history it replays is dropped (PDT has logged it), except turns whose prompt is
    /// newer than `import_after` (ms; 0 = every turn), which were taken in Devin's terminal and are imported.
    pub fn begin_load(&mut self, import_after: Option<i64>) {
        self.loading = true;
        self.import_after = import_after;
    }

    /// `session/load` has answered: the last imported turn ends.
    pub fn end_load(&mut self) -> Vec<DevinOut> {
        let mut out = Vec::new();
        if self.importing.take().is_some() {
            self.close_turn(TurnStatus::Completed, None, &mut out);
        }
        self.loading = false;
        self.import_after = None;
        out
    }

    /// A replayed prompt: it ends the turn being imported, and starts one when it is newer than `import_after`.
    fn replayed_prompt(&mut self, u: &Value) -> Vec<DevinOut> {
        let mut out = Vec::new();
        let client = meta(u, "cognition.ai/clientMessageId").and_then(Value::as_str).map(String::from);
        if client.is_some() && self.importing == client {
            return out;
        }
        if self.importing.take().is_some() {
            self.close_turn(TurnStatus::Completed, None, &mut out);
        }
        let Some(after) = self.import_after else { return out };
        let at = meta(u, "cognition.ai/timestamp").and_then(Value::as_str).and_then(iso_ms);
        if at.is_some_and(|t| t > after) {
            let text = u.get("content").and_then(block_text).unwrap_or_default().to_string();
            self.open_turn(&text, MessageSource::Terminal, &mut out);
            self.importing = Some(client.unwrap_or_default());
        }
        out
    }

    fn open_turn(&mut self, text: &str, source: MessageSource, out: &mut Vec<DevinOut>) {
        let turn = self.id("turn").trim_start_matches("turn:").to_string();
        self.turn = Some(turn.clone());
        self.interrupt_requested = false;
        out.push(DevinOut::Event {
            item_id: Some(format!("turn:{turn}")),
            turn_id: Some(turn.clone()),
            event: AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None },
        });
        out.push(DevinOut::Event {
            item_id: Some(format!("u:{turn}")),
            turn_id: Some(turn),
            event: AgentEvent::UserMessage { text: text.to_string(), images: vec![], source },
        });
    }

    fn id(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}:{}-{}", self.prefix, self.next)
    }

    fn event(&self, item_id: Option<String>, event: AgentEvent) -> DevinOut {
        DevinOut::Event { item_id, turn_id: self.turn.clone(), event }
    }

    /// `path` relative to the session's folder, when inside it.
    fn inside(&self, path: &str) -> Option<String> {
        self.roots.iter().find_map(|root| {
            let root = root.trim_end_matches(['/', '\\']);
            let rest = path.strip_prefix(root)?.strip_prefix(['/', '\\'])?;
            (!rest.is_empty()).then(|| rest.replace('\\', "/"))
        })
    }

    fn shown_path(&self, path: &str) -> String {
        self.inside(path).unwrap_or_else(|| path.to_string())
    }

    fn file_diff(&self, path: &str, old: Option<&str>, new: &str) -> String {
        let p = self.shown_path(path);
        let (a, b) = if p.starts_with('/') || p.as_bytes().get(1) == Some(&b':') {
            (p.clone(), p.clone())
        } else {
            (format!("a/{p}"), format!("b/{p}"))
        };
        let a = if old.is_none() { "/dev/null".to_string() } else { a };
        format!("--- {a}\n+++ {b}\n{}", hunks(old, new))
    }

    /// Closes the streaming message (as said on the way, or the turn's answer) and the thought.
    fn close_text(&mut self, phase: MessagePhase, out: &mut Vec<DevinOut>) {
        if let Some((item, summary)) = self.thought.take()
            && !summary.trim().is_empty()
        {
            out.push(self.event(Some(item), AgentEvent::Reasoning { summary }));
        }
        if let Some((item, markdown)) = self.message.take() {
            let id = item.trim_start_matches("m:").to_string();
            out.push(self.event(Some(item), AgentEvent::AgentMessage { id, markdown, phase }));
        }
    }

    fn tool_event(&self, id: &str, t: &Tool) -> DevinOut {
        DevinOut::Event {
            item_id: Some(format!("t:{id}")),
            turn_id: t.turn_id.clone(),
            event: AgentEvent::ToolCall {
                id: id.to_string(),
                tool: t.tool.clone(),
                title: t.title.clone(),
                input: t.input.clone(),
                status: t.status,
                output: t.output.clone(),
                exit_code: t.exit_code,
                diff: t.diff.clone(),
                parent_id: None,
            },
        }
    }

    fn request_event(&self, id: &str, r: &OpenRequest, state: RequestState, answer: Option<Answer>) -> DevinOut {
        DevinOut::Event {
            item_id: Some(format!("q:{id}")),
            turn_id: r.turn_id.clone(),
            event: AgentEvent::Request {
                id: id.to_string(),
                request_kind: r.kind,
                tool: r.tool.clone(),
                title: r.title.clone(),
                description: None,
                input: r.input.clone(),
                tool_call_id: r.tool_call_id.clone(),
                can_always: r.kind == RequestKind::Approval && r.can_always(),
                diff: r.diff.clone(),
                state,
                answer,
            },
        }
    }

    fn usage_event(&self) -> DevinOut {
        DevinOut::Event {
            item_id: Some("usage".into()),
            turn_id: None,
            event: AgentEvent::Usage {
                context_used: self.context_used,
                context_window: self.context_window,
                rate_limits: vec![],
                cost_usd: None,
            },
        }
    }

    /// PDT is sending a prompt: the user's message, and the start of a turn unless one is running (Devin takes a
    /// prompt sent mid-turn into the running turn).
    pub fn begin_prompt(&mut self, text: &str) -> Vec<DevinOut> {
        self.prompts += 1;
        let mut out = Vec::new();
        let source = if self.turn.is_some() { MessageSource::Steer } else { MessageSource::Pdt };
        if self.turn.is_none() {
            self.open_turn(text, source, &mut out);
        } else {
            self.close_text(MessagePhase::Commentary, &mut out);
            let item = self.id("u");
            out.push(
                self.event(Some(item), AgentEvent::UserMessage { text: text.to_string(), images: vec![], source }),
            );
        }
        out
    }

    /// A `session/prompt` call was answered (`Err`: the JSON-RPC error's message, and whether Devin says a retry may
    /// work). The turn ends with the last prompt's answer.
    pub fn end_prompt(&mut self, result: Result<&Value, (String, bool)>) -> Vec<DevinOut> {
        self.prompts = self.prompts.saturating_sub(1);
        let mut out = Vec::new();
        if self.turn.is_none() {
            return out;
        }
        let (status, reason) = match &result {
            Err((message, retryable)) => {
                out.push(self.event(None, AgentEvent::Error { text: message.clone(), retryable: *retryable }));
                (TurnStatus::Failed, Some(message.clone()))
            }
            Ok(v) => match v.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn") {
                "end_turn" if !self.interrupt_requested => (TurnStatus::Completed, None),
                "end_turn" | "cancelled" => (TurnStatus::Interrupted, None),
                "max_tokens" => (TurnStatus::Failed, Some("Devin hit its output limit".into())),
                "max_turn_requests" => {
                    (TurnStatus::Failed, Some("Devin hit its limit of requests for one turn".into()))
                }
                "refusal" => (TurnStatus::Failed, Some("Devin refused".into())),
                other => (TurnStatus::Failed, Some(other.to_string())),
            },
        };
        if self.prompts > 0 && result.is_ok() {
            return out;
        }
        self.prompts = 0;
        self.close_turn(status, reason, &mut out);
        out
    }

    fn close_turn(&mut self, status: TurnStatus, reason: Option<String>, out: &mut Vec<DevinOut>) {
        self.close_text(MessagePhase::Final, out);
        out.extend(self.expire(true));
        let Some(turn) = self.turn.clone() else { return };
        for (id, t) in self.tools.iter_mut().filter(|(_, t)| t.turn_id.as_deref() == Some(turn.as_str())) {
            if t.status == ToolStatus::Running {
                t.status = if status == TurnStatus::Completed { ToolStatus::Done } else { ToolStatus::Failed };
                let t = t.clone();
                out.push(DevinOut::Event {
                    item_id: Some(format!("t:{id}")),
                    turn_id: t.turn_id.clone(),
                    event: AgentEvent::ToolCall {
                        id: id.clone(),
                        tool: t.tool,
                        title: t.title,
                        input: t.input,
                        status: t.status,
                        output: t.output,
                        exit_code: t.exit_code,
                        diff: t.diff,
                        parent_id: None,
                    },
                });
            }
        }
        let files = std::mem::take(&mut self.turn_files);
        if !files.is_empty() {
            let diff = files
                .iter()
                .map(|(p, created, h)| {
                    let a = if *created { "/dev/null".to_string() } else { format!("a/{p}") };
                    format!("--- {a}\n+++ b/{p}\n{h}")
                })
                .collect::<String>();
            out.push(DevinOut::Event {
                item_id: Some(format!("diff:{turn}")),
                turn_id: Some(turn.clone()),
                event: AgentEvent::TurnDiff { diff },
            });
        }
        out.push(DevinOut::Event {
            item_id: Some(format!("turn-end:{turn}")),
            turn_id: Some(turn),
            event: AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(status), duration_ms: None, reason },
        });
        self.turn = None;
        self.interrupt_requested = false;
        self.tools.clear();
    }

    /// Expires open requests (`all`; else none): the turn ended or the process stopped.
    fn expire(&mut self, all: bool) -> Vec<DevinOut> {
        if !all {
            return vec![];
        }
        let mut ids: Vec<String> = self.requests.keys().cloned().collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                self.requests.remove(&id).map(|r| self.request_event(&id, &r, RequestState::Expired, None))
            })
            .collect()
    }

    /// The turn is being cancelled: every open request gets ACP's "cancelled" answer (the protocol asks clients to
    /// answer them) and expires.
    pub fn cancel_requests(&mut self) -> (Vec<(Value, Value)>, Vec<DevinOut>) {
        let replies = self
            .requests
            .values()
            .map(|r| {
                let result = if r.kind == RequestKind::Question {
                    json!({"action": "cancel"})
                } else {
                    json!({"outcome": {"outcome": "cancelled"}})
                };
                (r.rpc_id.clone(), result)
            })
            .collect();
        (replies, self.expire(true))
    }

    /// The process ended: open requests expire and a running turn ends as interrupted (`reason`).
    pub fn on_exit(&mut self, reason: &str) -> Vec<DevinOut> {
        let mut out = Vec::new();
        if self.turn.is_some() {
            self.close_turn(TurnStatus::Interrupted, Some(reason.to_string()), &mut out);
        } else {
            out.extend(self.expire(true));
        }
        self.prompts = 0;
        out
    }

    /// Maps a `session/update` notification's `update`.
    pub fn on_update(&mut self, u: &Value) -> Vec<DevinOut> {
        let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        match kind {
            "config_option_update" => return self.config_options(u.get("configOptions")),
            "current_mode_update" => {
                return vec![DevinOut::Settings { mode: s(u, "currentModeId"), model: None }];
            }
            "available_commands_update" => {
                let commands = u
                    .get("availableCommands")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(AgentCommandInfo {
                                    name: s(c, "name")?,
                                    description: s(c, "description").unwrap_or_default(),
                                    argument_hint: c.get("input").and_then(|i| s(i, "hint")),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                return vec![DevinOut::Commands(commands)];
            }
            "usage_update" => {
                // Sub-agents report their own context too; the conversation's is the root agent's.
                if meta(u, "cognition.ai/subagent_context").is_some() {
                    return out;
                }
                self.context_used = u.get("used").and_then(Value::as_i64);
                self.context_window = u.get("size").and_then(Value::as_i64);
                return vec![self.usage_event()];
            }
            "user_message_chunk" if self.loading => return self.replayed_prompt(u),
            _ if (self.loading && self.importing.is_none()) || self.turn.is_none() => return out,
            "agent_message_chunk" => {
                let Some(text) = u.get("content").and_then(block_text).map(String::from) else { return out };
                if let Some((item, summary)) = self.thought.take()
                    && !summary.trim().is_empty()
                {
                    out.push(self.event(Some(item), AgentEvent::Reasoning { summary }));
                }
                if self.message.is_none() {
                    let item = self.id("m");
                    self.message = Some((item, String::new()));
                }
                let (item, so_far) = self.message.as_mut().expect("message");
                so_far.push_str(&text);
                out.push(DevinOut::Delta { item_id: item.clone(), field: DeltaField::Markdown, text });
            }
            "agent_thought_chunk" => {
                let Some(text) = u.get("content").and_then(block_text) else { return out };
                if self.message.is_some() {
                    self.close_text(MessagePhase::Commentary, &mut out);
                }
                if self.thought.is_none() {
                    let item = self.id("th");
                    self.thought = Some((item, String::new()));
                }
                self.thought.as_mut().expect("thought").1.push_str(text);
            }
            "tool_call" | "tool_call_update" => {
                let Some(id) = s(u, "toolCallId") else { return out };
                self.close_text(MessagePhase::Commentary, &mut out);
                let changed = self.tool_update(&id, u, kind == "tool_call");
                if let Some(t) = changed {
                    out.push(self.tool_event(&id, &t));
                }
            }
            "plan" => {
                let entries = u.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
                let markdown = entries
                    .iter()
                    .map(|e| {
                        let done = s(e, "status").as_deref() == Some("completed");
                        format!("- [{}] {}\n", if done { "x" } else { " " }, s(e, "content").unwrap_or_default())
                    })
                    .collect::<String>();
                if !markdown.is_empty() {
                    let turn = self.turn.clone().unwrap_or_default();
                    out.push(self.event(Some(format!("plan:{turn}")), AgentEvent::Plan { markdown }));
                }
            }
            _ => {}
        }
        out
    }

    fn config_options(&self, options: Option<&Value>) -> Vec<DevinOut> {
        let options = options.and_then(Value::as_array).cloned().unwrap_or_default();
        let find = |id: &str| options.iter().find(|o| s(o, "id").as_deref() == Some(id));
        let mut out = vec![DevinOut::Settings {
            mode: find("mode").and_then(|o| s(o, "currentValue")),
            model: find("model").and_then(|o| s(o, "currentValue")),
        }];
        if let Some(models) = find("model").and_then(|o| o.get("options")).and_then(Value::as_array) {
            out.push(DevinOut::Models(
                models
                    .iter()
                    .filter_map(|m| {
                        let value = s(m, "value")?;
                        Some(AgentModel {
                            display_name: s(m, "name").unwrap_or_else(|| value.clone()),
                            description: s(m, "description").filter(|d| !d.is_empty()),
                            value,
                        })
                    })
                    .collect(),
            ));
        }
        out
    }

    /// Applies a `tool_call` (new) or `tool_call_update` and returns the call when it changed.
    fn tool_update(&mut self, id: &str, u: &Value, new: bool) -> Option<Tool> {
        let turn = self.turn.clone();
        let name = meta(u, "cognition.ai/inferenceToolName").and_then(Value::as_str).map(String::from);
        let before = self.tools.get(id).cloned();
        let mut t = before.clone().unwrap_or_else(|| Tool {
            tool: name.clone().or_else(|| s(u, "kind")).unwrap_or_else(|| "tool".into()),
            title: String::new(),
            input: Value::Null,
            status: ToolStatus::Running,
            output: None,
            exit_code: None,
            diff: None,
            edits: vec![],
            exit_plan: false,
            turn_id: turn,
        });
        if new && let Some(n) = name {
            t.tool = n;
        }
        if let Some(input) = u.get("rawInput") {
            t.input = input.clone();
        }
        if meta(u, "cognition.ai/isExitPlan").and_then(Value::as_bool) == Some(true) || t.tool == "exit_plan_mode" {
            t.exit_plan = true;
        }
        let command = t.input.get("command").and_then(Value::as_str).map(one_line);
        let path = t.input.get("file_path").and_then(Value::as_str).map(|p| self.shown_path(p));
        let title = command.or(path).or_else(|| s(u, "title").map(|x| one_line(&x)));
        if let Some(title) = title.filter(|x| !x.is_empty()) {
            t.title = title;
        }
        if let Some(content) = u.get("content").and_then(Value::as_array) {
            let mut texts = Vec::new();
            let mut edits = Vec::new();
            for c in content {
                match c.get("type").and_then(Value::as_str) {
                    Some("diff") => {
                        if let (Some(p), Some(n)) = (s(c, "path"), s(c, "newText")) {
                            edits.push((p, s(c, "oldText"), n));
                        }
                    }
                    Some("content") => {
                        if let Some(x) = c.get("content").and_then(block_text) {
                            texts.push(x.to_string());
                        }
                    }
                    _ => {}
                }
            }
            if !texts.is_empty() {
                // Devin sends a command's whole output so far each time, not the new part.
                t.output = Some(cap_text(texts.join("\n").trim_end(), MAX_TOOL_OUTPUT));
            }
            if !edits.is_empty() {
                t.diff = Some(edits.iter().map(|(p, o, n)| self.file_diff(p, o.as_deref(), n)).collect::<String>());
                t.edits = edits;
            }
        }
        if t.tool == "write_plan"
            && let Some(plan) = t.input.get("content").and_then(Value::as_str)
        {
            self.plan = Some(plan.to_string());
        }
        if let Some(code) = meta(u, "terminal_exit").and_then(|e| e.get("exit_code")).and_then(Value::as_i64) {
            t.exit_code = i32::try_from(code).ok();
        }
        // A call that failed or was refused stays so (Devin can report it completed after cancelling it).
        if !matches!(t.status, ToolStatus::Failed | ToolStatus::Declined) {
            match s(u, "status").as_deref() {
                Some("completed") => t.status = ToolStatus::Done,
                Some("failed") if meta(u, "cognition.ai/rejected").and_then(Value::as_bool) == Some(true) => {
                    t.status = ToolStatus::Declined;
                }
                Some("failed") => t.status = ToolStatus::Failed,
                _ => {}
            }
            if t.status == ToolStatus::Done && before.as_ref().is_none_or(|b| b.status != ToolStatus::Done) {
                self.note_turn_edits(&t);
            }
        }
        self.tools.insert(id.to_string(), t.clone());
        (before.as_ref() != Some(&t)).then_some(t)
    }

    /// Adds a finished call's edits inside the folder to the turn's diff.
    fn note_turn_edits(&mut self, t: &Tool) {
        for (path, old, new) in &t.edits {
            let Some(rel) = self.inside(path) else { continue };
            let h = hunks(old.as_deref(), new);
            match self.turn_files.iter_mut().find(|(p, ..)| *p == rel) {
                Some(f) => f.2.push_str(&h),
                None => self.turn_files.push((rel, old.is_none(), h)),
            }
        }
    }

    /// Maps an agent request: approvals, plans and questions become requests; the rest are refused.
    pub fn on_request(&mut self, rpc_id: Value, method: &str, p: &Value) -> Vec<DevinOut> {
        let id = match &rpc_id {
            Value::String(x) => x.clone(),
            other => other.to_string(),
        };
        let r = match method {
            "session/request_permission" => {
                let tool_call_id = p.get("toolCall").and_then(|t| s(t, "toolCallId"));
                let tool = tool_call_id.as_deref().and_then(|t| self.tools.get(t)).cloned();
                let options: Vec<(String, String)> = p
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter().filter_map(|o| Some((s(o, "optionId")?, s(o, "kind").unwrap_or_default()))).collect()
                    })
                    .unwrap_or_default();
                let plan =
                    tool.as_ref().is_some_and(|t| t.exit_plan) || options.iter().any(|(id, _)| id.starts_with("plan_"));
                if plan {
                    let markdown = self
                        .plan
                        .clone()
                        .or_else(|| tool.as_ref().and_then(|t| s(&t.input, "plan")))
                        .unwrap_or_default();
                    OpenRequest {
                        rpc_id,
                        kind: RequestKind::Plan,
                        tool: "ExitPlanMode".into(),
                        title: "Plan".into(),
                        input: json!({"plan": markdown}),
                        tool_call_id,
                        diff: None,
                        options,
                        fields: BTreeMap::new(),
                        turn_id: self.turn.clone(),
                    }
                } else {
                    let (name, title, input, diff) = match &tool {
                        Some(t) => (t.tool.clone(), t.title.clone(), t.input.clone(), t.diff.clone()),
                        None => ("tool".into(), String::new(), Value::Null, None),
                    };
                    OpenRequest {
                        rpc_id,
                        kind: RequestKind::Approval,
                        tool: name,
                        title,
                        input,
                        tool_call_id,
                        diff,
                        options,
                        fields: BTreeMap::new(),
                        turn_id: self.turn.clone(),
                    }
                }
            }
            "elicitation/create" if p.get("mode").and_then(Value::as_str).unwrap_or("form") == "form" => {
                let (questions, fields) = questions(p);
                OpenRequest {
                    rpc_id,
                    kind: RequestKind::Question,
                    tool: "AskUserQuestion".into(),
                    title: questions
                        .first()
                        .and_then(|q| q["question"].as_str())
                        .map(String::from)
                        .or_else(|| s(p, "message"))
                        .unwrap_or_default(),
                    input: json!({"questions": questions}),
                    tool_call_id: None,
                    diff: None,
                    options: vec![],
                    fields,
                    turn_id: self.turn.clone(),
                }
            }
            _ => return vec![DevinOut::RespondError { id: rpc_id, message: format!("PDT does not handle {method}") }],
        };
        let out = vec![self.request_event(&id, &r, RequestState::Open, None)];
        self.requests.insert(id, r);
        out
    }

    /// The JSON-RPC result answering an open request, and the events it changes.
    pub fn answer(&mut self, request_id: &str, answer: &Answer) -> Result<DevinAnswer, AnswerError> {
        let r = self.requests.get(request_id).ok_or(AnswerError::Closed)?;
        let note = |m: &Option<String>| m.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(String::from);
        let selected = |id: Option<String>| -> Result<Value, AnswerError> {
            let id = id.ok_or(AnswerError::WrongAnswer)?;
            Ok(json!({"outcome": {"outcome": "selected", "optionId": id}}))
        };
        let (result, follow_up) = match (r.kind, answer) {
            (RequestKind::Question, Answer::Approve { .. } | Answer::Plan { .. })
            | (RequestKind::Approval, Answer::Question { .. } | Answer::Plan { .. })
            | (RequestKind::Plan, Answer::Question { .. }) => return Err(AnswerError::WrongAnswer),
            (RequestKind::Question, Answer::Deny { .. }) => (json!({"action": "decline"}), None),
            (RequestKind::Question, Answer::Question { answers }) => {
                let mut content = serde_json::Map::new();
                for (q, picked) in answers {
                    let Some((field, multi)) = r.fields.get(q) else { continue };
                    let v = if *multi { json!(picked) } else { json!(picked.first().cloned().unwrap_or_default()) };
                    content.insert(field.clone(), v);
                }
                (json!({"action": "accept", "content": content}), None)
            }
            (RequestKind::Approval, Answer::Approve { always }) => {
                let id = if *always && r.can_always() { r.option("allow_session") } else { r.option("allow_once") };
                (selected(id)?, None)
            }
            (RequestKind::Plan, Answer::Approve { .. })
            | (RequestKind::Plan, Answer::Plan { approve: true, mode: None, .. }) => {
                (selected(r.option("plan_accept_edits").or_else(|| r.option("allow_once")))?, None)
            }
            (RequestKind::Plan, Answer::Plan { approve: true, mode: Some(m), .. }) => {
                let id = match m {
                    PermissionMode::Bypass => r.option("plan_bypass"),
                    _ => r.option("plan_accept_edits"),
                };
                (selected(id.or_else(|| r.option("allow_once")))?, None)
            }
            (_, Answer::Deny { message }) => (selected(r.option("reject_once"))?, note(message)),
            (_, Answer::Plan { approve: false, feedback, .. }) => (selected(r.option("reject_once"))?, note(feedback)),
        };
        let r = self.requests.remove(request_id).expect("checked above");
        let events = vec![self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()))];
        Ok(DevinAnswer { rpc_id: r.rpc_id, result, events, follow_up })
    }
}

/// An elicitation form's fields as PDT questions (`{question, header, options, multiSelect}`, as Claude's
/// AskUserQuestion has them), with each question's field. Devin names fields `q0`, `q1`, … in order.
fn questions(p: &Value) -> (Vec<Value>, BTreeMap<String, (String, bool)>) {
    let schema = p.get("requestedSchema").cloned().unwrap_or_default();
    let props = schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut keys: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|k| k.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut rest: Vec<String> = props.keys().filter(|k| !keys.contains(k)).cloned().collect();
    // q2 before q10.
    rest.sort_by_key(|k| {
        (k.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().unwrap_or(u64::MAX), k.clone())
    });
    keys.extend(rest);
    let mut out = Vec::new();
    let mut fields = BTreeMap::new();
    let only = props.len() == 1;
    for key in keys {
        let Some(f) = props.get(&key) else { continue };
        let multi = f.get("type").and_then(Value::as_str) == Some("array");
        let choices = if multi { f.get("items").cloned().unwrap_or_default() } else { f.clone() };
        let options: Vec<Value> =
            if let Some(one_of) = choices.get("oneOf").or(choices.get("anyOf")).and_then(Value::as_array) {
                one_of
                    .iter()
                    .filter_map(|o| {
                        let label = o.get("const").and_then(Value::as_str)?;
                        let description = s(o, "title").filter(|t| t != label);
                        Some(json!({"label": label, "description": description}))
                    })
                    .collect()
            } else {
                choices
                    .get("enum")
                    .and_then(Value::as_array)
                    .map(|e| e.iter().filter_map(Value::as_str).map(|l| json!({"label": l})).collect())
                    .unwrap_or_default()
            };
        let question = s(f, "description")
            .or_else(|| only.then(|| s(p, "message")).flatten())
            .or_else(|| s(f, "title"))
            .unwrap_or_else(|| key.clone());
        let header = s(f, "title").filter(|h| *h != question);
        out.push(json!({"question": question, "header": header, "options": options, "multiSelect": multi}));
        fields.insert(question, (key, multi));
    }
    (out, fields)
}

/// The `session/prompt` content for a message: images (`(media type, base64 data)`) before the text.
pub fn prompt_blocks(text: &str, images: &[(String, String)]) -> Value {
    let mut blocks: Vec<Value> =
        images.iter().map(|(media, data)| json!({"type": "image", "mimeType": media, "data": data})).collect();
    if !text.trim().is_empty() || blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": text}));
    }
    Value::Array(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDIT: &str = include_str!("../../tests/fixtures/devin/edit.jsonl");
    const APPROVAL: &str = include_str!("../../tests/fixtures/devin/approval.jsonl");
    const QUESTION_PLAN: &str = include_str!("../../tests/fixtures/devin/question_plan.jsonl");
    const CANCEL: &str = include_str!("../../tests/fixtures/devin/cancel.jsonl");

    const CWD: &str = "/tmp/acp-probe/repo";

    fn mapper() -> DevinMapper {
        DevinMapper::new(CWD, Some(format!("/private{CWD}")), "p")
    }

    /// Feeds recorded frames as the adapter would: updates, agent requests (answered with `answer` when given) and
    /// prompt results. `prompt` sends each prompt's text before the first frame of its turn.
    fn feed(m: &mut DevinMapper, frames: &str, mut answer: impl FnMut(&AgentEvent) -> Option<Answer>) -> Vec<DevinOut> {
        let mut out = Vec::new();
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            if m.turn().is_none() && (v.get("method").is_some_and(|x| x == "session/update")) {
                let k = v["params"]["update"]["sessionUpdate"].as_str().unwrap_or("");
                if !matches!(
                    k,
                    "config_option_update"
                        | "current_mode_update"
                        | "available_commands_update"
                        | "usage_update"
                        | "session_info_update"
                ) {
                    out.extend(m.begin_prompt("go"));
                }
            }
            match (v.get("method").and_then(Value::as_str), v.get("id")) {
                (Some("session/update"), None) => out.extend(m.on_update(&v["params"]["update"])),
                (Some(_), None) => {}
                (Some(method), Some(id)) => {
                    let evs = m.on_request(id.clone(), method, &v["params"]);
                    let ask = evs.iter().find_map(|e| match e {
                        DevinOut::Event { event: ev @ AgentEvent::Request { .. }, .. } => Some(ev.clone()),
                        _ => None,
                    });
                    out.extend(evs);
                    if let Some(ev) = ask
                        && let AgentEvent::Request { id, .. } = &ev
                        && let Some(a) = answer(&ev)
                    {
                        let a = m.answer(id, &a).unwrap();
                        out.push(DevinOut::Event {
                            item_id: None,
                            turn_id: None,
                            event: AgentEvent::Unknown { raw: json!({"sent": a.result, "followUp": a.follow_up}) },
                        });
                        out.extend(a.events);
                    }
                }
                (None, Some(_)) => {
                    if let Some(r) = v.get("result") {
                        out.extend(m.end_prompt(Ok(r)));
                    } else {
                        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
                        out.extend(m.end_prompt(Err((msg, false))));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn events(out: &[DevinOut]) -> Vec<(Option<String>, AgentEvent)> {
        out.iter()
            .filter_map(|o| match o {
                DevinOut::Event { item_id, event, .. } => Some((item_id.clone(), event.clone())),
                _ => None,
            })
            .collect()
    }

    fn last_tool<'a>(ev: &'a [(Option<String>, AgentEvent)], tool: &str) -> &'a AgentEvent {
        ev.iter()
            .rev()
            .map(|(_, e)| e)
            .find(|e| matches!(e, AgentEvent::ToolCall { tool: t, .. } if t == tool))
            .unwrap()
    }

    #[test]
    fn a_recorded_turn_maps_to_messages_tools_diffs_and_its_end() {
        let mut m = mapper();
        let out = feed(&mut m, EDIT, |_| None);
        let ev = events(&out);
        assert!(matches!(&ev[0].1, AgentEvent::Turn { phase: TurnPhase::Started, .. }));
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { text, source: MessageSource::Pdt, .. } if text == "go"));
        // Thoughts become reasoning; the closing message is the turn's answer, streamed first.
        assert!(
            ev.iter()
                .any(|(_, e)| matches!(e, AgentEvent::Reasoning { summary } if summary.contains("user wants me to")))
        );
        let deltas: String = out
            .iter()
            .filter_map(|o| match o {
                DevinOut::Delta { field: DeltaField::Markdown, text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Done. 1 file (a.txt).");
        assert!(ev.iter().any(|(_, e)| matches!(e,
            AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } if markdown == "Done. 1 file (a.txt).")));
        // The read names the file inside the folder (Devin reports the /private path).
        let AgentEvent::ToolCall { title, status, .. } = last_tool(&ev, "read") else { panic!() };
        assert_eq!((title.as_str(), *status), ("a.txt", ToolStatus::Done));
        let AgentEvent::ToolCall { title, status, diff, .. } = last_tool(&ev, "edit") else { panic!() };
        assert_eq!((title.as_str(), *status), ("a.txt", ToolStatus::Done));
        assert_eq!(diff.as_deref(), Some("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-hello\n+goodbye\n"));
        let AgentEvent::ToolCall { title, output, exit_code, .. } = last_tool(&ev, "exec") else { panic!() };
        assert_eq!(title, "ls -la");
        assert_eq!(*exit_code, Some(0));
        assert!(output.as_deref().unwrap().ends_with("a.txt"), "{output:?}");
        let n = ev.len();
        assert!(matches!(&ev[n - 2].1, AgentEvent::TurnDiff { diff } if diff.contains("+goodbye")));
        assert!(matches!(
            &ev[n - 1].1,
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert!(ev.iter().any(|(_, e)| matches!(
            e,
            AgentEvent::Usage { context_used: Some(12029), context_window: Some(200000), .. }
        )));
        assert_eq!(m.turn(), None);
        // Tool calls update one row each.
        let edits = ev.iter().filter(|(i, _)| i.as_deref() == Some("t:call_136003b5d4fb4aa7a1412539")).count();
        assert!(edits >= 2);
    }

    #[test]
    fn approvals_offer_the_session_option_and_rejects_are_declined_with_a_note() {
        let mut m = mapper();
        let mut n = 0;
        let out = feed(&mut m, APPROVAL, |e| {
            n += 1;
            let AgentEvent::Request { request_kind, can_always, title, tool, .. } = e else { return None };
            assert_eq!(*request_kind, RequestKind::Approval);
            assert!(*can_always);
            assert_eq!((title.as_str(), tool.as_str()), ("touch b.txt", "exec"));
            Some(if n == 1 {
                Answer::Deny { message: Some(" not now ".into()) }
            } else {
                Answer::Approve { always: true }
            })
        });
        let ev = events(&out);
        let sent: Vec<&Value> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Unknown { raw } => Some(raw),
                _ => None,
            })
            .collect();
        assert_eq!(sent[0]["sent"]["outcome"]["optionId"], "reject_once");
        assert_eq!(sent[0]["followUp"], "not now");
        assert_eq!(sent[1]["sent"]["outcome"]["optionId"], "allow_session", "never a persistent rule");
        let declined = ev.iter().any(|(_, e)| matches!(e, AgentEvent::ToolCall { status: ToolStatus::Declined, .. }));
        assert!(declined);
        let ends = ev
            .iter()
            .filter(|(_, e)| {
                matches!(e, AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. })
            })
            .count();
        assert_eq!(ends, 2);
        assert!(m.answer("nope", &Answer::Approve { always: false }).is_err());
    }

    #[test]
    fn questions_and_plans_become_requests_answered_in_devins_terms() {
        let mut m = mapper();
        let out = feed(&mut m, QUESTION_PLAN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                assert_eq!(title, "Which color do you prefer?");
                assert_eq!(input["questions"][0]["options"][1]["label"], "Blue");
                assert_eq!(input["questions"][0]["header"], "Color preference");
                let answers = BTreeMap::from([("Which color do you prefer?".to_string(), vec!["Blue".to_string()])]);
                Some(Answer::Question { answers })
            }
            AgentEvent::Request { request_kind: RequestKind::Plan, input, .. } => {
                assert!(input["plan"].as_str().unwrap().starts_with("# Add c.txt"), "{input}");
                Some(Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::Bypass) })
            }
            _ => None,
        });
        let ev = events(&out);
        let sent: Vec<&Value> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Unknown { raw } => Some(&raw["sent"]),
                _ => None,
            })
            .collect();
        assert_eq!(sent[0], &json!({"action": "accept", "content": {"q0": "Blue"}}));
        assert_eq!(sent[1], &json!({"outcome": {"outcome": "selected", "optionId": "plan_bypass"}}));
        // Mode changes are reported for the runtime; the plan file outside the folder stays out of the turn's diff.
        assert!(out.iter().any(|o| matches!(o, DevinOut::Settings { mode: Some(m), .. } if m == "accept-edits")));
        let diffs: Vec<&String> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::TurnDiff { diff } => Some(diff),
                _ => None,
            })
            .collect();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0], "--- /dev/null\n+++ b/c.txt\n@@ -0,0 +1 @@\n+Blue\n");
    }

    #[test]
    fn a_cancelled_turn_ends_interrupted_and_its_tool_stays_failed() {
        let mut m = mapper();
        let out = feed(&mut m, CANCEL, |_| Some(Answer::Approve { always: false }));
        let ev = events(&out);
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "exec") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed, "the late `completed` does not undo the cancel");
        assert!(matches!(&ev.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
    }

    #[test]
    fn prompts_sent_mid_turn_join_it_and_the_turn_ends_with_the_last_answer() {
        let mut m = mapper();
        m.begin_prompt("first");
        let steer = events(&m.begin_prompt("second"));
        assert!(matches!(&steer[0].1, AgentEvent::UserMessage { source: MessageSource::Steer, .. }));
        let done = json!({"stopReason": "end_turn"});
        assert!(events(&m.end_prompt(Ok(&done))).is_empty());
        let end = events(&m.end_prompt(Ok(&done)));
        assert!(matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Completed), .. }));
        // A late answer for a turn already ended maps to nothing.
        assert!(m.end_prompt(Ok(&done)).is_empty());
        // An error ends the turn as failed, with the error shown.
        m.begin_prompt("x");
        let end = events(&m.end_prompt(Err(("Quota exhausted".into(), true))));
        assert!(matches!(&end[0].1, AgentEvent::Error { retryable: true, .. }));
        assert!(
            matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Failed), reason: Some(r), .. } if r == "Quota exhausted")
        );
        // Interrupting: requests are answered as cancelled, and the turn ends interrupted even on end_turn.
        m.begin_prompt("y");
        m.on_request(
            json!("r1"),
            "session/request_permission",
            &json!({"toolCall": {"toolCallId": "c"}, "options": [{"optionId": "allow_once", "kind": "allow_once"}]}),
        );
        m.note_interrupt();
        let (replies, expired) = m.cancel_requests();
        assert_eq!(replies, vec![(json!("r1"), json!({"outcome": {"outcome": "cancelled"}}))]);
        assert!(matches!(&events(&expired)[0].1, AgentEvent::Request { state: RequestState::Expired, .. }));
        let end = events(&m.end_prompt(Ok(&done)));
        assert!(matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
    }

    #[test]
    fn replayed_history_is_dropped_but_settings_and_commands_are_kept() {
        let mut m = mapper();
        m.begin_load(None);
        assert!(
            m.on_update(&json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "old"}}))
                .is_empty()
        );
        assert!(
            m.on_update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "old"}}))
                .is_empty()
        );
        let out = m.on_update(&json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "compact", "description": "Force conversation compaction"},
            {"name": "plan", "description": "Plan", "input": {"hint": "[prompt]"}}]}));
        assert!(
            matches!(&out[0], DevinOut::Commands(c) if c.len() == 2 && c[1].argument_hint.as_deref() == Some("[prompt]"))
        );
        let out = m.on_update(&json!({"sessionUpdate": "config_option_update", "configOptions": [
            {"id": "mode", "currentValue": "plan"},
            {"id": "model", "currentValue": "swe-1-6-slow", "options": [{"value": "swe-1-6-slow", "name": "SWE-1.6 Slow"}]}]}));
        assert_eq!(out[0], DevinOut::Settings { mode: Some("plan".into()), model: Some("swe-1-6-slow".into()) });
        assert!(matches!(&out[1], DevinOut::Models(v) if v[0].display_name == "SWE-1.6 Slow"));
        assert!(m.end_load().is_empty());
        // Unknown agent requests are refused.
        let out = m.on_request(json!(7), "fs/read_text_file", &json!({}));
        assert!(matches!(&out[0], DevinOut::RespondError { id, .. } if *id == json!(7)));
        // The process stopping ends a running turn as interrupted.
        m.begin_prompt("z");
        let end = events(&m.on_exit("Devin stopped"));
        assert!(
            matches!(&end.last().unwrap().1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), reason: Some(r), .. } if r == "Devin stopped")
        );
    }

    #[test]
    fn turns_taken_in_the_terminal_are_imported_from_the_replay() {
        let chunk = |kind: &str, text: &str, id: &str, at: &str| {
            json!({"sessionUpdate": kind, "content": {"type": "text", "text": text},
                   "_meta": {"cognition.ai/clientMessageId": id, "cognition.ai/timestamp": at}})
        };
        let switched = iso_ms("2026-10-19T23:00:00+00:00").unwrap();
        let mut m = mapper();
        m.begin_load(Some(switched));
        let mut out = Vec::new();
        out.extend(m.on_update(&chunk("user_message_chunk", "old", "a", "2026-10-19T22:10:22.692914+00:00")));
        out.extend(m.on_update(&chunk("agent_message_chunk", "old answer", "", "2026-10-19T22:10:26+00:00")));
        out.extend(m.on_update(&chunk("user_message_chunk", "Reply PINEAPPLE", "b", "2026-10-19T23:53:11.25+00:00")));
        out.extend(m.on_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Ran ls", "kind": "execute",
            "rawInput": {"command": "ls"}, "_meta": {"cognition.ai/inferenceToolName": "exec"}}),
        ));
        out.extend(
            m.on_update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed"})),
        );
        out.extend(m.on_update(&chunk("agent_message_chunk", "PINEAPPLE", "", "2026-10-19T23:53:12+00:00")));
        out.extend(m.on_update(&chunk("user_message_chunk", "again", "c", "2026-10-19T23:54:00Z")));
        out.extend(m.on_update(&chunk("agent_message_chunk", "ok", "", "2026-10-19T23:54:01Z")));
        out.extend(m.end_load());
        let ev = events(&out);
        let said: Vec<String> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::UserMessage { text, source: MessageSource::Terminal, .. } => Some(format!("> {text}")),
                AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } => Some(markdown.clone()),
                AgentEvent::ToolCall { title, status: ToolStatus::Done, .. } => Some(format!("$ {title}")),
                AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. } => {
                    Some("end".into())
                }
                _ => None,
            })
            .collect();
        assert_eq!(said, ["> Reply PINEAPPLE", "$ ls", "PINEAPPLE", "end", "> again", "ok", "end"]);
        assert_eq!(m.turn(), None);
        // Without an anchor nothing is imported.
        let mut m = mapper();
        m.begin_load(None);
        assert!(m.on_update(&chunk("user_message_chunk", "x", "a", "2030-01-01T00:00:00Z")).is_empty());
        assert!(m.on_update(&chunk("agent_message_chunk", "y", "", "2030-01-01T00:00:00Z")).is_empty());
        assert!(m.end_load().is_empty());
    }

    #[test]
    fn rfc3339_times_become_ms() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2026-10-19T23:53:11.255937+00:00"), Some(1_792_453_991_255));
        assert_eq!(iso_ms("2000-02-29T12:00:00-05:00"), Some(951_843_600_000));
        assert_eq!(iso_ms("2026-10-19T23:53:11.2Z"), Some(1_792_453_991_200));
        assert_eq!(iso_ms("yesterday"), None);
    }

    #[test]
    fn modes_map_both_ways_and_prompts_put_images_first() {
        assert_eq!(devin_mode(PermissionMode::Ask), "accept-edits");
        for m in DEVIN_MODES {
            assert_eq!(from_devin_mode(devin_mode(m)), Some(m));
        }
        assert_eq!(from_devin_mode("smart"), None);
        let b = prompt_blocks("look", &[("image/png".into(), "AAAA".into())]);
        assert_eq!(
            b,
            json!([{"type": "image", "mimeType": "image/png", "data": "AAAA"}, {"type": "text", "text": "look"}])
        );
        assert_eq!(prompt_blocks("", &[("image/png".into(), "A".into())]).as_array().unwrap().len(), 1);
    }

    #[test]
    fn multi_select_questions_answer_with_lists() {
        let mut m = mapper();
        m.begin_prompt("q");
        let p = json!({"mode": "form", "message": "Pick", "requestedSchema": {"type": "object", "properties": {
            "q0": {"type": "array", "title": "Fruit", "description": "Which fruit?", "items": {"anyOf": [{"const": "Apple"}, {"const": "Pear"}]}},
            "q1": {"type": "string", "title": "Why", "description": "Why?"}}, "required": ["q0"]}});
        let out = m.on_request(json!("e1"), "elicitation/create", &p);
        let AgentEvent::Request { input, .. } = &events(&out)[0].1 else { panic!() };
        assert_eq!(input["questions"][0]["multiSelect"], true);
        assert_eq!(input["questions"][1]["question"], "Why?");
        let answers = BTreeMap::from([
            ("Which fruit?".to_string(), vec!["Apple".to_string(), "Pear".to_string()]),
            ("Why?".to_string(), vec!["tasty".to_string()]),
        ]);
        let a = m.answer("e1", &Answer::Question { answers }).unwrap();
        assert_eq!(a.result, json!({"action": "accept", "content": {"q0": ["Apple", "Pear"], "q1": "tasty"}}));
        let out = m.on_request(json!("e2"), "elicitation/create", &p);
        assert!(!out.is_empty());
        assert_eq!(m.answer("e2", &Answer::Deny { message: None }).unwrap().result, json!({"action": "decline"}));
    }
}
