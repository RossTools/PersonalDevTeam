# Phase 5b: Codex conversations

Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md (§3.2, §8.1, §13 5b). Protocol facts:
docs/superpowers/notes/phase-5-agent-protocols.md ("5b checks"). Builds on 5a
(docs/superpowers/plans/2026-10-18-phase-5a-claude-conversations.md). Ticket PDT-2.

Goal: a new-tab "Codex" opens a conversation driven through PDT's own `codex app-server`, and the same thread can be
opened in the real Codex TUI, live, beside it.

## 0. First task: transport (done 2026-10-18)

- One `codex app-server --listen ws://127.0.0.1:<port> --ws-auth capability-token --ws-token-file
  <data dir>/codex/token` per pdtd, on every OS. The TUI attaches with `--remote ws://… --remote-auth-token-env`;
  checked live on macOS. This replaces the spec's unix socket with a Windows fallback: one transport (plain TCP on
  Windows too), tested the same way everywhere. The token file is 0600; the port is a free loopback port.
- Never the user's shared daemon (`~/.codex/app-server-control`).

## 1. pdt-core (TDD)

- `conversation/codex.rs`: `CodexMapper` — notifications for one thread → events, server requests → `request` events
  (approval: command / file change / permissions; question: `item/tool/requestUserInput`), answers → JSON-RPC result
  (`accept` / `acceptForSession` / `decline`; questions `{answers:{id:{answers:[…]}}}`), `serverRequest/resolved`
  closes a request answered elsewhere (the TUI), deltas (`item/agentMessage/delta`,
  `item/commandExecution/outputDelta`), usage (`thread/tokenUsage/updated`, `account/rateLimits/updated`),
  `turn/diff/updated` → `turn_diff`, known noise ignored, other thread notifications → `unknown`. Items mapped from
  `thread/turns/list` for history import (source `terminal` for turns PDT didn't see).
- Permission modes: Ask/Accept edits → `on-request` + `workspace-write`; Plan → `on-request` + `read-only` (Codex's
  plan mode is experimental; this is the safe fallback); Bypass → `never` + `danger-full-access`.

## 2. pdt-agents

- `adapter/codex.rs`: `CodexServer` (spawns and supervises the app-server: free port, token file, readiness by
  connecting, restart with backoff, one WebSocket client connection multiplexing every thread, request ids, server
  requests routed by `threadId`) and `CodexAdapter` / `CodexLive` (thread/start or thread/resume, turn/start, steer
  while a turn runs, interrupt, Send now = interrupt + start, settings as next-turn overrides, close =
  `thread/unsubscribe`). `OpenSpec` gains `external_id`; `AdapterOutput` gains `ExternalId` (the thread id, stored as
  `external_session_id`) and `History` (turns PDT has not logged, imported on resume).

## 3. pdt-api

- Codex sessions default to `mode: conversation` (Codex terminal stays available). Capabilities from `model/list`.
- `POST /api/sessions/{id}/terminal` opens the live shared TUI in a PTY keyed by the session id
  (`codex resume <thread> --remote ws://… --remote-auth-token-env PDT_CODEX_TOKEN`); the PTY WebSocket is the usual
  `/api/sessions/{id}/pty`. `DELETE` closes it. Closing the TUI leaves the conversation as it was. Session gains
  `terminalOpen`.
- fake-agent gains `app-server` mode (WebSocket JSON-RPC with the token check), mirroring its Claude commands.
- Tests: start, stream, approve / always / deny (note sent as the next message), steer, interrupt, send now,
  resolved-elsewhere, server crash → restart and resume, history import, restart recovery, terminal open/close.

## 4. Web

- New-tab "Codex" (x) opens a conversation; "Codex (terminal)" keeps the TUI. Codex sessions in conversation mode get
  a `Conversation | Terminal` switch in the header: Terminal opens (or shows) the live TUI; Conversation goes back
  (the TUI keeps running until closed from its menu).

## Out of 5b

Claude's terminal handoff and the switch for Claude (5c); question/plan cards (5c); Codex collaboration plan mode and
queue APIs (experimental; later, behind a capability check).
