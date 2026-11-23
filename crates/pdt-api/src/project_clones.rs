//! Add project by URL (PDT-712): the clone runs as a job of its own, so a page that reloads or closes doesn't strand
//! it. The request waits for it and answers with the project; the clones under way are listed with git's progress,
//! and one can be cancelled (git is killed, its folder removed).

use crate::error::{ApiError, codes};
use crate::projects::{add, ensure_parent, projects_dir};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::project::{CloneJob, CloneProject, Project};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct Running {
    job: Mutex<CloneJob>,
    cancel: AtomicBool,
    /// Set once git is done: a Cancel then comes too late.
    cloned: AtomicBool,
}

/// The clones under way in this pdtd, by id.
#[derive(Default)]
pub(crate) struct CloneJobs(Mutex<HashMap<String, Arc<Running>>>);

impl CloneJobs {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Running>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers `job`, unless another clone already goes into its folder or the folder exists. The clone under way
    /// is checked first: git makes its folder at once, and "add it by path" would be the wrong advice for it.
    fn start(&self, job: CloneJob) -> Result<Arc<Running>, ApiError> {
        let mut map = self.map();
        if map.values().any(|r| r.job.lock().unwrap_or_else(|e| e.into_inner()).dest == job.dest) {
            return Err(ApiError::conflict(codes::PATH_EXISTS, format!("{} is already being cloned", job.dest)));
        }
        if std::path::Path::new(&job.dest).exists() {
            return Err(ApiError::conflict(
                codes::PATH_EXISTS,
                format!("{} already exists; add it by path instead", job.dest),
            ));
        }
        let running = Arc::new(Running {
            job: Mutex::new(job.clone()),
            cancel: AtomicBool::new(false),
            cloned: AtomicBool::new(false),
        });
        map.insert(job.id, running.clone());
        Ok(running)
    }

    fn list(&self) -> Vec<CloneJob> {
        let mut list: Vec<CloneJob> =
            self.map().values().map(|r| r.job.lock().unwrap_or_else(|e| e.into_inner()).clone()).collect();
        list.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.id.cmp(&b.id)));
        list
    }
}

/// Takes a job off the list when its clone is over, however it ended.
struct Done(Arc<CloneJobs>, String);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.map().remove(&self.1);
    }
}

pub(crate) async fn clone(
    State(st): State<AppState>,
    payload: Result<Json<CloneProject>, JsonRejection>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let Json(input) = payload?;
    let url = input.url.trim().to_string();
    let bad = |m: &str| ApiError::invalid(m.to_string());
    if url.is_empty() || url.starts_with('-') {
        return Err(bad("enter the repository's URL"));
    }
    let repo = pdt_core::git_url::repo_name(&url).ok_or_else(|| bad("the URL does not name a repository"))?;
    let key = input.key.as_deref().filter(|k| !k.trim().is_empty()).map(pdt_core::project::valid_key).transpose()?;
    let name = input.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&repo).to_string();
    let parent = match input.parent_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => pdt_core::project::resolve_folder(d, st.home_dir.get().as_deref())
            .map_err(|e| ApiError::bad_request(codes::INVALID_FOLDER, e.to_string()))?,
        None => {
            let setting = st.store.get_host_settings().await?.projects_dir;
            projects_dir(&st, setting.as_deref()).ok_or_else(|| bad("set a Projects folder in Host settings"))?
        }
    };
    let check = parent.clone();
    tokio::task::spawn_blocking(move || ensure_parent(&check)).await.map_err(ApiError::internal)??;
    let dest = parent.join(&repo);
    let running = st.clone_jobs.start(CloneJob {
        id: uuid::Uuid::now_v7().to_string(),
        url: url.clone(),
        dest: dest.to_string_lossy().into_owned(),
        name: name.clone(),
        progress: String::new(),
        started_at: now_ms(),
    })?;
    // A task of its own: dropping this request (a reload) leaves it to finish and add the project.
    tokio::spawn(run(st, running, url, dest, name, key)).await.map_err(ApiError::internal)?
}

async fn run(
    st: AppState,
    running: Arc<Running>,
    url: String,
    dest: PathBuf,
    name: String,
    key: Option<String>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let id = running.job.lock().unwrap_or_else(|e| e.into_inner()).id.clone();
    let _done = Done(st.clone_jobs.clone(), id);
    let target = dest.clone();
    let job = running.clone();
    let cloned = tokio::task::spawn_blocking(move || {
        let mut progress =
            |line: &str| line.clone_into(&mut running.job.lock().unwrap_or_else(|e| e.into_inner()).progress);
        let watch = pdt_git::CloneWatch {
            stall: pdt_git::CLONE_STALL_TIMEOUT,
            cancel: &running.cancel,
            progress: &mut progress,
        };
        pdt_git::clone_watched(&url, &target, watch)
    })
    .await
    .map_err(ApiError::internal)?;
    cloned.map_err(|e| match e {
        pdt_git::GitError::Command(m) => {
            tracing::warn!(error = %m, "git clone failed");
            ApiError::upstream(codes::CLONE_FAILED, format!("git clone failed: {}", crate::error::short_text(&m)))
        }
        other => other.into(),
    })?;
    // A Cancel that got in as git finished still wins (see `cancel`): either it sees `cloned` and is refused, or
    // this sees its flag.
    job.cloned.store(true, Ordering::SeqCst);
    if job.cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(pdt_git::GitError::Cancelled.into());
    }
    match add(&st, dest.clone(), name, None, key).await {
        Ok(p) => Ok((StatusCode::CREATED, Json(p))),
        Err(e) => {
            // The clone is ours and unused: don't leave it behind.
            let _ = std::fs::remove_dir_all(&dest);
            Err(e)
        }
    }
}

pub(crate) async fn list(State(st): State<AppState>) -> Json<Vec<CloneJob>> {
    Json(st.clone_jobs.list())
}

/// Stops a clone: git is killed and its folder removed; the request that started it answers `clone_cancelled`. Once
/// git is done and the project is being added, it is too late (409 `clone_finished`).
pub(crate) async fn cancel(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let running = st.clone_jobs.map().get(&id).cloned().ok_or_else(|| ApiError::not_found("clone"))?;
    if running.cloned.load(Ordering::SeqCst) {
        return Err(ApiError::conflict(codes::CLONE_FINISHED, "the clone has finished; the project is being added"));
    }
    running.cancel.store(true, Ordering::SeqCst);
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get, post};
    axum::Router::new()
        .route("/projects/clone", post(clone))
        .route("/projects/clones", get(list))
        .route("/projects/clones/{id}", delete(cancel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, dest: &str) -> CloneJob {
        CloneJob {
            id: id.into(),
            url: "u".into(),
            dest: dest.into(),
            name: "n".into(),
            progress: String::new(),
            started_at: 0,
        }
    }

    #[test]
    fn one_clone_per_folder_and_off_the_list_when_done() {
        let jobs = Arc::new(CloneJobs::default());
        let tmp = tempfile::tempdir().unwrap();
        let (r, s) = (tmp.path().join("r"), tmp.path().join("s"));
        let (r, s) = (r.to_str().unwrap(), s.to_str().unwrap());
        jobs.start(job("a", r)).unwrap();
        // git has made the folder: a second clone hears it's under way, not "add it by path".
        std::fs::create_dir(r).unwrap();
        let busy = jobs.start(job("b", r)).err().unwrap();
        assert_eq!(busy.code(), codes::PATH_EXISTS);
        assert!(busy.message().contains("already being cloned"), "{}", busy.message());
        jobs.start(job("c", s)).unwrap();
        assert_eq!(jobs.list().iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        drop(Done(jobs.clone(), "a".into()));
        assert_eq!(jobs.list().len(), 1);
        let there = jobs.start(job("b", r)).err().unwrap();
        assert!(there.message().contains("add it by path"), "{}", there.message());
        std::fs::remove_dir(r).unwrap();
        jobs.start(job("b", r)).unwrap();
    }
}
