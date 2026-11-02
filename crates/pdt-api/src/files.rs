//! The Files view (workspaces spec §6.5, editing PDT-157): a checkout's files as git sees them (`.gitignore`
//! honoured), a file's text, and saving, creating, renaming and deleting files. Paths never leave the checkout,
//! symlinks included, and `.git` is off limits. A save names the hash of the text it started from, so a file that
//! changed on disk since is never overwritten silently.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use ts_rs::TS;

/// Files larger than this are not sent, nor saved.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilesQuery {
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileEntry {
    pub name: String,
    /// Relative to the checkout, `/`-separated.
    pub path: String,
    pub dir: bool,
    /// Git status (`M`, `A`, `D`, `??`, ...); for a folder, `M` when anything inside changed.
    pub status: Option<String>,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileContent {
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub status: Option<String>,
    /// Set when the file is shown: not binary and at most 1 MB.
    pub text: Option<String>,
    pub binary: bool,
    pub too_large: bool,
    /// The SHA-256 of the file's bytes (hex), when they were read: a save sends it back as `baseHash`.
    pub hash: Option<String>,
}

/// `PUT /projects/{id}/files/content`: writes `text` to `path`. `baseHash` is the hash of the file as it was opened;
/// leave it out to create a file that must not exist yet.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveFile {
    #[serde(default)]
    #[ts(optional)]
    pub worktree_id: Option<String>,
    pub path: String,
    pub text: String,
    #[serde(default)]
    #[ts(optional)]
    pub base_hash: Option<String>,
}

/// `POST /projects/{id}/files/rename`: moves a file or folder to a path that is free.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameFile {
    #[serde(default)]
    #[ts(optional)]
    pub worktree_id: Option<String>,
    pub from: String,
    pub to: String,
}

/// `POST /projects/{id}/files/tell`: tells an agent session working in the checkout that the user changed files.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TellFilesChanged {
    pub session_id: String,
    pub paths: Vec<String>,
}

fn outside() -> ApiError {
    ApiError::bad_request(codes::PATH_OUTSIDE, "that path is outside the checkout")
}

/// `raw` as clean `/`-separated segments, refusing anything that could leave the checkout or reach `.git`.
pub(crate) fn clean_path(raw: &str) -> Result<String, ApiError> {
    let mut parts = Vec::new();
    for seg in raw.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return Err(outside()),
            s if s.contains(':') || s.contains('\0') => return Err(outside()),
            s => parts.push(s),
        }
    }
    if parts.first().is_some_and(|s| s.eq_ignore_ascii_case(".git")) {
        return Err(outside());
    }
    Ok(parts.join("/"))
}

/// A path to write, as `clean_path` takes it but with no `.git` anywhere in it (a submodule's or a nested repo's),
/// and naming something.
fn clean_write_path(raw: &str) -> Result<String, ApiError> {
    let rel = clean_path(raw)?;
    if rel.is_empty() {
        return Err(ApiError::invalid("name a file"));
    }
    // Windows drops trailing dots and spaces from names, so `.git.` is `.git` there.
    if rel.split('/').any(|seg| seg.trim_end_matches(['.', ' ']).eq_ignore_ascii_case(".git")) {
        return Err(outside());
    }
    Ok(rel)
}

fn hash_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// The entries directly in a plain folder (a Personal project's), as `entries` takes them: a folder as `<dir>/.`.
/// Blocking.
fn plain_files(full: &FsPath, rel: &str) -> Vec<String> {
    let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
    let Ok(read) = std::fs::read_dir(full) else { return vec![] };
    let mut out: Vec<String> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.file_type().ok()?.is_dir();
            Some(if dir { format!("{prefix}{name}/.") } else { format!("{prefix}{name}") })
        })
        .collect();
    out.sort();
    out
}

/// The checkout (project root or one of its worktrees), the cleaned path inside it, and whether it is a Personal
/// project's plain folder (no git).
async fn resolve(st: &AppState, project_id: &str, q: &FilesQuery) -> Result<(PathBuf, String, bool), ApiError> {
    let project = st.project(project_id).await?;
    let checkout = match q.worktree_id.as_deref().filter(|w| !w.is_empty()) {
        Some(wid) => {
            let wt = st.store.get_worktree(wid).await?.filter(|w| w.project_id == project.id);
            PathBuf::from(wt.or_404("worktree")?.path)
        }
        None => PathBuf::from(&project.repo_path),
    };
    Ok((checkout, clean_path(&q.path)?, !project.has_repo()))
}

/// Refuses a path that resolves (through symlinks) outside the checkout. Missing paths pass: git may list a deleted
/// file. Blocking.
pub(crate) fn contained(checkout: &FsPath, rel: &str) -> Result<PathBuf, ApiError> {
    let root = std::fs::canonicalize(checkout).map_err(|_| ApiError::not_found("checkout"))?;
    let full = checkout.join(rel);
    match std::fs::canonicalize(&full) {
        Ok(real) if !real.starts_with(&root) => Err(outside()),
        _ => Ok(full),
    }
}

/// Like [`contained`], for a path that may not exist yet: its nearest existing folder must resolve inside the
/// checkout too (so a symlinked folder can't lead out). Blocking.
fn writable(checkout: &FsPath, rel: &str) -> Result<PathBuf, ApiError> {
    let root = std::fs::canonicalize(checkout).map_err(|_| ApiError::not_found("checkout"))?;
    let full = contained(checkout, rel)?;
    let mut dir = full.parent();
    while let Some(d) = dir {
        if let Ok(real) = std::fs::canonicalize(d) {
            if !real.starts_with(&root) {
                return Err(outside());
            }
            break;
        }
        dir = d.parent();
    }
    Ok(full)
}

fn file_changed() -> ApiError {
    ApiError::conflict(codes::FILE_CHANGED, "the file changed on disk since it was opened")
}

/// The file as the view shows it. Blocking.
fn read_file(checkout: &FsPath, full: &FsPath, rel: String, personal: bool) -> Result<FileContent, ApiError> {
    let meta = std::fs::metadata(full).ok().filter(|m| m.is_file()).or_404("file")?;
    let status = if personal {
        None
    } else {
        // Asked of the file's folder: git's status takes folders.
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        pdt_git::status(checkout, dir)?.into_iter().find(|(p, _)| *p == rel).map(|(_, c)| c)
    };
    let size = meta.len();
    if size > MAX_FILE_BYTES {
        return Ok(FileContent { path: rel, size, status, text: None, binary: false, too_large: true, hash: None });
    }
    let bytes = std::fs::read(full)?;
    let hash = Some(hash_of(&bytes));
    let binary = bytes.iter().take(8000).any(|b| *b == 0);
    let text = if binary { None } else { String::from_utf8(bytes).ok() };
    Ok(FileContent { path: rel, size, status, binary: text.is_none(), text, too_large: false, hash })
}

/// The entries directly inside `dir`, from git's file list and status (folders first, then by name).
pub(crate) fn entries(dir: &str, files: &[String], status: &[(String, String)]) -> Vec<FileEntry> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    let mut out: BTreeMap<(bool, String), FileEntry> = BTreeMap::new();
    for f in files {
        let Some(rest) = f.strip_prefix(&prefix) else { continue };
        let (name, is_dir) = match rest.split_once('/') {
            Some((d, _)) => (d, true),
            None => (rest, false),
        };
        let path = format!("{prefix}{name}");
        out.entry((!is_dir, name.to_string())).or_insert_with(|| FileEntry {
            name: name.to_string(),
            path,
            dir: is_dir,
            status: None,
        });
    }
    for e in out.values_mut() {
        e.status = if e.dir {
            let inside = format!("{}/", e.path);
            status.iter().any(|(p, _)| p.starts_with(&inside)).then(|| "M".to_string())
        } else {
            status.iter().find(|(p, _)| *p == e.path).map(|(_, c)| c.clone())
        };
    }
    out.into_values().collect()
}

pub(crate) async fn list(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FilesQuery>,
) -> Result<Json<Vec<FileEntry>>, ApiError> {
    let (checkout, rel, personal) = resolve(&st, &id, &q).await?;
    let list = tokio::task::spawn_blocking(move || -> Result<Vec<FileEntry>, ApiError> {
        let full = contained(&checkout, &rel)?;
        if personal {
            return Ok(entries(&rel, &plain_files(&full, &rel), &[]));
        }
        let files = pdt_git::list_files(&checkout, &rel)?;
        let status = pdt_git::status(&checkout, &rel)?;
        Ok(entries(&rel, &files, &status))
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(list))
}

pub(crate) async fn content(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FilesQuery>,
) -> Result<Json<FileContent>, ApiError> {
    let (checkout, rel, personal) = resolve(&st, &id, &q).await?;
    if rel.is_empty() {
        return Err(ApiError::invalid("name a file"));
    }
    let file = tokio::task::spawn_blocking(move || -> Result<FileContent, ApiError> {
        let full = contained(&checkout, &rel)?;
        read_file(&checkout, &full, rel, personal)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(file))
}

/// Saves a file (or creates one, without `baseHash`). Refused with 409 `file_changed` when the file on disk is not
/// the one the editor started from, and `file_exists` when a new file's name is taken; binary and too-large files
/// stay read-only.
pub(crate) async fn save(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<SaveFile>,
) -> Result<Json<FileContent>, ApiError> {
    let rel = clean_write_path(&b.path)?;
    let q = FilesQuery { worktree_id: b.worktree_id, path: rel.clone() };
    let (checkout, rel, personal) = resolve(&st, &id, &q).await?;
    if b.text.len() as u64 > MAX_FILE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::TOO_LARGE,
            "files over 1 MB can't be edited here",
        ));
    }
    let file = tokio::task::spawn_blocking(move || -> Result<FileContent, ApiError> {
        let full = writable(&checkout, &rel)?;
        match std::fs::symlink_metadata(&full) {
            Ok(_) => {
                let meta = std::fs::metadata(&full)?;
                if meta.is_dir() {
                    return Err(ApiError::conflict(codes::FILE_EXISTS, "a folder has that name"));
                }
                let Some(base) = b.base_hash.as_deref() else {
                    return Err(ApiError::conflict(codes::FILE_EXISTS, "a file with that name exists"));
                };
                if meta.len() > MAX_FILE_BYTES {
                    return Err(not_editable());
                }
                let bytes = std::fs::read(&full)?;
                if bytes.iter().take(8000).any(|b| *b == 0) || std::str::from_utf8(&bytes).is_err() {
                    return Err(not_editable());
                }
                if hash_of(&bytes) != base {
                    return Err(file_changed());
                }
            }
            Err(_) if b.base_hash.is_some() => return Err(file_changed()),
            Err(_) => {
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        std::fs::write(&full, b.text.as_bytes())?;
        read_file(&checkout, &full, rel, personal)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(file))
}

fn not_editable() -> ApiError {
    ApiError::bad_request(codes::NOT_EDITABLE, "binary files and files over 1 MB can't be edited here")
}

/// Renames (moves) a file or folder inside the checkout; the new path must be free.
pub(crate) async fn rename(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<RenameFile>,
) -> Result<StatusCode, ApiError> {
    let from = clean_write_path(&b.from)?;
    let to = clean_write_path(&b.to)?;
    let q = FilesQuery { worktree_id: b.worktree_id, path: from.clone() };
    let (checkout, from, _) = resolve(&st, &id, &q).await?;
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        let src = writable(&checkout, &from)?;
        let dst = writable(&checkout, &to)?;
        if std::fs::symlink_metadata(&src).is_err() {
            return Err(ApiError::not_found("file"));
        }
        if std::fs::symlink_metadata(&dst).is_ok() {
            return Err(ApiError::conflict(codes::FILE_EXISTS, "something already has that name"));
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(ApiError::invalid("a folder can't move into itself"));
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).map_err(ApiError::from)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a file (not a folder).
pub(crate) async fn delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FilesQuery>,
) -> Result<StatusCode, ApiError> {
    clean_write_path(&q.path)?;
    let (checkout, rel, _) = resolve(&st, &id, &q).await?;
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        let full = writable(&checkout, &rel)?;
        let meta = std::fs::symlink_metadata(&full).map_err(|_| ApiError::not_found("file"))?;
        if meta.is_dir() {
            return Err(ApiError::invalid("only files can be deleted here"));
        }
        std::fs::remove_file(&full).map_err(ApiError::from)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(StatusCode::NO_CONTENT)
}

/// The message an agent gets when the user changed files under it.
fn tell_text(paths: &[String]) -> String {
    let list = paths.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ");
    let (these, them) = if paths.len() == 1 { ("this file", "it") } else { ("these files", "them") };
    format!(
        "[{}] The user edited {these} in the Files view: {list}. Read {them} again before you change {them}.",
        pdt_core::brand::SHORT_NAME
    )
}

/// Tells an agent session of the project which files the user changed, through the outbox (it waits for the agent
/// to be free, as `session.send` does).
pub(crate) async fn tell(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<TellFilesChanged>,
) -> Result<StatusCode, ApiError> {
    let s = st.store.get_session(&b.session_id).await?.filter(|s| s.project_id == id);
    let s = s.or_404("session")?;
    if !crate::pm::takes_messages(&s) {
        return Err(ApiError::conflict(codes::SESSION_ENDED, "that session has ended or is not an agent"));
    }
    let paths = b.paths.iter().map(|p| clean_write_path(p)).collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err(ApiError::invalid("name a file"));
    }
    let m = pdt_store::SessionMessage {
        id: uuid::Uuid::now_v7().to_string(),
        attempt_id: None,
        session_id: Some(s.id.clone()),
        ticket_id: s.ticket.as_ref().map(|t| t.id.clone()),
        dedupe: None,
        text: tell_text(&paths),
        timeline: serde_json::Value::Null,
        run_id: None,
        step_id: None,
        status: "waiting".into(),
        told_session: None,
        created_at: crate::routes::now_ms(),
        resolved_at: None,
        by_user: true,
        pending_id: None,
        reopens: false,
    };
    st.store.queue_session_message(&m).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/projects/{id}/files", get(list))
        .route("/projects/{id}/files/content", get(content).put(save).delete(self::delete))
        .route("/projects/{id}/files/rename", post(rename))
        .route("/projects/{id}/files/tell", post(tell))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_and_away_from_git() {
        assert_eq!(clean_path("").unwrap(), "");
        assert_eq!(clean_path("/src//main.rs").unwrap(), "src/main.rs");
        assert_eq!(clean_path("src\\lib\\x.rs").unwrap(), "src/lib/x.rs");
        assert_eq!(clean_path("./a/./b").unwrap(), "a/b");
        for bad in ["..", "a/../../b", "C:\\Windows", "c:/x", ".git/config", ".GIT", "a\0b"] {
            assert!(clean_path(bad).is_err(), "{bad}");
        }
        assert_eq!(clean_path("docs/.git-notes").unwrap(), "docs/.git-notes");
    }

    #[test]
    fn written_paths_never_reach_any_git_folder() {
        assert_eq!(clean_write_path("src/new.rs").unwrap(), "src/new.rs");
        assert_eq!(clean_write_path("docs/.gitignore").unwrap(), "docs/.gitignore");
        for bad in ["", "/", ".git/HEAD", "sub/.git/config", "sub/.GIT", "x/.git./y", "x/.git /y", "../x"] {
            assert!(clean_write_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_agent_is_told_which_files_changed() {
        assert_eq!(
            tell_text(&["a.rs".into()]),
            "[PDT] The user edited this file in the Files view: `a.rs`. Read it again before you change it."
        );
        assert!(
            tell_text(&["a.rs".into(), "b.rs".into()])
                .contains("these files in the Files view: `a.rs`, `b.rs`. Read them")
        );
    }

    #[test]
    fn entries_are_the_direct_children_with_status() {
        let files: Vec<String> =
            ["README.md", "src/main.rs", "src/deep/x.rs", "web/a.ts"].iter().map(|s| s.to_string()).collect();
        let status = vec![("src/deep/x.rs".to_string(), "??".to_string()), ("README.md".to_string(), "M".to_string())];
        let root = entries("", &files, &status);
        let names: Vec<(&str, bool, Option<&str>)> =
            root.iter().map(|e| (e.name.as_str(), e.dir, e.status.as_deref())).collect();
        assert_eq!(names, [("src", true, Some("M")), ("web", true, None), ("README.md", false, Some("M"))]);
        let src = entries("src", &files, &status);
        assert_eq!(src.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["src/deep", "src/main.rs"]);
    }
}
