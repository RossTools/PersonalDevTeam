use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pdt_agents::PtyError;
use pdt_core::project::ValidationError;
use pdt_core::session::SessionValidationError;
use pdt_core::workspace::WorkspaceValidationError;
use pdt_core::worktree::WorktreeValidationError;
use pdt_git::GitError;
use pdt_store::StoreError;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", format!("{what} not found"))
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ApiErrorBody { code: self.code.into(), message: self.message })).into_response()
    }
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
