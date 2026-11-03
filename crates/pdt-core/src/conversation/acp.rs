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
use std::collections::{BTreeMap, HashMap, HashSet};

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
    /// Answer an agent request now with a result (PDT's own tools need no approval).
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
    /// The mode to put the session in after answering (an approved plan of the agent's own plan request, which
    /// leaves plan mode only when told to).
    pub mode: Option<PermissionMode>,
}

/// One question of an agent's own question request (Cursor's `cursor/ask_question`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtQuestion {
    /// The agent's id for the question.
    pub id: String,
    pub question: String,
    pub header: Option<String>,
    /// Its options: (the agent's id, label).
    pub options: Vec<(String, String)>,
    pub multi: bool,
}

/// An agent's own request (a method that is not ACP's) that PDT shows the user.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtRequest {
    Questions(Vec<ExtQuestion>),
    /// A plan to approve.
    Plan {
        markdown: String,
    },
}

/// The user's answer to an agent's own request, for its extensions to put in the agent's terms.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtAnswer {
    /// Question id → the ids of the options picked.
    Answered(Vec<(String, Vec<String>)>),
    /// Not answered by picking options: skipped, or answered in the user's own words (`reason` says which).
    Skipped {
        reason: Option<String>,
    },
    Cancelled,
    Plan {
        approve: bool,
        feedback: Option<String>,
    },
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

    /// A failed edit call that the user approved and the agent then ran as another call (Antigravity's new file): it
    /// is shown done, with its diff, instead of failed.
    fn ran_elsewhere(&self, _update: &Value) -> bool {
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

    /// A call's output and exit code from the agent's `rawOutput`, for an agent that reports a command's output there
    /// instead of in `content` (Cursor's `{exitCode, stdout, stderr}`).
    fn raw_output(&self, _update: &Value) -> Option<(String, Option<i32>)> {
        None
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

    /// A form field's `title` is its question and its `description` the question's header (OhMyPi's); false: the
    /// description is the question and the title its header (Devin's).
    fn question_in_title(&self) -> bool {
        false
    }

    /// The form field that takes an answer in the user's own words to the question in field `key`, when the agent
    /// gives each question one (OhMyPi's `<key>__other`); None: own words go in the question's field.
    fn other_field(&self, _key: &str) -> Option<String> {
        None
    }

    /// A JSON-RPC error's `data` says a retry may work.
    fn retryable(&self, _data: &Value) -> bool {
        false
    }

    /// An agent's own request (`method` is not ACP's) that asks the user something; None: PDT refuses it.
    fn ext_request(&self, _method: &str, _params: &Value) -> Option<ExtRequest> {
        None
    }

    /// The JSON-RPC result answering an agent's own request.
    fn ext_answer(&self, _method: &str, _answer: &ExtAnswer) -> Value {
        Value::Null
    }
}

/// Plain ACP, with no agent's extra fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainAcp;

impl AcpExtensions for PlainAcp {}
#[derive(Debug, Clone, PartialEq)]
struct Tool {
    tool: String,
    /// The call's ACP kind (`read`, `edit`, `execute`, …), when the agent gave one.
    kind: Option<String>,
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
    /// Question text → the form field that answers it.
    fields: BTreeMap<String, FormField>,
    /// The method of an agent's own request (`AcpExtensions::ext_request`), answered in its terms.
    ext: Option<String>,
    /// An agent's own question request's questions.
    ext_questions: Vec<ExtQuestion>,
    turn_id: Option<String>,
}

/// The form field answering one of an elicitation's questions.
#[derive(Debug, Clone, PartialEq)]
struct FormField {
    key: String,
    /// It takes several answers (an array).
    multi: bool,
    /// Its options' labels.
    labels: Vec<String>,
    /// The field for an answer in the user's own words, when the form has one apart (`AcpExtensions::other_field`).
    other: Option<String>,
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
    /// The running turn's calls whose approval was refused (by the user or for them).
    refused: HashSet<String>,
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
    /// The session's permission mode, when PDT keeps it by answering permission requests itself (an agent with no
    /// modes of its own, PDT-234).
    enforce: Option<PermissionMode>,
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
            refused: HashSet::new(),
            requests: HashMap::new(),
            plan: None,
            turn_files: Vec::new(),
            context_used: None,
            context_window: None,
            loading: false,
            import_after: None,
            importing: None,
            enforce: None,
        }
    }

    /// PDT keeps the session in `mode` by answering the agent's permission requests itself where the mode decides
    /// them (`crate::acp_agent::enforced_answer`); None: every request goes to the user.
    pub fn enforce_mode(&mut self, mode: Option<PermissionMode>) {
        self.enforce = mode;
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
                rows: None,
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
        self.refused.clear();
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
                let result = if let Some(method) = &r.ext {
                    self.ext.ext_answer(method, &ExtAnswer::Cancelled)
                } else if r.kind == RequestKind::Question {
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

    /// A `session/new` or `session/load` answer: its `configOptions` (the model and its choices), which some agents
    /// (Grok) send only there.
    pub fn on_session(&self, answer: &Value) -> Vec<AcpOut> {
        answer.get("configOptions").map(|o| self.config_options(Some(o))).unwrap_or_default()
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
            kind: None,
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
        // A call first reported without its name (only its kind) gets it from a later update.
        let unnamed = before.as_ref().is_none_or(|b| Some(&b.tool) == b.kind.as_ref() || b.tool == "tool");
        if (new || unnamed)
            && let Some(n) = name
        {
            t.tool = n;
        }
        if let Some(k) = s(u, "kind") {
            t.kind = Some(k);
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
        if let Some((output, code)) = self.ext.raw_output(u) {
            if !output.trim().is_empty() {
                t.output = Some(cap_text(output.trim_end(), MAX_TOOL_OUTPUT));
            }
            t.exit_code = code.or(t.exit_code);
        }
        // A call that failed or was refused stays so (an agent can report it completed after cancelling it).
        if !matches!(t.status, ToolStatus::Failed | ToolStatus::Declined) {
            match s(u, "status").as_deref() {
                Some("completed") => t.status = ToolStatus::Done,
                Some("failed") if self.ext.rejected(u) => {
                    t.status = ToolStatus::Declined;
                }
                Some("failed")
                    if t.kind.as_deref() == Some("edit") && !self.refused.contains(id) && self.ext.ran_elsewhere(u) =>
                {
                    t.status = ToolStatus::Done;
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
                        ext: None,
                        ext_questions: vec![],
                        turn_id: self.turn.clone(),
                    }
                } else {
                    let (mut name, title, input, diff) = match &tool {
                        Some(t) => (t.tool.clone(), t.title.clone(), t.input.clone(), t.diff.clone()),
                        // Asked before the agent reported the call (OhMyPi does): the request's own call names it.
                        None => {
                            let call = p.get("toolCall").cloned().unwrap_or_default();
                            let input = call.get("rawInput").cloned().unwrap_or(Value::Null);
                            let title = input
                                .get("command")
                                .and_then(Value::as_str)
                                .map(one_line)
                                .or_else(|| s(&call, "title").map(|t| one_line(&t)))
                                .unwrap_or_default();
                            let name =
                                self.ext.tool_name(&call).or_else(|| s(&call, "kind")).unwrap_or_else(|| "tool".into());
                            (name, title, input, None)
                        }
                    };
                    // The request's own call names the tool when the agent's earlier report of it didn't (Cursor
                    // asks before its `tool_call` has its `rawInput`).
                    if let Some(own) = p.get("toolCall").and_then(|c| self.ext.tool_name(c)) {
                        name = own;
                    }
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
                    // An agent with no modes: PDT answers what the session's mode decides.
                    let kind = p
                        .get("toolCall")
                        .and_then(|t| s(t, "kind"))
                        .or_else(|| tool.as_ref().and_then(|t| t.kind.clone()))
                        .unwrap_or_default();
                    if let Some(allow) = self.enforce.and_then(|m| crate::acp_agent::enforced_answer(m, &kind))
                        && let Some((option, _)) =
                            options.iter().find(|(_, k)| k == if allow { "allow_once" } else { "reject_once" })
                    {
                        if let (false, Some(call)) = (allow, &tool_call_id) {
                            self.refused.insert(call.clone());
                        }
                        return vec![AcpOut::Respond {
                            id: rpc_id,
                            result: json!({"outcome": {"outcome": "selected", "optionId": option}}),
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
                        ext: None,
                        ext_questions: vec![],
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
                    ext: None,
                    ext_questions: vec![],
                    turn_id: self.turn.clone(),
                }
            }
            _ => match self.ext.ext_request(method, p) {
                Some(ExtRequest::Questions(qs)) => OpenRequest {
                    rpc_id,
                    kind: RequestKind::Question,
                    tool: "AskUserQuestion".into(),
                    title: qs.first().map(|q| q.question.clone()).unwrap_or_default(),
                    input: json!({"questions": qs.iter().map(|q| json!({
                        "question": q.question,
                        "header": q.header,
                        "options": q.options.iter().map(|(_, label)| json!({"label": label})).collect::<Vec<_>>(),
                        "multiSelect": q.multi,
                    })).collect::<Vec<_>>()}),
                    tool_call_id: s(p, "toolCallId"),
                    diff: None,
                    options: vec![],
                    always: None,
                    fields: BTreeMap::new(),
                    ext: Some(method.to_string()),
                    ext_questions: qs,
                    turn_id: self.turn.clone(),
                },
                Some(ExtRequest::Plan { markdown }) => OpenRequest {
                    rpc_id,
                    kind: RequestKind::Plan,
                    tool: "ExitPlanMode".into(),
                    title: "Plan".into(),
                    input: json!({"plan": markdown}),
                    tool_call_id: s(p, "toolCallId"),
                    diff: None,
                    options: vec![],
                    always: None,
                    fields: BTreeMap::new(),
                    ext: Some(method.to_string()),
                    ext_questions: vec![],
                    turn_id: self.turn.clone(),
                },
                None => {
                    return vec![AcpOut::RespondError { id: rpc_id, message: format!("PDT does not handle {method}") }];
                }
            },
        };
        let out = vec![self.request_event(&id, &r, RequestState::Open, None)];
        self.requests.insert(id, r);
        out
    }

    /// The JSON-RPC result answering an open request, and the events it changes.
    pub fn answer(&mut self, request_id: &str, answer: &Answer) -> Result<AcpAnswer, AnswerError> {
        let r = self.requests.get(request_id).ok_or(AnswerError::Closed)?;
        let note = |m: &Option<String>| m.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(String::from);
        if let Some(method) = r.ext.clone() {
            let (ext_answer, mode, follow_up) = ext_answer(r, answer, note)?;
            let result = self.ext.ext_answer(&method, &ext_answer);
            let r = self.requests.remove(request_id).expect("checked above");
            let events = vec![self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()))];
            return Ok(AcpAnswer { rpc_id: r.rpc_id, result, events, follow_up, mode });
        }
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
                    let Some(f) = r.fields.get(q) else { continue };
                    // Own words go in their own field when the form has one.
                    let (chosen, own): (Vec<&String>, Vec<&String>) = match &f.other {
                        Some(_) => picked.iter().partition(|p| f.labels.contains(p)),
                        None => (picked.iter().collect(), Vec::new()),
                    };
                    let v = if f.multi {
                        json!(chosen)
                    } else {
                        json!(chosen.first().copied().cloned().unwrap_or_default())
                    };
                    if f.multi || !chosen.is_empty() || f.other.is_none() {
                        content.insert(f.key.clone(), v);
                    }
                    if let (Some(other), false) = (&f.other, own.is_empty()) {
                        let own: Vec<&str> = own.iter().map(|o| o.as_str()).collect();
                        content.insert(other.clone(), json!(own.join(", ")));
                    }
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
        if let (Answer::Deny { .. } | Answer::Plan { approve: false, .. }, Some(call)) = (answer, &r.tool_call_id) {
            self.refused.insert(call.clone());
        }
        let events = vec![self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()))];
        Ok(AcpAnswer { rpc_id: r.rpc_id, result, events, follow_up, mode: None })
    }
}

/// The user's answer to an agent's own request (`OpenRequest::ext`): the answer in [`ExtAnswer`]'s terms, the mode an
/// approved plan goes on in (the one picked, else Ask) and what to tell the agent next (an approved plan is built).
type ExtAnswered = (ExtAnswer, Option<PermissionMode>, Option<String>);

fn ext_answer(
    r: &OpenRequest,
    answer: &Answer,
    note: impl Fn(&Option<String>) -> Option<String>,
) -> Result<ExtAnswered, AnswerError> {
    const BUILD: &str = "The plan is approved: implement it now.";
    Ok(match (r.kind, answer) {
        (RequestKind::Question, Answer::Question { answers }) => {
            let mut picked = Vec::new();
            let mut own = Vec::new();
            for q in &r.ext_questions {
                let Some(chosen) = answers.get(&q.question) else { continue };
                let ids: Vec<String> = chosen
                    .iter()
                    .filter_map(|c| q.options.iter().find(|(_, label)| label == c).map(|(id, _)| id.clone()))
                    .collect();
                if ids.len() < chosen.len() {
                    own.push(format!("{}: {}", q.question, chosen.join(", ")));
                }
                picked.push((q.id.clone(), ids));
            }
            if own.is_empty() {
                (ExtAnswer::Answered(picked), None, None)
            } else {
                // The agent's answer takes only its options' ids: words of the user's own go in the reason.
                let reason = format!("The user answered in their own words. {}", own.join("; "));
                (ExtAnswer::Skipped { reason: Some(reason) }, None, None)
            }
        }
        (RequestKind::Question, Answer::Deny { message }) => (ExtAnswer::Skipped { reason: note(message) }, None, None),
        (RequestKind::Plan, Answer::Approve { .. }) => {
            (ExtAnswer::Plan { approve: true, feedback: None }, Some(PermissionMode::Ask), Some(BUILD.into()))
        }
        (RequestKind::Plan, Answer::Plan { approve: true, mode, .. }) => (
            ExtAnswer::Plan { approve: true, feedback: None },
            Some(mode.unwrap_or(PermissionMode::Ask)),
            Some(BUILD.into()),
        ),
        (RequestKind::Plan, Answer::Plan { approve: false, feedback, .. }) => {
            (ExtAnswer::Plan { approve: false, feedback: note(feedback) }, None, None)
        }
        (RequestKind::Plan, Answer::Deny { message }) => {
            (ExtAnswer::Plan { approve: false, feedback: note(message) }, None, None)
        }
        _ => return Err(AnswerError::WrongAnswer),
    })
}

/// An elicitation form's fields as PDT questions (`{question, header, options, multiSelect}`, as Claude's
/// AskUserQuestion has them), with each question's field: the required ones first, then the rest in the order the
/// agent's extensions give them. A question's own-words field (`AcpExtensions::other_field`) is no question of its
/// own.
fn questions(p: &Value, ext: &impl AcpExtensions) -> (Vec<Value>, BTreeMap<String, FormField>) {
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
    let other = |key: &str| ext.other_field(key).filter(|o| props.contains_key(o));
    let companions: Vec<String> = keys.iter().filter_map(|k| other(k)).collect();
    keys.retain(|k| !companions.contains(k));
    let mut out = Vec::new();
    let mut fields = BTreeMap::new();
    let only = keys.len() == 1;
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
                        let description = s(o, "description").or_else(|| s(o, "title").filter(|t| t != label));
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
        let message = || only.then(|| s(p, "message")).flatten();
        let (question, header) = if ext.question_in_title() {
            (message().or_else(|| s(f, "title")), s(f, "description"))
        } else {
            (s(f, "description").or_else(message).or_else(|| s(f, "title")), s(f, "title"))
        };
        let mut question = question.unwrap_or_else(|| key.clone());
        // Each question answers one field: two with the same text are told apart.
        let base = question.clone();
        for n in 2.. {
            if !fields.contains_key(&question) {
                break;
            }
            question = format!("{base} ({n})");
        }
        let header = header.filter(|h| *h != question);
        let labels = options.iter().filter_map(|o| o["label"].as_str().map(String::from)).collect();
        out.push(json!({"question": question, "header": header, "options": options, "multiSelect": multi}));
        fields.insert(question, FormField { key: key.clone(), multi, labels, other: other(&key) });
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
    fn with_no_modes_pdt_answers_what_the_session_mode_decides() {
        let mut m = mapper();
        m.begin_prompt("go");
        let options = json!([{"optionId": "y", "kind": "allow_once"}, {"optionId": "n", "kind": "reject_once"}]);
        let ask = |m: &mut AcpMapper<PlainAcp>, id: u64, kind: &str| {
            let call = format!("c{id}");
            m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": "t", "kind": kind}));
            m.on_request(
                json!(id),
                "session/request_permission",
                &json!({"toolCall": {"toolCallId": call}, "options": options}),
            )
        };
        // Not enforced: every request goes to the user.
        assert!(matches!(&ask(&mut m, 1, "read")[..], [AcpOut::Event { .. }]));
        m.enforce_mode(Some(PermissionMode::Plan));
        assert_eq!(
            ask(&mut m, 2, "edit"),
            vec![AcpOut::Respond {
                id: json!(2),
                result: json!({"outcome": {"outcome": "selected", "optionId": "n"}})
            }]
        );
        assert!(matches!(&ask(&mut m, 3, "fetch")[..], [AcpOut::Event { .. }]), "Plan asks about a fetch");
        m.enforce_mode(Some(PermissionMode::Bypass));
        assert_eq!(
            ask(&mut m, 4, "execute"),
            vec![AcpOut::Respond {
                id: json!(4),
                result: json!({"outcome": {"outcome": "selected", "optionId": "y"}})
            }]
        );
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

    /// A second plain ACP turn: a new file, an edit outside the folder, a call that fails, requests PDT refuses and
    /// a multi-select form.
    const EDITS: &str = include_str!("../../tests/fixtures/acp/edits.jsonl");

    #[test]
    fn a_turn_diff_has_only_the_folders_files_and_failed_calls_stay_failed() {
        let mut m = mapper();
        let out = feed(&mut m, EDITS, |e| match e {
            AgentEvent::Request { request_kind: RequestKind::Question, input, title, .. } => {
                // A form with one field asks its message; an option whose title is its label has no description.
                assert_eq!(title, "Which checks?");
                let q = &input["questions"][0];
                assert_eq!((q["header"].as_str(), q["multiSelect"].as_bool()), (Some("Checks"), Some(true)));
                assert_eq!(q["options"][0], json!({"label": "lint", "description": "Run the linter"}));
                assert_eq!(q["options"][1], json!({"label": "test", "description": null}));
                let answers = BTreeMap::from([("Which checks?".to_string(), vec!["lint".into(), "test".into()])]);
                Some(Answer::Question { answers })
            }
            _ => None,
        });
        let ev = events(&out);
        // The message said before the first call is commentary; the turn has no answer of its own.
        assert!(ev.iter().any(|e| matches!(e,
            AgentEvent::AgentMessage { markdown, phase: MessagePhase::Commentary, .. } if markdown == "Writing the files.")));
        assert!(!ev.iter().any(|e| matches!(e, AgentEvent::AgentMessage { phase: MessagePhase::Final, .. })));
        let AgentEvent::ToolCall { diff, .. } = last_tool(&ev, "w1") else { panic!() };
        assert_eq!(diff.as_deref(), Some("--- /dev/null\n+++ b/docs/new.txt\n@@ -0,0 +1 @@\n+fresh\n"));
        // A file outside the folder keeps its absolute path on both sides.
        let AgentEvent::ToolCall { diff, title, .. } = last_tool(&ev, "w2") else { panic!() };
        assert_eq!(title, "/etc/hosts");
        assert!(diff.as_deref().unwrap().starts_with("--- /etc/hosts\n+++ /etc/hosts\n"));
        // A failed call stays failed when the agent later says it completed; a long title is cut to its first line.
        let AgentEvent::ToolCall { status, output, title, .. } = last_tool(&ev, "x1") else { panic!() };
        assert_eq!(
            (*status, output.as_deref(), title.as_str()),
            (ToolStatus::Failed, Some("make: *** No rule"), "Run make…")
        );
        // A call still running when the turn fails fails with it.
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "r1") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed);
        // URL elicitations and the client methods PDT does not offer are refused at once.
        let refused: Vec<(&Value, &str)> = out
            .iter()
            .filter_map(|o| match o {
                AcpOut::RespondError { id, message } => Some((id, message.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            refused,
            [
                (&json!(10), "PDT does not handle elicitation/create"),
                (&json!(11), "PDT does not handle fs/read_text_file")
            ]
        );
        assert_eq!(sent(&ev)[0]["sent"], json!({"action": "accept", "content": {"checks": ["lint", "test"]}}));
        let n = ev.len();
        let AgentEvent::TurnDiff { diff } = &ev[n - 2] else { panic!("{:?}", ev[n - 2]) };
        assert_eq!(diff, "--- /dev/null\n+++ b/docs/new.txt\n@@ -0,0 +1 @@\n+fresh\n");
        assert!(matches!(&ev[n - 1], AgentEvent::Turn { status: Some(TurnStatus::Failed), reason: Some(r), .. }
            if r == "The agent refused"));
    }

    #[test]
    fn each_stop_reason_ends_the_turn_its_own_way() {
        let end = |result: Result<&Value, (String, bool)>, interrupt: bool| {
            let mut m = mapper();
            m.begin_prompt("go");
            if interrupt {
                m.note_interrupt();
            }
            let out = events(&m.end_prompt(result));
            assert_eq!(m.turn(), None);
            out
        };
        let status = |ev: &[AgentEvent]| match ev.last().unwrap() {
            AgentEvent::Turn { phase: TurnPhase::Ended, status, reason, .. } => (status.unwrap(), reason.clone()),
            other => panic!("{other:?}"),
        };
        assert_eq!(status(&end(Ok(&json!({})), false)), (TurnStatus::Completed, None));
        assert_eq!(status(&end(Ok(&json!({"stopReason": "end_turn"})), true)), (TurnStatus::Interrupted, None));
        assert_eq!(status(&end(Ok(&json!({"stopReason": "cancelled"})), false)), (TurnStatus::Interrupted, None));
        assert_eq!(
            status(&end(Ok(&json!({"stopReason": "max_turn_requests"})), false)),
            (TurnStatus::Failed, Some("The agent hit its limit of requests for one turn".into()))
        );
        assert_eq!(
            status(&end(Ok(&json!({"stopReason": "something_new"})), false)),
            (TurnStatus::Failed, Some("something_new".into()))
        );
        let failed = end(Err(("Internal error".into(), true)), false);
        assert!(matches!(&failed[0], AgentEvent::Error { text, retryable: true } if text == "Internal error"));
        assert_eq!(status(&failed), (TurnStatus::Failed, Some("Internal error".into())));
        // An answer with no turn running (a prompt PDT never saw start) changes nothing.
        assert!(mapper().end_prompt(Ok(&json!({}))).is_empty());
    }

    #[test]
    fn a_prompt_sent_mid_turn_steers_it_and_the_turn_ends_with_the_last_answer() {
        let mut m = mapper();
        m.begin_prompt("first");
        let chunk = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "On it"}});
        m.on_update(&chunk);
        let ev = events(&m.begin_prompt("also this"));
        // The message so far is closed as said on the way, and the new prompt joins the running turn.
        assert!(
            matches!(&ev[0], AgentEvent::AgentMessage { markdown, phase: MessagePhase::Commentary, .. } if markdown == "On it")
        );
        assert!(
            matches!(&ev[1], AgentEvent::UserMessage { text, source: MessageSource::Steer, .. } if text == "also this")
        );
        assert!(!ev.iter().any(|e| matches!(e, AgentEvent::Turn { .. })));
        // The first prompt's answer leaves the turn running; the second's ends it.
        assert!(m.end_prompt(Ok(&json!({"stopReason": "end_turn"}))).is_empty());
        assert!(m.turn().is_some());
        let ev = events(&m.end_prompt(Ok(&json!({"stopReason": "end_turn"}))));
        assert!(matches!(ev.last().unwrap(), AgentEvent::Turn { status: Some(TurnStatus::Completed), .. }));
        // An error answer ends the turn even with a prompt still waiting.
        m.begin_prompt("a");
        m.begin_prompt("b");
        let ev = events(&m.end_prompt(Err(("boom".into(), false))));
        assert!(matches!(ev.last().unwrap(), AgentEvent::Turn { status: Some(TurnStatus::Failed), .. }));
        assert_eq!(m.turn(), None);
    }

    #[test]
    fn cancelling_answers_every_open_request_and_exiting_ends_the_turn() {
        let mut m = mapper();
        m.begin_prompt("go");
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "kind": "execute", "title": "ls"}));
        let options = json!([{"optionId": "y", "kind": "allow_once"}, {"optionId": "n", "kind": "reject_once"}]);
        m.on_request(
            json!(1),
            "session/request_permission",
            &json!({"toolCall": {"toolCallId": "c1"}, "options": options}),
        );
        let form = json!({"requestedSchema": {"properties": {"x": {"type": "string"}}}});
        m.on_request(json!("q-2"), "elicitation/create", &form);
        assert_eq!(m.open_requests(), 2);
        let (mut replies, out) = m.cancel_requests();
        replies.sort_by_key(|(id, _)| id.to_string());
        assert_eq!(
            replies,
            [(json!("q-2"), json!({"action": "cancel"})), (json!(1), json!({"outcome": {"outcome": "cancelled"}})),]
        );
        let ev = events(&out);
        assert_eq!(ev.len(), 2);
        assert!(ev.iter().all(|e| matches!(e, AgentEvent::Request { state: RequestState::Expired, .. })));
        assert_eq!(m.open_requests(), 0);
        // A request answered after it expired is closed.
        assert_eq!(m.answer("1", &Answer::Approve { always: false }).unwrap_err(), AnswerError::Closed);
        let ev = events(&m.on_exit("Devin stopped"));
        let AgentEvent::ToolCall { status, .. } = last_tool(&ev, "c1") else { panic!() };
        assert_eq!(*status, ToolStatus::Failed);
        assert!(
            matches!(ev.last().unwrap(), AgentEvent::Turn { status: Some(TurnStatus::Interrupted), reason: Some(r), .. }
            if r == "Devin stopped")
        );
        // With no turn running, exiting only expires what is open.
        m.on_request(json!(5), "elicitation/create", &form);
        let ev = events(&m.on_exit("gone"));
        assert!(matches!(&ev[..], [AgentEvent::Request { state: RequestState::Expired, .. }]));
    }

    #[test]
    fn updates_outside_a_turn_are_dropped_but_settings_and_usage_are_not() {
        let mut m = mapper();
        let chunk = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "late"}});
        assert!(m.on_update(&chunk).is_empty());
        assert!(m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "t"})).is_empty());
        let usage = m.on_update(&json!({"sessionUpdate": "usage_update", "used": 10, "size": 20}));
        assert!(matches!(
            &events(&usage)[..],
            [AgentEvent::Usage { context_used: Some(10), context_window: Some(20), .. }]
        ));
        let commands = json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "plan", "description": "Plan first", "input": {"hint": "[goal]"}}, {"description": "no name"}]});
        let out = m.on_update(&commands);
        let [AcpOut::Commands(c)] = &out[..] else { panic!("{out:?}") };
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].name.as_str(), c[0].argument_hint.as_deref()), ("plan", Some("[goal]")));
        // A model picker from the config options, in a live update or the session's answer.
        let options = json!([{"id": "mode", "currentValue": "agent"},
            {"id": "model", "currentValue": "fast", "options": [
                {"value": "fast", "name": "Fast", "description": ""}, {"value": "deep", "description": "Thinks longer"}, {"name": "no value"}]}]);
        let out = m.on_update(&json!({"sessionUpdate": "config_option_update", "configOptions": options}));
        assert_eq!(out[0], AcpOut::Settings { mode: Some("agent".into()), model: Some("fast".into()) });
        let AcpOut::Models(models) = &out[1] else { panic!() };
        assert_eq!(models.len(), 2);
        assert_eq!((models[0].display_name.as_str(), models[0].description.as_deref()), ("Fast", None));
        assert_eq!(
            (models[1].display_name.as_str(), models[1].description.as_deref()),
            ("deep", Some("Thinks longer"))
        );
        assert_eq!(m.on_session(&json!({"configOptions": options})), out);
        assert!(m.on_session(&json!({"sessionId": "s"})).is_empty());
        // An empty thought is never shown; a message after a thought closes it.
        m.begin_prompt("go");
        let thought = |t: &str| json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": t}});
        m.on_update(&thought("  "));
        assert!(events(&m.on_update(&chunk)).is_empty());
        let ev = events(&m.on_update(&thought("hmm")));
        assert!(
            matches!(&ev[..], [AgentEvent::AgentMessage { markdown, phase: MessagePhase::Commentary, .. }] if markdown == "late")
        );
        let ev = events(&m.end_prompt(Ok(&json!({}))));
        assert!(matches!(&ev[0], AgentEvent::Reasoning { summary } if summary == "hmm"));
    }

    #[test]
    fn prompts_put_images_before_the_text() {
        assert_eq!(prompt_blocks("hi", &[]), json!([{"type": "text", "text": "hi"}]));
        assert_eq!(prompt_blocks("", &[]), json!([{"type": "text", "text": ""}]));
        let img = vec![("image/png".to_string(), "AAAA".to_string())];
        assert_eq!(prompt_blocks("  ", &img), json!([{"type": "image", "mimeType": "image/png", "data": "AAAA"}]));
        assert_eq!(
            prompt_blocks("look", &img),
            json!([{"type": "image", "mimeType": "image/png", "data": "AAAA"}, {"type": "text", "text": "look"}])
        );
    }

    /// An agent with the extras PDT's agents add: tool names, a session-scoped allow, plan requests, PDT's own tools,
    /// replayed prompts with times and its own question and plan requests.
    #[derive(Debug)]
    struct Extended;
    impl AcpExtensions for Extended {
        fn name(&self) -> &str {
            "Probe"
        }
        fn tool_name(&self, u: &Value) -> Option<String> {
            meta(u, "tool").and_then(Value::as_str).map(String::from)
        }
        fn is_exit_plan(&self, _u: &Value, tool: &str) -> bool {
            tool == "exit_plan"
        }
        fn written_plan(&self, tool: &str, input: &Value) -> Option<String> {
            (tool == "write_plan").then(|| s(input, "plan")).flatten()
        }
        fn rejected(&self, u: &Value) -> bool {
            meta(u, "rejected").is_some()
        }
        fn replayed_prompt(&self, u: &Value) -> (Option<String>, Option<i64>) {
            (meta(u, "id").and_then(Value::as_str).map(String::from), meta(u, "at").and_then(Value::as_i64))
        }
        fn session_option(&self) -> Option<&str> {
            Some("allow_session")
        }
        fn is_plan_request(&self, ids: &[&str]) -> bool {
            ids.contains(&"plan_build")
        }
        fn plan_option(&self, mode: Option<PermissionMode>) -> Option<&str> {
            Some(if mode == Some(PermissionMode::AcceptEdits) { "plan_edits" } else { "plan_build" })
        }
        fn pdt_tool_prefix(&self) -> Option<&str> {
            Some("mcp__pdt__")
        }
        fn ext_request(&self, method: &str, p: &Value) -> Option<ExtRequest> {
            match method {
                "probe/ask" => Some(ExtRequest::Questions(vec![ExtQuestion {
                    id: "q1".into(),
                    question: s(p, "question")?,
                    header: None,
                    options: vec![("o1".into(), "Yes".into()), ("o2".into(), "No".into())],
                    multi: false,
                }])),
                "probe/plan" => Some(ExtRequest::Plan { markdown: s(p, "plan")? }),
                _ => None,
            }
        }
        fn ext_answer(&self, method: &str, answer: &ExtAnswer) -> Value {
            json!({"method": method, "answer": format!("{answer:?}")})
        }
    }

    fn extended() -> AcpMapper<Extended> {
        AcpMapper::with_extensions(Extended, "/work/repo", Some("/private/work/repo".into()), "x")
    }

    fn permission(m: &mut AcpMapper<Extended>, id: u64, call: &str, options: &[(&str, &str)]) -> Vec<AcpOut> {
        let options: Vec<Value> = options.iter().map(|(id, kind)| json!({"optionId": id, "kind": kind})).collect();
        m.on_request(
            json!(id),
            "session/request_permission",
            &json!({"toolCall": {"toolCallId": call}, "options": options}),
        )
    }

    #[test]
    fn an_agents_extensions_name_tools_allow_for_the_session_and_approve_plans() {
        let mut m = extended();
        m.begin_prompt("go");
        // The canonical folder counts as the session's when the agent resolves symlinks… which `Extended` does not.
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "e", "kind": "edit",
            "rawInput": {"file_path": "/work/repo/src/a.rs"}, "_meta": {"tool": "edit_file"}}));
        let ev = events(&permission(
            &mut m,
            1,
            "e",
            &[("ok", "allow_once"), ("allow_session", "allow_always"), ("no", "reject_once")],
        ));
        let AgentEvent::Request { tool, title, can_always, id, .. } = &ev[0] else { panic!() };
        assert_eq!((tool.as_str(), title.as_str(), *can_always), ("edit_file", "src/a.rs", true));
        let a = m.answer(id, &Answer::Approve { always: true }).unwrap();
        assert_eq!(a.result["outcome"]["optionId"], "allow_session");
        // PDT's own tools run without asking.
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "p", "kind": "other", "_meta": {"tool": "mcp__pdt__get_ticket"}}));
        assert_eq!(
            permission(&mut m, 2, "p", &[("ok", "allow_once"), ("no", "reject_once")]),
            vec![AcpOut::Respond {
                id: json!(2),
                result: json!({"outcome": {"outcome": "selected", "optionId": "ok"}})
            }]
        );
        // A plan written in plan mode is what the exit-plan request asks to approve.
        m.on_update(&json!({"sessionUpdate": "tool_call", "toolCallId": "w", "kind": "edit",
            "rawInput": {"plan": "1. Do it"}, "_meta": {"tool": "write_plan"}}));
        m.on_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "x", "kind": "other", "_meta": {"tool": "exit_plan"}}),
        );
        let opts = [("plan_build", "allow_once"), ("plan_edits", "allow_always"), ("keep", "reject_once")];
        let ev = events(&permission(&mut m, 3, "x", &opts));
        let AgentEvent::Request { request_kind, input, id, .. } = &ev[0] else { panic!() };
        assert_eq!((*request_kind, input["plan"].as_str()), (RequestKind::Plan, Some("1. Do it")));
        assert_eq!(m.answer(id, &Answer::Question { answers: BTreeMap::new() }).unwrap_err(), AnswerError::WrongAnswer);
        let approve = Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::AcceptEdits) };
        assert_eq!(m.answer(id, &approve).unwrap().result["outcome"]["optionId"], "plan_edits");
        // A plan kept with feedback rejects it and tells the agent why; the call is then refused.
        let ev = events(&permission(&mut m, 4, "x", &opts));
        let AgentEvent::Request { id, .. } = &ev[0] else { panic!() };
        let keep = Answer::Plan { approve: false, feedback: Some("  smaller steps ".into()), mode: None };
        let a = m.answer(id, &keep).unwrap();
        assert_eq!(
            (a.result["outcome"]["optionId"].as_str(), a.follow_up.as_deref()),
            (Some("keep"), Some("smaller steps"))
        );
        // A call the agent says the user rejected is declined, not failed.
        let ev =
            events(&m.on_update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "e", "status": "failed",
            "_meta": {"rejected": true}})));
        let AgentEvent::ToolCall { status, .. } = &ev[0] else { panic!() };
        assert_eq!(*status, ToolStatus::Declined);
        let end = events(&m.end_prompt(Ok(&json!({"stopReason": "max_tokens"}))));
        assert!(
            matches!(end.last().unwrap(), AgentEvent::Turn { reason: Some(r), .. } if r == "Probe hit its output limit")
        );
    }

    #[test]
    fn an_agents_own_requests_are_answered_in_its_terms() {
        let mut m = extended();
        m.begin_prompt("go");
        let ev = events(&m.on_request(json!(1), "probe/ask", &json!({"question": "Ship it?", "toolCallId": "c"})));
        let AgentEvent::Request { request_kind, title, input, tool_call_id, .. } = &ev[0] else { panic!() };
        assert_eq!(
            (*request_kind, title.as_str(), tool_call_id.as_deref()),
            (RequestKind::Question, "Ship it?", Some("c"))
        );
        assert_eq!(input["questions"][0]["options"][1]["label"], "No");
        let pick = |label: &str| Answer::Question {
            answers: BTreeMap::from([("Ship it?".to_string(), vec![label.to_string()])]),
        };
        let a = m.answer("1", &pick("Yes")).unwrap();
        assert_eq!(a.result["answer"], r#"Answered([("q1", ["o1"])])"#);
        assert_eq!(a.rpc_id, json!(1));
        // Words of the user's own skip the question, saying what they wrote.
        m.on_request(json!(2), "probe/ask", &json!({"question": "Ship it?"}));
        let a = m.answer("2", &pick("Only on Friday")).unwrap();
        assert!(
            a.result["answer"]
                .as_str()
                .unwrap()
                .contains("The user answered in their own words. Ship it?: Only on Friday")
        );
        // A plan approved goes on in Ask (or the mode picked) and is then built.
        m.on_request(json!(3), "probe/plan", &json!({"plan": "- a"}));
        let a = m.answer("3", &Answer::Approve { always: false }).unwrap();
        assert_eq!(
            (a.mode, a.follow_up.as_deref()),
            (Some(PermissionMode::Ask), Some("The plan is approved: implement it now."))
        );
        m.on_request(json!(4), "probe/plan", &json!({"plan": "- a"}));
        let a =
            m.answer("4", &Answer::Plan { approve: true, feedback: None, mode: Some(PermissionMode::Bypass) }).unwrap();
        assert_eq!(a.mode, Some(PermissionMode::Bypass));
        m.on_request(json!(5), "probe/plan", &json!({"plan": "- a"}));
        let a = m.answer("5", &Answer::Deny { message: Some("no".into()) }).unwrap();
        assert_eq!(
            (a.result["answer"].as_str(), a.mode),
            (Some(r#"Plan { approve: false, feedback: Some("no") }"#), None)
        );
        m.on_request(json!(6), "probe/plan", &json!({"plan": "- a"}));
        assert_eq!(m.answer("6", &pick("x")).unwrap_err(), AnswerError::WrongAnswer);
        // Cancelling answers it in the agent's terms too.
        let (replies, _) = m.cancel_requests();
        assert_eq!(replies, [(json!(6), json!({"method": "probe/plan", "answer": "Cancelled"}))]);
        let out = m.on_request(json!(7), "probe/other", &json!({}));
        assert!(matches!(&out[..], [AcpOut::RespondError { .. }]));
    }

    #[test]
    fn loading_imports_only_turns_taken_outside_pdt() {
        let prompt = |id: &str, at: i64, text: &str| {
            json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": text},
                   "_meta": {"id": id, "at": at}})
        };
        let chunk = |t: &str| json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": t}});
        let mut m = extended();
        m.begin_load(Some(1000));
        // A turn PDT has logged already is dropped, with its answer.
        assert!(m.on_update(&prompt("p1", 500, "old")).is_empty());
        assert!(m.on_update(&chunk("old answer")).is_empty());
        // A newer one was typed in the agent's own terminal: it is imported.
        let ev = events(&m.on_update(&prompt("p2", 2000, "from the terminal")));
        assert!(
            matches!(&ev[1], AgentEvent::UserMessage { text, source: MessageSource::Terminal, .. } if text == "from the terminal")
        );
        // A prompt in several chunks is one prompt.
        assert!(m.on_update(&prompt("p2", 2000, "more")).is_empty());
        m.on_update(&chunk("new answer"));
        // The next replayed prompt ends the imported turn.
        let ev = events(&m.on_update(&prompt("p3", 3000, "and again")));
        let kinds: Vec<&str> = ev.iter().map(|e| e.kind()).collect();
        assert_eq!(kinds, ["agent_message", "turn", "turn", "user_message"]);
        let ev = events(&m.end_load());
        assert!(matches!(
            ev.last().unwrap(),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert_eq!(m.turn(), None);
        // Without `import_after`, nothing is imported.
        let mut m = extended();
        m.begin_load(None);
        assert!(m.on_update(&prompt("p9", 9000, "x")).is_empty());
        assert!(m.end_load().is_empty());
    }
}
