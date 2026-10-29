//! Each project's forge (PDT-176): the one `.pdt/project.toml` names (`forge = "github"`: a forge plugin's id,
//! PDT-241; or `"local"`: PDT itself, PDT-178), else the forge plugin that claims the project's `origin` (GitHub:
//! github.com, or a GitHub Enterprise Server PDT has an account for, PDT-177, PDT-244). The PR poller, merges, agents'
//! instructions, the issue triage and the forge's workflow nodes ask it, and skip what its [`Capabilities`] say it
//! can't do. A flow tied to no project uses the forge's own account ([`AppState::forge_account`]).

use crate::routes::AppState;
use pdt_core::forge::ForgeKind;
use pdt_forge::{Capabilities, CiProvider, Forge, ForgePlugin};
use std::sync::Arc;

/// A project's forge and the forge's name for its repository.
#[derive(Clone)]
pub(crate) struct ProjectForge {
    pub kind: ForgeKind,
    pub api: Arc<dyn Forge>,
    /// GitHub's `owner/name`; the local forge's, the project's checkout.
    pub repo: String,
    /// The forge's own CI (PDT-242): GitHub Actions, the local forge's `[ci]` command.
    pub ci: Option<Arc<dyn CiProvider>>,
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
        let (command, push) = config.map(|c| (c.ci.map(|c| c.command), c.push)).unwrap_or_default();
        let ci = command
            .is_some()
            .then(|| Arc::new(crate::local_forge::LocalCi::new(st, project_id)) as Arc<dyn CiProvider>);
        let api = Arc::new(crate::local_forge::LocalForge::new(st, project_id, repo_path, command, push));
        return Some(ProjectForge { kind: ForgeKind::LOCAL, api, repo: repo_path.to_string(), ci });
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
    // The account for the origin's host (PDT-244): github.com's, or an Enterprise Server's.
    let host = remote.host.to_ascii_lowercase();
    Some(ProjectForge { kind: plugin.kind(), api: plugin.forge_on(&host), repo, ci: plugin.ci_on(&host) })
}

/// Each forge `plugin` signs in as, for its Connect items and status: the public host's (None), then each other host
/// it has an account for (PDT-244).
pub(crate) fn signed_forges(plugin: &dyn ForgePlugin) -> Vec<(Option<String>, Arc<dyn Forge>)> {
    let mut out = vec![(None, plugin.forge())];
    if let Some(l) = plugin.login() {
        let public = l.spec().default_api_url.map(|u| l.host_of(&u));
        for a in l.accounts() {
            if public.as_deref() != Some(a.host.as_str()) {
                out.push((Some(a.host.clone()), plugin.forge_on(&a.host)));
            }
        }
    }
    out
}

/// The forge a flow on `kind`'s repositories uses (a forge's workflow node): the project's, when the run has a project
/// on that forge, else the forge's account. Err is a message for the user.
pub(crate) async fn forge_for(
    st: &AppState,
    kind: &ForgeKind,
    project_id: Option<&str>,
) -> Result<Arc<dyn Forge>, String> {
    Ok(forge_and_ci_for(st, kind, project_id).await?.0)
}

/// [`forge_for`] with the CI it reads: the project's CI providers ([`crate::ci::project_ci`]), else the forge's own CI.
pub(crate) async fn forge_and_ci_for(
    st: &AppState,
    kind: &ForgeKind,
    project_id: Option<&str>,
) -> Result<(Arc<dyn Forge>, Vec<Arc<dyn CiProvider>>), String> {
    if let Some(id) = project_id
        && let Ok(Some(p)) = st.store.get_project(id).await
        && let Some(f) = project_forge(st, id, &p.repo_path).await
        && f.kind == *kind
    {
        let ci = crate::ci::project_ci(st, id, &p.repo_path, Some(&f)).await;
        return Ok((f.api, ci));
    }
    let plugin = st.forge_plugin(kind).ok_or_else(|| format!("PDT has no {kind} forge"))?;
    Ok((plugin.forge(), plugin.ci().into_iter().collect()))
}
