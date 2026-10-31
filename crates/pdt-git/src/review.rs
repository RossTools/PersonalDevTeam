//! Review (Phase 6, review spec §2): what a worktree's branch changed, file by file, the two sides of a file, and
//! merging the branch into its base locally. Everything but [`merge_local`] is read-only.

use crate::{GitError, git_bytes, git_with_env, is_ancestor, is_dirty, list_worktrees, merge_base, rev_parse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A file a branch changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    /// Checkout-relative, `/`-separated; the new name of a rename, the old one of a deleted file.
    pub path: String,
    /// The name before a rename.
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Not tracked by git (and not ignored): a new file nobody has added.
    pub untracked: bool,
    pub binary: bool,
    pub added: u32,
    pub removed: u32,
    /// Changes whenever either side of the file does (both sides' blob ids).
    pub version: String,
    /// Marked `linguist-generated` in `.gitattributes` (as GitHub reads it).
    pub generated: bool,
    /// Differs from `HEAD` (staged, unstaged or untracked): not committed yet.
    pub uncommitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Bytes read for the binary check (as git does).
const SNIFF: usize = 8000;

fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(SNIFF)].contains(&0)
}

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    git_with_env(dir, args, &[])?.map_err(|f| GitError::Command(f.stderr))
}

fn run_bytes(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
    git_bytes(dir, args, stdin)?.map_err(|f| GitError::Command(f.stderr))
}

/// The commit a worktree's changes are measured from: the newer of `HEAD`'s merge bases with `base` and with
/// `origin/<base>` (a branch started from a newer `origin/<base>` than the local base must not show main's own
/// commits), else `fallback` (the commit the branch started from) when there is no common ancestor.
pub fn review_base(worktree: &Path, base: &str, fallback: &str) -> String {
    let local = merge_base(worktree, "HEAD", &format!("refs/heads/{base}"));
    let remote = merge_base(worktree, "HEAD", &format!("refs/remotes/origin/{base}"));
    match (local, remote) {
        (Some(l), Some(r)) => {
            if is_ancestor(worktree, &l, &r) {
                r
            } else {
                l
            }
        }
        (Some(c), None) | (None, Some(c)) => c,
        (None, None) => rev_parse(worktree, fallback).unwrap_or_else(|| fallback.to_string()),
    }
}

fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
}

fn is_zero(oid: &str) -> bool {
    oid.bytes().all(|b| b == b'0')
}

/// Every file the worktree changed since `merge_base`: committed, staged and unstaged changes (renames detected),
/// and untracked files that are not ignored. Sorted by path.
pub fn diff_files(worktree: &Path, merge_base: &str) -> Result<Vec<DiffEntry>, GitError> {
    // `--raw`: status and blob ids (a working-tree side that differs from the index shows as zeros).
    let raw = run_bytes(
        worktree,
        &["-c", "core.quotepath=off", "diff", "-M", "--raw", "--no-abbrev", "-z", merge_base, "--"],
        None,
    )?;
    let fields = nul_fields(&raw);
    let mut entries: BTreeMap<String, DiffEntry> = BTreeMap::new();
    let mut hash_later: Vec<String> = vec![];
    let mut old_blobs: BTreeMap<String, String> = BTreeMap::new();
    let mut i = 0;
    while i < fields.len() {
        let meta = &fields[i];
        let parts: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        if parts.len() < 5 {
            i += 1;
            continue;
        }
        let (old_oid, new_oid, code) = (parts[2], parts[3], parts[4]);
        let renamed = code.starts_with('R') || code.starts_with('C');
        let (old_path, path) = if renamed {
            let p =
                (Some(fields.get(i + 1).cloned().unwrap_or_default()), fields.get(i + 2).cloned().unwrap_or_default());
            i += 3;
            p
        } else {
            let p = (None, fields.get(i + 1).cloned().unwrap_or_default());
            i += 2;
            p
        };
        let status = match code.chars().next() {
            Some('A') => FileStatus::Added,
            Some('D') => FileStatus::Deleted,
            Some('R') => FileStatus::Renamed,
            Some('C') => FileStatus::Added,
            _ => FileStatus::Modified,
        };
        let new_blob = if status == FileStatus::Deleted {
            String::new()
        } else if is_zero(new_oid) {
            hash_later.push(path.clone());
            String::new()
        } else {
            new_oid.to_string()
        };
        old_blobs.insert(path.clone(), if is_zero(old_oid) { String::new() } else { old_oid.to_string() });
        let old_path = if code.starts_with('C') { None } else { old_path };
        entries.insert(
            path.clone(),
            DiffEntry {
                path,
                old_path,
                status,
                untracked: false,
                binary: false,
                added: 0,
                removed: 0,
                version: new_blob,
                generated: false,
                uncommitted: false,
            },
        );
    }

    // `--numstat`: line counts (`-` for binary). A rename is `a\tr\t` then both names as their own fields.
    let num =
        run_bytes(worktree, &["-c", "core.quotepath=off", "diff", "-M", "--numstat", "-z", merge_base, "--"], None)?;
    let fields = nul_fields(&num);
    let mut i = 0;
    while i < fields.len() {
        let mut cols = fields[i].splitn(3, '\t');
        let (a, r, rest) = (cols.next().unwrap_or(""), cols.next().unwrap_or(""), cols.next().unwrap_or(""));
        let path = if rest.is_empty() {
            let p = fields.get(i + 2).cloned().unwrap_or_default();
            i += 3;
            p
        } else {
            i += 1;
            rest.to_string()
        };
        if let Some(e) = entries.get_mut(&path) {
            e.binary = a == "-" && r == "-";
            e.added = a.parse().unwrap_or(0);
            e.removed = r.parse().unwrap_or(0);
        }
    }

    // Untracked files: all added lines.
    let others =
        run_bytes(worktree, &["-c", "core.quotepath=off", "ls-files", "--others", "--exclude-standard", "-z"], None)?;
    for path in nul_fields(&others) {
        let bytes = std::fs::read(worktree.join(&path)).unwrap_or_default();
        let binary = is_binary(&bytes);
        let added = if binary {
            0
        } else {
            let n = bytes.iter().filter(|b| **b == b'\n').count();
            (n + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))) as u32
        };
        hash_later.push(path.clone());
        old_blobs.insert(path.clone(), String::new());
        entries.insert(
            path.clone(),
            DiffEntry {
                path,
                old_path: None,
                status: FileStatus::Added,
                untracked: true,
                binary,
                added,
                removed: 0,
                version: String::new(),
                generated: false,
                uncommitted: true,
            },
        );
    }

    // Blob ids of the working-tree sides, in one git call.
    if !hash_later.is_empty() {
        let input = hash_later.join("\n") + "\n";
        let out = run_bytes(worktree, &["hash-object", "--stdin-paths"], Some(input.as_bytes()))?;
        for (path, oid) in hash_later.iter().zip(String::from_utf8_lossy(&out).lines()) {
            if let Some(e) = entries.get_mut(path) {
                e.version = oid.trim().to_string();
            }
        }
    }
    // Files that differ from `HEAD` are not committed yet.
    let head = run_bytes(worktree, &["-c", "core.quotepath=off", "diff", "--name-only", "-z", "HEAD", "--"], None)?;
    for path in nul_fields(&head) {
        if let Some(e) = entries.get_mut(&path) {
            e.uncommitted = true;
        }
    }

    // `linguist-generated`, in one git call. Read from the working tree's `.gitattributes`; no answer marks nothing.
    if !entries.is_empty() {
        let input = entries.keys().map(|p| format!("{p}\0")).collect::<String>();
        if let Ok(Ok(out)) =
            git_bytes(worktree, &["check-attr", "-z", "--stdin", "linguist-generated"], Some(input.as_bytes()))
        {
            // `-z`: path, attribute, value, each NUL-terminated.
            let fields: Vec<String> = out.split(|b| *b == 0).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
            for [path, _attr, value] in fields.as_chunks::<3>().0 {
                if matches!(value.as_str(), "set" | "true")
                    && let Some(e) = entries.get_mut(path)
                {
                    e.generated = true;
                }
            }
        }
    }
    for e in entries.values_mut() {
        let old = old_blobs.get(&e.path).map(String::as_str).unwrap_or("");
        e.version = format!("{}:{}", short(old), short(&e.version));
    }
    Ok(entries.into_values().collect())
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(12)]
}

/// The file at `path` in commit `rev`, or None when that commit has no such file.
pub fn file_at(worktree: &Path, rev: &str, path: &str) -> Result<Option<Vec<u8>>, GitError> {
    let spec = format!("{rev}:{path}");
    match git_bytes(worktree, &["cat-file", "blob", &spec], None)? {
        Ok(bytes) => Ok(Some(bytes)),
        Err(f) if f.code == Some(128) => Ok(None),
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}

/// Whether commit `rev` has any file under the folder `dir` (e.g. `.github/workflows`), or the file `dir` itself
/// (`.gitlab-ci.yml`); false when it can't tell.
pub fn has_files_under(repo: &Path, rev: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    matches!(git_bytes(repo, &["ls-tree", "--name-only", rev, "--", dir], None), Ok(Ok(out)) if !out.trim_ascii().is_empty())
}

/// What merging `branch` into `base` would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePreview {
    /// Commits on `branch` that `base` lacks.
    pub commits: u32,
    /// Files that would conflict (empty when the merge is clean).
    pub conflicts: Vec<String>,
    /// The checkout that has `base` checked out, if any.
    pub base_checkout: Option<PathBuf>,
    /// Whether that checkout has uncommitted changes.
    pub base_dirty: bool,
}

/// The tree a merge of `branch` into `base` makes, and the files that conflict (`git merge-tree --write-tree`: exit 0
/// is clean, 1 has conflicts; stdout is the tree, then the conflicted files, either way).
fn merge_tree(repo: &Path, base: &str, branch: &str) -> Result<(String, Vec<String>), GitError> {
    use std::process::Command;
    let mut cmd = Command::new("git");
    cmd.env("LC_ALL", "C").env("LANGUAGE", "C");
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    let out = cmd
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=off", "merge-tree", "--write-tree", "--name-only", "--no-messages", base, branch])
        .output()
        .map_err(|e| GitError::Command(e.to_string()))?;
    let code = out.status.code();
    if !matches!(code, Some(0) | Some(1)) {
        return Err(GitError::Command(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let tree = lines.next().unwrap_or("").to_string();
    let mut files: Vec<String> =
        if code == Some(1) { lines.take_while(|l| !l.is_empty()).map(String::from).collect() } else { vec![] };
    files.dedup();
    Ok((tree, files))
}

/// Counts `branch`'s commits that `base` lacks, finds conflicts and where `base` is checked out. `base` and `branch`
/// are local branch names.
pub fn merge_preview(repo: &Path, branch: &str, base: &str) -> Result<MergePreview, GitError> {
    let range = format!("refs/heads/{base}..refs/heads/{branch}");
    let commits = run(repo, &["rev-list", "--count", &range])?.trim().parse().unwrap_or(0);
    let conflicts = if commits == 0 { vec![] } else { merge_tree(repo, &head_ref(base), &head_ref(branch))?.1 };
    let base_checkout = list_worktrees(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(base)).map(|w| w.path);
    let base_dirty = match &base_checkout {
        Some(dir) if dir.is_dir() => is_dirty(dir)?,
        _ => false,
    };
    Ok(MergePreview { commits, conflicts, base_checkout, base_dirty })
}

fn head_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// What [`merge_local`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// `base` now points at the merge commit `commit`.
    Merged { commit: String },
    /// Nothing was merged: these files conflict.
    Conflict { files: Vec<String> },
    /// The checkout holding `base` has uncommitted changes.
    BaseDirty { checkout: PathBuf },
    /// `branch` has no commits `base` lacks.
    Nothing,
}

/// Merges `branch` into `base` with a merge commit (`--no-ff`), or with `squash` one commit with a single parent
/// holding the same changes; either way with the user's own git identity and `message`. Where a checkout has `base`
/// checked out it merges there (`git merge`, which needs it clean, and is aborted on a conflict); otherwise no
/// checkout is touched: `merge-tree` writes the tree, `commit-tree` the commit and `update-ref` moves `base` only if
/// nobody moved it meanwhile.
pub fn merge_local(
    repo: &Path,
    branch: &str,
    base: &str,
    message: &str,
    squash: bool,
) -> Result<MergeOutcome, GitError> {
    let preview = merge_preview(repo, branch, base)?;
    if preview.commits == 0 {
        return Ok(MergeOutcome::Nothing);
    }
    if !preview.conflicts.is_empty() {
        return Ok(MergeOutcome::Conflict { files: preview.conflicts });
    }
    if let Some(dir) = preview.base_checkout.filter(|d| d.is_dir()) {
        if preview.base_dirty {
            return Ok(MergeOutcome::BaseDirty { checkout: dir });
        }
        let branch_ref = head_ref(branch);
        if squash {
            // `--squash` only stages the changes (no MERGE_HEAD to abort); the checkout was clean, so a failure resets.
            let done = match git_with_env(&dir, &["merge", "--squash", &branch_ref], &[])? {
                Ok(_) => git_with_env(&dir, &["commit", "-m", message], &[])?,
                Err(f) => Err(f),
            };
            if let Err(f) = done {
                let _ = git_with_env(&dir, &["reset", "--merge"], &[]);
                return Err(GitError::Command(f.stderr));
            }
        } else if let Err(f) = git_with_env(&dir, &["merge", "--no-ff", "--no-edit", "-m", message, &branch_ref], &[])?
        {
            let _ = git_with_env(&dir, &["merge", "--abort"], &[]);
            return Err(GitError::Command(f.stderr));
        }
        let commit = run(&dir, &["rev-parse", "HEAD"])?;
        return Ok(MergeOutcome::Merged { commit });
    }
    let base_ref = head_ref(base);
    let old = rev_parse(repo, &base_ref).ok_or_else(|| GitError::UnknownRef(base.to_string()))?;
    let theirs = rev_parse(repo, &head_ref(branch)).ok_or_else(|| GitError::UnknownRef(branch.to_string()))?;
    let (tree, conflicts) = merge_tree(repo, &old, &theirs)?;
    if !conflicts.is_empty() {
        return Ok(MergeOutcome::Conflict { files: conflicts });
    }
    let commit = if squash {
        run(repo, &["commit-tree", &tree, "-p", &old, "-m", message])?
    } else {
        run(repo, &["commit-tree", &tree, "-p", &old, "-p", &theirs, "-m", message])?
    };
    run(repo, &["update-ref", "-m", message, &base_ref, &commit, &old])?;
    Ok(MergeOutcome::Merged { commit })
}

/// Merges `branch` into `base` by a fast-forward when `base` hasn't moved since the branch left it, else as
/// [`merge_local`] does with a merge commit (`message`). A fast-forward moves the checkout that has `base` checked out
/// (`git merge --ff-only`, which needs it clean) or, with none, only the ref.
pub fn merge_fast_forward(repo: &Path, branch: &str, base: &str, message: &str) -> Result<MergeOutcome, GitError> {
    let (base_ref, branch_ref) = (head_ref(base), head_ref(branch));
    let old = rev_parse(repo, &base_ref).ok_or_else(|| GitError::UnknownRef(base.to_string()))?;
    let theirs = rev_parse(repo, &branch_ref).ok_or_else(|| GitError::UnknownRef(branch.to_string()))?;
    if old == theirs || !is_ancestor(repo, &old, &theirs) {
        return merge_local(repo, branch, base, message, false);
    }
    let preview = merge_preview(repo, branch, base)?;
    match preview.base_checkout.filter(|d| d.is_dir()) {
        Some(dir) if preview.base_dirty => Ok(MergeOutcome::BaseDirty { checkout: dir }),
        Some(dir) => {
            if let Err(f) = git_with_env(&dir, &["merge", "--ff-only", &branch_ref], &[])? {
                return Err(GitError::Command(f.stderr));
            }
            Ok(MergeOutcome::Merged { commit: run(&dir, &["rev-parse", "HEAD"])? })
        }
        None => {
            run(repo, &["update-ref", "-m", message, &base_ref, &theirs, &old])?;
            Ok(MergeOutcome::Merged { commit: theirs })
        }
    }
}

/// What [`prepare_merge`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedMerge {
    /// `commit` is what `base` (now `old`) becomes: `branch`'s head when `base` hasn't moved since it left, else a merge
    /// commit of the two. Nothing moved yet: [`advance_branch`] moves `base`.
    Ready {
        old: String,
        commit: String,
    },
    Conflict {
        files: Vec<String>,
    },
    BaseDirty {
        checkout: PathBuf,
    },
    Nothing,
}

/// [`merge_fast_forward`] without moving anything (PDT-245): the commit `base` would become, so it can be pushed to a
/// remote first and `base` moved only once the remote took it. A merge commit is made with the user's own identity and
/// `message`; nobody refers to it until `base` does.
pub fn prepare_merge(repo: &Path, branch: &str, base: &str, message: &str) -> Result<PreparedMerge, GitError> {
    let preview = merge_preview(repo, branch, base)?;
    if preview.commits == 0 {
        return Ok(PreparedMerge::Nothing);
    }
    if !preview.conflicts.is_empty() {
        return Ok(PreparedMerge::Conflict { files: preview.conflicts });
    }
    if let Some(dir) = preview.base_checkout.filter(|d| d.is_dir() && preview.base_dirty) {
        return Ok(PreparedMerge::BaseDirty { checkout: dir });
    }
    let old = rev_parse(repo, &head_ref(base)).ok_or_else(|| GitError::UnknownRef(base.to_string()))?;
    let theirs = rev_parse(repo, &head_ref(branch)).ok_or_else(|| GitError::UnknownRef(branch.to_string()))?;
    if is_ancestor(repo, &old, &theirs) {
        return Ok(PreparedMerge::Ready { old, commit: theirs });
    }
    let (tree, conflicts) = merge_tree(repo, &old, &theirs)?;
    if !conflicts.is_empty() {
        return Ok(PreparedMerge::Conflict { files: conflicts });
    }
    let commit = run(repo, &["commit-tree", &tree, "-p", &old, "-p", &theirs, "-m", message])?;
    Ok(PreparedMerge::Ready { old, commit })
}

/// Fast-forwards local `branch` from `old` to `commit` (a descendant): in the checkout that has it checked out
/// (`git merge --ff-only`, which needs it clean), else only the ref. Fails when `branch` is no longer at `old`.
pub fn advance_branch(repo: &Path, branch: &str, old: &str, commit: &str) -> Result<(), GitError> {
    let branch_ref = head_ref(branch);
    if rev_parse(repo, &branch_ref).as_deref() != Some(old) {
        return Err(GitError::Command(format!("{branch} moved meanwhile")));
    }
    let checkout = list_worktrees(repo)?.into_iter().find(|w| w.branch.as_deref() == Some(branch)).map(|w| w.path);
    match checkout.filter(|d| d.is_dir()) {
        Some(dir) => {
            if is_dirty(&dir)? {
                return Err(GitError::Command(format!("{} has uncommitted changes", dir.display())));
            }
            run(&dir, &["merge", "--ff-only", "--quiet", commit]).map(|_| ())
        }
        None => run(repo, &["update-ref", &branch_ref, commit, old]).map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn write(dir: &Path, file: &str, text: &[u8]) {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn commit_all(dir: &Path, msg: &str) -> String {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        git(dir, &["rev-parse", "HEAD"])
    }

    /// `main` with a few files, a worktree on `feature` from there. Returns (repo, worktree, base commit). The repo
    /// gets a local identity so merges made by the code under test can commit.
    fn setup(tmp: &Path) -> (PathBuf, PathBuf, String) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        // The merge under test runs git without the helpers' `-c core.autocrlf=false` (Windows CI sets it true).
        git(&repo, &["config", "core.autocrlf", "false"]);
        write(&repo, "a.txt", b"one\ntwo\nthree\n");
        write(&repo, "src/b.rs", b"fn b() {}\n");
        write(&repo, "src/old.rs", b"old\nfile\nhere\nwith\nlines\n");
        write(&repo, "gone.txt", b"gone\n");
        write(&repo, ".gitignore", b"*.log\n");
        let base = commit_all(&repo, "base");
        let wt = tmp.join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "feature", wt.to_str().unwrap(), "main"]);
        (repo, wt, base)
    }

    #[test]
    fn lists_committed_staged_unstaged_and_untracked_changes_with_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, base) = setup(tmp.path());
        assert!(diff_files(&wt, &base).unwrap().is_empty());

        write(&wt, "src/b.rs", b"fn b() {\n    1\n}\n");
        commit_all(&wt, "b");
        git(&wt, &["mv", "src/old.rs", "src/new.rs"]);
        commit_all(&wt, "rename");
        write(&wt, "a.txt", b"one\n2\nthree\n");
        git(&wt, &["add", "a.txt"]);
        std::fs::remove_file(wt.join("gone.txt")).unwrap();
        write(&wt, "web/new.tsx", b"x\ny");
        write(&wt, "img.bin", b"\x00\x01\x02");
        write(&wt, "debug.log", b"ignored\n");
        write(&repo, "main-only.txt", b"m\n");
        commit_all(&repo, "main moves");

        let mb = review_base(&wt, "main", &base);
        assert_eq!(mb, base, "main moved on, but the merge base is where the branch left it");
        let files = diff_files(&wt, &mb).unwrap();
        let brief: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.old_path.as_deref(), f.status, f.untracked, f.binary, f.added, f.removed))
            .collect();
        assert_eq!(
            brief,
            [
                ("a.txt", None, FileStatus::Modified, false, false, 1, 1),
                ("gone.txt", None, FileStatus::Deleted, false, false, 0, 1),
                ("img.bin", None, FileStatus::Added, true, true, 0, 0),
                ("src/b.rs", None, FileStatus::Modified, false, false, 3, 1),
                ("src/new.rs", Some("src/old.rs"), FileStatus::Renamed, false, false, 0, 0),
                ("web/new.tsx", None, FileStatus::Added, true, false, 2, 0),
            ]
        );
        assert!(files.iter().all(|f| f.version.contains(':')));
        let uncommitted: Vec<_> = files.iter().filter(|f| f.uncommitted).map(|f| f.path.as_str()).collect();
        assert_eq!(
            uncommitted,
            ["a.txt", "gone.txt", "img.bin", "web/new.tsx"],
            "staged, deleted and untracked; not committed"
        );
    }

    #[test]
    fn has_files_under_looks_in_the_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let (_repo, wt, _base) = setup(tmp.path());
        assert!(!has_files_under(&wt, "HEAD", ".github/workflows"));
        write(&wt, ".github/workflows/ci.yml", b"on: push\n");
        assert!(!has_files_under(&wt, "HEAD", ".github/workflows"), "not committed yet");
        commit_all(&wt, "ci");
        assert!(has_files_under(&wt, "HEAD", ".github/workflows"));
        assert!(!has_files_under(&wt, "nope", ".github/workflows"), "an unknown commit");
        assert!(!has_files_under(&wt, "HEAD", ".gitlab-ci.yml"));
        assert!(!has_files_under(&wt, "HEAD", ".github/work"), "a folder by its whole name only");
        write(&wt, ".gitlab-ci.yml", b"test:\n  script: [make]\n");
        commit_all(&wt, "gitlab ci");
        assert!(has_files_under(&wt, "HEAD", ".gitlab-ci.yml"), "a file");
    }

    #[test]
    fn marks_files_gitattributes_call_generated() {
        let tmp = tempfile::tempdir().unwrap();
        let (_repo, wt, base) = setup(tmp.path());
        write(
            &wt,
            ".gitattributes",
            b"gen/** linguist-generated\n*.pb.go linguist-generated=true\nkeep.pb.go -linguist-generated\n",
        );
        write(&wt, "gen/api.ts", b"x\n");
        write(&wt, "svc.pb.go", b"x\n");
        write(&wt, "keep.pb.go", b"x\n");
        write(&wt, "a.txt", b"hand\n");
        let generated: Vec<_> =
            diff_files(&wt, &base).unwrap().into_iter().filter(|f| f.generated).map(|f| f.path).collect();
        assert_eq!(generated, ["gen/api.ts", "svc.pb.go"]);
    }

    #[test]
    fn a_files_version_changes_with_its_content() {
        let tmp = tempfile::tempdir().unwrap();
        let (_repo, wt, base) = setup(tmp.path());
        write(&wt, "a.txt", b"changed\n");
        let v1 = diff_files(&wt, &base).unwrap()[0].version.clone();
        assert_eq!(diff_files(&wt, &base).unwrap()[0].version, v1, "stable while unchanged");
        write(&wt, "a.txt", b"changed again\n");
        assert_ne!(diff_files(&wt, &base).unwrap()[0].version, v1);
        write(&wt, "new.txt", b"n\n");
        let n1 = diff_files(&wt, &base).unwrap()[1].version.clone();
        write(&wt, "new.txt", b"n2\n");
        assert_ne!(diff_files(&wt, &base).unwrap()[1].version, n1, "untracked files too");
    }

    #[test]
    fn the_merge_base_follows_a_newer_origin_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _wt, base) = setup(tmp.path());
        // origin/main is ahead of the local main, and a branch started from it.
        write(&repo, "upstream.txt", b"u\n");
        let upstream = commit_all(&repo, "upstream");
        git(&repo, &["update-ref", "refs/remotes/origin/main", &upstream]);
        git(&repo, &["update-ref", "refs/heads/main", &base]);
        let wt2 = tmp.path().join("wt2");
        git(&repo, &["worktree", "add", "-q", "-b", "f2", wt2.to_str().unwrap(), "origin/main"]);
        write(&wt2, "mine.txt", b"m\n");
        let mb = review_base(&wt2, "main", &base);
        assert_eq!(mb, upstream);
        let files: Vec<_> = diff_files(&wt2, &mb).unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(files, ["mine.txt"]);
        // No common ancestor at all: the fallback.
        assert_eq!(review_base(&wt2, "no-such", &base), base);
    }

    #[test]
    fn reads_a_file_at_a_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let (_repo, wt, base) = setup(tmp.path());
        assert_eq!(file_at(&wt, &base, "a.txt").unwrap().unwrap(), b"one\ntwo\nthree\n");
        assert_eq!(file_at(&wt, &base, "nope.txt").unwrap(), None);
    }

    #[test]
    fn merges_into_a_base_nobody_has_checked_out() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, _base) = setup(tmp.path());
        // The main checkout moves off main so no checkout holds it.
        git(&repo, &["switch", "-q", "-c", "elsewhere"]);
        assert_eq!(merge_local(&repo, "feature", "main", "m", false).unwrap(), MergeOutcome::Nothing);
        write(&wt, "a.txt", b"one\nTWO\nthree\n");
        let head = commit_all(&wt, "two");
        let p = merge_preview(&repo, "feature", "main").unwrap();
        assert_eq!((p.commits, p.conflicts.len(), p.base_checkout, p.base_dirty), (1, 0, None, false));
        let MergeOutcome::Merged { commit } = merge_local(&repo, "feature", "main", "Merge feature", false).unwrap()
        else {
            panic!("not merged")
        };
        assert_eq!(git(&repo, &["rev-parse", "main"]), commit);
        assert_eq!(git(&repo, &["log", "-1", "--format=%s", "main"]), "Merge feature");
        assert!(is_ancestor(&repo, &head, &commit));
        assert_eq!(git(&repo, &["rev-parse", "main^2"]), head, "a merge commit, not a fast-forward");
    }

    #[test]
    fn fast_forwards_when_the_base_has_not_moved_else_makes_a_merge_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, _base) = setup(tmp.path());
        assert_eq!(merge_fast_forward(&repo, "feature", "main", "m").unwrap(), MergeOutcome::Nothing);
        write(&wt, "a.txt", b"one\nTWO\nthree\n");
        let head = commit_all(&wt, "two");
        write(&repo, "a.txt", b"dirty\n");
        assert!(matches!(merge_fast_forward(&repo, "feature", "main", "m").unwrap(), MergeOutcome::BaseDirty { .. }));
        git(&repo, &["checkout", "--", "a.txt"]);
        let done = merge_fast_forward(&repo, "feature", "main", "m").unwrap();
        assert_eq!(done, MergeOutcome::Merged { commit: head.clone() }, "a fast-forward");
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one\nTWO\nthree\n", "checkout moved");

        // No checkout of main: the ref moves alone.
        write(&wt, "c.txt", b"c\n");
        let head = commit_all(&wt, "c");
        git(&repo, &["switch", "-q", "-c", "elsewhere"]);
        assert_eq!(merge_fast_forward(&repo, "feature", "main", "m").unwrap(), MergeOutcome::Merged { commit: head });

        // main moved on: a merge commit.
        git(&repo, &["switch", "-q", "main"]);
        write(&repo, "d.txt", b"d\n");
        commit_all(&repo, "main moves");
        write(&wt, "e.txt", b"e\n");
        let head = commit_all(&wt, "e");
        let MergeOutcome::Merged { commit } = merge_fast_forward(&repo, "feature", "main", "Merge e").unwrap() else {
            panic!("not merged")
        };
        assert_eq!(git(&repo, &["rev-parse", &format!("{commit}^2")]), head);
        assert_eq!(git(&repo, &["log", "-1", "--format=%s", "main"]), "Merge e");
    }

    #[test]
    fn prepares_a_merge_without_moving_the_base_then_advances_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, base) = setup(tmp.path());
        assert_eq!(prepare_merge(&repo, "feature", "main", "m").unwrap(), PreparedMerge::Nothing);
        write(&wt, "a.txt", b"one\nTWO\nthree\n");
        let head = commit_all(&wt, "two");
        write(&repo, "a.txt", b"dirty\n");
        assert!(matches!(prepare_merge(&repo, "feature", "main", "m").unwrap(), PreparedMerge::BaseDirty { .. }));
        assert!(advance_branch(&repo, "main", &base, &head).is_err(), "a dirty checkout is not moved");
        git(&repo, &["checkout", "--", "a.txt"]);
        let ready = prepare_merge(&repo, "feature", "main", "m").unwrap();
        assert_eq!(ready, PreparedMerge::Ready { old: base.clone(), commit: head.clone() }, "a fast-forward");
        assert_eq!(git(&repo, &["rev-parse", "main"]), base, "nothing moved");
        assert!(advance_branch(&repo, "main", &head, &head).is_err(), "main is not at the old commit");
        advance_branch(&repo, "main", &base, &head).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one\nTWO\nthree\n", "checkout moved");

        // main moved on: a merge commit, made but not referred to until main is advanced (here with no checkout).
        write(&repo, "d.txt", b"d\n");
        let moved = commit_all(&repo, "main moves");
        git(&repo, &["switch", "-q", "-c", "elsewhere"]);
        write(&wt, "e.txt", b"e\n");
        let head = commit_all(&wt, "e");
        let PreparedMerge::Ready { old, commit } = prepare_merge(&repo, "feature", "main", "Merge e").unwrap() else {
            panic!("not ready")
        };
        assert_eq!((old.as_str(), git(&repo, &["rev-parse", "main"])), (moved.as_str(), moved.clone()));
        assert_eq!(git(&repo, &["rev-parse", &format!("{commit}^2")]), head);
        advance_branch(&repo, "main", &old, &commit).unwrap();
        assert_eq!(git(&repo, &["log", "-1", "--format=%s", "main"]), "Merge e");
    }

    #[test]
    fn merges_in_the_checkout_that_has_the_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, _base) = setup(tmp.path());
        write(&wt, "src/b.rs", b"fn b() { 2 }\n");
        commit_all(&wt, "b");
        write(&repo, "a.txt", b"one\ntwo\nthree\nfour\n");
        commit_all(&repo, "main moves");

        write(&repo, "a.txt", b"dirty\n");
        let p = merge_preview(&repo, "feature", "main").unwrap();
        assert!(p.base_dirty);
        assert!(matches!(merge_local(&repo, "feature", "main", "m", false).unwrap(), MergeOutcome::BaseDirty { .. }));
        git(&repo, &["checkout", "--", "a.txt"]);

        let MergeOutcome::Merged { commit } = merge_local(&repo, "feature", "main", "Merge it", false).unwrap() else {
            panic!("not merged")
        };
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), commit);
        assert_eq!(std::fs::read_to_string(repo.join("src/b.rs")).unwrap(), "fn b() { 2 }\n", "checkout updated");
        assert!(git(&repo, &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn squashes_into_one_commit_with_or_without_a_checkout_of_the_base() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, base) = setup(tmp.path());
        write(&wt, "src/b.rs", b"fn b() { 2 }\n");
        commit_all(&wt, "b");
        write(&wt, "a.txt", b"one\nTWO\nthree\n");
        let head = commit_all(&wt, "two");

        let MergeOutcome::Merged { commit } = merge_local(&repo, "feature", "main", "PDT-1: It", true).unwrap() else {
            panic!("not squashed")
        };
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), commit);
        assert_eq!(git(&repo, &["rev-parse", "main^"]), base, "one commit on top of the base");
        assert_eq!(git(&repo, &["rev-list", "--parents", "-n1", "main"]).split(' ').count(), 2, "a single parent");
        assert_eq!(git(&repo, &["log", "-1", "--format=%s", "main"]), "PDT-1: It");
        assert!(!is_ancestor(&repo, &head, &commit));
        assert_eq!(git(&repo, &["rev-parse", "main^{tree}"]), git(&repo, &["rev-parse", "feature^{tree}"]));
        assert!(git(&repo, &["status", "--porcelain"]).is_empty());

        // No checkout on main: commit-tree with one parent.
        write(&wt, "c.txt", b"c\n");
        commit_all(&wt, "c");
        git(&repo, &["switch", "-q", "-c", "elsewhere"]);
        let MergeOutcome::Merged { commit: second } = merge_local(&repo, "feature", "main", "again", true).unwrap()
        else {
            panic!("not squashed")
        };
        assert_eq!(git(&repo, &["rev-parse", "main"]), second);
        assert_eq!(git(&repo, &["rev-parse", "main^"]), commit);
        assert_eq!(git(&repo, &["rev-parse", "main^{tree}"]), git(&repo, &["rev-parse", "feature^{tree}"]));
    }

    #[test]
    fn a_conflict_is_reported_and_nothing_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, wt, _base) = setup(tmp.path());
        write(&wt, "a.txt", b"one\nmine\nthree\n");
        write(&wt, "src/b.rs", b"mine\n");
        commit_all(&wt, "mine");
        write(&repo, "a.txt", b"one\ntheirs\nthree\n");
        write(&repo, "src/b.rs", b"theirs\n");
        let before = commit_all(&repo, "theirs");
        let p = merge_preview(&repo, "feature", "main").unwrap();
        assert_eq!(p.conflicts, ["a.txt", "src/b.rs"]);
        assert_eq!(
            merge_local(&repo, "feature", "main", "m", false).unwrap(),
            MergeOutcome::Conflict { files: vec!["a.txt".into(), "src/b.rs".into()] }
        );
        assert_eq!(git(&repo, &["rev-parse", "main"]), before);
        assert!(git(&repo, &["status", "--porcelain"]).is_empty());
        // Also with no checkout on main.
        git(&repo, &["switch", "-q", "-c", "elsewhere"]);
        assert!(matches!(merge_local(&repo, "feature", "main", "m", false).unwrap(), MergeOutcome::Conflict { .. }));
        assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    }
}
