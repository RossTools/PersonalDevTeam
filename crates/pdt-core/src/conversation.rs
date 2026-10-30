//! Conversations (structured agents spec §4): the normalized events every agent adapter produces, the requests the
//! user answers, the WebSocket frames, and activity derived from events. Pure.

pub mod acp;
pub mod claude;
pub mod codex;
pub mod composer;
pub mod cursor;
pub mod devin;
pub mod draft;

use crate::activity::Activity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;

/// The most a tool call's output keeps (1 MB); longer output is cut with a note.
pub const MAX_TOOL_OUTPUT: usize = 1024 * 1024;
/// The most of the raw protocol frames kept with one event (64 KB).
pub const MAX_RAW: usize = 64 * 1024;
/// The longest message the composer sends (64 KB).
pub const MAX_MESSAGE: usize = 64 * 1024;

/// How an agent session is shown and driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionMode {
    /// The agent's own TUI in a terminal.
    #[default]
    Terminal,
    /// PDT drives the agent's machine protocol and draws the conversation.
    Conversation,
}

impl SessionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionMode::Terminal => "terminal",
            SessionMode::Conversation => "conversation",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [SessionMode::Terminal, SessionMode::Conversation].into_iter().find(|m| m.as_str() == s)
    }
}

/// PDT's permission setting for a session (spec §6.1), mapped to each agent's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PermissionMode {
    /// The agent asks before anything that needs permission.
    #[default]
    Ask,
    /// File edits run without asking; commands still ask.
    AcceptEdits,
    /// The agent plans and asks before changing anything.
    Plan,
    /// Nothing asks (dangerous).
    Bypass,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 4] =
        [PermissionMode::Ask, PermissionMode::AcceptEdits, PermissionMode::Plan, PermissionMode::Bypass];

    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Ask => "ask",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Plan => "plan",
            PermissionMode::Bypass => "bypass",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }
    /// Claude Code's `--permission-mode` / `set_permission_mode` value.
    pub fn claude(self) -> &'static str {
        match self {
            PermissionMode::Ask => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::Bypass => "bypassPermissions",
        }
    }
    /// How the UI and notices name it.
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask",
            PermissionMode::AcceptEdits => "Accept edits",
            PermissionMode::Plan => "Plan",
            PermissionMode::Bypass => "Bypass",
        }
    }
    /// The PDT mode for Claude's value; None for modes PDT does not offer (e.g. `auto`, `dontAsk`).
    pub fn from_claude(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.claude() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MessageSource {
    /// Sent from PDT's composer.
    Pdt,
    /// Typed in the agent's own TUI (imported history).
    Terminal,
    /// Sent while a turn was running and taken into it.
    Steer,
    /// Sent by PDT itself, not typed by the user: news about the attempt's pull request (spec §14).
    Notice,
}

/// Which of PDT's own messages to a PM conversation a user message is (PDT-211): its view shows one collapsed line.
/// The agent still gets the whole text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PdtMessage {
    /// The daily update ("Since your briefing…").
    Update,
    /// A request for a journal checkpoint (after a compaction, or a quiet day).
    Checkpoint,
    /// The request for the journal entry of a conversation the user closed.
    Journal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MessagePhase {
    /// Said on the way (before or between tool calls).
    Commentary,
    /// The turn's answer.
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
    /// The user (or the permission mode) refused it.
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RequestKind {
    Approval,
    Question,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RequestState {
    Open,
    Answered,
    /// The turn ended or the agent stopped before it was answered.
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TurnPhase {
    Started,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoticeLevel {
    Info,
    Warning,
}

/// One rate-limit window as the agent reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RateLimitWindow {
    /// The agent's name for it (`five_hour`, `seven_day`).
    pub name: String,
    /// 0–1.
    pub utilization: f64,
    /// When the window resets (ms).
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
}

/// Until when a hit limit stops the agent (ms; `i64::MAX` when the agent gave no reset time), or None when no window
/// is at its limit. Nothing is sent automatically before then (spec §14).
pub fn limited_until(windows: &[RateLimitWindow], now: i64) -> Option<i64> {
    windows.iter().filter(|w| w.utilization >= 1.0).map(|w| w.resets_at.unwrap_or(i64::MAX)).filter(|&t| t > now).max()
}

/// The user's answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Answer {
    /// Let it run; `always` also allows the same thing for the rest of the session (never written to settings).
    Approve {
        #[serde(default)]
        always: bool,
    },
    /// Refuse, optionally telling the agent why.
    Deny {
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        message: Option<String>,
    },
    /// Answers to a question request: question text → chosen labels (or the user's own text).
    Question { answers: BTreeMap<String, Vec<String>> },
    /// A plan request: approve it (optionally going on in `mode`, e.g. Accept edits), or keep planning with feedback.
    Plan {
        approve: bool,
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        feedback: Option<String>,
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        mode: Option<PermissionMode>,
    },
}

/// A normalized conversation event (spec §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AgentEvent {
    UserMessage {
        text: String,
        #[serde(default)]
        images: Vec<String>,
        source: MessageSource,
        /// One of PDT's own messages to a PM conversation (with `source` `notice`). Older pdtd versions ignore it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pdt: Option<PdtMessage>,
    },
    AgentMessage {
        id: String,
        markdown: String,
        phase: MessagePhase,
    },
    Reasoning {
        summary: String,
    },
    ToolCall {
        id: String,
        tool: String,
        /// One line: the command, the file, the pattern; empty when the tool has no obvious subject.
        title: String,
        #[ts(type = "unknown")]
        input: Value,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        output: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        exit_code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        diff: Option<String>,
        /// The sub-agent call this one ran under.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        parent_id: Option<String>,
    },
    Request {
        id: String,
        request_kind: RequestKind,
        tool: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        description: Option<String>,
        #[ts(type = "unknown")]
        input: Value,
        /// The tool call this request is for.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        tool_call_id: Option<String>,
        /// Whether "Always allow in this session" is offered.
        can_always: bool,
        /// The change a file-edit request would make (unified diff).
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        diff: Option<String>,
        state: RequestState,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        answer: Option<Answer>,
    },
    Plan {
        markdown: String,
    },
    Turn {
        phase: TurnPhase,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        status: Option<TurnStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        duration_ms: Option<i64>,
        /// Why an interrupted or failed turn ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reason: Option<String>,
    },
    Usage {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        context_used: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        context_window: Option<i64>,
        #[serde(default)]
        rate_limits: Vec<RateLimitWindow>,
        /// The agent's running list-price cost estimate, in USD.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        cost_usd: Option<f64>,
    },
    TurnDiff {
        diff: String,
    },
    Compaction {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        before: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        after: Option<i64>,
    },
    /// The agent started over on a new conversation (Claude's `/clear`); shown as a divider. `trigger`: the agent's
    /// reason (`clear`, or `resume` when the TUI switched to another saved conversation).
    ConversationReset {
        trigger: String,
    },
    Notice {
        level: NoticeLevel,
        text: String,
        /// When the condition ends (ms), e.g. a rate limit's reset.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional, type = "number")]
        until: Option<i64>,
    },
    Error {
        text: String,
        retryable: bool,
    },
    Unknown {
        #[ts(type = "unknown")]
        raw: Value,
    },
}

impl AgentEvent {
    /// The `session_events.kind` column.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentEvent::UserMessage { .. } => "user_message",
            AgentEvent::AgentMessage { .. } => "agent_message",
            AgentEvent::Reasoning { .. } => "reasoning",
            AgentEvent::ToolCall { .. } => "tool_call",
            AgentEvent::Request { .. } => "request",
            AgentEvent::Plan { .. } => "plan",
            AgentEvent::Turn { .. } => "turn",
            AgentEvent::Usage { .. } => "usage",
            AgentEvent::TurnDiff { .. } => "turn_diff",
            AgentEvent::Compaction { .. } => "compaction",
            AgentEvent::ConversationReset { .. } => "conversation_reset",
            AgentEvent::Notice { .. } => "notice",
            AgentEvent::Error { .. } => "error",
            AgentEvent::Unknown { .. } => "unknown",
        }
    }
}

/// A stored event. An item that changes (a tool call finishing, a request answered) keeps its row and `pos` but gets
/// a new `seq`, so "events after seq N" includes every change while `pos` keeps conversation order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConversationEvent {
    #[ts(type = "number")]
    pub seq: i64,
    #[ts(type = "number")]
    pub pos: i64,
    #[ts(type = "number")]
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<String>,
    pub event: AgentEvent,
}

/// Which text field a streaming delta extends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DeltaField {
    /// An agent message's markdown.
    Markdown,
    /// A tool call's output.
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Delivery {
    /// At the agent's next opportunity: injected at a tool boundary, or the next turn.
    #[default]
    Next,
    /// Interrupt what it is doing and take this now.
    Now,
}

/// A message the agent has not taken yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PendingMessage {
    pub id: String,
    pub text: String,
    pub delivery: Delivery,
    /// Attachment ids of the images it carries.
    #[serde(default)]
    pub images: Vec<String>,
    /// PDT sent it on its own (a pull-request notice), not the user.
    #[serde(default)]
    pub notice: bool,
    /// Which of PDT's own messages to a PM conversation it is (PDT-211).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pdt: Option<PdtMessage>,
}

/// Work the agent left running in the background (a shell command, a sub-agent, a monitor). It reports back on its
/// own, so the conversation keeps its process while any run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackgroundTask {
    pub id: String,
    pub description: String,
    /// The agent's kind of task, e.g. Claude's `local_bash`, `local_agent`.
    pub kind: String,
}

/// What a conversation is doing, pushed to every viewer on change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConversationState {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub activity: Option<Activity>,
    /// The agent process is running (false: parked, it starts again on the next message).
    pub live: bool,
    pub turn_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub pending: Vec<PendingMessage>,
    /// The agent stopped unexpectedly: why, with the end of its error output.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stopped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_version: Option<String>,
    /// Codex: the agent's own TUI is open on this conversation (live, beside it).
    pub terminal_open: bool,
    /// Who pays: "Claude plan", "API key", "ChatGPT plan" (spec §14), once the agent has said.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub billing: Option<String>,
    /// Background tasks still running.
    #[serde(default)]
    pub background_tasks: Vec<BackgroundTask>,
}

/// Server → client frames on `WS /api/sessions/{id}/events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ConversationServerMessage {
    /// A new or changed event row.
    Event {
        event: ConversationEvent,
    },
    /// Streaming text for an item not stored yet (or still changing). A client that connects mid-item gets the text
    /// so far as one delta.
    Delta {
        item_id: String,
        field: DeltaField,
        text: String,
    },
    State {
        state: ConversationState,
    },
    /// Sent once after the stored events the client asked for; live frames follow.
    Ready {},
}

/// `POST /api/sessions/{id}/messages`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SendMessage {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub delivery: Option<Delivery>,
    /// Ids from `POST /api/sessions/{id}/attachments`.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub images: Option<Vec<String>>,
}

/// `POST /api/sessions/{id}/requests/{requestId}`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnswerRequest {
    pub answer: Answer,
}

/// One model the agent offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentModel {
    /// What to pass to the agent (`opus`, `default`).
    pub value: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
}

/// One slash command the agent offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentCommandInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub argument_hint: Option<String>,
}

/// `GET /api/agents/{agent}/capabilities`: what the composer offers. Empty lists until a conversation has started
/// once in this pdtd (the agent reports them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentCapabilities {
    pub conversation: bool,
    pub permission_modes: Vec<PermissionMode>,
    pub models: Vec<AgentModel>,
    pub commands: Vec<AgentCommandInfo>,
    /// Commands that work only in the agent's own TUI (Claude's `terminal_slash_commands`).
    #[serde(default)]
    pub terminal_commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
}

/// The facts activity is derived from (spec §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActivityFacts {
    pub turn_running: bool,
    pub open_requests: usize,
    /// The last turn failed or the agent stopped with an error, and nothing was sent since.
    pub errored: bool,
    /// The conversation has any events.
    pub has_history: bool,
}

/// `needs_you` while a request is open (or after an error), `working` during a turn, else `idle`; `unknown` only for a
/// conversation with nothing in it that is not working.
pub fn conversation_activity(f: ActivityFacts) -> Activity {
    if f.open_requests > 0 || f.errored {
        Activity::NeedsYou
    } else if f.turn_running {
        Activity::Working
    } else if f.has_history {
        Activity::Idle
    } else {
        Activity::Unknown
    }
}

/// Cuts `s` to at most `max` bytes on a character boundary, adding a note when it was cut.
pub fn cap_text(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… (cut: {} more bytes)", &s[..cut], s.len() - cut)
}

/// A model name the agent is given: 1 to 100 characters, no spaces.
pub fn valid_model(m: &str) -> bool {
    !m.is_empty() && m.len() <= 100 && !m.contains(char::is_whitespace)
}

/// A user message as sent: never longer than `MAX_MESSAGE`, and blank only when it carries images.
pub fn valid_message(text: &str, images: usize) -> bool {
    (images > 0 || !text.trim().is_empty()) && text.len() <= MAX_MESSAGE && images <= composer::MAX_IMAGES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hit_limit_holds_until_it_resets() {
        let w = |u: f64, r: Option<i64>| RateLimitWindow { name: "five_hour".into(), utilization: u, resets_at: r };
        assert_eq!(limited_until(&[w(0.9, Some(500))], 100), None);
        assert_eq!(limited_until(&[w(1.0, Some(500)), w(1.0, Some(900)), w(0.2, Some(2000))], 100), Some(900));
        assert_eq!(limited_until(&[w(1.0, Some(50))], 100), None, "already reset");
        assert_eq!(limited_until(&[w(1.0, None)], 100), Some(i64::MAX));
        assert_eq!(limited_until(&[], 100), None);
    }

    #[test]
    fn permission_modes_map_to_claude() {
        assert_eq!(PermissionMode::Ask.claude(), "default");
        assert_eq!(PermissionMode::AcceptEdits.claude(), "acceptEdits");
        assert_eq!(PermissionMode::Plan.claude(), "plan");
        assert_eq!(PermissionMode::Bypass.claude(), "bypassPermissions");
        for m in PermissionMode::ALL {
            assert_eq!(PermissionMode::from_claude(m.claude()), Some(m));
            assert_eq!(PermissionMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(PermissionMode::from_claude("auto"), None);
        assert_eq!(SessionMode::parse("conversation"), Some(SessionMode::Conversation));
        assert_eq!(SessionMode::parse("x"), None);
    }

    #[test]
    fn activity_follows_requests_turns_and_errors() {
        let f = ActivityFacts::default();
        assert_eq!(conversation_activity(f), Activity::Unknown);
        assert_eq!(conversation_activity(ActivityFacts { has_history: true, ..f }), Activity::Idle);
        assert_eq!(conversation_activity(ActivityFacts { turn_running: true, ..f }), Activity::Working);
        let open = ActivityFacts { turn_running: true, open_requests: 1, has_history: true, ..f };
        assert_eq!(conversation_activity(open), Activity::NeedsYou);
        assert_eq!(conversation_activity(ActivityFacts { errored: true, has_history: true, ..f }), Activity::NeedsYou);
    }

    #[test]
    fn events_serialize_with_their_kind() {
        let e = AgentEvent::Turn {
            phase: TurnPhase::Ended,
            status: Some(TurnStatus::Completed),
            duration_ms: Some(5),
            reason: None,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "turn");
        assert_eq!(v["durationMs"], 5);
        assert_eq!(e.kind(), "turn");
        let back: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
        let a: Answer = serde_json::from_str(r#"{"type":"approve"}"#).unwrap();
        assert_eq!(a, Answer::Approve { always: false });
        let a: Answer = serde_json::from_str(r#"{"type":"plan","approve":false,"feedback":"more"}"#).unwrap();
        assert_eq!(a, Answer::Plan { approve: false, feedback: Some("more".into()), mode: None });
    }

    #[test]
    fn text_is_capped_on_a_char_boundary() {
        assert_eq!(cap_text("abc", 5), "abc");
        let s = cap_text("ééé", 3);
        assert!(s.starts_with('é') && s.contains("cut: 4 more bytes"), "{s}");
        assert!(valid_message("hi", 0) && !valid_message("  \n", 0) && !valid_message(&"x".repeat(MAX_MESSAGE + 1), 0));
        assert!(valid_message("", 1) && !valid_message("x", composer::MAX_IMAGES + 1));
    }
}
