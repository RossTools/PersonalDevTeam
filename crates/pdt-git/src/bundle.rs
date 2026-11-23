//! Moving a branch between two repositories that can't reach each other with git (PDT-730): one writes a bundle
//! (`git bundle`), the other fetches from that file. An owner instance and its remote instance swap them over their
//! link, for a repository with no remote both could use.

use crate::{GitError, command_failed, git_with_env, rev_parse};
use std::path::Path;

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    git_with_env(dir, args, &[])?.map_err(|f| command_failed(f.stderr))
}

/// Whether `repo` has the commit `sha`.
pub fn has_commit(repo: &Path, sha: &str) -> bool {
    crate::is_commit_id(sha)
        && git_with_env(repo, &["cat-file", "-e", &format!("{sha}^{{commit}}")], &[]).is_ok_and(|r| r.is_ok())
}

/// Writes `branch` to the bundle `file`, without the commits `have` reaches (those of them `repo` has: the other end
/// has them already). False when that leaves nothing to write (`have` has the branch's head); Err when there is no
/// such branch.
pub fn create_bundle(repo: &Path, file: &Path, branch: &str, have: &[String]) -> Result<bool, GitError> {
    let head = format!("refs/heads/{branch}");
    if rev_parse(repo, &head).is_none() {
        return Err(GitError::UnknownRef(branch.to_string()));
    }
    let exclude: Vec<String> = have.iter().filter(|h| has_commit(repo, h)).map(|h| format!("^{h}")).collect();
    let file = file.to_string_lossy();
    let mut args = vec!["bundle", "create", "--quiet", &*file, head.as_str()];
    args.extend(exclude.iter().map(String::as_str));
    match git_with_env(repo, &args, &[])? {
        Ok(_) => Ok(true),
        Err(f) if f.stderr.contains("empty bundle") => Ok(false),
        Err(f) => Err(command_failed(f.stderr)),
    }
}

/// Fetches `branch` from the bundle `file` into `repo`'s `refs/heads/<to>`: forced with `force` (the branch was
/// rewritten), else only as a fast-forward. Git refuses to move a branch a checkout has checked out.
pub fn fetch_bundle(repo: &Path, file: &Path, branch: &str, to: &str, force: bool) -> Result<(), GitError> {
    let spec = format!("{}refs/heads/{branch}:refs/heads/{to}", if force { "+" } else { "" });
    let file = file.to_string_lossy();
    run(repo, &["fetch", "--quiet", "--no-tags", "--end-of-options", &file, &spec]).map(|_| ())
}

/// Points `branch` at `sha`, a commit `repo` has, wherever it was. Git refuses to move a branch a checkout has checked
/// out.
pub fn set_branch(repo: &Path, branch: &str, sha: &str) -> Result<(), GitError> {
    run(repo, &["branch", "--force", "--end-of-options", branch, sha]).map(|_| ())
}

/// Makes `dir` a repository of `branch` from the bundle `file`: its checkout on the branch's head (detached, so the
/// branch can move later without touching it), and `branch` itself.
pub fn init_from_bundle(dir: &Path, file: &Path, branch: &str) -> Result<(), GitError> {
    std::fs::create_dir_all(dir).map_err(|e| GitError::Command(format!("creating {}: {e}", dir.display())))?;
    run(dir, &["init", "--quiet"])?;
    let file = file.to_string_lossy();
    run(dir, &["fetch", "--quiet", "--no-tags", "--end-of-options", &file, &format!("refs/heads/{branch}")])?;
    run(dir, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
    run(dir, &["branch", "--end-of-options", branch, "FETCH_HEAD"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

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

    #[test]
    fn a_branch_crosses_in_bundles_with_only_what_the_other_end_lacks() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        git(&a, &["init", "-q", "-b", "main"]);
        let first = commit(&a, "one.txt");
        let file = tmp.path().join("x.bundle");

        // A new repository from the whole branch: checked out, detached, with the branch.
        assert!(create_bundle(&a, &file, "main", &[]).unwrap());
        init_from_bundle(&b, &file, "main").unwrap();
        assert_eq!(rev_parse(&b, "refs/heads/main").as_deref(), Some(first.as_str()));
        assert!(b.join("one.txt").is_file());
        assert!(git(&b, &["status", "--porcelain=2", "--branch"]).contains("branch.head (detached)"));

        // Then only the new commits: what `b` has is left out, and a branch it hasn't got arrives.
        git(&a, &["checkout", "-q", "-b", "work"]);
        let work = commit(&a, "two.txt");
        assert!(!has_commit(&b, &work) && has_commit(&b, &first) && !has_commit(&b, "not-a-sha"));
        assert!(create_bundle(&a, &file, "work", &[first.clone(), "0".repeat(40)]).unwrap());
        let listed = git(&a, &["bundle", "list-heads", &file.to_string_lossy()]);
        assert!(listed.contains("refs/heads/work"), "{listed}");
        assert!(git(&a, &["bundle", "verify", &file.to_string_lossy()]).contains(&first), "it needs the first commit");
        fetch_bundle(&b, &file, "work", "work", false).unwrap();
        assert_eq!(rev_parse(&b, "refs/heads/work").as_deref(), Some(work.as_str()));

        // A rewritten branch needs `force`; nothing new is no bundle; no branch is an error.
        git(&a, &["reset", "-q", "--hard", &first]);
        let other = commit(&a, "three.txt");
        assert!(create_bundle(&a, &file, "work", std::slice::from_ref(&first)).unwrap());
        assert!(fetch_bundle(&b, &file, "work", "work", false).is_err());
        fetch_bundle(&b, &file, "work", "work", true).unwrap();
        assert_eq!(rev_parse(&b, "refs/heads/work").as_deref(), Some(other.as_str()));
        assert!(!create_bundle(&a, &file, "work", std::slice::from_ref(&other)).unwrap());
        set_branch(&b, "work", &work).unwrap();
        assert_eq!(rev_parse(&b, "refs/heads/work").as_deref(), Some(work.as_str()));
        assert!(matches!(create_bundle(&a, &file, "gone", &[]), Err(GitError::UnknownRef(_))));
    }
}
