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
    /// Its name as the board shows it (`Won't do`).
    pub fn label(self) -> &'static str {
        match self {
            TicketStage::Draft => "Draft",
            TicketStage::Ready => "Ready",
            TicketStage::Todo => "To do",
            TicketStage::InProgress => "In progress",
            TicketStage::Done => "Done",
            TicketStage::WontDo => "Won't do",
        }
    }

    /// Done and Won't do are closed: hidden like done, and they no longer block anything.
    pub fn is_closed(self) -> bool {
        matches!(self, TicketStage::Done | TicketStage::WontDo)
    }

    /// Draft, Ready and To do: the stages before work starts. Moving a ticket with an active attempt to one stops its
    /// work (PDT-159).
    pub fn is_before_work(self) -> bool {
        matches!(self, TicketStage::Draft | TicketStage::Ready | TicketStage::Todo)
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

/// Where a ticket came from (PDT-201). An `external` one was made from outside input (an imported issue, issue or bug
/// triage, an issue tracker integration), which may carry prompt injection: it reaches To do only when the user puts it
/// there (see [`external_gate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TicketSource {
    /// Made by the user (and every ticket made before sources were kept).
    #[default]
    User,
    /// Made by the PM in a conversation.
    Pm,
    /// Made by a workflow.
    Workflow,
    /// Made from outside input.
    External,
    /// Made by an agent session other than the PM (through pdt-mcp or the HTTP API; PDT-363): its `created` event
    /// names the session, its persona and the ticket it works on.
    Agent,
}
str_enum!(TicketSource { User => "user", Pm => "pm", Workflow => "workflow", External => "external", Agent => "agent" });

/// The label that marks a ticket for agents to work on unattended: adding it to an external ticket is gated like To
/// do (PDT-201).
pub const AGENTIC_LABEL: &str = "agentic";

/// Whether a change puts a ticket in front of agents (PDT-201): to To do or In progress from a stage before it, or
/// adding the [`AGENTIC_LABEL`].
pub fn boards(
    from: Option<TicketStage>,
    to: Option<TicketStage>,
    labels: &[String],
    new_labels: Option<&[String]>,
) -> bool {
    let on_board = |s: TicketStage| matches!(s, TicketStage::Todo | TicketStage::InProgress);
    let moves = to.is_some_and(|to| on_board(to) && from.is_none_or(|f| f != to && !on_board(f)));
    let has = |l: &[String]| l.iter().any(|x| x == AGENTIC_LABEL);
    moves || new_labels.is_some_and(|n| has(n) && !has(labels))
}

/// The Todo gate (PDT-201): an external ticket the user hasn't let in yet reaches To do (or In progress, or gets the
/// `agentic` label) only when the user does it, or the PM through a call the user confirmed. `Ok(true)` when this
/// change lets it in, `Ok(false)` when the gate has nothing to say, Err why it is refused (for the agent or workflow).
pub fn external_gate(source: TicketSource, admitted: bool, boards: bool, by: &By, key: &str) -> Result<bool, String> {
    if source != TicketSource::External || admitted || !boards {
        return Ok(false);
    }
    if by.actor == Actor::User || by.confirmed {
        return Ok(true);
    }
    Err(format!(
        "{key} came from outside PDT (an issue or a report), so only the user can put it in To do, start it or label          it {AGENTIC_LABEL}: they do it on the board, or confirm the PM's edit_ticket. Leave it where it is and tell          the user why it should go ahead."
    ))
}

/// Who made a ticket, for `ticket.created` (PDT-363): `external` or `pm` by its source, else `agent` for an agent
/// session, `workflow` for PDT itself (a workflow node, a bug report), else `user`.
pub fn creator(source: TicketSource, by: &By) -> &'static str {
    match (source, by.actor) {
        (TicketSource::External, _) => "external",
        (TicketSource::Pm, _) => "pm",
        (TicketSource::Agent, _) | (_, Actor::Agent) => "agent",
        (TicketSource::Workflow, _) | (_, Actor::System) => "workflow",
        (TicketSource::User, Actor::User) => "user",
    }
}

/// The labels added and removed going from `old` to `new` (each sorted as given).
pub fn label_change(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let added = new.iter().filter(|l| !old.contains(l)).cloned().collect();
    let removed = old.iter().filter(|l| !new.contains(l)).cloned().collect();
    (added, removed)
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
    /// Outside input reached an agent session, or was held back from one (PDT-202): `{source, url, sessionId, via,
    /// runId, workflowName}`; a comment held back: `{held: true, source, url, author, pr}` with its text as the body.
    OutsideInput,
    /// The branch was brought up to date with its base, or a conflict with it left for merge time (PDT-328):
    /// `{attemptId, how, base, files, regenerated, pr}` (`how`: [`crate::merge_update::MergeUpdateHow`]).
    MergeUpdate,
}
str_enum!(EventKind {
    Created => "created", Edited => "edited", Moved => "moved", Linked => "linked", Unlinked => "unlinked",
    Started => "started", Session => "session", Merged => "merged", Abandoned => "abandoned",
    Unblocked => "unblocked", Overlap => "overlap", Comment => "comment", Waiting => "waiting", Pr => "pr",
    Parted => "parted", Feedback => "feedback", Review => "review", Outcome => "outcome",
    OutsideInput => "outside_input", MergeUpdate => "merge_update",
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

/// The agent session behind a ticket change or comment made through pdt-mcp (PDT-163). Events keep it in their data as
/// `agent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentBy {
    pub session_id: String,
    /// The session's project (its tab opens there).
    #[serde(default)]
    pub project_id: Option<String>,
    /// The persona it runs (Coder, Reviewer, QA, PM), if any.
    #[serde(default)]
    pub persona: Option<String>,
    /// The ticket the session works on, if any: its key (`PDT-34`) and id.
    #[serde(default)]
    pub from_ticket: Option<String>,
    #[serde(default)]
    pub from_ticket_id: Option<String>,
}

impl AgentBy {
    /// Who made an event, as its data says: `agent`, or the `sessionId` and `persona` an agent's comment carried
    /// before PDT-163. None when nothing is known.
    pub fn of_event(kind: EventKind, actor: Actor, data: Option<&serde_json::Value>) -> Option<AgentBy> {
        let data = data?;
        if let Some(a) = data.get("agent").and_then(|a| serde_json::from_value(a.clone()).ok()) {
            return Some(a);
        }
        if kind != EventKind::Comment || actor != Actor::Agent {
            return None;
        }
        let session_id = data.get("sessionId")?.as_str()?.to_string();
        let persona = data.get("persona").and_then(|p| p.as_str()).map(String::from);
        Some(AgentBy { session_id, project_id: None, persona, from_ticket: None, from_ticket_id: None })
    }
}

/// Who changes a ticket: the actor and, for an agent through pdt-mcp, its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct By {
    pub actor: Actor,
    pub agent: Option<AgentBy>,
    /// The user confirmed this very call (the PM's `edit_ticket`, PDT-201): it may let an external ticket in.
    pub confirmed: bool,
}

impl By {
    pub fn agent(a: AgentBy) -> By {
        By { actor: Actor::Agent, agent: Some(a), confirmed: false }
    }

    /// `data` with the agent added as `agent` (an object, or a new one when there is no data).
    pub fn stamp(&self, data: Option<serde_json::Value>) -> Option<serde_json::Value> {
        let Some(a) = &self.agent else { return data };
        let mut data = match data {
            Some(serde_json::Value::Object(m)) => m,
            None | Some(serde_json::Value::Null) => serde_json::Map::new(),
            // Not an object: left as it is.
            Some(other) => return Some(other),
        };
        data.insert("agent".into(), serde_json::to_value(a).unwrap_or_default());
        Some(serde_json::Value::Object(data))
    }
}

impl From<Actor> for By {
    fn from(actor: Actor) -> By {
        By { actor, agent: None, confirmed: false }
    }
}

impl From<&By> for By {
    fn from(by: &By) -> By {
        by.clone()
    }
}

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
    /// CI of its branch without a pull request (a CI integration's builds of the branch; with a PR it is in `pr.ci`). In memory.
    #[serde(default)]
    pub ci: Option<crate::pr::CiStatus>,
    /// Where it is with its base's conflicts (PDT-328): deferred to merge time, being updated, or with an agent. In memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub merge_update: Option<crate::merge_update::MergeUpdateStatus>,
}

/// An issue in another tracker that a ticket is linked to (Phase 7e: an issue tracker integration's, or GitHub's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalLink {
    /// The tracker: an integration's id, or `github`.
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
    /// When auto-start takes it, while it waits out the project's start delay (PDT-146; filled in by the API, in
    /// memory).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub auto_start_at: Option<i64>,
    /// Why "Pick up tickets" passed it over last time for overlapping work in flight ("overlaps PDT-4 in api";
    /// PDT-329; filled in by the API, in memory).
    #[serde(default)]
    pub pickup_hold: Option<String>,
    /// Why pickup is paused, on a ticket auto-start would take but for the pause ("the disk is almost full"; PDT-370;
    /// filled in by the API).
    #[serde(default)]
    pub pickup_paused: Option<String>,
    /// Why "Pick up tickets" passes it over until it changes: its start failed with this error (PDT-381; filled in by
    /// the API, in memory). Try again (`POST /api/tickets/{id}/retry-auto-start`) clears it.
    #[serde(default)]
    pub pickup_failed: Option<String>,
    /// The issues in other trackers it is linked to (an issue it was imported from).
    #[serde(default)]
    pub external: Vec<ExternalLink>,
    /// Where it came from (PDT-201).
    #[serde(default)]
    pub source: TicketSource,
    /// An external ticket the user let in (moved to To do, started or labelled `agentic`): the Todo gate is open.
    #[serde(default)]
    pub admitted: bool,
    /// The agent session that made it, from its `created` event (PDT-363): "Created by Coder · from PDT-345".
    #[serde(default)]
    pub created_by: Option<AgentBy>,
    /// A ticket proposal waiting for the user in the Inbox (PDT-366): held in Draft, off the board and lists.
    #[serde(default)]
    pub proposed: bool,
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
    /// Its turn ended while background tasks it started run (PDT-142): it goes on when they finish.
    Background,
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
    /// What it waits on (`background`): the background tasks' descriptions.
    #[serde(default)]
    pub background: Vec<String>,
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
    /// The agent session that made it, when known (PDT-163; from `data`).
    #[serde(default)]
    pub agent: Option<AgentBy>,
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
    /// true: only ticket proposals waiting in the Inbox (PDT-366). Left out (or false), lists leave them out.
    #[serde(default)]
    #[ts(optional)]
    pub proposed: Option<bool>,
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
    /// Only ticket proposals; otherwise none (PDT-366).
    pub proposed: bool,
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

/// An agent's areas for a ticket, checked against the project's configured ones (PDT-329): every one must be among
/// them, so a typo can't hide a ticket from overlap. A project without areas takes none.
pub fn agent_areas(areas: &[String], configured: &[String]) -> Result<Vec<String>, String> {
    let areas = normalize_areas(areas).map_err(|e| e.to_string())?;
    let unknown: Vec<&str> = areas.iter().filter(|a| !configured.contains(a)).map(String::as_str).collect();
    match (unknown.is_empty(), configured.is_empty()) {
        (true, _) => Ok(areas),
        (false, true) => Err("the project has no areas (`[[area]]` in its .pdt/project.toml): leave areas out".into()),
        (false, false) => {
            Err(format!("not areas of the project: {}. Its areas are: {}", unknown.join(", "), configured.join(", ")))
        }
    }
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

/// What a non-PM agent session (a Coder, Reviewer, QA…) asks of `edit_ticket` on a ticket (PDT-162).
#[derive(Debug, Clone)]
pub struct AgentEdit<'a> {
    /// The ticket as the agent named it.
    pub given: &'a str,
    /// The ticket's key (`PDT-12`).
    pub key: &'a str,
    pub stage: TicketStage,
    /// It is the session's own ticket.
    pub own: bool,
    /// The session created it.
    pub created: bool,
    /// The edit changes its title or replaces its description.
    pub rewrites: bool,
}

/// Whether a non-PM agent may make an edit: its own ticket freely; another only named by its exact key, and never
/// rewriting the title or description of one in progress or closed that it didn't create. The error says why and
/// points at `add_ticket_comment`.
pub fn agent_may_edit(e: &AgentEdit) -> Result<(), String> {
    if e.own {
        return Ok(());
    }
    let exact = parse_key(e.given).and_then(|k| k.key.map(|key| format_key(&key, k.number)));
    if exact.as_deref() != Some(e.key) {
        return Err(format!(
            "{} is not this session's ticket: name another ticket by its exact key (such as {}), not an id, a \
             number or a search result. To tell its owner something, use add_ticket_comment.",
            e.given.trim(),
            e.key
        ));
    }
    let worked_on = e.stage == TicketStage::InProgress || e.stage.is_closed();
    if worked_on && e.rewrites && !e.created {
        return Err(format!(
            "{} is {} and isn't this session's ticket, so its title and description stay as they are. Use \
             add_ticket_comment to say what should change.",
            e.key,
            match e.stage {
                TicketStage::InProgress => "in progress",
                TicketStage::Done => "done",
                _ => "closed",
            }
        ));
    }
    Ok(())
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
            proposed: self.proposed == Some(true),
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
    /// Who made it: set by the caller, never by a request (`user` from [`NewTicket::validate`]).
    pub source: TicketSource,
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
            source: TicketSource::User,
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
    fn agents_edit_their_own_ticket_and_others_only_by_exact_key() {
        let e = |given, stage, own, created, rewrites| {
            agent_may_edit(&AgentEdit { given, key: "CB-42", stage, own, created, rewrites })
        };
        use TicketStage::*;
        // Its own ticket: any way it names it, any field.
        assert!(e("01a1-uuid", InProgress, true, false, true).is_ok());
        // Another, by its exact key (any case): fine while nobody works on it, or for fields other than the text.
        assert!(e("CB-42", Todo, false, false, true).is_ok());
        assert!(e(" cb-42 ", Draft, false, false, true).is_ok());
        assert!(e("CB-42", InProgress, false, false, false).is_ok());
        assert!(e("CB-42", Done, false, false, false).is_ok());
        // One it created, even once work starts.
        assert!(e("CB-42", InProgress, false, true, true).is_ok());
        // An id, a bare number or another key is refused.
        for given in ["01a1-uuid", "42", "#42", "CB-33"] {
            let err = e(given, Todo, false, false, false).unwrap_err();
            assert!(err.contains("exact key") && err.contains("add_ticket_comment"), "{err}");
        }
        // Rewriting one in progress or closed that it didn't create.
        for (stage, word) in [(InProgress, "in progress"), (Done, "done"), (WontDo, "closed")] {
            let err = e("CB-42", stage, false, false, true).unwrap_err();
            assert!(err.contains(word) && err.contains("add_ticket_comment"), "{err}");
        }
    }

    #[test]
    fn stages_round_trip_and_know_when_they_are_closed() {
        for s in TicketStage::ALL {
            assert_eq!(TicketStage::parse(s.as_str()), Some(*s));
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.as_str()));
        }
        assert!(TicketStage::Done.is_closed() && TicketStage::WontDo.is_closed());
        assert!(!TicketStage::Draft.is_closed() && !TicketStage::Ready.is_closed());
        assert!(!TicketStage::Todo.is_closed() && !TicketStage::InProgress.is_closed());
        let before: Vec<TicketStage> = TicketStage::ALL.iter().copied().filter(|s| s.is_before_work()).collect();
        assert_eq!(before, [TicketStage::Draft, TicketStage::Ready, TicketStage::Todo]);
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
        let known = v(&["api", "web"]);
        assert_eq!(agent_areas(&v(&["web ", "api", "web"]), &known), Ok(v(&["api", "web"])));
        assert_eq!(agent_areas(&[], &known), Ok(vec![]));
        let e = agent_areas(&v(&["ap", "web"]), &known).unwrap_err();
        assert!(e.contains("not areas of the project: ap") && e.contains("api, web"), "{e}");
        assert!(agent_areas(&v(&["web"]), &[]).unwrap_err().contains("has no areas"));
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
            auto_start_at: None,
            pickup_hold: None,
            pickup_paused: None,
            pickup_failed: None,
            external: vec![],
            source: TicketSource::External,
            admitted: false,
            created_by: None,
            proposed: false,
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
        assert_eq!((v["source"].as_str(), v["admitted"].as_bool()), (Some("external"), Some(false)));
    }

    #[test]
    fn boarding_is_a_move_onto_the_board_or_the_agentic_label() {
        let l = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (d, r, t, ip) = (TicketStage::Draft, TicketStage::Ready, TicketStage::Todo, TicketStage::InProgress);
        assert!(boards(Some(d), Some(t), &[], None));
        assert!(boards(Some(r), Some(ip), &[], None));
        assert!(boards(None, Some(t), &[], None), "made in To do");
        assert!(!boards(None, Some(d), &[], None));
        assert!(!boards(Some(t), Some(t), &[], None));
        assert!(!boards(Some(t), Some(ip), &[], None), "already on the board");
        assert!(!boards(Some(d), Some(r), &[], None));
        assert!(!boards(Some(d), Some(TicketStage::Done), &[], None));
        assert!(boards(Some(d), None, &l(&["bug"]), Some(&l(&["bug", "agentic"]))));
        assert!(!boards(Some(d), None, &l(&["agentic"]), Some(&l(&["agentic", "bug"]))), "had it already");
        assert!(!boards(Some(d), None, &[], Some(&l(&["bug"]))));
    }

    #[test]
    fn only_the_user_or_a_confirmed_call_lets_an_external_ticket_in() {
        let user: By = Actor::User.into();
        let system: By = Actor::System.into();
        let agent = By::agent(coder());
        let confirmed = By { confirmed: true, ..By::agent(coder()) };
        let ext = TicketSource::External;
        assert_eq!(external_gate(ext, false, true, &user, "P-1"), Ok(true));
        assert_eq!(external_gate(ext, false, true, &confirmed, "P-1"), Ok(true));
        let refused = external_gate(ext, false, true, &system, "P-1").unwrap_err();
        assert!(refused.contains("P-1") && refused.contains("only the user"), "{refused}");
        assert!(external_gate(ext, false, true, &agent, "P-1").is_err());
        // Let in already, not boarding, or not external: nothing to say.
        assert_eq!(external_gate(ext, true, true, &system, "P-1"), Ok(false));
        assert_eq!(external_gate(ext, false, false, &system, "P-1"), Ok(false));
        for s in [TicketSource::User, TicketSource::Pm, TicketSource::Workflow, TicketSource::Agent] {
            assert_eq!(external_gate(s, false, true, &system, "P-1"), Ok(false));
        }
    }

    #[test]
    fn who_made_a_ticket_reads_its_source_then_its_actor() {
        let agent = By::agent(coder());
        assert_eq!(creator(TicketSource::User, &Actor::User.into()), "user");
        assert_eq!(creator(TicketSource::Agent, &agent), "agent");
        assert_eq!(creator(TicketSource::Pm, &agent), "pm");
        assert_eq!(creator(TicketSource::External, &agent), "external", "a triage session's ticket");
        assert_eq!(creator(TicketSource::External, &Actor::System.into()), "external");
        assert_eq!(creator(TicketSource::Workflow, &Actor::System.into()), "workflow");
        assert_eq!(creator(TicketSource::User, &Actor::System.into()), "workflow", "PDT made it");
        assert_eq!(creator(TicketSource::User, &Actor::Agent.into()), "agent", "an agent's curl, no session");
    }

    #[test]
    fn a_label_change_is_what_was_added_and_removed() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(label_change(&v(&["a", "b"]), &v(&["b", "c"])), (v(&["c"]), v(&["a"])));
        assert_eq!(label_change(&v(&[]), &v(&["a"])), (v(&["a"]), v(&[])));
        assert_eq!(label_change(&v(&["a"]), &v(&["a"])), (v(&[]), v(&[])));
    }

    fn coder() -> AgentBy {
        AgentBy {
            session_id: "s1".into(),
            project_id: Some("p1".into()),
            persona: Some("Coder".into()),
            from_ticket: Some("CB-34".into()),
            from_ticket_id: Some("t34".into()),
        }
    }

    #[test]
    fn an_agents_change_carries_its_session_in_the_events_data() {
        let by = By::agent(coder());
        let data = by.stamp(Some(serde_json::json!({"fields": ["title"]}))).unwrap();
        assert_eq!(data["fields"], serde_json::json!(["title"]));
        assert_eq!(data["agent"]["sessionId"], "s1");
        assert_eq!(data["agent"]["fromTicket"], "CB-34");
        assert_eq!(by.stamp(None).unwrap()["agent"]["persona"], "Coder");
        assert_eq!(AgentBy::of_event(EventKind::Edited, Actor::Agent, Some(&data)), Some(coder()));
        // The user's changes carry nothing.
        let user: By = Actor::User.into();
        assert_eq!(user.stamp(None), None);
        assert_eq!(AgentBy::of_event(EventKind::Edited, Actor::User, None), None);
    }

    #[test]
    fn older_agent_comments_are_read_from_their_flat_data() {
        let old = serde_json::json!({"sessionId": "s1", "persona": "Coder"});
        let a = AgentBy::of_event(EventKind::Comment, Actor::Agent, Some(&old)).unwrap();
        assert_eq!((a.session_id.as_str(), a.persona.as_deref(), a.from_ticket), ("s1", Some("Coder"), None));
        // A PR notice's sessionId is the session it told, not who made it.
        assert_eq!(AgentBy::of_event(EventKind::Pr, Actor::System, Some(&old)), None);
        assert_eq!(AgentBy::of_event(EventKind::Comment, Actor::Agent, Some(&serde_json::json!({}))), None);
    }
}
