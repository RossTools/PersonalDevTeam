//! The event log's types and payloads (Phase 7a spec §2). Each type is a trigger kind in the catalog, with its payload
//! as the trigger's output schema. CI types are provider-neutral (`provider`).

use crate::pr::{CiState, Notice};
use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CI_RUN_FAILED: &str = "ci.run_failed";
pub const CI_RUN_PASSED: &str = "ci.run_passed";
pub const PR_CONFLICT: &str = "pr.conflict";
pub const PR_QUEUE_LEFT: &str = "pr.queue_left";
pub const PR_MERGED: &str = "pr.merged";
pub const PR_BEHIND: &str = "pr.behind";
pub const ATTEMPT_STARTED: &str = "attempt.started";
pub const ATTEMPT_MERGED: &str = "attempt.merged";
pub const ATTEMPT_ABANDONED: &str = "attempt.abandoned";
pub const TICKET_STAGE_CHANGED: &str = "ticket.stage_changed";
pub const TICKET_COMMENTED: &str = "ticket.commented";
pub const TICKET_OVERLAP_FOUND: &str = "ticket.overlap_found";
pub const AGENT_OUTCOME: &str = "agent.outcome";
pub const TICKET_REFINE_REQUESTED: &str = "ticket.refine_requested";

/// The CI provider of GitHub Actions runs.
pub const GITHUB_ACTIONS: &str = "github_actions";

/// One event type: its id, what it means and its payload's schema.
pub struct EventType {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Schema,
}

fn schema_of<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

/// Every event type, in catalog order. Adding one: a constant, an entry here, and a sensor or command that emits it.
pub const EVENT_TYPES: &[EventType] = &[
    EventType {
        id: CI_RUN_FAILED,
        title: "CI failed",
        description: "CI failed on the head commit of an attempt's pull request (once per set of failed runs).",
        schema: schema_of::<CiEvent>,
    },
    EventType {
        id: CI_RUN_PASSED,
        title: "CI passed",
        description: "Every CI run on the head commit of an attempt's pull request passed.",
        schema: schema_of::<CiEvent>,
    },
    EventType {
        id: PR_CONFLICT,
        title: "Pull request conflicts",
        description: "An attempt's pull request conflicts with its base branch (once per head commit).",
        schema: schema_of::<PrEvent>,
    },
    EventType {
        id: PR_QUEUE_LEFT,
        title: "Dropped from the merge queue",
        description: "The merge queue removed an attempt's pull request without merging it.",
        schema: schema_of::<PrEvent>,
    },
    EventType {
        id: PR_MERGED,
        title: "Pull request merged",
        description: "GitHub reports an attempt's pull request merged.",
        schema: schema_of::<PrEvent>,
    },
    EventType {
        id: PR_BEHIND,
        title: "Base moved while a pull request waits",
        description: "An open pull request with auto-merge on, not in a merge queue, is behind its base branch.",
        schema: schema_of::<PrEvent>,
    },
    EventType {
        id: ATTEMPT_STARTED,
        title: "Ticket started",
        description: "Start made a new attempt (branch and worktree) for a ticket.",
        schema: schema_of::<AttemptEvent>,
    },
    EventType {
        id: ATTEMPT_MERGED,
        title: "Attempt merged",
        description: "An attempt ended as merged: detected by git or GitHub, merged in PDT, or the ticket was closed.",
        schema: schema_of::<AttemptEvent>,
    },
    EventType {
        id: ATTEMPT_ABANDONED,
        title: "Attempt abandoned",
        description: "An attempt was abandoned (by the user, or because its ticket was closed unmerged).",
        schema: schema_of::<AttemptEvent>,
    },
    EventType {
        id: TICKET_STAGE_CHANGED,
        title: "Ticket moved",
        description: "A ticket moved to another stage.",
        schema: schema_of::<TicketStageChanged>,
    },
    EventType {
        id: TICKET_COMMENTED,
        title: "Ticket commented",
        description: "Someone commented on a ticket: the user, an agent or a workflow (its actor says which).",
        schema: schema_of::<TicketCommented>,
    },
    EventType {
        id: TICKET_OVERLAP_FOUND,
        title: "Overlap found",
        description: "Two tickets in flight change the same files.",
        schema: schema_of::<TicketOverlapFound>,
    },
    EventType {
        id: AGENT_OUTCOME,
        title: "Agent outcome",
        description: "An agent step on a ticket ended with an outcome (submitted by the agent, or taken over by the user).",
        schema: schema_of::<AgentOutcome>,
    },
    EventType {
        id: TICKET_REFINE_REQUESTED,
        title: "Refine requested",
        description: "Someone asked for a Draft or Ready ticket to be refined (Refine on the ticket).",
        schema: schema_of::<TicketRefineRequested>,
    },
];

pub fn event_type(id: &str) -> Option<&'static EventType> {
    EVENT_TYPES.iter().find(|t| t.id == id)
}

/// The attempt an event is about.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptRef {
    pub project_id: String,
    pub ticket_id: String,
    /// `PDT-42`.
    pub ticket_key: String,
    pub attempt_id: String,
    pub branch: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiRunRef {
    #[ts(type = "number")]
    pub id: u64,
    pub attempt: u32,
    pub workflow: String,
    pub state: CiState,
    pub url: String,
    pub failed_jobs: Vec<String>,
}

/// `ci.run_failed`, `ci.run_passed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    /// `github_actions` (Buildkite and others later).
    pub provider: String,
    pub pr: Option<u32>,
    pub head_sha: String,
    /// The run to look at.
    pub url: String,
    pub runs: Vec<CiRunRef>,
    /// The failed jobs, all runs together.
    pub jobs: Vec<String>,
    /// The message PDT tells the agent (failures only).
    pub notice: Option<Notice>,
}

/// `pr.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    pub head_sha: String,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    pub notice: Notice,
}

/// How an attempt came to be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeVia {
    /// Merge detection found it (git or GitHub): the usual way, after a PR merges.
    Detected,
    /// The user merged it locally in PDT (Merge…).
    MergedHere,
    /// The user closed the ticket (Done says the work landed).
    Closed,
}

/// `attempt.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    /// `user` or `system`.
    pub actor: String,
    /// `attempt.merged` only.
    pub via: Option<MergeVia>,
    pub pr: Option<u32>,
    /// Merge… asked to finish the ticket once this merges (PDT-75): the finish gate's answer ahead of time.
    pub finish_on_merge: bool,
    pub finish_delete_branch: bool,
    /// The agent session Start opened (`attempt.started`).
    pub session_id: Option<String>,
    /// The message PDT tells the agent (`attempt.merged`).
    pub notice: Option<Notice>,
}

/// `ticket.stage_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketStageChanged {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub from: String,
    pub to: String,
    pub actor: String,
}

/// `ticket.commented`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketCommented {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub body: String,
    /// `user` (in PDT), `agent` (an agent through the API) or `system` (a workflow).
    pub actor: String,
    /// The ticket's active attempt, when it has one.
    pub attempt_id: Option<String>,
}

/// `ticket.overlap_found`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketOverlapFound {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub other_ticket_id: String,
    pub other_key: String,
    pub files: Vec<String>,
    /// `overlap` or `conflict`.
    pub level: String,
}

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

/// `ticket.refine_requested`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketRefineRequested {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub title: String,
    /// `draft` or `ready`.
    pub stage: String,
    /// `user` or `agent`.
    pub actor: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_unique_and_has_a_schema() {
        let mut ids = std::collections::HashSet::new();
        for t in EVENT_TYPES {
            assert!(ids.insert(t.id), "{} twice", t.id);
            assert!(t.id.contains('.'));
            assert!((t.schema)().as_object().is_some_and(|o| o.contains_key("properties")), "{}", t.id);
        }
        assert!(event_type(PR_CONFLICT).is_some() && event_type("nope").is_none());
    }

    #[test]
    fn payloads_are_flat_camel_case() {
        let e = TicketCommented {
            project_id: "p".into(),
            ticket_id: "t".into(),
            ticket_key: "PDT-1".into(),
            body: "b".into(),
            actor: "user".into(),
            attempt_id: None,
        };
        assert_eq!(serde_json::to_value(&e).unwrap()["ticketKey"], "PDT-1");
        let a = AttemptEvent {
            attempt: AttemptRef { attempt_id: "a".into(), ..Default::default() },
            actor: "system".into(),
            via: Some(MergeVia::MergedHere),
            pr: None,
            finish_on_merge: true,
            finish_delete_branch: false,
            session_id: None,
            notice: None,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!((v["attemptId"].as_str(), v["via"].as_str()), (Some("a"), Some("merged_here")));
        assert_eq!(serde_json::from_value::<AttemptEvent>(v).unwrap(), a);
    }
}
