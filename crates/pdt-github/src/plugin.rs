//! The GitHub forge plugin (PDT-241): what registers GitHub in PDT's forge registry. It claims remotes on github.com and
//! on the GitHub Enterprise Server the login is for, and opens the one [`GithubApi`] the in-app login signs in. GitHub
//! Actions ([`GithubActions`]) is its CI (PDT-242).

use crate::{GithubActions, GithubApi};
use pdt_forge::{CiProvider, Forge, ForgeKind, ForgePlugin};
use std::sync::Arc;

pub struct GithubPlugin {
    api: Arc<GithubApi>,
}

impl GithubPlugin {
    pub fn new(api: Arc<GithubApi>) -> Self {
        Self { api }
    }

    /// The client the login signs in.
    pub fn api(&self) -> &Arc<GithubApi> {
        &self.api
    }
}

impl ForgePlugin for GithubPlugin {
    fn kind(&self) -> ForgeKind {
        ForgeKind::GITHUB
    }

    fn label(&self) -> &'static str {
        "GitHub"
    }

    fn claims_host(&self, host: &str) -> bool {
        host.eq_ignore_ascii_case("github.com")
            || host.eq_ignore_ascii_case("www.github.com")
            || self.api.account().is_some_and(|a| crate::login::web_host(&a.api_url).eq_ignore_ascii_case(host))
    }

    fn forge(&self) -> Arc<dyn Forge> {
        self.api.clone()
    }

    fn ci(&self) -> Option<Arc<dyn CiProvider>> {
        Some(Arc::new(GithubActions::new(self.api.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_forge::ForgeRegistry;

    #[test]
    fn github_claims_github_com_and_the_logins_enterprise_server() {
        let api = Arc::new(GithubApi::offline());
        let r = ForgeRegistry::new(vec![Arc::new(GithubPlugin::new(api.clone()))]);
        assert_eq!(r.detect("git@github.com:o/r.git").map(|(_, r)| r).as_deref(), Some("o/r"));
        assert_eq!(r.detect("https://www.github.com/o/r").map(|(_, r)| r).as_deref(), Some("o/r"));
        assert!(r.detect("https://ghe.example.com/o/r.git").is_none(), "no login for it");
        api.set_account(Some(crate::Account {
            token: "t".into(),
            api_url: "https://ghe.example.com/api/v3".into(),
            login: "ross".into(),
            signed_in_at: None,
        }));
        assert_eq!(r.detect("https://ghe.example.com/o/r.git").map(|(_, r)| r).as_deref(), Some("o/r"));
        assert_eq!(r.detect("git@github.com:o/r.git").map(|(_, r)| r).as_deref(), Some("o/r"), "github.com still");
        assert!(r.detect("https://ghe.example.com/o/r/tree/main").is_none());
    }
}
