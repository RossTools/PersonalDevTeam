//! What an integration crate (`crates/pdt-int-<id>`, PDT-173) builds on: the [`Integration`] trait and actions, node
//! kinds, the app's state, and the helpers PDT's own integrations use (outbound HTTP, connections and their secrets,
//! CI of attempts, built-in workflows and templates).

pub use crate::error::ApiError;
pub use crate::integrations::action::{Action, ActionCx, AnyAction, ConnectionFace, NodeFace, ToolFace, Via};
pub use crate::integrations::{Integration, Poller, Registry, Tool, tool_info};
pub use crate::mcp::tools::Cx;
pub use crate::notifications::on_event;
pub use crate::overlap::decorate as decorate_ticket;
pub use crate::routes::{AppState, now_ms};
pub use crate::workflow::builtins::Builtin;
pub use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of, spec,
};

/// Workflow templates an integration ships.
pub mod templates {
    pub use crate::workflow::templates::{builtin, doc, params};
}

/// Outbound HTTP: one client, capped bodies, errors without secrets.
pub mod http {
    pub use crate::workflow::nodes::http::{
        BODY_CAP, call_json, call_json_capped, client, read_body, send_error, service,
    };
}

/// Connections, their secrets and their health.
pub mod connections {
    pub use crate::connection_health::{Uses, forget, report};
    pub use crate::workflow::routes::{
        ConnectionInput, ConnectionSecret, connection_of_kind, connection_with_secret, create_connection, put_secret,
    };
}

/// Attempts' pull requests and CI, for a CI integration (which reports its CI as a [`pdt_forge::CiProvider`],
/// [`crate::Integration::ci_provider`]).
pub mod ci {
    pub use crate::github::{attempt_ref, merge_refused, merge_requested, run_refs, watched_pr};
}
