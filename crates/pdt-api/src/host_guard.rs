use crate::error::ApiError;
use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Rejects requests whose `Host` header is not in `allowed_hosts` (DNS-rebinding guard).
/// Matching is case-insensitive and includes the port. Requests without a `Host` header are allowed.
pub fn with_host_guard(router: Router, allowed_hosts: Vec<String>) -> Router {
    let allowed: Arc<Vec<String>> = Arc::new(allowed_hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect());
    router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let allowed = allowed.clone();
        async move {
            if let Some(host) = req.headers().get(header::HOST) {
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
            let res: Response = next.run(req).await;
            res
        }
    }))
}
