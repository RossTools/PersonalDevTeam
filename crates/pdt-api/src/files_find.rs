//! Find files by name for the command palette (PDT-586): a checkout's files as git lists them (`.gitignore`
//! honoured), ranked for the query like the composer's `@` mentions. Each checkout's list is kept for a few seconds,
//! so typing doesn't run `git ls-files` on every keystroke.

use crate::error::ApiError;
use crate::files::{FilesQuery, resolve};
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use pdt_core::conversation::composer::rank_paths;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Paths sent at most.
pub const MAX_FOUND: usize = 20;
/// How long a checkout's file list is reused.
const TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FindQuery {
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    q: String,
}

/// A checkout's files, as `git ls-files` listed them.
type Files = Arc<Vec<String>>;

/// Each checkout's file list, with when it was read.
#[derive(Default)]
struct FileLists {
    lists: Mutex<HashMap<PathBuf, (Instant, Files)>>,
}

impl FileLists {
    /// The checkout's files: the kept list while it is younger than `ttl`, else `load`'s (kept when it succeeds).
    /// Lists past their time are dropped. Blocking (`load` runs git).
    fn get(
        &self,
        checkout: &FsPath,
        now: Instant,
        ttl: Duration,
        load: impl FnOnce() -> Result<Vec<String>, ApiError>,
    ) -> Result<Files, ApiError> {
        {
            let mut lists = self.lists.lock().unwrap_or_else(|e| e.into_inner());
            lists.retain(|_, (at, _)| now.saturating_duration_since(*at) < ttl);
            if let Some((_, files)) = lists.get(checkout) {
                return Ok(files.clone());
            }
        }
        let files = Arc::new(load()?);
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).insert(checkout.to_path_buf(), (now, files.clone()));
        Ok(files)
    }
}

static LISTS: LazyLock<FileLists> = LazyLock::new(FileLists::default);

/// `GET /projects/{id}/files/find?q=&worktreeId=`: the checkout's files best matching `q` by name, best first (at most
/// 20). A Personal project's plain folder, or an empty query, finds nothing.
pub(crate) async fn find(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FindQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let fq = FilesQuery { worktree_id: q.worktree_id, path: String::new() };
    let (checkout, _, plain) = resolve(&st, &id, &fq).await?;
    let query = q.q.trim().to_string();
    if plain || query.is_empty() {
        return Ok(Json(vec![]));
    }
    let found = tokio::task::spawn_blocking(move || -> Result<Vec<String>, ApiError> {
        let files = LISTS.get(&checkout, Instant::now(), TTL, || Ok(pdt_git::list_files(&checkout, "")?))?;
        Ok(rank_paths(&query, &files, MAX_FOUND).into_iter().map(String::from).collect())
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(found))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    axum::Router::new().route("/projects/{id}/files/find", axum::routing::get(find))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkouts_list_is_kept_for_a_while() {
        let lists = FileLists::default();
        let (a, b) = (FsPath::new("/a"), FsPath::new("/b"));
        let t0 = Instant::now();
        let ttl = Duration::from_secs(5);
        let loads = std::cell::Cell::new(0);
        let load = |name: &str| {
            loads.set(loads.get() + 1);
            Ok(vec![format!("{name}{}", loads.get())])
        };
        assert_eq!(*lists.get(a, t0, ttl, || load("a")).unwrap(), ["a1"]);
        assert_eq!(*lists.get(a, t0 + Duration::from_secs(4), ttl, || load("a")).unwrap(), ["a1"], "kept");
        assert_eq!(*lists.get(b, t0 + Duration::from_secs(4), ttl, || load("b")).unwrap(), ["b2"], "per checkout");
        assert_eq!(*lists.get(a, t0 + Duration::from_secs(5), ttl, || load("a")).unwrap(), ["a3"], "read again");
        assert_eq!(loads.get(), 3);
        assert!(lists.get(b, t0 + Duration::from_secs(20), ttl, || Err(ApiError::internal("git"))).is_err());
        assert_eq!(lists.lists.lock().unwrap().len(), 0, "old lists and failures aren't kept");
    }
}
