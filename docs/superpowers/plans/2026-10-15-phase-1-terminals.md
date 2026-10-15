# Phase 1 — Terminals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real terminals inside PDT. Shells and interactive `claude` / `codex` sessions run as PTYs owned by `pdtd`, in a project's repo or in a git worktree. They survive closed windows and stream to any number of xterm.js views over WebSocket. A stable install procedure lets PDT be developed inside PDT.

**Architecture:**
- **`pdt-agents` (new crate):** holds the PTY manager (`portable-pty` plus threads), with a scrollback ring buffer and a broadcast channel per session.
- **`pdt-core`:** gains pure session and worktree types and the launch-command rules.
- **`pdt-git`:** gains worktree add and an exclude helper (removal comes with merge in Phase 4).
- **`pdt-store`:** gains tables for worktrees and sessions.
- **`pdt-api`:** gains REST routes plus a WebSocket byte stream per session, guarded by the existing Host allowlist and a new Origin check.
- **`pdtd`:**
  - writes logs to a file;
  - marks sessions that were running at the last stop as `interrupted`;
  - shuts down gracefully (signal or `POST /api/shutdown`).
- **Web:** gains `PtySocket` (reconnecting WebSocket protocol), `TerminalView` (xterm) and the session list. The bottom panel becomes real terminal tabs.
- **Desktop:** checks `/api/health` before attaching, logs `pdtd`'s startup errors, and shows an error dialog.

**Tech stack:**
- **Existing:** everything from Phase 0.
- **New Rust crates:** `portable-pty` 0.9, `which`, `tracing-appender`, `futures-util`, `tokio-tungstenite` (tests only), `tauri-plugin-dialog` 2.x, and axum's `ws` feature.
- **New web packages:** `@xterm/xterm` 6.0.0 and `@xterm/addon-fit`.

**Spec:** `docs/superpowers/specs/2026-10-14-personal-dev-team-design.md`: §3.1, §4 (Session), §5.1, §5.5, §8, §9, and §11 (Phase 1 and the stable/dev installs). Also read `docs/superpowers/notes/phase-0-carryover.md`; this plan covers every item in its "Must address" section.

**Branch:** `phase-1-terminals` (already created from `main`).

## Decisions this plan makes

The spec leaves these open. Each one is recorded here so it can be reviewed and reversed.

1. **Worktrees are first-class records** (`worktrees` table) owned by a project. In Phase 2, an Attempt will point at a worktree instead of owning the path itself.
2. **Worktree location:** `<repo>/.worktrees/<branch with / → ->`, inside the repo directory. `add_worktree` also adds `/.worktrees/` to the repo's `.git/info/exclude`, a local file that is never committed. That keeps worktrees out of the main checkout's `git status` and out of searches in the main checkout (`rg` and agents respect the exclude file), for any repo.
3. **Worktrees are removed only after their branch is merged.** Merging arrives with review (Phase 4, spec §7.3: Merge removes the worktree). Phase 1 therefore has no remove route, button or git function. When removal arrives, it must still refuse a dirty worktree (spec §8). Abandoned worktrees get their cleanup with Attempts (Phase 2 or 4).
4. **Session owner in Phase 1:** a `project_id` plus an optional `worktree_id`. Phase 2 adds the attempt.
5. **Session statuses used in Phase 1:** `running`, `finished` (the process exited; `exit_code` recorded) and `interrupted` (`pdtd` stopped while it was running). The other spec statuses arrive with structured agents.
6. **Restart** reuses the same session id and working folder. For agent terminals it resumes the previous conversation: `claude --continue`, or `codex resume --last`.
7. **Environment of PTY children:**
   - `PDT_PROFILE`, `PDT_PORT`, `PDT_DATA_DIR`, `PDT_WEB_DIR` and `PDT_ALLOWED_HOSTS` are always removed. Otherwise a `cargo run -p pdtd` typed into a terminal hosted by the stable `pdtd` would inherit the stable port.
   - `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are removed for agent terminals (spec §5.5).
   - `TERM=xterm-256color` and `COLORTERM=truecolor` are set.
8. **Shell:**
   - Windows: `pwsh -NoLogo` when `pwsh` is on PATH, otherwise `powershell.exe -NoLogo`.
   - macOS: `$SHELL -l`, falling back to `/bin/zsh -l`.
   - On Windows, `.cmd` and `.bat` shims (npm installs) are run through `%ComSpec% /d /c`.
9. **WebSocket protocol:**
   - Server to client: binary frames carry output, starting with the scrollback. A text frame `{"type":"exit","code":N}` reports the exit. Close code 4000 means "you fell behind; reconnect".
   - Client to server: binary frames carry input. Text frames carry JSON control messages (`{"type":"resize","cols","rows"}`).
10. **Live status in the UI** comes from polling the session list every 2 s. The push event stream arrives with structured agents (Phase 3).
11. **Origin check:** a request carrying an `Origin` header must have that origin's `host[:port]` in the Host allowlist. A missing Origin is allowed; `null` and foreign origins get 403 `forbidden_origin`. The dev profile also allows the Vite dev server (`localhost:5173`, `127.0.0.1:5173`). `[::1]:<port>` joins the default list.
12. **Shutdown endpoint:** `POST /api/shutdown` triggers a graceful stop. It is the only reliable way to stop a windowless `pdtd` gracefully on Windows. It sits behind the Host and Origin guards; device tokens come in Phase 2.
13. **Stable install is a real installer.** Tauri builds an NSIS installer (Windows, per-user, no admin) and a `.app` (macOS).
    - **What's bundled:** `pdtd` as a Tauri `externalBin` sidecar, which lands next to the desktop exe, and `web/dist` as a bundled resource.
    - **Where the bundle settings live:** in `desktop/src-tauri/tauri.bundle.conf.json`, merged only by `tauri build --config …`. Ordinary `cargo build`, dev runs and CI never need the sidecar staged.
    - **Release scripts** (`scripts/release-stable.ps1` and `.sh`):
      1. Build the installer.
      2. Stop the stable `pdtd` through `POST /api/shutdown` (an installer can't replace a running exe).
      3. Install silently: NSIS `/S` on Windows, or copy the `.app` into `~/Applications` on macOS.
      4. Launch the app.
    - **Signing:** the installer is unsigned. Locally built files carry no download mark, so SmartScreen and Gatekeeper don't block them. Signing and the Tauri updater come with public distribution.
    - **Naming exception:** the scripts repeat the slug, port and binary names, an accepted exception to "names only in `brand.rs`", like `tauri.conf.json`.
14. **No Playwright yet.** End-to-end coverage comes from a real-socket WebSocket test (Task 7) and a `pdtd` process lifecycle test (Task 8). Playwright arrives with `fake-agent` in Phase 3.

## Global Constraints

- **Toolchain:** Rust is pinned to `1.99.0`, edition `2024`. Run cargo from **PowerShell** on Windows.
- **New dependencies:** add them with `cargo add` or `pnpm add` so they resolve to the latest version that day. Never type a version from memory.
  - Exception: `tauri-plugin-dialog` must be pinned to the **2.x** line (`cargo add tauri-plugin-dialog@2`), because the latest release is a 3.0 alpha.
- **Errors:** library crates use `thiserror`; only `pdtd` uses `anyhow`. API errors are always `{ "code": string, "message": string }` JSON.
- **Listening and names:**
  - `pdtd` listens only on `127.0.0.1`.
  - Names live in `crates/pdt-core/src/brand.rs`. The only exceptions are the Tauri config files (`tauri.conf.json`, `tauri.bundle.conf.json`) and `scripts/release-stable.*`.
- **Tests must never touch port 7420 or the real data folders.** Use port 0 or a free port, plus temporary folders. Any test that starts `pdtd` removes every inherited `PDT_*` variable.
- **Colors** come only from `web/ui/tokens.css`; xterm's theme reads those tokens at runtime.
- **Timestamps** are `i64` Unix milliseconds, exported to TypeScript as `number`.
- **Generated TypeScript types:** `cargo test` regenerates `web/src/api/gen`; commit the results.
- **Platforms:** everything builds and passes on Windows and macOS. PTY tests must run on both. Windows uses ConPTY; expect escape sequences around output, so assert with `contains`.
- **TDD** applies to `pdt-core`, `pdt-git`, `pdt-store`, `pdt-agents` and `pdt-api`.

## Review Focus

1. **A dev `pdtd` started from a terminal that the stable `pdtd` hosts** must not inherit `PDT_PORT` or `PDT_PROFILE`. Pinned in Task 1 (`scrubbed_env_always_removes_pdt_vars`) and Task 6 (the spawn spec is built with `scrubbed_env`).
2. **Reconnecting after a dropped connection or a lag** must not duplicate or corrupt the screen. The client resets the terminal on each new connection before the scrollback replays. Pinned in Task 9 (`resets_before_replay_on_reconnect`) and Task 7 (`second_client_gets_scrollback`).
3. **Starting an agent that isn't installed** (for example `codex` missing on CI) returns 400 `program_not_found` with a readable message and leaves no orphan session row. Pinned in Task 6 (`missing_program_is_400_and_leaves_no_row`).
4. **`pdtd` stopping while sessions run** leaves them `interrupted` after the next start, and they can be restarted. Pinned in Task 8 (`shutdown_marks_running_sessions_interrupted`) and Task 6 (`restart_finished_session`).
5. **A worktree inside the repo** must not appear as untracked in the main checkout's `git status`, even on the first worktree of a fresh repo. Creating a second worktree must not add a second exclude line. Pinned in Task 2 (`worktrees_are_excluded_from_the_main_checkout`).

---

### Task 1: Core session and worktree types

**Files:**
- Create: `crates/pdt-core/src/session.rs`, `crates/pdt-core/src/worktree.rs`
- Modify: `crates/pdt-core/src/lib.rs`, `crates/pdt-core/Cargo.toml` (dev-dependency `serde_json`)

**Interfaces:**
- Produces (`pdt_core::session`):
  - Enums: `SessionKind { Shell, AgentTui }`, `AgentKind { Claude, Codex }`, `SessionStatus { Running, Finished, Interrupted }`.
    - Each has `as_str()` and `parse(&str) -> Option<Self>`, using snake_case strings identical to their serde form.
  - Structs: `Session`, `NewSession`, `ValidSession`, `TerminalSize { cols, rows }` (with `validate()`), `SessionValidationError`.
  - Constant `MAX_TERMINAL_DIM = 1000`.
  - `Platform { windows, has_pwsh, unix_shell, claude_program, codex_program }` and `Launch { program, args }`.
  - Functions: `launch(kind, agent, &Platform, resume) -> Launch`, `default_title(kind, agent) -> &'static str`, `scrubbed_env(kind) -> Vec<&'static str>`.
  - Messages: `PtyClientMessage::Resize { cols, rows }` and `PtyServerMessage::Exit { code }`.
- Produces (`pdt_core::worktree`): `Worktree`, `NewWorktree`, `ValidWorktree`, `WorktreeValidationError`, `WORKTREES_DIR = ".worktrees"`, and `worktree_path(repo_root: &Path, branch: &str) -> PathBuf`.

- [ ] **Step 1: Add the dev-dependency and module declarations**

Run (PowerShell, repo root): `cargo add serde_json --dev -p pdt-core`

`crates/pdt-core/src/lib.rs`:

```rust
//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod brand;
pub mod project;
pub mod session;
pub mod worktree;
```

- [ ] **Step 2: Write the failing tests for `session.rs`**

Create `crates/pdt-core/src/session.rs` with just the test module, then fill in the types in Step 4:

```rust
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
        assert_eq!(launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, false)), ("claude".into(), vec![]));
        assert_eq!(
            launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Claude), &p, true)),
            ("claude".into(), vec!["--continue".into()])
        );
        assert_eq!(launch_of(launch(SessionKind::AgentTui, Some(AgentKind::Codex), &p, false)), ("codex".into(), vec![]));
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
```

- [ ] **Step 3: Run the tests and confirm they fail to compile**

Run: `cargo test -p pdt-core`
Expected: compile errors (the types aren't defined yet).

- [ ] **Step 4: Implement `session.rs`, above the test module**

```rust
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
        [SessionStatus::Running, SessionStatus::Finished, SessionStatus::Interrupted].into_iter().find(|k| k.as_str() == s)
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
            program: platform
                .unix_shell
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/zsh".into()),
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
    Exit { code: i32 },
}
```

- [ ] **Step 5: Write the failing tests for `worktree.rs`, then implement it**

`crates/pdt-core/src/worktree.rs`:

```rust
//! Git worktree records. Validation here is a fast pre-check; `git check-ref-format` is the final word.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Worktree {
    pub id: String,
    pub project_id: String,
    pub branch: String,
    /// Absolute path, native separators.
    pub path: String,
    pub base_commit: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewWorktree {
    pub branch: String,
    /// Branch or commit to start from; defaults to the project's base branch.
    #[serde(default)]
    #[ts(optional)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidWorktree {
    pub branch: String,
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WorktreeValidationError {
    #[error("branch name must not be empty")]
    EmptyBranch,
    #[error("invalid branch name: {0}")]
    InvalidBranch(String),
}

impl NewWorktree {
    pub fn validate(&self) -> Result<ValidWorktree, WorktreeValidationError> {
        let branch = self.branch.trim();
        if branch.is_empty() {
            return Err(WorktreeValidationError::EmptyBranch);
        }
        let bad = branch.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
            || branch.starts_with('-')
            || branch.starts_with('/')
            || branch.ends_with('/')
            || branch.ends_with('.')
            || branch.ends_with(".lock")
            || branch.contains("..")
            || branch.contains("//")
            || branch.contains("@{");
        if bad {
            return Err(WorktreeValidationError::InvalidBranch(branch.into()));
        }
        let base = self.base.as_deref().map(str::trim).filter(|b| !b.is_empty()).map(String::from);
        Ok(ValidWorktree { branch: branch.into(), base })
    }
}

/// Directory (relative to the repo root) that holds a project's worktrees. pdt-git keeps it in `.git/info/exclude`.
pub const WORKTREES_DIR: &str = ".worktrees";

/// `<repo>/.worktrees/<branch, with '/' replaced by '-'>`.
pub fn worktree_path(repo_root: &Path, branch: &str) -> PathBuf {
    repo_root.join(WORKTREES_DIR).join(branch.replace('/', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nw(branch: &str, base: Option<&str>) -> NewWorktree {
        NewWorktree { branch: branch.into(), base: base.map(Into::into) }
    }

    #[test]
    fn trims_and_defaults_base() {
        assert_eq!(
            nw(" feature/x ", Some(" ")).validate().unwrap(),
            ValidWorktree { branch: "feature/x".into(), base: None }
        );
        assert_eq!(nw("a", Some("dev")).validate().unwrap().base.as_deref(), Some("dev"));
    }

    #[test]
    fn rejects_bad_branch_names() {
        assert_eq!(nw("  ", None).validate(), Err(WorktreeValidationError::EmptyBranch));
        for bad in ["has space", "-flag", "a..b", "x.lock", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", "a@{b", "a/", "/a", "a//b", "a."] {
            assert!(matches!(nw(bad, None).validate(), Err(WorktreeValidationError::InvalidBranch(_))), "{bad}");
        }
    }

    #[test]
    fn path_is_inside_the_repo() {
        let repo = Path::new("work").join("my repo");
        assert_eq!(worktree_path(&repo, "feature/x"), repo.join(".worktrees").join("feature-x"));
    }
}
```

- [ ] **Step 6: Run the tests and confirm they pass; check the generated TS files**

Run: `cargo test -p pdt-core`
Expected: all tests pass. These new files appear in `web/src/api/gen/`: `Session.ts`, `SessionKind.ts`, `AgentKind.ts`, `SessionStatus.ts`, `NewSession.ts`, `TerminalSize.ts`, `PtyClientMessage.ts`, `PtyServerMessage.ts`, `Worktree.ts`, `NewWorktree.ts`.

Run: `cargo clippy -p pdt-core --all-targets -- -D warnings` and `cargo fmt`.

- [ ] **Step 7: Commit**

```bash
git add crates/pdt-core web/src/api/gen Cargo.lock
git commit -m "feat(core): session, worktree and launch types"
```

---

### Task 2: Git worktrees

**Files:**
- Modify: `crates/pdt-git/src/lib.rs`, `crates/pdt-api/src/error.rs` (new `GitError` variants must be mapped, or `pdt-api` stops compiling)

**Interfaces:**
- Consumes: `git_with_env`, `Failed`, `GitError` (existing, in `pdt-git`).
- Produces:
  - `pdt_git::check_branch_name(repo: &Path, branch: &str) -> Result<(), GitError>`
  - `pdt_git::add_worktree(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<String /* base commit sha */, GitError>`
  - `pdt_git::ensure_excluded(repo: &Path, pattern: &str) -> Result<(), GitError>`: adds a line to the repository's shared `info/exclude` once.
  - New `GitError` variants: `InvalidBranch(String)`, `BranchExists(String)`, `UnknownRef(String)`.
  - API mapping:

    | Variant | Status | Code |
    |---|---|---|
    | `InvalidBranch` | 400 | `invalid_branch` |
    | `BranchExists` | 409 | `branch_exists` |
    | `UnknownRef` | 400 | `unknown_ref` |

  - Not in this phase: removing worktrees (decision 3).

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `crates/pdt-git/src/lib.rs`)

```rust
    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repo on `main` with one commit; returns the HEAD sha.
    fn repo_with_commit(dir: &Path) -> String {
        std::fs::create_dir_all(dir).unwrap();
        init_repo(dir);
        git_ok(dir, &["commit", "--allow-empty", "-m", "init", "-q"]);
        git_ok(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn add_worktree_creates_branch_at_base() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let head = repo_with_commit(&repo);
        let wt = repo.join(".worktrees").join("feature-x");
        let sha = add_worktree(&repo, &wt, "feature/x", "main").unwrap();
        assert_eq!(sha, head);
        let info = resolve_repo(&wt).unwrap();
        assert_eq!(info.current_branch.as_deref(), Some("feature/x"));
        assert!(same_dir(&info.root, &wt));
    }

    #[test]
    fn add_worktree_errors_are_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        let wt = |n: &str| tmp.path().join("wts").join(n);
        assert!(matches!(add_worktree(&repo, &wt("a"), "main", "main"), Err(GitError::BranchExists(b)) if b == "main"));
        assert!(matches!(add_worktree(&repo, &wt("b"), "b", "no-such-ref"), Err(GitError::UnknownRef(r)) if r == "no-such-ref"));
        assert!(matches!(add_worktree(&repo, &wt("c"), "bad..name", "main"), Err(GitError::InvalidBranch(_))));
    }

    #[test]
    fn worktrees_are_excluded_from_the_main_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        add_worktree(&repo, &repo.join(".worktrees").join("a"), "a", "main").unwrap();
        add_worktree(&repo, &repo.join(".worktrees").join("b"), "b", "main").unwrap();
        assert_eq!(git_ok(&repo, &["status", "--porcelain"]), "", "main checkout stays clean");
        let exclude = std::fs::read_to_string(repo.join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(exclude.lines().filter(|l| l.trim() == "/.worktrees/").count(), 1);
    }

    #[test]
    fn ensure_excluded_keeps_existing_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        repo_with_commit(&repo);
        let file = repo.join(".git").join("info").join("exclude");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "*.log").unwrap(); // no trailing newline
        ensure_excluded(&repo, "/.worktrees/").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "*.log\n/.worktrees/\n");
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p pdt-git`
Expected: compile errors (the functions and variants don't exist yet).

- [ ] **Step 3: Implement**

Add these variants to `GitError`:

```rust
    #[error("invalid branch name: {0}")]
    InvalidBranch(String),
    #[error("a branch named {0} already exists")]
    BranchExists(String),
    #[error("unknown base branch or commit: {0}")]
    UnknownRef(String),
```

Add these functions (after `classify_failure`):

```rust
/// Checks a branch name with `git check-ref-format --branch`.
pub fn check_branch_name(repo: &Path, branch: &str) -> Result<(), GitError> {
    match git_with_env(repo, &["check-ref-format", "--branch", branch], &[])? {
        Ok(_) => Ok(()),
        Err(_) => Err(GitError::InvalidBranch(branch.to_string())),
    }
}

/// Creates `path` as a new worktree on new branch `branch`, starting at `base`. Returns the base commit SHA.
pub fn add_worktree(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<String, GitError> {
    check_branch_name(repo, branch)?;
    let spec = format!("{base}^{{commit}}");
    let sha = match git_with_env(repo, &["rev-parse", "--verify", "--quiet", &spec], &[])? {
        Ok(s) if !s.is_empty() => s,
        _ => return Err(GitError::UnknownRef(base.to_string())),
    };
    if path.exists() {
        return Err(GitError::Command(format!("worktree path already exists: {}", path.display())));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| GitError::Command(format!("creating {}: {e}", parent.display())))?;
    }
    let path_str = path.to_string_lossy();
    match git_with_env(repo, &["worktree", "add", "-b", branch, &path_str, &sha], &[])? {
        Ok(_) => Ok(sha),
        Err(f) if f.stderr.contains("already exists") && f.stderr.contains("branch") => {
            Err(GitError::BranchExists(branch.to_string()))
        }
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}

/// Adds `pattern` as a line to the repository's `info/exclude` (shared by all its worktrees) unless it is
/// already there. This is a local ignore file that is never committed.
pub fn ensure_excluded(repo: &Path, pattern: &str) -> Result<(), GitError> {
    let common = match git_with_env(repo, &["rev-parse", "--git-common-dir"], &[])? {
        Ok(s) => PathBuf::from(native_separators(s)),
        Err(f) => return Err(classify_failure(repo, f)),
    };
    let common = if common.is_absolute() { common } else { repo.join(common) };
    let file = common.join("info").join("exclude");
    let io = |e: std::io::Error| GitError::Command(format!("updating {}: {e}", file.display()));
    let mut text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    if text.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(&file, text).map_err(io)
}
```

In `crates/pdt-api/src/error.rs`, extend the `From<GitError>` match:

```rust
            GitError::InvalidBranch(_) => Self::new(StatusCode::BAD_REQUEST, "invalid_branch", e.to_string()),
            GitError::BranchExists(_) => Self::new(StatusCode::CONFLICT, "branch_exists", e.to_string()),
            GitError::UnknownRef(_) => Self::new(StatusCode::BAD_REQUEST, "unknown_ref", e.to_string()),
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p pdt-git; cargo test -p pdt-api; cargo clippy --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-git crates/pdt-api/src/error.rs
git commit -m "feat(git): create worktrees and keep them out of the main checkout"
```

---

### Task 3: Store worktrees and sessions

**Files:**
- Create: `crates/pdt-store/migrations/0002_worktrees_sessions.sql`, `crates/pdt-store/src/worktrees.rs`, `crates/pdt-store/src/sessions.rs`
- Modify: `crates/pdt-store/src/lib.rs` (modules), `crates/pdt-store/src/error.rs` (`Corrupt` variant), `crates/pdt-store/src/migrate.rs` (backup filename collisions, a carry-over item)

**Interfaces:**
- Consumes: `Worktree`, `Session`, `SessionKind`, `AgentKind` and `SessionStatus` from Task 1.
- Produces (all on `Store`, all `async`, all returning `Result<_, StoreError>`):
  - `insert_worktree(&Worktree)`, `list_worktrees(project_id) -> Vec<Worktree>`, `get_worktree(id) -> Option<Worktree>` (no delete; removal comes with merge in Phase 4)
  - `insert_session(&Session)`, `list_sessions(project_id) -> Vec<Session>` (oldest first), `get_session(id) -> Option<Session>`
  - `end_session(id, status: SessionStatus, exit_code: Option<i32>, ended_at: i64) -> bool`: updates only a row that is currently `running`.
  - `set_session_running(id) -> bool`: clears `exit_code` and `ended_at`.
  - `delete_session(id) -> bool`
  - `mark_running_interrupted(ended_at) -> u64`
  - `StoreError::Corrupt(String)`
  - A unique `(project_id, branch)` or `path` gives `StoreError::Conflict`.

- [ ] **Step 1: Add the migration** (never edit `0001`)

`crates/pdt-store/migrations/0002_worktrees_sessions.sql`:

```sql
CREATE TABLE worktrees (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    branch      TEXT NOT NULL,
    path        TEXT NOT NULL UNIQUE,
    base_commit TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE (project_id, branch)
);

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    agent       TEXT,
    title       TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    status      TEXT NOT NULL,
    exit_code   INTEGER,
    created_at  INTEGER NOT NULL,
    ended_at    INTEGER
);

CREATE INDEX sessions_by_project ON sessions (project_id, created_at);
```

- [ ] **Step 2: Write the failing tests**

Add the `mod tests` blocks below to the new files. Create each file with only `use` lines plus the tests first.

`crates/pdt-store/src/worktrees.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use crate::{Store, StoreError};
    use pdt_core::project::Project;
    use pdt_core::worktree::Worktree;

    pub(crate) async fn store_with_project(dir: &std::path::Path) -> Store {
        let store = Store::open(&dir.join("pdt.db")).await.unwrap();
        let p = Project { id: "p".into(), name: "P".into(), repo_path: "/r".into(), base_branch: "main".into(), created_at: 1 };
        store.insert_project(&p).await.unwrap();
        store
    }

    fn wt(id: &str, branch: &str) -> Worktree {
        Worktree {
            id: id.into(),
            project_id: "p".into(),
            branch: branch.into(),
            path: format!("/r/.worktrees/{branch}"),
            base_commit: "abc".into(),
            created_at: 2,
        }
    }

    #[tokio::test]
    async fn worktree_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_worktree(&wt("w1", "a")).await.unwrap();
        assert_eq!(store.list_worktrees("p").await.unwrap(), vec![wt("w1", "a")]);
        assert_eq!(store.get_worktree("w1").await.unwrap(), Some(wt("w1", "a")));
        assert_eq!(store.get_worktree("nope").await.unwrap(), None);
        assert!(store.list_worktrees("other").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_branch_is_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_worktree(&wt("w1", "a")).await.unwrap();
        let mut dup = wt("w2", "a");
        dup.path = "/elsewhere".into();
        assert!(matches!(store.insert_worktree(&dup).await, Err(StoreError::Conflict(_))));
    }
}
```

`crates/pdt-store/src/sessions.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use crate::StoreError;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

    fn session(id: &str, created_at: i64) -> Session {
        Session {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Codex),
            title: "Codex".into(),
            cwd: "C:\\r é".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at,
            ended_at: None,
        }
    }

    #[tokio::test]
    async fn session_roundtrip_in_creation_order() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("b", 20)).await.unwrap();
        store.insert_session(&session("a", 10)).await.unwrap();
        let ids: Vec<_> = store.list_sessions("p").await.unwrap().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(store.get_session("a").await.unwrap(), Some(session("a", 10)));
        assert_eq!(store.get_session("zz").await.unwrap(), None);
    }

    #[tokio::test]
    async fn end_session_only_touches_running_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert!(store.end_session("a", SessionStatus::Finished, Some(3), 99).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.exit_code, s.ended_at), (SessionStatus::Finished, Some(3), Some(99)));
        assert!(!store.end_session("a", SessionStatus::Interrupted, None, 100).await.unwrap(), "already ended");
        assert!(store.set_session_running("a").await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.exit_code, s.ended_at), (SessionStatus::Running, None, None));
    }

    #[tokio::test]
    async fn mark_running_interrupted_counts_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.insert_session(&session("b", 2)).await.unwrap();
        store.end_session("b", SessionStatus::Finished, Some(0), 5).await.unwrap();
        assert_eq!(store.mark_running_interrupted(7).await.unwrap(), 1);
        let a = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((a.status, a.ended_at), (SessionStatus::Interrupted, Some(7)));
        assert_eq!(store.get_session("b").await.unwrap().unwrap().status, SessionStatus::Finished);
    }

    #[tokio::test]
    async fn delete_session_and_corrupt_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert!(store.delete_session("a").await.unwrap());
        assert!(!store.delete_session("a").await.unwrap());
        sqlx::query(
            "INSERT INTO sessions (id, project_id, kind, title, cwd, status, created_at) \
             VALUES ('x', 'p', 'teleporter', 't', '/', 'running', 1)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(matches!(store.get_session("x").await, Err(StoreError::Corrupt(_))));
    }

    #[tokio::test]
    async fn session_in_a_worktree_roundtrips_and_needs_a_real_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let w = pdt_core::worktree::Worktree {
            id: "w".into(),
            project_id: "p".into(),
            branch: "b".into(),
            path: "/w".into(),
            base_commit: "c".into(),
            created_at: 1,
        };
        store.insert_worktree(&w).await.unwrap();
        let mut s = session("a", 1);
        s.worktree_id = Some("w".into());
        store.insert_session(&s).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap(), Some(s));
        let mut orphan = session("b", 2);
        orphan.worktree_id = Some("missing".into());
        assert!(store.insert_session(&orphan).await.is_err(), "foreign key enforced");
    }
}
```

Add a test to `migrate.rs`'s `mod tests`:

```rust
    #[test]
    fn backup_names_never_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let first = super::backup_file(tmp.path(), 42);
        assert_eq!(first, tmp.path().join("pdt-42.db"));
        std::fs::write(&first, "").unwrap();
        let second = super::backup_file(tmp.path(), 42);
        assert_eq!(second, tmp.path().join("pdt-42-1.db"));
    }
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test -p pdt-store`
Expected: compile errors.

- [ ] **Step 4: Implement**

`crates/pdt-store/src/error.rs`: add this variant:

```rust
    #[error("corrupt row: {0}")]
    Corrupt(String),
```

`crates/pdt-store/src/lib.rs`: add `mod sessions;` and `mod worktrees;` under `mod migrate;`. `pool` stays private; the new modules are submodules, so they can reach it.

`crates/pdt-store/src/migrate.rs`: replace the `let file = ...` line in `backup_if_pending` with `let file = backup_file(backup_dir, jiff::Timestamp::now().as_millisecond());`, and add:

```rust
/// `pdt-<ms>.db`, or `pdt-<ms>-<n>.db` when that name is taken.
pub(crate) fn backup_file(dir: &Path, stamp: i64) -> std::path::PathBuf {
    (0u32..)
        .map(|n| dir.join(if n == 0 { format!("pdt-{stamp}.db") } else { format!("pdt-{stamp}-{n}.db") }))
        .find(|p| !p.exists())
        .expect("an unused backup name exists")
}
```

`crates/pdt-store/src/worktrees.rs` (above the tests):

```rust
use crate::{Store, StoreError};
use pdt_core::worktree::Worktree;

type WorktreeRow = (String, String, String, String, String, i64);

fn to_worktree((id, project_id, branch, path, base_commit, created_at): WorktreeRow) -> Worktree {
    Worktree { id, project_id, branch, path, base_commit, created_at }
}

const COLUMNS: &str = "id, project_id, branch, path, base_commit, created_at";

impl Store {
    pub async fn insert_worktree(&self, w: &Worktree) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO worktrees (id, project_id, branch, path, base_commit, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&w.id)
        .bind(&w.project_id)
        .bind(&w.branch)
        .bind(&w.path)
        .bind(&w.base_commit)
        .bind(w.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_worktrees(&self, project_id: &str) -> Result<Vec<Worktree>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM worktrees WHERE project_id = ? ORDER BY created_at, id");
        let rows: Vec<WorktreeRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(to_worktree).collect())
    }

    pub async fn get_worktree(&self, id: &str) -> Result<Option<Worktree>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM worktrees WHERE id = ?");
        let row: Option<WorktreeRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(to_worktree))
    }
}
```

> sqlx 0.9 only accepts `&'static str` or `AssertSqlSafe(String)` as SQL (`SqlSafeStr`). `COLUMNS` is a constant, so wrapping the formatted string in `AssertSqlSafe` is sound. If `AssertSqlSafe` has a different path in 0.9.0, check `cargo doc -p sqlx --open`, or write the column list out literally in each query. Never format a user value into SQL.

`crates/pdt-store/src/sessions.rs` (above the tests):

```rust
use crate::{Store, StoreError};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "id, project_id, worktree_id, kind, agent, title, cwd, status, exit_code, created_at, ended_at";

fn to_session(r: &SqliteRow) -> Result<Session, StoreError> {
    let kind: String = r.try_get("kind")?;
    let agent: Option<String> = r.try_get("agent")?;
    let status: String = r.try_get("status")?;
    let corrupt = |what: &str, v: &str| StoreError::Corrupt(format!("unknown session {what} {v:?}"));
    Ok(Session {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        worktree_id: r.try_get("worktree_id")?,
        kind: SessionKind::parse(&kind).ok_or_else(|| corrupt("kind", &kind))?,
        agent: match agent {
            Some(a) => Some(AgentKind::parse(&a).ok_or_else(|| corrupt("agent", &a))?),
            None => None,
        },
        title: r.try_get("title")?,
        cwd: r.try_get("cwd")?,
        status: SessionStatus::parse(&status).ok_or_else(|| corrupt("status", &status))?,
        exit_code: r.try_get("exit_code")?,
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

impl Store {
    pub async fn insert_session(&self, s: &Session) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (id, project_id, worktree_id, kind, agent, title, cwd, status, exit_code, created_at, ended_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.id)
        .bind(&s.project_id)
        .bind(&s.worktree_id)
        .bind(s.kind.as_str())
        .bind(s.agent.map(AgentKind::as_str))
        .bind(&s.title)
        .bind(&s.cwd)
        .bind(s.status.as_str())
        .bind(s.exit_code)
        .bind(s.created_at)
        .bind(s.ended_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<Session>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE project_id = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_session).transpose()
    }

    /// Ends a session that is still `running`. Returns false when it was not running (already ended).
    pub async fn end_session(
        &self,
        id: &str,
        status: SessionStatus,
        exit_code: Option<i32>,
        ended_at: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET status = ?, exit_code = ?, ended_at = ? WHERE id = ? AND status = 'running'",
        )
        .bind(status.as_str())
        .bind(exit_code)
        .bind(ended_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn set_session_running(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET status = 'running', exit_code = NULL, ended_at = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_session(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM sessions WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks every `running` session as `interrupted` (pdtd stopped or restarted). Returns how many changed.
    pub async fn mark_running_interrupted(&self, ended_at: i64) -> Result<u64, StoreError> {
        let r = sqlx::query("UPDATE sessions SET status = 'interrupted', ended_at = ? WHERE status = 'running'")
            .bind(ended_at)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
}
```

`pdt-store` needs the `pdt_core::session` types, which it already gets through `pdt-core`.

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test -p pdt-store; cargo clippy --all-targets -- -D warnings`
Expected: all pass. The existing `backs_up_before_pending_migration` test still passes.

- [ ] **Step 6: Commit**

```bash
git add crates/pdt-store
git commit -m "feat(store): worktrees and sessions tables"
```

---

### Task 4: PTY manager (`pdt-agents`)

**Files:**
- Create: `crates/pdt-agents/Cargo.toml`, `crates/pdt-agents/src/lib.rs`, `crates/pdt-agents/src/pty.rs`, `crates/pdt-agents/src/launch.rs`, `crates/pdt-agents/tests/pty.rs`
- Modify: root `Cargo.toml` (add `pdt-agents` to `[workspace.dependencies]` and `default-members`)

**Interfaces:**
- Consumes: `pdt_core::session::Platform`.
- Produces (`pdt_agents`):
  - `PtyManager` (`Clone`, `Default`) with:
    - `spawn(SpawnSpec) -> Result<oneshot::Receiver<i32>, PtyError>`
    - `attach(id) -> Result<Attachment, PtyError>`
    - `write(id, &[u8])`, `resize(id, cols, rows)`, `kill(id)`, all returning `Result<(), PtyError>`
    - `is_running(id) -> bool`, `remove(id)`, `async shutdown()`
  - `SpawnSpec { id, program: PathBuf, args, cwd: PathBuf, env_remove: Vec<String>, env: Vec<(String, String)>, cols, rows }`
  - `Attachment { scrollback: Vec<u8>, exited: Option<i32>, events: broadcast::Receiver<PtyEvent> }`
  - `PtyEvent::{Output(Arc<[u8]>), Exit(i32)}`
  - `PtyError::{ProgramNotFound(String), Spawn(String), NotFound(String), AlreadyExists(String), NotRunning(String), Io(String)}`
  - `SCROLLBACK_BYTES = 512 * 1024`
  - `launch::detect_platform() -> Platform`
  - `launch::resolve(program: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), PtyError>`

**Concurrency design** (follow it exactly; this is the load-bearing part of Phase 1):
- One `Handle` per session holds:
  - `Mutex<State { scrollback, exited }>`
  - a `broadcast::Sender<PtyEvent>` (capacity 1024)
  - a `std::sync::mpsc::Sender<Vec<u8>>` for input
  - `Mutex<Option<Box<dyn MasterPty + Send>>>`
  - `Mutex<Box<dyn ChildKiller + Send + Sync>>`
- Three plain threads per session:
  - **reader:** reads 8 KiB chunks. **While holding the state lock** it appends to the scrollback and broadcasts `Output`. `attach` takes the same lock to snapshot the scrollback and subscribe, so a client never misses or duplicates bytes.
  - **writer:** drains the input channel into the PTY writer.
  - **waiter:** runs `child.wait()`, then sleeps 200 ms so the reader can flush. Under the state lock it sets `exited` and broadcasts `Exit`. Then it **drops the master** (on Windows, ConPTY doesn't EOF the reader until the master is closed) and sends the code on the oneshot.
- Drop the slave right after spawning. Take `child.clone_killer()` before moving the child into the waiter thread.
- If the `portable-pty` 0.9 API differs from the names used here (`native_pty_system`, `openpty`, `PtySize`, `CommandBuilder`, `try_clone_reader`, `take_writer`, `clone_killer`, `ExitStatus::exit_code`), adapt it and note the change in your report.

- [ ] **Step 1: Create the crate**

Run (PowerShell, repo root):

```powershell
cargo new --lib crates/pdt-agents --vcs none
cargo add -p pdt-agents portable-pty which thiserror tracing
cargo add -p pdt-agents tokio --features sync,time,rt,macros
cargo add -p pdt-agents pdt-core --path crates/pdt-core
cargo add -p pdt-agents --dev tokio --features full
```

Then make the `Cargo.toml` header match the other crates: `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `publish.workspace = true`. Change `pdt-core` to `pdt-core.workspace = true`. In the root `Cargo.toml`, add `pdt-agents = { path = "crates/pdt-agents" }` to `[workspace.dependencies]` and `"crates/pdt-agents"` to `default-members`.

`crates/pdt-agents/src/lib.rs`:

```rust
//! Process adapters. Phase 1: PTY terminals. Phase 3 adds the structured Claude and Codex adapters.

pub mod launch;
pub mod pty;

pub use pty::{Attachment, PtyError, PtyEvent, PtyManager, SCROLLBACK_BYTES, SpawnSpec};
```

- [ ] **Step 2: Write the failing tests**

Unit tests for the scrollback ring go at the bottom of `src/pty.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::Scrollback;

    #[test]
    fn scrollback_keeps_the_tail() {
        let mut s = Scrollback::new(8);
        s.push(b"abc");
        s.push(b"defgh");
        assert_eq!(s.snapshot(), b"abcdefgh");
        s.push(b"ij");
        assert_eq!(s.snapshot(), b"cdefghij");
        s.push(b"0123456789");
        assert_eq!(s.snapshot(), b"23456789");
    }
}
```

`src/launch.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_is_reported() {
        assert!(matches!(resolve("pdt-definitely-missing-xyz", &[]), Err(PtyError::ProgramNotFound(p)) if p == "pdt-definitely-missing-xyz"));
    }

    #[test]
    fn found_program_keeps_args() {
        let (path, args) = resolve("git", &["--version".into()]).unwrap();
        assert!(path.is_file(), "{path:?}");
        assert_eq!(args, ["--version"]);
    }

    #[test]
    fn platform_defaults_to_plain_agent_names() {
        let p = detect_platform();
        assert_eq!((p.claude_program.as_str(), p.codex_program.as_str()), ("claude", "codex"));
        assert_eq!(p.windows, cfg!(windows));
    }
}
```

`crates/pdt-agents/tests/pty.rs`:

```rust
use pdt_agents::{Attachment, PtyError, PtyEvent, PtyManager, SpawnSpec};
use std::path::PathBuf;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(20);

/// Runs a one-off command: `cmd.exe /d /c <win>` on Windows, `/bin/sh -c <unix>` elsewhere.
fn oneshot_cmd(id: &str, win: &str, unix: &str) -> SpawnSpec {
    let (program, args) = if cfg!(windows) {
        (PathBuf::from("cmd.exe"), vec!["/d".into(), "/c".into(), win.into()])
    } else {
        (PathBuf::from("/bin/sh"), vec!["-c".into(), unix.into()])
    };
    spec(id, program, args)
}

/// An interactive shell that waits for input.
fn interactive(id: &str) -> SpawnSpec {
    if cfg!(windows) {
        spec(id, "cmd.exe".into(), vec!["/d".into(), "/q".into()])
    } else {
        spec(id, "/bin/sh".into(), vec![])
    }
}

fn spec(id: &str, program: PathBuf, args: Vec<String>) -> SpawnSpec {
    SpawnSpec {
        id: id.into(),
        program,
        args,
        cwd: std::env::temp_dir(),
        env_remove: vec![],
        env: vec![("PDT_TEST_VAR".into(), "from-spec".into())],
        cols: 80,
        rows: 24,
    }
}

/// Collects output until the exit event (or the timeout).
async fn drain(att: Attachment) -> (String, Option<i32>) {
    let mut out = att.scrollback;
    if let Some(code) = att.exited {
        return (String::from_utf8_lossy(&out).into_owned(), Some(code));
    }
    let mut events = att.events;
    let result = tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(b)) => out.extend_from_slice(&b),
                Ok(PtyEvent::Exit(code)) => return Some(code),
                Err(_) => return None,
            }
        }
    })
    .await
    .unwrap_or(None);
    (String::from_utf8_lossy(&out).into_owned(), result)
}

#[tokio::test]
async fn output_and_exit_code() {
    let m = PtyManager::new();
    let exit = m.spawn(oneshot_cmd("a", "echo hello-pty& exit /b 3", "echo hello-pty; exit 3")).unwrap();
    let att = m.attach("a").unwrap();
    let (out, code) = drain(att).await;
    assert!(out.contains("hello-pty"), "{out:?}");
    assert_eq!(code, Some(3));
    assert_eq!(tokio::time::timeout(WAIT, exit).await.unwrap().unwrap(), 3);
    assert!(!m.is_running("a"));
}

#[tokio::test]
async fn env_from_spec_is_applied() {
    let m = PtyManager::new();
    m.spawn(oneshot_cmd("a", "echo %PDT_TEST_VAR%", "echo \"$PDT_TEST_VAR\"")).unwrap();
    let (out, _) = drain(m.attach("a").unwrap()).await;
    assert!(out.contains("from-spec"), "{out:?}");
}

#[tokio::test]
async fn input_reaches_the_process() {
    let m = PtyManager::new();
    m.spawn(interactive("a")).unwrap();
    let att = m.attach("a").unwrap();
    // The typed text contains "4000+2000+21"; only the evaluated output contains "6021".
    let line: &[u8] =
        if cfg!(windows) { b"set /a 4000+2000+21 & exit\r" } else { b"echo $((4000+2000+21)); exit\r" };
    m.write("a", line).unwrap();
    let (out, code) = drain(att).await;
    assert!(out.contains("6021"), "{out:?}");
    assert!(code.is_some());
}

#[tokio::test]
async fn late_attach_gets_scrollback_and_exit() {
    let m = PtyManager::new();
    let exit = m.spawn(oneshot_cmd("a", "echo late-attach-ok", "echo late-attach-ok")).unwrap();
    tokio::time::timeout(WAIT, exit).await.unwrap().unwrap();
    let att = m.attach("a").unwrap();
    assert_eq!(att.exited, Some(0));
    assert!(String::from_utf8_lossy(&att.scrollback).contains("late-attach-ok"));
}

#[tokio::test]
async fn kill_ends_the_process() {
    let m = PtyManager::new();
    let exit = m.spawn(interactive("a")).unwrap();
    assert!(m.is_running("a"));
    m.resize("a", 100, 30).unwrap();
    m.kill("a").unwrap();
    tokio::time::timeout(WAIT, exit).await.expect("exit after kill").unwrap();
    assert!(matches!(m.write("a", b"x"), Err(PtyError::NotRunning(_))));
    assert!(matches!(m.kill("a"), Err(PtyError::NotRunning(_))));
}

#[tokio::test]
async fn ids_are_checked() {
    let m = PtyManager::new();
    m.spawn(interactive("a")).unwrap();
    assert!(matches!(m.spawn(interactive("a")), Err(PtyError::AlreadyExists(_))));
    assert!(matches!(m.attach("nope"), Err(PtyError::NotFound(_))));
    assert!(matches!(m.write("nope", b"x"), Err(PtyError::NotFound(_))));
    m.remove("a");
    assert!(matches!(m.attach("a"), Err(PtyError::NotFound(_))));
}

#[tokio::test]
async fn shutdown_kills_everything() {
    let m = PtyManager::new();
    let e1 = m.spawn(interactive("a")).unwrap();
    let e2 = m.spawn(interactive("b")).unwrap();
    tokio::time::timeout(WAIT, m.shutdown()).await.unwrap();
    assert!(!m.is_running("a") && !m.is_running("b"));
    e1.await.unwrap();
    e2.await.unwrap();
}

#[tokio::test]
async fn missing_working_directory_is_a_spawn_error() {
    let m = PtyManager::new();
    let mut s = interactive("a");
    s.cwd = std::env::temp_dir().join("pdt-no-such-dir-xyz");
    assert!(matches!(m.spawn(s), Err(PtyError::Spawn(_))));
    assert!(matches!(m.attach("a"), Err(PtyError::NotFound(_))));
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test -p pdt-agents`
Expected: compile errors.

- [ ] **Step 4: Implement `src/pty.rs`** (above the unit tests)

```rust
//! PTY-backed terminal sessions owned by pdtd. Each session keeps a scrollback buffer and broadcasts its output
//! to any number of attached clients, so terminals outlive the windows that show them.

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

pub const SCROLLBACK_BYTES: usize = 512 * 1024;
/// After the child exits, give the reader a moment to flush the last output before announcing the exit.
const FLUSH_GRACE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Removed from the inherited environment before `env` is applied.
    pub env_remove: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone)]
pub enum PtyEvent {
    Output(Arc<[u8]>),
    Exit(i32),
}

/// A consistent view for a new client: everything so far, then every later event.
pub struct Attachment {
    pub scrollback: Vec<u8>,
    pub exited: Option<i32>,
    pub events: broadcast::Receiver<PtyEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("{0} was not found on PATH")]
    ProgramNotFound(String),
    #[error("failed to start the terminal: {0}")]
    Spawn(String),
    #[error("terminal session {0} does not exist")]
    NotFound(String),
    #[error("terminal session {0} already exists")]
    AlreadyExists(String),
    #[error("terminal session {0} is not running")]
    NotRunning(String),
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

pub(crate) struct Scrollback {
    buf: VecDeque<u8>,
    cap: usize,
}

impl Scrollback {
    pub(crate) fn new(cap: usize) -> Self {
        Self { buf: VecDeque::with_capacity(cap.min(64 * 1024)), cap }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let bytes = if bytes.len() > self.cap { &bytes[bytes.len() - self.cap..] } else { bytes };
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.cap);
        self.buf.drain(..overflow);
        self.buf.extend(bytes);
    }

    pub(crate) fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

struct State {
    scrollback: Scrollback,
    exited: Option<i32>,
}

struct Handle {
    state: Mutex<State>,
    events: broadcast::Sender<PtyEvent>,
    input: std_mpsc::Sender<Vec<u8>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

impl Handle {
    fn exited(&self) -> bool {
        self.state.lock().unwrap().exited.is_some()
    }
}

#[derive(Clone, Default)]
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Handle>>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the process. The receiver resolves with the exit code once it has ended.
    pub fn spawn(&self, spec: SpawnSpec) -> Result<oneshot::Receiver<i32>, PtyError> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(&spec.id) {
            return Err(PtyError::AlreadyExists(spec.id));
        }
        if !spec.cwd.is_dir() {
            return Err(PtyError::Spawn(format!("working directory does not exist: {}", spec.cwd.display())));
        }
        let spawn_err = |e: &dyn std::fmt::Display| PtyError::Spawn(e.to_string());
        let size = PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| spawn_err(&e))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        for key in &spec.env_remove {
            cmd.env_remove(key);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| spawn_err(&e))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let io = pair.master.try_clone_reader().and_then(|r| Ok((r, pair.master.take_writer()?)));
        let (mut reader, mut writer) = match io {
            Ok(rw) => rw,
            Err(e) => {
                let _ = child.kill();
                return Err(spawn_err(&e));
            }
        };

        let (events, _) = broadcast::channel(1024);
        let (input_tx, input_rx) = std_mpsc::channel::<Vec<u8>>();
        let handle = Arc::new(Handle {
            state: Mutex::new(State { scrollback: Scrollback::new(SCROLLBACK_BYTES), exited: None }),
            events,
            input: input_tx,
            master: Mutex::new(Some(pair.master)),
            killer: Mutex::new(killer),
        });

        let h = handle.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Append and broadcast under one lock so `attach` sees each byte exactly once.
                        let mut st = h.state.lock().unwrap();
                        st.scrollback.push(&buf[..n]);
                        let _ = h.events.send(PtyEvent::Output(Arc::from(&buf[..n])));
                    }
                }
            }
        });

        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        });

        let (exit_tx, exit_rx) = oneshot::channel();
        let h = handle.clone();
        let id = spec.id.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code() as i32).unwrap_or(-1);
            std::thread::sleep(FLUSH_GRACE);
            {
                let mut st = h.state.lock().unwrap();
                st.exited = Some(code);
                let _ = h.events.send(PtyEvent::Exit(code));
            }
            // Closing the master ends the reader (on Windows, ConPTY keeps the pipe open until then).
            h.master.lock().unwrap().take();
            tracing::debug!(session = %id, code, "terminal exited");
            let _ = exit_tx.send(code);
        });

        sessions.insert(spec.id, handle);
        Ok(exit_rx)
    }

    fn get(&self, id: &str) -> Result<Arc<Handle>, PtyError> {
        self.sessions.lock().unwrap().get(id).cloned().ok_or_else(|| PtyError::NotFound(id.to_string()))
    }

    pub fn attach(&self, id: &str) -> Result<Attachment, PtyError> {
        let h = self.get(id)?;
        let st = h.state.lock().unwrap();
        Ok(Attachment { scrollback: st.scrollback.snapshot(), exited: st.exited, events: h.events.subscribe() })
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), PtyError> {
        let h = self.get(id)?;
        if h.exited() {
            return Err(PtyError::NotRunning(id.to_string()));
        }
        h.input.send(bytes.to_vec()).map_err(|_| PtyError::NotRunning(id.to_string()))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), PtyError> {
        let h = self.get(id)?;
        let master = h.master.lock().unwrap();
        let master = master.as_ref().ok_or_else(|| PtyError::NotRunning(id.to_string()))?;
        master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    pub fn kill(&self, id: &str) -> Result<(), PtyError> {
        let h = self.get(id)?;
        if h.exited() {
            return Err(PtyError::NotRunning(id.to_string()));
        }
        h.killer.lock().unwrap().kill().map_err(|e| PtyError::Io(e.to_string()))
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.get(id).is_ok_and(|h| !h.exited())
    }

    /// Forgets a session, killing it first if it is still running.
    pub fn remove(&self, id: &str) {
        if let Some(h) = self.sessions.lock().unwrap().remove(id)
            && !h.exited()
        {
            let _ = h.killer.lock().unwrap().kill();
        }
    }

    /// Kills every running session and waits (up to 5 s) for them to report their exit.
    pub async fn shutdown(&self) {
        let handles: Vec<Arc<Handle>> = self.sessions.lock().unwrap().values().cloned().collect();
        for h in &handles {
            if !h.exited() {
                let _ = h.killer.lock().unwrap().kill();
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while handles.iter().any(|h| !h.exited()) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
```

- [ ] **Step 5: Implement `src/launch.rs`** (above its tests)

```rust
//! Host detection and program resolution for terminal sessions.

use crate::pty::PtyError;
use pdt_core::session::Platform;
use std::path::PathBuf;

pub fn detect_platform() -> Platform {
    Platform {
        windows: cfg!(windows),
        has_pwsh: cfg!(windows) && which::which("pwsh").is_ok(),
        unix_shell: std::env::var("SHELL").ok(),
        claude_program: "claude".into(),
        codex_program: "codex".into(),
    }
}

/// Finds `program` on PATH (or accepts a path). On Windows, `.cmd`/`.bat` shims such as npm-installed CLIs
/// cannot be started directly in a PTY, so they run through `%ComSpec% /d /c`.
pub fn resolve(program: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), PtyError> {
    let path = which::which(program).map_err(|_| PtyError::ProgramNotFound(program.to_string()))?;
    let is_script = cfg!(windows)
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if is_script {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut all = vec!["/d".to_string(), "/c".to_string(), path.to_string_lossy().into_owned()];
        all.extend(args.iter().cloned());
        return Ok((PathBuf::from(comspec), all));
    }
    Ok((path, args.to_vec()))
}
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test -p pdt-agents` (twice in a row, to catch flakiness), then `cargo clippy --all-targets -- -D warnings`.
Expected: all pass on Windows. Note in your report that CI covers macOS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/pdt-agents
git commit -m "feat(agents): PTY manager with scrollback and broadcast"
```

---

### Task 5: Worktree API

**Files:**
- Create: `crates/pdt-api/src/worktrees.rs`, `crates/pdt-api/tests/worktrees.rs`
- Modify:
  - `crates/pdt-api/src/routes.rs`: add `AppState::new`, register routes, use `From<JsonRejection>`, and give the project conflict its own message.
  - `crates/pdt-api/src/error.rs`: `From<JsonRejection>`, `From<WorktreeValidationError>`, and a generic conflict message.
  - `crates/pdt-api/src/lib.rs`
  - `crates/pdt-api/tests/api.rs`: `AppState { store }` becomes `AppState::new(store)`.
  - `crates/pdtd/src/main.rs`: same change.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `AppState::new(store: Store) -> AppState`. Fields stay `pub`; Tasks 6 and 8 add `ptys`, `platform` and `shutdown` inside `new`, so callers never change again.
  - Routes:
    - `GET /api/projects/{id}/worktrees` returns `Worktree[]` (404 if the project is unknown).
    - `POST /api/projects/{id}/worktrees` takes a `NewWorktree` body and returns 201 with a `Worktree`. It first makes sure `/.worktrees/` is in the repo's `info/exclude`.
    - No delete route: worktrees are removed after merge (Phase 4).

- [ ] **Step 1: Write the failing tests**

`crates/pdt-api/tests/worktrees.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// Router plus a registered project whose repo has one commit on main. Returns (app, project id, repo path).
async fn setup(tmp: &Path) -> (axum::Router, String, std::path::PathBuf) {
    let repo = tmp.join("my repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = router(AppState::new(store), None);
    let (s, p) = call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED);
    (app, p["id"].as_str().unwrap().to_string(), repo)
}

#[tokio::test]
async fn create_and_list_worktrees_inside_the_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    let (s, wt) = call(&app, "POST", &format!("/api/projects/{pid}/worktrees"), Some(json!({"branch": "feature/x"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{wt}");
    let path = std::path::PathBuf::from(wt["path"].as_str().unwrap());
    assert!(path.is_dir());
    assert!(path.ends_with(Path::new(".worktrees").join("feature-x")));
    assert_eq!(std::fs::canonicalize(path.parent().unwrap().parent().unwrap()).unwrap(), std::fs::canonicalize(&repo).unwrap());
    assert_eq!(wt["branch"], "feature/x");
    assert_eq!(wt["baseCommit"].as_str().unwrap().len(), 40);

    let status = std::process::Command::new("git").current_dir(&repo).args(["status", "--porcelain"]).output().unwrap();
    assert!(status.stdout.is_empty(), "main checkout stays clean: {}", String::from_utf8_lossy(&status.stdout));

    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/worktrees"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (s, _) = call(&app, "DELETE", &format!("/api/worktrees/{}", wt["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no removal before merge");
}

#[tokio::test]
async fn worktree_errors_have_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, _) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/worktrees");
    for (body, status, code) in [
        (json!({"branch": "  "}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"branch": "a b"}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"branch": "main"}), StatusCode::CONFLICT, "branch_exists"),
        (json!({"branch": "x", "base": "nope"}), StatusCode::BAD_REQUEST, "unknown_ref"),
    ] {
        let (s, v) = call(&app, "POST", &uri, Some(body.clone())).await;
        assert_eq!((s, v["code"].as_str().unwrap()), (status, code), "{body}");
    }
    let (s, v) = call(&app, "GET", "/api/projects/nope/worktrees", None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::NOT_FOUND, "not_found"));
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p pdt-api --test worktrees`
Expected: compile errors (`AppState::new` and the routes don't exist yet).

- [ ] **Step 3: Implement**

`error.rs` additions:

```rust
use axum::extract::rejection::JsonRejection;
use pdt_core::worktree::WorktreeValidationError;

impl From<JsonRejection> for ApiError {
    fn from(rej: JsonRejection) -> Self {
        Self::new(rej.status(), "invalid_input", rej.body_text())
    }
}

impl From<WorktreeValidationError> for ApiError {
    fn from(e: WorktreeValidationError) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}
```

Change the `StoreError::Conflict` arm to the generic message `"this record already exists"`.

In `routes.rs`:
- `create_project` becomes `let Json(input) = payload?;`.
- Its insert maps a conflict to the project-specific message:

```rust
    st.store.insert_project(&project).await.map_err(|e| match e {
        pdt_store::StoreError::Conflict(_) => {
            ApiError::new(StatusCode::CONFLICT, "conflict", "a project for this repository already exists")
        }
        other => other.into(),
    })?;
```

Add the constructor and the routes:

```rust
impl AppState {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}
```

```rust
        .route("/projects/{id}/worktrees", get(worktrees::list).post(worktrees::create))
```

Add `mod worktrees;` to `lib.rs`. Add a shared helper in `routes.rs`: `pub(crate) fn now_ms() -> i64 { jiff::Timestamp::now().as_millisecond() }`, and use it in `create_project` too.

`crates/pdt-api/src/worktrees.rs`:

```rust
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_core::worktree::{NewWorktree, WORKTREES_DIR, Worktree, worktree_path};
use std::path::PathBuf;

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Worktree>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_worktrees(&id).await?))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewWorktree>, JsonRejection>,
) -> Result<(StatusCode, Json<Worktree>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let project = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, &valid.branch);
    let base = valid.base.unwrap_or_else(|| project.base_branch.clone());
    let (branch, target) = (valid.branch.clone(), path.clone());
    let base_commit = tokio::task::spawn_blocking(move || {
        pdt_git::ensure_excluded(&repo, &format!("/{WORKTREES_DIR}/"))?;
        pdt_git::add_worktree(&repo, &target, &branch, &base)
    })
    .await
    .map_err(ApiError::internal)??;
    let wt = Worktree {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        branch: valid.branch,
        path: path.to_string_lossy().into_owned(),
        base_commit,
        created_at: now_ms(),
    };
    st.store.insert_worktree(&wt).await?;
    Ok((StatusCode::CREATED, Json(wt)))
}
```

Update `tests/api.rs` (`router(AppState::new(store), web_dir)`) and `crates/pdtd/src/main.rs` (`AppState::new(store)`).

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test; cargo clippy --all-targets -- -D warnings`
Expected: all pass, including the existing `duplicate_repo_is_conflict`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-api crates/pdtd/src/main.rs
git commit -m "feat(api): worktree routes"
```

---

### Task 6: Session API

**Files:**
- Create: `crates/pdt-api/src/sessions.rs`, `crates/pdt-api/tests/sessions.rs`
- Modify:
  - `crates/pdt-api/Cargo.toml` (add `pdt-agents`)
  - `routes.rs` (`AppState` gains `ptys` and `platform`; register routes)
  - `error.rs` (`From<PtyError>`, `From<SessionValidationError>`)

**Interfaces:**
- Consumes: `PtyManager`, `SpawnSpec`, `launch::{detect_platform, resolve}` (Task 4), the Task 1 types, and the Task 3 store methods.
- Produces:
  - `AppState { store, ptys: PtyManager, platform: Platform }`. `AppState::new` fills `ptys` with `PtyManager::new()` and `platform` with `detect_platform()`. Tests may overwrite `state.platform` fields.
  - Routes:

    | Route | Result |
    |---|---|
    | `GET /api/projects/{id}/sessions` | `Session[]` |
    | `POST /api/sessions` (`NewSession`) | 201 `Session` |
    | `GET /api/sessions/{id}` | `Session` |
    | `POST /api/sessions/{id}/kill` | 204, or 409 `not_running` |
    | `POST /api/sessions/{id}/restart` (`TerminalSize`) | 200 `Session`, or 409 `already_running` |
    | `DELETE /api/sessions/{id}` | 204, or 409 `session_running` |

  - `PtyError` to API error:

    | Variant | Status | Code |
    |---|---|---|
    | `ProgramNotFound` | 400 | `program_not_found` |
    | `Spawn` | 500 | `spawn_failed` (message passed through) |
    | `NotFound`, `NotRunning` | 409 | `not_running` |
    | `AlreadyExists` | 409 | `already_running` |
    | `Io` | 500 | `internal` |

  - `SessionValidationError` maps to 400 `invalid_input`.

- [ ] **Step 1: Write the failing tests**

`crates/pdt-api/tests/sessions.rs`. Copy the `call`, `git` and `setup` helpers from `tests/worktrees.rs`, but have `setup` return the `AppState` too: build `let state = AppState::new(store);` and `router(state.clone(), None)`. Then add these tests:

```rust
async fn wait_status(app: &axum::Router, id: &str, want: &str) -> Value {
    for _ in 0..100 {
        let (_, s) = call(app, "GET", &format!("/api/sessions/{id}"), None).await;
        if s["status"] == want {
            return s;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("session {id} never became {want}");
}

#[tokio::test]
async fn shell_session_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, repo) = setup(tmp.path()).await;
    let (s, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24}))).await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    assert_eq!(sess["status"], "running");
    assert_eq!(sess["title"], "Shell");
    assert_eq!(sess["worktreeId"], Value::Null);
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), &repo));
    let id = sess["id"].as_str().unwrap().to_string();

    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (s, v) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "session_running"));

    let (s, _) = call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let ended = wait_status(&app, &id, "finished").await;
    assert!(ended["endedAt"].is_number());
    let (s, v) = call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "not_running"));

    let (s, _) = call(&app, "DELETE", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "GET", &format!("/api/sessions/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn restart_finished_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, sess) =
        call(&app, "POST", "/api/sessions", Some(json!({"projectId": pid, "kind": "shell", "cols": 80, "rows": 24}))).await;
    let id = sess["id"].as_str().unwrap().to_string();
    let (s, v) = call(&app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "already_running"));
    call(&app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    wait_status(&app, &id, "finished").await;
    let (s, v) = call(&app, "POST", &format!("/api/sessions/{id}/restart"), Some(json!({"cols": 80, "rows": 24}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str().unwrap(), &v["exitCode"]), ("running", &Value::Null));
    assert!(state.ptys.is_running(&id));
    state.ptys.shutdown().await;
}

#[tokio::test]
async fn missing_program_is_400_and_leaves_no_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, mut state, pid, _) = setup(tmp.path()).await;
    state.platform.codex_program = "pdt-no-such-codex-xyz".into();
    let app = router(state, None);
    let (s, v) = call(
        &app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "kind": "agent_tui", "agent": "codex", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::BAD_REQUEST, "program_not_found"));
    assert!(v["message"].as_str().unwrap().contains("pdt-no-such-codex-xyz"));
    let (_, list) = call(&app, "GET", &format!("/api/projects/{pid}/sessions"), None).await;
    assert!(list.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_session_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, pid, _) = setup(tmp.path()).await;
    for (body, status, code) in [
        (json!({"projectId": pid, "kind": "agent_tui", "cols": 80, "rows": 24}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"projectId": pid, "kind": "shell", "agent": "claude", "cols": 80, "rows": 24}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"projectId": pid, "kind": "shell", "cols": 0, "rows": 24}), StatusCode::BAD_REQUEST, "invalid_input"),
        (json!({"projectId": pid, "kind": "teleporter", "cols": 80, "rows": 24}), StatusCode::UNPROCESSABLE_ENTITY, "invalid_input"),
        (json!({"projectId": "nope", "kind": "shell", "cols": 80, "rows": 24}), StatusCode::NOT_FOUND, "not_found"),
        (json!({"projectId": pid, "worktreeId": "nope", "kind": "shell", "cols": 80, "rows": 24}), StatusCode::BAD_REQUEST, "invalid_input"),
    ] {
        let (s, v) = call(&app, "POST", "/api/sessions", Some(body.clone())).await;
        assert_eq!((s, v["code"].as_str().unwrap_or("")), (status, code), "{body}");
    }
}

#[tokio::test]
async fn session_runs_in_its_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, pid, _) = setup(tmp.path()).await;
    let (_, wt) = call(&app, "POST", &format!("/api/projects/{pid}/worktrees"), Some(json!({"branch": "w"}))).await;
    let wid = wt["id"].as_str().unwrap();
    let (s, sess) = call(
        &app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": pid, "worktreeId": wid, "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(sess["worktreeId"], wid);
    assert!(same_dir(Path::new(sess["cwd"].as_str().unwrap()), Path::new(wt["path"].as_str().unwrap())));
    state.ptys.shutdown().await;
}

fn same_dir(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}
```

> An unknown `kind` fails in serde (`JsonRejection`, 422). The status comes from axum; the code is still `invalid_input`.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p pdt-api --test sessions`
Expected: compile errors.

- [ ] **Step 3: Implement**

Run: `cargo add -p pdt-api pdt-agents --path crates/pdt-agents`, then switch the entry to `pdt-agents.workspace = true`.

In `routes.rs`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub ptys: pdt_agents::PtyManager,
    pub platform: pdt_core::session::Platform,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self { store, ptys: pdt_agents::PtyManager::new(), platform: pdt_agents::launch::detect_platform() }
    }
}
```

Register the routes:

```rust
        .route("/projects/{id}/sessions", get(sessions::list))
        .route("/sessions", axum::routing::post(sessions::create))
        .route("/sessions/{id}", get(sessions::get_one).delete(sessions::remove))
        .route("/sessions/{id}/kill", axum::routing::post(sessions::kill))
        .route("/sessions/{id}/restart", axum::routing::post(sessions::restart))
```

`error.rs` additions:

```rust
use pdt_agents::PtyError;
use pdt_core::session::SessionValidationError;

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
            PtyError::Io(_) => Self::internal(e),
        }
    }
}
```

`crates/pdt-api/src/sessions.rs`:

```rust
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::SpawnSpec;
use pdt_core::session::{
    NewSession, Session, SessionStatus, TerminalSize, default_title, launch, scrubbed_env,
};
use std::path::PathBuf;

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Session>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_sessions(&id).await?))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Session>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

async fn load(st: &AppState, id: &str) -> Result<Session, ApiError> {
    st.store.get_session(id).await?.ok_or_else(|| ApiError::not_found("session"))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewSession>, JsonRejection>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let Json(input) = payload?;
    let valid = input.validate()?;
    let project = st.store.get_project(&valid.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let (worktree_id, cwd) = match &valid.worktree_id {
        Some(wid) => {
            let wt = st.store.get_worktree(wid).await?.filter(|w| w.project_id == project.id).ok_or_else(|| {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "worktree does not belong to this project")
            })?;
            (Some(wt.id), wt.path)
        }
        None => (None, project.repo_path.clone()),
    };
    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        worktree_id,
        kind: valid.kind,
        agent: valid.agent,
        title: default_title(valid.kind, valid.agent).into(),
        cwd,
        status: SessionStatus::Running,
        exit_code: None,
        created_at: now_ms(),
        ended_at: None,
    };
    st.store.insert_session(&session).await?;
    if let Err(e) = start_pty(&st, &session, valid.size, false) {
        st.store.delete_session(&session.id).await?;
        return Err(e);
    }
    Ok((StatusCode::CREATED, Json(session)))
}

/// Spawns the session's process and records its exit when it ends.
fn start_pty(st: &AppState, s: &Session, size: TerminalSize, resume: bool) -> Result<(), ApiError> {
    let l = launch(s.kind, s.agent, &st.platform, resume);
    let (program, args) = pdt_agents::launch::resolve(&l.program, &l.args)?;
    let spec = SpawnSpec {
        id: s.id.clone(),
        program,
        args,
        cwd: PathBuf::from(&s.cwd),
        env_remove: scrubbed_env(s.kind).into_iter().map(String::from).collect(),
        env: vec![("TERM".into(), "xterm-256color".into()), ("COLORTERM".into(), "truecolor".into())],
        cols: size.cols,
        rows: size.rows,
    };
    let exit = st.ptys.spawn(spec)?;
    let (store, id) = (st.store.clone(), s.id.clone());
    tokio::spawn(async move {
        if let Ok(code) = exit.await
            && let Err(e) = store.end_session(&id, SessionStatus::Finished, Some(code), now_ms()).await
        {
            tracing::warn!(session = %id, error = %e, "failed to record session exit");
        }
    });
    Ok(())
}

pub(crate) async fn kill(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    st.ptys.kill(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<TerminalSize>, JsonRejection>,
) -> Result<Json<Session>, ApiError> {
    let Json(size) = payload?;
    let size = size.validate()?;
    let session = load(&st, &id).await?;
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "already_running", "this session is already running"));
    }
    st.ptys.remove(&id);
    // Mark running before spawning, so an instant exit is recorded after (not before) this update.
    st.store.set_session_running(&id).await?;
    if let Err(e) = start_pty(&st, &session, size, true) {
        st.store.end_session(&id, SessionStatus::Interrupted, None, now_ms()).await?;
        return Err(e);
    }
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    if st.ptys.is_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "session_running", "stop the session before removing it"));
    }
    st.ptys.remove(&id);
    st.store.delete_session(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

Add `mod sessions;` to `lib.rs`.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p pdt-api; cargo clippy --all-targets -- -D warnings`
Expected: all pass. The shell tests start a real shell; allow a few seconds on Windows.

- [ ] **Step 5: Commit**

```bash
git add crates/pdt-api Cargo.toml Cargo.lock
git commit -m "feat(api): terminal session routes"
```

---

### Task 7: PTY WebSocket and Origin guard

**Files:**
- Create: `crates/pdt-api/src/pty_ws.rs`, `crates/pdt-api/tests/pty_ws.rs`
- Modify:
  - root `Cargo.toml` (axum `ws` feature)
  - `crates/pdt-api/Cargo.toml` (`futures-util`; `serde_json` moves to `[dependencies]`; dev-dependency `tokio-tungstenite`)
  - `host_guard.rs` (Origin check)
  - `routes.rs` (route)
  - `tests/api.rs` (Origin tests)
  - `crates/pdtd/src/config.rs` (IPv6 and dev Vite hosts)
  - `web/vite.config.ts` (proxy WebSockets)

**Interfaces:**
- Consumes: `PtyManager::attach`, `write`, `resize`; `PtyClientMessage` and `PtyServerMessage`.
- Produces:
  - `GET /api/sessions/{id}/pty` (WebSocket). Protocol: see decision 9.
    - 404 `not_found` for an unknown session.
    - 409 `not_running` when the session has no terminal (for example after a `pdtd` restart).
    - 400 `websocket_required` for a plain GET.
  - `with_host_guard` also enforces Origin. 403 `forbidden_origin`, with the message `origin <origin> is not allowed`.
  - The default allowed hosts become `127.0.0.1:<port>`, `localhost:<port>` and `[::1]:<port>`. The dev profile adds `localhost:5173` and `127.0.0.1:5173`.

- [ ] **Step 1: Add the dependencies**

- In the root `Cargo.toml`, change `axum = "0.8.9"` to `axum = { version = "0.8.9", features = ["ws"] }`.
- Run:

  ```powershell
  cargo add -p pdt-api futures-util
  cargo add -p pdt-api --dev tokio-tungstenite
  ```

- Move `serde_json.workspace = true` from `[dev-dependencies]` to `[dependencies]` in `crates/pdt-api/Cargo.toml`.

- [ ] **Step 2: Write the failing tests**

Append to `crates/pdt-api/tests/api.rs`:

```rust
async fn call_origin(app: &axum::Router, method: &str, origin: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri("/api/health").header("host", "127.0.0.1:7420");
    if let Some(o) = origin {
        req = req.header("origin", o);
    }
    let res = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn origin_must_match_an_allowed_host() {
    let tmp = tempfile::tempdir().unwrap();
    let app = with_host_guard(
        app(tmp.path(), None).await,
        vec!["127.0.0.1:7420".into(), "localhost:7420".into(), "box.tail.ts.net".into()],
    );
    assert_eq!(call_origin(&app, "GET", None).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("http://127.0.0.1:7420")).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("http://LOCALHOST:7420")).await.0, StatusCode::OK);
    assert_eq!(call_origin(&app, "GET", Some("https://box.tail.ts.net")).await.0, StatusCode::OK);
    for bad in ["http://evil.example", "null", "http://127.0.0.1:9999", "file://", "chrome-extension://abc"] {
        let (status, body) = call_origin(&app, "GET", Some(bad)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bad}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["code"], "forbidden_origin");
        assert_eq!(v["message"], format!("origin {bad} is not allowed"));
    }
}

#[tokio::test]
async fn health_reports_version_and_wrong_content_type_is_json() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None).await;
    let (_, body) = call(&app, "GET", "/api/health", None).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["version"], env!("CARGO_PKG_VERSION"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/projects")
        .header("content-type", "text/plain")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "invalid_input");
}
```

`crates/pdt-api/tests/pty_ws.rs`:

```rust
use futures_util::{SinkExt, StreamExt};
use pdt_api::{AppState, router, with_host_guard};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const WAIT: Duration = Duration::from_secs(30);

/// Serves the guarded router on a random port. Returns (port, state).
async fn serve(tmp: &Path) -> (u16, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = with_host_guard(router(state.clone(), None), vec![format!("127.0.0.1:{port}")]);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (port, state)
}

/// Minimal HTTP/1.0 JSON client (no extra dependencies).
async fn http(port: u16, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

async fn shell_session(tmp: &Path, port: u16) -> String {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    let (_, p) = http(port, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    let (status, s) = http(
        port,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": p["id"], "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(status, 201, "{s}");
    s["id"].as_str().unwrap().to_string()
}

/// Typed text contains "4000+2000+21"; only the shell's evaluated output contains "6021".
const PROBE: &str = if cfg!(windows) { "echo (4000+2000+21)\r" } else { "echo $((4000+2000+21))\r" };

async fn read_until<S>(ws: &mut S, needle: &str) -> String
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut out = String::new();
    tokio::time::timeout(WAIT, async {
        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Binary(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Message::Text(t) => out.push_str(t.as_str()),
                _ => {}
            }
            if out.contains(needle) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}; got {out:?}"));
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_streams_input_output_and_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;
    let url = format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty");

    let (mut ws, _) = connect_async(&url).await.unwrap();
    ws.send(Message::Text(r#"{"type":"resize","cols":100,"rows":30}"#.into())).await.unwrap();
    ws.send(Message::Binary(PROBE.as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, "6021").await;

    // second_client_gets_scrollback: a late client sees earlier output immediately.
    let (mut ws2, _) = connect_async(&url).await.unwrap();
    read_until(&mut ws2, "6021").await;

    let (status, _) = http(port, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert_eq!(status, 204);
    let tail = read_until(&mut ws, "\"type\":\"exit\"").await;
    assert!(tail.contains("\"code\""), "{tail}");
    state.ptys.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_rejects_foreign_origin_and_unknown_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let (port, state) = serve(tmp.path()).await;
    let id = shell_session(tmp.path(), port).await;

    let mut req = format!("ws://127.0.0.1:{port}/api/sessions/{id}/pty").into_client_request().unwrap();
    req.headers_mut().insert("origin", "http://evil.example".parse().unwrap());
    match connect_async(req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => assert_eq!(res.status(), 403),
        other => panic!("expected 403, got {other:?}"),
    }

    match connect_async(format!("ws://127.0.0.1:{port}/api/sessions/nope/pty")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => assert_eq!(res.status(), 404),
        other => panic!("expected 404, got {other:?}"),
    }

    let (status, body) = http(port, "GET", &format!("/api/sessions/{id}/pty"), None).await;
    assert_eq!((status, body["code"].as_str().unwrap()), (400, "websocket_required"));
    state.ptys.shutdown().await;
}
```

In `crates/pdtd/src/config.rs`, update `allowed_hosts_default_and_extra`, and add a dev-profile test:

```rust
    #[test]
    fn allowed_hosts_default_and_extra() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!(c.allowed_hosts, ["127.0.0.1:7420", "localhost:7420", "[::1]:7420"]);
        let c = Config::from_lookup(
            lookup(&[("PDT_PORT", "9000"), ("PDT_ALLOWED_HOSTS", " box.tail.ts.net:9000 ,, other:1 ,")]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            c.allowed_hosts,
            ["127.0.0.1:9000", "localhost:9000", "[::1]:9000", "box.tail.ts.net:9000", "other:1"]
        );
    }

    #[test]
    fn dev_profile_allows_the_vite_dev_server() {
        let c = Config::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some("/d".into()), None).unwrap();
        assert!(c.allowed_hosts.contains(&"localhost:5173".to_string()));
        assert!(c.allowed_hosts.contains(&"127.0.0.1:5173".to_string()));
        let stable = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert!(!stable.allowed_hosts.iter().any(|h| h.ends_with(":5173")));
    }
```

`empty_env_values_are_unset` asserts `c.allowed_hosts.len() == 2`; change it to `3`.

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test -p pdt-api; cargo test -p pdtd`
Expected: failures, or compile errors.

- [ ] **Step 4: Implement**

`host_guard.rs`: replace the middleware body with the following, and update the doc comment to mention Origin:

```rust
            let headers = req.headers();
            if let Some(host) = headers.get(header::HOST) {
                let host = String::from_utf8_lossy(host.as_bytes()).into_owned();
                if !allowed.contains(&host.to_ascii_lowercase()) {
                    return ApiError::new(StatusCode::FORBIDDEN, "forbidden_host", format!("host {host} is not allowed"))
                        .into_response();
                }
            }
            if let Some(origin) = headers.get(header::ORIGIN) {
                let origin = String::from_utf8_lossy(origin.as_bytes()).into_owned();
                if !origin_allowed(&origin, &allowed) {
                    return ApiError::new(
                        StatusCode::FORBIDDEN,
                        "forbidden_origin",
                        format!("origin {origin} is not allowed"),
                    )
                    .into_response();
                }
            }
            next.run(req).await
```

Also add:

```rust
/// An Origin is allowed when it is http(s) and its `host[:port]` is an allowed Host value.
fn origin_allowed(origin: &str, allowed: &[String]) -> bool {
    let lower = origin.to_ascii_lowercase();
    let Some(authority) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) else {
        return false;
    };
    !authority.is_empty() && !authority.contains('/') && allowed.iter().any(|h| h == authority)
}
```

`routes.rs`: add `.route("/sessions/{id}/pty", get(pty_ws::connect))` and `mod pty_ws;` (in `lib.rs`).

`crates/pdt-api/src/pty_ws.rs`:

```rust
//! WebSocket bridge between a browser terminal (xterm.js) and a PTY session. See the protocol in
//! docs/superpowers/plans/2026-10-15-phase-1-terminals.md, decision 9.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use pdt_agents::{Attachment, PtyEvent, PtyManager};
use pdt_core::session::{PtyClientMessage, PtyServerMessage};
use tokio::sync::broadcast::error::RecvError;

/// Close code telling the client it fell behind and should reconnect (it will get the scrollback again).
const CLOSE_LAGGED: u16 = 4000;

pub(crate) async fn connect(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    let ws = ws.map_err(|_| {
        ApiError::new(StatusCode::BAD_REQUEST, "websocket_required", "this endpoint needs a WebSocket upgrade")
    })?;
    let attachment = st.ptys.attach(&id).map_err(|_| {
        ApiError::new(StatusCode::CONFLICT, "not_running", "this session has no terminal; restart it")
    })?;
    let ptys = st.ptys.clone();
    Ok(ws.on_upgrade(move |socket| pump(socket, ptys, id, attachment)))
}

fn exit_message(code: i32) -> Message {
    Message::Text(serde_json::to_string(&PtyServerMessage::Exit { code }).expect("serializable").into())
}

async fn pump(socket: WebSocket, ptys: PtyManager, id: String, att: Attachment) {
    let (mut tx, mut rx) = socket.split();
    let Attachment { scrollback, exited, mut events } = att;
    if !scrollback.is_empty() && tx.send(Message::Binary(scrollback.into())).await.is_err() {
        return;
    }
    if let Some(code) = exited {
        let _ = tx.send(exit_message(code)).await;
        let _ = tx.close().await;
        return;
    }
    let output = async {
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(bytes)) => {
                    if tx.send(Message::Binary(bytes.to_vec().into())).await.is_err() {
                        return;
                    }
                }
                Ok(PtyEvent::Exit(code)) => {
                    let _ = tx.send(exit_message(code)).await;
                    break;
                }
                Err(RecvError::Lagged(_)) => {
                    let frame = CloseFrame { code: CLOSE_LAGGED, reason: "lagged".into() };
                    let _ = tx.send(Message::Close(Some(frame))).await;
                    return;
                }
                Err(RecvError::Closed) => break,
            }
        }
        let _ = tx.close().await;
    };
    let input = async {
        while let Some(Ok(msg)) = rx.next().await {
            match msg {
                Message::Binary(bytes) => {
                    let _ = ptys.write(&id, &bytes);
                }
                Message::Text(text) => {
                    if let Ok(PtyClientMessage::Resize { cols, rows }) = serde_json::from_str(text.as_str()) {
                        let _ = ptys.resize(&id, cols, rows);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };
    tokio::select! {
        _ = output => {}
        _ = input => {}
    }
}
```

`crates/pdtd/src/config.rs`, in `from_lookup`:

```rust
        let mut allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
        if profile == Profile::Dev {
            // `pnpm --dir web dev` serves the UI on 5173 and proxies /api here with its own Host and Origin.
            allowed_hosts.extend([format!("localhost:{VITE_DEV_PORT}"), format!("127.0.0.1:{VITE_DEV_PORT}")]);
        }
```

with `const VITE_DEV_PORT: u16 = 5173;` at module level.

`web/vite.config.ts`: `server: { proxy: { '/api': { target: 'http://127.0.0.1:7421', ws: true } } },`

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test; cargo clippy --all-targets -- -D warnings; pnpm --dir web typecheck`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates web/vite.config.ts
git commit -m "feat(api): PTY WebSocket stream and Origin guard"
```

---

### Task 8: pdtd lifecycle: file logs, graceful shutdown, interrupted sessions

**Files:**
- Create: `crates/pdtd/src/logging.rs`, `crates/pdtd/src/shutdown.rs`, `crates/pdtd/tests/lifecycle.rs`
- Modify:
  - `crates/pdtd/src/main.rs`
  - `crates/pdtd/Cargo.toml` (`tracing-appender`, `jiff`)
  - `crates/pdt-api/src/routes.rs` (`AppState.shutdown`; `POST /api/shutdown`)

**Interfaces:**
- Consumes: `AppState`, `Store::mark_running_interrupted`, `PtyManager::shutdown`.
- Produces:
  - `AppState.shutdown: Arc<tokio::sync::Notify>`
  - `POST /api/shutdown` returns 202 and triggers the graceful stop.
  - Logs go to `<data_dir>/logs/pdtd.<date>.log` (daily, 14 kept) as well as stderr.
  - On start, sessions still marked `running` become `interrupted`.
  - On a graceful stop, running sessions are marked `interrupted`, PTYs are killed, and the pool is closed.

**Startup order (keep it exactly):**
1. Parse the config. Errors go to stderr.
2. Bind.
3. Create the data folder.
4. Take the lock.
5. Initialise logging (stderr plus file).
6. Emit config warnings. Doing this after the bind is a carry-over item.
7. Open the store.
8. Mark running sessions interrupted.
9. Serve until shutdown.

- [ ] **Step 1: Write the failing lifecycle test**

`crates/pdtd/tests/lifecycle.rs`:

```rust
//! Starts the real pdtd binary on a free port with a temp data dir. Never touches the stable instance.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start(port: u16, data: &Path) -> Daemon {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pdtd"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let child = cmd
        .env("PDT_PROFILE", "dev")
        .env("PDT_PORT", port.to_string())
        .env("PDT_DATA_DIR", data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let d = Daemon(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if http(port, "GET", "/api/health", None).0 == 200 {
            return d;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("pdtd did not become healthy");
}

fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return (0, serde_json::Value::Null) };
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    if s.write_all(req.as_bytes()).is_err() {
        return (0, serde_json::Value::Null);
    }
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let Some((head, body)) = text.split_once("\r\n\r\n") else { return (0, serde_json::Value::Null) };
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(body).unwrap_or(serde_json::Value::Null))
}

fn wait_exit(d: &mut Daemon, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = d.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "pdtd did not exit");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn shutdown_marks_running_sessions_interrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    let port = free_port();

    let mut d = start(port, &data);
    let body = serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string();
    let (status, project) = http(port, "POST", "/api/projects", Some(&body));
    assert_eq!(status, 201, "{project}");
    let body = serde_json::json!({"projectId": project["id"], "kind": "shell", "cols": 80, "rows": 24}).to_string();
    let (status, session) = http(port, "POST", "/api/sessions", Some(&body));
    assert_eq!(status, 201, "{session}");
    let id = session["id"].as_str().unwrap().to_string();

    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    let status = wait_exit(&mut d, Duration::from_secs(20));
    assert!(status.success(), "graceful shutdown exits 0, got {status:?}");

    let logs: Vec<_> = std::fs::read_dir(data.join("logs")).unwrap().flatten().collect();
    assert!(logs.iter().any(|e| e.file_name().to_string_lossy().starts_with("pdtd")), "log file written");

    let _d = start(port, &data);
    let (_, s) = http(port, "GET", &format!("/api/sessions/{id}"), None);
    assert_eq!(s["status"], "interrupted");
    assert!(s["endedAt"].is_number());
}
```

Run: `cargo add -p pdtd --dev serde_json tempfile` (`tempfile` may already be present).

- [ ] **Step 2: Run the test and confirm it fails**

Run: `cargo test -p pdtd --test lifecycle`
Expected: FAIL. `/api/shutdown` returns 404 and the test panics. There is no log file either.

- [ ] **Step 3: Implement**

Run: `cargo add -p pdtd tracing-appender jiff`, then switch `jiff` to `jiff.workspace = true`.

In `crates/pdt-api/src/routes.rs`, `AppState` gains `pub shutdown: std::sync::Arc<tokio::sync::Notify>`, which `new` initialises with `Arc::new(Notify::new())`. Add the route:

```rust
        .route("/shutdown", axum::routing::post(request_shutdown))
```

```rust
/// Asks pdtd to stop gracefully (running sessions become `interrupted`). Guarded by the Host/Origin checks.
async fn request_shutdown(State(st): State<AppState>) -> StatusCode {
    tracing::info!("shutdown requested over the API");
    st.shutdown.notify_one();
    StatusCode::ACCEPTED
}
```

`crates/pdtd/src/logging.rs`:

```rust
use anyhow::Context;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Logs to stderr and to `<log_dir>/pdtd.<date>.log` (daily, 14 files kept). Keep the guard alive.
pub fn init(log_dir: &Path) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir).with_context(|| format!("creating log dir {}", log_dir.display()))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("pdtd")
        .filename_suffix("log")
        .max_log_files(14)
        .build(log_dir)
        .context("creating the log file")?;
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_filter(filter()))
        .with(fmt::layer().with_ansi(false).with_writer(file).with_filter(filter()))
        .try_init()
        .context("initialising logging")?;
    Ok(guard)
}
```

`crates/pdtd/src/shutdown.rs`:

```rust
use pdt_api::AppState;
use std::future::IntoFuture;
use std::time::Duration;
use tokio::net::TcpListener;

/// After cleanup, open connections get this long to finish before pdtd exits anyway.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

async fn wait_for_signal(st: &AppState) {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
        _ = st.shutdown.notified() => {}
    }
}

/// Serves until Ctrl-C, SIGTERM or `POST /api/shutdown`. Running sessions are marked `interrupted` and their
/// processes killed (which also ends their WebSockets) before connections drain.
pub async fn serve(listener: TcpListener, app: axum::Router, state: AppState) -> anyhow::Result<()> {
    let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel::<()>();
    let st = state.clone();
    let graceful = async move {
        wait_for_signal(&st).await;
        tracing::info!("shutting down");
        match st.store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await {
            Ok(n) => tracing::info!(sessions = n, "marked running sessions interrupted"),
            Err(e) => tracing::error!(error = %e, "failed to mark sessions interrupted"),
        }
        st.ptys.shutdown().await;
        let _ = cleaned_tx.send(());
    };
    let server = axum::serve(listener, app).with_graceful_shutdown(graceful).into_future();
    tokio::select! {
        r = server => r?,
        _ = async {
            if cleaned_rx.await.is_ok() {
                tokio::time::sleep(DRAIN_TIMEOUT).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => tracing::warn!("connections did not close within {DRAIN_TIMEOUT:?}; exiting anyway"),
    }
    Ok(())
}
```

`crates/pdtd/src/main.rs`:

```rust
mod config;
mod lock;
mod logging;
mod shutdown;

use anyhow::Context;
use pdt_api::{AppState, router, with_host_guard};
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME};
use pdt_store::Store;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        tracing::error!("{e:#}");
        eprintln!("{DAEMON_BIN}: {e:#}");
        std::process::exit(2);
    }
}

async fn run() -> anyhow::Result<()> {
    let cfg = config::Config::from_env()?;
    // Order matters: bind the port, lock the data dir, start logging, and only then open (and migrate) the database.
    let listener = config::bind(cfg.port).await?;
    std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating data dir {}", cfg.data_dir.display()))?;
    let _lock = lock::lock_data_dir(&cfg.data_dir)?;
    let _log_guard = logging::init(&cfg.data_dir.join("logs"))?;
    if let Ok(p) = std::env::var("PDT_PROFILE")
        && !matches!(p.trim(), "" | "dev" | "stable")
    {
        tracing::warn!("PDT_PROFILE={p:?} is not 'dev' or 'stable'; treating it as stable");
    }
    if cfg.web_dir.is_none() {
        tracing::warn!("the web UI is not served (no web dir found); set PDT_WEB_DIR to the built web/dist directory");
    }
    let db_path = cfg.data_dir.join("pdt.db");
    let store = Store::open(&db_path).await.with_context(|| format!("opening database {}", db_path.display()))?;
    let interrupted = store.mark_running_interrupted(jiff::Timestamp::now().as_millisecond()).await?;
    if interrupted > 0 {
        tracing::info!(sessions = interrupted, "sessions from the previous run are now interrupted");
    }
    let state = AppState::new(store.clone());
    let app = with_host_guard(router(state.clone(), cfg.web_dir.clone()), cfg.allowed_hosts.clone());
    tracing::info!(
        profile = ?cfg.profile, port = cfg.port, data_dir = %cfg.data_dir.display(),
        web_dir = ?cfg.web_dir, "{PRODUCT_NAME} daemon listening on http://127.0.0.1:{}", cfg.port
    );
    shutdown::serve(listener, app, state).await?;
    store.close().await;
    tracing::info!("stopped");
    Ok(())
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test; cargo clippy --all-targets -- -D warnings`
Expected: all pass, including `shutdown_marks_running_sessions_interrupted`.

- [ ] **Step 5: Commit**

```bash
git add crates Cargo.lock
git commit -m "feat(pdtd): file logs, graceful shutdown, interrupted sessions"
```

---

### Task 9: Web terminal (client, `PtySocket`, `TerminalView`)

**Files:**
- Create: `web/src/terminal/ptySocket.ts`, `web/src/terminal/ptySocket.test.ts`, `web/src/terminal/TerminalView.tsx`, `web/src/terminal/TerminalView.test.tsx`, `web/src/terminal/theme.ts`
- Modify: `web/src/api/client.ts` (new endpoints; handle 204), `web/src/api/client.test.ts`, `web/package.json` (xterm)

**Interfaces:**
- Consumes: the generated types `Session`, `NewSession`, `Worktree`, `NewWorktree`, `TerminalSize`, `PtyClientMessage` and `PtyServerMessage`.
- Produces:
  - `api.listWorktrees(projectId)`, `api.createWorktree(projectId, NewWorktree)`
  - `api.listSessions(projectId)`, `api.createSession(NewSession)`, `api.killSession(id)`, `api.restartSession(id, TerminalSize)`, `api.deleteSession(id)`
  - `ptyUrl(sessionId, loc?)`
  - `class PtySocket(url, handlers, factory?, opts?)` with `connect()`, `send(text)`, `resize(cols, rows)`, `retry()` and `dispose()`. `PtyStatus` is `'connecting' | 'open' | 'reconnecting' | 'exited' | 'failed'`.
  - `<TerminalView sessionId />`

- [ ] **Step 1: Add the packages**

Run: `pnpm --dir web add @xterm/xterm @xterm/addon-fit`. Confirm that `@xterm/xterm` resolves to 6.0.0, the spec pin. If pnpm 12's release-age guard blocks a version, add a `minimumReleaseAgeExclude` entry with a comment, as Phase 0 did.

- [ ] **Step 2: Write the failing tests**

Append to `web/src/api/client.test.ts`:

```ts
  it('returns undefined for 204 responses and encodes ids', async () => {
    const fetchMock = vi.fn().mockImplementation(async () => new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    await expect(api.deleteSession('a/b')).resolves.toBeUndefined();
    expect(fetchMock.mock.calls[0][0]).toBe('/api/sessions/a%2Fb');
    expect(fetchMock.mock.calls[0][1].method).toBe('DELETE');
  });

  it('posts new sessions as JSON', async () => {
    const fetchMock = stubFetch(200, { id: 's1' });
    await api.createSession({ projectId: 'p', kind: 'shell', cols: 80, rows: 24 });
    expect(fetchMock.mock.calls[0][0]).toBe('/api/sessions');
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ projectId: 'p', kind: 'shell', cols: 80, rows: 24 });
  });
```

`web/src/terminal/ptySocket.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PtySocket, ptyUrl, type SocketLike } from './ptySocket';

class FakeSocket implements SocketLike {
  binaryType: BinaryType = 'blob';
  readyState = 0;
  sent: unknown[] = [];
  closed = false;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  send(data: string | ArrayBufferLike | ArrayBufferView) { this.sent.push(data); }
  close() { this.closed = true; }
  open() { this.readyState = 1; this.onopen?.(new Event('open')); }
  data(bytes: string) { this.onmessage?.({ data: new TextEncoder().encode(bytes).buffer } as MessageEvent); }
  text(s: string) { this.onmessage?.({ data: s } as MessageEvent); }
  drop() { this.readyState = 3; this.onclose?.({ code: 1006 } as CloseEvent); }
}

function setup() {
  const sockets: FakeSocket[] = [];
  const events: string[] = [];
  const h = {
    onData: (b: Uint8Array) => events.push(`data:${new TextDecoder().decode(b)}`),
    onReset: () => events.push('reset'),
    onExit: (c: number) => events.push(`exit:${c}`),
    onStatus: (s: string) => events.push(`status:${s}`),
  };
  const sock = new PtySocket('ws://x/api/sessions/s/pty', h, () => {
    const s = new FakeSocket();
    sockets.push(s);
    return s;
  }, { baseDelayMs: 100, maxDelayMs: 400, maxFailures: 3 });
  return { sock, sockets, events };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('ptyUrl', () => {
  it('uses ws or wss to match the page', () => {
    expect(ptyUrl('a b', { protocol: 'http:', host: '127.0.0.1:7420' })).toBe('ws://127.0.0.1:7420/api/sessions/a%20b/pty');
    expect(ptyUrl('s', { protocol: 'https:', host: 'box.ts.net' })).toBe('wss://box.ts.net/api/sessions/s/pty');
  });
});

describe('PtySocket', () => {
  it('streams data, sends input as bytes and resize as JSON', () => {
    const { sock, sockets, events } = setup();
    sock.resize(100, 30);
    sock.connect();
    sockets[0].open();
    expect(sockets[0].binaryType).toBe('arraybuffer');
    expect(sockets[0].sent).toEqual([JSON.stringify({ type: 'resize', cols: 100, rows: 30 })]);
    sockets[0].data('hello');
    sock.send('ls\r');
    expect(new TextDecoder().decode(sockets[0].sent[1] as Uint8Array)).toBe('ls\r');
    expect(events).toEqual(['status:connecting', 'reset', 'status:open', 'data:hello']);
  });

  it('resets_before_replay_on_reconnect', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].data('one');
    sockets[0].drop();
    vi.advanceTimersByTime(100);
    expect(sockets).toHaveLength(2);
    sockets[1].open();
    sockets[1].data('one-replayed');
    expect(events.slice(-4)).toEqual(['status:reconnecting', 'reset', 'status:open', 'data:one-replayed']);
  });

  it('stops after exit and does not reconnect', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    sockets[0].open();
    sockets[0].text(JSON.stringify({ type: 'exit', code: 2 }));
    sockets[0].drop();
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(1);
    expect(events).toContain('exit:2');
    expect(events).toContain('status:exited');
  });

  it('gives up after repeated failures and can retry', () => {
    const { sock, sockets, events } = setup();
    sock.connect();
    for (let i = 0; i < 3; i++) {
      sockets[i].drop();
      vi.advanceTimersByTime(1000);
    }
    expect(events.at(-1)).toBe('status:failed');
    const count = sockets.length;
    sock.retry();
    expect(sockets).toHaveLength(count + 1);
  });

  it('dispose closes and cancels reconnects', () => {
    const { sock, sockets } = setup();
    sock.connect();
    sockets[0].drop();
    sock.dispose();
    vi.advanceTimersByTime(10_000);
    expect(sockets).toHaveLength(1);
  });

  it('drops input while disconnected', () => {
    const { sock, sockets } = setup();
    sock.connect();
    sock.send('x');
    expect(sockets[0].sent).toEqual([]);
  });
});
```

`web/src/terminal/TerminalView.test.tsx`:

```tsx
import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  term: null as null | { written: unknown[]; dataCb: ((d: string) => void) | null; disposed: boolean; resetCount: number },
  sock: null as null | { handlers: Record<string, (...a: unknown[]) => void>; sent: string[]; disposed: boolean; connected: boolean },
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    written: unknown[] = [];
    dataCb: ((d: string) => void) | null = null;
    disposed = false;
    resetCount = 0;
    constructor() { h.term = this; }
    loadAddon() {}
    open() {}
    write(d: unknown) { this.written.push(d); }
    reset() { this.resetCount++; }
    onData(cb: (d: string) => void) { this.dataCb = cb; return { dispose() {} }; }
    dispose() { this.disposed = true; }
  },
}));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('./ptySocket', () => ({
  ptyUrl: (id: string) => `ws://test/${id}`,
  PtySocket: class {
    handlers: Record<string, (...a: unknown[]) => void>;
    sent: string[] = [];
    disposed = false;
    connected = false;
    constructor(_url: string, handlers: Record<string, (...a: unknown[]) => void>) {
      this.handlers = handlers;
      h.sock = this;
    }
    connect() { this.connected = true; }
    send(d: string) { this.sent.push(d); }
    resize() {}
    retry() {}
    dispose() { this.disposed = true; }
  },
}));

import { TerminalView } from './TerminalView';

describe('TerminalView', () => {
  it('wires the socket to the terminal and cleans up', () => {
    const { unmount } = render(<TerminalView sessionId="s1" />);
    expect(h.sock!.connected).toBe(true);
    h.sock!.handlers.onReset();
    h.sock!.handlers.onData(new Uint8Array([104, 105]));
    expect(h.term!.resetCount).toBe(1);
    expect(h.term!.written).toHaveLength(1);
    h.term!.dataCb!('ls\r');
    expect(h.sock!.sent).toEqual(['ls\r']);
    h.sock!.handlers.onExit(3);
    expect(String(h.term!.written.at(-1))).toContain('exited with code 3');
    unmount();
    expect(h.sock!.disposed).toBe(true);
    expect(h.term!.disposed).toBe(true);
  });
});
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `pnpm --dir web test`
Expected: failures (the modules don't exist yet).

- [ ] **Step 4: Implement**

`web/src/api/client.ts`: add the imports for the generated types. In `request`, before `return (await res.json()) as T;`, add `if (res.status === 204) return undefined as T;`. Then add:

```ts
const enc = encodeURIComponent;

export const api = {
  health: () => request<Health>('/api/health'),
  listProjects: () => request<Project[]>('/api/projects'),
  createProject: (p: NewProject) => request<Project>('/api/projects', { method: 'POST', body: JSON.stringify(p) }),
  listWorktrees: (projectId: string) => request<Worktree[]>(`/api/projects/${enc(projectId)}/worktrees`),
  createWorktree: (projectId: string, w: NewWorktree) =>
    request<Worktree>(`/api/projects/${enc(projectId)}/worktrees`, { method: 'POST', body: JSON.stringify(w) }),
  listSessions: (projectId: string) => request<Session[]>(`/api/projects/${enc(projectId)}/sessions`),
  createSession: (s: NewSession) => request<Session>('/api/sessions', { method: 'POST', body: JSON.stringify(s) }),
  killSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/kill`, { method: 'POST' }),
  restartSession: (id: string, size: TerminalSize) =>
    request<Session>(`/api/sessions/${enc(id)}/restart`, { method: 'POST', body: JSON.stringify(size) }),
  deleteSession: (id: string) => request<void>(`/api/sessions/${enc(id)}`, { method: 'DELETE' }),
};
```

`web/src/terminal/ptySocket.ts`:

```ts
import type { PtyClientMessage } from '../api/gen/PtyClientMessage';
import type { PtyServerMessage } from '../api/gen/PtyServerMessage';

/** The subset of WebSocket that PtySocket uses (lets tests supply a fake). */
export interface SocketLike {
  binaryType: BinaryType;
  readyState: number;
  onopen: ((ev: Event) => void) | null;
  onmessage: ((ev: MessageEvent) => void) | null;
  onclose: ((ev: CloseEvent) => void) | null;
  send(data: string | ArrayBufferLike | ArrayBufferView): void;
  close(): void;
}

export type PtyStatus = 'connecting' | 'open' | 'reconnecting' | 'exited' | 'failed';

export interface PtyHandlers {
  onData(bytes: Uint8Array): void;
  /** Called when each connection opens, before the server replays the scrollback. Clear the screen here. */
  onReset(): void;
  onExit(code: number): void;
  onStatus?(status: PtyStatus): void;
}

export interface PtySocketOptions {
  baseDelayMs: number;
  maxDelayMs: number;
  /** Consecutive connections that close without ever opening before giving up (status 'failed'). */
  maxFailures: number;
}

const OPEN = 1;
const DEFAULTS: PtySocketOptions = { baseDelayMs: 500, maxDelayMs: 5000, maxFailures: 8 };

export function ptyUrl(sessionId: string, loc: Pick<Location, 'protocol' | 'host'> = window.location): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/sessions/${encodeURIComponent(sessionId)}/pty`;
}

/** A PTY WebSocket that reconnects with backoff until the process exits or the owner disposes it. */
export class PtySocket {
  private sock: SocketLike | null = null;
  private size: { cols: number; rows: number } | null = null;
  private attempt = 0;
  private failures = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private exited = false;
  private disposed = false;
  private readonly encoder = new TextEncoder();
  private readonly opts: PtySocketOptions;

  constructor(
    private readonly url: string,
    private readonly h: PtyHandlers,
    private readonly factory: (url: string) => SocketLike = (u) => new WebSocket(u),
    opts: Partial<PtySocketOptions> = {},
  ) {
    this.opts = { ...DEFAULTS, ...opts };
  }

  connect(): void {
    if (this.disposed || this.exited) return;
    this.h.onStatus?.(this.attempt === 0 ? 'connecting' : 'reconnecting');
    const s = this.factory(this.url);
    s.binaryType = 'arraybuffer';
    let opened = false;
    s.onopen = () => {
      opened = true;
      this.attempt = 0;
      this.failures = 0;
      this.h.onReset();
      this.h.onStatus?.('open');
      if (this.size) this.control({ type: 'resize', ...this.size });
    };
    s.onmessage = (ev) => {
      if (typeof ev.data === 'string') {
        const msg = JSON.parse(ev.data) as PtyServerMessage;
        if (msg.type === 'exit') {
          this.exited = true;
          this.h.onStatus?.('exited');
          this.h.onExit(msg.code);
        }
        return;
      }
      this.h.onData(new Uint8Array(ev.data as ArrayBuffer));
    };
    s.onclose = () => {
      if (this.sock === s) this.sock = null;
      if (!opened) this.failures++;
      this.scheduleReconnect();
    };
    this.sock = s;
  }

  private scheduleReconnect(): void {
    if (this.disposed || this.exited) return;
    if (this.failures >= this.opts.maxFailures) {
      this.h.onStatus?.('failed');
      return;
    }
    const delay = Math.min(this.opts.maxDelayMs, this.opts.baseDelayMs * 2 ** this.attempt);
    this.attempt++;
    this.timer = setTimeout(() => this.connect(), delay);
  }

  /** Manual reconnect after the socket gave up. */
  retry(): void {
    if (this.timer) clearTimeout(this.timer);
    this.failures = 0;
    this.attempt = 1;
    this.connect();
  }

  send(text: string): void {
    if (this.sock?.readyState === OPEN) this.sock.send(this.encoder.encode(text));
  }

  resize(cols: number, rows: number): void {
    this.size = { cols, rows };
    this.control({ type: 'resize', cols, rows });
  }

  private control(msg: PtyClientMessage): void {
    if (this.sock?.readyState === OPEN) this.sock.send(JSON.stringify(msg));
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer) clearTimeout(this.timer);
    this.sock?.close();
    this.sock = null;
  }
}
```

> In the `'streams data…'` test, the resize is sent at open time (`size` was set before `connect`), and `resize()` before open sends nothing. The order of events (`status:connecting`, `reset`, `status:open`) matches this implementation; keep them aligned.

`web/src/terminal/theme.ts`:

```ts
import type { ITheme } from '@xterm/xterm';

/** Reads a design token from the document (tokens.css is the only place colors live). */
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function terminalTheme(): ITheme {
  return {
    background: cssVar('--pdt-bg'),
    foreground: cssVar('--pdt-fg'),
    cursor: cssVar('--pdt-fg'),
    selectionBackground: cssVar('--pdt-accent-soft'),
  };
}
```

`web/src/terminal/TerminalView.tsx`:

```tsx
import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { Button } from '@ui';
import { PtySocket, ptyUrl, type PtyStatus } from './ptySocket';
import { cssVar, terminalTheme } from './theme';

export function TerminalView({ sessionId }: { sessionId: string }) {
  const host = useRef<HTMLDivElement>(null);
  const sockRef = useRef<PtySocket | null>(null);
  const [status, setStatus] = useState<PtyStatus>('connecting');

  useEffect(() => {
    const el = host.current!;
    const term = new Terminal({
      fontFamily: cssVar('--pdt-font-mono') || 'monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 5000,
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    const sock = new PtySocket(ptyUrl(sessionId), {
      onData: (bytes) => term.write(bytes),
      onReset: () => term.reset(),
      onExit: (code) => term.write(`\r\n\x1b[2m[process exited with code ${code}]\x1b[0m\r\n`),
      onStatus: setStatus,
    });
    sockRef.current = sock;
    const refit = () => {
      try {
        fit.fit();
      } catch {
        // Not laid out yet (hidden panel); the next resize fixes it.
      }
      sock.resize(term.cols, term.rows);
    };
    refit();
    const input = term.onData((d) => sock.send(d));
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(refit);
    observer?.observe(el);
    sock.connect();
    return () => {
      observer?.disconnect();
      input.dispose();
      sock.dispose();
      term.dispose();
      sockRef.current = null;
    };
  }, [sessionId]);

  return (
    <div className="relative flex-1 min-h-0 bg-bg">
      <div ref={host} data-testid="terminal" className="absolute inset-0 p-1" />
      {status === 'reconnecting' && <p className="absolute right-2 top-1 text-xs text-waiting">Reconnecting…</p>}
      {status === 'failed' && (
        <div role="alert" className="absolute inset-x-2 top-2 flex items-center gap-2 rounded-pdt border border-border bg-surface p-2 text-xs">
          <span className="flex-1">Could not connect to this terminal.</span>
          <Button onClick={() => sockRef.current?.retry()}>Reconnect</Button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `pnpm --dir web test; pnpm --dir web typecheck; pnpm --dir web build`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add web pnpm-lock.yaml pnpm-workspace.yaml
git commit -m "feat(web): xterm terminal view over a reconnecting PTY socket"
```

---

### Task 10: Sessions UI, terminal panel and accessibility carry-overs

**Files:**
- Create:
  - `web/src/sessions/SessionList.tsx`, `web/src/sessions/SessionView.tsx`, `web/src/sessions/WorktreeForm.tsx`
  - `web/src/sessions/status.ts`, `web/src/sessions/sessions.test.tsx`
  - `web/src/test-utils.tsx`
- Modify:
  - `web/src/shell/AppShell.tsx`, `web/src/shell/BottomPanel.tsx`, `web/src/shell/IconRail.tsx`
  - `web/src/shell/AppShell.test.tsx`
  - `web/src/projects/ProjectsList.tsx`, `web/src/projects/AddProjectForm.tsx`

**Interfaces:**
- Consumes: the Task 9 `api.*` functions and `TerminalView`.
- Produces:
  - Selecting a project shows its sessions, grouped under "Repository" and each worktree branch. Each group has **Shell / Claude / Codex** buttons, labelled `New <Shell|Claude|Codex> in <group>`.
  - Selecting a session opens it in the work area, with Stop, Restart and Remove controls.
  - The bottom panel shows tabs for the project's running shells, a **New shell** button and a **Maximize** toggle. A visible **Toggle terminal panel** button opens it (a carry-over item).
  - The session list re-polls every 2 s.

- [ ] **Step 1: Add the test helpers**

`web/src/test-utils.tsx`:

```tsx
import { render } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { vi } from 'vitest';

export function renderWithQuery(ui: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

export interface Call { method: string; path: string; body: unknown }
type Route = unknown | ((call: Call) => unknown);

/** Stubs fetch with `"METHOD /path"` routes. `undefined` responds 204; unknown routes respond 404. */
export function stubApi(routes: Record<string, Route>): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = {
      method: (init?.method ?? 'GET').toUpperCase(),
      path: String(input),
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    };
    calls.push(call);
    const key = `${call.method} ${call.path}`;
    if (!(key in routes)) return new Response(JSON.stringify({ code: 'not_found', message: key }), { status: 404 });
    const route = routes[key];
    const value = typeof route === 'function' ? (route as (c: Call) => unknown)(call) : route;
    return value === undefined ? new Response(null, { status: 204 }) : new Response(JSON.stringify(value), { status: 200 });
  }));
  return calls;
}
```

- [ ] **Step 2: Write the failing tests**

`web/src/sessions/sessions.test.tsx`:

```tsx
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { SessionList } from './SessionList';
import { SessionView } from './SessionView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

afterEach(() => vi.unstubAllGlobals());

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const session = (over: Partial<Session>): Session => ({
  id: 's1', projectId: 'p', worktreeId: null, kind: 'shell', agent: null, title: 'Shell', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null, ...over,
});
const worktree = { id: 'w', projectId: 'p', branch: 'feature/x', path: '/r/.worktrees/feature-x', baseCommit: 'c', createdAt: 1 };

describe('SessionList', () => {
  it('groups sessions under the repository and worktrees', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [worktree],
      'GET /api/projects/p/sessions': [session({}), session({ id: 's2', worktreeId: 'w', kind: 'agent_tui', agent: 'claude', title: 'Claude' })],
    });
    renderWithQuery(<SessionList project={project} selectedId="s2" onSelect={() => {}} />);
    const repo = await screen.findByRole('region', { name: 'Repository' });
    expect(repo).toHaveTextContent('Shell');
    const wt = screen.getByRole('region', { name: 'feature/x' });
    expect(wt).toHaveTextContent('Claude');
    expect(screen.getByRole('button', { name: /Claude/, current: true })).toBeInTheDocument();
  });

  it('starts sessions in the chosen place and selects them', async () => {
    const calls = stubApi({
      'GET /api/projects/p/worktrees': [worktree],
      'GET /api/projects/p/sessions': [],
      'POST /api/sessions': (c) => session({ id: 'new', ...(c.body as object) }),
    });
    const onSelect = vi.fn();
    renderWithQuery(<SessionList project={project} selectedId={null} onSelect={onSelect} />);
    await userEvent.click(await screen.findByRole('button', { name: 'New Shell in Repository' }));
    await userEvent.click(screen.getByRole('button', { name: 'New Claude in feature/x' }));
    const posts = calls.filter((c) => c.method === 'POST').map((c) => c.body);
    expect(posts).toEqual([
      { projectId: 'p', kind: 'shell', cols: 120, rows: 32 },
      { projectId: 'p', worktreeId: 'w', kind: 'agent_tui', agent: 'claude', cols: 120, rows: 32 },
    ]);
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('new'));
  });

  it('shows server errors', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionList project={project} selectedId={null} onSelect={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'New Codex in Repository' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('POST /api/sessions');
  });
});

describe('SessionView', () => {
  it('stops a running session', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/kill': undefined, 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({})} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('s1');
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/s1/kill')).toBe(true));
  });

  it('offers restart for an interrupted agent and shows no terminal', async () => {
    const calls = stubApi({
      'POST /api/sessions/s1/restart': session({ kind: 'agent_tui', agent: 'claude' }),
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionView session={session({ status: 'interrupted', kind: 'agent_tui', agent: 'claude', title: 'Claude' })} onRemoved={() => {}} />);
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    expect(screen.getByText(/resumes its last conversation/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/restart')?.body).toEqual({ cols: 120, rows: 32 }));
  });

  it('removes an ended session', async () => {
    stubApi({ 'DELETE /api/sessions/s1': undefined, 'GET /api/projects/p/sessions': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={onRemoved} />);
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
  });
});
```

Replace `web/src/shell/AppShell.test.tsx` with:

```tsx
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null };

function stubAlpha() {
  stubApi({
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: [shell],
  });
}

describe('AppShell', () => {
  it('renders the five rail views and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [] });
    renderWithQuery(<AppShell />);
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('toggles the bottom panel with Ctrl+` and with the on-screen button', async () => {
    stubApi({ 'GET /api/projects': [] });
    renderWithQuery(<AppShell />);
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    const toggle = screen.getByRole('button', { name: 'Toggle terminal panel' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toHaveTextContent('Select a project to open a terminal.');
  });

  it('shows the selected project name, not its id', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    const main = screen.getByRole('main');
    expect(main).toHaveTextContent('Project Alpha');
    expect(main).not.toHaveTextContent(project.id);
    expect(screen.getByRole('button', { name: 'Alpha' })).toHaveAttribute('aria-current', 'true');
  });

  it('opens a selected session in the work area and shells as panel tabs', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    // The session row's name includes its StatusDot label ("running Shell"), unlike "New Shell in Repository".
    await userEvent.click(await screen.findByRole('button', { name: 'running Shell' }));
    expect(screen.getByRole('main')).toContainElement(screen.getByTestId('terminal-view'));
    await userEvent.click(screen.getByRole('button', { name: 'Toggle terminal panel' }));
    expect(await screen.findByRole('tab', { name: /Shell/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('button', { name: 'Maximize terminal panel' }));
    expect(screen.getByRole('button', { name: 'Restore terminal panel' })).toBeInTheDocument();
    expect(screen.getAllByTestId('terminal-view')).toHaveLength(1);
  });
});
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `pnpm --dir web test`
Expected: failures.

- [ ] **Step 4: Implement**

`web/src/sessions/status.ts`:

```ts
import type { Status } from '@ui';
import type { SessionStatus } from '../api/gen/SessionStatus';

export const INITIAL_SIZE = { cols: 120, rows: 32 } as const;

export function toDot(status: SessionStatus): Status {
  switch (status) {
    case 'running': return 'running';
    case 'interrupted': return 'waiting';
    case 'finished': return 'idle';
  }
}
```

`web/src/sessions/SessionList.tsx`:

```tsx
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { NewSession } from '../api/gen/NewSession';
import type { Project } from '../api/gen/Project';
import { INITIAL_SIZE, toDot } from './status';
import { WorktreeForm } from './WorktreeForm';

type Choice = 'Shell' | 'Claude' | 'Codex';
const CHOICES: Choice[] = ['Shell', 'Claude', 'Codex'];

function newSession(projectId: string, worktreeId: string | null, choice: Choice): NewSession {
  const base = { projectId, ...(worktreeId ? { worktreeId } : {}) };
  return choice === 'Shell'
    ? { ...base, kind: 'shell', ...INITIAL_SIZE }
    : { ...base, kind: 'agent_tui', agent: choice === 'Claude' ? 'claude' : 'codex', ...INITIAL_SIZE };
}

const small = 'rounded-pdt px-1.5 py-0.5 text-[11px] text-muted hover:text-fg hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function SessionList({ project, selectedId, onSelect }: {
  project: Project;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const qc = useQueryClient();
  const worktrees = useQuery({ queryKey: ['worktrees', project.id], queryFn: () => api.listWorktrees(project.id) });
  const sessions = useQuery({
    queryKey: ['sessions', project.id],
    queryFn: () => api.listSessions(project.id),
    refetchInterval: 2000,
  });
  const create = useMutation({
    mutationFn: api.createSession,
    onSuccess: (s) => {
      void qc.invalidateQueries({ queryKey: ['sessions', project.id] });
      onSelect(s.id);
    },
  });

  if (worktrees.isPending || sessions.isPending) return <p className="px-2 text-xs text-muted">Loading…</p>;
  if (worktrees.isError || sessions.isError) {
    return <p role="alert" className="px-2 text-xs text-failed">{(worktrees.error ?? sessions.error)?.message}</p>;
  }
  const groups = [
    { id: null as string | null, label: 'Repository' },
    ...worktrees.data.map((w) => ({ id: w.id as string | null, label: w.branch })),
  ];
  const error = create.error;

  return (
    <div className="flex flex-col gap-2 py-2">
      {groups.map((g) => (
        <section key={g.id ?? 'repo'} aria-label={g.label}>
          <div className="flex items-center gap-0.5 px-2 text-xs text-muted">
            <span className="flex-1 truncate font-medium" title={g.label}>{g.label}</span>
            {CHOICES.map((c) => (
              <button
                key={c}
                type="button"
                aria-label={`New ${c} in ${g.label}`}
                disabled={create.isPending}
                onClick={() => create.mutate(newSession(project.id, g.id, c))}
                className={small}
              >
                {c}
              </button>
            ))}
          </div>
          <ul className="flex flex-col gap-0.5">
            {sessions.data.filter((s) => s.worktreeId === g.id).map((s) => (
              <li key={s.id}>
                <button
                  type="button"
                  aria-current={s.id === selectedId ? 'true' : undefined}
                  onClick={() => onSelect(s.id)}
                  className={`w-full flex items-center gap-2 text-left px-3 py-1 rounded-pdt text-sm ${s.id === selectedId ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
                >
                  <StatusDot status={toDot(s.status)} />
                  <span className="truncate">{s.title}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {error && <p role="alert" className="px-2 text-xs text-failed">{error.message}</p>}
      <WorktreeForm projectId={project.id} />
    </div>
  );
}
```

`web/src/sessions/WorktreeForm.tsx`:

```tsx
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';

const field = 'w-full rounded-pdt border border-border bg-surface px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';

export function WorktreeForm({ projectId }: { projectId: string }) {
  const qc = useQueryClient();
  const [branch, setBranch] = useState('');
  const m = useMutation({
    mutationFn: (b: string) => api.createWorktree(projectId, { branch: b }),
    onSuccess: () => {
      setBranch('');
      void qc.invalidateQueries({ queryKey: ['worktrees', projectId] });
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    m.mutate(branch);
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-2 px-2">
      <label className="text-xs text-muted">
        New worktree branch
        <input className={field} value={branch} required aria-invalid={m.isError || undefined} onChange={(e) => setBranch(e.target.value)} />
      </label>
      {m.isError && <p role="alert" className="text-xs text-failed">{m.error.message}</p>}
      <Button type="submit" disabled={m.isPending}>Create worktree</Button>
    </form>
  );
}
```

`web/src/sessions/SessionView.tsx`:

```tsx
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Session } from '../api/gen/Session';
import { TerminalView } from '../terminal/TerminalView';
import { INITIAL_SIZE, toDot } from './status';

export function SessionView({ session, onRemoved }: { session: Session; onRemoved: () => void }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['sessions', session.projectId] });
  const kill = useMutation({ mutationFn: () => api.killSession(session.id), onSuccess: refresh });
  const restart = useMutation({ mutationFn: () => api.restartSession(session.id, INITIAL_SIZE), onSuccess: refresh });
  const remove = useMutation({
    mutationFn: () => api.deleteSession(session.id),
    onSuccess: () => {
      void refresh();
      onRemoved();
    },
  });
  const running = session.status === 'running';
  const error = kill.error ?? restart.error ?? remove.error;

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <StatusDot status={toDot(session.status)} />
        <span className="font-medium">{session.title}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-muted" title={session.cwd}>{session.cwd}</span>
        {running ? (
          <Button variant="ghost" onClick={() => kill.mutate()}>Stop</Button>
        ) : (
          <>
            <Button variant="ghost" onClick={() => restart.mutate()}>Restart</Button>
            <Button variant="ghost" onClick={() => remove.mutate()}>Remove</Button>
          </>
        )}
      </header>
      {error && <p role="alert" className="px-3 py-1 text-xs text-failed">{error.message}</p>}
      {session.status === 'interrupted' ? (
        <p className="p-4 text-sm text-muted">
          This session stopped when the daemon shut down. Restart it to continue
          {session.kind === 'agent_tui' ? '; the agent resumes its last conversation' : ''}.
        </p>
      ) : (
        // Remount on status change so a restarted session gets a fresh socket.
        <TerminalView key={`${session.id}-${session.status}`} sessionId={session.id} />
      )}
    </div>
  );
}
```

`web/src/shell/BottomPanel.tsx`:

```tsx
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Maximize2, Minimize2, Plus } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { TerminalView } from '../terminal/TerminalView';

export function BottomPanel({ project, maximized, onToggleMaximize }: {
  project: Project | null;
  maximized: boolean;
  onToggleMaximize: () => void;
}) {
  const qc = useQueryClient();
  const sessions = useQuery({
    queryKey: ['sessions', project?.id ?? null],
    queryFn: () => api.listSessions(project!.id),
    enabled: project !== null,
    refetchInterval: 2000,
  });
  const shells = (sessions.data ?? []).filter((s) => s.kind === 'shell' && s.status === 'running');
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = shells.find((s) => s.id === activeId) ?? shells[0] ?? null;
  const create = useMutation({
    mutationFn: () => api.createSession({ projectId: project!.id, kind: 'shell', cols: 120, rows: 12 }),
    onSuccess: (s) => {
      setActiveId(s.id);
      void qc.invalidateQueries({ queryKey: ['sessions', project!.id] });
    },
  });

  return (
    <section aria-label="Terminal panel" className={`${maximized ? 'flex-1' : 'h-64'} border-t border-border bg-surface flex flex-col min-h-0`}>
      <div className="flex items-center gap-1 px-2 py-1 border-b border-border text-xs">
        <div role="tablist" aria-label="Terminals" className="flex items-center gap-1 overflow-x-auto">
          {shells.map((s, i) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              aria-selected={s.id === active?.id}
              onClick={() => setActiveId(s.id)}
              className={`rounded-pdt px-2 py-0.5 ${s.id === active?.id ? 'bg-accent-soft text-fg' : 'text-muted hover:bg-surface-2'}`}
            >
              {s.title} {i + 1}
            </button>
          ))}
        </div>
        <Button variant="ghost" aria-label="New shell" disabled={!project || create.isPending} onClick={() => create.mutate()}>
          <Plus size={14} aria-hidden />
        </Button>
        <span className="flex-1" />
        <Button variant="ghost" aria-label={maximized ? 'Restore terminal panel' : 'Maximize terminal panel'} onClick={onToggleMaximize}>
          {maximized ? <Minimize2 size={14} aria-hidden /> : <Maximize2 size={14} aria-hidden />}
        </Button>
      </div>
      {create.isError && <p role="alert" className="px-3 py-1 text-xs text-failed">{create.error.message}</p>}
      {!project ? (
        <p className="p-3 text-xs text-muted">Select a project to open a terminal.</p>
      ) : active ? (
        <TerminalView key={active.id} sessionId={active.id} />
      ) : (
        <p className="p-3 text-xs text-muted">No shells running. Press + to start one.</p>
      )}
    </section>
  );
}
```

`web/src/shell/AppShell.tsx`: rewrite the component body. Keep `PHASE_NOTE` and the keyboard effect.

```tsx
import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { PanelBottom } from 'lucide-react';
import { Button } from '@ui';
import { api } from '../api/client';
import { ProjectsList } from '../projects/ProjectsList';
import { AddProjectForm } from '../projects/AddProjectForm';
import { SessionList } from '../sessions/SessionList';
import { SessionView } from '../sessions/SessionView';
import { BottomPanel } from './BottomPanel';
import { IconRail } from './IconRail';
import { VIEWS, type ViewId } from './views';

// PHASE_NOTE unchanged

export function AppShell() {
  const [view, setView] = useState<ViewId>('sessions');
  const [bottomOpen, setBottomOpen] = useState(false);
  const [bottomMax, setBottomMax] = useState(false);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);

  useEffect(() => { /* Ctrl+` effect unchanged */ }, []);

  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const project = projects.data?.find((p) => p.id === projectId) ?? null;
  const sessions = useQuery({
    queryKey: ['sessions', projectId],
    queryFn: () => api.listSessions(projectId!),
    enabled: projectId !== null,
    refetchInterval: 2000,
  });
  const session = sessions.data?.find((s) => s.id === sessionId) ?? null;
  const selectProject = (id: string) => {
    setProjectId(id);
    setSessionId(null);
  };
  const label = VIEWS.find((v) => v.id === view)!.label;

  return (
    <div className="h-full flex flex-col-reverse md:flex-row bg-bg text-fg">
      <IconRail active={view} onSelect={setView} />
      <aside aria-label={`${label} list`} className="md:w-72 max-h-[40vh] md:max-h-none border-border md:border-r flex flex-col min-h-0">
        <h2 className="px-3 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted">{label}</h2>
        <div className="flex-1 overflow-auto px-1">
          {view === 'sessions' ? (
            <>
              <ProjectsList selectedId={projectId} onSelect={selectProject} />
              {project && <SessionList project={project} selectedId={sessionId} onSelect={setSessionId} />}
              <AddProjectForm />
            </>
          ) : (
            <p className="px-2 text-xs text-muted">{PHASE_NOTE[view]}</p>
          )}
        </div>
      </aside>
      <main className="flex-1 flex flex-col min-w-0 min-h-0">
        <div className="flex items-center gap-2 border-b border-border px-3 py-1 text-sm">
          <span className="flex-1 truncate text-muted">{project ? `Project ${project.name}` : 'Select a project to get started.'}</span>
          <Button variant="ghost" aria-label="Toggle terminal panel" aria-expanded={bottomOpen} onClick={() => setBottomOpen((o) => !o)}>
            <PanelBottom size={16} aria-hidden />
          </Button>
        </div>
        {!(bottomOpen && bottomMax) && (
          <div className="flex-1 flex flex-col min-h-0">
            {session ? (
              <SessionView session={session} onRemoved={() => setSessionId(null)} />
            ) : (
              project && <p className="p-4 text-sm text-muted">Start a shell or an agent from the list.</p>
            )}
          </div>
        )}
        {bottomOpen && <BottomPanel project={project} maximized={bottomMax} onToggleMaximize={() => setBottomMax((m) => !m)} />}
      </main>
    </div>
  );
}
```

> The test `opens a selected session…` expects exactly one `terminal-view` once the panel is maximized: the work area is hidden, and the panel's shell tab shows the same session. Keep this behaviour.

Carry-over accessibility fixes:
- `ProjectsList.tsx`: add `aria-current={selectedId === p.id ? 'true' : undefined}` to the project button.
- `AddProjectForm.tsx`: add `required` and `aria-invalid={m.isError || undefined}` to both inputs.
- `IconRail.tsx`: add `min-h-11 min-w-11 md:min-h-0 md:min-w-0 flex items-center justify-center` to the button classes, for 44 px touch targets on mobile.

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `pnpm --dir web test; pnpm --dir web typecheck; pnpm --dir web build`
Expected: all pass. The existing `projects.test.tsx` still passes.

- [ ] **Step 6: Try it by hand (dev profile only)**

In PowerShell:

```powershell
pnpm --dir web build
$env:PDT_PROFILE='dev'; $env:PDT_WEB_DIR="$PWD\web\dist"; cargo run -p pdtd
```

1. Open `http://127.0.0.1:7421/` and add this repo as a project.
2. Start a Shell and a Claude session.
3. Type in both, open the bottom panel, and start a shell there.
4. Reload the page; the terminals redraw from scrollback.
5. Stop `pdtd` with Ctrl-C, start it again, and check that the sessions show as interrupted and Restart works.

Describe what you saw in the report. Never use port 7420.

- [ ] **Step 7: Commit**

```bash
git add web
git commit -m "feat(web): sessions list, terminal work area and panel tabs"
```

---

### Task 11: Desktop health check, startup log and error dialog

**Files:**
- Modify: `desktop/src-tauri/src/daemon.rs`, `desktop/src-tauri/src/main.rs`, `desktop/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `GET /api/health` (`{status, name, version}`) and `PRODUCT_NAME`.
- Produces:
  - `enum Probe { Free, Ours, Foreign(String) }`
  - `probe(port) -> Probe`
  - `classify_health(raw: &[u8]) -> Probe`
  - `ensure_daemon(port, probe: impl Fn(u16) -> Probe, spawn, timeout) -> Result<(), String>`
  - `launch_log_path(profile) -> Option<PathBuf>`
  - `with_log_tail(msg, profile) -> String`

**Behavior:**
- `Ours` attaches.
- `Free` spawns `pdtd` (stdout and stderr appended to `<data dir>/logs/pdtd-launch.log`), then waits for `Ours`.
- `Foreign` keeps probing until the timeout, because a starting `pdtd` may not answer yet. It then fails with `port <port> is in use by something that is not Personal Dev Team (<why>)`.
- On failure, a native error dialog shows the message plus the last 20 lines of the launch log, then the app exits with code 1.

- [ ] **Step 1: Add the dependencies**

Run (PowerShell): `cargo add -p pdt-desktop tauri-plugin-dialog@2 serde_json dirs`. Confirm that `tauri-plugin-dialog` resolves to 2.x (2.8.3 at the time of writing). Never use the 3.0 alpha.

- [ ] **Step 2: Write the failing tests**

Replace the tests module in `daemon.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    /// Serves `response` to every connection on a background thread; returns the port.
    fn fake_server(response: &'static str) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn classifies_health_responses() {
        let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"status\":\"ok\",\"name\":\"Personal Dev Team\",\"version\":\"0.1.0\"}";
        assert_eq!(classify_health(ok), Probe::Ours);
        assert!(matches!(classify_health(b"HTTP/1.1 200 OK\r\n\r\n{\"name\":\"other\"}"), Probe::Foreign(w) if w.contains("other")));
        assert!(matches!(classify_health(b"HTTP/1.1 404 Not Found\r\n\r\n"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"SSH-2.0-OpenSSH"), Probe::Foreign(_)));
    }

    #[test]
    fn probes_real_sockets() {
        assert_eq!(probe(free_port()), Probe::Free);
        let ours = fake_server("HTTP/1.0 200 OK\r\n\r\n{\"name\":\"Personal Dev Team\"}");
        assert_eq!(probe(ours), Probe::Ours);
        let foreign = fake_server("HTTP/1.0 200 OK\r\n\r\n<html>hi</html>");
        assert!(matches!(probe(foreign), Probe::Foreign(_)));
    }

    #[test]
    fn attaches_without_spawning_when_ours() {
        let spawned = Cell::new(false);
        ensure_daemon(1, |_| Probe::Ours, || { spawned.set(true); Ok(()) }, Duration::from_millis(300)).unwrap();
        assert!(!spawned.get());
    }

    #[test]
    fn spawns_when_free_and_waits_for_health() {
        let calls = Cell::new(0);
        let probe = |_| {
            calls.set(calls.get() + 1);
            if calls.get() < 4 { Probe::Free } else { Probe::Ours }
        };
        let spawned = Cell::new(false);
        ensure_daemon(1, probe, || { spawned.set(true); Ok(()) }, Duration::from_secs(3)).unwrap();
        assert!(spawned.get());
    }

    #[test]
    fn foreign_port_fails_without_spawning() {
        let spawned = Cell::new(false);
        let err = ensure_daemon(9, |_| Probe::Foreign("it identifies as \"x\"".into()), || { spawned.set(true); Ok(()) }, Duration::from_millis(300))
            .unwrap_err();
        assert!(!spawned.get());
        assert!(err.contains("port 9 is in use by something that is not Personal Dev Team"), "{err}");
    }

    #[test]
    fn timeout_and_spawn_failure_messages() {
        let err = ensure_daemon(9, |_| Probe::Free, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
        let err = ensure_daemon(9, |_| Probe::Free, || Err(std::io::Error::other("nope")), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("failed to start pdtd"), "{err}");
    }

    #[test]
    fn log_tail_is_appended() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("pdtd-launch.log");
        std::fs::write(&log, (1..=30).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let msg = append_tail("boom".into(), &log, 20);
        assert!(msg.starts_with("boom"));
        assert!(msg.contains("line 30") && msg.contains("line 11") && !msg.contains("line 10\n"));
        assert_eq!(append_tail("boom".into(), &tmp.path().join("missing.log"), 20), "boom");
    }
}
```

Run: `cargo add -p pdt-desktop --dev tempfile`.

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `pnpm --dir web build; cargo test -p pdt-desktop`
Expected: compile errors.

- [ ] **Step 4: Implement**

`daemon.rs` (replace `is_listening` and `ensure_daemon`; keep `pdtd_path`):

```rust
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME, Profile};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub enum Probe {
    /// Nothing is listening.
    Free,
    /// A Personal Dev Team daemon answered /api/health.
    Ours,
    /// Something else (or a daemon still starting up) holds the port.
    Foreign(String),
}

pub fn probe(port: u16) -> Probe {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else { return Probe::Free };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    if let Err(e) = s.write_all(req.as_bytes()) {
        return Probe::Foreign(e.to_string());
    }
    let mut buf = Vec::new();
    if let Err(e) = s.take(64 * 1024).read_to_end(&mut buf) {
        return Probe::Foreign(e.to_string());
    }
    classify_health(&buf)
}

pub fn classify_health(raw: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Probe::Foreign("it did not answer like a web server".into());
    };
    let status_line = head.lines().next().unwrap_or("");
    if !status_line.starts_with("HTTP/1.") || status_line.split_whitespace().nth(1) != Some("200") {
        return Probe::Foreign(format!("its health check answered {status_line:?}"));
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v["name"] == PRODUCT_NAME => Probe::Ours,
        Ok(v) => Probe::Foreign(format!("it identifies as {}", v["name"])),
        Err(_) => Probe::Foreign("its health check is not JSON".into()),
    }
}

/// Attaches to a running daemon, or spawns one and waits until it answers its health check.
pub fn ensure_daemon(
    port: u16,
    probe: impl Fn(u16) -> Probe,
    spawn: impl FnOnce() -> std::io::Result<()>,
    timeout: Duration,
) -> Result<(), String> {
    let mut last = probe(port);
    match last {
        Probe::Ours => return Ok(()),
        Probe::Free => spawn().map_err(|e| format!("failed to start {DAEMON_BIN}: {e}"))?,
        Probe::Foreign(_) => {}
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        last = probe(port);
        if last == Probe::Ours {
            return Ok(());
        }
    }
    Err(match last {
        Probe::Foreign(why) => format!("port {port} is in use by something that is not {PRODUCT_NAME} ({why})"),
        _ => format!("{DAEMON_BIN} did not start listening on port {port} within {timeout:?}"),
    })
}

/// Where the desktop app sends a spawned daemon's stdout/stderr (startup errors land here before pdtd's own log).
pub fn launch_log_path(profile: Profile) -> Option<PathBuf> {
    let data_dir = match std::env::var("PDT_DATA_DIR") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => dirs::data_local_dir()?.join(profile.data_dir_name()),
    };
    Some(data_dir.join("logs").join(format!("{DAEMON_BIN}-launch.log")))
}

pub fn with_log_tail(msg: String, profile: Profile) -> String {
    match launch_log_path(profile) {
        Some(p) => append_tail(msg, &p, 20),
        None => msg,
    }
}

fn append_tail(msg: String, log: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(log) else { return msg };
    let all: Vec<&str> = text.lines().collect();
    let tail = all[all.len().saturating_sub(lines)..].join("\n");
    if tail.trim().is_empty() { msg } else { format!("{msg}\n\nLast lines of {}:\n{tail}", log.display()) }
}
```

In `spawn_pdtd`, replace the `stdout`/`stderr` null redirects with the launch log:

```rust
    let log = launch_log_path(profile).and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::File::options().create(true).append(true).open(p).ok()
    });
    let (out, err) = match log.and_then(|f| Some((f.try_clone().ok()?, f))) {
        Some((a, b)) => (Stdio::from(a), Stdio::from(b)),
        None => (Stdio::null(), Stdio::null()),
    };
    cmd.env("PDT_PROFILE", if profile == Profile::Dev { "dev" } else { "stable" })
        .env("PDT_PORT", profile.port().to_string())
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
```

`main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::time::Duration;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

fn main() {
    let profile = Profile::from_env_value(std::env::var("PDT_PROFILE").ok().as_deref());
    let port = profile.port();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            if let Err(e) = daemon::ensure_daemon(port, daemon::probe, || daemon::spawn_pdtd(profile), Duration::from_secs(15)) {
                // Non-blocking: blocking dialogs must not run on the main thread, where setup runs.
                app.dialog()
                    .message(daemon::with_log_tail(e, profile))
                    .kind(MessageDialogKind::Error)
                    .title(PRODUCT_NAME)
                    .show(|_| std::process::exit(1));
                return Ok(());
            }
            let url = format!("http://127.0.0.1:{port}/").parse()?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title(PRODUCT_NAME)
                .inner_size(1440.0, 900.0)
                .min_inner_size(360.0, 480.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the desktop shell");
}
```

> If the dialog plugin's 2.x API differs (`DialogExt`, `MessageDialogKind`, `.show(cb)`), adapt it and note the change in your report. If the plugin needs a capability for Rust-only use, add a minimal `capabilities/default.json` and say so.

- [ ] **Step 5: Run the tests and confirm they pass; try it by hand**

Run: `pnpm --dir web build; cargo test -p pdt-desktop; cargo clippy -p pdt-desktop --all-targets -- -D warnings`
Expected: all pass.

Try it by hand (dev profile only):
1. With nothing on 7421, run `$env:PDT_PROFILE='dev'; $env:PDT_WEB_DIR="$PWD\web\dist"; cargo build -p pdtd; cargo run -p pdt-desktop`. The window opens.
2. Close it. In another terminal, run `python -m http.server 7421` (or any other listener) and launch the app again. An error dialog names the foreign port.

Describe both results in the report.

- [ ] **Step 6: Commit**

```bash
git add desktop Cargo.lock
git commit -m "feat(desktop): health-checked attach, launch log, error dialog"
```

---

### Task 12: Stable installer, release scripts, docs, CI and carry-over close-out

**Files:**
- Create:
  - `desktop/src-tauri/tauri.bundle.conf.json`
  - `scripts/stage-pdtd.mjs`
  - `scripts/release-stable.ps1`, `scripts/release-stable.sh`
- Modify:
  - `crates/pdtd/src/config.rs` (find `web/` inside a macOS app bundle)
  - `.gitignore` (`desktop/src-tauri/binaries/`)
  - `CLAUDE.md`
  - `crates/pdt-core/src/brand.rs` (doc comment)
  - `.github/workflows/ci.yml`
  - `docs/superpowers/notes/phase-0-carryover.md`
  - `web/ui/components.test.tsx`

**Interfaces:**
- Consumes: `POST /api/shutdown` (Task 8), and the desktop app's rule that `pdtd` sits next to its exe.
- Produces:
  - A per-user installer: NSIS on Windows, a `.app` on macOS.
    - It bundles `pdtd` as a Tauri sidecar (`externalBin`) and `web/dist` as the `web` resource.
    - `<install>/web` (Windows) and `Contents/Resources/web` (macOS) are both found by `pdtd`.
  - `scripts/release-stable.*` builds the installer, stops the stable `pdtd`, installs silently and launches the app. The `-BuildOnly` / `--build-only` flag stops after building.

**Why a separate bundle config:**
- `tauri-build` checks at compile time that every `externalBin` file exists.
- If the sidecar were in `tauri.conf.json`, every `cargo build -p pdt-desktop` (dev runs, CI clippy and tests) would need a staged release `pdtd` first.
- `tauri build --config src-tauri/tauri.bundle.conf.json` merges the bundling settings only when an installer is built.

- [ ] **Step 1: Write the failing `pdtd` config test**

Add to `crates/pdtd/src/config.rs` tests:

```rust
    #[test]
    fn web_dir_inside_a_macos_app_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let macos = tmp.path().join("Contents").join("MacOS");
        let resources_web = tmp.path().join("Contents").join("Resources").join("web");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources_web).unwrap();
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), Some(macos)).unwrap();
        assert_eq!(c.web_dir.map(|d| std::fs::canonicalize(d).unwrap()), Some(std::fs::canonicalize(resources_web).unwrap()));
    }
```

Run: `cargo test -p pdtd web_dir` and confirm it **fails**.

- [ ] **Step 2: Implement the lookup**

In `from_lookup`, replace the `web_dir` expression with:

```rust
        // Next to the exe (Windows installer, dev layouts), or in a macOS bundle's Contents/Resources.
        let bundled_web = |d: PathBuf| [d.join("web"), d.join("..").join("Resources").join("web")].into_iter().find(|w| w.is_dir());
        let web_dir = lookup("PDT_WEB_DIR").map(PathBuf::from).or_else(|| exe_dir.and_then(bundled_web));
```

Run: `cargo test -p pdtd` and confirm it passes, including `web_dir_next_to_exe_when_present`.

- [ ] **Step 3: Add the bundle config and the sidecar staging script**

`desktop/src-tauri/tauri.bundle.conf.json` (merged over `tauri.conf.json` by `tauri build --config`):

```json
{
  "bundle": {
    "externalBin": ["binaries/pdtd"],
    "resources": { "../../web/dist/": "web/" },
    "windows": { "nsis": { "installMode": "currentUser" } }
  }
}
```

`scripts/stage-pdtd.mjs`:

```js
// Copies target/release/pdtd to desktop/src-tauri/binaries/pdtd-<host triple>, the name Tauri's externalBin expects.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const host = execFileSync('rustc', ['-vV'], { cwd: root, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]?.trim();
if (!host) throw new Error('could not read the host triple from `rustc -vV`');
const ext = process.platform === 'win32' ? '.exe' : '';
const out = join(root, 'desktop', 'src-tauri', 'binaries');
mkdirSync(out, { recursive: true });
copyFileSync(join(root, 'target', 'release', `pdtd${ext}`), join(out, `pdtd-${host}${ext}`));
console.log(`staged pdtd for ${host}`);
```

Add `desktop/src-tauri/binaries/` to `.gitignore`.

- [ ] **Step 4: Write the Windows release script**

`scripts/release-stable.ps1`:

```powershell
#Requires -Version 5.1
<#
Builds the Personal Dev Team installer and installs it as the *stable* app (per user, no admin).
Run this yourself from a normal terminal. Agents must never run it without -BuildOnly: it stops the stable pdtd
that hosts them. Running sessions become 'interrupted' and can be restarted from the UI (agents resume).
#>
param([switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$port = 7420
$product = 'Personal Dev Team'

function Invoke-Native([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}
function Test-Port([int]$p) {
    try { $c = New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1', $p); $c.Close(); $true } catch { $false }
}

Push-Location $root
try {
    Invoke-Native 'pnpm install' { pnpm install --frozen-lockfile }
    Invoke-Native 'web build' { pnpm --dir web build }
    Invoke-Native 'pdtd build' { cargo build --release -p pdtd }
    Invoke-Native 'stage pdtd' { node scripts/stage-pdtd.mjs }
    $bundleConf = Join-Path $root 'desktop\src-tauri\tauri.bundle.conf.json'
    Invoke-Native 'installer build' { pnpm --dir desktop exec tauri build --config $bundleConf --bundles nsis }
} finally { Pop-Location }

$setup = Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw 'no NSIS installer was produced' }
Write-Host "Built $($setup.FullName)"
if ($BuildOnly) { return }

$installDir = Join-Path $env:LOCALAPPDATA $product
$window = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" -and $_.ProcessName -ne 'pdtd' }
if ($window) { throw 'Close the stable desktop window first (its sessions keep running in pdtd).' }

if (Test-Port $port) {
    Write-Host "Stopping the stable pdtd on port $port..."
    Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" | Out-Null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Test-Port $port) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Milliseconds 250 }
    if (Test-Port $port) { throw "pdtd on port $port did not stop within 20 seconds" }
}
Get-Process pdtd -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" } |
    Wait-Process -Timeout 10 -ErrorAction SilentlyContinue

Write-Host 'Installing...'
$p = Start-Process $setup.FullName -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "installer failed (exit $($p.ExitCode))" }

$app = Get-ChildItem $installDir -Filter '*.exe' | Where-Object { $_.BaseName -notin @('pdtd', 'uninstall') } | Select-Object -First 1
if (-not $app) { throw "installed, but no app exe was found in $installDir" }
Write-Host "Installed to $installDir. Starting $($app.Name) (it starts pdtd)."
Start-Process $app.FullName
```

- [ ] **Step 5: Write the macOS release script**

`scripts/release-stable.sh`:

```bash
#!/usr/bin/env bash
# Builds the Personal Dev Team app bundle and installs it into ~/Applications as the *stable* app.
# Run this yourself. Agents must never run it without --build-only: it stops the stable pdtd that hosts them.
# Running sessions become 'interrupted' and can be restarted from the UI (agents resume).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
port=7420
product="Personal Dev Team"
build_only=false
[[ "${1:-}" == "--build-only" ]] && build_only=true

up() { curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$port/api/health"; }

cd "$root"
pnpm install --frozen-lockfile
pnpm --dir web build
cargo build --release -p pdtd
node scripts/stage-pdtd.mjs
pnpm --dir desktop exec tauri build --config "$root/desktop/src-tauri/tauri.bundle.conf.json" --bundles app

bundle="$root/target/release/bundle/macos/$product.app"
[[ -d "$bundle" ]] || { echo "no app bundle was produced at $bundle" >&2; exit 1; }
echo "Built $bundle"
$build_only && exit 0

dest="$HOME/Applications/$product.app"
# Any process from the installed bundle other than pdtd means the window is still open.
if pgrep -fl "$dest/Contents/MacOS/" | grep -v "/Contents/MacOS/pdtd" | grep -q .; then
  echo "Close the stable desktop window first (its sessions keep running in pdtd)." >&2
  exit 1
fi

if up; then
  echo "Stopping the stable pdtd on port $port..."
  curl -fsS -X POST "http://127.0.0.1:$port/api/shutdown" >/dev/null
  for _ in $(seq 1 80); do up || break; sleep 0.25; done
  if up; then echo "pdtd did not stop within 20 seconds" >&2; exit 1; fi
fi

mkdir -p "$HOME/Applications"
rm -rf "$dest"
cp -R "$bundle" "$dest"
echo "Installed to $dest. Starting it (it starts pdtd)."
open "$dest"
```

Run: `git update-index --chmod=+x scripts/release-stable.sh` after `git add`.

- [ ] **Step 6: Build the installer (build only, never install) and inspect it**

Run (PowerShell): `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-stable.ps1 -BuildOnly`
Expected: a `*-setup.exe` under `target\release\bundle\nsis\`.

Then check what the installer will lay down. Expand it with 7-Zip if available; otherwise read the generated NSIS script under `target\release\nsis\`. In your report, give:
- the install directory;
- the app exe name;
- confirmation that `pdtd.exe` and `web\index.html` sit next to the app exe.

If the install directory or exe name differs from what the script assumes (`%LOCALAPPDATA%\Personal Dev Team`, any exe that isn't `pdtd` or `uninstall`), fix the script. **Do not run the installer and do not run the script without `-BuildOnly`.** Both touch the stable instance.

`cargo build -p pdt-desktop` and `cargo test -p pdt-desktop` must still work without the staged sidecar. Delete `desktop/src-tauri/binaries/` and run both to prove it.

- [ ] **Step 7: Update the docs, brand note, CI and carry-over**

- **`crates/pdt-core/src/brand.rs`:** extend the module doc to: `A rename touches this file, desktop/src-tauri/tauri.conf.json (productName, identifier) and scripts/release-stable.* (product name, slug, port) only.`
- **`CLAUDE.md`:**
  - **Layout:** add `crates/pdt-agents — PTY terminals (later: structured agent adapters)`.
  - **Running:** add these bullets:
    - `Terminals: sessions are PTYs owned by pdtd; they survive closed windows. WebSocket at /api/sessions/{id}/pty.`
    - `Logs: <data dir>/logs/pdtd.<date>.log; the desktop app writes a spawned daemon's startup output to <data dir>/logs/pdtd-launch.log.`
    - `Worktrees live inside the repo at .worktrees/<branch> (listed in .git/info/exclude). They are removed only after merge (Phase 4).`
    - `The dev profile also accepts the Vite dev server (localhost:5173) in its Host/Origin allowlist.`
    - `Stable app: scripts/release-stable.ps1 (Windows) / scripts/release-stable.sh (macOS) build the installer (Tauri; pdtd is a sidecar via src-tauri/tauri.bundle.conf.json), stop the stable pdtd, install and launch. -BuildOnly / --build-only only builds.`
  - **Rules:** add `- Never run scripts/release-stable.* without -BuildOnly/--build-only, never run the built installer, and never POST /api/shutdown on port 7420. Only the user updates the stable app.`
- **`.github/workflows/ci.yml`:**
  - Rename the step `Rust toolchain (from rust-toolchain.toml)` to `Show Rust toolchain`, with `run: rustup show`.
  - Add a second job `bundle` that only runs on `main` and on manual runs, so the installer config can't silently break:

```yaml
  bundle:
    if: github.ref == 'refs/heads/main' || github.event_name == 'workflow_dispatch'
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: windows-latest
            bundles: nsis
          - os: macos-latest
            bundles: app
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
      - uses: Swatinem/rust-cache@v2
      - uses: pnpm/action-setup@v6
      - uses: actions/setup-node@v7
        with:
          node-version: 24
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm --dir web build
      - run: cargo build --release -p pdtd
      - run: node scripts/stage-pdtd.mjs
      - run: pnpm --dir desktop exec tauri build --config ${{ github.workspace }}/desktop/src-tauri/tauri.bundle.conf.json --bundles ${{ matrix.bundles }}
```

- **`docs/superpowers/notes/phase-0-carryover.md`:** under each item, note `Done in Phase 1 (Task N)` or `Still open`. Still open: the `GitMissing` test and the environment notes. Add a "Phase 4" line: `Worktree removal happens on merge and must refuse dirty worktrees (Phase 1 decision 3).`

Append to `web/ui/components.test.tsx` (carry-over tests):

```tsx
  it('StatusDot labels every status', () => {
    for (const s of ['running', 'waiting', 'failed', 'idle'] as const) {
      const { unmount } = render(<StatusDot status={s} />);
      expect(screen.getByRole('img', { name: s })).toBeInTheDocument();
      unmount();
    }
  });

  it('ghost Button renders with its label', () => {
    render(<Button variant="ghost">Quiet</Button>);
    expect(screen.getByRole('button', { name: 'Quiet' })).toBeInTheDocument();
  });
```

Adjust the imports to match the file's existing ones.

- [ ] **Step 8: Run the full check**

Run: `cargo fmt --all -- --check; cargo clippy --all-targets -- -D warnings; cargo test; pnpm --dir web typecheck; pnpm --dir web test; pnpm --dir web build; cargo clippy -p pdt-desktop --all-targets -- -D warnings; cargo test -p pdt-desktop`
Expected: all green. `git status --porcelain -- web/src/api/gen` is empty after `cargo test`.

- [ ] **Step 9: Commit and push the branch**

```bash
git add scripts desktop/src-tauri/tauri.bundle.conf.json .gitignore CLAUDE.md crates .github/workflows/ci.yml docs web/ui
git commit -m "feat: stable installer with pdtd sidecar, release scripts, Phase 1 notes"
git push -u origin phase-1-terminals
```

Then check that CI is green on both operating systems (`gh run list --branch phase-1-terminals`). Start the `bundle` job once by hand (`gh workflow run ci --ref phase-1-terminals`) and report its result.
