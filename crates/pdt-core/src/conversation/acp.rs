//! The Agent Client Protocol (JSON-RPC 2.0 on stdio, <https://agentclientprotocol.com>): `session/update`
//! notifications and the agent's requests (`session/request_permission`, `elicitation/create`) → normalized events,
//! answers → JSON-RPC results. Pure; the adapter in pdt-agents owns the process. What one agent adds to plain ACP (its
//! extra fields, its option ids, its tool names) comes from its [`AcpExtensions`] (Devin's: `super::devin`).
//!
//! ACP has no message or turn ids: a turn is one `session/prompt` call (plus any prompts sent while it runs, which
//! the agent takes into it and answers together), and an agent message is a run of `agent_message_chunk`s that a
//! tool call, a thought or the turn's end closes.

use super::{
    AgentCommandInfo, AgentEvent, AgentModel, Answer, DeltaField, MAX_TOOL_OUTPUT, MessagePhase, MessageSource,
    PermissionMode, RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus, cap_text,
};
use crate::conversation::claude::AnswerError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// What one message from the agent turns into.
#[derive(Debug, Clone, PartialEq)]
// Short-lived (mapped, then stored or sent at once): boxing the event would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum AcpOut {
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
    /// The session's mode or model, as the agent names them, when it reports either.
    Settings {
        mode: Option<String>,
        model: Option<String>,
    },
    /// The models the session may switch to.
    Models(Vec<AgentModel>),
    /// The slash commands the agent offers.
    Commands(Vec<AgentCommandInfo>),
    /// Answer an agent request now (one PDT does not handle).
    RespondError {
        id: Value,
        message: String,
    },
    /// Answer an agent request now with a result (PDT's own tools need no approval, Phase 7b).
    Respond {
        id: Value,
        result: Value,
    },
}

/// An answer to send for an open request.
#[derive(Debug, Clone, PartialEq)]
pub struct AcpAnswer {
    /// The agent request's JSON-RPC id.
    pub rpc_id: Value,
    pub result: Value,
    /// The changed request event.
    pub events: Vec<AcpOut>,
    /// ACP's reject carries no message: the user's note goes to the agent as the next message.
    pub follow_up: Option<String>,
}

/// What one agent adds to plain ACP, for the mapper: the extra fields it sends and the names it gives things. Each
/// method's default is plain ACP's answer.
pub trait AcpExtensions: std::fmt::Debug + Send {
    /// The agent's name, in what PDT says about it ("Devin hit its output limit").
    fn name(&self) -> &str {
        "The agent"
    }

    /// The agent reports paths through symlinks resolved (`/private/tmp/…` for `/tmp/…` on macOS): the session's
    /// folder is matched in its canonical form too.
    fn resolves_symlinks(&self) -> bool {
        false
    }

    /// A tool call's name from the agent's extra fields (plain ACP has only the call's `kind`).
    fn tool_name(&self, _update: &Value) -> Option<String> {
        None
    }

    /// The call is the agent asking to leave plan mode (`tool`: its name).
    fn is_exit_plan(&self, _update: &Value, _tool: &str) -> bool {
        false
    }

    /// The plan a call writes (plan mode), which an exit-plan request then asks to approve.
    fn written_plan(&self, _tool: &str, _input: &Value) -> Option<String> {
        None
    }

    /// A failed call was the user's refusal.
    fn rejected(&self, _update: &Value) -> bool {
        false
    }

    /// A call's `content` is its whole output so far, so each update replaces it (ACP's rule); false: each update's
    /// text adds to it.
    fn cumulative_output(&self) -> bool {
        true
    }

    /// A `usage_update` that is not the conversation's (a sub-agent's).
    fn ignore_usage(&self, _update: &Value) -> bool {
        false
    }

    /// A replayed prompt's id and time (ms), for importing turns taken outside PDT (`import_after`). Without a time,
    /// nothing is imported.
    fn replayed_prompt(&self, _update: &Value) -> (Option<String>, Option<i64>) {
        (None, None)
    }

    /// The permission option that allows a call for this session only. Plain ACP's `allow_always` saves a rule in the
    /// agent's settings, which PDT never does, so plain ACP has none.
    fn session_option(&self) -> Option<&str> {
        None
    }

    /// The permission request (by its options' ids) is the agent asking to approve a plan.
    fn is_plan_request(&self, _option_ids: &[&str]) -> bool {
        false
    }

    /// The option approving a plan and leaving plan mode for `mode` (None: the agent's usual mode).
    fn plan_option(&self, _mode: Option<PermissionMode>) -> Option<&str> {
        None
    }

    /// The prefix the agent gives pdt-mcp's tool names (`mcp__pdt__get_ticket`): those calls run without asking.
    fn pdt_tool_prefix(&self) -> Option<&str> {
        None
    }

    /// Orders a form's optional fields (required ones come first, in the schema's order); default: by name.
    fn order_fields(&self, _keys: &mut [String]) {}

    /// A JSON-RPC error's `data` says a retry may work.
    fn retryable(&self, _data: &Value) -> bool {
        false
    }
}

/// Plain ACP, with no agent's extra fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainAcp;

impl AcpExtensions for PlainAcp {}
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
    /// optionId by kind (`allow_once`, `reject_once`, …).
    options: Vec<(String, String)>,
    /// The option allowing the call for this session only, when the request offers one.
    always: Option<String>,
    /// Question text → the form field that answers it, and whether it takes several.
    fields: BTreeMap<String, (String, bool)>,
    turn_id: Option<String>,
}

impl OpenRequest {
    fn option(&self, id_or_kind: &str) -> Option<String> {
        self.options.iter().find(|(id, kind)| id == id_or_kind || kind == id_or_kind).map(|(id, _)| id.clone())
    }
    fn can_always(&self) -> bool {
        self.always.is_some()
    }
}

/// The state of one ACP session's stream.
#[derive(Debug)]
pub struct AcpMapper<P> {
    ext: P,
    /// The session's folder (and its canonical form, when the agent reports paths through symlinks resolved).
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
    /// The plan the agent last wrote (plan mode): what an exit-plan request asks to approve.
    plan: Option<String>,
    /// The files the running turn changed inside the folder, in the order first changed: (path, created, hunks).
    turn_files: Vec<(String, bool, String)>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    /// `session/load` is replaying the saved conversation, which PDT has logged already.
    loading: bool,
    /// While loading: replayed turns whose prompt is newer than this (ms) were taken outside PDT (in the agent's
    /// own terminal) and are imported.
    import_after: Option<i64>,
    /// The replayed turn being imported (its prompt's id).
    importing: Option<String>,
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

impl<P: AcpExtensions> AcpMapper<P> {
    /// A mapper for a session working in `cwd` (plus its canonical path, when that differs and the agent reports
    /// paths that way). `prefix` keeps ids from this process apart from an earlier one's.
    pub fn with_extensions(
        ext: P,
        cwd: impl Into<String>,
        canonical: Option<String>,
        prefix: impl Into<String>,
    ) -> Self {
        let mut roots = vec![cwd.into()];
        if ext.resolves_symlinks() {
            roots.extend(canonical.filter(|c| !roots.contains(c)));
        }
        Self {
            ext,
            roots,
            prefix: prefix.into(),
            next: 0,
            turn: None,
            prompts: 0,
            interrupt_requested: false,
            message: None,
            thought: None,
            tools: HashMap::new(),
            requests: HashMap::new(),
            plan: None,
            turn_files: Vec::new(),
            context_used: None,
            context_window: None,
            loading: false,
            import_after: None,
            importing: None,
        }
    }

    /// The agent's extensions.
    pub fn extensions(&self) -> &P {
        &self.ext
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
    /// newer than `import_after` (ms; 0 = every turn), which were taken outside PDT and are imported.
    pub fn begin_load(&mut self, import_after: Option<i64>) {
        self.loading = true;
        self.import_after = import_after;
    }

    /// `session/load` has answered: the last imported turn ends.
    pub fn end_load(&mut self) -> Vec<AcpOut> {
        let mut out = Vec::new();
        if self.importing.take().is_some() {
            self.close_turn(TurnStatus::Completed, None, &mut out);
        }
        self.loading = false;
        self.import_after = None;
        out
    }

    /// A replayed prompt: it ends the turn being imported, and starts one when it is newer than `import_after`.
    fn replayed_prompt(&mut self, u: &Value) -> Vec<AcpOut> {
        let mut out = Vec::new();
        let (client, at) = self.ext.replayed_prompt(u);
        if client.is_some() && self.importing == client {
            return out;
        }
        if self.importing.take().is_some() {
            self.close_turn(TurnStatus::Completed, None, &mut out);
        }
        let Some(after) = self.import_after else { return out };
        if at.is_some_and(|t| t > after) {
            let text = u.get("content").and_then(block_text).unwrap_or_default().to_string();
            self.open_turn(&text, MessageSource::Terminal, &mut out);
            self.importing = Some(client.unwrap_or_default());
        }
        out
    }

    fn open_turn(&mut self, text: &str, source: MessageSource, out: &mut Vec<AcpOut>) {
        let turn = self.id("turn").trim_start_matches("turn:").to_string();
        self.turn = Some(turn.clone());
        self.interrupt_requested = false;
        out.push(AcpOut::Event {
            item_id: Some(format!("turn:{turn}")),
            turn_id: Some(turn.clone()),
            event: AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None },
        });
        out.push(AcpOut::Event {
            item_id: Some(format!("u:{turn}")),
            turn_id: Some(turn),
            event: AgentEvent::UserMessage { text: text.to_string(), images: vec![], source, pdt: None },
        });
    }

    fn id(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}:{}-{}", self.prefix, self.next)
    }

    fn event(&self, item_id: Option<String>, event: AgentEvent) -> AcpOut {
        AcpOut::Event { item_id, turn_id: self.turn.clone(), event }
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
    fn close_text(&mut self, phase: MessagePhase, out: &mut Vec<AcpOut>) {
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

    fn tool_event(&self, id: &str, t: &Tool) -> AcpOut {
        AcpOut::Event {
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

    fn request_event(&self, id: &str, r: &OpenRequest, state: RequestState, answer: Option<Answer>) -> AcpOut {
        AcpOut::Event {
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

    fn usage_event(&self) -> AcpOut {
        AcpOut::Event {
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

    /// PDT is sending a prompt: the user's message, and the start of a turn unless one is running (the agent takes a
    /// prompt sent mid-turn into the running turn).
    pub fn begin_prompt(&mut self, text: &str) -> Vec<AcpOut> {
        self.prompts += 1;
        let mut out = Vec::new();
        let source = if self.turn.is_some() { MessageSource::Steer } else { MessageSource::Pdt };
        if self.turn.is_none() {
            self.open_turn(text, source, &mut out);
        } else {
            self.close_text(MessagePhase::Commentary, &mut out);
            let item = self.id("u");
            out.push(self.event(
                Some(item),
                AgentEvent::UserMessage { text: text.to_string(), images: vec![], source, pdt: None },
            ));
        }
        out
    }

    /// A `session/prompt` call was answered (`Err`: the JSON-RPC error's message, and whether the agent says a retry may
    /// work). The turn ends with the last prompt's answer.
    pub fn end_prompt(&mut self, result: Result<&Value, (String, bool)>) -> Vec<AcpOut> {
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
                "max_tokens" => (TurnStatus::Failed, Some(format!("{} hit its output limit", self.ext.name()))),
                "max_turn_requests" => {
                    (TurnStatus::Failed, Some(format!("{} hit its limit of requests for one turn", self.ext.name())))
                }
                "refusal" => (TurnStatus::Failed, Some(format!("{} refused", self.ext.name()))),
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

    fn close_turn(&mut self, status: TurnStatus, reason: Option<String>, out: &mut Vec<AcpOut>) {
        self.close_text(MessagePhase::Final, out);
        out.extend(self.expire(true));
        let Some(turn) = self.turn.clone() else { return };
        for (id, t) in self.tools.iter_mut().filter(|(_, t)| t.turn_id.as_deref() == Some(turn.as_str())) {
            if t.status == ToolStatus::Running {
                t.status = if status == TurnStatus::Completed { ToolStatus::Done } else { ToolStatus::Failed };
                let t = t.clone();
                out.push(AcpOut::Event {
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
            out.push(AcpOut::Event {
                item_id: Some(format!("diff:{turn}")),
                turn_id: Some(turn.clone()),
                event: AgentEvent::TurnDiff { diff },
            });
        }
        out.push(AcpOut::Event {
            item_id: Some(format!("turn-end:{turn}")),
            turn_id: Some(turn),
            event: AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(status), duration_ms: None, reason },
        });
        self.turn = None;
        self.interrupt_requested = false;
        self.tools.clear();
    }

    /// Expires open requests (`all`; else none): the turn ended or the process stopped.
    fn expire(&mut self, all: bool) -> Vec<AcpOut> {
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
    pub fn cancel_requests(&mut self) -> (Vec<(Value, Value)>, Vec<AcpOut>) {
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
    pub fn on_exit(&mut self, reason: &str) -> Vec<AcpOut> {
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
    pub fn on_update(&mut self, u: &Value) -> Vec<AcpOut> {
        let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        match kind {
            "config_option_update" => return self.config_options(u.get("configOptions")),
            "current_mode_update" => {
                return vec![AcpOut::Settings { mode: s(u, "currentModeId"), model: None }];
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
                return vec![AcpOut::Commands(commands)];
            }
            "usage_update" => {
                // Sub-agents report their own context too; the conversation's is the root agent's.
                if self.ext.ignore_usage(u) {
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
                out.push(AcpOut::Delta { item_id: item.clone(), field: DeltaField::Markdown, text });
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

    fn config_options(&self, options: Option<&Value>) -> Vec<AcpOut> {
        let options = options.and_then(Value::as_array).cloned().unwrap_or_default();
        let find = |id: &str| options.iter().find(|o| s(o, "id").as_deref() == Some(id));
        let mut out = vec![AcpOut::Settings {
            mode: find("mode").and_then(|o| s(o, "currentValue")),
            model: find("model").and_then(|o| s(o, "currentValue")),
        }];
        if let Some(models) = find("model").and_then(|o| o.get("options")).and_then(Value::as_array) {
            out.push(AcpOut::Models(
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
        let name = self.ext.tool_name(u);
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
        if self.ext.is_exit_plan(u, &t.tool) {
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
                let text = texts.join("\n");
                let whole = match t.output.as_deref().filter(|_| !self.ext.cumulative_output()) {
                    Some(so_far) => format!("{so_far}\n{text}"),
                    None => text,
                };
                t.output = Some(cap_text(whole.trim_end(), MAX_TOOL_OUTPUT));
            }
            if !edits.is_empty() {
                t.diff = Some(edits.iter().map(|(p, o, n)| self.file_diff(p, o.as_deref(), n)).collect::<String>());
                t.edits = edits;
            }
        }
        if let Some(plan) = self.ext.written_plan(&t.tool, &t.input) {
            self.plan = Some(plan);
        }
        if let Some(code) = meta(u, "terminal_exit").and_then(|e| e.get("exit_code")).and_then(Value::as_i64) {
            t.exit_code = i32::try_from(code).ok();
        }
        // A call that failed or was refused stays so (an agent can report it completed after cancelling it).
        if !matches!(t.status, ToolStatus::Failed | ToolStatus::Declined) {
            match s(u, "status").as_deref() {
                Some("completed") => t.status = ToolStatus::Done,
                Some("failed") if self.ext.rejected(u) => {
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
    pub fn on_request(&mut self, rpc_id: Value, method: &str, p: &Value) -> Vec<AcpOut> {
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
                let ids: Vec<&str> = options.iter().map(|(id, _)| id.as_str()).collect();
                let plan = tool.as_ref().is_some_and(|t| t.exit_plan) || self.ext.is_plan_request(&ids);
                let always = self.ext.session_option().filter(|o| ids.contains(o)).map(String::from);
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
                        always,
                        fields: BTreeMap::new(),
                        turn_id: self.turn.clone(),
                    }
                } else {
                    let (name, title, input, diff) = match &tool {
                        Some(t) => (t.tool.clone(), t.title.clone(), t.input.clone(), t.diff.clone()),
                        None => ("tool".into(), String::new(), Value::Null, None),
                    };
                    // PDT's own tools (pdt-mcp): the session only has those its persona allows, so they run
                    // without asking, as Claude's `--allowedTools` and Codex's approval mode do.
                    if self.ext.pdt_tool_prefix().is_some_and(|prefix| name.starts_with(prefix))
                        && let Some((once, _)) = options.iter().find(|(_, kind)| kind == "allow_once")
                    {
                        return vec![AcpOut::Respond {
                            id: rpc_id,
                            result: json!({"outcome": {"outcome": "selected", "optionId": once}}),
                        }];
                    }
                    OpenRequest {
                        rpc_id,
                        kind: RequestKind::Approval,
                        tool: name,
                        title,
                        input,
                        tool_call_id,
                        diff,
                        options,
                        always,
                        fields: BTreeMap::new(),
                        turn_id: self.turn.clone(),
                    }
                }
            }
            "elicitation/create" if p.get("mode").and_then(Value::as_str).unwrap_or("form") == "form" => {
                let (questions, fields) = questions(p, &self.ext);
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
                    always: None,
                    fields,
                    turn_id: self.turn.clone(),
                }
            }
            _ => return vec![AcpOut::RespondError { id: rpc_id, message: format!("PDT does not handle {method}") }],
        };
        let out = vec![self.request_event(&id, &r, RequestState::Open, None)];
        self.requests.insert(id, r);
        out
    }

    /// The JSON-RPC result answering an open request, and the events it changes.
    pub fn answer(&mut self, request_id: &str, answer: &Answer) -> Result<AcpAnswer, AnswerError> {
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
                let id = match &r.always {
                    Some(session) if *always => Some(session.clone()),
                    _ => r.option("allow_once"),
                };
                (selected(id)?, None)
            }
            (RequestKind::Plan, Answer::Approve { .. })
            | (RequestKind::Plan, Answer::Plan { approve: true, mode: None, .. }) => {
                let id = self.ext.plan_option(None).and_then(|o| r.option(o));
                (selected(id.or_else(|| r.option("allow_once")))?, None)
            }
            (RequestKind::Plan, Answer::Plan { approve: true, mode: Some(m), .. }) => {
                let id = self.ext.plan_option(Some(*m)).and_then(|o| r.option(o));
                (selected(id.or_else(|| r.option("allow_once")))?, None)
            }
            (_, Answer::Deny { message }) => (selected(r.option("reject_once"))?, note(message)),
            (_, Answer::Plan { approve: false, feedback, .. }) => (selected(r.option("reject_once"))?, note(feedback)),
        };
        let r = self.requests.remove(request_id).expect("checked above");
        let events = vec![self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()))];
        Ok(AcpAnswer { rpc_id: r.rpc_id, result, events, follow_up })
    }
}

/// An elicitation form's fields as PDT questions (`{question, header, options, multiSelect}`, as Claude's
/// AskUserQuestion has them), with each question's field: the required ones first, then the rest in the order the
/// agent's extensions give them.
fn questions(p: &Value, ext: &impl AcpExtensions) -> (Vec<Value>, BTreeMap<String, (String, bool)>) {
    let schema = p.get("requestedSchema").cloned().unwrap_or_default();
    let props = schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut keys: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|k| k.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut rest: Vec<String> = props.keys().filter(|k| !keys.contains(k)).cloned().collect();
    ext.order_fields(&mut rest);
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

    /// A turn from a plain ACP agent, with no agent's extra fields.
    const TURN: &str = include_str!("../../tests/fixtures/acp/turn.jsonl");

    fn mapper() -> AcpMapper<PlainAcp> {
        AcpMapper::with_extensions(PlainAcp, "/work/repo", Some("/private/work/repo".into()), "p")
    }

    /// Feeds frames as the adapter would, starting the turn before the first frame and answering requests with
    /// `answer`; what PDT sent back is recorded as `Unknown { raw: {sent, followUp} }`.
    fn feed(
        m: &mut AcpMapper<PlainAcp>,
        frames: &str,
        mut answer: impl FnMut(&AgentEvent) -> Option<Answer>,
    ) -> Vec<AcpOut> {
        let mut out = m.begin_prompt("go");
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            match (v.get("method").and_then(Value::as_str), v.get("id")) {
                (Some("session/update"), None) => out.extend(m.on_update(&v["params"]["update"])),
                (Some(method), Some(id)) => {
                    let evs = m.on_request(id.clone(), method, &v["params"]);
                    let ask = evs.iter().find_map(|e| match e {
                        AcpOut::Event { event: ev @ AgentEvent::Request { .. }, .. } => Some(ev.clone()),
                        _ => None,
                    });
                    out.extend(evs);
                    if let Some(ev) = ask
                        && let AgentEvent::Request { id, .. } = &ev
                        && let Some(a) = answer(&ev)
                    {
                        let a = m.answer(id, &a).unwrap();
                        out.push(AcpOut::Event {
                            item_id: None,
                            turn_id: None,
                            event: AgentEvent::Unknown { raw: json!({"sent": a.result, "followUp": a.follow_up}) },
                        });
                        out.extend(a.events);
                    }
                }
                (None, Some(_)) => out.extend(m.end_prompt(Ok(&v["result"]))),
                _ => {}
            }
        }
        out
    }

    fn events(out: &[AcpOut]) -> Vec<AgentEvent> {
        out.iter()
            .filter_map(|o| match o {
                AcpOut::Event { event, .. } => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    fn sent(ev: &[AgentEvent]) -> Vec<Value> {
        ev.iter()
            .filter_map(|e| match e {
                AgentEvent::Unknown { raw } => Some(raw.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_tool<'a>(ev: &'a [AgentEvent], id: &str) -> &'a AgentEvent {
        ev.iter().rev().find(|e| matches!(e, AgentEvent::ToolCall { id: i, .. } if i == id)).unwrap()
    }

    #[test]
    fn a_plain_acp_turn_maps_to_messages_tools_requests_and_its_end() {
        let mut m = mapper();
        let out = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, can_always, title, tool, diff, .. } => {
                // Plain ACP's "always" saves a rule in the agent's settings: PDT never offers it.
                assert!(!*can_always);
                assert_eq!((title.as_str(), tool.as_str()), ("a.txt", "edit"));
                assert!(diff.as_deref().unwrap().contains("+goodbye"));
                Some(Answer::Approve { always: true })
            }
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                assert_eq!(title, "Which colour?");
                assert_eq!(input["questions"][0]["header"], "Colour");
                assert_eq!(input["questions"][0]["options"][1]["label"], "Blue");
                assert_eq!(input["questions"][1]["question"], "Anything else?");
                let answers = BTreeMap::from([("Which colour?".to_string(), vec!["Blue".to_string()])]);
                Some(Answer::Question { answers })
            }
            _ => None,
        });
        let ev = events(&out);
        assert!(matches!(&ev[0], AgentEvent::Turn { phase: TurnPhase::Started, .. }));
        assert!(matches!(&ev[1], AgentEvent::UserMessage { text, source: MessageSource::Pdt, .. } if text == "go"));
        assert!(
            ev.iter()
                .any(|e| matches!(e, AgentEvent::Reasoning { summary } if summary == "Read the file, then change it."))
        );
        // Without an agent's tool names, a call is named by its kind; its content is its whole output so far.
        let AgentEvent::ToolCall { tool, title, status, output, .. } = last_tool(&ev, "t1") else { panic!() };
        assert_eq!((tool.as_str(), title.as_str(), *status), ("execute", "ls", ToolStatus::Done));
        assert_eq!(output.as_deref(), Some("a.txt\nb.txt"));
        let AgentEvent::ToolCall { status, diff, .. } = last_tool(&ev, "t2") else { panic!() };
        assert_eq!(*status, ToolStatus::Done);
        assert_eq!(diff.as_deref(), Some("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-hello\n+goodbye\n"));
        let sent = sent(&ev);
        assert_eq!(sent[0]["sent"], json!({"outcome": {"outcome": "selected", "optionId": "yes"}}));
        assert_eq!(sent[1]["sent"], json!({"action": "accept", "content": {"color": "Blue"}}));
        assert!(ev.iter().any(
            |e| matches!(e, AgentEvent::Plan { markdown } if markdown == "- [x] Edit a.txt\n- [ ] Tell the user\n")
        ));
        assert!(ev.iter().any(|e| matches!(e,
            AgentEvent::AgentMessage { markdown, phase: MessagePhase::Final, .. } if markdown == "Changed a.txt.")));
        assert!(
            ev.iter()
                .any(|e| matches!(e, AgentEvent::Usage { context_used: Some(5000), context_window: Some(100000), .. }))
        );
        let n = ev.len();
        assert!(matches!(&ev[n - 2], AgentEvent::TurnDiff { diff } if diff.contains("+goodbye")));
        assert!(matches!(
            &ev[n - 1],
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        // Mode and commands are reported as the agent names them.
        assert!(out.iter().any(|o| matches!(o, AcpOut::Settings { mode: Some(m), .. } if m == "code")));
        assert!(out.iter().any(|o| matches!(o, AcpOut::Commands(c) if c[0].name == "init")));
        assert_eq!(m.turn(), None);
    }

    #[test]
    fn plain_acp_rejects_and_limits_name_the_agent_generically() {
        let mut m = mapper();
        let out = feed(&mut m, TURN, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Approval, .. } => {
                Some(Answer::Deny { message: Some("not that file".into()) })
            }
            _ => None,
        });
        let sent = sent(&events(&out));
        assert_eq!(sent[0]["sent"]["outcome"]["optionId"], "no");
        assert_eq!(sent[0]["followUp"], "not that file");
        m.begin_prompt("x");
        let end = events(&m.end_prompt(Ok(&json!({"stopReason": "max_tokens"}))));
        assert!(matches!(end.last().unwrap(),
            AgentEvent::Turn { status: Some(TurnStatus::Failed), reason: Some(r), .. } if r == "The agent hit its output limit"));
    }

    #[test]
    fn plain_acp_has_no_plan_requests_imports_or_auto_allowed_tools() {
        let mut m = mapper();
        m.begin_prompt("go");
        // A path through the resolved symlink is not the session's folder for plain ACP.
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "mcp__pdt__get_ticket",
            "kind": "read", "rawInput": {"file_path": "/private/work/repo/a.txt"}}));
        let options = json!([{"optionId": "plan_x", "kind": "allow_once"}, {"optionId": "n", "kind": "reject_once"}]);
        let out = m.on_request(
            json!(3),
            "session/request_permission",
            &json!({"toolCall": {"toolCallId": "c1"}, "options": options}),
        );
        let AgentEvent::Request { request_kind, title, .. } = &events(&out)[0] else { panic!("{out:?}") };
        assert_eq!((*request_kind, title.as_str()), (RequestKind::Approval, "/private/work/repo/a.txt"));
        // A replayed conversation is dropped whatever `import_after` says: plain ACP stamps no times.
        let mut m = mapper();
        m.begin_load(Some(0));
        let chunk = json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "old"}});
        assert!(m.on_update(&chunk).is_empty());
        assert!(m.end_load().is_empty());
    }

    #[test]
    fn output_that_is_not_cumulative_is_appended() {
        #[derive(Debug)]
        struct Streaming;
        impl AcpExtensions for Streaming {
            fn cumulative_output(&self) -> bool {
                false
            }
        }
        let mut m = AcpMapper::with_extensions(Streaming, "/w", None, "p");
        m.begin_prompt("go");
        let part = |t: &str| {
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "t",
            "content": [{"type": "content", "content": {"type": "text", "text": t}}]})
        };
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "t", "kind": "execute"}));
        m.on_update(&part("one"));
        let out = m.on_update(&part("two"));
        let AgentEvent::ToolCall { output, .. } = &events(&out)[0] else { panic!() };
        assert_eq!(output.as_deref(), Some("one\ntwo"));
    }
}
