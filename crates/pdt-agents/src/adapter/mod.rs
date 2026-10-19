//! Structured agent adapters (structured agents spec §3.1): each owns an agent's process and protocol framing and turns
//! it into normalized events. They know nothing about HTTP or the store.

pub mod claude;
pub mod codex;

use pdt_core::conversation::{AgentCapabilities, AgentEvent, Answer, Delivery, DeltaField, PermissionMode};
use pdt_core::session::AgentKind;
use std::path::PathBuf;

/// What a live conversation reports.
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterOutput {
    /// A new or changed event, with the protocol frame it came from.
    Event { item_id: Option<String>, turn_id: Option<String>, event: AgentEvent, raw: Option<String> },
    /// Streaming text for an item.
    Delta { item_id: String, field: DeltaField, text: String },
    /// What the agent says about itself (each turn for Claude).
    Info(AgentInfo),
    /// The agent's models and commands (once per process).
    Capabilities(AgentCapabilities),
    /// The process ended. `expected`: PDT closed it.
    Exited { code: Option<i32>, stderr_tail: String, expected: bool },
    /// The agent's own id for this conversation (Codex: the thread id), to keep as `external_session_id`.
    ExternalId(String),
    /// The agent's history (Codex: `thread/turns/list`, oldest first), for importing turns PDT did not see.
    History(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentInfo {
    pub model: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub version: Option<String>,
    /// Claude: `none` when it bills the plan login.
    pub api_key_source: Option<String>,
}

pub type EventSink = tokio::sync::mpsc::UnboundedSender<AdapterOutput>;

/// How to start (or resume) an agent conversation for a session.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenSpec {
    pub session_id: String,
    /// The agent's own conversation id (Codex: the thread id), when it has one.
    pub external_id: Option<String>,
    /// Continue the saved conversation (Claude: `--resume <session id>`), else start it (`--session-id`).
    pub resume: bool,
    pub cwd: PathBuf,
    /// The agent program (name or path), resolved on `search_path`.
    pub program: String,
    pub search_path: Option<String>,
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

/// A live setting change.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionChange {
    /// Some(None): back to the agent's default model.
    pub model: Option<Option<String>>,
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{0} was not found")]
    ProgramNotFound(String),
    #[error("could not start the agent: {0}")]
    Spawn(String),
    #[error("the agent is not running")]
    NotRunning,
    #[error("this request is no longer open")]
    RequestClosed,
    #[error("that answer does not fit this request")]
    WrongAnswer,
    #[error("the agent refused: {0}")]
    Refused(String),
    #[error("the agent did not answer in time")]
    Timeout,
}

#[async_trait::async_trait]
pub trait AgentAdapter: Send + Sync {
    fn agent(&self) -> AgentKind;
    /// Starts or resumes the agent's conversation for a session; outputs arrive on `sink`.
    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>, AdapterError>;
}

#[async_trait::async_trait]
pub trait LiveConversation: Send + Sync {
    async fn send(&self, text: &str, delivery: Delivery) -> Result<(), AdapterError>;
    async fn answer(&self, request_id: &str, answer: &Answer) -> Result<(), AdapterError>;
    async fn interrupt(&self) -> Result<(), AdapterError>;
    async fn set(&self, change: &SessionChange) -> Result<(), AdapterError>;
    /// Stops the process and keeps the conversation (parking, Stop, shutdown).
    async fn close(&self);
    /// The process is still running.
    fn is_alive(&self) -> bool;
}
