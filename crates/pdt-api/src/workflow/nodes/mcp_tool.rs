//! `mcp.tool` (Phase 7e spec §7.2): calls a tool of an MCP server (a connection of kind `mcp`). The catalog also lists
//! each cached tool as its own entry (`mcp:<server>/<tool>`), which makes an `mcp.tool` node preset for that tool.

use crate::mcp_client::{McpTool, McpTools};
use crate::workflow::catalog::{BoxFut, Category, KindPreset, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::mcp::{check_arguments, tool_output};
use pdt_core::workflow::validate::{Issue, Severity};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "mcp.tool";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpToolConfig {
    /// An `mcp` connection (the server).
    #[schemars(extend("x-connection-kind" = "mcp"))]
    pub server: String,
    /// The tool's name on that server.
    pub tool: String,
    /// The tool's arguments, as its input schema says.
    #[serde(default)]
    #[schemars(with = "serde_json::Map<String, Value>")]
    #[ts(type = "Record<string, unknown> | string")]
    pub arguments: Value,
}

fn is_ref(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.contains("{{"))
}

pub struct McpToolKind;

impl NodeKind for McpToolKind {
    fn spec(&self) -> KindSpec {
        spec::<McpToolConfig, Value>(
            KIND,
            VERSION,
            Category::Action,
            "MCP tool",
            "Calls a tool of an MCP server (a connection of kind mcp). The output is the tool's structured content, else \
             its text (parsed when it is JSON); a tool error fails the step.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: McpToolConfig = parse(config)?;
        if c.server.trim().is_empty() {
            return Err("server is empty: name an mcp connection".into());
        }
        if c.tool.trim().is_empty() {
            return Err("tool is empty".into());
        }
        if !(c.arguments.is_null() || c.arguments.is_object() || is_ref(&c.arguments)) {
            return Err("arguments must be an object".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: McpToolConfig = parse(&input)?;
            let arguments = match c.arguments {
                Value::Null => Default::default(),
                Value::Object(m) => m,
                other => return Err(format!("arguments must be an object, not {other}")),
            };
            if cx.dry_run {
                let what = format!("call {} on {} with {}", c.tool, c.server, Value::Object(arguments));
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let result = cx.st.mcp.call(cx.st, &c.server, c.tool.trim(), arguments).await?;
            Ok(Outcome::Next { port: "out".into(), output: tool_output(&result)? })
        })
    }
}

/// The catalog entry for one cached tool: `mcp:<server>/<tool>`, an `mcp.tool` node preset for it, its config schema
/// `mcp.tool`'s with the tool's input schema as `arguments`.
pub fn tool_entry(base: &KindSpec, server: &str, tool: &McpTool) -> KindSpec {
    let mut config_schema = base.config_schema.clone();
    if let Some(props) = config_schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        props.insert("arguments".into(), tool.input_schema.clone());
    }
    let title = tool.title.clone().unwrap_or_else(|| tool.name.clone());
    KindSpec {
        id: format!("mcp:{server}/{}", tool.name),
        version: base.version,
        category: Category::Action,
        title: format!("{title} ({server})"),
        description: if tool.description.is_empty() {
            format!("The {} tool of the MCP server {server}.", tool.name)
        } else {
            tool.description.clone()
        },
        config_schema,
        output_schema: tool.output_schema.clone().unwrap_or_else(|| json!({})),
        ports: base.ports.clone(),
        dynamic_ports: false,
        preset: Some(KindPreset {
            kind: KIND.into(),
            config: json!({"server": server, "tool": tool.name, "arguments": {}}),
        }),
    }
}

/// An entry per cached tool of every server.
pub fn tool_entries(base: &KindSpec, cached: &[McpTools]) -> Vec<KindSpec> {
    cached.iter().flat_map(|s| s.tools.iter().map(|t| tool_entry(base, &s.server, t))).collect()
}

/// Warnings about `mcp.tool` nodes whose server's tools are cached: a tool the server doesn't have, or arguments
/// that don't fit its input schema. Unknown servers and references are left for the run.
pub fn tool_issues(doc: &WorkflowDoc, cached: impl Fn(&str) -> Option<McpTools>) -> Vec<Issue> {
    let mut out = vec![];
    for n in doc.nodes.iter().filter(|n| n.kind == KIND) {
        let Ok(c) = parse::<McpToolConfig>(&n.config) else { continue };
        if is_ref(&json!(c.server)) || is_ref(&json!(c.tool)) {
            continue;
        }
        let Some(tools) = cached(c.server.trim()).filter(|t| t.listed_at.is_some()) else { continue };
        let warn = |message: String| Issue {
            severity: Severity::Warning,
            code: "mcp_tool".into(),
            node: Some(n.id.clone()),
            message,
        };
        match tools.tools.iter().find(|t| t.name == c.tool.trim()) {
            None => out.push(warn(format!("{}: the MCP server {} has no tool {}", n.id, c.server, c.tool))),
            Some(t) => {
                for problem in check_arguments(&t.input_schema, &c.arguments) {
                    out.push(warn(format!("{}: {} arguments: {problem}", n.id, t.name)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached() -> McpTools {
        McpTools {
            server: "files".into(),
            tools: vec![McpTool {
                name: "read".into(),
                title: Some("Read a file".into()),
                description: "Reads one.".into(),
                input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
                output_schema: None,
            }],
            error: None,
            listed_at: Some(1),
            tried_at: 1,
        }
    }

    #[test]
    fn each_tool_is_an_entry_presetting_mcp_tool() {
        let base = McpToolKind.spec();
        let e = &tool_entries(&base, &[cached()])[0];
        assert_eq!(
            (e.id.as_str(), e.title.as_str(), e.category),
            ("mcp:files/read", "Read a file (files)", Category::Action)
        );
        assert_eq!(e.config_schema["properties"]["arguments"]["required"], json!(["path"]));
        assert_eq!(e.config_schema["properties"]["server"]["x-connection-kind"], "mcp");
        let p = e.preset.as_ref().unwrap();
        assert_eq!((p.kind.as_str(), &p.config), (KIND, &json!({"server": "files", "tool": "read", "arguments": {}})));
    }

    #[test]
    fn validation_warns_about_unknown_tools_and_bad_arguments() {
        let doc: WorkflowDoc = serde_json::from_value(json!({"name": "x", "nodes": [
            {"id": "a", "kind": "mcp.tool", "version": 1, "config": {"server": "files", "tool": "read", "arguments": {}}},
            {"id": "b", "kind": "mcp.tool", "version": 1, "config": {"server": "files", "tool": "write"}},
            {"id": "c", "kind": "mcp.tool", "version": 1, "config": {"server": "files", "tool": "read",
                "arguments": {"path": "{{trigger.path}}"}}},
            {"id": "d", "kind": "mcp.tool", "version": 1, "config": {"server": "other", "tool": "anything"}}]}))
        .unwrap();
        let issues = tool_issues(&doc, |s| (s == "files").then(cached));
        let got: Vec<_> = issues.iter().map(|i| (i.node.as_deref().unwrap(), i.message.as_str())).collect();
        assert_eq!(
            got,
            [("a", "a: read arguments: path is required"), ("b", "b: the MCP server files has no tool write")]
        );
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }
}
