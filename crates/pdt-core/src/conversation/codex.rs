//! Codex's `codex app-server` protocol for one thread:
//! notifications and server requests → normalized events, and answers → JSON-RPC results. Pure; the adapter in
//! pdt-agents owns the server process and the connection.

use super::{
    AgentEvent, Answer, DeltaField, MAX_TOOL_OUTPUT, MessagePhase, MessageSource, NoticeLevel, PermissionMode,
    RateLimitWindow, RequestKind, RequestState, ToolStatus, TurnPhase, TurnStatus, cap_text,
};
use crate::conversation::claude::AnswerError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// What one message from the server turns into.
#[derive(Debug, Clone, PartialEq)]
// Short-lived (mapped, then stored or sent at once): boxing the event would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum CodexOut {
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
    /// Answer a server request now (one PDT does not handle).
    RespondError {
        id: Value,
        message: String,
    },
}

/// An answer to send for an open request.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexAnswer {
    /// The server request's JSON-RPC id.
    pub rpc_id: Value,
    pub result: Value,
    /// The changed request event.
    pub events: Vec<CodexOut>,
    /// Codex's decline carries no message: the user's note goes to the agent as the next message.
    pub follow_up: Option<String>,
}

/// Codex's approval policy and sandbox for a PDT permission mode. Plan has no stable Codex equivalent,
/// so it runs read-only and asks.
pub fn codex_policy(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Ask | PermissionMode::AcceptEdits => ("on-request", "workspace-write"),
        PermissionMode::Plan => ("on-request", "read-only"),
        PermissionMode::Bypass => ("never", "danger-full-access"),
    }
}

/// The `sandboxPolicy` object `turn/start` takes for a sandbox mode name; `network` lets commands in a sandbox reach
/// the network (the PM, PDT-100).
pub fn sandbox_policy(sandbox: &str, network: bool) -> Value {
    let mut p = match sandbox {
        "read-only" => json!({"type": "readOnly"}),
        "danger-full-access" => return json!({"type": "dangerFullAccess"}),
        _ => json!({"type": "workspaceWrite"}),
    };
    if network {
        p["networkAccess"] = json!(true);
    }
    p
}

pub fn item_key(id: &str) -> String {
    format!("i:{id}")
}

#[derive(Debug, Clone)]
struct ToolInfo {
    tool: String,
    title: String,
    input: Value,
}

#[derive(Debug, Clone)]
struct OpenRequest {
    rpc_id: Value,
    method: String,
    kind: RequestKind,
    tool: String,
    title: String,
    description: Option<String>,
    input: Value,
    can_always: bool,
    turn_id: Option<String>,
    /// Question text → Codex's question id.
    question_ids: BTreeMap<String, String>,
    /// Codex offers `decline` (deny, keep going); without it, deny is `cancel` (which also ends the turn).
    can_decline: bool,
}

/// The state of one thread's stream.
#[derive(Debug, Default)]
pub struct CodexMapper {
    turn: Option<String>,
    interrupt_requested: bool,
    /// Messages PDT sent that Codex has not echoed yet: an echo beyond these was typed in the TUI.
    own_messages: usize,
    tools: HashMap<String, ToolInfo>,
    requests: HashMap<String, OpenRequest>,
    rate_limits: BTreeMap<String, RateLimitWindow>,
    context_used: Option<i64>,
    context_window: Option<i64>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

/// `/bin/zsh -lc 'touch x'` → `touch x`: Codex runs commands through a login shell; its parsed actions name the
/// command itself.
fn command_title(item: &Value) -> String {
    let actions: Vec<&str> = item
        .get("commandActions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("command").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let raw = if actions.is_empty() {
        let c = item.get("command").and_then(Value::as_str).unwrap_or("");
        unwrap_shell(c).to_string()
    } else {
        actions.join(" && ")
    };
    one_line(&raw)
}

fn unwrap_shell(c: &str) -> &str {
    for prefix in ["/bin/zsh -lc ", "/bin/bash -lc ", "bash -lc ", "zsh -lc ", "/bin/sh -c "] {
        if let Some(rest) = c.strip_prefix(prefix) {
            return rest.trim_matches('\'').trim_matches('"');
        }
    }
    c
}

fn one_line(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(200).collect();
    if line.chars().count() > 200 || raw.lines().nth(1).is_some() {
        t.push('…');
    }
    t
}

fn file_titles(changes: &[Value]) -> String {
    let paths: Vec<&str> = changes.iter().filter_map(|c| c.get("path").and_then(Value::as_str)).collect();
    match paths.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} +{} more", rest.len()),
    }
}

fn file_diff(changes: &[Value]) -> Option<String> {
    let parts: Vec<String> = changes
        .iter()
        .filter_map(|c| {
            let path = c.get("path").and_then(Value::as_str)?;
            let diff = c.get("diff").and_then(Value::as_str).unwrap_or("");
            Some(if diff.starts_with("---") || diff.starts_with("diff ") {
                diff.to_string()
            } else {
                format!("--- a/{path}\n+++ b/{path}\n{diff}")
            })
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn tool_status(status: Option<&str>, done: bool) -> ToolStatus {
    match status {
        Some("failed") => ToolStatus::Failed,
        Some("declined") => ToolStatus::Declined,
        Some("completed") => ToolStatus::Done,
        _ if done => ToolStatus::Done,
        _ => ToolStatus::Running,
    }
}

fn user_text(content: &Value) -> (String, Vec<String>) {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for c in content.as_array().into_iter().flatten() {
        match c.get("type").and_then(Value::as_str) {
            Some("text") => text.push(c.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
            Some("image") | Some("localImage") => images.push("image".to_string()),
            _ => {}
        }
    }
    (text.join("\n"), images)
}

/// The tool-call view of an item that runs something; None for messages and other non-tool items.
fn tool_of(item: &Value) -> Option<ToolInfo> {
    let kind = item.get("type").and_then(Value::as_str)?;
    let info = match kind {
        "commandExecution" => ToolInfo {
            tool: "Bash".into(),
            title: command_title(item),
            input: json!({"command": unwrap_shell(item.get("command").and_then(Value::as_str).unwrap_or("")),
                          "cwd": item.get("cwd")}),
        },
        "fileChange" => {
            let changes = item.get("changes").and_then(Value::as_array).cloned().unwrap_or_default();
            ToolInfo { tool: "Edit".into(), title: file_titles(&changes), input: json!({"changes": changes}) }
        }
        "mcpToolCall" => ToolInfo {
            tool: format!("mcp:{}/{}", s(item, "server").unwrap_or_default(), s(item, "tool").unwrap_or_default()),
            title: s(item, "tool").unwrap_or_default(),
            input: item.get("arguments").cloned().unwrap_or(Value::Null),
        },
        "dynamicToolCall" => ToolInfo {
            tool: s(item, "tool").unwrap_or_else(|| "tool".into()),
            title: String::new(),
            input: item.get("arguments").cloned().unwrap_or(Value::Null),
        },
        "webSearch" => ToolInfo {
            tool: "WebSearch".into(),
            title: one_line(&s(item, "query").unwrap_or_default()),
            input: json!({"query": item.get("query")}),
        },
        "imageView" => ToolInfo {
            tool: "Read".into(),
            title: s(item, "path").unwrap_or_default(),
            input: json!({"file_path": item.get("path")}),
        },
        "collabAgentToolCall"
        | "subAgentActivity"
        | "imageGeneration"
        | "sleep"
        | "enteredReviewMode"
        | "exitedReviewMode" => ToolInfo { tool: kind.to_string(), title: String::new(), input: item.clone() },
        _ => return None,
    };
    Some(info)
}

impl CodexMapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn turn(&self) -> Option<&str> {
        self.turn.as_deref()
    }

    pub fn open_requests(&self) -> usize {
        self.requests.len()
    }

    /// PDT asked to interrupt: a turn that ends `interrupted` was the user's doing, not a failure.
    pub fn note_interrupt(&mut self) {
        self.interrupt_requested = true;
    }

    /// PDT sent a message; its echo is PDT's (or a steer), not the TUI's.
    pub fn note_sent(&mut self) {
        self.own_messages += 1;
    }

    /// A message PDT tried to send did not reach Codex.
    pub fn note_unsent(&mut self) {
        self.own_messages = self.own_messages.saturating_sub(1);
    }

    fn event(&self, item_id: Option<String>, event: AgentEvent) -> CodexOut {
        CodexOut::Event { item_id, turn_id: self.turn.clone(), event }
    }

    fn request_event(&self, id: &str, r: &OpenRequest, state: RequestState, answer: Option<Answer>) -> CodexOut {
        CodexOut::Event {
            item_id: Some(format!("r:{id}")),
            turn_id: r.turn_id.clone(),
            event: AgentEvent::Request {
                id: id.to_string(),
                request_kind: r.kind,
                tool: r.tool.clone(),
                title: r.title.clone(),
                description: r.description.clone(),
                input: r.input.clone(),
                tool_call_id: (r.kind == RequestKind::Approval).then(|| id.to_string()),
                can_always: r.can_always,
                diff: (r.tool == "Edit")
                    .then(|| {
                        file_diff(r.input.get("changes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]))
                    })
                    .flatten(),
                state,
                answer,
            },
        }
    }

    fn usage_event(&self) -> CodexOut {
        CodexOut::Event {
            item_id: Some(super::claude::USAGE_ITEM.into()),
            turn_id: None,
            event: AgentEvent::Usage {
                context_used: self.context_used,
                context_window: self.context_window,
                rate_limits: self.rate_limits.values().cloned().collect(),
                cost_usd: None,
            },
        }
    }

    fn expire(&mut self, only_turn: Option<&str>) -> Vec<CodexOut> {
        let mut ids: Vec<String> = self
            .requests
            .iter()
            .filter(|(_, r)| only_turn.is_none_or(|t| r.turn_id.as_deref() == Some(t)))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                self.requests.remove(&id).map(|r| self.request_event(&id, &r, RequestState::Expired, None))
            })
            .collect()
    }

    /// The connection or the server went away: open requests expire and a running turn ends as interrupted.
    pub fn on_exit(&mut self, reason: &str) -> Vec<CodexOut> {
        let mut out = self.expire(None);
        if let Some(id) = self.turn.take() {
            out.push(CodexOut::Event {
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
        out
    }

    /// The event for a completed item (live or from history). `source` is for user messages.
    pub fn item_event(&mut self, item: &Value, done: bool, source: MessageSource) -> Option<(String, AgentEvent)> {
        let id = s(item, "id")?;
        let key = item_key(&id);
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        let event = match kind {
            "userMessage" if done => {
                let (text, images) = user_text(&item["content"]);
                AgentEvent::UserMessage { text, images, source, pdt: None }
            }
            "agentMessage" if done => AgentEvent::AgentMessage {
                id: id.clone(),
                markdown: s(item, "text").unwrap_or_default(),
                phase: if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
                    MessagePhase::Final
                } else {
                    MessagePhase::Commentary
                },
            },
            "reasoning" if done => {
                let summary: Vec<String> = item
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default();
                let summary = summary.join("\n\n");
                if summary.trim().is_empty() {
                    return None;
                }
                AgentEvent::Reasoning { summary }
            }
            "plan" if done => AgentEvent::Plan { markdown: s(item, "text").unwrap_or_default() },
            "contextCompaction" if done => AgentEvent::Compaction { before: None, after: None },
            "userMessage" | "agentMessage" | "reasoning" | "plan" | "contextCompaction" | "hookPrompt"
            | "functionCallOutput" => return None,
            _ => {
                let info = tool_of(item).or_else(|| self.tools.get(&id).cloned())?;
                if done {
                    self.tools.remove(&id);
                } else {
                    self.tools.insert(id.clone(), info.clone());
                }
                let status = tool_status(item.get("status").and_then(Value::as_str), done);
                let output = match kind {
                    "commandExecution" => s(item, "aggregatedOutput"),
                    "mcpToolCall" => item
                        .get("error")
                        .filter(|e| !e.is_null())
                        .or(item.get("result").filter(|r| !r.is_null()))
                        .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())),
                    _ => None,
                }
                .filter(|_| done)
                .map(|o| cap_text(&o, MAX_TOOL_OUTPUT));
                let diff = if kind == "fileChange" {
                    file_diff(item.get("changes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]))
                } else {
                    None
                };
                AgentEvent::ToolCall {
                    id: id.clone(),
                    tool: info.tool,
                    title: info.title,
                    input: info.input,
                    status,
                    output,
                    exit_code: item.get("exitCode").and_then(Value::as_i64).map(|c| c as i32),
                    diff,
                    parent_id: None,
                }
            }
        };
        Some((key, event))
    }

    /// Maps a notification for this thread (or a global one, such as rate limits).
    pub fn on_notification(&mut self, method: &str, p: &Value) -> Vec<CodexOut> {
        let mut out = Vec::new();
        match method {
            "turn/started" => {
                let id = p.pointer("/turn/id").and_then(Value::as_str).unwrap_or_default().to_string();
                self.turn = Some(id.clone());
                out.push(CodexOut::Event {
                    item_id: Some(format!("turn:{id}")),
                    turn_id: Some(id),
                    event: AgentEvent::Turn {
                        phase: TurnPhase::Started,
                        status: None,
                        duration_ms: None,
                        reason: None,
                    },
                });
            }
            "turn/completed" => {
                let turn = &p["turn"];
                let id = s(turn, "id").or(self.turn.clone()).unwrap_or_default();
                out.extend(self.expire(Some(&id)));
                let status = match turn.get("status").and_then(Value::as_str) {
                    Some("interrupted") => TurnStatus::Interrupted,
                    Some("failed") => TurnStatus::Failed,
                    _ => TurnStatus::Completed,
                };
                let error = turn.pointer("/error/message").and_then(Value::as_str).map(String::from);
                let reason = match status {
                    TurnStatus::Completed => None,
                    TurnStatus::Interrupted => Some("Interrupted".to_string()),
                    TurnStatus::Failed => Some(error.clone().unwrap_or_else(|| "The turn failed".into())),
                };
                if status == TurnStatus::Failed {
                    out.push(
                        self.event(
                            None,
                            AgentEvent::Error { text: reason.clone().unwrap_or_default(), retryable: false },
                        ),
                    );
                }
                out.push(CodexOut::Event {
                    item_id: Some(format!("turn-end:{id}")),
                    turn_id: Some(id),
                    event: AgentEvent::Turn {
                        phase: TurnPhase::Ended,
                        status: Some(status),
                        duration_ms: turn.get("durationMs").and_then(Value::as_i64),
                        reason,
                    },
                });
                self.turn = None;
                self.interrupt_requested = false;
            }
            "item/started" | "item/completed" => {
                let done = method == "item/completed";
                let item = &p["item"];
                let source = if item.get("type").and_then(Value::as_str) == Some("userMessage") && done {
                    if self.own_messages > 0 {
                        self.own_messages -= 1;
                        MessageSource::Pdt
                    } else {
                        MessageSource::Terminal
                    }
                } else {
                    MessageSource::Pdt
                };
                if let Some((key, event)) = self.item_event(item, done, source) {
                    out.push(self.event(Some(key), event));
                }
            }
            "item/agentMessage/delta" => {
                if let (Some(id), Some(text)) = (s(p, "itemId"), s(p, "delta"))
                    && !text.is_empty()
                {
                    out.push(CodexOut::Delta { item_id: item_key(&id), field: DeltaField::Markdown, text });
                }
            }
            "item/commandExecution/outputDelta" => {
                if let (Some(id), Some(text)) = (s(p, "itemId"), s(p, "delta"))
                    && !text.is_empty()
                {
                    out.push(CodexOut::Delta { item_id: item_key(&id), field: DeltaField::Output, text });
                }
            }
            "thread/tokenUsage/updated" => {
                let u = &p["tokenUsage"];
                if let Some(t) = u.pointer("/last/totalTokens").and_then(Value::as_i64) {
                    self.context_used = Some(t);
                }
                if let Some(w) = u.get("modelContextWindow").and_then(Value::as_i64) {
                    self.context_window = Some(w);
                }
                out.push(self.usage_event());
            }
            "account/rateLimits/updated" => {
                let r = &p["rateLimits"];
                for key in ["primary", "secondary"] {
                    let Some(w) = r.get(key).filter(|w| w.is_object()) else { continue };
                    let mins = w.get("windowDurationMins").and_then(Value::as_i64);
                    let name = match mins {
                        Some(300) => "five_hour".to_string(),
                        Some(10080) => "seven_day".to_string(),
                        Some(m) => format!("{m}_minutes"),
                        None => key.to_string(),
                    };
                    let window = RateLimitWindow {
                        name: name.clone(),
                        utilization: w.get("usedPercent").and_then(Value::as_f64).unwrap_or(0.0) / 100.0,
                        resets_at: w.get("resetsAt").and_then(Value::as_i64).map(|s| s * 1000),
                    };
                    self.rate_limits.insert(name, window);
                }
                out.push(self.usage_event());
                if r.get("rateLimitReachedType").is_some_and(|t| !t.is_null()) {
                    let until =
                        self.rate_limits.values().filter(|w| w.utilization >= 1.0).filter_map(|w| w.resets_at).max();
                    let text =
                        "Codex's usage limit is reached. Nothing is sent to it until you send again.".to_string();
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until }));
                }
            }
            "turn/diff/updated" => {
                let turn = s(p, "turnId").or(self.turn.clone()).unwrap_or_default();
                let diff = s(p, "diff").unwrap_or_default();
                out.push(CodexOut::Event {
                    item_id: Some(format!("diff:{turn}")),
                    turn_id: Some(turn),
                    event: AgentEvent::TurnDiff { diff },
                });
            }
            "turn/plan/updated" => {
                let turn = s(p, "turnId").or(self.turn.clone()).unwrap_or_default();
                let steps: Vec<String> = p
                    .get("plan")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|x| {
                                let done = x.get("status").and_then(Value::as_str) == Some("completed");
                                format!("- [{}] {}", if done { "x" } else { " " }, s(x, "step").unwrap_or_default())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut markdown = s(p, "explanation").map(|e| format!("{e}\n\n")).unwrap_or_default();
                markdown.push_str(&steps.join("\n"));
                out.push(CodexOut::Event {
                    item_id: Some(format!("plan:{turn}")),
                    turn_id: Some(turn),
                    event: AgentEvent::Plan { markdown },
                });
            }
            "serverRequest/resolved" => {
                let rpc = p.get("requestId").cloned().unwrap_or(Value::Null);
                if let Some(id) = self.requests.iter().find(|(_, r)| r.rpc_id == rpc).map(|(id, _)| id.clone())
                    && let Some(r) = self.requests.remove(&id)
                {
                    // Answered in the TUI, or dropped because PDT interrupted the turn.
                    let state = if self.interrupt_requested { RequestState::Expired } else { RequestState::Answered };
                    out.push(self.request_event(&id, &r, state, None));
                }
            }
            "error" => {
                let text = p
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex reported an error")
                    .to_string();
                if p.get("willRetry").and_then(Value::as_bool).unwrap_or(false) {
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None }));
                } else {
                    out.push(self.event(None, AgentEvent::Error { text, retryable: false }));
                }
            }
            "warning" => {
                let text = s(p, "message").unwrap_or_default();
                if !text.is_empty() {
                    out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Warning, text, until: None }));
                }
            }
            "model/rerouted" => {
                let to = s(p, "toModel").unwrap_or_default();
                let text = format!("Codex switched this turn to {to}");
                out.push(self.event(None, AgentEvent::Notice { level: NoticeLevel::Info, text, until: None }));
            }
            // Bookkeeping that the events above already cover, or that PDT does not show.
            "thread/started"
            | "thread/status/changed"
            | "thread/readState/changed"
            | "thread/settings/updated"
            | "thread/name/updated"
            | "thread/goal/updated"
            | "thread/goal/cleared"
            | "thread/compacted"
            | "thread/closed"
            | "thread/archived"
            | "thread/unarchived"
            | "thread/queue/changed"
            | "thread/metadata/updated"
            | "thread/prediction/updated"
            | "thread/environment/connected"
            | "thread/environment/disconnected"
            | "turn/moderationMetadata"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/summaryPartAdded"
            | "item/reasoning/textDelta"
            | "item/plan/delta"
            | "item/fileChange/outputDelta"
            | "item/fileChange/patchUpdated"
            | "item/mcpToolCall/progress"
            | "item/commandExecution/terminalInteraction"
            | "item/autoApprovalReview/started"
            | "item/autoApprovalReview/completed"
            | "hook/started"
            | "hook/completed"
            | "mcpServer/startupStatus/updated"
            | "remoteControl/status/changed"
            | "account/updated"
            | "skills/changed"
            | "configWarning"
            | "deprecationNotice"
            | "model/verification"
            | "model/safetyBuffering/updated"
            | "thread/attachment/updated"
            | "thread/project/updated"
            | "project/changed" => {}
            _ => out.push(self.event(None, AgentEvent::Unknown { raw: json!({"method": method, "params": p}) })),
        }
        out
    }

    /// Maps a server request for this thread: approvals and questions become requests; the rest are refused.
    pub fn on_server_request(&mut self, rpc_id: Value, method: &str, p: &Value) -> Vec<CodexOut> {
        let item = s(p, "itemId").unwrap_or_default();
        let turn_id = s(p, "turnId").or(self.turn.clone());
        let known = self.tools.get(&item).cloned();
        let open = match method {
            "item/commandExecution/requestApproval" => {
                let title = command_title(p);
                // Only the session-scoped allow: the exec-policy amendment would persist in the user's Codex config.
                let decisions = p.get("availableDecisions").and_then(Value::as_array);
                let can_always = decisions.is_none_or(|d| d.iter().any(|x| x == "acceptForSession"));
                let can_decline = decisions.is_none_or(|d| d.iter().any(|x| x == "decline"));
                OpenRequest {
                    rpc_id,
                    method: method.into(),
                    kind: RequestKind::Approval,
                    tool: "Bash".into(),
                    title: if title.is_empty() { known.map(|k| k.title).unwrap_or_default() } else { title },
                    description: s(p, "reason"),
                    input: json!({"command": unwrap_shell(p.get("command").and_then(Value::as_str).unwrap_or("")),
                                  "cwd": p.get("cwd")}),
                    can_always,
                    turn_id,
                    question_ids: BTreeMap::new(),
                    can_decline,
                }
            }
            "item/fileChange/requestApproval" => OpenRequest {
                rpc_id,
                method: method.into(),
                kind: RequestKind::Approval,
                tool: "Edit".into(),
                title: known.as_ref().map(|k| k.title.clone()).unwrap_or_default(),
                description: s(p, "reason"),
                input: known.map(|k| k.input).unwrap_or_else(|| json!({})),
                can_always: true,
                turn_id,
                question_ids: BTreeMap::new(),
                can_decline: true,
            },
            "item/permissions/requestApproval" => OpenRequest {
                rpc_id,
                method: method.into(),
                kind: RequestKind::Approval,
                tool: "Permissions".into(),
                title: s(p, "reason").unwrap_or_else(|| "More permissions".into()),
                description: None,
                input: json!({"permissions": p.get("permissions"), "cwd": p.get("cwd")}),
                can_always: true,
                turn_id,
                question_ids: BTreeMap::new(),
                can_decline: true,
            },
            "item/tool/requestUserInput" => {
                let qs = p.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut ids = BTreeMap::new();
                let questions: Vec<Value> = qs
                    .iter()
                    .map(|q| {
                        let text = s(q, "question").unwrap_or_default();
                        ids.insert(text.clone(), s(q, "id").unwrap_or_default());
                        json!({"question": text, "header": q.get("header"), "multiSelect": false,
                               "options": q.get("options").cloned().unwrap_or(json!([])),
                               "isOther": q.get("isOther")})
                    })
                    .collect();
                OpenRequest {
                    rpc_id,
                    method: method.into(),
                    kind: RequestKind::Question,
                    tool: "AskUserQuestion".into(),
                    title: questions.first().and_then(|q| q["question"].as_str()).unwrap_or("").to_string(),
                    description: None,
                    input: json!({"questions": questions}),
                    can_always: false,
                    turn_id,
                    question_ids: ids,
                    can_decline: true,
                }
            }
            _ => {
                return vec![CodexOut::RespondError {
                    id: rpc_id,
                    message: format!("{method} is not supported by PDT"),
                }];
            }
        };
        let id = if item.is_empty() { format!("req-{}", open.rpc_id) } else { item };
        let ev = self.request_event(&id, &open, RequestState::Open, None);
        self.requests.insert(id, open);
        vec![ev]
    }

    /// The JSON-RPC result answering an open request.
    pub fn answer(&mut self, request_id: &str, answer: &Answer) -> Result<CodexAnswer, AnswerError> {
        let r = self.requests.get(request_id).ok_or(AnswerError::Closed)?;
        let mut follow_up = None;
        let result = match (r.kind, answer) {
            (RequestKind::Question, Answer::Question { answers }) => {
                let mut out = serde_json::Map::new();
                for (q, labels) in answers {
                    let id = r.question_ids.get(q).cloned().unwrap_or_else(|| q.clone());
                    out.insert(id, json!({"answers": labels}));
                }
                json!({"answers": out})
            }
            (RequestKind::Question, Answer::Deny { .. }) => json!({"answers": {}}),
            (RequestKind::Approval, Answer::Approve { always }) if r.method == "item/permissions/requestApproval" => {
                json!({"permissions": r.input.get("permissions").cloned().unwrap_or(json!({})),
                       "scope": if *always { "session" } else { "turn" }})
            }
            (RequestKind::Approval, Answer::Deny { message }) if r.method == "item/permissions/requestApproval" => {
                follow_up = message.clone().filter(|m| !m.trim().is_empty());
                json!({"permissions": {}})
            }
            (RequestKind::Approval, Answer::Approve { always }) => {
                json!({"decision": if *always && r.can_always { "acceptForSession" } else { "accept" }})
            }
            (RequestKind::Approval, Answer::Deny { message }) => {
                follow_up = message.clone().filter(|m| !m.trim().is_empty());
                json!({"decision": if r.can_decline { "decline" } else { "cancel" }})
            }
            _ => return Err(AnswerError::WrongAnswer),
        };
        let r = self.requests.remove(request_id).ok_or(AnswerError::Closed)?;
        let events = vec![self.request_event(request_id, &r, RequestState::Answered, Some(answer.clone()))];
        Ok(CodexAnswer { rpc_id: r.rpc_id, result, events, follow_up })
    }
}

/// Events for turns from `thread/turns/list` (oldest first) that are not in `known_turns`, for importing history
/// (turns taken while PDT was not listening). Each turn: started, its items, ended.
pub fn history_events(turns: &[Value], known_turns: &std::collections::HashSet<String>) -> Vec<CodexOut> {
    let mut m = CodexMapper::new();
    let mut out = Vec::new();
    for t in turns {
        let Some(id) = s(t, "id") else { continue };
        if known_turns.contains(&id) {
            continue;
        }
        m.turn = Some(id.clone());
        out.push(CodexOut::Event {
            item_id: Some(format!("turn:{id}")),
            turn_id: Some(id.clone()),
            event: AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None },
        });
        for item in t.get("items").and_then(Value::as_array).into_iter().flatten() {
            if let Some((key, event)) = m.item_event(item, true, MessageSource::Terminal) {
                out.push(CodexOut::Event { item_id: Some(key), turn_id: Some(id.clone()), event });
            }
        }
        let status = match t.get("status").and_then(Value::as_str) {
            Some("inProgress") => continue,
            Some("interrupted") => TurnStatus::Interrupted,
            Some("failed") => TurnStatus::Failed,
            _ => TurnStatus::Completed,
        };
        out.push(CodexOut::Event {
            item_id: Some(format!("turn-end:{id}")),
            turn_id: Some(id.clone()),
            event: AgentEvent::Turn {
                phase: TurnPhase::Ended,
                status: Some(status),
                duration_ms: t.get("durationMs").and_then(Value::as_i64),
                reason: None,
            },
        });
    }
    out
}

/// The Codex commands PDT runs (Codex has no generic "run a slash command").
pub fn commands() -> Vec<super::AgentCommandInfo> {
    vec![
        super::AgentCommandInfo {
            name: "compact".into(),
            description: "Summarise the conversation to free up context".into(),
            argument_hint: None,
        },
        super::AgentCommandInfo {
            name: "review".into(),
            description: "Review the uncommitted changes (or follow your instructions)".into(),
            argument_hint: Some("[instructions]".into()),
        },
    ]
}

/// A message that is one of [`commands`]: the RPC and its params (without `threadId`). Anything else is a message.
pub fn slash_command(text: &str) -> Option<(&'static str, Value)> {
    let t = text.trim();
    let (name, args) = t.strip_prefix('/')?.split_once(char::is_whitespace).unwrap_or((t.strip_prefix('/')?, ""));
    let args = args.trim();
    match name {
        "compact" if args.is_empty() => Some(("thread/compact/start", json!({}))),
        "review" if args.is_empty() => Some(("review/start", json!({"target": {"type": "uncommittedChanges"}}))),
        "review" => Some(("review/start", json!({"target": {"type": "custom", "instructions": args}}))),
        _ => None,
    }
}

/// Who pays, from `account/read` (only the account's type is read): "ChatGPT plan", "API key", "Amazon Bedrock".
pub fn billing_label(account_read: &Value) -> Option<String> {
    let label = match account_read.pointer("/account/type").and_then(Value::as_str)? {
        "chatgpt" => "ChatGPT plan",
        "apiKey" => "API key",
        "amazonBedrock" => "Amazon Bedrock",
        _ => return None,
    };
    Some(label.to_string())
}

/// A turn's input: the text, then each image by its local path.
pub fn user_input(text: &str, image_paths: &[String]) -> Value {
    let mut items = Vec::new();
    if !text.trim().is_empty() || image_paths.is_empty() {
        items.push(json!({"type": "text", "text": text}));
    }
    items.extend(image_paths.iter().map(|p| json!({"type": "localImage", "path": p})));
    Value::Array(items)
}

/// Follows a hidden side turn (Make ticket's draft): keeps the last agent message and says how the turn ended.
#[derive(Debug, Default)]
pub struct SideTurn {
    last: Option<String>,
}

impl SideTurn {
    /// Some once the turn is over: the last agent message, or why there is none.
    pub fn on_notification(&mut self, method: &str, p: &Value) -> Option<Result<String, String>> {
        match method {
            "item/completed" if p.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage") => {
                self.last = p.pointer("/item/text").and_then(Value::as_str).map(String::from);
                None
            }
            "turn/completed" => {
                let turn = &p["turn"];
                match turn.get("status").and_then(Value::as_str) {
                    Some("interrupted") => Some(Err("the side request was interrupted".into())),
                    Some("failed") => Some(Err(turn
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("the side request failed")
                        .to_string())),
                    _ => Some(self.last.take().ok_or_else(|| "Codex gave no answer".to_string())),
                }
            }
            "error" if !p.get("willRetry").and_then(Value::as_bool).unwrap_or(false) => Some(Err(p
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex reported an error")
                .to_string())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_side_turn_answers_with_its_last_message() {
        let mut t = SideTurn::default();
        let msg = |text: &str| json!({"item": {"type": "agentMessage", "id": "i", "text": text}});
        assert_eq!(t.on_notification("item/completed", &msg("first")), None);
        assert_eq!(t.on_notification("item/completed", &json!({"item": {"type": "reasoning"}})), None);
        assert_eq!(t.on_notification("item/completed", &msg("{\"title\":\"x\"}")), None);
        assert_eq!(
            t.on_notification("turn/completed", &json!({"turn": {"status": "completed"}})),
            Some(Ok("{\"title\":\"x\"}".into()))
        );
        let mut empty = SideTurn::default();
        assert!(matches!(empty.on_notification("turn/completed", &json!({"turn": {}})), Some(Err(_))));
        let failed = json!({"turn": {"status": "failed", "error": {"message": "usage limit reached"}}});
        assert_eq!(
            SideTurn::default().on_notification("turn/completed", &failed),
            Some(Err("usage limit reached".into()))
        );
        let retrying = json!({"error": {"message": "reconnecting"}, "willRetry": true});
        assert_eq!(SideTurn::default().on_notification("error", &retrying), None);
        assert!(matches!(SideTurn::default().on_notification("error", &json!({"error": {}})), Some(Err(_))));
    }

    #[test]
    fn commands_map_to_their_rpcs_and_images_to_local_paths() {
        assert_eq!(slash_command(" /compact "), Some(("thread/compact/start", json!({}))));
        assert_eq!(slash_command("/review"), Some(("review/start", json!({"target": {"type": "uncommittedChanges"}}))));
        assert_eq!(
            slash_command("/review  the auth code"),
            Some(("review/start", json!({"target": {"type": "custom", "instructions": "the auth code"}})))
        );
        assert_eq!(slash_command("/compact now"), None);
        assert_eq!(slash_command("/model gpt"), None);
        assert_eq!(slash_command("compact"), None);
        assert_eq!(commands().len(), 2);
        assert_eq!(
            billing_label(&json!({"account": {"type": "chatgpt", "email": "x", "planType": "pro"}})).as_deref(),
            Some("ChatGPT plan")
        );
        assert_eq!(billing_label(&json!({"account": null})), None);
        assert_eq!(user_input("hi", &[]), json!([{"type": "text", "text": "hi"}]));
        assert_eq!(user_input("", &["/d/a.png".into()]), json!([{"type": "localImage", "path": "/d/a.png"}]));
    }

    const APPROVAL: &str = include_str!("../../tests/fixtures/codex/approval.jsonl");

    /// Feeds recorded frames (notifications and server requests) up to and including the first line matching `stop`.
    fn feed(m: &mut CodexMapper, lines: &[&str]) -> Vec<CodexOut> {
        let mut out = Vec::new();
        for l in lines {
            let v: Value = serde_json::from_str(l).unwrap();
            let method = v["method"].as_str().unwrap_or("");
            if v.get("id").is_some() {
                out.extend(m.on_server_request(v["id"].clone(), method, &v["params"]));
            } else {
                out.extend(m.on_notification(method, &v["params"]));
            }
        }
        out
    }

    fn events(out: &[CodexOut]) -> Vec<(Option<String>, AgentEvent)> {
        out.iter()
            .filter_map(|o| match o {
                CodexOut::Event { item_id, event, .. } => Some((item_id.clone(), event.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_recorded_turn_with_an_approval_maps_to_events() {
        let lines: Vec<&str> = APPROVAL.lines().collect();
        let at = lines.iter().position(|l| l.contains("requestApproval")).unwrap();
        let mut m = CodexMapper::new();
        m.note_sent();
        let out = feed(&mut m, &lines[..=at]);
        let ev = events(&out);
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(kinds, ["turn", "user_message", "agent_message", "tool_call", "request"], "{ev:#?}");
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { source: MessageSource::Pdt, .. }));
        assert!(matches!(&ev[2].1, AgentEvent::AgentMessage { phase: MessagePhase::Commentary, .. }));
        let AgentEvent::ToolCall { title, tool, status, .. } = &ev[3].1 else { panic!() };
        assert_eq!((title.as_str(), tool.as_str(), *status), ("touch codex_probe.txt", "Bash", ToolStatus::Running));
        let AgentEvent::Request { id, can_always, title, .. } = &ev[4].1 else { panic!() };
        assert!(!can_always, "Codex offered only a persistent rule, which PDT never writes");
        assert_eq!(title, "touch codex_probe.txt");
        assert!(out.iter().any(|o| matches!(o, CodexOut::Delta { field: DeltaField::Markdown, .. })));

        let a = m.answer(id, &Answer::Approve { always: true }).unwrap();
        assert_eq!(a.result, json!({"decision": "accept"}));
        assert_eq!(a.rpc_id, json!(0));
        assert_eq!(m.answer(id, &Answer::Approve { always: false }), Err(AnswerError::Closed));

        m.note_sent();
        let rest = feed(&mut m, &lines[at + 1..]);
        let ev = events(&rest);
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(
            kinds,
            [
                "tool_call",
                "usage",
                "usage",
                "agent_message",
                "usage",
                "usage",
                "turn",
                "turn",
                "user_message",
                "agent_message",
                "usage",
                "usage",
                "turn"
            ],
        );
        assert!(matches!(&ev[0].1, AgentEvent::ToolCall { status: ToolStatus::Done, exit_code: Some(0), .. }));
        assert!(
            matches!(&ev[3].1, AgentEvent::AgentMessage { phase: MessagePhase::Final, markdown, .. } if markdown == "done")
        );
        let AgentEvent::Usage { context_used, context_window, rate_limits, .. } = &ev[1].1 else { panic!() };
        assert!(context_used.is_some() && *context_window == Some(475_000));
        assert!(rate_limits.is_empty(), "rate limits arrive after token usage");
        let AgentEvent::Usage { rate_limits, .. } = &ev[2].1 else { panic!() };
        assert_eq!(rate_limits.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["five_hour", "seven_day"]);
        assert!(matches!(
            &ev[6].1,
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), .. }
        ));
        assert!(m.turn().is_none());
    }

    #[test]
    fn messages_typed_in_the_tui_are_marked_and_resolved_requests_close() {
        let mut m = CodexMapper::new();
        let user = json!({"item": {"type": "userMessage", "id": "u1", "content": [{"type": "text", "text": "hi"}]}});
        let ev = events(&m.on_notification("item/completed", &user));
        assert!(matches!(&ev[0].1, AgentEvent::UserMessage { source: MessageSource::Terminal, .. }));
        m.on_server_request(
            json!(7),
            "item/commandExecution/requestApproval",
            &json!({"itemId": "x", "command": "ls"}),
        );
        let ev = events(&m.on_notification("serverRequest/resolved", &json!({"requestId": 7})));
        let decisions =
            json!({"itemId": "y", "command": "ls", "availableDecisions": ["accept", "acceptForSession", "cancel"]});
        m.on_server_request(json!(8), "item/commandExecution/requestApproval", &decisions);
        let a = m.answer("y", &Answer::Deny { message: None }).unwrap();
        assert_eq!(a.result, json!({"decision": "cancel"}), "no decline offered");
        assert!(matches!(&ev[0].1, AgentEvent::Request { state: RequestState::Answered, answer: None, .. }));
        assert_eq!(m.open_requests(), 0);
    }

    #[test]
    fn deny_sends_the_note_as_a_follow_up_and_questions_use_codex_ids() {
        let mut m = CodexMapper::new();
        m.on_server_request(json!(1), "item/fileChange/requestApproval", &json!({"itemId": "f"}));
        let a = m.answer("f", &Answer::Deny { message: Some("not that file".into()) }).unwrap();
        assert_eq!((a.result, a.follow_up.as_deref()), (json!({"decision": "decline"}), Some("not that file")));
        let q = json!({"itemId": "q", "questions": [{"id": "q1", "header": "H", "question": "Which?",
                       "options": [{"label": "A", "description": ""}]}]});
        let ev = events(&m.on_server_request(json!(2), "item/tool/requestUserInput", &q));
        assert!(matches!(&ev[0].1, AgentEvent::Request { request_kind: RequestKind::Question, .. }));
        assert_eq!(m.answer("q", &Answer::Approve { always: false }), Err(AnswerError::WrongAnswer));
        let answers = BTreeMap::from([("Which?".to_string(), vec!["A".to_string()])]);
        let a = m.answer("q", &Answer::Question { answers }).unwrap();
        assert_eq!(a.result, json!({"answers": {"q1": {"answers": ["A"]}}}));
        let out = m.on_server_request(json!(3), "item/tool/call", &json!({}));
        assert!(matches!(&out[0], CodexOut::RespondError { .. }));
    }

    #[test]
    fn file_changes_carry_their_diff_and_turns_end_with_their_requests() {
        let mut m = CodexMapper::new();
        m.on_notification("turn/started", &json!({"turn": {"id": "t1"}}));
        let item = json!({"type": "fileChange", "id": "f", "status": "inProgress",
                          "changes": [{"path": "a.rs", "kind": {"type": "update"}, "diff": "@@ -1 +1 @@\n-a\n+b\n"},
                                      {"path": "b.rs", "kind": {"type": "add"}, "diff": "+x\n"}]});
        let ev = events(&m.on_notification("item/started", &json!({"item": item})));
        let AgentEvent::ToolCall { title, diff, .. } = &ev[0].1 else { panic!() };
        assert_eq!(title, "a.rs +1 more");
        assert!(diff.as_deref().unwrap().contains("+++ b/a.rs"));
        let ev = events(&m.on_server_request(
            json!(4),
            "item/fileChange/requestApproval",
            &json!({"itemId": "f", "turnId": "t1"}),
        ));
        let AgentEvent::Request { diff, .. } = &ev[0].1 else { panic!() };
        assert!(diff.as_deref().is_some_and(|d| d.contains("+++ b/b.rs")), "the card shows the change: {diff:?}");
        m.note_interrupt();
        let ev = events(&m.on_notification("turn/completed", &json!({"turn": {"id": "t1", "status": "interrupted"}})));
        assert!(
            matches!(&ev[0].1, AgentEvent::Request { state: RequestState::Expired, title, .. } if title == "a.rs +1 more")
        );
        assert!(matches!(&ev[1].1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), .. }));
        let ev = events(&m.on_notification(
            "turn/completed",
            &json!({"turn": {"id": "t2", "status": "failed", "error": {"message": "boom"}}}),
        ));
        assert!(matches!(&ev[0].1, AgentEvent::Error { text, .. } if text == "boom"));
        let ev = events(&m.on_notification("brand/new", &json!({"threadId": "x"})));
        assert!(matches!(&ev[0].1, AgentEvent::Unknown { .. }));
        assert!(m.on_notification("thread/status/changed", &json!({})).is_empty());
    }

    #[test]
    fn history_imports_only_unknown_turns() {
        let turns = vec![
            json!({"id": "t1", "status": "completed", "items": [{"type": "userMessage", "id": "u", "content": [{"type": "text", "text": "a"}]}]}),
            json!({"id": "t2", "status": "completed", "durationMs": 5, "items": [
                {"type": "userMessage", "id": "u2", "content": [{"type": "text", "text": "from tui"}]},
                {"type": "commandExecution", "id": "c", "command": "/bin/zsh -lc 'ls'", "status": "completed", "aggregatedOutput": "x", "exitCode": 0},
                {"type": "agentMessage", "id": "m", "text": "ok", "phase": "final_answer"}]}),
        ];
        let known = std::collections::HashSet::from(["t1".to_string()]);
        let ev = events(&history_events(&turns, &known));
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(kinds, ["turn", "user_message", "tool_call", "agent_message", "turn"]);
        assert!(matches!(&ev[1].1, AgentEvent::UserMessage { source: MessageSource::Terminal, .. }));
        assert!(matches!(&ev[2].1, AgentEvent::ToolCall { title, .. } if title == "ls"));
    }

    #[test]
    fn policies_follow_the_permission_mode() {
        assert_eq!(codex_policy(PermissionMode::Ask), ("on-request", "workspace-write"));
        assert_eq!(codex_policy(PermissionMode::Bypass), ("never", "danger-full-access"));
        assert_eq!(codex_policy(PermissionMode::Plan).1, "read-only");
        assert_eq!(sandbox_policy("read-only", false), json!({"type": "readOnly"}));
        assert_eq!(sandbox_policy("workspace-write", true), json!({"type": "workspaceWrite", "networkAccess": true}));
        assert_eq!(sandbox_policy("danger-full-access", true), json!({"type": "dangerFullAccess"}));
    }

    /// A turn that uses each kind of tool Codex has, asks for more permissions, hits its usage limit and fails.
    const TOOLS: &str = include_str!("../../tests/fixtures/codex/tools.jsonl");

    #[test]
    fn a_recorded_turn_with_every_kind_of_tool_maps_to_events() {
        let mut m = CodexMapper::new();
        m.note_sent();
        let mut out = Vec::new();
        for l in TOOLS.lines() {
            let v: Value = serde_json::from_str(l).unwrap();
            let method = v["method"].as_str().unwrap();
            if let Some(id) = v.get("id") {
                let asked = m.on_server_request(id.clone(), method, &v["params"]);
                out.extend(asked);
                let a = m.answer("p1", &Answer::Approve { always: true }).unwrap();
                assert_eq!(
                    a.result,
                    json!({"permissions": {"network": {"enabled": true}}, "scope": "session"}),
                    "the permissions asked for, for the rest of the session"
                );
                out.extend(a.events);
            } else {
                out.extend(m.on_notification(method, &v["params"]));
            }
        }
        let ev = events(&out);
        let kinds: Vec<&str> = ev.iter().map(|(_, e)| e.kind()).collect();
        assert_eq!(
            kinds,
            [
                "turn",
                "user_message",
                "reasoning",
                "plan",
                "tool_call",
                "tool_call",
                "tool_call",
                "tool_call",
                "tool_call",
                "tool_call",
                "tool_call",
                "request",
                "request",
                "tool_call",
                "compaction",
                "turn_diff",
                "notice",
                "notice",
                "notice",
                "usage",
                "notice",
                "error",
                "error",
                "turn"
            ],
            "{ev:#?}"
        );
        let find = |key: &str| ev.iter().rev().find(|(k, _)| k.as_deref() == Some(key)).map(|(_, e)| e).unwrap();
        assert!(matches!(find("i:u1"), AgentEvent::UserMessage { text, images, source: MessageSource::Pdt, .. }
            if text == "Look around" && images.len() == 1));
        assert!(
            matches!(find("i:r1"), AgentEvent::Reasoning { summary } if summary == "**Plan**\n\nSearch, then read.")
        );
        assert!(matches!(find("plan:t1"), AgentEvent::Plan { markdown }
            if markdown == "Two steps\n\n- [x] Search the web\n- [ ] Read the file"));
        let AgentEvent::ToolCall { tool, title, status, .. } = find("i:w1") else { panic!() };
        assert_eq!((tool.as_str(), title.as_str(), *status), ("WebSearch", "rust similar crate", ToolStatus::Done));
        let AgentEvent::ToolCall { tool, title, input, output, status, .. } = find("i:m1") else { panic!() };
        assert_eq!((tool.as_str(), title.as_str(), *status), ("mcp:pdt/get_ticket", "get_ticket", ToolStatus::Done));
        assert_eq!(input, &json!({"ticket": "PDT-1"}));
        assert!(output.as_deref().is_some_and(|o| o.contains("\"ok\"")), "{output:?}");
        // A failed MCP call shows its error.
        let AgentEvent::ToolCall { output, status, .. } = find("i:m2") else { panic!() };
        assert_eq!((output.as_deref(), *status), (Some("not in a workflow step"), ToolStatus::Failed));
        // A command is shown without the login shell Codex runs it in; its output streams, then arrives whole.
        let AgentEvent::ToolCall { title, input, output, exit_code, status, .. } = find("i:c1") else { panic!() };
        assert_eq!((title.as_str(), input["command"].as_str()), ("cargo test", Some("cargo test")));
        assert_eq!(
            (output.as_deref(), *exit_code, *status),
            (Some("running 3 tests\ntest result: FAILED"), Some(101), ToolStatus::Failed)
        );
        let deltas: Vec<&CodexOut> = out.iter().filter(|o| matches!(o, CodexOut::Delta { .. })).collect();
        assert_eq!(
            deltas,
            [&CodexOut::Delta { item_id: "i:c1".into(), field: DeltaField::Output, text: "running 3 tests\n".into() }]
        );
        let AgentEvent::Request { tool, title, state, can_always, .. } = find("r:p1") else { panic!() };
        assert_eq!(
            (tool.as_str(), title.as_str(), *state, *can_always),
            ("Permissions", "Reach crates.io", RequestState::Answered, true)
        );
        let AgentEvent::ToolCall { tool, title, .. } = find("i:v1") else { panic!() };
        assert_eq!((tool.as_str(), title.as_str()), ("Read", "/tmp/shot.png"));
        assert!(matches!(find("diff:t1"), AgentEvent::TurnDiff { diff } if diff.contains("+b")));
        let notices: Vec<(NoticeLevel, &str, Option<i64>)> = ev
            .iter()
            .filter_map(|(_, e)| match e {
                AgentEvent::Notice { level, text, until } => Some((*level, text.as_str(), *until)),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            [
                (NoticeLevel::Info, "Codex switched this turn to gpt-small", None),
                (NoticeLevel::Warning, "Your config has an unknown key", None),
                (NoticeLevel::Warning, "stream disconnected, retrying", None),
                (
                    NoticeLevel::Warning,
                    "Codex's usage limit is reached. Nothing is sent to it until you send again.",
                    Some(1_792_400_000_000)
                ),
            ]
        );
        let AgentEvent::Usage { rate_limits, .. } = find("usage") else { panic!() };
        let windows: Vec<(&str, f64)> = rate_limits.iter().map(|w| (w.name.as_str(), w.utilization)).collect();
        assert_eq!(windows, [("60_minutes", 0.4), ("five_hour", 1.0)]);
        assert!(matches!(&ev[21].1, AgentEvent::Error { text, retryable: false } if text == "usage limit reached"));
        assert!(
            matches!(find("turn-end:t1"), AgentEvent::Turn { status: Some(TurnStatus::Failed), duration_ms: Some(4200), reason: Some(r), .. }
            if r == "usage limit reached")
        );
        assert_eq!(m.turn(), None);
    }

    #[test]
    fn requests_name_their_command_and_expire_when_the_server_goes() {
        let mut m = CodexMapper::new();
        m.on_notification("turn/started", &json!({"turn": {"id": "t1"}}));
        // A request with no item is keyed by its rpc id; a command's parsed actions name it.
        let p = json!({"command": "/bin/zsh -lc 'ls && pwd'", "commandActions": [{"command": "ls"}, {"command": "pwd"}],
                       "reason": "List files"});
        let ev = events(&m.on_server_request(json!(9), "item/commandExecution/requestApproval", &p));
        let (key, AgentEvent::Request { id, title, description, tool_call_id, can_always, input, .. }) = &ev[0] else {
            panic!()
        };
        assert_eq!(key.as_deref(), Some("r:req-9"));
        assert_eq!((id.as_str(), title.as_str(), description.as_deref()), ("req-9", "ls && pwd", Some("List files")));
        assert_eq!((tool_call_id.as_deref(), *can_always), (Some("req-9"), true));
        assert_eq!(input["command"], "ls && pwd");
        assert_eq!(
            m.answer("req-9", &Answer::Approve { always: true }).unwrap().result,
            json!({"decision": "acceptForSession"})
        );
        // Denied more permissions: none granted, and the note goes to the agent next.
        m.on_server_request(
            json!(10),
            "item/permissions/requestApproval",
            &json!({"itemId": "p", "permissions": {"x": 1}}),
        );
        let a = m.answer("p", &Answer::Deny { message: Some(" ".into()) }).unwrap();
        assert_eq!((a.result, a.follow_up), (json!({"permissions": {}}), None));
        m.on_server_request(
            json!(11),
            "item/permissions/requestApproval",
            &json!({"itemId": "p2", "permissions": {"y": 2}}),
        );
        let a = m.answer("p2", &Answer::Approve { always: false }).unwrap();
        assert_eq!(a.result, json!({"permissions": {"y": 2}, "scope": "turn"}));
        // A declined question answers nothing.
        m.on_server_request(json!(12), "item/tool/requestUserInput", &json!({"itemId": "q", "questions": []}));
        assert_eq!(m.answer("q", &Answer::Deny { message: None }).unwrap().result, json!({"answers": {}}));
        // Resolved after PDT interrupted: the request expired rather than being answered elsewhere.
        m.on_server_request(json!(13), "item/fileChange/requestApproval", &json!({"itemId": "f"}));
        m.note_interrupt();
        let ev = events(&m.on_notification("serverRequest/resolved", &json!({"requestId": 13})));
        assert!(matches!(&ev[0].1, AgentEvent::Request { state: RequestState::Expired, .. }));
        // The server going away expires what is open and ends the turn.
        m.on_server_request(json!(14), "item/fileChange/requestApproval", &json!({"itemId": "g"}));
        let ev = events(&m.on_exit("Codex stopped"));
        assert!(matches!(&ev[0].1, AgentEvent::Request { state: RequestState::Expired, .. }));
        assert!(
            matches!(&ev[1].1, AgentEvent::Turn { status: Some(TurnStatus::Interrupted), reason: Some(r), .. } if r == "Codex stopped")
        );
        assert!(m.on_exit("again").is_empty());
        // A message PDT failed to send is not counted as its own.
        m.note_sent();
        m.note_unsent();
        m.note_unsent();
        let user = json!({"item": {"type": "userMessage", "id": "u", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}});
        let ev = events(&m.on_notification("item/completed", &user));
        assert!(
            matches!(&ev[0].1, AgentEvent::UserMessage { text, source: MessageSource::Terminal, .. } if text == "a\nb")
        );
    }

    #[test]
    fn tool_items_of_other_kinds_keep_their_name_from_when_they_started() {
        let mut m = CodexMapper::new();
        let started = json!({"item": {"type": "dynamicToolCall", "id": "d", "tool": "lookup", "arguments": {"q": 1}}});
        let ev = events(&m.on_notification("item/started", &started));
        assert!(matches!(&ev[0].1, AgentEvent::ToolCall { tool, status: ToolStatus::Running, .. } if tool == "lookup"));
        let sub = json!({"item": {"type": "subAgentActivity", "id": "s"}});
        assert!(
            matches!(&events(&m.on_notification("item/completed", &sub))[0].1, AgentEvent::ToolCall { tool, .. } if tool == "subAgentActivity")
        );
        // Items PDT does not show, and partial messages, are not events.
        for item in [
            json!({"type": "hookPrompt", "id": "h"}),
            json!({"type": "agentMessage", "id": "a", "text": "partial"}),
            json!({"type": "somethingNew", "id": "n"}),
            json!({"type": "agentMessage"}),
        ] {
            assert!(m.on_notification("item/started", &json!({"item": item})).is_empty(), "{item}");
        }
        assert!(m.on_notification("item/agentMessage/delta", &json!({"itemId": "a", "delta": ""})).is_empty());
        let long = "x".repeat(250);
        let p = json!({"command": format!("echo {long}")});
        let ev = events(&m.on_server_request(json!(1), "item/commandExecution/requestApproval", &p));
        let AgentEvent::Request { title, .. } = &ev[0].1 else { panic!() };
        assert_eq!(title.chars().count(), 201, "200 characters and an ellipsis");
        assert!(title.ends_with('…'));
    }
}
