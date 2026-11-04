//! Search inside files for the Files view (PDT-587): `git grep` over the checkout being browsed (`--no-index` in a
//! Personal project's plain folder), with caps on what comes back and a deadline, so a search over a large checkout
//! never holds a worker for long.

use crate::error::ApiError;
use crate::files::{FilesQuery, resolve};
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::path::Path as FsPath;
use std::time::Duration;
use ts_rs::TS;

/// Matching lines sent in all.
pub const MAX_MATCHES: usize = 2000;
/// Matching lines sent per file.
pub const MAX_PER_FILE: usize = 50;
/// After this, git is stopped and what it found is sent.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Longer lines are cut around the match.
const MAX_LINE_CHARS: usize = 300;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchQuery {
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    case: bool,
    #[serde(default)]
    word: bool,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    glob: String,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchLine {
    /// 1-based.
    pub line: u32,
    /// The line, or the part of it around the first match when it is long.
    pub text: String,
    /// Where `text` starts in the line (UTF-16 units): above 0 when its start was cut.
    pub offset: u32,
    /// The end of the line was cut.
    pub cut_end: bool,
    /// The matches in `text`, `[start, end)` in UTF-16 units.
    pub ranges: Vec<[u32; 2]>,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchFile {
    /// Relative to the checkout, `/`-separated.
    pub path: String,
    /// The matching lines sent (at most 50).
    pub lines: Vec<SearchLine>,
    /// Matching lines in the file (as far as the search got), sent or not.
    pub count: u32,
}

/// `GET /projects/{id}/files/search`: the files whose contents match, in path order.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub files: Vec<SearchFile>,
    /// A cap was reached: only the first matches are here.
    pub capped: bool,
    /// The search ran out of time: only what it found by then is here.
    pub timed_out: bool,
}

/// Runs the search in `checkout`. Blocking.
fn search_in(checkout: &FsPath, q: SearchQuery, plain: bool, timeout: Duration) -> Result<SearchResult, ApiError> {
    let r = pdt_git::grep(
        checkout,
        &pdt_git::GrepQuery {
            pattern: q.q,
            match_case: q.case,
            whole_word: q.word,
            regex: q.regex,
            glob: q.glob,
            no_index: plain,
            max_matches: MAX_MATCHES,
            max_per_file: MAX_PER_FILE,
            timeout,
            max_line_chars: MAX_LINE_CHARS,
        },
    )?;
    let files = r
        .files
        .into_iter()
        .map(|f| SearchFile {
            path: f.path,
            count: f.count as u32,
            lines: f
                .lines
                .into_iter()
                .map(|l| SearchLine {
                    line: l.line,
                    text: l.text,
                    offset: l.offset,
                    cut_end: l.cut_end,
                    ranges: l.ranges.into_iter().map(|(s, e)| [s, e]).collect(),
                })
                .collect(),
        })
        .collect();
    Ok(SearchResult { files, capped: r.capped, timed_out: r.timed_out })
}

pub(crate) async fn search(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResult>, ApiError> {
    let fq = FilesQuery { worktree_id: q.worktree_id.clone(), path: String::new() };
    let (checkout, _, plain) = resolve(&st, &id, &fq).await?;
    let r = tokio::task::spawn_blocking(move || search_in(&checkout, q, plain, TIMEOUT))
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(r))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    axum::Router::new().route("/projects/{id}/files/search", axum::routing::get(search))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_out_of_time_sends_what_it_has() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "needle\n").unwrap();
        let q = SearchQuery { q: "needle".into(), ..Default::default() };
        let r = search_in(tmp.path(), q, true, Duration::ZERO).unwrap();
        assert!(r.timed_out);
        let q = SearchQuery { q: "needle".into(), ..Default::default() };
        let r = search_in(tmp.path(), q, true, TIMEOUT).unwrap();
        assert!(!r.timed_out);
        assert_eq!(r.files[0].path, "a.txt");
        assert_eq!(r.files[0].lines[0].ranges, [[0, 6]]);
    }
}
