//! Security headers on every response (PDT-409): no page may frame PDT (clickjacking), browsers don't sniff types or
//! send referrers, and the web UI runs under a Content-Security-Policy that keeps script to PDT's own bundle.
//!
//! A response that sets its own CSP keeps it (attachments are served sandboxed).

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS};
use axum::middleware::Next;
use axum::response::Response;

/// The web UI's policy. Script only from PDT itself (IronCalc's sheet editor compiles WebAssembly); styles may be
/// inline because CodeMirror, xterm and the toasts inject them; images from anywhere, as Markdown links them; frames
/// over https for embedded documents (Google Docs and the like); connections (`fetch`, the WebSockets) only to PDT.
pub(crate) const WEB_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src * data: blob:; font-src 'self' data:; connect-src 'self'; \
     frame-src 'self' https:; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'self'; \
     frame-ancestors 'none'";

/// The API's policy: its responses are data, never a page to frame.
pub(crate) const API_CSP: &str = "frame-ancestors 'none'";

pub(crate) async fn add(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if !h.contains_key(CONTENT_SECURITY_POLICY) {
        h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(if api { API_CSP } else { WEB_CSP }));
    }
    res
}
