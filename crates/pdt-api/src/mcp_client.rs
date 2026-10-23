//! PDT as an MCP client (Phase 7e spec §7.2): MCP servers configured as connections of kind `mcp`, whose tools
//! workflows call (`mcp.tool`) and the catalog lists. One live client per server, started on first use and dropped
//! after 5 minutes idle; each server's tool list cached in memory, refreshed when its connection is saved and every
//! 10 minutes.
//!
//! A connection's `config.transport` is `stdio` (`command`, `args`, `env`, `cwd`; the secret, if any, in the env var
//! `config.secretEnv`) or `http` (the connection's `url`, streamable HTTP; the secret as a bearer token).

use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::Connection;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ts_rs::TS;

pub const MCP: &str = "mcp";

/// How long a server may sit unused before it is stopped.
pub const IDLE: Duration = Duration::from_secs(5 * 60);
/// How old a server's tool list may get before it is listed again.
pub const RELIST_MS: i64 = 10 * 60 * 1000;
/// The longest starting a server (the MCP handshake included) or listing its tools may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How many of a stdio server's last stderr lines are kept, to explain a failed start.
const STDERR_LINES: usize = 20;

/// One tool a server offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpTool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    /// JSON Schema of its arguments.
    #[ts(type = "Record<string, unknown>")]
    pub input_schema: Value,
    #[ts(type = "Record<string, unknown> | null")]
    pub output_schema: Option<Value>,
}

/// A server's tools as last listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpTools {
    /// The connection's name.
    pub server: String,
    pub tools: Vec<McpTool>,
    /// Why the last listing failed (the tools are then from the one before, if any).
    pub error: Option<String>,
    /// When the tools were last listed (ms).
    #[ts(type = "number | null")]
    pub listed_at: Option<i64>,
    /// When a listing was last tried (ms).
    #[ts(type = "number")]
    pub tried_at: i64,
}

struct Live {
    service: RunningService<RoleClient, ()>,
    /// The connection's settings and secret it was started with; a change starts it again.
    fingerprint: String,
    last_used: Instant,
}

/// The live clients and the tool cache.
#[derive(Default)]
pub struct McpPool {
    /// By connection id.
    live: tokio::sync::Mutex<HashMap<String, Live>>,
    /// By connection name.
    tools: Mutex<HashMap<String, McpTools>>,
    /// A background refresh is running.
    refreshing: std::sync::atomic::AtomicBool,
    /// By connection id: held while that server's client is checked and, if need be, started.
    starting: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

fn fingerprint(conn: &Connection, secret: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(&(&conn.kind, &conn.url, &conn.config)).unwrap_or_default());
    h.update(secret.unwrap_or_default());
    hex::encode(h.finalize())
}

fn cfg_str<'a>(conn: &'a Connection, key: &str) -> Option<&'a str> {
    conn.config.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// `stdio` or `http`: what the config says, else `http` when the connection has a URL and no command.
pub fn transport(conn: &Connection) -> &str {
    match cfg_str(conn, "transport") {
        Some(t) => t,
        None if cfg_str(conn, "command").is_none() && !conn.url.trim().is_empty() => "http",
        None => "stdio",
    }
}

/// What is wrong with an `mcp` connection's settings, if anything.
pub fn check_connection(conn: &Connection) -> Result<(), String> {
    match transport(conn) {
        "stdio" => {
            cfg_str(conn, "command").ok_or_else(|| format!("{} has no command (config.command)", conn.name))?;
            if let Some(a) = conn.config.get("args").filter(|a| !a.is_null())
                && !a.as_array().is_some_and(|l| l.iter().all(Value::is_string))
            {
                return Err(format!("{}: config.args must be a list of text", conn.name));
            }
            if let Some(e) = conn.config.get("env").filter(|e| !e.is_null())
                && !e.as_object().is_some_and(|m| m.values().all(Value::is_string))
            {
                return Err(format!("{}: config.env must map names to text", conn.name));
            }
            Ok(())
        }
        "http" if conn.url.starts_with("http://") || conn.url.starts_with("https://") => Ok(()),
        "http" => Err(format!("{} has no http(s) address", conn.name)),
        other => Err(format!("{}: transport {other:?} is not stdio or http", conn.name)),
    }
}

fn stdio_command(conn: &Connection, secret: Option<&str>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cfg_str(conn, "command").unwrap_or_default());
    for a in conn.config.get("args").and_then(|a| a.as_array()).into_iter().flatten() {
        cmd.arg(a.as_str().unwrap_or_default());
    }
    for (k, v) in conn.config.get("env").and_then(|e| e.as_object()).into_iter().flatten() {
        cmd.env(k, v.as_str().unwrap_or_default());
    }
    if let (Some(var), Some(secret)) = (cfg_str(conn, "secretEnv"), secret) {
        cmd.env(var, secret);
    }
    if let Some(dir) = cfg_str(conn, "cwd") {
        cmd.current_dir(dir);
    }
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Starts a client for `conn` (the MCP handshake done).
async fn connect(conn: &Connection, secret: Option<&str>) -> Result<RunningService<RoleClient, ()>, String> {
    check_connection(conn)?;
    match transport(conn) {
        "stdio" => {
            let program = cfg_str(conn, "command").unwrap_or_default().to_string();
            let (proc, stderr) = rmcp::transport::TokioChildProcess::builder(stdio_command(conn, secret))
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("{}: could not start {program}: {e}", conn.name))?;
            let tail = Arc::new(Mutex::new(VecDeque::new()));
            if let Some(stderr) = stderr {
                tokio::spawn(drain_stderr(conn.name.clone(), stderr, tail.clone()));
            }
            match tokio::time::timeout(CONNECT_TIMEOUT, ().serve(proc)).await {
                Ok(Ok(s)) => Ok(s),
                failed => {
                    let why = match failed {
                        Ok(Err(e)) => e.to_string(),
                        _ => "it did not answer in time".to_string(),
                    };
                    // Give its last words a moment to arrive.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let said: Vec<String> = tail.lock().unwrap().iter().cloned().collect();
                    let said = if said.is_empty() { String::new() } else { format!(" ({})", said.join(" / ")) };
                    Err(format!("{}: {program} did not start as an MCP server: {why}{said}", conn.name))
                }
            }
        }
        _ => {
            use rmcp::transport::StreamableHttpClientTransport;
            use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
            let mut cfg = StreamableHttpClientTransportConfig::with_uri(conn.url.trim());
            if let Some(s) = secret {
                cfg = cfg.auth_header(s);
            }
            let t = StreamableHttpClientTransport::with_client(crate::workflow::nodes::http::client().clone(), cfg);
            match tokio::time::timeout(CONNECT_TIMEOUT, ().serve(t)).await {
                Ok(Ok(s)) => Ok(s),
                Ok(Err(e)) => Err(format!("{}: {}: {e}", conn.name, conn.url)),
                Err(_) => Err(format!("{}: {} did not answer in time", conn.name, conn.url)),
            }
        }
    }
}

async fn drain_stderr(server: String, stderr: tokio::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(server = %server, "mcp server: {line}");
        let mut t = tail.lock().unwrap();
        if t.len() == STDERR_LINES {
            t.pop_front();
        }
        t.push_back(line);
    }
}

/// A tool from `tools/list`.
pub fn tool_of(v: &Value) -> Option<McpTool> {
    let s = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    Some(McpTool {
        name: s("name")?,
        title: s("title").or_else(|| v["annotations"]["title"].as_str().map(String::from)),
        description: s("description").unwrap_or_default(),
        input_schema: v.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({"type": "object"})),
        output_schema: v.get("outputSchema").filter(|o| !o.is_null()).cloned(),
    })
}

/// Whether an error means the client is unusable (so the next use starts it again), rather than the server saying no.
fn broken(e: &ServiceError) -> bool {
    !matches!(e, ServiceError::McpError(_))
}

impl McpPool {
    /// A live client for the `mcp` connection called `name`, started if need be.
    async fn peer(&self, st: &AppState, name: &str) -> Result<(String, Peer<RoleClient>), String> {
        let (first, _) = crate::workflow::routes::connection_with_secret(st, name.trim()).await?;
        // One start at a time per server (others are not held up), with its settings read again under the lock: a
        // caller that read them before a change (a listing begun before the secret was saved) then finds the newer
        // client instead of replacing it while it is in use.
        let gate = self.starting.lock().unwrap().entry(first.id.clone()).or_default().clone();
        let _started = gate.lock().await;
        let (conn, secret) = crate::workflow::routes::connection_with_secret(st, name.trim()).await?;
        if conn.kind != MCP {
            return Err(format!("{} is a {} connection, not {MCP}", conn.name, conn.kind));
        }
        let secret = secret.filter(|s| !s.is_empty());
        let fp = fingerprint(&conn, secret.as_deref());
        {
            let mut live = self.live.lock().await;
            if let Some(l) = live.get_mut(&conn.id) {
                if l.fingerprint == fp && !l.service.is_closed() && !l.service.is_transport_closed() {
                    l.last_used = Instant::now();
                    return Ok((conn.id, l.service.peer().clone()));
                }
                if let Some(old) = live.remove(&conn.id) {
                    tokio::spawn(stop(old));
                }
            }
        }
        let service = connect(&conn, secret.as_deref()).await?;
        let peer = service.peer().clone();
        let mut live = self.live.lock().await;
        if let Some(old) = live.insert(conn.id.clone(), Live { service, fingerprint: fp, last_used: Instant::now() }) {
            tokio::spawn(stop(old));
        }
        Ok((conn.id, peer))
    }

    async fn forget(&self, id: &str) {
        if let Some(old) = self.live.lock().await.remove(id) {
            tokio::spawn(stop(old));
        }
    }

    /// Lists the server's tools now and caches them (on failure, the error, keeping the tools from before).
    pub async fn refresh(&self, st: &AppState, name: &str) -> McpTools {
        let listed = async {
            let (id, peer) = self.peer(st, name).await?;
            match tokio::time::timeout(CONNECT_TIMEOUT, peer.list_all_tools()).await {
                Ok(Ok(tools)) => Ok(tools),
                Ok(Err(e)) => {
                    if broken(&e) {
                        self.forget(&id).await;
                    }
                    Err(format!("{name}: listing its tools failed: {e}"))
                }
                Err(_) => Err(format!("{name}: listing its tools took too long")),
            }
        }
        .await;
        let now = now_ms();
        let mut cache = self.tools.lock().unwrap();
        let entry = cache.entry(name.to_string()).or_insert_with(|| McpTools {
            server: name.to_string(),
            tools: vec![],
            error: None,
            listed_at: None,
            tried_at: now,
        });
        entry.tried_at = now;
        match listed {
            Ok(tools) => {
                let tools: Vec<McpTool> =
                    tools.iter().filter_map(|t| tool_of(&serde_json::to_value(t).unwrap_or_default())).collect();
                entry.tools = tools;
                entry.error = None;
                entry.listed_at = Some(now);
            }
            Err(e) => entry.error = Some(e),
        }
        entry.clone()
    }

    /// Calls `tool` on the server called `name`; the result is MCP's `CallToolResult` as JSON.
    pub async fn call(
        &self,
        st: &AppState,
        name: &str,
        tool: &str,
        arguments: Map<String, Value>,
    ) -> Result<Value, String> {
        let (id, peer) = self.peer(st, name).await?;
        let params = CallToolRequestParams::new(tool.to_string()).with_arguments(arguments);
        match peer.call_tool(params).await {
            Ok(r) => serde_json::to_value(r).map_err(|e| e.to_string()),
            Err(e) => {
                if broken(&e) {
                    self.forget(&id).await;
                }
                Err(format!("{name}: {tool}: {e}"))
            }
        }
    }

    /// The cached tools of the server called `name`.
    pub fn cached(&self, name: &str) -> Option<McpTools> {
        self.tools.lock().unwrap().get(name).cloned()
    }

    /// Every server's cached tools, by name.
    pub fn all_cached(&self) -> Vec<McpTools> {
        let mut all: Vec<McpTools> = self.tools.lock().unwrap().values().cloned().collect();
        all.sort_by(|a, b| a.server.cmp(&b.server));
        all
    }

    /// Drops what is cached for servers that are no longer `mcp` connections (deleted or renamed).
    pub fn keep_only(&self, names: &[String]) {
        self.tools.lock().unwrap().retain(|n, _| names.contains(n));
    }

    /// Stops servers unused for `idle`, and those whose connection is gone.
    pub async fn reap(&self, idle: Duration, ids: &[String]) {
        self.starting.lock().unwrap().retain(|id, _| ids.contains(id));
        let mut live = self.live.lock().await;
        let gone: Vec<String> = live
            .iter()
            .filter(|(id, l)| l.last_used.elapsed() >= idle || !ids.contains(id) || l.service.is_closed())
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some(old) = live.remove(&id) {
                tokio::spawn(stop(old));
            }
        }
    }

    /// Stops every server (pdtd is stopping).
    pub async fn shutdown(&self) {
        let all: Vec<Live> = self.live.lock().await.drain().map(|(_, l)| l).collect();
        futures_util::future::join_all(all.into_iter().map(stop)).await;
    }
}

async fn stop(l: Live) {
    // Closing the transport ends a stdio server's input; dropping the client kills one that lingers.
    let _ = tokio::time::timeout(Duration::from_secs(3), l.service.cancel()).await;
}

/// The `mcp` connections there are.
async fn mcp_connections(st: &AppState) -> Vec<Connection> {
    match st.store.list_connections().await {
        Ok(all) => all.into_iter().filter(|c| c.kind == MCP).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list connections");
            vec![]
        }
    }
}

/// Housekeeping (every minute): stops idle servers, forgets deleted ones, and lists again the tools of servers whose
/// list is older than 10 minutes (in the background, one refresh at a time).
pub async fn maintain(st: &AppState) {
    let conns = mcp_connections(st).await;
    let ids: Vec<String> = conns.iter().map(|c| c.id.clone()).collect();
    let names: Vec<String> = conns.iter().map(|c| c.name.clone()).collect();
    st.mcp.reap(IDLE, &ids).await;
    st.mcp.keep_only(&names);
    let now = now_ms();
    let due: Vec<String> =
        names.into_iter().filter(|n| st.mcp.cached(n).is_none_or(|t| now - t.tried_at >= RELIST_MS)).collect();
    if due.is_empty() || st.mcp.refreshing.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        futures_util::future::join_all(due.iter().map(|n| st.mcp.refresh(&st, n))).await;
        st.mcp.refreshing.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// After an `mcp` connection is saved or deleted: forget what is gone, and list a saved one's tools (in the
/// background, so saving never waits on the server).
pub async fn connection_changed(st: &AppState, saved: Option<&Connection>) {
    let names: Vec<String> = mcp_connections(st).await.into_iter().map(|c| c.name).collect();
    st.mcp.keep_only(&names);
    if let Some(c) = saved.filter(|c| c.kind == MCP) {
        let (st, name) = (st.clone(), c.name.clone());
        tokio::spawn(async move {
            st.mcp.refresh(&st, &name).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conn(url: &str, config: Value) -> Connection {
        Connection {
            id: "c".into(),
            name: "srv".into(),
            kind: MCP.into(),
            url: url.into(),
            config,
            has_secret: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn the_transport_follows_the_config() {
        assert_eq!(transport(&conn("", json!({"command": "npx"}))), "stdio");
        assert_eq!(transport(&conn("https://x/mcp", json!({}))), "http");
        assert_eq!(transport(&conn("https://x/mcp", json!({"transport": "stdio", "command": "x"}))), "stdio");
        assert!(
            check_connection(&conn("", json!({"command": "npx", "args": ["-y", "srv"], "env": {"A": "1"}}))).is_ok()
        );
        assert!(check_connection(&conn("", json!({}))).unwrap_err().contains("no command"));
        assert!(check_connection(&conn("", json!({"command": "x", "args": "-y srv"}))).unwrap_err().contains("list"));
        assert!(check_connection(&conn("", json!({"command": "x", "env": {"A": 1}}))).unwrap_err().contains("env"));
        assert!(check_connection(&conn("ftp://x", json!({"transport": "http"}))).is_err());
        assert!(check_connection(&conn("", json!({"transport": "ws"}))).unwrap_err().contains("not stdio or http"));
    }

    #[test]
    fn a_changed_setting_or_secret_changes_the_fingerprint() {
        let a = conn("", json!({"command": "x"}));
        let b = conn("", json!({"command": "y"}));
        assert_eq!(fingerprint(&a, Some("s")), fingerprint(&a, Some("s")));
        assert_ne!(fingerprint(&a, Some("s")), fingerprint(&a, Some("t")));
        assert_ne!(fingerprint(&a, None), fingerprint(&b, None));
    }

    #[test]
    fn tools_take_their_title_from_annotations_too() {
        let t =
            tool_of(&json!({"name": "a", "annotations": {"title": "A"}, "inputSchema": {"type": "object"}})).unwrap();
        assert_eq!((t.title.as_deref(), t.description.as_str(), t.output_schema), (Some("A"), "", None));
        assert!(tool_of(&json!({"description": "no name"})).is_none());
    }
}
