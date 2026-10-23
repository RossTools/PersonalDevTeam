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
  block (API address, session, project, checkout, guides, worktrees and runs through the API; pdt-core
  `agent_context`) plus the project's **agent instructions** (`project_settings.agent_instructions`,
  `agentInstructions` in the settings API; kept when a PUT leaves it out, max 16 KB), written to
  `<data dir>/sessions/<id>/instructions.md`.
- A session on an active attempt (Start's session) also gets "This session's ticket" (key, API path, branch;
  follow-up work in its own ticket; Done, the worktree and the branch are PDT's or the user's) and, when enabled
  workflows message the attempt's agent, "Messages from PDT" (PDT-83): which events reach it as `[PDT]` messages, how
  to act on each kind, and, for any `ci.*` / `pr.*` / `attempt.merged` one, that PDT watches the pull request so CI
  isn't watched in a shell. The events come from pdt-core `workflow::news::agent_news`: enabled project and
  host-wide workflows whose trigger (and its `projects`) leads by edges, or a `workflow.call`, to a `session.send`
  with `attempt` (trigger conditions aren't read). The text is fixed when the session starts or restarts; a chat that
  Start takes over keeps what it started with.
  - Claude gets `--append-system-prompt-file <that file>`.
  - Codex terminals get `-c developer_instructions=<TOML string>` (through a Windows `.cmd` shim, a pointer to the
    file), Codex conversations `developerInstructions` on `thread/start`/`thread/resume`; the user's own
    `developer_instructions` from config.toml come first, since PDT's value replaces them (pdt-agents
    `instructions.rs`).
  - Devin gets them from a SessionStart hook (below).
- The project's Settings page has the instructions box and "Set up with an agent" / "Write instructions with an
  agent", which start a conversation in the main checkout with the matching prompt.

## pdt-mcp (Phase 7b)

PDT's MCP server (pdt-api `mcp/`; spec docs/superpowers/specs/2026-10-20-phase-7b-agent-steps-design.md §2):
`POST /api/mcp`, Streamable HTTP answered with JSON (`initialize`, `ping`, `tools/list`, `tools/call`; batches;
notifications get 202; `GET`/`DELETE` 405).

- **Identity.** `Authorization: Bearer <session id>.<HMAC-SHA256 of it>` under the host key `<data dir>/mcp.key` (32
  random bytes, owner-only, made on first use). Nothing is stored per session; a token works while its session exists,
  is an agent session and is not `finished` (401 otherwise). A tool call knows its session, and through it the
  project, ticket (the session's attempt's, else its agent step's) and agent step.
- **Tools** (`mcp/tools.rs`): `get_ticket`, `submit_outcome`, `ask_user`, `add_ticket_comment`, `add_review_comment`
  (the default set), and `update_ticket` (Phase 7c: title and description of the session's own ticket, the store's
  `patch_ticket_by` as the agent; the built-in Refiner lists it), `catalog_search`, `catalog_get`, `workflow_get`,
  `workflow_validate`, `workflow_dry_run`, `workflow_save` for personas that list them, and the PM's tools ([The
  PM](#the-pm-pdt-81)). Inputs are the API's request types (schemars schemas) and each tool
  runs the route's own function (ticket `get_one`, the store's comment, review `add_comment`, workflow
  `create_workflow` with author `agent`, so a new workflow starts disabled, and `save` with `propose`, so a change is a
  proposal the user accepts or discards). `tools/list` gives only the session's set (its
  persona version's, else the default); read-only tools carry `readOnlyHint`. Errors the agent can act on are tool
  results with `isError`.
- **Per launch, never the user's config** (pdt-agents `mcp.rs`): Claude (conversation and terminal) gets
  `--mcp-config <session dir>/mcp.json` (owner-only; it holds the token) and `--allowedTools mcp__pdt__…` for its set
  less the asking tools (`create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `send_to_session`,
  `discord_change`, `discord_webhook`:
  pdt-core `ASKING_TOOLS`), which Claude
  asks about per call;
  Codex conversations a thread `config` `mcp_servers.pdt` with `http_headers` and `default_tools_approval_mode =
  "approve"`, Codex terminals the same through `-c` with the token in `PDT_MCP_TOKEN` (`bearer_token_env_var`); Devin
  conversations an ACP `mcpServers` entry, and the mapping allows `mcp__pdt__*` permission requests at once (pdt-core
  `conversation/devin.rs`). Devin terminals get no pdt-mcp (their MCP servers live in a file of Devin's own).
- **Instructions.** `agent_context` names the server and the session's tools; a persona's step session also gets
  "Your role: <persona>" ([workflows.md](workflows.md#agent-steps-phase-7b)); a Personal project's session is told it
  works in a folder with no branches.
- Tests: `fake-agent` calls a tool when a message's first line is `mcp: <tool> <json>` (`mcp-list` lists them),
  asking first, as Claude does, for a listed tool `--allowedTools` leaves out; crates/pdt-api/tests/agent_steps.rs and
  pm.rs serve the API on a local port for it.

## The PM (PDT-81)

A host-level project manager to talk to (spec docs/superpowers/specs/2026-10-21-pm-persona-design.md; pdt-api `pm.rs`,
pdt-core `pm.rs`, pdt-store `pm.rs`, web `pm/`).

- **Where it lives.** PM sessions are conversations in the **host project** (`kind: host`, key `_HOST`, folder
  `<data dir>/pm`, no git, no workspace), made on first use and never listed (`list_projects` leaves it out). Each
  runs in the **work folder** (PDT-100, pdt-core `pm::work_folder`): the deepest folder holding every project's
  folder (each checkout's parent) and Host settings' Projects folder (its default `~/work` only while there are no
  projects); the home folder when they share nothing below the root. They have no tab: the PM rail item (`Ctrl+] m`,
  the palette's Go to) shows them, and opening one from the Inbox goes there.
- **Sessions.** `GET /api/pm` → `{current, sessions}` (`pm_sessions`: rotation `none` | `journal` | `done`, reason,
  journal); `POST /api/pm/sessions {text?}` ends the current one (reason `user`, no journal) and starts one of the
  built-in **PM** persona's current version (its agent and model; permission mode `ask` unless the persona sets one).
  The web starts one with the first message when there is none; an ended session is read-only.
- **Its agent's own tools** (PDT-100), with the agent's usual permission prompts: read and search checkouts and
  worktrees, git, web search and fetch, commands and installs, file edits, subagents. Claude and Devin keep theirs;
  Codex also gets `web_search = "live"` and network access in its workspace-write sandbox (thread config
  `sandbox_workspace_write.network_access`, `networkAccess` on each turn's `sandboxPolicy`; pdt-agents
  `mcp::CODEX_PM_CONFIG`).
- **PDT's tools.** pdt-mcp with the PM's set (pdt-core `PM_TOOLS`): `get_overview` (projects with open tickets per
  stage, tickets in progress with branch/PR/CI/agent in words, the Inbox, running agent sessions), `search_tickets`,
  `get_ticket`, `get_session` (state, open requests, last messages),
  `list_workflow_runs`, `search_notes`, `save_note`, `delete_note`, `write_journal`, the catalog and workflow tools
  (`workflow_get` by name searches every project; `workflow_save` needs a project), and the asking tools
  `create_ticket` (a draft by default; the timeline gets "Created by the PM."), `edit_ticket` (closing ends the
  attempt, as the API; `parent` takes a key or id, `""` detaches), `link_tickets` / `unlink_tickets` (`ticket`, `kind`:
  `blocks`, `blockedBy` or `relates`, and `other` in the same project; the store's link rules, so self-links, blocking
  cycles and unknown tickets are errors; the `linked`/`unlinked` and `parted` events are the agent's) and
  `send_to_session` (through the outbox, "[PDT] From the project manager: …").
- **Discord** (PDT-98; pdt-api `discord.rs`, pdt-core `discord.rs`). Through a `discord_bot` connection (`connection`:
  its name, or the only one), with Discord's REST API (no gateway): `discord_get` (the server, its categories,
  channels and webhooks, each webhook with the connection it is saved as), and the asking tools `discord_change`
  (`changes`, in order: `create` a channel or category, `rename`, `move` into a category or out, `archive`: move into
  an Archive category; nothing is deleted; the first failure stops the rest and the result says what was done) and
  `discord_webhook` (a webhook on `channel`, made first with `createChannel`, saved as a `discord_webhook` connection
  called `name` that `discord.post` uses; the URL goes to the keychain and never into the result; a taken name is
  refused before anything is made, and a webhook that can't be saved is removed). The approval card's title is one
  line per change (pdt-core `discord::ask_title`).
- **Asking.** Claude asks before each asking tool (the approval card shows the arguments, also in the Inbox's
  Approvals). When the request's target project (`create_ticket`'s `project`, `edit_ticket`'s, `link_tickets`' and `unlink_tickets`' ticket,
  `send_to_session`'s session) has the policy **The PM acts without asking** (`project_settings.pm_acts`, `pmActs` in
  project settings), PDT answers yes itself (conversations.rs → `pm::request_opened`); for the Discord tools the
  setting is the bot connection's `config.pmActs` (a Discord server is in no project). Codex and Devin can't ask per
  tool, so there the asking tools are refused unless the project (or the bot connection) lets the PM act.
- **Briefing.** Built by pdtd when the session's instructions are first written and kept in
  `<session dir>/briefing.md` (a restart sends the same text): projects with open tickets per stage and whether the PM
  may act, tickets in progress, the Inbox, events since the previous PM session started (counts and the latest
  lines), the last three journal entries, and pinned notes (id, project, title). Sections are capped at 30 lines. The
  instructions are pdt-core `pm::instructions`: who the PM is, its work folder, its tools (the asking ones named; its
  agent's own), PDT's API and guides, the persona's instructions, the briefing (each project with its folder).
- **Notes** (`pm_notes`, FTS5 table `pm_notes_fts` kept by triggers): `kind` note | journal, host-wide or per project,
  pinned or not. `GET /api/pm/notes?q=&project=&kind=` (words are quoted prefixes, all required; a project gets its
  own and host-wide notes), `POST`, `PATCH /api/pm/notes/{id}` (`project: ""` makes it host-wide), `DELETE`. The PM
  view's Notes page lists, searches, adds, edits, pins and deletes them.
- **Paths and the phone** (PDT-113). `/pm` is the current session (or the start box), `/pm/notes` the notes,
  `/pm/notes/<id>` one note, `/pm/earlier` the sessions that ended (when, how it ended, the journal's first line) and
  `/pm/<session id>` one of them, read-only with "Go to the current session". On a phone the conversation is the
  whole screen (no header: the top bar has the title), ⋯ has New session… (asks first), Notes and Earlier sessions,
  a tap on a note opens it as its own screen, and the start box shows the usual text but isn't focused (the keyboard
  would cover it).
- **Rotation.** When a PM session ends a turn with its context at least 60 % full (the last `usage` event) or 24 h
  old (`AppState.pm_rotate_age_ms`), PDT queues "[PDT] This PM session is ending…" through the outbox (rotation
  `journal`); when that turn ends the session is stopped (`done`; "(no journal)" if it wrote none). The user's next
  message starts a new session.
- **As a workflow step.** The PM persona can be an `agent` node's persona; its asking tools still ask (the approval
  waits in the Inbox's Approvals). The built-in template **Triage new GitHub issues** (`builtin:issue-triage`) runs it
  on each new issue of a repository: it drafts a ticket in the chosen project, marks a duplicate, or leaves it.

## Activity

- Claude sessions get `--settings <data dir>/sessions/<id>/claude-settings.json` (hooks run
  `pdtd hook claude <Event>`).
- Codex gets `-c notify=[pdtd, hook, codex-notify]`, chaining the user's own `notify` from config.toml after
  `--chain` (pdt-agents `hooks.rs`). Terminal Codex also reads its OSC 7501 status, with screen heuristics for older
  CLIs (docs/superpowers/notes/phase-2-agent-signals.md).
- The user's own settings are never edited.
- `attention_at` is set when a session becomes idle or needs the user; it is **unseen** until `last_viewed_at`
  (`POST /api/sessions/{id}/view`, any device) passes it. Unseen and needs-you sessions make notifications
  (`session.needs_you`, `session.turn_ended` after 30 s; [workflows.md](workflows.md#notifications)); viewing one
  clears and reads them. An open session's pane posts the view when it opens, when the tab becomes visible and when
  a poll finds attention it hasn't cleared yet (a newer `attentionAt` while unseen; not on every poll).

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
- On touch devices (PDT-115, web/src/terminal/keyBar.ts) a key bar sits under the terminal, above the on-screen
  keyboard: `Esc Tab Ctrl ^C ↑ ↓ ← → ⌃] ⌨`, scrolling sideways when it doesn't fit and never taking focus. Ctrl is
  sticky: tapped, it applies to the next key typed or tapped (Ctrl+R, Ctrl+arrow), then lets go; tapped again it
  lets go. ⌃] is PDT's prefix: the next typed key runs its shortcut (`runPrefixKey`), and ⌃] twice sends Ctrl+] to
  the program. ⌨ puts the keyboard away (keys then leave it down) and brings it back. A drag scrolls the scrollback; a
  long press selects the word under the finger, dragging extends it, and a Copy bar copies it. On a phone the
  session's ⋯ has Paste (the clipboard as a bracketed paste), Compose and Font size, which is kept in the browser
  (`localStorage` `pdt.terminal.fontSize`) and applies to every terminal.
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
until the checks load or when they fail. Only a program missing from the search path counts as not found: one whose
check times out (30 s) or cannot start is found but not ok, and pdtd's startup check runs again (up to 3 times, 30 s
apart) while an agent's check is unanswered, since a CLI's first launch after a restart can be slow (PDT-92).

## Host defaults (PDT-69)

Host settings `defaultAgent` (claude), `defaultSessionMode` (conversation), `ticketPermissionMode` (bypass: the
agent works on its own branch) and `sessionPermissionMode` (ask), PDT-75, and `updateCheckMinutes` (60; 5 min to a
week), in Settings → This host (Agents, Updates). The permission modes are only what the Start dialog and the new-tab
chooser pick first (in a terminal only bypass changes anything); requests without one keep the agent's default. The web reads them from the `['host-settings']` query
(web/src/common/useAgentDefaults.ts): ticket Start preselects them, the new-tab chooser, palette and "… with an agent"
menus list the default agent's choices first, default view first (`byDefault` in web/src/sessions/newSession.ts), and
"… with an agent" starts in the default view. pdtd's update loop re-reads the interval every minute, so a change
applies without waiting out the old one.

`maxAgentTurns` (4; 0: no cap; Phase 7c), Settings → This host → Agents "Agents running at once": the most agent
sessions running a turn before PDT's own work waits for a slot ([workflows.md](workflows.md#pipeline-policies-and-scheduling-phase-7c)).

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
