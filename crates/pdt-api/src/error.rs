use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pdt_agents::PtyError;
use pdt_core::project::ValidationError;
use pdt_core::session::SessionValidationError;
use pdt_core::ticket::TicketValidationError;
use pdt_core::workspace::WorkspaceValidationError;
use pdt_core::worktree::WorktreeValidationError;
use pdt_git::GitError;
use pdt_store::{StoreError, TicketError};
use serde::Serialize;
use ts_rs::TS;

/// Every code an [`ApiError`] has: the web and agents match on them, so a code means one failure with one status.
/// - 400 the request is wrong: `invalid_input` ([`ApiError::invalid`]; 422 when its JSON does not fit), or a more
///   specific code ([`ApiError::bad_request`]); `unknown_*` is something the request names that does not exist.
/// - 403 not allowed ([`ApiError::forbidden`]); 404 what the URL names does not exist (`not_found`, [`ApiError::not_found`]);
///   409 not now, in this state ([`ApiError::conflict`]).
/// - 502 a service PDT called failed ([`ApiError::upstream`]); 503 PDT can't do it now ([`ApiError::unavailable`]):
///   `keychain_unavailable` ([`ApiError::keychain`]), `write_failed` ([`ApiError::write_failed`]), `read_failed` (pdt-mcp
///   couldn't check a token).
/// - 500 `internal` ([`ApiError::internal`]) and the few failures of PDT's own setup (`git_missing`, `spawn_failed`).
pub mod codes {
    pub const AGENT_ERROR: &str = "agent_error";
    pub const AGENT_NOT_FOUND: &str = "agent_not_found";
    pub const AGENT_NOT_RUNNING: &str = "agent_not_running";
    pub const AGENT_OFF: &str = "agent_off";
    pub const ALREADY_FINISHED: &str = "already_finished";
    pub const ALREADY_LINKED: &str = "already_linked";
    pub const ALREADY_RUNNING: &str = "already_running";
    pub const ATTEMPT_ACTIVE: &str = "attempt_active";
    pub const ATTEMPT_INACTIVE: &str = "attempt_inactive";
    pub const BAD_PATTERN: &str = "bad_pattern";
    pub const BAD_SIGNATURE: &str = "bad_signature";
    pub const BRANCH_EXISTS: &str = "branch_exists";
    pub const BRANCH_NOT_DELETED: &str = "branch_not_deleted";
    pub const BUG_REPORTS_OFF: &str = "bug_reports_off";
    pub const BUILTIN_PERSONA: &str = "builtin_persona";
    pub const BUILTIN_TEMPLATE: &str = "builtin_template";
    pub const BUILTIN_WORKFLOW: &str = "builtin_workflow";
    pub const CANNOT_ANSWER: &str = "cannot_answer";
    pub const CANNOT_CANCEL: &str = "cannot_cancel";
    pub const CANNOT_RESUME: &str = "cannot_resume";
    pub const CANNOT_RETRY: &str = "cannot_retry";
    pub const CANNOT_RUN: &str = "cannot_run";
    pub const CHAT_NOT_STARTABLE: &str = "chat_not_startable";
    pub const CHECKSUM_MISMATCH: &str = "checksum_mismatch";
    pub const CI_FAILED: &str = "ci_failed";
    pub const CI_PENDING: &str = "ci_pending";
    pub const CI_UNREADABLE: &str = "ci_unreadable";
    pub const CLONE_FAILED: &str = "clone_failed";
    pub const COMMENT_SENT: &str = "comment_sent";
    pub const CONFLICT: &str = "conflict";
    pub const CONNECTION_EXISTS: &str = "connection_exists";
    pub const CUSTOMIZED: &str = "customized";
    pub const DEVICE_FLOW_FAILED: &str = "device_flow_failed";
    pub const DOCUMENT_MISSING: &str = "document_missing";
    pub const DOWNLOAD_FAILED: &str = "download_failed";
    pub const DRAFT_TIMEOUT: &str = "draft_timeout";
    pub const DRAFT_UNSUPPORTED: &str = "draft_unsupported";
    pub const FILE_CHANGED: &str = "file_changed";
    pub const FILE_EXISTS: &str = "file_exists";
    pub const FOLDER_EXISTS: &str = "folder_exists";
    pub const FOLDER_MISSING: &str = "folder_missing";
    pub const FOLDER_UNREADABLE: &str = "folder_unreadable";
    pub const FORBIDDEN_HOST: &str = "forbidden_host";
    pub const FORBIDDEN_IDENTITY: &str = "forbidden_identity";
    pub const FORBIDDEN_ORIGIN: &str = "forbidden_origin";
    pub const FORGE_ERROR: &str = "forge_error";
    pub const FORGE_UNAVAILABLE: &str = "forge_unavailable";
    pub const GITHUB_ERROR: &str = "github_error";
    pub const GITHUB_UNREACHABLE: &str = "github_unreachable";
    pub const GIT_ERROR: &str = "git_error";
    pub const GIT_IDENTITY_MISSING: &str = "git_identity_missing";
    pub const GIT_MISSING: &str = "git_missing";
    pub const HOST_BUSY: &str = "host_busy";
    pub const IMAGE_TOO_LARGE: &str = "image_too_large";
    pub const INTEGRATION_DISABLED: &str = "integration_disabled";
    pub const INTERNAL: &str = "internal";
    pub const INVALID_BRANCH: &str = "invalid_branch";
    pub const INVALID_EXPORT: &str = "invalid_export";
    pub const INVALID_FOLDER: &str = "invalid_folder";
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const INVALID_MESSAGE: &str = "invalid_message";
    pub const INVALID_PARENT: &str = "invalid_parent";
    pub const INVALID_RUN_CONFIG: &str = "invalid_run_config";
    pub const INVALID_STAGE: &str = "invalid_stage";
    pub const INVALID_TITLE: &str = "invalid_title";
    pub const INVALID_TOKEN: &str = "invalid_token";
    pub const IN_USE: &str = "in_use";
    pub const JIRA_FAILED: &str = "jira_failed";
    pub const JIRA_ISSUE_NOT_FOUND: &str = "jira_issue_not_found";
    pub const JIRA_NOT_LINKED: &str = "jira_not_linked";
    pub const KEYCHAIN_UNAVAILABLE: &str = "keychain_unavailable";
    pub const KEY_TAKEN: &str = "key_taken";
    pub const LINK_CYCLE: &str = "link_cycle";
    pub const LINK_FAILED: &str = "link_failed";
    pub const MERGE_BLOCKED: &str = "merge_blocked";
    pub const MERGE_FAILED: &str = "merge_failed";
    pub const METHOD_NOT_ALLOWED: &str = "method_not_allowed";
    pub const NEEDS_MAPPING: &str = "needs_mapping";
    pub const NEEDS_YOU: &str = "needs_you";
    pub const NOTHING_TO_ASK: &str = "nothing_to_ask";
    pub const NOTHING_TO_DRAFT: &str = "nothing_to_draft";
    pub const NOT_AN_AGENT: &str = "not_an_agent";
    pub const NOT_A_FOLDER: &str = "not_a_folder";
    pub const NOT_A_GIT_REPO: &str = "not_a_git_repo";
    pub const NOT_BUILTIN: &str = "not_builtin";
    pub const NOT_CONVERSATION: &str = "not_conversation";
    pub const NOT_EDITABLE: &str = "not_editable";
    pub const NOT_FOUND: &str = "not_found";
    pub const NOT_INSTALLABLE: &str = "not_installable";
    pub const NOT_MCP: &str = "not_mcp";
    pub const NOT_PROPOSED: &str = "not_proposed";
    pub const NOT_RUNNING: &str = "not_running";
    pub const NOT_SUPERVISED: &str = "not_supervised";
    pub const NOT_THE_UI: &str = "not_the_ui";
    pub const NOT_THE_USER: &str = "not_the_user";
    pub const NOT_YOURS: &str = "not_yours";
    pub const NO_ACCOUNT: &str = "no_account";
    pub const NO_AGENT_SESSION: &str = "no_agent_session";
    pub const NO_BUG_PROJECT: &str = "no_bug_project";
    pub const NO_CLIENT_ID: &str = "no_client_id";
    pub const NO_COMMITS: &str = "no_commits";
    pub const NO_DEVICE_FLOW: &str = "no_device_flow";
    pub const NO_DRAFTS: &str = "no_drafts";
    pub const NO_FREE_PORT: &str = "no_free_port";
    pub const NO_GITHUB_LOGIN: &str = "no_github_login";
    pub const NO_NEXT_PART: &str = "no_next_part";
    pub const NO_PROVIDER: &str = "no_provider";
    pub const NO_REFINE_WORKFLOW: &str = "no_refine_workflow";
    pub const NO_SIGN_IN: &str = "no_sign_in";
    pub const NO_TURN_YET: &str = "no_turn_yet";
    pub const NO_UPDATE: &str = "no_update";
    pub const OWNER_REQUIRED: &str = "owner_required";
    pub const PARTS_OPEN: &str = "parts_open";
    pub const PATH_EXISTS: &str = "path_exists";
    pub const PATH_NOT_FOUND: &str = "path_not_found";
    pub const PATH_OUTSIDE: &str = "path_outside";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const PERSONAL_PROJECT: &str = "personal_project";
    pub const PERSONA_IN_USE: &str = "persona_in_use";
    pub const PM_CLOSED: &str = "pm_closed";
    pub const PROGRAM_NOT_FOUND: &str = "program_not_found";
    pub const PROJECT_WORKSPACE: &str = "project_workspace";
    pub const PROVIDER_BUSY: &str = "provider_busy";
    pub const PROVIDER_FAILED: &str = "provider_failed";
    pub const PR_UNREADABLE: &str = "pr_unreadable";
    pub const READ_FAILED: &str = "read_failed";
    pub const REFINE_RUNNING: &str = "refine_running";
    pub const REGISTRY_UNAVAILABLE: &str = "registry_unavailable";
    pub const REQUEST_CLOSED: &str = "request_closed";
    pub const RUN_ACTIVE: &str = "run_active";
    pub const RUN_STOPPING: &str = "run_stopping";
    pub const SECRET_EXISTS: &str = "secret_exists";
    pub const SESSION_ENDED: &str = "session_ended";
    pub const SESSION_NOT_RUNNING: &str = "session_not_running";
    pub const SESSION_RUNNING: &str = "session_running";
    pub const SHEET_CONFLICT: &str = "sheet_conflict";
    pub const SHUTTING_DOWN: &str = "shutting_down";
    pub const SIGN_IN_FAILED: &str = "sign_in_failed";
    pub const SIGN_IN_TIMEOUT: &str = "sign_in_timeout";
    pub const SPAWN_FAILED: &str = "spawn_failed";
    pub const STALE_UI_TOKEN: &str = "stale_ui_token";
    pub const START_AT_LOGIN_FAILED: &str = "start_at_login_failed";
    pub const START_AT_LOGIN_UNAVAILABLE: &str = "start_at_login_unavailable";
    pub const START_BLOCKED: &str = "start_blocked";
    pub const SWITCH_UNSUPPORTED: &str = "switch_unsupported";
    pub const TERMINAL_UNSUPPORTED: &str = "terminal_unsupported";
    pub const TICKET_ACTIVE: &str = "ticket_active";
    pub const TICKET_CLOSED: &str = "ticket_closed";
    pub const TICKET_HAS_PARTS: &str = "ticket_has_parts";
    pub const TICKET_NOT_REFINABLE: &str = "ticket_not_refinable";
    pub const TICKET_PROPOSAL: &str = "ticket_proposal";
    pub const TOKEN_NO_ISSUE_ACCESS: &str = "token_no_issue_access";
    pub const TOKEN_REJECTED: &str = "token_rejected";
    pub const TOO_LARGE: &str = "too_large";
    pub const TOO_MANY_ATTACHMENTS: &str = "too_many_attachments";
    pub const TOO_MANY_FAILURES: &str = "too_many_failures";
    pub const TOO_MANY_RUNS: &str = "too_many_runs";
    pub const TRACKER_ERROR: &str = "tracker_error";
    pub const TURN_RUNNING: &str = "turn_running";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const UNKNOWN_AGENT: &str = "unknown_agent";
    pub const UNKNOWN_ATTACHMENT: &str = "unknown_attachment";
    pub const UNKNOWN_BRANCH: &str = "unknown_branch";
    pub const UNKNOWN_FORGE: &str = "unknown_forge";
    pub const UNKNOWN_REF: &str = "unknown_ref";
    pub const UNKNOWN_RUN: &str = "unknown_run";
    pub const UNKNOWN_WORKSPACE: &str = "unknown_workspace";
    pub const UNRECOGNIZED: &str = "unrecognized";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const UNSUPPORTED_IMAGE: &str = "unsupported_image";
    pub const UNTRUSTED_COMMANDS: &str = "untrusted_commands";
    pub const UNTRUSTED_TICKET: &str = "untrusted_ticket";
    pub const UPDATES_DISABLED: &str = "updates_disabled";
    pub const UPDATE_FAILED: &str = "update_failed";
    pub const UPDATE_IN_PROGRESS: &str = "update_in_progress";
    pub const WEBSOCKET_REQUIRED: &str = "websocket_required";
    pub const WORKFLOW_EXISTS: &str = "workflow_exists";
    pub const WORKFLOW_INVALID: &str = "workflow_invalid";
    pub const WORKFLOW_IN_REPO: &str = "workflow_in_repo";
    pub const WORKTREE_ATTEMPT: &str = "worktree_attempt";
    pub const WORKTREE_BUSY: &str = "worktree_busy";
    pub const WORKTREE_DIRTY: &str = "worktree_dirty";
    pub const WORKTREE_MISSING: &str = "worktree_missing";
    pub const WORKTREE_PATH_TAKEN: &str = "worktree_path_taken";
    pub const WRITE_FAILED: &str = "write_failed";
    pub const WRONG_ANSWER: &str = "wrong_answer";
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    /// What the client needs to act on the error, for the few that carry it (`untrusted_commands`: the commands).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    data: Option<serde_json::Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ApiError {
    /// An error with a code from [`codes`]; the constructors below give the usual statuses.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), data: None }
    }

    /// The error with what the client needs to act on it ([`ApiErrorBody::data`]).
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    /// 404: what the URL names does not exist.
    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, codes::NOT_FOUND, format!("{what} not found"))
    }

    /// 400 `invalid_input`: the request is wrong, as the message says.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, codes::INVALID_INPUT, message)
    }

    /// 400 with a code more specific than `invalid_input`.
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// 403: the caller may not do this.
    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    /// 409: not in this state (it is running, already done, in use).
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    /// 502: a service PDT called (a forge, a registry, a provider) failed.
    pub fn upstream(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, code, message)
    }

    /// 503: PDT can't do this now (it is shutting down, a service is off), but may later.
    pub fn unavailable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
    }

    /// 503 `keychain_unavailable`: the OS keychain refused or failed a call; logged in full, said in
    /// [`short_text`].
    pub fn keychain(e: impl std::fmt::Display) -> Self {
        let msg = e.to_string();
        tracing::warn!(error = %msg, "keychain call failed");
        Self::unavailable(codes::KEYCHAIN_UNAVAILABLE, short_text(&msg))
    }

    /// 500: logged with what it says, which the response doesn't carry.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::internal_as("internal error", err)
    }

    /// 500 `internal` saying `message`, with `err`, which the response doesn't carry, logged.
    pub fn internal_as(message: &str, err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "{message}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, codes::INTERNAL, message)
    }

    /// An error known only by its text (the workflow engine's): a failed write when it says so
    /// ([`pdt_store::is_write_failure`]), else [`ApiError::internal`].
    pub fn from_text(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        if pdt_store::is_write_failure(&msg) {
            return Self::write_failed(msg);
        }
        Self::internal(msg)
    }

    /// A write that could not be made (a full disk, PDT-323): 503, which [`note_write_failures`] records, so the
    /// Inbox says PDT can't save changes. `msg` is logged; the response says [`pdt_store::WRITE_FAILED`] and whether
    /// the disk is full, not the database's own text.
    pub fn write_failed(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::error!(error = %msg, "a write failed");
        Self::unavailable(codes::WRITE_FAILED, write_failed_text(&msg))
    }

    /// A file operation's failure, `doing` saying what PDT was doing ("writing the briefing"): a missing file is 404,
    /// one PDT may not touch 403, a full disk or read-only volume a failed write, anything else internal.
    pub fn io(doing: &str, e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        let says = |m: &dyn std::fmt::Display| if doing.is_empty() { m.to_string() } else { format!("{doing}: {m}") };
        match e.kind() {
            NotFound => Self::new(StatusCode::NOT_FOUND, codes::NOT_FOUND, says(&"no such file or folder")),
            PermissionDenied => Self::forbidden(codes::PERMISSION_DENIED, says(&"permission denied")),
            StorageFull | QuotaExceeded | ReadOnlyFilesystem => {
                Self::write_failed(format!("{}: {}", pdt_store::WRITE_FAILED, says(&e)))
            }
            _ if doing.is_empty() => Self::internal(e),
            _ => Self::internal_as(&format!("{doing} failed (PDT's log says why)"), says(&e)),
        }
    }
}

impl ApiError {
    /// Reading or opening `what` failed: 404 `what` not found when it is missing, else as [`ApiError::io`] says
    /// (PDT-524: a permission error or a full disk isn't a missing file).
    pub fn io_404(what: &str, doing: &str, e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound { Self::not_found(what) } else { Self::io(doing, e) }
    }
}

/// `Option` to a 404: `st.store.get_x(id).await?.or_404("x")?`.
pub trait OrNotFound<T> {
    fn or_404(self, what: &str) -> Result<T, ApiError>;
}

impl<T> OrNotFound<T> for Option<T> {
    fn or_404(self, what: &str) -> Result<T, ApiError> {
        self.ok_or_else(|| ApiError::not_found(what))
    }
}

/// A response's failed write ([`ApiError::write_failed`]), for [`note_write_failures`].
#[derive(Clone)]
struct WriteFailed(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let failed = (self.code == codes::WRITE_FAILED).then(|| WriteFailed(self.message.clone()));
        let mut r =
            (self.status, Json(ApiErrorBody { code: self.code.into(), message: self.message, data: self.data }))
                .into_response();
        if let Some(f) = failed {
            r.extensions_mut().insert(f);
        }
        r
    }
}

/// Middleware: a request that failed to write records it in the store (PDT-323), so the user hears of a full disk
/// from their own clicks too, not only once the engine writes.
pub(crate) async fn note_write_failures(
    axum::extract::State(st): axum::extract::State<crate::routes::AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let r = next.run(req).await;
    if let Some(WriteFailed(msg)) = r.extensions().get::<WriteFailed>() {
        st.store.note_write_failure(msg);
    }
    r
}

impl From<ValidationError> for ApiError {
    fn from(e: ValidationError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<WorktreeValidationError> for ApiError {
    fn from(e: WorktreeValidationError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<WorkspaceValidationError> for ApiError {
    fn from(e: WorkspaceValidationError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<SessionValidationError> for ApiError {
    fn from(e: SessionValidationError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<TicketValidationError> for ApiError {
    fn from(e: TicketValidationError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<TicketError> for ApiError {
    fn from(e: TicketError) -> Self {
        match e {
            TicketError::NotFound(what) => Self::not_found(what),
            TicketError::Invalid(_) => Self::invalid(e.to_string()),
            TicketError::Cycle(_) => Self::conflict(codes::LINK_CYCLE, e.to_string()),
            TicketError::Active(ref key) => Self::conflict(
                codes::TICKET_ACTIVE,
                format!("{key} has an active attempt; abandon it or finish the work first"),
            ),
            TicketError::InvalidParent(_) => Self::conflict(codes::INVALID_PARENT, e.to_string()),
            TicketError::PartsOpen { .. } => Self::conflict(codes::PARTS_OPEN, e.to_string()),
            TicketError::HasParts(_) => Self::conflict(codes::TICKET_HAS_PARTS, e.to_string()),
            TicketError::Untrusted(_) => Self::forbidden(codes::UNTRUSTED_TICKET, e.to_string()),
            TicketError::Store(s) => s.into(),
        }
    }
}

impl From<PtyError> for ApiError {
    fn from(e: PtyError) -> Self {
        match e {
            PtyError::ProgramNotFound(_) => Self::bad_request(codes::PROGRAM_NOT_FOUND, e.to_string()),
            PtyError::Spawn(_) => {
                tracing::error!(error = %e, "a terminal could not start");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    codes::SPAWN_FAILED,
                    "the terminal could not start (PDT's log says why)",
                )
            }
            PtyError::NotFound(_) | PtyError::NotRunning(_) => {
                Self::conflict(codes::NOT_RUNNING, "this session is not running")
            }
            PtyError::AlreadyExists(_) => Self::conflict(codes::ALREADY_RUNNING, e.to_string()),
            PtyError::ShuttingDown => Self::unavailable(codes::SHUTTING_DOWN, "the daemon is shutting down"),
            PtyError::Io(_) => Self::internal(e),
            PtyError::InputFull(_) => Self::conflict(codes::CONFLICT, e.to_string()),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::io("", e)
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::internal(e)
    }
}

impl From<crate::secrets::SecretError> for ApiError {
    fn from(e: crate::secrets::SecretError) -> Self {
        Self::keychain(e)
    }
}

/// JSON that does not fit what it should be (a step's input): it says where.
impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rej: JsonRejection) -> Self {
        Self::new(rej.status(), codes::INVALID_INPUT, rej.body_text())
    }
}

impl From<GitError> for ApiError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::PathNotFound(_) => Self::bad_request(codes::PATH_NOT_FOUND, e.to_string()),
            GitError::NotARepo(_) => Self::bad_request(codes::NOT_A_GIT_REPO, e.to_string()),
            GitError::GitMissing => Self::new(StatusCode::INTERNAL_SERVER_ERROR, codes::GIT_MISSING, e.to_string()),
            GitError::Command(ref said) => {
                tracing::warn!(error = %said, "git failed");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    codes::GIT_ERROR,
                    format!("git failed: {}", short_text(said)),
                )
            }
            GitError::InvalidBranch(_) => Self::bad_request(codes::INVALID_BRANCH, e.to_string()),
            GitError::BranchExists(_) => Self::conflict(codes::BRANCH_EXISTS, e.to_string()),
            GitError::UnknownRef(_) => Self::bad_request(codes::UNKNOWN_REF, e.to_string()),
            GitError::BadPattern(_) => Self::bad_request(codes::BAD_PATTERN, e.to_string()),
            GitError::NoIdentity => Self::conflict(codes::GIT_IDENTITY_MISSING, e.to_string()),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => Self::conflict(codes::CONFLICT, "this record already exists"),
            e @ StoreError::Write(_) => Self::write_failed(e.to_string()),
            StoreError::Io(e) => Self::io("", e),
            other => Self::internal_as(DB_UNREADABLE, other),
        }
    }
}

/// What a database error other than a failed write says (its own text goes to the log).
pub(crate) const DB_UNREADABLE: &str = "PDT could not read its database (its log says why)";

/// How many characters of a foreign error [`short_text`] keeps.
const SHORT_CHARS: usize = 200;

/// What a failed write's response says for `raw`, the database's text: that PDT can't save changes
/// ([`pdt_store::WRITE_FAILED`]) and, when it is so, that the disk is full ([`pdt_store::is_disk_full`]).
fn write_failed_text(raw: &str) -> String {
    if pdt_store::is_disk_full(raw) {
        format!("{}: the database or disk is full", pdt_store::WRITE_FAILED)
    } else {
        format!("{} (PDT's log says why)", pdt_store::WRITE_FAILED)
    }
}

/// What a foreign error's text (git's stderr, the OS's, a keychain's) shows the user, its full text being logged:
/// the first line that says what failed (git's `fatal:` or `error:` line, without that, when it has one), each
/// absolute path cut to its last part (no home folder or checkout in screenshots and bug reports), and at most
/// [`SHORT_CHARS`] characters.
pub fn short_text(raw: &str) -> String {
    let lines = || raw.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines()
        .find_map(|l| l.strip_prefix("fatal: ").or_else(|| l.strip_prefix("error: ")))
        .or_else(|| lines().next())
        .unwrap_or("");
    let short = line.split(' ').map(without_path).collect::<Vec<_>>().join(" ");
    match short.char_indices().nth(SHORT_CHARS) {
        Some((at, _)) => format!("{}…", &short[..at]),
        None => short,
    }
}

/// `word`, an absolute path (`/…`, `~/…`, `C:\…`, `\\server\…`) cut to its last part, keeping the quotes and
/// punctuation around it.
fn without_path(word: &str) -> std::borrow::Cow<'_, str> {
    const OPEN: &[char] = &['\'', '"', '`', '(', '[', '<'];
    const CLOSE: &[char] = &['\'', '"', '`', ')', ']', '>', ',', '.', ';', ':'];
    let rest = word.trim_start_matches(OPEN);
    let core = rest.trim_end_matches(CLOSE);
    let b = core.as_bytes();
    let absolute = (b.len() > 1 && (b[0] == b'/' || b.starts_with(b"~/") || b.starts_with(b"\\\\")))
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'));
    if !absolute {
        return word.into();
    }
    let last = core.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default();
    let lead = &word[..word.len() - rest.len()];
    format!("{lead}{last}{}", &rest[core.len()..]).into()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn git_command_error_is_500_git_error_with_stderrs_failing_line_without_paths() {
        let stderr = "hint: see git help\nfatal: '/Users/someone/work/repo/.worktrees/feat' already exists\n";
        let e = ApiError::from(GitError::Command(stderr.into()));
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(e.code, "git_error");
        assert_eq!(e.message, "git failed: 'feat' already exists");
    }

    #[test]
    fn a_missing_git_identity_says_how_to_set_it() {
        let e = ApiError::from(GitError::NoIdentity);
        assert_eq!((e.status, e.code), (StatusCode::CONFLICT, "git_identity_missing"));
        assert!(e.message.contains("git config --global user.name"), "{}", e.message);
    }

    #[test]
    fn short_text_keeps_the_failing_line_without_paths() {
        assert_eq!(short_text("boom"), "boom");
        assert_eq!(short_text("\n  error: pathspec 'src/x' did not match\nmore"), "pathspec 'src/x' did not match");
        assert_eq!(short_text("could not create /home/me/p/notes: denied"), "could not create notes: denied");
        assert_eq!(short_text(r"open (C:\Users\me\p\a.txt) failed"), "open (a.txt) failed");
        assert_eq!(short_text("~/work/x/ is gone; origin/main and / stay"), "x is gone; origin/main and / stay");
        let long = "x".repeat(300);
        assert_eq!(short_text(&long).chars().count(), SHORT_CHARS + 1);
    }

    #[test]
    fn store_errors_say_what_failed_not_the_databases_text() {
        let db = sqlx::Error::Protocol("code: 13 secret detail".into());
        let e = ApiError::from(StoreError::Db(db));
        assert_eq!((e.code, e.message.as_str()), (codes::INTERNAL, DB_UNREADABLE));
        let e = ApiError::write_failed(
            "PDT can't save changes: error returned from database: (code: 13) database or disk is full",
        );
        assert_eq!(e.code, codes::WRITE_FAILED);
        assert_eq!(e.message, "PDT can't save changes: the database or disk is full");
        assert!(pdt_store::is_write_failure(&e.message) && pdt_store::is_disk_full(&e.message));
        let e = ApiError::write_failed("PDT can't save changes: (code: 10) disk I/O error");
        assert_eq!(e.message, "PDT can't save changes (PDT's log says why)");
        assert!(!pdt_store::is_disk_full(&e.message));
    }

    #[test]
    fn io_errors_map_by_kind() {
        use std::io::{Error, ErrorKind};
        let e = ApiError::from(Error::from(ErrorKind::NotFound));
        assert_eq!((e.status, e.code), (StatusCode::NOT_FOUND, codes::NOT_FOUND));
        let e = ApiError::io("reading notes.md", Error::from(ErrorKind::PermissionDenied));
        assert_eq!((e.status, e.code), (StatusCode::FORBIDDEN, codes::PERMISSION_DENIED));
        assert_eq!(e.message, "reading notes.md: permission denied");
        for kind in [ErrorKind::StorageFull, ErrorKind::ReadOnlyFilesystem, ErrorKind::QuotaExceeded] {
            let e = ApiError::io("writing notes.md", Error::from(kind));
            assert_eq!((e.status, e.code), (StatusCode::SERVICE_UNAVAILABLE, codes::WRITE_FAILED), "{kind:?}");
            assert!(pdt_store::is_write_failure(&e.message), "the Inbox hears of it: {}", e.message);
        }
        let e = ApiError::from(Error::other("boom"));
        assert_eq!(
            (e.status, e.code, e.message.as_str()),
            (StatusCode::INTERNAL_SERVER_ERROR, codes::INTERNAL, "internal error")
        );
    }

    #[test]
    fn a_failed_write_is_known_by_its_type_or_engine_text() {
        assert_eq!(ApiError::internal(format!("{}: disk full", pdt_store::WRITE_FAILED)).code, codes::INTERNAL);
        assert_eq!(ApiError::from_text(format!("{}: disk full", pdt_store::WRITE_FAILED)).code, codes::WRITE_FAILED);
        assert_eq!(ApiError::from_text("the step failed").code, codes::INTERNAL);
    }

    /// The `.rs` files under `dir`, recursively.
    pub(crate) fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// The workflow says what failed through its one `err` ([`crate::workflow::err`]), never a database's or git's
    /// own text: a step's error shows in the run view and in stuck Inbox items.
    #[test]
    fn workflow_errors_go_through_err() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/workflow");
        let mut files = vec![];
        sources(&root, &mut files);
        assert!(files.len() > 30, "{}", files.len());
        let raw = files
            .iter()
            .filter(|f| {
                let text = std::fs::read_to_string(f).unwrap();
                text.contains(concat!("map_err(|e| e", ".to_string())"))
                    || text.contains(concat!("fn err", "(")) && !f.ends_with("workflow/mod.rs")
            })
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>();
        assert!(raw.is_empty(), "use crate::workflow::err (or say what failed):\n{}", raw.join("\n"));
    }

    /// Every code is in [`codes`], once, named after itself; no route spells one out (a typo would be a new code).
    #[test]
    fn codes_come_from_the_one_list() {
        let src = include_str!("error.rs");
        let consts: Vec<(&str, &str)> = src
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const ")?.strip_suffix("\";")?.split_once(": &str = \""))
            .collect();
        assert!(consts.len() > 100, "{}", consts.len());
        let mut seen = std::collections::HashSet::new();
        for (name, value) in &consts {
            assert_eq!(name.to_lowercase(), *value, "{name}");
            assert!(seen.insert(*value), "{value} is listed twice");
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = vec![];
        sources(&root.join("src"), &mut files);
        // Every plugin crate's too: their codes are the API's.
        let plugins = std::fs::read_dir(root.join(".."))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("pdt-int-")))
            .collect::<Vec<_>>();
        assert!(plugins.len() > 5, "{}", plugins.len());
        for p in plugins {
            sources(&p.join("src"), &mut files);
        }
        // Without their `(`, so this list doesn't match itself.
        let calls =
            ["ApiError::new", "Self::new", "::conflict", "::bad_request", "::forbidden", "::upstream", "::unavailable"];
        let (mut spelled, mut seen_calls) = (vec![], 0);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            // A body written by hand, not through an `ApiError`.
            if text.contains(concat!("\"code\": ", "\"")) {
                spelled.push(format!("{}: a hand-written \"code\"", f.display()));
            }
            for call in calls.map(|c| format!("{c}(")) {
                for (at, _) in text.match_indices(&call) {
                    seen_calls += 1;
                    let args = &text[at + call.len()..];
                    let args = if call.ends_with("new(") { args.split_once(',').map_or("", |a| a.1) } else { args };
                    if args.trim_start().starts_with('"') {
                        spelled.push(format!(
                            "{}: {}",
                            f.display(),
                            &args.trim_start()[..args.trim_start().len().min(40)]
                        ));
                    }
                }
            }
        }
        assert!(seen_calls > 200, "the scan found the calls: {seen_calls}");
        assert!(spelled.is_empty(), "use a constant from `codes`:\n{}", spelled.join("\n"));
    }
}
