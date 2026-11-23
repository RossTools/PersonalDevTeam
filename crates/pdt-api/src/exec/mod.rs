//! The executor (PDT-632): where an attempt's work runs. Local is everything else in pdt-api; remote is this module,
//! the commands an owner sends one of its remote instances over the project link and the reports that come back.
//!
//! - **Conversations.** The owner keeps every conversation's logic (its events in the store, activity, cards,
//!   approvals, workflows): a remote session's live process is a [`owner::RemoteLive`] in place of an adapter's, and
//!   the remote instance runs only the agent's adapter, numbering its outputs in a buffer the owner reads
//!   (`/link/exec/events`, a long poll), so a dropped link or a restarted owner loses nothing.
//! - **Agents' PDT tools.** The remote instance serves `/api/mcp` to its agents and passes each call, with the session's
//!   token, to the owner (`/link/exec/mcp`), retrying while the link is down: the owner answers it as its own.
//! - **Terminals** (PDT-633). A remote terminal session (an agent's TUI or a shell) is a PTY on the remote instance,
//!   under the owner's session id, in the remote project's checkout or one of its worktrees and nowhere else
//!   ([`terminals`] on the owner, `remote_pty` there). The remote keeps its scrollback while nobody watches; a browser's
//!   terminal on the owner is bridged to it (`/link/exec/pty/ws/{ticket}`, a one-time ticket the owner gets with a
//!   signed call), and its exit comes back with the events. Its agent's hooks are passed on to the owner
//!   (`/link/exec/activity`).
//! - **Git.** The remote instance keeps the project's checkout (cloned on first use) and its worktrees; the owner asks
//!   it to make and remove worktrees and to look at or update a branch.
//!
//! Owner → remote calls are signed with the owner's key for the link and checked by `link::from_owner`; the remote's
//! with its key, checked by `link::from_remote`.

mod owner;
mod remote;
mod remote_pty;
mod terminals;

pub use owner::resume_pollers;
pub(crate) use owner::{
    branch_head, has_changes, is_routed, joined_instance, make_worktree, open as open_remote, remove_worktree,
    update_base, worktree_instance,
};
pub use remote::RunningView;
pub(crate) use remote::{checkout_dir, close_link, is_relayed, relay_mcp, running_view};
pub(crate) use remote_pty::relay_activity;
pub(crate) use terminals::{
    bridge as bridge_pty, close as close_terminal, is_remote_terminal, open as open_terminal, write as write_terminal,
};

use pdt_agents::adapter::{AdapterOutput, AgentInfo};
use pdt_core::conversation::{
    AgentCapabilities, AgentEvent, Answer, BackgroundTask, Delivery, DeltaField, PermissionMode,
};
use pdt_core::host::CredentialPolicy;
use pdt_core::session::AgentKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The executor's paths, under `/api`.
pub(crate) const OPEN_PATH: &str = "/api/link/exec/open";
pub(crate) const SEND_PATH: &str = "/api/link/exec/send";
pub(crate) const ANSWER_PATH: &str = "/api/link/exec/answer";
pub(crate) const INTERRUPT_PATH: &str = "/api/link/exec/interrupt";
pub(crate) const SET_PATH: &str = "/api/link/exec/set";
pub(crate) const CLOSE_PATH: &str = "/api/link/exec/close";
pub(crate) const EVENTS_PATH: &str = "/api/link/exec/events";
pub(crate) const WORKTREE_PATH: &str = "/api/link/exec/worktree";
pub(crate) const REMOVE_WORKTREE_PATH: &str = "/api/link/exec/remove-worktree";
pub(crate) const GIT_PATH: &str = "/api/link/exec/git";
pub(crate) const MCP_PATH: &str = "/api/link/exec/mcp";
pub(crate) const PTY_OPEN_PATH: &str = "/api/link/exec/pty/open";
pub(crate) const PTY_ATTACH_PATH: &str = "/api/link/exec/pty/attach";
pub(crate) const PTY_WRITE_PATH: &str = "/api/link/exec/pty/write";
/// The WebSocket a ticket from [`PTY_ATTACH_PATH`] opens: `<this>/<ticket>`.
pub(crate) const PTY_WS_PATH: &str = "/api/link/exec/pty/ws";
pub(crate) const ACTIVITY_PATH: &str = "/api/link/exec/activity";

/// What the executor keeps in memory, on both ends.
#[derive(Default)]
pub struct ExecRuntime {
    owner: owner::OwnerSide,
    remote: remote::RemoteSide,
}

impl ExecRuntime {
    /// How long the owner's long poll waits for events, and how long it waits after a failed one (tests shorten
    /// them).
    pub fn set_poll_timing(&self, wait: std::time::Duration, retry: std::time::Duration) {
        self.owner.set_timing(wait, retry);
    }
}

/// `/link/exec/open`: start (or attach to) a session's agent on the remote instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenReq {
    pub session_id: String,
    pub agent: AgentKind,
    pub title: String,
    pub ticket_key: Option<String>,
    /// The worktree's branch; None: the project's checkout.
    pub branch: Option<String>,
    /// The project's base branch, fetched when the checkout is used (cloned first when it isn't there).
    #[serde(default)]
    pub base: Option<String>,
    pub external_id: Option<String>,
    /// The conversation has events already (it resumes when the agent saved it).
    pub has_history: bool,
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub credentials: CredentialPolicy,
    pub instructions: Option<WireInstructions>,
    pub mcp: Option<WireMcp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireInstructions {
    pub text: String,
    pub file_name: String,
}

/// pdt-mcp for a remote session: the owner's token and the tools it gets; the remote instance serves the address.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireMcp {
    pub token: String,
    pub tools: Vec<String>,
    pub asks: Vec<String>,
    pub pm: bool,
    pub locked: bool,
    pub only_pdt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Opened {
    /// The process's id there: its events carry it.
    pub proc: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendReq {
    pub session_id: String,
    pub text: String,
    pub images: Vec<WireImage>,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireImage {
    pub name: String,
    pub media_type: String,
    /// Base64.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerReq {
    pub session_id: String,
    pub request_id: String,
    pub answer: Answer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionReq {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetReq {
    pub session_id: String,
    /// The model changes (to `model`; None: the agent's default).
    pub set_model: bool,
    pub model: Option<String>,
    pub permission_mode: Option<PermissionMode>,
}

/// `/link/exec/events`: the events after `after` of the remote instance's boot `boot`, waiting up to `wait_ms` for
/// some. Those up to `after` are taken: the remote instance forgets them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PollReq {
    pub boot: Option<String>,
    pub after: i64,
    pub wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Batch {
    /// The remote instance's process: a new one numbers its events afresh (and runs none of the old processes).
    pub boot: String,
    pub events: Vec<WireEvent>,
    /// The sessions whose process runs there now.
    pub alive: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireEvent {
    pub seq: i64,
    pub session_id: String,
    pub proc: String,
    pub out: WireOut,
}

/// An adapter's output on the wire ([`AdapterOutput`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum WireOut {
    Event {
        item_id: Option<String>,
        turn_id: Option<String>,
        event: AgentEvent,
        raw: Option<String>,
    },
    Delta {
        item_id: String,
        field: DeltaField,
        text: String,
    },
    Info {
        model: Option<String>,
        permission_mode: Option<PermissionMode>,
        version: Option<String>,
        api_key_source: Option<String>,
        billing: Option<String>,
    },
    Capabilities {
        caps: AgentCapabilities,
    },
    Exited {
        code: Option<i32>,
        stderr_tail: String,
        expected: bool,
    },
    ExternalId {
        id: String,
    },
    History {
        turns: Vec<Value>,
    },
    BackgroundTasks {
        tasks: Vec<BackgroundTask>,
    },
}

impl From<AdapterOutput> for WireOut {
    fn from(o: AdapterOutput) -> Self {
        match o {
            AdapterOutput::Event { item_id, turn_id, event, raw } => WireOut::Event { item_id, turn_id, event, raw },
            AdapterOutput::Delta { item_id, field, text } => WireOut::Delta { item_id, field, text },
            AdapterOutput::Info(i) => WireOut::Info {
                model: i.model,
                permission_mode: i.permission_mode,
                version: i.version,
                api_key_source: i.api_key_source,
                billing: i.billing,
            },
            AdapterOutput::Capabilities(caps) => WireOut::Capabilities { caps },
            AdapterOutput::Exited { code, stderr_tail, expected } => WireOut::Exited { code, stderr_tail, expected },
            AdapterOutput::ExternalId(id) => WireOut::ExternalId { id },
            AdapterOutput::History(turns) => WireOut::History { turns },
            AdapterOutput::BackgroundTasks(tasks) => WireOut::BackgroundTasks { tasks },
        }
    }
}

impl From<WireOut> for AdapterOutput {
    fn from(o: WireOut) -> Self {
        match o {
            WireOut::Event { item_id, turn_id, event, raw } => AdapterOutput::Event { item_id, turn_id, event, raw },
            WireOut::Delta { item_id, field, text } => AdapterOutput::Delta { item_id, field, text },
            WireOut::Info { model, permission_mode, version, api_key_source, billing } => {
                AdapterOutput::Info(AgentInfo { model, permission_mode, version, api_key_source, billing })
            }
            WireOut::Capabilities { caps } => AdapterOutput::Capabilities(caps),
            WireOut::Exited { code, stderr_tail, expected } => AdapterOutput::Exited { code, stderr_tail, expected },
            WireOut::ExternalId { id } => AdapterOutput::ExternalId(id),
            WireOut::History { turns } => AdapterOutput::History(turns),
            WireOut::BackgroundTasks { tasks } => AdapterOutput::BackgroundTasks(tasks),
        }
    }
}

/// `/link/exec/worktree`: a worktree for `branch` (made from `base`, or checked out when it exists and
/// `existing_branch`), in the remote project's checkout, cloned first if it isn't there.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorktreeReq {
    pub branch: String,
    pub base: String,
    pub existing_branch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorktreeMade {
    pub path: String,
    pub base_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoveWorktreeReq {
    pub branch: String,
    pub force: bool,
    pub delete_branch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Removed {
    /// Why the branch stayed, when it was to be deleted and wasn't.
    pub branch_error: Option<String>,
}

/// `/link/exec/git`: a look at, or an update of, a worktree's branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitReq {
    pub branch: String,
    pub op: GitOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum GitOp {
    /// Whether the branch has commits of its own or uncommitted changes (`attempt.has_changes`).
    HasChanges { base: String, base_commit: String },
    /// The branch's head there (merge detection): `{head}`, null when it has none.
    Head,
    /// Merges the base in and pushes (`attempt.update_base`): `{port, output}`.
    UpdateBase { base: String, remote_pr: bool },
}

/// `/link/exec/mcp`: an agent's call to pdt-mcp, with its session's token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpReq {
    pub token: String,
    pub accept: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpReply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

/// `/link/exec/pty/open`: start (or attach to) a terminal session there. The session is the owner's row; the remote
/// instance runs it in `branch`'s worktree of its checkout (None: the checkout itself, cloned first when it isn't
/// there), whatever the row's folder says.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PtyOpenReq {
    pub session: pdt_core::session::Session,
    pub ticket_key: Option<String>,
    pub branch: Option<String>,
    /// The project's base branch, fetched when the checkout is used.
    pub base: String,
    pub cols: u16,
    pub rows: u16,
    /// Made just now: the agent starts afresh (else it resumes its conversation when the remote has it saved).
    pub fresh: bool,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub credentials: CredentialPolicy,
    pub instructions: Option<WireInstructions>,
    pub mcp: Option<WireMcp>,
}

/// `/link/exec/pty/attach`: a one-time ticket for the session's terminal WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PtyTicket {
    pub ticket: String,
}

/// `/link/exec/pty/write`: bytes typed into a terminal there (a message PDT types into an agent's TUI).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PtyWriteReq {
    pub session_id: String,
    /// Base64.
    pub data: String,
}

/// `/link/exec/activity`: a hook report from a terminal agent there, for its session on the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityReq {
    pub session_id: String,
    pub report: pdt_core::activity::HookReport,
}

/// The executor's routes (the other end calls them); `link::router` merges them.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/link/exec/open", post(remote::open_route))
        .route("/link/exec/send", post(remote::send_route))
        .route("/link/exec/answer", post(remote::answer_route))
        .route("/link/exec/interrupt", post(remote::interrupt_route))
        .route("/link/exec/set", post(remote::set_route))
        .route("/link/exec/close", post(remote::close_route))
        .route("/link/exec/events", post(remote::events_route))
        .route("/link/exec/worktree", post(remote::worktree_route))
        .route("/link/exec/remove-worktree", post(remote::remove_worktree_route))
        .route("/link/exec/git", post(remote::git_route))
        .route("/link/exec/mcp", post(owner::mcp_route))
        .route("/link/exec/pty/open", post(remote_pty::open_route))
        .route("/link/exec/pty/attach", post(remote_pty::attach_route))
        .route("/link/exec/pty/write", post(remote_pty::write_route))
        .route("/link/exec/pty/ws/{ticket}", axum::routing::get(remote_pty::ws_route))
        .route("/link/exec/activity", post(terminals::activity_route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_outputs_cross_the_wire_unchanged() {
        let outs = vec![
            AdapterOutput::Event {
                item_id: Some("i".into()),
                turn_id: None,
                event: AgentEvent::Error { text: "x".into(), retryable: true },
                raw: None,
            },
            AdapterOutput::Delta { item_id: "i".into(), field: DeltaField::Markdown, text: "he".into() },
            AdapterOutput::Info(AgentInfo { model: Some("m".into()), ..Default::default() }),
            AdapterOutput::Exited { code: Some(1), stderr_tail: "bye".into(), expected: false },
            AdapterOutput::ExternalId("t".into()),
        ];
        for o in outs {
            let wire: WireOut = o.clone().into();
            let back: WireOut = serde_json::from_str(&serde_json::to_string(&wire).unwrap()).unwrap();
            assert_eq!(AdapterOutput::from(back), o);
        }
    }
}
