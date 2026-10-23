# Conversations (structured agent sessions)

Phase 5. Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md; protocol facts:
docs/superpowers/notes/phase-5-agent-protocols.md; plans: docs/superpowers/plans/2026-10-18-phase-5a…,
2026-10-18-phase-5b…, 2026-10-19-phase-5c… and 2026-10-19-phase-5d…. Devin's conversations are in
[agents.md](agents.md#devin-pdt-54-pdt-55).

An agent session (Claude, Codex or Devin) is `mode: conversation` by default (`mode: terminal` for the TUI; ticket
Start too).

## Runtime and API (5a)

- Claude: pdtd drives `claude -p` stream-json (pdt-agents `adapter/claude.rs`; never `--bare`, no PDT hook settings,
  API keys scrubbed as for terminals) and maps frames to normalized events in pdt-core `conversation/claude.rs` (TDD;
  fixture in crates/pdt-core/tests/fixtures/claude).
- pdt-api `conversations.rs` owns each session's runtime: events go to `session_events` (an item that changes keeps
  its row and `pos` but gets a new `seq`; display order is `pos`), streaming text goes only to live viewers as
  deltas, activity is derived from events.
- API:
  - `GET /api/sessions/{id}/events`: JSON (stored events after `after`, up to `limit`; or with `last=N` the newest N,
    oldest first), or the WebSocket stream when upgraded (stored events after `after`, `state`, partial deltas,
    `ready`, then live).
  - `POST …/messages {text, delivery: next|now}`, `POST …/requests/{requestId} {answer}` (409 `request_closed`),
    `POST …/interrupt`.
  - `PATCH /api/sessions/{id}` (`title`, `model`, `permissionMode`), `GET /api/agents/{agent}/capabilities`.
- Lifecycle: the process starts with the session, parks after 15 min idle and reopens on the next message
  (`--resume` once Claude saved the conversation); Kill stops it (`finished`), sending reopens it.
- Background tasks (PDT-96): Claude's `background_tasks_changed` becomes `ConversationState.backgroundTasks` (shown
  over the composer); a conversation with any running is never parked, since parking would end them before they
  report back. A finished task's report (a replayed user message with `origin.kind: task-notification`) starts a turn
  as Claude's own and shows as a notice with its summary, not as the user's message (live and in imported history). A pdtd restart ends
  a running turn as interrupted and expires open requests (`recover_conversations`); conversations are never
  auto-resumed as terminals (the web offers Continue on an interrupted last turn; nothing is sent without a click).
- "Always allow" sends Claude's own suggestions with `destination: "session"` only (never settings files).

## Codex (5b)

- One PDT-owned `codex app-server --listen ws://127.0.0.1:<free port> --ws-auth capability-token --ws-token-file
  <data dir>/codex/token` per pdtd on every OS (never the user's shared daemon), started on first use and again after
  a crash (pdt-agents `adapter/codex.rs`; mapping in pdt-core `conversation/codex.rs`). It follows the host's
  credential setting (one server serves every project).
- The thread id is the session's `external_session_id`. A message sent while a turn runs is steered into it; Send now
  interrupts and starts a new turn. A deny note is sent as the next message (Codex's decline has none). "Always
  allow" is `acceptForSession` only when offered (never the persistent exec-policy amendment).
- On resume, turns PDT has not logged are imported from `thread/turns/list` (`source: terminal`); a thread that never
  took a turn cannot be resumed after the server restarts, so a new one starts with a notice.
- `POST|DELETE /api/sessions/{id}/terminal` opens/closes the real TUI live on the same thread (`codex resume <thread>
  --remote ws://… --remote-auth-token-env PDT_CODEX_TOKEN`, PTY keyed by the session id; 409 `no_turn_yet` before the
  first turn); `ConversationState.terminalOpen`.

## Switching, diffs, plans (5c part 1)

- Claude (and Devin) sessions switch between conversation and TUI one at a time with
  `POST /api/sessions/{id}/mode {mode, cols?, rows?, interrupt?}` (409 `turn_running` unless `interrupt`; 409
  `switch_unsupported` for Codex and shells).
  - To terminal parks `-p`, stores the transcript's last uuid in `sessions.history_from` (`""` when Claude had not
    saved it yet) and restarts as a terminal session (`--resume`, hooks).
  - Back to conversation types `/exit` into the TUI (killed after 5 s), imports the transcript after `history_from`
    (`claude::history_events`, `source: terminal`; everything for a session that was always a terminal) and clears
    it.
- Edit/MultiEdit/Write calls and edit requests carry a unified `diff` (from `structuredPatch` / a created file's
  content, else the input), paths relative to the session folder; each turn ends with a `turn_diff` of its edits
  inside the folder.
- `Answer::Plan.mode` approves a plan into another mode (Claude `setMode`, session only).

## Composer (5c part 2)

- Images go up first as `POST /api/sessions/{id}/attachments` (raw body; PNG/JPEG/GIF/WebP by their bytes, ≤ 10 MB;
  stored under `<data dir>/sessions/<id>/attachments/`, removed with the session; `GET …/attachments/{attachmentId}`
  serves them) and a message names them in `images` (≤ 10; text may then be blank); Claude gets base64 image blocks,
  Codex `localImage` paths; the echoed `user_message` row lists the attachment ids.
- `GET /api/sessions/{id}/files?q=` ranks the session folder's files (git's list) for `@` (pdt-core
  `conversation/composer.rs`).
- `/` lists capabilities' `commands` (Claude: `initialize`; its `terminal_slash_commands` arrive as
  `terminalCommands`, shown as "Only in the terminal"). Claude commands go as text (a local one like `/context` is
  never echoed, so the mapper shows it when its turn starts); Codex's `/compact` and `/review [instructions]` run
  `thread/compact/start` / `review/start`.
- Unsent text is kept per session in sessionStorage (web/src/conversation/drafts.ts), so clicking away from a
  conversation and back keeps it (and a reload of the tab); sending clears it. The PM's start box (no session yet)
  keeps its text the same way, under `pm-start`.
- `ConversationState.billing` ("Claude plan" / "API key" from `apiKeySource`; Codex from `account/read`'s account
  type only) shows in the composer, with a rate-limit line from 80 % (web `limitWarning`).

## Tickets (5d)

- Start takes `sessionMode` (`conversation` default, the ticket prompt as its first message; `permissionMode`, or
  `bypassPermissions` for the `bypass` permission mode; the dialog sends the host's ticket default, PDT-75) and `fromSession`: a chat with no ticket in a worktree off the base branch becomes the
  attempt (its worktree and branch, `sessions.attempt_id` set, the ticket tab opened just before the chat's; no new
  session and nothing sent); else 409 `chat_not_startable`. Ticket sessions carry `mode`.
- `POST /api/sessions/{id}/ticket-draft` → `{title, description}` (Make ticket…, the chat's ⋯ menu): a hidden side
  request that never touches the conversation. Claude: `-p --resume <id> --fork-session --no-session-persistence
  --tools "" --output-format json --json-schema …` with the prompt on stdin (`structured_output`); Codex: an ephemeral
  read-only `thread/fork` (`excludeTurns: true`) plus a `turn/start` with `outputSchema`. 409 `nothing_to_draft`
  before the agent saved a turn, 504 `draft_timeout` after 120 s, no retries.
- PR notices (PDT-44) and other `session.send` messages from workflows (docs/reference/workflows.md) also reach
  conversations: the newest running agent session of the attempt, once it is idle with
  nothing queued, its terminal closed, no rate limit hit (`limited_until` from `usage` rate limits) and quiet for
  20 s; the message is `source: notice` ("Sent automatically").

## Tests and web

- Tests use `fake-agent` (crates/pdt-api/src/bin/fake-agent; `run:`, `ask:`, `slow`, `exit`, `env:`, `who`) as
  `claude`, and `fake-agent app-server` / `fake-agent resume` as Codex's server and TUI (`fake-agent acp` for Devin).
- web/src/conversation (events socket + reducer, MessageList with turn Changes and Continue, composer); `@ui`
  ApprovalCard (approvals with a diff preview, questions with "Other…", plans with "Approve, accept edits" for
  Claude; a tool's arguments as the title's summary with "All arguments ▾" for the rest; Allow/Deny), DiffViewer,
  MessageBubble, ToolCallRow. A finished run of tool calls folds to one line ("Ran 3 commands", or for one other tool
  "search_tickets · 2 calls").
- **Pinned approval bar** (web `PinnedApprovalBar`, PDT-113): in every conversation, while a request is open, a line
  over the composer ("⚠ wants create_ticket · Review", "· 1 more" when there are others); Review scrolls to the
  oldest open request's card and focuses it.
- **On a phone** (PDT-113): the composer's text grows to about five lines with one round button beside it (⬆ sends;
  ■ Stop while the agent works and the box is empty; text while it works queues), and attach, model, permission mode
  and the agent's state (working / ready / stopped) sit on a row under it. With the on-screen keyboard up, Enter
  adds a new line (a hardware keyboard, which brings no on-screen one, keeps Enter to send: web
  `common/viewport.ts` compares the visual viewport with the layout one), and the shell fits the visible part
  above the keyboard so the composer stays in view. Approval cards put Deny and Allow side by side at their foot as
  44 px buttons.
- The session header's Conversation | Terminal switch (Codex: live shared TUI; Claude/Devin: `…/mode`); new-tab
  "Claude" (c) is a conversation, "Claude (terminal)" (t) the TUI.
- The Start dialog's Conversation | Terminal choice (the host's default view), the ticket tab's Conversation
  (web/src/tickets/TicketConversation.tsx: the ticket's sessions newest first over the chosen conversation),
  MakeTicketDialog (draft, then the new-ticket form with "Start from this chat").
