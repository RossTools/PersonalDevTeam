//! Bringing a local branch up to date with `origin` (PDT-56): a fast-forward only, never a merge, and never touching
//! uncommitted work. Blocking, like the rest.

use crate::{GitError, git_with_env, is_ancestor, list_worktrees, remote_url, rev_parse};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What [`fast_forward`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum FastForward {
    /// The branch moved from `from` to `to`.
    Updated { from: String, to: String },
    /// The branch already has everything `origin` has (or more).
    UpToDate,
    /// The repository has no `origin`.
    NoRemote,
    /// There is no local branch of that name.
    NoLocalBranch,
    /// `origin` has no branch of that name, or the fetch failed (`message`).
    FetchFailed { message: String },
    /// The local branch has commits `origin` lacks, and `origin` has commits it lacks: left alone.
    Diverged,
    /// The branch is checked out in `checkout` and git refused to move it there (uncommitted changes in the way,
    /// a merge or rebase in progress…).
    Blocked { checkout: PathBuf, message: String },
}

/// Fetches `branch` from `origin` and fast-forwards the local `branch` to it. Where the branch is checked out (the
/// main checkout or a worktree), that checkout is updated with `git merge --ff-only`, which refuses rather than
/// overwrite uncommitted changes; elsewhere only the ref moves (and only if nobody moved it meanwhile).
pub fn fast_forward(repo: &Path, branch: &str, timeout: Duration) -> Result<FastForward, GitError> {
    if remote_url(repo, "origin").is_none() {
        return Ok(FastForward::NoRemote);
    }
    let local_ref = format!("refs/heads/{branch}");
    let Some(local) = rev_parse(repo, &local_ref) else { return Ok(FastForward::NoLocalBranch) };
    if let Err(e) = crate::fetch(repo, "origin", branch, timeout) {
        return Ok(FastForward::FetchFailed { message: e.to_string() });
    }
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let Some(remote) = rev_parse(repo, &remote_ref) else {
        return Ok(FastForward::FetchFailed { message: format!("origin has no branch {branch}") });
    };
    if is_ancestor(repo, &remote, &local) {
        return Ok(FastForward::UpToDate);
    }
    if !is_ancestor(repo, &local, &remote) {
        return Ok(FastForward::Diverged);
    }
    let checkout = list_worktrees(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(branch)).map(|w| w.path);
    let result = match &checkout {
        Some(dir) => git_with_env(dir, &["merge", "--ff-only", "--quiet", &remote], &[])?,
        None => git_with_env(repo, &["update-ref", &local_ref, &remote, &local], &[])?,
    };
    match result {
        Ok(_) => Ok(FastForward::Updated { from: local, to: remote }),
        Err(f) => {
            Ok(FastForward::Blocked { checkout: checkout.unwrap_or_else(|| repo.to_path_buf()), message: f.stderr })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(["-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str, text: &str) -> String {
        std::fs::write(dir.join(file), text).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("edit {file}")]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// An upstream repo on `main` with one commit, and a clone of it. Returns (upstream, clone).
    fn upstream_and_clone(tmp: &Path) -> (PathBuf, PathBuf) {
        let upstream = tmp.join("upstream");
        std::fs::create_dir_all(&upstream).unwrap();
        git(&upstream, &["init", "-q", "-b", "main"]);
        commit(&upstream, "a.txt", "a\n");
        let clone = tmp.join("clone");
        git(tmp, &["clone", "-q", &upstream.to_string_lossy(), &clone.to_string_lossy()]);
        // The checkouts PDT updates run with the host's own config; keep files byte-exact on Windows runners.
        git(&clone, &["config", "core.autocrlf", "false"]);
        (upstream, clone)
    }

    const T: Duration = Duration::from_secs(20);

    #[test]
    fn the_checked_out_base_is_fast_forwarded_with_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        let before = rev_parse(&clone, "main").unwrap();
        assert_eq!(fast_forward(&clone, "main", T).unwrap(), FastForward::UpToDate);
        let next = commit(&upstream, "b.txt", "b\n");
        // An untracked file the update doesn't touch is no obstacle.
        std::fs::write(clone.join("notes.txt"), "mine\n").unwrap();
        assert_eq!(fast_forward(&clone, "main", T).unwrap(), FastForward::Updated { from: before, to: next.clone() });
        assert_eq!(rev_parse(&clone, "main").as_deref(), Some(next.as_str()));
        assert_eq!(std::fs::read_to_string(clone.join("b.txt")).unwrap(), "b\n");
        assert_eq!(std::fs::read_to_string(clone.join("notes.txt")).unwrap(), "mine\n");
    }

    #[test]
    fn a_base_not_checked_out_moves_only_its_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        git(&clone, &["checkout", "-q", "-b", "feature"]);
        let next = commit(&upstream, "b.txt", "b\n");
        assert!(matches!(fast_forward(&clone, "main", T).unwrap(), FastForward::Updated { .. }));
        assert_eq!(rev_parse(&clone, "main").as_deref(), Some(next.as_str()));
        assert!(!clone.join("b.txt").exists(), "the feature checkout is untouched");
    }

    #[test]
    fn a_base_checked_out_in_a_worktree_is_updated_there() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        git(&clone, &["checkout", "-q", "-b", "feature"]);
        let wt = tmp.path().join("wt");
        git(&clone, &["worktree", "add", "-q", &wt.to_string_lossy(), "main"]);
        let next = commit(&upstream, "b.txt", "b\n");
        assert!(matches!(fast_forward(&clone, "main", T).unwrap(), FastForward::Updated { .. }));
        assert_eq!(rev_parse(&clone, "main").as_deref(), Some(next.as_str()));
        assert!(wt.join("b.txt").exists());
    }

    #[test]
    fn uncommitted_changes_in_the_way_block_it_and_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        let before = rev_parse(&clone, "main").unwrap();
        commit(&upstream, "a.txt", "changed upstream\n");
        std::fs::write(clone.join("a.txt"), "changed here\n").unwrap();
        assert!(matches!(fast_forward(&clone, "main", T).unwrap(), FastForward::Blocked { .. }));
        assert_eq!(rev_parse(&clone, "main").as_deref(), Some(before.as_str()));
        assert_eq!(std::fs::read_to_string(clone.join("a.txt")).unwrap(), "changed here\n");
    }

    #[test]
    fn local_commits_are_never_merged_or_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        let mine = commit(&clone, "m.txt", "m\n");
        assert_eq!(fast_forward(&clone, "main", T).unwrap(), FastForward::UpToDate, "ahead of origin");
        commit(&upstream, "b.txt", "b\n");
        assert_eq!(fast_forward(&clone, "main", T).unwrap(), FastForward::Diverged);
        assert_eq!(rev_parse(&clone, "main").as_deref(), Some(mine.as_str()));
    }

    #[test]
    fn no_remote_no_branch_and_a_failed_fetch_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, clone) = upstream_and_clone(tmp.path());
        assert_eq!(fast_forward(&upstream, "main", T).unwrap(), FastForward::NoRemote);
        assert_eq!(fast_forward(&clone, "nope", T).unwrap(), FastForward::NoLocalBranch);
        git(&clone, &["branch", "-q", "only-here"]);
        assert!(matches!(fast_forward(&clone, "only-here", T).unwrap(), FastForward::FetchFailed { .. }));
    }
}
