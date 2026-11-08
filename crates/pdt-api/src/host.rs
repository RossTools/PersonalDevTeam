use crate::error::{ApiError, codes};
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::host::{HostSettings, HostSettingsError, HostSettingsPatch, UpdatePolicy};
use pdt_core::network::{NetworkError, NetworkSettings, network_path};
use pdt_core::session::{AgentKind, Platform};
use serde::Serialize;
use ts_rs::TS;

/// What the environment sets over Settings (PDT-231), Network as pdtd started with it, and whether pdt-host
/// supervises this pdtd.
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    /// `PDT_PORT`, when the user set it (pdt-host passing the setting on doesn't count).
    pub port: Option<u16>,
    /// `PDT_ALLOWED_HOSTS`'s hosts.
    pub allowed_hosts: Option<Vec<String>>,
    /// `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM` / `PDT_DEVIN_PROGRAM`.
    pub claude_program: Option<String>,
    pub codex_program: Option<String>,
    pub devin_program: Option<String>,
    /// `PDT_ACP_PROGRAM`: the program every ACP agent runs instead of its own (a stand-in, or a missing path so none
    /// is found).
    pub acp_program: Option<String>,
    /// `network.json` when pdtd started: a setting that differs from it applies after a restart.
    pub network_at_start: NetworkSettings,
    /// pdt-host started this pdtd, so it starts it again when it exits (Restart now).
    pub supervised: bool,
    /// The profile's own port, the one used when neither `PDT_PORT` nor the setting gives one.
    pub default_port: u16,
}

impl HostEnv {
    pub fn program(&self, agent: AgentKind) -> Option<&str> {
        match agent {
            AgentKind::Claude => self.claude_program.as_deref(),
            AgentKind::Codex => self.codex_program.as_deref(),
            AgentKind::Devin => self.devin_program.as_deref(),
            AgentKind::Acp(_) => None,
        }
    }
}

const AGENTS: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Devin];

/// A built-in agent's program in the platform (ACP agents name theirs in their definition).
fn program_slot(p: &mut Platform, agent: AgentKind) -> Option<&mut String> {
    match agent {
        AgentKind::Claude => Some(&mut p.claude_program),
        AgentKind::Codex => Some(&mut p.codex_program),
        AgentKind::Devin => Some(&mut p.devin_program),
        AgentKind::Acp(_) => None,
    }
}

/// `~/…` in a program setting, in the home folder.
pub(crate) fn expand_home(path: &str, home: Option<&std::path::Path>) -> String {
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\'])).to_string_lossy().into_owned()
        }
        _ => path.to_string(),
    }
}

/// `platform` with the agent programs Settings → Agents names, except where the environment named one (pdtd put
/// those in `platform` already).
pub fn with_program_settings(
    platform: &Platform,
    env: &HostEnv,
    s: &HostSettings,
    home: Option<&std::path::Path>,
) -> Platform {
    let mut p = platform.clone();
    for agent in AGENTS {
        if env.program(agent).is_none()
            && let Some(path) = s.program(agent)
            && let Some(slot) = program_slot(&mut p, agent)
        {
            *slot = expand_home(path, home);
        }
    }
    p
}

impl AppState {
    /// The platform agent sessions and checks launch with: each agent's program from the environment, else from
    /// Settings → Agents, else the one on the PATH.
    pub async fn agent_platform(&self) -> Platform {
        match self.store.get_host_settings().await {
            Ok(s) => with_program_settings(&self.platform, &self.host_env, &s, self.home_dir.get().as_deref()),
            Err(e) => {
                tracing::warn!(error = %e, "could not read the host settings; agents launch from the PATH");
                self.platform.clone()
            }
        }
    }
}

/// An agent's program in Settings → Agents.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentProgramView {
    pub agent: AgentKind,
    /// The environment variable that sets it (`PDT_CLAUDE_PROGRAM`), and its value when it is set: it then wins.
    pub env_var: String,
    pub from_env: Option<String>,
    /// Where the program in use was found, or null when it wasn't.
    pub found: Option<String>,
}

/// Settings → Network: the stored settings, what the environment sets over them, and whether a restart is needed.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NetworkView {
    pub settings: NetworkSettings,
    pub port_from_env: Option<u16>,
    pub allowed_hosts_from_env: Option<Vec<String>>,
    /// The port pdtd listens on now.
    pub listen_port: u16,
    /// The port used when no setting or `PDT_PORT` gives one.
    pub default_port: u16,
    /// The settings differ from the ones pdtd started with (where the environment doesn't override them).
    pub restart_needed: bool,
    /// pdt-host supervises pdtd, so Restart now brings it back; otherwise the user restarts it.
    pub can_restart: bool,
}

fn read_network(st: &AppState) -> NetworkSettings {
    std::fs::read(network_path(&st.data_dir)).map(|b| NetworkSettings::parse(&b)).unwrap_or_default()
}

/// Whether `now` (the file) differs from what pdtd started with, in what the environment doesn't override.
pub fn restart_needed(env: &HostEnv, now: &NetworkSettings) -> bool {
    let start = &env.network_at_start;
    (env.port.is_none() && now.port != start.port)
        || (env.allowed_hosts.is_none() && now.allowed_hosts != start.allowed_hosts)
}

fn network_view(st: &AppState) -> NetworkView {
    let settings = read_network(st);
    let env = &st.host_env;
    NetworkView {
        restart_needed: restart_needed(env, &settings),
        settings,
        port_from_env: env.port,
        allowed_hosts_from_env: env.allowed_hosts.clone(),
        listen_port: st.listen_port,
        default_port: env.default_port,
        can_restart: env.supervised,
    }
}

/// Where a program is found on `search` (the login PATH), or None.
pub(crate) fn found(program: &str, search: Option<&str>) -> Option<String> {
    pdt_agents::launch::resolve(program, &[], search).ok().map(|(exe, _)| exe.to_string_lossy().into_owned())
}

/// An ACP agent in Settings → Agents (PDT-234): its definition and where its command was found.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpAgentView {
    pub agent: AgentKind,
    #[serde(flatten)]
    pub def: pdt_core::acp_agent::AcpAgentDef,
    /// Where its command was found, or null when it wasn't.
    pub found: Option<String>,
    /// The version PDT installed, for a profile PDT installs (`installer`); null when it hasn't.
    pub installed_version: Option<String>,
    /// The path set over the installed program (Settings → Agents), for a profile PDT installs.
    pub program: Option<String>,
    /// The sign-in its server keeps, for one PDT can sign in (Antigravity's `auth.type`: `oauth-personal`, …); null
    /// when it has none (or PDT can't tell).
    pub signed_in: Option<String>,
}

/// Every ACP agent this host has, with where each one's command was found.
pub(crate) async fn acp_agent_views(st: &AppState, s: &HostSettings) -> Vec<AcpAgentView> {
    let defs = pdt_core::acp_agent::all(&s.acp_agents);
    let search = st.platform.login_path.clone();
    let home = st.home_dir.get();
    let data_dir = st.data_dir.clone();
    let env_program = st.host_env.acp_program.clone();
    let s = s.clone();
    tokio::task::spawn_blocking(move || {
        defs.into_iter()
            .filter_map(|def| {
                let agent = def.kind()?;
                let installed_version = def
                    .installer
                    .as_deref()
                    .and_then(|id| crate::toolchains::installed(&data_dir, id))
                    .map(|i| i.version);
                let program = def.installer.as_ref().and_then(|_| s.acp_programs.get(&def.id).cloned());
                let signed_in = crate::toolchains::signed_in(def.flavor, home.as_deref());
                let launch =
                    crate::toolchains::launch_def(&data_dir, home.as_deref(), &s, def.clone(), env_program.as_deref());
                let found = found(&expand_home(&launch.command, home.as_deref()), search.as_deref());
                Some(AcpAgentView { agent, def, found, installed_version, program, signed_in })
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

async fn program_views(st: &AppState) -> Vec<AgentProgramView> {
    let platform = st.agent_platform().await;
    let search = st.platform.login_path.clone();
    let env = st.host_env.clone();
    tokio::task::spawn_blocking(move || {
        let mut platform = platform;
        AGENTS
            .iter()
            .filter_map(|&agent| {
                let program = program_slot(&mut platform, agent)?.clone();
                let found = found(&program, search.as_deref());
                let env_var = match agent {
                    AgentKind::Claude => "PDT_CLAUDE_PROGRAM",
                    AgentKind::Codex => "PDT_CODEX_PROGRAM",
                    _ => "PDT_DEVIN_PROGRAM",
                }
                .to_string();
                Some(AgentProgramView { agent, env_var, from_env: env.program(agent).map(String::from), found })
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsView {
    pub settings: HostSettings,
    /// `PDT_OWNER_LOGIN`, which overrides `settings.ownerLogin` when set.
    pub owner_from_env: Option<String>,
    pub token_present: bool,
    /// Why the token could not be read (e.g. the keychain is waiting on an approval prompt); `tokenPresent` is then
    /// false. The rest of the settings still load.
    pub token_error: Option<String>,
    /// The last result of applying the remote-access setting.
    pub remote: crate::remote::RemoteStatus,
    /// Where clones go: `settings.projectsDir`, else `~/work`.
    pub projects_dir: Option<String>,
    /// Claude's, Codex's and Devin's programs (Settings → Agents).
    pub programs: Vec<AgentProgramView>,
    /// The ACP agents: the built-in profiles and the user's.
    pub acp_agents: Vec<AcpAgentView>,
    /// The agents switched on (`settings.disabledAgents` left out): the built-ins, then the ACP agents. What every
    /// agent picker offers (PDT-270).
    pub enabled_agents: Vec<AgentKind>,
    pub network: NetworkView,
}

impl From<HostSettingsError> for ApiError {
    fn from(e: HostSettingsError) -> Self {
        ApiError::invalid(e.to_string())
    }
}

impl From<NetworkError> for ApiError {
    fn from(e: NetworkError) -> Self {
        ApiError::invalid(e.to_string())
    }
}

pub(crate) async fn view(st: &AppState) -> Result<HostSettingsView, ApiError> {
    let (token_present, token_error) = match crate::update::read_token(st).await {
        Ok(t) => (t.is_some(), None),
        Err(e) => (false, Some(e.message().to_string())),
    };
    let settings = st.store.get_host_settings().await?;
    let projects_dir =
        crate::projects::projects_dir(st, settings.projects_dir.as_deref()).map(|p| p.to_string_lossy().into_owned());
    let programs = program_views(st).await;
    let acp_agents = acp_agent_views(st, &settings).await;
    Ok(HostSettingsView {
        enabled_agents: settings.enabled_agents(),
        acp_agents,
        projects_dir,
        settings,
        owner_from_env: st.owner_from_env.clone(),
        token_present,
        token_error,
        remote: st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        programs,
        network: network_view(st),
    })
}

pub(crate) async fn get_settings(State(st): State<AppState>) -> Result<Json<HostSettingsView>, ApiError> {
    Ok(Json(view(&st).await?))
}

pub(crate) async fn put_settings(
    State(st): State<AppState>,
    payload: Result<Json<HostSettingsPatch>, JsonRejection>,
) -> Result<Json<HostSettingsView>, ApiError> {
    let Json(patch) = payload?;
    let guard = st.settings_lock.lock().await;
    let before = st.store.get_host_settings().await?;
    let to_auto =
        before.update_policy != UpdatePolicy::AutoWhenIdle && patch.update_policy == Some(UpdatePolicy::AutoWhenIdle);
    let next = before.apply(&patch)?;
    let effective_owner = st.owner_from_env.clone().or(next.owner_login.clone());
    if next.remote_access && effective_owner.is_none() {
        return Err(ApiError::bad_request(
            codes::OWNER_REQUIRED,
            "set the owner (a Tailscale login) before turning on remote access",
        ));
    }
    st.store.put_host_settings(&next).await?;
    st.refresh_owner().await;
    if patch.remote_access.is_some() {
        crate::remote::apply_locked(&st).await;
    }
    drop(guard);
    // The update already on offer installs when idle now too (PDT-369).
    if to_auto {
        crate::install_updates::wake(&st).await;
    }
    // Get started's steps for the default agent and tickets' permissions: picked here.
    if patch.default_agent.is_some() || patch.ticket_permission_mode.is_some() {
        let confirmed = pdt_core::setup::SetupPatch {
            default_agent_confirmed: patch.default_agent.is_some().then_some(true),
            permissions_confirmed: patch.ticket_permission_mode.is_some().then_some(true),
            ..Default::default()
        };
        crate::setup::confirm(&st, &confirmed).await?;
    }
    // Another agent program: what the checks found (logins, the agents offered) is out of date.
    if patch.claude_program.is_some()
        || patch.codex_program.is_some()
        || patch.devin_program.is_some()
        || patch.acp_agents.is_some()
        || patch.acp_programs.is_some()
    {
        let st = st.clone();
        tokio::spawn(async move {
            crate::checks::run_host_checks(&st).await;
        });
    }
    Ok(Json(view(&st).await?))
}

/// Saves Settings → Network (`network.json`); it applies when pdtd next starts.
pub(crate) async fn put_network(
    State(st): State<AppState>,
    payload: Result<Json<NetworkSettings>, JsonRejection>,
) -> Result<Json<HostSettingsView>, ApiError> {
    let Json(next) = payload?;
    let next = next.validated()?;
    let guard = st.settings_lock.lock().await;
    let runs = st.store.get_host_settings().await?.run_ports;
    if let Some(p) = next.port
        && (runs.start..=runs.end).contains(&p)
    {
        return Err(ApiError::invalid(format!(
            "port {p} is in the runs' port range ({}–{}); pick one outside it",
            runs.start, runs.end
        )));
    }
    let path = network_path(&st.data_dir);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&next).expect("network settings serialize");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| ApiError::io(&format!("saving {}", path.display()), e))?;
    drop(guard);
    tracing::info!(port = ?next.port, hosts = ?next.allowed_hosts, "network settings saved; they apply after a restart");
    Ok(Json(view(&st).await?))
}

/// Restart now (Settings → Network): pdtd exits and pdt-host starts it again, with the new network settings. Only
/// under pdt-host; a pdtd started any other way would just stop.
pub(crate) async fn post_restart(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    if !st.host_env.supervised {
        return Err(ApiError::conflict(
            codes::NOT_SUPERVISED,
            "this PDT wasn't started by its host supervisor, so it can't restart itself: stop it and start it again",
        ));
    }
    tracing::info!("restart requested from Settings");
    st.shutdown.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/host/settings", get(get_settings).put(put_settings))
        .route("/host/network", put(put_network))
        .route("/host/restart", post(post_restart))
}
