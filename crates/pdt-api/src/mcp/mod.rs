//! pdt-mcp: PDT's MCP server at `POST /api/mcp`, Streamable HTTP answered with JSON. Each agent
//! session gets its own bearer token (`<session id>.<HMAC of it under the host key>`), so a tool call knows its
//! session, and through it the project, ticket and agent step. The tools (`tools/`, one module per group) run the
//! API's own handlers.

pub(crate) mod listen;
pub(crate) mod tools;

pub(crate) use listen::{ToolWatch, tools_may_have_changed};

use crate::routes::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pdt_core::conversation::SessionMode;
use pdt_core::mcp::{self as proto, Request, Response as RpcResponse};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const KEY_FILE: &str = "mcp.key";

/// The host key tokens are made with: 32 random bytes in `<data dir>/mcp.key`, made on first use (owner-only).
fn host_key(st: &AppState) -> std::io::Result<Vec<u8>> {
    let mut slot = st.mcp_key.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(k) = slot.as_ref() {
        return Ok(k.clone());
    }
    let path = st.data_dir.join(KEY_FILE);
    // A key file PDT can't read is an error, not a missing key: replacing it would end every session's token.
    let read = match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let key = match read.and_then(|t| hex_decode(t.trim())).filter(|k| k.len() == 32) {
        Some(k) => k,
        None => {
            let mut k = uuid::Uuid::new_v4().as_bytes().to_vec();
            k.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
            write_private(&path, &hex_encode(&k))?;
            k
        }
    };
    *slot = Some(key.clone());
    Ok(key)
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, text)
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// HMAC-SHA256 (RFC 2104).
fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK { Sha256::digest(key).to_vec() } else { key.to_vec() };
    k.resize(BLOCK, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new().chain_update(&ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(&opad).chain_update(inner).finalize().to_vec()
}

fn mac(key: &[u8], session_id: &str) -> String {
    hex_encode(&hmac(key, session_id.as_bytes()))
}

/// The bearer token of a session.
pub(crate) fn token(st: &AppState, session_id: &str) -> std::io::Result<String> {
    Ok(proto::token(session_id, &mac(&host_key(st)?, session_id)))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Why [`authorize`] refused a request.
pub(crate) enum Refused {
    /// No token, a bad one, or one whose session ended: 401.
    Unauthorized,
    /// PDT couldn't read the host key or the session (logged): 503, so a client isn't sent to sign in (PDT-524).
    ReadFailed(&'static str),
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        match self {
            Refused::Unauthorized => unauthorized(),
            Refused::ReadFailed(what) => crate::error::ApiError::unavailable(
                crate::error::codes::READ_FAILED,
                format!("{what}, so the request can't be checked (PDT's log says why)"),
            )
            .into_response(),
        }
    }
}

fn read_failed(what: &'static str, e: impl std::fmt::Display) -> Refused {
    tracing::error!(error = %e, "{what}");
    Refused::ReadFailed(what)
}

/// The session a request's token belongs to, while it exists and has not ended.
async fn authorize(st: &AppState, headers: &HeaderMap) -> Result<Session, Refused> {
    let Some((id, given)) =
        headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()).and_then(proto::parse_token)
    else {
        return Err(Refused::Unauthorized);
    };
    let key = host_key(st).map_err(|e| read_failed("could not read pdt-mcp's host key", e))?;
    if !same(&mac(&key, id), given) {
        return Err(Refused::Unauthorized);
    }
    match st.store.get_session(id).await {
        Ok(Some(s)) if !matches!(s.status, SessionStatus::Finished) && s.kind == SessionKind::AgentTui => Ok(s),
        Ok(_) => Err(Refused::Unauthorized),
        Err(e) => Err(read_failed("could not read the token's session", e)),
    }
}

/// The tools a session gets: its persona's current set, else (no persona) the default set. A running session follows
/// its persona to newer versions for its tools (PDT-501: PDT updates the built-in personas' tools as plugins and
/// releases add them), while its instructions stay those it was launched with; a persona that is gone leaves it its
/// launch version's set. A persona PDT can't read gives none (PDT-414: it fails closed).
pub(crate) async fn session_tools(st: &AppState, s: &Session) -> Vec<String> {
    let Some(p) = &s.persona else {
        return pdt_core::persona::DEFAULT_TOOLS.iter().map(|t| t.to_string()).collect();
    };
    match st.store.get_persona(&p.id).await {
        Ok(Some(current)) => return current.setup.tool_set(st.registry().tool_table()),
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(session = %s.id, persona = %p.id, error = %e, "could not read the session's persona: no tools");
            return vec![];
        }
    }
    match st.store.get_persona_version(&p.id, p.version).await {
        Ok(Some(v)) => v.setup.tool_set(st.registry().tool_table()),
        Ok(None) => {
            tracing::warn!(session = %s.id, persona = %p.id, version = p.version, "the session's persona version is gone: no tools");
            vec![]
        }
        Err(e) => {
            tracing::warn!(session = %s.id, persona = %p.id, error = %e, "could not read the session's persona version: no tools");
            vec![]
        }
    }
}

/// Whether a session runs a locked-down persona (PDT-201).
pub(crate) async fn locked_down(st: &AppState, s: &Session) -> bool {
    let Some(p) = &s.persona else { return false };
    matches!(st.store.get_persona(&p.id).await, Ok(Some(p)) if pdt_core::persona::locked_down(&p.role))
}

/// Whether a session can be given pdt-mcp: every agent conversation, and Claude and Codex terminals (Devin keeps MCP
/// servers in a file of its own).
pub(crate) fn offered(s: &Session) -> bool {
    s.kind == SessionKind::AgentTui && (s.mode == SessionMode::Conversation || s.agent != Some(AgentKind::Devin))
}

/// pdt-mcp for a session's launch, with Claude's config file written; None when it gets none.
pub(crate) async fn server_for(st: &AppState, s: &Session) -> Option<pdt_agents::mcp::McpServer> {
    if !offered(s) {
        return None;
    }
    let token = match token(st, &s.id) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(session = %s.id, error = %e, "could not make the session's pdt-mcp token");
            return None;
        }
    };
    let tools = session_tools(st, s).await;
    let reg = st.registry();
    let mut m = pdt_agents::mcp::McpServer {
        url: format!("http://127.0.0.1:{}/api/mcp", st.listen_port),
        token,
        // Asking tools (PDT-81) stay out of Claude's allowed list, so it asks before each call; one that asks in PDT's
        // Inbox (PDT-475) is in it, so the user isn't asked twice.
        tools: tools.iter().filter(|t| !reg.tool_table().claude_asks(t)).cloned().collect(),
        // ... and each has an ask rule, so no allow rule in a settings file switches Claude's prompt off (PDT-535).
        // Not in Bypass, where PDT asks itself (`tools::confirmable`): Claude would prompt for the rules too.
        asks: if s.asks_nothing() {
            vec![]
        } else {
            tools.iter().filter(|t| reg.tool_table().claude_asks(t)).cloned().collect()
        },
        claude_config: None,
        claude_settings: None,
        pm: crate::pm::is_pm_session(st, s).await,
        locked: locked_down(st, s).await,
        // The user's MCP servers (PDT-322); `agent_mcp::targeted` gives a locked-down session none.
        extra: crate::agent_mcp::servers_for(st, s).await,
        only_pdt: st.store.get_host_settings().await.map(|h| h.mcp_only_pdt).unwrap_or(false),
        codex_user_servers: match (s.agent, st.platform.codex_home.as_deref()) {
            (Some(AgentKind::Codex), Some(home)) => pdt_agents::mcp::codex_config_servers(home),
            _ => vec![],
        },
        codex_shell_exclude: match (s.agent, st.platform.codex_home.as_deref()) {
            (Some(AgentKind::Codex), Some(home)) => pdt_agents::mcp::codex_config_shell_exclude(home),
            _ => vec![],
        },
    };
    if s.agent.unwrap_or(AgentKind::Claude) == AgentKind::Claude {
        let dir: PathBuf = st.data_dir.join("sessions").join(&s.id);
        match pdt_agents::mcp::write_claude_config(&dir, &m) {
            Ok(path) => m.claude_config = Some(path),
            Err(e) => {
                tracing::warn!(session = %s.id, error = %e, "could not write the session's MCP config");
                return None;
            }
        }
        // What this launch's process asks for; a conversation's mode changes without a relaunch, so `confirmable`
        // goes by this, not by the mode it is in now.
        st.claude_ask_rules.lock().unwrap_or_else(|e| e.into_inner()).insert(s.id.clone(), m.asks.clone());
        if s.mode == SessionMode::Conversation {
            // A terminal's hook settings file takes the rules instead (Claude reads one `--settings` file).
            match pdt_agents::mcp::write_claude_settings(&dir, &m) {
                Ok(path) => m.claude_settings = path,
                Err(e) => {
                    tracing::warn!(session = %s.id, error = %e, "could not write the session's permission settings");
                    return None;
                }
            }
        }
    }
    Some(m)
}

const INSTRUCTIONS: &str = "PDT's tools for this agent session: read the ticket you work on, finish a workflow step \
with an outcome, ask the user, and comment. They know which session you are.";

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        axum::Json(json!({"code": "unauthorized", "message": "a valid pdt-mcp session token is required"})),
    )
        .into_response()
}

/// `POST /api/mcp`. A request whose session's tools changed since it listed them, from a client that takes a stream,
/// is answered as one with `notifications/tools/list_changed` first (the client may not hold the `GET` stream open).
pub(crate) async fn post(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let session = match authorize(&st, &headers).await {
        Ok(s) => s,
        Err(r) => return r.into_response(),
    };
    let parsed: Result<Value, _> = serde_json::from_slice(&body);
    let value = match parsed {
        Ok(v) => v,
        Err(e) => {
            return rpc(RpcResponse::err(Value::Null, proto::PARSE_ERROR, format!("not JSON: {e}"))).into_response();
        }
    };
    // A batch (2025-03-26 clients): each request answered, notifications not.
    if let Value::Array(list) = value {
        let mut out = vec![];
        for v in list {
            if let Some(r) = handle(&st, &session, v).await {
                out.push(r);
            }
        }
        if out.is_empty() {
            return StatusCode::ACCEPTED.into_response();
        }
        return axum::Json(out).into_response();
    }
    let method = value.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    match handle(&st, &session, value).await {
        Some(r)
            if listen::accepts_stream(&headers)
                && !matches!(method.as_str(), "initialize" | "tools/list")
                && listen::changed_since_listed(&st, &session).await =>
        {
            listen::notice_then(&r)
        }
        Some(r) => rpc(r).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn rpc(r: RpcResponse) -> axum::Json<RpcResponse> {
    axum::Json(r)
}

/// `DELETE /api/mcp`, and a `GET` that takes no stream: no protocol sessions.
pub(crate) async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

/// Answers one JSON-RPC message; None for a notification.
async fn handle(st: &AppState, session: &Session, v: Value) -> Option<RpcResponse> {
    let req: Request = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => return Some(RpcResponse::err(Value::Null, proto::INVALID_REQUEST, e.to_string())),
    };
    let id = req.id.clone()?;
    let result = match req.method.as_str() {
        "initialize" => {
            let asked = req.params.get("protocolVersion").and_then(Value::as_str);
            Ok(proto::initialize_result(asked, pdt_core::BUILD_VERSION, INSTRUCTIONS))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let list = listen::listed(st, session).await;
            listen::record(st, session, &list);
            Ok(json!({"tools": list}))
        }
        "tools/call" => {
            let Some(name) = req.params.get("name").and_then(Value::as_str) else {
                return Some(RpcResponse::err(id, proto::INVALID_PARAMS, "tools/call needs a tool name"));
            };
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let allowed = session_tools(st, session).await;
            let reg = st.registry();
            if !reg.tool_table().contains(name) {
                return Some(RpcResponse::err(id, proto::INVALID_PARAMS, format!("no tool is called {name}")));
            }
            if !allowed.iter().any(|t| t == name) {
                Ok(proto::tool_error(&format!("{name} is not one of this session's tools")))
            } else {
                let input = args.clone();
                let cx = tools::Cx { st, session, input: &input, confirmed: false };
                // No secret's value ever goes back to an agent (PDT-158).
                let r = st.store.redactor();
                Ok(match reg.call_tool(&cx, name, args).await {
                    Ok(v) => proto::tool_result(&r.json(&v)),
                    Err(e) => proto::tool_error(&r.string(&e)),
                })
            }
        }
        m if m.starts_with("notifications/") => return None,
        m => Err(RpcResponse::err(id.clone(), proto::METHOD_NOT_FOUND, format!("pdt-mcp has no method {m}"))),
    };
    Some(match result {
        Ok(v) => RpcResponse::ok(id, v),
        Err(e) => e,
    })
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new().route("/mcp", post(self::post).get(listen::get).delete(not_allowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2.
        let mac = hmac(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex_encode(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hex_decode("0aff"), Some(vec![10, 255]));
        assert_eq!(hex_decode("abc"), None);
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
