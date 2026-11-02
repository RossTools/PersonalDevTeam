//! The project manager (PDT-81): a host-level conversation
//! with PDT's tools only. PDT owns its memory: the briefing every new PM session starts with (rendered here from data
//! pdt-api gathers), notes searched with FTS5, journal checkpoints and daily updates. Pure.

use crate::brand::{PRODUCT_NAME, SHORT_NAME};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use ts_rs::TS;

/// How far back the first PM conversation's briefing reads events (ms).
pub const BRIEF_SINCE_MS: i64 = 24 * 60 * 60 * 1000;

/// How early a conversation may be brought up to date ([`update_due`], ms): a schedule fires a moment less than a day
/// after the run that last brought it up to date, and still finds it due.
pub const UPDATE_SLACK_MS: i64 = 15 * 60 * 1000;

/// How long a reset waits for the conversation's journal turn before it records none (PDT-308, ms).
pub const RESET_WAIT_MS: i64 = 3 * 60 * 1000;

/// What the journal is for, said in every request for it.
const JOURNAL_WHAT: &str = "what you and the user are working on, what was decided, and the threads still open \
                            (under a `Still open` heading). It replaces this conversation's earlier entry, so write \
                            the whole picture; later PM conversations start from it";

/// What PDT sends a PM conversation the user closed (spec §5.3).
pub fn journal_request() -> String {
    format!(
        "[{SHORT_NAME}] The user closed this PM conversation, so it is ending. Write its journal entry with \
         `write_journal`: {JOURNAL_WHAT}. Then end your turn."
    )
}

/// What PDT sends a PM conversation the user is resetting (PDT-308): a fresh one with its name starts from this entry.
pub fn reset_request() -> String {
    format!(
        "[{SHORT_NAME}] The user is resetting this PM conversation: it ends, and a fresh conversation with the same \
         name starts from your journal entry. Write it with `write_journal`: {JOURNAL_WHAT}. Then end your turn."
    )
}

/// What PDT sends a PM conversation whose context the agent just compacted (spec §5.3): a checkpoint, and it carries on.
pub fn checkpoint_request() -> String {
    format!(
        "[{SHORT_NAME}] Your context was just compacted. Write a journal checkpoint with `write_journal`: \
         {JOURNAL_WHAT}. Then end your turn; this conversation carries on."
    )
}

/// The journal recorded when a rotating session wrote none.
pub const NO_JOURNAL: &str = "(no journal)";

/// A PM conversation's title until its first message names it (PDT-149).
pub const NEW_TITLE: &str = "New conversation";
/// A title made from a first message is at most this long (characters, with the ellipsis).
const AUTO_TITLE_CHARS: usize = 60;
/// A briefing's summary of another conversation is at most this long.
const SUMMARY_CHARS: usize = 120;

/// The first line of `text` with its Markdown heading or list marks taken off, cut to `max` characters at a word
/// (with "…"). Empty when the text has no words.
fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.trim_start_matches(['#', '>', '-', '*']).trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max - 1).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => cut[..i].trim_end().to_string(),
        _ => cut,
    };
    format!("{cut}…")
}

/// A PM conversation's title from its first message (its first line, shortened); [`NEW_TITLE`] when it has none.
pub fn conversation_title(first: &str) -> String {
    let t = first_line(first, AUTO_TITLE_CHARS);
    if t.is_empty() { NEW_TITLE.to_string() } else { t }
}

/// One line saying where a conversation stands, from its last message (the briefing's list of the others).
pub fn summary_line(last: &str) -> String {
    first_line(last, SUMMARY_CHARS)
}

const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 16 * 1024;
const MAX_JOURNAL: usize = 8 * 1024;

/// Why a PM session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RotateReason {
    /// Its context was 60 % full (before PDT-167: conversations now compact and carry on).
    Context,
    /// It was a day old (before PDT-167).
    Age,
    /// The user closed it.
    User,
    /// The user reset it (PDT-308): a fresh conversation with its name and settings carries on from its journal.
    Reset,
}

impl RotateReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RotateReason::Context => "context",
            RotateReason::Age => "age",
            RotateReason::User => "user",
            RotateReason::Reset => "reset",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RotateReason::Context, RotateReason::Age, RotateReason::User, RotateReason::Reset]
            .into_iter()
            .find(|r| r.as_str() == s)
    }
}

/// A PM session's rotation state (`pm_sessions.rotation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Rotation {
    /// In use.
    None,
    /// Closed by the user and asked for its journal; it ends when that turn ends.
    Journal,
    /// Ended.
    Done,
}

impl Rotation {
    pub fn as_str(self) -> &'static str {
        match self {
            Rotation::None => "none",
            Rotation::Journal => "journal",
            Rotation::Done => "done",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Rotation::None, Rotation::Journal, Rotation::Done].into_iter().find(|r| r.as_str() == s)
    }
}

/// A note, a journal entry or one of the user's own notes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoteKind {
    #[default]
    Note,
    /// A PM conversation's own entry: checkpoints while it runs, the last one as it ends.
    Journal,
    // PDT-126. (Doc comments here reach agents in the tools' schemas: no ticket keys.)
    /// The user's own note: never in a briefing, and the PM can find it but not write or delete it.
    User,
}

impl NoteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Note => "note",
            NoteKind::Journal => "journal",
            NoteKind::User => "user",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [NoteKind::Note, NoteKind::Journal, NoteKind::User].into_iter().find(|k| k.as_str() == s)
    }
    /// Whether the PM's tools may change or delete a note of this kind (never the user's own).
    pub fn pm_may_write(self) -> bool {
        self != NoteKind::User
    }
}

/// One of the PM's notes: what PDT's data can't tell it (preferences, decisions, people, rules at work).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmNote {
    pub id: String,
    /// None: host-wide.
    pub project_id: Option<String>,
    pub kind: NoteKind,
    pub title: String,
    pub body: String,
    /// Listed in every briefing.
    pub pinned: bool,
    /// A journal entry's PM session.
    pub session_id: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// `POST /api/pm/notes` (and the `save_note` tool without an id).
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewNote {
    /// A project id or key; host-wide when absent.
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    #[ts(optional)]
    pub pinned: Option<bool>,
    /// `note` (the PM's, the default) or `user`; journal entries are only written by PM conversations.
    #[serde(default)]
    #[ts(optional)]
    pub kind: Option<NoteKind>,
}

/// `PATCH /api/pm/notes/{id}`: absent fields are left alone; `project: ""` makes it host-wide.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotePatch {
    #[serde(default)]
    #[ts(optional)]
    pub project: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub body: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub pinned: Option<bool>,
}

/// A note's title, trimmed, or why not.
pub fn valid_title(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() {
        return Err("a note needs a title".into());
    }
    if t.contains('\n') || t.chars().count() > MAX_TITLE {
        return Err(format!("a note's title is one line of at most {MAX_TITLE} characters"));
    }
    Ok(t.to_string())
}

pub fn valid_body(b: &str) -> Result<String, String> {
    if b.len() > MAX_BODY {
        return Err(format!("a note is at most {} KB", MAX_BODY / 1024));
    }
    Ok(b.trim_end().to_string())
}

pub fn valid_journal(b: &str) -> Result<String, String> {
    let b = b.trim();
    if b.is_empty() {
        return Err("the journal entry is empty".into());
    }
    if b.len() > MAX_JOURNAL {
        return Err(format!("a journal entry is at most {} KB", MAX_JOURNAL / 1024));
    }
    Ok(b.to_string())
}

/// An FTS5 `MATCH` expression for what the user typed: each word a quoted prefix, all of them required. None when
/// there are no words.
pub fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// `GET /api/pm`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmState {
    /// The newest open PM conversation (for clients that know only one); None when none is open.
    pub current: Option<Session>,
    /// Every PM conversation: the open ones (latest activity first), then the ended ones (newest first).
    pub sessions: Vec<PmSessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmSessionInfo {
    pub session: Session,
    /// Not ended: it takes messages (PDT-149: several can be open at once).
    pub open: bool,
    /// Its title: from its first message until the user renames it.
    pub title: String,
    /// It has a reply the user hasn't seen.
    pub unseen: bool,
    /// When something last happened in it (its last event; its start when it has none).
    #[ts(type = "number")]
    pub last_activity: i64,
    pub rotation: Rotation,
    pub reason: Option<RotateReason>,
    /// Its journal entry: the latest checkpoint, or the one it wrote as it ended.
    pub journal: Option<String>,
    /// Where it is held besides PDT: `discord` for one the user talks to through the Discord bot's DMs (PDT-304).
    #[serde(default)]
    pub via: Option<String>,
    /// The conversation this one was reset from (PDT-308): ended, readable, with its own `resetFrom` when it was a
    /// reset one too.
    #[serde(default)]
    pub reset_from: Option<String>,
}

/// `POST /api/pm/sessions`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartPm {
    /// The new conversation's first message.
    #[serde(default)]
    #[ts(optional)]
    pub text: Option<String>,
    /// Its title; made from the first message when left out.
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

/// `PATCH /api/pm/sessions/{id}`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmSessionPatch {
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

/// What an asking tool's call acts on, so the project's policy can be found (PDT-81 spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskTarget {
    /// A project id or key.
    Project(String),
    /// A ticket id or key.
    Ticket(String),
    Session(String),
    /// A document by id (PDT-317): the PM acts without asking when a project it is linked to lets it.
    Document(String),
    /// A connection of `kind` by name (None: the only one of its kind), whose `config.pmActs` says whether the PM
    /// acts without asking (an integration's tools, through their connection).
    Connection {
        kind: &'static str,
        name: Option<String>,
    },
    /// A call no policy answers for the user (a destructive change through a connection, or one that sends a
    /// connection's secret).
    Always,
}

/// Whether a connection the PM acts through lets it act without asking (`config.pmActs`).
pub fn connection_lets_pm_act(config: &Value) -> bool {
    config.get("pmActs").and_then(Value::as_bool) == Some(true)
}

/// The target of a call to one of PDT's own asking tools, from its arguments (an agent's tool name, `mcp__pdt__<tool>`,
/// or the bare name). An integration's tools say theirs themselves (pdt-api's `Integration::ask_target`).
pub fn ask_target(tool: &str, input: &Value) -> Option<AskTarget> {
    let tool = crate::mcp::pdt_tool(tool);
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    match tool {
        "create_ticket" | "workflow_run" => field("project").map(|p| AskTarget::Project(p.into())),
        "edit_ticket" | "link_tickets" | "unlink_tickets" | "stop_ticket" => {
            field("ticket").map(|t| AskTarget::Ticket(t.into()))
        }
        "send_to_session" => field("session").map(|s| AskTarget::Session(s.into())),
        _ => None,
    }
}

/// What the briefing is made from (pdt-api gathers it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BriefingData {
    pub now: i64,
    pub projects: Vec<BriefProject>,
    pub in_flight: Vec<BriefTicket>,
    pub inbox: Vec<BriefInbox>,
    /// When the previous PM session started (None: this is the first).
    pub since: Option<i64>,
    /// Events since then, newest first.
    pub events: Vec<BriefEvent>,
    /// Newest first.
    pub journal: Vec<BriefJournal>,
    /// The journal of the conversation this one was reset from (PDT-308): the briefing leads with it.
    pub reset_from: Option<BriefJournal>,
    pub pinned: Vec<BriefNote>,
    /// The other open PM conversations (PDT-149), latest activity first.
    pub others: Vec<BriefConversation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefConversation {
    pub title: String,
    /// Its last activity.
    pub at: i64,
    /// Where it stands ([`summary_line`]); empty when nothing was said yet.
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefProject {
    pub name: String,
    pub key: String,
    pub personal: bool,
    /// Its checkout (a Personal project's folder).
    pub folder: String,
    /// (stage label, open tickets in it), stages with none left out.
    pub stages: Vec<(String, u32)>,
    pub pm_acts: bool,
    /// The enabled workflow that releases it (a Cut a release), by name; None when it has none.
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefTicket {
    pub key: String,
    pub title: String,
    pub stage: String,
    /// Branch, PR and CI, already worded ("PR #12 open, CI failed").
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefInbox {
    pub kind: String,
    pub ticket: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefEvent {
    pub at: i64,
    pub kind: String,
    /// A ticket key, when the event is about one.
    pub ticket: Option<String>,
    /// The ticket's title.
    pub title: Option<String>,
    /// The event's payload (an update words it: [`update_message`]).
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefJournal {
    pub at: i64,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefNote {
    pub id: String,
    /// The project's key; None: host-wide.
    pub project: Option<String>,
    pub title: String,
}

/// A ticket in flight in words: its branch, PR, CI, merge queue and who has it ("branch `x`; PR #4 open, CI
/// failed; Coder working").
pub fn ticket_state(t: &crate::ticket::TicketSummary) -> String {
    use crate::pr::{CiState, Mergeable, PrState};
    use crate::ticket::TicketAgentState;
    let mut parts: Vec<String> = vec![];
    match (&t.attempt, &t.parts) {
        // A parent is never started: its parts are.
        (None, Some(p)) => {
            parts.push(format!("a parent: {} of {} parts done, {} in progress", p.done, p.total, p.progress))
        }
        (Some(a), _) if a.branch.is_empty() => parts.push("started (no branch)".into()),
        (Some(a), _) => parts.push(format!("branch `{}`", a.branch)),
        (None, None) => parts.push("not started".into()),
    }
    if let Some(pr) = t.attempt.as_ref().and_then(|a| a.pr.as_ref()) {
        let state = match (pr.state, pr.draft) {
            (PrState::Open, true) => "draft",
            (PrState::Open, false) => "open",
            (PrState::Merged, _) => "merged",
            (PrState::Closed, _) => "closed",
        };
        let mut p = format!("PR #{} {state}", pr.number);
        if let Some(ci) = &pr.ci {
            let c = match ci.state {
                CiState::Passed => "passed",
                CiState::Failed => "failed",
                CiState::Running => "running",
                CiState::Queued => "queued",
                CiState::Cancelled => "cancelled",
            };
            p.push_str(&format!(", CI {c}"));
        }
        if pr.mergeable == Mergeable::Conflicting {
            p.push_str(", conflicts with its base");
        }
        if pr.in_merge_queue {
            p.push_str(", in the merge queue");
        } else if pr.auto_merge {
            p.push_str(", auto-merge on");
        }
        parts.push(p);
    }
    if let Some(a) = &t.agent {
        let who = a.persona.as_deref().unwrap_or("a gate");
        let what = match a.state {
            TicketAgentState::Working => "working",
            TicketAgentState::Asking => "waiting for the user's answers",
            TicketAgentState::Stalled => "stalled",
            TicketAgentState::Yours => "waiting for the user",
            TicketAgentState::Queued => "queued for a slot",
            TicketAgentState::Background => "waiting on background tasks",
        };
        match (a.state, a.background.as_slice()) {
            (TicketAgentState::Background, tasks @ [_, ..]) => {
                parts.push(format!("{who} waiting on background task: {}", tasks.join(", ")))
            }
            _ => parts.push(format!("{who} {what}")),
        }
    }
    if t.blocked_by > 0 {
        parts.push(format!("blocked by {}", t.blocked_by));
    }
    if t.waiting {
        parts.push("waiting until clear".into());
    }
    parts.join("; ")
}

/// Lines per briefing section.
pub const BRIEF_CAP: usize = 30;
/// Journal entries in a briefing.
pub const BRIEF_JOURNALS: usize = 3;

/// "5 min ago", "3 h ago", "2 d ago".
pub fn ago(now: i64, at: i64) -> String {
    let s = (now - at).max(0) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

fn capped<T>(items: &[T], out: &mut String, line: impl Fn(&T) -> String) {
    for i in items.iter().take(BRIEF_CAP) {
        out.push_str(&line(i));
        out.push('\n');
    }
    if items.len() > BRIEF_CAP {
        out.push_str(&format!("- … and {} more\n", items.len() - BRIEF_CAP));
    }
}

fn inbox_section(inbox: &[BriefInbox], out: &mut String) {
    if inbox.is_empty() {
        out.push_str("- Empty.\n");
    }
    capped(inbox, out, |i| match &i.ticket {
        Some(k) => format!("- {} · {k}: {}", i.kind, i.title),
        None => format!("- {}: {}", i.kind, i.title),
    });
}

/// Events, newest first: counts per kind, then the newest of them.
fn events_section(now: i64, events: &[BriefEvent], out: &mut String) {
    if events.is_empty() {
        out.push_str("- Nothing happened.\n");
        return;
    }
    let mut counts: Vec<(&str, usize)> = vec![];
    for e in events {
        match counts.iter_mut().find(|(k, _)| *k == e.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((&e.kind, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    out.push_str(&format!(
        "Counts: {}.\n\n",
        counts.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    ));
    capped(events, out, |e| match &e.ticket {
        Some(k) => format!("- {} · {} · {k}", ago(now, e.at), e.kind),
        None => format!("- {} · {}", ago(now, e.at), e.kind),
    });
}

/// What a daily update ([`update_message`]) is made from (pdt-api gathers it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateData {
    pub now: i64,
    /// When the conversation was last brought up to date: its briefing, or the last update.
    pub since: i64,
    /// Whether `since` is the briefing's time (this is the conversation's first update).
    pub first: bool,
    /// Events since then, newest first.
    pub events: Vec<BriefEvent>,
    pub inbox: Vec<BriefInbox>,
    /// Other PM conversations' journal entries written since then, newest first.
    pub journal: Vec<BriefConversationJournal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefConversationJournal {
    /// The conversation's title.
    pub title: String,
    pub at: i64,
    pub body: String,
}

/// Whether a PM conversation is due its daily update (spec §5.3, PDT-372): `after` (the PM daily checkpoint
/// workflow's, a day by default) has passed since it was last brought up to date, give or take [`UPDATE_SLACK_MS`].
/// pdt-api also wants the user to have written to it since, unless the workflow says otherwise.
pub fn update_due(caught_up_at: i64, now: i64, after: i64) -> bool {
    now - caught_up_at + UPDATE_SLACK_MS >= after
}

/// Events an update leaves out (PDT-211): the clock and overlaps, which say nothing the PM acts on, and sessions' turns
/// and Inbox items (PDT-364), which come by the dozen and which the PM sees for itself.
const UPDATE_NOISE: &[&str] = &[
    crate::workflow::events::SCHEDULE_FIRED,
    crate::workflow::events::TICKET_OVERLAP_FOUND,
    crate::workflow::events::TICKET_OVERLAP_CLEARED,
    crate::workflow::events::SESSION_TURN_ENDED,
    crate::workflow::events::SESSION_NEEDS_YOU,
    crate::workflow::events::SESSION_EXITED,
    crate::workflow::events::INBOX_CREATED,
    crate::workflow::events::INBOX_ANSWERED,
    crate::workflow::events::AGENT_STALLED,
    crate::workflow::events::AGENT_LOOP_LIMIT,
    crate::workflow::events::AGENT_TIMED_OUT,
];
/// Tickets an update lists.
pub const UPDATE_TICKETS: usize = 20;
/// Lines of a journal entry's "Still open" an update quotes.
pub const STILL_OPEN_LINES: usize = 15;

/// What an event did to its ticket, in a few words ("stage → Done", "PR #208 merged", "CI passed").
fn event_words(e: &BriefEvent) -> String {
    use crate::workflow::events as ev;
    let p = &e.payload;
    let pr = || p.get("pr").and_then(Value::as_u64).map_or("PR".to_string(), |n| format!("PR #{n}"));
    let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match e.kind.as_str() {
        ev::TICKET_STAGE_CHANGED => {
            let to = text("to").replace('_', " ");
            let mut c = to.chars();
            match c.next() {
                Some(f) => format!("stage → {}{}", f.to_uppercase(), c.as_str()),
                None => "moved".into(),
            }
        }
        ev::PR_MERGED => format!("{} merged", pr()),
        ev::PR_CONFLICT => format!("{} conflicts with its base", pr()),
        ev::PR_QUEUE_LEFT => format!("{} dropped from the merge queue", pr()),
        ev::PR_BEHIND => format!("{} behind its base", pr()),
        ev::PR_COMMENTED => format!("{} commented", pr()),
        ev::PR_REVIEWED => format!("{} reviewed", pr()),
        ev::CI_RUN_PASSED => "CI passed".into(),
        ev::CI_RUN_FAILED => "CI failed".into(),
        ev::ATTEMPT_STARTED => "started".into(),
        ev::ATTEMPT_MERGED => "merged".into(),
        ev::ATTEMPT_ABANDONED => "abandoned".into(),
        ev::ATTEMPT_LATE_COMMITS => "commits pushed after the merge".into(),
        ev::TICKET_COMMENTED => "commented".into(),
        ev::TICKET_CREATED => {
            let who = Some(text("persona")).filter(|w| !w.is_empty()).unwrap_or_else(|| text("actor"));
            format!("created by {who}")
        }
        ev::TICKET_UNBLOCKED => "unblocked".into(),
        ev::TICKET_LABELS_CHANGED => {
            let list = |k: &str, sign: char| {
                let l = p.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
                l.iter().filter_map(Value::as_str).map(|x| format!("{sign}{x}")).collect::<Vec<_>>()
            };
            format!("labels {}", [list("added", '+'), list("removed", '-')].concat().join(" "))
        }
        ev::TICKET_PRIORITY_CHANGED => format!("priority → {}", text("to")),
        ev::TICKET_PARENT_CHANGED => match text("to") {
            to if to.is_empty() => "no longer a part".into(),
            to => format!("part of {to}"),
        },
        ev::AGENT_OUTCOME => match (text("persona"), text("outcome")) {
            (who, what) if !who.is_empty() && !what.is_empty() => format!("{who} {what}"),
            _ => "an agent step ended".into(),
        },
        ev::RUN_FAILED => format!("workflow {} failed", text("workflowName")),
        ev::RUN_FINISHED => format!("workflow {} finished", text("workflowName")),
        kind => kind.to_string(),
    }
}

/// An update's events (PDT-211), newest first: the noise left out, counts per kind, then one line per ticket (the
/// most recently active first) saying what happened to it, oldest first. False when no ticket had any.
fn ticket_events_section(events: &[BriefEvent], out: &mut String) -> bool {
    let kept: Vec<&BriefEvent> = events.iter().filter(|e| !UPDATE_NOISE.contains(&e.kind.as_str())).collect();
    let mut tickets: Vec<(&str, Option<&str>, Vec<String>)> = vec![];
    for e in &kept {
        let Some(key) = e.ticket.as_deref() else { continue };
        let i = match tickets.iter().position(|(k, _, _)| *k == key) {
            Some(i) => i,
            None => {
                tickets.push((key, None, vec![]));
                tickets.len() - 1
            }
        };
        let t = &mut tickets[i];
        t.1 = t.1.or(e.title.as_deref());
        t.2.push(event_words(e));
    }
    if tickets.is_empty() {
        out.push_str("- Nothing happened to any ticket.\n");
        return false;
    }
    let mut counts: Vec<(&str, usize)> = vec![];
    for e in &kept {
        match counts.iter_mut().find(|(k, _)| *k == e.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((&e.kind, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    out.push_str(&format!(
        "Counts: {}.\n\n",
        counts.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    ));
    for (key, title, happened) in tickets.iter().take(UPDATE_TICKETS) {
        // Oldest first, and the same thing twice in a row once ("CI failed ×2").
        let mut said: Vec<(String, usize)> = vec![];
        for w in happened.iter().rev() {
            match said.last_mut() {
                Some((last, n)) if last == w => *n += 1,
                _ => said.push((w.clone(), 1)),
            }
        }
        let said =
            said.into_iter().map(|(w, n)| if n > 1 { format!("{w} ×{n}") } else { w }).collect::<Vec<_>>().join(", ");
        match title {
            Some(t) => out.push_str(&format!("- {key} {t}: {said}\n")),
            None => out.push_str(&format!("- {key}: {said}\n")),
        }
    }
    if tickets.len() > UPDATE_TICKETS {
        out.push_str(&format!("- … and {} more tickets\n", tickets.len() - UPDATE_TICKETS));
    }
    true
}

/// The "Still open" section of a journal entry (its lines, up to the next heading or section label), cut to
/// [`STILL_OPEN_LINES`] lines; None when the entry has none.
pub fn still_open(body: &str) -> Option<String> {
    let label = |l: &str| l.trim().trim_start_matches(['#', '*', '_']).trim().to_lowercase();
    let is_section = |l: &str| {
        let t = l.trim();
        t.starts_with('#') || (t.starts_with("**") && t.trim_end_matches(':').ends_with("**"))
    };
    let mut lines = body.lines();
    let first = lines.by_ref().find(|l| label(l).starts_with("still open"))?;
    let mut open: Vec<&str> = vec![];
    // "Still open: the DNS cutover" carries a thread of its own.
    if let Some((_, rest)) = first.split_once(':') {
        let rest = rest.trim().trim_start_matches(['*', '_']).trim();
        if !rest.is_empty() {
            open.push(rest);
        }
    }
    open.extend(lines.take_while(|l| !is_section(l)));
    while open.last().is_some_and(|l| l.trim().is_empty()) {
        open.pop();
    }
    let open: Vec<&str> = open.into_iter().skip_while(|l| l.trim().is_empty()).collect();
    if open.is_empty() {
        return None;
    }
    let mut text = open.iter().take(STILL_OPEN_LINES).copied().collect::<Vec<_>>().join("\n");
    if open.len() > STILL_OPEN_LINES {
        text.push_str(&format!("\n- … and {} more lines", open.len() - STILL_OPEN_LINES));
    }
    Some(text)
}

/// The daily update a PM conversation in use gets in place of ending (spec §5.3, PDT-211): what happened to tickets
/// since its briefing or last update, the Inbox, what the other conversations' new journal entries leave open, and a
/// request for a journal checkpoint. None when nothing relevant happened (no ticket events, an empty Inbox, no new
/// journal entries): the PM daily checkpoint workflow asks only for a checkpoint then ([`checkpoint_due`]).
pub fn update_message(d: &UpdateData) -> Option<String> {
    let since = if d.first { "your briefing" } else { "the last update" };
    let mut events = String::new();
    let happened = ticket_events_section(&d.events, &mut events);
    if !happened && d.inbox.is_empty() && d.journal.is_empty() {
        return None;
    }
    let mut out = format!(
        "[{SHORT_NAME}] Since {since} ({}): what changed, so this conversation stays current. Look things up with \
         your tools for more.\n\n## Tickets\n\n{events}",
        ago(d.now, d.since)
    );
    out.push_str("\n## Inbox\n\n");
    inbox_section(&d.inbox, &mut out);
    if !d.journal.is_empty() {
        out.push_str(
            "\n## New journal entries from other PM conversations\n\nWhat each leaves open; the full entries are in \
             later PM conversations' briefings (and `search_notes`).\n",
        );
        for j in d.journal.iter().take(BRIEF_JOURNALS) {
            let open = still_open(&j.body).unwrap_or_else(|| "(Nothing marked still open.)".into());
            out.push_str(&format!("\n### {} ({})\n\nStill open:\n{open}\n", j.title, ago(d.now, j.at)));
        }
    }
    out.push_str(&format!(
        "\nThen write a journal checkpoint with `write_journal`: {JOURNAL_WHAT}. Then end your turn; this \
         conversation carries on.\n"
    ));
    Some(out)
}

/// What a quiet day asks a PM conversation in use for: only a journal checkpoint.
pub fn quiet_checkpoint_request() -> String {
    format!(
        "[{SHORT_NAME}] Nothing that needs you happened since the last update. Write a journal checkpoint with \
         `write_journal`: {JOURNAL_WHAT}. Then end your turn; this conversation carries on."
    )
}

/// Whether a PM conversation is asked for a journal checkpoint ([`quiet_checkpoint_request`], PDT-211): the user wrote
/// to it since its last checkpoint (`checkpointed_at`, its journal entry's time), or it has none yet.
pub fn checkpoint_due(checkpointed_at: Option<i64>, user_wrote_at: Option<i64>) -> bool {
    user_wrote_at.is_some_and(|wrote| checkpointed_at.is_none_or(|at| wrote > at))
}

/// What PDT's own messages to a PM conversation are queued under in the outbox (`dedupe`), one waiting at a time: it
/// is also how the conversation view knows them ([`pdt_message`]).
pub const UPDATE_KEY: &str = "pm:update";
pub const CHECKPOINT_KEY: &str = "pm:checkpoint";
pub const JOURNAL_KEY: &str = "pm:journal";

/// Which of PDT's own messages to a PM conversation an outbox message queued under `dedupe` is.
pub fn pdt_message(dedupe: &str) -> Option<crate::conversation::PdtMessage> {
    use crate::conversation::PdtMessage;
    match dedupe {
        UPDATE_KEY => Some(PdtMessage::Update),
        CHECKPOINT_KEY => Some(PdtMessage::Checkpoint),
        JOURNAL_KEY => Some(PdtMessage::Journal),
        _ => None,
    }
}

/// The briefing, as Markdown.
pub fn briefing(d: &BriefingData) -> String {
    let mut out = format!(
        "# Briefing\n\nBuilt by {SHORT_NAME} when this session started. Look things up with your tools for anything \
         newer or more detailed.\n"
    );
    if let Some(j) = &d.reset_from {
        out.push_str(&format!(
            "\n## This conversation before it was reset\n\nThe user reset this conversation ({}): it carries on from \
             the journal it wrote then.\n\n{}\n",
            ago(d.now, j.at),
            j.body.trim()
        ));
    }
    out.push_str("\n## Projects\n\n");
    if d.projects.is_empty() {
        out.push_str("- None yet.\n");
    }
    capped(&d.projects, &mut out, |p| {
        let stages = if p.stages.is_empty() {
            "no open tickets".to_string()
        } else {
            p.stages.iter().map(|(s, n)| format!("{s} {n}")).collect::<Vec<_>>().join(", ")
        };
        let kind = if p.personal { " (personal, no repo)" } else { "" };
        let acts = if p.pm_acts { "; you may act here without asking" } else { "" };
        let release = p.release.as_ref().map(|w| format!("; released with the workflow `{w}`")).unwrap_or_default();
        format!("- {} (`{}`) in `{}`{kind}: {stages}{acts}{release}", p.name, p.key, p.folder)
    });
    if d.projects.iter().any(|p| p.release.is_some()) {
        out.push_str(
            "\nReleases happen only when the user asks for one: then start the project's release workflow (named \
             above) with `workflow_run`, the project and the inputs it declares (`workflow_get`'s runInputs). Never \
             run the forge's own release workflow directly. A project without one has no release process in PDT: ask \
             the user how they release it.\n",
        );
    }
    out.push_str("\n## In flight\n\n");
    if d.in_flight.is_empty() {
        out.push_str("- Nothing in progress or in review.\n");
    }
    capped(&d.in_flight, &mut out, |t| format!("- {} {} ({}): {}", t.key, t.title, t.stage, t.state));
    out.push_str("\n## Inbox\n\n");
    inbox_section(&d.inbox, &mut out);
    match d.since {
        Some(since) => out.push_str(&format!("\n## Since the last PM session ({})\n\n", ago(d.now, since))),
        None => out.push_str("\n## Recent events\n\n"),
    }
    events_section(d.now, &d.events, &mut out);
    if !d.journal.is_empty() {
        out.push_str("\n## Journal (from earlier and other PM conversations, newest first)\n");
        for j in d.journal.iter().take(BRIEF_JOURNALS) {
            out.push_str(&format!("\n### {}\n\n{}\n", ago(d.now, j.at), j.body.trim()));
        }
    }
    if !d.others.is_empty() {
        out.push_str(
            "\n## Other open PM conversations\n\nThe user talks to the PM in several conversations, one per topic; \
             this is one of them. The others have their own context: only notes and journal entries are shared.\n\n",
        );
        capped(&d.others, &mut out, |c| match c.summary.as_str() {
            "" => format!("- {} ({})", c.title, ago(d.now, c.at)),
            s => format!("- {} ({}): {s}", c.title, ago(d.now, c.at)),
        });
    }
    out.push_str("\n## Pinned notes\n\n");
    if d.pinned.is_empty() {
        out.push_str("- None. Save what you learn about the user with `save_note`.\n");
    }
    capped(&d.pinned, &mut out, |n| match &n.project {
        Some(p) => format!("- `{}` · {p}: {}", n.id, n.title),
        None => format!("- `{}` · host-wide: {}", n.id, n.title),
    });
    out
}

/// The PM's working folder (PDT-100): the deepest folder holding the projects' folders (each checkout's parent) and
/// the Projects folder new clones go in. None when they share nothing below the file system's root (or there are
/// none): the caller picks a fallback.
pub fn work_folder<'a>(
    checkouts: impl IntoIterator<Item = &'a Path>,
    projects_dir: Option<&'a Path>,
) -> Option<PathBuf> {
    let mut common: Option<PathBuf> = None;
    for dir in checkouts.into_iter().filter_map(Path::parent).chain(projects_dir) {
        common = Some(match common {
            None => dir.to_path_buf(),
            Some(c) => c.components().zip(dir.components()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect(),
        });
    }
    common.filter(|c| c.components().any(|x| matches!(x, Component::Normal(_))))
}

/// Where a PM session runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmContext<'a> {
    pub session_id: &'a str,
    /// pdtd's port, for its API and guides.
    pub port: u16,
    /// Where the session runs ([`work_folder`]).
    pub work_folder: &'a str,
    /// pdt-mcp tools the session has.
    pub tools: &'a [String],
    /// Every pdt-mcp tool PDT has (which of them ask).
    pub tool_table: &'a crate::integration::ToolTable,
    /// The persona's instructions.
    pub persona: &'a str,
    /// What the integrations whose tools the session has say about them (pdt-api's `Integration::pm_prompt`), as
    /// Markdown list items.
    pub integrations: &'a [&'a str],
    /// A workflow step's section (`workflow::agent::step_section`, PDT-152): the role and its outcome rules, in place
    /// of the persona's instructions.
    pub step: Option<&'a str>,
    /// [`briefing`]'s text.
    pub briefing: &'a str,
}

/// A PM session's instructions: who it is, its tools, the persona's instructions and the briefing.
pub fn instructions(ctx: &PmContext) -> String {
    let tools = |asking: bool| {
        ctx.tools
            .iter()
            .filter(|t| ctx.tool_table.asks(t) == asking)
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let asking = tools(true);
    let mut out = format!(
        "# {PRODUCT_NAME}'s project manager\n\n\
         {SHORT_NAME} started this session (`{session}`) as the user's project manager (the PM). It is not in any \
         one project: it works across every project {SHORT_NAME} has, from the work folder `{work}`, which holds \
         their checkouts (the briefing gives each project's folder).\n\
         - {SHORT_NAME}'s tools, on its MCP server `{server}`, are how you see and change {SHORT_NAME}: {read}.\n\
         - Your agent's own tools are yours too, with its usual permission prompts: read and search any project's \
         checkout and worktrees (`git log`, `git show`, `git diff <base>...<branch>` for what an attempt changed), \
         search and read the web, run commands, install tools, and edit files.\n\
         - {SHORT_NAME}'s API is http://127.0.0.1:{port}/api; its guides for agents (tickets, runs, a project's \
         `.pdt/project.toml`) are listed at `GET http://127.0.0.1:{port}/api/guides`.\n",
        session = ctx.session_id,
        work = ctx.work_folder,
        port = ctx.port,
        server = crate::mcp::SERVER_NAME,
        read = tools(false),
    );
    if !asking.is_empty() {
        out.push_str(&format!(
            "- These act for the user, so each call waits for their yes (they see the call and its arguments) unless \
             the project (for an integration's tools, their connection) lets you act without asking: {asking}. A refusal is an \
             answer: don't retry it unchanged.\n"
        ));
    }
    let role = match ctx.step {
        Some(step) => step.trim().to_string(),
        None => format!("## Your role\n\n{}", ctx.persona.trim()),
    };
    out.push_str(&format!("- Messages starting `[{SHORT_NAME}]` come from {SHORT_NAME}, not the user.\n\n{role}\n\n"));
    if !ctx.integrations.is_empty() {
        let items: Vec<&str> = ctx.integrations.iter().map(|t| t.trim()).collect();
        out.push_str(&format!("## Integrations\n\n{}\n\n", items.join("\n")));
    }
    out.push_str(ctx.briefing.trim_end());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const H: i64 = 3_600_000;

    #[test]
    fn a_daily_update_is_due_after_a_day_of_use() {
        let day = 24 * H;
        assert!(!update_due(0, 23 * H, day));
        assert!(update_due(0, 24 * H, day));
        assert!(update_due(H, 25 * H, day) && !update_due(H, 24 * H, day), "a day from the last update");
        // An hourly schedule's run a day later fires a moment before a full day has passed since the last update.
        assert!(update_due(5_000, 24 * H, day), "a schedule's next day");
        assert!(update_due(0, 0, 0), "no wait");
    }

    fn event(at: i64, kind: &str, ticket: Option<(&str, &str)>, payload: Value) -> BriefEvent {
        BriefEvent {
            at,
            kind: kind.into(),
            ticket: ticket.map(|t| t.0.to_string()),
            title: ticket.map(|t| t.1.to_string()),
            payload,
        }
    }

    #[test]
    fn ticket_changes_read_as_a_few_words() {
        let words = |kind: &str, p: Value| event_words(&event(1, kind, None, p));
        use crate::workflow::events as ev;
        assert_eq!(words(ev::TICKET_CREATED, json!({"actor": "agent", "persona": "Coder"})), "created by Coder");
        assert_eq!(words(ev::TICKET_CREATED, json!({"actor": "user", "persona": null})), "created by user");
        assert_eq!(words(ev::TICKET_UNBLOCKED, json!({})), "unblocked");
        assert_eq!(words(ev::RUN_FAILED, json!({"workflowName": "Pipeline"})), "workflow Pipeline failed");
        assert_eq!(words(ev::TICKET_LABELS_CHANGED, json!({"added": ["ui"], "removed": ["docs"]})), "labels +ui -docs");
        assert_eq!(words(ev::TICKET_PRIORITY_CHANGED, json!({"from": "low", "to": "high"})), "priority → high");
        assert_eq!(words(ev::TICKET_PARENT_CHANGED, json!({"to": "PDT-2"})), "part of PDT-2");
        assert_eq!(words(ev::TICKET_PARENT_CHANGED, json!({"to": ""})), "no longer a part");
    }

    fn update() -> UpdateData {
        UpdateData {
            now: 25 * H,
            since: H,
            first: true,
            events: vec![event(2 * H, "ticket.moved", Some(("PDT-3", "Login")), json!({}))],
            inbox: vec![BriefInbox { kind: "approval".into(), ticket: None, title: "Merge it?".into() }],
            journal: vec![BriefConversationJournal {
                title: "CB cutover".into(),
                at: 3 * H,
                body: "## Decided\n\nCut over Friday.\n\n## Still open\n\n- DNS for cb.example\n".into(),
            }],
        }
    }

    #[test]
    fn checkpoints_and_updates_keep_the_conversation_going() {
        let full = update_message(&update()).unwrap();
        for text in [checkpoint_request(), quiet_checkpoint_request(), full.clone()] {
            assert!(text.starts_with("[PDT] ") && text.contains("`write_journal`"), "{text}");
            assert!(text.contains("this conversation carries on") && text.contains("`Still open` heading"), "{text}");
        }
        assert!(checkpoint_request().contains("compacted"));
        assert!(journal_request().contains("closed this PM conversation"));
        for want in [
            "[PDT] Since your briefing (1 d ago)",
            "Counts: ticket.moved 1.",
            "- PDT-3 Login: ticket.moved",
            "- approval: Merge it?",
            "### CB cutover (22 h ago)\n\nStill open:\n- DNS for cb.example\n",
            "the full entries are in later PM conversations' briefings",
        ] {
            assert!(full.contains(want), "missing {want:?} in\n{full}");
        }
        assert!(!full.contains("Cut over Friday"), "only what is still open: {full}");
        let later = update_message(&UpdateData { first: false, journal: vec![], ..update() }).unwrap();
        assert!(later.starts_with("[PDT] Since the last update") && !later.contains("New journal entries"), "{later}");
    }

    #[test]
    fn an_update_leaves_out_the_noise_and_groups_events_by_ticket() {
        let pdt208 = Some(("PDT-208", "Phone Inbox"));
        let d = UpdateData {
            // Newest first.
            events: vec![
                event(9 * H, "schedule.fired", None, json!({})),
                event(8 * H, "ci.run_passed", pdt208, json!({"pr": 208})),
                event(7 * H, "ticket.overlap_cleared", Some(("PDT-9", "Other")), json!({})),
                event(6 * H, "pr.merged", pdt208, json!({"pr": 208})),
                event(5 * H, "ticket.stage_changed", pdt208, json!({"from": "in_progress", "to": "done"})),
                event(
                    4 * H,
                    "agent.outcome",
                    Some(("PDT-5", "Docs")),
                    json!({"persona": "Coder", "outcome": "submitted"}),
                ),
                event(3 * H, "ci.run_failed", pdt208, json!({"pr": 208})),
                event(2 * H, "ci.run_failed", pdt208, json!({"pr": 208})),
                event(2 * H, "github.issue.opened", None, json!({})),
            ],
            inbox: vec![],
            journal: vec![],
            ..update()
        };
        let text = update_message(&d).unwrap();
        for want in [
            "Counts: ci.run_failed 2, agent.outcome 1, ci.run_passed 1, github.issue.opened 1, pr.merged 1, \
             ticket.stage_changed 1.",
            "- PDT-208 Phone Inbox: CI failed ×2, stage → Done, PR #208 merged, CI passed\n- PDT-5 Docs: Coder submitted\n",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
        for noise in ["schedule.fired", "overlap", "PDT-9"] {
            assert!(!text.contains(noise), "{noise} in\n{text}");
        }
        let many = UpdateData {
            events: (0..UPDATE_TICKETS + 2)
                .map(|i| event(2 * H, "ticket.commented", Some((&format!("PDT-{i}"), "T")), json!({})))
                .collect(),
            ..d
        };
        let text = update_message(&many).unwrap();
        assert!(text.contains("- … and 2 more tickets") && !text.contains("PDT-21 "), "{text}");
    }

    #[test]
    fn journals_are_cut_to_what_is_still_open() {
        assert_eq!(
            still_open("Decided things.\n\n**Still open:**\n- a\n- b\n\n**Next**\n- c").as_deref(),
            Some("- a\n- b")
        );
        assert_eq!(still_open("Still open: the DNS cutover\n").as_deref(), Some("the DNS cutover"));
        assert_eq!(still_open("## Decided\n- x\n"), None);
        let long = format!("### Still open\n{}", (0..20).map(|i| format!("- t{i}")).collect::<Vec<_>>().join("\n"));
        let cut = still_open(&long).unwrap();
        assert_eq!(cut.lines().count(), STILL_OPEN_LINES + 1);
        assert!(cut.ends_with("- … and 5 more lines") && cut.contains("- t14") && !cut.contains("- t15"), "{cut}");
        let d = UpdateData {
            journal: vec![BriefConversationJournal { title: "Plain".into(), at: 3 * H, body: "Just notes.".into() }],
            ..update()
        };
        let text = update_message(&d).unwrap();
        assert!(
            text.contains("### Plain (22 h ago)\n\nStill open:\n(Nothing marked still open.)")
                && !text.contains("Just notes")
        );
    }

    #[test]
    fn a_quiet_day_asks_only_for_a_checkpoint_or_nothing() {
        let quiet = UpdateData {
            events: vec![
                event(2 * H, "schedule.fired", None, json!({})),
                event(3 * H, "github.issue.opened", None, json!({})),
            ],
            inbox: vec![],
            journal: vec![],
            ..update()
        };
        assert_eq!(update_message(&quiet), None, "nothing relevant: no update");
        let wrote = Some(24 * H);
        assert!(checkpoint_due(None, wrote), "no checkpoint yet");
        assert!(checkpoint_due(Some(23 * H), wrote), "the user wrote since");
        assert!(!checkpoint_due(Some(24 * H + 1), wrote), "the checkpoint is newer than every change");
        assert!(!checkpoint_due(None, None), "the user never wrote to it");
    }

    #[test]
    fn pdts_own_messages_are_known_by_their_outbox_key() {
        use crate::conversation::PdtMessage;
        assert_eq!(pdt_message(UPDATE_KEY), Some(PdtMessage::Update));
        assert_eq!(pdt_message(CHECKPOINT_KEY), Some(PdtMessage::Checkpoint));
        assert_eq!(pdt_message(JOURNAL_KEY), Some(PdtMessage::Journal));
        assert_eq!(pdt_message("prompt:x"), None);
    }

    #[test]
    fn fts_queries_quote_every_word_as_a_prefix() {
        assert_eq!(fts_query("  ").as_deref(), None);
        assert_eq!(fts_query("Deploy key").as_deref(), Some("\"deploy\"* \"key\"*"));
        assert_eq!(fts_query("a\"b OR c*").as_deref(), Some("\"a\"* \"b\"* \"or\"* \"c\"*"));
    }

    #[test]
    fn note_kinds_round_trip_and_the_users_are_not_the_pms() {
        for k in [NoteKind::Note, NoteKind::Journal, NoteKind::User] {
            assert_eq!(NoteKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(NoteKind::parse("idea"), None);
        assert!(NoteKind::Note.pm_may_write() && NoteKind::Journal.pm_may_write());
        assert!(!NoteKind::User.pm_may_write());
    }

    #[test]
    fn notes_are_checked() {
        assert_eq!(valid_title("  Work hours ").unwrap(), "Work hours");
        assert!(valid_title(" ").is_err() && valid_title("a\nb").is_err());
        assert!(valid_body(&"x".repeat(MAX_BODY + 1)).is_err());
        assert_eq!(valid_body("text\n\n").unwrap(), "text");
        assert!(valid_journal("  ").is_err());
    }

    #[test]
    fn asking_tools_name_what_they_act_on() {
        let t = |tool: &str, v: Value| ask_target(tool, &v);
        assert_eq!(t("mcp__pdt__create_ticket", json!({"project": "PDT"})), Some(AskTarget::Project("PDT".into())));
        assert_eq!(t("edit_ticket", json!({"ticket": " PDT-4 "})), Some(AskTarget::Ticket("PDT-4".into())));
        assert_eq!(t("stop_ticket", json!({"ticket": "PDT-4"})), Some(AskTarget::Ticket("PDT-4".into())));
        assert_eq!(t("link_tickets", json!({"ticket": "PDT-5"})), Some(AskTarget::Ticket("PDT-5".into())));
        assert_eq!(t("mcp__pdt__unlink_tickets", json!({"ticket": "PDT-5"})), Some(AskTarget::Ticket("PDT-5".into())));
        assert_eq!(t("send_to_session", json!({"session": "s1"})), Some(AskTarget::Session("s1".into())));
        assert_eq!(t("create_ticket", json!({})), None);
        assert_eq!(t("mcp:pdt/stop_ticket", json!({"ticket": "PDT-4"})), Some(AskTarget::Ticket("PDT-4".into())));
        assert_eq!(t("get_ticket", json!({"ticket": "PDT-4"})), None);
    }

    #[test]
    fn a_ticket_in_flight_reads_as_words() {
        use crate::pr::{CiState, CiStatus, Mergeable};
        use crate::ticket::{AttemptState, AttemptSummary, TicketAgent, TicketAgentState, TicketStage};
        let mut t = crate::policy::tests::ticket("1", TicketStage::InProgress);
        assert_eq!(ticket_state(&t), "not started");
        let mut pr = crate::pr::tests::pr();
        pr.ci = Some(CiStatus { state: CiState::Failed, url: String::new(), runs: vec![] });
        pr.mergeable = Mergeable::Conflicting;
        t.attempt = Some(AttemptSummary {
            id: "a".into(),
            branch: "pdt-P-1-x".into(),
            base_branch: "main".into(),
            worktree_id: None,
            state: AttemptState::Active,
            created_at: 0,
            ended_at: None,
            last_head: None,
            workspace_id: None,
            pr: Some(pr),
            ci: None,
            merge_update: None,
        });
        t.agent = Some(TicketAgent {
            persona: Some("Coder".into()),
            state: TicketAgentState::Stalled,
            session_id: None,
            busy: false,
            run_id: None,
            background: vec![],
        });
        t.blocked_by = 2;
        assert_eq!(
            ticket_state(&t),
            "branch `pdt-P-1-x`; PR #12 open, CI failed, conflicts with its base, auto-merge on; Coder stalled; \
             blocked by 2"
        );
    }

    fn data() -> BriefingData {
        BriefingData {
            now: 10 * H,
            projects: vec![BriefProject {
                name: "Personal Dev Team".into(),
                key: "PDT".into(),
                personal: false,
                folder: "/w/pdt".into(),
                stages: vec![("In progress".into(), 2), ("To do".into(), 5)],
                pm_acts: true,
                release: None,
            }],
            in_flight: vec![BriefTicket {
                key: "PDT-81".into(),
                title: "PM persona".into(),
                stage: "In progress".into(),
                state: "PR #105 open, CI failed".into(),
            }],
            inbox: vec![BriefInbox { kind: "question".into(), ticket: Some("PDT-81".into()), title: "Which?".into() }],
            since: Some(8 * H),
            events: vec![
                event(9 * H, "pr.ci_failed", Some(("PDT-81", "PM persona")), json!({})),
                event(8 * H + 1, "ticket.stage_changed", Some(("PDT-80", "Notes")), json!({})),
                event(8 * H + 2, "pr.ci_failed", None, json!({})),
            ],
            journal: vec![BriefJournal { at: 8 * H, body: "Talked about the PM.\n".into() }],
            reset_from: None,
            pinned: vec![BriefNote { id: "n1".into(), project: None, title: "Prefers short answers".into() }],
            others: vec![
                BriefConversation { title: "CB cutover".into(), at: 9 * H, summary: "Waiting on DNS.".into() },
                BriefConversation { title: "Fresh".into(), at: 10 * H, summary: String::new() },
            ],
        }
    }

    #[test]
    fn a_conversation_is_titled_from_its_first_message() {
        assert_eq!(conversation_title("\n## Plan the CB cutover\nmore"), "Plan the CB cutover");
        assert_eq!(conversation_title("  "), NEW_TITLE);
        let long = "Why does the merge queue keep dropping the pull request for the login page after a rebase";
        let t = conversation_title(long);
        assert_eq!(t, "Why does the merge queue keep dropping the pull request…");
        assert!(t.chars().count() <= AUTO_TITLE_CHARS);
        let word = "x".repeat(80);
        assert_eq!(conversation_title(&word).chars().count(), AUTO_TITLE_CHARS, "one long word is cut anyway");
        assert_eq!(summary_line("- Done: the PR merged.\n\nNext"), "Done: the PR merged.");
    }

    #[test]
    fn the_briefing_has_every_section() {
        let b = briefing(&data());
        for want in [
            "- Personal Dev Team (`PDT`) in `/w/pdt`: In progress 2, To do 5; you may act here without asking",
            "- PDT-81 PM persona (In progress): PR #105 open, CI failed",
            "- question · PDT-81: Which?",
            "## Since the last PM session (2 h ago)",
            "Counts: pr.ci_failed 2, ticket.stage_changed 1.",
            "- 1 h ago · pr.ci_failed · PDT-81",
            "### 2 h ago\n\nTalked about the PM.\n",
            "- `n1` · host-wide: Prefers short answers",
            "## Other open PM conversations",
            "- CB cutover (1 h ago): Waiting on DNS.",
            "- Fresh (just now)\n",
        ] {
            assert!(b.contains(want), "missing {want:?} in\n{b}");
        }
    }

    /// PDT-308: a reset conversation's briefing leads with its journal from before the reset, ahead of the others.
    #[test]
    fn a_reset_conversation_starts_from_its_own_journal_first() {
        let mut d = data();
        assert!(!briefing(&d).contains("before it was reset"));
        d.reset_from = Some(BriefJournal { at: 9 * H, body: "We were planning the cutover.\n".into() });
        let b = briefing(&d);
        let own = b.find("## This conversation before it was reset").expect(&b);
        assert!(
            b[own..].contains("(1 h ago): it carries on from the journal it wrote then.\n\nWe were planning"),
            "{b}"
        );
        assert!(own < b.find("## Projects").unwrap() && own < b.find("## Journal").unwrap(), "first:\n{b}");
        assert_eq!(RotateReason::parse("reset"), Some(RotateReason::Reset));
        assert!(reset_request().contains("`write_journal`") && reset_request().contains("same name"));
    }

    /// Release guidance only for a project with a release workflow (PDT-296): a PM elsewhere gets none.
    #[test]
    fn only_a_project_with_a_release_workflow_gets_release_guidance() {
        let mut d = data();
        let b = briefing(&d);
        assert!(!b.contains("release"), "no release workflow, no release guidance:\n{b}");
        d.projects[0].release = Some("Cut a release".into());
        let b = briefing(&d);
        assert!(b.contains("you may act here without asking; released with the workflow `Cut a release`"), "{b}");
        assert!(b.contains("Releases happen only when the user asks for one") && b.contains("`workflow_run`"), "{b}");
        assert!(b.contains("Never run the forge's own release workflow directly"), "{b}");
    }

    #[test]
    fn an_empty_briefing_says_so_and_long_sections_are_capped() {
        let b = briefing(&BriefingData { now: 1, ..Default::default() });
        assert!(b.contains("- None yet.") && b.contains("## Recent events") && b.contains("- Nothing happened."));
        assert!(!b.contains("## Journal") && !b.contains("## Other open PM conversations"));
        let mut d = data();
        d.inbox = (0..BRIEF_CAP + 3)
            .map(|i| BriefInbox { kind: "gate".into(), ticket: None, title: format!("g{i}") })
            .collect();
        assert!(briefing(&d).contains("- … and 3 more"));
    }

    #[test]
    fn instructions_split_the_tools_that_ask() {
        let tools: Vec<String> = ["get_overview", "create_ticket"].map(String::from).to_vec();
        use crate::integration::{ToolDecl, ToolTable};
        let table =
            ToolTable::new([ToolDecl::new("get_overview", "").read_only(), ToolDecl::new("create_ticket", "").asks()]);
        let text = instructions(&PmContext {
            session_id: "s1",
            port: 7421,
            work_folder: "/w",
            tools: &tools,
            tool_table: &table,
            persona: "You are the PM.",
            integrations: &[],
            briefing: "# Briefing\n",
            step: None,
        });
        assert!(text.contains("are how you see and change PDT: `get_overview`."), "{text}");
        assert!(text.contains("lets you act without asking: `create_ticket`."), "{text}");
        assert!(text.contains("## Your role\n\nYou are the PM.\n\n# Briefing\n"), "{text}");
        for want in
            ["from the work folder `/w`", "search and read the web", "install tools", "127.0.0.1:7421/api/guides"]
        {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
        // A workflow step's session (PDT-152): the step's role and outcome rules instead of the persona's text.
        let step = instructions(&PmContext {
            session_id: "s1",
            port: 7421,
            work_folder: "/w",
            tools: &tools,
            tool_table: &table,
            persona: "You are the PM.",
            integrations: &[],
            briefing: "# Briefing\n",
            step: Some("\n# Your role: PM\n\nFinish by calling `submit_outcome`.\n"),
        });
        assert!(
            step.contains("not the user.\n\n# Your role: PM\n\nFinish by calling `submit_outcome`.\n\n# Briefing\n"),
            "{step}"
        );
        assert!(!step.contains("## Your role"), "{step}");
        // PDT-175: what the session's integrations say about their tools, between the role and the briefing.
        let with = instructions(&PmContext {
            session_id: "s1",
            port: 7421,
            work_folder: "/w",
            tools: &tools,
            tool_table: &table,
            persona: "You are the PM.",
            integrations: &["- Lights: `blink` blinks them.\n", "- Doors: `lock` locks them."],
            briefing: "# Briefing\n",
            step: None,
        });
        assert!(
            with.contains(
                "You are the PM.\n\n## Integrations\n\n- Lights: `blink` blinks them.\n- Doors: `lock` locks them.\n\n\
                 # Briefing\n"
            ),
            "{with}"
        );
        assert!(!text.contains("## Integrations"), "none without integrations: {text}");
    }

    #[test]
    fn the_work_folder_holds_every_project() {
        let root = std::env::temp_dir();
        let (w, a, b) = (root.join("work"), root.join("work").join("a"), root.join("work").join("team").join("b"));
        assert_eq!(work_folder([a.as_path()], None), Some(w.clone()), "a checkout's parent");
        assert_eq!(work_folder([a.as_path(), b.as_path()], None), Some(w.clone()));
        assert_eq!(work_folder([b.as_path()], Some(&w)), Some(w.clone()), "and the Projects folder");
        assert_eq!(work_folder([], Some(&w)), Some(w.clone()));
        let other = root.join("elsewhere").join("c");
        assert_eq!(work_folder([a.as_path(), other.as_path()], None), Some(root.clone()));
        assert_eq!(work_folder([], None), None);
        let top = root.ancestors().last().unwrap().to_path_buf();
        assert_eq!(work_folder([a.as_path()], Some(&top)), None, "nothing shared below the root");
    }
}
