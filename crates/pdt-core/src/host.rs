//! Per-host settings. Pure: pdt-store stores them, pdt-api edits them.

use crate::brand::Profile;
use crate::conversation::{PermissionMode, SessionMode};
use crate::session::AgentKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdatePolicy {
    AutoWhenIdle,
    Notify,
    Manual,
}

/// Which releases this host updates to (PDT-210).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateChannel {
    /// Full releases only (GitHub's latest release).
    #[default]
    Stable,
    /// The newest by SemVer of the full releases and the nightly pre-releases (`X.Y.Z-nightly.YYYYMMDD`, PDT-283;
    /// the older experimental `X.Y.Z-exp.N` too), which are built from main and may carry migrations no stable
    /// release has yet. Read from `experimental`, the channel's old name, too: a host set to it stays on it.
    #[serde(alias = "experimental")]
    Nightly,
}

/// When updates may install by themselves (PDT-369): between two local times, `from` after `to` wrapping midnight
/// (02:00–06:00, or 23:00–05:00). Equal ends: any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallWindow {
    pub from: u16,
    pub to: u16,
}

impl InstallWindow {
    pub fn parse(from: &str, to: &str) -> Result<Self, String> {
        let at =
            |s: &str| crate::notification::parse_clock(s).ok_or_else(|| format!("{s:?} is not a time of day (HH:MM)"));
        Ok(Self { from: at(from)?, to: at(to)? })
    }

    /// Whether `minute` (since local midnight) is in it.
    pub fn contains(self, minute: u16) -> bool {
        match self.from.cmp(&self.to) {
            std::cmp::Ordering::Less => (self.from..self.to).contains(&minute),
            std::cmp::Ordering::Greater => minute >= self.from || minute < self.to,
            std::cmp::Ordering::Equal => true,
        }
    }
}

/// Whether agent sessions get the host's API-key variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialPolicy {
    /// ANTHROPIC_API_KEY and OPENAI_API_KEY are removed, so agents use the logged-in subscription.
    Subscription,
    /// Agents get the host's environment unchanged (API keys, Bedrock/Vertex variables, proxies).
    PassThrough,
}

impl CredentialPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialPolicy::Subscription => "subscription",
            CredentialPolicy::PassThrough => "pass_through",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [CredentialPolicy::Subscription, CredentialPolicy::PassThrough].into_iter().find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

pub const DEFAULT_RUN_PORTS: PortRange = PortRange { start: 8400, end: 8499 };
/// The dev instance's default, apart from the stable one's so their runs never compete for ports or mappings.
pub const DEV_RUN_PORTS: PortRange = PortRange { start: 8500, end: 8599 };
pub const DEFAULT_MAX_RUNS: u32 = 4;
/// How many agents may run a turn at once for PDT's own work; 0 is no cap.
pub const DEFAULT_MAX_AGENT_TURNS: u32 = 4;
const MAX_AGENT_TURNS_LIMIT: u32 = 64;
/// How many triage runs one author's issues start an hour (PDT-202); 0 is no cap.
pub const DEFAULT_TRIAGE_PER_AUTHOR: u32 = 3;
/// How many triage runs one repository's issues (or one webhook) start an hour; 0 is no cap.
pub const DEFAULT_TRIAGE_PER_REPO: u32 = 20;
const TRIAGE_LIMIT: u32 = 1000;
const MAX_RUNS_LIMIT: u32 = 32;
const MAX_PORTS_IN_RANGE: u32 = 1000;
pub const DEFAULT_UPDATE_CHECK_MINUTES: u32 = 60;
/// How often update checks may run: every 5 minutes at most, once a week at least.
pub const UPDATE_CHECK_MINUTES: std::ops::RangeInclusive<u32> = 5..=7 * 24 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct HostSettings {
    pub remote_access: bool,
    /// The Tailscale login allowed to use this host remotely. `PDT_OWNER_LOGIN` overrides it.
    pub owner_login: Option<String>,
    pub update_policy: UpdatePolicy,
    /// Stable or nightly releases. Never a downgrade: back on stable, a host stays on its nightly build
    /// until a newer stable release exists.
    pub update_channel: UpdateChannel,
    pub run_ports: PortRange,
    pub max_concurrent_runs: u32,
    pub agent_credentials: CredentialPolicy,
    /// Where "clone by URL" puts new repositories by default; `~/work` when unset.
    pub projects_dir: Option<String>,
    /// The agent the app offers first (ticket Start, the new-tab chooser, the palette, "… with an agent").
    pub default_agent: AgentKind,
    /// Whether a new agent session is a conversation or the agent's terminal by default.
    pub default_session_mode: SessionMode,
    /// The permission mode a ticket's Start offers first (bypass: the agent works on its own branch, PDT-75).
    pub ticket_permission_mode: PermissionMode,
    /// The permission mode a new session offers first (the new-tab chooser).
    pub session_permission_mode: PermissionMode,
    /// Minutes between automatic update checks (unless the update policy is manual).
    pub update_check_minutes: u32,
    /// The most agents running a turn at once before PDT's own work (agent steps, workflow messages) waits for a
    /// slot; 0: no cap. What the user does is never held.
    pub max_agent_turns: u32,
    /// The most triage runs (a workflow with an agent step started by a new issue or a webhook) one issue author
    /// starts an hour; more are skipped (PDT-202). 0: no cap.
    pub triage_per_author_hour: u32,
    /// The same per repository (or webhook).
    pub triage_per_repo_hour: u32,
    /// Where "Report a bug" sends reports (PDT-153).
    pub bug_reports: crate::bug_report::BugReportSetting,
    /// The GitHub repository (`owner/name`) to take updates from and file bug reports on instead of the built-in one:
    /// an emergency override for a move redirects cannot cover (PDT-195). `PDT_RELEASE_REPO` overrides it.
    pub release_repo: Option<String>,
    /// The agent programs to launch instead of the ones found on the PATH (a full path, or `~/…`). The matching
    /// `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM` / `PDT_DEVIN_PROGRAM` overrides each (PDT-231).
    pub claude_program: Option<String>,
    pub codex_program: Option<String>,
    pub devin_program: Option<String>,
    /// The ACP agents the user added (Settings → Agents → Add ACP agent; PDT-234), agent kinds `acp:<id>`.
    pub acp_agents: Vec<crate::acp_agent::AcpAgentConfig>,
    /// The program to launch for a built-in ACP profile PDT installs (by profile id, PDT-239), instead of the one PDT
    /// installed: one installed by hand (a full path, or `~/…`).
    #[ts(type = "Record<string, string>")]
    pub acp_programs: std::collections::BTreeMap<String, String>,
    /// The agents switched off in Settings → Agents (PDT-270): never offered, and no new session starts on them.
    /// Every agent is on unless it is here. The default agent never is.
    pub disabled_agents: Vec<AgentKind>,
    /// A default model for each agent (Settings → Agents, PDT-265). A session with no model of its own (nor its
    /// persona's) starts with it; an agent without one uses the agent's own default (`HostSettings::model_for`).
    #[ts(type = "Partial<Record<AgentKind, string>>")]
    pub agent_models: std::collections::BTreeMap<AgentKind, String>,
    /// The provider new documents go to (PDT-314) unless their project names one; none: the first PDT has.
    pub documents_provider: Option<String>,
    /// Where local documents go (a full path, or `~/…`) unless their project names a folder; none: `documents` in
    /// PDT's data folder.
    pub documents_dir: Option<String>,
    /// Settings → MCP servers → "Use only PDT's MCP servers" (PDT-322): sessions ignore the agents' own MCP configs
    /// where the agent can.
    pub mcp_only_pdt: bool,
    /// The low-disk threshold (GB; 0: off) as set before it was Clean up storage's input (PDT-370): read once, when
    /// that workflow is installed.
    pub low_disk_gb: u32,
    /// The same for the critical threshold.
    pub critical_disk_gb: u32,
}

/// A GitHub `owner/name`: two non-empty parts of letters, digits, `-`, `_` and `.`, neither `.` nor `..`.
pub fn is_repo_name(s: &str) -> bool {
    let ok = |p: &str| {
        !p.is_empty() && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    matches!(s.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            remote_access: false,
            owner_login: None,
            update_policy: UpdatePolicy::Notify,
            update_channel: UpdateChannel::Stable,
            run_ports: DEFAULT_RUN_PORTS,
            max_concurrent_runs: DEFAULT_MAX_RUNS,
            agent_credentials: CredentialPolicy::Subscription,
            projects_dir: None,
            default_agent: AgentKind::Claude,
            default_session_mode: SessionMode::Conversation,
            ticket_permission_mode: PermissionMode::Bypass,
            session_permission_mode: PermissionMode::Ask,
            update_check_minutes: DEFAULT_UPDATE_CHECK_MINUTES,
            max_agent_turns: DEFAULT_MAX_AGENT_TURNS,
            triage_per_author_hour: DEFAULT_TRIAGE_PER_AUTHOR,
            triage_per_repo_hour: DEFAULT_TRIAGE_PER_REPO,
            bug_reports: Default::default(),
            release_repo: None,
            claude_program: None,
            codex_program: None,
            devin_program: None,
            acp_agents: Vec::new(),
            acp_programs: Default::default(),
            disabled_agents: Vec::new(),
            agent_models: Default::default(),
            documents_provider: None,
            documents_dir: None,
            mcp_only_pdt: false,
            low_disk_gb: crate::storage::DEFAULT_LOW_DISK_GB,
            critical_disk_gb: crate::storage::DEFAULT_CRITICAL_DISK_GB,
        }
    }
}

/// A partial update. `owner_login: Some("")` (or blank) clears the owner.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsPatch {
    #[serde(default)]
    #[ts(optional)]
    pub remote_access: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub owner_login: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub update_policy: Option<UpdatePolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub update_channel: Option<UpdateChannel>,
    #[serde(default)]
    #[ts(optional)]
    pub run_ports: Option<PortRange>,
    #[serde(default)]
    #[ts(optional)]
    pub max_concurrent_runs: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub agent_credentials: Option<CredentialPolicy>,
    /// Blank clears it (back to `~/work`).
    #[serde(default)]
    #[ts(optional)]
    pub projects_dir: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub default_agent: Option<AgentKind>,
    #[serde(default)]
    #[ts(optional)]
    pub default_session_mode: Option<SessionMode>,
    #[serde(default)]
    #[ts(optional)]
    pub ticket_permission_mode: Option<PermissionMode>,
    #[serde(default)]
    #[ts(optional)]
    pub session_permission_mode: Option<PermissionMode>,
    #[serde(default)]
    #[ts(optional)]
    pub update_check_minutes: Option<u32>,
    /// 0 for no cap.
    #[serde(default)]
    #[ts(optional)]
    pub max_agent_turns: Option<u32>,
    /// 0 for no cap.
    #[serde(default)]
    #[ts(optional)]
    pub triage_per_author_hour: Option<u32>,
    /// 0 for no cap.
    #[serde(default)]
    #[ts(optional)]
    pub triage_per_repo_hour: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub bug_reports: Option<crate::bug_report::BugReportSetting>,
    /// Blank clears it (back to the built-in release repository).
    #[serde(default)]
    #[ts(optional)]
    pub release_repo: Option<String>,
    /// Blank clears it (back to the one on the PATH).
    #[serde(default)]
    #[ts(optional)]
    pub claude_program: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub codex_program: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub devin_program: Option<String>,
    /// The whole list of the user's ACP agents.
    #[serde(default)]
    #[ts(optional)]
    pub acp_agents: Option<Vec<crate::acp_agent::AcpAgentConfig>>,
    /// Programs for the built-in ACP profiles it names (by id); the others keep theirs. Blank clears one (back to the
    /// one PDT installs).
    #[serde(default)]
    #[ts(optional, type = "Record<string, string>")]
    pub acp_programs: Option<std::collections::BTreeMap<String, String>>,
    /// The whole list of the agents switched off.
    #[serde(default)]
    #[ts(optional)]
    pub disabled_agents: Option<Vec<AgentKind>>,
    /// Default models for the agents it names; the others keep theirs. Blank clears one (back to the agent's own).
    #[serde(default)]
    #[ts(optional, type = "Partial<Record<AgentKind, string>>")]
    pub agent_models: Option<std::collections::BTreeMap<AgentKind, String>>,
    /// Blank clears it (back to the first provider PDT has).
    #[serde(default)]
    #[ts(optional)]
    pub documents_provider: Option<String>,
    /// Blank clears it (back to PDT's data folder).
    #[serde(default)]
    #[ts(optional)]
    pub documents_dir: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub mcp_only_pdt: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HostSettingsError {
    #[error("the run port range must lie within 1024–65535, start at or below its end, and hold at most 1000 ports")]
    BadPortRange,
    #[error("the maximum number of concurrent runs must be between 1 and 32")]
    BadMaxRuns,
    #[error("the owner login must not contain spaces or control characters")]
    BadOwner,
    #[error("the projects folder must be a full path (or start with ~ for your home folder)")]
    BadProjectsDir,
    #[error("update checks must run between every 5 minutes and once a week")]
    BadUpdateInterval,
    #[error("the most agents running at once must be between 0 (no cap) and 64")]
    BadMaxAgentTurns,
    #[error("the triage caps must be between 0 (no cap) and 1000 runs an hour")]
    BadTriageCap,
    #[error("the release repository must be a GitHub owner/name, e.g. RossTools/PersonalDevTeam")]
    BadReleaseRepo,
    #[error("an agent program must be a full path (or start with ~ for your home folder)")]
    BadProgram,
    #[error("a model name must be 1 to 100 characters with no spaces")]
    BadModel,
    #[error("that agent isn't one this host has")]
    UnknownAgent,
    #[error("{0} is the default agent: pick another default before switching it off")]
    DefaultAgentOff(String),
    #[error("that isn't a document provider's id")]
    BadDocumentsProvider,
    #[error("the documents folder must be a full path (or start with ~ for your home folder)")]
    BadDocumentsDir,
    #[error("{0}")]
    BadAcpAgent(#[from] crate::acp_agent::AcpAgentError),
}

impl HostSettings {
    /// The settings a host starts with before anything is stored: the defaults, with the dev profile's own run
    /// port range.
    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Stable => Self::default(),
            Profile::Dev => Self { run_ports: DEV_RUN_PORTS, ..Self::default() },
        }
    }

    pub fn apply(&self, p: &HostSettingsPatch) -> Result<HostSettings, HostSettingsError> {
        let mut s = self.clone();
        if let Some(v) = p.remote_access {
            s.remote_access = v;
        }
        if let Some(raw) = &p.owner_login {
            let login = raw.trim();
            if login.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(HostSettingsError::BadOwner);
            }
            s.owner_login = (!login.is_empty()).then(|| login.to_string());
        }
        if let Some(v) = p.update_policy {
            s.update_policy = v;
        }
        if let Some(v) = p.update_channel {
            s.update_channel = v;
        }
        if let Some(r) = p.run_ports {
            let size = u32::from(r.end).saturating_sub(u32::from(r.start)) + 1;
            if r.start < 1024 || r.start > r.end || size > MAX_PORTS_IN_RANGE {
                return Err(HostSettingsError::BadPortRange);
            }
            s.run_ports = r;
        }
        if let Some(n) = p.max_concurrent_runs {
            if !(1..=MAX_RUNS_LIMIT).contains(&n) {
                return Err(HostSettingsError::BadMaxRuns);
            }
            s.max_concurrent_runs = n;
        }
        if let Some(c) = p.agent_credentials {
            s.agent_credentials = c;
        }
        if let Some(d) = &p.projects_dir {
            let d = d.trim();
            if !d.is_empty() && !d.starts_with('~') && !std::path::Path::new(d).is_absolute() {
                return Err(HostSettingsError::BadProjectsDir);
            }
            s.projects_dir = (!d.is_empty()).then(|| d.to_string());
        }
        if let Some(a) = p.default_agent {
            s.default_agent = a;
        }
        if let Some(m) = p.default_session_mode {
            s.default_session_mode = m;
        }
        if let Some(m) = p.ticket_permission_mode {
            s.ticket_permission_mode = m;
        }
        if let Some(m) = p.session_permission_mode {
            s.session_permission_mode = m;
        }
        if let Some(n) = p.update_check_minutes {
            if !UPDATE_CHECK_MINUTES.contains(&n) {
                return Err(HostSettingsError::BadUpdateInterval);
            }
            s.update_check_minutes = n;
        }
        if let Some(n) = p.max_agent_turns {
            if n > MAX_AGENT_TURNS_LIMIT {
                return Err(HostSettingsError::BadMaxAgentTurns);
            }
            s.max_agent_turns = n;
        }
        for (given, field) in [
            (p.triage_per_author_hour, &mut s.triage_per_author_hour),
            (p.triage_per_repo_hour, &mut s.triage_per_repo_hour),
        ] {
            if let Some(n) = given {
                if n > TRIAGE_LIMIT {
                    return Err(HostSettingsError::BadTriageCap);
                }
                *field = n;
            }
        }
        if let Some(b) = p.bug_reports {
            s.bug_reports = b;
        }
        if let Some(b) = p.mcp_only_pdt {
            s.mcp_only_pdt = b;
        }
        if let Some(raw) = &p.documents_provider {
            let id = raw.trim();
            if !id.is_empty() && !crate::document::is_provider_id(id) {
                return Err(HostSettingsError::BadDocumentsProvider);
            }
            s.documents_provider = (!id.is_empty()).then(|| id.to_string());
        }
        if let Some(raw) = &p.documents_dir {
            s.documents_dir =
                crate::document::clean_folder(Some(raw)).map_err(|_| HostSettingsError::BadDocumentsDir)?;
        }
        if let Some(r) = &p.release_repo {
            let r = r.trim();
            if !r.is_empty() && !is_repo_name(r) {
                return Err(HostSettingsError::BadReleaseRepo);
            }
            s.release_repo = (!r.is_empty()).then(|| r.to_string());
        }
        for (given, field) in [
            (&p.claude_program, &mut s.claude_program),
            (&p.codex_program, &mut s.codex_program),
            (&p.devin_program, &mut s.devin_program),
        ] {
            if let Some(raw) = given {
                *field = program_path(raw)?;
            }
        }
        for (id, raw) in p.acp_programs.iter().flatten() {
            if !crate::acp_agent::builtin_profiles().iter().any(|d| d.id == *id && d.installer.is_some()) {
                return Err(HostSettingsError::UnknownAgent);
            }
            match program_path(raw)? {
                Some(path) => s.acp_programs.insert(id.clone(), path),
                None => s.acp_programs.remove(id),
            };
        }
        if let Some(agents) = &p.acp_agents {
            s.acp_agents = crate::acp_agent::validate(agents)?;
        }
        // The default agent is one the host has: naming a missing ACP agent fails, and removing the default one goes
        // back to the first built-in agent that is on (Claude, usually).
        if s.default_agent.is_acp() && s.acp_agent(s.default_agent).is_none() {
            if p.default_agent.is_some() {
                return Err(HostSettingsError::UnknownAgent);
            }
            let off = p.disabled_agents.as_ref().unwrap_or(&s.disabled_agents);
            s.default_agent = AgentKind::ALL.into_iter().find(|a| !off.contains(a)).unwrap_or(AgentKind::Claude);
        }
        if let Some(off) = &p.disabled_agents {
            let mut list: Vec<AgentKind> = vec![];
            for &a in off {
                if a.is_acp() && s.acp_agent(a).is_none() {
                    return Err(HostSettingsError::UnknownAgent);
                }
                if !list.contains(&a) {
                    list.push(a);
                }
            }
            s.disabled_agents = list;
        }
        // A removed ACP agent's switch goes with it.
        let acp = s.acp_agents.clone();
        s.disabled_agents.retain(|&a| !a.is_acp() || crate::acp_agent::find(&acp, a.acp_id().unwrap_or("")).is_some());
        if s.disabled_agents.contains(&s.default_agent) {
            return Err(HostSettingsError::DefaultAgentOff(s.agent_label(s.default_agent)));
        }
        if let Some(models) = &p.agent_models {
            for (&agent, model) in models {
                let model = model.trim();
                if agent.is_acp() && s.acp_agent(agent).is_none() {
                    return Err(HostSettingsError::UnknownAgent);
                }
                if model.is_empty() {
                    s.agent_models.remove(&agent);
                } else if crate::conversation::valid_model(model) {
                    s.agent_models.insert(agent, model.to_string());
                } else {
                    return Err(HostSettingsError::BadModel);
                }
            }
        }
        // A removed ACP agent's default model goes with it.
        let gone: Vec<AgentKind> =
            s.agent_models.keys().filter(|&&a| a.is_acp() && s.acp_agent(a).is_none()).copied().collect();
        for a in gone {
            s.agent_models.remove(&a);
        }
        Ok(s)
    }
}

/// A program setting: a full path or one starting with `~`, trimmed; blank is none.
fn program_path(raw: &str) -> Result<Option<String>, HostSettingsError> {
    let path = raw.trim();
    if !path.is_empty() && !path.starts_with('~') && !std::path::Path::new(path).is_absolute() {
        return Err(HostSettingsError::BadProgram);
    }
    Ok((!path.is_empty()).then(|| path.to_string()))
}

/// The project's own choice when it has one, else the host's.
pub fn effective_credentials(host: CredentialPolicy, project: Option<CredentialPolicy>) -> CredentialPolicy {
    project.unwrap_or(host)
}

impl HostSettings {
    /// The agent program the host setting names, if any.
    pub fn program(&self, agent: AgentKind) -> Option<&str> {
        match agent {
            AgentKind::Claude => self.claude_program.as_deref(),
            AgentKind::Codex => self.codex_program.as_deref(),
            AgentKind::Devin => self.devin_program.as_deref(),
            // Its command is in its definition (`acp_agent`).
            AgentKind::Acp(_) => None,
        }
    }

    /// Whether the agent is switched on (Settings → Agents, PDT-270).
    pub fn agent_on(&self, agent: AgentKind) -> bool {
        !self.disabled_agents.contains(&agent)
    }

    /// Why a new session can't start on `agent` when it is switched off: "Codex is switched off in Settings → Agents".
    pub fn agent_off(&self, agent: AgentKind) -> Option<String> {
        (!self.agent_on(agent)).then(|| format!("{} is switched off in Settings → Agents", self.agent_label(agent)))
    }

    /// Every agent this host has and has switched on: the built-ins, then the ACP agents (built-in profiles, then the
    /// user's).
    pub fn enabled_agents(&self) -> Vec<AgentKind> {
        let acp = crate::acp_agent::all(&self.acp_agents).into_iter().filter_map(|d| d.kind());
        AgentKind::ALL.into_iter().chain(acp).filter(|&a| self.agent_on(a)).collect()
    }

    /// The agent's name: a built-in's, or an ACP agent's label (its id when this host has no such agent).
    pub fn agent_label(&self, agent: AgentKind) -> String {
        match self.acp_agent(agent) {
            Some(d) => d.label,
            None => agent.label().to_string(),
        }
    }

    /// The model a session of `agent` starts with: its own (`chosen`: the user's pick, or its persona's), else the
    /// host's default for the agent, else None (the agent's own default).
    pub fn model_for(&self, agent: Option<AgentKind>, chosen: Option<&str>) -> Option<String> {
        chosen.map(String::from).or_else(|| agent.and_then(|a| self.agent_models.get(&a).cloned()))
    }

    /// The ACP agent an agent kind names, when it is one this host has (a built-in profile or the user's).
    pub fn acp_agent(&self, agent: AgentKind) -> Option<crate::acp_agent::AcpAgentDef> {
        crate::acp_agent::find(&self.acp_agents, agent.acp_id()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch() -> HostSettingsPatch {
        HostSettingsPatch::default()
    }

    #[test]
    fn install_windows_wrap_midnight_and_equal_ends_are_any_time() {
        let night = InstallWindow::parse("02:00", "06:00").unwrap();
        assert!(night.contains(120) && night.contains(359));
        assert!(!night.contains(360) && !night.contains(119) && !night.contains(23 * 60));
        let late = InstallWindow::parse("23:00", "05:00").unwrap();
        assert!(late.contains(23 * 60) && late.contains(0) && late.contains(299));
        assert!(!late.contains(300) && !late.contains(12 * 60));
        assert!(InstallWindow::parse("04:00", "04:00").unwrap().contains(17 * 60));
        assert!(InstallWindow::parse("2am", "06:00").is_err());
    }

    #[test]
    fn defaults_match_the_spec() {
        let s = HostSettings::default();
        assert!(!s.remote_access);
        assert_eq!(s.owner_login, None);
        assert_eq!(s.update_policy, UpdatePolicy::Notify);
        assert_eq!(s.run_ports, PortRange { start: 8400, end: 8499 });
        assert_eq!(s.max_concurrent_runs, 4);
        assert_eq!(s.agent_credentials, CredentialPolicy::Subscription);
        assert_eq!(s.default_agent, AgentKind::Claude);
        assert_eq!(s.default_session_mode, SessionMode::Conversation);
        assert_eq!(
            (s.ticket_permission_mode, s.session_permission_mode),
            (PermissionMode::Bypass, PermissionMode::Ask)
        );
        assert_eq!(s.update_check_minutes, 60);
    }

    /// Settings stored by an older version (without the newer fields) read back with their defaults.
    #[test]
    fn stored_settings_without_the_defaults_read_back() {
        let s: HostSettings = serde_json::from_str(r#"{"remoteAccess":true,"maxConcurrentRuns":2}"#).unwrap();
        assert_eq!(s.default_agent, AgentKind::Claude);
        assert_eq!(s.default_session_mode, SessionMode::Conversation);
        assert_eq!(s.ticket_permission_mode, PermissionMode::Bypass);
        assert_eq!(s.update_check_minutes, DEFAULT_UPDATE_CHECK_MINUTES);
    }

    #[test]
    fn default_agent_view_and_update_interval_are_set_and_checked() {
        let p: HostSettingsPatch = serde_json::from_str(
            r#"{"defaultAgent":"codex","defaultSessionMode":"terminal","updateCheckMinutes":360,
                "ticketPermissionMode":"accept_edits","sessionPermissionMode":"plan"}"#,
        )
        .unwrap();
        let s = HostSettings::default().apply(&p).unwrap();
        assert_eq!(
            (s.ticket_permission_mode, s.session_permission_mode),
            (PermissionMode::AcceptEdits, PermissionMode::Plan)
        );
        assert_eq!(s.default_agent, AgentKind::Codex);
        assert_eq!(s.default_session_mode, SessionMode::Terminal);
        assert_eq!(s.update_check_minutes, 360);
        let minutes = |n| HostSettingsPatch { update_check_minutes: Some(n), ..patch() };
        assert_eq!(s.apply(&minutes(4)), Err(HostSettingsError::BadUpdateInterval));
        assert_eq!(s.apply(&minutes(7 * 24 * 60 + 1)), Err(HostSettingsError::BadUpdateInterval));
        assert_eq!(s.apply(&minutes(5)).unwrap().update_check_minutes, 5);
        assert_eq!(s.apply(&patch()).unwrap(), s, "an empty patch keeps them");
    }

    #[test]
    fn the_release_repository_override_is_an_owner_and_name() {
        let set =
            |r: &str| HostSettings::default().apply(&HostSettingsPatch { release_repo: Some(r.into()), ..patch() });
        assert_eq!(set(" Acme/pdt.fork ").unwrap().release_repo.as_deref(), Some("Acme/pdt.fork"));
        assert_eq!(set("").unwrap().release_repo, None, "blank clears it");
        for bad in ["acme", "acme/", "/pdt", "a/b/c", "a b/c", "../x", "https://github.com/a/b"] {
            assert_eq!(set(bad), Err(HostSettingsError::BadReleaseRepo), "{bad}");
        }
        assert!(is_repo_name(crate::brand::RELEASE_REPO));
    }

    /// The dev instance's runs default to their own port range, so they never collide with (or take over the
    /// Tailscale mappings of) the stable instance's runs on the same machine.
    #[test]
    fn dev_defaults_use_their_own_run_ports() {
        assert_eq!(HostSettings::for_profile(Profile::Stable), HostSettings::default());
        let dev = HostSettings::for_profile(Profile::Dev);
        assert_eq!(dev.run_ports, PortRange { start: 8500, end: 8599 });
        assert_eq!(HostSettings { run_ports: DEFAULT_RUN_PORTS, ..dev }, HostSettings::default(), "only the ports");
    }

    #[test]
    fn missing_fields_take_defaults_when_reading_old_json() {
        let s: HostSettings = serde_json::from_str(r#"{"remoteAccess":true}"#).unwrap();
        assert!(s.remote_access);
        assert_eq!(s.max_concurrent_runs, 4);
        assert_eq!(s.update_channel, UpdateChannel::Stable, "hosts stay on stable");
    }

    /// PDT-283: the experimental channel became nightly; a host saved on it reads as nightly and is saved so.
    #[test]
    fn the_experimental_channel_reads_as_nightly() {
        let s: HostSettings = serde_json::from_str(r#"{"updateChannel":"experimental"}"#).unwrap();
        assert_eq!(s.update_channel, UpdateChannel::Nightly);
        assert!(serde_json::to_string(&s).unwrap().contains(r#""updateChannel":"nightly""#));
        let p: HostSettingsPatch = serde_json::from_str(r#"{"updateChannel":"experimental"}"#).unwrap();
        assert_eq!(p.update_channel, Some(UpdateChannel::Nightly));
    }

    #[test]
    fn wire_names() {
        assert_eq!(serde_json::to_string(&UpdatePolicy::AutoWhenIdle).unwrap(), r#""auto_when_idle""#);
        assert_eq!(serde_json::to_string(&UpdateChannel::Nightly).unwrap(), r#""nightly""#);
        assert_eq!(serde_json::to_string(&CredentialPolicy::PassThrough).unwrap(), r#""pass_through""#);
        for c in [CredentialPolicy::Subscription, CredentialPolicy::PassThrough] {
            assert_eq!(CredentialPolicy::parse(c.as_str()), Some(c));
        }
        assert_eq!(CredentialPolicy::parse("api"), None);
    }

    #[test]
    fn patch_applies_only_given_fields() {
        let s = HostSettings::default();
        let p = HostSettingsPatch {
            update_policy: Some(UpdatePolicy::Manual),
            owner_login: Some("  me@example.com ".into()),
            ..patch()
        };
        let out = s.apply(&p).unwrap();
        assert_eq!(out.update_policy, UpdatePolicy::Manual);
        assert_eq!(out.owner_login.as_deref(), Some("me@example.com"));
        assert_eq!(out.run_ports, s.run_ports);
        let cleared = out.apply(&HostSettingsPatch { owner_login: Some("  ".into()), ..patch() }).unwrap();
        assert_eq!(cleared.owner_login, None);
    }

    #[test]
    fn patch_validation() {
        let s = HostSettings::default();
        let ports = |start, end| HostSettingsPatch { run_ports: Some(PortRange { start, end }), ..patch() };
        assert_eq!(s.apply(&ports(9000, 8999)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(80, 90)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(10000, 11000)), Err(HostSettingsError::BadPortRange), "1001 ports");
        assert!(s.apply(&ports(10000, 10999)).is_ok());
        let runs = |n| HostSettingsPatch { max_concurrent_runs: Some(n), ..patch() };
        assert_eq!(s.apply(&runs(0)), Err(HostSettingsError::BadMaxRuns));
        assert_eq!(s.apply(&runs(33)), Err(HostSettingsError::BadMaxRuns));
        assert!(s.apply(&runs(32)).is_ok());
        let turns = |n| HostSettingsPatch { max_agent_turns: Some(n), ..patch() };
        assert_eq!(s.apply(&turns(0)).unwrap().max_agent_turns, 0, "no cap");
        assert_eq!(s.apply(&turns(65)), Err(HostSettingsError::BadMaxAgentTurns));
        assert_eq!(s.max_agent_turns, 4);
        let triage = HostSettingsPatch { triage_per_author_hour: Some(0), triage_per_repo_hour: Some(50), ..patch() };
        let t = s.apply(&triage).unwrap();
        assert_eq!((t.triage_per_author_hour, t.triage_per_repo_hour), (0, 50));
        let over = HostSettingsPatch { triage_per_repo_hour: Some(1001), ..patch() };
        assert_eq!(s.apply(&over), Err(HostSettingsError::BadTriageCap));
        assert_eq!((s.triage_per_author_hour, s.triage_per_repo_hour), (3, 20));
        let owner = |o: &str| HostSettingsPatch { owner_login: Some(o.into()), ..patch() };
        assert_eq!(s.apply(&owner("two words")), Err(HostSettingsError::BadOwner));
        assert_eq!(s.apply(&owner("a\u{7}b")), Err(HostSettingsError::BadOwner));
    }

    #[test]
    fn project_override_wins() {
        use CredentialPolicy::*;
        assert_eq!(effective_credentials(Subscription, None), Subscription);
        assert_eq!(effective_credentials(Subscription, Some(PassThrough)), PassThrough);
        assert_eq!(effective_credentials(PassThrough, Some(Subscription)), Subscription);
    }

    #[test]
    fn acp_agents_are_checked_and_the_default_agent_stays_one_the_host_has() {
        use crate::acp_agent::AcpAgentConfig;
        let agent = |id: &str| AcpAgentConfig {
            id: id.into(),
            label: "Fake".into(),
            command: "fake-acp".into(),
            args: vec![],
            env: vec![],
        };
        let kind = AgentKind::parse("acp:fake").unwrap();
        let unknown = HostSettingsPatch { default_agent: Some(kind), ..patch() };
        assert_eq!(HostSettings::default().apply(&unknown).unwrap_err(), HostSettingsError::UnknownAgent);
        let added = HostSettings::default()
            .apply(&HostSettingsPatch { acp_agents: Some(vec![agent("fake")]), default_agent: Some(kind), ..patch() })
            .unwrap();
        assert_eq!(added.default_agent, kind);
        assert_eq!(added.acp_agent(kind).unwrap().command, "fake-acp");
        let removed = added.apply(&HostSettingsPatch { acp_agents: Some(vec![]), ..patch() }).unwrap();
        assert_eq!(removed.default_agent, AgentKind::Claude);
        let bad = HostSettingsPatch { acp_agents: Some(vec![agent("Bad id")]), ..patch() };
        assert!(matches!(HostSettings::default().apply(&bad), Err(HostSettingsError::BadAcpAgent(_))));
    }

    #[test]
    fn agents_switch_off_and_on_but_never_the_default() {
        let s = HostSettings::default();
        assert_eq!(s.enabled_agents()[..3], AgentKind::ALL, "every agent starts on");
        let off = |a: Vec<AgentKind>| HostSettingsPatch { disabled_agents: Some(a), ..patch() };
        let codex_off = s.apply(&off(vec![AgentKind::Codex, AgentKind::Codex])).unwrap();
        assert_eq!(codex_off.disabled_agents, [AgentKind::Codex], "once");
        assert!(!codex_off.agent_on(AgentKind::Codex) && codex_off.agent_on(AgentKind::Claude));
        assert!(!codex_off.enabled_agents().contains(&AgentKind::Codex));
        assert_eq!(
            codex_off.agent_off(AgentKind::Codex).as_deref(),
            Some("Codex is switched off in Settings → Agents")
        );
        assert_eq!(codex_off.agent_off(AgentKind::Claude), None);
        assert_eq!(
            s.apply(&off(vec![AgentKind::Claude])),
            Err(HostSettingsError::DefaultAgentOff("Claude".into())),
            "the default agent stays on"
        );
        assert_eq!(
            codex_off.apply(&HostSettingsPatch { default_agent: Some(AgentKind::Codex), ..patch() }),
            Err(HostSettingsError::DefaultAgentOff("Codex".into())),
            "nor is one switched off made the default"
        );
        let moved = s
            .apply(&HostSettingsPatch {
                default_agent: Some(AgentKind::Codex),
                disabled_agents: Some(vec![AgentKind::Claude]),
                ..patch()
            })
            .unwrap();
        assert_eq!(
            (moved.default_agent, moved.disabled_agents.as_slice()),
            (AgentKind::Codex, &[AgentKind::Claude][..])
        );
        assert!(codex_off.apply(&off(vec![])).unwrap().agent_on(AgentKind::Codex), "and back on");
        let unknown = off(vec![AgentKind::parse("acp:nobody").unwrap()]);
        assert_eq!(s.apply(&unknown), Err(HostSettingsError::UnknownAgent));
    }

    #[test]
    fn a_removed_acp_agent_takes_its_switch_with_it() {
        use crate::acp_agent::AcpAgentConfig;
        let agent =
            AcpAgentConfig { id: "gone".into(), label: "Gone".into(), command: "g".into(), args: vec![], env: vec![] };
        let kind = AgentKind::parse("acp:gone").unwrap();
        let s = HostSettings::default()
            .apply(&HostSettingsPatch { acp_agents: Some(vec![agent]), disabled_agents: Some(vec![kind]), ..patch() })
            .unwrap();
        assert_eq!(s.agent_off(kind).as_deref(), Some("Gone is switched off in Settings → Agents"));
        let removed = s.apply(&HostSettingsPatch { acp_agents: Some(vec![]), ..patch() }).unwrap();
        assert!(removed.disabled_agents.is_empty());
    }

    #[test]
    fn agent_models_are_set_per_agent_and_cleared_by_blank() {
        let models = |pairs: &[(AgentKind, &str)]| HostSettingsPatch {
            agent_models: Some(pairs.iter().map(|(a, m)| (*a, m.to_string())).collect()),
            ..patch()
        };
        let s = HostSettings::default()
            .apply(&models(&[(AgentKind::Claude, " opus "), (AgentKind::Codex, "gpt-5")]))
            .unwrap();
        assert_eq!(s.agent_models.get(&AgentKind::Claude).map(String::as_str), Some("opus"));
        // Naming one agent leaves the others alone; blank clears it.
        let s = s.apply(&models(&[(AgentKind::Codex, " ")])).unwrap();
        assert_eq!(s.agent_models.len(), 1);
        assert_eq!(s.apply(&models(&[(AgentKind::Claude, "two words")])), Err(HostSettingsError::BadModel));
        let json: HostSettingsPatch = serde_json::from_str(r#"{"agentModels":{"claude":"sonnet"}}"#).unwrap();
        assert_eq!(s.apply(&json).unwrap().agent_models.get(&AgentKind::Claude).map(String::as_str), Some("sonnet"));
        let back: HostSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.agent_models, s.agent_models);
    }

    #[test]
    fn an_acp_agents_model_needs_the_agent_and_goes_with_it() {
        let kind = AgentKind::parse("acp:fake-model").unwrap();
        let model = HostSettingsPatch { agent_models: Some([(kind, "m1".to_string())].into()), ..patch() };
        assert_eq!(HostSettings::default().apply(&model), Err(HostSettingsError::UnknownAgent));
        let agent = crate::acp_agent::AcpAgentConfig {
            id: "fake-model".into(),
            label: "Fake".into(),
            command: "fake-acp".into(),
            args: vec![],
            env: vec![],
        };
        let added = HostSettings::default()
            .apply(&HostSettingsPatch { acp_agents: Some(vec![agent]), ..patch() })
            .and_then(|s| s.apply(&model))
            .unwrap();
        assert_eq!(added.model_for(Some(kind), None).as_deref(), Some("m1"));
        let removed = added.apply(&HostSettingsPatch { acp_agents: Some(vec![]), ..patch() }).unwrap();
        assert!(removed.agent_models.is_empty());
    }

    #[test]
    fn a_sessions_own_model_wins_over_the_agents_default() {
        let s = HostSettings { agent_models: [(AgentKind::Claude, "opus".to_string())].into(), ..Default::default() };
        // The session's own choice (the user's pick, or its persona's model) wins.
        assert_eq!(s.model_for(Some(AgentKind::Claude), Some("haiku")).as_deref(), Some("haiku"));
        // Else the agent's default model.
        assert_eq!(s.model_for(Some(AgentKind::Claude), None).as_deref(), Some("opus"));
        // Else none: the agent CLI's own default.
        assert_eq!(s.model_for(Some(AgentKind::Codex), None), None);
        assert_eq!(s.model_for(None, None), None);
    }

    #[test]
    fn agent_programs_are_full_paths_and_blank_clears_them() {
        let s = HostSettings::default();
        let abs = std::env::temp_dir().join("bin").join("claude").to_string_lossy().into_owned();
        let set = s
            .apply(&HostSettingsPatch {
                claude_program: Some(format!(" {abs} ")),
                devin_program: Some("~/bin/devin".into()),
                ..patch()
            })
            .unwrap();
        assert_eq!(set.program(AgentKind::Claude), Some(abs.as_str()));
        assert_eq!(set.program(AgentKind::Devin), Some("~/bin/devin"));
        assert_eq!(set.program(AgentKind::Codex), None);
        assert_eq!(
            s.apply(&HostSettingsPatch { codex_program: Some("codex".into()), ..patch() }),
            Err(HostSettingsError::BadProgram)
        );
        let cleared = set.apply(&HostSettingsPatch { claude_program: Some(" ".into()), ..patch() }).unwrap();
        assert_eq!(cleared.claude_program, None);
        assert_eq!(cleared.devin_program.as_deref(), Some("~/bin/devin"));
    }

    #[test]
    fn an_installed_acp_profiles_program_can_be_set_and_cleared() {
        let s = HostSettings::default();
        let one = |id: &str, path: &str| HostSettingsPatch {
            acp_programs: Some([(id.to_string(), path.to_string())].into()),
            ..patch()
        };
        let set = s.apply(&one("antigravity", " ~/agy/agy_acp_server.par ")).unwrap();
        assert_eq!(set.acp_programs.get("antigravity").map(String::as_str), Some("~/agy/agy_acp_server.par"));
        assert_eq!(set.apply(&one("antigravity", "agy")), Err(HostSettingsError::BadProgram));
        // Only a profile PDT installs: the others name their command in their definition.
        assert_eq!(s.apply(&one("cursor", "~/c")), Err(HostSettingsError::UnknownAgent));
        assert_eq!(s.apply(&one("nope", "~/c")), Err(HostSettingsError::UnknownAgent));
        assert!(set.apply(&one("antigravity", "")).unwrap().acp_programs.is_empty());
    }

    #[test]
    fn the_projects_folder_is_set_and_cleared() {
        // A full path on this platform (on Windows that needs a drive letter).
        let dir = std::env::temp_dir().join("code").to_string_lossy().into_owned();
        let s = HostSettings::default()
            .apply(&HostSettingsPatch { projects_dir: Some(format!(" {dir} ")), ..patch() })
            .unwrap();
        assert_eq!(s.projects_dir.as_deref(), Some(dir.as_str()));
        let s = s.apply(&HostSettingsPatch { projects_dir: Some(" ".into()), ..patch() }).unwrap();
        assert_eq!(s.projects_dir, None);
        let s = s.apply(&HostSettingsPatch { projects_dir: Some("~/code".into()), ..patch() }).unwrap();
        assert_eq!(s.projects_dir.as_deref(), Some("~/code"));
        let bad = s.apply(&HostSettingsPatch { projects_dir: Some("code".into()), ..patch() });
        assert_eq!(bad, Err(HostSettingsError::BadProjectsDir));
    }

    #[test]
    fn the_documents_settings_are_set_and_cleared() {
        let s = HostSettings::default()
            .apply(&HostSettingsPatch {
                documents_provider: Some(" google ".into()),
                documents_dir: Some("~/Documents/pdt".into()),
                ..patch()
            })
            .unwrap();
        assert_eq!(s.documents_provider.as_deref(), Some("google"));
        assert_eq!(s.documents_dir.as_deref(), Some("~/Documents/pdt"));
        let s = s
            .apply(&HostSettingsPatch {
                documents_provider: Some("".into()),
                documents_dir: Some(" ".into()),
                ..patch()
            })
            .unwrap();
        assert_eq!((s.documents_provider, s.documents_dir), (None, None));
        let bad = HostSettings::default().apply(&HostSettingsPatch { documents_dir: Some("docs".into()), ..patch() });
        assert_eq!(bad, Err(HostSettingsError::BadDocumentsDir));
        let bad =
            HostSettings::default().apply(&HostSettingsPatch { documents_provider: Some("A B".into()), ..patch() });
        assert_eq!(bad, Err(HostSettingsError::BadDocumentsProvider));
    }
}
