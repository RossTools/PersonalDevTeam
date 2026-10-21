//! What PDT tells every agent session it starts, beyond the repository's own CLAUDE.md / AGENTS.md: where PDT is,
//! which session, project and checkout this is, where PDT's guides are, and the project's own instructions from PDT's
//! settings. This lets a repository PDT cannot (or should not) commit files to still get a workflow. A session working
//! on a ticket also learns the rules for ticket work and which `[PDT]` messages the enabled workflows will send it.

use crate::brand::{PRODUCT_NAME, SHORT_NAME};
use crate::workflow::events as ev;
use std::collections::BTreeSet;

/// The longest project instructions PDT accepts (bytes).
pub const MAX_PROJECT_INSTRUCTIONS: usize = 16 * 1024;

/// Where an agent session runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext<'a> {
    /// pdtd's own port, on 127.0.0.1.
    pub port: u16,
    pub session_id: &'a str,
    pub project_id: &'a str,
    pub project_name: &'a str,
    pub project_key: &'a str,
    /// The checkout the session runs in.
    pub checkout: &'a str,
    /// The worktree's branch; None for the main checkout.
    pub branch: Option<&'a str>,
    /// The project's main checkout and base branch.
    pub repo_path: &'a str,
    pub base_branch: &'a str,
    /// The ticket, when the session works on one (an active attempt's session).
    pub ticket: Option<TicketWork<'a>>,
}

/// The ticket a session works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketWork<'a> {
    pub ticket_id: &'a str,
    /// `PDT-42`.
    pub key: &'a str,
    pub branch: &'a str,
    /// The attempt's base branch.
    pub base: &'a str,
    /// The event types enabled workflows turn into messages to this session (`workflow::news::agent_news`).
    pub news: BTreeSet<&'static str>,
}

/// The instructions text for one session: PDT's context, then the project's own instructions when it has any.
pub fn session_instructions(ctx: &SessionContext, project: Option<&str>) -> String {
    let api = format!("http://127.0.0.1:{}/api", ctx.port);
    let place = match ctx.branch {
        Some(b) => format!("worktree `{}` on branch `{b}`", ctx.checkout),
        None => format!("the main checkout `{}` (base branch `{}`)", ctx.checkout, ctx.base_branch),
    };
    let mut out = format!(
        "# Running in {PRODUCT_NAME} ({SHORT_NAME})\n\n\
         {SHORT_NAME} started this session. Its API is {api} (local only, no authentication).\n\
         - This session: `{session}` (`GET {api}/sessions/{session}`).\n\
         - Project: {name} (key `{key}`, id `{project}`), main checkout `{repo}`. This session works in {place}.\n\
         - {SHORT_NAME}'s guides for agents, as Markdown: `GET {api}/guides` lists them; `GET {api}/guides/agents` \
         covers tickets, worktrees, runs and tests; `GET {api}/guides/run-config` is how runs and tests are \
         configured. Read them when the work involves {SHORT_NAME}.\n\
         - Make a worktree for a new branch with `POST {api}/projects/{project}/worktrees`, and start the project's \
         runs and tests with `POST {api}/projects/{project}/runs`, so {SHORT_NAME} knows about them.\n",
        session = ctx.session_id,
        name = ctx.project_name,
        key = ctx.project_key,
        project = ctx.project_id,
        repo = ctx.repo_path,
    );
    if let Some(t) = &ctx.ticket {
        out.push_str(&ticket_section(&api, t));
    }
    if let Some(text) = project.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str(&format!(
            "\n# Instructions for this project\n\n\
             From {SHORT_NAME}'s project settings (kept on this host, not in the repository). Follow them as you \
             would the repository's own instructions.\n\n{text}\n"
        ));
    }
    out
}

fn ticket_section(api: &str, t: &TicketWork) -> String {
    let (key, branch) = (t.key, t.branch);
    let mut out = format!(
        "\n## This session's ticket\n\n\
         This session works on ticket {key} (`GET {api}/tickets/{id}`) on branch `{branch}`.\n\
         - Follow-up work that turns up goes in a ticket of its own, not into this change (the project's instructions \
         say whether to ask first). Comment on the ticket with anything the user should know.\n\
         - Don't move the ticket to Done, remove its worktree or delete its branch unless the user asks: {SHORT_NAME} \
         or the user does that once the work is merged.\n",
        id = t.ticket_id,
    );
    let has = |kind: &str| t.news.contains(kind);
    let merged = has(ev::PR_MERGED) || has(ev::ATTEMPT_MERGED);
    let base = format!("`{}`", t.base);
    let events: Vec<String> = [
        (has(ev::CI_RUN_FAILED), "CI fails on its pull request".to_string()),
        (has(ev::CI_RUN_PASSED), "CI passes".into()),
        (has(ev::PR_CONFLICT), format!("the pull request conflicts with {base}")),
        (has(ev::PR_QUEUE_LEFT), "the merge queue drops it".into()),
        (has(ev::PR_BEHIND), format!("{base} moves while it waits to merge")),
        (merged, "it is merged".into()),
        (has(ev::ATTEMPT_ABANDONED), "the attempt is abandoned".into()),
        (has(ev::TICKET_STAGE_CHANGED), "the ticket moves to another stage".into()),
        (has(ev::TICKET_COMMENTED), "the user comments on the ticket".into()),
        (has(ev::TICKET_OVERLAP_FOUND), "another ticket in flight changes the same files".into()),
    ]
    .into_iter()
    .filter_map(|(on, what)| on.then_some(what))
    .collect();
    if events.is_empty() {
        return out;
    }
    out.push_str(&format!(
        "\n## Messages from {SHORT_NAME}\n\n\
         Workflows the user enabled send this session a message starting `[{SHORT_NAME}]` when {}. Each says what \
         happened; act on it as it asks:\n",
        sentence_list(&events),
    ));
    let rules = [
        (
            has(ev::CI_RUN_FAILED),
            "CI failed: read the failed jobs' logs (the message names the run), fix the cause, \
                                  run the checks and push.",
        ),
        (
            has(ev::PR_CONFLICT) || has(ev::PR_BEHIND),
            "Conflict, or the base moved: merge the base branch in, resolve \
                                  any conflicts, run the checks and push.",
        ),
        (
            has(ev::PR_QUEUE_LEFT),
            "Dropped from the merge queue: fix what it names, run the checks, push, and turn \
                                  auto-merge back on as it says.",
        ),
        (merged, "Merged: the work is in. Push nothing more to the branch and start nothing new on it."),
        (
            has(ev::TICKET_COMMENTED),
            "A ticket comment: the user wrote it; answer or act on it as you would their own \
                                  message.",
        ),
    ];
    for (_, rule) in rules.iter().filter(|(on, _)| *on) {
        out.push_str(&format!("- {rule}\n"));
    }
    let watched =
        [ev::CI_RUN_FAILED, ev::CI_RUN_PASSED, ev::PR_CONFLICT, ev::PR_QUEUE_LEFT, ev::PR_MERGED, ev::PR_BEHIND];
    if merged || watched.iter().any(|k| has(k)) {
        out.push_str(&format!(
            "\n{SHORT_NAME} watches the pull request and sends these, so don't poll or watch CI in a shell (`gh run watch`, `gh pr checks --watch`, sleep loops): push, then carry \
             on or end your turn.\n"
        ));
    }
    out
}

/// `a`, `a or b`, `a, b or c`.
fn sentence_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Project instructions as saved: trimmed, None when empty. Err when longer than `MAX_PROJECT_INSTRUCTIONS`.
pub fn clean_project_instructions(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = raw.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    if t.len() > MAX_PROJECT_INSTRUCTIONS {
        return Err(format!("agent instructions are limited to {} KB", MAX_PROJECT_INSTRUCTIONS / 1024));
    }
    Ok(Some(t.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(branch: Option<&'static str>) -> SessionContext<'static> {
        SessionContext {
            port: 7420,
            session_id: "s1",
            project_id: "p1",
            project_name: "Work App",
            project_key: "WA",
            checkout: if branch.is_some() { "/r/.worktrees/x" } else { "/r" },
            branch,
            repo_path: "/r",
            base_branch: "main",
            ticket: None,
        }
    }

    fn on_ticket(news: &[&'static str]) -> String {
        let mut c = ctx(Some("pdt-WA-7-x"));
        c.ticket = Some(TicketWork {
            ticket_id: "t7",
            key: "WA-7",
            branch: "pdt-WA-7-x",
            base: "main",
            news: news.iter().copied().collect(),
        });
        session_instructions(&c, None)
    }

    #[test]
    fn a_ticket_session_gets_the_ticket_rules() {
        let t = on_ticket(&[]);
        assert!(t.contains("works on ticket WA-7 (`GET http://127.0.0.1:7420/api/tickets/t7`)"), "{t}");
        assert!(t.contains("Follow-up work") && t.contains("Don't move the ticket to Done"), "{t}");
        assert!(!t.contains("Messages from") && !t.contains("gh run watch"), "no workflow, no messages: {t}");
        let plain = session_instructions(&ctx(Some("pdt-WA-7-x")), None);
        assert!(!plain.contains("This session's ticket"), "{plain}");
        assert!(plain.contains("/projects/p1/worktrees") && plain.contains("/projects/p1/runs"), "{plain}");
    }

    #[test]
    fn pr_notices_mean_no_watching_ci() {
        let t = on_ticket(&[ev::CI_RUN_FAILED, ev::PR_CONFLICT, ev::PR_QUEUE_LEFT, ev::PR_MERGED, ev::ATTEMPT_MERGED]);
        assert!(
            t.contains(
                "when CI fails on its pull request, the pull request conflicts with `main`, the merge queue drops it \
                 or it is merged."
            ),
            "{t}"
        );
        assert!(t.contains("- CI failed:") && t.contains("- Conflict") && t.contains("- Dropped from the merge queue"));
        assert!(t.contains("- Merged: the work is in. Push nothing more"), "{t}");
        assert!(t.contains("don't poll or watch CI in a shell"), "{t}");
        assert!(!t.contains("comments on the ticket") && !t.contains("- A ticket comment"), "{t}");
    }

    #[test]
    fn comments_alone_say_nothing_about_ci() {
        let t = on_ticket(&[ev::TICKET_COMMENTED]);
        assert!(t.contains("when the user comments on the ticket. ") && t.contains("- A ticket comment:"), "{t}");
        assert!(!t.contains("gh run watch") && !t.contains("- Merged") && !t.contains("- CI failed"), "{t}");
    }

    #[test]
    fn a_merge_alone_still_means_no_watching() {
        let t = on_ticket(&[ev::ATTEMPT_MERGED]);
        assert!(t.contains("when it is merged. ") && t.contains("gh run watch"), "{t}");
    }

    #[test]
    fn names_the_api_session_project_and_guides() {
        let t = session_instructions(&ctx(None), None);
        assert!(t.contains("http://127.0.0.1:7420/api"), "{t}");
        assert!(t.contains("`s1`") && t.contains("`p1`") && t.contains("key `WA`"), "{t}");
        assert!(t.contains("the main checkout `/r` (base branch `main`)"), "{t}");
        assert!(t.contains("/guides/agents") && t.contains("/guides/run-config"), "{t}");
        assert!(!t.contains("Instructions for this project"), "no project section without instructions");
        assert!(!t.contains("This session's ticket"), "{t}");
    }

    #[test]
    fn names_the_worktree_branch() {
        let t = session_instructions(&ctx(Some("feat/x")), None);
        assert!(t.contains("worktree `/r/.worktrees/x` on branch `feat/x`"), "{t}");
    }

    #[test]
    fn appends_the_project_instructions_after_the_context() {
        let t = session_instructions(&ctx(None), Some("  Run `make check` before pushing.\n"));
        let at = t.find("# Instructions for this project").expect("section");
        assert!(at > t.find("guides/agents").unwrap());
        assert!(t.ends_with("Run `make check` before pushing.\n"), "{t}");
        assert_eq!(session_instructions(&ctx(None), Some(" \n ")), session_instructions(&ctx(None), None));
    }

    #[test]
    fn cleans_and_limits_project_instructions() {
        assert_eq!(clean_project_instructions(None), Ok(None));
        assert_eq!(clean_project_instructions(Some("  ")), Ok(None));
        assert_eq!(clean_project_instructions(Some(" a\n")), Ok(Some("a".into())));
        assert!(clean_project_instructions(Some(&"x".repeat(MAX_PROJECT_INSTRUCTIONS))).is_ok());
        assert!(clean_project_instructions(Some(&"x".repeat(MAX_PROJECT_INSTRUCTIONS + 1))).is_err());
    }
}
