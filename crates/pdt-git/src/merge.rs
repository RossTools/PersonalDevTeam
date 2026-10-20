//! Merge detection for ticket attempts (tickets spec §6): is a branch's work in the base branch, by a merge, a
//! fast-forward or a squash? Read-only: no refs are created and no working tree is touched (a squash check writes one
//! unreferenced commit object, which `git gc` collects).

use crate::{GitError, PDT_COMMIT_CONFIG, git_with_env};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The commit `rev` names (`<rev>^{commit}`), or None when it names none.
pub fn rev_parse(repo: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    match git_with_env(repo, &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec], &[]) {
        Ok(Ok(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Whether commit `a` is an ancestor of (or the same as) commit `b`.
pub fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
    matches!(git_with_env(repo, &["merge-base", "--is-ancestor", a, b], &[]), Ok(Ok(_)))
}

/// The best common ancestor of `a` and `b`.
pub fn merge_base(repo: &Path, a: &str, b: &str) -> Option<String> {
    match git_with_env(repo, &["merge-base", a, b], &[]) {
        Ok(Ok(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Whether the whole change of `head` since it left `base` (one squashed patch: `head`'s tree on top of their merge
/// base) is already in `base` as an equivalent patch, as after a squash merge. False when `head` changes nothing.
pub fn patch_in(repo: &Path, head: &str, base: &str) -> bool {
    let Some(mb) = merge_base(repo, head, base) else { return false };
    let tree = |c: &str| rev_parse_tree(repo, c);
    let Some(head_tree) = tree(head) else { return false };
    if tree(&mb).as_deref() == Some(head_tree.as_str()) {
        return false;
    }
    let mut args = PDT_COMMIT_CONFIG.to_vec();
    args.extend(["commit-tree", &head_tree, "-p", &mb, "-m", "pdt merge check"]);
    let squashed = match git_with_env(repo, &args, &[]) {
        Ok(Ok(s)) if !s.is_empty() => s,
        _ => return false,
    };
    // `-` marks a commit whose patch is already upstream.
    match git_with_env(repo, &["cherry", base, &squashed], &[]) {
        Ok(Ok(out)) => out.lines().any(|l| l.starts_with("- ")),
        _ => false,
    }
}

fn rev_parse_tree(repo: &Path, commit: &str) -> Option<String> {
    let spec = format!("{commit}^{{tree}}");
    match git_with_env(repo, &["rev-parse", "--verify", "--quiet", &spec], &[]) {
        Ok(Ok(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Whether the branch whose head is `head` (started at `base_commit`) holds commits of its own: it moved from
/// `base_commit` to a commit that none of `bases` (the ones that exist) contains. A branch that only caught up with
/// its base (fast-forwarded to a newer main) has none.
pub fn has_own_commits(repo: &Path, head: &str, base_commit: &str, bases: &[&str]) -> bool {
    head != base_commit && !bases.iter().filter_map(|b| rev_parse(repo, b)).any(|base| is_ancestor(repo, head, &base))
}

/// Whether the branch whose head is `head` (started at `base_commit`) is merged into any of `bases` (branch names or
/// refs such as `main` and `origin/main`; ones that don't exist are skipped). A branch with no commits of its own
/// (`head` is `base_commit`) is never merged. Ancestry alone cannot tell a merged branch from one that only caught up
/// with its base, so callers ask this only once [`has_own_commits`] has been true for the branch.
pub fn is_merged(repo: &Path, head: &str, base_commit: &str, bases: &[&str]) -> bool {
    if head == base_commit {
        return false;
    }
    bases.iter().filter_map(|b| rev_parse(repo, b)).any(|base| {
        if base == base_commit && !is_ancestor(repo, head, &base) {
            // The base has not moved since the branch started, so nothing can have been squashed into it.
            return false;
        }
        is_ancestor(repo, head, &base) || patch_in(repo, head, &base)
    })
}

/// Where a new branch off `base` should start: `origin/<base>` when it is ahead of the local `base` (or there is no
/// local one), else `base`. A local base that has commits `origin` lacks is kept.
pub fn start_point(repo: &Path, base: &str) -> String {
    let origin = format!("origin/{base}");
    let Some(remote) = rev_parse(repo, &format!("refs/remotes/{origin}")) else { return base.to_string() };
    match rev_parse(repo, &format!("refs/heads/{base}")) {
        Some(local) if local != remote && is_ancestor(repo, &local, &remote) => origin,
        None => origin,
        Some(_) => base.to_string(),
    }
}

/// `git fetch <remote> <branch>` with no prompts (a credential or host-key question fails it instead), killed after
/// `timeout`. Updates `<remote>/<branch>`.
pub fn fetch(repo: &Path, remote: &str, branch: &str, timeout: Duration) -> Result<(), GitError> {
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .unwrap_or_else(|_| "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string());
    let mut cmd = Command::new("git");
    cmd.env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", ssh)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    cmd.arg("-C").arg(repo).args(["fetch", "--quiet", "--no-tags", "--end-of-options", remote, branch]);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;
    // Read stderr on its own thread so a chatty failure can never fill the pipe and stall the wait below.
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Command(format!("git fetch {remote} {branch} timed out after {timeout:?}")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(GitError::Command(e.to_string())),
        }
    };
    let err = reader.join().unwrap_or_default();
    if status.success() { Ok(()) } else { Err(GitError::Command(err.trim().to_string())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        std::fs::write(dir.join(file), text).unwrap();
    }

    fn commit(dir: &Path, file: &str, text: &str) -> String {
        write(dir, file, text);
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("edit {file}")]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// A repo on `main` with a file, and branch `feature` (checked out) started there. Returns (repo, base commit).
    fn repo_with_branch(tmp: &Path) -> (PathBuf, String) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        let base = commit(&repo, "a.txt", "one\ntwo\nthree\n");
        git(&repo, &["checkout", "-q", "-b", "feature"]);
        (repo, base)
    }

    fn head(repo: &Path, rev: &str) -> String {
        rev_parse(repo, rev).unwrap()
    }

    #[test]
    fn rev_parse_ancestry_and_merge_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        let f = commit(&repo, "b.txt", "b\n");
        assert_eq!(rev_parse(&repo, "main").as_deref(), Some(base.as_str()));
        assert_eq!(rev_parse(&repo, "feature").as_deref(), Some(f.as_str()));
        assert_eq!(rev_parse(&repo, "no-such-branch"), None);
        assert_eq!(rev_parse(&repo, "--help"), None);
        assert!(is_ancestor(&repo, &base, &f));
        assert!(!is_ancestor(&repo, &f, &base));
        assert_eq!(merge_base(&repo, "main", "feature").as_deref(), Some(base.as_str()));
    }

    #[test]
    fn a_regular_merge_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        let f = commit(&repo, "b.txt", "b\n");
        assert!(!is_merged(&repo, &f, &base, &["main"]));
        git(&repo, &["checkout", "-q", "main"]);
        commit(&repo, "c.txt", "c\n");
        git(&repo, &["merge", "-q", "--no-ff", "-m", "merge", "feature"]);
        assert!(is_merged(&repo, &f, &base, &["main"]));
    }

    #[test]
    fn a_fast_forward_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        let f = commit(&repo, "b.txt", "b\n");
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--ff-only", "feature"]);
        assert!(is_merged(&repo, &f, &base, &["main"]));
    }

    #[test]
    fn a_squash_merge_is_detected_without_touching_refs_or_the_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        commit(&repo, "b.txt", "b\n");
        let f = commit(&repo, "a.txt", "one\nTWO\nthree\n");
        git(&repo, &["checkout", "-q", "main"]);
        assert!(!is_merged(&repo, &f, &base, &["main"]), "not merged yet");
        git(&repo, &["merge", "-q", "--squash", "feature"]);
        git(&repo, &["commit", "-q", "-m", "squashed"]);
        let refs = git(&repo, &["for-each-ref"]);
        let status = git(&repo, &["status", "--porcelain"]);
        assert!(is_merged(&repo, &f, &base, &["main"]));
        assert_eq!(git(&repo, &["for-each-ref"]), refs, "no refs made");
        assert_eq!(git(&repo, &["status", "--porcelain"]), status, "working tree untouched");
    }

    #[test]
    fn a_squash_merge_after_main_moved_on_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        let f = commit(&repo, "b.txt", "b\n");
        git(&repo, &["checkout", "-q", "main"]);
        commit(&repo, "c.txt", "c\n");
        git(&repo, &["merge", "-q", "--squash", "feature"]);
        git(&repo, &["commit", "-q", "-m", "squashed"]);
        commit(&repo, "d.txt", "d\n");
        assert!(is_merged(&repo, &f, &base, &["main"]));
    }

    #[test]
    fn an_unmerged_branch_and_a_branch_without_commits_are_not_merged() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        // No commits of its own, even after main moves on.
        assert!(!is_merged(&repo, &base, &base, &["main"]));
        let f = commit(&repo, "b.txt", "b\n");
        git(&repo, &["checkout", "-q", "main"]);
        commit(&repo, "c.txt", "c\n");
        assert!(!is_merged(&repo, &base, &base, &["main"]));
        assert!(!is_merged(&repo, &f, &base, &["main", "origin/main"]));
        // A different change to the same file is not the branch's change.
        commit(&repo, "b.txt", "B\n");
        assert!(!is_merged(&repo, &f, &base, &["main"]));
    }

    #[test]
    fn a_deleted_branch_is_recognised_by_its_last_head() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        commit(&repo, "b.txt", "b\n");
        let last_head = head(&repo, "feature");
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--squash", "feature"]);
        git(&repo, &["commit", "-q", "-m", "squashed"]);
        git(&repo, &["branch", "-q", "-D", "feature"]);
        assert_eq!(rev_parse(&repo, "refs/heads/feature"), None);
        assert!(is_merged(&repo, &last_head, &base, &["main"]));
    }

    #[test]
    fn a_branch_that_only_caught_up_with_main_has_no_own_commits_until_it_commits() {
        // PDT-45: started at an old main, fast-forwarded to a newer one, then a commit squash-merged.
        let tmp = tempfile::tempdir().unwrap();
        let (repo, base) = repo_with_branch(tmp.path());
        let bases = ["main", "origin/main"];
        assert!(!has_own_commits(&repo, &base, &base, &bases));
        git(&repo, &["checkout", "-q", "main"]);
        let newer = commit(&repo, "c.txt", "c\n");
        git(&repo, &["checkout", "-q", "feature"]);
        git(&repo, &["merge", "-q", "--ff-only", "main"]);
        assert_eq!(head(&repo, "feature"), newer);
        assert!(!has_own_commits(&repo, &newer, &base, &bases), "caught up, nothing of its own");
        let f = commit(&repo, "b.txt", "b\n");
        assert!(has_own_commits(&repo, &f, &base, &bases));
        assert!(!is_merged(&repo, &f, &base, &bases));
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--squash", "feature"]);
        git(&repo, &["commit", "-q", "-m", "squashed"]);
        assert!(has_own_commits(&repo, &f, &base, &bases), "a squash leaves the branch's commits out of main");
        assert!(is_merged(&repo, &f, &base, &bases));
        // Fast-forwarded into main: no longer its own, so callers rely on having seen it before.
        git(&repo, &["checkout", "-q", "-b", "other", &base]);
        let o = commit(&repo, "o.txt", "o\n");
        assert!(has_own_commits(&repo, &o, &base, &bases));
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--no-edit", "other"]);
        assert!(!has_own_commits(&repo, &o, &base, &bases));
        assert!(is_merged(&repo, &o, &base, &bases));
    }

    #[test]
    fn a_new_branch_starts_from_origin_when_it_is_ahead() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, _) = repo_with_branch(tmp.path());
        git(&upstream, &["checkout", "-q", "main"]);
        // No remote: the local base.
        assert_eq!(start_point(&upstream, "main"), "main");
        let clone = tmp.path().join("clone");
        git(tmp.path(), &["clone", "-q", &upstream.to_string_lossy(), &clone.to_string_lossy()]);
        assert_eq!(start_point(&clone, "main"), "main", "level with origin");
        commit(&upstream, "n.txt", "n\n");
        git(&clone, &["fetch", "-q", "origin"]);
        assert_eq!(start_point(&clone, "main"), "origin/main", "origin is ahead");
        assert_eq!(start_point(&clone, "feature"), "origin/feature", "no local branch");
        assert_eq!(start_point(&clone, "nope"), "nope");
        commit(&clone, "l.txt", "l\n");
        assert_eq!(start_point(&clone, "main"), "main", "diverged: the local base is kept");
    }

    #[test]
    fn fetch_updates_the_remote_branch_and_reports_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let (upstream, base) = repo_with_branch(tmp.path());
        git(&upstream, &["checkout", "-q", "main"]);
        let clone = tmp.path().join("clone");
        git(tmp.path(), &["clone", "-q", &upstream.to_string_lossy(), &clone.to_string_lossy()]);
        let next = commit(&upstream, "n.txt", "n\n");
        assert_eq!(rev_parse(&clone, "origin/main").as_deref(), Some(base.as_str()));
        fetch(&clone, "origin", "main", Duration::from_secs(20)).unwrap();
        assert_eq!(rev_parse(&clone, "origin/main").as_deref(), Some(next.as_str()));
        assert!(fetch(&clone, "nope", "main", Duration::from_secs(20)).is_err());
        assert!(fetch(&clone, "origin", "no-such-branch", Duration::from_secs(20)).is_err());
    }
}
