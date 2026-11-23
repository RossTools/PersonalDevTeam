//! The integrations this pdtd was built with (PDT-173): one cargo feature each, in the order of
//! `pdt_core::integration::KNOWN`. Which of them an instance uses is its own setting (Settings → Plugins).

use pdt_api::Integration;
use pdt_forge::ForgePlugin;
use pdt_forge::transport::Transport;
use std::sync::Arc;

// One push per feature, so a build without any is an empty list.
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn built() -> Vec<Arc<dyn Integration>> {
    let mut out: Vec<Arc<dyn Integration>> = vec![];
    #[cfg(feature = "discord")]
    out.push(Arc::new(pdt_int_discord::Discord));
    #[cfg(feature = "jira")]
    out.push(Arc::new(pdt_int_jira::Jira));
    #[cfg(feature = "buildkite")]
    out.push(Arc::new(pdt_int_buildkite::Buildkite));
    #[cfg(feature = "gitlab")]
    out.push(Arc::new(pdt_int_gitlab::Gitlab));
    #[cfg(feature = "google")]
    out.push(Arc::new(pdt_int_google::Google::new(google_transport())));
    #[cfg(feature = "arr")]
    out.push(Arc::new(pdt_int_arr::Arr));
    #[cfg(feature = "scryer")]
    out.push(Arc::new(pdt_int_scryer::Scryer));
    #[cfg(feature = "plex")]
    out.push(Arc::new(pdt_int_plex::Plex));
    out
}

/// How the Google plugin reaches Google: the network on pdtd's runtime (none outside one, as in a unit test).
#[cfg(feature = "google")]
fn google_transport() -> Arc<dyn Transport> {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => Arc::new(pdt_forge::transport::ReqwestTransport::on(h)),
        Err(_) => Arc::new(pdt_forge::transport::Offline),
    }
}

/// The forge plugins of the integrations built in (PDT-246: GitLab), each reaching its forge through `transport`. One
/// counts while its integration is enabled.
#[allow(unused_mut, unused_variables, clippy::vec_init_then_push)]
pub fn forges(transport: Arc<dyn Transport>) -> Vec<Arc<dyn ForgePlugin>> {
    let mut out: Vec<Arc<dyn ForgePlugin>> = vec![];
    #[cfg(feature = "gitlab")]
    out.push(Arc::new(pdt_int_gitlab::GitlabPlugin::new(Arc::new(pdt_int_gitlab::GitlabApi::new(transport)))));
    out
}

#[cfg(all(
    test,
    feature = "discord",
    feature = "jira",
    feature = "buildkite",
    feature = "gitlab",
    feature = "google",
    feature = "arr",
    feature = "scryer",
    feature = "plex"
))]
mod tests;

#[cfg(test)]
mod order {
    /// Every integration built in is one PDT knows, under its feature's name, in the list's order.
    #[test]
    fn built_integrations_are_known_and_in_order() {
        let ids: Vec<&str> = super::built().iter().map(|i| i.id()).collect();
        let known: Vec<&str> =
            pdt_core::integration::KNOWN.iter().map(|k| k.id).filter(|id| ids.contains(id)).collect();
        assert_eq!(ids, known);
    }
}
