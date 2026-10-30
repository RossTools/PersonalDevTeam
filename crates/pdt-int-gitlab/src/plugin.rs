//! The GitLab forge plugin (PDT-246): what registers GitLab in PDT's forge registry. It claims remotes on gitlab.com
//! and on each self-hosted GitLab the user has signed in to (Settings › Plugins › GitLab: the host of the API URL they
//! gave), and opens one [`GitlabApi`] per host, signed in with that host's account. GitLab CI ([`GitlabCi`]) is its CI.
//! Its login ([`ForgeLogin`]) takes a pasted token, or the OAuth device flow with an OAuth application's client ID.

use crate::api::{GitlabApi, LoginCheck};
use crate::ci::GitlabCi;
use crate::login::{self, GITLAB_HOST};
use pdt_core::git_url::Remote;
use pdt_forge::login::{AccountState, DeviceCode, DevicePoll, LoginSpec, RenewHook, Whoami};
use pdt_forge::{CiProvider, Forge, ForgeAccount, ForgeKind, ForgeLinks, ForgeLogin, ForgePlugin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// GitLab's forge id (`forge = "gitlab"` in `.pdt/project.toml`).
pub const GITLAB: ForgeKind = ForgeKind::from_static("gitlab");

pub struct GitlabPlugin {
    /// gitlab.com's.
    api: Arc<GitlabApi>,
    /// Each self-hosted server's with an account, by web host.
    servers: RwLock<BTreeMap<String, Arc<GitlabApi>>>,
    /// Stores an account a client renewed.
    renewed: Arc<RwLock<Option<RenewHook>>>,
}

/// `www.gitlab.com` is gitlab.com.
fn host_key(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    if h == "www.gitlab.com" { GITLAB_HOST.into() } else { h }
}

impl GitlabPlugin {
    /// The plugin with `api` as gitlab.com's client; self-hosted servers' share its transport.
    pub fn new(api: Arc<GitlabApi>) -> Self {
        let p = Self { api, servers: RwLock::default(), renewed: Arc::default() };
        p.wire(&p.api, GITLAB_HOST);
        p
    }

    /// Hands the accounts `api` renews for `host` to the hook [`ForgeLogin::on_renewed`] set.
    fn wire(&self, api: &GitlabApi, host: &str) {
        let (hook, host) = (self.renewed.clone(), host.to_string());
        api.on_renewed(Some(Arc::new(move |old: &str, a: &ForgeAccount| {
            let hook = hook.read().unwrap_or_else(|e| e.into_inner()).clone();
            match hook {
                Some(h) => h(&host, old, a),
                None => tracing::warn!(host, "a renewed GitLab login has nowhere to be stored"),
            }
        })));
    }

    /// gitlab.com's client.
    pub fn api(&self) -> &Arc<GitlabApi> {
        &self.api
    }

    /// The client of `host`: gitlab.com's, or a self-hosted server's with an account.
    pub fn api_on(&self, host: &str) -> Option<Arc<GitlabApi>> {
        let h = host_key(host);
        if h == GITLAB_HOST {
            return Some(self.api.clone());
        }
        self.servers.read().unwrap_or_else(|e| e.into_inner()).get(&h).cloned()
    }

    fn apis(&self) -> Vec<(String, Arc<GitlabApi>)> {
        let servers = self.servers.read().unwrap_or_else(|e| e.into_inner());
        std::iter::once((GITLAB_HOST.to_string(), self.api.clone()))
            .chain(servers.iter().map(|(h, a)| (h.clone(), a.clone())))
            .collect()
    }
}

/// A project's path on GitLab (`group/sub/app`: any depth of groups); not a page inside one (`…/-/tree/main`).
pub fn project_of(remote: &Remote) -> Option<String> {
    (remote.path.len() >= 2 && !remote.path.iter().any(|s| s == "-")).then(|| remote.path.join("/"))
}

impl ForgePlugin for GitlabPlugin {
    fn kind(&self) -> ForgeKind {
        GITLAB
    }

    fn label(&self) -> &'static str {
        "GitLab"
    }

    /// gitlab.com, and every self-hosted GitLab PDT has an account for.
    fn claims_host(&self, host: &str) -> bool {
        self.api_on(host).is_some()
    }

    /// The project's full path, nested groups and all.
    fn repo_of(&self, remote: &Remote) -> Option<String> {
        if self.claims_host(&remote.host) { project_of(remote) } else { None }
    }

    fn links(&self, remote: &Remote, repo: &str) -> Option<ForgeLinks> {
        let host = host_key(&remote.host);
        // A self-hosted server's web URL may have a port or a path: the account's API URL says.
        let web = self
            .api_on(&host)
            .and_then(|a| a.account())
            .map_or_else(|| format!("https://{host}"), |a| login::web_url(&a.api_url));
        let repo_url = format!("{web}/{repo}");
        Some(ForgeLinks {
            kind: GITLAB,
            name: "GitLab".into(),
            repo: repo.into(),
            branch_url: Some(format!("{repo_url}/-/tree/{{branch}}")),
            compare_url: Some(format!(
                "{repo_url}/-/merge_requests/new?merge_request%5Bsource_branch%5D={{branch}}\
                 &merge_request%5Btarget_branch%5D={{base}}"
            )),
            repo_url,
        })
    }

    fn forge(&self) -> Arc<dyn Forge> {
        self.api.clone()
    }

    fn ci(&self) -> Option<Arc<dyn CiProvider>> {
        Some(Arc::new(GitlabCi::new(self.api.clone())))
    }

    fn forge_on(&self, host: &str) -> Arc<dyn Forge> {
        self.api_on(host).unwrap_or_else(|| self.api.clone())
    }

    fn ci_on(&self, host: &str) -> Option<Arc<dyn CiProvider>> {
        Some(Arc::new(GitlabCi::new(self.api_on(host).unwrap_or_else(|| self.api.clone()))))
    }

    fn login(&self) -> Option<&dyn ForgeLogin> {
        Some(self)
    }
}

impl ForgeLogin for GitlabPlugin {
    fn spec(&self) -> LoginSpec {
        LoginSpec {
            default_api_url: Some(login::GITLAB_API.into()),
            api_url_example: "https://gitlab.example.com/api/v4".into(),
            scopes: login::SCOPES.into(),
            token_help: "A personal access token with the api scope (User settings › Access tokens), or a project or \
                         group access token with the api scope and the Developer role or above (Maintainer to merge \
                         into a protected branch). With the GitLab CLI, `glab auth status -t` shows one."
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

    /// gitlab.com's client signs in or out; a self-hosted server gets a client of its own with its first account, and
    /// loses it with its last (its projects then have no forge, as before it had one).
    fn set_account(&self, host: &str, account: Option<ForgeAccount>) {
        let h = host_key(host);
        if h == GITLAB_HOST {
            return self.api.set_account(account);
        }
        let mut servers = self.servers.write().unwrap_or_else(|e| e.into_inner());
        match account {
            Some(a) => servers
                .entry(h.clone())
                .or_insert_with(|| {
                    let api = Arc::new(GitlabApi::new(self.api.transport()));
                    self.wire(&api, &h);
                    api
                })
                .set_account(Some(a)),
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

    fn on_renewed(&self, hook: RenewHook) {
        *self.renewed.write().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn check_accounts(&self) -> bool {
        let mut recovered = false;
        for (host, api) in self.apis() {
            match api.check_login() {
                Some(LoginCheck::Recovered) => recovered = true,
                Some(LoginCheck::Failed(e)) => {
                    tracing::debug!(host, error = %e, "the GitLab login check got no answer")
                }
                _ => {}
            }
        }
        recovered
    }
}
