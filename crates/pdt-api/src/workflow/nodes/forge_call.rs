//! Forge and CI calls off the async threads, for the nodes that read or write a forge: their errors name the forge
//! (or CI provider) they went to, so a GitLab failure doesn't read as GitHub's.

use pdt_forge::{CiProvider, Forge, ForgeError};
use std::sync::Arc;

/// Runs `f` on `api` off the async threads.
pub(crate) async fn forge_call<T: Send + 'static>(
    api: &Arc<dyn Forge>,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let (api, name) = (api.clone(), api.name());
    tokio::task::spawn_blocking(move || f(api.as_ref()))
        .await
        .map_err(|e| format!("the {name} task failed: {e}"))?
        .map_err(|e| format!("{name}: {e}"))
}

/// Runs `f` on CI provider `p` off the async threads.
pub(crate) async fn ci_call<T: Send + 'static>(
    p: &Arc<dyn CiProvider>,
    f: impl FnOnce(&dyn CiProvider) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let (p, label) = (p.clone(), p.label().to_string());
    tokio::task::spawn_blocking(move || f(p.as_ref()))
        .await
        .map_err(|e| format!("the {label} task failed: {e}"))?
        .map_err(|e| format!("{label}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn errors_name_the_forge_or_ci_they_came_from() {
        let forge: Arc<dyn Forge> = Arc::new(pdt_forge::testing::FakeForge::default());
        let e = forge_call(&forge, |_| Err::<(), _>(ForgeError::Failed("down".into()))).await.unwrap_err();
        assert_eq!(e, "the fake forge: down");
        assert_eq!(forge_call(&forge, |_| Ok(3)).await, Ok(3));

        let ci: Arc<dyn CiProvider> = Arc::new(pdt_forge::ci::testing::FakeCi::actions());
        let e = ci_call(&ci, |_| Err::<(), _>(ForgeError::Failed("down".into()))).await.unwrap_err();
        assert_eq!(e, format!("{}: down", ci.label()));
    }
}
