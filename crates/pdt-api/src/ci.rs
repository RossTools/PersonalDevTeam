//! Each project's CI providers (PDT-242): `.pdt/project.toml`'s `[ci] providers` (ids, in order), else its forge's own
//! CI (GitHub Actions, the local forge's `[ci]` command) and every CI integration set up for it (Buildkite: a
//! connection and pipelines in its settings). The PR poller reads them all (`crate::github`), the badge rolls them up
//! into one status, and an automated merge waits for each (`crate::review`).

use crate::forge::ProjectForge;
use crate::routes::AppState;
use pdt_core::pr::CiSource;
use pdt_forge::CiProvider;
use std::sync::Arc;

/// The CI providers of the project at `repo_path`, whose forge is `forge`.
pub(crate) async fn project_ci(
    st: &AppState,
    project_id: &str,
    repo_path: &str,
    forge: Option<&ProjectForge>,
) -> Vec<Arc<dyn CiProvider>> {
    let config = crate::runconfig::load_effective(st, project_id, std::path::Path::new(repo_path)).await.ok();
    let mut found: Vec<Arc<dyn CiProvider>> = forge.and_then(|f| f.ci.clone()).into_iter().collect();
    if let Ok(settings) = st.store.get_project_settings(project_id).await {
        for i in st.registry().integrations() {
            if let Some(p) = i.ci_provider(st, project_id, &settings)
                && !found.iter().any(|f| f.id() == p.id())
            {
                found.push(p);
            }
        }
    }
    choose(found, config.and_then(|c| c.ci_providers).as_deref(), project_id)
}

/// The providers `listed` names, in its order (an id the project doesn't have is skipped); all of them without a list.
fn choose(found: Vec<Arc<dyn CiProvider>>, listed: Option<&[String]>, project_id: &str) -> Vec<Arc<dyn CiProvider>> {
    let Some(ids) = listed else { return found };
    ids.iter()
        .filter_map(|id| {
            let p = found.iter().find(|p| p.id() == id).cloned();
            if p.is_none() {
                tracing::debug!(project_id, provider = %id, "[ci] providers names a provider the project doesn't have");
            }
            p
        })
        .collect()
}

/// How notices name `p`.
pub(crate) fn source(p: &dyn CiProvider) -> CiSource {
    CiSource { id: p.id().to_string(), label: p.label().to_string(), logs: p.capabilities().logs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_forge::ci::testing::FakeCi;
    use pdt_forge::{CiCapabilities, CiConfig};

    fn ids(v: &[Arc<dyn CiProvider>]) -> Vec<&str> {
        v.iter().map(|p| p.id()).collect()
    }

    #[test]
    fn a_list_picks_and_orders_the_providers_a_project_has() {
        let bk = || FakeCi::new("buildkite", "Buildkite", CiCapabilities::NONE, CiConfig::Known(true));
        let found = || -> Vec<Arc<dyn CiProvider>> { vec![Arc::new(FakeCi::actions()), Arc::new(bk())] };
        assert_eq!(ids(&choose(found(), None, "p")), ["github_actions", "buildkite"], "none listed: all");
        let listed = ["buildkite".to_string(), "woodpecker".into(), "github_actions".into()];
        assert_eq!(ids(&choose(found(), Some(&listed), "p")), ["buildkite", "github_actions"]);
        assert!(choose(found(), Some(&[]), "p").is_empty(), "an empty list: no CI");
    }
}
