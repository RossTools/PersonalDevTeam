//! Security headers on every response (PDT-409): no page may frame PDT (clickjacking), browsers don't sniff types or
//! send referrers, and the web UI runs under a Content-Security-Policy that keeps script to PDT's own bundle and the
//! exact origins the enabled plugins name ([`crate::Integration::web_sources`], PDT-499).
//!
//! A response that sets its own CSP keeps it (attachments are served sandboxed).

use crate::integrations::WebSources;
use crate::routes::AppState;
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS};
use axum::middleware::Next;
use axum::response::Response;

/// The API's policy: its responses are data, never a page to frame.
pub(crate) const API_CSP: &str = "frame-ancestors 'none'";

/// The web UI's policy, with `plugins`' sources. Script only from PDT itself (IronCalc's sheet editor compiles
/// WebAssembly) and the plugins; styles may be inline because CodeMirror, xterm and the toasts inject them; images
/// from anywhere, as Markdown links them; frames from PDT and the plugins (an embedded document); connections
/// (`fetch`, the WebSockets) only to PDT.
pub(crate) fn web_csp(plugins: &[WebSources]) -> String {
    let with = |own: &str, of: fn(&WebSources) -> &'static [&'static str]| {
        let mut v = vec![own];
        for s in plugins.iter().flat_map(of) {
            if !v.contains(s) {
                v.push(s);
            }
        }
        v.join(" ")
    };
    format!(
        "default-src 'self'; script-src {}; style-src 'self' 'unsafe-inline'; img-src * data: blob:; \
         font-src 'self' data:; connect-src 'self'; frame-src {}; worker-src 'self' blob:; object-src 'none'; \
         base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        with("'self' 'wasm-unsafe-eval'", |s| s.script),
        with("'self'", |s| s.frame),
    )
}

pub(crate) async fn add(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if !h.contains_key(CONTENT_SECURITY_POLICY) {
        let csp = if api {
            HeaderValue::from_static(API_CSP)
        } else {
            let sources: Vec<WebSources> = st.registry().integrations().iter().map(|i| i.web_sources()).collect();
            // A plugin's sources are its own constants; one that isn't a valid header leaves the policy without them.
            HeaderValue::try_from(web_csp(&sources)).unwrap_or_else(|_| HeaderValue::from_static(WEB_CSP_ALONE))
        };
        h.insert(CONTENT_SECURITY_POLICY, csp);
    }
    res
}

/// [`web_csp`] with no plugin's sources.
const WEB_CSP_ALONE: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src * data: blob:; font-src 'self' data:; connect-src 'self'; \
     frame-src 'self'; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'self'; \
     frame-ancestors 'none'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_plugins_everything_is_pdts_own() {
        assert_eq!(web_csp(&[]), WEB_CSP_ALONE);
    }

    #[test]
    fn a_plugins_sources_join_their_directives_once() {
        let google = WebSources { script: &["https://apis.google.com"], frame: &["https://docs.google.com"] };
        let csp = web_csp(&[google, WebSources::default(), google]);
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval' https://apis.google.com;"), "{csp}");
        assert!(csp.contains("frame-src 'self' https://docs.google.com;"), "{csp}");
        assert!(csp.contains("connect-src 'self';"), "{csp}");
        assert_eq!(csp.matches("https://apis.google.com").count(), 1, "{csp}");
    }
}
