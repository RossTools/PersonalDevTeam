//! A project's branch naming (PDT-594): which template names new tickets' branches (the repository's `[branch]`,
//! else the project's setting on this host, else the default `pdt-{key}-{slug}`), the name a ticket's branch gets,
//! and Settings' view, preview and save (`/api/projects/{id}/branch-names`). The rules are pdt-core `branch_name`.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::branch_name::{BranchFacts, BranchNaming, BranchSettings, PartInfo, part_list, sample, unique};
use pdt_core::project::Project;
use pdt_core::ticket::{Ticket, TicketFilter, TicketSort};
use pdt_core::worktree::worktree_path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use ts_rs::TS;

/// Where a project's branch naming comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BranchSource {
    /// `[branch]` in the repository's `.pdt/project.toml`, which wins over the setting.
    File,
    /// The project's setting on this host.
    Setting,
    /// Neither: `pdt-{key}-{slug}`.
    Default,
}

/// The naming new branches get, and where it comes from.
pub(crate) struct Effective {
    pub naming: BranchNaming,
    pub source: BranchSource,
    /// Why the repository's `[branch]` (or its whole file) can't be used; the default names branches meanwhile.
    pub file_error: Option<String>,
}

/// The project's branch naming now: the main checkout's `[branch]` when it has one, else the setting, else the
/// default. A bad `[branch]` gives the default (and says why); a file that doesn't parse at all leaves the setting.
pub(crate) async fn effective(st: &AppState, project: &Project) -> Result<Effective, ApiError> {
    let mut file_error = None;
    if !project.is_personal() {
        match crate::runconfig::load_effective(st, &project.id, &PathBuf::from(&project.repo_path)).await {
            Ok(c) if c.branch.is_some() => {
                let naming = c.branch.as_ref().and_then(|b| BranchNaming::new(b).ok()).unwrap_or_default();
                return Ok(Effective { naming, source: BranchSource::File, file_error: c.branch_error });
            }
            Ok(_) => {}
            Err(e) => file_error = Some(e.message().to_string()),
        }
    }
    let setting = st.store.get_project_branch_names(&project.id).await?;
    let (naming, source) = match setting.template.is_some() || !setting.types.is_empty() {
        true => match BranchNaming::new(&setting) {
            Ok(n) => (n, BranchSource::Setting),
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "the saved branch naming is not valid; using the default");
                (BranchNaming::default(), BranchSource::Default)
            }
        },
        false => (BranchNaming::default(), BranchSource::Default),
    };
    Ok(Effective { naming, source, file_error })
}

/// `defaultBranch`: the name the project's naming gives the ticket as it is now. Logged, and the store's
/// `pdt-{key}-{slug}` kept, when the naming can't be read.
pub(crate) async fn apply(st: &AppState, t: &mut Ticket) {
    let named = async {
        let project = st.project(&t.summary.project_id).await?;
        Ok::<_, ApiError>(effective(st, &project).await?.naming.name(&BranchFacts::of(&t.summary)))
    };
    match named.await {
        Ok(n) => t.default_branch = n,
        Err(e) => tracing::warn!(ticket = %t.summary.key, error = ?e, "could not read the project's branch naming"),
    }
}

/// The branch Start makes for `ticket`, named `name` by the project's naming: `name`, or `name-2`, `name-3`, … when
/// it is taken. A name is the ticket's own (not taken) when one of its attempts had it, when a worktree of the
/// project has it that no active attempt uses (a Start that failed after making it), or when it has the ticket's
/// `pdt-<KEY>` prefix (no other ticket's branch can, so the default names never change); otherwise it is taken when
/// a worktree, a local branch, `origin`'s branch or the worktree folder has it.
pub(crate) async fn start_branch(
    st: &AppState,
    project: &Project,
    ticket: &Ticket,
    name: &str,
) -> Result<String, ApiError> {
    let worktrees = st.store.list_worktrees(&project.id).await?;
    let active = st.store.list_active_attempts().await?;
    let in_use: Vec<&str> = active.iter().filter_map(|a| a.attempt.worktree_id.as_deref()).collect();
    let mut own: Vec<String> = ticket.attempts.iter().map(|a| a.branch.clone()).collect();
    own.extend(worktrees.iter().filter(|w| !in_use.contains(&w.id.as_str())).map(|w| w.branch.clone()));
    let other: Vec<String> = worktrees.into_iter().map(|w| w.branch).filter(|b| !own.contains(b)).collect();
    let (repo, name, key) = (PathBuf::from(&project.repo_path), name.to_string(), ticket.summary.key.clone());
    tokio::task::spawn_blocking(move || {
        let origin = pdt_git::remote_url(&repo, "origin").is_some();
        unique(&name, |c| {
            if own.iter().any(|o| o == c) || pdt_core::start::is_attempt_branch(c, &key) {
                return false;
            }
            other.iter().any(|o| o == c)
                || pdt_git::branch_exists(&repo, c)
                || (origin && pdt_git::rev_parse(&repo, &format!("refs/remotes/origin/{c}")).is_some())
                || worktree_path(&repo, c).exists()
        })
    })
    .await
    .map_err(ApiError::internal)
}

/// A name for a ticket, for Settings' preview.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchPreview {
    /// What it is named for: "A bug with a GitHub issue and a Jira issue", "PDT-12 Fix the login".
    pub label: String,
    pub name: String,
}

/// `GET /api/projects/{id}/branch-names`.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchNamesView {
    /// The project's setting on this host.
    pub setting: BranchSettings,
    /// The template new branches get, and its `{type}` map (the defaults included).
    pub template: String,
    pub types: BTreeMap<String, String>,
    pub source: BranchSource,
    /// Why the repository's `[branch]` can't be used (the default names branches meanwhile).
    pub file_error: Option<String>,
    /// Names may have a `/`, which some tools handle badly.
    pub slash: bool,
    pub previews: Vec<BranchPreview>,
    /// The parts a template can use.
    pub parts: Vec<PartInfo>,
}

/// `POST /api/projects/{id}/branch-names/preview`: what a template would give, before it is saved.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchNamesPreview {
    /// Why the template can't be used; no previews then.
    pub error: Option<String>,
    pub slash: bool,
    pub previews: Vec<BranchPreview>,
}

/// Settings' examples: a sample bug with and without links, then the project's two newest tickets.
async fn previews(st: &AppState, project: &Project, naming: &BranchNaming) -> Result<Vec<BranchPreview>, ApiError> {
    let with_key = |mut f: BranchFacts| {
        f.project_key = project.key.clone();
        f
    };
    let mut out = vec![
        BranchPreview {
            label: "A bug with a GitHub issue and a Jira issue".into(),
            name: naming.name(&with_key(sample(true))),
        },
        BranchPreview {
            label: "A ticket with no links".into(),
            name: naming.name(&with_key(BranchFacts {
                title: "Add a dark theme".into(),
                labels: vec![],
                ..sample(false)
            })),
        },
    ];
    let filter =
        TicketFilter { project: Some(project.id.clone()), sort: TicketSort::Created, limit: 2, ..Default::default() };
    for t in st.store.query_tickets(&filter).await?.items {
        out.push(BranchPreview { label: format!("{} {}", t.key, t.title), name: naming.name(&BranchFacts::of(&t)) });
    }
    Ok(out)
}

async fn view(st: &AppState, project_id: &str) -> Result<BranchNamesView, ApiError> {
    let project = st.project(project_id).await?;
    let e = effective(st, &project).await?;
    Ok(BranchNamesView {
        setting: st.store.get_project_branch_names(project_id).await?,
        template: e.naming.template().to_string(),
        types: e.naming.types(),
        source: e.source,
        file_error: e.file_error,
        slash: e.naming.has_slash(),
        previews: previews(st, &project, &e.naming).await?,
        parts: part_list(),
    })
}

pub(crate) async fn get_branch_names(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BranchNamesView>, ApiError> {
    Ok(Json(view(&st, &id).await?))
}

/// A setting as given, tidied: a blank template is none.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchNamesInput {
    /// Null or "" for the default.
    pub template: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub types: Option<BTreeMap<String, String>>,
}

fn setting_of(input: BranchNamesInput) -> BranchSettings {
    BranchSettings {
        template: input.template.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
        types: input.types.unwrap_or_default(),
    }
}

/// `PUT /api/projects/{id}/branch-names`: replaces the setting; an invalid template is refused.
pub(crate) async fn put_branch_names(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<BranchNamesInput>, JsonRejection>,
) -> Result<Json<BranchNamesView>, ApiError> {
    let Json(input) = payload?;
    st.project(&id).await?;
    let setting = setting_of(input);
    BranchNaming::new(&setting).map_err(ApiError::invalid)?;
    st.store.put_project_branch_names(&id, &setting).await?;
    Ok(Json(view(&st, &id).await?))
}

pub(crate) async fn preview_branch_names(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<BranchNamesInput>, JsonRejection>,
) -> Result<Json<BranchNamesPreview>, ApiError> {
    let Json(input) = payload?;
    let project = st.project(&id).await?;
    Ok(Json(match BranchNaming::new(&setting_of(input)) {
        Ok(n) => BranchNamesPreview { error: None, slash: n.has_slash(), previews: previews(&st, &project, &n).await? },
        Err(e) => BranchNamesPreview { error: Some(e), slash: false, previews: vec![] },
    }))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/projects/{id}/branch-names", get(get_branch_names).put(put_branch_names))
        .route("/projects/{id}/branch-names/preview", post(preview_branch_names))
}
