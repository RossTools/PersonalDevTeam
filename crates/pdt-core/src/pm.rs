//! The project manager (PDT-81, docs/superpowers/specs/2026-10-21-pm-persona-design.md): a host-level conversation
//! with PDT's tools only. PDT owns its memory: the briefing every new PM session starts with (rendered here from data
//! pdt-api gathers), notes searched with FTS5, journal checkpoints and daily updates. Pure.

use crate::brand::{PRODUCT_NAME, SHORT_NAME};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use ts_rs::TS;

/// How often a PM conversation that is in use is brought up to date ([`update_message`]) and asked for a journal
/// checkpoint (ms).
pub const UPDATE_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

/// What the journal is for, said in every request for it.
const JOURNAL_WHAT: &str = "what you and the user are working on, what was decided, and the threads still open. It \
                            replaces this conversation's earlier entry, so write the whole picture; later PM \
                            conversations start from it";

/// What PDT sends a PM conversation the user closed (spec §5.3).
pub fn journal_request() -> String {
    format!(
        "[{SHORT_NAME}] The user closed this PM conversation, so it is ending. Write its journal entry with \
         `write_journal`: {JOURNAL_WHAT}. Then end your turn."
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
}

impl RotateReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RotateReason::Context => "context",
            RotateReason::Age => "age",
            RotateReason::User => "user",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RotateReason::Context, RotateReason::Age, RotateReason::User].into_iter().find(|r| r.as_str() == s)
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
    /// The user's own note (PDT-126): never in a briefing, and the PM can find it but not write or delete it.
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
        "create_ticket" => field("project").map(|p| AskTarget::Project(p.into())),
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

/// Whether a PM conversation that just ended a turn is due its daily update (spec §5.3): `every` (normally
/// [`UPDATE_EVERY_MS`]) has passed since it was last brought up to date. pdt-api also wants the user to have written
/// to it since.
pub fn update_due(caught_up_at: i64, now: i64, every: i64) -> bool {
    now - caught_up_at >= every
}

/// The daily update a PM conversation in use gets in place of ending (spec §5.3): what happened since its briefing
/// or last update, the Inbox, the other conversations' new journal entries, and a request for a journal checkpoint.
pub fn update_message(d: &UpdateData) -> String {
    let since = if d.first { "your briefing" } else { "the last update" };
    let mut out = format!(
        "[{SHORT_NAME}] Since {since} ({}): what changed, so this conversation stays current. Look things up with \
         your tools for more.\n\n## Events\n\n",
        ago(d.now, d.since)
    );
    events_section(d.now, &d.events, &mut out);
    out.push_str("\n## Inbox\n\n");
    inbox_section(&d.inbox, &mut out);
    if !d.journal.is_empty() {
        out.push_str("\n## New journal entries from other PM conversations\n");
        for j in d.journal.iter().take(BRIEF_JOURNALS) {
            out.push_str(&format!("\n### {} ({})\n\n{}\n", j.title, ago(d.now, j.at), j.body.trim()));
        }
    }
    out.push_str(&format!(
        "\nThen write a journal checkpoint with `write_journal`: {JOURNAL_WHAT}. Then end your turn; this \
         conversation carries on.\n"
    ));
    out
}

/// The briefing, as Markdown.
pub fn briefing(d: &BriefingData) -> String {
    let mut out = format!(
        "# Briefing\n\nBuilt by {SHORT_NAME} when this session started. Look things up with your tools for anything \
         newer or more detailed.\n"
    );
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
        format!("- {} (`{}`) in `{}`{kind}: {stages}{acts}", p.name, p.key, p.folder)
    });
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
        let day = UPDATE_EVERY_MS;
        assert!(!update_due(0, 24 * H - 1, day));
        assert!(update_due(0, 24 * H, day));
        assert!(update_due(H, 25 * H, day) && !update_due(H, 24 * H, day), "a day from the last update");
    }

    #[test]
    fn checkpoints_and_updates_keep_the_conversation_going() {
        for text in [checkpoint_request(), update_message(&UpdateData::default())] {
            assert!(text.starts_with("[PDT] ") && text.contains("`write_journal`"), "{text}");
            assert!(text.contains("this conversation carries on"), "{text}");
        }
        assert!(checkpoint_request().contains("compacted"));
        assert!(journal_request().contains("closed this PM conversation"));
        let d = UpdateData {
            now: 25 * H,
            since: H,
            first: true,
            events: vec![BriefEvent { at: 2 * H, kind: "ticket.moved".into(), ticket: Some("PDT-3".into()) }],
            inbox: vec![BriefInbox { kind: "approval".into(), ticket: None, title: "Merge it?".into() }],
            journal: vec![BriefConversationJournal {
                title: "CB cutover".into(),
                at: 3 * H,
                body: "Cut over Friday.".into(),
            }],
        };
        let text = update_message(&d);
        for want in [
            "[PDT] Since your briefing (1 d ago)",
            "Counts: ticket.moved 1.",
            "- 23 h ago · ticket.moved · PDT-3",
            "- approval: Merge it?",
            "### CB cutover (22 h ago)\n\nCut over Friday.",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
        let later = update_message(&UpdateData { first: false, journal: vec![], ..d });
        assert!(later.starts_with("[PDT] Since the last update") && !later.contains("New journal entries"), "{later}");
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
                BriefEvent { at: 9 * H, kind: "pr.ci_failed".into(), ticket: Some("PDT-81".into()) },
                BriefEvent { at: 8 * H + 1, kind: "ticket.stage_changed".into(), ticket: Some("PDT-80".into()) },
                BriefEvent { at: 8 * H + 2, kind: "pr.ci_failed".into(), ticket: None },
            ],
            journal: vec![BriefJournal { at: 8 * H, body: "Talked about the PM.\n".into() }],
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
