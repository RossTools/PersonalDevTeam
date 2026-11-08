//! The folder picker's view of the host (PDT-5): one folder's subfolders, never files, so the browser can pick a
//! path on the host without typing it. Paths are in the host's own form (`C:\…` on Windows), whatever the browser's
//! OS. Like every route it is owner-only (the identity guard); it reads only what the daemon's user can already read.

use crate::error::{ApiError, codes};
use crate::extract::ApiQuery;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::host_path::{self, Crumb, PathStyle};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// More subfolders than this are cut off (`truncated`); typing a name narrows the list instead.
pub const MAX_FOLDERS: usize = 2000;

const STYLE: PathStyle = PathStyle::host();

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FoldersQuery {
    /// Absolute, or `~`/`~/…`; empty means the home folder.
    #[serde(default)]
    path: String,
    /// Include hidden folders (names starting with `.`; on Windows also the hidden attribute).
    #[serde(default)]
    hidden: bool,
    /// When `path` does not exist, list the nearest folder above it that does (opening the picker on a typed path).
    #[serde(default)]
    nearest: bool,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderListing {
    /// The folder listed, absolute, in the host's form.
    pub path: String,
    /// `path` with the home folder written `~`.
    pub display: String,
    pub parent: Option<String>,
    /// The host's separator (`/` or `\`).
    pub sep: String,
    /// From `~` when inside the home folder, else from the root.
    pub crumbs: Vec<Crumb>,
    /// Places to jump to: Home, the Projects folder, the roots (`/`, or each drive on Windows).
    pub places: Vec<Crumb>,
    pub folders: Vec<FolderEntry>,
    /// More than [`MAX_FOLDERS`] subfolders; the rest are left out.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    /// `path` with the home folder written `~`.
    pub display: String,
    pub hidden: bool,
    /// A git checkout (it has a `.git` folder or file).
    pub git: bool,
    /// The PDT project whose repository this is.
    pub project: Option<FolderProject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderProject {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewFolder {
    /// The folder to create it in (absolute or `~/…`).
    pub parent: String,
    pub name: String,
}

fn home(st: &AppState) -> Option<String> {
    st.home_dir.get().and_then(|h| host_path::normalize(&h.to_string_lossy(), STYLE))
}

/// The absolute folder `input` names (`~` expanded; empty is the home folder).
fn resolve(input: &str, home: Option<&str>) -> Result<String, ApiError> {
    let input = if input.trim().is_empty() { "~" } else { input };
    host_path::expand(input, home, STYLE).ok_or_else(|| match input.trim().starts_with('~') && home.is_none() {
        true => ApiError::bad_request(codes::INVALID_FOLDER, "the home folder is not known; enter the full path"),
        false => ApiError::bad_request(
            codes::INVALID_FOLDER,
            format!("{:?} is not a full path; start it with {} (or ~ for your home folder)", input.trim(), root_hint()),
        ),
    })
}

fn root_hint() -> &'static str {
    if cfg!(windows) { r"a drive (C:\)" } else { "/" }
}

/// Reads an io error on `display` as an API error.
fn io_error(e: &std::io::Error, display: &str) -> ApiError {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => ApiError::new(StatusCode::NOT_FOUND, codes::FOLDER_MISSING, format!("{display} does not exist")),
        NotADirectory => ApiError::bad_request(codes::NOT_A_FOLDER, format!("{display} is not a folder")),
        PermissionDenied => {
            ApiError::forbidden(codes::PERMISSION_DENIED, format!("{display} cannot be read: permission denied"))
        }
        _ => ApiError::bad_request(codes::FOLDER_UNREADABLE, format!("{display} cannot be read: {e}")),
    }
}

/// `dir/name` in the host's form.
fn child(dir: &str, name: &str) -> String {
    match dir.ends_with(STYLE.sep()) {
        true => format!("{dir}{name}"),
        false => format!("{dir}{}{name}", STYLE.sep()),
    }
}

#[cfg(windows)]
fn hidden_attr(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn hidden_attr(_: &std::fs::Metadata) -> bool {
    false
}

/// The subfolders of `dir` (following symlinks to folders): name and whether it is hidden, sorted by name ignoring
/// case. Names that are not valid UTF-8 are skipped (their paths could not round-trip through the browser).
fn read_folders(dir: &Path, with_hidden: bool) -> std::io::Result<(Vec<(String, bool)>, bool)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        let is_dir = ft.is_dir() || (ft.is_symlink() && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()));
        if !is_dir {
            continue;
        }
        let hidden = name.starts_with('.') || entry.metadata().is_ok_and(|m| hidden_attr(&m));
        if hidden && !with_hidden {
            continue;
        }
        out.push((name, hidden));
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    let truncated = out.len() > MAX_FOLDERS;
    out.truncate(MAX_FOLDERS);
    Ok((out, truncated))
}

/// `/`, or each drive that exists on Windows.
fn roots() -> Vec<Crumb> {
    if cfg!(windows) {
        (b'C'..=b'Z')
            .map(|d| format!("{}:\\", d as char))
            .filter(|r| Path::new(r).is_dir())
            .map(|r| Crumb { name: r.trim_end_matches('\\').to_string(), path: r })
            .collect()
    } else {
        vec![Crumb { name: "/".into(), path: "/".into() }]
    }
}

fn entry(
    st_projects: &[pdt_core::project::Project],
    dir: &str,
    name: String,
    hidden: bool,
    home: Option<&str>,
) -> FolderEntry {
    let path = child(dir, &name);
    let fs_path = PathBuf::from(&path);
    let git = fs_path.join(".git").exists();
    // Git reports a repository's real path (symlinks resolved), so a checkout reached through a link is matched by
    // its real path too. Only git checkouts can be projects, which keeps this to one lookup per repository.
    let real = git.then(|| std::fs::canonicalize(&fs_path).ok()).flatten().map(|p| p.to_string_lossy().into_owned());
    let project = st_projects
        .iter()
        .filter(|_| git)
        .find(|p| {
            host_path::same_path(&p.repo_path, &path, STYLE)
                || real.as_deref().is_some_and(|r| host_path::same_path(&p.repo_path, r, STYLE))
        })
        .map(|p| FolderProject { id: p.id.clone(), name: p.name.clone(), key: p.key.clone() });
    FolderEntry { display: host_path::tilde(&path, home, STYLE), git, name, path, hidden, project }
}

pub(crate) async fn list(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<FoldersQuery>,
) -> Result<Json<FolderListing>, ApiError> {
    let home = home(&st);
    let mut path = resolve(&q.path, home.as_deref())?;
    let projects = st.store.list_projects().await?;
    let projects_dir = crate::projects::projects_dir(&st, st.store.get_host_settings().await?.projects_dir.as_deref())
        .and_then(|p| host_path::normalize(&p.to_string_lossy(), STYLE));
    let listing = tokio::task::spawn_blocking(move || -> Result<FolderListing, ApiError> {
        if q.nearest {
            while !Path::new(&path).is_dir() {
                match host_path::parent(&path, STYLE) {
                    Some(p) => path = p,
                    None => break,
                }
            }
        }
        let display = host_path::tilde(&path, home.as_deref(), STYLE);
        let (names, truncated) = read_folders(Path::new(&path), q.hidden).map_err(|e| io_error(&e, &display))?;
        let folders = names.into_iter().map(|(n, h)| entry(&projects, &path, n, h, home.as_deref())).collect();
        let mut places = Vec::new();
        if let Some(h) = &home {
            places.push(Crumb { name: "Home".into(), path: h.clone() });
        }
        if let Some(p) = projects_dir.filter(|p| Path::new(p).is_dir()) {
            places.push(Crumb { name: "Projects folder".into(), path: p });
        }
        places.extend(roots());
        Ok(FolderListing {
            parent: host_path::parent(&path, STYLE),
            sep: STYLE.sep().to_string(),
            crumbs: host_path::crumbs(&path, home.as_deref(), STYLE),
            places,
            folders,
            truncated,
            display,
            path,
        })
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(listing))
}

/// Makes one new folder inside an existing one and returns it.
pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewFolder>, JsonRejection>,
) -> Result<(StatusCode, Json<FolderEntry>), ApiError> {
    let Json(input) = payload?;
    let home = home(&st);
    let parent = resolve(&input.parent, home.as_deref())?;
    let name = host_path::folder_name(&input.name, STYLE)
        .map_err(|e| ApiError::bad_request(codes::INVALID_FOLDER, e.to_string()))?;
    let projects = st.store.list_projects().await?;
    let made = tokio::task::spawn_blocking(move || -> Result<FolderEntry, ApiError> {
        let parent_display = host_path::tilde(&parent, home.as_deref(), STYLE);
        if !Path::new(&parent).is_dir() {
            let e = std::io::Error::from(match Path::new(&parent).exists() {
                true => std::io::ErrorKind::NotADirectory,
                false => std::io::ErrorKind::NotFound,
            });
            return Err(io_error(&e, &parent_display));
        }
        let path = child(&parent, &name);
        let display = host_path::tilde(&path, home.as_deref(), STYLE);
        std::fs::create_dir(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                ApiError::conflict(codes::FOLDER_EXISTS, format!("{display} already exists"))
            }
            _ => io_error(&e, &display),
        })?;
        let hidden = name.starts_with('.');
        Ok(entry(&projects, &parent, name, hidden, home.as_deref()))
    })
    .await
    .map_err(ApiError::internal)??;
    Ok((StatusCode::CREATED, Json(made)))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/host/folders", get(list).post(create))
}
