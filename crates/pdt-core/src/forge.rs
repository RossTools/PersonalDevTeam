//! Forges (PDT-176): where a project's change requests, CI, reviews and issues live. GitHub is one; the trait the
//! SDLC talks to is `pdt_forge::Forge`. This module holds what pure code needs: which forge a project names, and what
//! a forge tells agents.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The forge a project names in `.pdt/project.toml` (`forge = "github"`). Without one, a project whose `origin` is
/// on github.com uses GitHub and any other has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ForgeKind {
    Github,
    /// PDT itself (PDT-178), for a repository with no remote: a change request is the attempt's branch compared with
    /// its base, CI is the `[ci]` command run in the attempt's worktree, and merging is a local merge into the base.
    Local,
    /// No forge: no change requests, CI or issues are read; merges are local.
    None,
}

/// What a forge adds to the instructions of an agent session on ticket work (`agent_context`): how to read CI and
/// answer on a change request with its tools. Each part is left out when None.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeNotes {
    /// One line on reading CI for the session's branch; `{branch}` stands for it.
    pub ci: Option<String>,
    /// How to answer a comment on a change request, e.g. "`gh pr comment`, or a reply to the review comment".
    pub reply: Option<String>,
    /// The command that watches CI in a shell, named as one not to run.
    pub watch: Option<String>,
}
