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
pub const TICKET_CREATED: &str = "ticket.created";
pub const TICKET_STAGE_CHANGED: &str = "ticket.stage_changed";
pub const TICKET_UNBLOCKED: &str = "ticket.unblocked";
pub const TICKET_LABELS_CHANGED: &str = "ticket.labels_changed";
pub const TICKET_PRIORITY_CHANGED: &str = "ticket.priority_changed";
pub const TICKET_PARENT_CHANGED: &str = "ticket.parent_changed";
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
pub const DOCUMENT_CHANGED: &str = "document.changed";
pub const DOCUMENT_COMMENT_ADDED: &str = "document.comment_added";
pub const INBOX_CREATED: &str = "inbox.created";
pub const INBOX_ANSWERED: &str = "inbox.answered";
pub const SESSION_NEEDS_YOU: &str = "session.needs_you";
pub const SESSION_TURN_ENDED: &str = "session.turn_ended";
pub const SESSION_EXITED: &str = "session.exited";
pub const RUN_FAILED: &str = "run.failed";
pub const RUN_FINISHED: &str = "run.finished";
pub const RELEASE_PUBLISHED: &str = "release.published";
pub const UPDATE_AVAILABLE: &str = "update.available";
pub const UPDATE_INSTALLED: &str = "update.installed";
pub const DISK_LOW: &str = "disk.low";
pub const DISK_RECOVERED: &str = "disk.recovered";
pub const CONNECTION_FAILED: &str = "connection.failed";
pub const TODO_DUE: &str = "todo.due";

pub const AGENT_OUTCOME: &str = "agent.outcome";
pub const AGENT_STALLED: &str = "agent.stalled";
pub const AGENT_LOOP_LIMIT: &str = "agent.loop_limit";
pub const AGENT_TIMED_OUT: &str = "agent.timed_out";
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

/// Whether an event's payload (a trigger) says the user wrote its outside fields (`byUser` on
/// `document.comment_added`, PDT-318): they are not fenced, and no outside input is recorded.
pub fn by_user(payload: &serde_json::Value) -> bool {
    payload.get("byUser").and_then(serde_json::Value::as_bool) == Some(true)
}

/// The event types a workflow's own runs can cause (PDT-364): their payload's `workflowId` names the workflow whose run
/// caused them (the run itself, its Inbox item, its agent step's session). A trigger skips its own workflow's unless
/// it sets `includeOwn`, so a workflow that reports run failures never reports its own.
pub const OWN_RUN_EVENTS: &[&str] = &[
    INBOX_CREATED,
    INBOX_ANSWERED,
    SESSION_NEEDS_YOU,
    SESSION_TURN_ENDED,
    SESSION_EXITED,
    RUN_FAILED,
    RUN_FINISHED,
    AGENT_STALLED,
    AGENT_LOOP_LIMIT,
    AGENT_TIMED_OUT,
];

/// The events of an agent step that is stuck (PDT-367): "When an agent is stuck" handles them.
pub const AGENT_STUCK_EVENTS: &[&str] = &[AGENT_STALLED, AGENT_LOOP_LIMIT, AGENT_TIMED_OUT];

/// Whether an event of type `kind` with `payload` came from a run of workflow `workflow_id` (see [`OWN_RUN_EVENTS`]).
pub fn from_own_run(kind: &str, payload: &serde_json::Value, workflow_id: &str) -> bool {
    OWN_RUN_EVENTS.contains(&kind) && payload.get("workflowId").and_then(serde_json::Value::as_str) == Some(workflow_id)
}

/// How many runs in a row `run.*` events may start one another (PDT-364): workflow A reports B's runs and B reports
/// A's would go on forever. A run started by a `run.*` event is one further down the chain than that event's run;
/// a run this far down logs no `run.*` events.
pub const MAX_RUN_CHAIN: u32 = 3;

/// A run's place in a chain of runs started by `run.*` events, from its trigger: 0 unless a `run.*` event started it.
pub fn run_chain(trigger: &serde_json::Value) -> u32 {
    match trigger.get("type").and_then(serde_json::Value::as_str) {
        Some(RUN_FAILED | RUN_FINISHED) => {
            let up = trigger.get("chain").and_then(serde_json::Value::as_u64).unwrap_or(0);
            u32::try_from(up).unwrap_or(MAX_RUN_CHAIN).saturating_add(1)
        }
        _ => 0,
    }
}

/// A short tail of an agent's last message for a payload (`lastMessage`): its last `max` characters, on a word
/// boundary when one is near, with `…` before it when it was cut.
pub fn message_tail(text: &str, max: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - max).map_or(0, |(i, _)| i);
    let tail = &text[start..];
    // Start at the next word when one begins soon.
    let tail = match tail.find(char::is_whitespace) {
        Some(i) if i < max / 5 => tail[i..].trim_start(),
        _ => tail,
    };
    format!("…{tail}")
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
        id: TICKET_CREATED,
        title: "Ticket created",
        description: "A ticket was made: by the user, the PM, an agent, a workflow or from outside input (`actor`). An \
                      agent's has its `persona`, `sessionId` and the ticket it was working on (`fromTicket`); parts \
                      made with Make into parts have `viaParts`; an agent's `reason` says why it made it.",
        schema: schema_of::<TicketCreated>,
        outside: &[],
    },
    EventType {
        id: TICKET_STAGE_CHANGED,
        title: "Ticket moved",
        description: "A ticket moved to another stage. A part names its parent (`parentId`, `parentKey`).",
        schema: schema_of::<TicketStageChanged>,
        outside: &[],
    },
    EventType {
        id: TICKET_UNBLOCKED,
        title: "Ticket unblocked",
        description: "The last open ticket blocking a ticket reached Done or Won't do (`reason`: `closed`), or its \
                      link was removed (`unlinked`).",
        schema: schema_of::<TicketUnblocked>,
        outside: &[],
    },
    EventType {
        id: TICKET_LABELS_CHANGED,
        title: "Ticket labels changed",
        description: "Labels were added to or removed from a ticket (`added`, `removed`; `labels` is the new set).",
        schema: schema_of::<TicketLabelsChanged>,
        outside: &[],
    },
    EventType {
        id: TICKET_PRIORITY_CHANGED,
        title: "Ticket priority changed",
        description: "A ticket's priority changed (`from`, `to`).",
        schema: schema_of::<TicketPriorityChanged>,
        outside: &[],
    },
    EventType {
        id: TICKET_PARENT_CHANGED,
        title: "Ticket parent changed",
        description: "A ticket was made a part of another, moved to another parent, or detached (`from` and `to` are \
                      the parents' keys, empty for none).",
        schema: schema_of::<TicketParentChanged>,
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
        description: "Someone commented on a ticket: the user, an agent or a workflow (its actor says which, its author \
                      which persona: `PM`, `Coder` …).",
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
        id: AGENT_STALLED,
        title: "Agent stalled",
        description: "An agent step's turn ended without an outcome, or its session stopped or was removed (`why`). \
                      `nudges` and `restarts` count what was already tried for the step; `goal` is its prompt and \
                      `lastMessage` the end of what the agent last said. Skips this workflow's own steps unless the \
                      trigger sets `includeOwn`.",
        schema: schema_of::<AgentStuck>,
        outside: &[],
    },
    EventType {
        id: AGENT_LOOP_LIMIT,
        title: "Agent step at its loop limit",
        description: "An agent step is about to run more times in one run than its loop limit allows (`passes`). \
                      Skips this workflow's own steps unless the trigger sets `includeOwn`.",
        schema: schema_of::<AgentStuck>,
        outside: &[],
    },
    EventType {
        id: AGENT_TIMED_OUT,
        title: "Agent step timed out",
        description: "An agent step had no outcome within its time limit or its turns (`why`); `restarts` counts the \
                      fresh sessions it already had. Skips this workflow's own steps unless the trigger sets \
                      `includeOwn`.",
        schema: schema_of::<AgentStuck>,
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
        id: INBOX_CREATED,
        title: "Inbox item opened",
        description: "Something new waits on the user in the Inbox: a question, a gate, an approval, a stalled step, a \
                      proposal … (`kind`, `group`). Not a session waiting: that is `session.needs_you`. Skips items \
                      this workflow's own runs opened unless the trigger sets `includeOwn`.",
        schema: schema_of::<InboxCreated>,
        outside: &[],
    },
    EventType {
        id: INBOX_ANSWERED,
        title: "Inbox item answered",
        description: "Someone answered an Inbox item (`choice`, `answer`; `by`: `user` or `agent`). `ageMs` is how long \
                      it waited. Skips items this workflow's own runs opened unless the trigger sets `includeOwn`.",
        schema: schema_of::<InboxAnswered>,
        outside: &[],
    },
    EventType {
        id: SESSION_NEEDS_YOU,
        title: "Session needs you",
        description: "An agent session waits on a permission prompt or a question. `workflowStep` says whether a \
                      workflow's agent step runs it or it is the user's own; `lastMessage` is the end of what the \
                      agent last said. Skips this workflow's own steps' sessions unless the trigger sets `includeOwn`.",
        schema: schema_of::<SessionEvent>,
        outside: &[],
    },
    EventType {
        id: SESSION_TURN_ENDED,
        title: "Session turn ended",
        description: "An agent session finished a turn and waits for its next message (once per turn: can be many). \
                      Skips this workflow's own steps' sessions unless the trigger sets `includeOwn`.",
        schema: schema_of::<SessionEvent>,
        outside: &[],
    },
    EventType {
        id: SESSION_EXITED,
        title: "Session exited",
        description: "An agent session's process ended: `exitCode` (none when unknown), `stoppedByPdt` when PDT or the \
                      user stopped it rather than the agent ending or crashing. Skips this workflow's own steps' \
                      sessions unless the trigger sets `includeOwn`.",
        schema: schema_of::<SessionExited>,
        outside: &[],
    },
    EventType {
        id: RUN_FAILED,
        title: "Workflow run failed",
        description: "A workflow run failed: its failing step (`failedNode`, `failedKind`) and `error`. Never this \
                      workflow's own runs unless the trigger sets `includeOwn`.",
        schema: schema_of::<RunEnded>,
        outside: &[],
    },
    EventType {
        id: RUN_FINISHED,
        title: "Workflow run finished",
        description: "A workflow run ended without failing (`status`: `succeeded`, `quiet` or `cancelled`), with the \
                      node it ended at (`endNode`) and `durationMs`. Never this workflow's own runs unless the trigger \
                      sets `includeOwn`.",
        schema: schema_of::<RunEnded>,
        outside: &[],
    },
    EventType {
        id: RELEASE_PUBLISHED,
        title: "Release published",
        description: "A release (or pre-release: `prerelease`) was published on a project's repository, on a forge \
                      that has releases (GitHub, GitLab). Checked every minute for the projects an enabled trigger \
                      covers; the first look at a project only records its releases.",
        schema: schema_of::<ReleasePublished>,
        outside: &[],
    },
    EventType {
        id: UPDATE_AVAILABLE,
        title: "PDT update available",
        description: "A check found a newer version of PDT on this host's update channel (once per version).",
        schema: schema_of::<UpdateAvailable>,
        outside: &[],
    },
    EventType {
        id: UPDATE_INSTALLED,
        title: "PDT updated",
        description: "PDT started on a version it was just updated to (`automatic`: installed without the user asking).",
        schema: schema_of::<UpdateInstalled>,
        outside: &[],
    },
    EventType {
        id: DISK_LOW,
        title: "Disk space low",
        description: "A disk PDT uses fell below the low-disk threshold (`level`: `low`), or below the critical one \
                      (`critical`). Once per drop, not every check; `reclaimable` is the build output in finished \
                      tickets' worktrees.",
        schema: schema_of::<DiskEvent>,
        outside: &[],
    },
    EventType {
        id: DISK_RECOVERED,
        title: "Disk space recovered",
        description: "A disk that was low has enough free space again (`from`: the level it was at).",
        schema: schema_of::<DiskEvent>,
        outside: &[],
    },
    EventType {
        id: CONNECTION_FAILED,
        title: "Connection failing",
        description: "A connection (`source`: `connection`) or a forge login (`forge`) stopped working: a revoked \
                      token, the service down. Once each time it starts failing, not on every failed use.",
        schema: schema_of::<ConnectionFailed>,
        outside: &[],
    },
    EventType {
        id: TODO_DUE,
        title: "Todo due",
        description: "One of the user's todos comes due: its day starts, or its time comes (`overdue` when PDT was \
                      off then). Once per todo and due date.",
        schema: schema_of::<TodoDue>,
        outside: &[],
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

/// The documents' event types (PDT-318), the documents integration's triggers: PDT looks at the documents enabled
/// workflows watch every minute.
pub const DOCUMENT_EVENT_TYPES: &[EventType] = &[
    EventType {
        id: DOCUMENT_CHANGED,
        title: "Document changed",
        description: "A document's content changed, through PDT or at its provider (its version moved). Checked every \
                      minute for the documents a workflow watches: the one it names, else those linked to its \
                      projects; the first look only records the version.",
        schema: schema_of::<DocumentChanged>,
        outside: &[],
    },
    EventType {
        id: DOCUMENT_COMMENT_ADDED,
        title: "New comment on a document",
        description: "Someone commented on a document or replied in a comment's thread (`reply`; `thread` is the \
                      comment to reply to). Agents' and workflows' own comments don't count. Checked every minute for \
                      the documents a workflow watches; the first look only records the comments already there.",
        schema: schema_of::<DocumentCommentAdded>,
        // What the people a document is shared with wrote: fenced for an agent unless the user wrote it (`byUser`).
        outside: &["body", "quote"],
    },
];

/// One of PDT's own event types (every integration's: pdt-api's registry).
pub fn event_type(id: &str) -> Option<&'static EventType> {
    EVENT_TYPES.iter().chain(GITHUB_EVENT_TYPES).chain(DOCUMENT_EVENT_TYPES).find(|t| t.id == id)
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
    /// The ticket's title when it happened.
    #[serde(default)]
    pub ticket_title: String,
}

/// The document an event is about (`document.*`), and the project it is for: an event is logged once for each project
/// the document is linked to (`projectIds`, in order), or once without a project when it is linked to none.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentRef {
    /// One of `projectIds`; empty for a document linked to no project.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_id: String,
    pub document_id: String,
    pub title: String,
    /// `doc` or `sheet`.
    pub kind: String,
    /// The provider that keeps it (`local`, `google`).
    pub provider: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Every project it is linked to.
    #[serde(default)]
    pub project_ids: Vec<String>,
    /// The keys of the tickets it is linked to.
    #[serde(default)]
    pub tickets: Vec<String>,
}

/// `document.changed` (PDT-318).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentChanged {
    #[serde(flatten)]
    #[ts(flatten)]
    pub document: DocumentRef,
    /// The provider's version now.
    pub version: String,
    /// The version PDT saw before.
    pub previous_version: String,
}

/// `document.comment_added` (PDT-318).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentCommentAdded {
    #[serde(flatten)]
    #[ts(flatten)]
    pub document: DocumentRef,
    /// The new comment's (or reply's) id.
    pub comment_id: String,
    /// The comment whose thread it is in: the one to reply to (the comment itself, or the one it replies to).
    pub thread: String,
    /// It is a reply in a thread, not a new comment.
    pub reply: bool,
    /// The name it shows.
    pub author: String,
    /// The user wrote it (in PDT, or as the signed-in account): its text is not outside input.
    pub by_user: bool,
    pub body: String,
    /// The text the thread is on, when the provider says.
    #[serde(default)]
    pub quote: Option<String>,
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
    /// Its parent, when it is a part: id and key (PDT-377).
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub parent_key: Option<String>,
}

/// `ticket.created` (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketCreated {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub title: String,
    pub stage: String,
    pub labels: Vec<String>,
    pub priority: String,
    /// Its parent, when it is a part: id and key.
    pub parent_id: Option<String>,
    pub parent_key: Option<String>,
    /// Who made it: `user`, `pm`, `agent`, `workflow` or `external` (from outside input).
    pub actor: String,
    /// The persona of the agent session that made it (`PM`, `Coder` …).
    pub persona: Option<String>,
    pub session_id: Option<String>,
    /// The ticket that agent session was working on: its key and id.
    pub from_ticket: Option<String>,
    pub from_ticket_id: Option<String>,
    /// Made with Make into parts.
    pub via_parts: bool,
    /// Why the agent made it (`create_ticket`'s `reason`; PDT-366).
    #[serde(default)]
    pub reason: Option<String>,
}

/// Who changed a ticket, on the `ticket.*_changed` events (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketChangeRef {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub title: String,
    pub stage: String,
    /// `user`, `agent` or `system` (a workflow or PDT).
    pub actor: String,
    /// The persona of the agent session that did it, and the session.
    pub persona: Option<String>,
    pub session_id: Option<String>,
}

/// `ticket.unblocked` (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketUnblocked {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub title: String,
    pub stage: String,
    pub priority: String,
    pub labels: Vec<String>,
    /// The last blocker: its id and key.
    pub blocker_id: String,
    pub blocker_key: String,
    /// `closed` (it reached Done or Won't do) or `unlinked` (its link was removed).
    pub reason: String,
}

/// `ticket.labels_changed` (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketLabelsChanged {
    #[serde(flatten)]
    #[ts(flatten)]
    pub ticket: TicketChangeRef,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Its labels now.
    pub labels: Vec<String>,
}

/// `ticket.priority_changed` (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketPriorityChanged {
    #[serde(flatten)]
    #[ts(flatten)]
    pub ticket: TicketChangeRef,
    pub from: String,
    pub to: String,
}

/// `ticket.parent_changed` (PDT-363).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketParentChanged {
    #[serde(flatten)]
    #[ts(flatten)]
    pub ticket: TicketChangeRef,
    /// The parent before and after: keys (empty for none) and ids.
    pub from: String,
    pub to: String,
    pub from_id: Option<String>,
    pub to_id: Option<String>,
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
    /// Who wrote it: `user`, the persona of the agent session that did (`PM`, `Coder` …), else the actor.
    #[serde(default)]
    pub author: String,
    /// The ticket's attachments it links to (screenshots, files), in the order it shows them.
    #[serde(default)]
    pub attachments: Vec<CommentAttachment>,
    /// The attachments as lines for a message (a blank line, then each name and its local path), or empty: filled in
    /// as runs see the event.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub attachment_list: String,
}

/// An attachment a comment links to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommentAttachment {
    pub id: String,
    pub name: String,
    pub media_type: String,
    /// The file's name in the ticket's attachments folder.
    pub file_name: String,
    /// Its local path, filled in as runs see the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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

/// The Inbox item an `inbox.*` event is about (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxRef {
    pub item_id: String,
    /// `question`, `gate`, `approval`, `stalled`, `ticket_proposal` …
    pub kind: String,
    /// Its group in the Inbox (`question`, `approval`, `gate`, `accept` …).
    pub group: String,
    pub title: String,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub ticket_key: Option<String>,
    /// The agent session it is from, when it is one's (a question, an approval).
    pub session_id: Option<String>,
    /// The persona whose step it is (`Coder` …).
    pub persona: Option<String>,
    /// The workflow run it waits in, and its workflow.
    pub run_id: Option<String>,
    pub workflow_id: Option<String>,
    /// When it opened (ms since the epoch), and how long ago that was when the event was logged.
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub age_ms: i64,
}

/// `inbox.created` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxCreated {
    #[serde(flatten)]
    #[ts(flatten)]
    pub item: InboxRef,
}

/// `inbox.answered` (PDT-364).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxAnswered {
    #[serde(flatten)]
    #[ts(flatten)]
    pub item: InboxRef,
    /// The choice taken (`allow`, `approve`, `outcome:approved` …), when it was one.
    pub choice: Option<String>,
    /// What was written: a question's answer, a note.
    pub answer: Option<String>,
    /// `user` (in PDT) or `agent` (through the API).
    pub by: String,
}

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

/// A release asset (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseAssetRef {
    pub name: String,
    /// Where it downloads from.
    pub url: String,
    /// Bytes, when the forge says.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
}

/// `release.published` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleasePublished {
    pub project_id: String,
    /// The forge (`github`, `gitlab` …) and the repository on it (`owner/name`).
    pub forge: String,
    pub repo: String,
    pub tag: String,
    pub name: String,
    /// Its page.
    pub url: String,
    pub prerelease: bool,
    /// When it was published (the forge's timestamp text), when known.
    pub published_at: Option<String>,
    /// Its notes (Markdown).
    pub notes: String,
    pub assets: Vec<ReleaseAssetRef>,
}

/// `update.available` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateAvailable {
    /// The new version, and the one running.
    pub version: String,
    pub current: String,
    pub tag: String,
    /// The host's update channel (`stable` or `nightly`).
    pub channel: String,
    /// A nightly build.
    pub nightly: bool,
    /// The release's page.
    pub notes_url: String,
    pub notes: String,
    pub published_at: Option<String>,
}

/// `update.installed` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstalled {
    pub version: String,
    pub from_version: String,
    pub channel: String,
    /// The release's page.
    pub notes_url: String,
    /// Installed without the user asking (the automatic update when idle).
    pub automatic: bool,
    /// When it was installed (ms since the epoch).
    #[ts(type = "number")]
    pub at: i64,
}

/// `disk.low` and `disk.recovered` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskEvent {
    /// PDT's data folder and the projects' folders on this disk.
    pub folders: Vec<String>,
    #[ts(type = "number")]
    pub free: u64,
    #[ts(type = "number")]
    pub total: u64,
    /// `ok`, `low` or `critical`, now; and before.
    pub level: String,
    pub from: String,
    /// The thresholds, in GB (0: that warning is off).
    pub low_gb: u32,
    pub critical_gb: u32,
    /// Bytes of build output in finished tickets' worktrees (as last measured): what cleaning them would free.
    #[ts(type = "number")]
    pub reclaimable: u64,
}

/// `connection.failed` (PDT-365).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionFailed {
    /// The connection's name, or for a forge login the forge and host (`GitHub (github.com)`).
    pub name: String,
    /// The connection's kind (`http`, `jira`, `scryer` …), or the forge's (`github`, `gitlab` …).
    pub kind: String,
    /// `connection` or `forge`.
    pub source: String,
    pub error: String,
}

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
        for t in EVENT_TYPES.iter().chain(GITHUB_EVENT_TYPES).chain(DOCUMENT_EVENT_TYPES) {
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
    fn own_runs_are_told_by_the_payloads_workflow() {
        let p = serde_json::json!({"workflowId": "w1"});
        assert!(from_own_run(RUN_FAILED, &p, "w1"));
        assert!(!from_own_run(RUN_FAILED, &p, "w2"));
        assert!(!from_own_run(RUN_FINISHED, &serde_json::json!({}), "w1"));
        assert!(from_own_run(SESSION_TURN_ENDED, &p, "w1") && from_own_run(INBOX_CREATED, &p, "w1"));
        // Older events carry a workflowId too (an agent outcome is its own run's step): not guarded.
        assert!(!from_own_run(AGENT_OUTCOME, &p, "w1"));
        for id in OWN_RUN_EVENTS {
            let schema = serde_json::to_value((event_type(id).unwrap().schema)()).unwrap();
            assert!(schema["properties"].get("workflowId").is_some(), "{id}");
            assert!(event_type(id).unwrap().description.contains("includeOwn"), "{id}");
        }
    }

    #[test]
    fn a_chain_of_runs_started_by_run_events_counts_up() {
        assert_eq!(run_chain(&serde_json::json!({"type": "ticket.stage_changed"})), 0);
        assert_eq!(run_chain(&serde_json::json!({})), 0);
        assert_eq!(run_chain(&serde_json::json!({"type": RUN_FAILED, "chain": 0})), 1);
        assert_eq!(run_chain(&serde_json::json!({"type": RUN_FINISHED, "chain": 2})), 3);
    }

    #[test]
    fn a_message_tail_keeps_the_end() {
        assert_eq!(message_tail("  short  ", 20), "short");
        let t = message_tail("The first part goes away and the last words stay", 20);
        assert!(t.starts_with('…') && t.ends_with("words stay"), "{t}");
        assert!(t.chars().count() <= 21, "{t}");
        assert_eq!(message_tail("ééééé", 3), "…ééé");
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
            author: "user".into(),
            attachments: vec![],
            attachment_list: String::new(),
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
            ticket_title: "Faster boards".into(),
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["labels"], serde_json::json!(["automerge"]));
        assert_eq!((v["attemptId"].as_str(), v["via"].as_str()), (Some("a"), Some("merged_here")));
        assert_eq!(serde_json::from_value::<AttemptEvent>(v).unwrap(), a);
    }
}
