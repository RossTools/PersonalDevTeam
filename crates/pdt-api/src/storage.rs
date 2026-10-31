//! Worktree storage (PDT-324): what each worktree takes on disk (measured in the background and kept in memory),
//! cleaning its build output, the automatic cleanup of finished tickets' worktrees, and low free disk space (an Inbox
//! item, and auto-start waits while a disk is critically low). The rules are pdt-core `storage`.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::inbox::InboxItem;
use pdt_core::project::{Project, ProjectKind};
use pdt_core::storage::{
    Cleanup, DiskLevel, DiskSpace, MAX_CLEANUP_DAYS, OutputPath, StorageSettings, WorktreeTicket, cleanup_due,
    detect_build_output, disk_level,
};
use pdt_core::ticket::TicketStage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ts_rs::TS;

/// How often the storage loop runs: it starts measuring what is stale, and cleans up hourly.
const STORAGE_EVERY: Duration = Duration::from_secs(60);
/// A worktree's size is measured again once it is this old.
const MEASURE_EVERY_MS: i64 = 30 * 60 * 1000;
/// How often the automatic cleanup looks for finished tickets' worktrees.
const CLEANUP_EVERY_MS: i64 = 60 * 60 * 1000;
/// How deep a `**/<name>` build output is looked for.
const MAX_DEPTH: usize = 12;

/// Free and total space of the disk holding a folder; None when it can't be read.
pub type DiskProbe = Arc<dyn Fn(&FsPath) -> Option<DiskSpace> + Send + Sync>;

/// The disk under a folder, read with the OS (the nearest folder above it that exists).
pub fn os_disk_space(path: &FsPath) -> Option<DiskSpace> {
    path.ancestors()
        .find_map(|d| Some(DiskSpace { free: fs4::available_space(d).ok()?, total: fs4::total_space(d).ok()? }))
}

/// What a measure found in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeSize {
    /// Bytes on disk, build output included.
    #[ts(type = "number")]
    pub total: u64,
    /// Bytes of build output.
    #[ts(type = "number")]
    pub build: u64,
    /// It has uncommitted changes (untracked files too).
    pub uncommitted: bool,
    /// Commits its branch's remote lacks (or every commit, when the branch was never pushed: see `neverPushed`).
    pub unpushed: u32,
    /// The repository has a remote but this branch was never pushed.
    pub never_pushed: bool,
    #[ts(type = "number")]
    pub measured_at: i64,
}

/// The in-memory sizes and the loop's state.
#[derive(Default)]
pub struct StorageState {
    sizes: Mutex<HashMap<String, WorktreeSize>>,
    measuring: AtomicBool,
    /// Measure again once the pass under way ends (something was cleaned or a refresh asked meanwhile).
    again: AtomicBool,
    last_cleanup: Mutex<Option<i64>>,
    /// Since when a disk has been low (the Inbox item's time).
    low_since: Mutex<Option<i64>>,
    /// Auto-start was paused at the scheduler's last look (logged once when it changes).
    paused: AtomicBool,
}

impl StorageState {
    fn size(&self, id: &str) -> Option<WorktreeSize> {
        self.sizes.lock().unwrap_or_else(|e| e.into_inner()).get(id).copied()
    }
    fn put(&self, id: &str, s: WorktreeSize) {
        self.sizes.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), s);
    }
    fn forget(&self, id: &str) {
        self.sizes.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
}

/// Bytes a file takes on disk.
#[cfg(unix)]
fn on_disk(md: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.blocks() * 512
}
#[cfg(not(unix))]
fn on_disk(md: &std::fs::Metadata) -> u64 {
    md.len()
}

/// A hard-linked file's identity, so one linked twice (a package store's links) is counted once.
#[cfg(unix)]
fn link_id(md: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (md.nlink() > 1 && !md.is_dir()).then(|| (md.dev(), md.ino()))
}
#[cfg(not(unix))]
fn link_id(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// The bytes under `dir` and how many of them are build output (`patterns`, matched against paths below `dir`).
/// Symbolic links are counted, not followed. Blocking.
pub fn measure(dir: &FsPath, patterns: &[OutputPath]) -> (u64, u64) {
    let (mut total, mut build) = (0u64, 0u64);
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut stack: Vec<(PathBuf, Vec<String>, bool)> = vec![(dir.to_path_buf(), vec![], false)];
    while let Some((path, rel, in_build)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else { continue };
        for e in entries.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let mut rel = rel.clone();
            rel.push(e.file_name().to_string_lossy().into_owned());
            let refs: Vec<&str> = rel.iter().map(String::as_str).collect();
            let is_build = in_build || patterns.iter().any(|p| p.matches(&refs));
            if link_id(&md).is_none_or(|id| seen.insert(id)) {
                let bytes = on_disk(&md);
                total += bytes;
                if is_build {
                    build += bytes;
                }
            }
            if md.is_dir() {
                stack.push((e.path(), rel, is_build));
            }
        }
    }
    (total, build)
}

/// The build output that exists under `dir`: each path below it (`/`-separated) that a pattern names, not looking
/// inside `.git` or inside one already found. Blocking.
pub fn find_outputs(dir: &FsPath, patterns: &[OutputPath]) -> Vec<String> {
    let mut out = vec![];
    let anywhere = patterns.iter().any(|p| matches!(p, OutputPath::Anywhere(_)));
    for p in patterns {
        if let OutputPath::At(segs) = p {
            let path = segs.iter().fold(dir.to_path_buf(), |d, s| d.join(s));
            if std::fs::symlink_metadata(&path).is_ok() {
                out.push(segs.join("/"));
            }
        }
    }
    if !anywhere {
        return out;
    }
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(dir.to_path_buf(), vec![])];
    while let Some((path, rel)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let mut rel = rel.clone();
            rel.push(name);
            let refs: Vec<&str> = rel.iter().map(String::as_str).collect();
            if patterns.iter().any(|p| p.matches(&refs)) {
                let joined = rel.join("/");
                if !out.contains(&joined) {
                    out.push(joined);
                }
                continue;
            }
            // Never inside a folder a literal path already names (it goes whole).
            if out.iter().any(|o| *o == rel.join("/")) {
                continue;
            }
            if rel.len() < MAX_DEPTH && e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((e.path(), rel));
            }
        }
    }
    out
}

/// What cleaning a worktree did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanResult {
    /// Bytes freed.
    #[ts(type = "number")]
    pub freed: u64,
    /// The build output removed, as paths in the worktree.
    pub removed: Vec<String>,
    /// Build output left alone: git tracks something in it, or doesn't ignore it.
    pub kept: Vec<String>,
    /// Paths that could not be removed (fully), with why.
    pub errors: Vec<String>,
}

/// Removes the build output under `dir` that git ignores and tracks nothing in. Blocking.
pub fn clean_checkout(dir: &FsPath, patterns: &[OutputPath]) -> Result<CleanResult, pdt_git::GitError> {
    let mut r = CleanResult::default();
    for rel in find_outputs(dir, patterns) {
        if !pdt_git::is_disposable(dir, &rel)? {
            r.kept.push(rel);
            continue;
        }
        let path = rel.split('/').fold(dir.to_path_buf(), |d, s| d.join(s));
        let Ok(md) = std::fs::symlink_metadata(&path) else { continue };
        let bytes = if md.is_dir() { measure(&path, &[]).0 + on_disk(&md) } else { on_disk(&md) };
        let removed = if md.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => {
                r.freed += bytes;
                r.removed.push(rel);
            }
            Err(e) => r.errors.push(format!("{rel}: {e}")),
        }
    }
    Ok(r)
}

/// A project's build output paths: its `[storage] build_output`, else detected from its main checkout.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildOutputPaths {
    pub paths: Vec<String>,
    /// Detected from the files in the checkout, not declared in the run configuration.
    pub detected: bool,
    /// Why the run configuration could not be read (the paths are detected then).
    pub error: Option<String>,
}

impl BuildOutputPaths {
    fn patterns(&self) -> Vec<OutputPath> {
        self.paths.iter().filter_map(|p| OutputPath::parse(p).ok()).collect()
    }
}

pub(crate) async fn build_output(st: &AppState, project: &Project) -> BuildOutputPaths {
    let repo = PathBuf::from(&project.repo_path);
    let (declared, error) = match crate::runconfig::load_effective(st, &project.id, &repo).await {
        Ok(c) => (c.build_output, None),
        Err(e) => (None, Some(e.message().to_string())),
    };
    if let Some(paths) = declared {
        return BuildOutputPaths { paths, detected: false, error };
    }
    let paths =
        tokio::task::spawn_blocking(move || detect_build_output(|f| repo.join(f).is_file())).await.unwrap_or_default();
    BuildOutputPaths { paths, detected: true, error }
}

/// Why a worktree can't be cleaned or removed now: a session or run works in it.
async fn busy(st: &AppState, project_id: &str, worktree_id: &str) -> Result<Option<&'static str>, ApiError> {
    let sessions = st.store.list_sessions(project_id).await?;
    let running = sessions
        .iter()
        .filter(|s| s.worktree_id.as_deref() == Some(worktree_id))
        .any(|s| st.ptys.is_running(&s.id) || s.status == pdt_core::session::SessionStatus::Running);
    if running {
        return Ok(Some("a session is running in this worktree"));
    }
    let run = st.store.list_active_runs().await?.into_iter().any(|r| r.worktree_id.as_deref() == Some(worktree_id));
    Ok(run.then_some("a run is going in this worktree"))
}

/// Measures every worktree whose size is missing or stale (all of them with `force`), in a task of its own; one at a
/// time. A call while one is under way has it measure again once it ends.
pub(crate) fn spawn_measure(st: &AppState, force: bool) {
    if st.storage.measuring.swap(true, Ordering::SeqCst) {
        st.storage.again.store(true, Ordering::SeqCst);
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        struct Done(Arc<StorageState>);
        impl Drop for Done {
            fn drop(&mut self) {
                self.0.measuring.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(st.storage.clone());
        let mut force = force;
        loop {
            measure_all(&st, force).await;
            if !st.storage.again.swap(false, Ordering::SeqCst) {
                break;
            }
            force = false;
        }
    });
}

async fn measure_all(st: &AppState, force: bool) {
    let Ok(projects) = st.store.list_projects().await else { return };
    let mut known = HashSet::new();
    for p in projects.iter().filter(|p| p.kind != ProjectKind::Personal) {
        let Ok(worktrees) = st.store.list_worktrees(&p.id).await else { continue };
        let patterns = build_output(st, p).await.patterns();
        for w in worktrees {
            known.insert(w.id.clone());
            let fresh = st.storage.size(&w.id).is_some_and(|s| now_ms() - s.measured_at < MEASURE_EVERY_MS);
            if (fresh && !force) || st.shutting_down.load(Ordering::SeqCst) {
                continue;
            }
            let (path, patterns) = (PathBuf::from(&w.path), patterns.clone());
            let measured = tokio::task::spawn_blocking(move || {
                if !path.is_dir() {
                    return None;
                }
                let (total, build) = measure(&path, &patterns);
                let uncommitted = pdt_git::is_dirty(&path).unwrap_or(false);
                let u = pdt_git::unpushed(&path).unwrap_or_default();
                Some(WorktreeSize {
                    total,
                    build,
                    uncommitted,
                    unpushed: u.ahead,
                    never_pushed: u.unpushed_branch,
                    measured_at: now_ms(),
                })
            })
            .await
            .ok()
            .flatten();
            match measured {
                Some(m) => st.storage.put(&w.id, m),
                None => st.storage.forget(&w.id),
            }
        }
    }
    st.storage.sizes.lock().unwrap_or_else(|e| e.into_inner()).retain(|id, _| known.contains(id));
}

/// One disk, with the folders on it PDT uses.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskView {
    /// PDT's data folder and the projects' folders on this disk.
    pub folders: Vec<String>,
    pub space: DiskSpace,
    pub level: DiskLevel,
}

/// Which disk a folder is on (the nearest folder above it that exists): its device on Unix, its volume (drive or
/// share) elsewhere; None when it can't be read.
pub fn disk_id(path: &FsPath) -> Option<String> {
    let existing = path.ancestors().find(|d| d.exists())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(std::fs::metadata(existing).ok()?.dev().to_string())
    }
    #[cfg(not(unix))]
    {
        match std::fs::canonicalize(existing).ok()?.components().next()? {
            std::path::Component::Prefix(p) => Some(p.as_os_str().to_string_lossy().to_ascii_uppercase()),
            _ => None,
        }
    }
}

/// One entry per disk from what each folder's probe read: folders on the same disk (by `disk_id`; without one, by
/// equal sizes) go together, with the least free space any of them read, since each was read at its own moment.
fn group_disks(found: Vec<(String, Option<String>, DiskSpace)>, low_gb: u32, critical_gb: u32) -> Vec<DiskView> {
    let mut groups: Vec<(Option<String>, DiskView)> = vec![];
    for (folder, id, space) in found {
        let same = |(gid, d): &&mut (Option<String>, DiskView)| match (&id, gid) {
            (Some(a), Some(b)) => a == b,
            (None, None) => d.space == space,
            _ => false,
        };
        match groups.iter_mut().find(same) {
            Some((_, d)) => {
                if !d.folders.contains(&folder) {
                    d.folders.push(folder);
                }
                if space.free < d.space.free {
                    d.space = space;
                }
            }
            None => groups.push((id, DiskView { folders: vec![folder], space, level: DiskLevel::Ok })),
        }
    }
    groups
        .into_iter()
        .map(|(_, mut d)| {
            d.level = disk_level(d.space.free, low_gb, critical_gb);
            d
        })
        .collect()
}

/// The disks with PDT's data folder and each project's folder, one entry per disk.
pub(crate) async fn disks(st: &AppState) -> Vec<DiskView> {
    let host = st.store.get_host_settings().await.unwrap_or_default();
    let mut folders = vec![st.data_dir.clone()];
    folders.extend(st.store.list_projects().await.unwrap_or_default().into_iter().map(|p| PathBuf::from(p.repo_path)));
    let probe = st.disk_space.clone();
    let found = tokio::task::spawn_blocking(move || {
        folders
            .into_iter()
            .filter_map(|f| {
                let space = probe(&f)?;
                Some((f.to_string_lossy().into_owned(), disk_id(&f), space))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    group_disks(found, host.low_disk_gb, host.critical_disk_gb)
}

/// Whether auto-start waits: a disk PDT uses is below the critical threshold.
pub(crate) async fn auto_start_paused(st: &AppState) -> bool {
    let paused = disks(st).await.iter().any(|d| d.level == DiskLevel::Critical);
    if st.storage.paused.swap(paused, Ordering::SeqCst) != paused {
        if paused {
            tracing::warn!("a disk is almost full: auto-start starts no new tickets until space is freed");
        } else {
            tracing::info!("there is disk space again: auto-start goes on");
        }
    }
    paused
}

/// The Inbox's low-disk item while a disk PDT uses is low: the fullest one.
pub(crate) async fn low_disk(st: &AppState) -> Option<InboxItem> {
    let worst = disks(st).await.into_iter().filter(|d| d.level != DiskLevel::Ok).min_by_key(|d| d.space.free);
    let mut since = st.storage.low_since.lock().unwrap_or_else(|e| e.into_inner());
    let Some(d) = worst else {
        *since = None;
        return None;
    };
    let at = *since.get_or_insert_with(now_ms);
    Some(pdt_core::inbox::low_disk_item(d.space.free, &d.folders, d.level == DiskLevel::Critical, at))
}

/// A worktree's ticket, for the Storage view.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeTicketView {
    pub id: String,
    pub key: String,
    pub title: String,
    pub stage: TicketStage,
    /// Done or Won't do, with no attempt working in the worktree: the automatic cleanup and Clean all finished
    /// take it.
    pub finished: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorktreeStorage {
    pub id: String,
    pub branch: String,
    pub path: String,
    /// The folder is gone.
    pub missing: bool,
    /// None until it has been measured.
    pub size: Option<WorktreeSize>,
    pub ticket: Option<WorktreeTicketView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectStorage {
    pub project_id: String,
    pub name: String,
    pub key: String,
    pub build_output: BuildOutputPaths,
    pub settings: StorageSettings,
    pub worktrees: Vec<WorktreeStorage>,
    /// The measured worktrees' bytes, and how many are build output.
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub build: u64,
}

/// `GET /api/storage`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageView {
    pub disks: Vec<DiskView>,
    pub projects: Vec<ProjectStorage>,
    pub low_disk_gb: u32,
    pub critical_disk_gb: u32,
    /// A disk is below the critical threshold: auto-start starts nothing new.
    pub auto_start_paused: bool,
    /// Worktrees are being measured now.
    pub measuring: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ProjectQuery {
    project: Option<String>,
}

async fn project_storage(st: &AppState, p: &Project) -> Result<ProjectStorage, ApiError> {
    let tickets: HashMap<String, pdt_store::WorktreeTicketRow> =
        st.store.worktree_tickets(&p.id).await?.into_iter().map(|t| (t.worktree_id.clone(), t)).collect();
    let mut worktrees = vec![];
    let (mut total, mut build) = (0, 0);
    for w in st.store.list_worktrees(&p.id).await? {
        let size = st.storage.size(&w.id);
        if let Some(s) = size {
            total += s.total;
            build += s.build;
        }
        let ticket = tickets.get(&w.id).map(|t| WorktreeTicketView {
            id: t.ticket_id.clone(),
            key: t.ticket_key.clone(),
            title: t.title.clone(),
            stage: t.stage,
            finished: t.stage.is_closed() && !t.attempt_active,
        });
        worktrees.push(WorktreeStorage {
            missing: !FsPath::new(&w.path).is_dir(),
            id: w.id,
            branch: w.branch,
            path: w.path,
            size,
            ticket,
        });
    }
    Ok(ProjectStorage {
        project_id: p.id.clone(),
        name: p.name.clone(),
        key: p.key.clone(),
        build_output: build_output(st, p).await,
        settings: st.store.get_project_storage(&p.id).await?,
        worktrees,
        total,
        build,
    })
}

/// `GET /api/storage?project=`: every project's worktrees with their sizes as last measured, and the disks.
pub(crate) async fn view(
    State(st): State<AppState>,
    Query(q): Query<ProjectQuery>,
) -> Result<Json<StorageView>, ApiError> {
    let host = st.store.get_host_settings().await?;
    let mut projects = vec![];
    for p in st.store.list_projects().await? {
        if p.kind == ProjectKind::Personal || q.project.as_ref().is_some_and(|id| *id != p.id) {
            continue;
        }
        projects.push(project_storage(&st, &p).await?);
    }
    let disks = disks(&st).await;
    Ok(Json(StorageView {
        auto_start_paused: disks.iter().any(|d| d.level == DiskLevel::Critical),
        disks,
        projects,
        low_disk_gb: host.low_disk_gb,
        critical_disk_gb: host.critical_disk_gb,
        measuring: st.storage.measuring.load(Ordering::SeqCst),
    }))
}

/// `POST /api/storage/refresh`: measures every worktree again, in the background.
pub(crate) async fn refresh(State(st): State<AppState>) -> StatusCode {
    spawn_measure(&st, true);
    StatusCode::NO_CONTENT
}

/// Cleans one worktree's build output: 409 `worktree_busy` while a session or run works in it.
pub(crate) async fn clean_worktree(st: &AppState, id: &str) -> Result<CleanResult, ApiError> {
    let wt = st.store.get_worktree(id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    let project = st.store.get_project(&wt.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    if let Some(why) = busy(st, &project.id, id).await? {
        return Err(ApiError::new(StatusCode::CONFLICT, "worktree_busy", format!("{why}; stop it first")));
    }
    let patterns = build_output(st, &project).await.patterns();
    let path = PathBuf::from(&wt.path);
    if !path.is_dir() {
        return Ok(CleanResult::default());
    }
    let r =
        tokio::task::spawn_blocking(move || clean_checkout(&path, &patterns)).await.map_err(ApiError::internal)??;
    if !r.removed.is_empty() {
        tracing::info!(branch = %wt.branch, freed = r.freed, removed = ?r.removed, "cleaned a worktree's build output");
    }
    st.storage.forget(id);
    spawn_measure(st, false);
    Ok(r)
}

/// `POST /api/worktrees/{id}/clean`.
pub(crate) async fn clean(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<CleanResult>, ApiError> {
    Ok(Json(clean_worktree(&st, &id).await?))
}

/// What Clean all finished did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanAllResult {
    /// Worktrees that had build output removed.
    pub cleaned: u32,
    #[ts(type = "number")]
    pub freed: u64,
    /// Worktrees skipped or failed, with why.
    pub skipped: Vec<String>,
}

/// The worktrees of finished tickets (Done or Won't do, no attempt in them), by project.
async fn finished_worktrees(st: &AppState, project: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
    let mut out = vec![];
    for p in st.store.list_projects().await? {
        if project.is_some_and(|id| id != p.id) {
            continue;
        }
        for t in st.store.worktree_tickets(&p.id).await? {
            if t.stage.is_closed() && !t.attempt_active {
                out.push((t.worktree_id, t.ticket_key));
            }
        }
    }
    Ok(out)
}

/// `POST /api/storage/clean-finished?project=`: cleans the build output of every finished ticket's worktree.
pub(crate) async fn clean_finished(
    State(st): State<AppState>,
    Query(q): Query<ProjectQuery>,
) -> Result<Json<CleanAllResult>, ApiError> {
    let mut r = CleanAllResult::default();
    for (id, key) in finished_worktrees(&st, q.project.as_deref()).await? {
        match clean_worktree(&st, &id).await {
            Ok(c) if !c.removed.is_empty() => {
                r.cleaned += 1;
                r.freed += c.freed;
            }
            Ok(_) => {}
            Err(e) => r.skipped.push(format!("{key}: {}", e.message())),
        }
    }
    Ok(Json(r))
}

/// `PUT /api/projects/{id}/storage`: the project's automatic cleanup.
pub(crate) async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<StorageSettings>, JsonRejection>,
) -> Result<Json<StorageSettings>, ApiError> {
    let Json(s) = payload?;
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    if s.clean_after_days > MAX_CLEANUP_DAYS || s.remove_after_days > MAX_CLEANUP_DAYS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("cleanup delays are at most {MAX_CLEANUP_DAYS} days (0: never)"),
        ));
    }
    st.store.put_project_storage(&id, &s).await?;
    Ok(Json(s))
}

/// One pass of the automatic cleanup: each finished ticket's worktree, once its project's delays have passed, has
/// its build output cleaned or is removed (its branch stays). Worktrees in use, with uncommitted changes, or
/// without a ticket are left alone.
pub async fn auto_cleanup(st: &AppState) {
    auto_cleanup_at(st, now_ms()).await
}

/// [`auto_cleanup`] as of `now` (tests move the clock).
pub async fn auto_cleanup_at(st: &AppState, now: i64) {
    let Ok(projects) = st.store.list_projects().await else { return };
    for p in projects.iter().filter(|p| p.kind != ProjectKind::Personal) {
        let Ok(settings) = st.store.get_project_storage(&p.id).await else { continue };
        let Ok(tickets) = st.store.worktree_tickets(&p.id).await else { continue };
        for t in tickets {
            let ticket = WorktreeTicket { stage: t.stage, closed_at: t.closed_at, attempt_active: t.attempt_active };
            match cleanup_due(Some(ticket), settings, now) {
                Cleanup::Nothing => {}
                Cleanup::Clean => {
                    if let Err(e) = clean_worktree(st, &t.worktree_id).await {
                        tracing::info!(ticket = %t.ticket_key, error = %e.message(), "cleanup left a worktree's build output");
                    }
                }
                Cleanup::Remove => match crate::worktrees::remove_worktree(st, &t.worktree_id, false, false).await {
                    Ok(()) => {
                        st.storage.forget(&t.worktree_id);
                        tracing::info!(ticket = %t.ticket_key, "cleanup removed a finished ticket's worktree");
                    }
                    Err(e) => {
                        tracing::info!(ticket = %t.ticket_key, error = %e.message(), "cleanup kept a finished ticket's worktree")
                    }
                },
            }
        }
    }
}

/// pdtd's storage loop: measures stale worktrees in the background, and runs the automatic cleanup hourly.
pub async fn run_storage(st: AppState) {
    crate::loops::run_every(st, "storage", STORAGE_EVERY, crate::loops::Start::AfterOnePeriod, |st| async move {
        let due = {
            let mut last = st.storage.last_cleanup.lock().unwrap_or_else(|e| e.into_inner());
            let due = last.is_none_or(|at| now_ms() - at >= CLEANUP_EVERY_MS);
            if due {
                *last = Some(now_ms());
            }
            due
        };
        if due {
            auto_cleanup(&st).await;
        }
        spawn_measure(&st, false);
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &FsPath, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn patterns(list: &[&str]) -> Vec<OutputPath> {
        list.iter().map(|p| OutputPath::parse(p).unwrap()).collect()
    }

    #[test]
    fn measures_a_tree_and_its_build_output() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(&d.join("src/main.rs"), 10_000);
        write(&d.join("target/debug/big"), 200_000);
        write(&d.join("web/node_modules/pkg/index.js"), 50_000);
        let (total, build) = measure(d, &patterns(&["target", "**/node_modules"]));
        assert!(build >= 250_000, "{build}");
        assert!(total >= build + 10_000, "{total} {build}");
        let (_, none) = measure(d, &[]);
        assert_eq!(none, 0);
    }

    #[test]
    fn folders_on_one_disk_are_one_disk_whatever_each_read_free() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b/not/yet"));
        std::fs::create_dir_all(&a).unwrap();
        assert!(disk_id(&a).is_some());
        assert_eq!(disk_id(&a), disk_id(&b), "a folder that doesn't exist yet is on its parent's disk");
        let gb = pdt_core::storage::GB;
        let space = |free| DiskSpace { free: free * gb, total: 500 * gb };
        let d = group_disks(
            vec![
                ("/data".into(), Some("1".into()), space(30)),
                ("/work/a".into(), Some("1".into()), space(19)),
                ("/ext".into(), Some("2".into()), space(19)),
                ("/x".into(), None, space(40)),
                ("/y".into(), None, space(40)),
            ],
            20,
            5,
        );
        assert_eq!(d.len(), 3, "{d:?}");
        assert_eq!(d[0].folders, ["/data", "/work/a"]);
        assert_eq!((d[0].space.free, d[0].level), (19 * gb, DiskLevel::Low), "the least free it read");
        assert_eq!(d[1].folders, ["/ext"]);
        assert_eq!(d[2].folders, ["/x", "/y"]);
    }

    #[test]
    fn finds_build_output_without_looking_inside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        write(&d.join("target/x/node_modules/a"), 1);
        write(&d.join("web/node_modules/a/node_modules/b"), 1);
        write(&d.join("node_modules/c"), 1);
        write(&d.join(".git/node_modules/x"), 1);
        let mut found = find_outputs(d, &patterns(&["target", "web/dist", "**/node_modules"]));
        found.sort();
        assert_eq!(found, ["node_modules", "target", "web/node_modules"]);
    }
}
