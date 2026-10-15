use crate::error::ApiError;
use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Rejects requests whose `Host` header is not in `allowed_hosts` (DNS-rebinding guard), and requests whose
/// `Origin` header (when present) is not an http(s) origin whose `host[:port]` is in `allowed_hosts`
/// (cross-site WebSocket / CSRF guard). Matching is case-insensitive and includes the port. Requests without a `Host` header are allowed.
pub fn with_host_guard(router: Router, allowed_hosts: Vec<String>) -> Router {
    let allowed: Arc<Vec<String>> = Arc::new(allowed_hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect());
    router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let allowed = allowed.clone();
        async move {
            let headers = req.headers();
            if let Some(host) = headers.get(header::HOST) {
                let host = String::from_utf8_lossy(host.as_bytes()).into_owned();
                if !allowed.contains(&host.to_ascii_lowercase()) {
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
fn origin_allowed(origin: &str, allowed: &[String]) -> bool {
    let lower = origin.to_ascii_lowercase();
    let Some(authority) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) else {
        return false;
    };
    !authority.is_empty() && !authority.contains('/') && allowed.iter().any(|h| h == authority)
}
