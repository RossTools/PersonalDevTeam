//! Signing in to GitLab (PDT-246): gitlab.com and self-hosted servers, by their API URL (`https://<host>/api/v4`);
//! a pasted token (a personal, group or project access token, or an OAuth token), or the OAuth device flow (GitLab
//! 17.1+, an OAuth application with the device flow on); renewing an OAuth token with its refresh token.

use pdt_forge::login::{DeviceCode, DevicePoll, TokenGrant, Whoami};
use pdt_forge::transport::{Request, Transport, form};
use serde_json::Value;

/// gitlab.com's API.
pub const GITLAB_API: &str = "https://gitlab.com/api/v4";

/// gitlab.com's web host.
pub const GITLAB_HOST: &str = "gitlab.com";

/// The scope PDT asks for: `api` reads and writes merge requests, notes, approvals, issues and pipelines (`read_api`
/// can't merge, comment or retry).
pub const SCOPES: &str = "api";

/// An API URL as given, tidied: gitlab.com's when empty; a bare server URL (`https://gitlab.example.com`) gets
/// `/api/v4`.
pub fn api_url(given: &str) -> String {
    let u = given.trim().trim_end_matches('/');
    if u.is_empty() {
        return GITLAB_API.into();
    }
    let u = if u.contains("://") { u.to_string() } else { format!("https://{u}") };
    if u.ends_with("/api/v4") { u } else { format!("{u}/api/v4") }
}

/// The website of an API URL (`https://gitlab.com/api/v4` → `https://gitlab.com`).
pub fn web_url(api: &str) -> String {
    let api = api_url(api);
    api.strip_suffix("/api/v4").map(String::from).unwrap_or(api)
}

/// The host name of an API URL's site (`gitlab.com`, `gitlab.example.com`): the host a project's `origin` is on.
pub fn web_host(api: &str) -> String {
    let w = web_url(api);
    let rest = w.split_once("://").map_or(w.as_str(), |(_, r)| r);
    rest.split(['/', ':']).next().unwrap_or_default().to_ascii_lowercase()
}

/// The headers of an API call with `token` (a personal access token and an OAuth token both go as a bearer token).
pub(crate) fn headers(token: &str) -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {token}")), ("Accept", "application/json".into())]
}

fn post_form(t: &dyn Transport, url: String, pairs: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let req = Request {
        method: "POST",
        url,
        headers: vec![
            ("Accept", "application/json".into()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
        ],
        body: Some(form(pairs)),
    };
    let res = t.send(&req)?;
    let v: Value = serde_json::from_str(&res.body).map_err(|_| format!("GitLab answered {}", res.status))?;
    Ok((res.status, v))
}

fn oauth_error(v: &Value) -> Option<String> {
    let e = v["error"].as_str()?;
    Some(v["error_description"].as_str().unwrap_or(e).to_string())
}

/// Starts a device flow for OAuth application `client_id` (GitLab 17.1+).
pub fn start_device_flow(t: &dyn Transport, api: &str, client_id: &str) -> Result<DeviceCode, String> {
    let url = format!("{}/oauth/authorize_device", web_url(api));
    let (status, v) = post_form(t, url, &[("client_id", client_id), ("scope", SCOPES)])?;
    if status == 404 {
        return Err("this GitLab has no device flow (it needs GitLab 17.1 or later)".into());
    }
    if let Some(e) = oauth_error(&v) {
        return Err(match v["error"].as_str() {
            Some("invalid_client") => "GitLab does not know this OAuth application, or its device flow is off".into(),
            _ => e,
        });
    }
    if !(200..300).contains(&status) {
        return Err(format!("GitLab answered {status}"));
    }
    let s = |k: &str| v[k].as_str().map(String::from).ok_or_else(|| format!("no {k} in GitLab's answer"));
    Ok(DeviceCode {
        device_code: s("device_code")?,
        user_code: s("user_code")?,
        verification_uri: s("verification_uri")?,
        expires_in: v["expires_in"].as_u64().unwrap_or(300),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

/// Asks whether the user has entered the code yet.
pub fn poll_device_flow(t: &dyn Transport, api: &str, client_id: &str, device_code: &str) -> DevicePoll {
    let url = format!("{}/oauth/token", web_url(api));
    let pairs = [
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];
    let v = match post_form(t, url, &pairs) {
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
        None => DevicePoll::Failed("no token in GitLab's answer".into()),
    }
}

/// The token of an `oauth/token` answer: GitLab's OAuth tokens expire (in two hours) and come with a refresh token.
fn grant(v: &Value) -> Option<TokenGrant> {
    let token = v["access_token"].as_str().filter(|t| !t.is_empty())?;
    Some(TokenGrant {
        token: token.into(),
        expires_in: v["expires_in"].as_u64().filter(|s| *s > 0),
        refresh_token: v["refresh_token"].as_str().filter(|t| !t.is_empty()).map(String::from),
        refresh_token_expires_in: None,
    })
}

/// Why a refresh got no new token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshError {
    /// GitLab refused the refresh token (revoked, or already used): only signing in again helps.
    Refused(String),
    /// No answer, or a server error: ask again later.
    Failed(String),
}

/// Swaps `refresh_token` of OAuth application `client_id` for a new token and a new refresh token (the old one stops
/// working).
pub fn refresh_token(
    t: &dyn Transport,
    api: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenGrant, RefreshError> {
    let req = Request {
        method: "POST",
        url: format!("{}/oauth/token", web_url(api)),
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
        return Err(RefreshError::Failed(format!("GitLab answered {}", res.status)));
    }
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    if let Some(e) = oauth_error(&v) {
        return Err(RefreshError::Refused(e));
    }
    if !(200..300).contains(&res.status) {
        return Err(RefreshError::Refused(format!("GitLab answered {}", res.status)));
    }
    grant(&v).ok_or_else(|| RefreshError::Failed("no token in GitLab's answer".into()))
}

/// Checks `token` against GitLab: who it is (`GET /user`), and its scopes when it is an access token
/// (`GET /personal_access_tokens/self`; an OAuth token's are not read).
pub fn whoami(t: &dyn Transport, api: &str, token: &str) -> Result<Whoami, String> {
    let api = api_url(api);
    let get = |path: &str| Request { method: "GET", url: format!("{api}/{path}"), headers: headers(token), body: None };
    let res = t.send(&get("user"))?;
    if res.status == 401 {
        return Err("GitLab does not accept this token".into());
    }
    if !(200..300).contains(&res.status) {
        return Err(format!("GitLab answered {}", res.status));
    }
    let v: Value = serde_json::from_str(&res.body).map_err(|e| e.to_string())?;
    let login = v["username"].as_str().filter(|l| !l.is_empty()).ok_or("no username in GitLab's answer")?;
    let scopes = match t.send(&get("personal_access_tokens/self")) {
        Ok(r) if (200..300).contains(&r.status) => serde_json::from_str::<Value>(&r.body)
            .ok()
            .and_then(|v| v["scopes"].as_array().cloned())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        _ => vec![],
    };
    Ok(Whoami { login: login.into(), scopes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_forge::transport::Response;
    use pdt_forge::transport::testing::Recorded;

    #[test]
    fn api_urls_of_gitlab_com_and_a_server() {
        assert_eq!(api_url(" "), GITLAB_API);
        assert_eq!(api_url("https://gitlab.com/api/v4/"), GITLAB_API);
        assert_eq!(api_url("https://gitlab.example.com"), "https://gitlab.example.com/api/v4");
        assert_eq!(api_url("gitlab.example.com/"), "https://gitlab.example.com/api/v4");
        assert_eq!(api_url("https://example.com/gitlab"), "https://example.com/gitlab/api/v4", "a relative root");
        assert_eq!(web_url(""), "https://gitlab.com");
        assert_eq!(web_url("https://git.example.org:8443/api/v4"), "https://git.example.org:8443");
        assert_eq!(web_host("https://Git.Example.org:8443/api/v4"), "git.example.org");
        assert_eq!(web_host(GITLAB_API), GITLAB_HOST);
    }

    #[test]
    fn the_device_flow_gives_a_code_then_a_token_that_renews() {
        let t = Recorded::new();
        t.ok(
            "POST",
            "https://gitlab.com/oauth/authorize_device",
            r#"{"device_code":"dc","user_code":"0A44L90H","verification_uri":"https://gitlab.com/oauth/device",
                "verification_uri_complete":"https://gitlab.com/oauth/device?user_code=0A44L90H",
                "expires_in":300,"interval":5}"#,
        );
        let c = start_device_flow(&t, "", "app-id").unwrap();
        assert_eq!(
            (c.user_code.as_str(), c.verification_uri.as_str()),
            ("0A44L90H", "https://gitlab.com/oauth/device")
        );
        assert_eq!(t.sent()[0].body.as_deref(), Some("client_id=app-id&scope=api"));

        let token = "https://gitlab.com/oauth/token";
        t.ok("POST", token, r#"{"error":"authorization_pending","error_description":"pending"}"#)
            .ok("POST", token, r#"{"error":"slow_down"}"#)
            .ok(
                "POST",
                token,
                r#"{"access_token":"glo_1","token_type":"Bearer","expires_in":7200,"refresh_token":"glr_1",
                    "scope":"api","created_at":1793300000}"#,
            )
            .ok("POST", token, r#"{"error":"access_denied"}"#)
            .ok("POST", token, r#"{"error":"expired_token"}"#);
        let poll = || poll_device_flow(&t, "", "app-id", "dc");
        assert_eq!(poll(), DevicePoll::Pending);
        assert_eq!(poll(), DevicePoll::SlowDown(10));
        let want = TokenGrant {
            token: "glo_1".into(),
            expires_in: Some(7200),
            refresh_token: Some("glr_1".into()),
            refresh_token_expires_in: None,
        };
        assert_eq!(poll(), DevicePoll::Token(want));
        assert_eq!(poll(), DevicePoll::Denied);
        assert_eq!(poll(), DevicePoll::Expired);
        assert!(t.sent()[1].body.as_deref().unwrap().contains("grant_type=urn%3Aietf%3Aparams%3Aoauth"));

        t.ok("POST", token, r#"{"access_token":"glo_2","expires_in":7200,"refresh_token":"glr_2"}"#)
            .on(
                "POST",
                token,
                Response::new(
                    400,
                    r#"{"error":"invalid_grant","error_description":"The provided authorization grant is invalid"}"#,
                ),
            )
            .on("POST", token, Response::new(503, "down"));
        let g = refresh_token(&t, "", "app-id", "glr_1").unwrap();
        assert_eq!((g.token.as_str(), g.refresh_token.as_deref()), ("glo_2", Some("glr_2")));
        assert_eq!(
            t.sent().last().unwrap().body.as_deref(),
            Some("client_id=app-id&grant_type=refresh_token&refresh_token=glr_1")
        );
        assert!(
            matches!(refresh_token(&t, "", "app-id", "glr_1"), Err(RefreshError::Refused(m)) if m.contains("invalid"))
        );
        assert!(matches!(refresh_token(&t, "", "app-id", "glr_2"), Err(RefreshError::Failed(_))));

        let t = Recorded::new();
        let e = start_device_flow(&t, "https://old.example.com", "c").unwrap_err();
        assert!(e.contains("17.1"), "an older GitLab: {e}");
    }

    #[test]
    fn whoami_names_the_user_and_an_access_tokens_scopes() {
        let t = Recorded::new();
        let api = "https://gitlab.example.com/api/v4";
        t.ok("GET", &format!("{api}/user"), r#"{"id":7,"username":"alexk","name":"Alex"}"#)
            .ok("GET", &format!("{api}/personal_access_tokens/self"), r#"{"id":1,"scopes":["api","read_repository"]}"#)
            .ok("GET", &format!("{api}/user"), r#"{"id":7,"username":"alexk"}"#)
            .on("GET", &format!("{api}/user"), Response::new(401, r#"{"message":"401 Unauthorized"}"#));
        let w = whoami(&t, "https://gitlab.example.com", "glpat-x").unwrap();
        assert_eq!((w.login.as_str(), w.scopes.clone()), ("alexk", vec!["api".to_string(), "read_repository".into()]));
        assert!(t.sent()[0].headers.contains(&("Authorization", "Bearer glpat-x".into())));
        assert!(whoami(&t, api, "glo_oauth").unwrap().scopes.is_empty(), "an OAuth token: no access token to read");
        assert!(whoami(&t, api, "bad").unwrap_err().contains("does not accept"));
    }
}
