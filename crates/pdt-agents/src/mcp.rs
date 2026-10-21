//! How pdt-mcp (Phase 7b spec §2.3) reaches each agent, per launch and never through the user's own config: Claude
//! gets `--mcp-config` (a file in the session's folder) and `--allowedTools` for PDT's tools; Codex a `mcp_servers.pdt`
//! override (the app-server thread's `config`, the TUI's `-c` with the token in an environment variable); Devin's ACP
//! session an `mcpServers` entry.

use pdt_core::mcp::SERVER_NAME;
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
    /// The tools the session may call (so Claude can be told not to ask about them).
    pub tools: Vec<String>,
    /// Claude's `--mcp-config` file (written by [`write_claude_config`]).
    pub claude_config: Option<PathBuf>,
}

impl McpServer {
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

/// Claude's `--mcp-config` document.
pub fn claude_config(m: &McpServer) -> Value {
    json!({"mcpServers": {SERVER_NAME: {"type": "http", "url": m.url, "headers": {"Authorization": m.bearer()}}}})
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
    if !m.tools.is_empty() {
        a.push("--allowedTools".into());
        a.push(m.tools.iter().map(|t| format!("mcp__{SERVER_NAME}__{t}")).collect::<Vec<_>>().join(","));
    }
    a
}

/// A Codex app-server thread's `config` overrides.
pub fn codex_thread_config(m: &McpServer) -> Value {
    json!({format!("mcp_servers.{SERVER_NAME}"): {"url": m.url, "http_headers": {"Authorization": m.bearer()}}})
}

/// A Codex terminal's `-c` overrides; the token goes in [`TOKEN_ENV`] (see [`codex_tui_env`]), not the command line.
pub fn codex_tui_args(m: &McpServer) -> Vec<String> {
    vec![
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.url={}", crate::hooks::toml_string(&m.url)),
        "-c".into(),
        format!("mcp_servers.{SERVER_NAME}.bearer_token_env_var={}", crate::hooks::toml_string(TOKEN_ENV)),
    ]
}

pub fn codex_tui_env(m: &McpServer) -> (String, String) {
    (TOKEN_ENV.to_string(), m.token.clone())
}

/// Devin's ACP `mcpServers` list for `session/new` and `session/load`.
pub fn acp_servers(m: Option<&McpServer>) -> Value {
    match m {
        Some(m) => json!([{
            "type": "http", "name": SERVER_NAME, "url": m.url,
            "headers": [{"name": "Authorization", "value": m.bearer()}],
        }]),
        None => json!([]),
    }
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
        }
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
    }

    #[test]
    fn codex_gets_overrides_and_keeps_the_token_off_the_command_line() {
        let m = server(None);
        let c = codex_thread_config(&m);
        assert_eq!(c["mcp_servers.pdt"]["url"], m.url);
        assert_eq!(c["mcp_servers.pdt"]["http_headers"]["Authorization"], "Bearer s1.ab");
        let a = codex_tui_args(&m);
        assert_eq!(a.len(), 4);
        for kv in [&a[1], &a[3]] {
            let t: toml::Table = toml::from_str(kv).unwrap();
            assert!(t.contains_key("mcp_servers"), "{kv}");
        }
        assert!(!a.iter().any(|x| x.contains("s1.ab")));
        assert_eq!(codex_tui_env(&m), (TOKEN_ENV.to_string(), "s1.ab".to_string()));
    }

    #[test]
    fn devin_gets_an_acp_server_entry() {
        let v = acp_servers(Some(&server(None)));
        assert_eq!(v[0]["type"], "http");
        assert_eq!(v[0]["headers"][0], json!({"name": "Authorization", "value": "Bearer s1.ab"}));
        assert_eq!(acp_servers(None), json!([]));
    }
}
