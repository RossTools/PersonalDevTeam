# Personal Dev Team — Structured Agents (Phase 5) Design

Status: accepted, 2026-10-18. Amends the main spec §5 (sessions and adapters) and the UX spec
§8.2 (conversation view). Builds on Phase 4 (tickets, attempts, Start; docs/superpowers/specs/2026-10-18-tickets-design.md),
which merges first. Protocol facts it relies on, with what was verified and what wasn't:
docs/superpowers/notes/phase-5-agent-protocols.md.

## 1. Purpose

Today an agent session is the agent's own TUI in a terminal: PDT can only type into it and guess its state from hooks
and screen text. Phase 5 makes **the conversation the normal way to work with Claude and Codex**: PDT drives each CLI
through its machine-readable protocol and draws the conversation itself — messages, tool calls, diffs, approvals,
questions, plans — with a real composer. The CLIs stay the backend: their logins, models, tools, MCP servers, skills,
hooks and transcripts are the user's own, and any conversation can be opened in the real TUI when PDT's view isn't
enough.

### Success criteria

- New Claude and Codex tabs open as conversations; the user can work a whole ticket without opening a terminal.
- An approval, a question or a plan is answered with one click (or `y`/`n`) on desktop and on a phone.
- PDT knows a session's state from the protocol (working, needs you, idle) with no screen reading.
- Any conversation opens in the real TUI and comes back without losing a turn; for Codex both stay live together.
- A conversation's history survives pdtd restarts and updates, and rebuilds from the agent's own files if PDT's log is
  missing.
- Unknown protocol events never crash a session; they show as a collapsed raw row.

## 2. Decisions (made with the user, 2026-10-18)

- **Headless protocol, PDT renders everything.** Claude: `claude -p` stream-json; Codex: `codex app-server`; Devin
  later: `devin acp` (§11). Not an overlay on a live TUI (screen-read approvals are fragile).
- **Conversation by default; terminal is the escape hatch.** New-tab Claude `c` / Codex `x` and Start open
  conversations; "Claude (terminal)" / "Codex (terminal)" stay in the new-tab picker; any agent session can switch
  between its conversation and its terminal (§8).
- **Builds on Phase 4:** Start gets a mode (conversation default), the ticket tab's Conversation tab comes alive.
- Devin: researched now (notes file), built later; the adapter layer stays agent-neutral so it slots in.

## 3. Shape

```
browser ── GET/POST /api/sessions/{id}/… ─┐        ┌─ ClaudeAdapter ── claude -p (one process per live session)
        ── WS /api/sessions/{id}/events ──┤ pdtd ──┼─ CodexAdapter ─── codex app-server (one per pdtd, unix socket)
                                          │        └─ (DevinAdapter ── devin acp, later)
                                    event log (SQLite)
```

- **pdt-agents** gains `adapter/` with the trait, the normalized event types and one module per agent. Adapters own
  processes and protocol framing; they know nothing about HTTP or the store.
- **pdt-api** gains a `ConversationManager` (beside `PtyManager`): one per pdtd, it maps session ids to live adapter
  sessions, appends events to the store, fans them out to WebSocket subscribers, and derives activity.
- **pdt-core** holds the pure parts: `AgentEvent` and friends, activity derivation from events, permission-mode
  mapping, slash-command routing (all TDD).

### 3.1 The adapter trait (replaces main spec §5.2)

```rust
#[async_trait]
trait AgentAdapter: Send + Sync {
    fn agent(&self) -> AgentKind;
    /// Starts or resumes the agent's conversation for a session; events arrive on `sink`.
    async fn open(&self, spec: OpenSpec, sink: EventSink) -> Result<Box<dyn LiveConversation>>;
    /// Reads the agent's own history (Claude jsonl, Codex thread/turns/list) as normalized events.
    async fn read_history(&self, spec: &HistorySpec) -> Result<Vec<AgentEvent>>;
    async fn capabilities(&self) -> AgentCapabilities;   // models, modes, slash commands, version
}

#[async_trait]
trait LiveConversation: Send + Sync {
    async fn send(&self, msg: UserInput, delivery: Delivery) -> Result<()>;  // Delivery: Next | Now
    async fn answer(&self, request_id: &str, answer: Answer) -> Result<()>;   // approval, question, plan
    async fn interrupt(&self) -> Result<()>;
    async fn set(&self, change: SessionChange) -> Result<()>;               // model, permission mode, effort
    async fn command(&self, cmd: AgentCommand) -> Result<()>;               // compact, review, …
    async fn close(&self) -> Result<()>;                                     // park: stop the process, keep the conversation
}
```

`OpenSpec`: session id, external id (Claude: the session id; Codex: the thread id once known), cwd, model, permission
mode, persona instructions (Phase 7; empty now), MCP config (Phase 7's pdt-mcp), PDT hook settings, env (with the
§5.5 key scrubbing of the main spec).

### 3.2 Processes

| | Claude | Codex |
|---|---|---|
| Process | one `claude -p` per **live** session (flags in the notes file, always with `--permission-prompt-tool stdio`) | one `codex app-server --listen unix://<data dir>/codex/app.sock` per pdtd, started on first use; pdtd connects as a WebSocket client |
| Session ↔ conversation | `--session-id <session id>` on first open, `--resume <session id>` after | `thread/start` → `external_session_id = thread.id`; `thread/resume` after |
| Parking | the process exits after **15 min idle** with no open request; the next send resumes it (`--resume`), invisible to the user | threads unload by themselves after 30 min idle; `thread/resume` on next send |
| Crash | the session shows "Agent stopped" with the stderr tail; next send resumes | pdtd restarts the app-server (backoff), re-resumes threads with subscribers |

- Codex gets its **own** app-server, never the user's shared daemon (`~/.codex/app-server-control`), so PDT's
  sessions don't mix with `codex agents` and PDT controls its lifetime and config overrides (`-c notify=…` stays for
  the terminal path only).
- The unix socket is what lets the Codex TUI attach live (§8). Windows: whether `--listen unix://` works is unverified;
  fall back to `ws://127.0.0.1:<port from the host range>` with `--ws-auth` and a per-pdtd token. The plan's first task
  checks this.
- Minimum versions: Claude 2.1.302, Codex 0.163.0 (Settings → host checks warn when older, main spec §5.6).

## 4. Events

### 4.1 Normalized events (pdt-core)

| Kind | Fields (all have `seq`, `ts`, `turnId?`) | Claude source | Codex source |
|---|---|---|---|
| `user_message` | text, images[], `source` (pdt / terminal / steer) | replayed `user` | `userMessage` item |
| `agent_message` | id, markdown, `phase` (commentary / final) | `assistant` text blocks (joined by `message.id`) | `agentMessage` item |
| `reasoning` | summary (often empty) | thinking blocks | `reasoning` item |
| `tool_call` | id, tool, `title` (one line), input, status (running / done / failed / declined), output, exitCode?, diff?, parentId? | `tool_use` + `tool_result` | `commandExecution`, `fileChange`, `mcpToolCall`, `webSearch`, … |
| `request` | id, kind (`approval` / `question` / `plan`), payload, options, state (open / answered / expired), answer? | `control_request can_use_tool` (AskUserQuestion → question, ExitPlanMode → plan) | `*/requestApproval`, `item/tool/requestUserInput` |
| `plan` | markdown, steps? | ExitPlanMode input, TodoWrite/Task* | `plan` item, `turn/plan/updated` |
| `turn` | phase (started / ended), status (completed / interrupted / failed), usage?, durationMs | `result` | `turn/started`, `turn/completed` |
| `usage` | context used / window, rate-limit windows, cost (Claude list price, shown as "≈") | `result.usage`, `rate_limit_event` | `thread/tokenUsage/updated`, `account/rateLimits/updated` |
| `turn_diff` | unified diff of the turn | built from Edit/Write tool inputs | `turn/diff/updated` |
| `compaction` | before/after tokens | `compact_boundary` | `contextCompaction` item |
| `notice` | level, text (model changed, mode changed, agent restarted, …) | `system/status`, local command output | `thread/settings/updated`, `model/rerouted` |
| `error` | text, retryable | `result` errors, stderr | `error`, `turn/completed failed` |
| `unknown` | raw JSON | anything else | anything else |

- Each adapter keeps a **raw** copy of every protocol frame it received in the event's `raw` column (capped 64 KB per
  event; bigger tool output is truncated in `raw` but kept whole in `output` up to 1 MB, beyond which it's cut with a
  note). The raw frames make drift debuggable and feed the fake-agent fixtures.
- Streaming deltas (text, command output) are **not stored**: they go only to live WebSocket subscribers as
  `delta {eventId, field, text}`; the completed item is stored once. A client that connects mid-item gets the item's
  text so far as one delta.

### 4.2 Store

New migration (number after Phase 4's):

```sql
CREATE TABLE session_events (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,          -- per session, from 1
    ts         INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    item_id    TEXT,                      -- tool call / message / request id, for updates in place
    turn_id    TEXT,
    body       TEXT NOT NULL,             -- the normalized event (JSON)
    raw        TEXT,                      -- protocol frame(s), capped
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX session_events_by_item ON session_events (session_id, item_id);
ALTER TABLE sessions ADD COLUMN mode TEXT NOT NULL DEFAULT 'terminal';   -- terminal | conversation
ALTER TABLE sessions ADD COLUMN model TEXT;
ALTER TABLE sessions ADD COLUMN permission_mode TEXT;                     -- §6.1
ALTER TABLE sessions ADD COLUMN history_from TEXT;                        -- agent's last imported entry (§8.3)
```

- `SessionKind` stays `shell | agent_tui`, renamed in the API to `shell | agent` (the TS type follows); `mode` says
  how an agent session is shown and driven. Old rows are `terminal`.
- An item that changes (a tool call finishing, a request being answered) is **updated in place** by `item_id` and gets
  a new `seq` (the row moves to the end), so "events after seq N" always includes every change.
- The event log is PDT's record (main spec §4); the agent's own transcript remains the agent's. They are reconciled
  only when switching modes (§8.3) or when PDT's log is empty (rebuilt with `read_history`).

### 4.3 Activity

Derived from events, no hooks or screen reading: `working` between `turn started` and `turn ended`; `needs_you`
while any request is open (and on `error`); `idle` after a turn ends; `unknown` only when parked with no history.
Attention (`attention_at`, `unseen`) works as today. In terminal mode the existing hook/screen logic still applies.

## 5. API

| Route | Does |
|---|---|
| `POST /api/sessions` | gains `mode: conversation \| terminal` (default conversation for agents), `model?`, `permissionMode?`, `initialPrompt?` (sent as the first message in conversation mode) |
| `GET /api/sessions/{id}/events?after=<seq>&limit=` | stored events, oldest first |
| `WS /api/sessions/{id}/events?after=<seq>` | the same, then live: `event` (new or updated row), `delta`, `state` (activity, live/parked, model, mode, usage) |
| `POST /api/sessions/{id}/messages {text, images?, delivery: next\|now}` | send; opens/resumes the agent if parked. `now` interrupts the current tool (Claude `priority:"now"`; Codex interrupt + start) |
| `POST /api/sessions/{id}/requests/{reqId} {answer}` | answer an approval/question/plan; `409 request_closed` if already answered (another device, or the TUI for Codex) |
| `POST /api/sessions/{id}/interrupt` | stop the turn |
| `PATCH /api/sessions/{id}` | gains `model`, `permissionMode` (applied live: Claude `set_model`/`set_permission_mode`; Codex next `turn/start` overrides) |
| `POST /api/sessions/{id}/command {name, args}` | agent commands (§7.2) |
| `POST /api/sessions/{id}/mode {mode}` | switch conversation ↔ terminal (§8) |
| `GET /api/agents/{agent}/capabilities` | models, modes, slash commands, version (cached per pdtd, refreshed on version change) |

Images: `POST /api/sessions/{id}/attachments` (multipart, ≤ 10 MB, png/jpeg/gif/webp) stores into
`<data dir>/sessions/<id>/attachments/` and returns an id; messages reference ids. Claude gets base64 blocks, Codex
`localImage` paths.

## 6. Approvals, questions and plans

- Every request becomes a `request` event and an **`ApprovalCard`** in the stream (UX spec §8.2), with the session's
  dot amber (Needs you) until answered. A request has no timeout: it waits until answered, interrupted or the agent
  exits (then `expired`).
- **Approval** (command, file change, MCP tool, other tool): shows the command with its folder, or the diff; actions
  **Approve** (`y`), **Deny** (`n`, with an optional "tell it why" field → Claude `deny.message`; Codex has no message,
  so the note is sent as the next message), **Always allow in this session** — Claude `addRules … destination:
  "session"` from the request's own `permission_suggestions`, Codex `acceptForSession`. PDT **never** writes the
  user's or project's settings files (no `localSettings`/`projectSettings` destinations).
- **Edit before approve** (Claude only, `updatedInput`): deferred to 5c; when added, the card marks the call "edited"
  because the model still sees its original input.
- **Question** (Claude AskUserQuestion, Codex requestUserInput): a `QuestionCard` with each question's options as
  radio or checkbox chips plus "Other…" text, Submit; answers go back in the agent's shape.
- **Plan** (Claude ExitPlanMode, Codex plan mode): a `PlanCard` with the plan's markdown and **Approve plan** /
  **Keep planning** (with feedback text).
- Any open request can be answered from any device; the first answer wins and the card resolves everywhere.

### 6.1 Permission modes

One PDT setting per session, chosen in the composer footer and in Start / new-tab (default from the project, later
the persona):

| PDT mode | Claude | Codex |
|---|---|---|
| Ask (default) | `default` | `approvalPolicy: on-request`, `sandbox: workspace-write` |
| Accept edits | `acceptEdits` | `on-request`, `workspace-write` (edits in the sandbox already run) |
| Plan | `plan` | collaboration mode `plan` (experimental API) |
| Bypass | `bypassPermissions` | `never`, `danger-full-access` |

`bypass_permissions` from today becomes `permission_mode = bypass` (backfilled when the store opens); the skip-permissions
switch in the new-tab picker becomes this mode picker.

## 7. The conversation view (web)

UX spec §8.2 stands; this adds what the protocols showed.

- **`MessageList`** (virtualised, sticky-bottom, "Jump to latest"): user messages right-aligned in `surface-2`;
  agent `final` messages as `Markdown`; `commentary` messages smaller and muted, folded into the turn's activity;
  **`ToolCall`** rows ("Run `cargo test`" ✓ 12 s, "Edit `src/app.ts` +12 −3") expanding to input and output (output in
  a terminal-styled block with ANSI colours, diffs in `DiffViewer` inline). Consecutive tool calls in a turn collapse
  into "Ran 6 commands, edited 3 files" while the turn is done. Sub-agent calls (`parentId`) nest under their parent.
- **Turn footer**: duration, a "Changes" link opening the turn's diff (`turn_diff`), and for an interrupted or failed
  turn the reason.
- **Streaming**: text and command output stream through deltas; a working turn shows a quiet spinner row with the
  current tool ("Running `cargo test`… 0:42").
- **Composer** (the existing `Composer`, grown): Enter sends, Shift+Enter new line; while working, Enter **queues the
  message for the agent's next opportunity** (Claude injects at a tool boundary or runs it next; Codex `turn/steer`)
  and shows it as a pending bubble until the agent takes it; ⌘/Ctrl+Enter is **Send now** (interrupt and send).
  Escape (or the Stop button) interrupts. Paste or drop images. `@` opens a file picker over the Files API and inserts
  the path. Footer: model picker, permission-mode picker, context used (`usage`), rate-limit warning above 80 %.
- **Slash commands**: `/` opens a `CommandList` of the agent's commands (from capabilities: Claude's `initialize`
  commands minus `terminal_slash_commands`; Codex's built-ins PDT maps: `/compact`, `/review`, `/model`, `/rename`,
  `/goal`, plus skills). Claude commands are sent as text (Claude expands them); Codex ones go to their RPC; a command
  PDT can't run shows "Only in the terminal — Open in terminal".
- **Unknown events**: a collapsed "Unrecognised event" row with the raw JSON.
- **Phone**: the same view, composer pinned above the keyboard, cards full-width; this is the main reason phones get
  agent sessions at all.

### 7.1 Header and tabs

The session `ViewHeader` gets a `Conversation | Terminal` segmented switch (§8) and the agent, model and ticket key.
Tab titles and sidebar rows are unchanged (`PDT-42 · Claude`); conversation sessions use the same `AgentState` dot.

### 7.2 Agent commands

`POST …/command` names: `compact` (Claude: `/compact` as text; Codex `thread/compact/start`), `review` (Codex
`review/start`; Claude: its review command as text), `rename` (Claude `rename_session`; Codex `thread/name/set`),
`shell {command}` (Codex `thread/shellCommand`; Claude: refused, use a shell tab).

## 8. Switching between conversation and terminal

The switch is per session; the same row, tab and history serve both.

### 8.1 Codex: both live

"Terminal" opens a PTY running `codex resume <thread id> --remote unix://<data dir>/codex/app.sock` (with PDT's usual
env and notify). The TUI and the conversation share the thread live: turns typed in either appear in both, and an
approval can be answered in either (first wins; the other side sees `serverRequest/resolved`). Closing the terminal
leaves the thread running in the conversation. Activity comes from the app-server, not the notify hook. If attaching
fails (Windows transport, version), fall back to §8.2's handoff.

### 8.2 Claude (and later Devin): one at a time

- **To terminal**: if a turn is running, ask "Interrupt and open the terminal?" (or "Wait for this turn"); then close
  the headless process and start `claude --resume <id>` in a PTY with PDT's hook settings (today's terminal session).
  The conversation shows a banner "Open in the terminal" and its composer is replaced by **Return to conversation**.
- **To conversation**: stop the TUI (send `/exit`, then kill after 5 s), import the turns it took (§8.3), and mark the
  session `conversation`; the next send resumes headless.
- Today's terminal sessions get the same switch, so old sessions can become conversations.
- Verified in 5a: an interactive `claude --resume` opens a session made by `-p` cleanly (same transcript file). As
  built (5c), the switch changes the session's `mode`: the session view shows whichever side has it, with the
  Conversation | Terminal control in the header, rather than a banner over a read-only conversation.

### 8.3 Importing history

When a session comes back from the terminal, or its PDT log is empty, `read_history` reads the agent's transcript
from `history_from` onwards (Claude: entries after that `uuid` in the session jsonl; Codex: turns after that turn id
via `thread/turns/list`) and appends them as events with `source: terminal`. Imported events have no `raw` deltas or
usage detail; that's fine.

## 9. Lifecycle

- **pdtd restart / update**: Claude processes die with pdtd; the app-server too. Conversation sessions need no
  auto-resume: they show their log and reopen on the next send. A turn that was running is marked `interrupted` with
  a **Continue** button (sends "Continue where you left off."); open requests become `expired`. Terminal-mode
  sessions keep today's auto-resume.
- **Stop** (⋯ menu): closes the conversation's process and marks the session `finished`; **Restart** isn't needed —
  sending to a finished conversation reopens it (it becomes `running`).
- **Remove**: as today; the event rows cascade.

## 10. Tickets and scratch chats

- **Start** (Phase 4 §5) gains `mode` (conversation default); in conversation mode the ticket prompt is the first
  message instead of argv. The Start dialog's agent row becomes "Claude / Codex" × "Conversation / Terminal".
- **Ticket tab → Conversation** shows the attempt's conversation sessions with a switcher above the stream (UX spec
  §8.2); a conversation session belonging to an attempt still has its own tab too, until Phase 7 moves pipeline
  sessions into the ticket tab.
- **Scratch chats** are just conversation sessions with no attempt. Their ⋯ menu has **Make ticket…**: PDT asks the
  agent (as a hidden side request — a separate one-turn `claude -p` / Codex ephemeral thread given the transcript) for
  a title and summary and opens the ticket form with them; then links the session to the ticket's new attempt if the
  user chooses "Start from this chat".

## 11. Devin (later; adapter-neutral now)

From the research (notes file): Devin CLI is a local agent with an **ACP** server (`devin acp`, Agent Client Protocol
over stdio). The plan for it, not built in Phase 5:

1. **Terminal session type** first (works on the work laptop without any protocol risk): `devin` / `devin -r <id>`,
   activity from a PDT hooks file if `--config` can add one without touching the user's files, else OSC 9/777
   notifications. Make sure PDT's Claude hook settings never reach Devin (Devin also reads `~/.claude` hooks).
2. **`AcpAdapter`**: one `devin acp` per live session; `session/new` → `external_session_id`; `session/load` to resume;
   `session/prompt` per message (its response ends the turn); `session/update` → events; `session/request_permission`
   → approval; `elicitation/create` → question; `session/cancel` → interrupt; config options for model, `set_mode` for
   the permission mode. One-at-a-time switching like Claude (Devin locks a session to one process). The ACP framing
   and mapping are generic, so other ACP agents come almost free.

Hands-on checks before building it are listed in the notes file (org policy for custom ACP clients first).

## 12. Testing

- **pdt-core (TDD)**: each adapter's frame → event mapping from recorded fixtures (both agents, every event kind in
  §4.1, unknown frames); item updates in place; activity derivation; permission-mode mapping; slash-command routing;
  answer shapes for every request kind.
- **fake-agent** (main spec §9): a test binary that plays recorded stream-json and app-server sessions, answers
  control requests, honours interrupt, and can be pointed at with `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM`.
  Fixtures are recorded from real runs with a script in `scripts/` and re-recorded when minimum versions change.
- **pdt-api**: sessions in conversation mode end to end with fake-agent: send, stream, approve, deny, question, plan,
  interrupt, park and resume, pdtd restart → interrupted + Continue, `request_closed` races, mode switches with import.
- **Web**: MessageList (virtualised, sticky bottom, deltas), ToolCall, ApprovalCard / QuestionCard / PlanCard keys,
  Composer queue/now/slash/@/images, mode switch, phone layout. Gallery entries for every new `@ui` component.
- Windows + macOS: the Codex transport fallback has its own test; no real agent runs in CI.

## 13. Build order

- **5a — Claude conversations**: adapter trait, events + store, ConversationManager, events WebSocket, Claude adapter,
  fake-agent; web MessageList, ToolCall, Composer (send, queue, interrupt), ApprovalCard, permission modes, new-tab
  "Claude" opens a conversation. First task: confirm `claude --resume` in a TUI on a `-p` session, and the Windows
  `claude -p` path.
- **5b — Codex conversations**: pdtd's app-server (and the Windows transport check first), Codex adapter, history via
  `thread/turns/list`, live shared terminal (§8.1).
- **5c — the rest of the conversation**: QuestionCard, PlanCard, slash commands, images, `@` files, turn diffs, usage
  and rate limits, Claude's terminal handoff (§8.2) and history import, Continue after restart.
- **5d — tickets**: Start in conversation mode, ticket Conversation tab, scratch chats → Make ticket.

## 14. Terms and billing

Checked 2026-10-18 (docs/superpowers/notes/agent-terms-2026-10.md, with quotes and links). **Personal use is allowed**
by all three vendors: Anthropic explicitly lets `claude -p` draw from a Claude plan ("ordinary, individual usage"),
OpenAI lets local and open-source apps use app-server's ChatGPT login (never commercial or hosted), and Devin's ACP
mode is meant for third-party clients (the employer's policy decides the work laptop). **Releasing PDT to others** on
subscription logins is plausible as free, open-source, bring-your-own-CLI, but not settled for Anthropic; a release
needs a re-check and probably an API-key path. Guardrails (amending main spec §5.5):

- Launch only the user's installed, unmodified `claude` / `codex` / `devin`; never spoof a client, never disable their
  login methods. Login happens in the CLI's own flow (a terminal tab), never in PDT.
- Never read, copy, store or refresh agent credentials (`~/.claude`, the keychain entries, `~/.codex/auth.json`,
  Devin's `credentials.toml`). Sign in with ChatGPT would break this rule, so it's out unless chosen deliberately.
- Strip `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `OPENAI_API_KEY`, `WINDSURF_API_KEY` by default; a per-project
  **"Use API key"** setting passes one through (e.g. a Max plan's included API credits). Never pass `--bare`, and the
  Claude adapter fails loudly (a host check) if `-p` stops using the plan login: Anthropic says `--bare` will become
  the default for `-p`.
- Each session shows its billing source in the composer footer ("Claude plan", "API key", "ChatGPT plan",
  "Devin Enterprise"), with the vendors' own rate-limit events.
- **Rate limits stop, never retry.** When an agent reports a hit limit (Claude `rate_limit_event` / a rate-limit
  `result` error; Codex `account/rateLimits/updated` at 100 % or a rate-limit turn error), the session shows "Rate
  limited until <reset time>" as a `notice` and in its footer; PDT sends nothing to it automatically, and queued
  messages stay queued until the user sends again. Otherwise nothing sends messages without the user (Continue is a
  button, Make ticket is a click), with one deliberate exception (decided 2026-10-19): **PR notices** (PDT-25, PDT-44).
  News about the user's own pull request (CI failed, a conflict, removed from the merge queue, merged) is sent once
  per event to the newest running agent session of that ticket's attempt, terminal or conversation, only while it is
  idle, nobody has typed in it for 20 s and it is not rate limited. In a conversation the message is marked as PDT's.
- One owner per host: the owner-only Tailscale identity check stays; no sharing sessions with anyone else.

## 15. Open questions

- Windows: Codex `--listen unix://` support; `claude -p` stdio behaviour under Windows process handling.
- ~~Claude TUI resume of a `-p` session (§8.2).~~ Works both ways (checked in 5a); the switch shipped in 5c.
- Whether to use Codex's experimental API (plan mode, queue) from day one or only behind a capability check — proposed:
  capability check, fall back to PDT's own queue and no plan mode.
- Edit-before-approve (later; not in 5c).

## 16. Deferred

- Personas, pdt-mcp, the pipeline (Phase 7) — `OpenSpec` already carries their fields.
- **Concurrency cap** (Phase 7, when PDT starts work itself): a Host setting limiting agents running a turn at once,
  counting conversations and agent terminals alike; idle sessions don't count. Pipeline work over the cap waits
  ("Waiting for a slot"); the loop limit (main spec §6.2) keeps hand-offs from running unattended forever.
- Forking/rewinding conversations (Claude `rewind_files`, Codex `thread/fork`/`revert`).
- Background-terminal management for Codex (`thread/backgroundTerminals/*`).
- Devin (§11).
