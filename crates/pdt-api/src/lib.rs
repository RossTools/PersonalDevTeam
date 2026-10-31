mod activity;
mod attachments;
mod bug_reports;
mod checks;
mod ci;
mod connection_health;
mod conversations;
mod error;
mod files;
mod finish;
mod folders;
mod forge;
pub mod forge_login;
mod github;
mod github_issues;
mod github_pulls;
mod github_workflow_runs;
pub mod guides;
mod hooks;
mod host;
mod host_guard;
mod identity_guard;
mod inbox;
mod integrations;
mod issues;
mod local_forge;
mod local_network;
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
pub mod sdk;
pub mod secrets;
mod sessions;
mod setup;
mod slow;
mod start;
mod stop;
mod ticket_attachments;
mod tickets;
mod todos;
pub mod toolchains;
mod untrusted;
pub mod update;
pub mod vault;
mod workflow;
mod workspaces;
mod worktrees;

pub use activity::run_activity_ticker;
pub use checks::{HostChecks, run_host_checks};
pub use conversations::{
    IDLE_PARK, conversation_live, expire_pending_messages, park_idle_conversations, recover_conversations,
    run_conversation_ticker, shutdown_conversations,
};
pub use error::ApiErrorBody;
pub use github::refresh_prs;
pub use github_issues::refresh_issues;
pub use github_pulls::refresh_pulls;
pub use github_workflow_runs::refresh_workflow_runs;
pub use local_forge::run_local_ci_watch;

/// Stops the MCP servers workflows use (pdtd is stopping).
pub async fn shutdown_mcp(st: &AppState) {
    st.mcp.shutdown().await;
}
pub use host::HostEnv;
pub use host_guard::{AllowedHosts, with_host_guard};
pub use integrations::{
    Integration, Integrations, Registry, load_enabled as load_enabled_integrations, plugin_infos, start_services,
};
pub use local_network::prompt as prompt_local_network;
pub use loops::poll_pass;
pub use notifications::{Notifier, run_notifier, sync as sync_notifications};
pub use overlap::refresh_overlaps;
pub use remote::{RemoteStatus, RemoteWatch, apply_remote_access, run_remote_watch};
pub use routes::{AppState, Health, router};
pub use run_manager::{log_path as run_log_path, resume_runs, sweep_gone_runs};
pub use scheduler::{run_scheduler, schedule_tickets};
pub use sessions::{ResumePlan, plan_resume_interrupted, resume_interrupted};
pub use start::check_merges;
pub use update::{UpdateService, run_update_loop};
pub use workflow::{
    engine::{install_builtins, run_workflows},
    maintain_workflows,
};
