//! The repository a GitHub node reads or writes and the forge and CI it goes through (PDT-241): the one given, else
//! the run's project's on GitHub.

use crate::workflow::catalog::{StepCx, nonblank, setup_error};
use pdt_core::forge::ForgeKind;
use pdt_core::github_issue::valid_repo;
use pdt_forge::{CiProvider, Forge};
use std::sync::Arc;

/// Why `api` can't be read for releases ([`pdt_forge::Capabilities::releases`]), if it can't.
pub(crate) fn has_releases(api: &Arc<dyn Forge>) -> Result<(), String> {
    match api.capabilities().releases {
        true => Ok(()),
        false => Err(format!("{} has no releases", api.name())),
    }
}

pub(crate) fn templated(s: &str) -> bool {
    s.contains("{{")
}

/// A given `repo` must be owner/name (left alone while it is a template).
pub(crate) fn check_repo(repo: &Option<String>) -> Result<(), String> {
    match nonblank(repo) {
        Some(r) if !templated(r) && valid_repo(r).is_none() => Err(format!("{r:?} is not owner/name")),
        _ => Ok(()),
    }
}

/// A repository, the forge it is read through and the CI it has ([`repo_for_step`]).
pub(crate) struct RepoOf {
    pub repo: String,
    pub forge: Arc<dyn Forge>,
    /// The project's CI providers (PDT-242), else GitHub's own CI.
    pub ci: Vec<Arc<dyn CiProvider>>,
}

impl RepoOf {
    /// Its GitHub Actions, or why it has none (`what` it was wanted for).
    pub fn actions(&self, what: &str) -> Result<Arc<dyn CiProvider>, String> {
        self.ci
            .iter()
            .find(|p| p.id() == pdt_core::pr::GITHUB_ACTIONS)
            .cloned()
            .ok_or_else(|| format!("{} has no GitHub Actions to {what}", self.repo))
    }
}

/// The repository and the forge it is read through (PDT-241): the one given, through the run's project's forge when
/// that is GitHub, else GitHub's account; without one, the run's project's on GitHub. For a workflow step, so a
/// missing project or repository is a setup error ([`setup_error`]): the run fails at once (PDT-301).
pub(crate) async fn repo_for_step(cx: &StepCx<'_>, repo: &Option<String>) -> Result<RepoOf, String> {
    resolve_repo(cx.st, cx.run.project_id.as_deref(), repo)
        .await
        .map_err(|(setup, e)| if setup { setup_error(e) } else { e })
}

pub(crate) async fn repo_for(
    st: &crate::routes::AppState,
    project: Option<&str>,
    repo: &Option<String>,
) -> Result<RepoOf, String> {
    resolve_repo(st, project, repo).await.map_err(|(_, e)| e)
}

/// The repository, or why not: `(true, …)` when it is the setup's fault (none given and no project to take it from,
/// the project gone or not on GitHub, a repo that isn't owner/name).
async fn resolve_repo(
    st: &crate::routes::AppState,
    project: Option<&str>,
    repo: &Option<String>,
) -> Result<RepoOf, (bool, String)> {
    let passing = |e: String| (false, e);
    if let Some(r) = nonblank(repo) {
        valid_repo(r).ok_or_else(|| (true, format!("{r:?} is not owner/name")))?;
        let (forge, ci) = crate::forge::forge_and_ci_for(st, &ForgeKind::GITHUB, project).await.map_err(passing)?;
        return Ok(RepoOf { repo: r.to_string(), forge, ci });
    }
    let id = project.ok_or((
        true,
        "give repo (owner/name): the run has no project to take it from (start it in a project)".to_string(),
    ))?;
    let p = st
        .store
        .get_project(id)
        .await
        .map_err(|e| passing(e.to_string()))?
        .ok_or((true, "the run's project is gone".to_string()))?;
    match crate::forge::project_forge(st, id, &p.repo_path).await {
        Some(f) if f.kind == ForgeKind::GITHUB => {
            let ci = crate::ci::project_ci(st, id, &p.repo_path, Some(&f)).await;
            Ok(RepoOf { repo: f.repo, forge: f.api, ci })
        }
        _ => Err((true, format!("{} is not on GitHub: give repo (owner/name)", p.name))),
    }
}
