use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The agent session a `session.*` event is about (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionEvent {
    pub session_id: String,
    pub title: String,
    pub project_id: String,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
    /// `claude`, `codex`, `devin` …
    pub agent: Option<String>,
    /// `terminal` or `conversation`.
    pub mode: String,
    /// The persona a workflow's agent step runs in it (`Coder` …).
    pub persona: Option<String>,
    /// A workflow's agent step runs it (else it is the user's own session).
    pub workflow_step: bool,
    /// That step's run and workflow.
    pub run_id: Option<String>,
    pub workflow_id: Option<String>,
    /// The end of what the agent last said (at most a few hundred characters; empty when PDT can't tell).
    pub last_message: String,
}

/// `session.exited` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionExited {
    #[serde(flatten)]
    #[ts(flatten)]
    pub session: SessionEvent,
    /// The process's exit code, when it has one.
    pub exit_code: Option<i32>,
    /// PDT stopped it (the user's Stop or close, a restart): the agent didn't end or crash on its own.
    pub stopped_by_pdt: bool,
}
