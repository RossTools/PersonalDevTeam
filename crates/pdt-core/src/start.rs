//! Starting work on a ticket (tickets spec §5): the attempt's default branch, the agent's first prompt, and the
//! Start request.

use crate::brand::{SHORT_NAME, SLUG};
use crate::conversation::{PermissionMode, SessionMode};
use crate::session::{AgentKind, Session, TerminalSize};
use crate::ticket::{
    AttemptSummary, MAX_SLUG, OverlapLevel, Ticket, TicketOverlap, TicketValidationError, format_key, slug,
};
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

/// `pdt-<KEY>-<n>-<slug of the title>`, or `pdt-<KEY>-<n>` when the title has no ASCII letters or digits. No `/`:
/// some third-party tools break on slashes in branch names.
pub fn attempt_branch(project_key: &str, number: u32, title: &str) -> String {
    let s = slug(title);
    debug_assert!(s.len() <= MAX_SLUG);
    let key = format_key(project_key, number);
    if s.is_empty() { format!("{SLUG}-{key}") } else { format!("{SLUG}-{key}-{s}") }
}

/// Whether `branch` is one [`attempt_branch`] names for the ticket `ticket_key` (`PDT-42`), whatever its title was:
/// a branch PDT made for the ticket, which it may delete from the remote when the ticket goes to Won't do.
pub fn is_attempt_branch(branch: &str, ticket_key: &str) -> bool {
    let prefix = format!("{SLUG}-{ticket_key}");
    branch.strip_prefix(&prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

/// The parent and the other parts, for a part's first prompt (PDT-38).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartContext {
    pub parent_key: String,
    pub parent_title: String,
    pub parent_description: String,
    /// The parent's other parts in rank order: one line each, e.g. `PDT-46 5a: Done (#44)`.
    pub others: Vec<String>,
}

impl PartContext {
    /// The "Part of" block with the parent's description cut to `budget` bytes (None: whole).
    fn block(&self, budget: Option<usize>) -> String {
        let mut out = format!("Part of {}: {}", self.parent_key, self.parent_title);
        let d = self.parent_description.trim();
        let d = match budget {
            None => d.to_string(),
            Some(b) if b >= d.len() => d.to_string(),
            Some(b) => {
                let mut end = b;
                while !d.is_char_boundary(end) {
                    end -= 1;
                }
                let cut = d[..end].trim_end();
                if cut.is_empty() { description_cut_note() } else { format!("{cut}\n\n{}", description_cut_note()) }
            }
        };
        if !d.is_empty() {
            out.push_str("\n\n");
            out.push_str(&d);
        }
        if !self.others.is_empty() {
            out.push_str("\n\nThe other parts:\n");
            out.push_str(&self.others.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n"));
        }
        out
    }
}

/// The agent's first prompt for a ticket: `"<KEY>-<n>: <title>\n\n<description>\n\n(Ticket <KEY>-<n> in PDT. Work in
/// this worktree on branch <branch>. <ONE_TICKET>)"`, at most [`MAX_TICKET_PROMPT`] bytes. A description too long for
/// that is cut at a character boundary and ends with a note to read the ticket in PDT; an empty description leaves no
/// empty paragraph.
pub fn ticket_prompt(key: &str, title: &str, description: &str, branch: &str) -> String {
    part_prompt(key, title, description, branch, None)
}

/// [`ticket_prompt`] for a part: a "Part of" block (the parent's key, title and description, and one line per other
/// part) goes before the closing line. To fit [`MAX_TICKET_PROMPT`], the parent's description is cut first, then the
/// part's own.
pub fn part_prompt(key: &str, title: &str, description: &str, branch: &str, part: Option<&PartContext>) -> String {
    let (head, tail) = prompt_ends(key, title, branch);
    let description = description.trim();
    let join = |desc: &str, block: Option<String>| assemble(&head, desc, None, block, &tail);
    let full = join(description, part.map(|p| p.block(None)));
    if full.len() <= MAX_TICKET_PROMPT {
        return full;
    }
    // Cut the parent's description first.
    if let Some(p) = part {
        let without = join(description, Some(p.block(Some(0))));
        if without.len() <= MAX_TICKET_PROMPT {
            // A cut of n bytes adds n + 2 ("\n\n") to the bare block (heading, note, the other parts).
            let budget = (MAX_TICKET_PROMPT - without.len()).saturating_sub(2);
            let block = p.block(Some(budget));
            let out = join(description, Some(block));
            if out.len() <= MAX_TICKET_PROMPT {
                return out;
            }
            return without;
        }
    }
    // Then the ticket's own description (the block keeps no parent description).
    let block = part.map(|p| p.block(Some(0)));
    let note = description_cut_note();
    let fixed = join("", block.clone()).len() + note.len() + 4;
    let budget = MAX_TICKET_PROMPT.saturating_sub(fixed);
    let mut end = budget.min(description.len());
    while !description.is_char_boundary(end) {
        end -= 1;
    }
    let cut = description[..end].trim_end();
    let desc = if cut.is_empty() { note } else { format!("{cut}\n\n{note}") };
    let out = join(&desc, block);
    if out.len() <= MAX_TICKET_PROMPT {
        out
    } else {
        // A huge list of other parts: drop the block rather than overflow.
        join(&desc, None)
    }
}

/// [`part_prompt`] with the ticket's comments (oldest first) after its description, as block quotes under "Comments,
/// oldest first:". Comments only get the room the rest leaves: when the description or the parent's had to be cut there
/// are none; otherwise the newest are kept whole, the oldest dropped first (with a note saying how many), and a newest
/// comment too long for the room left is cut with a note. Blank comments are skipped.
pub fn start_prompt(
    key: &str,
    title: &str,
    description: &str,
    branch: &str,
    part: Option<&PartContext>,
    comments: &[String],
) -> String {
    let base = part_prompt(key, title, description, branch, part);
    let comments: Vec<&str> = comments.iter().map(|c| c.trim()).filter(|c| !c.is_empty()).collect();
    if comments.is_empty() {
        return base;
    }
    let (head, tail) = prompt_ends(key, title, branch);
    let description = description.trim();
    let block = part.map(|p| p.block(None));
    if base != assemble(&head, description, None, block.clone(), &tail) {
        return base;
    }
    // "\n\n" joins the comments to the rest.
    let room = MAX_TICKET_PROMPT.saturating_sub(base.len() + 2);
    match comments_block(&comments, room) {
        Some(c) => assemble(&head, description, Some(c), block, &tail),
        None => base,
    }
}

/// The first prompt's last sentence: an attempt's branch is for its ticket alone.
pub const ONE_TICKET: &str = "Keep to this ticket: ask the user before making a ticket for anything unrelated that turns up, rather than changing \
     it here.";

/// An empty `branch` is a Personal project's ticket: it works in a folder, with no branch.
fn prompt_ends(key: &str, title: &str, branch: &str) -> (String, String) {
    let place = if branch.is_empty() {
        "Work in this folder.".to_string()
    } else {
        format!("Work in this worktree on branch {branch}.")
    };
    (format!("{key}: {title}"), format!("(Ticket {key} in {SHORT_NAME}. {place} {ONE_TICKET})"))
}

fn assemble(head: &str, desc: &str, comments: Option<String>, block: Option<String>, tail: &str) -> String {
    let mut parts = vec![head.to_string()];
    if !desc.is_empty() {
        parts.push(desc.to_string());
    }
    parts.extend(comments);
    parts.extend(block);
    parts.push(tail.to_string());
    parts.join("\n\n")
}

/// The comments block in at most `room` bytes, newest comments kept first; None when not even a cut comment fits.
fn comments_block(comments: &[&str], room: usize) -> Option<String> {
    const HEADING: &str = "Comments, oldest first:";
    let quote = |c: &str| {
        c.lines().map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect::<Vec<_>>().join("\n")
    };
    let not_shown = |n: usize| match n {
        0 => None,
        1 => Some(format!("[1 earlier comment not shown: read them in {SHORT_NAME}.]")),
        n => Some(format!("[{n} earlier comments not shown: read them in {SHORT_NAME}.]")),
    };
    let build = |omitted: usize, kept: &[String]| {
        let mut parts = vec![HEADING.to_string()];
        parts.extend(not_shown(omitted));
        parts.extend(kept.iter().cloned());
        parts.join("\n\n")
    };
    let quoted: Vec<String> = comments.iter().map(|c| quote(c)).collect();
    let mut kept = 0;
    while kept < quoted.len() && build(quoted.len() - kept - 1, &quoted[quoted.len() - kept - 1..]).len() <= room {
        kept += 1;
    }
    if kept > 0 {
        return Some(build(quoted.len() - kept, &quoted[quoted.len() - kept..]));
    }
    // Not even the newest fits whole: cut it.
    let cut_note = format!("[Comment cut: read it in {SHORT_NAME}.]");
    let newest = &quoted[quoted.len() - 1];
    let fixed = build(quoted.len() - 1, &[String::new()]).len() + 2 + cut_note.len();
    let mut end = room.checked_sub(fixed)?.min(newest.len());
    while !newest.is_char_boundary(end) {
        end -= 1;
    }
    let cut = newest[..end].trim_end();
    if cut.is_empty() {
        return None;
    }
    Some(build(quoted.len() - 1, &[format!("{cut}\n\n{cut_note}")]))
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
    /// The session's permission mode (a conversation's; in the terminal only `bypass` changes anything). Wins over
    /// `bypassPermissions`. Absent: `bypass` with `bypassPermissions`, else the agent's default.
    #[serde(default)]
    #[ts(optional)]
    pub permission_mode: Option<PermissionMode>,
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
    /// Start without an agent session (PDT-72): the branch, worktree, attempt and the ticket's tab (in a new workspace
    /// with `newWorkspace`) only. `agent`, `sessionMode` and `bypassPermissions` don't apply.
    #[serde(default)]
    #[ts(optional)]
    pub no_session: Option<bool>,
    /// On a parent: start its next part (the first open part, by rank, that is not started or merged and has no
    /// open blockers) instead of refusing with `ticket_has_parts`.
    #[serde(default)]
    #[ts(optional)]
    pub next: Option<bool>,
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
    pub permission_mode: Option<PermissionMode>,
    pub tab: Option<TabPlacement>,
    pub new_workspace: bool,
    pub mode: StartMode,
    pub session_mode: SessionMode,
    pub from_session: Option<String>,
    /// Make no agent session.
    pub no_session: bool,
    /// On a parent: start its next part.
    pub next: bool,
    pub size: TerminalSize,
    /// The caller checked the running cap and the overlap rule itself, under its own lock (`ticket.start`, with the
    /// caps and overlap rule a workflow chose under, PDT-327, PDT-381): Start doesn't refuse for the project's cap, and
    /// refuses only for conflicts ([`refusing_overlaps`]). Never set from a request.
    pub pickup_checked: bool,
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
        let no_session = self.no_session.unwrap_or(false);
        if no_session && from_session.is_some() {
            return Err(TicketValidationError::Invalid("a start from a chat keeps the chat's session".into()));
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
            bypass_permissions: self
                .permission_mode
                .map_or(self.bypass_permissions.unwrap_or(false), |m| m == PermissionMode::Bypass),
            permission_mode: self.permission_mode,
            tab: self.tab.clone(),
            new_workspace: self.new_workspace.unwrap_or(false),
            mode: self.mode.unwrap_or_default(),
            session_mode,
            from_session,
            no_session,
            next: self.next.unwrap_or(false),
            size: TerminalSize { cols, rows },
            pickup_checked: false,
        })
    }
}

/// The overlaps Start refuses for (unless forced). A pickup that checked its own overlap rule (`pickup_checked`) has
/// already kept the ticket apart from work in flight as that rule says (by default only in hot areas), so Start refuses
/// it only for a conflict, which pickup's candidates leave out too; anyone else's Start refuses for any overlap.
pub fn refusing_overlaps(overlaps: &[TicketOverlap], pickup_checked: bool) -> Vec<&TicketOverlap> {
    overlaps.iter().filter(|o| !pickup_checked || o.level == OverlapLevel::Conflict).collect()
}

/// What Start made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartResult {
    pub ticket: Ticket,
    pub attempt: AttemptSummary,
    /// The agent session working on it; none for a start with `noSession`.
    pub session: Option<Session>,
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
    fn attempt_branches_are_known_by_their_ticket_key_whatever_the_title() {
        assert!(is_attempt_branch(&attempt_branch("PDT", 42, "Old title"), "PDT-42"));
        assert!(is_attempt_branch("pdt-PDT-42", "PDT-42"));
        assert!(!is_attempt_branch("pdt-PDT-420-other", "PDT-42"));
        assert!(!is_attempt_branch("feature/login", "PDT-42"));
        assert!(!is_attempt_branch("my-pdt-PDT-42", "PDT-42"));
    }

    #[test]
    fn default_branches_carry_the_key_and_a_short_slug() {
        assert_eq!(attempt_branch("PDT", 42, "Fix the Login bug!"), "pdt-PDT-42-fix-the-login-bug");
        assert_eq!(attempt_branch("PDT", 7, "日本語"), "pdt-PDT-7");
        assert_eq!(attempt_branch("AB", 1, "  "), "pdt-AB-1");
        let long = attempt_branch("PDT", 3, "a very long ticket title that keeps going well past forty characters");
        assert_eq!(long, "pdt-PDT-3-a-very-long-ticket-title-that-keeps");
        // Always a valid branch name, and never a slash.
        for t in ["x..y", "a/b", "trailing.", "-lead", "~^:?*[\\"] {
            let b = attempt_branch("PDT", 1, t);
            assert!(NewWorktree { branch: b.clone(), base: None }.validate().is_ok(), "{b}");
            assert!(!b.contains('/'), "{b}");
        }
    }

    #[test]
    fn the_prompt_names_the_ticket_and_the_branch() {
        assert_eq!(
            ticket_prompt("PDT-4", "Fix login", "Steps:\n1. open\n", "pdt/PDT-4-fix-login"),
            format!(
                "PDT-4: Fix login\n\nSteps:\n1. open\n\n(Ticket PDT-4 in PDT. Work in this worktree on branch \
             pdt/PDT-4-fix-login. {ONE_TICKET})"
            )
        );
        // No description, no empty paragraph.
        assert_eq!(
            ticket_prompt("PDT-4", "Fix login", "  \n", "b"),
            format!("PDT-4: Fix login\n\n(Ticket PDT-4 in PDT. Work in this worktree on branch b. {ONE_TICKET})")
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
            let end = &format!(
                "\n\n[Description cut: read the full ticket in PDT.]\n\n(Ticket K-1 in PDT. Work in this worktree on \
                       branch b. {ONE_TICKET})"
            );
            assert!(p.ends_with(end), "{p}");
            assert!(p.len() > MAX_TICKET_PROMPT - 8, "uses the space it has: {}", p.len());
        }
    }

    fn ctx(desc: &str, others: &[&str]) -> PartContext {
        PartContext {
            parent_key: "PDT-2".into(),
            parent_title: "Phase 5".into(),
            parent_description: desc.into(),
            others: others.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_part_prompt_names_the_parent_and_the_other_parts() {
        let c = ctx("Conversations everywhere.", &["PDT-46 5a: Done (#44)", "PDT-48 5d: To do"]);
        assert_eq!(
            part_prompt("PDT-47", "5b", "Codex.", "b", Some(&c)),
            format!(
                "PDT-47: 5b\n\nCodex.\n\nPart of PDT-2: Phase 5\n\nConversations everywhere.\n\nThe other parts:\n\
             - PDT-46 5a: Done (#44)\n- PDT-48 5d: To do\n\n(Ticket PDT-47 in PDT. Work in this worktree on branch b. {ONE_TICKET})"
            )
        );
        assert_eq!(
            part_prompt("PDT-47", "5b", "", "b", Some(&ctx("", &[]))),
            format!(
                "PDT-47: 5b\n\nPart of PDT-2: Phase 5\n\n(Ticket PDT-47 in PDT. Work in this worktree on branch b. {ONE_TICKET})"
            )
        );
        assert_eq!(part_prompt("PDT-4", "T", "d", "b", None), ticket_prompt("PDT-4", "T", "d", "b"));
    }

    #[test]
    fn a_long_part_prompt_cuts_the_parent_first_then_the_part() {
        let parent = "p".repeat(6000);
        let own = "o".repeat(4000);
        let p = part_prompt("PDT-47", "5b", &own, "b", Some(&ctx(&parent, &["PDT-46 5a: Done"])));
        assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
        assert!(p.contains(&own), "the part's description is whole");
        assert!(p.contains(&description_cut_note()) && p.contains("ppp"));
        assert!(p.contains("- PDT-46 5a: Done") && p.ends_with(&format!("on branch b. {ONE_TICKET})")));
        assert!(p.len() > MAX_TICKET_PROMPT - 64, "uses the space it has: {}", p.len());

        let own = "o".repeat(9000);
        let p = part_prompt("PDT-47", "5b", &own, "b", Some(&ctx(&parent, &["PDT-46 5a: Done"])));
        assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
        assert!(!p.contains("ppp"), "the parent's description went first");
        assert!(p.contains("Part of PDT-2: Phase 5") && p.contains("- PDT-46 5a: Done"));
        assert!(p.contains("ooo") && p.contains(&description_cut_note()));

        let many: Vec<String> = (0..2000).map(|i| format!("PDT-{i} a part with a longish title")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let p = part_prompt("PDT-47", "5b", "d", "b", Some(&ctx("x", &refs)));
        assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
        assert!(p.starts_with("PDT-47: 5b") && p.ends_with(&format!("on branch b. {ONE_TICKET})")));
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn comments_follow_the_description_oldest_first_and_quoted() {
        let c = ctx("Parent.", &["PDT-46 5a: Done"]);
        assert_eq!(
            start_prompt("PDT-47", "5b", "Codex.", "b", Some(&c), &strings(&["First.", " Second\n\nline two\n "])),
            format!(
                "PDT-47: 5b\n\nCodex.\n\nComments, oldest first:\n\n> First.\n\n> Second\n>\n> line two\n\n\
             Part of PDT-2: Phase 5\n\nParent.\n\nThe other parts:\n- PDT-46 5a: Done\n\n\
             (Ticket PDT-47 in PDT. Work in this worktree on branch b. {ONE_TICKET})"
            )
        );
        // No comments (or only blank ones): the same prompt as before.
        assert_eq!(start_prompt("PDT-4", "T", "d", "b", None, &[]), ticket_prompt("PDT-4", "T", "d", "b"));
        assert_eq!(
            start_prompt("PDT-4", "T", "d", "b", None, &strings(&[" \n"])),
            ticket_prompt("PDT-4", "T", "d", "b")
        );
    }

    #[test]
    fn comments_that_do_not_fit_drop_the_oldest_first() {
        let comments: Vec<String> = (0..10).map(|i| format!("{i}{}", "c".repeat(1000))).collect();
        let p = start_prompt("K-1", "T", "d", "b", None, &comments);
        assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
        assert!(p.contains(&format!("> 9{}", "c".repeat(1000))), "the newest is whole");
        assert!(!p.contains("> 0c"), "the oldest went first");
        let shown = (0..10).filter(|i| p.contains(&format!("> {i}c"))).count();
        assert!(shown >= 6, "uses the space it has: {shown}");
        assert!(p.contains(&format!("[{} earlier comments not shown: read them in PDT.]", 10 - shown)), "{p}");
        assert!(p.ends_with(&format!("(Ticket K-1 in PDT. Work in this worktree on branch b. {ONE_TICKET})")));
    }

    #[test]
    fn a_newest_comment_too_long_to_fit_is_cut() {
        let comments = strings(&["old", &"é".repeat(9000)]);
        let p = start_prompt("K-1", "T", "d", "b", None, &comments);
        assert!(p.len() <= MAX_TICKET_PROMPT, "{}", p.len());
        assert!(p.contains("> éé") && p.contains("[Comment cut: read it in PDT.]"), "{p}");
        assert!(p.contains("[1 earlier comment not shown: read them in PDT.]"));
        assert!(p.len() > MAX_TICKET_PROMPT - 64, "uses the space it has: {}", p.len());
    }

    #[test]
    fn a_cut_description_leaves_no_room_for_comments() {
        let p = start_prompt("K-1", "T", &"x".repeat(9000), "b", None, &strings(&["hello"]));
        assert_eq!(p, ticket_prompt("K-1", "T", &"x".repeat(9000), "b"));
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
                permission_mode: None,
                tab: None,
                new_workspace: false,
                mode: StartMode::Check,
                session_mode: SessionMode::Conversation,
                from_session: None,
                no_session: false,
                next: false,
                size: DEFAULT_SIZE,
                pickup_checked: false,
            }
        );
        assert!(start(serde_json::json!({"agent": "claude", "next": true})).unwrap().next);
        // A permission mode wins over bypassPermissions, which follows it.
        let m =
            start(serde_json::json!({"agent": "claude", "permissionMode": "plan", "bypassPermissions": true})).unwrap();
        assert_eq!((m.permission_mode, m.bypass_permissions), (Some(PermissionMode::Plan), false));
        let m = start(serde_json::json!({"agent": "claude", "permissionMode": "bypass"})).unwrap();
        assert_eq!((m.permission_mode, m.bypass_permissions), (Some(PermissionMode::Bypass), true));
        let t = start(serde_json::json!({"agent": "claude", "sessionMode": "terminal"})).unwrap();
        assert_eq!(t.session_mode, SessionMode::Terminal);
        let devin = start(serde_json::json!({"agent": "devin"})).unwrap();
        assert_eq!(devin.session_mode, SessionMode::Conversation);
        let chat = start(serde_json::json!({"agent": "claude", "fromSession": " s1 "})).unwrap();
        assert_eq!(chat.from_session.as_deref(), Some("s1"));
        assert!(start(serde_json::json!({"agent": "claude", "noSession": true})).unwrap().no_session);
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
            serde_json::json!({"agent": "claude", "fromSession": "s1", "noSession": true}),
        ] {
            assert!(matches!(start(bad.clone()), Err(TicketValidationError::Invalid(_))), "{bad}");
        }
        assert!(serde_json::from_value::<StartTicket>(serde_json::json!({"agent": "claude", "mode": "wait"})).is_err());
    }

    #[test]
    fn a_pickup_start_refuses_only_conflicts_and_anyone_elses_any_overlap() {
        let with = |key: &str, level| TicketOverlap {
            ticket_id: key.into(),
            key: key.into(),
            title: String::new(),
            stage: crate::ticket::TicketStage::InProgress,
            level,
            areas: vec!["web".into()],
            files: vec![],
            files_total: 0,
        };
        let list = vec![with("P-1", OverlapLevel::Overlap), with("P-2", OverlapLevel::Conflict)];
        let keys = |v: Vec<&TicketOverlap>| v.iter().map(|o| o.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(refusing_overlaps(&list, false)), ["P-1", "P-2"]);
        assert_eq!(keys(refusing_overlaps(&list, true)), ["P-2"]);
        assert!(refusing_overlaps(&list[..1], true).is_empty(), "a shared area pickup allowed starts");
    }

    #[test]
    fn started_sessions_are_titled_by_the_ticket() {
        assert_eq!(started_session_title(" Fix login "), "Fix login");
        let long = started_session_title(&"x".repeat(300));
        assert_eq!(long.chars().count(), crate::session::MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
    }
}
