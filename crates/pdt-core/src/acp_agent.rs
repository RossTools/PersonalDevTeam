//! ACP agents (PDT-234): agents that speak the Agent Client Protocol, other than Devin, as agent kinds `acp:<id>`.
//! Each is a definition: a profile that ships with PDT (OhMyPi's; each harness's ticket adds its own) or one the user
//! added in Settings → Agents (label, command, arguments, environment). They run only as conversations.

use crate::conversation::PermissionMode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most ACP agents a host may add.
pub const MAX_ACP_AGENTS: usize = 32;
const MAX_LABEL: usize = 40;
const MAX_ARGS: usize = 64;
const MAX_ENV: usize = 64;
const MAX_TEXT: usize = 4096;

/// An environment variable an ACP agent is launched with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// An ACP agent the user added (Settings → Agents → Add ACP agent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpAgentConfig {
    /// Lowercase letters, digits and `-`: its agent kind is `acp:<id>`.
    pub id: String,
    /// Its name in the app ("OpenCode").
    pub label: String,
    /// The program: a name looked up on the PATH, a full path, or `~/…`.
    pub command: String,
    /// The arguments that start its ACP server on stdio (`acp`, `--acp`, …).
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

/// An ACP agent PDT can run: a built-in profile or a user's agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcpAgentDef {
    pub id: String,
    pub label: String,
    /// The icon the app shows for it (a built-in profile's); None: the generic agent icon.
    pub icon: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    /// PDT's permission modes and the agent's session mode id for each. Empty: they come from the `availableModes`
    /// the agent advertises, and when none of those fit, PDT keeps the mode itself by answering the agent's
    /// permission requests.
    #[ts(skip)]
    #[serde(skip)]
    pub modes: Vec<(PermissionMode, String)>,
    /// What the agent adds to ACP (a built-in profile's; the user's agents are plain ACP).
    #[ts(skip)]
    #[serde(skip)]
    pub flavor: AcpFlavor,
    /// It ships with PDT (it can't be edited or removed).
    pub builtin: bool,
}

/// What an ACP agent adds to plain ACP, for the conversation mapper (`crate::conversation::omp`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AcpFlavor {
    #[default]
    Plain,
    /// OhMyPi's `omp acp` (notes: docs/superpowers/notes/omp-acp.md).
    Omp,
}

impl AcpAgentDef {
    pub fn kind(&self) -> Option<crate::session::AgentKind> {
        crate::session::AcpAgentId::new(&self.id).map(crate::session::AgentKind::Acp)
    }
}

impl From<&AcpAgentConfig> for AcpAgentDef {
    fn from(c: &AcpAgentConfig) -> Self {
        AcpAgentDef {
            id: c.id.clone(),
            label: c.label.clone(),
            icon: None,
            command: c.command.clone(),
            args: c.args.clone(),
            env: c.env.clone(),
            modes: Vec::new(),
            flavor: AcpFlavor::Plain,
            builtin: false,
        }
    }
}

/// The ACP profiles that ship with PDT. Each harness's own ticket adds its profile here.
pub fn builtin_profiles() -> Vec<AcpAgentDef> {
    vec![omp()]
}

/// OhMyPi (`omp acp`, PDT-238). Its modes come from those it advertises: `default` (Ask) and, when its plan mode is
/// on, `plan`; PDT keeps Accept edits and Bypass itself.
fn omp() -> AcpAgentDef {
    AcpAgentDef {
        id: "omp".into(),
        label: "OhMyPi".into(),
        icon: None,
        command: "omp".into(),
        args: vec!["acp".into()],
        env: Vec::new(),
        modes: Vec::new(),
        flavor: AcpFlavor::Omp,
        builtin: true,
    }
}

/// Every ACP agent: the built-in profiles, then the user's.
pub fn all(user: &[AcpAgentConfig]) -> Vec<AcpAgentDef> {
    let mut defs = builtin_profiles();
    defs.extend(user.iter().map(AcpAgentDef::from));
    defs
}

/// The ACP agent with this id.
pub fn find(user: &[AcpAgentConfig], id: &str) -> Option<AcpAgentDef> {
    all(user).into_iter().find(|d| d.id == id)
}

/// An agent id: 1 to 32 lowercase letters, digits and `-`, starting with a letter or digit.
pub fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

fn valid_env_name(name: &str) -> bool {
    let mut b = name.bytes();
    b.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn text_ok(s: &str) -> bool {
    s.len() <= MAX_TEXT && !s.contains('\0')
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AcpAgentError {
    #[error("an ACP agent's id must be 1 to 32 lowercase letters, digits and dashes")]
    BadId,
    #[error("two ACP agents share the id {0}")]
    DuplicateId(String),
    #[error("an ACP agent needs a label of at most 40 characters")]
    BadLabel,
    #[error("an ACP agent needs a command")]
    BadCommand,
    #[error("an ACP agent's arguments must not hold NUL characters (and there may be 64 at most)")]
    BadArgs,
    #[error("environment variable names are letters, digits and _, not starting with a digit (64 at most)")]
    BadEnv,
    #[error("a host may have at most 32 ACP agents")]
    TooMany,
}

/// The user's ACP agents, trimmed and checked: ids unique (and not a built-in profile's), labels and commands set.
pub fn validate(agents: &[AcpAgentConfig]) -> Result<Vec<AcpAgentConfig>, AcpAgentError> {
    if agents.len() > MAX_ACP_AGENTS {
        return Err(AcpAgentError::TooMany);
    }
    let builtins = builtin_profiles();
    let mut out: Vec<AcpAgentConfig> = Vec::new();
    for a in agents {
        let id = a.id.trim().to_string();
        if !valid_id(&id) {
            return Err(AcpAgentError::BadId);
        }
        if out.iter().any(|o| o.id == id) || builtins.iter().any(|b| b.id == id) {
            return Err(AcpAgentError::DuplicateId(id));
        }
        let label = a.label.trim().to_string();
        if label.is_empty() || label.chars().count() > MAX_LABEL || label.chars().any(char::is_control) {
            return Err(AcpAgentError::BadLabel);
        }
        let command = a.command.trim().to_string();
        if command.is_empty() || !text_ok(&command) || command.chars().any(char::is_control) {
            return Err(AcpAgentError::BadCommand);
        }
        if a.args.len() > MAX_ARGS || !a.args.iter().all(|x| text_ok(x)) {
            return Err(AcpAgentError::BadArgs);
        }
        if a.env.len() > MAX_ENV || !a.env.iter().all(|e| valid_env_name(e.name.trim()) && text_ok(&e.value)) {
            return Err(AcpAgentError::BadEnv);
        }
        let env = a.env.iter().map(|e| EnvVar { name: e.name.trim().to_string(), value: e.value.clone() }).collect();
        out.push(AcpAgentConfig { id, label, command, args: a.args.clone(), env });
    }
    Ok(out)
}

/// The PDT permission mode one of an agent's advertised session modes stands for, read from its id or name ("Accept
/// Edits", `bypassPermissions`, `yolo`); None when it fits none.
pub fn pdt_mode_for(id: &str, name: &str) -> Option<PermissionMode> {
    let norm = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    [norm(id), norm(name)].into_iter().find_map(|s| match s.as_str() {
        "ask" | "default" | "normal" | "manual" | "askalways" => Some(PermissionMode::Ask),
        "acceptedits" | "autoedit" | "autoedits" | "edit" | "edits" | "code" => Some(PermissionMode::AcceptEdits),
        "plan" | "planning" | "readonly" | "architect" => Some(PermissionMode::Plan),
        "bypass" | "bypasspermissions" | "yolo" | "fullaccess" | "dangerous" | "dontask" => {
            Some(PermissionMode::Bypass)
        }
        _ => None,
    })
}

/// The mode map from a session's `availableModes` (`[{id, name}]`): each PDT mode the first advertised mode that
/// fits it.
pub fn advertised_modes(available: &serde_json::Value) -> Vec<(PermissionMode, String)> {
    let mut map: Vec<(PermissionMode, String)> = Vec::new();
    for m in available.as_array().into_iter().flatten() {
        let Some(id) = m.get("id").and_then(|v| v.as_str()) else { continue };
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(mode) = pdt_mode_for(id, name)
            && !map.iter().any(|(m, _)| *m == mode)
        {
            map.push((mode, id.to_string()));
        }
    }
    map.sort_by_key(|(m, _)| PermissionMode::ALL.iter().position(|a| a == m));
    map
}

/// How PDT answers a permission request itself for an agent with no modes, keeping the session's mode: Some(true)
/// allows it, Some(false) rejects it, None asks the user. `kind` is the tool call's ACP kind (`read`, `edit`,
/// `execute`, …).
pub fn enforced_answer(mode: PermissionMode, kind: &str) -> Option<bool> {
    let reads = matches!(kind, "read" | "search" | "think");
    let edits = matches!(kind, "edit" | "delete" | "move");
    match mode {
        PermissionMode::Bypass => Some(true),
        _ if reads => Some(true),
        PermissionMode::AcceptEdits if edits => Some(true),
        PermissionMode::Plan if edits || kind == "execute" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str) -> AcpAgentConfig {
        AcpAgentConfig {
            id: id.into(),
            label: " Open Code ".into(),
            command: " opencode ".into(),
            args: vec!["acp".into()],
            env: vec![EnvVar { name: " OPENCODE_X ".into(), value: " v ".into() }],
        }
    }

    #[test]
    fn user_agents_are_trimmed_and_checked() {
        let v = validate(&[agent("opencode")]).unwrap();
        assert_eq!((v[0].label.as_str(), v[0].command.as_str()), ("Open Code", "opencode"));
        assert_eq!(v[0].env, vec![EnvVar { name: "OPENCODE_X".into(), value: " v ".into() }]);
        assert_eq!(validate(&[agent("Open")]).unwrap_err(), AcpAgentError::BadId);
        assert_eq!(validate(&[agent("-x")]).unwrap_err(), AcpAgentError::BadId);
        assert_eq!(
            validate(&[agent("a"), agent("a")]).unwrap_err(),
            AcpAgentError::DuplicateId("a".into()),
            "ids are unique"
        );
        let mut a = agent("a");
        a.label = " ".into();
        assert_eq!(validate(&[a]).unwrap_err(), AcpAgentError::BadLabel);
        let mut a = agent("a");
        a.command = "".into();
        assert_eq!(validate(&[a]).unwrap_err(), AcpAgentError::BadCommand);
        let mut a = agent("a");
        a.env[0].name = "1X".into();
        assert_eq!(validate(&[a]).unwrap_err(), AcpAgentError::BadEnv);
        let mut a = agent("a");
        a.args.push("x\0".into());
        assert_eq!(validate(&[a]).unwrap_err(), AcpAgentError::BadArgs);
        let many: Vec<_> = (0..=MAX_ACP_AGENTS).map(|n| agent(&format!("a{n}"))).collect();
        assert_eq!(validate(&many).unwrap_err(), AcpAgentError::TooMany);
    }

    #[test]
    fn omp_ships_built_in_and_its_id_is_taken() {
        let d = find(&[], "omp").unwrap();
        assert_eq!(
            (d.label.as_str(), d.command.as_str(), d.args.as_slice()),
            ("OhMyPi", "omp", &["acp".to_string()][..])
        );
        assert_eq!((d.builtin, d.flavor), (true, AcpFlavor::Omp));
        assert_eq!(d.kind().unwrap().as_str(), "acp:omp");
        assert_eq!(validate(&[agent("omp")]).unwrap_err(), AcpAgentError::DuplicateId("omp".into()));
        // It advertises `default` and `plan`: PDT's Ask and Plan.
        let available = json!([{"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]);
        assert_eq!(
            advertised_modes(&available),
            vec![(PermissionMode::Ask, "default".into()), (PermissionMode::Plan, "plan".into())]
        );
    }

    #[test]
    fn a_user_agent_is_found_by_id_with_no_mode_map() {
        let user = validate(&[agent("opencode")]).unwrap();
        let d = find(&user, "opencode").unwrap();
        assert_eq!((d.label.as_str(), d.builtin, d.modes.len()), ("Open Code", false, 0));
        assert_eq!(d.kind().unwrap().as_str(), "acp:opencode");
        assert!(find(&user, "other").is_none());
    }

    #[test]
    fn advertised_modes_map_by_id_or_name() {
        let available = json!([
            {"id": "default", "name": "Default"},
            {"id": "acceptEdits", "name": "Accept Edits"},
            {"id": "m3", "name": "Plan"},
            {"id": "yolo", "name": "YOLO"},
            {"id": "ask2", "name": "Ask"},
            {"id": "custom", "name": "Something else"}
        ]);
        assert_eq!(
            advertised_modes(&available),
            vec![
                (PermissionMode::Ask, "default".into()),
                (PermissionMode::AcceptEdits, "acceptEdits".into()),
                (PermissionMode::Plan, "m3".into()),
                (PermissionMode::Bypass, "yolo".into()),
            ]
        );
        assert!(advertised_modes(&json!(null)).is_empty());
    }

    #[test]
    fn pdt_keeps_the_mode_of_an_agent_with_none() {
        use PermissionMode::*;
        assert_eq!(enforced_answer(Bypass, "execute"), Some(true));
        assert_eq!(enforced_answer(Ask, "read"), Some(true));
        assert_eq!(enforced_answer(Ask, "edit"), None);
        assert_eq!(enforced_answer(Ask, "execute"), None);
        assert_eq!(enforced_answer(AcceptEdits, "edit"), Some(true));
        assert_eq!(enforced_answer(AcceptEdits, "execute"), None);
        assert_eq!(enforced_answer(Plan, "edit"), Some(false));
        assert_eq!(enforced_answer(Plan, "execute"), Some(false));
        assert_eq!(enforced_answer(Plan, "fetch"), None);
    }
}
