//! The project manager (PDT-81, docs/superpowers/specs/2026-10-21-pm-persona-design.md): a host-level conversation
//! with PDT's tools only. PDT owns its memory: the briefing every new PM session starts with (rendered here from data
//! pdt-api gathers), notes searched with FTS5, and rotation with a journal entry. Pure.

use crate::brand::{PRODUCT_NAME, SHORT_NAME};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use ts_rs::TS;

/// A PM session rotates once its context is this full (used / window)…
pub const ROTATE_CONTEXT: f64 = 0.6;
/// …or this old (ms).
pub const ROTATE_AGE_MS: i64 = 24 * 60 * 60 * 1000;

/// What PDT sends a PM session that is about to rotate.
pub fn journal_request() -> String {
    format!(
        "[{SHORT_NAME}] This PM session is ending. Write a short journal entry with `write_journal`: what you and the \
         user were working on, what was decided, and the threads still open. The next PM session starts from it. \
         Then end your turn."
    )
}

/// The journal recorded when a rotating session wrote none.
pub const NO_JOURNAL: &str = "(no journal)";

const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 16 * 1024;
const MAX_JOURNAL: usize = 8 * 1024;

/// Why a PM session rotated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RotateReason {
    /// Its context reached [`ROTATE_CONTEXT`].
    Context,
    /// It reached [`ROTATE_AGE_MS`].
    Age,
    /// The user started a new session.
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

/// Whether a PM session that just ended a turn should rotate: its context use (`used`, `window`) or its age
/// (`max_age`, normally [`ROTATE_AGE_MS`]).
pub fn rotate_reason(started_at: i64, now: i64, context: Option<(i64, i64)>, max_age: i64) -> Option<RotateReason> {
    if let Some((used, window)) = context
        && window > 0
        && used as f64 / window as f64 >= ROTATE_CONTEXT
    {
        return Some(RotateReason::Context);
    }
    (now - started_at >= max_age).then_some(RotateReason::Age)
}

/// A PM session's rotation state (`pm_sessions.rotation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Rotation {
    /// In use.
    None,
    /// Asked for its journal; it ends when that turn ends.
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
    /// Written by a PM session as it rotates.
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
    /// `note` (the PM's, the default) or `user`; journal entries are only written by rotation.
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
    /// The PM session in use; None until the user next writes to the PM.
    pub current: Option<Session>,
    /// Every PM session, newest first (the current one included).
    pub sessions: Vec<PmSessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PmSessionInfo {
    pub session: Session,
    pub rotation: Rotation,
    pub reason: Option<RotateReason>,
    /// The journal entry it wrote as it ended.
    pub journal: Option<String>,
}

/// `POST /api/pm/sessions`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartPm {
    /// The new session's first message.
    #[serde(default)]
    #[ts(optional)]
    pub text: Option<String>,
}

/// What an asking tool's call acts on, so the project's policy can be found (PDT-81 spec §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskTarget {
    /// A project id or key.
    Project(String),
    /// A ticket id or key.
    Ticket(String),
    Session(String),
    /// A connection of `kind` (`discord_bot`, `scryer`) by name (None: the only one of its kind), whose
    /// `config.pmActs` says whether the PM acts without asking.
    Connection {
        kind: &'static str,
        name: Option<String>,
    },
    /// A call no policy answers for the user: a destructive Scryer change, or one that sends a connection's secret.
    Always,
}

/// Whether a connection the PM acts through (a Discord bot, a Scryer) lets it act without asking (`config.pmActs`).
pub fn connection_lets_pm_act(config: &Value) -> bool {
    config.get("pmActs").and_then(Value::as_bool) == Some(true)
}

/// The target of an asking tool's input, from its arguments (Claude's tool name `mcp__pdt__<tool>` or the bare name).
pub fn ask_target(tool: &str, input: &Value) -> Option<AskTarget> {
    let tool = tool.strip_prefix(&format!("mcp__{}__", crate::mcp::SERVER_NAME)).unwrap_or(tool);
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    match tool {
        "create_ticket" => field("project").map(|p| AskTarget::Project(p.into())),
        "edit_ticket" | "link_tickets" | "unlink_tickets" => field("ticket").map(|t| AskTarget::Ticket(t.into())),
        "send_to_session" => field("session").map(|s| AskTarget::Session(s.into())),
        "discord_change" | "discord_webhook" => Some(AskTarget::Connection {
            kind: crate::discord::DISCORD_BOT,
            name: field("connection").map(String::from),
        }),
        "scryer_add" => {
            Some(AskTarget::Connection { kind: crate::scryer::SCRYER, name: field("connection").map(String::from) })
        }
        "scryer_mutate" if crate::scryer::mutate_always_asks(input) => Some(AskTarget::Always),
        "scryer_mutate" => {
            Some(AskTarget::Connection { kind: crate::scryer::SCRYER, name: field("connection").map(String::from) })
        }
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
    if d.inbox.is_empty() {
        out.push_str("- Empty.\n");
    }
    capped(&d.inbox, &mut out, |i| match &i.ticket {
        Some(k) => format!("- {} · {k}: {}", i.kind, i.title),
        None => format!("- {}: {}", i.kind, i.title),
    });
    match d.since {
        Some(since) => out.push_str(&format!("\n## Since the last PM session ({})\n\n", ago(d.now, since))),
        None => out.push_str("\n## Recent events\n\n"),
    }
    if d.events.is_empty() {
        out.push_str("- Nothing happened.\n");
    } else {
        let mut counts: Vec<(&str, usize)> = vec![];
        for e in &d.events {
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
        capped(&d.events, &mut out, |e| match &e.ticket {
            Some(k) => format!("- {} · {} · {k}", ago(d.now, e.at), e.kind),
            None => format!("- {} · {}", ago(d.now, e.at), e.kind),
        });
    }
    if !d.journal.is_empty() {
        out.push_str("\n## Journal (from earlier PM sessions, newest first)\n");
        for j in d.journal.iter().take(BRIEF_JOURNALS) {
            out.push_str(&format!("\n### {}\n\n{}\n", ago(d.now, j.at), j.body.trim()));
        }
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
    /// The persona's instructions.
    pub persona: &'a str,
    /// [`briefing`]'s text.
    pub briefing: &'a str,
}

/// A PM session's instructions: who it is, its tools, the persona's instructions and the briefing.
pub fn instructions(ctx: &PmContext) -> String {
    let tools = |asking: bool| {
        ctx.tools
            .iter()
            .filter(|t| crate::persona::asks(t) == asking)
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
             the project (for Discord and Scryer, the connection) lets you act without asking: {asking}. A refusal is an \
             answer: don't retry it unchanged.\n"
        ));
    }
    out.push_str(&format!(
        "- Messages starting `[{SHORT_NAME}]` come from {SHORT_NAME}, not the user.\n\n## Your role\n\n{}\n\n{}",
        ctx.persona.trim(),
        ctx.briefing.trim_end()
    ));
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const H: i64 = 3_600_000;

    #[test]
    fn rotates_on_a_full_context_or_old_age() {
        let day = ROTATE_AGE_MS;
        assert_eq!(rotate_reason(0, H, Some((59, 100)), day), None);
        assert_eq!(rotate_reason(0, H, Some((60, 100)), day), Some(RotateReason::Context));
        assert_eq!(rotate_reason(0, H, Some((10, 0)), day), None, "an unknown window never rotates");
        assert_eq!(rotate_reason(0, 24 * H, None, day), Some(RotateReason::Age));
        assert_eq!(rotate_reason(0, 24 * H - 1, None, day), None);
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
        assert_eq!(t("link_tickets", json!({"ticket": "PDT-5"})), Some(AskTarget::Ticket("PDT-5".into())));
        assert_eq!(t("mcp__pdt__unlink_tickets", json!({"ticket": "PDT-5"})), Some(AskTarget::Ticket("PDT-5".into())));
        assert_eq!(t("send_to_session", json!({"session": "s1"})), Some(AskTarget::Session("s1".into())));
        assert_eq!(t("create_ticket", json!({})), None);
        assert_eq!(
            t("discord_change", json!({"connection": "home"})),
            Some(AskTarget::Connection { kind: "discord_bot", name: Some("home".into()) })
        );
        assert_eq!(
            t("mcp__pdt__discord_webhook", json!({})),
            Some(AskTarget::Connection { kind: "discord_bot", name: None })
        );
        assert_eq!(
            t("mcp__pdt__scryer_add", json!({"connection": "media"})),
            Some(AskTarget::Connection { kind: "scryer", name: Some("media".into()) })
        );
        assert_eq!(
            t("scryer_mutate", json!({"mutation": "updateGeneralSettings"})),
            Some(AskTarget::Connection { kind: "scryer", name: None })
        );
        assert_eq!(t("scryer_mutate", json!({"mutation": "deleteIndexerConfig"})), Some(AskTarget::Always));
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
        }
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
        ] {
            assert!(b.contains(want), "missing {want:?} in\n{b}");
        }
    }

    #[test]
    fn an_empty_briefing_says_so_and_long_sections_are_capped() {
        let b = briefing(&BriefingData { now: 1, ..Default::default() });
        assert!(b.contains("- None yet.") && b.contains("## Recent events") && b.contains("- Nothing happened."));
        assert!(!b.contains("## Journal"));
        let mut d = data();
        d.inbox = (0..BRIEF_CAP + 3)
            .map(|i| BriefInbox { kind: "gate".into(), ticket: None, title: format!("g{i}") })
            .collect();
        assert!(briefing(&d).contains("- … and 3 more"));
    }

    #[test]
    fn instructions_split_the_tools_that_ask() {
        let tools: Vec<String> = ["get_overview", "create_ticket"].map(String::from).to_vec();
        let text = instructions(&PmContext {
            session_id: "s1",
            port: 7421,
            work_folder: "/w",
            tools: &tools,
            persona: "You are the PM.",
            briefing: "# Briefing\n",
        });
        assert!(text.contains("are how you see and change PDT: `get_overview`."), "{text}");
        assert!(text.contains("lets you act without asking: `create_ticket`."), "{text}");
        assert!(text.contains("## Your role\n\nYou are the PM.\n\n# Briefing\n"), "{text}");
        for want in
            ["from the work folder `/w`", "search and read the web", "install tools", "127.0.0.1:7421/api/guides"]
        {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
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
