# Phase 5a: Claude conversations

Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md (§13 5a). Protocol facts:
docs/superpowers/notes/phase-5-agent-protocols.md. Ticket PDT-2.

Goal: a new-tab "Claude" opens a **conversation**: PDT drives `claude -p` stream-json and draws messages, tool calls
and approval cards, with a composer that sends, queues and interrupts. "Claude (terminal)" keeps today's TUI.

## 0. First task (done 2026-10-18)

- `claude --resume <id>` in the TUI on a `-p` session: works both ways (notes file, "TUI ↔ `-p` round trip").
- Windows `claude -p`: not checkable by hand here; the adapter pipes stdio (no PTY), `.cmd` shims go through
  `cmd.exe /d /c` with flags only; the fake-agent API tests run on the Windows CI (`windows` label).

## 1. pdt-core (TDD)

- `conversation.rs`: `SessionMode` (terminal | conversation), `PermissionMode` (ask | accept_edits | plan | bypass,
  `claude_mode()`), `AgentEvent` (spec §4.1 kinds; `#[serde(tag = "kind")]`), `ConversationEvent` (a stored row:
  `seq`, `pos`, `ts`, `itemId`, `turnId`, `event`), `Answer` (approve {always} | deny {message} | question {answers}
  | plan {approve, feedback}), WebSocket frames `ConversationServerMessage` (`event` | `delta` | `state`),
  `ConversationState`, `UserInput`, `Delivery` (next | now), activity derivation.
- `conversation/claude.rs`: `ClaudeMapper`, frame → `MapOut` (`Upsert(event)` | `Delta` | `Init(info)` |
  `ControlReply`), from recorded frame shapes; answer → `control_response` body; user message → stdin line.

`pos` (deviation from §4.2): the row's first-seen position. An updated item gets a new `seq` (so "after seq N" sees
the change) but keeps `pos`, so the list stays in conversation order.

## 2. pdt-store

- Migration `0014_conversations.sql`: `session_events` (spec §4.2 + `pos`), `sessions.mode/model/permission_mode/
  history_from`. Backfill in `Store::open`: `permission_mode = 'bypass'` where `bypass_permissions = 1`.
- `upsert_event` (insert, or update in place by `item_id` with a new `seq`), `list_events(after, limit)`,
  `set_session_settings(model, permission_mode)`.

## 3. pdt-agents

- `adapter/mod.rs`: `AgentAdapter` / `LiveConversation` traits (spec §3.1), `OpenSpec`, `AdapterOutput` sink
  (`Event{event, raw}` | `Delta` | `Exited{code, stderr_tail}`).
- `adapter/claude.rs`: spawns `claude -p …` (tokio process, piped stdio, scrubbed env, login PATH; never `--bare`,
  never PDT's hook settings), reader task (stdout lines → `ClaudeMapper`), writer (stdin), control requests with
  replies, stderr tail, close = close stdin then kill after 5 s.

## 4. pdt-api

- `conversations.rs`: `ConversationManager` in `AppState` — live session map, per-session event pump (store upsert,
  broadcast, activity), partial text per streaming item, pending (queued) messages, idle parking (15 min, no open
  request), interrupt/answer routing, recovery at startup (running turns → `interrupted`, open requests →
  `expired`).
- Routes: `POST /api/sessions` gains `mode` (default `conversation` for Claude, `terminal` for Codex until 5b),
  `model`, `permissionMode`; `initialPrompt` is the first message in conversation mode.
  `GET /api/sessions/{id}/events?after&limit`, `WS /api/sessions/{id}/events?after`,
  `POST /api/sessions/{id}/messages {text, delivery}`, `POST /api/sessions/{id}/requests/{reqId} {answer}`
  (`409 request_closed`), `POST /api/sessions/{id}/interrupt`, `PATCH /api/sessions/{id}` gains `model`,
  `permissionMode`, `GET /api/agents/{agent}/capabilities`. Kill on a conversation closes the process and marks it
  `finished`; sending to a finished/interrupted conversation reopens it. Auto-resume skips conversations.
- `fake-agent` bin (pdt-api, test-only use): plays Claude stream-json — init, echo replies with deltas, `run: <cmd>`
  (approval), `slow` (interruptible), `ask:` (AskUserQuestion), `exit` (crash), honours `--resume`.
- Tests (pdt-api/tests/conversations.rs): send/stream, approve, deny, always-allow (session destination only),
  interrupt, queue while working, crash → error + next send resumes, park → resume, restart recovery,
  `request_closed` race, events WS replay + live.

## 5. Web

- `web/src/conversation/`: `ConversationView` (in `SessionView` when `mode === 'conversation'`), `useConversation`
  (events WS, replay then live, deltas, reconnect), `MessageList` (sticky bottom, Jump to latest, offscreen rows
  skipped with `content-visibility`), `ToolCallRow`, turn footer, `ConversationComposer` (Enter send / queue while
  working, ⌘/Ctrl+Enter send now, Escape or Stop interrupts, model + permission-mode pickers).
- `@ui`: `ApprovalCard` (Approve `y`, Deny `n` with "tell it why", Always allow), `MessageBubble`, `ToolCall` rows;
  gallery entries.
- New-tab chooser: "Claude" → conversation, "Claude (terminal)", "Codex" (terminal until 5b); the skip-permissions
  switch becomes the permission-mode picker for Claude conversations.

## Out of 5a (later sub-phases)

QuestionCard/PlanCard (5a shows questions and plans in the approval card: answer by approve/deny), slash command
list, images, `@` files, turn diffs, usage/rate-limit footer (rate-limit notices are in 5a), terminal handoff and
history import, Continue button (5c); Codex (5b); tickets (5d). The `agent_tui` → `agent` API rename is left for 5c
with the mode switch.
