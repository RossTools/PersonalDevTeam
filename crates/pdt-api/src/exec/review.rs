//! The Changes view, merge checks and overlaps for an attempt on a remote instance (PDT-635).
//!
//! - **Changes** cross the link whole: one call returns the diff's files and both sides of each (up to a budget), so
//!   opening Changes and reading its files is one round trip, not one per file. The owner keeps the last answer per
//!   worktree, stamped with a hash of the diff (merge base, head, dirty, and each file's version). It sends the stamp
//!   with its next call, and the remote instance answers "unchanged" when the diff is the same, without the sides.
//! - **Merge checks** (commits to merge, conflicts) run there, against the freshest of the base branch and its
//!   `origin` copy.
//! - **Overlaps:** the remote instance lists each attempt's changed files for the owner's overlap check.

use super::owner::call;
use super::{GIT_PATH, GitOp, GitReq};
use crate::error::{ApiError, codes};
use crate::routes::AppState;
use pdt_core::link::RemoteInstance;
use pdt_core::review::{ChangedFile, FileSides};
use pdt_core::worktree::Worktree;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a Changes call may take (it runs git there, and sends up to [`SIDES_BUDGET`]).
const CHANGES_TIMEOUT: Duration = Duration::from_secs(120);
/// How long listing an attempt's changed files for overlaps may take: the check runs every 30 s and waits for none.
const OVERLAP_TIMEOUT: Duration = Duration::from_secs(20);
/// The most text the sides in one Changes answer add up to; files past it are read one at a time when opened.
pub(super) const SIDES_BUDGET: usize = 8 * 1024 * 1024;
/// How long kept sides are served without a fresh Changes call confirming them.
const SIDES_FRESH: Duration = Duration::from_secs(60);
/// Worktrees whose Changes are kept at most.
const KEPT: usize = 64;

/// A remote worktree's changes, with the sides of its files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Bundle {
    /// The hash of everything but the sides: equal stamps, equal diffs.
    pub stamp: String,
    pub merge_base: String,
    pub head: String,
    pub dirty: bool,
    pub files: Vec<ChangedFile>,
    /// Both sides of the files within the budget (none for binary files).
    pub sides: Vec<FileSides>,
    /// For each of `sides`, the path the old side was read from.
    pub old_paths: Vec<String>,
}

impl Bundle {
    /// The merge base, head, dirty, and files.
    pub(crate) fn summary(&self) -> (String, String, bool, Vec<ChangedFile>) {
        (self.merge_base.clone(), self.head.clone(), self.dirty, self.files.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub(super) enum ChangesReply {
    /// The stamp the owner sent is current.
    Unchanged,
    Changed(Bundle),
}

/// What merging a remote branch would do.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeCheck {
    /// The worktree has uncommitted changes.
    pub dirty: bool,
    pub commits: u32,
    pub conflicts: Vec<String>,
}

struct Kept {
    bundle: Arc<Bundle>,
    /// By (old path, path).
    sides: HashMap<(String, String), FileSides>,
    /// When a Changes call last confirmed it.
    checked: Instant,
}

/// The owner's kept Changes, by worktree, base and fallback.
#[derive(Default)]
pub(super) struct ChangesCache {
    kept: Mutex<HashMap<String, Kept>>,
}

impl ChangesCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Kept>> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn forget(&self) {
        self.lock().clear();
    }

    fn stamp(&self, key: &str) -> Option<String> {
        self.lock().get(key).map(|k| k.bundle.stamp.clone())
    }

    /// The kept bundle for `key`, confirmed now (an "unchanged" answer).
    fn confirm(&self, key: &str, now: Instant) -> Option<Arc<Bundle>> {
        let mut kept = self.lock();
        let k = kept.get_mut(key)?;
        k.checked = now;
        Some(k.bundle.clone())
    }

    fn keep(&self, key: String, bundle: Bundle, now: Instant) -> Arc<Bundle> {
        let sides = bundle
            .sides
            .iter()
            .zip(&bundle.old_paths)
            .map(|(s, old)| ((old.clone(), s.path.clone()), s.clone()))
            .collect();
        let bundle = Arc::new(Bundle { sides: vec![], old_paths: vec![], ..bundle });
        let mut kept = self.lock();
        kept.insert(key, Kept { bundle: bundle.clone(), sides, checked: now });
        while kept.len() > KEPT {
            let Some(oldest) = kept.iter().min_by_key(|(_, k)| k.checked).map(|(id, _)| id.clone()) else { break };
            kept.remove(&oldest);
        }
        bundle
    }

    fn sides(&self, key: &str, old: &str, path: &str, now: Instant) -> Option<FileSides> {
        let kept = self.lock();
        let k = kept.get(key).filter(|k| now.saturating_duration_since(k.checked) < SIDES_FRESH)?;
        k.sides.get(&(old.to_string(), path.to_string())).cloned()
    }
}

fn key(wt: &Worktree, base: &str, fallback: &str) -> String {
    format!("{}\0{base}\0{fallback}", wt.id)
}

async fn git<T: serde::de::DeserializeOwned>(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    op: GitOp,
    timeout: Duration,
) -> Result<T, ApiError> {
    let req = GitReq { branch: branch.to_string(), op };
    call(st, i, GIT_PATH, &req, timeout).await.map_err(|e| e.passed_on(&i.name))
}

/// The remote worktree's changes against `base`: one call, answered "unchanged" when they are the ones kept.
pub(crate) async fn changes(
    st: &AppState,
    i: &RemoteInstance,
    wt: &Worktree,
    base: &str,
    fallback: &str,
) -> Result<Arc<Bundle>, ApiError> {
    let cache = &st.exec.changes;
    let key = key(wt, base, fallback);
    let mut known = cache.stamp(&key);
    loop {
        let op = GitOp::Changes { base: base.into(), fallback: fallback.into(), known: known.clone() };
        match git(st, i, &wt.branch, op, CHANGES_TIMEOUT).await? {
            ChangesReply::Changed(b) => return Ok(cache.keep(key, b, Instant::now())),
            ChangesReply::Unchanged => match cache.confirm(&key, Instant::now()) {
                Some(b) => return Ok(b),
                // Dropped from the cache meanwhile: ask for all of it.
                None if known.is_some() => known = None,
                None => return Err(ApiError::internal("the remote instance said unchanged to nothing")),
            },
        }
    }
}

/// A changed file's two sides in a remote worktree: kept from its last Changes, else read there.
pub(crate) async fn file_sides(
    st: &AppState,
    i: &RemoteInstance,
    wt: &Worktree,
    base: &str,
    fallback: &str,
    rel: String,
    old_rel: String,
) -> Result<FileSides, ApiError> {
    if let Some(s) = st.exec.changes.sides(&key(wt, base, fallback), &old_rel, &rel, Instant::now()) {
        return Ok(s);
    }
    let op = GitOp::Sides { base: base.into(), fallback: fallback.into(), path: rel, old_path: old_rel };
    git(st, i, &wt.branch, op, CHANGES_TIMEOUT).await
}

/// What merging the remote `branch` into `base` would do.
pub(crate) async fn merge_check(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    base: &str,
) -> Result<MergeCheck, ApiError> {
    git(st, i, branch, GitOp::MergeCheck { base: base.into() }, CHANGES_TIMEOUT).await
}

/// The files the remote `branch` changed against `base` (`fallback` without a merge base), for overlaps.
pub(crate) async fn changed_files(
    st: &AppState,
    i: &RemoteInstance,
    branch: &str,
    base: &str,
    fallback: &str,
) -> Result<Vec<String>, String> {
    let op = GitOp::ChangedFiles { base: base.into(), fallback: fallback.into() };
    git(st, i, branch, op, OVERLAP_TIMEOUT).await.map_err(|e| format!("{}: {}", i.name, e.message()))
}

/// The stamp of a diff: a hash of everything in it but the sides.
fn stamp(mb: &str, head: &str, dirty: bool, files: &[ChangedFile]) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_vec(&(mb, head, dirty, files)).unwrap_or_default();
    hex::encode(Sha256::digest(&text))
}

/// The worktree's changes (`GitOp::Changes`), on the remote instance: "unchanged" when they stamp as `known`, else
/// all of them with the sides of the files that fit in the budget. Blocking.
pub(super) fn changes_there(
    worktree: &Path,
    base: &str,
    fallback: &str,
    known: Option<&str>,
) -> Result<ChangesReply, ApiError> {
    let (merge_base, head, dirty, files) = crate::review::changes_in(worktree, base, fallback)?;
    let stamp = stamp(&merge_base, &head, dirty, &files);
    if known == Some(stamp.as_str()) {
        return Ok(ChangesReply::Unchanged);
    }
    let (mut sides, mut old_paths, mut left) = (vec![], vec![], SIDES_BUDGET);
    for f in files.iter().filter(|f| !f.binary) {
        let old = f.old_path.clone().unwrap_or_else(|| f.path.clone());
        // A side that can't be read now is read when the file is opened.
        let Ok(s) = crate::review::sides_at(worktree, &merge_base, f.path.clone(), &old) else { continue };
        let size = s.old.as_ref().map_or(0, String::len) + s.new.as_ref().map_or(0, String::len);
        if size > left {
            continue;
        }
        left -= size;
        sides.push(s);
        old_paths.push(old);
    }
    Ok(ChangesReply::Changed(Bundle { stamp, merge_base, head, dirty, files, sides, old_paths }))
}

/// `GitOp::MergeCheck`, on the remote instance: `branch` against the freshest of `base` and `origin/<base>`.
/// Blocking.
pub(super) fn merge_check_there(
    repo: &Path,
    worktree: &Path,
    branch: &str,
    base: &str,
) -> Result<MergeCheck, ApiError> {
    let dirty = worktree.is_dir() && pdt_git::is_dirty(worktree)?;
    let from = pdt_git::start_point(repo, base);
    if pdt_git::rev_parse(repo, &from).is_none() {
        return Err(ApiError::bad_request(codes::UNKNOWN_BRANCH, format!("{base} isn't in the checkout there")));
    }
    let (commits, conflicts) = pdt_git::merge_conflicts(repo, branch, &from)?;
    Ok(MergeCheck { dirty, commits, conflicts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::review::ChangeStatus;

    fn bundle(stamp: &str, files: &[&str]) -> Bundle {
        let file = |p: &&str| ChangedFile {
            path: p.to_string(),
            old_path: None,
            status: ChangeStatus::Modified,
            untracked: false,
            binary: false,
            added: 1,
            removed: 0,
            version: "v".into(),
            generated: false,
            uncommitted: false,
        };
        let side = |p: &&str| FileSides {
            path: p.to_string(),
            old: Some("a".into()),
            new: Some("b".into()),
            binary: false,
            too_large: false,
        };
        Bundle {
            stamp: stamp.into(),
            merge_base: "mb".into(),
            head: "h".into(),
            dirty: false,
            files: files.iter().map(file).collect(),
            sides: files.iter().map(side).collect(),
            old_paths: files.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn kept_sides_are_served_while_fresh_and_the_oldest_worktrees_go() {
        let cache = ChangesCache::default();
        let t0 = Instant::now();
        let b = cache.keep("w".into(), bundle("s1", &["a.rs"]), t0);
        assert!(b.sides.is_empty(), "the sides are kept apart");
        assert_eq!(cache.stamp("w").as_deref(), Some("s1"));
        assert!(cache.sides("w", "a.rs", "a.rs", t0 + Duration::from_secs(5)).is_some());
        assert!(cache.sides("w", "old.rs", "a.rs", t0).is_none(), "another old path is another file");
        assert!(cache.sides("w", "a.rs", "a.rs", t0 + SIDES_FRESH).is_none(), "stale until confirmed");
        assert!(cache.confirm("w", t0 + SIDES_FRESH).is_some());
        assert!(cache.sides("w", "a.rs", "a.rs", t0 + SIDES_FRESH).is_some());
        assert!(cache.confirm("nope", t0).is_none());

        for n in 0..KEPT {
            cache.keep(format!("x{n}"), bundle("s", &[]), t0 + SIDES_FRESH + Duration::from_secs(n as u64 + 1));
        }
        assert_eq!(cache.lock().len(), KEPT);
        assert!(cache.stamp("w").is_none(), "the least recently confirmed went");
    }

    #[test]
    fn the_stamp_follows_the_diff() {
        let b = bundle("", &["a.rs"]);
        let s = stamp("mb", "h", false, &b.files);
        assert_eq!(s, stamp("mb", "h", false, &b.files));
        assert_ne!(s, stamp("mb", "h", true, &b.files));
        let mut files = b.files.clone();
        files[0].version = "w".into();
        assert_ne!(s, stamp("mb", "h", false, &files));
    }
}
