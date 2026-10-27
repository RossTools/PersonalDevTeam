//! Actions (PDT-172): one capability defined once, as an [`Action`], and offered both as a workflow node kind and as a
//! pdt-mcp tool. An integration lists its actions ([`super::Integration::actions`]); the registry makes each one's
//! node kind ([`ActionNode`]) and tool ([`AnyAction::call`]). What differs between the two is declared here too: the
//! node's id, title and ports beside the tool's name and flags, and how each one shapes the output.

use super::tool_info;
use crate::mcp::tools::{Cx, confirmable};
use crate::routes::AppState;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of};
use pdt_core::integration::ToolDecl;
use pdt_core::mcp::ToolInfo;
use pdt_core::pm::AskTarget;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;

/// An action as a pdt-mcp tool: its name and flags (read-only, asks, open-world), title and description.
pub struct ToolFace {
    pub decl: ToolDecl,
    pub title: &'static str,
    pub description: &'static str,
}

/// An action as a workflow node kind: its id (`scryer.search`), title, description and ports (the first is the one a
/// dry run takes).
pub struct NodeFace {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub ports: &'static [(&'static str, &'static str)],
}

/// The kind of connection an action uses (its input's `connection` names one). A tool call may leave it out to use
/// the only connection of the kind.
pub struct ConnectionFace {
    pub kind: &'static str,
    /// The kind as a tool's schema names it (`Scryer`, `Discord bot`).
    pub label: &'static str,
}

/// Who runs an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// A workflow step.
    Node,
    /// An agent, through pdt-mcp.
    Tool,
}

/// What a running action can see.
pub struct ActionCx<'a> {
    pub st: &'a AppState,
    pub via: Via,
    /// The agent session calling it as a tool (None for a workflow step).
    pub session: Option<&'a pdt_core::session::Session>,
}

/// One capability: a workflow node kind and a pdt-mcp tool from one definition. Its input is the node's config and the
/// tool's arguments (one JSON Schema, from `Input`); its output is what both give back unless [`Action::node_output`]
/// or [`Action::tool_output`] shape it.
pub trait Action: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send + Sync;
    type Output: Serialize + JsonSchema + Send;

    fn tool(&self) -> ToolFace;

    /// Its node kind; None for an action that is a tool only.
    fn node(&self) -> Option<NodeFace> {
        None
    }

    /// The kind of connection it uses, if it uses one.
    fn connection(&self) -> Option<ConnectionFace> {
        None
    }

    /// Whether a node's config (references allowed) fits.
    fn check(&self, _input: &Self::Input) -> Result<(), String> {
        Ok(())
    }

    /// What a call does, in a line (`add the series tvdb 371980 to Scryer (media)`): a dry run's `would`, and the title
    /// of the user's approval.
    fn approval_title(&self, input: &Self::Input) -> String;

    /// What an asking tool's call acts on, from its arguments as the agent gave them: by default the connection they
    /// name (or the only one of its kind), whose settings say whether the PM acts without asking.
    fn ask_target(&self, args: &Value) -> AskTarget {
        match self.connection() {
            Some(c) => AskTarget::Connection {
                kind: c.kind,
                name: args.get("connection").and_then(Value::as_str).map(String::from),
            },
            None => AskTarget::Always,
        }
    }

    fn run<'a>(&'a self, cx: &'a ActionCx<'a>, input: Self::Input) -> BoxFut<'a, Result<Self::Output, String>>;

    /// The port a node leaves by with `out`: its first.
    fn port(&self, _out: &Self::Output) -> &'static str {
        self.node().and_then(|n| n.ports.first().map(|p| p.0)).unwrap_or("out")
    }

    /// A node's output from `out`.
    fn node_output(&self, out: Self::Output) -> Value {
        serde_json::to_value(out).unwrap_or(Value::Null)
    }

    /// The JSON Schema of [`Action::node_output`].
    fn node_output_schema(&self) -> Value {
        schema_of::<Self::Output>()
    }

    /// A tool call's result from `out`.
    fn tool_output(&self, out: Self::Output) -> Result<Value, String> {
        serde_json::to_value(out).map_err(|e| e.to_string())
    }
}

/// An [`Action`] with its types erased, as the registry keeps it.
pub trait AnyAction: Send + Sync {
    fn decl(&self) -> ToolDecl;

    /// The tool as `tools/list` gives it.
    fn info(&self) -> ToolInfo;

    /// Its node kind, if it has one.
    fn node_kind(self: Arc<Self>) -> Option<Box<dyn NodeKind>>;

    /// Runs it as a tool. Err is a message for the agent.
    fn call<'a>(&'a self, cx: &'a Cx<'a>, args: Value) -> BoxFut<'a, Result<Value, String>>;
}

impl<A: Action> AnyAction for A {
    fn decl(&self) -> ToolDecl {
        self.tool().decl
    }

    fn info(&self) -> ToolInfo {
        let t = self.tool();
        tool_info(&t.decl, t.title, t.description, tool_schema::<A::Input>(self.connection().as_ref()))
    }

    fn node_kind(self: Arc<Self>) -> Option<Box<dyn NodeKind>> {
        self.node().is_some().then(|| Box::new(ActionNode(self)) as Box<dyn NodeKind>)
    }

    fn call<'a>(&'a self, cx: &'a Cx<'a>, args: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move {
            let decl = self.tool().decl;
            let mut args = if args.is_null() { json!({}) } else { args };
            if decl.asks {
                confirmable(cx, decl.name, &self.ask_target(&args)).await?;
            }
            if let Some(c) = self.connection()
                && let Some(a) = args.as_object_mut()
            {
                let given = a.get("connection").and_then(Value::as_str).map(String::from);
                let conn = crate::workflow::routes::connection_of_kind(cx.st, c.kind, given.as_deref()).await?;
                a.insert("connection".into(), json!(conn.name));
            }
            let input: A::Input = crate::mcp::tools::args(args)?;
            let out = self.run(&ActionCx { st: cx.st, via: Via::Tool, session: Some(cx.session) }, input).await?;
            self.tool_output(out)
        })
    }
}

/// A tool's input schema: the node's config, with `connection` optional (the only one of its kind) when the action
/// uses one.
pub(crate) fn tool_schema<T: JsonSchema>(connection: Option<&ConnectionFace>) -> Value {
    let mut s = crate::mcp::tools::schema::<T>();
    let Some(conn) = connection else { return s };
    if let Some(r) = s.get_mut("required").and_then(Value::as_array_mut) {
        r.retain(|k| k != "connection");
    }
    if let Some(c) = s.pointer_mut("/properties/connection") {
        c["description"] = json!(format!("A {} connection's name; the only one when left out.", conn.label));
        if let Some(o) = c.as_object_mut() {
            o.remove("x-connection-kind");
        }
    }
    s
}

/// An action's node kind.
pub struct ActionNode<A>(Arc<A>);

impl<A: Action> NodeKind for ActionNode<A> {
    fn spec(&self) -> KindSpec {
        let n = self.0.node().expect("an action node has a node face");
        KindSpec {
            id: n.id.into(),
            version: 1,
            category: Category::Action,
            title: n.title.into(),
            description: n.description.into(),
            config_schema: schema_of::<A::Input>(),
            output_schema: self.0.node_output_schema(),
            ports: ports(n.ports),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        self.0.check(&parse(config)?)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let a = &self.0;
            let c: A::Input = parse(&input)?;
            if cx.dry_run {
                let first = a.node().and_then(|n| n.ports.first().map(|p| p.0)).unwrap_or("out");
                return Ok(Outcome::next(first, json!({"would": a.approval_title(&c)})));
            }
            let out = a.run(&ActionCx { st: cx.st, via: Via::Node, session: None }, c).await?;
            let port = a.port(&out);
            Ok(Outcome::Next { port: port.into(), output: a.node_output(out) })
        })
    }
}
