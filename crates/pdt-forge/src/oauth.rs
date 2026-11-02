//! The OAuth device flow and token refresh (RFC 8628, RFC 6749 §6), as the forges that sign in with them share them
//! (GitHub's and GitLab's): the user enters a code on the forge's site while PDT polls for the token; a token that
//! expires is renewed with its refresh token. Client ID only, no secret. Each forge names its endpoints in an
//! [`OAuthApp`].

use crate::login::{DeviceCode, DevicePoll, TokenGrant};
use crate::transport::{Request, Transport, form};
use serde_json::Value;

/// One forge host's OAuth endpoints, and what its answers mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthApp {
    /// The forge's name, for messages: `GitHub`.
    pub forge: &'static str,
    /// Where a device flow starts (`https://github.com/login/device/code`).
    pub device_code_url: String,
    /// Where a token is asked for, by device code or refresh token (`https://github.com/login/oauth/access_token`).
    pub token_url: String,
    /// The scopes PDT asks for.
    pub scopes: &'static str,
    /// What a 404 from the device flow means (an unknown app, or a server without one).
    pub no_device_flow: &'static str,
    /// Clearer messages for OAuth errors (`error`) when a device flow starts.
    pub start_errors: &'static [(&'static str, &'static str)],
    /// How long a device code lasts (seconds) when the answer doesn't say.
    pub code_expires_in: u64,
}

/// Posts a form; the answer's status and JSON. An answer that isn't JSON is Err (a 404's needn't be).
fn post_form(t: &dyn Transport, app: &OAuthApp, url: &str, pairs: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let req = Request {
        method: "POST",
        url: url.into(),
        headers: vec![
            ("Accept", "application/json".into()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
        ],
        body: Some(form(pairs)),
    };
    let res = t.send(&req)?;
    match serde_json::from_str(&res.body) {
        Ok(v) => Ok((res.status, v)),
        Err(_) if res.status == 404 => Ok((404, Value::Null)),
        Err(_) => Err(format!("{} answered {}", app.forge, res.status)),
    }
}

/// An OAuth error answer's message: its `error_description`, else its `error`.
fn oauth_error(v: &Value) -> Option<String> {
    let e = v["error"].as_str()?;
    Some(v["error_description"].as_str().unwrap_or(e).to_string())
}

/// Starts a device flow for OAuth app `client_id`.
pub fn start_device_flow(t: &dyn Transport, app: &OAuthApp, client_id: &str) -> Result<DeviceCode, String> {
    let (status, v) = post_form(t, app, &app.device_code_url, &[("client_id", client_id), ("scope", app.scopes)])?;
    if status == 404 {
        return Err(app.no_device_flow.into());
    }
    if let Some(m) = oauth_error(&v) {
        let code = v["error"].as_str().unwrap_or_default();
        return Err(app.start_errors.iter().find(|(c, _)| *c == code).map_or(m, |(_, known)| known.to_string()));
    }
    if !(200..300).contains(&status) {
        return Err(format!("{} answered {status}", app.forge));
    }
    let s = |k: &str| v[k].as_str().map(String::from).ok_or_else(|| format!("no {k} in {}'s answer", app.forge));
    Ok(DeviceCode {
        device_code: s("device_code")?,
        user_code: s("user_code")?,
        verification_uri: s("verification_uri")?,
        expires_in: v["expires_in"].as_u64().unwrap_or(app.code_expires_in),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

/// Asks whether the user has entered the code yet. A pending flow may come back as a 400 (GitLab's), so the answer's
/// `error` decides, not its status.
pub fn poll_device_flow(t: &dyn Transport, app: &OAuthApp, client_id: &str, device_code: &str) -> DevicePoll {
    let pairs = [
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];
    let v = match post_form(t, app, &app.token_url, &pairs) {
        Ok((404, _)) => return DevicePoll::Failed(app.no_device_flow.into()),
        Ok((_, v)) => v,
        Err(e) => return DevicePoll::Failed(e),
    };
    if let Some(g) = grant(&v) {
        return DevicePoll::Token(g);
    }
    match v["error"].as_str() {
        Some("authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown(v["interval"].as_u64().unwrap_or(10)),
        Some("expired_token") => DevicePoll::Expired,
        Some("access_denied") => DevicePoll::Denied,
        Some(e) => DevicePoll::Failed(v["error_description"].as_str().unwrap_or(e).to_string()),
        None => DevicePoll::Failed(format!("no token in {}'s answer", app.forge)),
    }
}

/// The token in a token answer, with its expiry and refresh token when it has them (a token that doesn't expire has
/// neither).
pub fn grant(v: &Value) -> Option<TokenGrant> {
    let token = v["access_token"].as_str().filter(|t| !t.is_empty())?;
    let refresh_token = v["refresh_token"].as_str().filter(|t| !t.is_empty()).map(String::from);
    Some(TokenGrant {
        token: token.into(),
        expires_in: v["expires_in"].as_u64().filter(|s| *s > 0),
        refresh_token_expires_in: refresh_token.as_ref().and(v["refresh_token_expires_in"].as_u64()),
        refresh_token,
    })
}

/// Why a refresh got no new token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    /// The forge refused the refresh token (revoked, expired or already used): only signing in again helps.
    Refused(String),
    /// No answer, or a server error: ask again later.
    Failed(String),
}

/// Swaps `refresh_token` of OAuth app `client_id` for a new token and a new refresh token (the old one stops
/// working).
pub fn refresh_token(
    t: &dyn Transport,
    app: &OAuthApp,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenGrant, RefreshError> {
    let req = Request {
        method: "POST",
        url: app.token_url.clone(),
        headers: vec![
            ("Accept", "application/json".into()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
        ],
        body: Some(form(&[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])),
    };
    let res = t.send(&req).map_err(RefreshError::Failed)?;
    if res.status >= 500 || res.status == 429 {
        return Err(RefreshError::Failed(format!("{} answered {}", app.forge, res.status)));
    }
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    if let Some(e) = oauth_error(&v) {
        return Err(RefreshError::Refused(e));
    }
    if !(200..300).contains(&res.status) {
        return Err(RefreshError::Refused(format!("{} answered {}", app.forge, res.status)));
    }
    grant(&v).ok_or_else(|| RefreshError::Failed(format!("no token in {}'s answer", app.forge)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Response;
    use crate::transport::testing::Recorded;

    fn app() -> OAuthApp {
        OAuthApp {
            forge: "Forge",
            device_code_url: "https://f.example/device".into(),
            token_url: "https://f.example/token".into(),
            scopes: "api",
            no_device_flow: "no device flow here",
            start_errors: &[("invalid_client", "an unknown app")],
            code_expires_in: 300,
        }
    }

    #[test]
    fn starting_a_device_flow_explains_the_forges_refusals() {
        let t = Recorded::new();
        t.ok("POST", "https://f.example/device", r#"{"device_code":"d","user_code":"U","verification_uri":"v"}"#)
            .on("POST", "https://f.example/device", Response::new(404, "<html>"))
            .on("POST", "https://f.example/device", Response::new(401, r#"{"error":"invalid_client"}"#))
            .on("POST", "https://f.example/device", Response::new(400, r#"{"error":"x","error_description":"why"}"#))
            .on("POST", "https://f.example/device", Response::new(502, "<html>"));
        let c = start_device_flow(&t, &app(), "c").unwrap();
        assert_eq!((c.user_code.as_str(), c.expires_in, c.interval), ("U", 300, 5), "the app's default lifetime");
        assert_eq!(t.sent()[0].body.as_deref(), Some("client_id=c&scope=api"));
        assert_eq!(start_device_flow(&t, &app(), "c").unwrap_err(), "no device flow here");
        assert_eq!(start_device_flow(&t, &app(), "c").unwrap_err(), "an unknown app");
        assert_eq!(start_device_flow(&t, &app(), "c").unwrap_err(), "why");
        assert_eq!(start_device_flow(&t, &app(), "c").unwrap_err(), "Forge answered 502");
    }

    #[test]
    fn a_pending_flow_may_answer_400() {
        let t = Recorded::new();
        t.on("POST", "https://f.example/token", Response::new(400, r#"{"error":"authorization_pending"}"#))
            .on("POST", "https://f.example/token", Response::new(404, "<html>"))
            .ok("POST", "https://f.example/token", "{}");
        assert_eq!(poll_device_flow(&t, &app(), "c", "d"), DevicePoll::Pending);
        assert_eq!(poll_device_flow(&t, &app(), "c", "d"), DevicePoll::Failed("no device flow here".into()));
        assert_eq!(poll_device_flow(&t, &app(), "c", "d"), DevicePoll::Failed("no token in Forge's answer".into()));
    }

    #[test]
    fn a_grant_has_a_refresh_tokens_expiry_only_with_a_refresh_token() {
        let g = grant(&serde_json::json!({"access_token": "a", "expires_in": 0, "refresh_token_expires_in": 9}));
        assert_eq!(g, Some(TokenGrant { token: "a".into(), ..Default::default() }));
        assert_eq!(grant(&serde_json::json!({"access_token": ""})), None);
    }
}
