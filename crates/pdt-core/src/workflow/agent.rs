//! Agent steps (Phase 7b spec §4): the outcomes a step declares, and the texts PDT sends a persona's session (the
//! first message, nudges). The node itself is pdt-api's `workflow/nodes/agent.rs`.

use crate::brand::SHORT_NAME;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ports an agent node has besides its outcomes.
pub const TIMEOUT: &str = "timeout";
pub const STOPPED: &str = "stopped";
/// The outcome that hands a step's work on (a Coder's): refused while the attempt's worktree holds what its remote
/// doesn't (PDT-161), and the outcome of the steps PR notices fold into (`fold_messages`).
pub const SUBMITTED: &str = "submitted";

/// One outcome a step allows: its port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct OutcomeSpec {
    /// `approved`; lower case, digits and `_`.
    pub id: String,
    /// When the agent should pick it.
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentStepState {
    /// The session works on the step.
    Working,
    /// The session asked the user (`ask_user`) and waits for the answers.
    Asking,
    /// Its turns ended without an outcome; the Inbox has it.
    Stalled,
    /// It has its outcome (or the step ended otherwise).
    Done,
    /// A manual step (Phase 7c): no session; the user does the work and picks the outcome in the Inbox.
    Manual,
}

impl AgentStepState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStepState::Working => "working",
            AgentStepState::Asking => "asking",
            AgentStepState::Stalled => "stalled",
            AgentStepState::Done => "done",
            AgentStepState::Manual => "manual",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Working, Self::Asking, Self::Stalled, Self::Done, Self::Manual].into_iter().find(|x| x.as_str() == s)
    }
}

/// A comment an agent hands in with its outcome: a review comment on the attempt's changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutcomeComment {
    /// The file, relative to the checkout.
    pub file: String,
    /// The first line (1-based, in the changed file).
    pub line: u32,
    /// The last line, for a range.
    #[serde(default)]
    pub end_line: Option<u32>,
    pub body: String,
}

/// An agent step's session binding (Phase 7b spec §4.2): which session runs it and what it may answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentStep {
    pub step_id: String,
    pub run_id: String,
    pub node_id: String,
    pub session_id: Option<String>,
    pub persona_id: Option<String>,
    pub persona_name: String,
    pub persona_version: u32,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    pub outcomes: Vec<OutcomeSpec>,
    pub state: AgentStepState,
    /// Turns that ended without an outcome.
    pub turns: u32,
    /// Nudges sent (automatic and the user's).
    pub nudges: u32,
    pub max_turns: u32,
    /// Automatic nudges before it counts as stalled.
    pub auto_nudges: u32,
    /// Since when its session has waited on background tasks it started (PDT-142): a turn that ended while they ran
    /// is neither counted, nudged nor stalled. None when it isn't waiting on any.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub background_since: Option<i64>,
    /// What those tasks are (their descriptions), as of the turn end.
    #[serde(default)]
    pub background: Vec<String>,
    /// How long it may wait on them before it stalls.
    #[serde(default)]
    #[ts(type = "number")]
    pub background_wait_ms: u64,
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub comments: Vec<OutcomeComment>,
    /// What the agent handed in with its outcome beyond the summary (`submit_outcome`'s `data`), for later steps.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub data: Option<serde_json::Value>,
    /// The user took it over and picked the outcome.
    pub by_user: bool,
    /// Fresh sessions it was restarted in ("When an agent is stuck", PDT-367), its own and the steps it resumes'.
    #[serde(default)]
    pub restarts: u32,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// A stuck agent step handed to "When an agent is stuck" (PDT-367), until one of its nodes acts on it or PDT falls back
/// to the Inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escalation {
    pub step_id: String,
    pub run_id: String,
    /// `stalled`, `loop_limit` or `timed_out`.
    pub kind: String,
    /// What happened, for the Inbox when PDT falls back to it.
    pub why: String,
    pub at: i64,
}

pub const STALLED: &str = "stalled";
pub const LOOP_LIMIT: &str = "loop_limit";
pub const TIMED_OUT: &str = "timed_out";

/// Whether `id` can be an outcome (and so a port).
pub fn valid_outcome_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 40 {
        return Err("an outcome id is 1 to 40 characters".into());
    }
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("outcome {id}: use lower case letters, digits and _"));
    }
    if [TIMEOUT, STOPPED, crate::workflow::doc::ERROR_PORT].contains(&id) {
        return Err(format!("{id} is a port every agent step has; name the outcome differently"));
    }
    Ok(())
}

/// `approved (it can merge as is), changes_requested (…)`.
pub fn outcome_list(outcomes: &[OutcomeSpec]) -> String {
    outcomes
        .iter()
        .map(|o| {
            if o.description.trim().is_empty() {
                format!("`{}`", o.id)
            } else {
                format!("`{}` ({})", o.id, o.description.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ticket a step is about, for its first message.
pub struct StepTicket<'a> {
    pub key: &'a str,
    pub title: &'a str,
}

/// The first message of a step: the task, the ticket and how to finish.
pub fn first_message(prompt: &str, ticket: Option<StepTicket>, outcomes: &[OutcomeSpec]) -> String {
    let mut out = prompt.trim().to_string();
    if let Some(t) = ticket {
        out.push_str(&format!(
            "\n\nTicket: {} — {} (`get_ticket` gives its description, comments and earlier outcomes).",
            t.key, t.title
        ));
    }
    out.push_str(&format!(
        "\n\nWhen you are done, call `submit_outcome` with one of: {}. If only the user can unblock you, call \
         `ask_user`.",
        outcome_list(outcomes)
    ));
    out
}

/// What a reused session gets on a later pass of its step in the same run (it already has the step's role and the
/// ticket): why it is back (`reason`, e.g. the Reviewer's summary or the user's note) and how to finish.
pub fn next_pass_message(reason: Option<&str>, outcomes: &[OutcomeSpec]) -> String {
    let mut out = format!("[{SHORT_NAME}] Your step is back for another pass.");
    if let Some(r) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        out.push_str(&format!("\n\n{r}"));
    }
    out.push_str(&format!(
        "\n\nAddress each point above, one by one (`get_ticket` has the earlier outcomes, their line comments and the \
         user's comments), then call `submit_outcome` with one of: {}.",
        outcome_list(outcomes)
    ));
    out
}

/// Why a step is back, from the step that sent it back: an agent's outcome and summary, the user's choice at a gate
/// and their note, or a refused merge's reason.
pub fn back_because(kind: &str, port: &str, output: &serde_json::Value) -> Option<String> {
    let text = |k: &str| output.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|t| !t.is_empty());
    match kind {
        "agent" => {
            let who =
                if text("by") == Some("user") { "The user" } else { text("persona").unwrap_or("The previous step") };
            let mut out = match text("summary") {
                Some(s) => format!("{who} answered `{port}`: {s}"),
                None => format!("{who} answered `{port}`."),
            };
            // Its line comments, each a finding to address (PDT-161).
            let comments: Vec<OutcomeComment> =
                output.get("comments").cloned().and_then(|c| serde_json::from_value(c).ok()).unwrap_or_default();
            if !comments.is_empty() {
                out.push_str("\n\nIts findings:");
                for c in &comments {
                    let at = match c.end_line {
                        Some(end) if end > c.line => format!("{}:{}-{end}", c.file, c.line),
                        _ => format!("{}:{}", c.file, c.line),
                    };
                    out.push_str(&format!("\n- `{at}`: {}", c.body.trim()));
                }
            }
            Some(out)
        }
        "gate" => Some(match text("text") {
            Some(t) => format!("The user chose `{port}`: {t}"),
            None => format!("The user chose `{port}`."),
        }),
        "attempt.merge" if port == "refused" => Some(match text("reason") {
            Some(r) => format!("The merge was refused: {r}."),
            None => "The merge was refused.".into(),
        }),
        _ => None,
    }
}

/// What a session is told when messages from workflows (PR notices: CI failed, a conflict, the base moved) meet a step
/// that hands in work (PDT-161). `own` are the session's other messages, its step's prompt first; `notices` the
/// workflows'. In one batch with the step's prompt they come after it, as more items for the pass; on their own while
/// such a step is working (`mid_step`) they say they belong to it. Otherwise they are just joined.
pub fn fold_messages(own: &[&str], notices: &[&str], mid_step: bool) -> String {
    let join = |v: &[&str]| v.iter().map(|t| t.trim()).collect::<Vec<_>>().join("\n\n");
    match (own.is_empty(), notices.is_empty()) {
        (_, true) => join(own),
        (false, false) => format!(
            "{}\n\n[{SHORT_NAME}] Also for this pass, once each point above is addressed: this came in while it \
             waited.\n\n{}",
            join(own),
            join(notices)
        ),
        (true, false) if mid_step => format!(
            "[{SHORT_NAME}] This came in while you work on your step. Take it in as one more item of the step, after \
             what the step's message asked, and finish the step as that message says.\n\n{}",
            join(notices)
        ),
        (true, false) => join(notices),
    }
}

/// The reminder a step's session gets when a turn ends without an outcome.
pub fn nudge_message(outcomes: &[OutcomeSpec]) -> String {
    format!(
        "[{SHORT_NAME}] You ended your turn without `submit_outcome`. Finish the step by calling it with one of: {}; \
         or call `ask_user` if you need the user.",
        outcome_list(outcomes)
    )
}

/// A nudge "When an agent is stuck" sends (PDT-367): the workflow's `message`, then the step's outcomes.
pub fn stuck_nudge_message(message: &str, outcomes: &[OutcomeSpec]) -> String {
    format!("[{SHORT_NAME}] {}\n\nThe step's outcomes: {}.", message.trim(), outcome_list(outcomes))
}

/// What the branch looks like when a step restarts in a fresh session (PDT-367).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchState {
    pub branch: String,
    /// Tracked files with uncommitted changes.
    pub changed: Vec<String>,
    /// Commits its upstream doesn't have.
    pub ahead: u32,
    /// The branch was never pushed.
    pub unpushed: bool,
    /// Its latest commits, newest first (`<short sha> <subject>`).
    pub commits: Vec<String>,
}

/// The first message of a fresh session a stuck step restarts in (PDT-367): the step's own first message
/// (`first`), then why it restarts, what the earlier session last said and the branch's state.
pub fn restart_message(first: &str, why: &str, last_message: &str, branch: Option<&BranchState>) -> String {
    let mut out = first.trim().to_string();
    out.push_str(&format!(
        "\n\n[{SHORT_NAME}] This is a fresh session: the step's earlier session got stuck ({}). Pick the work up \
         from where it is; don't start over.",
        why.trim().trim_end_matches('.')
    ));
    let last = last_message.trim();
    if !last.is_empty() {
        let quoted = last.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
        out.push_str(&format!("\n\nIts last message:\n\n{quoted}"));
    }
    if let Some(b) = branch {
        let mut state = vec![];
        if b.changed.is_empty() {
            state.push("no uncommitted changes".to_string());
        } else {
            const SHOWN: usize = 10;
            let mut files = b.changed.iter().take(SHOWN).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
            if b.changed.len() > SHOWN {
                files.push_str(&format!(" and {} more", b.changed.len() - SHOWN));
            }
            state.push(format!("uncommitted changes to {files}"));
        }
        if b.unpushed {
            state.push("never pushed".into());
        } else if b.ahead > 0 {
            state.push(format!(
                "{} not pushed",
                if b.ahead == 1 { "1 commit".into() } else { format!("{} commits", b.ahead) }
            ));
        }
        out.push_str(&format!("\n\nThe branch `{}`: {}.", b.branch, state.join("; ")));
        if !b.commits.is_empty() {
            out.push_str(&format!(
                "\nIts latest commits:\n{}",
                b.commits.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    out
}

/// What a step's session gets when the user resumes the step after it timed out or stopped (PDT-123): the user's note
/// first, if any, then why it stopped and how to finish.
pub fn resume_message(why: Option<&str>, note: Option<&str>, outcomes: &[OutcomeSpec]) -> String {
    let mut out = String::new();
    if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
        out.push_str(&format!("{n}\n\n"));
    }
    out.push_str(&format!("[{SHORT_NAME}] The user resumed your step"));
    match why.map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => out.push_str(&format!(" after it stopped: {w}")),
        None => out.push('.'),
    }
    out.push_str(&format!(
        "\n\nCarry on where you left off, then call `submit_outcome` with one of: {}; or call `ask_user` if you need \
         the user.",
        outcome_list(outcomes)
    ));
    out
}

/// The step a run can be resumed from (PDT-123): its latest agent step, when that one ended by `timeout` or
/// `stopped` (nothing after it in the run is an agent step, so the run has not moved on from it).
pub fn resumable_step(steps: &[super::api::Step]) -> Option<&super::api::Step> {
    let last = steps.iter().filter(|s| s.kind == "agent").max_by_key(|s| s.seq)?;
    (last.status == super::api::StepStatus::Succeeded && matches!(last.port.as_deref(), Some(TIMEOUT | STOPPED)))
        .then_some(last)
}

/// Why `submitted` is refused (PDT-161), or None when the worktree is clean and pushed: `changed` tracked files with
/// uncommitted changes, `ahead` commits `upstream` doesn't have, or a branch never pushed to the repository's remote
/// (`unpushed_branch`).
pub fn not_handed_in(changed: &[String], ahead: u32, upstream: Option<&str>, unpushed_branch: bool) -> Option<String> {
    if changed.is_empty() && ahead == 0 && !unpushed_branch {
        return None;
    }
    let mut what = vec![];
    if !changed.is_empty() {
        const SHOWN: usize = 5;
        let mut files = changed.iter().take(SHOWN).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
        if changed.len() > SHOWN {
            files.push_str(&format!(" and {} more", changed.len() - SHOWN));
        }
        what.push(format!("uncommitted changes to tracked files ({files})"));
    }
    if ahead > 0 {
        let commits = if ahead == 1 { "1 commit".to_string() } else { format!("{ahead} commits") };
        what.push(format!("{commits} not pushed to `{}`", upstream.unwrap_or("its upstream")));
    }
    if unpushed_branch {
        what.push("a branch that isn't pushed (it is not on `origin`)".to_string());
    }
    Some(format!(
        "`{SUBMITTED}` is refused: the worktree still has {}. The next step only sees what is committed and pushed: \
         commit and push it (run the checks first if anything changed), then submit again.",
        what.join(" and ")
    ))
}

/// The section a step's session gets in its instructions (spec §4.2).
pub fn step_section(persona: &str, instructions: &str, outcomes: &[OutcomeSpec]) -> String {
    let mut out = format!(
        "\n# Your role: {persona}\n\n\
         This session is a step in a {SHORT_NAME} workflow. {SHORT_NAME} moves the work on from the outcome you \
         submit, never from what you write.\n\
         - Finish by calling `submit_outcome` with one of: {}, and a short summary.\n\
         - If only the user can unblock you, call `ask_user` with your questions and end your turn; their answers \
         come back as a message. Keep each question to one line; put the background in `context`, each option's \
         trade-offs in its choice's `description`, and say which you recommend (`recommended`, `why`).\n\
         - Ending a turn without either counts as stalled.\n",
        outcome_list(outcomes)
    );
    let text = instructions.trim();
    if !text.is_empty() {
        out.push_str(&format!("\n{text}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes() -> Vec<OutcomeSpec> {
        vec![OutcomeSpec { id: "submitted".into(), description: "built".into() }]
    }

    /// PDT-367: the stuck workflow's nudge is its own message, then the step's outcomes.
    #[test]
    fn a_stuck_nudge_names_the_outcomes() {
        let m = stuck_nudge_message("  Finish with submit_outcome. ", &outcomes());
        assert_eq!(
            m,
            format!("[{SHORT_NAME}] Finish with submit_outcome.\n\nThe step's outcomes: `submitted` (built).")
        );
    }

    /// PDT-367: a fresh session gets the step's task, why it restarted, the last message and the branch.
    #[test]
    fn a_restart_message_carries_the_task_the_last_message_and_the_branch() {
        let b = BranchState {
            branch: "pdt-1".into(),
            changed: vec!["a.rs".into()],
            ahead: 2,
            unpushed: false,
            commits: vec!["abc123 Add a".into()],
        };
        let m = restart_message("Build it.", "Its turn ended without an outcome.", "Done\nmostly", Some(&b));
        assert!(m.starts_with("Build it.\n\n"), "{m}");
        assert!(m.contains("got stuck (Its turn ended without an outcome)"), "{m}");
        assert!(m.contains("> Done\n> mostly"), "{m}");
        assert!(m.contains("The branch `pdt-1`: uncommitted changes to `a.rs`; 2 commits not pushed."), "{m}");
        assert!(m.contains("- abc123 Add a"), "{m}");
        let clean = BranchState { branch: "b".into(), ..Default::default() };
        let m = restart_message("Build it.", "why", "", Some(&clean));
        assert!(!m.contains("last message") && m.contains("`b`: no uncommitted changes."), "{m}");
        assert!(!restart_message("Do.", "why", "", None).contains("branch"));
    }

    fn outs() -> Vec<OutcomeSpec> {
        vec![
            OutcomeSpec { id: "approved".into(), description: "it can merge".into() },
            OutcomeSpec { id: "changes_requested".into(), description: String::new() },
        ]
    }

    #[test]
    fn outcome_ids_are_port_names_but_not_the_reserved_ones() {
        assert!(valid_outcome_id("changes_requested").is_ok());
        assert!(valid_outcome_id("Approved").is_err());
        assert!(valid_outcome_id("").is_err());
        assert!(valid_outcome_id("timeout").is_err());
        assert!(valid_outcome_id("error").is_err());
    }

    #[test]
    fn the_first_message_has_the_task_ticket_and_outcomes() {
        let m = first_message(" Review it. ", Some(StepTicket { key: "PDT-4", title: "Pipeline" }), &outs());
        assert!(m.starts_with("Review it.\n\nTicket: PDT-4 — Pipeline"), "{m}");
        assert!(m.contains("`submit_outcome` with one of: `approved` (it can merge), `changes_requested`."), "{m}");
        assert!(!first_message("x", None, &outs()).contains("Ticket:"));
    }

    #[test]
    fn a_later_pass_says_why_it_is_back_and_not_the_whole_task_again() {
        let reason = back_because("gate", "reject", &serde_json::json!({"choice": "reject", "text": "Wrong colour."}));
        assert_eq!(reason.as_deref(), Some("The user chose `reject`: Wrong colour."));
        let m = next_pass_message(reason.as_deref(), &outs());
        assert!(
            m.starts_with("[PDT] Your step is back for another pass.\n\nThe user chose `reject`: Wrong colour."),
            "{m}"
        );
        assert!(m.ends_with("one of: `approved` (it can merge), `changes_requested`."), "{m}");
        assert!(!m.contains("Ticket:") && !m.contains("Your role"));
        let review = serde_json::json!({"persona": "Reviewer", "summary": "Add a test.", "by": "agent"});
        assert_eq!(
            back_because("agent", "changes_requested", &review).unwrap(),
            "Reviewer answered `changes_requested`: Add a test."
        );
        let mine = serde_json::json!({"persona": "Reviewer", "summary": "Add a test.", "by": "user"});
        assert_eq!(
            back_because("agent", "changes_requested", &mine).unwrap(),
            "The user answered `changes_requested`: Add a test."
        );
        assert_eq!(back_because("switch", "x", &serde_json::Value::Null), None);
        let refused = serde_json::json!({"code": "merge_blocked", "reason": "the pull request conflicts"});
        assert_eq!(
            back_because("attempt.merge", "refused", &refused).unwrap(),
            "The merge was refused: the pull request conflicts."
        );
        assert_eq!(back_because("attempt.merge", "queued", &serde_json::Value::Null), None);
        assert!(!next_pass_message(None, &outs()).contains("\n\n\n"));
    }

    #[test]
    fn a_later_pass_lists_the_findings_and_asks_for_each_one() {
        let qa = serde_json::json!({"persona": "QA", "summary": "Too slow.", "comments": [
            {"file": "src/a.rs", "line": 3, "body": " p95 is 900 ms, the target 200 ms. "},
            {"file": "src/b.rs", "line": 7, "endLine": 9, "body": "No test."}]});
        let reason = back_because("agent", "fail", &qa).unwrap();
        assert_eq!(
            reason,
            "QA answered `fail`: Too slow.\n\nIts findings:\n- `src/a.rs:3`: p95 is 900 ms, the target 200 ms.\n- \
             `src/b.rs:7-9`: No test."
        );
        let m = next_pass_message(Some(&reason), &outs());
        assert!(m.contains("No test.\n\nAddress each point above, one by one"), "{m}");
    }

    #[test]
    fn notices_come_after_a_steps_prompt_or_say_they_belong_to_the_step() {
        let pass = "[PDT] Your step is back for another pass.\n\nQA answered `fail`: Too slow.";
        let ci = "[PDT] CI failed on PR #4.";
        let m = fold_messages(&[pass], &[ci], false);
        assert!(m.starts_with(pass), "{m}");
        assert!(m.ends_with(&format!("this came in while it waited.\n\n{ci}")), "{m}");
        assert!(m.find("Too slow").unwrap() < m.find("CI failed").unwrap());
        let mid = fold_messages(&[], &[ci], true);
        assert!(mid.starts_with("[PDT] This came in while you work on your step.") && mid.ends_with(ci), "{mid}");
        assert!(!mid.contains("findings"), "a first pass has none: {mid}");
        assert_eq!(fold_messages(&[], &[ci, "two"], false), format!("{ci}\n\ntwo"));
        assert_eq!(fold_messages(&[pass, "nudge"], &[], true), format!("{pass}\n\nnudge"));
    }

    #[test]
    fn submitted_needs_a_clean_pushed_worktree() {
        assert_eq!(not_handed_in(&[], 0, Some("origin/x"), false), None);
        let files: Vec<String> = (1..=7).map(|i| format!("f{i}.rs")).collect();
        let m = not_handed_in(&files, 2, Some("origin/x"), false).unwrap();
        assert!(
            m.starts_with("`submitted` is refused: the worktree still has uncommitted changes to tracked files"),
            "{m}"
        );
        assert!(m.contains("`f5.rs` and 2 more) and 2 commits not pushed to `origin/x`."), "{m}");
        let m = not_handed_in(&[], 1, None, false).unwrap();
        assert!(m.contains("still has 1 commit not pushed to `its upstream`."), "{m}");
        let m = not_handed_in(&["a.rs".into()], 0, None, true).unwrap();
        assert!(m.contains("(`a.rs`) and a branch that isn't pushed (it is not on `origin`)."), "{m}");
    }

    #[test]
    fn a_resumed_step_hears_the_users_note_first_then_why_and_how_to_finish() {
        let m = resume_message(Some("8 turns ended without an outcome"), Some(" Push what you have. "), &outs());
        assert!(
            m.starts_with("Push what you have.\n\n[PDT] The user resumed your step after it stopped: 8 turns"),
            "{m}"
        );
        assert!(m.contains("`submit_outcome` with one of: `approved` (it can merge), `changes_requested`"), "{m}");
        let plain = resume_message(None, Some("  "), &outs());
        assert!(plain.starts_with("[PDT] The user resumed your step.\n\nCarry on"), "{plain}");
    }

    #[test]
    fn a_run_resumes_from_its_latest_agent_step_when_that_timed_out_or_stopped() {
        use super::super::api::{Step, StepStatus};
        let step = |seq: i64, kind: &str, port: Option<&str>| Step {
            id: format!("s{seq}"),
            run_id: "r".into(),
            seq,
            node_id: format!("n{seq}"),
            kind: kind.into(),
            status: StepStatus::Succeeded,
            input: serde_json::Value::Null,
            output: serde_json::Value::Null,
            port: port.map(String::from),
            attempts: 1,
            error: None,
            wake_at: None,
            wait: serde_json::Value::Null,
            started_at: None,
            ended_at: None,
        };
        let timed_out = vec![step(1, "manual", Some("out")), step(2, "agent", Some("timeout")), step(3, "end", None)];
        assert_eq!(resumable_step(&timed_out).map(|s| s.id.as_str()), Some("s2"));
        let stopped = vec![step(2, "agent", Some("stopped")), step(1, "manual", Some("out"))];
        assert_eq!(resumable_step(&stopped).map(|s| s.id.as_str()), Some("s2"));
        // The run moved on: a later agent step, or the step ended by an outcome.
        let moved = vec![step(2, "agent", Some("timeout")), step(3, "agent", Some("approved"))];
        assert!(resumable_step(&moved).is_none());
        assert!(resumable_step(&[step(2, "agent", Some("approved"))]).is_none());
        let mut waiting = step(2, "agent", None);
        waiting.status = StepStatus::Waiting;
        assert!(resumable_step(&[waiting]).is_none());
        assert!(resumable_step(&[step(1, "manual", Some("out"))]).is_none());
    }

    #[test]
    fn nudges_and_the_step_section_name_the_outcomes() {
        assert!(nudge_message(&outs()).starts_with("[PDT] You ended your turn without `submit_outcome`."));
        let s = step_section("Reviewer", "Be strict.\n", &outs());
        assert!(s.contains("# Your role: Reviewer"));
        assert!(s.contains("`approved` (it can merge), `changes_requested`"));
        // How to ask (PDT-476): one line, the background and trade-offs apart, a recommendation.
        assert!(s.contains("one line; put the background in `context`") && s.contains("(`recommended`, `why`)"), "{s}");
        assert!(s.ends_with("\nBe strict.\n"));
        assert!(!step_section("R", "  ", &outs()).ends_with("\n\n"));
    }
}
