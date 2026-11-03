//! How pdt-mcp reaches each agent, per launch and never through the user's own config: Claude
//! gets `--mcp-config` (a file in the session's folder) and `--allowedTools` for PDT's tools; Codex a `mcp_servers.pdt`
//! override (the app-server thread's `config`, the TUI's `-c` with the token in an environment variable); Devin's ACP
//! session an `mcpServers` entry.
//!
//! The MCP servers the user configured in PDT (PDT-322, [`ExtraServer`]) go the same way, beside pdt-mcp: with their
//! secrets filled in, and their tools allowed, asked or blocked as far as each agent can.

use pdt_core::mcp::SERVER_NAME;
use pdt_core::mcp_servers::{ToolMode, ToolPlan};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const CLAUDE_CONFIG_FILE: &str = "mcp.json";
/// The environment variable a Codex terminal reads its pdt-mcp token from.
pub const TOKEN_ENV: &str = "PDT_MCP_TOKEN";

/// The pdt-mcp endpoint for one session.
#[derive(Debug, Clone, PartialEq)]
pub struct McpServer {
    /// `http://127.0.0.1:<port>/api/mcp`.
    pub url: String,
    pub token: String,
    /// The tools the session may call without asking (Claude's `--allowedTools`). The session's asking tools
    /// (PDT-81) are left out, so Claude asks before each call.
    pub tools: Vec<String>,
    /// The PM (PDT-100): it researches, so Codex also gets live web search and network access in its sandbox (Claude
    /// and Devin have both already).
    pub pm: bool,
    /// A locked-down session that reads untrusted input (PDT-201): Claude gets only its read tools and no MCP servers
    /// but PDT's, Codex no web search and a read-only sandbox (the session's permission mode is Plan).
    pub locked: bool,
    /// Claude's `--mcp-config` file (written by [`write_claude_config`]).
    pub claude_config: Option<PathBuf>,
    /// The user's MCP servers this session gets (none for a locked-down one).
    pub extra: Vec<ExtraServer>,
    /// Settings → MCP servers → "Use only PDT's MCP servers": the agents' own MCP configs are ignored where the agent
    /// can (Claude `--strict-mcp-config`, Codex each of [`McpServer::codex_user_servers`] switched off).
    pub only_pdt: bool,
    /// The MCP servers in the user's own Codex config (`~/.codex/config.toml`), switched off with `only_pdt`.
    pub codex_user_servers: Vec<String>,
    /// The user's own Codex `shell_environment_policy.exclude`, kept when a terminal adds its secrets' names to it.
    pub codex_shell_exclude: Vec<String>,
}

/// One of the user's MCP servers, resolved for a launch: secrets and sign-ins filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraServer {
    /// The name agents see it under (`pdt_core::mcp_servers::agent_name`).
    pub name: String,
    pub transport: Transport,
    /// Its tools' modes.
    pub plan: ToolPlan,
    /// The header and environment names whose values are secrets: a Codex terminal takes them from its environment,
    /// never its command line.
    pub secret_keys: Vec<String>,
    /// Its tools have been listed (Test, or PDT's own use), so `plan` knows every tool there was then.
    pub listed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    /// A program the agent starts.
    Stdio { command: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String> },
    /// A remote server: streamable HTTP, or the older HTTP+SSE.
    Http { url: String, headers: Vec<(String, String)>, sse: bool },
}

/// MCP servers an ACP agent can take, from its `initialize` answer's `mcpCapabilities` (stdio always).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AcpMcpCaps {
    pub http: bool,
    pub sse: bool,
}

impl AcpMcpCaps {
    pub fn from_init(init: &Value) -> Self {
        let c = init.pointer("/agentCapabilities/mcpCapabilities");
        let on = |k: &str| c.and_then(|c| c.get(k)).and_then(Value::as_bool).unwrap_or(false);
        Self { http: on("http"), sse: on("sse") }
    }
}

impl McpServer {
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

/// Claude's `--mcp-config` document.
pub fn claude_config(m: &McpServer) -> Value {
    let mut servers = serde_json::Map::new();
    for x in m.extra.iter().filter(|x| claude_takes(x)) {
        servers.insert(x.name.clone(), claude_entry(x));
    }
    servers.insert(SERVER_NAME.into(), json!({"type": "http", "url": m.url, "headers": {"Authorization": m.bearer()}}));
    json!({"mcpServers": servers})
}

fn pairs(list: &[(String, String)]) -> Value {
    Value::Object(list.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect())
}

/// One server in Claude's `mcpServers` (Claude has no working folder for a program: it starts in the session's).
fn claude_entry(x: &ExtraServer) -> Value {
    match &x.transport {
        Transport::Stdio { command, args, env, .. } => {
            json!({"type": "stdio", "command": command, "args": args, "env": pairs(env)})
        }
        Transport::Http { url, headers, sse } => {
            json!({"type": if *sse { "sse" } else { "http" }, "url": url, "headers": pairs(headers)})
        }
    }
}

/// Claude blocks a server's tools one by one: one that blocks by default is left out until its tools are listed, or
/// the tools PDT doesn't know of would ask instead (Codex gets `enabled_tools`, which keeps them out).
fn claude_takes(x: &ExtraServer) -> bool {
    x.listed || x.plan.default != ToolMode::Block
}

/// Claude's tool rules for one server: allowed (`--allowedTools`) and blocked (`--disallowedTools`). A server whose
/// tools all share a mode is named whole (`mcp__slack`); otherwise each tool (`mcp__slack__search`). Asking tools
/// are in neither, so Claude asks.
fn claude_rules(x: &ExtraServer) -> (Vec<String>, Vec<String>) {
    let whole = format!("mcp__{}", x.name);
    let each = |mode| x.plan.with(mode).into_iter().map(|t| format!("{whole}__{t}")).collect::<Vec<_>>();
    let uniform = x.plan.uniform();
    let allowed =
        if uniform && x.plan.default == ToolMode::Allow { vec![whole.clone()] } else { each(ToolMode::Allow) };
    let blocked = if uniform && x.plan.default == ToolMode::Block { vec![whole] } else { each(ToolMode::Block) };
    (allowed, blocked)
}

/// Writes `<dir>/mcp.json` (owner-only: it holds the token) and returns its path.
pub fn write_claude_config(dir: &Path, m: &McpServer) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(CLAUDE_CONFIG_FILE);
    let text = serde_json::to_string_pretty(&claude_config(m)).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        f.write_all(text.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Claude (terminal or `-p`): `--mcp-config <file> --allowedTools mcp__pdt__…`. Nothing without the file.
pub fn claude_args(m: &McpServer) -> Vec<String> {
    let Some(file) = &m.claude_config else { return vec![] };
    let mut a = vec!["--mcp-config".to_string(), file.to_string_lossy().into_owned()];
    let mut allowed: Vec<String> = m.tools.iter().map(|t| format!("mcp__{SERVER_NAME}__{t}")).collect();
    let mut blocked: Vec<String> = vec![];
    for x in m.extra.iter().filter(|x| claude_takes(x)) {
        let (al, bl) = claude_rules(x);
        allowed.extend(al);
        blocked.extend(bl);
    }
    if !allowed.is_empty() {
        a.push("--allowedTools".into());
        a.push(allowed.join(","));
    }
    if !blocked.is_empty() {
        a.push("--disallowedTools".into());
        a.push(blocked.join(","));
    }
    if m.locked {
        a.extend(["--tools".into(), CLAUDE_READ_TOOLS.into()]);
    }
    if m.locked || m.only_pdt {
        a.push("--strict-mcp-config".into());
    }
    a
}

/// The built-in tools a locked-down Claude session keeps (`--tools`): reading files, no shell, web or edits.
pub const CLAUDE_READ_TOOLS: &str = "Read,Grep,Glob";

/// A locked-down session's Codex settings: no web search, and a read-only sandbox.
pub const CODEX_LOCKED_CONFIG: [(&str, &str); 2] = [("web_search", "\"disabled\""), ("sandbox_mode", "\"read-only\"")];

/// How long Codex waits for one of PDT's tools (`tool_timeout_sec`; Codex's own is a minute): an asking tool's call
/// waits for the user's answer, in a conversation (PDT-490) or a terminal's Inbox item (PDT-492).
pub const CODEX_PDT_TOOL_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// A Codex app-server thread's `config` overrides. PDT's tools run without asking (as Claude's `--allowedTools`):
/// a session only gets the tools its persona allows. The PM also gets [`CODEX_PM_CONFIG`].
pub fn codex_thread_config(m: &McpServer) -> Value {
    let server = json!({"url": m.url, "http_headers": {"Authorization": m.bearer()},
                        "default_tools_approval_mode": "approve", "tool_timeout_sec": CODEX_PDT_TOOL_TIMEOUT_SECS});
    let mut c = json!({format!("mcp_servers.{SERVER_NAME}"): server});
    for x in m.extra.iter().filter(|x| codex_takes(x)) {
        c[format!("mcp_servers.{}", x.name)] = codex_server(x, false);
    }
    for n in codex_switched_off(m) {
        c[format!("mcp_servers.{n}.enabled")] = json!(false);
    }
    for (k, v) in codex_extra(m) {
        c[k] = serde_json::from_str(v).unwrap_or(Value::Null);
    }
    c
}

/// Codex speaks stdio and streamable HTTP, not the older HTTP+SSE.
fn codex_takes(x: &ExtraServer) -> bool {
    !matches!(x.transport, Transport::Http { sse: true, .. })
}

/// The user's own Codex servers a session switches off ("Use only PDT's MCP servers"): those PDT doesn't give it.
fn codex_switched_off(m: &McpServer) -> Vec<&str> {
    if !m.only_pdt {
        return vec![];
    }
    m.codex_user_servers
        .iter()
        .map(String::as_str)
        .filter(|n| *n != SERVER_NAME && !m.extra.iter().any(|x| x.name == *n))
        .collect()
}

fn codex_approval(mode: ToolMode) -> &'static str {
    match mode {
        ToolMode::Allow => "approve",
        _ => "prompt",
    }
}

/// The environment variable a Codex terminal passes one secret header in. Like every name a terminal passes a secret
/// in, it is kept from the agent's shell commands ([`codex_shell_exclude`]).
pub fn codex_header_env(server: &str, header: &str) -> String {
    let clean = |s: &str| {
        s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect::<String>()
    };
    format!("PDT_MCP_SECRET_{}_{}", clean(server), clean(header))
}

/// A Codex terminal's `shell_environment_policy.exclude`: the user's own, and the names it passes secrets in
/// ([`codex_tui_env`]), so the commands the agent runs never have them in their environment (Codex itself and the MCP
/// servers it starts still do).
fn codex_shell_exclude(m: &McpServer) -> Vec<String> {
    let mut out = m.codex_shell_exclude.clone();
    out.push("PDT_MCP_*".into());
    for (k, _) in codex_tui_env(m) {
        if !k.starts_with("PDT_MCP_") && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// A server's Codex settings (`mcp_servers.<name>`). `tui`: a terminal's, whose secrets come from its environment
/// ([`codex_tui_env`]) rather than the value itself: `env_vars` for a program's, `env_http_headers` for headers.
fn codex_server(x: &ExtraServer, tui: bool) -> Value {
    let secret = |k: &str| tui && x.secret_keys.iter().any(|s| s == k);
    let mut v = match &x.transport {
        Transport::Stdio { command, args, env, cwd } => {
            let mut v = json!({"command": command, "args": args});
            let plain: Vec<(String, String)> = env.iter().filter(|(k, _)| !secret(k)).cloned().collect();
            if !plain.is_empty() {
                v["env"] = pairs(&plain);
            }
            let passed: Vec<&String> = env.iter().filter(|(k, _)| secret(k)).map(|(k, _)| k).collect();
            if !passed.is_empty() {
                v["env_vars"] = json!(passed);
            }
            if let Some(d) = cwd {
                v["cwd"] = json!(d);
            }
            v
        }
        Transport::Http { url, headers, .. } => {
            let mut v = json!({"url": url});
            let plain: Vec<(String, String)> = headers.iter().filter(|(k, _)| !secret(k)).cloned().collect();
            if !plain.is_empty() {
                v["http_headers"] = pairs(&plain);
            }
            let from_env: serde_json::Map<String, Value> = headers
                .iter()
                .filter(|(k, _)| secret(k))
                .map(|(k, _)| (k.clone(), Value::String(codex_header_env(&x.name, k))))
                .collect();
            if !from_env.is_empty() {
                v["env_http_headers"] = Value::Object(from_env);
            }
            v
        }
    };
    let plan = &x.plan;
    let default = if plan.default == ToolMode::Block { ToolMode::Ask } else { plan.default };
    v["default_tools_approval_mode"] = json!(codex_approval(default));
    if plan.default == ToolMode::Block {
        // Only the tools that may run.
        let open: Vec<&str> =
            plan.tools.iter().filter(|(_, m)| *m != ToolMode::Block).map(|(t, _)| t.as_str()).collect();
        v["enabled_tools"] = json!(open);
    }
    let blocked = plan.with(ToolMode::Block);
    if !blocked.is_empty() && plan.default != ToolMode::Block {
        v["disabled_tools"] = json!(blocked);
    }
    let differ: serde_json::Map<String, Value> = plan
        .tools
        .iter()
        .filter(|(_, m)| *m != ToolMode::Block && *m != default)
        .map(|(t, m)| (t.clone(), json!({"approval_mode": codex_approval(*m)})))
        .collect();
    if !differ.is_empty() {
        v["tools"] = Value::Object(differ);
    }
    v
}

/// JSON as an inline TOML value, for `-c key=<value>`.
fn toml_inline(v: &Value) -> String {
    match v {
        Value::String(s) => crate::hooks::toml_string(s),
        Value::Array(a) => format!("[{}]", a.iter().map(toml_inline).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => format!(
            "{{{}}}",
            m.iter().map(|(k, v)| format!("{} = {}", toml_key(k), toml_inline(v))).collect::<Vec<_>>().join(", ")
        ),
        Value::Null => "\"\"".into(),
        other => other.to_string(),
    }
}

fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        crate::hooks::toml_string(k)
    }
}

/// The Codex settings beyond pdt-mcp: the PM's, or a locked-down session's (which wins).
fn codex_extra(m: &McpServer) -> &'static [(&'static str, &'static str)] {
    match (m.locked, m.pm) {
        (true, _) => &CODEX_LOCKED_CONFIG,
        (false, true) => &CODEX_PM_CONFIG,
        (false, false) => &[],
    }
}

/// The PM's Codex settings (key, TOML/JSON value): live web search, and the network inside the workspace-write sandbox
/// (installs, fetching docs). Each turn's `sandboxPolicy` must say `networkAccess` too.
pub const CODEX_PM_CONFIG: [(&str, &str); 2] =
    [("web_search", "\"live\""), ("sandbox_workspace_write.network_access", "true")];

/// A Codex terminal's `-c` overrides; the token goes in [`TOKEN_ENV`] (see [`codex_tui_env`]), not the command line.
pub fn codex_tui_args(m: &McpServer) -> Vec<String> {
    [
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.url={}", crate::hooks::toml_string(&m.url)),
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.bearer_token_env_var={}", crate::hooks::toml_string(TOKEN_ENV)),
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.default_tools_approval_mode=\"approve\""),
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.tool_timeout_sec={CODEX_PDT_TOOL_TIMEOUT_SECS}"),
    ]
    .into_iter()
    .chain(
        m.extra
            .iter()
            .filter(|x| codex_takes(x))
            .flat_map(|x| ["-c".into(), format!("mcp_servers.{}={}", x.name, toml_inline(&codex_server(x, true)))]),
    )
    .chain(codex_switched_off(m).into_iter().flat_map(|n| ["-c".into(), format!("mcp_servers.{n}.enabled=false")]))
    .chain(codex_extra(m).iter().flat_map(|(k, v)| ["-c".into(), format!("{k}={v}")]))
    .chain(["-c".into(), format!("shell_environment_policy.exclude={}", toml_inline(&json!(codex_shell_exclude(m))))])
    .collect()
}

/// A Codex terminal's environment for MCP: pdt-mcp's token, and the secrets of the user's servers.
pub fn codex_tui_env(m: &McpServer) -> Vec<(String, String)> {
    let mut env = vec![(TOKEN_ENV.to_string(), m.token.clone())];
    for x in m.extra.iter().filter(|x| codex_takes(x)) {
        match &x.transport {
            Transport::Stdio { env: e, .. } => {
                env.extend(e.iter().filter(|(k, _)| x.secret_keys.contains(k)).cloned());
            }
            Transport::Http { headers, .. } => env.extend(
                headers
                    .iter()
                    .filter(|(k, _)| x.secret_keys.contains(k))
                    .map(|(k, v)| (codex_header_env(&x.name, k), v.clone())),
            ),
        }
    }
    env
}

fn codex_config(codex_home: &Path) -> Option<toml::Table> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    toml::from_str::<toml::Table>(&text).ok()
}

/// The MCP server names in a Codex config file (`[mcp_servers.<name>]`).
pub fn codex_config_servers(codex_home: &Path) -> Vec<String> {
    let Some(t) = codex_config(codex_home) else { return vec![] };
    t.get("mcp_servers").and_then(|v| v.as_table()).map(|m| m.keys().cloned().collect()).unwrap_or_default()
}

/// The `shell_environment_policy.exclude` patterns in a Codex config file.
pub fn codex_config_shell_exclude(codex_home: &Path) -> Vec<String> {
    let Some(t) = codex_config(codex_home) else { return vec![] };
    let list = t.get("shell_environment_policy").and_then(|p| p.get("exclude")).and_then(|v| v.as_array());
    list.map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn acp_pairs(list: &[(String, String)]) -> Value {
    Value::Array(list.iter().map(|(k, v)| json!({"name": k, "value": v})).collect())
}

/// An ACP agent's `mcpServers` list for `session/new` and `session/load`: pdt-mcp, and the user's servers the agent
/// can take (`caps`); the names of those it can't come second. ACP has no way to block a tool: that is only noted.
pub fn acp_servers(m: Option<&McpServer>, caps: AcpMcpCaps) -> (Value, Vec<String>) {
    let Some(m) = m else { return (json!([]), vec![]) };
    let mut list = vec![json!({
        "type": "http", "name": SERVER_NAME, "url": m.url,
        "headers": [{"name": "Authorization", "value": m.bearer()}],
    })];
    let mut skipped = vec![];
    for x in &m.extra {
        match &x.transport {
            Transport::Stdio { command, args, env, .. } => list.push(json!({
                "name": x.name, "command": command, "args": args, "env": acp_pairs(env),
            })),
            Transport::Http { url, headers, sse } if (*sse && caps.sse) || (!*sse && caps.http) => list.push(json!({
                "type": if *sse { "sse" } else { "http" }, "name": x.name, "url": url, "headers": acp_pairs(headers),
            })),
            Transport::Http { .. } => skipped.push(x.name.clone()),
        }
    }
    (Value::Array(list), skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(dir: Option<&Path>) -> McpServer {
        McpServer {
            url: "http://127.0.0.1:7421/api/mcp".into(),
            token: "s1.ab".into(),
            tools: vec!["get_ticket".into(), "submit_outcome".into()],
            claude_config: dir.map(|d| d.join(CLAUDE_CONFIG_FILE)),
            pm: false,
            locked: false,
            extra: vec![],
            only_pdt: false,
            codex_user_servers: vec![],
            codex_shell_exclude: vec![],
        }
    }

    /// Slack (remote, signed in: search allowed, posting blocked, the rest asks) and a local notes server.
    fn extras() -> Vec<ExtraServer> {
        use pdt_core::mcp_servers::ToolPolicy;
        let slack = ToolPolicy {
            default: ToolMode::Ask,
            overrides: [("search".to_string(), ToolMode::Allow), ("post_message".to_string(), ToolMode::Block)].into(),
        };
        vec![
            ExtraServer {
                name: "slack".into(),
                transport: Transport::Http {
                    url: "https://mcp.slack.com/mcp".into(),
                    headers: vec![("Authorization".into(), "Bearer xoxp-1".into()), ("X-Team".into(), "T1".into())],
                    sse: false,
                },
                plan: slack.plan(&["search".into(), "post_message".into(), "read".into()]),
                secret_keys: vec!["Authorization".into()],
                listed: true,
            },
            ExtraServer {
                name: "notes".into(),
                transport: Transport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "notes-mcp".into()],
                    env: vec![("NOTES_KEY".into(), "k-123456".into()), ("LOG".into(), "warn".into())],
                    cwd: Some("/tmp/notes".into()),
                },
                plan: ToolPolicy { default: ToolMode::Allow, overrides: Default::default() }.plan(&["find".into()]),
                secret_keys: vec!["NOTES_KEY".into()],
                listed: false,
            },
        ]
    }

    #[test]
    fn claude_gets_the_users_servers_with_their_tools_allowed_and_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let m = McpServer { extra: extras(), ..server(Some(dir.path())) };
        let path = write_claude_config(dir.path(), &m).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let slack = &v["mcpServers"]["slack"];
        assert_eq!(
            (slack["type"].as_str(), slack["headers"]["Authorization"].as_str()),
            (Some("http"), Some("Bearer xoxp-1"))
        );
        let notes = &v["mcpServers"]["notes"];
        assert_eq!(notes["type"], "stdio");
        assert_eq!(notes["env"]["NOTES_KEY"], "k-123456", "secrets filled in, in the owner-only file");
        assert_eq!(v["mcpServers"]["pdt"]["type"], "http");
        let a = claude_args(&m);
        let allowed = &a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1];
        assert_eq!(allowed, "mcp__pdt__get_ticket,mcp__pdt__submit_outcome,mcp__slack__search,mcp__notes");
        let blocked = &a[a.iter().position(|x| x == "--disallowedTools").unwrap() + 1];
        assert_eq!(blocked, "mcp__slack__post_message");
        assert!(!a.contains(&"--strict-mcp-config".to_string()), "the user's own servers load too");
        let only = McpServer { only_pdt: true, ..m };
        assert_eq!(claude_args(&only).last().unwrap(), "--strict-mcp-config");
    }

    #[test]
    fn codex_gets_the_users_servers_and_a_terminal_keeps_their_secrets_in_its_environment() {
        let m = McpServer { extra: extras(), ..server(None) };
        let c = codex_thread_config(&m);
        let slack = &c["mcp_servers.slack"];
        assert_eq!(slack["http_headers"]["Authorization"], "Bearer xoxp-1");
        assert_eq!(slack["disabled_tools"], json!(["post_message"]));
        assert_eq!(slack["default_tools_approval_mode"], "prompt");
        assert_eq!(slack["tools"]["search"]["approval_mode"], "approve");
        let notes = &c["mcp_servers.notes"];
        assert_eq!((notes["command"].as_str(), notes["cwd"].as_str()), (Some("npx"), Some("/tmp/notes")));
        assert_eq!(notes["default_tools_approval_mode"], "approve");

        let a = codex_tui_args(&m);
        assert!(!a.iter().any(|x| x.contains("xoxp-1") || x.contains("k-123456")), "no secret on the command line");
        let slack_arg = a.iter().find(|x| x.starts_with("mcp_servers.slack=")).unwrap();
        let t: toml::Table = toml::from_str(slack_arg).unwrap();
        let s = &t["mcp_servers"]["slack"];
        assert_eq!(s["env_http_headers"]["Authorization"].as_str(), Some("PDT_MCP_SECRET_SLACK_AUTHORIZATION"));
        assert_eq!(s["http_headers"]["X-Team"].as_str(), Some("T1"));
        let notes_arg = a.iter().find(|x| x.starts_with("mcp_servers.notes=")).unwrap();
        let t: toml::Table = toml::from_str(notes_arg).unwrap();
        assert_eq!(t["mcp_servers"]["notes"]["env_vars"].as_array().unwrap()[0].as_str(), Some("NOTES_KEY"));
        assert_eq!(t["mcp_servers"]["notes"]["env"]["LOG"].as_str(), Some("warn"));
        let env = codex_tui_env(&m);
        assert!(env.contains(&("PDT_MCP_SECRET_SLACK_AUTHORIZATION".into(), "Bearer xoxp-1".into())));
        assert!(env.contains(&("NOTES_KEY".into(), "k-123456".into())));
    }

    #[test]
    fn a_codex_terminals_shell_commands_never_get_its_secrets() {
        let m = McpServer { extra: extras(), codex_shell_exclude: vec!["AWS_*".into()], ..server(None) };
        let a = codex_tui_args(&m);
        let arg = a.iter().find(|x| x.starts_with("shell_environment_policy.exclude=")).unwrap();
        let t: toml::Table = toml::from_str(arg).unwrap();
        let list = t["shell_environment_policy"]["exclude"].as_array().unwrap();
        let list: Vec<&str> = list.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(list, ["AWS_*", "PDT_MCP_*", "NOTES_KEY"], "the user's own, then every name holding a secret");
        for (k, _) in codex_tui_env(&m) {
            assert!(k.starts_with("PDT_MCP_") || list.contains(&k.as_str()), "{k}");
        }
    }

    #[test]
    fn reads_the_users_codex_shell_excludes() {
        let home = std::env::temp_dir().join(format!("pdt-codex-excl-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "[shell_environment_policy]\nexclude = [\"AWS_*\", \"X\"]\n").unwrap();
        assert_eq!(codex_config_shell_exclude(&home), ["AWS_*", "X"]);
        std::fs::remove_dir_all(&home).unwrap();
        assert!(codex_config_shell_exclude(&home).is_empty());
    }

    #[test]
    fn a_codex_server_blocked_by_default_enables_only_its_open_tools() {
        use pdt_core::mcp_servers::ToolPolicy;
        let p = ToolPolicy { default: ToolMode::Block, overrides: [("search".to_string(), ToolMode::Allow)].into() };
        let x = ExtraServer {
            name: "s".into(),
            transport: Transport::Http { url: "https://s/mcp".into(), headers: vec![], sse: false },
            plan: p.plan(&["search".into(), "post".into()]),
            secret_keys: vec![],
            listed: true,
        };
        let v = codex_server(&x, false);
        assert_eq!(v["enabled_tools"], json!(["search"]));
        assert!(v.get("disabled_tools").is_none());
        let (_, blocked) = claude_rules(&x);
        assert_eq!(blocked, ["mcp__s__post"]);
        assert!(claude_takes(&x));
        // Before its tools are listed, Claude couldn't block the ones PDT doesn't know of: it goes without the server.
        let unlisted = ExtraServer { listed: false, ..x };
        assert!(!claude_takes(&unlisted));
        let dir = tempfile::tempdir().unwrap();
        let m = McpServer { extra: vec![unlisted.clone()], ..server(Some(dir.path())) };
        assert!(claude_config(&m)["mcpServers"].get("s").is_none());
        assert!(!claude_args(&m).iter().any(|a| a.contains("mcp__s")));
        assert!(codex_thread_config(&m).get("mcp_servers.s").is_some(), "Codex enables only the open tools");
    }

    #[test]
    fn only_pdt_switches_off_the_users_own_codex_servers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[mcp_servers.github]\ncommand = \"gh-mcp\"\n[mcp_servers.slack]\nurl = \"x\"\n",
        )
        .unwrap();
        let mine = codex_config_servers(dir.path());
        assert_eq!(mine, ["github", "slack"]);
        let m = McpServer { extra: extras(), only_pdt: true, codex_user_servers: mine, ..server(None) };
        let c = codex_thread_config(&m);
        assert_eq!(c["mcp_servers.github.enabled"], false);
        assert!(c.get("mcp_servers.slack.enabled").is_none(), "PDT's own slack replaces it");
        assert!(codex_tui_args(&m).contains(&"mcp_servers.github.enabled=false".to_string()));
        assert!(codex_config_servers(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn acp_agents_get_the_servers_they_can_take() {
        let m = McpServer { extra: extras(), ..server(None) };
        let (v, skipped) = acp_servers(Some(&m), AcpMcpCaps { http: true, sse: false });
        assert_eq!(v.as_array().unwrap().len(), 3);
        assert_eq!(v[1]["headers"][0], json!({"name": "Authorization", "value": "Bearer xoxp-1"}));
        assert_eq!(v[2]["env"][0], json!({"name": "NOTES_KEY", "value": "k-123456"}));
        assert!(v[2].get("type").is_none(), "ACP's stdio entries are untyped");
        assert!(skipped.is_empty());
        let (v, skipped) = acp_servers(Some(&m), AcpMcpCaps::default());
        assert_eq!((v.as_array().unwrap().len(), skipped), (2, vec!["slack".to_string()]));
        let caps = AcpMcpCaps::from_init(&json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}));
        assert_eq!(caps, AcpMcpCaps { http: true, sse: false });
    }

    #[test]
    fn claude_gets_a_private_config_file_and_its_tools_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_claude_config(dir.path(), &server(None)).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["pdt"]["type"], "http");
        assert_eq!(v["mcpServers"]["pdt"]["headers"]["Authorization"], "Bearer s1.ab");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        write_claude_config(dir.path(), &server(None)).unwrap();
        let a = claude_args(&server(Some(dir.path())));
        assert_eq!(a[0], "--mcp-config");
        assert!(a[1].ends_with("mcp.json"));
        assert_eq!(a[2..], ["--allowedTools", "mcp__pdt__get_ticket,mcp__pdt__submit_outcome"]);
        assert!(claude_args(&server(None)).is_empty());
        let pm = McpServer { pm: true, ..server(Some(dir.path())) };
        assert_eq!(claude_args(&pm), a, "the PM keeps Claude's own tools");
        // Triage (PDT-201): Claude's read tools only, and no MCP servers but PDT's.
        let locked = McpServer { locked: true, ..server(Some(dir.path())) };
        assert_eq!(claude_args(&locked)[4..], ["--tools", "Read,Grep,Glob", "--strict-mcp-config"]);
    }

    #[test]
    fn a_locked_down_codex_session_gets_no_web_and_a_read_only_sandbox() {
        let m = McpServer { locked: true, pm: true, ..server(None) };
        let c = codex_thread_config(&m);
        assert_eq!((c["web_search"].as_str(), c["sandbox_mode"].as_str()), (Some("disabled"), Some("read-only")));
        assert!(c.get("sandbox_workspace_write.network_access").is_none(), "not the PM's network");
        let a = codex_tui_args(&m);
        assert_eq!(a[8..12], ["-c", "web_search=\"disabled\"", "-c", "sandbox_mode=\"read-only\""]);
    }

    #[test]
    fn codex_gets_overrides_and_keeps_the_token_off_the_command_line() {
        let m = server(None);
        let c = codex_thread_config(&m);
        assert_eq!(c["mcp_servers.pdt"]["url"], m.url);
        assert_eq!(c["mcp_servers.pdt"]["http_headers"]["Authorization"], "Bearer s1.ab");
        let a = codex_tui_args(&m);
        assert_eq!(a.len(), 10);
        assert_eq!(a[7], format!("mcp_servers.pdt.tool_timeout_sec={CODEX_PDT_TOOL_TIMEOUT_SECS}"));
        assert_eq!(a[9], "shell_environment_policy.exclude=[\"PDT_MCP_*\"]", "its token stays out of shell commands");
        assert_eq!(c["mcp_servers.pdt"]["default_tools_approval_mode"], "approve");
        assert_eq!(c["mcp_servers.pdt"]["tool_timeout_sec"], CODEX_PDT_TOOL_TIMEOUT_SECS);
        for kv in [&a[1], &a[3], &a[5], &a[7]] {
            let t: toml::Table = toml::from_str(kv).unwrap();
            assert!(t.contains_key("mcp_servers"), "{kv}");
        }
        assert!(!a.iter().any(|x| x.contains("s1.ab")));
        assert_eq!(codex_tui_env(&m), [(TOKEN_ENV.to_string(), "s1.ab".to_string())]);
        assert!(c.get("web_search").is_none());

        // The PM researches: live web search and the network in its sandbox.
        let pm = McpServer { pm: true, ..m };
        let c = codex_thread_config(&pm);
        assert_eq!(c["web_search"], "live");
        assert_eq!(c["sandbox_workspace_write.network_access"], true);
        let a = codex_tui_args(&pm);
        assert_eq!(a[8..12], ["-c", "web_search=\"live\"", "-c", "sandbox_workspace_write.network_access=true"]);
        for kv in [&a[9], &a[11]] {
            toml::from_str::<toml::Table>(kv).unwrap();
        }
    }

    #[test]
    fn devin_gets_an_acp_server_entry() {
        let (v, _) = acp_servers(Some(&server(None)), AcpMcpCaps::default());
        assert_eq!(v[0]["type"], "http");
        assert_eq!(v[0]["headers"][0], json!({"name": "Authorization", "value": "Bearer s1.ab"}));
        assert_eq!(acp_servers(None, AcpMcpCaps::default()).0, json!([]));
    }
}
