//! Settings → MCP servers (PDT-322): the `mcp` connections with what agents get of them, Test, and OAuth sign-in.
//! Servers are added, changed and removed as connections (`/api/connections`); their agent settings are the
//! connection's `config.agents`.

use crate::error::ApiError;
use crate::mcp_client::{McpProbe, McpTools};
use crate::mcp_oauth::OAuthStatus;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::{Path, State};
use pdt_core::mcp_servers::{self as ms, AgentAccess};
use pdt_core::workflow::api::Connection;
use serde::Serialize;
use ts_rs::TS;

/// One server, as Settings shows it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpServerView {
    pub connection: Connection,
    /// The name agents see it under (`mcp__<name>__<tool>`).
    pub agent_name: String,
    pub access: AgentAccess,
    /// `stdio`, `http` or `sse`.
    pub transport: String,
    /// Its sign-in, when it signs in with OAuth.
    pub oauth: Option<OAuthStatus>,
    /// Its tools as last listed (Test, or PDT's own use).
    pub tools: Option<McpTools>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpServersView {
    pub servers: Vec<McpServerView>,
    /// "Use only PDT's MCP servers" (host settings' `mcpOnlyPdt`).
    pub only_pdt: bool,
}

async fn view(st: &AppState, c: Connection) -> McpServerView {
    let oauth = match ms::oauth_of(&c.config) {
        Some(_) => {
            Some(crate::mcp_oauth::status_of(crate::mcp_oauth::tokens(st, &c).await.ok().flatten().as_ref(), now_ms()))
        }
        None => None,
    };
    McpServerView {
        agent_name: ms::agent_name(&c.name),
        access: ms::access_of(&c.config),
        transport: crate::mcp_client::transport(&c).to_string(),
        tools: st.mcp.cached(&c.name),
        oauth,
        connection: c,
    }
}

/// `GET /api/mcp-servers`.
pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<McpServersView>, ApiError> {
    let mut servers = vec![];
    for c in crate::agent_mcp::servers(&st).await {
        servers.push(view(&st, c).await);
    }
    let only_pdt = st.store.get_host_settings().await?.mcp_only_pdt;
    Ok(Json(McpServersView { servers, only_pdt }))
}

async fn load(st: &AppState, id: &str) -> Result<Connection, ApiError> {
    st.store
        .get_connection(id)
        .await?
        .filter(|c| c.kind == crate::mcp_client::MCP)
        .ok_or_else(|| ApiError::not_found("MCP server"))
}

/// `POST /api/mcp-servers/{id}/test`: connects and lists the server's tools, resources and prompts.
pub(crate) async fn test(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<McpProbe>, ApiError> {
    let c = load(&st, &id).await?;
    Ok(Json(st.mcp.probe(&st, &c.name).await))
}

pub(crate) fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/mcp-servers", get(list))
        .route("/mcp-servers/{id}/test", post(test))
        .route("/mcp-servers/{id}/sign-in", post(crate::mcp_oauth::sign_in).delete(crate::mcp_oauth::sign_out))
        .route("/mcp-oauth/callback", get(crate::mcp_oauth::callback))
}
