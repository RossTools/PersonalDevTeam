use crate::error::{ApiError, codes};
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// The Host/Origin allowlist, shared so remote access can add the tailnet name at runtime. Lower-case `host[:port]`.
#[derive(Clone, Default)]
pub struct AllowedHosts(Arc<std::sync::RwLock<Vec<String>>>);

impl AllowedHosts {
    pub fn new(hosts: Vec<String>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect())))
    }
    pub fn contains(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.0.read().unwrap_or_else(|e| e.into_inner()).contains(&host)
    }
    pub fn add(&self, host: &str) {
        let host = host.to_ascii_lowercase();
        let mut v = self.0.write().unwrap_or_else(|e| e.into_inner());
        if !v.contains(&host) {
            v.push(host);
        }
    }
    pub fn remove(&self, host: &str) {
        let host = host.to_ascii_lowercase();
        self.0.write().unwrap_or_else(|e| e.into_inner()).retain(|h| *h != host);
    }
    pub fn snapshot(&self) -> Vec<String> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl From<Vec<String>> for AllowedHosts {
    fn from(v: Vec<String>) -> Self {
        AllowedHosts::new(v)
    }
}

/// Rejects requests whose `Host` header is not in `allowed` (DNS-rebinding guard), and requests whose
/// `Origin` header (when present) is not an http(s) origin whose `host[:port]` is in `allowed`
/// (cross-site WebSocket / CSRF guard). Matching is case-insensitive and includes the port. Requests without a `Host`
/// header are allowed. The list is read on every request, so changes to a shared [`AllowedHosts`] apply at once.
/// A change (any method but GET, HEAD and OPTIONS) whose `Sec-Fetch-Site` says a browser sent it from another site, or
/// another origin on this one, is refused too (CSRF defence in depth, PDT-419); curl and agents send no such header.
pub fn with_host_guard(router: Router, allowed: impl Into<AllowedHosts>) -> Router {
    let allowed: AllowedHosts = allowed.into();
    router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let allowed = allowed.clone();
        async move {
            let headers = req.headers();
            if let Some(host) = headers.get(header::HOST) {
                let host = String::from_utf8_lossy(host.as_bytes()).into_owned();
                if !allowed.contains(&host) {
                    return ApiError::forbidden(codes::FORBIDDEN_HOST, format!("host {host} is not allowed"))
                        .into_response();
                }
            }
            if let Some(origin) = headers.get(header::ORIGIN) {
                let origin = String::from_utf8_lossy(origin.as_bytes()).into_owned();
                if !origin_allowed(&origin, &allowed) {
                    return ApiError::forbidden(codes::FORBIDDEN_ORIGIN, format!("origin {origin} is not allowed"))
                        .into_response();
                }
            }
            if !fetch_site_allowed(req.method(), headers) {
                return ApiError::forbidden(codes::FORBIDDEN_ORIGIN, "cross-site requests may not change anything")
                    .into_response();
            }
            let res: Response = next.run(req).await;
            res
        }
    }))
}

/// An Origin is allowed when it is http(s) and its `host[:port]` is an allowed Host value.
fn origin_allowed(origin: &str, allowed: &AllowedHosts) -> bool {
    let lower = origin.to_ascii_lowercase();
    let Some(authority) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) else {
        return false;
    };
    !authority.is_empty() && !authority.contains('/') && allowed.contains(authority)
}

/// Reads leave alone; a change needs `Sec-Fetch-Site` absent (not a browser), `same-origin` or `none` (typed by the
/// user).
fn fetch_site_allowed(method: &Method, headers: &HeaderMap) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    headers.get("sec-fetch-site").is_none_or(|v| {
        let v = v.as_bytes();
        v.eq_ignore_ascii_case(b"same-origin") || v.eq_ignore_ascii_case(b"none")
    })
}

/// A route's optional JSON body, read from raw bytes: empty (or only whitespace) is `T::default()`, and anything else
/// must come as `application/json`, so a cross-site form or `text/plain` fetch, which needs no preflight, can't send
/// it (PDT-419).
pub(crate) fn optional_json<T: serde::de::DeserializeOwned + Default>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    if !is_json(headers) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            codes::INVALID_INPUT,
            "Expected request with `Content-Type: application/json`",
        ));
    }
    serde_json::from_slice(body).map_err(|e| ApiError::invalid(e.to_string()))
}

/// `application/json` or `application/<x>+json`, as axum's `Json` takes.
fn is_json(headers: &HeaderMap) -> bool {
    let Some(ct) = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let essence = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    essence == "application/json" || (essence.starts_with("application/") && essence.ends_with("+json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn changes_need_a_same_origin_or_absent_fetch_site() {
        for site in ["same-origin", "none", "Same-Origin"] {
            assert!(fetch_site_allowed(&Method::POST, &headers(&[("sec-fetch-site", site)])), "{site}");
        }
        assert!(fetch_site_allowed(&Method::DELETE, &headers(&[])));
        for site in ["cross-site", "same-site", "bogus"] {
            for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                assert!(!fetch_site_allowed(&m, &headers(&[("sec-fetch-site", site)])), "{m} {site}");
            }
            for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
                assert!(fetch_site_allowed(&m, &headers(&[("sec-fetch-site", site)])), "{m} {site}");
            }
        }
    }

    #[derive(serde::Deserialize, Default, Debug, PartialEq)]
    struct Body {
        #[serde(default)]
        a: u8,
    }

    #[test]
    fn optional_json_takes_empty_bodies_and_json_only() {
        assert_eq!(optional_json::<Body>(&headers(&[]), b"").unwrap(), Body::default());
        assert_eq!(
            optional_json::<Body>(&headers(&[("content-type", "text/plain")]), b" \n").unwrap(),
            Body::default()
        );
        let json = headers(&[("content-type", "application/json; charset=utf-8")]);
        assert_eq!(optional_json::<Body>(&json, br#"{"a":2}"#).unwrap(), Body { a: 2 });
        assert_eq!(
            optional_json::<Body>(&headers(&[("content-type", "application/merge+json")]), b"{}").unwrap(),
            Body::default()
        );
        for ct in [None, Some("text/plain"), Some("application/x-www-form-urlencoded"), Some("multipart/form-data")] {
            let h = ct.map_or_else(HeaderMap::new, |ct| headers(&[("content-type", ct)]));
            let e = optional_json::<Body>(&h, br#"{"a":2}"#).unwrap_err();
            assert_eq!(e.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct:?}");
        }
        assert_eq!(optional_json::<Body>(&json, b"{").unwrap_err().status(), StatusCode::BAD_REQUEST);
    }
}
