//! Starting work on a ticket (tickets spec §5): the attempt's default branch, the agent's first prompt, and the
//! Start request.

use crate::brand::{SHORT_NAME, SLUG};
use crate::conversation::SessionMode;
use crate::session::{AgentKind, Session, TerminalSize};
use crate::ticket::{AttemptSummary, MAX_SLUG, Ticket, TicketValidationError, format_key, slug};
use crate::workspace::TabPlacement;
use crate::worktree::NewWorktree;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The longest ticket prompt, in bytes; longer descriptions are cut.
pub const MAX_TICKET_PROMPT: usize = 8 * 1024;
/// Appended to a description that was cut to fit the prompt.
pub fn description_cut_note() -> String {
    format!("[Description cut: read the full ticket in {SHORT_NAME}.]")
}
/// Terminal size limits and defaults for a started session.
pub const START_COLS: std::ops::RangeInclusive<u16> = 20..=500;
pub const START_ROWS: std::ops::RangeInclusive<u16> = 5..=300;
pub const DEFAULT_SIZE: TerminalSize = TerminalSize { cols: 120, rows: 30 };

/// `pdt/<KEY>-<n>-<slug of the title>`, or `pdt/<KEY>-<n>` when the title has no ASCII letters or digits.
pub fn attempt_branch(project_key: &str, number: u32, title: &str) -> String {
    let s = slug(title);
    debug_assert!(s.len() <= MAX_SLUG);
    let key = format_key(project_key, number);
    if s.is_empty() { format!("{SLUG}/{key}") } else { format!("{SLUG}/{key}-{s}") }
}

/// The agent's first prompt for a ticket: `"<KEY>-<n>: <title>\n\n<description>\n\n(Ticket <KEY>-<n> in PDT. Work in
/// this worktree on branch <branch>.)"`, at most [`MAX_TICKET_PROMPT`] bytes. A description too long for that is cut
/// at a character boundary and ends with a note to read the ticket in PDT; an empty description leaves no empty
/// paragraph.
pub fn ticket_prompt(key: &str, title: &str, description: &str, branch: &str) -> String {
    let head = format!("{key}: {title}");
    let tail = format!("(Ticket {key} in {SHORT_NAME}. Work in this worktree on branch {branch}.)");
    let description = description.trim();
    if description.is_empty() {
        return format!("{head}\n\n{tail}");
    }
    let full = format!("{head}\n\n{description}\n\n{tail}");
    if full.len() <= MAX_TICKET_PROMPT {
        return full;
    }
    let note = description_cut_note();
    // head + "\n\n" + cut + "\n\n" + note + "\n\n" + tail
    let fixed = head.len() + tail.len() + note.len() + 6;
    let budget = MAX_TICKET_PROMPT.saturating_sub(fixed);
    let mut end = budget.min(description.len());
    while !description.is_char_boundary(end) {
        end -= 1;
    }
    let cut = description[..end].trim_end();
    if cut.is_empty() { format!("{head}\n\n{note}\n\n{tail}") } else { format!("{head}\n\n{cut}\n\n{note}\n\n{tail}") }
}

/// How Start treats findings such as open blockers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StartMode {
    /// Refuse with `start_blocked` when there is a finding (the dialog's first call).
    #[default]
    Check,
    /// Start anyway (the timeline records it).
    Force,
}

/// `POST /api/tickets/{id}/start`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartTicket {
    pub agent: AgentKind,
    /// The ticket's `defaultBranch` when absent or blank.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    /// Branch or commit to start from; the project's base branch when absent.
    #[serde(default)]
    #[ts(optional)]
    pub base: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub bypass_permissions: Option<bool>,
    /// Where the ticket's tab opens (the session's tab goes right after it); the end of the project's workspace when
    /// absent. Ignored with `newWorkspace`.
    #[serde(default)]
    #[ts(optional)]
    pub tab: Option<TabPlacement>,
    /// Open the ticket and session tabs in a new workspace named after the ticket.
    #[serde(default)]
    #[ts(optional)]
    pub new_workspace: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<StartMode>,
    /// How the agent runs: a conversation (the default; the ticket is its first message) or its terminal.
    #[serde(default)]
    #[ts(optional)]
    pub session_mode: Option<SessionMode>,
    /// Start from this chat (a conversation session with no ticket, in a worktree off the base branch) instead of a
    /// new session: the attempt takes its worktree and branch, and nothing is sent to the agent. `agent`, `branch`,
    /// `base` and `bypassPermissions` don't apply.
    #[serde(default)]
    #[ts(optional)]
    pub from_session: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub cols: Option<u16>,
    #[serde(default)]
    #[ts(optional)]
    pub rows: Option<u16>,
}

/// A checked [`StartTicket`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidStart {
    pub agent: AgentKind,
    /// None: the ticket's default branch.
    pub branch: Option<String>,
    pub base: Option<String>,
    pub bypass_permissions: bool,
    pub tab: Option<TabPlacement>,
    pub new_workspace: bool,
    pub mode: StartMode,
    pub session_mode: SessionMode,
    pub from_session: Option<String>,
    pub size: TerminalSize,
}

impl StartTicket {
    pub fn validate(&self) -> Result<ValidStart, TicketValidationError> {
        let nonblank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        let branch = match nonblank(&self.branch) {
            Some(b) => Some(
                NewWorktree { branch: b, base: None }
                    .validate()
                    .map_err(|e| TicketValidationError::Invalid(e.to_string()))?
                    .branch,
            ),
            None => None,
        };
        let from_session = nonblank(&self.from_session);
        if from_session.is_some() && (branch.is_some() || nonblank(&self.base).is_some()) {
            return Err(TicketValidationError::Invalid("a start from a chat keeps the chat's branch".into()));
        }
        let cols = self.cols.unwrap_or(DEFAULT_SIZE.cols);
        let rows = self.rows.unwrap_or(DEFAULT_SIZE.rows);
        if !START_COLS.contains(&cols) || !START_ROWS.contains(&rows) {
            return Err(TicketValidationError::Invalid(format!(
                "terminal size must be {}–{} columns by {}–{} rows",
                START_COLS.start(),
                START_COLS.end(),
                START_ROWS.start(),
                START_ROWS.end()
            )));
        }
        let session_mode = self.session_mode.unwrap_or(SessionMode::Conversation);
        Ok(ValidStart {
            agent: self.agent,
            branch,
            base: nonblank(&self.base),
            bypass_permissions: self.bypass_permissions.unwrap_or(false),
            tab: self.tab.clone(),
            new_workspace: self.new_workspace.unwrap_or(false),
            mode: self.mode.unwrap_or_default(),
            session_mode,
            from_session,
            size: TerminalSize { cols, rows },
        })
    }
}

/// What Start made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartResult {
    pub ticket: Ticket,
    pub attempt: AttemptSummary,
    pub session: Session,
}

/// The ticket title, within the session title limit (the sidebar and tabs show the ticket key and the agent's icon
/// beside it).
pub fn started_session_title(ticket_title: &str) -> String {
    let full = ticket_title.trim().to_string();
    let max = crate::session::MAX_TITLE_CHARS;
    if full.chars().count() <= max {
        return full;
    }
    let mut cut: String = full.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_branches_carry_the_key_and_a_short_slug() {
        assert_eq!(attempt_branch("PDT", 42, "Fix the Login bug!"), "pdt/PDT-42-fix-the-login-bug");
        assert_eq!(attempt_branch("PDT", 7, "日本語"), "pdt/PDT-7");
        assert_eq!(attempt_branch("AB", 1, "  "), "pdt/AB-1");
        let long = attempt_branch("PDT", 3, "a very long ticket title that keeps going well past forty characters");
        assert_eq!(long, "pdt/PDT-3-a-very-long-ticket-title-that-keeps");
        // Always a valid branch name.
        for t in ["x..y", "a/b", "trailing.", "-lead", "~^:?*[\\"] {
            let b = attempt_branch("PDT", 1, t);
            assert!(NewWorktree { branch: b.clone(), base: None }.validate().is_ok(), "{b}");
        }
    }

    #[test]
    fn the_prompt_names_the_ticket_and_the_branch() {
        assert_eq!(
            ticket_prompt("PDT-4", "Fix login", "Steps:\n1. open\n", "pdt/PDT-4-fix-login"),
            "PDT-4: Fix login\n\nSteps:\n1. open\n\n(Ticket PDT-4 in PDT. Work in this worktree on branch \
             pdt/PDT-4-fix-login.)"
        );
        // No description, no empty paragraph.
        assert_eq!(
            ticket_prompt("PDT-4", "Fix login", "  \n", "b"),
            "PDT-4: Fix login\n\n(Ticket PDT-4 in PDT. Work in this worktree on branch b.)"
        );
    }

    #[test]
    fn long_descriptions_are_cut_to_8_kb_at_a_char_boundary() {
        // Exactly at the cap: untouched.
        let head_tail = ticket_prompt("K-1", "T", "", "b").len() + 2;
        let fits = "x".repeat(MAX_TICKET_PROMPT - head_tail);
        assert_eq!(ticket_prompt("K-1", "T", &fits, "b").len(), MAX_TICKET_PROMPT);
        assert!(!ticket_prompt("K-1", "T", &fits, "b").contains("[Description cut"));
        // Over: cut, with the note, within the cap, on a char boundary (é is two bytes).
        for desc in ["é".repeat(10_000), format!("{fits}y"), "word ".repeat(5_000)] {
            let p = ticket_prompt("K-1", "T", &desc, "b");
            assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
            assert!(p.starts_with("K-1: T\n\n"));
            let end = "\n\n[Description cut: read the full ticket in PDT.]\n\n(Ticket K-1 in PDT. Work in this worktree on \
                       branch b.)";
            assert!(p.ends_with(end), "{p}");
            assert!(p.len() > MAX_TICKET_PROMPT - 8, "uses the space it has: {}", p.len());
        }
    }

    fn start(json: serde_json::Value) -> Result<ValidStart, TicketValidationError> {
        serde_json::from_value::<StartTicket>(json).unwrap().validate()
    }

    #[test]
    fn start_requests_are_checked_and_defaulted() {
        let v = start(serde_json::json!({"agent": "claude"})).unwrap();
        assert_eq!(
            v,
            ValidStart {
                agent: AgentKind::Claude,
                branch: None,
                base: None,
                bypass_permissions: false,
                tab: None,
                new_workspace: false,
                mode: StartMode::Check,
                session_mode: SessionMode::Conversation,
                from_session: None,
                size: DEFAULT_SIZE,
            }
        );
        let t = start(serde_json::json!({"agent": "claude", "sessionMode": "terminal"})).unwrap();
        assert_eq!(t.session_mode, SessionMode::Terminal);
        let devin = start(serde_json::json!({"agent": "devin"})).unwrap();
        assert_eq!(devin.session_mode, SessionMode::Conversation);
        let chat = start(serde_json::json!({"agent": "claude", "fromSession": " s1 "})).unwrap();
        assert_eq!(chat.from_session.as_deref(), Some("s1"));
        let v = start(serde_json::json!({
            "agent": "codex", "branch": " feature/x ", "base": " ", "bypassPermissions": true, "newWorkspace": true,
            "mode": "force", "cols": 20, "rows": 300, "tab": {"workspaceId": "w", "position": 2}
        }))
        .unwrap();
        assert_eq!(v.branch.as_deref(), Some("feature/x"));
        assert_eq!((v.base, v.bypass_permissions, v.new_workspace, v.mode), (None, true, true, StartMode::Force));
        assert_eq!(v.size, TerminalSize { cols: 20, rows: 300 });
        assert_eq!(v.tab, Some(TabPlacement { workspace_id: "w".into(), position: Some(2) }));
        assert_eq!(start(serde_json::json!({"agent": "claude", "branch": "  "})).unwrap().branch, None);
        for bad in [
            serde_json::json!({"agent": "claude", "branch": "a b"}),
            serde_json::json!({"agent": "claude", "branch": "x..y"}),
            serde_json::json!({"agent": "claude", "cols": 19}),
            serde_json::json!({"agent": "claude", "cols": 501}),
            serde_json::json!({"agent": "claude", "rows": 4}),
            serde_json::json!({"agent": "claude", "rows": 301}),
            serde_json::json!({"agent": "claude", "fromSession": "s1", "branch": "x"}),
            serde_json::json!({"agent": "claude", "fromSession": "s1", "base": "main"}),
        ] {
            assert!(matches!(start(bad.clone()), Err(TicketValidationError::Invalid(_))), "{bad}");
        }
        assert!(serde_json::from_value::<StartTicket>(serde_json::json!({"agent": "claude", "mode": "wait"})).is_err());
    }

    #[test]
    fn started_sessions_are_titled_by_the_ticket() {
        assert_eq!(started_session_title(" Fix login "), "Fix login");
        let long = started_session_title(&"x".repeat(300));
        assert_eq!(long.chars().count(), crate::session::MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
    }
}
