//! Signing in to GitHub (PDT-177): GitHub's hosts (github.com or a GitHub Enterprise Server, by its API URL), the
//! OAuth device flow ("Connect GitHub": the user enters a code on GitHub) and checking a token.

use crate::transport::{Request, Transport, form};
pub use pdt_forge::login::{DeviceCode, DevicePoll, TokenGrant, Whoami};
use serde_json::Value;

/// github.com's API.
pub const GITHUB_API: &str = "https://api.github.com";

/// github.com's web host: the host of the account [`crate::plugin::GithubPlugin::forge`] signs in with.
pub const GITHUB_HOST: &str = "github.com";

/// The OAuth scopes PDT asks for: `repo` reads and writes pull requests, issues and comments, reads Actions runs and
/// their logs, and merges, in public and private repositories.
pub const SCOPES: &str = "repo";

/// An API URL as given (spaces and a trailing slash dropped); github.com's when empty.
pub fn api_url(given: &str) -> String {
    let u = given.trim().trim_end_matches('/');
    if u.is_empty() { GITHUB_API.into() } else { u.into() }
}

/// The site an API URL belongs to: `https://github.com` for github.com's API, else the Enterprise host
/// (`https://ghe.example.com/api/v3` → `https://ghe.example.com`).
pub fn web_url(api: &str) -> String {
    let api = api_url(api);
    if api.eq_ignore_ascii_case(GITHUB_API) {
        return "https://github.com".into();
    }
    api.strip_suffix("/api/v3").map(String::from).unwrap_or(api)
}

/// The GraphQL endpoint of an API URL (Enterprise: `/api/graphql`).
pub fn graphql_url(api: &str) -> String {
    let api = api_url(api);
    match api.strip_suffix("/api/v3") {
        Some(host) => format!("{host}/api/graphql"),
        None => format!("{api}/graphql"),
    }
}

/// The host name of an API URL's site (`github.com`, `ghe.example.com`): the host a project's `origin` is on.
pub fn web_host(api: &str) -> String {
    let w = web_url(api);
    let rest = w.split_once("://").map_or(w.as_str(), |(_, r)| r);
    rest.split(['/', ':']).next().unwrap_or_default().to_ascii_lowercase()
}

fn post_form(t: &dyn Transport, url: String, pairs: &[(&str, &str)]) -> Result<Value, String> {
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
    let v: Value = serde_json::from_str(&res.body).map_err(|_| format!("GitHub answered {}", res.status))?;
    if res.status == 404 {
        return Err("GitHub does not know this OAuth App, or its device flow is off".into());
    }
    if !(200..300).contains(&res.status) {
        let m = v["error_description"].as_str().or(v["message"].as_str()).unwrap_or("an error");
        return Err(format!("GitHub answered {}: {m}", res.status));
    }
    Ok(v)
}

/// Starts a device flow for OAuth App `client_id`.
pub fn start_device_flow(t: &dyn Transport, api: &str, client_id: &str) -> Result<DeviceCode, String> {
    let url = format!("{}/login/device/code", web_url(api));
    let v = post_form(t, url, &[("client_id", client_id), ("scope", SCOPES)])?;
    if let Some(e) = v["error"].as_str() {
        let m = v["error_description"].as_str().unwrap_or(e);
        return Err(match e {
            "device_flow_disabled" => "the OAuth App's device flow is off (enable it in the app's settings)".into(),
            _ => m.to_string(),
        });
    }
    let s = |k: &str| v[k].as_str().map(String::from).ok_or_else(|| format!("no {k} in GitHub's answer"));
    Ok(DeviceCode {
        device_code: s("device_code")?,
        user_code: s("user_code")?,
        verification_uri: s("verification_uri")?,
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

/// Asks whether the user has entered the code yet.
pub fn poll_device_flow(t: &dyn Transport, api: &str, client_id: &str, device_code: &str) -> DevicePoll {
    let url = format!("{}/login/oauth/access_token", web_url(api));
    let pairs = [
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];
    let v = match post_form(t, url, &pairs) {
        Ok(v) => v,
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
        None => DevicePoll::Failed("no token in GitHub's answer".into()),
    }
}

/// The token in an answer of `login/oauth/access_token`, with its expiry and refresh token when the OAuth App issues
/// expiring tokens (PDT-260).
fn grant(v: &Value) -> Option<TokenGrant> {
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
    /// GitHub refused the refresh token (revoked, expired or already used): only signing in again helps.
    Refused(String),
    /// No answer, or a server error: ask again later.
    Failed(String),
}

/// Swaps refresh token `refresh_token` of OAuth App `client_id` for a new token and a new refresh token (the old one
/// stops working). Client ID only, as the device flow asks.
pub fn refresh_token(
    t: &dyn Transport,
    api: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenGrant, RefreshError> {
    let req = Request {
        method: "POST",
        url: format!("{}/login/oauth/access_token", web_url(api)),
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
        return Err(RefreshError::Failed(format!("GitHub answered {}", res.status)));
    }
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    if let Some(e) = v["error"].as_str() {
        return Err(RefreshError::Refused(v["error_description"].as_str().unwrap_or(e).to_string()));
    }
    if !(200..300).contains(&res.status) {
        return Err(RefreshError::Refused(format!("GitHub answered {}", res.status)));
    }
    grant(&v).ok_or_else(|| RefreshError::Failed("no token in GitHub's answer".into()))
}

/// Checks `token` against GitHub: who it is.
pub fn whoami(t: &dyn Transport, api: &str, token: &str) -> Result<Whoami, String> {
    let req =
        Request { method: "GET", url: format!("{}/user", api_url(api)), headers: crate::headers(token), body: None };
    let res = t.send(&req)?;
    if res.status == 401 {
        return Err("GitHub does not accept this token".into());
    }
    if !(200..300).contains(&res.status) {
        return Err(format!("GitHub answered {}", res.status));
    }
    let v: Value = serde_json::from_str(&res.body).map_err(|e| e.to_string())?;
    let login = v["login"].as_str().filter(|l| !l.is_empty()).ok_or("no login in GitHub's answer")?;
    let scopes = res
        .header("x-oauth-scopes")
        .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    Ok(Whoami { login: login.into(), scopes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Response;
    use crate::transport::testing::Recorded;

    #[test]
    fn enterprise_hosts_have_their_own_site_and_graphql() {
        assert_eq!(api_url(" "), GITHUB_API);
        assert_eq!(web_url(""), "https://github.com");
        assert_eq!(graphql_url("https://api.github.com/"), "https://api.github.com/graphql");
        assert_eq!(web_host(GITHUB_API), "github.com");
        let ghe = "https://ghe.example.com/api/v3/";
        assert_eq!(web_url(ghe), "https://ghe.example.com");
        assert_eq!(graphql_url(ghe), "https://ghe.example.com/api/graphql");
        assert_eq!(web_host(ghe), "ghe.example.com");
    }

    #[test]
    fn the_device_flow_gives_a_code_then_a_token() {
        let t = Recorded::new();
        t.ok(
            "POST",
            "https://github.com/login/device/code",
            r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device",
                "expires_in":899,"interval":5}"#,
        );
        let c = start_device_flow(&t, GITHUB_API, "Iv1.x").unwrap();
        assert_eq!((c.user_code.as_str(), c.interval, c.expires_in), ("ABCD-1234", 5, 899));
        let sent = &t.sent()[0];
        assert_eq!(sent.body.as_deref(), Some("client_id=Iv1.x&scope=repo"));
        assert!(sent.headers.contains(&("Accept", "application/json".into())));

        let token = "https://github.com/login/oauth/access_token";
        t.ok("POST", token, r#"{"error":"authorization_pending"}"#)
            .ok("POST", token, r#"{"error":"slow_down","interval":10}"#)
            .ok("POST", token, r#"{"access_token":"gho_x","token_type":"bearer","scope":"repo"}"#)
            .ok("POST", token, r#"{"error":"expired_token"}"#)
            .ok("POST", token, r#"{"error":"access_denied"}"#);
        let poll = || poll_device_flow(&t, GITHUB_API, "Iv1.x", "dc");
        assert_eq!(poll(), DevicePoll::Pending);
        assert_eq!(poll(), DevicePoll::SlowDown(10));
        let plain = TokenGrant { token: "gho_x".into(), ..Default::default() };
        assert_eq!(poll(), DevicePoll::Token(plain), "PDT's own app: a token that doesn't expire");
        assert_eq!(poll(), DevicePoll::Expired);
        assert_eq!(poll(), DevicePoll::Denied);
        assert!(t.sent()[1].body.as_deref().unwrap().contains("grant_type=urn%3Aietf%3Aparams%3Aoauth"));
        assert!(matches!(poll(), DevicePoll::Failed(_)), "a 404 (unknown app)");

        let t = Recorded::new();
        t.ok("POST", "https://ghe.example.com/login/device/code", r#"{"error":"device_flow_disabled"}"#);
        let e = start_device_flow(&t, "https://ghe.example.com/api/v3", "c").unwrap_err();
        assert!(e.contains("device flow is off"), "{e}");
    }

    #[test]
    fn an_expiring_token_comes_with_a_refresh_token_which_renews_it() {
        let t = Recorded::new();
        let token = "https://ghe.example.com/login/oauth/access_token";
        t.ok(
            "POST",
            token,
            r#"{"access_token":"ghu_1","expires_in":28800,"refresh_token":"ghr_1",
                "refresh_token_expires_in":15724799,"token_type":"bearer","scope":""}"#,
        );
        let api = "https://ghe.example.com/api/v3";
        let want = TokenGrant {
            token: "ghu_1".into(),
            expires_in: Some(28800),
            refresh_token: Some("ghr_1".into()),
            refresh_token_expires_in: Some(15_724_799),
        };
        assert_eq!(poll_device_flow(&t, api, "Iv1.own", "dc"), DevicePoll::Token(want));

        t.ok("POST", token, r#"{"access_token":"ghu_2","expires_in":28800,"refresh_token":"ghr_2"}"#)
            .ok("POST", token, r#"{"error":"bad_refresh_token","error_description":"The refresh token passed is incorrect or expired."}"#)
            .on("POST", token, Response::new(502, "<html>"));
        let g = refresh_token(&t, api, "Iv1.own", "ghr_1").unwrap();
        assert_eq!((g.token.as_str(), g.refresh_token.as_deref()), ("ghu_2", Some("ghr_2")));
        assert_eq!(
            t.sent()[1].body.as_deref(),
            Some("client_id=Iv1.own&grant_type=refresh_token&refresh_token=ghr_1"),
            "client ID only, no secret"
        );
        let e = refresh_token(&t, api, "Iv1.own", "ghr_1").unwrap_err();
        assert!(matches!(&e, RefreshError::Refused(m) if m.contains("incorrect or expired")), "{e:?}");
        assert!(matches!(refresh_token(&t, api, "Iv1.own", "ghr_2"), Err(RefreshError::Failed(_))), "a 502: later");
    }

    #[test]
    fn whoami_names_the_login_and_its_scopes() {
        let t = Recorded::new();
        t.on(
            "GET",
            "https://api.github.com/user",
            Response::new(200, r#"{"login":"ross"}"#).with_header("X-OAuth-Scopes", "repo, read:org"),
        );
        t.on("GET", "https://api.github.com/user", Response::new(401, r#"{"message":"Bad credentials"}"#));
        let w = whoami(&t, "", "gho_x").unwrap();
        assert_eq!((w.login.as_str(), w.scopes.clone()), ("ross", vec!["repo".to_string(), "read:org".to_string()]));
        assert!(t.sent()[0].headers.contains(&("Authorization", "Bearer gho_x".into())));
        assert!(whoami(&t, "", "bad").unwrap_err().contains("does not accept"));
    }
}
