# Personal Dev Team — Design Spec (Milestone 1)

- **Date:** 2026-10-14
- **Status:** Draft, awaiting review
- **Codename:** Personal Dev Team (PDT). This is a working name and will be renamed before any public release (see §12).

---

## 1. Purpose

Personal Dev Team is a personal command center for building large applications with AI coding agents. It brings together project management (tickets and a pipeline board), agent sessions (Claude Code and Codex, running on the user's subscriptions), code review (diffs plus feedback) and terminals in one app. The app runs on Windows and macOS, and the same interface can be reached from a browser over Tailscale.

The long-term goal is to cover the whole development lifecycle, with a team of agent personas (refiner, coder, reviewer, QA and others) that hand work to each other, the user approving at key points, and eventually an assistant-style interface on top.

**Guiding principles**
- **Self-hosting early.** The build order gets development of PDT running inside PDT as soon as possible.
- **Personal now, product-ready later.** Single user for now. Nothing in the design should prevent multiple users, hosting it as a service, or open-sourcing it later.
- **One interface, one design language.** The same web interface is used in the desktop app and in remote browsers.
- **Performance and scale.** A Rust core service and a modular structure, so the app can grow to a large feature set.

## 2. Scope

The full product is split into sub-projects, and each one gets its own spec, plan and build cycle:

1. Core + shell
2. Agent sessions
3. Work tracking
4. Review
5. Code navigation
6. Automation and full development lifecycle
7. Personas and assistant

**Milestone 1 (this spec)** covers sub-projects 1 and 2 completely, plus the parts of 3, 4 and 7 that the agent pipeline needs. In short, it's enough to create a ticket and have agents refine it, code it, review it and QA it, with the user giving the final accept, from desktop or web.

**In Milestone 1**
- The `pdtd` background service, SQLite storage, and an HTTP and WebSocket API
- Tauri desktop window, the web interface, and remote access over Tailscale
- The design system (Graphite theme)
- Session types: structured agent, interactive agent terminal, shell, scratch agent chat
- Projects, tickets, attempts and worktrees, and the pipeline board
- Diff review with line comments, sending feedback, approve and merge
- Versioned personas, the workflow engine, the MCP outcome tools, and the "Waiting on you" inbox

**Out of Milestone 1 (later sub-projects)**
- Process/dev-server sessions
- A full file explorer and code search (only a read-only file view from diffs is included)
- Scheduled jobs and automation
- Pushing branches and opening pull requests
- Epics and sub-tasks
- Editing workflows in the interface (the default workflow is set in a config file)
- Multiple repos per project
- The assistant interface
- Multiple users

## 3. Architecture

### 3.1 Processes

- **`pdtd`:** the background service, a single Rust program built on tokio and axum. It owns the database, git worktrees, every child process (agents and terminals) and the event stream.
  - It listens only on `127.0.0.1` by default.
  - It runs whether or not any window is open.
- **Desktop app:** a Tauri v2 tray app (no window since PR #2). It opens the web interface in the default browser. From Phase 2, `pdt-host` starts and supervises `pdtd` (see `2026-10-15-remote-host-design.md` §3).
- **Remote access:** per host, `pdtd` exposes itself with `tailscale serve`, and remote requests must carry the owner's Tailscale identity (see `2026-10-15-remote-host-design.md` §2).

### 3.2 Repository layout

```
/crates
  pdt-core      domain types and the workflow state machine (pure, no I/O)
  pdt-store     SQLite storage (sqlx) and migrations
  pdt-git       worktrees, branches, diffs and merges (calls the git CLI)
  pdt-agents    the AgentAdapter trait plus the claude, codex and pty adapters
  pdt-workflow  the workflow engine: stage transitions and session orchestration
  pdt-mcp       the per-session MCP server that exposes the outcome tools
  pdt-api       axum REST routes and the WebSocket event stream
  pdtd          the service program that wires everything together
  fake-agent    a test program that plays back recorded agent output
/web            React interface (Vite)
  /ui           design system: tokens and components
/desktop        Tauri v2 shell
/docs           specs, plans and ADRs
```

- Each crate has a single purpose, which keeps compile times and agent context small.
- `ts-rs` generates the TypeScript types for API payloads from the Rust types, so the server and interface can't drift apart.

### 3.3 Technology versions

These were checked against live registries on 2026-10-14. Pin these or newer at scaffold time.

| Area | Choice | Version |
|---|---|---|
| Rust | stable toolchain, edition 2024 | 1.99.0 |
| Async / HTTP | tokio, axum, tower, tower-http | 1.53.3 · 0.8.9 · 0.5.3 · 0.7.1 |
| Database | sqlx (SQLite) | 0.9.0 |
| Serialization | serde / serde_json | 1.0.229 / 1.0.154 |
| TS types | ts-rs | 12.0.1 |
| Terminals | portable-pty | 0.9.0 |
| Errors / logs | thiserror, anyhow, tracing, tracing-subscriber | 2.0.21 · 1.0.104 · 0.1.44 · 0.3.23 |
| IDs / time | uuid, jiff | 1.28.0 · 0.2.38 |
| File watching | notify | 8.2.0 |
| Desktop | tauri / tauri-build / @tauri-apps/cli / @tauri-apps/api | 2.12.4 / 2.7.3 / 2.12.3 / 2.12.4 (v2; v3 is alpha only) |
| Runtime | Node.js LTS, pnpm | 24.22.0 · 12.11.2 |
| Interface | react / react-dom, typescript | 19.3.1 · 7.0.2 |
| Build | vite, @vitejs/plugin-react | 8.3.4 · 6.1.2 |
| Data / routing | @tanstack/react-query, @tanstack/react-router | 5.104.2 · 1.170.44 |
| Styling | tailwindcss | 4.3.3 |
| Editor / diff | codemirror, @codemirror/merge, view, state | 6.0.2 · 6.12.2 · 6.43.14 · 6.7.6 |
| Terminal | @xterm/xterm | 6.0.0 |
| Tests | vitest, @playwright/test | 5.0.4 · 1.64.1 |
| Agent CLIs | Claude Code, Codex CLI | ≥ 2.1.300 · ≥ 0.162.1 |

Notes:
- TypeScript 7 (the native compiler), Vite 8 (built on Rolldown), Vitest 5, xterm 6 and pnpm 12 are all new major versions. Confirm the tooling works together when scaffolding.
- **Rust and pnpm are not installed** on the development machine yet, and the local Codex CLI (0.134) is out of date. Setting these up is the first task of Phase 0.
- `git` is called through its command line, not `gix`. That's more reliable for worktrees and merges, and avoids `gix`'s frequent breaking 0.x releases.

## 4. Domain model

All records are stored in SQLite (WAL mode) in a single database file in the data folder.

| Entity | Key fields |
|---|---|
| **Project** | id, name, repo_path, base_branch, workflow (default from config), default agent settings |
| **Ticket** | id, project_id, key (`PDT-42`), title, description (markdown), stage, priority, labels, loop_count, timestamps |
| **Attempt** | id, ticket_id, branch (`pdt/PDT-42-<slug>`), worktree_path, base_commit, state (active / merged / abandoned) |
| **Session** | id, kind (`agent` / `agent_tui` / `shell` / `scratch`), owner (attempt_id, or project_id for sessions not tied to a ticket), agent (`claude` / `codex` / none), persona_version_id, model, external_session_id, status, timestamps |
| **Event** | session_id, seq (increases within the session), ts, type, payload (JSON). Append-only. |
| **Persona** / **PersonaVersion** | name, role; each version has agent, model, instructions, permission profile, created_at. Versions never change once created. |
| **ReviewComment** | attempt_id, author (user or session_id), file, line range, body, state (draft / sent / resolved) |
| **InboxItem** | ticket_id or session_id, kind (clarification / manual_stage / approval / stalled / loop_limit / accept), payload, resolved_at |
| **Outcome** | ticket_id, stage, session_id, outcome, summary, ts. This is the audit trail of stage transitions. |

Session status is one of: `starting`, `running`, `waiting_approval`, `waiting_input`, `idle`, `finished`, `failed`, `interrupted`.

The event log is the single source of truth for what happened in a session. The interface's conversation view is a replay of it, and the WebSocket streams new events as they're appended.

## 5. Sessions and agent adapters

### 5.1 Session types (Milestone 1)

| Type | Description | Owner |
|---|---|---|
| **Agent (structured)** | Claude or Codex in machine-readable mode, running with a persona. Used for every pipeline stage. | An attempt |
| **Interactive agent terminal** | The real `claude` or `codex` interactive program running in a terminal. The escape hatch. | An attempt or project |
| **Shell** | PowerShell, bash or zsh in the project folder or an attempt's worktree | An attempt or project |
| **Scratch chat** | A structured agent conversation not tied to a ticket. Can be turned into a ticket later. | A project |

- Terminal sessions (interactive agent terminal and shell) run inside `pdtd` through `portable-pty`.
- The interface connects to them through a WebSocket byte stream shown in xterm.js.
- Any number of clients can attach to a terminal session at once, including desktop and web together.
- `pdtd` keeps a recent output buffer for each terminal, so a client that attaches late can redraw the screen.

### 5.2 AgentAdapter trait

```rust
trait AgentAdapter {
    async fn start(&self, spec: SessionSpec) -> Result<RunningSession>;   // new conversation
    async fn resume(&self, external_id: &str, spec: SessionSpec) -> Result<RunningSession>;
    async fn send(&self, s: &RunningSession, msg: UserMessage) -> Result<()>;
    async fn respond_approval(&self, s: &RunningSession, id: ApprovalId, decision: Decision) -> Result<()>;
    async fn interrupt(&self, s: &RunningSession) -> Result<()>;
    fn events(&self, s: &RunningSession) -> impl Stream<Item = AgentEvent>;  // normalized
}
```

`SessionSpec` carries the working folder, the persona instructions, the model, the per-session MCP server configuration, and the permission profile.

### 5.3 Claude adapter

- Runs `claude -p --output-format stream-json --input-format stream-json --verbose`, with persona instructions added to the system prompt and the per-session MCP config passed in.
- Resumes with `--resume <session_id>`.
- Parses the stream-json lines into `AgentEvent`s (message, tool_use, tool_result, usage, result, error).

### 5.4 Codex adapter

- Runs `codex app-server` and talks to it over stdio using JSON-RPC 2.0. This is OpenAI's documented way to embed Codex in another app, and it covers threads, streamed events and approvals.
- The WebSocket transport is experimental and is **not** used.
- Approval requests from Codex become `waiting_approval` events.

### 5.5 Billing and credentials

- Both adapters use the tools' existing local logins, so usage goes against the user's subscription.
- `pdtd` **removes `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`** from the agent environment unless the host or project is set to pass credentials through (`2026-10-15-remote-host-design.md` §6).
- `pdtd` never stores or handles agent credentials. This keeps a future product version within Anthropic's and OpenAI's terms, because each user's own installed tools and logins are used.

### 5.6 Normalization and drift

- Each adapter converts its tool's events into a shared `AgentEvent` type.
- Events the adapter doesn't recognize are stored as `unknown`, with the raw payload, and are never dropped or treated as a crash.
- Each adapter declares a minimum tool version. The Doctor (§8) warns when the installed version is older.

## 6. Workflow engine and personas

### 6.1 Default workflow

```
Draft → Refine → Ready → Code → Review → QA → Accept → Done
          │                 ▲       │       │      │
          └─ ask user       └───────┴───────┘    user
                          changes / fail
```

| Stage | Owner (default) | Allowed outcomes → next stage |
|---|---|---|
| Draft | user | `submit` → Refine |
| Refine | Refiner persona | `ready` → Ready · `needs_clarification` → inbox (back to Refine once answered) |
| Ready | — (queue) | the user presses Start, or auto-start if the project is set to do so → Code |
| Code | Coder persona | `submitted` → Review |
| Review | Reviewer persona | `approved` → QA · `changes_requested` → Code |
| QA | QA persona | `pass` → Accept · `fail` → Code |
| Accept | user | `merge` → Done · `reject` → Code |

- Any stage can be set to **manual**, meaning the user is the owner and the stage's work appears in the inbox.
- The workflow is defined per project in a config file (`.pdt/workflow.toml` in the repo, falling back to the built-in default). There's no workflow editor in the interface in Milestone 1.

### 6.2 Session rules

- **Coder:** one session lasts for the whole attempt. Review and QA feedback is sent into that same session as a follow-up message, so the coder keeps its context.
- **Reviewer and QA:** start a **fresh session on every pass**, for an unbiased look. Each new session is given the ticket, the diff, and the outcome summaries and comments from earlier passes.
- **Refiner:** a fresh session on each pass, given the ticket and the conversation history of any clarifications.
- **Loop limit:** each time Review or QA sends a ticket back to Code, `loop_count` goes up by one. At the limit (default 3), the ticket goes to the inbox as `loop_limit` instead of looping again.

### 6.3 MCP outcome tools

For every structured agent session, `pdtd` starts a session-scoped MCP server (`pdt-mcp`) and passes it to the agent through the agent's config. Tools:

- `get_ticket()`: ticket, stage, instructions, prior outcomes and comments
- `submit_outcome(outcome, summary, comments[])`: only outcomes allowed for the current stage are accepted; anything else returns an error the agent can see and fix
- `ask_user(questions[])`: creates an inbox item and moves the session to `waiting_input`
- `add_ticket_comment(body)` and `add_review_comment(file, line_range, body)`

Transitions are therefore **explicit, typed tool calls**, never guessed from the agent's text. The engine records an `Outcome`, moves the ticket, and starts or resumes the next owner's session.

**Stalled tickets:** if a structured session ends its turn without calling `submit_outcome` (and isn't waiting on the user), the ticket becomes `stalled`. An inbox item appears with **Nudge** (sends a reminder message) and **Take over** (makes the stage manual).

### 6.4 Personas

- A persona has a name, a role, and versions. Each version records the agent (claude/codex), model, instructions and permission profile.
- Editing a persona creates a new version. Every session records which persona version it ran with, so changes over time can be traced.
- Milestone 1 ships default Refiner, Coder, Reviewer and QA personas, with editing through a simple form in the interface.

## 7. Interface

### 7.1 Layout (Hybrid, "C v2")

- **Icon bar (left):** Inbox · Board · Sessions · Personas · Files
- **List column:** changes with the selected view. Examples: inbox items, or live sessions grouped by ticket and by project, including terminals and scratch chats.
- **Work area:**
  - A header showing the ticket, stage, loop count and persona.
  - Tabs: Conversation · Diff · Files (read-only) · Ticket.
- **Bottom panel:** can be shown, hidden or maximized. Holds tabs for terminal sessions (shells and interactive agent terminals), attached to whatever is in focus.
- **Board view:** columns are workflow stages. Cards show the owner persona and live session status.
- **On narrow (phone) screens:** collapses to a single column (icon bar → list → work area) for use over Tailscale.
- A command palette (Ctrl/⌘-K) for navigation.

### 7.2 Design language: Graphite

- **Base:** cool neutral grays. Dark-mode background `#0e0f12`, surface `#17181d`, border `#26282f`, text `#e7e8ec`, muted text `#8a8f9c`. Light mode is equivalent, with a `#f7f7f9` background.
- **Accent:** muted blue, `#3561b8` in dark mode (text on accent `#eef2fb`), `#2563eb` in light mode. Used for the primary button, selected item, links and focus ring.
- **Status colors** (kept separate from the accent): running green, waiting amber, failed red, idle gray.
- **Type:** Inter for the interface, JetBrains Mono for code and terminals.
- **Corners:** 6px radius. Dense but with room to breathe.
- **Implementation:** all values are tokens in `web/ui` (CSS custom properties exposed through Tailwind 4 theme variables). Components only reference tokens, never raw colors. Light and dark mode both follow the system setting and can be overridden manually.

### 7.3 Review

- **Diff:** the attempt branch compared with its merge-base on the base branch, shown with `@codemirror/merge`, in split or unified view.
- **Comments:** line comments start as drafts. **Send feedback** combines all drafts into one structured message, sends it into the Coder session, and marks the comments as sent.
- **Agent comments:** comments from agent reviewers (made with `add_review_comment`) appear in the same place, marked with their persona.
- **Accept stage:** **Merge** does a local merge into the base branch, then removes the worktree.

## 8. Error handling

- **Errors:** library crates use `thiserror` with typed errors, and only the `pdtd` program itself uses `anyhow`. API errors are returned as `{code, message}`.
- **Doctor:** runs at startup and from Settings. It checks:
  - `claude`, `codex` and `git` are present, along with their versions and login status
  - Whether API-key environment variables are set (warns about API billing)
  - Whether the data folder is writable
- **Agent process failures:**
  - A crash marks the session `failed`, with an event recording the exit code and the end of stderr.
  - When `pdtd` restarts, sessions that were running are marked `interrupted` and can be resumed with one click using `external_session_id`.
- **Git:**
  - A merge conflict on Accept sends the ticket back to Code with the conflict details.
  - A worktree with uncommitted changes is never deleted.
- **Database:**
  - Migrations only move forward.
  - The database file is copied automatically before each migration.
- **WebSocket:** clients reconnect and resume from the last `seq` they received for each session, so no events are missed.

## 9. Security

- `pdtd` listens only on loopback. Remote access goes through `tailscale serve` (which provides HTTPS and identity on the tailnet).
- Remote requests must carry the host owner's Tailscale identity; local requests are trusted. This replaces device tokens (`2026-10-15-remote-host-design.md` §2).
- No agent credentials are stored (see §5.5).
- Terminal and agent sessions have the same access to the machine as the user. That's acceptable for a single-user tool. Isolating sessions further is deferred to the product version.

## 10. Testing

- **Rust unit tests in each crate:**
  - `pdt-core` is pure logic, including state-machine tests of every workflow transition.
  - `pdt-store` tests use temporary SQLite files.
  - `pdt-git` tests use temporary repos.
- **`fake-agent`:** a program that plays back recorded Claude stream-json and Codex app-server conversations, including MCP outcome calls. It makes end-to-end pipeline tests (coder → reviewer → changes → QA → accept) repeatable without using the subscription. Recordings are captured from real tool runs and refreshed when tool versions change.
- **Interface:** Vitest 5 and Testing Library for components. Playwright end-to-end tests run against a `pdtd` that uses `fake-agent`.
- **Continuous integration:** GitHub Actions on Windows and macOS for every push. Both must pass, because terminal and file-path behavior differ between them.
- **Process:** test-driven development for `pdt-core`, `pdt-workflow`, `pdt-agents` and `pdt-git`.

## 11. Build order and self-hosting

| Phase | Delivers | Unlocks |
|---|---|---|
| 0. Foundation | Install the toolchain (Rust, pnpm, updated Codex); Cargo workspace; `pdtd` with SQLite and migrations; ts-rs; `web/ui` tokens and core components; Tauri window; the app frame; CI | — |
| 1. Terminals | PTY sessions (shell, interactive agent terminal), xterm view, projects, worktrees, sessions surviving closed windows | **PDT is developed inside PDT,** Herd-style |
| 2. Remote host | Remote access over Tailscale identity, `pdt-host` supervisor, token-based updates with rollback, run configurations with HTTPS previews, agent activity state, per-host settings (`2026-10-15-remote-host-design.md`) | **PDT moves to the Mac mini** and is developed there from any device |
| 3. Workspaces | herdr-style workspaces and tabs; project keys; git integration (`2026-10-17-workspaces-design.md`) | Day-to-day agent work without herdr |
| 4. Tickets and board | Tickets, the board, Start (attempt, worktree, session) | PDT's own backlog lives in PDT |
| 5. Structured agents | Claude and Codex adapters, event log, conversation view, approvals, scratch chats | Rich, machine-readable agent sessions |
| 6. Review | Diff view, line comments, sending feedback, merge | A review loop driven by the user |
| 7. Pipeline | Personas, workflow engine, `pdt-mcp`, Refine/Review/QA stages, inbox, loop limit, stalled handling | The full agent team |

**Stable versus dev installs:**
- The stable `pdtd` (a released build, default port, data folder `…/pdt/`) runs the real sessions that work on PDT.
- Dev builds run on a different port with a separate data folder (`…/pdt-dev/`). This way, rebuilding PDT never kills the agents doing the rebuilding.
- Updating the stable install is a deliberate step (build, stop, swap, start). Interrupted sessions resume afterwards.
- From Phase 2, dev instances are run configurations with their own port and data, and stable updates from GitHub releases through `pdt-host` (`2026-10-15-remote-host-design.md` §3, §4, §7).

## 12. Naming

"Personal Dev Team" is a codename. To keep a later rename cheap, the product name, the binary names (`pdtd`) and the crate prefix (`pdt-`) are each defined in a single place, and nothing else depends on the literal name. Names already rejected or found to clash: Atelier, Covey, Roster, Squad, Cadre, Brigade, Guild, Drydock, Gantry, Kiln, Foundry, Pegboard, and anything containing "virtual" or the letters "ads".

## 13. Open items (deferred, not blocking Milestone 1)

- The final product name, and the trademark, domain and crate-name checks that go with it.
- Exactly what Codex's `app-server` approval events and MCP configuration look like. This must be confirmed against the 0.162 documentation during Phase 3; the adapter design does not depend on the details.
- Device tokens, for remote access without Tailscale (deferred; Phase 2 uses Tailscale identity).
- The deferred items in `2026-10-15-remote-host-design.md` §9: phone UI, notifications, a multi-host UI, replacing hermes-agent, Devin support.
