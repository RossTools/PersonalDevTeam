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

    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", format!("{what} not found"))
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        let msg = err.to_string();
        if pdt_store::is_write_failure(&msg) {
            return Self::write_failed(msg);
        }
        tracing::error!(error = %msg, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }

    /// A write the database could not make (a full disk, PDT-323): 503 with what it said, which
    /// [`note_write_failures`] records, so the Inbox says PDT can't save changes.
    pub fn write_failed(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        tracing::error!(error = %msg, "a write failed");
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "write_failed", msg)
    }
}

/// A response's failed write ([`ApiError::write_failed`]), for [`note_write_failures`].
#[derive(Clone)]
struct WriteFailed(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let failed = (self.code == "write_failed").then(|| WriteFailed(self.message.clone()));
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
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<WorktreeValidationError> for ApiError {
    fn from(e: WorktreeValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<WorkspaceValidationError> for ApiError {
    fn from(e: WorkspaceValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<SessionValidationError> for ApiError {
    fn from(e: SessionValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<TicketValidationError> for ApiError {
    fn from(e: TicketValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

impl From<TicketError> for ApiError {
    fn from(e: TicketError) -> Self {
        match e {
            TicketError::NotFound(what) => Self::not_found(what),
            TicketError::Invalid(_) => Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string()),
            TicketError::Cycle(_) => Self::new(StatusCode::CONFLICT, "link_cycle", e.to_string()),
            TicketError::Active(ref key) => Self::new(
                StatusCode::CONFLICT,
                "ticket_active",
                format!("{key} has an active attempt; abandon it or finish the work first"),
            ),
            TicketError::InvalidParent(_) => Self::new(StatusCode::CONFLICT, "invalid_parent", e.to_string()),
            TicketError::PartsOpen { .. } => Self::new(StatusCode::CONFLICT, "parts_open", e.to_string()),
            TicketError::HasParts(_) => Self::new(StatusCode::CONFLICT, "ticket_has_parts", e.to_string()),
            TicketError::Untrusted(_) => Self::new(StatusCode::FORBIDDEN, "untrusted_ticket", e.to_string()),
            TicketError::Store(s) => s.into(),
        }
    }
}

impl From<PtyError> for ApiError {
    fn from(e: PtyError) -> Self {
        match e {
            PtyError::ProgramNotFound(_) => Self::new(StatusCode::BAD_REQUEST, "program_not_found", e.to_string()),
            PtyError::Spawn(_) => Self::new(StatusCode::INTERNAL_SERVER_ERROR, "spawn_failed", e.to_string()),
            PtyError::NotFound(_) | PtyError::NotRunning(_) => {
                Self::new(StatusCode::CONFLICT, "not_running", "this session is not running")
            }
            PtyError::AlreadyExists(_) => Self::new(StatusCode::CONFLICT, "already_running", e.to_string()),
            PtyError::ShuttingDown => {
                Self::new(StatusCode::SERVICE_UNAVAILABLE, "shutting_down", "the daemon is shutting down")
            }
            PtyError::Io(_) => Self::internal(e),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rej: JsonRejection) -> Self {
        Self::new(rej.status(), "invalid_input", rej.body_text())
    }
}

impl From<GitError> for ApiError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::PathNotFound(_) => Self::new(StatusCode::BAD_REQUEST, "path_not_found", e.to_string()),
            GitError::NotARepo(_) => Self::new(StatusCode::BAD_REQUEST, "not_a_git_repo", e.to_string()),
            GitError::GitMissing => Self::new(StatusCode::INTERNAL_SERVER_ERROR, "git_missing", e.to_string()),
            GitError::Command(_) => Self::new(StatusCode::INTERNAL_SERVER_ERROR, "git_error", e.to_string()),
            GitError::InvalidBranch(_) => Self::new(StatusCode::BAD_REQUEST, "invalid_branch", e.to_string()),
            GitError::BranchExists(_) => Self::new(StatusCode::CONFLICT, "branch_exists", e.to_string()),
            GitError::UnknownRef(_) => Self::new(StatusCode::BAD_REQUEST, "unknown_ref", e.to_string()),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(_) => Self::new(StatusCode::CONFLICT, "conflict", "this record already exists"),
            e @ StoreError::Write(_) => Self::write_failed(e.to_string()),
            other => Self::internal(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_command_error_is_500_git_error_with_stderr() {
        let e = ApiError::from(GitError::Command("boom".into()));
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(e.code, "git_error");
        assert!(e.message.contains("boom"));
    }
}
