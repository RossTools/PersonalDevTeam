use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `agent.outcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentOutcome {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub attempt_id: Option<String>,
    pub outcome: String,
    pub summary: String,
    /// The persona's name, and the version the step ran.
    pub persona: String,
    pub persona_version: u32,
    /// `agent`, or `user` when the user took the step over.
    pub by: String,
    pub session_id: Option<String>,
    pub workflow_id: Option<String>,
    pub run_id: Option<String>,
}

/// `agent.stalled`, `agent.loop_limit` and `agent.timed_out` (PDT-367): an agent step that is stuck, for "When an
/// agent is stuck" to handle (`agent.nudge`, `agent.restart`, `agent.inbox`, `agent.stop`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentStuck {
    /// `stalled`, `loop_limit` or `timed_out`.
    pub kind: String,
    /// What happened, in a sentence.
    pub why: String,
    /// The stuck step (the one `agent.*` nodes act on), its node, run and workflow.
    pub step_id: String,
    pub node_id: String,
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub persona: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
    pub attempt_id: Option<String>,
    /// The step's prompt.
    pub goal: String,
    /// The end of what the agent last said (empty when PDT can't tell).
    pub last_message: String,
    /// Automatic and the user's nudges the step had.
    pub nudges: u32,
    /// Fresh sessions the step had (`agent.restart`).
    pub restarts: u32,
    /// Turns that ended without an outcome.
    pub turns: u32,
    /// Earlier passes of the node in the run (`agent.loop_limit`).
    pub passes: u32,
}
