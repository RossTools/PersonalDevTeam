//! pdt-mcp (Phase 7b spec §2): PDT's MCP server at `POST /api/mcp`, Streamable HTTP answered with JSON. Each agent
//! session gets its own bearer token (`<session id>.<HMAC of it under the host key>`), so a tool call knows its
//! session, and through it the project, ticket and agent step. The tools (`tools.rs`) run the API's own handlers.

pub(crate) mod tools;

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
    let key = match std::fs::read_to_string(&path).ok().and_then(|t| hex_decode(t.trim())).filter(|k| k.len() == 32) {
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

/// The session a request's token belongs to, while it exists and has not ended.
async fn authorize(st: &AppState, headers: &HeaderMap) -> Option<Session> {
    let h = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (id, given) = proto::parse_token(h)?;
    let key = host_key(st).ok()?;
    if !same(&mac(&key, id), given) {
        return None;
    }
    let s = st.store.get_session(id).await.ok()??;
    (!matches!(s.status, SessionStatus::Finished) && s.kind == SessionKind::AgentTui).then_some(s)
}

/// The tools a session gets: its persona version's set, else the default set.
pub(crate) async fn session_tools(st: &AppState, s: &Session) -> Vec<String> {
    if let Some(p) = &s.persona
        && let Ok(Some(v)) = st.store.get_persona_version(&p.id, p.version).await
    {
        return v.setup.tool_set();
    }
    pdt_core::persona::DEFAULT_TOOLS.iter().map(|t| t.to_string()).collect()
}

/// Whether a session can be given pdt-mcp: every agent conversation, and Claude and Codex terminals (Devin keeps MCP
/// servers in a file of its own, spec §2.3).
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
    let mut m = pdt_agents::mcp::McpServer {
        url: format!("http://127.0.0.1:{}/api/mcp", st.listen_port),
        token,
        tools: session_tools(st, s).await,
        claude_config: None,
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
    }
    Some(m)
}

const INSTRUCTIONS: &str = "PDT's tools for this agent session: read the ticket you work on, finish a workflow step \
with an outcome, ask the user, and comment. They know which session you are.";

/// `POST /api/mcp`.
pub(crate) async fn post(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(session) = authorize(&st, &headers).await else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            axum::Json(json!({"code": "unauthorized", "message": "a valid pdt-mcp session token is required"})),
        )
            .into_response();
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
    match handle(&st, &session, value).await {
        Some(r) => rpc(r).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn rpc(r: RpcResponse) -> axum::Json<RpcResponse> {
    axum::Json(r)
}

/// `GET` and `DELETE /api/mcp`: no server-initiated stream and no protocol sessions.
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
            let allowed = session_tools(st, session).await;
            let list: Vec<_> = tools::infos().into_iter().filter(|t| allowed.contains(&t.name)).collect();
            Ok(json!({"tools": list}))
        }
        "tools/call" => {
            let Some(name) = req.params.get("name").and_then(Value::as_str) else {
                return Some(RpcResponse::err(id, proto::INVALID_PARAMS, "tools/call needs a tool name"));
            };
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let allowed = session_tools(st, session).await;
            if !tools::infos().iter().any(|t| t.name == name) {
                return Some(RpcResponse::err(id, proto::INVALID_PARAMS, format!("no tool is called {name}")));
            }
            if !allowed.iter().any(|t| t == name) {
                Ok(proto::tool_error(&format!("{name} is not one of this session's tools")))
            } else {
                let cx = tools::Cx { st, session };
                Ok(match tools::call(&cx, name, args).await {
                    Ok(v) => proto::tool_result(&v),
                    Err(e) => proto::tool_error(&e),
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
