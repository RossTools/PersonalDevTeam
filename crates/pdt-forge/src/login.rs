//! Signing in to a forge (PDT-244): every forge takes a pasted token for any of its hosts, and the ones that declare
//! it an OAuth device flow (GitHub's). PDT keeps one account per forge kind and host (`github:github.com`,
//! `github:ghe.example.com`), in an [`AccountStore`] the keychain holds; a project's forge is the account for its
//! `origin`'s host ([`crate::ForgePlugin::forge_on`]).

use crate::ForgeKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One account on a forge host: its token and the API it is for. Never sent to the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeAccount {
    pub token: String,
    /// The host's API (`https://api.github.com`, `https://ghe.example.com/api/v3`).
    pub api_url: String,
    /// Who the token signs in as.
    pub login: String,
    /// When the user signed in (Unix ms); unknown for a login stored before PDT-252.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in_at: Option<i64>,
}

/// What a forge's sign-in form asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSpec {
    /// The public host's API, prefilled (github.com's); None for a forge that is only self-hosted.
    pub default_api_url: Option<String>,
    /// An example API URL of a self-hosted server, for the form's placeholder.
    pub api_url_example: String,
    /// The scopes a token needs, as the forge names them (`repo`).
    pub scopes: String,
    /// What else to know about the token: fine-grained permissions, where to make one.
    pub token_help: String,
    /// The forge has an OAuth device flow ([`ForgeLogin::start_device_flow`]).
    pub device_flow: bool,
}

/// Who a token signs in as, and the scopes the forge says it has (empty when it doesn't say).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whoami {
    pub login: String,
    pub scopes: Vec<String>,
}

/// A device flow's code, for the user to enter at `verification_uri`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds.
    pub expires_in: u64,
    /// The fewest seconds between polls.
    pub interval: u64,
}

/// One poll of a device flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    /// The user has not entered the code yet.
    Pending,
    /// Polling too fast: wait this many seconds between polls from now on.
    SlowDown(u64),
    Token(String),
    Expired,
    Denied,
    Failed(String),
}

/// An account a forge is signed in with, and whether the forge still takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountState {
    /// The web host (lower case), the account's key.
    pub host: String,
    pub account: ForgeAccount,
    /// When the forge stopped accepting it (PDT-212).
    pub rejected_at: Option<i64>,
}

/// A forge plugin's sign-in ([`crate::ForgePlugin::login`]). Blocking; call from `spawn_blocking` in async code.
pub trait ForgeLogin: Send + Sync {
    fn spec(&self) -> LoginSpec;
    /// An API URL as the user gave it, tidied; the public host's when empty.
    fn api_url(&self, given: &str) -> String;
    /// The web host (lower case) of an API URL: the host a project's `origin` is on, and the account's key.
    fn host_of(&self, api_url: &str) -> String;
    /// The website of an API URL (`https://github.com`).
    fn web_url(&self, api_url: &str) -> String;
    /// Asks the forge who `token` is. Err says why it was refused, for the user.
    fn whoami(&self, api_url: &str, token: &str) -> Result<Whoami, String>;
    /// Starts a device flow for OAuth App `client_id` ([`LoginSpec::device_flow`]).
    fn start_device_flow(&self, _api_url: &str, _client_id: &str) -> Result<DeviceCode, String> {
        Err("this forge has no device flow: paste a token instead".into())
    }
    /// Asks whether the user has entered the code yet.
    fn poll_device_flow(&self, _api_url: &str, _client_id: &str, _device_code: &str) -> DevicePoll {
        DevicePoll::Failed("this forge has no device flow".into())
    }
    /// Signs the forge on `host` in with `account`, or out with None.
    fn set_account(&self, host: &str, account: Option<ForgeAccount>);
    /// The accounts it is signed in with, by host.
    fn accounts(&self) -> Vec<AccountState>;
    /// The light check of every account (the 5-minute login check, PDT-252): true when the forge accepts again one it
    /// had refused.
    fn check_accounts(&self) -> bool {
        false
    }
}

/// Every forge account PDT has, keyed `<kind>:<host>`: what the keychain holds (as JSON, one entry per instance).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: BTreeMap<String, ForgeAccount>,
}

impl AccountStore {
    /// The key of `kind`'s account on `host`: `github:github.com`.
    pub fn key(kind: &ForgeKind, host: &str) -> String {
        format!("{kind}:{}", host.trim().to_ascii_lowercase())
    }

    pub fn get(&self, kind: &ForgeKind, host: &str) -> Option<&ForgeAccount> {
        self.accounts.get(&Self::key(kind, host))
    }

    /// Adds `kind`'s account on `host`, replacing the one it had.
    pub fn put(&mut self, kind: &ForgeKind, host: &str, account: ForgeAccount) {
        self.accounts.insert(Self::key(kind, host), account);
    }

    pub fn remove(&mut self, kind: &ForgeKind, host: &str) -> Option<ForgeAccount> {
        self.accounts.remove(&Self::key(kind, host))
    }

    /// `kind`'s accounts, by host.
    pub fn of_kind<'a>(&'a self, kind: &ForgeKind) -> impl Iterator<Item = (&'a str, &'a ForgeAccount)> + 'a {
        let prefix = format!("{kind}:");
        self.accounts.iter().filter_map(move |(k, a)| k.strip_prefix(prefix.as_str()).map(|h| (h, a)))
    }

    /// The store a single login of `kind` on `host` from before PDT-244 becomes.
    pub fn migrated(kind: &ForgeKind, host: &str, account: ForgeAccount) -> Self {
        let mut s = Self::default();
        s.put(kind, host, account);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(login: &str, api: &str) -> ForgeAccount {
        ForgeAccount { token: format!("t-{login}"), api_url: api.into(), login: login.into(), signed_in_at: Some(1) }
    }

    #[test]
    fn accounts_are_kept_per_forge_and_host() {
        let gitlab = ForgeKind::new("gitlab").unwrap();
        let mut s = AccountStore::default();
        s.put(&ForgeKind::GITHUB, "github.com", acct("ross", "https://api.github.com"));
        s.put(&ForgeKind::GITHUB, "GHE.example.com", acct("rwest", "https://ghe.example.com/api/v3"));
        s.put(&gitlab, "gitlab.com", acct("rw", "https://gitlab.com/api/v4"));
        assert_eq!(s.get(&ForgeKind::GITHUB, "ghe.example.com").unwrap().login, "rwest", "hosts in any case");
        assert_eq!(s.get(&ForgeKind::GITHUB, "github.com").unwrap().login, "ross");
        assert!(s.get(&gitlab, "github.com").is_none());
        let hosts: Vec<_> = s.of_kind(&ForgeKind::GITHUB).map(|(h, a)| (h, a.login.as_str())).collect();
        assert_eq!(hosts, [("ghe.example.com", "rwest"), ("github.com", "ross")]);

        s.put(&ForgeKind::GITHUB, "github.com", acct("other", "https://api.github.com"));
        assert_eq!(s.of_kind(&ForgeKind::GITHUB).count(), 2, "replaced, not added");
        assert_eq!(s.remove(&ForgeKind::GITHUB, "github.com").unwrap().login, "other");
        assert_eq!(s.of_kind(&ForgeKind::GITHUB).count(), 1);
    }

    #[test]
    fn the_store_round_trips_and_reads_an_old_login() {
        let old = r#"{"token":"t","apiUrl":"https://api.github.com","login":"ross"}"#;
        let a: ForgeAccount = serde_json::from_str(old).unwrap();
        assert_eq!(a.signed_in_at, None, "a login from before PDT-252");
        let s = AccountStore::migrated(&ForgeKind::GITHUB, "github.com", a.clone());
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""github:github.com""#), "{json}");
        assert_eq!(serde_json::from_str::<AccountStore>(&json).unwrap(), s);
        assert_eq!(serde_json::from_str::<AccountStore>("{}").unwrap(), AccountStore::default());
    }
}
