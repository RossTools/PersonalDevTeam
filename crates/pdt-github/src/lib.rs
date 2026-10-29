//! The GitHub forge (PDT-176, PDT-177): GitHub's REST and GraphQL APIs for a branch's pull request and its Actions
//! runs (PDT-25), merges, reviews, comments, issues and releases (PDT-194: reading them, starting the release workflow), signed in with the token of the user's in-app login (Settings
//! › Plugins › GitHub). No GitHub CLI. Check runs are not read: fine-grained tokens cannot see them, so CI comes from
//! the Actions runs API. GETs are conditional (ETags: an unchanged answer costs no rate limit), and a used-up rate
//! limit stops every call until it resets. Blocking; call from `spawn_blocking` in async code.

pub mod login;
pub mod plugin;
pub mod transport;

use pdt_core::brand::SHORT_NAME;
use pdt_core::pr::{CiRun, Mergeable, PrState, QueueRemoval};
pub use pdt_forge::{
    Capabilities, ChangeComment, ChangeRequest, ChangeReview, CloseReason, Forge, ForgeError, ForgeNotes, Issue,
    IssueDetail, JobLog, MergeSettings, MergeStrategy, NewChangeRequest, NewIssue, OpenedChangeRequest, Release,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use transport::{Request, Response, Transport, encode};

/// How many recent runs are read per branch (and of merge-queue runs per repo).
const RUN_LIMIT: u32 = 30;
/// The most pages one list read follows.
const MAX_PAGES: usize = 10;
/// The most GET answers kept for conditional requests.
const ETAG_CACHE: usize = 500;
/// The most recent calls kept, for the log of a refused login (PDT-252).
const RECENT_CALLS: usize = 50;

const PR_FIELDS: &str = "number url title state createdAt isDraft headRefOid baseRefName mergeable \
mergeStateStatus isInMergeQueue mergeQueueEntry{position} autoMergeRequest{enabledAt} \
timelineItems(itemTypes:[ADDED_TO_MERGE_QUEUE_EVENT,REMOVED_FROM_MERGE_QUEUE_EVENT],last:1){nodes{__typename \
...on RemovedFromMergeQueueEvent{id reason actor{login}}}}";

fn pr_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$head:String!){{repository(owner:$owner,name:$name){{\
         pullRequests(headRefName:$head,first:5,orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{{PR_FIELDS}}}}}}}}}"
    )
}

fn pr_number_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$number:Int!){{repository(owner:$owner,name:$name){{\
         pullRequest(number:$number){{{PR_FIELDS}}}}}}}"
    )
}

const MERGE_SETTINGS_QUERY: &str = "query($owner:String!,$name:String!,$base:String!){repository(owner:$owner,\
name:$name){squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed mergeQueue(branch:$base){id}}}";

const PR_ID_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){id}}}";

const AUTO_MERGE: &str = "mutation($id:ID!,$method:PullRequestMergeMethod){enablePullRequestAutoMerge(input:\
{pullRequestId:$id,mergeMethod:$method}){clientMutationId}}";

const ENQUEUE: &str = "mutation($id:ID!){enqueuePullRequest(input:{pullRequestId:$id}){clientMutationId}}";

/// The text of a call made before the user connected GitHub.
pub fn not_connected() -> String {
    format!("GitHub is not connected: connect it in {SHORT_NAME}'s Settings › Plugins › GitHub")
}

/// What went wrong talking to GitHub.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GhError {
    #[error("{}", not_connected())]
    NotConnected,
    #[error("GitHub no longer accepts {SHORT_NAME}'s login: connect GitHub again in Settings › Plugins › GitHub")]
    Rejected,
    #[error("GitHub answered {0}: {1}")]
    Status(u16, String),
    #[error("GitHub's rate limit is used up until {0}")]
    RateLimited(String),
    #[error("GitHub could not be reached: {0}")]
    Network(String),
    #[error("unexpected answer from GitHub: {0}")]
    BadAnswer(String),
}

impl From<GhError> for ForgeError {
    fn from(e: GhError) -> Self {
        match e {
            GhError::NotConnected | GhError::Rejected => ForgeError::Unavailable(e.to_string()),
            _ => ForgeError::Failed(e.to_string()),
        }
    }
}

/// GitHub's part of agents' instructions: PDT's forge tools (pdt-mcp), not the GitHub CLI.
pub fn agent_notes() -> ForgeNotes {
    ForgeNotes {
        ci: Some(format!(
            "Read CI with {SHORT_NAME}'s tools: `ci_runs` (the runs on {{branch}} and their failed jobs) and \
             `ci_failed_logs` (the failed jobs' logs), or from the ticket's attempt (`pr.ci`: state, URL and runs, \
             when {SHORT_NAME} has seen the pull request). `change_request_view` shows the pull request with its \
             comments and reviews, `issue_view` an issue. Use these instead of the GitHub CLI (`gh`)."
        )),
        reply: Some("`change_request_comment`, with `replyTo` to answer a review comment".into()),
        watch: None,
    }
}

/// Where GitHub Actions workflows live.
pub const CI_DIRS: &[&str] = &[".github/workflows"];

/// What GitHub can do: everything.
pub const CAPABILITIES: Capabilities = Capabilities::ALL;

/// The longest comment PDT sends (GitHub's own limit is 65536).
pub const MAX_COMMENT: usize = 60_000;

/// Issues (and pull requests, which share their numbers) per page of [`GithubApi::recent_issues`], and the most pages
/// one read takes.
pub const ISSUE_PAGE: usize = 100;
pub const ISSUE_PAGES: u32 = 10;

/// The most lines of a failed job's log kept, and of characters.
pub const LOG_LINES: usize = 200;
pub const LOG_CHARS: usize = 30_000;

/// The headers of an API call with `token`.
pub(crate) fn headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/vnd.github+json".into()),
        ("X-GitHub-Api-Version", "2022-11-28".into()),
    ]
}

/// The login PDT uses: a token and the API it is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub token: String,
    /// github.com's API, or a GitHub Enterprise Server's (`https://ghe.example.com/api/v3`).
    pub api_url: String,
    /// Who the token signs in as.
    pub login: String,
    /// When the user signed in (Unix ms); unknown for a login stored before PDT-252.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in_at: Option<i64>,
}

/// One call to GitHub, for the log of a refused login: never its token, query or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    /// Unix ms.
    pub at: i64,
    pub method: &'static str,
    pub path: String,
    /// None: no answer (a network failure).
    pub status: Option<u16>,
}

/// What [`GithubApi::check_login`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCheck {
    /// GitHub accepts the login.
    Ok,
    /// GitHub accepts a login it had refused: the mark is cleared.
    Recovered,
    /// GitHub refuses the login.
    Rejected,
    /// No answer, or one that says nothing about the login.
    Failed(String),
}

/// The GitHub forge. With no [`Account`] every call fails as [`GhError::NotConnected`].
pub struct GithubApi {
    transport: Arc<dyn Transport>,
    account: RwLock<Option<Account>>,
    /// GET answers by URL: ETag and body.
    etags: Mutex<HashMap<String, (String, String)>>,
    /// When the rate limit resets (Unix seconds), while it is used up.
    limited_until: Mutex<Option<u64>>,
    /// A call wanted GitHub while it was not connected.
    wanted: AtomicBool,
    /// When GitHub refused the login (401, Unix ms): it is kept, but no call is sent until the user signs in again.
    rejected_at: Mutex<Option<i64>>,
    /// The last [`RECENT_CALLS`] calls, oldest first.
    recent: Mutex<VecDeque<CallRecord>>,
    /// When GitHub last accepted the login (Unix ms).
    last_ok_at: Mutex<Option<i64>>,
    /// The recent calls were logged for this login's first 401.
    logged_calls: AtomicBool,
}

impl GithubApi {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            account: RwLock::new(None),
            etags: Mutex::default(),
            limited_until: Mutex::default(),
            wanted: AtomicBool::new(false),
            rejected_at: Mutex::default(),
            recent: Mutex::default(),
            last_ok_at: Mutex::default(),
            logged_calls: AtomicBool::new(false),
        }
    }

    /// No network and no login: `AppState::new`'s forge, so tests can never reach GitHub.
    pub fn offline() -> Self {
        Self::new(Arc::new(transport::Offline))
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    pub fn account(&self) -> Option<Account> {
        self.account.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Signs in (or out, with None). Cached answers belong to the old login, so they go, and so does a refusal.
    pub fn set_account(&self, account: Option<Account>) {
        *self.account.write().unwrap_or_else(|e| e.into_inner()) = account;
        self.etags.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.last_ok_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.logged_calls.store(false, Ordering::SeqCst);
        self.wanted.store(false, Ordering::SeqCst);
    }

    /// The last calls to GitHub, oldest first.
    pub fn recent_calls(&self) -> Vec<CallRecord> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    /// Sends one request and keeps a record of it.
    fn send(&self, req: &Request) -> Result<Response, String> {
        let res = self.transport.send(req);
        let status = res.as_ref().ok().map(|r| r.status);
        let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.len() >= RECENT_CALLS {
            recent.pop_front();
        }
        recent.push_back(CallRecord { at: now_ms(), method: req.method, path: path_of(&req.url), status });
        drop(recent);
        if matches!(status, Some(200..=299 | 304)) {
            *self.last_ok_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms());
        }
        res
    }

    /// Logs a 401 or 403 (PDT-252): the call, GitHub's answer and the login's age; never the token. The first 401 of
    /// a login also logs the recent calls.
    fn log_refusal(&self, a: &Account, req: &Request, res: &Response) {
        let h = |name| res.header(name).unwrap_or("-").to_string();
        let now = now_ms();
        let ago = |t: Option<i64>| t.map_or_else(|| "unknown".into(), |t| minutes(now - t));
        let last_ok = *self.last_ok_at.lock().unwrap_or_else(|e| e.into_inner());
        let fields = format!(
            "method={} endpoint={} final_url={} message={:?} request_id={} sso={:?} www_authenticate={:?} \
             scopes={:?} login={} login_age={} last_worked={}",
            req.method,
            bare_url(&req.url),
            if res.url.is_empty() { bare_url(&req.url) } else { res.url.clone() },
            message(res),
            h("x-github-request-id"),
            h("x-github-sso"),
            h("www-authenticate"),
            h("x-oauth-scopes"),
            a.login,
            ago(a.signed_in_at),
            last_ok.map_or_else(
                || "never (since pdtd started or the login was made)".into(),
                |t| format!("{} ago", minutes(now - t))
            ),
        );
        if res.status == 403 {
            tracing::info!("GitHub answered 403: {fields}");
            return;
        }
        tracing::warn!("GitHub answered 401: {fields}");
        if !self.logged_calls.swap(true, Ordering::SeqCst) {
            let calls: Vec<String> = self
                .recent_calls()
                .iter()
                .map(|c| {
                    let status = c.status.map_or_else(|| "no answer".into(), |s| s.to_string());
                    format!("{} {} {} {}", clock_ms(c.at), c.method, c.path, status)
                })
                .collect();
            tracing::warn!("the last {} GitHub calls before the first 401:\n{}", calls.len(), calls.join("\n"));
        }
    }

    /// After a 401: whether GitHub really refuses the login, asked again with `GET /user`. Only a 401 from the API's
    /// own host counts; no answer doesn't (the login check asks again later).
    fn confirm_refused(&self, a: &Account) -> bool {
        let req = Request {
            method: "GET",
            url: format!("{}/user", login::api_url(&a.api_url)),
            headers: headers(&a.token),
            body: None,
        };
        match self.send(&req) {
            Ok(res) if res.status == 401 && same_host(&res.url, &req.url) => true,
            Ok(res) => {
                tracing::warn!(status = res.status, "GitHub refused one call, but accepts the login when asked again");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "GitHub refused one call, and could not be asked again");
                false
            }
        }
    }

    /// The light login check (PDT-252), every few minutes: `GET /rate_limit` (it costs no rate limit), so the time
    /// GitHub stops accepting the login is known to the minute. While the login is marked rejected, `GET /user`
    /// instead, and the mark goes when GitHub accepts it again. None without a login.
    pub fn check_login(&self) -> Option<LoginCheck> {
        let a = self.account()?;
        if self.rejected_at().is_none() {
            return Some(match self.call("GET", "rate_limit", None, false) {
                Ok(_) => LoginCheck::Ok,
                Err(GhError::Rejected) => LoginCheck::Rejected,
                Err(e) => LoginCheck::Failed(e.to_string()),
            });
        }
        let req = Request {
            method: "GET",
            url: format!("{}/user", login::api_url(&a.api_url)),
            headers: headers(&a.token),
            body: None,
        };
        Some(match self.send(&req) {
            Ok(res) if (200..300).contains(&res.status) => {
                let account = self.account.read().unwrap_or_else(|e| e.into_inner());
                if account.as_ref().is_some_and(|x| x.token == a.token) {
                    *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    self.logged_calls.store(false, Ordering::SeqCst);
                }
                tracing::info!(login = %a.login, "GitHub accepts the login it had refused again");
                LoginCheck::Recovered
            }
            Ok(res) if res.status == 401 => LoginCheck::Rejected,
            Ok(res) => LoginCheck::Failed(format!("GitHub answered {}: {}", res.status, message(&res))),
            Err(e) => LoginCheck::Failed(e),
        })
    }

    /// When GitHub stopped accepting the stored login (Unix ms), until the user signs in again.
    pub fn rejected_at(&self) -> Option<i64> {
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The login to call with: none, or one GitHub refused, sends nothing.
    fn signed_in(&self) -> Result<Account, GhError> {
        let a = self.account().ok_or_else(|| {
            self.wanted.store(true, Ordering::SeqCst);
            GhError::NotConnected
        })?;
        if self.rejected_at().is_some() {
            return Err(GhError::Rejected);
        }
        Ok(a)
    }

    /// GitHub answered 401 to `token`. A call in flight when the user signed in again doesn't mark the new login.
    fn reject(&self, token: &str) {
        let account = self.account.read().unwrap_or_else(|e| e.into_inner());
        if account.as_ref().is_some_and(|a| a.token == token) {
            let mut at = self.rejected_at.lock().unwrap_or_else(|e| e.into_inner());
            if at.is_none() {
                tracing::warn!(login = %account.as_ref().map_or("", |a| a.login.as_str()), "GitHub refuses the login: no more calls until it accepts it again or the user signs in again");
            }
            at.get_or_insert_with(now_ms);
        }
    }

    fn url(a: &Account, path: &str) -> String {
        if path.starts_with("https://") || path.starts_with("http://") {
            path.into()
        } else {
            format!("{}/{path}", login::api_url(&a.api_url))
        }
    }

    fn check_limit(&self) -> Result<(), GhError> {
        let mut until = self.limited_until.lock().unwrap_or_else(|e| e.into_inner());
        match *until {
            Some(t) if t > now_secs() => Err(GhError::RateLimited(clock(t))),
            Some(_) => {
                *until = None;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Notes a used-up rate limit from an answer: Err when this answer is the refusal.
    fn note_limit(&self, res: &Response) -> Result<(), GhError> {
        let refused = matches!(res.status, 403 | 429);
        let until = match (res.header("retry-after"), res.header("x-ratelimit-remaining")) {
            (Some(after), _) if refused => after.trim().parse::<u64>().ok().map(|s| now_secs() + s.max(1)),
            (_, Some("0")) => res.header("x-ratelimit-reset").and_then(|r| r.trim().parse::<u64>().ok()),
            _ => None,
        };
        let Some(until) = until else { return Ok(()) };
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
        if refused { Err(GhError::RateLimited(clock(until))) } else { Ok(()) }
    }

    /// One call: a JSON `body` for POST/PATCH/PUT. GETs are conditional when `cache`. Any 2xx is Ok.
    fn call(&self, method: &'static str, path: &str, body: Option<&Value>, cache: bool) -> Result<Response, GhError> {
        let a = self.signed_in()?;
        self.check_limit()?;
        let url = Self::url(&a, path);
        let mut headers = headers(&a.token);
        let cached = if cache && method == "GET" {
            self.etags.lock().unwrap_or_else(|e| e.into_inner()).get(&url).cloned()
        } else {
            None
        };
        if let Some((etag, _)) = &cached {
            headers.push(("If-None-Match", etag.clone()));
        }
        if body.is_some() {
            headers.push(("Content-Type", "application/json".into()));
        }
        let req = Request { method, url: url.clone(), headers, body: body.map(Value::to_string) };
        let mut res = self.send(&req).map_err(GhError::Network)?;
        self.note_limit(&res)?;
        if res.status == 304
            && let Some((_, body)) = cached
        {
            res.status = 200;
            res.body = body;
            return Ok(res);
        }
        if matches!(res.status, 401 | 403) {
            self.log_refusal(&a, &req, &res);
        }
        // Only a 401 from the API's host whose `GET /user` is refused too marks the login: not one from a redirect's
        // other host (a log's storage), nor one GitHub takes back when asked again (PDT-252).
        if res.status == 401 && same_host(&res.url, &url) && self.confirm_refused(&a) {
            self.reject(&a.token);
            return Err(GhError::Rejected);
        }
        if !(200..300).contains(&res.status) {
            return Err(GhError::Status(res.status, message(&res)));
        }
        if cache
            && method == "GET"
            && let Some(etag) = res.header("etag")
        {
            let mut c = self.etags.lock().unwrap_or_else(|e| e.into_inner());
            if c.len() >= ETAG_CACHE {
                c.clear();
            }
            c.insert(url, (etag.to_string(), res.body.clone()));
        }
        Ok(res)
    }

    fn get(&self, path: &str) -> Result<Value, GhError> {
        json(&self.call("GET", path, None, true)?.body)
    }

    fn send_json(&self, method: &'static str, path: &str, body: Value) -> Result<Value, GhError> {
        let res = self.call(method, path, Some(&body), false)?;
        if res.body.trim().is_empty() { Ok(Value::Null) } else { json(&res.body) }
    }

    /// Every item of a list, following its `Link: next` pages (at most [`MAX_PAGES`]).
    fn get_pages(&self, path: &str) -> Result<Vec<Value>, GhError> {
        let mut out = vec![];
        let mut next = Some(path.to_string());
        for _ in 0..MAX_PAGES {
            let Some(p) = next.take() else { break };
            let res = self.call("GET", &p, None, true)?;
            match json(&res.body)? {
                Value::Array(items) => out.extend(items),
                _ => return Err(GhError::BadAnswer("a list is not an array".into())),
            }
            next = res.header("link").and_then(next_link);
        }
        Ok(out)
    }

    /// A GraphQL query's `data`; its first error as [`GhError::Status`].
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, GhError> {
        let a = self.signed_in()?;
        let v =
            self.send_json("POST", &login::graphql_url(&a.api_url), json!({"query": query, "variables": variables}))?;
        graphql_data(v)
    }

    fn pr_node_id(&self, repo: &str, number: u32) -> Result<String, GhError> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql(PR_ID_QUERY, json!({"owner": owner, "name": name, "number": number}))?;
        d["repository"]["pullRequest"]["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| GhError::BadAnswer(format!("no pull request #{number}")))
    }

    fn merge_now(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), GhError> {
        let body = match strategy {
            Some(s) => json!({"merge_method": s.as_str()}),
            None => json!({}),
        };
        self.send_json("PUT", &format!("repos/{repo}/pulls/{number}/merge"), body).map(|_| ())
    }

    fn post_comment(&self, path: &str, body: &str) -> Result<String, GhError> {
        let v = self.send_json("POST", path, json!({"body": comment_body(body)}))?;
        Ok(s(&v, "html_url"))
    }
}

/// Unix seconds.
fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Unix milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// An answer's message (and its first error's), for errors and logs.
fn message(res: &Response) -> String {
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    let mut m = v["message"].as_str().unwrap_or("no message").to_string();
    if let Some(e) = v["errors"].as_array().and_then(|e| e.first()) {
        let more = e["message"].as_str().or(e["code"].as_str()).unwrap_or_default();
        if !more.is_empty() {
            m = format!("{m} ({more})");
        }
    }
    m
}

/// A URL without its query or fragment (a signed link's signature).
fn bare_url(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

/// A URL's path, without host or query.
fn path_of(url: &str) -> String {
    let u = bare_url(url);
    let rest = u.split_once("://").map_or(u.as_str(), |(_, r)| r);
    rest.find('/').map_or_else(|| "/".into(), |i| rest[i..].to_string())
}

/// A URL's scheme, host and port.
fn origin(url: &str) -> &str {
    let start = url.find("://").map_or(0, |i| i + 3);
    url[start..].find('/').map_or(url, |i| &url[..start + i])
}

/// Whether the answer came from the request's own host (an empty answer URL is the request's).
fn same_host(answered: &str, asked: &str) -> bool {
    answered.is_empty() || origin(answered).eq_ignore_ascii_case(origin(asked))
}

/// A duration in ms, for logs: `95 min`.
fn minutes(ms: i64) -> String {
    format!("{} min", ms.max(0) / 60_000)
}

/// A time in Unix ms, for logs: `HH:MM:SS UTC`.
fn clock_ms(ms: i64) -> String {
    let day = (ms / 1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02} UTC", day / 3600, (day % 3600) / 60, day % 60)
}

/// A reset time for a message: `HH:MM UTC`.
fn clock(t: u64) -> String {
    let day = t % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, (day % 3600) / 60)
}

/// The `rel="next"` URL of a `Link` header.
pub fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, rest) = part.split_once(';')?;
        rest.contains("rel=\"next\"").then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

fn owner_name(repo: &str) -> Result<(&str, &str), GhError> {
    repo.split_once('/').ok_or_else(|| GhError::BadAnswer(format!("not owner/name: {repo}")))
}

fn json(out: &str) -> Result<Value, GhError> {
    serde_json::from_str(out).map_err(|e| GhError::BadAnswer(e.to_string()))
}

fn graphql_data(v: Value) -> Result<Value, GhError> {
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(GhError::Status(200, e["message"].as_str().unwrap_or("GraphQL error").to_string()));
    }
    match v.get("data") {
        Some(d) if !d.is_null() => Ok(d.clone()),
        _ => Err(GhError::BadAnswer("no data in the answer".into())),
    }
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

/// A comment cut to [`MAX_COMMENT`] characters.
fn comment_body(body: &str) -> String {
    if body.chars().count() <= MAX_COMMENT {
        return body.to_string();
    }
    let mut out: String = body.chars().take(MAX_COMMENT - 1).collect();
    out.push('…');
    out
}

/// Whether an item's author is an insider on the repository (`author_association` OWNER, MEMBER or COLLABORATOR;
/// PDT-202). Without the field, they are not.
fn collaborator(v: &Value) -> bool {
    matches!(v["author_association"].as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

fn is_bot(user: &Value) -> bool {
    user["type"] == "Bot" || user["login"].as_str().is_some_and(|l| l.ends_with("[bot]"))
}

/// The newest open PR of a `pullRequests` list, else the newest one.
fn pick_pr(nodes: &[Value]) -> Option<&Value> {
    nodes.iter().find(|n| n["state"] == "OPEN").or_else(|| nodes.first())
}

/// The `pr_query` answer's `data`: the newest open PR, else the newest one.
pub fn parse_pull_requests(data: &Value) -> Result<Option<ChangeRequest>, GhError> {
    let nodes = data["repository"]["pullRequests"]["nodes"]
        .as_array()
        .ok_or_else(|| GhError::BadAnswer("no pullRequests in the answer".into()))?;
    Ok(pick_pr(nodes).map(parse_pr))
}

/// One pull request node.
pub fn parse_pr(n: &Value) -> ChangeRequest {
    let state = match n["state"].as_str() {
        Some("OPEN") => PrState::Open,
        Some("MERGED") => PrState::Merged,
        _ => PrState::Closed,
    };
    let mergeable = match n["mergeable"].as_str() {
        Some("MERGEABLE") => Mergeable::Mergeable,
        Some("CONFLICTING") => Mergeable::Conflicting,
        _ => Mergeable::Unknown,
    };
    ChangeRequest {
        number: n["number"].as_u64().and_then(|x| u32::try_from(x).ok()).unwrap_or_default(),
        url: s(n, "url"),
        title: s(n, "title"),
        state,
        draft: n["isDraft"].as_bool().unwrap_or(false),
        head_sha: s(n, "headRefOid"),
        base_branch: s(n, "baseRefName"),
        mergeable,
        behind: n["mergeStateStatus"] == "BEHIND",
        auto_merge: !n["autoMergeRequest"].is_null(),
        in_merge_queue: n["isInMergeQueue"].as_bool().unwrap_or(false),
        queue_position: n["mergeQueueEntry"]["position"].as_u64().and_then(|p| u32::try_from(p).ok()).map(|p| p + 1),
        queue_removal: queue_removal(&n["timelineItems"]["nodes"][0]),
        created_at: n["createdAt"].as_str().and_then(pdt_core::conversation::devin::iso_ms),
    }
}

/// The newest merge-queue event, when it is a removal.
fn queue_removal(e: &Value) -> Option<QueueRemoval> {
    if e["__typename"] != "RemovedFromMergeQueueEvent" {
        return None;
    }
    Some(QueueRemoval {
        id: s(e, "id"),
        reason: s(e, "reason"),
        by_queue: e["actor"]["login"].as_str().is_some_and(QueueRemoval::is_queue_actor),
    })
}

/// A created pull request (`POST …/pulls`).
pub fn parse_opened(v: &Value) -> Result<OpenedChangeRequest, GhError> {
    let number = v["number"].as_u64().and_then(|n| u32::try_from(n).ok());
    let number = number.ok_or_else(|| GhError::BadAnswer("no pull request number in the answer".into()))?;
    Ok(OpenedChangeRequest { number, url: s(v, "html_url") })
}

/// The `MERGE_SETTINGS_QUERY` answer's `data`.
pub fn parse_merge_settings(data: &Value) -> Result<MergeSettings, GhError> {
    let r = &data["repository"];
    if r.is_null() {
        return Err(GhError::BadAnswer("no repository in the answer".into()));
    }
    let b = |k: &str| r[k].as_bool().unwrap_or(false);
    Ok(MergeSettings {
        squash: b("squashMergeAllowed"),
        merge: b("mergeCommitAllowed"),
        rebase: b("rebaseMergeAllowed"),
        queue: r["mergeQueue"].is_object(),
    })
}

/// A workflow runs answer (`GET …/actions/runs`).
pub fn parse_runs(v: &Value) -> Result<Vec<CiRun>, GhError> {
    let list = v["workflow_runs"].as_array().ok_or_else(|| GhError::BadAnswer("no workflow_runs".into()))?;
    Ok(list
        .iter()
        .filter_map(|r| {
            Some(CiRun {
                id: r["id"].as_u64()?,
                attempt: r["run_attempt"].as_u64().and_then(|a| u32::try_from(a).ok()).unwrap_or(1),
                workflow: s(r, "name"),
                state: CiRun::state_of(r["status"].as_str().unwrap_or(""), r["conclusion"].as_str().unwrap_or("")),
                url: s(r, "html_url"),
                head_sha: s(r, "head_sha"),
                head_branch: s(r, "head_branch"),
                created_at: s(r, "created_at"),
                failed_jobs: vec![],
            })
        })
        .collect())
}

fn failed(job: &Value) -> bool {
    matches!(job["conclusion"].as_str(), Some("failure" | "timed_out" | "startup_failure"))
}

/// A jobs answer (`GET …/jobs`): the jobs that failed.
pub fn failed_job_list(v: &Value) -> Result<Vec<Value>, GhError> {
    let jobs = v["jobs"].as_array().ok_or_else(|| GhError::BadAnswer("no jobs in the answer".into()))?;
    Ok(jobs.iter().filter(|j| failed(j)).cloned().collect())
}

/// The end of a job's log: its last [`LOG_LINES`] lines (at most [`LOG_CHARS`] characters), without Actions'
/// timestamps and terminal colours.
pub fn log_tail(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let skipped = lines.len().saturating_sub(LOG_LINES);
    let mut out: String = lines[skipped..].iter().map(|l| format!("{}\n", clean_line(l))).collect();
    let chars = out.chars().count();
    if chars > LOG_CHARS {
        out = out.chars().skip(chars - LOG_CHARS).collect();
    }
    if skipped > 0 {
        out = format!("… ({skipped} earlier lines)\n{out}");
    }
    out
}

fn clean_line(l: &str) -> String {
    // `2026-10-19T02:00:10.1234567Z text`
    let l = match l.split_once(' ') {
        Some((t, rest)) if t.len() >= 20 && t.ends_with('Z') && t.as_bytes()[4] == b'-' && t.contains('T') => rest,
        _ => l,
    };
    let mut out = String::with_capacity(l.len());
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Reads pages of issues (newest first, `page(n)` the items of page `n`) until one reaches issue `after` or is the
/// last: one page without `after`.
pub fn read_issue_pages(
    after: Option<u64>,
    mut page: impl FnMut(u32) -> Result<Vec<Value>, GhError>,
) -> Result<Vec<Issue>, GhError> {
    let mut out = vec![];
    for n in 1..=ISSUE_PAGES {
        let items = page(n)?;
        let lowest = items.iter().filter_map(|v| v["number"].as_u64()).min();
        out.extend(issues_of(&items));
        let reached = match after {
            None => true,
            Some(a) => lowest.is_none_or(|l| l <= a),
        };
        if reached || items.len() < ISSUE_PAGE {
            break;
        }
    }
    Ok(out)
}

fn is_pr(v: &Value) -> bool {
    v.get("pull_request").is_some_and(|p| !p.is_null())
}

fn issues_of(items: &[Value]) -> Vec<Issue> {
    items.iter().filter(|v| !is_pr(v)).filter_map(issue_of).collect()
}

fn issue_of(v: &Value) -> Option<Issue> {
    Some(Issue {
        number: v["number"].as_u64()?,
        title: s(v, "title"),
        body: s(v, "body"),
        url: s(v, "html_url"),
        author: s(&v["user"], "login"),
        labels: v["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["name"].as_str().map(String::from)).collect())
            .unwrap_or_default(),
        created_at: s(v, "created_at"),
    })
}

/// An issue (`GET …/issues/{n}`).
pub fn parse_issue(v: &Value) -> Result<IssueDetail, GhError> {
    let issue = issue_of(v).ok_or_else(|| GhError::BadAnswer("no issue number in the answer".into()))?;
    Ok(IssueDetail {
        issue,
        id: v["id"].as_u64().ok_or_else(|| GhError::BadAnswer("no issue id in the answer".into()))?,
        closed: v["state"] == "closed",
        state_reason: v["state_reason"].as_str().filter(|r| !r.is_empty()).map(String::from),
        change_request: is_pr(v),
    })
}

/// Conversation comments (`in_review: false`) or review comments.
pub fn parse_comments(items: &[Value], in_review: bool) -> Vec<ChangeComment> {
    items
        .iter()
        .filter_map(|v| {
            Some(ChangeComment {
                id: v["id"].as_u64()?,
                author: s(&v["user"], "login"),
                is_bot: is_bot(&v["user"]),
                body: s(v, "body"),
                url: s(v, "html_url"),
                path: v["path"].as_str().map(String::from),
                line: v["line"].as_u64().and_then(|l| u32::try_from(l).ok()),
                in_review,
                collaborator: collaborator(v),
            })
        })
        .collect()
}

pub fn parse_reviews(items: &[Value]) -> Vec<ChangeReview> {
    items
        .iter()
        .filter_map(|v| {
            let state = match v["state"].as_str()? {
                "APPROVED" => "approved",
                "CHANGES_REQUESTED" => "changes_requested",
                "COMMENTED" => "commented",
                _ => return None,
            };
            Some(ChangeReview {
                id: v["id"].as_u64()?,
                author: s(&v["user"], "login"),
                is_bot: is_bot(&v["user"]),
                state: state.into(),
                body: s(v, "body"),
                url: s(v, "html_url"),
                collaborator: collaborator(v),
            })
        })
        .collect()
}

/// A release from `GET /releases/...`; drafts and pre-releases are not releases here.
pub fn parse_release(v: &Value) -> Result<Option<Release>, GhError> {
    parse_release_or_pre(v, false)
}

/// A release, or with `prereleases` a pre-release too; never a draft.
fn parse_release_or_pre(v: &Value, prereleases: bool) -> Result<Option<Release>, GhError> {
    let tag = s(v, "tag_name");
    if tag.is_empty() {
        return Err(GhError::BadAnswer("a release without a tag".into()));
    }
    if v["draft"].as_bool() == Some(true) || (!prereleases && v["prerelease"].as_bool() == Some(true)) {
        return Ok(None);
    }
    Ok(Some(Release {
        name: Some(s(v, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone()),
        tag,
        url: s(v, "html_url"),
        published_at: v["published_at"].as_str().map(String::from),
        notes: s(v, "body"),
    }))
}

/// GraphQL's name for a merge strategy.
fn merge_method(s: MergeStrategy) -> &'static str {
    match s {
        MergeStrategy::Squash => "SQUASH",
        MergeStrategy::Merge => "MERGE",
        MergeStrategy::Rebase => "REBASE",
    }
}

impl Forge for GithubApi {
    fn name(&self) -> &'static str {
        "GitHub"
    }
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn agent_notes(&self) -> ForgeNotes {
        agent_notes()
    }
    fn ci_dirs(&self) -> &'static [&'static str] {
        CI_DIRS
    }
    fn login_needed(&self) -> bool {
        (self.account().is_none() && self.wanted.load(Ordering::SeqCst)) || self.rejected_at().is_some()
    }
    fn login_rejected_at(&self) -> Option<i64> {
        self.rejected_at()
    }
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql(&pr_query(), json!({"owner": owner, "name": name, "head": branch}))?;
        Ok(parse_pull_requests(&d)?)
    }
    fn change_request_number(&self, repo: &str, number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        match self.graphql(&pr_number_query(), json!({"owner": owner, "name": name, "number": number})) {
            Ok(d) => Ok(Some(&d["repository"]["pullRequest"]).filter(|n| n.is_object()).map(parse_pr)),
            Err(GhError::Status(200, m)) if m.contains("Could not resolve to a PullRequest") => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn open_change_request(&self, repo: &str, new: &NewChangeRequest) -> Result<OpenedChangeRequest, ForgeError> {
        let body = json!({
            "title": new.title,
            "head": new.head,
            "base": new.base,
            "body": comment_body(&new.body),
            "draft": new.draft,
        });
        match self.send_json("POST", &format!("repos/{repo}/pulls"), body) {
            Ok(v) => Ok(parse_opened(&v)?),
            // A head GitHub doesn't have, or one with nothing the base lacks.
            Err(GhError::Status(422, m)) => Err(ForgeError::Failed(format!(
                "GitHub refused the pull request: {m}. Is {} pushed, with commits {} doesn't have?",
                new.head, new.base
            ))),
            Err(e) => Err(e.into()),
        }
    }
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
        let path = format!("repos/{repo}/actions/runs?branch={}&per_page={RUN_LIMIT}", encode(branch));
        Ok(parse_runs(&self.get(&path)?)?)
    }
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        Ok(parse_runs(&self.get(&format!("repos/{repo}/actions/runs?event=merge_group&per_page={RUN_LIMIT}"))?)?)
    }
    fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, ForgeError> {
        let v = self.get(&format!("repos/{repo}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100"))?;
        Ok(failed_job_list(&v)?.iter().map(|j| s(j, "name")).collect())
    }
    fn failed_logs(&self, repo: &str, run_id: u64) -> Result<Vec<JobLog>, ForgeError> {
        let v = self.get(&format!("repos/{repo}/actions/runs/{run_id}/jobs?filter=latest&per_page=100"))?;
        let mut out = vec![];
        for j in failed_job_list(&v)? {
            let Some(id) = j["id"].as_u64() else { continue };
            let log = match self.call("GET", &format!("repos/{repo}/actions/jobs/{id}/logs"), None, false) {
                Ok(res) => log_tail(&res.body),
                // Logs expire (and a job that never started has none): say so instead of failing the rest.
                Err(GhError::Status(404 | 410, _)) => "(no log: GitHub has none for this job)".into(),
                Err(e) => return Err(e.into()),
            };
            out.push(JobLog { name: s(&j, "name"), url: s(&j, "html_url"), log });
        }
        Ok(out)
    }
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        Ok(parse_merge_settings(
            &self.graphql(MERGE_SETTINGS_QUERY, json!({"owner": owner, "name": name, "base": base}))?,
        )?)
    }
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        let id = self.pr_node_id(repo, number)?;
        let method = strategy.map(merge_method);
        match self.graphql(AUTO_MERGE, json!({"id": id, "method": method})) {
            Ok(_) => Ok(()),
            // GitHub refuses auto-merge for a PR that could merge now ("is in clean status"): merge it, through the
            // merge queue when no strategy is given (a queue picks it).
            Err(GhError::Status(200, m)) if m.contains("clean status") => {
                if strategy.is_none() && self.graphql(ENQUEUE, json!({"id": id})).is_ok() {
                    return Ok(());
                }
                Ok(self.merge_now(repo, number, strategy)?)
            }
            Err(e) => Err(e.into()),
        }
    }
    fn viewer_login(&self) -> Result<String, ForgeError> {
        let login = s(&self.get("user")?, "login");
        if login.is_empty() { Err(GhError::BadAnswer("no login".into()).into()) } else { Ok(login) }
    }
    fn recent_issues(&self, repo: &str, after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        Ok(read_issue_pages(after, |n| {
            let path =
                format!("repos/{repo}/issues?state=all&sort=created&direction=desc&per_page={ISSUE_PAGE}&page={n}");
            match self.get(&path)? {
                Value::Array(items) => Ok(items),
                _ => Err(GhError::BadAnswer("issues are not an array".into())),
            }
        })?)
    }
    fn change_comments(&self, repo: &str, number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        let mut all =
            parse_comments(&self.get_pages(&format!("repos/{repo}/issues/{number}/comments?per_page=100"))?, false);
        all.extend(parse_comments(
            &self.get_pages(&format!("repos/{repo}/pulls/{number}/comments?per_page=100"))?,
            true,
        ));
        Ok(all)
    }
    fn change_reviews(&self, repo: &str, number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        Ok(parse_reviews(&self.get_pages(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100"))?))
    }
    fn comment_change_request(&self, repo: &str, number: u32, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/issues/{number}/comments"), body)?)
    }
    fn reply_review_comment(&self, repo: &str, number: u32, comment_id: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/pulls/{number}/comments/{comment_id}/replies"), body)?)
    }
    fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        match self.get(&format!("repos/{repo}/issues/{number}")) {
            Ok(v) => Ok(Some(parse_issue(&v)?)),
            Err(GhError::Status(404 | 410, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<ChangeComment>, ForgeError> {
        Ok(parse_comments(&self.get_pages(&format!("repos/{repo}/issues/{number}/comments?per_page=100"))?, false))
    }
    /// Labels the login can't set (no push access) or the repository lacks are dropped by GitHub, not refused.
    fn create_issue(&self, repo: &str, new: &NewIssue) -> Result<Issue, ForgeError> {
        let mut body = json!({"title": new.title, "body": comment_body(&new.body)});
        if !new.labels.is_empty() {
            body["labels"] = json!(new.labels);
        }
        Ok(parse_issue(&self.send_json("POST", &format!("repos/{repo}/issues"), body)?)?.issue)
    }
    fn comment_issue(&self, repo: &str, number: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/issues/{number}/comments"), body)?)
    }
    /// A POST adds them to the issue's (a PATCH of `labels` would replace the set).
    fn add_labels(&self, repo: &str, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        if labels.is_empty() {
            return Ok(());
        }
        self.send_json("POST", &format!("repos/{repo}/issues/{number}/labels"), json!({"labels": labels}))?;
        Ok(())
    }
    /// `state_reason`, and `duplicate_issue_id` for a duplicate (GitHub's "Close as duplicate").
    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        duplicate_of: Option<u64>,
    ) -> Result<(), ForgeError> {
        let mut body = json!({"state": "closed", "state_reason": reason.as_str()});
        if let (CloseReason::Duplicate, Some(id)) = (reason, duplicate_of) {
            body["duplicate_issue_id"] = json!(id);
        }
        self.send_json("PATCH", &format!("repos/{repo}/issues/{number}"), body)?;
        Ok(())
    }
    /// `workflow_dispatch`: the workflow must declare it, and the token needs the Actions write permission.
    fn run_workflow(
        &self,
        repo: &str,
        workflow: &str,
        git_ref: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<(), ForgeError> {
        let path = format!("repos/{repo}/actions/workflows/{}/dispatches", encode(workflow));
        match self.send_json("POST", &path, json!({"ref": git_ref, "inputs": inputs})) {
            Ok(_) => Ok(()),
            Err(GhError::Status(404, m)) => Err(ForgeError::Failed(format!(
                "GitHub has no workflow {workflow} in {repo} (or PDT's GitHub login can't start it): {m}"
            ))),
            Err(e) => Err(e.into()),
        }
    }
    fn release(&self, repo: &str, tag: Option<&str>) -> Result<Option<Release>, ForgeError> {
        let path = match tag {
            Some(t) => format!("repos/{repo}/releases/tags/{}", encode(t)),
            None => format!("repos/{repo}/releases/latest"),
        };
        match self.get(&path) {
            Ok(v) => Ok(parse_release(&v)?),
            Err(GhError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    /// GitHub lists releases newest first; drafts are skipped.
    fn newest_release(&self, repo: &str) -> Result<Option<Release>, ForgeError> {
        let list = self.get(&format!("repos/{repo}/releases?per_page=10"))?;
        for v in list.as_array().into_iter().flatten() {
            if let Some(r) = parse_release_or_pre(v, true)? {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
