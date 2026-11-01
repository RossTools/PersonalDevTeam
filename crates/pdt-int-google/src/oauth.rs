//! Signing in to Google (PDT-316): OAuth 2.0's authorization code flow with PKCE, with the user's own OAuth client. PDT
//! asks only for `drive.file` (the files PDT makes, and those the user picks in Google's file picker; the Docs and
//! Sheets APIs work on just those) and the account's email (to name it). Google sends the browser back to PDT's
//! callback with a code, which PDT trades for a refresh token it keeps as a secret.

use crate::api::Account;
use base64::Engine;
use pdt_forge::transport::{Request, Transport, encode, form};
use serde_json::Value;
use sha2::Digest;

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
/// Only the files PDT makes or the user picks, and who the account is.
pub const SCOPES: &str = "https://www.googleapis.com/auth/drive.file openid email";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OauthError {
    /// Google said no (a code or refresh token that is wrong, used, expired or taken back).
    Rejected(String),
    Offline(String),
    Failed(String),
}

impl std::fmt::Display for OauthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OauthError::Rejected(m) => write!(f, "Google turned it down: {m}"),
            OauthError::Offline(m) => write!(f, "PDT can't reach Google ({m})"),
            OauthError::Failed(m) => write!(f, "{m}"),
        }
    }
}

/// An access token, and how many seconds it lasts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    pub expires_in: i64,
    /// Given when the user signs in (with `access_type=offline` and consent).
    pub refresh_token: Option<String>,
}

/// Random URL-safe text of `bytes` random bytes.
pub fn random(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("the system's random numbers");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// PKCE's S256 challenge of a verifier.
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// Where the browser goes to sign in: Google's consent page, asking for a refresh token.
pub fn auth_url(client_id: &str, redirect_uri: &str, state: &str, verifier: &str, login_hint: Option<&str>) -> String {
    let mut pairs = vec![
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", SCOPES),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
        ("state", state),
        ("code_challenge_method", "S256"),
    ];
    let c = challenge(verifier);
    pairs.push(("code_challenge", &c));
    if let Some(h) = login_hint {
        pairs.push(("login_hint", h));
    }
    format!("{AUTH_URL}?{}", form(&pairs))
}

fn post_form(t: &dyn Transport, url: &str, body: String) -> Result<Value, OauthError> {
    let req = Request {
        method: "POST",
        url: url.into(),
        headers: vec![
            ("content-type", "application/x-www-form-urlencoded".into()),
            ("accept", "application/json".into()),
        ],
        body: Some(body),
    };
    let res = t.send(&req).map_err(OauthError::Offline)?;
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    if (200..300).contains(&res.status) {
        return Ok(v);
    }
    let what = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no reason given").to_string();
    if res.status == 400 || res.status == 401 {
        Err(OauthError::Rejected(what))
    } else {
        Err(OauthError::Failed(format!("Google's sign-in answered {}: {what}", res.status)))
    }
}

fn token_of(v: &Value) -> Result<Token, OauthError> {
    let access_token = v["access_token"].as_str().ok_or_else(|| OauthError::Failed("Google gave no token".into()))?;
    Ok(Token {
        access_token: access_token.into(),
        expires_in: v["expires_in"].as_i64().unwrap_or(3600),
        refresh_token: v["refresh_token"].as_str().map(str::to_string),
    })
}

/// Trades the code Google sent back for tokens.
pub fn exchange(
    t: &dyn Transport,
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Token, OauthError> {
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ]);
    token_of(&post_form(t, TOKEN_URL, body)?)
}

/// A new access token from the account's refresh token.
pub fn refresh(t: &dyn Transport, a: &Account) -> Result<Token, OauthError> {
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", &a.refresh_token),
        ("client_id", &a.client_id),
        ("client_secret", &a.client_secret),
    ]);
    token_of(&post_form(t, TOKEN_URL, body)?)
}

/// The signed-in account's email.
pub fn email(t: &dyn Transport, access_token: &str) -> Result<String, OauthError> {
    let req = Request {
        method: "GET",
        url: USERINFO_URL.into(),
        headers: vec![("authorization", format!("Bearer {access_token}"))],
        body: None,
    };
    let res = t.send(&req).map_err(OauthError::Offline)?;
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    match v["email"].as_str() {
        Some(e) if (200..300).contains(&res.status) => Ok(e.to_string()),
        _ => Err(OauthError::Failed(format!("Google didn't say which account signed in ({})", res.status))),
    }
}

/// Takes PDT's access back (signing out): Google forgets the refresh token. Errors are only logged by the caller.
pub fn revoke(t: &dyn Transport, token: &str) -> Result<(), OauthError> {
    post_form(t, &format!("{REVOKE_URL}?token={}", encode(token)), String::new()).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_forge::transport::testing::Recorded;

    #[test]
    fn the_consent_page_asks_for_drive_file_and_a_refresh_token() {
        let u =
            auth_url("cid.apps.googleusercontent.com", "http://127.0.0.1:7420/api/google/callback", "st8", "v", None);
        assert!(u.starts_with(AUTH_URL));
        for part in [
            "scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.file%20openid%20email",
            "access_type=offline",
            "prompt=consent",
            "state=st8",
            "code_challenge_method=S256",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A7420%2Fapi%2Fgoogle%2Fcallback",
        ] {
            assert!(u.contains(part), "{part} in {u}");
        }
        assert!(u.contains(&format!("code_challenge={}", challenge("v"))));
    }

    #[test]
    fn pkce_matches_the_rfc() {
        // RFC 7636 appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_ne!(random(32), random(32));
        assert!(random(32).len() >= 43);
    }

    #[test]
    fn a_code_is_traded_for_tokens() {
        let r = Recorded::new();
        r.ok("POST", TOKEN_URL, r#"{"access_token":"a","expires_in":3599,"refresh_token":"r"}"#);
        let t = exchange(&r, "cid", "sec", "code1", "ver", "http://127.0.0.1/cb").unwrap();
        assert_eq!(t, Token { access_token: "a".into(), expires_in: 3599, refresh_token: Some("r".into()) });
        let body = r.sent()[0].body.clone().unwrap();
        assert!(body.contains("grant_type=authorization_code") && body.contains("code_verifier=ver"), "{body}");
        let r = Recorded::new();
        r.on(
            "POST",
            TOKEN_URL,
            pdt_forge::transport::Response::new(400, r#"{"error":"invalid_grant","error_description":"Bad Request"}"#),
        );
        assert!(matches!(exchange(&r, "c", "s", "x", "v", "u"), Err(OauthError::Rejected(_))));
    }
}
