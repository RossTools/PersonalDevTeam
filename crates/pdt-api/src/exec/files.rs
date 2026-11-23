//! The Files view on a remote instance (PDT-635): the owner sends each request as one call (`/link/exec/files`, or
//! `/link/exec/download`, whose answer streams through), and the remote instance runs it with the same functions and
//! checks as a local one, in a folder it picks itself: the remote project's checkout, or a worktree of it named by
//! branch. Nothing the owner sends is a path on the remote instance's disk.

use super::owner::call;
use super::remote::{checked, checkout_dir};
use crate::error::{ApiError, codes};
use crate::files::{self, blocking, clean_path};
use crate::files_download::Download;
use crate::files_search::SearchQuery;
use crate::link;
use crate::routes::AppState;
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pdt_core::link::{OwnerLink, RemoteInstance};
use pdt_core::worktree::worktree_path;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long one Files call may take (a search stops itself after 10 s).
const FILES_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a download may take to stream through, end to end.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
/// The largest Files request the remote instance takes (a save holds at most 1 MB of text, escaped in JSON).
pub(super) const MAX_FILES_BODY: usize = 8 * 1024 * 1024;

/// A request of the Files view, as it crosses the link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum FilesOp {
    List { path: String },
    Content { path: String },
    Save { path: String, text: String, base_hash: Option<String> },
    Rename { from: String, to: String },
    Delete { path: String },
    Find { q: String },
    Search { q: SearchQuery },
}

/// `/link/exec/files`: `op` in the worktree of `branch`, or (None) in the remote project's checkout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FilesReq {
    branch: Option<String>,
    op: FilesOp,
}

/// `/link/exec/download`: `path` there, downloaded as `how` says.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DownloadReq {
    branch: Option<String>,
    path: String,
    how: Download,
}

/// Runs `op` on the remote instance `i`: its answer, or its error as it gave it.
pub(crate) async fn files<T: DeserializeOwned>(
    st: &AppState,
    i: &RemoteInstance,
    branch: Option<String>,
    op: FilesOp,
) -> Result<T, ApiError> {
    call(st, i, super::FILES_PATH, &FilesReq { branch, op }, FILES_TIMEOUT).await.map_err(|e| e.passed_on(&i.name))
}

/// Downloads `path` from the remote instance `i`, streamed through as it comes.
pub(crate) async fn download(
    st: &AppState,
    i: &RemoteInstance,
    branch: Option<String>,
    path: String,
    how: Download,
) -> Result<Response, ApiError> {
    let key = link::load_key(st, &i.link_id).await?;
    let req = DownloadReq { branch, path, how };
    let signed = Some((&key, i.link_id.as_str(), i.project_id.as_str()));
    let res = link::send(st, DOWNLOAD_TIMEOUT, &i.url, super::DOWNLOAD_PATH, signed, &req)
        .await
        .map_err(|e| e.passed_on(&i.name))?;
    if res.status() == StatusCode::NO_CONTENT {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let text = |name: header::HeaderName, or: &str| {
        res.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or(or).to_string()
    };
    let media_type = text(header::CONTENT_TYPE, "application/octet-stream");
    let disposition = text(header::CONTENT_DISPOSITION, "attachment");
    let len = res.content_length();
    Ok(crate::files_download::with_headers(&media_type, &disposition, len, Body::from_stream(res.bytes_stream())))
}

/// The folder a request from the owner on `link` names: the remote project's checkout, or its worktree of `branch`.
/// A branch name that could lead elsewhere, or a worktree folder that is a link or resolves outside the checkout, is
/// refused. Blocking.
pub(super) fn place(st: &AppState, link: &OwnerLink, branch: Option<&str>) -> Result<PathBuf, ApiError> {
    let repo = checkout_dir(st, link);
    let Some(branch) = branch else {
        if !repo.join(".git").exists() {
            return Err(ApiError::conflict(codes::WORKTREE_MISSING, "the project isn't checked out here yet"));
        }
        return Ok(repo);
    };
    pdt_core::worktree::NewWorktree { branch: branch.to_string(), base: None }
        .validate()
        .map_err(|e| ApiError::invalid(e.to_string()))?;
    let path = worktree_path(&repo, branch);
    pdt_git::check_worktree_path(&repo, &path)?;
    if !path.is_dir() {
        return Err(crate::review::missing());
    }
    Ok(path)
}

/// Runs `op` in `checkout` (a remote project's: never a Personal project's plain folder). Blocking.
fn run(checkout: &Path, op: FilesOp) -> Result<Value, ApiError> {
    fn json(v: Result<impl Serialize, ApiError>) -> Result<Value, ApiError> {
        serde_json::to_value(v?).map_err(ApiError::internal)
    }
    match op {
        FilesOp::List { path } => json(files::list_in(checkout, &clean_path(&path)?, false)),
        FilesOp::Content { path } => json(files::content_in(checkout, clean_path(&path)?, false)),
        FilesOp::Save { path, text, base_hash } => {
            json(files::save_in(checkout, path, &text, base_hash.as_deref(), false))
        }
        FilesOp::Rename { from, to } => files::rename_in(checkout, &from, &to).map(|()| Value::Null),
        FilesOp::Delete { path } => files::delete_in(checkout, &path).map(|()| Value::Null),
        FilesOp::Find { q } => json(crate::files_find::find_in(checkout, q.trim())),
        FilesOp::Search { q } => json(crate::files_search::search_here(checkout, q)),
    }
}

/// `/link/exec/files`, on the remote instance.
pub(crate) async fn files_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let (link, req): (OwnerLink, FilesReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let st2 = st.clone();
    let v = blocking(move || run(&place(&st2, &link, req.branch.as_deref())?, req.op)).await?;
    Ok(Json(v))
}

/// `/link/exec/download`, on the remote instance: the download as a local one answers it.
pub(crate) async fn download_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (link, req): (OwnerLink, DownloadReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let rel = clean_path(&req.path)?;
    let st2 = st.clone();
    let branch = req.branch.clone();
    let checkout = blocking(move || place(&st2, &link, branch.as_deref())).await?;
    crate::files_download::respond(checkout, rel, false, req.how).await
}
