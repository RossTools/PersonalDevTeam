//! The GitLab client (PDT-246): GitLab's REST API (v4) signed in with one host's account, through the forges'
//! shared client ([`RestClient`]: rate limit, token renewal, refused logins, paging). Every call names the project by
//! its URL-encoded path (`group/sub/app` → `group%2Fsub%2Fapp`), the forge's name for the repository. Blocking; call
//! from `spawn_blocking` in async code.

use crate::login;
use pdt_forge::ForgeAccount;
use pdt_forge::oauth::OAuthApp;
use pdt_forge::rest::{Flavor, RestClient, RestError};
pub use pdt_forge::rest::{LoginCheck, RenewedHook, now_ms};
use pdt_forge::transport::{Response, Transport, encode};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// How long a project's merge settings (merge trains on or off, merge method, squash) are kept.
const PROJECT_TTL_MS: i64 = 5 * 60_000;
/// GitLab's access level of a Developer: a member at it or above can push, and counts as an insider.
const DEVELOPER: u64 = 30;

/// GitLab's REST API, for the shared client: names, URLs, headers and error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gitlab;

impl Flavor for Gitlab {
    const NAME: &'static str = "GitLab";
    const LIMIT_REFUSALS: &'static [u16] = &[429];
    const LIMIT_HEADERS: &'static str = "ratelimit-";
    const REFUSAL_HEADERS: &'static [&'static str] = &["x-request-id", "www-authenticate"];
    fn api_url(given: &str) -> String {
        login::api_url(given)
    }
    fn web_host(api: &str) -> String {
        login::web_host(api)
    }
    fn headers(token: &str) -> Vec<(&'static str, String)> {
        login::headers(token)
    }
    fn message(res: &Response) -> String {
        message(res)
    }
    fn oauth(api: &str) -> OAuthApp {
        login::oauth_app(api)
    }
}

/// What went wrong talking to GitLab.
pub type GlError = RestError<Gitlab>;

/// How a project merges, as `GET /projects/:id` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectInfo {
    /// Merge trains are on (Premium and Ultimate, with merged-results pipelines).
    pub merge_trains: bool,
    /// Merge requests' pipelines run on the merged result (`refs/merge-requests/:iid/merge`), not on their head.
    pub merged_results: bool,
    /// `merge`, `rebase_merge` or `ff`.
    pub merge_method: String,
    /// `never`, `always`, `default_on` or `default_off`.
    pub squash_option: String,
}

/// One GitLab host's client: the shared REST client (which it derefs to), and what it learned of projects. With no
/// account every call fails as [`GlError::NotConnected`].
pub struct GitlabApi {
    rest: RestClient<Gitlab>,
    /// Projects' merge settings by repo, with when they were read (Unix ms).
    projects: Mutex<HashMap<String, (i64, ProjectInfo)>>,
    /// Whether a user (by id) is an insider on a repo.
    members: Mutex<HashMap<(String, u64), bool>>,
}

impl std::ops::Deref for GitlabApi {
    type Target = RestClient<Gitlab>;
    fn deref(&self) -> &RestClient<Gitlab> {
        &self.rest
    }
}

impl GitlabApi {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { rest: RestClient::new(transport), projects: Mutex::default(), members: Mutex::default() }
    }

    /// No network and no login, for tests.
    pub fn offline() -> Self {
        Self::new(Arc::new(pdt_forge::transport::Offline))
    }

    /// Signs in (or out, with None); what the old login learned goes with it.
    pub fn set_account(&self, account: Option<ForgeAccount>) {
        self.rest.set_account(account);
        self.projects.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.members.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// The website of the account's host (`https://gitlab.com`).
    pub(crate) fn web(&self) -> String {
        login::web_url(&self.account().map(|a| a.api_url).unwrap_or_default())
    }

    /// `repo`'s merge settings, read at most every [`PROJECT_TTL_MS`].
    pub(crate) fn project(&self, repo: &str) -> Result<ProjectInfo, GlError> {
        let now = now_ms();
        if let Some((at, p)) = self.projects.lock().unwrap_or_else(|e| e.into_inner()).get(repo)
            && now - at < PROJECT_TTL_MS
        {
            return Ok(p.clone());
        }
        let p = parse_project(&self.get(&project_path(repo))?);
        self.projects.lock().unwrap_or_else(|e| e.into_inner()).insert(repo.into(), (now, p.clone()));
        Ok(p)
    }

    /// Whether `user` (an author object) is an insider on `repo`: a member at Developer or above, directly or through
    /// a group. Asked once per user and repo.
    pub(crate) fn insider(&self, repo: &str, user: &Value) -> bool {
        let Some(id) = user["id"].as_u64() else { return false };
        let key = (repo.to_string(), id);
        if let Some(m) = self.members.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return *m;
        }
        let m = match self.get(&format!("{}/members/all/{id}", project_path(repo))) {
            Ok(v) => v["access_level"].as_u64().is_some_and(|l| l >= DEVELOPER),
            Err(GlError::Status(404, _)) => false,
            // Unknown: not an insider this time, and asked again next time.
            Err(_) => return false,
        };
        self.members.lock().unwrap_or_else(|e| e.into_inner()).insert(key, m);
        m
    }
}

/// `projects/<the URL-encoded path>`: how the API names a project.
pub(crate) fn project_path(repo: &str) -> String {
    format!("projects/{}", encode(repo))
}

/// `GET /projects/:id`'s merge settings.
pub fn parse_project(v: &Value) -> ProjectInfo {
    let merged_results = v["merge_pipelines_enabled"].as_bool().unwrap_or(false);
    ProjectInfo {
        merge_trains: merged_results && v["merge_trains_enabled"].as_bool().unwrap_or(false),
        merged_results,
        merge_method: v["merge_method"].as_str().unwrap_or("merge").into(),
        squash_option: v["squash_option"].as_str().unwrap_or("default_off").into(),
    }
}

/// An error answer's message: GitLab's `message` is a string, a list, or fields with lists (`{"base": ["…"]}`);
/// OAuth's is `error_description`.
pub(crate) fn message(res: &Response) -> String {
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    fn text(v: &Value) -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => Some(a.iter().filter_map(text).collect::<Vec<_>>().join("; ")),
            Value::Object(o) => {
                Some(o.iter().filter_map(|(k, v)| text(v).map(|t| format!("{k} {t}"))).collect::<Vec<_>>().join("; "))
            }
            _ => None,
        }
    }
    text(&v["message"])
        .or_else(|| text(&v["error_description"]))
        .or_else(|| text(&v["error"]))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "no message".into())
}

pub(crate) fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}
