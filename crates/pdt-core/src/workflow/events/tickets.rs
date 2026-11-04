use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
    /// `{{trigger.external.<tracker>}}`. Every link, however it was made.
    #[serde(default)]
    pub external: std::collections::BTreeMap<String, String>,
    /// Of those, the issue Done closes (or moves) in each tracker, `{{trigger.closing.<tracker>}}`: the issues PDT filed
    /// or imported, and those linked by hand with "Close it when this ticket is done" on. The trackers' Done built-ins
    /// act on these.
    #[serde(default)]
    pub closing: std::collections::BTreeMap<String, String>,
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

/// `ticket.idle` (PDT-510): a ticket in progress that has waited on nothing for a while.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TicketIdle {
    pub project_id: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub attempt_id: String,
    pub branch: String,
    /// Its open pull request, if it has one, and that pull request's URL.
    pub pr: Option<u32>,
    pub url: Option<String>,
    /// When it went idle (ms) and for how many minutes it has been.
    pub since: i64,
    pub idle_minutes: u64,
}
