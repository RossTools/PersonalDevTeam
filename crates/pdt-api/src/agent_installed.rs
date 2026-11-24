//! Whether an agent's program is on this machine, checked before Start makes a branch and worktree for it and before
//! the PM starts a conversation (PDT-718): a missing agent is refused up front, by name, instead of failing later with
//! its program's path.

use crate::error::{ApiError, codes};
use crate::routes::AppState;
use pdt_core::session::AgentKind;

/// 400 `agent_off` when `agent` is switched off, 409 `agent_not_found` when its program (as a session of it would
/// launch here) can't be found.
pub(crate) async fn ensure(st: &AppState, agent: AgentKind) -> Result<(), ApiError> {
    if let Some(why) = st.store.get_host_settings().await?.agent_off(agent) {
        return Err(ApiError::bad_request(codes::AGENT_OFF, why));
    }
    let program = match agent {
        AgentKind::Acp(_) => crate::conversations::acp_agent(st, agent).await?.command,
        _ => {
            let p = st.agent_platform().await;
            match agent {
                AgentKind::Codex => p.codex_program,
                AgentKind::Devin => p.devin_program,
                _ => p.claude_program,
            }
        }
    };
    // Searching the PATH touches the disk: on the blocking pool.
    let search = st.platform.login_path.clone();
    let found = tokio::task::spawn_blocking(move || crate::host::found(&program, search.as_deref()).is_some())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if found {
        return Ok(());
    }
    let name = crate::agent_login::label(st, agent).await;
    Err(ApiError::conflict(codes::AGENT_NOT_FOUND, not_installed(&name)))
}

/// What the user is told about a missing agent: its name and where to fix it, never a program path.
fn not_installed(name: &str) -> String {
    format!(
        "{name} isn't installed on this machine: install it, or set its path, under Agents on this machine in Get started"
    )
}
