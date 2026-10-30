//! The GitLab client (PDT-246): GitLab's REST API (v4) signed in with one host's account. Every call names the
//! project by its URL-encoded path (`group/sub/app` → `group%2Fsub%2Fapp`), the forge's name for the repository. A used-up
//! rate limit stops every call until it resets; an OAuth token is renewed with its refresh token before it expires;
//! a token GitLab refuses (and refuses again when asked `GET /user`) marks the login rejected until the user signs
//! in again. Blocking; call from `spawn_blocking` in async code.

use crate::login;
use pdt_core::brand::SHORT_NAME;
use pdt_forge::transport::{Request, Response, Transport, encode};
use pdt_forge::{ForgeAccount, ForgeError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// The most pages one list read follows.
pub(crate) const MAX_PAGES: usize = 10;
/// How long a project's merge settings (merge trains on or off, merge method, squash) are kept.
const PROJECT_TTL_MS: i64 = 5 * 60_000;
/// How long before its token expires an OAuth login is renewed.
const RENEW_BEFORE_MS: i64 = 15 * 60_000;
/// How long after a refresh that got no answer the next one waits.
const RENEW_RETRY_MS: i64 = 60_000;
/// GitLab's access level of a Developer: a member at it or above can push, and counts as an insider.
const DEVELOPER: u64 = 30;

/// The text of a call made before the user connected GitLab.
pub fn not_connected() -> String {
    format!("GitLab is not connected: connect it in {SHORT_NAME}'s Settings › Plugins › GitLab")
}

/// What went wrong talking to GitLab.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GlError {
    #[error("{}", not_connected())]
    NotConnected,
    #[error("GitLab no longer accepts {SHORT_NAME}'s login: connect GitLab again in Settings › Plugins › GitLab")]
    Rejected,
    #[error("GitLab answered {0}: {1}")]
    Status(u16, String),
    #[error("GitLab's rate limit is used up until {0}")]
    RateLimited(String),
    #[error("GitLab could not be reached: {0}")]
    Network(String),
    #[error("unexpected answer from GitLab: {0}")]
    BadAnswer(String),
}

impl From<GlError> for ForgeError {
    fn from(e: GlError) -> Self {
        match e {
            GlError::NotConnected | GlError::Rejected => ForgeError::Unavailable(e.to_string()),
            _ => ForgeError::Failed(e.to_string()),
        }
    }
}

/// Called with the token a renewal replaced and the renewed account, to store it.
pub type RenewedHook = Arc<dyn Fn(&str, &ForgeAccount) + Send + Sync>;

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

/// What [`GitlabApi::check_login`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCheck {
    Ok,
    /// GitLab accepts a login it had refused: the mark is cleared.
    Recovered,
    Rejected,
    /// No answer, or one that says nothing about the login.
    Failed(String),
}

/// How one try of a call ended.
enum Once {
    Done(Result<Response, GlError>),
    /// A 401, with GitLab's message.
    Refused(String),
}

/// One GitLab host's client. With no account every call fails as [`GlError::NotConnected`].
pub struct GitlabApi {
    transport: Arc<dyn Transport>,
    account: RwLock<Option<ForgeAccount>>,
    /// When the rate limit resets (Unix seconds), while it is used up.
    limited_until: Mutex<Option<u64>>,
    /// A call wanted GitLab while it was not connected.
    wanted: AtomicBool,
    /// When GitLab refused the login (Unix ms): no call is sent until the user signs in again.
    rejected_at: Mutex<Option<i64>>,
    /// Held while the token is renewed: the first refresh spends the refresh token.
    renewing: Mutex<()>,
    renew_after: Mutex<Option<i64>>,
    on_renewed: RwLock<Option<RenewedHook>>,
    /// Projects' merge settings by repo, with when they were read (Unix ms).
    projects: Mutex<HashMap<String, (i64, ProjectInfo)>>,
    /// Whether a user (by id) is an insider on a repo.
    members: Mutex<HashMap<(String, u64), bool>>,
}

impl GitlabApi {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            account: RwLock::new(None),
            limited_until: Mutex::default(),
            wanted: AtomicBool::new(false),
            rejected_at: Mutex::default(),
            renewing: Mutex::default(),
            renew_after: Mutex::default(),
            on_renewed: RwLock::default(),
            projects: Mutex::default(),
            members: Mutex::default(),
        }
    }

    /// No network and no login, for tests.
    pub fn offline() -> Self {
        Self::new(Arc::new(pdt_forge::transport::Offline))
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    pub fn account(&self) -> Option<ForgeAccount> {
        self.account.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Signs in (or out, with None); what the old login learned goes with it.
    pub fn set_account(&self, account: Option<ForgeAccount>) {
        *self.account.write().unwrap_or_else(|e| e.into_inner()) = account;
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.wanted.store(false, Ordering::SeqCst);
        self.projects.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.members.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Sets what stores the account once its token is renewed.
    pub fn on_renewed(&self, hook: Option<RenewedHook>) {
        *self.on_renewed.write().unwrap_or_else(|e| e.into_inner()) = hook;
    }

    pub fn rejected_at(&self) -> Option<i64> {
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn wanted(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// The website of the account's host (`https://gitlab.com`).
    pub(crate) fn web(&self) -> String {
        login::web_url(&self.account().map(|a| a.api_url).unwrap_or_default())
    }

    /// GitLab refused `token`. A call in flight when the user signed in again doesn't mark the new login.
    fn reject(&self, token: &str) {
        let account = self.account.read().unwrap_or_else(|e| e.into_inner());
        if account.as_ref().is_some_and(|a| a.token == token) {
            let mut at = self.rejected_at.lock().unwrap_or_else(|e| e.into_inner());
            if at.is_none() {
                tracing::warn!(login = %account.as_ref().map_or("", |a| a.login.as_str()), "GitLab refuses the login: no more calls until the user signs in again");
            }
            at.get_or_insert_with(now_ms);
        }
    }

    /// Renews the OAuth token of `stale` with its refresh token, and stores the new account at once (GitLab's refresh
    /// tokens are single-use). One renewal at a time; one that finds the token already renewed returns that account.
    fn renew(&self, stale: &ForgeAccount) -> Result<ForgeAccount, GlError> {
        let _held = self.renewing.lock().unwrap_or_else(|e| e.into_inner());
        let cur = self.account().ok_or(GlError::NotConnected)?;
        if cur.token != stale.token {
            return Ok(cur);
        }
        let Some(r) = cur.refresh.clone() else { return Err(GlError::Rejected) };
        let now = now_ms();
        if self.renew_after.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| now < t) {
            return Err(GlError::Network("the last try to renew the login got no answer".into()));
        }
        let host = login::web_host(&cur.api_url);
        match login::refresh_token(self.transport.as_ref(), &cur.api_url, &r.client_id, &r.token) {
            Ok(g) => {
                let (expires_at, refresh) = g.renewal(now_ms(), &r.client_id);
                let new = ForgeAccount { token: g.token, expires_at, refresh, ..cur.clone() };
                {
                    let mut account = self.account.write().unwrap_or_else(|e| e.into_inner());
                    if account.as_ref().is_none_or(|a| a.token != cur.token) {
                        return account.clone().ok_or(GlError::NotConnected);
                    }
                    *account = Some(new.clone());
                }
                *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = None;
                tracing::info!(login = %new.login, host, "renewed the GitLab login's token");
                let hook = self.on_renewed.read().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(hook) = hook {
                    hook(&cur.token, &new);
                }
                Ok(new)
            }
            Err(login::RefreshError::Refused(e)) => {
                tracing::warn!(login = %cur.login, host, error = %e, "GitLab refused to renew the login: connect GitLab again");
                self.reject(&cur.token);
                Err(GlError::Rejected)
            }
            Err(login::RefreshError::Failed(e)) => {
                tracing::warn!(login = %cur.login, host, error = %e, "renewing the GitLab login got no answer: trying again shortly");
                *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms() + RENEW_RETRY_MS);
                Err(GlError::Network(e))
            }
        }
    }

    /// The login to call with; one whose token expires within [`RENEW_BEFORE_MS`] is renewed first (while renewing
    /// gets no answer, the old token serves until it expires).
    fn signed_in(&self) -> Result<ForgeAccount, GlError> {
        let a = self.account().ok_or_else(|| {
            self.wanted.store(true, Ordering::SeqCst);
            GlError::NotConnected
        })?;
        if self.rejected_at().is_some() {
            return Err(GlError::Rejected);
        }
        match a.expires_at {
            Some(t) if a.refresh.is_some() && now_ms() >= t - RENEW_BEFORE_MS => match self.renew(&a) {
                Err(GlError::Network(_)) if now_ms() < t => Ok(a),
                r => r,
            },
            _ => Ok(a),
        }
    }

    /// The light login check (every few minutes): `GET /user`. A refused login with a refresh token is renewed; one
    /// GitLab accepts again loses its mark. None without a login.
    pub fn check_login(&self) -> Option<LoginCheck> {
        let a = self.account()?;
        let was_rejected = self.rejected_at().is_some();
        if was_rejected && a.refresh.is_none() {
            let req = Request { method: "GET", url: url(&a, "user"), headers: login::headers(&a.token), body: None };
            return Some(match self.transport.send(&req) {
                Ok(res) if (200..300).contains(&res.status) => {
                    let account = self.account.read().unwrap_or_else(|e| e.into_inner());
                    if account.as_ref().is_some_and(|x| x.token == a.token) {
                        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                    LoginCheck::Recovered
                }
                Ok(res) if res.status == 401 => LoginCheck::Rejected,
                Ok(res) => LoginCheck::Failed(format!("GitLab answered {}", res.status)),
                Err(e) => LoginCheck::Failed(e),
            });
        }
        if was_rejected {
            return Some(match self.renew(&a) {
                Ok(_) => LoginCheck::Recovered,
                Err(GlError::Rejected) => LoginCheck::Rejected,
                Err(e) => LoginCheck::Failed(e.to_string()),
            });
        }
        Some(match self.call("GET", "user", None) {
            Ok(_) => LoginCheck::Ok,
            Err(GlError::Rejected) => LoginCheck::Rejected,
            Err(e) => LoginCheck::Failed(e.to_string()),
        })
    }

    fn check_limit(&self) -> Result<(), GlError> {
        let mut until = self.limited_until.lock().unwrap_or_else(|e| e.into_inner());
        match *until {
            Some(t) if t > now_secs() => Err(GlError::RateLimited(clock(t))),
            Some(_) => {
                *until = None;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Notes a used-up rate limit (`RateLimit-Remaining: 0`, or a 429 with `Retry-After`): Err when this answer is
    /// the refusal.
    fn note_limit(&self, res: &Response) -> Result<(), GlError> {
        let refused = res.status == 429;
        let until = match (res.header("retry-after"), res.header("ratelimit-remaining")) {
            (Some(after), _) if refused => after.trim().parse::<u64>().ok().map(|s| now_secs() + s.max(1)),
            (_, Some("0")) => res.header("ratelimit-reset").and_then(|r| r.trim().parse::<u64>().ok()),
            _ if refused => Some(now_secs() + 60),
            _ => None,
        };
        let Some(until) = until else { return Ok(()) };
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
        if refused { Err(GlError::RateLimited(clock(until))) } else { Ok(()) }
    }

    /// One call: a JSON `body` for POST/PUT. Any 2xx is Ok.
    pub(crate) fn call(&self, method: &'static str, path: &str, body: Option<&Value>) -> Result<Response, GlError> {
        let mut a = self.signed_in()?;
        let mut renewed = false;
        loop {
            match self.call_once(&a, method, path, body) {
                Once::Done(r) => return r,
                Once::Refused(_) if !renewed && a.refresh.is_some() => {
                    renewed = true;
                    a = self.renew(&a)?;
                }
                Once::Refused(m) => {
                    // Only a 401 that `GET /user` gets too marks the login.
                    let req =
                        Request { method: "GET", url: url(&a, "user"), headers: login::headers(&a.token), body: None };
                    if matches!(self.transport.send(&req), Ok(r) if r.status == 401) {
                        self.reject(&a.token);
                        return Err(GlError::Rejected);
                    }
                    return Err(GlError::Status(401, m));
                }
            }
        }
    }

    fn call_once(&self, a: &ForgeAccount, method: &'static str, path: &str, body: Option<&Value>) -> Once {
        if let Err(e) = self.check_limit() {
            return Once::Done(Err(e));
        }
        let mut headers = login::headers(&a.token);
        if body.is_some() {
            headers.push(("Content-Type", "application/json".into()));
        }
        let req = Request { method, url: url(a, path), headers, body: body.map(Value::to_string) };
        let res = match self.transport.send(&req) {
            Ok(res) => res,
            Err(e) => return Once::Done(Err(GlError::Network(e))),
        };
        if let Err(e) = self.note_limit(&res) {
            return Once::Done(Err(e));
        }
        if res.status == 401 {
            return Once::Refused(message(&res));
        }
        if !(200..300).contains(&res.status) {
            return Once::Done(Err(GlError::Status(res.status, message(&res))));
        }
        Once::Done(Ok(res))
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value, GlError> {
        json(&self.call("GET", path, None)?.body)
    }

    pub(crate) fn send_json(&self, method: &'static str, path: &str, body: Value) -> Result<Value, GlError> {
        let res = self.call(method, path, Some(&body))?;
        if res.body.trim().is_empty() { Ok(Value::Null) } else { json(&res.body) }
    }

    /// Every item of a list, following its `Link: next` pages (at most [`MAX_PAGES`]).
    pub(crate) fn get_pages(&self, path: &str) -> Result<Vec<Value>, GlError> {
        let mut out = vec![];
        let mut next = Some(path.to_string());
        for _ in 0..MAX_PAGES {
            let Some(p) = next.take() else { break };
            let res = self.call("GET", &p, None)?;
            match json(&res.body)? {
                Value::Array(items) => out.extend(items),
                _ => return Err(GlError::BadAnswer("a list is not an array".into())),
            }
            next = res.header("link").and_then(next_link);
        }
        Ok(out)
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

fn url(a: &ForgeAccount, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.into()
    } else {
        format!("{}/{path}", login::api_url(&a.api_url))
    }
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

/// The `rel="next"` URL of a `Link` header.
pub fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, rest) = part.split_once(';')?;
        rest.contains("rel=\"next\"").then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

pub(crate) fn json(out: &str) -> Result<Value, GlError> {
    serde_json::from_str(out).map_err(|e| GlError::BadAnswer(e.to_string()))
}

pub(crate) fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A reset time for a message: `HH:MM UTC`.
fn clock(t: u64) -> String {
    let day = t % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, (day % 3600) / 60)
}
