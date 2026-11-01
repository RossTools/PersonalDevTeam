//! Cloning, worktree management and reading a checkout's files (workspaces spec §6). Blocking, like the rest.

use crate::{GitError, git_with_env, native_separators};
use std::path::{Path, PathBuf};

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    match git_with_env(dir, args, &[])? {
        Ok(s) => Ok(s),
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}

/// Clones `url` into `dest`, which must not exist yet. Git never prompts (a credential or host-key question fails
/// the clone instead of hanging); the host's own credential helper and SSH agent still work. A failed clone leaves
/// no folder behind.
pub fn clone(url: &str, dest: &Path) -> Result<(), GitError> {
    if dest.exists() {
        return Err(GitError::Command(format!("{} already exists", dest.display())));
    }
    let parent = dest.parent().ok_or_else(|| GitError::Command(format!("no parent folder for {}", dest.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| GitError::Command(format!("creating {}: {e}", parent.display())))?;
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .unwrap_or_else(|_| "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string());
    let dest_str = dest.to_string_lossy();
    let env = [("GIT_TERMINAL_PROMPT", "0"), ("GIT_SSH_COMMAND", ssh.as_str())];
    match git_with_env(parent, &["clone", "--quiet", "--", url, &dest_str], &env)? {
        Ok(_) => Ok(()),
        Err(f) => {
            let _ = std::fs::remove_dir_all(dest);
            Err(GitError::Command(f.stderr))
        }
    }
}

/// The URL of remote `name`, if it has one.
pub fn remote_url(repo: &Path, name: &str) -> Option<String> {
    match git_with_env(repo, &["remote", "get-url", name], &[]) {
        Ok(Ok(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// None when detached.
    pub branch: Option<String>,
    pub head: String,
}

/// Every worktree git knows for the repository (the main checkout first).
pub fn list_worktrees(repo: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
    let out = run(repo, &["worktree", "list", "--porcelain"])?;
    let mut list = Vec::new();
    let mut cur: Option<WorktreeEntry> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            list.extend(cur.take());
            cur = Some(WorktreeEntry {
                path: PathBuf::from(native_separators(p.to_string())),
                branch: None,
                head: String::new(),
            });
        } else if let Some(h) = line.strip_prefix("HEAD ")
            && let Some(c) = cur.as_mut()
        {
            c.head = h.to_string();
        } else if let Some(b) = line.strip_prefix("branch ")
            && let Some(c) = cur.as_mut()
        {
            c.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        }
    }
    list.extend(cur);
    Ok(list)
}

/// True when the checkout has uncommitted changes, untracked files included.
pub fn is_dirty(dir: &Path) -> Result<bool, GitError> {
    Ok(!run(dir, &["status", "--porcelain"])?.is_empty())
}

/// What a checkout holds that its remote doesn't (PDT-161): tracked files changed but not committed, and commits on
/// HEAD its upstream lacks. Untracked files don't count.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unpushed {
    /// Tracked files with uncommitted changes (staged or not), `/`-separated.
    pub changed: Vec<String>,
    /// What HEAD is compared with: its upstream, else `origin/<branch>` when that exists; None when it has neither.
    pub upstream: Option<String>,
    /// Commits on HEAD that `upstream` doesn't have.
    pub ahead: u32,
    /// The repository has an `origin` remote while `upstream` is None: the branch was never pushed.
    pub unpushed_branch: bool,
}

impl Unpushed {
    pub fn is_clean(&self) -> bool {
        self.changed.is_empty() && self.ahead == 0 && !self.unpushed_branch
    }
}

/// [`Unpushed`] for the checkout at `dir`.
pub fn unpushed(dir: &Path) -> Result<Unpushed, GitError> {
    let out = run(dir, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=no"])?;
    let mut changed = Vec::new();
    let mut parts = out.split('\0');
    while let Some(e) = parts.next() {
        if e.len() < 4 {
            continue;
        }
        let (code, path) = e.split_at(3);
        if code.starts_with('R') || code.starts_with('C') {
            parts.next(); // the original name of a rename or copy
        }
        changed.push(path.to_string());
    }
    let tracking = |args: &[&str]| match git_with_env(dir, args, &[]) {
        Ok(Ok(s)) if !s.is_empty() => Some(s),
        _ => None,
    };
    let upstream = tracking(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).or_else(|| {
        let branch = tracking(&["symbolic-ref", "--short", "-q", "HEAD"])?;
        let remote = format!("origin/{branch}");
        crate::rev_parse(dir, &format!("refs/remotes/{remote}")).map(|_| remote)
    });
    let ahead = match &upstream {
        Some(u) => run(dir, &["rev-list", "--count", &format!("{u}..HEAD")])?.trim().parse().unwrap_or(0),
        None => 0,
    };
    let unpushed_branch = upstream.is_none() && remote_url(dir, "origin").is_some();
    Ok(Unpushed { changed, upstream, ahead, unpushed_branch })
}

/// Whether `rel` (`/`-separated, below the checkout at `dir`) is safe to delete as build output: git ignores it and
/// tracks nothing in it, so neither committed files nor work waiting to be committed can be lost.
pub fn is_disposable(dir: &Path, rel: &str) -> Result<bool, GitError> {
    if !run(dir, &["ls-files", "-z", "--", rel])?.is_empty() {
        return Ok(false);
    }
    match git_with_env(dir, &["check-ignore", "-q", "--", rel], &[])? {
        Ok(_) => Ok(true),
        Err(f) if f.code == Some(1) => Ok(false),
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}

/// `git worktree remove [--force] <path>`.
pub fn remove_worktree(repo: &Path, path: &Path, force: bool) -> Result<(), GitError> {
    let p = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&p);
    run(repo, &args).map(|_| ())
}

/// Forgets worktrees whose folders are gone.
pub fn prune_worktrees(repo: &Path) -> Result<(), GitError> {
    run(repo, &["worktree", "prune"]).map(|_| ())
}

/// `git branch -D <branch>`.
pub fn delete_branch(repo: &Path, branch: &str) -> Result<(), GitError> {
    run(repo, &["branch", "-D", "--", branch]).map(|_| ())
}

fn pathspec(dir: &str) -> String {
    let d = dir.trim_matches('/');
    if d.is_empty() { ".".to_string() } else { format!("{d}/") }
}

/// Every file under `dir` (relative to the checkout, `/`-separated, "" for the root) that is tracked or untracked
/// but not ignored, as paths relative to the checkout with `/` separators.
pub fn list_files(checkout: &Path, dir: &str) -> Result<Vec<String>, GitError> {
    let spec = pathspec(dir);
    let out = run(
        checkout,
        &["-c", "core.quotepath=off", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", &spec],
    )?;
    let mut files: Vec<String> = out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Git status codes (`M`, `A`, `D`, `R`, `??`, ...) of changed files under `dir`, by `/`-separated path.
pub fn status(checkout: &Path, dir: &str) -> Result<Vec<(String, String)>, GitError> {
    let spec = pathspec(dir);
    let out = run(checkout, &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", &spec])?;
    let mut entries = Vec::new();
    let mut parts = out.split('\0');
    while let Some(e) = parts.next() {
        if e.len() < 4 {
            continue;
        }
        let (code, path) = e.split_at(3);
        let code = code.trim().to_string();
        if code.starts_with('R') || code.starts_with('C') {
            parts.next(); // the original name of a rename or copy
        }
        entries.push((path.to_string(), code));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "init"]);
    }

    /// Reading a checkout never rewrites its index: `git status` refreshes a stale index and writes it back under
    /// `index.lock`, which fails a git command the agent (or a test) runs in the worktree at that moment (PDT-335).
    #[test]
    fn reading_a_checkout_leaves_its_index_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("work");
        repo(&dir);
        // Same content, new timestamp: the index's stat data is stale, so a plain `git status` would refresh it.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(dir.join("README.md")).unwrap().set_modified(later).unwrap();
        let index = dir.join(".git/index");
        let before = std::fs::read(&index).unwrap();
        assert!(!is_dirty(&dir).unwrap());
        assert!(unpushed(&dir).unwrap().changed.is_empty());
        assert!(status(&dir, "").unwrap().is_empty());
        assert_eq!(std::fs::read(&index).unwrap(), before, "the index was rewritten");
    }

    #[test]
    fn only_ignored_untracked_paths_are_disposable() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("work");
        repo(&dir);
        std::fs::write(dir.join(".gitignore"), "/target\nnode_modules/\n").unwrap();
        for d in ["target/debug", "web/node_modules/x", "dist", "kept"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("kept/a.txt"), "a").unwrap();
        std::fs::write(dir.join("dist/out.js"), "x").unwrap();
        git(&dir, &["add", ".gitignore", "kept"]);
        git(&dir, &["commit", "-q", "-m", "more"]);
        assert!(is_disposable(&dir, "target").unwrap());
        assert!(is_disposable(&dir, "web/node_modules").unwrap());
        assert!(!is_disposable(&dir, "dist").unwrap(), "not ignored: untracked work");
        assert!(!is_disposable(&dir, "kept").unwrap(), "tracked");
        // Ignored, but a file in it was committed anyway.
        std::fs::write(dir.join("target/forced.txt"), "f").unwrap();
        git(&dir, &["add", "-f", "target/forced.txt"]);
        assert!(!is_disposable(&dir, "target").unwrap());
    }

    #[test]
    fn unpushed_counts_tracked_changes_and_commits_the_upstream_lacks() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("remote.git");
        let dir = tmp.path().join("work");
        repo(&dir);
        // No upstream: only uncommitted changes count.
        assert_eq!(unpushed(&dir).unwrap(), Unpushed::default());
        git(tmp.path(), &["init", "-q", "--bare", &remote.to_string_lossy()]);
        git(&dir, &["remote", "add", "origin", &remote.to_string_lossy()]);
        // With origin, a branch that isn't on it is unpushed.
        assert!(unpushed(&dir).unwrap().unpushed_branch);
        git(&dir, &["push", "-q", "-u", "origin", "main"]);
        let up = unpushed(&dir).unwrap();
        assert!(up.is_clean(), "{up:?}");
        assert_eq!(up.upstream.as_deref(), Some("origin/main"));
        assert!(!up.unpushed_branch);
        // An untracked file doesn't count; a changed tracked one does.
        std::fs::write(dir.join("scratch.txt"), "x\n").unwrap();
        assert!(unpushed(&dir).unwrap().is_clean());
        std::fs::write(dir.join("README.md"), "changed\n").unwrap();
        assert_eq!(unpushed(&dir).unwrap().changed, vec!["README.md".to_string()]);
        git(&dir, &["commit", "-q", "-am", "change"]);
        let up = unpushed(&dir).unwrap();
        assert_eq!((up.changed.len(), up.ahead), (0, 1), "{up:?}");
        git(&dir, &["push", "-q"]);
        assert!(unpushed(&dir).unwrap().is_clean());
        // Pushed without -u: origin/<branch> stands in for the upstream.
        git(&dir, &["checkout", "-q", "-b", "topic"]);
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        git(&dir, &["add", "b.txt"]);
        git(&dir, &["commit", "-q", "-m", "b"]);
        assert!(unpushed(&dir).unwrap().unpushed_branch);
        git(&dir, &["push", "-q", "origin", "topic"]);
        let up = unpushed(&dir).unwrap();
        assert_eq!((up.upstream.as_deref(), up.ahead), (Some("origin/topic"), 0), "{up:?}");
        git(&dir, &["commit", "-q", "--allow-empty", "-m", "c"]);
        assert_eq!(unpushed(&dir).unwrap().ahead, 1);
    }

    fn same(a: &Path, b: &Path) -> bool {
        std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
    }

    #[test]
    fn clones_a_local_repository_and_cleans_up_after_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src repo");
        repo(&src);
        let dest = tmp.path().join("out").join("copy");
        clone(&src.to_string_lossy(), &dest).unwrap();
        assert!(dest.join("README.md").is_file());
        assert!(remote_url(&dest, "origin").is_some());
        assert_eq!(remote_url(&dest, "nope"), None);
        assert!(clone(&src.to_string_lossy(), &dest).is_err(), "an existing folder is refused");

        let bad = tmp.path().join("out").join("bad");
        let missing = tmp.path().join("no such repo");
        assert!(clone(&missing.to_string_lossy(), &bad).is_err());
        assert!(!bad.exists(), "a failed clone leaves no folder");
    }

    #[test]
    fn lists_checks_and_removes_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("r");
        repo(&r);
        let wt = r.join(".worktrees").join("feat");
        git(&r, &["worktree", "add", "-q", "-b", "feat", &wt.to_string_lossy()]);
        let list = list_worktrees(&r).unwrap();
        assert_eq!(list.len(), 2);
        assert!(same(&list[0].path, &r));
        assert_eq!(list[1].branch.as_deref(), Some("feat"));
        assert!(same(&list[1].path, &wt));
        assert_eq!(list[1].head.len(), 40);

        assert!(!is_dirty(&wt).unwrap());
        std::fs::write(wt.join("new.txt"), "x").unwrap();
        assert!(is_dirty(&wt).unwrap());
        assert!(remove_worktree(&r, &wt, false).is_err(), "git refuses a dirty worktree without --force");
        remove_worktree(&r, &wt, true).unwrap();
        assert!(!wt.exists());
        delete_branch(&r, "feat").unwrap();
        assert!(!crate::branch_exists(&r, "feat"));
        prune_worktrees(&r).unwrap();
    }

    #[test]
    fn lists_files_honouring_gitignore_with_status() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path().join("r");
        repo(&r);
        std::fs::create_dir_all(r.join("src").join("deep")).unwrap();
        std::fs::create_dir_all(r.join("target")).unwrap();
        std::fs::write(r.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(r.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(r.join("src").join("deep").join("é.rs"), "\n").unwrap();
        std::fs::write(r.join("target").join("out.bin"), "x").unwrap();
        std::fs::write(r.join("debug.log"), "x").unwrap();
        git(&r, &["add", ".gitignore", "src/main.rs"]);
        git(&r, &["commit", "-q", "-m", "more"]);
        std::fs::write(r.join("README.md"), "changed\n").unwrap();

        let all = list_files(&r, "").unwrap();
        assert_eq!(all, [".gitignore", "README.md", "src/deep/é.rs", "src/main.rs"]);
        assert_eq!(list_files(&r, "src/").unwrap(), ["src/deep/é.rs", "src/main.rs"]);
        let st = status(&r, "").unwrap();
        assert!(st.contains(&("README.md".into(), "M".into())), "{st:?}");
        assert!(st.contains(&("src/deep/é.rs".into(), "??".into())), "{st:?}");
        assert!(!st.iter().any(|(p, _)| p.ends_with(".log")));
    }
}
