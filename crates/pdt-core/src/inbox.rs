//! The Inbox: what waits on the user. Questions from agents (`ask_user`), human gates, stalled
//! and timed-out agent steps and loop limits are stored items; approvals (agents' open permission requests) are
//! listed from the conversations. Pure types and rules; pdt-store keeps items, pdt-api makes and answers them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InboxKind {
    /// An agent's `ask_user`.
    Question,
    /// A workflow's human gate.
    Gate,
    /// An agent step whose session ended its turns without an outcome.
    Stalled,
    /// An agent step that would run once more than its loop limit allows.
    LoopLimit,
    /// An agent's open permission request (not stored).
    Approval,
    /// A manual step: the user does the stage's work and picks its outcome.
    Manual,
    /// An agent step that took its `timeout` port (PDT-123): Resume or Stop.
    TimedOut,
    /// A session waiting on a permission prompt or a question (not stored: its `session.needs_you` notification).
    Session,
    /// A notice nobody could be told (PDT-139): PR notices found no agent session for a conflict or a CI failure.
    /// One per attempt and notice; it closes once the cause went away.
    Notice,
    /// An agent's `request_secret` (PDT-158): the user types the value into PDT's masked form, never the chat.
    Secret,
    /// A forge PDT needs is not signed in (PDT-177): Connect GitHub in Settings. Not stored: listed while it lasts.
    Connect,
    /// A ticket triage drafted waits for the user's Approve (to To do) or Reject (to Won't do) (PDT-225). It closes
    /// once the ticket moves past Ready some other way.
    Admit,
    /// A call to one of PDT's asking tools from an agent that can't ask in its own prompt (Codex, Devin; PDT-256):
    /// Allow runs the call as the user's yes, Deny tells the agent no. Either way the agent hears the result.
    Confirm,
    /// A workflow version an agent proposed (PDT-320): Accept or Discard. Not stored: listed while the proposal waits,
    /// so it goes once it is accepted, discarded or replaced by a newer one.
    Proposal,
    /// An MCP server's OAuth sign-in was revoked or expired (PDT-322): Sign in again in Settings → MCP servers. Not
    /// stored: listed while it lasts.
    McpSignIn,
    /// PDT can't save changes (a full disk, PDT-323). Not stored (it couldn't be): listed while writes fail.
    Storage,
    /// A disk with PDT's data or a project's checkouts is running low on space (PDT-324), with Clean actions. Not
    /// stored: listed while it lasts.
    LowDisk,
    /// A ticket an agent made, held in Draft off the board until the user decides (PDT-366): Accept to To do, Accept
    /// to backlog, Duplicate of… or Reject. It closes once the ticket leaves Draft some other way.
    TicketProposal,
}

impl InboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InboxKind::Question => "question",
            InboxKind::Gate => "gate",
            InboxKind::Stalled => "stalled",
            InboxKind::LoopLimit => "loop_limit",
            InboxKind::Approval => "approval",
            InboxKind::Manual => "manual",
            InboxKind::Session => "session",
            InboxKind::TimedOut => "timed_out",
            InboxKind::Notice => "notice",
            InboxKind::Secret => "secret",
            InboxKind::Connect => "connect",
            InboxKind::Admit => "admit",
            InboxKind::Confirm => "confirm",
            InboxKind::Proposal => "proposal",
            InboxKind::McpSignIn => "mcp_sign_in",
            InboxKind::Storage => "storage",
            InboxKind::LowDisk => "low_disk",
            InboxKind::TicketProposal => "ticket_proposal",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Question,
            Self::Gate,
            Self::Stalled,
            Self::LoopLimit,
            Self::Approval,
            Self::Manual,
            Self::Session,
            Self::TimedOut,
            Self::Notice,
            Self::Secret,
            Self::Connect,
            Self::Admit,
            Self::Confirm,
            Self::Proposal,
            Self::McpSignIn,
            Self::Storage,
            Self::LowDisk,
            Self::TicketProposal,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

/// What kind of wait an item is (stored with it). The Inbox lists items by their [`InboxSection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InboxGroup {
    Question,
    Approval,
    /// Sessions waiting on a permission prompt or a question.
    Session,
    Stalled,
    LoopLimit,
    /// Notices no agent session could be told (PDT-139).
    Notice,
    /// Triaged tickets waiting for the user's approval (PDT-225).
    Admit,
    /// Workflow versions agents proposed (PDT-320).
    Proposal,
    /// Tickets agents proposed (PDT-366).
    TicketProposal,
    Gate,
    /// A gate with `inbox: accept`: the work is done and waits for the user's Accept.
    Accept,
}

impl InboxGroup {
    pub const ALL: [InboxGroup; 11] = [
        InboxGroup::Question,
        InboxGroup::Approval,
        InboxGroup::Session,
        InboxGroup::Stalled,
        InboxGroup::LoopLimit,
        InboxGroup::Notice,
        InboxGroup::Admit,
        InboxGroup::Proposal,
        InboxGroup::TicketProposal,
        InboxGroup::Gate,
        InboxGroup::Accept,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            InboxGroup::Question => "question",
            InboxGroup::Approval => "approval",
            InboxGroup::Session => "session",
            InboxGroup::Stalled => "stalled",
            InboxGroup::LoopLimit => "loop_limit",
            InboxGroup::Notice => "notice",
            InboxGroup::Admit => "admit",
            InboxGroup::Proposal => "proposal",
            InboxGroup::TicketProposal => "ticket_proposal",
            InboxGroup::Gate => "gate",
            InboxGroup::Accept => "accept",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.as_str() == s)
    }
    pub fn title(self) -> &'static str {
        match self {
            InboxGroup::Question => "Questions",
            InboxGroup::Approval => "Approvals",
            InboxGroup::Session => "Sessions waiting",
            InboxGroup::Stalled => "Stalled",
            InboxGroup::LoopLimit => "Loop limit",
            InboxGroup::Notice => "Unheard notices",
            InboxGroup::Admit => "Triaged tickets",
            InboxGroup::Proposal => "Workflow proposals",
            InboxGroup::TicketProposal => "Ticket proposals",
            InboxGroup::Gate => "Gates",
            InboxGroup::Accept => "Ready to accept",
        }
    }
}

/// Where the Inbox lists an item, most urgent first (PDT-361): a few sections, each holding one or more groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InboxSection {
    Question,
    Approval,
    /// Agent steps that stopped: stalled, at their loop limit, timed out.
    Stuck,
    /// Things only the user can do: unheard notices, sessions waiting, sign-ins, storage, triaged tickets, proposals.
    Act,
    /// Gates: the work waits for the user's Accept or choice.
    Accept,
}

impl InboxSection {
    pub fn title(self) -> &'static str {
        match self {
            InboxSection::Question => "Questions",
            InboxSection::Approval => "Approvals",
            InboxSection::Stuck => "Stuck",
            InboxSection::Act => "To act on",
            InboxSection::Accept => "Ready to accept",
        }
    }
}

impl InboxGroup {
    /// The section the Inbox lists this group's items under.
    pub fn section(self) -> InboxSection {
        match self {
            InboxGroup::Question => InboxSection::Question,
            InboxGroup::Approval => InboxSection::Approval,
            InboxGroup::Stalled | InboxGroup::LoopLimit => InboxSection::Stuck,
            InboxGroup::Session
            | InboxGroup::Notice
            | InboxGroup::Admit
            | InboxGroup::Proposal
            | InboxGroup::TicketProposal => InboxSection::Act,
            InboxGroup::Gate | InboxGroup::Accept => InboxSection::Accept,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InboxStatus {
    Open,
    /// The user answered it.
    Resolved,
    /// Its step or run went away, or it was dismissed.
    Closed,
}

impl InboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            InboxStatus::Open => "open",
            InboxStatus::Resolved => "resolved",
            InboxStatus::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Open, Self::Resolved, Self::Closed].into_iter().find(|k| k.as_str() == s)
    }
}

/// The longest `question` an agent may ask ([`valid_questions`]): one line, the background goes in `context`.
pub const QUESTION_MAX: usize = 120;

// Items asked before PDT-476 have plain-string choices ([`Choice`] reads them) and no context.
/// One of an agent's questions (`ask_user`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Question {
    /// The question itself, one line of at most 120 characters ("Which approach do you want?"). The background
    /// goes in `context`, each option's trade-offs in its choice's `description`.
    pub question: String,
    /// Markdown: the background the user needs to answer (what you found, what is at stake).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context: Option<String>,
    /// Answers to pick from, if it has a few likely ones (the user can still write their own).
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// The `label` of the choice you recommend; it is pre-selected for the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recommended: Option<String>,
    /// Why you recommend it, one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub why: Option<String>,
}

/// One of a question's answers to pick from: a short label, and what it means in Markdown. A plain string is a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Choice {
    /// A few words ("Pair the browser"): the answer the user picks and you get back.
    pub label: String,
    /// Markdown: what it means, its pros and cons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
}

impl Choice {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), description: None }
    }
}

impl From<&str> for Choice {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// A choice as `ask_user` takes it: an object, or (the old shape) a plain label.
impl<'de> Deserialize<'de> for Choice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            label: String,
            #[serde(default)]
            description: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum In {
            Label(String),
            Full(Full),
        }
        match In::deserialize(d).map_err(|_| {
            serde::de::Error::custom("a choice is {\"label\": \"…\", \"description\": \"…\"} (or a plain label)")
        })? {
            In::Label(label) => Ok(Self { label, description: None }),
            In::Full(f) => Ok(Self { label: f.label, description: f.description }),
        }
    }
}

/// A choice an item offers (`POST /api/inbox/{id}/answer {choice}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxChoice {
    pub id: String,
    pub title: String,
    /// Enter takes it.
    pub primary: bool,
}

impl InboxChoice {
    pub fn new(id: impl Into<String>, title: impl Into<String>, primary: bool) -> Self {
        Self { id: id.into(), title: title.into(), primary }
    }
}

/// The choice id that takes an agent step out by `outcome` (Take over).
pub fn outcome_choice(outcome: &str) -> String {
    format!("outcome:{outcome}")
}

/// The outcome a Take over choice names.
pub fn choice_outcome(choice: &str) -> Option<&str> {
    choice.strip_prefix("outcome:")
}

pub const NUDGE: &str = "nudge";
pub const AGAIN: &str = "again";
pub const STOP: &str = "stop";
pub const RESUME: &str = "resume";
/// A stalled step whose first message never reached its session: send it again, as the user's (PDT-144).
pub const RESEND: &str = "resend";
/// A triaged ticket's Approve: to To do, as the user's own move (PDT-225).
pub const APPROVE: &str = "approve";
/// A triaged ticket's Reject: to Won't do.
pub const REJECT: &str = "reject";
/// An asking tool call's Allow (PDT-256): PDT runs it.
pub const ALLOW: &str = "allow";
/// An asking tool call's Deny.
pub const DENY: &str = "deny";
/// A workflow proposal's Accept: it becomes the current version (PDT-320).
pub const ACCEPT: &str = "accept";
/// A workflow proposal's Discard: it is set aside.
pub const DISCARD: &str = "discard";
/// A ticket proposal's Accept to To do (PDT-366).
pub const ACCEPT_TODO: &str = "accept_todo";
/// A ticket proposal's Accept to backlog: to Ready.
pub const ACCEPT_BACKLOG: &str = "accept_backlog";
/// A ticket proposal's Duplicate of…: to Won't do, linked to the ticket it repeats. Reject is [`REJECT`].
pub const DUPLICATE: &str = "duplicate";

/// The id of the item that says PDT can't save changes ([`storage_item`]).
pub const STORAGE_ID: &str = "storage";

/// `bytes` for people: "512 MB", "1.5 GB".
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 || v >= 10.0 { format!("{v:.0} {}", UNITS[unit]) } else { format!("{v:.1} {}", UNITS[unit]) }
}

/// The item that says PDT can't save changes while writes fail (PDT-323): `disk_full` when the latest failure says
/// the disk is full, then with what is free on the disk with PDT's data (`free`, bytes). The database's own text is
/// in PDT's log, not here.
pub fn storage_item(disk_full: bool, free: Option<u64>, since: i64) -> InboxItem {
    let title = if disk_full { "PDT can't save changes: the disk is full" } else { "PDT can't save changes" };
    let text = if disk_full {
        let free = free.map(|b| format!(" ({} free)", human_bytes(b))).unwrap_or_default();
        format!(
            "The disk with PDT's data is full{free}, so PDT can't save anything: workflow runs, events and \
             conversations wait. Free up space on it; PDT saves again on its own within seconds, and starts what waited."
        )
    } else {
        "PDT can't save to its database (PDT's log says why). Workflow runs and events wait, and PDT tries again \
         every few seconds; this goes away once saving works."
            .to_string()
    };
    InboxItem {
        id: STORAGE_ID.into(),
        kind: InboxKind::Storage,
        group: InboxGroup::Notice,
        status: InboxStatus::Open,
        project_id: None,
        ticket_id: None,
        ticket_key: None,
        ticket_title: None,
        attempt_id: None,
        session_id: None,
        run_id: None,
        step_id: None,
        persona_id: None,
        persona_name: None,
        change_request: None,
        title: title.into(),
        body: serde_json::json!({"text": text, "diskFull": disk_full, "freeBytes": free}),
        choices: vec![],
        resolution: Value::Null,
        created_at: since,
        resolved_at: None,
    }
}

/// The id of the item that says a disk is running low ([`low_disk_item`]); a critically low one has its own, so
/// getting there is told again.
pub const LOW_DISK_ID: &str = "low_disk";
pub const CRITICAL_DISK_ID: &str = "low_disk:critical";

/// The item that says a disk is running low (PDT-324): the fullest disk's free bytes and the folders on it,
/// `critical` when it is below the critical threshold, `paused` when pickup is paused (Clean up storage pauses it then,
/// PDT-370), since `since`.
pub fn low_disk_item(free: u64, folders: &[String], critical: bool, paused: bool, since: i64) -> InboxItem {
    let free_text = human_bytes(free);
    let title = match (critical, paused) {
        (true, true) => format!("The disk is almost full ({free_text} free): auto-start is paused"),
        (true, false) => format!("The disk is almost full ({free_text} free)"),
        _ => format!("Disk space is low ({free_text} free)"),
    };
    let on = match folders {
        [] => String::new(),
        [one] => format!(" (with `{one}`)"),
        many => format!(" (with {})", many.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")),
    };
    let mut text = format!(
        "The disk{on} has {free_text} free. Worktrees' build output is usually what fills it: clean it in Settings › \
         Storage, or clean every finished ticket's worktree at once."
    );
    if critical && paused {
        text.push_str(" Until there is more space, auto-start starts no new tickets; agents already running carry on.");
    }
    InboxItem {
        id: if critical { CRITICAL_DISK_ID } else { LOW_DISK_ID }.into(),
        kind: InboxKind::LowDisk,
        group: InboxGroup::Notice,
        status: InboxStatus::Open,
        project_id: None,
        ticket_id: None,
        ticket_key: None,
        ticket_title: None,
        attempt_id: None,
        session_id: None,
        run_id: None,
        step_id: None,
        persona_id: None,
        persona_name: None,
        change_request: None,
        title,
        body: serde_json::json!({"text": text, "freeBytes": free, "folders": folders, "critical": critical}),
        choices: vec![],
        resolution: Value::Null,
        created_at: since,
        resolved_at: None,
    }
}

/// A workflow proposal's item id (PDT-320): one per workflow and proposed version.
pub fn proposal_id(workflow_id: &str, version: u32) -> String {
    format!("proposal:{workflow_id}:{version}")
}

/// The workflow and version a proposal's item id names.
pub fn parse_proposal_id(id: &str) -> Option<(&str, u32)> {
    let (w, v) = id.strip_prefix("proposal:")?.rsplit_once(':')?;
    Some((w, v.parse().ok()?)).filter(|(w, _)| !w.is_empty())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxItem {
    pub id: String,
    pub kind: InboxKind,
    pub group: InboxGroup,
    pub status: InboxStatus,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    /// Filled in by the API.
    pub ticket_key: Option<String>,
    pub ticket_title: Option<String>,
    pub attempt_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub persona_id: Option<String>,
    /// Filled in by the API.
    pub persona_name: Option<String>,
    /// The pull request (or a forge's merge request) a gate is about (PDT-306): its ticket's attempt's, the run
    /// trigger's or its ticket's external link. Filled in by the API, never stored.
    #[serde(default)]
    pub change_request: Option<ChangeRequestLink>,
    /// One line: a gate's step title or its question's first line (a long Markdown question goes in `body.question`).
    pub title: String,
    /// Kind-specific: `questions` (question), `question`/`choices` (gate), `outcomes`, `summary`, `turns`
    /// (stalled), `passes` (loop limit), `request` (approval), `notice`/`key`/`text`/`pr`/`url` (notice),
    /// `summary`/`issue`/`external` (admit), `tool`/`toolTitle`/`input`/`agent`/`sessionTitle` (confirm),
    /// `description`/`priority`/`labels`/`reason`/`persona`/`fromTicket`/`fromTicketId` (ticket proposal).
    #[ts(type = "Record<string, unknown>")]
    pub body: Value,
    pub choices: Vec<InboxChoice>,
    #[ts(type = "unknown")]
    pub resolution: Value,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub resolved_at: Option<i64>,
}

/// A change request's page on its forge, for the Inbox's Open pull request button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeRequestLink {
    pub url: String,
    /// What the forge calls it: `pull request`, GitLab's `merge request`.
    pub name: String,
}

/// A title is at most this long (characters, with the ellipsis).
pub const TITLE_CHARS: usize = 100;

/// A gate's title (PDT-306): its question in plain words when that is one line, else its step's title, else its
/// question's first line in plain words, cut to [`TITLE_CHARS`]. The whole question is the item's body.
pub fn gate_title(step_title: &str, question: &str) -> String {
    let line = plain_line(question);
    let one_line = question.trim().lines().count() <= 1;
    let step = step_title.trim();
    let t = if one_line || step.is_empty() { cut(&line, TITLE_CHARS) } else { step.to_string() };
    if t.is_empty() { "Your choice".to_string() } else { t }
}

/// The first line of Markdown `text` with no words, as plain text: links keep their text, and emphasis, code marks,
/// heading, quote and list marks go.
pub fn plain_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
    let mut out = String::new();
    let mut rest = line;
    // `[text](url)` → text; `![alt](url)` → alt.
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else { break };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else { break };
        out.push_str(rest[..open].trim_end_matches('!'));
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    let out: String = out.replace("**", "").replace("__", "").chars().filter(|c| !matches!(c, '`' | '*')).collect();
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut to `max` characters at a word, with "…".
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max - 1).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => cut[..i].trim_end().to_string(),
        _ => cut,
    };
    format!("{cut}…")
}

/// `GET /api/inbox`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxList {
    pub items: Vec<InboxItem>,
    /// Open items, sessions waiting included: the Needs-you count the rail badge and the page title show.
    pub total: u32,
}

/// `POST /api/inbox/{id}/answer`.
#[derive(Clone, Default, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InboxAnswer {
    #[serde(default)]
    #[ts(optional)]
    pub choice: Option<String>,
    /// A secret request's value (PDT-158): kept in the keychain only, never stored with the item or sent on.
    #[serde(default)]
    #[ts(optional)]
    pub secret: Option<String>,
    /// A question's answer as one text, or the words of a nudge.
    #[serde(default)]
    #[ts(optional)]
    pub text: Option<String>,
    /// One answer per question.
    #[serde(default)]
    #[ts(optional)]
    pub answers: Option<Vec<String>>,
    /// A ticket proposal's Accept or Duplicate of… (PDT-366): the user's edits, the `agentic` box and the ticket it
    /// repeats. Reject's reason is `text`.
    #[serde(default)]
    #[ts(optional)]
    pub proposal: Option<crate::ticket_proposal::ProposalAnswer>,
    /// A batch call's Allow (the PM's `inbox_decide`, PDT-475): the rows the user left ticked (0-based); every row
    /// when absent.
    #[serde(default)]
    #[ts(optional)]
    pub rows: Option<Vec<u32>>,
}

/// Never shows a secret's value.
impl std::fmt::Debug for InboxAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboxAnswer")
            .field("choice", &self.choice)
            .field("secret", &self.secret.as_ref().map(|_| "[secret]"))
            .field("text", &self.text)
            .field("answers", &self.answers)
            .field("proposal", &self.proposal)
            .field("rows", &self.rows)
            .finish()
    }
}

/// Sorts items as the Inbox lists them: by section, then oldest first.
pub fn sort(items: &mut [InboxItem]) {
    items.sort_by(|a, b| {
        a.group.section().cmp(&b.group.section()).then(a.created_at.cmp(&b.created_at)).then(a.id.cmp(&b.id))
    });
}

/// The message an agent gets with the user's answers.
pub fn answers_message(questions: &[Question], answer: &InboxAnswer) -> Result<String, String> {
    let mut out = String::from("[PDT] The user answered your questions:\n");
    match (&answer.answers, &answer.text) {
        (Some(list), _) if list.iter().any(|a| !a.trim().is_empty()) => {
            for (i, q) in questions.iter().enumerate() {
                let a = list.get(i).map(|a| a.trim()).filter(|a| !a.is_empty()).unwrap_or("(no answer)");
                out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, q.question.trim(), a.replace('\n', "\n   ")));
            }
        }
        (_, Some(t)) if !t.trim().is_empty() => {
            for (i, q) in questions.iter().enumerate() {
                out.push_str(&format!("\n{}. {}", i + 1, q.question.trim()));
            }
            out.push_str(&format!("\n\n{}\n", t.trim()));
        }
        _ => return Err("write an answer first".into()),
    }
    Ok(out)
}

/// Checks `ask_user`'s questions: trimmed, each `question` one short line, `recommended` one of its choices.
pub fn valid_questions(questions: Vec<Question>) -> Result<Vec<Question>, String> {
    let text = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let qs: Vec<Question> = questions
        .into_iter()
        .map(|q| Question {
            question: q.question.trim().to_string(),
            context: text(q.context),
            choices: q
                .choices
                .into_iter()
                .map(|c| Choice { label: c.label.trim().to_string(), description: text(c.description) })
                .filter(|c| !c.label.is_empty())
                .collect(),
            why: text(q.why).filter(|_| text(q.recommended.clone()).is_some()),
            recommended: text(q.recommended),
        })
        .filter(|q| !q.question.is_empty())
        .collect();
    if qs.is_empty() {
        return Err("ask at least one question".into());
    }
    if qs.len() > 10 {
        return Err("ask at most 10 questions at a time".into());
    }
    for (n, q) in qs.iter().enumerate() {
        let len = q.question.chars().count();
        if len > QUESTION_MAX || q.question.contains('\n') {
            return Err(format!(
                "question {} is too long ({len} characters, {} line(s)): keep `question` to one line of at most \
                 {QUESTION_MAX} characters (\"Which approach do you want?\"), and move the background into `context` \
                 (Markdown) and each option's trade-offs into its choice's `description`",
                n + 1,
                q.question.lines().count()
            ));
        }
        if let Some(r) = &q.recommended
            && !q.choices.iter().any(|c| &c.label == r)
        {
            return Err(format!(
                "question {}'s `recommended` is \"{r}\", which is none of its choices: give the `label` of one",
                n + 1
            ));
        }
    }
    Ok(qs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, group: InboxGroup, at: i64) -> InboxItem {
        InboxItem {
            id: id.into(),
            kind: InboxKind::Gate,
            group,
            status: InboxStatus::Open,
            project_id: None,
            ticket_id: None,
            ticket_key: None,
            ticket_title: None,
            attempt_id: None,
            session_id: None,
            run_id: None,
            step_id: None,
            persona_id: None,
            persona_name: None,
            change_request: None,
            title: String::new(),
            body: Value::Null,
            choices: vec![],
            resolution: Value::Null,
            created_at: at,
            resolved_at: None,
        }
    }

    #[test]
    fn questions_come_first_then_oldest_first() {
        let mut v = vec![
            item("accept", InboxGroup::Accept, 1),
            item("q2", InboxGroup::Question, 9),
            item("stall", InboxGroup::Stalled, 2),
            item("q1", InboxGroup::Question, 5),
        ];
        sort(&mut v);
        assert_eq!(v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["q1", "q2", "stall", "accept"]);
    }

    #[test]
    fn merged_sections_list_their_groups_oldest_first() {
        let mut v = vec![
            item("gate", InboxGroup::Gate, 3),
            item("loop", InboxGroup::LoopLimit, 1),
            item("accept", InboxGroup::Accept, 2),
            item("stall", InboxGroup::Stalled, 4),
            item("notice", InboxGroup::Notice, 9),
            item("session", InboxGroup::Session, 8),
            item("proposal", InboxGroup::Proposal, 7),
        ];
        sort(&mut v);
        assert_eq!(
            v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["loop", "stall", "proposal", "session", "notice", "accept", "gate"]
        );
        assert_eq!(InboxGroup::Admit.section(), InboxSection::Act);
        assert_eq!(InboxSection::Act.title(), "To act on");
    }

    #[test]
    fn answers_go_back_one_per_question_or_as_one_text() {
        let qs = vec![
            Question { question: "Which DB?".into(), choices: vec!["SQLite".into()], ..Default::default() },
            Question { question: "Keep the old API?".into(), ..Default::default() },
        ];
        let a = InboxAnswer { answers: Some(vec!["SQLite".into(), "".into()]), ..Default::default() };
        let m = answers_message(&qs, &a).unwrap();
        assert!(m.starts_with("[PDT] The user answered your questions:"));
        assert!(m.contains("1. Which DB?\n   SQLite"), "{m}");
        assert!(m.contains("2. Keep the old API?\n   (no answer)"), "{m}");
        let t = InboxAnswer { text: Some("Both: SQLite, and yes.".into()), ..Default::default() };
        assert!(answers_message(&qs, &t).unwrap().ends_with("Both: SQLite, and yes.\n"));
        assert!(answers_message(&qs, &InboxAnswer::default()).is_err());
    }

    #[test]
    fn questions_are_trimmed_and_bounded() {
        let q =
            |s: &str| Question { question: s.into(), choices: vec![" a ".into(), " ".into()], ..Default::default() };
        let v = valid_questions(vec![q(" Why? "), q("  ")]).unwrap();
        assert_eq!(v, vec![Question { question: "Why?".into(), choices: vec!["a".into()], ..Default::default() }]);
        assert!(valid_questions(vec![q(" ")]).is_err());
        assert!(valid_questions((0..11).map(|_| q("x")).collect()).is_err());
    }

    #[test]
    fn a_question_takes_context_described_choices_and_a_recommendation_or_the_old_plain_labels() {
        let q: Question = serde_json::from_value(serde_json::json!({
            "question": "Which approach do you want?",
            "context": "The ticket's fixes **don't** stop an agent.",
            "choices": [{"label": "A", "description": "Pair the browser."}, "B"],
            "recommended": "A", "why": "A much higher bar."
        }))
        .unwrap();
        assert_eq!(
            q.choices,
            vec![Choice { label: "A".into(), description: Some("Pair the browser.".into()) }, "B".into()]
        );
        assert_eq!(valid_questions(vec![q.clone()]).unwrap(), vec![q.clone()]);
        // Stored as objects, which read back.
        let back: Question = serde_json::from_value(serde_json::to_value(&q).unwrap()).unwrap();
        assert_eq!(back, q);
        let old: Question =
            serde_json::from_value(serde_json::json!({"question": "Which?", "choices": ["x", "y"]})).unwrap();
        assert_eq!(old.choices, vec![Choice::new("x"), Choice::new("y")]);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            serde_json::json!({"question": "Which?", "choices": [{"label": "x"}, {"label": "y"}]})
        );
        assert!(
            serde_json::from_value::<Question>(serde_json::json!({"question": "Q", "choices": [{"name": "x"}]}))
                .is_err()
        );
    }

    #[test]
    fn a_long_question_or_a_recommendation_of_no_choice_is_refused_with_what_to_do() {
        let long = Question { question: "x".repeat(QUESTION_MAX + 1), ..Default::default() };
        let e = valid_questions(vec![long]).unwrap_err();
        assert!(e.contains("question 1 is too long") && e.contains("`context`") && e.contains("`description`"), "{e}");
        assert!(valid_questions(vec![Question { question: "x".repeat(QUESTION_MAX), ..Default::default() }]).is_ok());
        assert!(valid_questions(vec![Question { question: "One?\nTwo?".into(), ..Default::default() }]).is_err());
        let rec = |r: &str| Question {
            question: "Which?".into(),
            choices: vec!["A".into(), "B".into()],
            recommended: Some(r.into()),
            why: Some(" Simpler. ".into()),
            ..Default::default()
        };
        let e = valid_questions(vec![rec("C")]).unwrap_err();
        assert!(e.contains("`recommended` is \"C\""), "{e}");
        assert_eq!(valid_questions(vec![rec(" B ")]).unwrap()[0].why.as_deref(), Some("Simpler."));
        // A why without a recommendation says nothing.
        let mut q = rec("A");
        q.recommended = None;
        assert_eq!(valid_questions(vec![q]).unwrap()[0].why, None);
    }

    #[test]
    fn a_gate_is_titled_by_its_step_else_its_question_in_plain_words() {
        let q = "**[acme/app#314](https://github.com/acme/app/pull/314)** by `bot`: Bump *tokio*\n\nCI: running";
        assert_eq!(gate_title("Ask me", q), "Ask me");
        // A short one-line question says more than its step's title.
        assert_eq!(
            gate_title("Merge blocked", "Merge blocked on `PDT-7`: no approval"),
            "Merge blocked on PDT-7: no approval"
        );
        let long = gate_title("Merge blocked", &"word ".repeat(60));
        assert!(long.starts_with("word") && long.ends_with('…'), "{long}");
        assert_eq!(gate_title(" ", q), "acme/app#314 by bot: Bump tokio");
        assert_eq!(gate_title("", "\n## Merge it?\n- more"), "Merge it?");
        assert_eq!(gate_title("", ""), "Your choice");
        let long = gate_title("", &"word ".repeat(60));
        assert!(long.chars().count() <= TITLE_CHARS && long.ends_with('…'), "{long}");
        assert_eq!(plain_line("![logo](x.png) and [a](b) [not a link"), "logo and a [not a link");
    }

    #[test]
    fn take_over_choices_name_the_outcome() {
        assert_eq!(outcome_choice("approved"), "outcome:approved");
        assert_eq!(choice_outcome("outcome:approved"), Some("approved"));
        assert_eq!(choice_outcome(NUDGE), None);
        assert_eq!(InboxGroup::parse("loop_limit"), Some(InboxGroup::LoopLimit));
        assert_eq!(InboxKind::parse(InboxKind::Stalled.as_str()), Some(InboxKind::Stalled));
    }

    #[test]
    fn a_proposal_id_names_its_workflow_and_version() {
        let id = proposal_id("01ab", 3);
        assert_eq!(parse_proposal_id(&id), Some(("01ab", 3)));
        assert_eq!(parse_proposal_id("proposal:01ab:x"), None);
        assert_eq!(parse_proposal_id("proposal::3"), None);
        assert_eq!(parse_proposal_id("gate:01ab:3"), None);
        assert_eq!(InboxKind::parse("proposal"), Some(InboxKind::Proposal));
        assert_eq!(InboxGroup::parse("proposal"), Some(InboxGroup::Proposal));
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(human_bytes(0), "0 bytes");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(300 * 1024 * 1024), "300 MB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024 / 2), "2.5 GB");
    }

    #[test]
    fn the_low_disk_item_says_what_is_free_and_whether_auto_start_waits() {
        let gb = 1024 * 1024 * 1024;
        let low = low_disk_item(12 * gb, &["/work".into()], false, false, 7);
        assert_eq!(
            (low.id.as_str(), low.kind, low.group, low.created_at),
            (LOW_DISK_ID, InboxKind::LowDisk, InboxGroup::Notice, 7)
        );
        assert_eq!(low.title, "Disk space is low (12 GB free)");
        let text = low.body["text"].as_str().unwrap();
        assert!(text.contains("(with `/work`)") && !text.contains("auto-start"), "{text}");
        let critical = low_disk_item(3 * gb, &["/a".into(), "/b".into()], true, true, 7);
        assert_eq!(critical.id, CRITICAL_DISK_ID);
        assert_eq!(critical.title, "The disk is almost full (3.0 GB free): auto-start is paused");
        let text = critical.body["text"].as_str().unwrap();
        assert!(text.contains("`/a`, `/b`") && text.contains("auto-start starts no new tickets"), "{text}");
        // Clean up storage set not to pause pickup: it says nothing of auto-start.
        let unpaused = low_disk_item(3 * gb, &["/a".into()], true, false, 7);
        assert_eq!(
            (unpaused.id.as_str(), unpaused.title.as_str()),
            (CRITICAL_DISK_ID, "The disk is almost full (3.0 GB free)")
        );
        assert!(!unpaused.body["text"].as_str().unwrap().contains("auto-start"));
    }

    #[test]
    fn the_storage_item_says_the_disk_is_full_and_what_is_free() {
        let i = storage_item(true, Some(12 * 1024 * 1024), 5);
        assert_eq!(
            (i.id.as_str(), i.kind, i.group, i.created_at),
            (STORAGE_ID, InboxKind::Storage, InboxGroup::Notice, 5)
        );
        assert_eq!(i.title, "PDT can't save changes: the disk is full");
        let text = i.body["text"].as_str().unwrap();
        assert!(text.contains("(12 MB free)"), "{text}");
        let other = storage_item(false, None, 5);
        assert_eq!(other.title, "PDT can't save changes");
        assert!(other.body["text"].as_str().unwrap().contains("PDT's log says why"));
        assert_eq!(InboxKind::parse("storage"), Some(InboxKind::Storage));
    }
}
