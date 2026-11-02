use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `todo.due` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TodoDue {
    pub todo_id: String,
    pub title: String,
    pub body: String,
    /// The day (`YYYY-MM-DD`) and time (`HH:MM`), local; and when that is (ms since the epoch).
    pub due_on: String,
    pub due_time: Option<String>,
    #[ts(type = "number")]
    pub due_at: i64,
    /// It was due before today (PDT was off when it came due).
    pub overdue: bool,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
}

/// `schedule.fired`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduleFired {
    pub workflow_id: String,
    pub node_id: String,
    /// The slot (ms since the epoch).
    #[ts(type = "number")]
    pub slot: i64,
    /// The slot in the schedule's zone, RFC 3339.
    pub slot_iso: String,
    /// The slot's date in the schedule's zone (`YYYY-MM-DD`), for run keys.
    pub date: String,
}
