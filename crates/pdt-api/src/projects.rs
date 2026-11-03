//! Project routes: list, add (by path or by cloning), rename/re-key, remove.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::project::{
    CloneProject, NewProject, Project, ProjectPatch, PullBranch, PullOutcome, PullResult, valid_key,
};
use pdt_git::FastForward;
use std::path::PathBuf;

/// Fills in `forge` from each project's `origin` and the forge plugin in use that claims it (git runs off the async
/// runtime).
pub(crate) async fn with_forge(st: &AppState, mut projects: Vec<Project>) -> Vec<Project> {
    let paths: Vec<Option<PathBuf>> =
        projects.iter().map(|p| p.has_repo().then(|| PathBuf::from(&p.repo_path))).collect();
    let plugins = st.forge_plugins();
    let links = tokio::task::spawn_blocking(move || {
        paths
            .iter()
            .map(|p| {
                let remote = pdt_core::git_url::parse_remote(&pdt_git::remote_url(p.as_ref()?, "origin")?)?;
                plugins.iter().find_map(|f| f.repo_of(&remote).and_then(|repo| f.links(&remote, &repo)))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    for (p, l) in projects.iter_mut().zip(links) {
        p.forge = l;
    }
    projects
}

async fn load(st: &AppState, id: &str) -> Result<Project, ApiError> {
    let p = st.project(id).await?;
    Ok(with_forge(st, vec![p]).await.remove(0))
}

fn key_taken() -> ApiError {
    ApiError::conflict(codes::KEY_TAKEN, "another project already uses this key")
}

pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(with_forge(&st, st.store.list_projects().await?).await))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Project>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

/// Registers the repository at `path` (any folder inside it) as a project.
async fn add(
    st: &AppState,
    path: PathBuf,
    name: String,
    base_branch: Option<String>,
    key: Option<String>,
) -> Result<Project, ApiError> {
    // The base defaults to the repository's default branch, not whichever branch happens to be checked out.
    let (info, default) = tokio::task::spawn_blocking(move || {
        let info = pdt_git::resolve_repo(&path)?;
        let default = pdt_git::default_branch(&info.root);
        Ok::<_, pdt_git::GitError>((info, default))
    })
    .await
    .map_err(ApiError::internal)??;
    if let Some(k) = &key
        && st.store.project_keys().await?.iter().any(|t| t == k)
    {
        return Err(key_taken());
    }
    let project = Project {
        id: uuid::Uuid::now_v7().to_string(),
        name,
        repo_path: info.root.to_string_lossy().into_owned(),
        base_branch: base_branch.or(default).or(info.current_branch).unwrap_or_else(|| "main".into()),
        created_at: now_ms(),
        key: key.unwrap_or_default(),
        forge: None,
        kind: Default::default(),
    };
    st.store.insert_project(&project).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(m) if m.contains("key") => key_taken(),
        pdt_store::StoreError::Conflict(_) => {
            ApiError::conflict(codes::CONFLICT, "a project for this repository already exists")
        }
        other => other.into(),
    })?;
    load(st, &project.id).await
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewProject>, JsonRejection>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let Json(input) = payload?;
    let v = input.validate()?;
    if v.kind == pdt_core::project::ProjectKind::Personal {
        return Ok((StatusCode::CREATED, Json(add_personal(&st, v.name, v.key).await?)));
    }
    // `~/…` (what the folder picker writes) is the home folder; anything else goes to git as typed.
    let path = match v.repo_path.starts_with('~') {
        true => pdt_core::project::resolve_folder(&v.repo_path, st.home_dir.as_deref())
            .map_err(|e| ApiError::bad_request(codes::INVALID_FOLDER, e.to_string()))?,
        false => PathBuf::from(&v.repo_path),
    };
    let p = add(&st, path, v.name, v.base_branch, v.key).await?;
    Ok((StatusCode::CREATED, Json(p)))
}

/// A Personal project: no repository, its folder `<data dir>/personal/<id>`.
async fn add_personal(st: &AppState, name: String, key: Option<String>) -> Result<Project, ApiError> {
    if let Some(k) = &key
        && st.store.project_keys().await?.iter().any(|t| t == k)
    {
        return Err(key_taken());
    }
    let id = uuid::Uuid::now_v7().to_string();
    let dir = st.data_dir.join("personal").join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::io("making the project's folder", e))?;
    let project = Project {
        id,
        name,
        repo_path: dir.to_string_lossy().into_owned(),
        base_branch: String::new(),
        created_at: now_ms(),
        key: key.unwrap_or_default(),
        forge: None,
        kind: pdt_core::project::ProjectKind::Personal,
    };
    st.store.insert_project(&project).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(m) if m.contains("key") => key_taken(),
        other => other.into(),
    })?;
    load(st, &project.id).await
}

/// 409 `personal_project` for what needs a repository.
pub(crate) fn needs_repo(p: &Project, what: &str) -> Result<(), ApiError> {
    if !p.has_repo() {
        return Err(ApiError::conflict(
            codes::PERSONAL_PROJECT,
            format!("{} is a Personal project (no repository): it has no {what}", p.name),
        ));
    }
    Ok(())
}

/// The folder new clones go in: the host setting (with `~` expanded), else `~/work`.
pub(crate) fn projects_dir(setting: Option<&str>) -> Option<PathBuf> {
    pdt_core::project::resolve_folder(setting.unwrap_or("~/work"), std::env::home_dir().as_deref()).ok()
}

/// Makes sure the folder a clone goes in exists. A missing folder is created only when its own parent exists, so a
/// mistyped path (`/User/me/work`) is reported instead of attempted.
fn ensure_parent(dir: &std::path::Path) -> Result<(), ApiError> {
    if dir.is_dir() {
        return Ok(());
    }
    if dir.exists() {
        return Err(ApiError::bad_request(codes::NOT_A_FOLDER, format!("{} is not a folder", dir.display())));
    }
    if !dir.parent().is_some_and(|p| p.is_dir()) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            codes::FOLDER_MISSING,
            format!("{} does not exist; check the path or create the folder first", dir.display()),
        ));
    }
    std::fs::create_dir(dir).map_err(|e| ApiError::io(&format!("creating {}", dir.display()), e))
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
    let key = input.key.as_deref().filter(|k| !k.trim().is_empty()).map(valid_key).transpose()?;
    let name = input.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&repo).to_string();
    let parent = match input.parent_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => pdt_core::project::resolve_folder(d, std::env::home_dir().as_deref())
            .map_err(|e| ApiError::bad_request(codes::INVALID_FOLDER, e.to_string()))?,
        None => {
            let setting = st.store.get_host_settings().await?.projects_dir;
            projects_dir(setting.as_deref()).ok_or_else(|| bad("set a Projects folder in Host settings"))?
        }
    };
    let check = parent.clone();
    tokio::task::spawn_blocking(move || ensure_parent(&check)).await.map_err(ApiError::internal)??;
    let dest = parent.join(&repo);
    if dest.exists() {
        return Err(ApiError::conflict(
            codes::PATH_EXISTS,
            format!("{} already exists; add it by path instead", dest.display()),
        ));
    }
    let target = dest.clone();
    tokio::task::spawn_blocking(move || pdt_git::clone(&url, &target)).await.map_err(ApiError::internal)?.map_err(
        |e| match e {
            pdt_git::GitError::Command(m) => ApiError::upstream(codes::CLONE_FAILED, format!("git clone failed: {m}")),
            other => other.into(),
        },
    )?;
    match add(&st, dest.clone(), name, None, key).await {
        Ok(p) => Ok((StatusCode::CREATED, Json(p))),
        Err(e) => {
            // The clone is ours and unused: don't leave it behind.
            let _ = std::fs::remove_dir_all(&dest);
            Err(e)
        }
    }
}

pub(crate) async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ProjectPatch>, JsonRejection>,
) -> Result<Json<Project>, ApiError> {
    let Json(patch) = payload?;
    let name = match patch.name.as_deref().map(str::trim) {
        Some("") => return Err(pdt_core::project::ValidationError::EmptyName.into()),
        other => other.map(String::from),
    };
    let key = patch.key.as_deref().map(valid_key).transpose()?;
    let base = match patch.base_branch.as_deref().map(str::trim) {
        Some("") => return Err(pdt_core::project::ValidationError::EmptyBranch.into()),
        Some(b) => Some(checked_base(&st, &id, b.to_string()).await?),
        None => None,
    };
    let found =
        st.store.update_project(&id, name.as_deref(), key.as_deref(), base.as_deref()).await.map_err(|e| match e {
            pdt_store::StoreError::Conflict(_) => key_taken(),
            other => other.into(),
        })?;
    if !found {
        return Err(ApiError::not_found("project"));
    }
    Ok(Json(load(&st, &id).await?))
}

/// `branch` if it names a branch of the project's repository, local or on `origin` (new worktrees start from it).
async fn checked_base(st: &AppState, id: &str, branch: String) -> Result<String, ApiError> {
    let project = st.project(id).await?;
    needs_repo(&project, "base branch")?;
    let repo = PathBuf::from(&project.repo_path);
    tokio::task::spawn_blocking(move || {
        if branch.starts_with('-') {
            return Err(pdt_git::GitError::InvalidBranch(branch).into());
        }
        pdt_git::check_branch_name(&repo, &branch)?;
        if !pdt_git::list_branches(&repo)?.iter().any(|b| b.name == branch) {
            return Err(ApiError::bad_request(
                codes::UNKNOWN_BRANCH,
                format!("{branch} is not a branch of this repository (locally or on origin)"),
            ));
        }
        Ok(branch)
    })
    .await
    .map_err(ApiError::internal)?
}

/// Removes a project from PDT: stops its sessions and runs and forgets everything about it (its tickets' attachments
/// too). The project's own files are never touched.
pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    st.project(&id).await?;
    for run in st.store.list_runs(&id).await? {
        if !crate::runs::discard(&st, &run).await {
            return Err(ApiError::conflict(
                codes::RUN_STOPPING,
                "a run of this project has not stopped yet; try again in a moment",
            ));
        }
    }
    for s in st.store.list_sessions(&id).await? {
        crate::sessions::discard(&st, &s.id).await;
    }
    st.store.delete_project(&id).await?;
    // Its tickets' attachments are PDT's own files, not the project's.
    crate::ticket_attachments::prune(&st).await;
    Ok(StatusCode::NO_CONTENT)
}

/// How long a pull's fetch may take.
pub(crate) const PULL_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Fast-forwards a branch (the base by default) to `origin`'s, in whichever checkout has it (PDT-56; "Move to Done
/// and remove the worktree" calls it so the main checkout has the merged work). Never merges and never touches
/// uncommitted work: anything else is reported in the outcome, not as an error.
pub(crate) async fn pull(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<PullBranch>, JsonRejection>,
) -> Result<Json<PullResult>, ApiError> {
    let input = match payload {
        Ok(Json(p)) => p,
        Err(JsonRejection::MissingJsonContentType(_)) => PullBranch::default(),
        Err(e) => return Err(e.into()),
    };
    let project = st.project(&id).await?;
    needs_repo(&project, "branches to pull")?;
    let branch = input.branch.filter(|b| !b.trim().is_empty()).unwrap_or_else(|| project.base_branch.clone());
    Ok(Json(pull_branch(&project, branch).await?))
}

/// Fast-forwards `branch` of `project` to `origin`'s (see `pull`).
pub(crate) async fn pull_branch(project: &pdt_core::project::Project, branch: String) -> Result<PullResult, ApiError> {
    let id = project.id.clone();
    let repo = PathBuf::from(&project.repo_path);
    let b = branch.clone();
    let ff = tokio::task::spawn_blocking(move || {
        if b.starts_with('-') {
            return Err(pdt_git::GitError::InvalidBranch(b));
        }
        pdt_git::check_branch_name(&repo, &b)?;
        pdt_git::fast_forward(&repo, &b, PULL_FETCH_TIMEOUT)
    })
    .await
    .map_err(ApiError::internal)??;
    let (outcome, message) = match ff {
        FastForward::Updated { .. } => (PullOutcome::Updated, None),
        FastForward::UpToDate => (PullOutcome::UpToDate, None),
        FastForward::NoRemote => (PullOutcome::NoRemote, None),
        FastForward::NoLocalBranch => (PullOutcome::NoBranch, None),
        FastForward::FetchFailed { message } => (PullOutcome::FetchFailed, Some(message)),
        FastForward::Diverged => (PullOutcome::Diverged, None),
        FastForward::Blocked { message, .. } => (PullOutcome::Blocked, Some(message)),
    };
    if let Some(m) = &message {
        tracing::info!(project = %id, branch = %branch, ?outcome, message = %m, "pull left the branch as it was");
    }
    Ok(PullResult { branch, outcome, message })
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/projects", get(list).post(create))
        .route("/projects/clone", post(clone))
        .route("/projects/{id}", get(get_one).patch(update).delete(remove))
        .route("/projects/{id}/pull", post(pull))
}
