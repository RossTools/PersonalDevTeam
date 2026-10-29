//! The GitHub forge plugin (PDT-241): what registers GitHub in PDT's forge registry. It claims remotes on github.com and
//! on the GitHub Enterprise Server the login is for, and opens the one [`GithubApi`] the in-app login signs in. GitHub
//! Actions ([`GithubActions`]) is its CI (PDT-242).

use crate::{GithubActions, GithubApi};
use pdt_core::git_url::Remote;
use pdt_forge::{CiProvider, Forge, ForgeKind, ForgeLinks, ForgePlugin};
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

    /// On the remote's host: github.com, or the Enterprise Server's web host.
    fn links(&self, remote: &Remote, repo: &str) -> Option<ForgeLinks> {
        let host = if remote.host.eq_ignore_ascii_case("www.github.com") { "github.com" } else { &remote.host };
        let repo_url = format!("https://{host}/{repo}");
        Some(ForgeLinks {
            kind: ForgeKind::GITHUB,
            name: "GitHub".into(),
            repo: repo.into(),
            branch_url: Some(format!("{repo_url}/tree/{{branch}}")),
            compare_url: Some(format!("{repo_url}/compare/{{base}}...{{branch}}?expand=1")),
            repo_url,
        })
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

    #[test]
    fn links_are_on_the_remotes_host() {
        let p = GithubPlugin::new(Arc::new(GithubApi::offline()));
        let remote = pdt_core::git_url::parse_remote("git@www.github.com:o/r.git").unwrap();
        let l = p.links(&remote, "o/r").unwrap();
        assert_eq!(l.repo_url, "https://github.com/o/r");
        assert_eq!(l.branch_url.as_deref(), Some("https://github.com/o/r/tree/{branch}"));
        assert_eq!(l.compare_url.as_deref(), Some("https://github.com/o/r/compare/{base}...{branch}?expand=1"));
        let ghe = pdt_core::git_url::parse_remote("https://ghe.example.com/team/app.git").unwrap();
        assert_eq!(p.links(&ghe, "team/app").unwrap().repo_url, "https://ghe.example.com/team/app");
    }
}
