use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `run.failed` and `run.finished` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunEnded {
    pub workflow_id: String,
    pub workflow_name: String,
    pub run_id: String,
    /// `succeeded`, `quiet`, `failed` or `cancelled`.
    pub status: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
    /// The run's error (a failed run's).
    pub error: Option<String>,
    /// The step that failed: its node id, kind and error.
    pub failed_node: Option<String>,
    pub failed_kind: Option<String>,
    pub failed_error: Option<String>,
    /// The node of the last step to end, and its kind.
    pub end_node: Option<String>,
    pub end_kind: Option<String>,
    #[ts(type = "number")]
    pub duration_ms: i64,
    /// The event type that started it (`ticket.stage_changed` …), or `manual`.
    pub trigger_type: String,
    /// The run that called it as a sub-workflow.
    pub parent_run_id: Option<String>,
    /// How far down a chain of runs started by `run.*` events it is (0: not started by one).
    pub chain: u32,
}
