//! Incoming webhooks: `POST /api/hooks/<name>`, where `<name>` is a `webhook` connection whose
//! secret authenticates the sender (a bearer token or GitHub's `X-Hub-Signature-256`). Records a `webhook.received`
//! event; workflows run on the engine's next tick. The identity guard lets these through (the senders are machines on
//! the tailnet with no user identity), never over Funnel; the secret is what lets them in.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use crate::workflow::nodes::webhook::WEBHOOK_CONNECTION;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{WEBHOOK_RECEIVED, WebhookReceived};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::BTreeMap;

/// The largest body a webhook takes.
pub const MAX_BODY: usize = 1024 * 1024;

/// The path prefix the identity guard lets through.
pub const HOOKS_PREFIX: &str = "/api/hooks/";

/// Equal, in time that depends on neither value: it compares their SHA-256 digests, so not even the secret's length
/// shows (PDT-421).
fn same(a: &[u8], b: &[u8]) -> bool {
    use sha2::Digest;
    let (a, b) = (Sha256::digest(a), Sha256::digest(b));
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// How many refused requests one hook name takes in a window before it refuses every request until the window ends.
const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW_MS: i64 = 60_000;
/// The most hook names it remembers: names are the caller's to choose, so the map can't grow without end.
const MAX_TRACKED: usize = 1024;

/// Refused webhook requests per hook name (named or not, so the limit says nothing about which names exist): each
/// name's window start and count. Slows guessing a secret (PDT-421).
#[derive(Default)]
pub(crate) struct HookFailures(std::sync::Mutex<std::collections::HashMap<String, (i64, u32)>>);

impl HookFailures {
    /// Whether `name` has used up its refusals for this window.
    fn limited(&self, name: &str, now: i64) -> bool {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(name).is_some_and(|&(start, n)| now - start < FAILURE_WINDOW_MS && n >= MAX_FAILURES)
    }

    fn failed(&self, name: &str, now: i64) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(name) {
            map.retain(|_, (start, _)| now - *start < FAILURE_WINDOW_MS);
            if map.len() >= MAX_TRACKED {
                map.clear();
            }
        }
        let e = map.entry(name.to_string()).or_insert((now, 0));
        if now - e.0 >= FAILURE_WINDOW_MS {
            *e = (now, 0);
        }
        e.1 += 1;
    }
}

/// Every refusal looks the same, whatever the reason (no such hook, no secret yet, the wrong secret), so a caller
/// can't learn which hooks exist. The log says which it was.
fn refused() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        codes::BAD_SIGNATURE,
        "send the hook's secret as Authorization: Bearer <secret> or an X-Hub-Signature-256",
    )
}

/// Whether the request carries `secret`: `Authorization: Bearer <secret>`, or `X-Hub-Signature-256: sha256=<hex HMAC
/// of the body>`.
pub fn authentic(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(token) = header("authorization").and_then(|a| a.strip_prefix("Bearer ")) {
        return same(token.trim().as_bytes(), secret.as_bytes());
    }
    if let Some(sig) = header("x-hub-signature-256").and_then(|s| s.strip_prefix("sha256="))
        && let Ok(sig) = hex::decode(sig)
        && let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
    {
        mac.update(body);
        return mac.verify_slice(&sig).is_ok();
    }
    false
}

/// Whether a header looks like a credential (`X-Api-Key`, `X-Gitlab-Token`, `X-Hub-Signature-256`, …): never stored.
fn credential(name: &str) -> bool {
    const WORDS: [&str; 8] = ["auth", "token", "secret", "signature", "password", "passwd", "credential", "cookie"];
    WORDS.iter().any(|w| name.contains(w)) || (name.ends_with("key") && !name.ends_with("idempotency-key"))
}

/// The headers a workflow sees: `content-type`, `user-agent` and the `x-*` ones, but no credential.
fn kept_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(n, _)| {
            let n = n.as_str();
            (n == "content-type" || n == "user-agent" || n.starts_with("x-")) && !credential(n)
        })
        .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

/// The sender's delivery id, for dedupe.
fn delivery(headers: &HeaderMap) -> Option<String> {
    ["x-github-delivery", "idempotency-key", "x-request-id"]
        .iter()
        .find_map(|n| headers.get(*n).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

pub(crate) async fn receive(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<BTreeMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if st.hook_failures.limited(&name, now_ms()) {
        tracing::warn!(hook = %name, "refused a webhook request: too many without the secret");
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            codes::TOO_MANY_FAILURES,
            "too many requests without the hook's secret: try again in a minute",
        ));
    }
    let hook = st.store.connection_by_name(&name).await?.filter(|c| c.kind == WEBHOOK_CONNECTION);
    let secret = match &hook {
        Some(hook) => crate::workflow::routes::connection_with_secret(&st, &hook.name)
            .await
            .map_err(ApiError::keychain)?
            .1
            .filter(|s| !s.is_empty()),
        None => None,
    };
    let why = match &secret {
        None if hook.is_none() => Some("there is no webhook connection by that name"),
        None => Some("the webhook has no secret yet: set one on its connection"),
        Some(secret) if !authentic(&headers, &body, secret) => Some("it doesn't carry the hook's secret"),
        Some(_) => None,
    };
    if let Some(why) = why {
        tracing::warn!(hook = %name, "refused a webhook request: {why}");
        st.hook_failures.failed(&name, now_ms());
        return Err(refused());
    }
    let body = serde_json::from_slice::<Value>(&body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
    let key = format!("webhook:{name}:{}", delivery(&headers).unwrap_or_else(|| uuid::Uuid::now_v7().to_string()));
    let payload = WebhookReceived {
        hook: name.clone(),
        method: method.to_string(),
        query,
        headers: kept_headers(&headers),
        body,
    };
    let e = NewLogEvent::new(WEBHOOK_RECEIVED, key, "webhook", payload);
    match st.store.emit_event(&e, now_ms()).await? {
        Some(ev) => {
            tracing::info!(hook = %name, event = %ev.id, "webhook received");
            Ok((StatusCode::ACCEPTED, Json(json!({"eventId": ev.id}))))
        }
        None => Ok((StatusCode::OK, Json(json!({"duplicate": true})))),
    }
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new().route("/hooks/{name}", post(receive).layer(axum::extract::DefaultBodyLimit::max(MAX_BODY)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (n, v) in pairs {
            h.insert(*n, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn a_bearer_secret_or_a_github_signature() {
        assert!(authentic(&headers(&[("authorization", "Bearer s3cret")]), b"{}", "s3cret"));
        assert!(!authentic(&headers(&[("authorization", "Bearer nope")]), b"{}", "s3cret"));
        assert!(!authentic(&headers(&[]), b"{}", "s3cret"));
        // GitHub's documented example: secret "It's a Secret to Everybody", body "Hello, World!".
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        let h = headers(&[("x-hub-signature-256", sig)]);
        assert!(authentic(&h, b"Hello, World!", "It's a Secret to Everybody"));
        assert!(!authentic(&h, b"Hello, World?", "It's a Secret to Everybody"));
        assert!(!authentic(&headers(&[("x-hub-signature-256", "sha256=zz")]), b"x", "s"));
    }

    #[test]
    fn a_name_refused_too_often_is_refused_for_the_rest_of_the_window() {
        let f = HookFailures::default();
        for _ in 0..MAX_FAILURES {
            assert!(!f.limited("deploys", 1_000));
            f.failed("deploys", 1_000);
        }
        assert!(f.limited("deploys", 1_000));
        assert!(!f.limited("other", 1_000), "per name");
        assert!(!f.limited("deploys", 1_000 + FAILURE_WINDOW_MS), "a new window");
        f.failed("deploys", 1_000 + FAILURE_WINDOW_MS);
        assert!(!f.limited("deploys", 1_000 + FAILURE_WINDOW_MS), "it starts counting again");
        for i in 0..MAX_TRACKED + 5 {
            f.failed(&format!("h{i}"), 2_000);
        }
        assert!(f.0.lock().unwrap().len() <= MAX_TRACKED);
    }

    #[test]
    fn workflows_see_some_headers_and_never_a_credential() {
        let h = headers(&[
            ("content-type", "application/json"),
            ("x-github-event", "issues"),
            ("x-hub-signature-256", "sha256=00"),
            ("x-hub-signature", "sha1=00"),
            ("authorization", "Bearer s"),
            ("cookie", "c"),
            ("x-api-key", "k"),
            ("x-gitlab-token", "t"),
            ("x-auth-user", "u"),
            ("x-webhook-secret", "s"),
            ("x-idempotency-key", "i"),
        ]);
        let kept = kept_headers(&h);
        assert_eq!(
            kept.keys().map(String::as_str).collect::<Vec<_>>(),
            ["content-type", "x-github-event", "x-idempotency-key"]
        );
        assert_eq!(delivery(&headers(&[("x-github-delivery", "abc")])).as_deref(), Some("abc"));
        assert_eq!(delivery(&headers(&[])), None);
    }
}
