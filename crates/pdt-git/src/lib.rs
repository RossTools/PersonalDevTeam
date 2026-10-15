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
}
