//! The workflow API's records (Phase 7a spec §9): workflows, versions, runs, steps, logged events, connections.

use super::doc::WorkflowDoc;
use super::validate::Issue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.as_str() == s)
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Scope {
    /// One project's events.
    Project,
    /// Every project's, narrowed by its triggers' `projects`.
    Host,
    /// Not about a code project (manual and scheduled triggers).
    Personal,
}
str_enum!(Scope { Project => "project", Host => "host", Personal => "personal" });

/// Where a workflow's document lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Source {
    /// pdtd's database.
    Host,
    /// `.pdt/workflows/<name>.json` in the project's repo (read-only in PDT).
    Repo,
}
str_enum!(Source { Host => "host", Repo => "repo" });

/// Who wrote a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Author {
    User,
    /// A built-in template PDT installed or updated.
    Builtin,
    /// Read from the repo's file.
    Repo,
    Agent,
}
str_enum!(Author { User => "user", Builtin => "builtin", Repo => "repo", Agent => "agent" });

/// Where a version stands (Phase 7d spec §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VersionStatus {
    /// Saved: the current version or an earlier one.
    Saved,
    /// Written by an agent and waiting for the user to accept or discard it.
    Proposed,
    /// A proposal the user (or a newer proposal) set aside.
    Discarded,
}
str_enum!(VersionStatus { Saved => "saved", Proposed => "proposed", Discarded => "discarded" });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: Scope,
    pub project_id: Option<String>,
    pub enabled: bool,
    /// The built-in template it was installed from.
    pub builtin: Option<String>,
    pub source: Source,
    /// The repo file, relative to the repo (`.pdt/workflows/x.json`).
    pub repo_file: Option<String>,
    /// The current version.
    pub version: u32,
    /// A version an agent proposed, waiting for review.
    #[serde(default)]
    pub proposed: Option<u32>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// A workflow with its current document and what validation says about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub workflow: Workflow,
    pub doc: WorkflowDoc,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowVersion {
    pub workflow_id: String,
    pub version: u32,
    pub author: Author,
    pub status: VersionStatus,
    pub note: String,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Only when one version is asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<WorkflowDoc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "WorkflowRunStatus")]
pub enum RunStatus {
    Running,
    /// Every step left is waiting (a time, an event, an answer, a delivery).
    Waiting,
    Succeeded,
    /// Ended with nothing to report: nobody was told.
    Quiet,
    Failed,
    Cancelled,
}
str_enum!(RunStatus {
    Running => "running", Waiting => "waiting", Succeeded => "succeeded", Quiet => "quiet", Failed => "failed",
    Cancelled => "cancelled",
});

impl RunStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, RunStatus::Succeeded | RunStatus::Quiet | RunStatus::Failed | RunStatus::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "WorkflowRun")]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    /// The workflow version it runs.
    pub version: u32,
    pub status: RunStatus,
    pub run_key: String,
    pub trigger_event_id: Option<String>,
    /// The trigger node it started from.
    pub trigger_node: String,
    /// The event payload (or the manual input) it started with.
    #[ts(type = "unknown")]
    pub trigger: Value,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    pub parent_run_id: Option<String>,
    pub parent_step_id: Option<String>,
    pub error: Option<String>,
    #[ts(type = "unknown")]
    pub output: Value,
    #[ts(type = "number | null")]
    pub wake_at: Option<i64>,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "WorkflowStepStatus")]
pub enum StepStatus {
    Pending,
    Running,
    Waiting,
    Succeeded,
    Failed,
    /// Cancelled with its run.
    Skipped,
}
str_enum!(StepStatus {
    Pending => "pending", Running => "running", Waiting => "waiting", Succeeded => "succeeded", Failed => "failed",
    Skipped => "skipped",
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "WorkflowStep")]
pub struct Step {
    pub id: String,
    pub run_id: String,
    /// Order in the run.
    #[ts(type = "number")]
    pub seq: i64,
    pub node_id: String,
    pub kind: String,
    pub status: StepStatus,
    /// The node's config with its references filled in.
    #[ts(type = "unknown")]
    pub input: Value,
    #[ts(type = "unknown")]
    pub output: Value,
    /// The port it left by.
    pub port: Option<String>,
    pub attempts: u32,
    pub error: Option<String>,
    #[ts(type = "number | null")]
    pub wake_at: Option<i64>,
    /// What a waiting step waits for (`{event: …}`, `{answer: …}`, `{delivery: …}`, `{run: …}`) and the port it takes
    /// on timeout.
    #[ts(type = "unknown")]
    pub wait: Value,
    #[ts(type = "number | null")]
    pub started_at: Option<i64>,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "WorkflowRunDetail")]
pub struct RunDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub run: Run,
    pub steps: Vec<Step>,
}

/// One entry of the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogEvent {
    #[ts(type = "number")]
    pub seq: i64,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub key: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    pub session_id: Option<String>,
    /// What emitted it: `pr_poller`, `tickets`, …
    pub source: String,
    #[ts(type = "unknown")]
    pub payload: Value,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub processed_at: Option<i64>,
}

/// An event to add to the log.
#[derive(Debug, Clone, PartialEq)]
pub struct NewLogEvent {
    pub kind: String,
    pub key: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    pub session_id: Option<String>,
    pub source: String,
    pub payload: Value,
}

impl NewLogEvent {
    pub fn new(kind: &str, key: impl Into<String>, source: &str, payload: impl Serialize) -> Self {
        let payload = serde_json::to_value(payload).unwrap_or(Value::Null);
        let s = |k: &str| payload.get(k).and_then(|v| v.as_str()).map(String::from);
        Self {
            kind: kind.into(),
            key: key.into(),
            project_id: s("projectId"),
            ticket_id: s("ticketId"),
            attempt_id: s("attemptId"),
            session_id: s("sessionId"),
            source: source.into(),
            payload,
        }
    }
}

/// A named external service (Phase 7a spec §7). Its secret is in the OS keychain, never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Connection {
    pub id: String,
    pub name: String,
    /// What it is (`discord_webhook`, `http`, `radarr`, …).
    pub kind: String,
    pub url: String,
    #[ts(type = "Record<string, unknown>")]
    pub config: Value,
    pub has_secret: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_event_takes_its_ids_from_the_payload() {
        let e = NewLogEvent::new("x.y", "k", "test", serde_json::json!({"ticketId": "t", "attemptId": "a", "n": 1}));
        assert_eq!((e.ticket_id.as_deref(), e.attempt_id.as_deref(), e.project_id), (Some("t"), Some("a"), None));
        assert_eq!(RunStatus::parse("quiet"), Some(RunStatus::Quiet));
        assert!(RunStatus::Quiet.is_finished() && !RunStatus::Waiting.is_finished());
    }
}
