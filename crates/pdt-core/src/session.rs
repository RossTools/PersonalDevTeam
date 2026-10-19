//! Terminal session types and the rules for launching them. Pure: callers supply the platform facts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const MAX_TERMINAL_DIM: u16 = 1000;
/// The longest first prompt a new agent session takes, in bytes.
pub const MAX_INITIAL_PROMPT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionKind {
    Shell,
    AgentTui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
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
    /// Agents only: launched with permission prompts bypassed (`--dangerously-skip-permissions` for Claude,
    /// `--dangerously-bypass-approvals-and-sandbox` for Codex). Persisted so restart/auto-resume keep it.
    pub bypass_permissions: bool,
    pub title: String,
    /// Working directory, native separators.
    pub cwd: String,
    pub status: SessionStatus,
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
    /// The agent's own conversation id. Only Codex needs it (Claude's conversation id is `id`); it is found in
    /// Codex's rollout files once a turn has been taken, and None until then.
    pub external_session_id: Option<String>,
    /// Codex only: when the session was last launched fresh (create, or restart/auto-resume with no conversation to
    /// resume), in ms. Matches the session to its conversation's start time. Resuming a bound conversation leaves it
    /// alone. None for rows from before it was recorded.
    #[ts(type = "number | null")]
    pub fresh_started_at: Option<i64>,
    /// Agents only; None for shells and ended sessions.
    pub activity: Option<crate::activity::Activity>,
    /// When the session last became idle or needs_you (ms).
    #[ts(type = "number | null")]
    pub attention_at: Option<i64>,
    /// When the user last viewed the session, on any device (ms).
    #[ts(type = "number | null")]
    pub last_viewed_at: Option<i64>,
    /// Computed: became idle or needs_you after the user last viewed it (on any device).
    pub unseen: bool,
    /// The ticket attempt this session works on (sessions made by Start).
    pub attempt_id: Option<String>,
    /// The attempt's ticket.
    pub ticket: Option<crate::ticket::SessionTicket>,
    /// Agents: shown as the agent's terminal or as a conversation. Shells are always `terminal`.
    pub mode: crate::conversation::SessionMode,
    /// Conversations: the model asked for (None: the agent's default).
    pub model: Option<String>,
    /// Conversations: PDT's permission setting.
    pub permission_mode: Option<crate::conversation::PermissionMode>,
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
    /// Agents only: bypass the agent's permission prompts. Absent/false launches normally; ignored for shells.
    #[serde(default)]
    #[ts(optional)]
    pub bypass_permissions: Option<bool>,
    pub cols: u16,
    pub rows: u16,
    /// Where the session's tab opens; the end of its project's workspace when absent.
    #[serde(default)]
    #[ts(optional)]
    pub tab: Option<crate::workspace::TabPlacement>,
    /// Agents only: the first prompt, given after the agent's flags when the session is created (restarts resume the
    /// conversation and never repeat it); in a conversation it is the first message. At most 16 KB.
    #[serde(default)]
    #[ts(optional)]
    pub initial_prompt: Option<String>,
    /// Agents only: `conversation` (the default) or `terminal`.
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<crate::conversation::SessionMode>,
    /// Conversations only: the model (None: the agent's default).
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    /// Conversations only: the permission mode (default `ask`, or `bypass` when `bypassPermissions` is set).
    #[serde(default)]
    #[ts(optional)]
    pub permission_mode: Option<crate::conversation::PermissionMode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidSession {
    pub project_id: String,
    pub worktree_id: Option<String>,
    pub kind: SessionKind,
    pub agent: Option<AgentKind>,
    pub bypass_permissions: bool,
    pub size: TerminalSize,
    pub initial_prompt: Option<String>,
    pub mode: crate::conversation::SessionMode,
    pub model: Option<String>,
    pub permission_mode: Option<crate::conversation::PermissionMode>,
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
    #[error("only an agent session takes an initial prompt")]
    PromptForShell,
    #[error("an initial prompt must not be blank and at most 16 KB")]
    BadPrompt,
    #[error("only an agent session can be a conversation")]
    ConversationForShell,
    #[error("a model name must be 1 to 100 characters with no spaces")]
    BadModel,
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
        // Bypass only applies to agents; a shell never carries it.
        let bypass_permissions = self.kind == SessionKind::AgentTui && self.bypass_permissions.unwrap_or(false);
        let initial_prompt = match &self.initial_prompt {
            None => None,
            Some(_) if self.kind != SessionKind::AgentTui => return Err(SessionValidationError::PromptForShell),
            Some(p) if p.trim().is_empty() || p.len() > MAX_INITIAL_PROMPT => {
                return Err(SessionValidationError::BadPrompt);
            }
            Some(p) => Some(p.clone()),
        };
        use crate::conversation::{PermissionMode, SessionMode};
        let mode = match (self.kind, self.agent, self.mode) {
            (SessionKind::Shell, _, Some(SessionMode::Conversation)) => {
                return Err(SessionValidationError::ConversationForShell);
            }
            (SessionKind::Shell, _, _) => SessionMode::Terminal,
            (_, _, m) => m.unwrap_or(SessionMode::Conversation),
        };
        let conversation = mode == SessionMode::Conversation;
        let model = match self.model.as_deref().map(str::trim).filter(|m| !m.is_empty() && conversation) {
            Some(m) if !crate::conversation::valid_model(m) => return Err(SessionValidationError::BadModel),
            m => m.map(String::from),
        };
        let permission_mode = conversation.then(|| {
            self.permission_mode.unwrap_or(if bypass_permissions {
                PermissionMode::Bypass
            } else {
                PermissionMode::Ask
            })
        });
        // The two settings agree: bypass is a permission mode for a conversation.
        let bypass_permissions = match permission_mode {
            Some(m) => m == PermissionMode::Bypass,
            None => bypass_permissions,
        };
        Ok(ValidSession {
            project_id: project_id.into(),
            worktree_id,
            kind: self.kind,
            agent: self.agent,
            bypass_permissions,
            size,
            initial_prompt,
            mode,
            model,
            permission_mode,
        })
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
    /// The user's login-shell PATH, when pdtd's own PATH is not it (a macOS app started by launchd gets only
    /// /usr/bin:/bin:/usr/sbin:/sbin). Programs are looked up on it and sessions run with it.
    pub login_path: Option<String>,
    /// Codex's home directory (`$CODEX_HOME`, else `~/.codex`), where it saves conversations. None when it cannot be
    /// determined.
    pub codex_home: Option<std::path::PathBuf>,
    /// Claude's config directory (`$CLAUDE_CONFIG_DIR`, else `~/.claude`), where it saves conversations. None when
    /// it cannot be determined.
    pub claude_config_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
}

/// How to start an agent session: a new conversation, or continue one that was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    Fresh,
    /// Claude's conversation id is the session id, so each session resumes its own conversation.
    Claude,
    /// Codex's own id for this session's conversation (found in its rollout files).
    Codex(String),
}

/// What to run for a session. Shells ignore `resume` and `bypass_permissions`. A resume that does not match the
/// agent starts fresh. `bypass_permissions` adds the agent's skip-all-prompts flag (dangerous; see `Session`).
pub fn launch(
    kind: SessionKind,
    agent: Option<AgentKind>,
    platform: &Platform,
    resume: &Resume,
    session_id: &str,
    bypass_permissions: bool,
) -> Launch {
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
        (SessionKind::AgentTui, Some(AgentKind::Codex)) => {
            let mut a = match resume {
                Resume::Codex(id) => args(&["resume", id]),
                _ => vec![],
            };
            // Valid on both the root command and `resume`, so appending always works.
            if bypass_permissions {
                a.push("--dangerously-bypass-approvals-and-sandbox".into());
            }
            Launch { program: platform.codex_program.clone(), args: a }
        }
        (SessionKind::AgentTui, _) => {
            let mut a = args(&[if *resume == Resume::Claude { "--resume" } else { "--session-id" }, session_id]);
            if bypass_permissions {
                a.push("--dangerously-skip-permissions".into());
            }
            Launch { program: platform.claude_program.clone(), args: a }
        }
    }
}

/// The first prompt as the agent's last argument. Both agents take a prompt after their flags; one that starts with
/// `-` gets a leading space so it is never read as a flag.
pub fn prompt_arg(prompt: &str) -> String {
    if prompt.starts_with('-') { format!(" {prompt}") } else { prompt.to_string() }
}

/// Characters `cmd.exe` would interpret in an argument (npm-installed agents on Windows are `.cmd` shims run through
/// it), so a file path containing one is never put on such a command line.
const CMD_METACHARACTERS: &[char] = &['&', '|', '<', '>', '^', '%', '!', '"', '(', ')', '\n', '\r'];

/// Whether `cmd.exe` would leave this argument alone.
pub fn cmd_safe(arg: &str) -> bool {
    !arg.contains(CMD_METACHARACTERS)
}

/// The short prompt given instead of the real one when the agent is a `.cmd` shim: the real prompt is in
/// `prompt_file`. For a ticket (`ticket_key`, e.g. `PDT-4`) it names the ticket and the file, or only the ticket
/// when the path is unsafe for cmd. Without a ticket it names the file; None (no prompt) when the path is unsafe.
pub fn shim_prompt(ticket_key: Option<&str>, prompt_file: &str) -> Option<String> {
    let safe = cmd_safe(prompt_file);
    match (ticket_key, safe) {
        (Some(key), true) => Some(format!("{key}: read {prompt_file} for the ticket, then start.")),
        (Some(key), false) => Some(format!("Start on ticket {key} (see {}).", crate::brand::SHORT_NAME)),
        (None, true) => Some(format!("Read {prompt_file} and follow it.")),
        (None, false) => None,
    }
}

/// Where the session runs: the worktree's branch, or the project name for the main checkout. The kind and agent are
/// not in the title; the UI shows them with an icon.
pub fn default_title(place: &str) -> String {
    place.to_string()
}

/// What a session runs, for messages that name it: "Claude", "Codex" or "Shell".
pub fn kind_label(kind: SessionKind, agent: Option<AgentKind>) -> &'static str {
    match (kind, agent) {
        (SessionKind::Shell, _) => "Shell",
        (SessionKind::AgentTui, Some(AgentKind::Codex)) => "Codex",
        (SessionKind::AgentTui, _) => "Claude",
    }
}

pub const MAX_TITLE_CHARS: usize = 100;

/// Trims a user-supplied session title; None when empty or longer than `MAX_TITLE_CHARS`.
pub fn clean_title(raw: &str) -> Option<String> {
    let t = raw.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TITLE_CHARS).then(|| t.to_string())
}

/// pdtd's own configuration lives in `PDT_*` variables; none is ever passed to a child, so a dev pdtd started inside a
/// stable-hosted terminal cannot inherit the stable port, data dir or owner. PDT sets its own variables afterwards.
const PDT_ENV_PREFIX: &str = "PDT_";
/// Removed for agents under the subscription policy, so they bill the user's subscription (spec §5.5; remote-host §6).
const API_KEY_ENV_VARS: [&str; 2] = ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"];
/// Inherited from the shell that started pdtd (often Claude Code itself): they disable color or make `claude`
/// believe it is nested. Never passed to any child. Only a running session's own `CLAUDE_CODE_*` variables are
/// listed: user settings such as `CLAUDE_CODE_OAUTH_TOKEN` or `CLAUDE_CODE_USE_BEDROCK` must pass through.
const LEAKED_ENV_VARS: [&str; 10] = [
    "NO_COLOR",
    "FORCE_COLOR",
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
];

/// Variables every terminal child gets: what the browser terminal (xterm.js) supports. `FORCE_HYPERLINK` makes
/// Claude Code and other CLIs send OSC 8 links, which stay clickable when a long URL wraps.
pub fn terminal_env() -> Vec<(String, String)> {
    [("TERM", "xterm-256color"), ("COLORTERM", "truecolor"), ("FORCE_HYPERLINK", "1")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Whether the inherited environment variable `name` must be removed from a child of this kind.
pub fn is_scrubbed(kind: SessionKind, name: &str, creds: crate::host::CredentialPolicy) -> bool {
    name.starts_with(PDT_ENV_PREFIX)
        || LEAKED_ENV_VARS.contains(&name)
        || (kind == SessionKind::AgentTui
            && creds == crate::host::CredentialPolicy::Subscription
            && API_KEY_ENV_VARS.contains(&name))
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
    /// replay: the client must not answer terminal queries in them again. A client that connected with
    /// `?replay=gzip` gets the replay as one gzip-compressed frame; live output after `ready` is never compressed.
    Ready,
    Exit {
        code: i32,
    },
    /// The session has no terminal in this daemon run (e.g. it was interrupted by a pdtd restart and not resumed).
    /// Sent instead of `ready`, then the server closes; the client should stop reconnecting until the session is
    /// restarted. A session that exited during this run still gets `ready` and `exit`.
    NotRunning,
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
            login_path: None,
            codex_home: None,
            claude_config_dir: None,
        }
    }

    const SID: &str = "0199e5a8-7c2e-7d4b-9a51-3f0e8c6b2d10";

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
            launch_of(launch(SessionKind::Shell, None, &platform(true, true, None), &Resume::Fresh, SID, false)),
            ("pwsh".into(), vec!["-NoLogo".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::Shell, None, &platform(true, false, None), &Resume::Fresh, SID, false)),
            ("powershell.exe".into(), vec!["-NoLogo".into()])
        );
        assert_eq!(
            launch_of(launch(
                SessionKind::Shell,
                None,
                &platform(false, false, Some("/bin/bash")),
                &Resume::Fresh,
                SID,
                false
            )),
            ("/bin/bash".into(), vec!["-l".into()])
        );
        assert_eq!(
            launch_of(launch(
                SessionKind::Shell,
                None,
                &platform(false, false, Some("  ")),
                &Resume::Fresh,
                SID,
                false
            )),
            ("/bin/zsh".into(), vec!["-l".into()])
        );
        // Bypass never affects a shell.
        assert_eq!(
            launch_of(launch(
                SessionKind::Shell,
                None,
                &platform(false, false, Some("/bin/bash")),
                &Resume::Fresh,
                SID,
                true
            )),
            ("/bin/bash".into(), vec!["-l".into()])
        );
    }

    #[test]
    fn agent_launch_and_resume() {
        let p = platform(false, false, None);
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, &Resume::Fresh, SID, false)),
            ("claude".into(), vec!["--session-id".into(), SID.into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, &Resume::Claude, SID, false)),
            ("claude".into(), vec!["--resume".into(), SID.into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, &Resume::Fresh, SID, false)),
            ("codex".into(), vec![])
        );
        assert_eq!(
            launch_of(launch(
                SessionKind::AgentTui,
                Some(AgentKind::Codex),
                &p,
                &Resume::Codex("019-abc".into()),
                SID,
                false
            )),
            ("codex".into(), vec!["resume".into(), "019-abc".into()])
        );
        // A resume for the wrong agent starts fresh, and a Claude session never gets Codex's id.
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, &Resume::Claude, SID, false)),
            ("codex".into(), vec![])
        );
        assert_eq!(
            launch_of(launch(
                SessionKind::AgentTui,
                Some(AgentKind::Claude),
                &p,
                &Resume::Codex("x".into()),
                SID,
                false
            )),
            ("claude".into(), vec!["--session-id".into(), SID.into()])
        );
        let mut custom = p.clone();
        custom.codex_program = "/opt/codex".into();
        assert_eq!(
            launch(SessionKind::AgentTui, Some(AgentKind::Codex), &custom, &Resume::Fresh, SID, false).program,
            "/opt/codex"
        );
    }

    #[test]
    fn bypass_permissions_adds_the_skip_flag() {
        let p = platform(false, false, None);
        // Claude: the flag follows the conversation args, fresh or resumed.
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, &Resume::Fresh, SID, true)),
            ("claude".into(), vec!["--session-id".into(), SID.into(), "--dangerously-skip-permissions".into()])
        );
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, &Resume::Claude, SID, true)),
            ("claude".into(), vec!["--resume".into(), SID.into(), "--dangerously-skip-permissions".into()])
        );
        // Codex: fresh and resumed both take the bypass flag.
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, &Resume::Fresh, SID, true)),
            ("codex".into(), vec!["--dangerously-bypass-approvals-and-sandbox".into()])
        );
        assert_eq!(
            launch_of(launch(
                SessionKind::AgentTui,
                Some(AgentKind::Codex),
                &p,
                &Resume::Codex("019-abc".into()),
                SID,
                true
            )),
            (
                "codex".into(),
                vec!["resume".into(), "019-abc".into(), "--dangerously-bypass-approvals-and-sandbox".into()]
            )
        );
    }

    #[test]
    fn scrub_rules() {
        use crate::host::CredentialPolicy::{PassThrough, Subscription};
        for kind in [SessionKind::Shell, SessionKind::AgentTui] {
            for creds in [Subscription, PassThrough] {
                for v in [
                    "PDT_PROFILE",
                    "PDT_PORT",
                    "PDT_DATA_DIR",
                    "PDT_WEB_DIR",
                    "PDT_ALLOWED_HOSTS",
                    "PDT_OWNER_LOGIN",
                    "PDT_SESSION_ID",
                    "PDT_HOOK_PORT",
                    "PDT_RUN_DATA",
                    "PDT_ANYTHING_NEW",
                ] {
                    assert!(is_scrubbed(kind, v, creds), "{kind:?}/{creds:?} keeps {v}");
                }
                for v in [
                    "NO_COLOR",
                    "FORCE_COLOR",
                    "CLAUDECODE",
                    "CLAUDE_PID",
                    // A running Claude Code session's own variables (pdtd started from inside one).
                    "CLAUDE_CODE_ENTRYPOINT",
                    "CLAUDE_CODE_SESSION_ID",
                    "CLAUDE_CODE_CHILD_SESSION",
                    "CLAUDE_CODE_SESSION_ATTENDED",
                    "CLAUDE_CODE_MESSAGING_SOCKET",
                    "CLAUDE_CODE_MESSAGING_TOKEN",
                ] {
                    assert!(is_scrubbed(kind, v, creds), "{kind:?}/{creds:?} keeps {v}");
                }
                // User settings, not session state: they must reach the agent.
                for v in [
                    "PATH",
                    "TERM",
                    "PDT",
                    "XPDT_PORT",
                    "CLAUDE_CONFIG_DIR",
                    "CLAUDE_CODE_OAUTH_TOKEN",
                    "CLAUDE_CODE_USE_BEDROCK",
                ] {
                    assert!(!is_scrubbed(kind, v, creds), "{kind:?} drops {v}");
                }
            }
        }
        assert!(is_scrubbed(SessionKind::AgentTui, "ANTHROPIC_API_KEY", Subscription));
        assert!(is_scrubbed(SessionKind::AgentTui, "OPENAI_API_KEY", Subscription));
        assert!(!is_scrubbed(SessionKind::AgentTui, "ANTHROPIC_API_KEY", PassThrough));
        assert!(!is_scrubbed(SessionKind::AgentTui, "OPENAI_API_KEY", PassThrough));
        assert!(!is_scrubbed(SessionKind::Shell, "ANTHROPIC_API_KEY", Subscription));
    }

    #[test]
    fn terminal_env_matches_xterm() {
        let env = terminal_env();
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("TERM"), Some("xterm-256color"));
        assert_eq!(get("COLORTERM"), Some("truecolor"));
        assert_eq!(get("FORCE_HYPERLINK"), Some("1"));
    }

    fn new(kind: SessionKind, agent: Option<AgentKind>, cols: u16, rows: u16) -> NewSession {
        NewSession {
            project_id: " p1 ".into(),
            worktree_id: Some("  ".into()),
            kind,
            agent,
            bypass_permissions: None,
            cols,
            rows,
            tab: None,
            initial_prompt: None,
            mode: None,
            model: None,
            permission_mode: None,
        }
    }

    #[test]
    fn agents_default_to_conversations() {
        use crate::conversation::{PermissionMode, SessionMode};
        let v = new(SessionKind::AgentTui, Some(AgentKind::Claude), 80, 24).validate().unwrap();
        assert_eq!((v.mode, v.permission_mode), (SessionMode::Conversation, Some(PermissionMode::Ask)));
        let mut bypass = new(SessionKind::AgentTui, Some(AgentKind::Claude), 80, 24);
        bypass.bypass_permissions = Some(true);
        assert_eq!(bypass.validate().unwrap().permission_mode, Some(PermissionMode::Bypass));
        let mut plan = new(SessionKind::AgentTui, Some(AgentKind::Claude), 80, 24);
        plan.permission_mode = Some(PermissionMode::Bypass);
        plan.model = Some(" opus ".into());
        let v = plan.validate().unwrap();
        assert!(v.bypass_permissions, "bypass mode and the bypass flag agree");
        assert_eq!(v.model.as_deref(), Some("opus"));
        plan.model = Some("two words".into());
        assert_eq!(plan.validate().unwrap_err(), SessionValidationError::BadModel);
        let mut term = new(SessionKind::AgentTui, Some(AgentKind::Claude), 80, 24);
        term.mode = Some(SessionMode::Terminal);
        term.model = Some("opus".into());
        let v = term.validate().unwrap();
        assert_eq!((v.mode, v.permission_mode, v.model), (SessionMode::Terminal, None, None));
        let codex = new(SessionKind::AgentTui, Some(AgentKind::Codex), 80, 24).validate().unwrap();
        assert_eq!(codex.mode, SessionMode::Conversation);
        let mut codex = new(SessionKind::AgentTui, Some(AgentKind::Codex), 80, 24);
        codex.mode = Some(SessionMode::Terminal);
        assert_eq!(codex.validate().unwrap().mode, SessionMode::Terminal);
        let mut shell = new(SessionKind::Shell, None, 80, 24);
        assert_eq!(shell.clone().validate().unwrap().mode, SessionMode::Terminal);
        shell.mode = Some(SessionMode::Conversation);
        assert_eq!(shell.validate().unwrap_err(), SessionValidationError::ConversationForShell);
    }

    #[test]
    fn validate_session_input() {
        let v = new(SessionKind::Shell, None, 80, 24).validate().unwrap();
        assert_eq!(v.project_id, "p1");
        assert_eq!(v.worktree_id, None, "blank worktree id means the repo root");
        assert_eq!(v.size, TerminalSize { cols: 80, rows: 24 });
        // Bypass rides through for an agent, but a shell never carries it.
        let mut agent = new(SessionKind::AgentTui, Some(AgentKind::Claude), 80, 24);
        agent.bypass_permissions = Some(true);
        assert!(agent.validate().unwrap().bypass_permissions);
        let mut shell = new(SessionKind::Shell, None, 80, 24);
        shell.bypass_permissions = Some(true);
        assert!(!shell.validate().unwrap().bypass_permissions, "a shell drops bypass");
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
    fn initial_prompts_are_for_agents_and_bounded() {
        let mut agent = new(SessionKind::AgentTui, Some(AgentKind::Codex), 80, 24);
        agent.initial_prompt = Some("Fix it\nplease".into());
        assert_eq!(agent.validate().unwrap().initial_prompt.as_deref(), Some("Fix it\nplease"));
        agent.initial_prompt = Some(" \n ".into());
        assert_eq!(agent.validate().unwrap_err(), SessionValidationError::BadPrompt);
        agent.initial_prompt = Some("x".repeat(MAX_INITIAL_PROMPT));
        assert!(agent.validate().is_ok());
        agent.initial_prompt = Some("x".repeat(MAX_INITIAL_PROMPT + 1));
        assert_eq!(agent.validate().unwrap_err(), SessionValidationError::BadPrompt);
        let mut shell = new(SessionKind::Shell, None, 80, 24);
        shell.initial_prompt = Some("ls".into());
        assert_eq!(shell.validate().unwrap_err(), SessionValidationError::PromptForShell);
    }

    #[test]
    fn the_prompt_is_the_last_argument_after_the_agent_flags() {
        let p = platform(false, false, None);
        let mut l = launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, &Resume::Fresh, SID, true);
        l.args.extend(["--settings".to_string(), "/d/s.json".to_string()]);
        l.args.push(prompt_arg("PDT-4: Fix login\n\nSteps & more"));
        assert_eq!(
            l.args,
            [
                "--session-id",
                SID,
                "--dangerously-skip-permissions",
                "--settings",
                "/d/s.json",
                "PDT-4: Fix login\n\nSteps & more"
            ]
        );
        let mut l = launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, &Resume::Fresh, SID, true);
        l.args.push(prompt_arg("--help me"));
        assert_eq!(l.args, ["--dangerously-bypass-approvals-and-sandbox", " --help me"]);
    }

    #[test]
    fn cmd_shims_get_a_short_prompt_naming_the_file() {
        let file = r"C:\Users\me\AppData\Local\pdt\sessions\0199\prompt.md";
        assert_eq!(
            shim_prompt(Some("PDT-4"), file).unwrap(),
            format!("PDT-4: read {file} for the ticket, then start.")
        );
        assert_eq!(shim_prompt(None, file).unwrap(), format!("Read {file} and follow it."));
        for bad in [
            "C:\\a&b\\p.md",
            "C:\\a|b",
            "C:\\a<b",
            "C:\\a>b",
            "C:\\a^b",
            "C:\\100%\\p",
            "C:\\a!b",
            "C:\\\"q\"",
            "C:\\(x)",
            "a\nb",
        ] {
            assert_eq!(shim_prompt(Some("PDT-4"), bad).unwrap(), "Start on ticket PDT-4 (see PDT).", "{bad:?}");
            assert_eq!(shim_prompt(None, bad), None, "{bad:?}");
        }
        // Spaces are fine: cmd passes a quoted argument through.
        assert!(shim_prompt(None, r"C:\Users\Ross West\p.md").unwrap().contains("Ross West"));
    }

    #[test]
    fn pty_messages_wire_format() {
        assert_eq!(serde_json::to_string(&PtyServerMessage::Exit { code: 3 }).unwrap(), r#"{"type":"exit","code":3}"#);
        assert_eq!(serde_json::to_string(&PtyServerMessage::Ready).unwrap(), r#"{"type":"ready"}"#);
        assert_eq!(serde_json::to_string(&PtyServerMessage::NotRunning).unwrap(), r#"{"type":"not_running"}"#);
        let m: PtyClientMessage = serde_json::from_str(r#"{"type":"resize","cols":100,"rows":30}"#).unwrap();
        assert_eq!(m, PtyClientMessage::Resize { cols: 100, rows: 30 });
    }

    #[test]
    fn titles() {
        assert_eq!(kind_label(SessionKind::Shell, None), "Shell");
        assert_eq!(kind_label(SessionKind::AgentTui, Some(AgentKind::Codex)), "Codex");
        assert_eq!(kind_label(SessionKind::AgentTui, Some(AgentKind::Claude)), "Claude");
        assert_eq!(default_title("feature-x"), "feature-x");
    }

    #[test]
    fn title_cleaning() {
        assert_eq!(clean_title("  hi  ").as_deref(), Some("hi"));
        assert_eq!(clean_title("   "), None);
        assert_eq!(clean_title(&"x".repeat(100)).map(|t| t.len()), Some(100));
        assert_eq!(clean_title(&"x".repeat(101)), None);
        assert_eq!(clean_title(&"é".repeat(100)).map(|t| t.chars().count()), Some(100));
    }
}
