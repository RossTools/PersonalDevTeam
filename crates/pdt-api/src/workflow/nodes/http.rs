//! `http.request` (Phase 7e spec §3.2): one HTTP request, optionally through a connection (its base URL, and its
//! secret sent the way its config says). A status outside `expect` is an error.

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::api::Connection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use ts_rs::TS;

/// The largest response body a step keeps.
pub const BODY_CAP: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct HttpConfig {
    /// A connection: `url` may then be relative to its URL, and its secret is sent (`config.auth`: `bearer`
    /// (default), `basic` with `config.username`, `header` with `config.header`, `query` with `config.param`).
    #[serde(default)]
    #[schemars(extend("x-connection-kind" = "any"))]
    pub connection: Option<String>,
    #[serde(default = "get")]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub query: BTreeMap<String, Value>,
    /// JSON (sent as `application/json`) or a string (sent as it is).
    #[serde(default)]
    #[ts(type = "unknown")]
    #[schemars(extend("x-format" = "code", "x-language" = "json"))]
    pub body: Value,
    /// The statuses that count as success: `"2xx"` (default), `"any"`, or a list (`[200, 404]`).
    #[serde(default = "two_xx")]
    #[ts(type = "string | number[]")]
    pub expect: Value,
}

fn get() -> String {
    "GET".into()
}

fn two_xx() -> Value {
    json!("2xx")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HttpOutput {
    pub status: u16,
    pub ok: bool,
    #[ts(type = "Record<string, string>")]
    pub headers: BTreeMap<String, String>,
    /// Parsed JSON when the response is JSON, else the text.
    #[ts(type = "unknown")]
    pub body: Value,
}

/// Whether `status` is one `expect` allows.
pub fn expected(expect: &Value, status: u16) -> Result<bool, String> {
    match expect {
        Value::String(s) if s == "any" => Ok(true),
        Value::String(s) if s.len() == 3 && s.ends_with("xx") => {
            let class = s[..1].parse::<u16>().map_err(|_| format!("expect {s:?} is not like \"2xx\""))?;
            Ok(status / 100 == class)
        }
        Value::Array(list) => Ok(list.iter().any(|v| v.as_u64() == Some(u64::from(status)))),
        Value::Number(n) => Ok(n.as_u64() == Some(u64::from(status))),
        other => Err(format!("expect {other} is not \"2xx\", \"any\" or a list of statuses")),
    }
}

/// `url` against a connection's base URL; absolute ones stay as they are.
pub fn full_url(base: Option<&str>, url: &str) -> String {
    let absolute = url.starts_with("http://") || url.starts_with("https://");
    match base.filter(|b| !b.is_empty()) {
        Some(b) if !absolute => format!("{}/{}", b.trim_end_matches('/'), url.trim_start_matches('/')),
        _ => url.to_string(),
    }
}

/// Whether `url` is at or below `base` (so a connection's secret never goes to another host).
pub fn under(base: &str, url: &str) -> bool {
    let base = base.trim_end_matches('/');
    !base.is_empty()
        && url.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'))
}

/// The longest connecting to a service may take, for every call through [`client`].
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The HTTP client of workflows' nodes, the sensors and MCP servers. It bounds connecting only: MCP keeps streams open
/// for as long as a server lives, so each caller bounds its own requests (nodes by the step timeout, sensors by
/// `AppState::sensor_timeout`).
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("pdt-workflows/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("an HTTP client")
    })
}

/// A request's error without its URL, with its causes. reqwest's own message ends with the URL, and a URL can be a
/// secret (a webhook URL's token, a key sent as a query parameter): every outbound call's error goes through
/// this before it reaches a step, a run, a notification or the log. A connection macOS's Local Network permission
/// blocked names the setting instead of "No route to host" (PDT-156).
pub fn send_error(e: reqwest::Error) -> String {
    let host = e.url().and_then(|u| u.host_str()).map(String::from);
    let e = e.without_url();
    if let Some(m) = pdt_core::local_network::explain_failure(cfg!(target_os = "macos"), host.as_deref(), os_error(&e))
    {
        return m;
    }
    let mut out = e.to_string();
    let mut cause = std::error::Error::source(&e);
    while let Some(c) = cause {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cause = c.source();
    }
    out
}

/// The OS error code somewhere in an error's causes (hyper wraps a failed connect's `io::Error`).
fn os_error(e: &(dyn std::error::Error + 'static)) -> Option<i32> {
    let mut cause = Some(e);
    while let Some(c) = cause {
        if let Some(code) = c.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error) {
            return Some(code);
        }
        cause = c.source();
    }
    None
}

/// Adds a connection's secret to a request, the way its config says.
pub fn authorize(
    req: reqwest::RequestBuilder,
    conn: &Connection,
    secret: Option<&str>,
) -> Result<reqwest::RequestBuilder, String> {
    let Some(secret) = secret else { return Ok(req) };
    let cfg = |k: &str| conn.config.get(k).and_then(|v| v.as_str()).map(String::from);
    Ok(match cfg("auth").as_deref().unwrap_or("bearer") {
        "bearer" => req.bearer_auth(secret),
        "basic" => req.basic_auth(cfg("username").unwrap_or_default(), Some(secret)),
        "header" => req.header(cfg("header").ok_or("the connection's auth is header but it names no header")?, secret),
        "query" => req.query(&[(cfg("param").ok_or("the connection's auth is query but it names no param")?, secret)]),
        "none" => req,
        other => return Err(format!("the connection's auth {other:?} is not bearer, basic, header, query or none")),
    })
}

/// Reads a response body, up to `cap` bytes.
pub async fn read_body(mut resp: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(send_error)? {
        let room = cap.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if chunk.len() > room {
            break;
        }
    }
    Ok(out)
}

/// A connection of `kind` and its secret, for kinds made for one service (an integration's).
pub async fn service(st: &crate::routes::AppState, name: &str, kind: &str) -> Result<(Connection, String), String> {
    if name.trim().is_empty() {
        return Err(format!("connection is empty: name a {kind} connection"));
    }
    let (conn, secret) = crate::workflow::routes::connection_with_secret(st, name.trim()).await?;
    if conn.kind != kind {
        return Err(format!("{} is a {} connection, not {kind}", conn.name, conn.kind));
    }
    if conn.url.trim().is_empty() {
        return Err(format!("{} has no address", conn.name));
    }
    let secret = secret.filter(|s| !s.is_empty()).ok_or_else(|| format!("{} has no secret", conn.name))?;
    Ok((conn, secret))
}

/// One JSON call to a service: `path` under its `base` URL, with `headers` (its key); a status other than 2xx is an
/// error naming `who`, the status and the start of the answer. An empty answer is null.
pub async fn call_json(
    who: &str,
    method: reqwest::Method,
    base: &str,
    path: &str,
    headers: &[(&str, &str)],
    query: &[(&str, String)],
    body: Option<&Value>,
) -> Result<Value, String> {
    call_json_capped(who, method, base, path, headers, query, body, BODY_CAP).await
}

/// [`call_json`] reading at most `cap` bytes of the answer (a GraphQL schema's introspection is bigger than most).
#[allow(clippy::too_many_arguments)]
pub async fn call_json_capped(
    who: &str,
    method: reqwest::Method,
    base: &str,
    path: &str,
    headers: &[(&str, &str)],
    query: &[(&str, String)],
    body: Option<&Value>,
    cap: usize,
) -> Result<Value, String> {
    let url = full_url(Some(base), path);
    let mut req = client().request(method.clone(), &url).header("accept", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    if !query.is_empty() {
        req = req.query(query);
    }
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| format!("{who}: {method} {path}: {}", send_error(e)))?;
    let status = resp.status().as_u16();
    let bytes = read_body(resp, cap).await?;
    let text = String::from_utf8_lossy(&bytes);
    if !(200..300).contains(&status) {
        let start: String = text.chars().take(300).collect();
        return Err(format!("{who} answered {status} to {method} {path}: {start}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("{who}: {method} {path} did not answer JSON: {e}"))
}

pub struct Http;

impl NodeKind for Http {
    fn spec(&self) -> KindSpec {
        spec::<HttpConfig, HttpOutput>(
            "http.request",
            1,
            Category::Action,
            "HTTP request",
            "Sends an HTTP request, through a connection (its URL and secret) or to a full URL. A status outside \
             `expect` fails the step; the response is `{{steps.<id>.body}}`.",
            &[("out", "Then")],
            false,
        )
    }

    /// GET, HEAD, PUT, DELETE and OPTIONS can be sent again; other methods (POST, PATCH) cut short aren't sent again on
    /// their own (PDT-131).
    fn rerun_safe(&self, input: &Value) -> bool {
        let method = input.get("method").and_then(Value::as_str).unwrap_or("GET");
        ["GET", "HEAD", "PUT", "DELETE", "OPTIONS"].contains(&method.to_uppercase().as_str())
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: HttpConfig = parse(config)?;
        if c.url.trim().is_empty() && c.connection.is_none() {
            return Err("url is empty".into());
        }
        if !c.method.contains("{{") {
            reqwest::Method::from_bytes(c.method.to_uppercase().as_bytes())
                .map_err(|_| format!("{:?} is not an HTTP method", c.method))?;
        }
        expected(&c.expect, 200).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: HttpConfig = parse(&input)?;
            let conn = match &c.connection {
                Some(name) if !name.trim().is_empty() => {
                    Some(crate::workflow::routes::connection_with_secret(cx.st, name).await?)
                }
                _ => None,
            };
            let base = conn.as_ref().map(|(c, _)| c.url.as_str());
            let url = full_url(base, &c.url);
            let method = c.method.to_uppercase();
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("{method} {url}")})));
            }
            let method =
                reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| format!("{method} is not a method"))?;
            let mut req = client().request(method.clone(), &url);
            if let Some((conn, secret)) = &conn {
                // The secret only goes to the connection's own address.
                if under(&conn.url, &url) {
                    req = authorize(req, conn, secret.as_deref())?;
                }
            }
            for (k, v) in &c.headers {
                req = req.header(k, v);
            }
            let query: Vec<(String, String)> = c
                .query
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                .collect();
            if !query.is_empty() {
                req = req.query(&query);
            }
            req = match &c.body {
                Value::Null => req,
                Value::String(s) => req.body(s.clone()),
                v => req.json(v),
            };
            let resp = req.send().await.map_err(|e| format!("{method} {url}: {}", send_error(e)))?;
            let status = resp.status().as_u16();
            let headers: BTreeMap<String, String> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
                .collect();
            let is_json = headers.get("content-type").is_some_and(|t| t.contains("json"));
            let bytes = read_body(resp, BODY_CAP).await?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let body =
                if is_json { serde_json::from_str(&text).unwrap_or(Value::String(text.clone())) } else { json!(text) };
            if !expected(&c.expect, status)? {
                let start: String = text.chars().take(300).collect();
                return Err(format!("{method} {url} answered {status}: {start}"));
            }
            Ok(Outcome::next("out", HttpOutput { status, ok: (200..300).contains(&status), headers, body }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_error_is_found_among_the_causes() {
        #[derive(Debug)]
        struct Wrap(std::io::Error);
        impl std::fmt::Display for Wrap {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("tcp connect error")
            }
        }
        impl std::error::Error for Wrap {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        assert_eq!(os_error(&Wrap(std::io::Error::from_raw_os_error(65))), Some(65));
        assert_eq!(os_error(&Wrap(std::io::Error::other("no code"))), None);
    }

    #[test]
    fn expect_takes_a_class_any_or_a_list() {
        assert!(expected(&json!("2xx"), 204).unwrap());
        assert!(!expected(&json!("2xx"), 404).unwrap());
        assert!(expected(&json!("4xx"), 404).unwrap());
        assert!(expected(&json!("any"), 500).unwrap());
        assert!(expected(&json!([200, 404]), 404).unwrap());
        assert!(!expected(&json!([200]), 201).unwrap());
        assert!(expected(&json!("ok"), 200).is_err());
    }

    #[test]
    fn relative_urls_join_the_connection() {
        assert_eq!(full_url(Some("http://h:7878/api/"), "/v3/movie"), "http://h:7878/api/v3/movie");
        assert_eq!(full_url(Some("http://h"), "https://other/x"), "https://other/x");
        assert_eq!(full_url(None, "https://x/y"), "https://x/y");
        assert!(under("http://h:1/api", "http://h:1/api/v3"));
        assert!(under("http://h/", "http://h?x=1"));
        assert!(!under("http://h", "http://h.evil.com/x"));
        assert!(!under("", "http://h"));
    }
}
