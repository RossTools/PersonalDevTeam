//! Tickets (tickets spec §3, §4): the types the store and API share, and the text rules for titles, labels, slugs
//! and keys. A ticket's key is `<project key>-<number>`, e.g. `PDT-42`; numbers are never reused within a project.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_TITLE: usize = 200;
pub const MAX_DESCRIPTION: usize = 100_000;
pub const MAX_LABEL: usize = 30;
pub const MAX_LABELS: usize = 20;
pub const MAX_AREA: usize = 60;
pub const MAX_COMMENT: usize = 20_000;
pub const MAX_SLUG: usize = 40;
/// Page sizes for the ticket list and the timeline.
pub const DEFAULT_LIMIT: u32 = 100;
pub const MAX_LIMIT: u32 = 500;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TicketStage {
    Draft,
    Ready,
    /// Picked to be worked on next: on the board. Ready tickets wait in the backlog.
    Todo,
    InProgress,
    Done,
    WontDo,
}
str_enum!(TicketStage { Draft => "draft", Ready => "ready", Todo => "todo", InProgress => "in_progress", Done => "done", WontDo => "wont_do" });

impl TicketStage {
    /// Done and Won't do are closed: hidden like done, and they no longer block anything.
    pub fn is_closed(self) -> bool {
        matches!(self, TicketStage::Done | TicketStage::WontDo)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}
str_enum!(Priority { None => "none", Low => "low", Medium => "medium", High => "high", Urgent => "urgent" });

impl Priority {
    /// Higher is more urgent (sorting by priority puts the highest first).
    pub fn weight(self) -> u8 {
        match self {
            Priority::None => 0,
            Priority::Low => 1,
            Priority::Medium => 2,
            Priority::High => 3,
            Priority::Urgent => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkKind {
    /// `from` blocks `to`: `to` can't sensibly start until `from` is done.
    Blocks,
    /// Symmetric.
    Relates,
}
str_enum!(LinkKind { Blocks => "blocks", Relates => "relates" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkDirection {
    /// This ticket is the link's `from` (for `blocks`: it blocks the other).
    Outgoing,
    /// This ticket is the link's `to` (for `blocks`: it is blocked by the other).
    Incoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AttemptState {
    Active,
    Merged,
    Abandoned,
}
str_enum!(AttemptState { Active => "active", Merged => "merged", Abandoned => "abandoned" });

/// Ordered: `none < overlap < conflict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OverlapLevel {
    #[default]
    None,
    Overlap,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EventKind {
    Created,
    Edited,
    Moved,
    Linked,
    Unlinked,
    Started,
    Session,
    Merged,
    Abandoned,
    Unblocked,
    Overlap,
    Comment,
    /// "Wait until clear" was set or cleared: `{waiting}`.
    Waiting,
    /// News about the attempt's pull request (CI failed, conflict, left the merge queue, merged) and whether the
    /// agent was told: `{attemptId, notice, key, pr, url, jobs, sessionId, told}`.
    Pr,
    /// A part was added to or removed from a parent (on both tickets): `{parentId, parentKey, partId, partKey,
    /// removed}`.
    Parted,
    /// Review comments were sent to the coder session (review spec §2.5): `{worktreeId, sessionId, count}`.
    Feedback,
    /// The user approved the attempt's changes: `{verdict: "approved", head}`.
    Review,
    /// An agent step's outcome (Phase 7b): `{outcome, summary, persona, personaVersion, by, sessionId, attemptId,
    /// workflowId, workflowName, runId}`.
    Outcome,
}
str_enum!(EventKind {
    Created => "created", Edited => "edited", Moved => "moved", Linked => "linked", Unlinked => "unlinked",
    Started => "started", Session => "session", Merged => "merged", Abandoned => "abandoned",
    Unblocked => "unblocked", Overlap => "overlap", Comment => "comment", Waiting => "waiting", Pr => "pr",
    Parted => "parted", Feedback => "feedback", Review => "review", Outcome => "outcome",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Actor {
    User,
    System,
    /// An agent: through pdt-mcp (the event's data names the session and persona), or the API (a comment it posted
    /// with curl).
    Agent,
}
str_enum!(Actor { User => "user", System => "system", Agent => "agent" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TicketSort {
    /// Backlog order.
    #[default]
    Rank,
    /// Most urgent first, then backlog order.
    Priority,
    /// Most recently updated first.
    Updated,
    /// Newest first.
    Created,
}
str_enum!(TicketSort { Rank => "rank", Priority => "priority", Updated => "updated", Created => "created" });

/// A ticket's attempt (a branch + worktree + sessions working on it; Phase 4b creates them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptSummary {
    pub id: String,
    pub branch: String,
    pub base_branch: String,
    pub worktree_id: Option<String>,
    pub state: AttemptState,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
    /// The branch head when merge detection last looked (so a merged branch deleted afterwards is still recognised).
    pub last_head: Option<String>,
    /// The workspace Start made for this attempt (gone with it when the ticket is finished); None when its tabs went
    /// into an existing workspace or the workspace was deleted.
    pub workspace_id: Option<String>,
    /// Its pull request on GitHub as pdtd last saw it (active attempts of projects on GitHub; kept in memory).
    #[serde(default)]
    pub pr: Option<crate::pr::PullRequestStatus>,
    /// CI of its branch without a pull request (Buildkite builds branches; with a PR it is in `pr.ci`). In memory.
    #[serde(default)]
    pub ci: Option<crate::pr::CiStatus>,
}

/// An issue in another tracker that a ticket is linked to (Phase 7e: Jira).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalLink {
    /// `jira`.
    pub system: String,
    /// `ABC-123`.
    pub key: String,
    pub url: String,
}

/// The ticket a session works on (through its attempt, else the ticket a workflow step started it on), for the sidebar
/// and tab titles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionTicket {
    pub id: String,
    /// `PDT-42`.
    pub key: String,
}

/// A session of a ticket: one of its attempts', or one a workflow step started on it (a Reviewer, QA or Refiner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttemptSession {
    pub id: String,
    pub title: String,
    pub status: crate::session::SessionStatus,
    pub agent: Option<crate::session::AgentKind>,
    /// None for a step's session that isn't the attempt's own (PR notices go to the attempt's).
    pub attempt_id: Option<String>,
    /// The persona it runs, if a workflow step started it.
    pub persona: Option<String>,
    /// A conversation or the agent's terminal (the ticket tab's Conversation shows conversations).
    pub mode: crate::conversation::SessionMode,
}

/// A row of the backlog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketSummary {
    pub id: String,
    pub project_id: String,
    /// `<project key>-<number>`, e.g. `PDT-42`.
    pub key: String,
    pub number: u32,
    pub title: String,
    pub stage: TicketStage,
    pub priority: Priority,
    /// Sorted.
    pub labels: Vec<String>,
    /// Sorted.
    pub areas: Vec<String>,
    /// Fractional index: lexicographic order is backlog order within the project.
    pub rank: String,
    /// Start chose "Wait until clear".
    pub waiting: bool,
    /// Waiting, and nothing is in the way any more: no open blockers and no overlap.
    pub clear_to_start: bool,
    /// How many open tickets block this one.
    pub blocked_by: u32,
    /// The active attempt, else the latest one.
    pub attempt: Option<AttemptSummary>,
    /// The worst planned/actual overlap with the project's tickets that have an active attempt (tickets spec §7).
    pub overlap: OverlapLevel,
    /// The keys of the tickets it overlaps, conflicts first.
    pub overlap_with: Vec<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub closed_at: Option<i64>,
    /// The parent, when this ticket is a part (PDT-38).
    pub parent: Option<crate::parts::TicketParent>,
    /// The roll-up of its parts, when it has any (filled in by the API).
    pub parts: Option<crate::parts::PartsProgress>,
    /// Where a pipeline workflow has it (Phase 7c): `refine` while Draft or Ready, the others while In progress. Any
    /// change of stage clears it.
    #[serde(default)]
    pub pipeline_stage: Option<PipelineStage>,
    /// Who works on it now in a workflow and in what state (filled in by the API, in memory).
    #[serde(default)]
    pub agent: Option<TicketAgent>,
    /// The issues in other trackers it is linked to (a Jira issue it was imported from).
    #[serde(default)]
    pub external: Vec<ExternalLink>,
}

/// A ticket's stage in a pipeline workflow (Phase 7c spec §2): the Board splits In progress by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PipelineStage {
    Refine,
    Code,
    Review,
    Qa,
    Accept,
    /// Merging: the run merges it or waits for the merge (PDT-124).
    Merge,
    /// The merge was refused for something no agent can fix: the Inbox asks the user.
    MergeBlocked,
}
str_enum!(PipelineStage {
    Refine => "refine", Code => "code", Review => "review", Qa => "qa", Accept => "accept", Merge => "merge",
    MergeBlocked => "merge_blocked"
});

/// The state of whoever works on a ticket in a workflow now (Phase 7c spec §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TicketAgentState {
    /// A persona's session works on a step.
    Working,
    /// It asked the user something and waits for the answers.
    Asking,
    /// Its turns ended without an outcome; the Inbox has it.
    Stalled,
    /// The user's turn: a manual step or a gate waits on them.
    Yours,
    /// Waiting for a slot under the host's cap on agents running a turn.
    Queued,
}

/// Who owns a ticket's current workflow step, for the Board's cards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketAgent {
    /// The persona's name; null for a gate.
    pub persona: Option<String>,
    pub state: TicketAgentState,
    pub session_id: Option<String>,
    /// Its session is running a turn now.
    pub busy: bool,
    pub run_id: Option<String>,
}

/// How a ticket overlaps another one that has an active attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketOverlap {
    /// The other ticket.
    pub ticket_id: String,
    pub key: String,
    pub title: String,
    pub stage: TicketStage,
    pub level: OverlapLevel,
    /// Configured areas both tickets list (planned overlap).
    pub areas: Vec<String>,
    /// Files both active attempts changed (actual overlap), sorted; at most [`MAX_OVERLAP_FILES`].
    pub files: Vec<String>,
    /// How many files they share in all.
    pub files_total: u32,
}

/// The most shared files a [`TicketOverlap`] lists.
pub const MAX_OVERLAP_FILES: usize = 50;

/// The other end of a link, as seen from one ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkedTicket {
    pub kind: LinkKind,
    pub direction: LinkDirection,
    /// The other ticket.
    pub id: String,
    pub key: String,
    pub title: String,
    pub stage: TicketStage,
}

/// A full ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Ticket {
    #[serde(flatten)]
    pub summary: TicketSummary,
    /// Markdown.
    pub description: String,
    /// Ordered by kind, direction, then the other ticket's number.
    pub links: Vec<LinkedTicket>,
    /// Newest first.
    pub attempts: Vec<AttemptSummary>,
    /// The branch Start uses unless told otherwise: `pdt/<KEY>-<n>-<slug of the title>`.
    pub default_branch: String,
    /// The sessions of all its attempts, newest first.
    pub sessions: Vec<AttemptSession>,
    /// Its overlaps with the project's tickets that have an active attempt, conflicts first.
    pub overlaps: Vec<TicketOverlap>,
    /// Its parts in rank order (empty when it has none).
    pub part_list: Vec<TicketSummary>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewTicket {
    pub title: String,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    /// Draft when absent.
    #[serde(default)]
    #[ts(optional)]
    pub stage: Option<TicketStage>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<Priority>,
    #[serde(default)]
    #[ts(optional)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub areas: Option<Vec<String>>,
    /// Place it directly above this ticket (and below `after`, if given). Without either, it goes to the top of its
    /// stage's list.
    #[serde(default)]
    #[ts(optional)]
    pub before: Option<String>,
    /// Place it directly below this ticket.
    #[serde(default)]
    #[ts(optional)]
    pub after: Option<String>,
    /// Make it a part of this ticket (id). Without `before`/`after` it goes after the parent's last part.
    #[serde(default)]
    #[ts(optional)]
    pub parent: Option<String>,
    /// With `parent`: the parent's last part (by rank) blocks the new one.
    #[serde(default)]
    #[ts(optional)]
    pub blocked_by_previous: Option<bool>,
}

/// Edits a ticket; absent fields are left alone. `labels` and `areas` replace the whole set.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketPatch {
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub stage: Option<TicketStage>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<Priority>,
    #[serde(default)]
    #[ts(optional)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub areas: Option<Vec<String>>,
    /// Wait until clear (true; it also goes to the top of its stage) or stop waiting.
    #[serde(default)]
    #[ts(optional)]
    pub waiting: Option<bool>,
    /// Make it a part of this ticket (id), or `""` to detach it from its parent.
    #[serde(default)]
    #[ts(optional)]
    pub parent: Option<String>,
}

/// A patch with its text checked and normalised.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub stage: Option<TicketStage>,
    pub priority: Option<Priority>,
    pub labels: Option<Vec<String>>,
    pub areas: Option<Vec<String>>,
    pub waiting: Option<bool>,
    /// Some(None) detaches.
    pub parent: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RankEdge {
    Top,
    Bottom,
}

/// Moves a ticket in the backlog: directly above `before`, directly below `after`, between the two, or to the top or
/// bottom of its stage's list (`to`).
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RankMove {
    #[serde(default)]
    #[ts(optional)]
    pub before: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub after: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub to: Option<RankEdge>,
}

/// Adds or removes a link from this ticket to `to`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketLink {
    pub to: String,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewComment {
    pub body: String,
    /// Who wrote it: `user` from PDT's own UI. Left out (any other API caller: an agent's curl) it is `agent`, so an
    /// agent's comment never passes for the user's (workflows react to the user's comments).
    #[serde(default)]
    pub by: Option<Actor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketEvent {
    pub id: String,
    pub ticket_id: String,
    pub kind: EventKind,
    pub actor: Actor,
    /// Comment text, or a short description.
    pub body: Option<String>,
    /// Details: `moved` {from, to}; `edited` {fields}; `linked`/`unlinked` {kind, direction, ticketId, key}.
    #[ts(type = "unknown")]
    pub data: Option<serde_json::Value>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventPage {
    pub items: Vec<TicketEvent>,
    /// Pass back as `cursor` for the next page; null at the end.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventQuery {
    #[serde(default)]
    #[ts(optional)]
    pub cursor: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub limit: Option<u32>,
    /// `true`: newest first. Oldest first by default.
    #[serde(default)]
    #[ts(optional)]
    pub newest_first: Option<bool>,
}

/// `GET /api/tickets` query. List filters are comma-separated (`stage=draft,ready`); labels must all match.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketQuery {
    /// One project's tickets; every project's when absent.
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    /// Words (prefix-matched in titles and descriptions), or a key (`PDT-12`, `pdt-12`, `12`).
    #[serde(default)]
    #[ts(optional)]
    pub q: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub stage: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
    /// true: only blocked tickets; false: only unblocked ones.
    #[serde(default)]
    #[ts(optional)]
    pub blocked: Option<bool>,
    /// true: only tickets with an overlap or conflict; false: only those with none.
    #[serde(default)]
    #[ts(optional)]
    pub conflicts: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub sort: Option<TicketSort>,
    /// Closed tickets only if they were closed at or after this time (ms); open tickets are unaffected.
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub closed_after: Option<i64>,
    /// One ticket's parts (its id), or `none` for tickets that are not parts.
    #[serde(default)]
    #[ts(optional)]
    pub parent: Option<String>,
    /// true: only tickets with parts; false: only tickets without.
    #[serde(default)]
    #[ts(optional)]
    pub has_parts: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub cursor: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// A checked [`TicketQuery`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketFilter {
    pub project: Option<String>,
    /// Trimmed, non-empty.
    pub q: Option<String>,
    pub stages: Vec<TicketStage>,
    pub priorities: Vec<Priority>,
    pub labels: Vec<String>,
    pub blocked: Option<bool>,
    pub conflicts: Option<bool>,
    /// Only these tickets (the API turns `conflicts` into this, since overlap is computed outside the store).
    pub only_ids: Option<Vec<String>>,
    /// Never these tickets.
    pub except_ids: Vec<String>,
    pub sort: TicketSort,
    pub closed_after: Option<i64>,
    pub parent: Option<ParentFilter>,
    pub has_parts: Option<bool>,
    pub cursor: Option<String>,
    pub limit: u32,
}

/// `parent=` in a ticket query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentFilter {
    /// The parts of this ticket.
    Of(String),
    /// Tickets that are not parts.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketPage {
    pub items: Vec<TicketSummary>,
    /// Pass back as `cursor` for the next page; null at the end.
    pub next_cursor: Option<String>,
    /// How many tickets match the filters (all pages).
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BulkAction {
    /// `value`: the stage.
    Stage,
    /// `value`: the priority.
    Priority,
    /// `value`: the label.
    AddLabel,
    /// `value`: the label.
    RemoveLabel,
    /// To the top of each ticket's stage, keeping their order.
    Top,
    Delete,
}

/// Applies one action to many tickets: all of them or none.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BulkRequest {
    pub ids: Vec<String>,
    pub action: BulkAction,
    #[serde(default)]
    #[ts(optional)]
    pub value: Option<String>,
}

/// A checked bulk action.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidBulk {
    Stage(TicketStage),
    Priority(Priority),
    AddLabel(String),
    RemoveLabel(String),
    Top,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BulkResult {
    /// Tickets changed (or deleted).
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TicketValidationError {
    #[error("a ticket title must not be empty")]
    EmptyTitle,
    #[error("a ticket title must be at most {MAX_TITLE} characters")]
    TitleTooLong,
    #[error("a ticket description must be at most {MAX_DESCRIPTION} characters")]
    DescriptionTooLong,
    #[error("a label is 1–{MAX_LABEL} characters with no commas (got {0:?})")]
    BadLabel(String),
    #[error("a ticket has at most {MAX_LABELS} labels")]
    TooManyLabels,
    #[error("an area name is 1–{MAX_AREA} characters with no commas (got {0:?})")]
    BadArea(String),
    #[error("a comment must be 1–{MAX_COMMENT} characters")]
    BadComment,
    #[error("unknown {0}: {1:?}")]
    Unknown(&'static str, String),
    #[error("{0}")]
    Invalid(String),
}

type VResult<T> = Result<T, TicketValidationError>;

/// A trimmed title of 1–200 characters (line breaks become spaces).
pub fn valid_title(title: &str) -> VResult<String> {
    let t: String = title.trim().chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    if t.is_empty() {
        return Err(TicketValidationError::EmptyTitle);
    }
    if t.chars().count() > MAX_TITLE {
        return Err(TicketValidationError::TitleTooLong);
    }
    Ok(t)
}

pub fn valid_description(d: &str) -> VResult<String> {
    if d.chars().count() > MAX_DESCRIPTION {
        return Err(TicketValidationError::DescriptionTooLong);
    }
    Ok(d.trim_end().to_string())
}

/// Trimmed, lower-cased, inner whitespace collapsed; 1–30 characters, no commas or control characters.
pub fn normalize_label(label: &str) -> VResult<String> {
    let l = label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let n = l.chars().count();
    if n == 0 || n > MAX_LABEL || l.contains(',') || l.chars().any(char::is_control) {
        return Err(TicketValidationError::BadLabel(label.to_string()));
    }
    Ok(l)
}

/// Normalised, de-duplicated and sorted; at most 20.
pub fn normalize_labels(labels: &[String]) -> VResult<Vec<String>> {
    let mut out = labels.iter().map(|l| normalize_label(l)).collect::<VResult<Vec<_>>>()?;
    out.sort();
    out.dedup();
    if out.len() > MAX_LABELS {
        return Err(TicketValidationError::TooManyLabels);
    }
    Ok(out)
}

/// Area names (matched against `.pdt/project.toml` in Phase 4c): trimmed, 1–60 characters, no commas; kept as
/// written, de-duplicated and sorted.
pub fn normalize_areas(areas: &[String]) -> VResult<Vec<String>> {
    let mut out = Vec::with_capacity(areas.len());
    for a in areas {
        let t = a.trim();
        let n = t.chars().count();
        if n == 0 || n > MAX_AREA || t.contains(',') || t.chars().any(char::is_control) {
            return Err(TicketValidationError::BadArea(a.clone()));
        }
        out.push(t.to_string());
    }
    out.sort();
    out.dedup();
    if out.len() > MAX_LABELS {
        return Err(TicketValidationError::Invalid(format!("a ticket has at most {MAX_LABELS} areas")));
    }
    Ok(out)
}

pub fn valid_comment(body: &str) -> VResult<String> {
    let b = body.trim();
    if b.is_empty() || b.chars().count() > MAX_COMMENT {
        return Err(TicketValidationError::BadComment);
    }
    Ok(b.to_string())
}

/// For branch names (Phase 4b): lower-case ASCII letters and digits joined by single dashes, at most 40 characters,
/// cut at a word boundary where possible. Accented letters lose their accents where the letter is plain ASCII
/// underneath; anything else separates words. May be empty.
pub fn slug(title: &str) -> String {
    let mut words: Vec<String> = vec![];
    let mut cur = String::new();
    for c in title.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let mut out = String::new();
    for w in words {
        let extra = if out.is_empty() { w.len() } else { w.len() + 1 };
        if out.len() + extra > MAX_SLUG {
            if out.is_empty() {
                out = w[..MAX_SLUG].to_string();
            }
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w);
    }
    out
}

fn fold_accent(c: char) -> Vec<char> {
    let base = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'ç' => 'c',
        'Ç' => 'C',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'O',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'ý' | 'ÿ' => 'y',
        'Ý' => 'Y',
        'ß' => return vec!['s', 's'],
        other => other,
    };
    vec![base]
}

/// A ticket reference typed by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRef {
    /// Upper-cased project key; None for a bare number.
    pub key: Option<String>,
    pub number: u32,
}

/// `PDT-12`, `pdt-12`, `12` or `#12` (trimmed). None for anything else.
pub fn parse_key(s: &str) -> Option<KeyRef> {
    let s = s.trim();
    let num = |n: &str| -> Option<u32> {
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        n.parse().ok().filter(|&n| n > 0)
    };
    if let Some(n) = num(s.strip_prefix('#').unwrap_or(s)) {
        return Some(KeyRef { key: None, number: n });
    }
    let (key, n) = s.rsplit_once('-')?;
    let key = crate::project::valid_key(key).ok()?;
    Some(KeyRef { key: Some(key), number: num(n)? })
}

pub fn format_key(project_key: &str, number: u32) -> String {
    format!("{project_key}-{number}")
}

fn split_list(s: Option<&str>) -> impl Iterator<Item = &str> {
    s.unwrap_or("").split(',').map(str::trim).filter(|x| !x.is_empty())
}

impl TicketQuery {
    pub fn validate(&self) -> VResult<TicketFilter> {
        let stages = split_list(self.stage.as_deref())
            .map(|s| TicketStage::parse(s).ok_or_else(|| TicketValidationError::Unknown("stage", s.into())))
            .collect::<VResult<Vec<_>>>()?;
        let priorities = split_list(self.priority.as_deref())
            .map(|s| Priority::parse(s).ok_or_else(|| TicketValidationError::Unknown("priority", s.into())))
            .collect::<VResult<Vec<_>>>()?;
        let labels = split_list(self.label.as_deref()).map(normalize_label).collect::<VResult<Vec<_>>>()?;
        let limit = match self.limit {
            None => DEFAULT_LIMIT,
            Some(0) => return Err(TicketValidationError::Invalid("limit must be at least 1".into())),
            Some(n) => n.min(MAX_LIMIT),
        };
        let nonblank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        Ok(TicketFilter {
            project: nonblank(&self.project),
            q: nonblank(&self.q),
            stages,
            priorities,
            labels,
            blocked: self.blocked,
            conflicts: self.conflicts,
            only_ids: None,
            except_ids: vec![],
            sort: self.sort.unwrap_or_default(),
            closed_after: self.closed_after,
            parent: nonblank(&self.parent).map(|p| if p == "none" { ParentFilter::None } else { ParentFilter::Of(p) }),
            has_parts: self.has_parts,
            cursor: nonblank(&self.cursor),
            limit,
        })
    }
}

/// A checked [`NewTicket`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidNewTicket {
    pub title: String,
    pub description: String,
    pub stage: TicketStage,
    pub priority: Priority,
    pub labels: Vec<String>,
    pub areas: Vec<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub parent: Option<String>,
    pub blocked_by_previous: bool,
}

impl NewTicket {
    pub fn validate(&self) -> VResult<ValidNewTicket> {
        let nonblank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        Ok(ValidNewTicket {
            title: valid_title(&self.title)?,
            description: valid_description(self.description.as_deref().unwrap_or(""))?,
            stage: self.stage.unwrap_or(TicketStage::Draft),
            priority: self.priority.unwrap_or(Priority::None),
            labels: normalize_labels(self.labels.as_deref().unwrap_or(&[]))?,
            areas: normalize_areas(self.areas.as_deref().unwrap_or(&[]))?,
            before: nonblank(&self.before),
            after: nonblank(&self.after),
            parent: nonblank(&self.parent),
            blocked_by_previous: self.blocked_by_previous.unwrap_or(false),
        })
    }
}

impl TicketPatch {
    pub fn validate(&self) -> VResult<ValidPatch> {
        Ok(ValidPatch {
            title: self.title.as_deref().map(valid_title).transpose()?,
            description: self.description.as_deref().map(valid_description).transpose()?,
            stage: self.stage,
            priority: self.priority,
            labels: self.labels.as_deref().map(normalize_labels).transpose()?,
            areas: self.areas.as_deref().map(normalize_areas).transpose()?,
            waiting: self.waiting,
            parent: self.parent.as_deref().map(|p| Some(p.trim()).filter(|s| !s.is_empty()).map(String::from)),
        })
    }
}

impl BulkRequest {
    pub fn validate(&self) -> VResult<ValidBulk> {
        if self.ids.is_empty() {
            return Err(TicketValidationError::Invalid("no tickets given".into()));
        }
        if self.ids.len() > 10_000 {
            return Err(TicketValidationError::Invalid("at most 10000 tickets at once".into()));
        }
        let value = || {
            self.value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| TicketValidationError::Invalid("this action needs a value".into()))
        };
        Ok(match self.action {
            BulkAction::Stage => {
                let v = value()?;
                ValidBulk::Stage(
                    TicketStage::parse(v).ok_or_else(|| TicketValidationError::Unknown("stage", v.into()))?,
                )
            }
            BulkAction::Priority => {
                let v = value()?;
                ValidBulk::Priority(
                    Priority::parse(v).ok_or_else(|| TicketValidationError::Unknown("priority", v.into()))?,
                )
            }
            BulkAction::AddLabel => ValidBulk::AddLabel(normalize_label(value()?)?),
            BulkAction::RemoveLabel => ValidBulk::RemoveLabel(normalize_label(value()?)?),
            BulkAction::Top => ValidBulk::Top,
            BulkAction::Delete => ValidBulk::Delete,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_round_trip_and_know_when_they_are_closed() {
        for s in TicketStage::ALL {
            assert_eq!(TicketStage::parse(s.as_str()), Some(*s));
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.as_str()));
        }
        assert!(TicketStage::Done.is_closed() && TicketStage::WontDo.is_closed());
        assert!(!TicketStage::Draft.is_closed() && !TicketStage::Ready.is_closed());
        assert!(!TicketStage::Todo.is_closed() && !TicketStage::InProgress.is_closed());
        assert_eq!(TicketStage::parse("todo"), Some(TicketStage::Todo));
        assert_eq!(TicketStage::parse("wont_do"), Some(TicketStage::WontDo));
        assert_eq!(TicketStage::parse("Done"), None);
        for e in EventKind::ALL {
            assert_eq!(serde_json::to_value(e).unwrap(), serde_json::json!(e.as_str()));
        }
    }

    #[test]
    fn priorities_order_by_weight() {
        let mut p = Priority::ALL.to_vec();
        p.sort_by_key(|p| std::cmp::Reverse(p.weight()));
        assert_eq!(p, [Priority::Urgent, Priority::High, Priority::Medium, Priority::Low, Priority::None]);
    }

    #[test]
    fn titles_are_trimmed_single_line_and_bounded() {
        assert_eq!(valid_title("  Fix it \n"), Ok("Fix it".into()));
        assert_eq!(valid_title("a\nb"), Ok("a b".into()));
        assert_eq!(valid_title(" \n "), Err(TicketValidationError::EmptyTitle));
        assert_eq!(valid_title(&"é".repeat(200)), Ok("é".repeat(200)));
        assert_eq!(valid_title(&"x".repeat(201)), Err(TicketValidationError::TitleTooLong));
    }

    #[test]
    fn labels_are_normalised_deduplicated_and_bounded() {
        assert_eq!(normalize_label("  Bug  Fix "), Ok("bug fix".into()));
        assert_eq!(normalize_label("UI"), Ok("ui".into()));
        for bad in ["", "   ", "a,b", &"x".repeat(31), "a\u{7}"] {
            assert!(normalize_label(bad).is_err(), "{bad:?}");
        }
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(normalize_labels(&v(&["UI", "bug", "ui "])), Ok(v(&["bug", "ui"])));
        let many: Vec<String> = (0..21).map(|i| format!("l{i}")).collect();
        assert_eq!(normalize_labels(&many), Err(TicketValidationError::TooManyLabels));
        assert_eq!(normalize_areas(&v(&[" migrations", "web", "web"])), Ok(v(&["migrations", "web"])));
        assert!(normalize_areas(&v(&[""])).is_err());
    }

    #[test]
    fn slugs_are_ascii_dashed_and_short() {
        assert_eq!(slug("Fix the Login bug!"), "fix-the-login-bug");
        assert_eq!(slug("  Café crème — v2.0 "), "cafe-creme-v2-0");
        assert_eq!(slug("日本語"), "");
        assert_eq!(slug("straße"), "strasse");
        let long = slug("a very long ticket title that keeps going well past forty characters");
        assert!(long.len() <= 40 && !long.ends_with('-'), "{long}");
        assert_eq!(long, "a-very-long-ticket-title-that-keeps");
        assert_eq!(slug(&"x".repeat(50)), "x".repeat(40));
    }

    #[test]
    fn keys_parse_in_the_forms_people_type() {
        let r = |k: Option<&str>, n| Some(KeyRef { key: k.map(Into::into), number: n });
        assert_eq!(parse_key("PDT-12"), r(Some("PDT"), 12));
        assert_eq!(parse_key(" pdt-12 "), r(Some("PDT"), 12));
        assert_eq!(parse_key("12"), r(None, 12));
        assert_eq!(parse_key("#7"), r(None, 7));
        assert_eq!(parse_key("PDT2-3"), r(Some("PDT2"), 3));
        for bad in ["", "PDT", "PDT-", "PDT-x", "-12", "0", "PDT-0", "fix login", "P-1", "12a", "1-2"] {
            assert_eq!(parse_key(bad), None, "{bad:?}");
        }
        assert_eq!(format_key("PDT", 42), "PDT-42");
    }

    #[test]
    fn queries_parse_their_lists_and_bound_the_limit() {
        let q = TicketQuery {
            stage: Some("draft, ready".into()),
            priority: Some("high".into()),
            label: Some("UI,bug".into()),
            q: Some("  ".into()),
            limit: Some(10_000),
            ..Default::default()
        };
        let f = q.validate().unwrap();
        assert_eq!(f.stages, [TicketStage::Draft, TicketStage::Ready]);
        assert_eq!(f.priorities, [Priority::High]);
        assert_eq!(f.labels, ["ui", "bug"]);
        assert_eq!((f.q, f.limit, f.sort), (None, MAX_LIMIT, TicketSort::Rank));
        assert_eq!(TicketQuery::default().validate().unwrap().limit, DEFAULT_LIMIT);
        let bad = TicketQuery { stage: Some("doing".into()), ..Default::default() };
        assert!(matches!(bad.validate(), Err(TicketValidationError::Unknown("stage", _))));
        assert!(TicketQuery { limit: Some(0), ..Default::default() }.validate().is_err());
    }

    #[test]
    fn a_patch_tells_absent_blank_and_a_parent_apart() {
        let p: TicketPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(p.validate().unwrap().parent, None);
        let p: TicketPatch = serde_json::from_str(r#"{"parent":" "}"#).unwrap();
        assert_eq!(p.validate().unwrap().parent, Some(None), "blank detaches");
        let p: TicketPatch = serde_json::from_str(r#"{"parent":" t1 "}"#).unwrap();
        assert_eq!(p.validate().unwrap().parent, Some(Some("t1".into())));
        let q = TicketQuery { parent: Some("none".into()), has_parts: Some(true), ..Default::default() };
        let f = q.validate().unwrap();
        assert_eq!((f.parent, f.has_parts), (Some(ParentFilter::None), Some(true)));
        let q = TicketQuery { parent: Some("t1".into()), ..Default::default() };
        assert_eq!(q.validate().unwrap().parent, Some(ParentFilter::Of("t1".into())));
        let n: NewTicket = serde_json::from_str(r#"{"title":"x","parent":"p","blockedByPrevious":true}"#).unwrap();
        let v = n.validate().unwrap();
        assert_eq!((v.parent.as_deref(), v.blocked_by_previous), (Some("p"), true));
    }

    #[test]
    fn bulk_actions_need_valid_values() {
        let b =
            |action, value: Option<&str>| BulkRequest { ids: vec!["a".into()], action, value: value.map(Into::into) };
        assert_eq!(b(BulkAction::Stage, Some("ready")).validate(), Ok(ValidBulk::Stage(TicketStage::Ready)));
        assert_eq!(b(BulkAction::AddLabel, Some(" UI ")).validate(), Ok(ValidBulk::AddLabel("ui".into())));
        assert_eq!(b(BulkAction::Top, None).validate(), Ok(ValidBulk::Top));
        assert!(b(BulkAction::Stage, None).validate().is_err());
        assert!(b(BulkAction::Priority, Some("huge")).validate().is_err());
        assert!(BulkRequest { ids: vec![], action: BulkAction::Delete, value: None }.validate().is_err());
    }

    #[test]
    fn a_full_ticket_serialises_flat() {
        let s = TicketSummary {
            id: "t".into(),
            project_id: "p".into(),
            key: "PDT-1".into(),
            number: 1,
            title: "T".into(),
            stage: TicketStage::InProgress,
            priority: Priority::None,
            labels: vec![],
            areas: vec![],
            rank: "V".into(),
            waiting: false,
            clear_to_start: false,
            blocked_by: 0,
            attempt: None,
            overlap: OverlapLevel::None,
            overlap_with: vec![],
            created_at: 1,
            updated_at: 2,
            closed_at: None,
            parent: None,
            parts: None,
            pipeline_stage: None,
            agent: None,
            external: vec![],
        };
        let t = Ticket {
            summary: s,
            description: "d".into(),
            links: vec![],
            attempts: vec![],
            default_branch: "pdt/PDT-1-t".into(),
            sessions: vec![],
            overlaps: vec![],
            part_list: vec![],
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["key"], "PDT-1");
        assert_eq!(v["stage"], "in_progress");
        assert_eq!(v["blockedBy"], 0);
        assert_eq!(v["description"], "d");
        assert_eq!(v["defaultBranch"], "pdt/PDT-1-t");
        assert_eq!(v["sessions"], serde_json::json!([]));
    }
}
