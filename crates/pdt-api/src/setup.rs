//! Get started (PDT-230): the first-run checklist. Its steps tick themselves off from the host's real state (the
//! checks, the GitHub login, the projects); what the host remembers about it (dismissed, GitHub skipped, the default
//! agent and tickets' permissions confirmed) is stored under its own key in `host_settings`, the first time anyone asks.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use pdt_core::session::AgentKind;
use pdt_core::setup::{SetupFacts, SetupPatch, SetupState, SetupSteps};
use serde::Serialize;
use ts_rs::TS;

const SETUP_KEY: &str = "setup";

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupView {
    pub state: SetupState,
    pub steps: SetupSteps,
    /// How many of the five steps are done.
    pub done: u32,
    /// The app opens on Get started.
    pub opens_on_launch: bool,
    pub default_agent: AgentKind,
}

async fn stored(st: &AppState) -> Result<Option<SetupState>, ApiError> {
    Ok(st.store.get_host_value(SETUP_KEY).await?.and_then(|v| serde_json::from_value(v).ok()))
}

async fn store(st: &AppState, s: &SetupState) -> Result<(), ApiError> {
    st.store.put_host_value(SETUP_KEY, &serde_json::to_value(s).expect("setup state serializes")).await?;
    Ok(())
}

async fn facts(st: &AppState) -> Result<SetupFacts, ApiError> {
    let cached = st.checks.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let c = match cached {
        Some(c) => c,
        None => crate::checks::run_host_checks(st).await,
    };
    // A switched-off agent counts as absent (PDT-270): it isn't one PDT uses. One found but logged out doesn't count
    // either (PDT-556): its first session would stop at once.
    let host = st.store.get_host_settings().await?;
    Ok(SetupFacts {
        git: c.git.found,
        claude: c.claude.usable() && host.agent_on(AgentKind::Claude),
        codex: c.codex.usable() && host.agent_on(AgentKind::Codex),
        devin: c.devin.usable() && host.agent_on(AgentKind::Devin),
        // An ACP agent counts too (PDT-561): a host with only Cursor, Grok or OhMyPi can finish this step.
        acp: host.enabled_agents().into_iter().any(|a| c.acp.get(a.as_str()).is_some_and(|chk| chk.usable())),
        github_connected: crate::forge_login::connected(st, &pdt_core::forge::ForgeKind::GITHUB),
        github_off: !st.integrations.is_enabled("github"),
        projects: st.store.list_projects().await?.len(),
    })
}

/// Records what Settings set that a step asks for (the default agent, tickets' permission mode): that step is then
/// done.
pub(crate) async fn confirm(st: &AppState, p: &SetupPatch) -> Result<(), ApiError> {
    let _guard = st.setup_lock.lock().await;
    let before = stored(st).await?;
    // Never asked for yet: the first look decides whether this host needs setting up; remember the pick then.
    let s = match &before {
        Some(s) => s.clone(),
        None => SetupState::first_seen(&facts(st).await?),
    };
    let next = s.apply(p);
    if before.as_ref() != Some(&next) {
        store(st, &next).await?;
    }
    Ok(())
}

async fn view(st: &AppState, patch: Option<&SetupPatch>) -> Result<SetupView, ApiError> {
    let _guard = st.setup_lock.lock().await;
    let f = facts(st).await?;
    let before = stored(st).await?;
    let mut s = before.clone().unwrap_or_else(|| SetupState::first_seen(&f));
    if let Some(p) = patch {
        s = s.apply(p);
    }
    let steps = SetupSteps::read(&s, &f);
    if steps.all_done() {
        s.finished = true;
    }
    if before.as_ref() != Some(&s) {
        store(st, &s).await?;
    }
    let settings = st.store.get_host_settings().await?;
    Ok(SetupView {
        opens_on_launch: s.opens_on_launch(),
        done: steps.done(),
        steps,
        state: s,
        default_agent: settings.default_agent,
    })
}

pub(crate) async fn get_setup(State(st): State<AppState>) -> Result<Json<SetupView>, ApiError> {
    Ok(Json(view(&st, None).await?))
}

pub(crate) async fn put_setup(
    State(st): State<AppState>,
    payload: Result<Json<SetupPatch>, JsonRejection>,
) -> Result<Json<SetupView>, ApiError> {
    let Json(patch) = payload?;
    Ok(Json(view(&st, Some(&patch)).await?))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/host/setup", get(get_setup).put(put_setup))
}
