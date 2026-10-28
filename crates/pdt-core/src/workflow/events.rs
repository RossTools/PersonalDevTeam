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
pub const ATTEMPT_LATE_COMMITS: &str = "attempt.late_commits";
pub const TICKET_STAGE_CHANGED: &str = "ticket.stage_changed";
pub const TICKET_COMMENTED: &str = "ticket.commented";
pub const TICKET_OVERLAP_FOUND: &str = "ticket.overlap_found";
pub const TICKET_OVERLAP_CLEARED: &str = "ticket.overlap_cleared";
pub const PR_COMMENTED: &str = "pr.commented";
pub const PR_REVIEWED: &str = "pr.reviewed";
pub const GITHUB_ISSUE_OPENED: &str = "github.issue.opened";
pub const SCHEDULE_FIRED: &str = "schedule.fired";
pub const WEBHOOK_RECEIVED: &str = "webhook.received";

pub const AGENT_OUTCOME: &str = "agent.outcome";
pub const OUTSIDE_INPUT: &str = "session.outside_input";
pub const TRIAGE_SKIPPED: &str = "triage.skipped";
pub const TICKET_REFINE_REQUESTED: &str = "ticket.refine_requested";

/// The event types that carry outside input (PDT-201): what someone outside PDT wrote. A run they start triages.
pub const UNTRUSTED_EVENTS: &[&str] = &[GITHUB_ISSUE_OPENED, WEBHOOK_RECEIVED];

/// Whether an event of type `kind` carries outside input.
pub fn untrusted(kind: &str) -> bool {
    UNTRUSTED_EVENTS.contains(&kind)
}

/// Whether an event's payload (a trigger) was written by someone outside the repository (PDT-202: `outside` on
/// `pr.commented` and `pr.reviewed`): it is never sent into an agent session.
pub fn by_outsider(payload: &serde_json::Value) -> bool {
    payload.get("outside").and_then(serde_json::Value::as_bool) == Some(true)
}

/// The CI provider of GitHub Actions runs.
pub const GITHUB_ACTIONS: &str = "github_actions";

/// One event type: its id, what it means and its payload's schema.
pub struct EventType {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Schema,
    /// The payload's fields that hold what someone outside PDT wrote (PDT-202): fenced wherever they go to an agent
    /// (an agent step's prompt, `session.send`'s text). Paths as in references (`issue.body`).
    pub outside: &'static [&'static str],
}

/// `T`'s JSON Schema, for an [`EventType`]'s `schema`.
pub fn schema_of<T: JsonSchema>() -> Schema {
    schemars::schema_for!(T)
}

/// PDT's own event types, in catalog order; an integration declares its own (pdt-api's `Integration::event_types`). A trigger kind of an event type is the plain event trigger unless a node
/// kind claims the type (pdt-api `NodeKind::event_type`: a schedule, a webhook, GitHub issues). Adding one: a constant,
/// an entry here, and a sensor or command that emits it.
pub const EVENT_TYPES: &[EventType] = &[
    EventType {
        id: CI_RUN_FAILED,
        title: "CI failed",
        description: "CI failed on the head commit of an attempt's pull request, or of its branch on a CI \
                      integration (once per set of failed runs).",
        schema: schema_of::<CiEvent>,
        outside: &[],
    },
    EventType {
        id: CI_RUN_PASSED,
        title: "CI passed",
        description: "Every CI run on the head commit of an attempt's pull request (or its branch on a CI integration) \
                      passed.",
        schema: schema_of::<CiEvent>,
        outside: &[],
    },
    EventType {
        id: PR_CONFLICT,
        title: "Pull request conflicts",
        description: "An attempt's pull request conflicts with its base branch (once per head commit).",
        schema: schema_of::<PrEvent>,
        outside: &[],
    },
    EventType {
        id: PR_QUEUE_LEFT,
        title: "Dropped from the merge queue",
        description: "The merge queue removed an attempt's pull request without merging it.",
        schema: schema_of::<PrEvent>,
        outside: &[],
    },
    EventType {
        id: PR_MERGED,
        title: "Pull request merged",
        description: "GitHub reports an attempt's pull request merged.",
        schema: schema_of::<PrEvent>,
        outside: &[],
    },
    EventType {
        id: PR_BEHIND,
        title: "Base moved while a pull request waits",
        description: "An open pull request with auto-merge on, not in a merge queue, is behind its base branch.",
        schema: schema_of::<PrEvent>,
        outside: &[],
    },
    EventType {
        id: ATTEMPT_STARTED,
        title: "Ticket started",
        description: "Start made a new attempt (branch and worktree) for a ticket.",
        schema: schema_of::<AttemptEvent>,
        outside: &[],
    },
    EventType {
        id: ATTEMPT_MERGED,
        title: "Attempt merged",
        description: "An attempt ended as merged: detected by git or GitHub, merged in PDT, or the ticket was closed.",
        schema: schema_of::<AttemptEvent>,
        outside: &[],
    },
    EventType {
        id: ATTEMPT_ABANDONED,
        title: "Attempt abandoned",
        description: "An attempt was abandoned (by the user, or because its ticket was closed unmerged).",
        schema: schema_of::<AttemptEvent>,
        outside: &[],
    },
    EventType {
        id: ATTEMPT_LATE_COMMITS,
        title: "Commits pushed after the merge",
        description: "An attempt's pull request merged, and its branch has commits the merged pull request doesn't \
                      (pushed after the merge), so the attempt stays open (once per branch head).",
        schema: schema_of::<LateCommitsEvent>,
        outside: &[],
    },
    EventType {
        id: TICKET_STAGE_CHANGED,
        title: "Ticket moved",
        description: "A ticket moved to another stage.",
        schema: schema_of::<TicketStageChanged>,
        outside: &[],
    },
    EventType {
        id: TICKET_COMMENTED,
        title: "Ticket commented",
        description: "Someone commented on a ticket: the user, an agent or a workflow (its actor says which).",
        schema: schema_of::<TicketCommented>,
        outside: &[],
    },
    EventType {
        id: TICKET_OVERLAP_FOUND,
        title: "Overlap found",
        description: "Two tickets in flight change the same files (once when it starts, then once per newly shared \
                      file or worse level; one event on each ticket).",
        schema: schema_of::<TicketOverlapFound>,
        outside: &[],
    },
    EventType {
        id: TICKET_OVERLAP_CLEARED,
        title: "Overlap cleared",
        description: "Two tickets that changed the same files no longer do (one finished, or the files were put back).",
        schema: schema_of::<TicketOverlapCleared>,
        outside: &[],
    },
    EventType {
        id: AGENT_OUTCOME,
        title: "Agent outcome",
        description: "An agent step on a ticket ended with an outcome (submitted by the agent, or taken over by the user).",
        schema: schema_of::<AgentOutcome>,
        outside: &[],
    },
    EventType {
        id: PR_COMMENTED,
        title: "Pull request commented",
        description: "Someone commented on an attempt's pull request: on the conversation, or on a line in a review.",
        schema: schema_of::<PrCommented>,
        // Only collaborators' reach agents (`outside`); their text is still fenced where a workflow quotes it.
        outside: &["body"],
    },
    EventType {
        id: PR_REVIEWED,
        title: "Pull request reviewed",
        description: "Someone submitted a review of an attempt's pull request (approved, changes requested, commented).",
        schema: schema_of::<PrReviewed>,
        // Only collaborators' reach agents (`outside`); their text is still fenced where a workflow quotes it.
        outside: &["body"],
    },
    EventType {
        id: GITHUB_ISSUE_OPENED,
        title: "GitHub issue opened",
        description: "A new issue in a watched GitHub repository (the first look at a repository only records where it \
                      is).",
        schema: schema_of::<GithubIssueOpened>,
        outside: &["title", "body"],
    },
    EventType {
        id: SCHEDULE_FIRED,
        title: "On a schedule",
        description: "Every interval, or at the times of a cron expression.",
        schema: schema_of::<ScheduleFired>,
        outside: &[],
    },
    EventType {
        id: WEBHOOK_RECEIVED,
        title: "Webhook",
        description: "Another machine posted to this host's webhook address (POST /api/hooks/<name>), with its secret.",
        schema: schema_of::<WebhookReceived>,
        outside: &["query", "headers", "body"],
    },
    EventType {
        id: OUTSIDE_INPUT,
        title: "Outside input reached an agent",
        description: "Text from outside PDT (an issue, a webhook, an imported ticket's description) went into an agent \
                      session: its source, the session and the ticket (also on the ticket's timeline).",
        schema: schema_of::<OutsideInput>,
        outside: &[],
    },
    EventType {
        id: TRIAGE_SKIPPED,
        title: "Triage skipped",
        description: "A new issue (or webhook) was not triaged: its author or repository had reached the hourly cap \
                      (Settings → Host). The Inbox sums them up.",
        schema: schema_of::<TriageSkipped>,
        outside: &[],
    },
    EventType {
        id: TICKET_REFINE_REQUESTED,
        title: "Refine requested",
        description: "Someone asked for a Draft or Ready ticket to be refined (Refine on the ticket).",
        schema: schema_of::<TicketRefineRequested>,
        outside: &[],
    },
];

/// One of PDT's own event types (every integration's: pdt-api's registry).
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
    /// `github_actions`, or a CI integration's id.
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
    /// The pull request's auto-merge is on (false without one).
    #[serde(default)]
    pub auto_merge: bool,
    /// The pull request is in the merge queue.
    #[serde(default)]
    pub in_merge_queue: bool,
    /// The pull request was set to merge (Merge…, `attempt.merge`, auto-merge or the merge queue) and hasn't merged
    /// (PDT-104): PR notices gets it merging again.
    #[serde(default)]
    pub merge_requested: bool,
    /// A workflow's merge of the attempt was refused for something its agent can fix (a conflict, uncommitted
    /// changes), and the pull request is still open (PDT-139): PR notices puts the Coder on its later failures.
    #[serde(default)]
    pub merge_refused: bool,
    /// An agent step is working on the attempt (a workflow already has an agent on it).
    #[serde(default)]
    pub agent_working: bool,
    /// A workflow's merge of the attempt waits for this CI (PDT-151): it handles a failure itself.
    #[serde(default)]
    pub merge_waiting: bool,
}

/// `attempt.late_commits` (PDT-151).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LateCommitsEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    /// The head the pull request merged at.
    pub merged_head: String,
    /// The branch's head now.
    pub head: String,
    /// The commits after `mergedHead`, newest first.
    pub commits: Vec<crate::pr::LateCommit>,
    /// One line for the Inbox: "CB-25: 1 commit pushed after PR #308 merged: cd5d0b7 …".
    pub summary: String,
    /// What PDT tells the attempt's agent.
    pub notice: Notice,
    /// The commits as a list for a ticket's description (`- cd5d0b7 Fix the build`).
    pub list: String,
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
    /// The pull request was set to merge (Merge…, `attempt.merge`, auto-merge or the merge queue) and hasn't merged
    /// (PDT-104): PR notices gets it merging again.
    #[serde(default)]
    pub merge_requested: bool,
    /// A workflow's merge of the attempt was refused for something its agent can fix (a conflict, uncommitted
    /// changes), and the pull request is still open (PDT-139): PR notices puts the Coder on its later failures.
    #[serde(default)]
    pub merge_refused: bool,
    /// An agent step is working on the attempt (a workflow already has an agent on it).
    #[serde(default)]
    pub agent_working: bool,
}

/// How an attempt came to be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeVia {
    /// Merge detection found it (git or GitHub): the usual way, after a PR merges.
    Detected,
    /// The user merged it locally in PDT (Merge…), which finishes the ticket itself.
    MergedHere,
    /// A workflow or an agent merged it locally in PDT (`attempt.merge`, or the API without `by: "user"`, PDT-181):
    /// Finish merged tickets finishes it, as a detected merge.
    Automated,
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
    /// The ticket's labels when it happened, so a condition can branch on them (`contains` `automerge`).
    #[serde(default)]
    pub labels: Vec<String>,
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
    /// The GitHub issue the ticket is linked to, if any (`owner/name#12`).
    #[serde(default)]
    pub github_issue: Option<String>,
    /// The issue in each other tracker the ticket is linked to, by the tracker (an integration's id, or `github`):
    /// `{{trigger.external.<tracker>}}`.
    #[serde(default)]
    pub external: std::collections::BTreeMap<String, String>,
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
    /// The workflow whose run wrote it for the user (a note with a gate's choice, which that run passes on itself).
    #[serde(default)]
    pub workflow_id: Option<String>,
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

/// `ticket.overlap_cleared`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketOverlapCleared {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub other_ticket_id: String,
    pub other_key: String,
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

/// `pr.commented`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrCommented {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    /// The comment.
    pub url: String,
    #[ts(type = "number")]
    pub comment_id: u64,
    pub author: String,
    pub body: String,
    /// A review comment's file and line.
    pub path: Option<String>,
    pub line: Option<u32>,
    /// On a line, in a review (else on the conversation).
    pub in_review: bool,
    /// Written by the user themself (the account PDT is logged in to GitHub as).
    pub by_owner: bool,
    pub is_bot: bool,
    /// Written by someone who isn't a collaborator on the repository (not its owner, a member or a collaborator, nor
    /// a bot installed on it; PDT-202): shown in PDT, never sent into an agent session.
    #[serde(default)]
    pub outside: bool,
    pub notice: Notice,
}

/// `pr.reviewed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrReviewed {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    pub pr: u32,
    pub url: String,
    #[ts(type = "number")]
    pub review_id: u64,
    pub author: String,
    /// `approved`, `changes_requested` or `commented`.
    pub state: String,
    pub body: String,
    pub by_owner: bool,
    pub is_bot: bool,
    /// As on `pr.commented`.
    #[serde(default)]
    pub outside: bool,
    pub notice: Notice,
}

/// `github.issue.opened`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubIssueOpened {
    /// The project whose repository it is, if any.
    pub project_id: Option<String>,
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    pub labels: Vec<String>,
    pub created_at: String,
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

/// `webhook.received`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebhookReceived {
    /// The webhook connection's name.
    pub hook: String,
    pub method: String,
    #[ts(type = "Record<string, string>")]
    pub query: std::collections::BTreeMap<String, String>,
    /// `content-type`, `user-agent` and the `x-*` headers, names in lower case.
    #[ts(type = "Record<string, string>")]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Parsed JSON, else the text.
    #[ts(type = "unknown")]
    pub body: serde_json::Value,
}

/// `session.outside_input` (PDT-202): outside text went into an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutsideInput {
    pub project_id: Option<String>,
    /// The ticket the session works on, or the one made from the input; none for a triage that made none.
    pub ticket_id: Option<String>,
    pub session_id: String,
    /// Where the text came from: `owner/name#12`, a comment's URL, `webhook <name>`, `jira ABC-1`.
    pub source: String,
    /// A link to it, when there is one.
    pub url: Option<String>,
    /// How it went in: `prompt` (an agent step's prompt), `message` (`session.send`), `ticket` (the session works
    /// on an external ticket, whose description is outside text), `triage` (the triage session that made the
    /// ticket).
    pub via: String,
    pub run_id: Option<String>,
    pub workflow_name: Option<String>,
}

/// Why triage skipped an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TriageCap {
    /// Its author had reached the cap per author.
    Author,
    /// Its repository (or webhook) had reached the cap per repository.
    Repo,
}

/// `triage.skipped` (PDT-202).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TriageSkipped {
    pub project_id: Option<String>,
    /// `owner/name`, or `webhook <name>`.
    pub repo: String,
    /// The issue's, when it is one.
    pub number: Option<u64>,
    pub url: Option<String>,
    pub author: Option<String>,
    pub workflow_id: String,
    pub workflow_name: String,
    /// The event that would have started the run.
    pub event_id: String,
    pub cap: TriageCap,
    /// The cap reached (runs an hour).
    pub limit: u32,
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
            workflow_id: None,
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
            labels: vec!["automerge".into()],
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["labels"], serde_json::json!(["automerge"]));
        assert_eq!((v["attemptId"].as_str(), v["via"].as_str()), (Some("a"), Some("merged_here")));
        assert_eq!(serde_json::from_value::<AttemptEvent>(v).unwrap(), a);
    }
}
