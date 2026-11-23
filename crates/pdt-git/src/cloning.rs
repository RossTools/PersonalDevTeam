//! Cloning a repository (PDT-712): no prompts, git's progress as it goes, killed when it stalls or is cancelled.

use crate::{GitError, PDT_GIT_CONFIG, command_failed};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a clone may go without a word of progress before it counts as stalled and is killed. git reports
/// progress every second or so while it transfers, so only a host that stopped answering goes this long.
pub const CLONE_STALL_TIMEOUT: Duration = Duration::from_secs(300);

/// How a clone is watched: when it counts as stalled, the flag that cancels it, and who hears its progress.
pub struct CloneWatch<'a> {
    pub stall: Duration,
    pub cancel: &'a AtomicBool,
    /// Each line git reports (`Receiving objects:  45% (450/1000)`), as it comes.
    pub progress: &'a mut dyn FnMut(&str),
}

/// Clones `url` into `dest`, which must not exist yet, killed after [`CLONE_STALL_TIMEOUT`] without progress. Git
/// never prompts (a credential or host-key question fails the clone instead of hanging); the host's own credential
/// helper and SSH agent still work. A failed clone leaves no folder behind.
pub fn clone(url: &str, dest: &Path) -> Result<(), GitError> {
    let cancel = AtomicBool::new(false);
    clone_watched(url, dest, CloneWatch { stall: CLONE_STALL_TIMEOUT, cancel: &cancel, progress: &mut |_| {} })
}

/// [`clone`], reporting git's progress, killed when it goes `watch.stall` without any or `watch.cancel` is set
/// ([`GitError::Cancelled`]). A clone that fails, stalls or is cancelled leaves no folder behind.
pub fn clone_watched(url: &str, dest: &Path, watch: CloneWatch<'_>) -> Result<(), GitError> {
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .unwrap_or_else(|_| "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string());
    clone_with_ssh(url, dest, &ssh, watch)
}

fn clone_with_ssh(url: &str, dest: &Path, ssh: &str, watch: CloneWatch<'_>) -> Result<(), GitError> {
    if dest.exists() {
        return Err(GitError::Command(format!("{} already exists", dest.display())));
    }
    let parent = dest.parent().ok_or_else(|| GitError::Command(format!("no parent folder for {}", dest.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| GitError::Command(format!("creating {}: {e}", parent.display())))?;
    let result = run_clone(url, dest, parent, ssh, watch);
    if result.is_err() {
        remove_soon(dest);
    }
    result
}

/// Removes what a failed clone left. A killed git's transport helper (ssh, git-remote-https) can still hold a file
/// open for a moment, which on Windows stops the removal: try again for a few seconds.
fn remove_soon(dest: &Path) {
    for _ in 0..20 {
        if std::fs::remove_dir_all(dest).is_ok() || !dest.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn run_clone(url: &str, dest: &Path, parent: &Path, ssh: &str, watch: CloneWatch<'_>) -> Result<(), GitError> {
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
    cmd.args(PDT_GIT_CONFIG).arg("-C").arg(parent).args(["clone", "--progress", "--", url]).arg(dest);
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { GitError::GitMissing } else { GitError::Command(e.to_string()) }
    })?;
    // git rewrites a progress line in place with `\r`: each piece between `\r`s and `\n`s is one report. Read on a
    // thread of its own, which is left behind when git is killed (a hung ssh may hold the pipe open).
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(mut pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let (mut buf, mut line) = ([0u8; 4096], Vec::new());
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for &b in &buf[..n] {
                    if b == b'\r' || b == b'\n' {
                        if !line.is_empty() && tx.send(String::from_utf8_lossy(&line).trim().to_string()).is_err() {
                            return;
                        }
                        line.clear();
                    } else {
                        line.push(b);
                    }
                }
            }
            if !line.is_empty() {
                let _ = tx.send(String::from_utf8_lossy(&line).trim().to_string());
            }
        });
    }
    let mut said: Vec<String> = Vec::new();
    let mut heard = |line: String, said: &mut Vec<String>| {
        if line.is_empty() {
            return;
        }
        (watch.progress)(&line);
        // Progress lines are noise in an error; git's own words (`fatal: …`) are kept.
        if !is_progress(&line) {
            said.push(line);
        }
    };
    let mut last_heard = Instant::now();
    let kill = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    let status = loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                last_heard = Instant::now();
                heard(line, &mut said);
            }
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if watch.cancel.load(Ordering::Relaxed) {
            kill(&mut child);
            return Err(GitError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if last_heard.elapsed() >= watch.stall => {
                kill(&mut child);
                return Err(GitError::Command(format!("git clone stalled: no progress for {}", in_words(watch.stall))));
            }
            Ok(None) => {}
            Err(e) => {
                kill(&mut child);
                return Err(GitError::Command(e.to_string()));
            }
        }
    };
    // What git said last, up to the moment it exited (the reader may still hold a line or two).
    let until = Instant::now() + Duration::from_millis(500);
    while let Ok(line) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        heard(line, &mut said);
    }
    if status.success() {
        return Ok(());
    }
    let mut why = said.join("\n");
    if why.is_empty() {
        why = match status.code() {
            Some(c) => format!("git exited with code {c}"),
            None => "git was terminated by a signal".to_string(),
        };
    }
    Err(command_failed(why))
}

/// Whether `line` is one of git's progress reports (`Receiving objects:  45% (450/1000)`, `…, done.`) rather than
/// something it has to say.
fn is_progress(line: &str) -> bool {
    line.starts_with("Cloning into") || line.contains('%') || line.ends_with("done.")
}

/// `300s` as `5 minutes`, `2s` as `2 seconds`.
fn in_words(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        60 => "a minute".to_string(),
        s if s >= 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        1 => "a second".to_string(),
        s if s >= 1 => format!("{s} seconds"),
        _ => format!("{}ms", d.as_millis()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_url;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo(tmp: &Path) -> PathBuf {
        let src = tmp.join("src repo");
        std::fs::create_dir_all(&src).unwrap();
        git(&src, &["init", "-q", "-b", "main"]);
        std::fs::write(src.join("README.md"), "hi\n").unwrap();
        git(&src, &["add", "."]);
        git(&src, &["commit", "-q", "-m", "init"]);
        src
    }

    #[test]
    fn clones_a_local_repository_and_cleans_up_after_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let src = repo(tmp.path());
        let dest = tmp.path().join("out").join("copy");
        clone(&src.to_string_lossy(), &dest).unwrap();
        assert!(dest.join("README.md").is_file());
        assert!(remote_url(&dest, "origin").is_some());
        assert_eq!(remote_url(&dest, "nope"), None);
        assert!(clone(&src.to_string_lossy(), &dest).is_err(), "an existing folder is refused");

        let bad = tmp.path().join("out").join("bad");
        let missing = tmp.path().join("no such repo");
        let Err(GitError::Command(why)) = clone(&missing.to_string_lossy(), &bad) else { panic!("cloned nothing") };
        assert!(why.contains("does not exist") || why.contains("fatal"), "git's own words: {why}");
        assert!(!bad.exists(), "a failed clone leaves no folder");
    }

    #[test]
    fn reports_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let src = repo(tmp.path());
        let cancel = AtomicBool::new(false);
        let mut lines = Vec::new();
        let mut progress = |l: &str| lines.push(l.to_string());
        // `file://` makes git transfer (and report) objects rather than copy the folder.
        let url = format!("file://{}", src.to_string_lossy().replace('\\', "/"));
        let watch = CloneWatch { stall: CLONE_STALL_TIMEOUT, cancel: &cancel, progress: &mut progress };
        clone_watched(&url, &tmp.path().join("copy"), watch).unwrap();
        assert!(lines.iter().any(|l| l.starts_with("Cloning into")), "{lines:?}");
    }

    /// An ssh "host" that never answers.
    const SILENT_SSH: &str = "sh -c 'sleep 20'";

    #[test]
    fn a_stalled_clone_is_killed_and_leaves_no_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("stalled");
        let cancel = AtomicBool::new(false);
        let watch = CloneWatch { stall: Duration::from_secs(1), cancel: &cancel, progress: &mut |_| {} };
        let started = Instant::now();
        let r = clone_with_ssh("ssh://example.invalid/o/r.git", &dest, SILENT_SSH, watch);
        assert!(matches!(&r, Err(GitError::Command(m)) if m.contains("stalled")), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "killed, not waited out");
        assert!(!dest.exists());
    }

    #[test]
    fn a_cancelled_clone_is_killed_and_leaves_no_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("cancelled");
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let r = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                cancel.store(true, Ordering::Relaxed);
            });
            let watch = CloneWatch { stall: CLONE_STALL_TIMEOUT, cancel: &cancel, progress: &mut |_| {} };
            clone_with_ssh("ssh://example.invalid/o/r.git", &dest, SILENT_SSH, watch)
        });
        assert!(matches!(r, Err(GitError::Cancelled)), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!dest.exists());
    }

    #[test]
    fn tells_progress_from_what_git_has_to_say() {
        assert!(is_progress("Receiving objects:  45% (450/1000), 1.20 MiB | 600.00 KiB/s"));
        assert!(is_progress("remote: Enumerating objects: 5, done."));
        assert!(!is_progress("fatal: repository 'x' not found"));
        assert_eq!(in_words(Duration::from_secs(300)), "5 minutes");
        assert_eq!(in_words(Duration::from_secs(2)), "2 seconds");
    }
}
