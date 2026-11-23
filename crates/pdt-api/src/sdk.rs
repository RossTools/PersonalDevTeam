//! What an integration crate (`crates/pdt-int-<id>`, PDT-173) builds on: the [`Integration`] trait and actions, node
//! kinds, the app's state, and the helpers PDT's own integrations use (outbound HTTP, connections and their secrets,
//! CI of attempts, built-in workflows and templates).

pub use crate::error::{ApiError, OrNotFound, codes};
pub use crate::extract::{ApiJson, ApiPath, ApiQuery};
pub use crate::integrations::action::{Action, ActionCx, AnyAction, ConnectionFace, NodeFace, ToolFace, Via};
pub use crate::integrations::{
    Imported, Integration, NotificationChannel, Poller, Registry, Service, Tool, WebSources, tool_info,
};
pub use crate::mcp::tools::Cx;
pub use crate::notifications::on_event;
pub use crate::overlap::decorate as decorate_ticket;
pub use crate::routes::{AppState, now_ms};
pub use crate::untrusted::from_ui;
pub use crate::workflow::builtins::Builtin;
pub use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of, spec,
};
pub use crate::workflow::err;

/// Workflow templates an integration ships.
pub mod templates {
    pub use crate::workflow::templates::{builtin, doc, params};
}

/// Outbound HTTP: one client, capped bodies, errors without secrets.
pub mod http {
    pub use crate::workflow::nodes::http::{
        BODY_CAP, CallAuth, CallOpts, call_json, call_json_capped, call_json_with, client, read_body, send_error,
        service,
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

/// Linking tickets to issues already in a tracker (PDT-481, [`crate::Integration::find_issue`]); a forge plugin's
/// issues are found and read through its forge.
pub mod issues {
    pub use crate::issue_links::{FindError, FoundIssue, IssueNow, IssuePage, find_forge_issue, read_forge_link};
    pub use pdt_core::issue_link::IssueState;
    pub use pdt_store::StoredExternalLink;
}

/// Document providers (PDT-314: Google's Docs and Sheets, [`crate::Integration::document_providers`]).
pub mod documents {
    pub use crate::documents::{DocumentProvider, NewFile, ProviderError, ProviderResult};
    pub use crate::integrations::action::{as_agent, by_agent};
    pub use pdt_core::document::*;
}

/// PM conversations, for an integration that holds them elsewhere too (PDT-304: Discord's DMs).
pub mod pm {
    pub use crate::pm::{
        close_conversation, conversations, reset_conversation, send_from_user, set_via, start_conversation,
    };
}

/// Conversations' events and requests, for an integration that follows them (PDT-304).
pub mod conversations {
    pub use crate::conversations::{answer_as_user, is_working, subscribe_all};
}
