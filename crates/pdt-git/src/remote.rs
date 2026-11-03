//! Talking to a remote (PDT-245): fetch and push with no prompts, killed after a timeout. Credentials are the user's
//! own (ssh keys, a credential helper); a question for them fails the command instead of waiting.

use crate::GitError;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a fetch may take when a failure just means using the local refs and someone waits on it (Start).
pub const QUICK_FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a fetch or push may take while someone waits on it (a pull, Stop deleting the remote branch).
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a fetch or push may take in background work (a workflow's conflict step, the local forge).
pub const BACKGROUND_REMOTE_TIMEOUT: Duration = Duration::from_secs(120);

/// How a command against a remote ended.
pub(crate) struct RemoteOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `git -C repo args…` with no prompts (`GIT_TERMINAL_PROMPT=0`, ssh in batch mode), killed after `timeout`.
pub(crate) fn run_remote(repo: &Path, args: &[&str], timeout: Duration) -> Result<RemoteOutput, GitError> {
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .unwrap_or_else(|_| "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string());
    let mut cmd = Command::new("git");
    cmd.env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_SSH_COMMAND", ssh)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    cmd.arg("-C").arg(repo).args(args);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;
    // Read each pipe on its own thread so a chatty command can never fill one and stall the wait below.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_string(&mut s);
            }
            s
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let what = args.iter().take_while(|a| **a != "--end-of-options").copied().collect::<Vec<_>>().join(" ");
                return Err(GitError::Command(format!("git {what} timed out after {timeout:?}")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(GitError::Command(e.to_string())),
        }
    };
    Ok(RemoteOutput {
        ok: status.success(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// What [`push`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    /// The remote has commits the push would lose (it moved on): fetch, catch up and push again. The text is git's.
    Rejected(String),
}

/// `git push <remote> <refspecs…>` with no prompts, killed after `timeout`. A push the remote refuses as not a
/// fast-forward is [`PushOutcome::Rejected`]; any other failure (no such remote, no access, a hook said no) is an Err.
pub fn push(repo: &Path, remote: &str, refspecs: &[&str], timeout: Duration) -> Result<PushOutcome, GitError> {
    let mut args = vec!["push", "--porcelain", "--end-of-options", remote];
    args.extend(refspecs);
    let out = run_remote(repo, &args, timeout)?;
    if out.ok {
        return Ok(PushOutcome::Pushed);
    }
    // `--porcelain`: a ref the remote refused is `!\t<from>:<to>\t[rejected] (fetch first)` (or `(non-fast-forward)`);
    // `[remote rejected]` is a hook's no, which fetching can't fix.
    let rejected: Vec<&str> = out.stdout.lines().filter(|l| l.starts_with('!') && l.contains("\t[rejected]")).collect();
    if !rejected.is_empty() {
        return Ok(PushOutcome::Rejected(rejected.join("\n")));
    }
    let why = [out.stderr.trim(), out.stdout.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("git push failed");
    Err(GitError::Command(why.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str) -> String {
        std::fs::write(dir.join(file), file).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", file]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// A bare remote and two clones of it on `main`. Returns (remote, mine, theirs).
    fn setup(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let remote = tmp.join("remote.git");
        git(tmp, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let mine = tmp.join("mine");
        git(tmp, &["init", "-q", "-b", "main", mine.to_str().unwrap()]);
        commit(&mine, "a.txt");
        git(&mine, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&mine, &["push", "-q", "origin", "main"]);
        let theirs = tmp.join("theirs");
        git(tmp, &["clone", "-q", remote.to_str().unwrap(), theirs.to_str().unwrap()]);
        (remote, mine, theirs)
    }

    #[test]
    fn pushes_and_tells_a_rejected_push_from_a_failed_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (remote, mine, theirs) = setup(tmp.path());
        let t = REMOTE_TIMEOUT;
        let b = commit(&mine, "b.txt");
        assert_eq!(push(&mine, "origin", &["refs/heads/main:refs/heads/main"], t).unwrap(), PushOutcome::Pushed);
        assert_eq!(git(&remote, &["rev-parse", "main"]), b);

        // The remote moves on: the next push is refused, and a fetch catches up.
        git(&theirs, &["pull", "-q", "--ff-only"]);
        let c = commit(&theirs, "c.txt");
        git(&theirs, &["push", "-q", "origin", "main"]);
        commit(&mine, "e.txt");
        match push(&mine, "origin", &["refs/heads/main:refs/heads/main"], t).unwrap() {
            PushOutcome::Rejected(why) => assert!(why.contains("refs/heads/main"), "{why}"),
            other => panic!("{other:?}"),
        }
        crate::fetch(&mine, "origin", "main", t).unwrap();
        assert_eq!(crate::rev_parse(&mine, "refs/remotes/origin/main").as_deref(), Some(c.as_str()));

        assert!(push(&mine, "nowhere", &["refs/heads/main:refs/heads/main"], t).is_err(), "no such remote");
        assert!(push(&mine, "--upload-pack=x", &["main"], t).is_err(), "a remote is never an option");
    }
}
