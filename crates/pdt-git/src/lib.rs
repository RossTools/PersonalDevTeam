//! Thin wrapper over the `git` CLI. Blocking; call from `spawn_blocking` in async code.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub current_branch: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("path does not exist or is not a directory: {0}")]
    PathNotFound(PathBuf),
    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),
    #[error("git is not installed or not on PATH")]
    GitMissing,
    #[error("git command failed: {0}")]
    Command(String),
    #[error("invalid branch name: {0}")]
    InvalidBranch(String),
    #[error("a branch named {0} already exists")]
    BranchExists(String),
    #[error("unknown base branch or commit: {0}")]
    UnknownRef(String),
}

/// A git invocation that ran but exited non-zero.
struct Failed {
    code: Option<i32>,
    stderr: String,
}

/// Runs `git -C dir args…`. Outer Err means git could not be run at all;
/// inner Err means git ran and exited non-zero (with its exit code and stderr).
/// `inherited_env` is set on the command first, standing in for variables inherited from the parent process;
/// the sanitising below must neutralise them. Production callers pass none.
fn git_with_env(dir: &Path, args: &[&str], inherited_env: &[(&str, &str)]) -> Result<Result<String, Failed>, GitError> {
    let mut cmd = Command::new("git");
    cmd.envs(inherited_env.iter().copied());
    // Deterministic messages, and no repository redirection from the caller's environment.
    cmd.env("LC_ALL", "C").env("LANGUAGE", "C");
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    let output = cmd.arg("-C").arg(dir).args(args).output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(Ok(stdout.trim_end_matches(['\r', '\n']).to_string()))
    } else {
        let code = output.status.code();
        let mut stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            stderr = match code {
                Some(c) => format!("git exited with code {c}"),
                None => "git was terminated by a signal".to_string(),
            };
        }
        Ok(Err(Failed { code, stderr }))
    }
}

fn native_separators(s: String) -> String {
    if cfg!(windows) { s.replace('/', "\\") } else { s }
}

/// Resolves any path inside a working tree to the repo root and its current branch.
pub fn resolve_repo(path: &Path) -> Result<RepoInfo, GitError> {
    resolve_repo_with_env(path, &[])
}

fn resolve_repo_with_env(path: &Path, inherited_env: &[(&str, &str)]) -> Result<RepoInfo, GitError> {
    if !path.is_dir() {
        return Err(GitError::PathNotFound(path.to_path_buf()));
    }
    let root = match git_with_env(path, &["rev-parse", "--show-toplevel"], inherited_env)? {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return Err(GitError::NotARepo(path.to_path_buf())),
        Err(f) => return Err(classify_failure(path, f)),
    };
    let root = PathBuf::from(native_separators(root));
    // Exit code 1 from `symbolic-ref --quiet` means HEAD is detached, not an error.
    let current_branch = match git_with_env(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"], inherited_env)? {
        Ok(s) if !s.is_empty() => Some(s),
        Ok(_) => None,
        Err(f) if f.code == Some(1) => None,
        Err(f) => return Err(GitError::Command(f.stderr)),
    };
    Ok(RepoInfo { root, current_branch })
}

fn classify_failure(path: &Path, f: Failed) -> GitError {
    let lower = f.stderr.to_lowercase();
    if lower.contains("not a git repository") || lower.contains("must be run in a work tree") {
        GitError::NotARepo(path.to_path_buf())
    } else if lower.contains("dubious ownership") {
        GitError::Command(format!("git refused this repository (safe.directory): {}", f.stderr))
    } else {
        GitError::Command(f.stderr)
    }
}

/// Checks a branch name with `git check-ref-format --branch`.
pub fn check_branch_name(repo: &Path, branch: &str) -> Result<(), GitError> {
    match git_with_env(repo, &["check-ref-format", "--branch", branch], &[])? {
        Ok(_) => Ok(()),
        Err(_) => Err(GitError::InvalidBranch(branch.to_string())),
    }
}

/// Creates `path` as a new worktree on new branch `branch`, starting at `base`. Returns the base commit SHA.
pub fn add_worktree(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<String, GitError> {
    check_branch_name(repo, branch)?;
    let spec = format!("{base}^{{commit}}");
    let sha = match git_with_env(repo, &["rev-parse", "--verify", "--quiet", &spec], &[])? {
        Ok(s) if !s.is_empty() => s,
        _ => return Err(GitError::UnknownRef(base.to_string())),
    };
    if path.exists() {
        return Err(GitError::Command(format!("worktree path already exists: {}", path.display())));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| GitError::Command(format!("creating {}: {e}", parent.display())))?;
    }
    let path_str = path.to_string_lossy();
    match git_with_env(repo, &["worktree", "add", "-b", branch, &path_str, &sha], &[])? {
        Ok(_) => Ok(sha),
        Err(f) if f.stderr.contains("already exists") && f.stderr.contains("branch") => {
            Err(GitError::BranchExists(branch.to_string()))
        }
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}

/// Adds `pattern` as a line to the repository's `info/exclude` (shared by all its worktrees) unless it is
/// already there. This is a local ignore file that is never committed.
pub fn ensure_excluded(repo: &Path, pattern: &str) -> Result<(), GitError> {
    let common = match git_with_env(repo, &["rev-parse", "--git-common-dir"], &[])? {
        Ok(s) => PathBuf::from(native_separators(s)),
        Err(f) => return Err(classify_failure(repo, f)),
    };
    let common = if common.is_absolute() { common } else { repo.join(common) };
    let file = common.join("info").join("exclude");
    let io = |e: std::io::Error| GitError::Command(format!("updating {}: {e}", file.display()));
    let mut text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    if text.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(&file, text).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(dir: &Path) {
        let ok = Command::new("git").args(["init", "-q", "-b", "main"]).arg(dir).status().unwrap().success();
        assert!(ok, "git init failed");
    }

    fn same_dir(a: &Path, b: &Path) -> bool {
        std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
    }

    #[test]
    fn resolves_root_from_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let sub = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        let info = resolve_repo(&sub).unwrap();
        assert!(same_dir(&info.root, tmp.path()));
        assert_eq!(info.current_branch.as_deref(), Some("main"));
    }

    #[test]
    fn resolves_path_with_spaces_and_unicode() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("my repo é");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let info = resolve_repo(&repo).unwrap();
        assert!(same_dir(&info.root, &repo));
        assert!(info.root.to_string_lossy().ends_with("my repo é"));
        if cfg!(windows) {
            assert!(!info.root.to_string_lossy().contains('/'), "expected native separators");
        }
    }

    #[test]
    fn detached_head_has_no_branch() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let git_in = |args: &[&str]| {
            let ok = Command::new("git")
                .current_dir(tmp.path())
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git_in(&["commit", "--allow-empty", "-m", "init", "-q"]);
        git_in(&["checkout", "-q", "--detach"]);
        let info = resolve_repo(tmp.path()).unwrap();
        assert_eq!(info.current_branch, None);
    }

    #[test]
    fn plain_directory_is_not_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(resolve_repo(tmp.path()), Err(GitError::NotARepo(_))));
    }

    #[test]
    fn ignores_inherited_git_dir_env() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let bogus = tmp.path().join("no-such-git-dir");
        let bogus = bogus.to_string_lossy();
        let info = resolve_repo_with_env(
            tmp.path(),
            &[("GIT_DIR", &bogus), ("GIT_WORK_TREE", &bogus), ("GIT_INDEX_FILE", &bogus), ("GIT_COMMON_DIR", &bogus)],
        )
        .unwrap();
        assert!(same_dir(&info.root, tmp.path()));
        assert_eq!(info.current_branch.as_deref(), Some("main"));
    }

    #[test]
    fn bare_repo_is_not_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = Command::new("git").args(["init", "-q", "--bare"]).arg(tmp.path()).status().unwrap().success();
        assert!(ok);
        assert!(matches!(resolve_repo(tmp.path()), Err(GitError::NotARepo(_))));
    }

    #[test]
    fn dubious_ownership_message_mentions_safe_directory() {
        let f = Failed { code: Some(128), stderr: "fatal: detected dubious ownership in repository at 'x'".into() };
        let msg = classify_failure(Path::new("x"), f).to_string();
        assert!(msg.contains("git refused this repository (safe.directory): fatal: detected dubious"), "{msg}");
    }

    #[test]
    fn missing_path_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert!(matches!(resolve_repo(&missing), Err(GitError::PathNotFound(_))));
    }

    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repo on `main` with one commit; returns the HEAD sha.
    fn repo_with_commit(dir: &Path) -> String {
        std::fs::create_dir_all(dir).unwrap();
        init_repo(dir);
        git_ok(dir, &["commit", "--allow-empty", "-m", "init", "-q"]);
        git_ok(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn add_worktree_creates_branch_at_base() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let head = repo_with_commit(&repo);
        let wt = repo.join(".worktrees").join("feature-x");
        let sha = add_worktree(&repo, &wt, "feature/x", "main").unwrap();
        assert_eq!(sha, head);
        let info = resolve_repo(&wt).unwrap();
        assert_eq!(info.current_branch.as_deref(), Some("feature/x"));
        assert!(same_dir(&info.root, &wt));
    }

    #[test]
    fn add_worktree_errors_are_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        let wt = |n: &str| tmp.path().join("wts").join(n);
        assert!(matches!(add_worktree(&repo, &wt("a"), "main", "main"), Err(GitError::BranchExists(b)) if b == "main"));
        assert!(
            matches!(add_worktree(&repo, &wt("b"), "b", "no-such-ref"), Err(GitError::UnknownRef(r)) if r == "no-such-ref")
        );
        assert!(matches!(add_worktree(&repo, &wt("c"), "bad..name", "main"), Err(GitError::InvalidBranch(_))));
    }

    #[test]
    fn worktrees_are_excluded_from_the_main_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        add_worktree(&repo, &repo.join(".worktrees").join("a"), "a", "main").unwrap();
        add_worktree(&repo, &repo.join(".worktrees").join("b"), "b", "main").unwrap();
        assert_eq!(git_ok(&repo, &["status", "--porcelain"]), "", "main checkout stays clean");
        let exclude = std::fs::read_to_string(repo.join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(exclude.lines().filter(|l| l.trim() == "/.worktrees/").count(), 1);
    }

    #[test]
    fn ensure_excluded_keeps_existing_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        let file = repo.join(".git").join("info").join("exclude");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "*.log").unwrap(); // no trailing newline
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "*.log\n/.worktrees/\n");
    }
}
