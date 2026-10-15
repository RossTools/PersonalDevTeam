//! Terminal session types and the rules for launching them. Pure: callers supply the platform facts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_TERMINAL_DIM: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionKind {
    Shell,
    AgentTui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionStatus {
    Running,
    Finished,
    Interrupted,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Shell => "shell",
            SessionKind::AgentTui => "agent_tui",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [SessionKind::Shell, SessionKind::AgentTui].into_iter().find(|k| k.as_str() == s)
    }
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [AgentKind::Claude, AgentKind::Codex].into_iter().find(|k| k.as_str() == s)
    }
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Running => "running",
            SessionStatus::Finished => "finished",
            SessionStatus::Interrupted => "interrupted",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [SessionStatus::Running, SessionStatus::Finished, SessionStatus::Interrupted]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    /// None means the session runs in the project's main checkout.
    pub worktree_id: Option<String>,
    pub kind: SessionKind,
    pub agent: Option<AgentKind>,
    pub title: String,
    /// Working directory, native separators.
    pub cwd: String,
    pub status: SessionStatus,
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn validate(self) -> Result<Self, SessionValidationError> {
        let ok = |d: u16| (1..=MAX_TERMINAL_DIM).contains(&d);
        if ok(self.cols) && ok(self.rows) { Ok(self) } else { Err(SessionValidationError::BadSize) }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewSession {
    pub project_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub worktree_id: Option<String>,
    pub kind: SessionKind,
    #[serde(default)]
    #[ts(optional)]
    pub agent: Option<AgentKind>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidSession {
    pub project_id: String,
    pub worktree_id: Option<String>,
    pub kind: SessionKind,
    pub agent: Option<AgentKind>,
    pub size: TerminalSize,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SessionValidationError {
    #[error("project id must not be empty")]
    EmptyProject,
    #[error("an agent terminal needs an agent (claude or codex)")]
    MissingAgent,
    #[error("a shell session must not name an agent")]
    UnexpectedAgent,
    #[error("terminal size must be between 1 and 1000 columns and rows")]
    BadSize,
}

impl NewSession {
    pub fn validate(&self) -> Result<ValidSession, SessionValidationError> {
        let project_id = self.project_id.trim();
        if project_id.is_empty() {
            return Err(SessionValidationError::EmptyProject);
        }
        let worktree_id = self.worktree_id.as_deref().map(str::trim).filter(|w| !w.is_empty()).map(String::from);
        match (self.kind, self.agent) {
            (SessionKind::AgentTui, None) => return Err(SessionValidationError::MissingAgent),
            (SessionKind::Shell, Some(_)) => return Err(SessionValidationError::UnexpectedAgent),
            _ => {}
        }
        let size = TerminalSize { cols: self.cols, rows: self.rows }.validate()?;
        Ok(ValidSession { project_id: project_id.into(), worktree_id, kind: self.kind, agent: self.agent, size })
    }
}

/// Facts about the host that decide which programs to launch. Built by `pdt_agents::launch::detect_platform`.
#[derive(Debug, Clone, PartialEq)]
pub struct Platform {
    pub windows: bool,
    pub has_pwsh: bool,
    pub unix_shell: Option<String>,
    pub claude_program: String,
    pub codex_program: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
}

/// What to run for a session. `resume` continues the agent's last conversation in the working directory.
pub fn launch(kind: SessionKind, agent: Option<AgentKind>, platform: &Platform, resume: bool) -> Launch {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match (kind, agent) {
        (SessionKind::Shell, _) if platform.windows => Launch {
            program: if platform.has_pwsh { "pwsh" } else { "powershell.exe" }.into(),
            args: args(&["-NoLogo"]),
        },
        (SessionKind::Shell, _) => Launch {
            program: platform.unix_shell.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "/bin/zsh".into()),
            args: args(&["-l"]),
        },
        (SessionKind::AgentTui, Some(AgentKind::Codex)) => Launch {
            program: platform.codex_program.clone(),
            args: if resume { args(&["resume", "--last"]) } else { vec![] },
        },
        (SessionKind::AgentTui, _) => Launch {
            program: platform.claude_program.clone(),
            args: if resume { args(&["--continue"]) } else { vec![] },
        },
    }
}

pub fn default_title(kind: SessionKind, agent: Option<AgentKind>) -> &'static str {
    match (kind, agent) {
        (SessionKind::Shell, _) => "Shell",
        (SessionKind::AgentTui, Some(AgentKind::Codex)) => "Codex",
        (SessionKind::AgentTui, _) => "Claude",
    }
}

/// pdtd's own configuration variables (keep in sync with crates/pdtd/src/config.rs). Never passed to children:
/// a dev pdtd started inside a stable-hosted terminal must not inherit the stable port or data dir.
const PDT_ENV_VARS: [&str; 5] = ["PDT_PROFILE", "PDT_PORT", "PDT_DATA_DIR", "PDT_WEB_DIR", "PDT_ALLOWED_HOSTS"];
/// Removed for agents so they bill the user's subscription, not an API key (spec §5.5).
const API_KEY_ENV_VARS: [&str; 2] = ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"];

pub fn scrubbed_env(kind: SessionKind) -> Vec<&'static str> {
    let mut vars = PDT_ENV_VARS.to_vec();
    if kind == SessionKind::AgentTui {
        vars.extend(API_KEY_ENV_VARS);
    }
    vars
}

/// Client → server control messages on the PTY WebSocket (sent as text frames).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum PtyClientMessage {
    Resize { cols: u16, rows: u16 },
}

/// Server → client control messages on the PTY WebSocket (sent as text frames).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum PtyServerMessage {
    /// Sent once per connection after the scrollback replay (even an empty one). Binary frames before it are
    /// replay: the client must not answer terminal queries in them again.
    Ready,
    Exit {
        code: i32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(windows: bool, has_pwsh: bool, unix_shell: Option<&str>) -> Platform {
        Platform {
            windows,
            has_pwsh,
            unix_shell: unix_shell.map(Into::into),
            claude_program: "claude".into(),
            codex_program: "codex".into(),
        }
    }

    fn launch_of(l: Launch) -> (String, Vec<String>) {
        (l.program, l.args)
    }

    #[test]
    fn enum_strings_match_serde() {
        for k in [SessionKind::Shell, SessionKind::AgentTui] {
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{}\"", k.as_str()));
            assert_eq!(SessionKind::parse(k.as_str()), Some(k));
        }
        for a in [AgentKind::Claude, AgentKind::Codex] {
            assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{}\"", a.as_str()));
            assert_eq!(AgentKind::parse(a.as_str()), Some(a));
        }
        for s in [SessionStatus::Running, SessionStatus::Finished, SessionStatus::Interrupted] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_str()));
            assert_eq!(SessionStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(SessionKind::AgentTui.as_str(), "agent_tui");
        assert_eq!(SessionKind::parse("bogus"), None);
    }

    #[test]
    fn shell_launch_per_platform() {
        assert_eq!(
            launch_of(launch(SessionKind::Shell, None, &platform(true, true, None), false)),
            ("pwsh".into(), vec!["-NoLogo".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::Shell, None, &platform(true, false, None), false)),
            ("powershell.exe".into(), vec!["-NoLogo".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::Shell, None, &platform(false, false, Some("/bin/bash")), false)),
            ("/bin/bash".into(), vec!["-l".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::Shell, None, &platform(false, false, Some("  ")), false)),
            ("/bin/zsh".into(), vec!["-l".into()])
        );
    }

    #[test]
    fn agent_launch_and_resume() {
        let p = platform(false, false, None);
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, false)),
            ("claude".into(), vec![])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, true)),
            ("claude".into(), vec!["--continue".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, false)),
            ("codex".into(), vec![])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, true)),
            ("codex".into(), vec!["resume".into(), "--last".into()])
        );
        let mut custom = p.clone();
        custom.codex_program = "/opt/codex".into();
        assert_eq!(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &custom, false).program, "/opt/codex");
    }

    #[test]
    fn scrubbed_env_always_removes_pdt_vars() {
        for kind in [SessionKind::Shell, SessionKind::AgentTui] {
            let vars = scrubbed_env(kind);
            for v in ["PDT_PROFILE", "PDT_PORT", "PDT_DATA_DIR", "PDT_WEB_DIR", "PDT_ALLOWED_HOSTS"] {
                assert!(vars.contains(&v), "{kind:?} keeps {v}");
            }
        }
        assert!(scrubbed_env(SessionKind::AgentTui).contains(&"ANTHROPIC_API_KEY"));
        assert!(scrubbed_env(SessionKind::AgentTui).contains(&"OPENAI_API_KEY"));
        assert!(!scrubbed_env(SessionKind::Shell).contains(&"ANTHROPIC_API_KEY"));
    }

    fn new(kind: SessionKind, agent: Option<AgentKind>, cols: u16, rows: u16) -> NewSession {
        NewSession { project_id: " p1 ".into(), worktree_id: Some("  ".into()), kind, agent, cols, rows }
    }

    #[test]
    fn validate_session_input() {
        let v = new(SessionKind::Shell, None, 80, 24).validate().unwrap();
        assert_eq!(v.project_id, "p1");
        assert_eq!(v.worktree_id, None, "blank worktree id means the repo root");
        assert_eq!(v.size, TerminalSize { cols: 80, rows: 24 });
        assert_eq!(
            new(SessionKind::AgentTui, None, 80, 24).validate().unwrap_err(),
            SessionValidationError::MissingAgent
        );
        assert_eq!(
            new(SessionKind::Shell, Some(AgentKind::Claude), 80, 24).validate().unwrap_err(),
            SessionValidationError::UnexpectedAgent
        );
        assert_eq!(new(SessionKind::Shell, None, 0, 24).validate().unwrap_err(), SessionValidationError::BadSize);
        assert_eq!(new(SessionKind::Shell, None, 80, 1001).validate().unwrap_err(), SessionValidationError::BadSize);
        let mut blank = new(SessionKind::Shell, None, 80, 24);
        blank.project_id = " ".into();
        assert_eq!(blank.validate().unwrap_err(), SessionValidationError::EmptyProject);
    }

    #[test]
    fn pty_messages_wire_format() {
        assert_eq!(serde_json::to_string(&PtyServerMessage::Exit { code: 3 }).unwrap(), r#"{"type":"exit","code":3}"#);
        assert_eq!(serde_json::to_string(&PtyServerMessage::Ready).unwrap(), r#"{"type":"ready"}"#);
        let m: PtyClientMessage = serde_json::from_str(r#"{"type":"resize","cols":100,"rows":30}"#).unwrap();
        assert_eq!(m, PtyClientMessage::Resize { cols: 100, rows: 30 });
    }

    #[test]
    fn titles() {
        assert_eq!(default_title(SessionKind::Shell, None), "Shell");
        assert_eq!(default_title(SessionKind::AgentTui, Some(AgentKind::Claude)), "Claude");
        assert_eq!(default_title(SessionKind::AgentTui, Some(AgentKind::Codex)), "Codex");
    }
}
