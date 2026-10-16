use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pdt_core::identity::{Caller, IdentityHeaders, check, classify, decode_header_value};
use serde::Serialize;
use ts_rs::TS;

pub(crate) fn caller(headers: &HeaderMap) -> Caller {
    let login =
        headers.get("tailscale-user-login").map(|v| decode_header_value(&String::from_utf8_lossy(v.as_bytes())));
    classify(IdentityHeaders {
        login: login.as_deref(),
        forwarded: headers.contains_key("x-forwarded-for") || headers.contains_key("x-forwarded-host"),
        funnel: headers.contains_key("tailscale-funnel-request"),
    })
}

pub(crate) async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    match check(&caller(req.headers()), st.owner().as_deref()) {
        Ok(()) => next.run(req).await,
        Err(denied) => {
            tracing::warn!(reason = %denied, path = %req.uri().path(), "refused a remote request");
            ApiError::new(StatusCode::FORBIDDEN, "forbidden_identity", denied.to_string()).into_response()
        }
    }
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct WhoAmI {
    /// "local" or "tailnet".
    pub caller: String,
    pub login: Option<String>,
}

pub(crate) async fn whoami(headers: HeaderMap) -> Json<WhoAmI> {
    Json(match caller(&headers) {
        Caller::Tailnet { login } => WhoAmI { caller: "tailnet".into(), login: Some(login) },
        _ => WhoAmI { caller: "local".into(), login: None },
    })
}
