//! Each project's forge (PDT-176): the one `.pdt/project.toml` names (`forge = "github"`: a forge plugin's id,
//! PDT-241; or `"local"`: PDT itself, PDT-178), else the forge plugin that claims the project's `origin` (GitHub:
//! github.com, or the GitHub Enterprise Server the GitHub login is for, PDT-177). The PR poller, merges, agents'
//! instructions, the issue triage and the forge's workflow nodes ask it, and skip what its [`Capabilities`] say it
//! can't do. A flow tied to no project uses the forge's own account ([`AppState::forge_account`]).

use crate::routes::AppState;
use pdt_core::forge::ForgeKind;
use pdt_forge::{Capabilities, Forge, ForgePlugin};
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

impl AppState {
    /// The forge plugins in use: those whose kind an enabled integration declares ([`crate::Integration::forge_kind`]).
    pub(crate) fn forge_plugins(&self) -> Vec<Arc<dyn ForgePlugin>> {
        let registry = self.registry();
        let on = |k: &ForgeKind| registry.integrations().iter().any(|i| i.forge_kind().as_ref() == Some(k));
        self.forges.plugins().iter().filter(|p| on(&p.kind())).cloned().collect()
    }

    /// The forge plugin of `kind`, while it is in use.
    pub(crate) fn forge_plugin(&self, kind: &ForgeKind) -> Option<Arc<dyn ForgePlugin>> {
        self.forge_plugins().into_iter().find(|p| p.kind() == *kind)
    }

    /// The forge of `kind` as its account, for a flow tied to no project. Err is a message for the user.
    pub(crate) fn forge_account(&self, kind: &ForgeKind) -> Result<Arc<dyn Forge>, String> {
        self.forge_plugin(kind).map(|p| p.forge()).ok_or_else(|| format!("PDT has no {kind} forge"))
    }
}

/// The forge of the project at `repo_path` (its main checkout); None when it has none (`forge = "none"`, an origin
/// no forge claims, a Personal project's folder).
pub(crate) async fn project_forge(st: &AppState, project_id: &str, repo_path: &str) -> Option<ProjectForge> {
    // A file that can't be read leaves the choice to the origin, as a project without one.
    let config = crate::runconfig::load_effective(st, project_id, std::path::Path::new(repo_path)).await.ok();
    let named = config.as_ref().and_then(|c| c.forge.clone());
    if named == Some(ForgeKind::NONE) {
        return None;
    }
    if named == Some(ForgeKind::LOCAL) {
        let command = config.and_then(|c| c.ci).map(|c| c.command);
        let api = Arc::new(crate::local_forge::LocalForge::new(st, project_id, repo_path, command));
        return Some(ProjectForge { kind: ForgeKind::LOCAL, api, repo: repo_path.to_string() });
    }
    let plugins = st.forge_plugins();
    // A forge no plugin in use provides is left to the origin, as a project that names none.
    let named = named.and_then(|k| match plugins.iter().find(|p| p.kind() == k) {
        Some(p) => Some(p.clone()),
        None => {
            tracing::debug!(project_id, forge = %k, "no forge plugin of this kind is in use; the origin decides");
            None
        }
    });
    let path = std::path::PathBuf::from(repo_path);
    let url = tokio::task::spawn_blocking(move || pdt_git::remote_url(&path, "origin")).await.ok().flatten()?;
    let remote = pdt_core::git_url::parse_remote(&url)?;
    let (plugin, repo) = match named {
        Some(p) => {
            let repo = p.repo_of(&remote)?;
            (p, repo)
        }
        None => plugins.iter().find_map(|p| p.repo_of(&remote).map(|r| (p.clone(), r)))?,
    };
    Some(ProjectForge { kind: plugin.kind(), api: plugin.forge(), repo })
}

/// The forge a flow on `kind`'s repositories uses (a forge's workflow node): the project's, when the run has a project
/// on that forge, else the forge's account. Err is a message for the user.
pub(crate) async fn forge_for(
    st: &AppState,
    kind: &ForgeKind,
    project_id: Option<&str>,
) -> Result<Arc<dyn Forge>, String> {
    if let Some(id) = project_id
        && let Ok(Some(p)) = st.store.get_project(id).await
        && let Some(f) = project_forge(st, id, &p.repo_path).await
        && f.kind == *kind
    {
        return Ok(f.api);
    }
    st.forge_account(kind)
}
