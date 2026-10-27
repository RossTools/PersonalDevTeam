//! Each project's forge (PDT-176): the one `.pdt/project.toml` names (`forge = "github"`, or `"local"`: PDT itself,
//! PDT-178), else GitHub when the project's `origin` is on github.com (or on the GitHub Enterprise Server the GitHub
//! login is for, PDT-177). The PR poller, merges, agents' instructions and the issue triage ask it, and
//! skip what its [`Capabilities`] say it can't do.

use crate::routes::AppState;
use pdt_core::forge::ForgeKind;
use pdt_forge::{Capabilities, Forge};
use std::sync::Arc;

/// A project's forge and the forge's name for its repository.
#[derive(Clone)]
pub(crate) struct ProjectForge {
    pub kind: ForgeKind,
    pub api: Arc<dyn Forge>,
    /// GitHub's `owner/name`; the local forge's, the project's checkout.
    pub repo: String,
}

impl ProjectForge {
    pub fn caps(&self) -> Capabilities {
        self.api.capabilities()
    }
}

/// The forge of the project at `repo_path` (its main checkout); None when it has none (`forge = "none"`, an origin
/// no forge knows, a Personal project's folder).
pub(crate) async fn project_forge(st: &AppState, project_id: &str, repo_path: &str) -> Option<ProjectForge> {
    // A file that can't be read leaves the choice to the origin, as a project without one.
    let config = crate::runconfig::load_effective(st, project_id, std::path::Path::new(repo_path)).await.ok();
    match config.as_ref().and_then(|c| c.forge) {
        Some(ForgeKind::None) => None,
        Some(ForgeKind::Local) => {
            let command = config.and_then(|c| c.ci).map(|c| c.command);
            let api = Arc::new(crate::local_forge::LocalForge::new(st, project_id, repo_path, command));
            Some(ProjectForge { kind: ForgeKind::Local, api, repo: repo_path.to_string() })
        }
        Some(ForgeKind::Github) | None => {
            let host = st.github_api.account().map(|a| pdt_github::login::web_host(&a.api_url));
            let repo = crate::github::github_repo_on(repo_path, host).await?;
            Some(ProjectForge { kind: ForgeKind::Github, api: st.github.clone(), repo })
        }
    }
}
