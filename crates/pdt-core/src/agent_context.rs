//! What PDT tells every agent session it starts, beyond the repository's own CLAUDE.md / AGENTS.md: where PDT is,
//! which session, project and checkout this is, where PDT's guides are, and the project's own instructions from PDT's
//! settings. This lets a repository PDT cannot (or should not) commit files to still get a workflow.

use crate::brand::{PRODUCT_NAME, SHORT_NAME};

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
         configured. Read them when the work involves {SHORT_NAME}.\n",
        session = ctx.session_id,
        name = ctx.project_name,
        key = ctx.project_key,
        project = ctx.project_id,
        repo = ctx.repo_path,
    );
    if let Some(text) = project.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str(&format!(
            "\n# Instructions for this project\n\n\
             From {SHORT_NAME}'s project settings (kept on this host, not in the repository). Follow them as you \
             would the repository's own instructions.\n\n{text}\n"
        ));
    }
    out
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
        }
    }

    #[test]
    fn names_the_api_session_project_and_guides() {
        let t = session_instructions(&ctx(None), None);
        assert!(t.contains("http://127.0.0.1:7420/api"), "{t}");
        assert!(t.contains("`s1`") && t.contains("`p1`") && t.contains("key `WA`"), "{t}");
        assert!(t.contains("the main checkout `/r` (base branch `main`)"), "{t}");
        assert!(t.contains("/guides/agents") && t.contains("/guides/run-config"), "{t}");
        assert!(!t.contains("Instructions for this project"), "no project section without instructions");
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
