//! The REST client the forges share (PDT-435): one host's account, its calls through a [`Transport`], and what every
//! forge's API needs around them. A used-up rate limit stops every call until it resets; an expiring OAuth token is
//! renewed with its refresh token first (one renewal at a time: a refresh token is single-use); a 401 renews it once,
//! then asks the login check's endpoint again, and only a 401 there too marks the login rejected, after which nothing
//! is sent until the user signs in again. GETs can be conditional (ETags: an unchanged answer costs no rate limit).
//! The last calls are kept (never their token, query or body) and logged with a login's first 401.
//!
//! What differs between forges is a [`Flavor`]: their name, URLs, headers, rate-limit headers and error messages.
//! Blocking; call from `spawn_blocking` in async code.

use crate::ForgeError;
use crate::login::ForgeAccount;
use crate::oauth::{self, OAuthApp, RefreshError};
use crate::transport::{Offline, Request, Response, Transport};
use pdt_core::brand::SHORT_NAME;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// The most pages one list read follows.
pub const MAX_PAGES: usize = 10;
/// The most GET answers kept for conditional requests.
pub const ETAG_CACHE: usize = 500;
/// The most recent calls kept, for the log of a refused login (PDT-252).
pub const RECENT_CALLS: usize = 50;
/// How long before its token expires a login is renewed (PDT-260): more than the login check's 5 minutes, so an idle
/// PDT renews it in time too.
pub const RENEW_BEFORE_MS: i64 = 15 * 60_000;
/// How long after a refresh that got no answer the next one waits.
pub const RENEW_RETRY_MS: i64 = 60_000;

/// What one forge's REST API is like.
pub trait Flavor: Send + Sync + 'static {
    /// The forge's name in messages, logs and Settings › Plugins: `GitHub`.
    const NAME: &'static str;
    /// The light login check's path (`user`; GitHub's `rate_limit` costs no rate limit). A 401 is confirmed with
    /// `user`.
    const CHECK_PATH: &'static str = "user";
    /// The statuses that refuse a call for the rate limit (with the rate-limit headers, or a `Retry-After`; a 429
    /// alone waits a minute).
    const LIMIT_REFUSALS: &'static [u16];
    /// The rate-limit headers' prefix: `<prefix>remaining`, `<prefix>reset` (lowercase).
    const LIMIT_HEADERS: &'static str;
    /// Answer headers logged with a refused call (lowercase).
    const REFUSAL_HEADERS: &'static [&'static str] = &[];
    /// GETs are conditional (ETags).
    const ETAGS: bool = false;
    /// An API URL as given, tidied (the public host's when empty).
    fn api_url(given: &str) -> String;
    /// The host name of an API URL's site.
    fn web_host(api: &str) -> String;
    /// The headers of an API call with `token`.
    fn headers(token: &str) -> Vec<(&'static str, String)>;
    /// An error answer's message.
    fn message(res: &Response) -> String;
    /// The OAuth endpoints of the host of API URL `api`, to renew a token.
    fn oauth(api: &str) -> OAuthApp;
}

/// What went wrong talking to a forge `F`.
#[derive(Debug, Clone, PartialEq)]
pub enum RestError<F> {
    NotConnected,
    Rejected,
    Status(u16, String),
    RateLimited(String),
    Network(String),
    BadAnswer(String),
    #[doc(hidden)]
    _Flavor(std::convert::Infallible, PhantomData<F>),
}

/// The text of a call made before the user connected forge `name`.
pub fn not_connected(name: &str) -> String {
    format!("{name} is not connected: connect it in {SHORT_NAME}'s Settings › Plugins › {name}")
}

impl<F: Flavor> std::fmt::Display for RestError<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = F::NAME;
        match self {
            RestError::NotConnected => write!(f, "{}", not_connected(n)),
            RestError::Rejected => {
                write!(f, "{n} no longer accepts {SHORT_NAME}'s login: connect {n} again in Settings › Plugins › {n}")
            }
            RestError::Status(code, m) => write!(f, "{n} answered {code}: {m}"),
            RestError::RateLimited(t) => write!(f, "{n}'s rate limit is used up until {t}"),
            RestError::Network(e) => write!(f, "{n} could not be reached: {e}"),
            RestError::BadAnswer(e) => write!(f, "unexpected answer from {n}: {e}"),
            RestError::_Flavor(never, _) => match *never {},
        }
    }
}

impl<F: Flavor + std::fmt::Debug> std::error::Error for RestError<F> {}

impl<F: Flavor> From<RestError<F>> for ForgeError {
    fn from(e: RestError<F>) -> Self {
        match e {
            RestError::NotConnected | RestError::Rejected => ForgeError::Unavailable(e.to_string()),
            _ => ForgeError::Failed(e.to_string()),
        }
    }
}

/// One call, for the log of a refused login: never its token, query or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    /// Unix ms.
    pub at: i64,
    pub method: &'static str,
    pub path: String,
    /// None: no answer (a network failure).
    pub status: Option<u16>,
}

/// What [`RestClient::check_login`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCheck {
    /// The forge accepts the login.
    Ok,
    /// The forge accepts a login it had refused: the mark is cleared.
    Recovered,
    /// The forge refuses the login.
    Rejected,
    /// No answer, or one that says nothing about the login.
    Failed(String),
}

/// Called with the token a renewal replaced and the renewed account, to store it.
pub type RenewedHook = Arc<dyn Fn(&str, &ForgeAccount) + Send + Sync>;

/// How one try of a call ended.
enum Once<F> {
    Done(Result<Response, RestError<F>>),
    /// A 401 from the API's own host, with the forge's message.
    Refused(String),
}

/// One forge host's client. With no account every call fails as [`RestError::NotConnected`].
pub struct RestClient<F> {
    transport: Arc<dyn Transport>,
    account: RwLock<Option<ForgeAccount>>,
    /// GET answers by URL: ETag and body.
    etags: Mutex<HashMap<String, (String, String)>>,
    /// When the rate limit resets (Unix seconds), while it is used up.
    limited_until: Mutex<Option<u64>>,
    /// A call wanted the forge while it was not connected.
    wanted: AtomicBool,
    /// When the forge refused the login (Unix ms): it is kept, but no call is sent until the user signs in again.
    rejected_at: Mutex<Option<i64>>,
    /// The last [`RECENT_CALLS`] calls, oldest first.
    recent: Mutex<VecDeque<CallRecord>>,
    /// When the forge last accepted the login (Unix ms).
    last_ok_at: Mutex<Option<i64>>,
    /// The recent calls were logged for this login's first 401.
    logged_calls: AtomicBool,
    /// Held while the token is renewed: two refreshes never run at once (the first one's refresh token is spent).
    renewing: Mutex<()>,
    /// No refresh before this (Unix ms), after one that got no answer.
    renew_after: Mutex<Option<i64>>,
    /// Stores a renewed account.
    on_renewed: RwLock<Option<RenewedHook>>,
    flavor: PhantomData<F>,
}

impl<F: Flavor> RestClient<F> {
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
            renewing: Mutex::default(),
            renew_after: Mutex::default(),
            on_renewed: RwLock::default(),
            flavor: PhantomData,
        }
    }

    /// No network and no login: tests' forges can never reach a forge.
    pub fn offline() -> Self {
        Self::new(Arc::new(Offline))
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    pub fn account(&self) -> Option<ForgeAccount> {
        self.account.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Signs in (or out, with None). Cached answers belong to the old login, so they go, and so does a refusal.
    pub fn set_account(&self, account: Option<ForgeAccount>) {
        *self.account.write().unwrap_or_else(|e| e.into_inner()) = account;
        self.etags.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.last_ok_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.logged_calls.store(false, Ordering::SeqCst);
        self.wanted.store(false, Ordering::SeqCst);
        *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Sets what stores the account once its token is renewed (PDT-260).
    pub fn on_renewed(&self, hook: Option<RenewedHook>) {
        *self.on_renewed.write().unwrap_or_else(|e| e.into_inner()) = hook;
    }

    /// When the forge stopped accepting the stored login (Unix ms), until the user signs in again.
    pub fn rejected_at(&self) -> Option<i64> {
        *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A call wanted the forge while it was not connected.
    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// The last calls, oldest first.
    pub fn recent_calls(&self) -> Vec<CallRecord> {
        self.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
    }

    /// The forge refused `token`. A call in flight when the user signed in again doesn't mark the new login.
    pub fn reject(&self, token: &str) {
        let account = self.account.read().unwrap_or_else(|e| e.into_inner());
        if account.as_ref().is_some_and(|a| a.token == token) {
            let mut at = self.rejected_at.lock().unwrap_or_else(|e| e.into_inner());
            if at.is_none() {
                tracing::warn!(login = %account.as_ref().map_or("", |a| a.login.as_str()), "{} refuses the login: no more calls until it accepts it again or the user signs in again", F::NAME);
            }
            at.get_or_insert_with(now_ms);
        }
    }

    /// Renews the token of `stale` with its refresh token (PDT-260), and stores the new account at once: the forge
    /// issues a new refresh token with each new token and the old one stops working. One renewal at a time; one that
    /// finds the token already renewed (or replaced by a new sign-in) returns that account. A refresh token the forge
    /// refuses, or one that has expired, marks the login rejected; no answer is [`RestError::Network`], and the next
    /// try waits [`RENEW_RETRY_MS`].
    pub fn renew(&self, stale: &ForgeAccount) -> Result<ForgeAccount, RestError<F>> {
        let _held = self.renewing.lock().unwrap_or_else(|e| e.into_inner());
        let cur = self.account().ok_or(RestError::NotConnected)?;
        if cur.token != stale.token {
            return Ok(cur);
        }
        let Some(r) = cur.refresh.clone() else { return Err(RestError::Rejected) };
        let (name, host) = (F::NAME, F::web_host(&cur.api_url));
        let now = now_ms();
        if r.expires_at.is_some_and(|t| now >= t) {
            tracing::warn!(login = %cur.login, host, "the {name} login's refresh token has expired: connect {name} again");
            self.reject(&cur.token);
            return Err(RestError::Rejected);
        }
        if self.renew_after.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| now < t) {
            return Err(RestError::Network("the last try to renew the login got no answer".into()));
        }
        match oauth::refresh_token(self.transport.as_ref(), &F::oauth(&cur.api_url), &r.client_id, &r.token) {
            Ok(g) => {
                let now = now_ms();
                let (expires_at, refresh) = g.renewal(now, &r.client_id);
                let new = ForgeAccount { token: g.token, expires_at, refresh, ..cur.clone() };
                {
                    let mut account = self.account.write().unwrap_or_else(|e| e.into_inner());
                    if account.as_ref().is_none_or(|a| a.token != cur.token) {
                        // The user signed in again meanwhile: theirs stands.
                        return account.clone().ok_or(RestError::NotConnected);
                    }
                    *account = Some(new.clone());
                }
                *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = None;
                self.logged_calls.store(false, Ordering::SeqCst);
                let left = |t: Option<i64>| t.map_or_else(|| "never".into(), |t| minutes(t - now));
                tracing::info!(
                    login = %new.login, host, client_id = %r.client_id, expires_in = %left(new.expires_at),
                    refresh_expires_in = %left(new.refresh.as_ref().and_then(|r| r.expires_at)),
                    "renewed the {name} login's token"
                );
                let hook = self.on_renewed.read().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(hook) = hook {
                    hook(&cur.token, &new);
                }
                Ok(new)
            }
            Err(RefreshError::Refused(e)) => {
                tracing::warn!(login = %cur.login, host, error = %e, "{name} refused to renew the login: connect {name} again");
                self.reject(&cur.token);
                Err(RestError::Rejected)
            }
            Err(RefreshError::Failed(e)) => {
                tracing::warn!(login = %cur.login, host, error = %e, "renewing the {name} login got no answer: trying again shortly");
                *self.renew_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms() + RENEW_RETRY_MS);
                Err(RestError::Network(e))
            }
        }
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

    /// Logs a 401 or 403 (PDT-252): the call, the forge's answer and the login's age; never the token. The first 401
    /// of a login also logs the recent calls.
    fn log_refusal(&self, a: &ForgeAccount, req: &Request, res: &Response) {
        let now = now_ms();
        let ago = |t: Option<i64>| t.map_or_else(|| "unknown".into(), |t| minutes(now - t));
        let last_ok = *self.last_ok_at.lock().unwrap_or_else(|e| e.into_inner());
        let headers: String =
            F::REFUSAL_HEADERS.iter().map(|h| format!(" {h}={:?}", res.header(h).unwrap_or("-"))).collect();
        let fields = format!(
            "method={} endpoint={} final_url={} message={:?}{headers} login={} login_age={} last_worked={}",
            req.method,
            bare_url(&req.url),
            if res.url.is_empty() { bare_url(&req.url) } else { res.url.clone() },
            F::message(res),
            a.login,
            ago(a.signed_in_at),
            last_ok.map_or_else(
                || "never (since pdtd started or the login was made)".into(),
                |t| format!("{} ago", minutes(now - t))
            ),
        );
        let name = F::NAME;
        if res.status == 403 {
            tracing::info!("{name} answered 403: {fields}");
            return;
        }
        tracing::warn!("{name} answered 401: {fields}");
        if !self.logged_calls.swap(true, Ordering::SeqCst) {
            let calls: Vec<String> = self
                .recent_calls()
                .iter()
                .map(|c| {
                    let status = c.status.map_or_else(|| "no answer".into(), |s| s.to_string());
                    format!("{} {} {} {}", clock_ms(c.at), c.method, c.path, status)
                })
                .collect();
            tracing::warn!("the last {} {name} calls before the first 401:\n{}", calls.len(), calls.join("\n"));
        }
    }

    /// `GET /user` with the login `a`.
    fn ask_user(&self, a: &ForgeAccount) -> (Request, Result<Response, String>) {
        let req = Request { method: "GET", url: url::<F>(a, "user"), headers: F::headers(&a.token), body: None };
        let res = self.send(&req);
        (req, res)
    }

    /// After a 401: whether the forge really refuses the login, asked again with `GET /user`. Only a 401 from the
    /// API's own host counts; no answer doesn't (the login check asks again later).
    fn confirm_refused(&self, a: &ForgeAccount) -> bool {
        match self.ask_user(a) {
            (req, Ok(res)) if res.status == 401 && same_host(&res.url, &req.url) => true,
            (_, Ok(res)) => {
                tracing::warn!(
                    status = res.status,
                    "{} refused one call, but accepts the login when asked again",
                    F::NAME
                );
                false
            }
            (_, Err(e)) => {
                tracing::warn!(error = %e, "{} refused one call, and could not be asked again", F::NAME);
                false
            }
        }
    }

    /// The light login check (PDT-252), every few minutes: `GET` [`Flavor::CHECK_PATH`], so the time the forge stops
    /// accepting the login is known to the minute. While the login is marked rejected, a login with a refresh token is
    /// renewed, one without asks `GET /user`, and the mark goes when the forge accepts it again. None without a login.
    pub fn check_login(&self) -> Option<LoginCheck> {
        let a = self.account()?;
        if self.rejected_at().is_none() {
            return Some(match self.call("GET", F::CHECK_PATH, None, false) {
                Ok(_) => LoginCheck::Ok,
                Err(RestError::Rejected) => LoginCheck::Rejected,
                Err(e) => LoginCheck::Failed(e.to_string()),
            });
        }
        if a.refresh.is_some() {
            // A refused token renews; a refused refresh token stays refused.
            return Some(match self.renew(&a) {
                Ok(_) => {
                    tracing::info!(login = %a.login, "{} accepts the login it had refused again, renewed", F::NAME);
                    LoginCheck::Recovered
                }
                Err(RestError::Rejected) => LoginCheck::Rejected,
                Err(e) => LoginCheck::Failed(e.to_string()),
            });
        }
        Some(match self.ask_user(&a).1 {
            Ok(res) if (200..300).contains(&res.status) => {
                let account = self.account.read().unwrap_or_else(|e| e.into_inner());
                if account.as_ref().is_some_and(|x| x.token == a.token) {
                    *self.rejected_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    self.logged_calls.store(false, Ordering::SeqCst);
                }
                tracing::info!(login = %a.login, "{} accepts the login it had refused again", F::NAME);
                LoginCheck::Recovered
            }
            Ok(res) if res.status == 401 => LoginCheck::Rejected,
            Ok(res) => LoginCheck::Failed(format!("{} answered {}: {}", F::NAME, res.status, F::message(&res))),
            Err(e) => LoginCheck::Failed(e),
        })
    }

    /// The login to call with: none, or one the forge refused, sends nothing. One whose token expires within
    /// [`RENEW_BEFORE_MS`] is renewed first (PDT-260); while renewing gets no answer, the old token serves until it
    /// expires.
    pub fn signed_in(&self) -> Result<ForgeAccount, RestError<F>> {
        let a = self.account().ok_or_else(|| {
            self.wanted.store(true, Ordering::SeqCst);
            RestError::NotConnected
        })?;
        if self.rejected_at().is_some() {
            return Err(RestError::Rejected);
        }
        match a.expires_at {
            Some(t) if a.refresh.is_some() && now_ms() >= t - RENEW_BEFORE_MS => match self.renew(&a) {
                Err(RestError::Network(_)) if now_ms() < t => Ok(a),
                r => r,
            },
            _ => Ok(a),
        }
    }

    fn check_limit(&self) -> Result<(), RestError<F>> {
        let mut until = self.limited_until.lock().unwrap_or_else(|e| e.into_inner());
        match *until {
            Some(t) if t > now_secs() => Err(RestError::RateLimited(clock(t))),
            Some(_) => {
                *until = None;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Notes a used-up rate limit from an answer: Err when this answer is the refusal.
    fn note_limit(&self, res: &Response) -> Result<(), RestError<F>> {
        let refused = F::LIMIT_REFUSALS.contains(&res.status);
        let header = |name: &str| res.header(&format!("{}{name}", F::LIMIT_HEADERS)).map(str::trim);
        let until = match (res.header("retry-after"), header("remaining")) {
            (Some(after), _) if refused => after.trim().parse::<u64>().ok().map(|s| now_secs() + s.max(1)),
            (_, Some("0")) => header("reset").and_then(|r| r.parse::<u64>().ok()),
            _ if refused && res.status == 429 => Some(now_secs() + 60),
            _ => None,
        };
        let Some(until) = until else { return Ok(()) };
        *self.limited_until.lock().unwrap_or_else(|e| e.into_inner()) = Some(until);
        if refused { Err(RestError::RateLimited(clock(until))) } else { Ok(()) }
    }

    /// One call: a JSON `body` for POST/PATCH/PUT; `path` is under the API URL, or a whole URL. GETs are conditional
    /// when `cache` and the forge has ETags. Any 2xx is Ok.
    pub fn call(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
        cache: bool,
    ) -> Result<Response, RestError<F>> {
        let mut a = self.signed_in()?;
        let mut renewed = false;
        loop {
            match self.call_once(&a, method, path, body, cache && F::ETAGS) {
                Once::Done(r) => return r,
                // A login that renews gets a new token and one more try (PDT-260) before it counts as refused.
                Once::Refused(_) if !renewed && a.refresh.is_some() => {
                    renewed = true;
                    tracing::info!(login = %a.login, "{} refused the login's token: renewing it and asking again", F::NAME);
                    a = self.renew(&a)?;
                }
                // Only a 401 whose `GET /user` is refused too marks the login: not one the forge takes back when
                // asked again (PDT-252).
                Once::Refused(m) => {
                    if self.confirm_refused(&a) {
                        self.reject(&a.token);
                        return Err(RestError::Rejected);
                    }
                    return Err(RestError::Status(401, m));
                }
            }
        }
    }

    /// One try of [`Self::call`] with the login `a`.
    fn call_once(
        &self,
        a: &ForgeAccount,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
        cache: bool,
    ) -> Once<F> {
        if let Err(e) = self.check_limit() {
            return Once::Done(Err(e));
        }
        let url = url::<F>(a, path);
        let mut headers = F::headers(&a.token);
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
        let mut res = match self.send(&req) {
            Ok(res) => res,
            Err(e) => return Once::Done(Err(RestError::Network(e))),
        };
        if let Err(e) = self.note_limit(&res) {
            return Once::Done(Err(e));
        }
        if res.status == 304
            && let Some((_, body)) = cached
        {
            res.status = 200;
            res.body = body;
            return Once::Done(Ok(res));
        }
        if matches!(res.status, 401 | 403) {
            self.log_refusal(a, &req, &res);
        }
        // A 401 from a redirect's other host (a log's storage) says nothing about the login.
        if res.status == 401 && same_host(&res.url, &url) {
            return Once::Refused(F::message(&res));
        }
        if !(200..300).contains(&res.status) {
            return Once::Done(Err(RestError::Status(res.status, F::message(&res))));
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
        Once::Done(Ok(res))
    }

    /// A GET's JSON.
    pub fn get(&self, path: &str) -> Result<Value, RestError<F>> {
        json(&self.call("GET", path, None, true)?.body)
    }

    /// A call with a JSON body; its JSON answer (Null when empty).
    pub fn send_json(&self, method: &'static str, path: &str, body: Value) -> Result<Value, RestError<F>> {
        let res = self.call(method, path, Some(&body), false)?;
        if res.body.trim().is_empty() { Ok(Value::Null) } else { json(&res.body) }
    }

    /// Every item of a list, following its `Link: next` pages (at most [`MAX_PAGES`]) on the API's own host.
    pub fn get_pages(&self, path: &str) -> Result<Vec<Value>, RestError<F>> {
        let mut out = vec![];
        let mut next = Some(path.to_string());
        for _ in 0..MAX_PAGES {
            let Some(p) = next.take() else { break };
            let res = self.call("GET", &p, None, true)?;
            match json(&res.body)? {
                Value::Array(items) => out.extend(items),
                _ => return Err(RestError::BadAnswer("a list is not an array".into())),
            }
            next = res.header("link").and_then(next_link);
            // `call` sends the token to any whole URL it is given: a next page off the API's host is refused (PDT-543).
            if let Some(n) = &next
                && (n.starts_with("https://") || n.starts_with("http://"))
                && !self.account().is_some_and(|a| same_host(n, &F::api_url(&a.api_url)))
            {
                return Err(RestError::BadAnswer(format!("the next page is on another host: {}", bare_url(n))));
            }
        }
        Ok(out)
    }
}

/// `path` under the account's API URL; a whole URL as it is.
fn url<F: Flavor>(a: &ForgeAccount, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.into()
    } else {
        format!("{}/{path}", F::api_url(&a.api_url))
    }
}

/// An answer's JSON.
pub fn json<F>(out: &str) -> Result<Value, RestError<F>> {
    serde_json::from_str(out).map_err(|e| RestError::BadAnswer(e.to_string()))
}

/// The `rel="next"` URL of a `Link` header.
pub fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, rest) = part.split_once(';')?;
        rest.contains("rel=\"next\"").then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

/// Unix seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A URL without its query or fragment (a signed link's signature).
pub fn bare_url(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

/// A URL's path, without host or query.
pub fn path_of(url: &str) -> String {
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
pub fn same_host(answered: &str, asked: &str) -> bool {
    answered.is_empty() || origin(answered).eq_ignore_ascii_case(origin(asked))
}

/// A duration in ms, for logs: `95 min`.
fn minutes(ms: i64) -> String {
    format!("{} min", ms.max(0) / 60_000)
}

/// A time in Unix ms, for logs: `HH:MM:SS UTC`.
pub fn clock_ms(ms: i64) -> String {
    let day = (ms / 1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02} UTC", day / 3600, (day % 3600) / 60, day % 60)
}

/// A reset time for a message: `HH:MM UTC`.
pub fn clock(t: u64) -> String {
    let day = t % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, (day % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::testing::Recorded;

    #[derive(Debug, Clone, PartialEq)]
    struct Test;

    impl Flavor for Test {
        const NAME: &'static str = "Forge";
        const LIMIT_REFUSALS: &'static [u16] = &[429];
        const LIMIT_HEADERS: &'static str = "ratelimit-";
        fn api_url(given: &str) -> String {
            given.into()
        }
        fn web_host(_: &str) -> String {
            "f.example".into()
        }
        fn headers(token: &str) -> Vec<(&'static str, String)> {
            vec![("Authorization", format!("Bearer {token}"))]
        }
        fn message(res: &Response) -> String {
            res.body.clone()
        }
        fn oauth(_: &str) -> OAuthApp {
            OAuthApp {
                forge: Self::NAME,
                device_code_url: "https://f.example/device".into(),
                token_url: "https://f.example/token".into(),
                scopes: "api",
                no_device_flow: "none",
                start_errors: &[],
                code_expires_in: 300,
            }
        }
    }

    fn client() -> (RestClient<Test>, Arc<Recorded>) {
        let t = Arc::new(Recorded::new());
        let c = RestClient::<Test>::new(t.clone());
        c.set_account(Some(ForgeAccount {
            token: "t".into(),
            api_url: "https://f.example/api".into(),
            ..Default::default()
        }));
        (c, t)
    }

    #[test]
    fn errors_name_the_forge() {
        let e: RestError<Test> = RestError::Rejected;
        assert_eq!(
            e.to_string(),
            format!("Forge no longer accepts {SHORT_NAME}'s login: connect Forge again in Settings › Plugins › Forge")
        );
        assert!(matches!(ForgeError::from(e), ForgeError::Unavailable(_)));
        let e: RestError<Test> = RestError::Status(500, "boom".into());
        assert_eq!(ForgeError::from(e), ForgeError::Failed("Forge answered 500: boom".into()));
        assert!(RestClient::<Test>::offline().get("x").unwrap_err().to_string().contains("Forge is not connected"));
    }

    #[test]
    fn a_429_without_headers_waits_a_minute_and_stops_every_call() {
        let (c, t) = client();
        t.on("GET", "https://f.example/api/a", Response::new(429, "slow down"));
        assert!(matches!(c.get("a"), Err(RestError::RateLimited(_))));
        assert!(matches!(c.get("a"), Err(RestError::RateLimited(_))));
        assert_eq!(t.sent().len(), 1, "the second call never left");
    }

    #[test]
    fn without_etags_gets_are_not_conditional() {
        let (c, t) = client();
        t.on("GET", "https://f.example/api/a", Response::new(200, "1").with_header("ETag", "e")).ok(
            "GET",
            "https://f.example/api/a",
            "2",
        );
        assert_eq!(c.get("a").unwrap(), Value::from(1));
        assert_eq!(c.get("a").unwrap(), Value::from(2));
        assert!(!t.sent()[1].headers.iter().any(|(k, _)| *k == "If-None-Match"));
    }

    #[test]
    fn pages_are_followed_only_on_the_apis_host() {
        let (c, t) = client();
        let link = |to: &str| format!("<{to}>; rel=\"next\"");
        t.on(
            "GET",
            "https://f.example/api/a",
            Response::new(200, "[1]").with_header("Link", &link("https://f.example/api/a?page=2")),
        )
        .ok("GET", "https://f.example/api/a?page=2", "[2]");
        assert_eq!(c.get_pages("a").unwrap(), [Value::from(1), Value::from(2)]);

        for elsewhere in
            ["https://evil.example/api/a?page=2", "http://f.example/api/a?page=2", "https://f.example:8443/api/a"]
        {
            let (c, t) = client();
            t.on("GET", "https://f.example/api/a", Response::new(200, "[1]").with_header("Link", &link(elsewhere)));
            assert!(matches!(c.get_pages("a"), Err(RestError::BadAnswer(_))), "{elsewhere}");
            assert_eq!(t.sent().len(), 1, "{elsewhere} never received the token");
        }
    }

    #[test]
    fn urls_for_logs() {
        assert_eq!(path_of("https://api.github.com/repos/o/r?x=1"), "/repos/o/r");
        assert_eq!(path_of("https://api.github.com"), "/");
        assert!(same_host("", "https://api.github.com/x"));
        assert!(same_host("https://API.github.com/y", "https://api.github.com/x"));
        assert!(!same_host("https://blob.core.windows.net/y", "https://api.github.com/x"));
        assert_eq!(clock_ms(90_061_000), "01:01:01 UTC");
        assert_eq!(
            next_link(r#"<https://x/a?page=2>; rel="next", <https://x/a?page=5>; rel="last""#).as_deref(),
            Some("https://x/a?page=2")
        );
    }
}
