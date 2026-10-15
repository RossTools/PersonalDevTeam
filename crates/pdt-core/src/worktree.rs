//! Git worktree records. Validation here is a fast pre-check; `git check-ref-format` is the final word.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Worktree {
    pub id: String,
    pub project_id: String,
    pub branch: String,
    /// Absolute path, native separators.
    pub path: String,
    pub base_commit: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewWorktree {
    pub branch: String,
    /// Branch or commit to start from; defaults to the project's base branch.
    #[serde(default)]
    #[ts(optional)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidWorktree {
    pub branch: String,
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WorktreeValidationError {
    #[error("branch name must not be empty")]
    EmptyBranch,
    #[error("invalid branch name: {0}")]
    InvalidBranch(String),
}

impl NewWorktree {
    pub fn validate(&self) -> Result<ValidWorktree, WorktreeValidationError> {
        let branch = self.branch.trim();
        if branch.is_empty() {
            return Err(WorktreeValidationError::EmptyBranch);
        }
        let bad = branch.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
            || branch.starts_with('-')
            || branch.starts_with('/')
            || branch.ends_with('/')
            || branch.ends_with('.')
            || branch.ends_with(".lock")
            || branch.contains("..")
            || branch.contains("//")
            || branch.contains("@{");
        if bad {
            return Err(WorktreeValidationError::InvalidBranch(branch.into()));
        }
        let base = self.base.as_deref().map(str::trim).filter(|b| !b.is_empty()).map(String::from);
        Ok(ValidWorktree { branch: branch.into(), base })
    }
}

/// Directory (relative to the repo root) that holds a project's worktrees. pdt-git keeps it in `.git/info/exclude`.
pub const WORKTREES_DIR: &str = ".worktrees";

/// `<repo>/.worktrees/<branch, with '/' replaced by '-'>`.
pub fn worktree_path(repo_root: &Path, branch: &str) -> PathBuf {
    repo_root.join(WORKTREES_DIR).join(branch.replace('/', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nw(branch: &str, base: Option<&str>) -> NewWorktree {
        NewWorktree { branch: branch.into(), base: base.map(Into::into) }
    }

    #[test]
    fn trims_and_defaults_base() {
        assert_eq!(
            nw(" feature/x ", Some(" ")).validate().unwrap(),
            ValidWorktree { branch: "feature/x".into(), base: None }
        );
        assert_eq!(nw("a", Some("dev")).validate().unwrap().base.as_deref(), Some("dev"));
    }

    #[test]
    fn rejects_bad_branch_names() {
        assert_eq!(nw("  ", None).validate(), Err(WorktreeValidationError::EmptyBranch));
        for bad in [
            "has space",
            "-flag",
            "a..b",
            "x.lock",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "a@{b",
            "a/",
            "/a",
            "a//b",
            "a.",
        ] {
            assert!(matches!(nw(bad, None).validate(), Err(WorktreeValidationError::InvalidBranch(_))), "{bad}");
        }
    }

    #[test]
    fn path_is_inside_the_repo() {
        let repo = Path::new("work").join("my repo");
        assert_eq!(worktree_path(&repo, "feature/x"), repo.join(".worktrees").join("feature-x"));
    }
}
