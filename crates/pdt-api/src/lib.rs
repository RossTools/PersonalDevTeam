mod activity;
mod attachments;
mod buildkite;
mod checks;
mod connection_health;
mod conversations;
mod discord;
mod error;
mod files;
mod finish;
mod folders;
mod github;
mod github_issues;
pub mod guides;
mod hooks;
mod host;
mod host_guard;
mod identity_guard;
mod inbox;
mod jira;
pub mod loops;
mod mcp;
mod mcp_client;
mod notifications;
mod overlap;
mod parts;
mod personas;
mod pm;
mod project_settings;
mod projects;
mod pty_ws;
pub mod remote;
mod review;
mod routes;
mod run_manager;
mod runconfig;
mod runs;
mod scheduler;
mod scryer;
pub mod secrets;
mod sessions;
mod slow;
mod start;
mod tickets;
mod todos;
pub mod update;
mod workflow;
mod workspaces;
mod worktrees;

pub use activity::run_activity_ticker;
pub use buildkite::refresh_buildkite;
pub use checks::{HostChecks, run_host_checks};
pub use conversations::{
    IDLE_PARK, expire_pending_messages, park_idle_conversations, recover_conversations, run_conversation_ticker,
    shutdown_conversations,
};
pub use error::ApiErrorBody;
pub use github::refresh_prs;
pub use github_issues::refresh_issues;

/// Stops the MCP servers workflows use (pdtd is stopping).
pub async fn shutdown_mcp(st: &AppState) {
    st.mcp.shutdown().await;
}
pub use host_guard::{AllowedHosts, with_host_guard};
pub use jira::refresh_jira;
pub use loops::poll_pass;
pub use notifications::{Notifier, run_notifier, sync as sync_notifications};
pub use overlap::refresh_overlaps;
pub use remote::{RemoteStatus, apply_remote_access};
pub use routes::{AppState, Health, router};
pub use run_manager::{log_path as run_log_path, resume_runs, sweep_gone_runs};
pub use scheduler::{run_scheduler, schedule_tickets};
pub use sessions::{ResumePlan, plan_resume_interrupted, resume_interrupted};
pub use start::check_merges;
pub use update::{UpdateService, run_update_loop};
pub use workflow::{engine::run_workflows, maintain_workflows};
