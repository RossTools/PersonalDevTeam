//! Git between an owner and its remote instance over their link (PDT-730), for what git alone can't do: a project
//! with no `origin` (the local forge's usual case) can't be cloned there, and the owner's local forge reads, runs CI
//! on and merges a branch that lives there. Each side asks the other for a bundle of one branch, leaving out the
//! commits it says it has (`git bundle`), and fetches from it.
//!
//! - **The base, owner → remote.** A remote checkout with no `origin` is made from a bundle of the owner's base branch
//!   and catches up with it the same way whenever it would have fetched it (`/link/exec/git/base`, the remote asks).
//! - **A branch, remote → owner.** The local forge reads a remote attempt's branch head there and, when it is new to
//!   the owner, fetches it into the owner's checkout (`/link/exec/git/branch`, the owner asks), where it runs as any
//!   branch's change request.

use super::owner::{branch_head, send};
use super::remote::checked;
use crate::error::ApiError;
use crate::link::{self, PeerError};
use crate::routes::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pdt_core::link::{OwnerLink, RemoteInstance};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) const BASE_PATH: &str = "/api/link/exec/git/base";
pub(super) const BRANCH_PATH: &str = "/api/link/exec/git/branch";

/// How long a bundle may take to make and send (a whole repository's, the first time).
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// A bundle of `branch`, without the commits `have` reaches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BundleReq {
    pub branch: String,
    pub have: Vec<String>,
}

/// A bundle file of this instance's, removed when dropped.
struct TempBundle(PathBuf);

impl TempBundle {
    fn new(st: &AppState) -> std::io::Result<Self> {
        let dir = st.data_dir.join("bundles");
        std::fs::create_dir_all(&dir)?;
        Ok(TempBundle(dir.join(format!("{}.bundle", uuid::Uuid::now_v7().simple()))))
    }
}

impl Drop for TempBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Answers with the bundle of `req.branch` in `repo` (204 when the other end has its head already).
async fn serve(st: &AppState, repo: PathBuf, req: BundleReq) -> Result<Response, ApiError> {
    pdt_core::worktree::NewWorktree { branch: req.branch.clone(), base: None }
        .validate()
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let tmp = TempBundle::new(st)?;
    let path = tmp.0.clone();
    let file = tokio::task::spawn_blocking(move || -> Result<Option<std::fs::File>, ApiError> {
        if !pdt_git::create_bundle(&repo, &path, &req.branch, &req.have)? {
            return Ok(None);
        }
        Ok(Some(std::fs::File::open(&path)?))
    })
    .await
    .map_err(ApiError::internal)??;
    // Open, the file outlives its name (on Windows too: Rust opens files to be deleted while open).
    drop(tmp);
    let Some(file) = file else { return Ok(StatusCode::NO_CONTENT.into_response()) };
    let len = file.metadata()?.len();
    let stream = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(file));
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_LENGTH, len.to_string())],
        Body::from_stream(stream),
    )
        .into_response())
}

/// Writes a bundle the other end sent to a file of this instance's; None when it sent none (nothing new).
async fn receive(st: &AppState, mut res: reqwest::Response) -> Result<Option<TempBundle>, String> {
    use tokio::io::AsyncWriteExt;
    if res.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let tmp = TempBundle::new(st).map_err(|e| e.to_string())?;
    let mut file = tokio::fs::File::create(&tmp.0).await.map_err(|e| e.to_string())?;
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("the bundle didn't arrive whole: {e}"))? {
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(Some(tmp))
}

// ---- The owner's end ----

/// `/link/exec/git/base`: a remote instance asks for the project's base branch (any branch of the owner's checkout).
pub(crate) async fn base_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let i = link::from_remote(&st, &method, &uri, &headers, &body).await?;
    let req: BundleReq = serde_json::from_slice(&body).map_err(|e| ApiError::invalid(e.to_string()))?;
    let project = st.store.get_project(&i.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    serve(&st, PathBuf::from(project.repo_path), req).await
}

/// Brings `branch` of a remote attempt on `i` into the owner's checkout at `repo`, as it is there (forced: the agent
/// may rewrite it): its head, None when it has none there. Err when the instance can't be asked (the owner's branch
/// stays as it was).
pub(crate) async fn pull_branch(
    st: &AppState,
    i: &RemoteInstance,
    repo: &Path,
    branch: &str,
    base: &str,
) -> Result<Option<String>, String> {
    let Some(head) = branch_head(st, i, branch).await? else { return Ok(None) };
    let (r, b, h) = (repo.to_path_buf(), branch.to_string(), head.clone());
    let (here, known) = tokio::task::spawn_blocking(move || {
        (pdt_git::rev_parse(&r, &format!("refs/heads/{b}")), pdt_git::has_commit(&r, &h))
    })
    .await
    .map_err(|e| e.to_string())?;
    if here.as_deref() == Some(head.as_str()) {
        return Ok(Some(head));
    }
    if known {
        // A head the owner has (the branch went back, or was merged here): only the branch moves.
        let (r, b, h) = (repo.to_path_buf(), branch.to_string(), head.clone());
        tokio::task::spawn_blocking(move || pdt_git::set_branch(&r, &b, &h))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        return Ok(Some(head));
    }
    let base_tip = {
        let (r, b) = (repo.to_path_buf(), base.to_string());
        tokio::task::spawn_blocking(move || pdt_git::rev_parse(&r, &format!("refs/heads/{b}")))
            .await
            .map_err(|e| e.to_string())?
    };
    let req = BundleReq { branch: branch.into(), have: base_tip.into_iter().chain(here).collect() };
    let res = send(st, i, BRANCH_PATH, &req, BUNDLE_TIMEOUT).await.map_err(|e| format!("{}: {}", i.name, e.text()))?;
    let Some(bundle) = receive(st, res).await? else {
        return Err(format!("{} sent no commits for {branch}", i.name));
    };
    let (r, b) = (repo.to_path_buf(), branch.to_string());
    tokio::task::spawn_blocking(move || {
        pdt_git::fetch_bundle(&r, &bundle.0, &b, &b, true)?;
        Ok::<_, pdt_git::GitError>(pdt_git::rev_parse(&r, &format!("refs/heads/{b}")))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

// ---- The remote instance's end ----

/// `/link/exec/git/branch`: the owner asks for a branch of this instance's checkout of the project.
pub(crate) async fn branch_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (link, req): (OwnerLink, BundleReq) = checked(&st, &method, &uri, &headers, &body).await?;
    serve(&st, super::checkout_dir(&st, &link), req).await
}

/// Brings `base` in the remote project's checkout at `repo` up to the owner's, making the checkout from it first when
/// there is none: for a checkout with no `origin` to fetch it from. Only a fast-forward: a base with commits of its own
/// there stays as it is (Err).
pub(super) async fn sync_base(st: &AppState, link: &OwnerLink, repo: &Path, base: &str) -> Result<(), ApiError> {
    let (r, b) = (repo.to_path_buf(), base.to_string());
    let (exists, tip) = tokio::task::spawn_blocking(move || {
        let exists = r.join(".git").exists();
        (exists, exists.then(|| pdt_git::rev_parse(&r, &format!("refs/heads/{b}"))).flatten())
    })
    .await
    .map_err(ApiError::internal)?;
    let key = link::load_key(st, &link.link_id).await?;
    let req = BundleReq { branch: base.into(), have: tip.into_iter().collect() };
    let signed = Some((&key, link.link_id.as_str(), link.owner_project_id.as_str()));
    let res = link::send(st, BUNDLE_TIMEOUT, &link.owner_url, BASE_PATH, signed, &req)
        .await
        .map_err(|e: PeerError| e.into_api(&link.owner_name))?;
    let Some(bundle) = receive(st, res).await.map_err(ApiError::internal)? else { return Ok(()) };
    let (r, b) = (repo.to_path_buf(), base.to_string());
    tokio::task::spawn_blocking(move || {
        if exists {
            pdt_git::fetch_bundle(&r, &bundle.0, &b, &b, false)
        } else {
            pdt_git::init_from_bundle(&r, &bundle.0, &b)
        }
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(())
}
