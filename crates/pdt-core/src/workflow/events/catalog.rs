use super::*;

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
        id: UPDATE_FAILED,
        title: "PDT update failed",
        description: "PDT started again after an update that failed or was rolled back (`rolledBack`), on the version \
                      it ran before; `message` says what went wrong.",
        schema: schema_of::<UpdateFailed>,
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
