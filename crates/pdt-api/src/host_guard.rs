use crate::error::ApiError;
use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
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
pub fn with_host_guard(router: Router, allowed: impl Into<AllowedHosts>) -> Router {
    let allowed: AllowedHosts = allowed.into();
    router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let allowed = allowed.clone();
        async move {
            let headers = req.headers();
            if let Some(host) = headers.get(header::HOST) {
                let host = String::from_utf8_lossy(host.as_bytes()).into_owned();
                if !allowed.contains(&host) {
                    return ApiError::new(
                        StatusCode::FORBIDDEN,
                        "forbidden_host",
                        format!("host {host} is not allowed"),
                    )
                    .into_response();
                }
            }
            if let Some(origin) = headers.get(header::ORIGIN) {
                let origin = String::from_utf8_lossy(origin.as_bytes()).into_owned();
                if !origin_allowed(&origin, &allowed) {
                    return ApiError::new(
                        StatusCode::FORBIDDEN,
                        "forbidden_origin",
                        format!("origin {origin} is not allowed"),
                    )
                    .into_response();
                }
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
