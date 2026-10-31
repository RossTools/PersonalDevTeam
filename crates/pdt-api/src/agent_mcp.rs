//! The user's MCP servers in agent sessions (PDT-322): which `mcp` connections a session gets
//! (`pdt_core::mcp_servers::AgentAccess`), and each one resolved for a launch, with its secrets and OAuth sign-in
//! filled in (`pdt_agents::mcp::ExtraServer`). PDT's own client (`mcp_client`: workflows, Settings → MCP servers →
//! Test) starts servers from the same resolution.

use crate::routes::AppState;
use crate::vault::Usage;
use pdt_agents::mcp::{ExtraServer, Transport};
use pdt_core::mcp_servers::{self as ms, Audience, ValueRef};
use pdt_core::secret::User;
use pdt_core::session::Session;
use pdt_core::workflow::api::Connection;
use std::collections::HashMap;

/// A server ready to start: how, and which of its header and environment names hold secrets.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Resolved {
    pub transport: Transport,
    pub secret_keys: Vec<String>,
}

/// Who a resolution is for: an agent session (its secrets' project and persona limits apply), or the user (PDT's own
/// client: workflows, Test).
pub(crate) enum For<'a> {
    Session(&'a Session),
    User,
}

fn text(conn: &Connection, key: &str) -> Option<String> {
    conn.config.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// `conn` resolved: its headers' and environment's secrets, the connection's own secret (a program's `secretEnv`, an
/// address's bearer token), and its OAuth sign-in (a bearer token, refreshed if due). Err says what is missing.
pub(crate) async fn resolve(
    st: &AppState,
    conn: &Connection,
    own_secret: Option<&str>,
    who: For<'_>,
) -> Result<Resolved, String> {
    crate::mcp_client::check_connection(conn)?;
    let names = ms::secrets_named(&conn.config);
    let values = if names.is_empty() {
        HashMap::new()
    } else {
        match who {
            For::Session(s) => {
                let usage = Usage {
                    by: s.title.clone(),
                    how: format!("MCP server {}", conn.name),
                    session_id: Some(s.id.clone()),
                    run_id: None,
                };
                let user = User::Agent { project: &s.project_id, persona: s.persona.as_ref().map(|p| p.id.as_str()) };
                crate::vault::values_for(st, &names, user, &usage).await?
            }
            For::User => {
                let usage = Usage {
                    by: "PDT".into(),
                    how: format!("MCP server {}", conn.name),
                    session_id: None,
                    run_id: None,
                };
                crate::vault::values_for_user(st, &names, &usage).await?
            }
        }
    };
    let mut secret_keys = vec![];
    let mut fill = |key: &str| -> Vec<(String, String)> {
        ms::named_values(&conn.config, key)
            .into_iter()
            .map(|(k, r)| match r {
                ValueRef::Text(t) => (k, t),
                ValueRef::Secret(n) => {
                    secret_keys.push(k.clone());
                    let v = values.get(&n).cloned().unwrap_or_default();
                    (k, v)
                }
            })
            .collect()
    };
    let transport = match crate::mcp_client::transport(conn) {
        "stdio" => {
            let mut env = fill("env");
            if let (Some(var), Some(secret)) = (text(conn, "secretEnv"), own_secret.filter(|s| !s.is_empty())) {
                env.retain(|(k, _)| *k != var);
                env.push((var.clone(), secret.to_string()));
                secret_keys.push(var);
            }
            let args = conn
                .config
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            Transport::Stdio { command: text(conn, "command").unwrap_or_default(), args, env, cwd: text(conn, "cwd") }
        }
        t => {
            let mut headers = fill("headers");
            let bearer = if ms::oauth_of(&conn.config).is_some() {
                Some(crate::mcp_oauth::access_token(st, conn).await?)
            } else {
                own_secret.filter(|s| !s.is_empty()).map(String::from)
            };
            if let Some(token) = bearer {
                headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
                headers.push(("Authorization".into(), format!("Bearer {token}")));
                secret_keys.push("Authorization".into());
            }
            Transport::Http { url: conn.url.trim().to_string(), headers, sse: t == "sse" }
        }
    };
    secret_keys.sort();
    secret_keys.dedup();
    Ok(Resolved { transport, secret_keys })
}

/// The `mcp` connections there are.
pub(crate) async fn servers(st: &AppState) -> Vec<Connection> {
    match st.store.list_connections().await {
        Ok(all) => all.into_iter().filter(|c| c.kind == crate::mcp_client::MCP).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list connections");
            vec![]
        }
    }
}

/// Which kind of session `s` is: the PM's, a ticket's or workflow step's, or one the user started.
async fn audience(st: &AppState, s: &Session) -> (bool, Option<String>, bool) {
    let pm = crate::pm::is_pm_session(st, s).await;
    let persona = s.persona.as_ref().map(|p| p.id.clone());
    let ticket = s.attempt_id.is_some() || persona.is_some();
    (pm, persona, ticket)
}

/// The servers a session gets, by connection: none for a locked-down session (PDT-201), nor a server whose every
/// tool is blocked.
pub(crate) async fn targeted(st: &AppState, s: &Session) -> Vec<Connection> {
    if crate::mcp::locked_down(st, s).await {
        return vec![];
    }
    let (pm, persona, ticket) = audience(st, s).await;
    let who = if pm {
        Audience::Pm
    } else if ticket {
        Audience::Ticket { persona: persona.as_deref() }
    } else {
        Audience::User
    };
    servers(st)
        .await
        .into_iter()
        .filter(|c| {
            let a = ms::access_of(&c.config);
            a.reaches(who, &s.project_id) && !a.tools.plan(&known_tools(st, c)).all_blocked()
        })
        .collect()
}

/// The server's tools as last listed (Test, or PDT's own use).
fn known_tools(st: &AppState, c: &Connection) -> Vec<String> {
    st.mcp.cached(&c.name).map(|t| t.tools.into_iter().map(|t| t.name).collect()).unwrap_or_default()
}

/// The names agents see a session's servers under (the agent context note).
pub(crate) async fn names_for(st: &AppState, s: &Session) -> Vec<String> {
    if !crate::mcp::offered(s) {
        return vec![];
    }
    targeted(st, s).await.iter().map(|c| ms::agent_name(&c.name)).collect()
}

/// The servers a session's launch gets, resolved. One that can't be (a missing secret, a sign-in that expired) is left
/// out, with a warning in the log.
pub(crate) async fn servers_for(st: &AppState, s: &Session) -> Vec<ExtraServer> {
    let mut out: Vec<ExtraServer> = vec![];
    for c in targeted(st, s).await {
        let own = match crate::workflow::routes::connection_with_secret(st, &c.name).await {
            Ok((_, secret)) => secret,
            Err(e) => {
                tracing::warn!(session = %s.id, server = %c.name, error = %e, "could not read an MCP server's secret");
                None
            }
        };
        match resolve(st, &c, own.as_deref(), For::Session(s)).await {
            Ok(r) => {
                let name = ms::agent_name(&c.name);
                if out.iter().any(|x| x.name == name) {
                    tracing::warn!(session = %s.id, server = %c.name, "two MCP servers share an agent name; one is left out");
                    continue;
                }
                out.push(ExtraServer {
                    name,
                    transport: r.transport,
                    plan: ms::access_of(&c.config).tools.plan(&known_tools(st, &c)),
                    secret_keys: r.secret_keys,
                });
            }
            Err(e) => tracing::warn!(session = %s.id, server = %c.name, error = %e, "an MCP server was left out"),
        }
    }
    out
}
