# Agent sessions: terminals, instructions, activity, restart, Devin

How PDT launches and resumes Claude, Codex and Devin sessions and the terminals they run in. Conversations (the
structured view) are in [conversations.md](conversations.md). Code: crates/pdt-agents (PTYs, `adapter/`,
`hooks.rs`, `instructions.rs`, `detached.rs`), crates/pdt-api `sessions.rs`, `pty_ws.rs`, `activity.rs`.

## Instructions and guides (PDT-46)

- pdtd serves its guides itself, built in with `include_str!` (pdt-api `guides.rs`): `GET /api/guides` lists them,
  `GET /api/guides/{name}` returns Markdown (`agents` = docs/agent-guide.md, `run-config` = docs/run-config.md,
  `set-up-project` / `set-up-workflow` = docs/prompts/*.md below their `---` line). Guides link to each other by API
  path, never by repo path (a test checks).
- Every agent session (Claude, Codex or Devin; terminal or conversation) gets PDT's instructions: a short context
  block (API address, session, project, checkout, guides; pdt-core `agent_context`) plus the project's **agent
  instructions** (`project_settings.agent_instructions`, `agentInstructions` in the settings API; kept when a PUT
  leaves it out, max 16 KB), written to `<data dir>/sessions/<id>/instructions.md`.
  - Claude gets `--append-system-prompt-file <that file>`.
  - Codex terminals get `-c developer_instructions=<TOML string>` (through a Windows `.cmd` shim, a pointer to the
    file), Codex conversations `developerInstructions` on `thread/start`/`thread/resume`; the user's own
    `developer_instructions` from config.toml come first, since PDT's value replaces them (pdt-agents
    `instructions.rs`).
  - Devin gets them from a SessionStart hook (below).
- The project's Settings page has the instructions box and "Set up with an agent" / "Write instructions with an
  agent", which start a conversation in the main checkout with the matching prompt.

## Activity

- Claude sessions get `--settings <data dir>/sessions/<id>/claude-settings.json` (hooks run
  `pdtd hook claude <Event>`).
- Codex gets `-c notify=[pdtd, hook, codex-notify]`, chaining the user's own `notify` from config.toml after
  `--chain` (pdt-agents `hooks.rs`). Terminal Codex also reads its OSC 7501 status, with screen heuristics for older
  CLIs (docs/superpowers/notes/phase-2-agent-signals.md).
- The user's own settings are never edited.

## Terminals

- Sessions are PTYs owned by pdtd; they survive closed windows. WebSocket at `/api/sessions/{id}/pty`; with
  `?replay=gzip` (the browser asks when it has DecompressionStream) the scrollback replay arrives as one gzip frame.
  Scrollback is in memory only.
- In the browser: Shift+Enter sends ESC CR (a line break in Claude Code, Codex, zsh); Compose sends a multi-line
  prompt as one bracketed paste plus Enter; a mouse selection is copied when the mouse is released, and OSC 52 copies
  also reach the viewer's clipboard (reads refused); http(s) links (plain and OSC 8; children get
  `FORCE_HYPERLINK=1`) open on Cmd/Ctrl+click or a tap, and hovering a wrapped OSC 8 link underlines all its rows
  (web/src/terminal/linkSpan.ts reads xterm's undocumented cell `extended.urlId`); Option+drag selects while the
  program owns the mouse.
- Reconnect: a terminal that connects while the startup auto-resume has yet to get to its session waits (up to 30 s)
  and attaches once it starts. The WebSocket for an existing session with no PTY otherwise upgrades, sends
  `{"type":"not_running"}` and closes; the browser then shows "Session ended." and reattaches when the session list
  shows it `running` again. Otherwise the browser retries forever (backoff capped at 5 s).

## Restart and resume

- Claude runs with `--session-id <session id>` and restarts with `--resume <conversation id>`: the session id until
  `/clear` (PDT-52) starts a new conversation, then the id Claude moved to, kept in `external_session_id`
  (`Session::claude_conversation_id`). Conversations learn it from the next `system/init`'s `session_id` (not
  `conversation_reset`'s `new_conversation_id`, which names no transcript; `conversation_reset` itself becomes a
  divider event); terminals from the SessionStart hook's `session_id` (`pdtd hook` forwards it), which also logs the
  old transcript's TUI turns and the divider so a later switch to a conversation imports the new transcript from its
  start. Resumes, transcript reads, the resume fallback, Make ticket and stdin messages all use that id.
- Resume fallback: Claude saves `<claude config dir>/projects/*/<session id>.jsonl` only after its first turn. If that
  file is missing, restart/auto-resume launches `--session-id <conversation id>` (fresh conversation, same id) instead
  of `--resume`. Config dir is `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
- Codex cannot be given an id, so pdtd finds each Codex session's conversation in
  `$CODEX_HOME/sessions/**/rollout-*.jsonl` (default `~/.codex`), matches it by working folder and start time against
  the session's `fresh_started_at` (when it was last launched without a conversation), stores it as
  `external_session_id` (at process exit or restart) and restarts with `codex resume <id>`. Fresh Codex launches in
  one folder are spaced 5 s apart so start times stay unambiguous (a create or restart can wait up to 5 s). A session
  that never took a turn restarts fresh.
- Auto-resume: at startup pdtd decides how to resume every terminal session left `interrupted` (binding Codex
  conversations) before serving, then restarts them in the background once it is serving (resume mode, 120x30;
  clients resize on attach; a manual Restart that gets there first wins and auto-resume skips that session; several
  fresh Codex sessions in one folder start 5 s apart); a session that fails to start stays `interrupted`. Stopped
  (`finished`) sessions and conversations are never auto-resumed (a conversation reopens on its next message).

## Offered agents (PDT-55)

The new-tab chooser, palette, ticket Start and the project's "… with an agent" menus offer only agents whose host
check found them (web/src/common/useAgentAvailability.ts, the `['host-checks']` query Host settings' "Run checks
again" refreshes); an installed agent that is not logged in stays with a "not logged in" hint; everything is offered
until the checks load or when they fail.

## Host defaults (PDT-69)

Host settings `defaultAgent` (claude), `defaultSessionMode` (conversation) and `updateCheckMinutes` (60; 5 min to a
week), in Settings → This host (Agents, Updates). The web reads them from the `['host-settings']` query
(web/src/common/useAgentDefaults.ts): ticket Start preselects them, the new-tab chooser, palette and "… with an agent"
menus list the default agent's choices first, default view first (`byDefault` in web/src/sessions/newSession.ts), and
"… with an agent" starts in the default view. pdtd's update loop re-reads the interval every minute, so a change
applies without waiting out the old one.

## Devin (PDT-54, PDT-55)

Facts about the CLI: docs/superpowers/notes/devin-cli.md. `AgentKind::Devin`, program `devin` (`PDT_DEVIN_PROGRAM`);
a conversation by default like the others (`mode: terminal` for its TUI).

- Terminal: bypass is `--permission-mode dangerous`; the first prompt goes after `--` (a bare argument is a folder to
  Devin); activity comes from its OSC 7501 status (`app=devin`, the same parser as Codex).
- Config: Devin has no instructions flag and `--config` replaces the user's config, so each session (terminal or
  conversation) gets `--config <data dir>/sessions/<id>/devin-config.json`: the user's config
  (`$XDG_CONFIG_HOME/devin/config.json`, else `~/.config/devin/config.json`; `%APPDATA%\devin\config.json` on
  Windows; JSONC) plus a `SessionStart` hook `pdtd hook devin --instructions <file>`, which prints the instructions as
  `additionalContext` and, for a terminal, posts Devin's `session_id` (stored as `external_session_id`, latest wins);
  restart/auto-resume run `--resume <id>`. An unreadable user config means no `--config` (and so no hook).
- Conversations (PDT-55) run `devin [--config <file>] acp`, one Agent Client Protocol process (JSON-RPC on stdio) per
  live conversation (pdt-agents `adapter/devin.rs`, mapping in pdt-core `conversation/devin.rs`):
  - `session/new` (its id is `external_session_id`) or `session/load` to resume (its history replay is dropped; a
    session Devin never saved, i.e. never prompted, gives -32016 and a new one starts with a notice).
  - A turn is one `session/prompt`; a message sent while it runs joins it (steer); Send now cancels first
    (`session/cancel`).
  - Approvals are `session/request_permission` ("Always allow" = `allow_session` only, never Devin's saved rules; a
    deny note goes as the next message), questions `elicitation/create` (PDT offers the `elicitation.form`
    capability), plans the exit-plan permission (`plan_accept_edits`/`plan_bypass`); edits carry whole-file old/new
    text, diffed with `similar`.
  - Devin has no ask-before-edits mode: Ask maps to its Code mode (`accept-edits`); capabilities offer Accept edits /
    Plan / Bypass and new Devin conversations start in Accept edits.
  - No Make ticket (409 `draft_unsupported`; ACP has no fork).
- Conversation | Terminal switches like Claude's, one at a time (`POST …/mode`; Devin locks a session to one
  process): to terminal parks ACP, stores the switch time (ms) in `history_from` and restarts as a terminal
  (`--resume <id>`); back types `/exit`, then opens ACP at once, whose `session/load` replay imports the turns whose
  prompt is newer than that time (`source: terminal`; every turn for a session that was always a terminal) and then
  clears `history_from` (a failed open keeps it and the next message retries).
- Tests use `fake-agent acp`. The host check runs `devin auth status` (exit 0 either way; reads the text) and treats a
  missing Devin as fine.
- Web: new-tab "Devin" (d) and "Devin (terminal)" (e), palette "New Devin here" / "New Devin (terminal) here", ticket
  Start's agent choice, Set up / Write instructions with an agent.
