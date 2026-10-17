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
