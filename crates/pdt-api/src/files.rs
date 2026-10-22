//! The read-only folder view (workspaces spec §6.5): a checkout's files as git sees them (`.gitignore` honoured)
//! and a file's text. Paths never leave the checkout, symlinks included, and `.git` is off limits.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use ts_rs::TS;

/// Files larger than this are not sent.
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
}

fn outside() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "path_outside", "that path is outside the checkout")
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
    let project = st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let checkout = match q.worktree_id.as_deref().filter(|w| !w.is_empty()) {
        Some(wid) => {
            let wt = st.store.get_worktree(wid).await?.filter(|w| w.project_id == project.id);
            PathBuf::from(wt.ok_or_else(|| ApiError::not_found("worktree"))?.path)
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
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "name a file"));
    }
    let file = tokio::task::spawn_blocking(move || -> Result<FileContent, ApiError> {
        let full = contained(&checkout, &rel)?;
        let meta = std::fs::metadata(&full).ok().filter(|m| m.is_file()).ok_or_else(|| ApiError::not_found("file"))?;
        let status = if personal {
            None
        } else {
            pdt_git::status(&checkout, &rel)?.into_iter().find(|(p, _)| *p == rel).map(|(_, c)| c)
        };
        let size = meta.len();
        if size > MAX_FILE_BYTES {
            return Ok(FileContent { path: rel, size, status, text: None, binary: false, too_large: true });
        }
        let bytes = std::fs::read(&full).map_err(ApiError::internal)?;
        let binary = bytes.iter().take(8000).any(|b| *b == 0);
        let text = if binary { None } else { String::from_utf8(bytes).ok() };
        Ok(FileContent { path: rel, size, status, binary: text.is_none(), text, too_large: false })
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(file))
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
