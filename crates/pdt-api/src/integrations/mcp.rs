//! MCP servers: the `mcp` connection, and the `mcp.tool` action its tools become.

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::mcp_tool::McpToolKind;
use pdt_core::integration::{ConfigField, ConnectionKind, FieldOption, FieldType, FieldWhen};
use pdt_core::integration::{Icon, PluginCategory};

pub struct Mcp;

impl Integration for Mcp {
    fn id(&self) -> &'static str {
        "mcp"
    }

    fn label(&self) -> &'static str {
        "MCP servers"
    }

    fn description(&self) -> &'static str {
        "Tools from MCP servers, as workflow actions."
    }

    fn category(&self) -> Option<PluginCategory> {
        Some(PluginCategory::General)
    }

    fn icon(&self) -> Icon {
        Icon { letters: "M", color: "#6b7080" }
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        let stdio = || Some(FieldWhen::new("transport", "stdio"));
        vec![ConnectionKind {
            id: "mcp".into(),
            label: "MCP server".into(),
            plural: "MCP servers".into(),
            setup: "A Model Context Protocol server whose tools workflows call (each tool is an action in the \
                    catalog). PDT starts a program’s server when it is first used and stops it after 5 minutes idle."
                .into(),
            description: "An MCP server: its tools become workflow actions.".into(),
            url: false,
            url_placeholder: Some("https://example.com/mcp".into()),
            url_when: Some(FieldWhen::new("transport", "http")),
            secret_label: "Secret".into(),
            secret_hint: "Optional. A program gets it in the environment variable named below; an HTTP server gets \
                          it as a bearer token."
                .into(),
            secret_optional: true,
            fields: vec![
                ConfigField {
                    r#type: Some(FieldType::Choice),
                    options: Some(vec![
                        FieldOption { value: "stdio".into(), label: "A program PDT starts (stdio)".into() },
                        FieldOption { value: "http".into(), label: "An address (HTTP)".into() },
                    ]),
                    ..ConfigField::new("transport", "Runs as")
                },
                ConfigField {
                    placeholder: Some("npx".into()),
                    hint: Some(
                        "Found on PATH, or a full path. On Windows, a .cmd (npx) needs cmd with /c first.".into(),
                    ),
                    when: stdio(),
                    ..ConfigField::new("command", "Program")
                },
                ConfigField {
                    r#type: Some(FieldType::Lines),
                    placeholder: Some("-y\n@modelcontextprotocol/server-filesystem\n/Users/me/notes".into()),
                    hint: Some("One per line.".into()),
                    when: stdio(),
                    optional: true,
                    ..ConfigField::new("args", "Arguments")
                },
                ConfigField {
                    r#type: Some(FieldType::Env),
                    placeholder: Some("LOG_LEVEL=warn".into()),
                    hint: Some("NAME=value, one per line.".into()),
                    when: stdio(),
                    optional: true,
                    ..ConfigField::new("env", "Environment")
                },
                ConfigField {
                    placeholder: Some("API_TOKEN".into()),
                    hint: Some("The environment variable the secret is passed in.".into()),
                    when: stdio(),
                    optional: true,
                    ..ConfigField::new("secretEnv", "Secret’s variable")
                },
            ],
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(McpToolKind)]
    }
}
