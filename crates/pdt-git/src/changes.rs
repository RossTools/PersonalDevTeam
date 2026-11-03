//! The files an attempt changed (tickets spec §7, actual overlap): everything that differs from where the branch
//! left its base, committed or not, plus untracked files. Read-only.

use crate::{GitError, git_with_env};
use std::collections::BTreeSet;
use std::path::Path;

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    git_with_env(dir, args, &[])?.map_err(|f| GitError::Command(f.stderr))
}

fn nul_list(out: &str) -> impl Iterator<Item = &str> {
    out.split('\0').filter(|s| !s.is_empty())
}

/// The checkout-relative paths (always `/`-separated, sorted) that the worktree's branch changed since its merge base
/// with `base` (a branch, or `fallback`, the commit it started from, when there is no common ancestor or `base` is
/// gone): committed, staged and unstaged changes (a rename lists both names) and untracked files that are not ignored.
pub fn changed_files(worktree: &Path, base: &str, fallback: &str) -> Result<Vec<String>, GitError> {
    let mb = crate::merge_base(worktree, "HEAD", base).unwrap_or_else(|| fallback.to_string());
    let mut files = BTreeSet::new();
    let diff = run(
        worktree,
        &["-c", "core.quotepath=off", "diff", "--name-only", "--no-renames", "-z", "--end-of-options", &mb, "--"],
    )?;
    files.extend(nul_list(&diff).map(String::from));
    let untracked = run(worktree, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    files.extend(nul_list(&untracked).map(String::from));
    Ok(files.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
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

    fn write(dir: &Path, file: &str, text: &str) {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn commit_all(dir: &Path, msg: &str) -> String {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// `main` with a few files and a worktree on `feature` started there. Returns (repo, worktree, base commit).
    fn setup(tmp: &Path) -> (PathBuf, PathBuf, String) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        write(&repo, "a.txt", "a\n");
        write(&repo, "src/b.rs", "b\n");
        write(&repo, "src/old name.rs", "old\n");
        write(&repo, "gone.txt", "gone\n");
        write(&repo, ".gitignore", "*.log\n");
        let base = commit_all(&repo, "base");
        let wt = tmp.join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "feature", wt.to_str().unwrap(), "main"]);
        (repo, wt, base)
    }

    #[test]
    fn committed_staged_unstaged_untracked_deleted_and_renamed_files_count() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, base) = setup(tmp.path());
        assert!(changed_files(&wt, "main", &base).unwrap().is_empty());

        write(&wt, "src/b.rs", "changed\n");
        commit_all(&wt, "commit b");
        git(&wt, &["mv", "src/old name.rs", "src/new name.rs"]);
        commit_all(&wt, "rename");
        write(&wt, "a.txt", "staged\n");
        git(&wt, &["add", "a.txt"]);
        std::fs::remove_file(wt.join("gone.txt")).unwrap();
        write(&wt, "web/new.tsx", "untracked\n");
        write(&wt, "debug.log", "ignored\n");
        // main moves on: its own changes are not the branch's.
        write(&repo, "main-only.txt", "m\n");
        commit_all(&repo, "main moves");

        assert_eq!(
            changed_files(&wt, "main", &base).unwrap(),
            ["a.txt", "gone.txt", "src/b.rs", "src/new name.rs", "src/old name.rs", "web/new.tsx"]
        );
    }

    #[test]
    fn a_missing_base_falls_back_to_the_start_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let (_repo, wt, base) = setup(tmp.path());
        write(&wt, "a.txt", "x\n");
        assert_eq!(changed_files(&wt, "no-such-branch", &base).unwrap(), ["a.txt"]);
        assert!(changed_files(&tmp.path().join("missing"), "main", &base).is_err());
    }
}
