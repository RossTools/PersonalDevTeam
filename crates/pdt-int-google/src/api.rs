//! Calls to Google's APIs as one signed-in account: an access token from the account's refresh token (kept in memory
//! until it expires, renewed once when Google turns it down), and Google's errors as [`ProviderError`]s people can act
//! on (a rate limit, a file PDT may not open, a sign-in to renew, no network). Blocking, through a
//! [`pdt_forge::transport::Transport`]: the provider calls it from `spawn_blocking`; tests use recorded answers.

use crate::oauth;
use pdt_api::sdk::documents::ProviderError;
use pdt_core::secret::Redactor;
use pdt_forge::transport::{Request, Response, Transport};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const DRIVE: &str = "https://www.googleapis.com/drive/v3";
pub const DOCS: &str = "https://docs.googleapis.com/v1";
pub const SHEETS: &str = "https://sheets.googleapis.com/v4";

/// A signed-in Google account: its connection, OAuth client and refresh token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub connection_id: String,
    /// The connection's name, for messages.
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// Signed in with PDT's agentic client, for an agent's document tools (PDT-360).
    pub agent: bool,
}

impl Account {
    /// What its access token is kept under: the agentic client's apart from the standard one's.
    pub fn token_key(&self) -> String {
        token_key(&self.connection_id, self.agent)
    }
}

/// What a connection's access token is kept under, for its standard or agentic client.
pub fn token_key(connection_id: &str, agent: bool) -> String {
    if agent { format!("{connection_id}:agent") } else { connection_id.to_string() }
}

/// Access tokens by connection, with when they expire (ms). Each new one is also given to pdtd's redactor (once it is
/// known, [`Tokens::redact_with`]), so it is kept out of what PDT stores and logs as the sign-ins are.
#[derive(Default)]
pub struct Tokens {
    by_id: Mutex<HashMap<String, (String, i64)>>,
    redactor: OnceLock<Arc<Redactor>>,
}

impl Tokens {
    /// Registers each access token from now on with `r` (the first call's).
    pub fn redact_with(&self, r: &Arc<Redactor>) {
        let _ = self.redactor.set(r.clone());
    }
    fn get(&self, id: &str, now: i64) -> Option<String> {
        let m = self.by_id.lock().unwrap_or_else(|e| e.into_inner());
        m.get(id).filter(|(_, exp)| *exp > now + 60_000).map(|(t, _)| t.clone())
    }
    fn put(&self, id: &str, token: &str, expires_at: i64) {
        if let Some(r) = self.redactor.get() {
            r.set_labelled(&format!("google-access:{id}"), token, "google");
        }
        self.by_id.lock().unwrap_or_else(|e| e.into_inner()).insert(id.into(), (token.into(), expires_at));
    }
    pub fn forget(&self, id: &str) {
        self.by_id.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }
}

/// What a call sends.
pub enum Body {
    None,
    Json(Value),
}

/// One account's calls.
pub struct Session<'a> {
    pub transport: &'a dyn Transport,
    pub account: &'a Account,
    pub tokens: &'a Tokens,
    /// The time now, in ms (a test's clock).
    pub now: fn() -> i64,
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

impl<'a> Session<'a> {
    pub fn new(transport: &'a dyn Transport, account: &'a Account, tokens: &'a Tokens) -> Self {
        Self { transport, account, tokens, now: now_ms }
    }

    /// An access token: the one kept, else a new one from the refresh token.
    pub fn access_token(&self, renew: bool) -> Result<String, ProviderError> {
        let now = (self.now)();
        if !renew && let Some(t) = self.tokens.get(&self.account.token_key(), now) {
            return Ok(t);
        }
        let t = oauth::refresh(self.transport, self.account).map_err(|e| match e {
            oauth::OauthError::Rejected(_) => ProviderError::Failed(format!(
                "Google turned down the sign-in of {} (it expired or was taken back): sign in again in Settings → \
                 Plugins → Google",
                self.account.name
            )),
            oauth::OauthError::Offline(m) => offline(&m),
            oauth::OauthError::Failed(m) => ProviderError::Failed(format!("Google's sign-in failed: {m}")),
        })?;
        self.tokens.put(&self.account.token_key(), &t.access_token, now + t.expires_in * 1000);
        Ok(t.access_token)
    }

    fn send(&self, method: &'static str, url: &str, body: &Body, token: &str) -> Result<Response, ProviderError> {
        let mut headers = vec![("authorization", format!("Bearer {token}")), ("accept", "application/json".into())];
        let body = match body {
            Body::None => None,
            Body::Json(v) => {
                headers.push(("content-type", "application/json".into()));
                Some(v.to_string())
            }
        };
        let req = Request { method, url: url.into(), headers, body };
        self.transport.send(&req).map_err(|e| offline(&e))
    }

    /// One call: the answer's body (a 2xx), with the token renewed once if Google turned it down.
    pub fn call_raw(&self, method: &'static str, url: &str, body: Body) -> Result<String, ProviderError> {
        let mut res = self.send(method, url, &body, &self.access_token(false)?)?;
        if res.status == 401 {
            self.tokens.forget(&self.account.token_key());
            res = self.send(method, url, &body, &self.access_token(true)?)?;
        }
        if (200..300).contains(&res.status) {
            return Ok(res.body);
        }
        Err(error_of(res.status, &res.body, &self.account.name))
    }

    /// One call with a JSON answer (`Null` for an empty one).
    pub fn call(&self, method: &'static str, url: &str, body: Body) -> Result<Value, ProviderError> {
        let text = self.call_raw(method, url, body)?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| ProviderError::Failed(format!("Google's answer wasn't JSON: {e}")))
    }

    pub fn get(&self, url: &str) -> Result<Value, ProviderError> {
        self.call("GET", url, Body::None)
    }
    pub fn post(&self, url: &str, body: Value) -> Result<Value, ProviderError> {
        self.call("POST", url, Body::Json(body))
    }
    pub fn patch(&self, url: &str, body: Value) -> Result<Value, ProviderError> {
        self.call("PATCH", url, Body::Json(body))
    }
}

pub fn offline(e: &str) -> ProviderError {
    ProviderError::Failed(format!("PDT can't reach Google ({e}). Is this machine offline?"))
}

/// Google's error answer as a [`ProviderError`]: a rate limit is [`ProviderError::Busy`], a missing file
/// [`ProviderError::NotFound`], a bad request [`ProviderError::Invalid`].
pub fn error_of(status: u16, body: &str, account: &str) -> ProviderError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = &v["error"];
    let message = err["message"].as_str().or_else(|| err.as_str()).unwrap_or("no message").trim().to_string();
    let reasons: Vec<&str> =
        err["errors"].as_array().into_iter().flatten().filter_map(|e| e["reason"].as_str()).collect();
    let limited = status == 429
        || err["status"].as_str() == Some("RESOURCE_EXHAUSTED")
        || reasons
            .iter()
            .any(|r| r.contains("RateLimitExceeded") || r.contains("rateLimitExceeded") || *r == "quotaExceeded");
    if limited {
        return ProviderError::Busy(format!(
            "Google is limiting how often PDT may call it ({message}). Try again in a minute."
        ));
    }
    match status {
        404 => ProviderError::NotFound,
        400 => ProviderError::Invalid(format!("Google refused the change: {message}")),
        401 => ProviderError::Failed(format!(
            "Google turned down the sign-in of {account}: sign in again in Settings → Plugins → Google"
        )),
        403 => ProviderError::Failed(format!(
            "Google won't let PDT do that ({message}). PDT can only open files it made or that you picked in \
             Google's file picker."
        )),
        s if s >= 500 => ProviderError::Failed(format!("Google had a problem ({s}): {message}. Try again later.")),
        s => ProviderError::Failed(format!("Google answered {s}: {message}")),
    }
}

/// A query value, escaped.
pub fn q(s: &str) -> String {
    pdt_forge::transport::encode(s)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdt_forge::transport::testing::Recorded;

    pub(crate) fn account() -> Account {
        Account {
            connection_id: "c1".into(),
            name: "Work".into(),
            client_id: "cid".into(),
            client_secret: "csecret".into(),
            refresh_token: "rtoken".into(),
            agent: false,
        }
    }

    #[test]
    fn errors_say_what_to_do() {
        let busy =
            r#"{"error":{"code":403,"message":"Rate Limit Exceeded","errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        assert!(matches!(error_of(403, busy, "Work"), ProviderError::Busy(m) if m.contains("Try again")));
        assert!(matches!(error_of(429, "{}", "Work"), ProviderError::Busy(_)));
        assert_eq!(error_of(404, r#"{"error":{"message":"File not found"}}"#, "Work"), ProviderError::NotFound);
        let denied = r#"{"error":{"code":403,"message":"The user has not granted the app 1 read access"}}"#;
        assert!(matches!(error_of(403, denied, "Work"), ProviderError::Failed(m) if m.contains("picked")));
        assert!(
            matches!(error_of(400, r#"{"error":{"message":"Invalid range"}}"#, "W"), ProviderError::Invalid(m) if m.contains("Invalid range"))
        );
        assert!(matches!(error_of(503, "", "W"), ProviderError::Failed(m) if m.contains("503")));
    }

    #[test]
    fn a_token_is_kept_and_renewed_once_when_turned_down() {
        let r = Recorded::new();
        r.ok("POST", oauth::TOKEN_URL, r#"{"access_token":"t1","expires_in":3600}"#)
            .ok("GET", "https://x/a", r#"{"n":1}"#)
            .on("GET", "https://x/b", Response::new(401, "{}"))
            .ok("POST", oauth::TOKEN_URL, r#"{"access_token":"t2","expires_in":3600}"#)
            .ok("GET", "https://x/b", r#"{"n":2}"#);
        let (a, tokens) = (account(), Tokens::default());
        let s = Session::new(&r, &a, &tokens);
        assert_eq!(s.get("https://x/a").unwrap()["n"], 1);
        assert_eq!(s.get("https://x/b").unwrap()["n"], 2);
        let sent = r.sent();
        let auth = |i: usize| sent[i].headers.iter().find(|(k, _)| *k == "authorization").unwrap().1.clone();
        assert_eq!((auth(1), auth(2), auth(4)), ("Bearer t1".into(), "Bearer t1".into(), "Bearer t2".into()));
        assert!(sent[0].body.as_deref().unwrap().contains("refresh_token=rtoken"));
    }

    #[test]
    fn no_network_is_offline() {
        let a = account();
        let tokens = Tokens::default();
        let s = Session::new(&pdt_forge::transport::Offline, &a, &tokens);
        assert!(matches!(s.get("https://x"), Err(ProviderError::Failed(m)) if m.contains("offline")));
    }

    #[test]
    fn a_taken_back_sign_in_says_sign_in_again() {
        let r = Recorded::new();
        r.on("POST", oauth::TOKEN_URL, Response::new(400, r#"{"error":"invalid_grant"}"#));
        let (a, tokens) = (account(), Tokens::default());
        let s = Session::new(&r, &a, &tokens);
        assert!(matches!(s.get("https://x"), Err(ProviderError::Failed(m)) if m.contains("sign in again")));
    }
}
