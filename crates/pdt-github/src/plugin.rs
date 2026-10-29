//! The GitHub forge plugin (PDT-241): what registers GitHub in PDT's forge registry. It claims remotes on github.com and
//! on each GitHub Enterprise Server it has an account for, and opens one [`GithubApi`] per host, signed in with that
//! host's account (PDT-244). GitHub Actions ([`GithubActions`]) is its CI (PDT-242). Its login
//! ([`ForgeLogin`]) takes a pasted token or GitHub's OAuth device flow.

use crate::login::{self, GITHUB_HOST};
use crate::{GithubActions, GithubApi, LoginCheck};
use pdt_core::git_url::Remote;
use pdt_forge::login::{AccountState, DeviceCode, DevicePoll, LoginSpec, Whoami};
use pdt_forge::{CiProvider, Forge, ForgeAccount, ForgeKind, ForgeLinks, ForgeLogin, ForgePlugin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

pub struct GithubPlugin {
    /// github.com's.
    api: Arc<GithubApi>,
    /// Each Enterprise Server's with an account, by web host.
    servers: RwLock<BTreeMap<String, Arc<GithubApi>>>,
}

/// `www.github.com` is github.com.
fn host_key(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    if h == "www.github.com" { GITHUB_HOST.into() } else { h }
}

impl GithubPlugin {
    /// The plugin with `api` as github.com's client; Enterprise Servers' share its transport.
    pub fn new(api: Arc<GithubApi>) -> Self {
        Self { api, servers: RwLock::default() }
    }

    /// github.com's client.
    pub fn api(&self) -> &Arc<GithubApi> {
        &self.api
    }

    /// The client of `host`: github.com's, or an Enterprise Server's with an account.
    pub fn api_on(&self, host: &str) -> Option<Arc<GithubApi>> {
        let h = host_key(host);
        if h == GITHUB_HOST {
            return Some(self.api.clone());
        }
        self.servers.read().unwrap_or_else(|e| e.into_inner()).get(&h).cloned()
    }

    /// Every client: github.com's first.
    fn apis(&self) -> Vec<(String, Arc<GithubApi>)> {
        let servers = self.servers.read().unwrap_or_else(|e| e.into_inner());
        std::iter::once((GITHUB_HOST.to_string(), self.api.clone()))
            .chain(servers.iter().map(|(h, a)| (h.clone(), a.clone())))
            .collect()
    }
}

impl ForgePlugin for GithubPlugin {
    fn kind(&self) -> ForgeKind {
        ForgeKind::GITHUB
    }

    fn label(&self) -> &'static str {
        "GitHub"
    }

    /// github.com, and every Enterprise Server PDT has an account for.
    fn claims_host(&self, host: &str) -> bool {
        self.api_on(host).is_some()
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

    fn forge_on(&self, host: &str) -> Arc<dyn Forge> {
        self.api_on(host).unwrap_or_else(|| self.api.clone())
    }

    fn ci_on(&self, host: &str) -> Option<Arc<dyn CiProvider>> {
        Some(Arc::new(GithubActions::new(self.api_on(host).unwrap_or_else(|| self.api.clone()))))
    }

    fn login(&self) -> Option<&dyn ForgeLogin> {
        Some(self)
    }
}

impl ForgeLogin for GithubPlugin {
    fn spec(&self) -> LoginSpec {
        LoginSpec {
            default_api_url: Some(login::GITHUB_API.into()),
            api_url_example: "https://github.example.com/api/v3".into(),
            scopes: login::SCOPES.into(),
            token_help: "A classic token with the repo scope, or a fine-grained one with read and write on pull \
                         requests, issues and contents, and read on actions and metadata. With the GitHub CLI, \
                         `gh auth token` prints one."
                .into(),
            device_flow: true,
        }
    }

    fn api_url(&self, given: &str) -> String {
        login::api_url(given)
    }

    fn host_of(&self, api_url: &str) -> String {
        host_key(&login::web_host(api_url))
    }

    fn web_url(&self, api_url: &str) -> String {
        login::web_url(api_url)
    }

    fn whoami(&self, api_url: &str, token: &str) -> Result<Whoami, String> {
        login::whoami(self.api.transport().as_ref(), api_url, token)
    }

    fn start_device_flow(&self, api_url: &str, client_id: &str) -> Result<DeviceCode, String> {
        login::start_device_flow(self.api.transport().as_ref(), api_url, client_id)
    }

    fn poll_device_flow(&self, api_url: &str, client_id: &str, device_code: &str) -> DevicePoll {
        login::poll_device_flow(self.api.transport().as_ref(), api_url, client_id, device_code)
    }

    /// github.com's client signs in or out; an Enterprise Server gets a client of its own with its first account,
    /// and loses it with its last (its projects then have no forge, as before it had one).
    fn set_account(&self, host: &str, account: Option<ForgeAccount>) {
        let h = host_key(host);
        if h == GITHUB_HOST {
            return self.api.set_account(account);
        }
        let mut servers = self.servers.write().unwrap_or_else(|e| e.into_inner());
        match account {
            Some(a) => {
                servers.entry(h).or_insert_with(|| Arc::new(GithubApi::new(self.api.transport()))).set_account(Some(a))
            }
            None => {
                servers.remove(&h);
            }
        }
    }

    fn accounts(&self) -> Vec<AccountState> {
        self.apis()
            .into_iter()
            .filter_map(|(host, api)| {
                api.account().map(|account| AccountState { host, account, rejected_at: api.rejected_at() })
            })
            .collect()
    }

    fn check_accounts(&self) -> bool {
        let mut recovered = false;
        for (host, api) in self.apis() {
            match api.check_login() {
                Some(LoginCheck::Recovered) => recovered = true,
                Some(LoginCheck::Failed(e)) => {
                    tracing::debug!(host, error = %e, "the GitHub login check got no answer")
                }
                _ => {}
            }
        }
        recovered
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
        let plugin = r.get(&ForgeKind::GITHUB).unwrap();
        plugin
            .login()
            .unwrap()
            .set_account("ghe.example.com", Some(account("rwest", "https://ghe.example.com/api/v3")));
        assert_eq!(r.detect("https://ghe.example.com/o/r.git").map(|(_, r)| r).as_deref(), Some("o/r"));
        assert_eq!(r.detect("git@github.com:o/r.git").map(|(_, r)| r).as_deref(), Some("o/r"), "github.com still");
        assert!(r.detect("https://ghe.example.com/o/r/tree/main").is_none());
        plugin.login().unwrap().set_account("ghe.example.com", None);
        assert!(r.detect("https://ghe.example.com/o/r.git").is_none(), "signed out");
    }

    fn account(login: &str, api: &str) -> crate::Account {
        crate::Account { token: format!("t-{login}"), api_url: api.into(), login: login.into(), signed_in_at: None }
    }

    #[test]
    fn two_hosts_are_signed_in_at_once_each_with_its_own_client() {
        let api = Arc::new(GithubApi::offline());
        let p = GithubPlugin::new(api.clone());
        let l = p.login().unwrap();
        assert_eq!(l.host_of(&l.api_url("")), "github.com");
        assert_eq!(l.host_of("https://GHE.example.com/api/v3/"), "ghe.example.com");
        l.set_account("github.com", Some(account("ross", "https://api.github.com")));
        l.set_account("ghe.example.com", Some(account("rwest", "https://ghe.example.com/api/v3")));
        let hosts: Vec<_> = l.accounts().into_iter().map(|a| (a.host, a.account.login)).collect();
        assert_eq!(hosts, [("github.com".to_string(), "ross".to_string()), ("ghe.example.com".into(), "rwest".into())]);
        assert_eq!(api.account().unwrap().login, "ross", "github.com's client");
        let ghe = p.api_on("ghe.example.com").unwrap();
        assert!(!Arc::ptr_eq(&ghe, &api));
        assert_eq!(ghe.account().unwrap().login, "rwest");
        assert!(Arc::ptr_eq(&p.api_on("www.github.com").unwrap(), &api));
        assert!(p.api_on("gitlab.com").is_none());
        // Replacing one host's account leaves the other's.
        l.set_account("ghe.example.com", Some(account("other", "https://ghe.example.com/api/v3")));
        assert!(Arc::ptr_eq(&p.api_on("ghe.example.com").unwrap(), &ghe), "the same client, signed in again");
        assert_eq!((ghe.account().unwrap().login, api.account().unwrap().login), ("other".into(), "ross".into()));
        l.set_account("github.com", None);
        assert!(api.account().is_none());
        assert_eq!(l.accounts().len(), 1);
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
