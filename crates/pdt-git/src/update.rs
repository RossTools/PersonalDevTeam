//! Bringing a branch up to date with its base in its worktree (PDT-328): merge the base in, take the base's side of
//! generated files, stage what a regeneration changed, commit the merge. Commits use the user's own git identity, as
//! an agent's would. `git rerere` replays a resolution recorded earlier.

use crate::{GitError, git_with_env, is_ancestor};
use std::path::Path;

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    git_with_env(dir, args, &[])?.map_err(|f| GitError::Command(f.stderr))
}

/// Turns on `git rerere` for the repository (shared by its worktrees), unless it is set either way already: a
/// conflict resolved once is resolved the same way the next time.
pub fn enable_rerere(repo: &Path) -> Result<(), GitError> {
    if let Ok(Ok(_)) = git_with_env(repo, &["config", "--get", "rerere.enabled"], &[]) {
        return Ok(());
    }
    run(repo, &["config", "rerere.enabled", "true"]).map(|_| ())
}

/// What [`merge_base_in`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseMerge {
    /// The branch has the base already; nothing changed.
    Current,
    /// The base merged in (its resolutions replayed by rerere, if any): `commit` is the merge.
    Merged { commit: String },
    /// The merge is in progress with these files conflicting (repository-relative, `/`-separated).
    Conflicts { files: Vec<String> },
}

/// Merges `base` (a ref: `origin/main`) into the branch checked out in `worktree`, which must be clean. Conflicts
/// leave the merge in progress; any other failure (untracked files in the way) aborts it.
pub fn merge_base_in(worktree: &Path, base: &str) -> Result<BaseMerge, GitError> {
    if is_ancestor(worktree, base, "HEAD") {
        return Ok(BaseMerge::Current);
    }
    let args = ["-c", "rerere.enabled=true", "-c", "rerere.autoUpdate=true", "merge", "--no-ff", "--no-edit", base];
    let failed = match git_with_env(worktree, &args, &[])? {
        Ok(_) => return Ok(BaseMerge::Merged { commit: run(worktree, &["rev-parse", "HEAD"])? }),
        Err(f) => f,
    };
    if !merge_in_progress(worktree) {
        return Err(GitError::Command(failed.stderr));
    }
    let files = unmerged_files(worktree)?;
    if !files.is_empty() {
        return Ok(BaseMerge::Conflicts { files });
    }
    // rerere resolved every conflict: commit the merge.
    match commit_merge(worktree) {
        Ok(commit) => Ok(BaseMerge::Merged { commit }),
        Err(e) => {
            abort_merge(worktree);
            Err(e)
        }
    }
}

/// Whether a merge is in progress in `worktree` (it has a `MERGE_HEAD`).
pub fn merge_in_progress(worktree: &Path) -> bool {
    matches!(git_with_env(worktree, &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"], &[]), Ok(Ok(s)) if !s.is_empty())
}

/// The files with unresolved conflicts.
pub fn unmerged_files(worktree: &Path) -> Result<Vec<String>, GitError> {
    let out = run(worktree, &["diff", "--name-only", "--diff-filter=U", "-z"])?;
    let mut files: Vec<String> = out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect();
    files.dedup();
    Ok(files)
}

/// Resolves each of `files` by taking the merged-in side (the base's), or removing it where the base deleted it.
pub fn take_theirs(worktree: &Path, files: &[String]) -> Result<(), GitError> {
    for f in files {
        if git_with_env(worktree, &["checkout", "--theirs", "--", f], &[])?.is_ok() {
            run(worktree, &["add", "--", f])?;
        } else {
            run(worktree, &["rm", "-q", "--cached", "--ignore-unmatch", "--", f])?;
            let _ = std::fs::remove_file(worktree.join(f));
        }
    }
    Ok(())
}

/// Stages every change (new, changed, deleted files) under the `globs` (as in `.pdt/project.toml`); a glob that
/// matches nothing is skipped.
pub fn stage_globs(worktree: &Path, globs: &[String]) -> Result<(), GitError> {
    for g in globs {
        let spec = format!(":(glob){}", g.trim().trim_start_matches("./"));
        if let Err(f) = git_with_env(worktree, &["add", "-A", "--", &spec], &[])?
            && !f.stderr.contains("did not match")
        {
            return Err(GitError::Command(f.stderr));
        }
    }
    Ok(())
}

/// Commits the merge in progress with git's own message; the new head.
pub fn commit_merge(worktree: &Path) -> Result<String, GitError> {
    run(worktree, &["commit", "--no-edit", "--no-verify"])?;
    run(worktree, &["rev-parse", "HEAD"])
}

/// Aborts the merge in progress, if any.
pub fn abort_merge(worktree: &Path) {
    if merge_in_progress(worktree) {
        let _ = git_with_env(worktree, &["merge", "--abort"], &[]);
    }
}

/// Moves the worktree's branch back to `commit`, dropping a merge made since (the worktree was clean before it).
pub fn undo_merge(worktree: &Path, commit: &str) -> Result<(), GitError> {
    run(worktree, &["reset", "-q", "--hard", commit]).map(|_| ())
}

/// `git diff` of the conflicted `files` (the conflict markers, against both sides).
pub fn conflict_diff(worktree: &Path, files: &[String]) -> String {
    let mut args = vec!["diff", "--no-color", "--"];
    args.extend(files.iter().map(String::as_str));
    match git_with_env(worktree, &args, &[]) {
        Ok(Ok(s)) => s,
        _ => String::new(),
    }
}

/// The subjects of the first-parent commits `to` has that `from` doesn't, newest first, at most `limit` (the tickets
/// merged into a base since a branch last had it).
pub fn first_parent_subjects(repo: &Path, from: &str, to: &str, limit: usize) -> Vec<String> {
    let range = format!("{from}..{to}");
    let n = format!("-{limit}");
    match git_with_env(repo, &["log", "--first-parent", "--format=%s", &n, &range, "--"], &[]) {
        Ok(Ok(s)) => s.lines().map(String::from).collect(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").current_dir(dir).args(["-c", "core.autocrlf=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("{file}: {}", text.trim())]);
    }

    /// A repository on `main` with `a.txt` and `gen.txt`, and a worktree on branch `b` started from it.
    fn setup(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        // Windows runners set autocrlf globally: a checkout would write the files back with CRLF.
        git(&repo, &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.join("gen.txt"), "gen 1\n").unwrap();
        write(&repo, "a.txt", "one\n");
        let wt = tmp.join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "b", wt.to_str().unwrap()]);
        (repo, wt)
    }

    #[test]
    fn a_branch_that_has_its_base_is_current_and_a_clean_merge_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt) = setup(tmp.path());
        assert_eq!(merge_base_in(&wt, "main").unwrap(), BaseMerge::Current);
        write(&wt, "b.txt", "branch\n");
        write(&repo, "c.txt", "main\n");
        let BaseMerge::Merged { commit } = merge_base_in(&wt, "main").unwrap() else { panic!("merged") };
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), commit);
        assert!(wt.join("c.txt").exists());
        assert_eq!(first_parent_subjects(&repo, "HEAD~1", "main", 10), ["c.txt: main"]);
    }

    #[test]
    fn conflicts_stay_in_progress_and_generated_ones_take_the_bases_side() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt) = setup(tmp.path());
        write(&wt, "a.txt", "branch\n");
        write(&wt, "gen.txt", "gen branch\n");
        write(&repo, "a.txt", "main\n");
        write(&repo, "gen.txt", "gen main\n");
        let BaseMerge::Conflicts { files } = merge_base_in(&wt, "main").unwrap() else { panic!("conflicts") };
        assert_eq!(files, ["a.txt", "gen.txt"]);
        assert!(merge_in_progress(&wt));
        assert!(conflict_diff(&wt, &files[..1]).contains("<<<<<<<"));

        take_theirs(&wt, &files[1..]).unwrap();
        std::fs::write(wt.join("gen.txt"), "regenerated\n").unwrap();
        stage_globs(&wt, &["gen.*".into(), "nothing/**".into()]).unwrap();
        assert_eq!(unmerged_files(&wt).unwrap(), ["a.txt"]);
        abort_merge(&wt);
        assert!(!merge_in_progress(&wt));
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "branch\n");
    }

    #[test]
    fn a_resolution_recorded_once_is_replayed() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt) = setup(tmp.path());
        enable_rerere(&repo).unwrap();
        assert_eq!(git(&repo, &["config", "--get", "rerere.enabled"]), "true");
        write(&wt, "a.txt", "branch\n");
        write(&repo, "a.txt", "main\n");
        let head = git(&wt, &["rev-parse", "HEAD"]);
        assert!(matches!(merge_base_in(&wt, "main").unwrap(), BaseMerge::Conflicts { .. }));
        std::fs::write(wt.join("a.txt"), "both\n").unwrap();
        git(&wt, &["add", "a.txt"]);
        commit_merge(&wt).unwrap();
        // The same conflict again (the merge undone): rerere resolves it and the merge commits.
        git(&wt, &["reset", "-q", "--hard", &head]);
        let BaseMerge::Merged { .. } = merge_base_in(&wt, "main").unwrap() else { panic!("replayed") };
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "both\n");
    }

    #[test]
    fn a_user_who_turned_rerere_off_keeps_it_off() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = setup(tmp.path());
        git(&repo, &["config", "rerere.enabled", "false"]);
        enable_rerere(&repo).unwrap();
        assert_eq!(git(&repo, &["config", "--get", "rerere.enabled"]), "false");
    }
}
