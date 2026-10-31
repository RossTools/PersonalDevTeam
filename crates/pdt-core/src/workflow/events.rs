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
pub const GITHUB_PR_OPENED: &str = "github.pull_request.opened";
pub const GITHUB_PR_PUSHED: &str = "github.pull_request.pushed";
pub const GITHUB_PR_CLOSED: &str = "github.pull_request.closed";
pub const GITHUB_WORKFLOW_FAILED: &str = "github.workflow.failed";
pub const SCHEDULE_FIRED: &str = "schedule.fired";
pub const WEBHOOK_RECEIVED: &str = "webhook.received";

pub const AGENT_OUTCOME: &str = "agent.outcome";
pub const OUTSIDE_INPUT: &str = "session.outside_input";
pub const TRIAGE_SKIPPED: &str = "triage.skipped";
pub const TICKET_REFINE_REQUESTED: &str = "ticket.refine_requested";
pub const TICKET_STARTABLE: &str = "ticket.startable";
pub const SLOT_FREED: &str = "slot.freed";

/// The event types that carry outside input (PDT-201): what someone outside PDT wrote. A run they start triages.
pub const UNTRUSTED_EVENTS: &[&str] =
    &[GITHUB_ISSUE_OPENED, GITHUB_PR_OPENED, GITHUB_PR_PUSHED, GITHUB_PR_CLOSED, WEBHOOK_RECEIVED];

/// The pull request events (PDT-268): pull requests on a project's repository that PDT didn't open.
pub const GITHUB_PR_EVENTS: &[&str] = &[GITHUB_PR_OPENED, GITHUB_PR_PUSHED, GITHUB_PR_CLOSED];

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
pub use crate::pr::GITHUB_ACTIONS;

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
        id: TICKET_STARTABLE,
        title: "Ticket can start",
        description: "In a project with auto-start on, a To do or Ready ticket became one auto-start could take: it \
                      went the project's start delay unchanged, or its last blocker or conflict cleared (once each \
                      time; again after PDT restarts).",
        schema: schema_of::<TicketStartable>,
        outside: &[],
    },
    EventType {
        id: SLOT_FREED,
        title: "Room to start another",
        description: "In a project with auto-start on, fewer tickets are in flight (an attempt merged or was \
                      abandoned) or its cap rose; or an agent slot freed on this host (`reason`: `attempt`, `cap` or \
                      `agent`; an agent slot has no project).",
        schema: schema_of::<SlotFreed>,
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

/// The GitHub plugin's event types (its triggers), apart from PDT's own: they leave the catalog while it is off
/// (PDT-251).
pub const GITHUB_EVENT_TYPES: &[EventType] = &[
    EventType {
        id: GITHUB_ISSUE_OPENED,
        title: "GitHub issue opened",
        description: "A new issue in a watched GitHub repository (the first look at a repository only records where it \
                      is).",
        schema: schema_of::<GithubIssueOpened>,
        outside: &["title", "body"],
    },
    EventType {
        id: GITHUB_PR_OPENED,
        title: "Outside pull request opened",
        description: "A pull request PDT didn't open (not a ticket's branch) on a project's GitHub repository: from a \
                      fork, a bot such as Dependabot, or a collaborator (`kind`). The first look at a repository only \
                      records the ones already open.",
        schema: schema_of::<GithubPullRequest>,
        outside: &["title", "body"],
    },
    EventType {
        id: GITHUB_PR_PUSHED,
        title: "Outside pull request pushed",
        description: "New commits on a pull request PDT didn't open (its head moved): `previousHead` is where it was.",
        schema: schema_of::<GithubPullRequest>,
        outside: &["title", "body"],
    },
    EventType {
        id: GITHUB_PR_CLOSED,
        title: "Outside pull request closed",
        description: "A pull request PDT didn't open was closed or merged (`merged`).",
        schema: schema_of::<GithubPullRequest>,
        outside: &["title", "body"],
    },
    EventType {
        id: GITHUB_WORKFLOW_FAILED,
        title: "GitHub workflow run failed",
        description: "A run of one GitHub Actions workflow failed, however it was started (a schedule, by hand, a \
                      push): each failed attempt once. Checked every `every` (default 30 minutes); the first look \
                      only records where the runs are.",
        schema: schema_of::<GithubWorkflowFailed>,
        // A run's title is its commit's or pull request's, which a contributor may have written.
        outside: &["title"],
    },
];

/// One of PDT's own event types (every integration's: pdt-api's registry).
pub fn event_type(id: &str) -> Option<&'static EventType> {
    EVENT_TYPES.iter().chain(GITHUB_EVENT_TYPES).find(|t| t.id == id)
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
    /// Its CI provider (`github_actions`, `buildkite`, `local`).
    #[serde(default)]
    pub provider: String,
    /// The provider's id for it (PDT-242: text; events logged before were numbers).
    #[serde(deserialize_with = "id_text")]
    pub id: String,
    pub attempt: u32,
    /// Its workflow or pipeline.
    #[serde(alias = "workflow")]
    pub name: String,
    pub state: CiState,
    pub url: String,
    pub failed_jobs: Vec<String>,
}

/// A run id as text, from text or a number.
fn id_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(serde::de::Error::custom(format!("a run id is text, not {other}"))),
    }
}

/// `ci.run_failed`, `ci.run_passed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub attempt: AttemptRef,
    /// The CI provider whose runs these are: `github_actions`, `local`, or a CI integration's id (`buildkite`).
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
    /// The user closed the ticket (Done says the work landed, unless its PR is unmerged).
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
    /// The agent session that wrote it (an agent's comment through pdt-mcp), so a watching PM conversation skips its own.
    #[serde(default)]
    pub by_session: Option<String>,
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

/// `github.pull_request.opened`, `github.pull_request.pushed` and `github.pull_request.closed` (PDT-268).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubPullRequest {
    /// The project whose repository it is.
    pub project_id: String,
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    /// `outside`, `bot` or `collaborator`.
    pub kind: crate::outside_pr::AuthorKind,
    /// The repository its branch is in (`owner/name`); none when that fork was deleted.
    pub head_repo: Option<String>,
    pub head_branch: String,
    pub head_sha: String,
    /// Its branch is in another repository.
    pub fork: bool,
    /// The branch it merges into.
    pub base: String,
    pub labels: Vec<String>,
    pub draft: bool,
    pub changed_files: u64,
    pub additions: u64,
    pub deletions: u64,
    /// For a Dependabot update, what it bumps (`major`, `minor`, `patch`), when its title says.
    pub update_type: Option<crate::outside_pr::UpdateType>,
    pub created_at: String,
    /// `pushed`: the head before the push.
    pub previous_head: Option<String>,
    /// `closed`: it was merged.
    pub merged: bool,
}

/// `github.workflow.failed` (PDT-292): one failed attempt of a watched workflow's run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubWorkflowFailed {
    /// The watching workflow and its trigger node.
    pub workflow_id: String,
    pub node_id: String,
    /// The project whose repository it is, if any.
    pub project_id: Option<String>,
    /// `owner/name`.
    pub repo: String,
    /// The workflow's name as Actions shows it (`release`) and its file (`.github/workflows/release.yml`).
    pub workflow: String,
    pub path: String,
    /// The run's id, and the attempt that failed (a re-run's is 2, 3…).
    pub run_id: String,
    pub attempt: u32,
    pub title: String,
    pub branch: String,
    /// What started it: `schedule`, `workflow_dispatch`, `push`, `pull_request`…
    pub event: String,
    pub head_sha: String,
    pub url: String,
    /// The version or tag it is for (a tag it ran on, or one in its title), when it has one.
    pub version: Option<String>,
    /// The jobs that failed.
    pub failed_jobs: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
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

/// `ticket.startable` (PDT-327). The ticket is `ticket`, not `ticketId`: a pickup run it starts may start another
/// ticket, so the run isn't the ticket's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketStartable {
    pub project_id: String,
    pub ticket: StartableTicket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartableTicket {
    pub id: String,
    pub key: String,
    pub title: String,
    /// `todo` or `ready`.
    pub stage: String,
    pub priority: String,
    pub labels: Vec<String>,
}

/// `slot.freed` (PDT-327).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlotFreed {
    /// The project with fewer tickets in flight; none for an agent slot (the host's).
    pub project_id: Option<String>,
    /// `attempt`, `cap` or `agent`.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ci_run_logged_before_providers_still_reads() {
        let old = serde_json::json!({"id": 391, "attempt": 2, "workflow": "ci", "state": "failed", "url": "u",
            "failedJobs": []});
        let r: CiRunRef = serde_json::from_value(old).unwrap();
        assert_eq!((r.provider.as_str(), r.id.as_str(), r.name.as_str()), ("", "391", "ci"));
        let new = serde_json::to_value(&CiRunRef { provider: "buildkite".into(), id: "app/3".into(), ..r }).unwrap();
        assert_eq!((new["id"].as_str(), new["name"].as_str()), (Some("app/3"), Some("ci")));
        assert_eq!(serde_json::from_value::<CiRunRef>(new).unwrap().provider, "buildkite");
    }

    #[test]
    fn every_type_is_unique_and_has_a_schema() {
        let mut ids = std::collections::HashSet::new();
        for t in EVENT_TYPES.iter().chain(GITHUB_EVENT_TYPES) {
            assert!(ids.insert(t.id), "{} twice", t.id);
            assert!(t.id.contains('.'));
            assert!((t.schema)().as_object().is_some_and(|o| o.contains_key("properties")), "{}", t.id);
        }
        assert!(event_type(PR_CONFLICT).is_some() && event_type("nope").is_none());
    }

    #[test]
    fn outside_pull_requests_are_untrusted_and_their_text_fenced() {
        for id in GITHUB_PR_EVENTS {
            assert!(untrusted(id), "{id}");
            assert_eq!(event_type(id).unwrap().outside, ["title", "body"], "{id}");
        }
        let schema = serde_json::to_value((event_type(GITHUB_PR_OPENED).unwrap().schema)()).unwrap();
        for f in ["kind", "fork", "headRepo", "updateType", "changedFiles", "previousHead", "merged"] {
            assert!(schema["properties"].get(f).is_some(), "{f}");
        }
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
            by_session: None,
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
