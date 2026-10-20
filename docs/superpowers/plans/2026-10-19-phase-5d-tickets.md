# Phase 5d — tickets and conversations

Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md §10; UX spec §8.2. Last sub-phase of PDT-2.

## Scope

1. **Start in conversation mode.** `StartTicket.sessionMode` (`conversation` default, `terminal`): a conversation's
   first message is the ticket prompt (the same `ticket_prompt` text), sent through the conversation's queue like
   any message. `bypassPermissions` becomes the `bypass` permission mode for a conversation (as `POST /api/sessions`
   does). The Start dialog's agent row becomes Claude / Codex × Conversation / Terminal (remembered per browser).
2. **Ticket tab → Conversation.** The section tab comes alive: a switcher of the ticket's agent sessions (all its
   attempts, newest first, with their state) above the selected one's conversation. A terminal session shows a line
   with Open (its own tab) instead. The session keeps its own tab too (Phase 7 changes that).
3. **Make ticket… from a scratch chat.** A conversation session with no attempt has Make ticket… in its ⋯ menu.
   - `POST /api/sessions/{id}/ticket-draft` → `{title, description}`: a hidden one-turn side request that forks the
     conversation and never changes it.
     - Claude: `claude -p --resume <id> --fork-session --no-session-persistence --tools "" --output-format json
       --json-schema <schema> <prompt>` in the session's folder, with the conversation's program, env and model;
       the answer is `structured_output` (checked 2026-10-19 with 2.1.302: the fork is not saved). Never `--bare`.
     - Codex: `thread/fork {threadId, ephemeral: true, excludeTurns: true, sandbox: read-only, approvalPolicy: never}`, then
       `turn/start` with `outputSchema`; the answer is the last agent message.
     - 409 `nothing_to_draft` before the agent has saved a turn; 504 on timeout (120 s). Nothing retries.
   - The ticket form opens prefilled (Drafting… first; Cancel aborts). With "Start from this chat" (only for a chat
     in a worktree off the base branch) creating it also starts it from the chat:
     `StartTicket.fromSession` adopts the chat's worktree and branch, makes the attempt, sets the session's
     `attempt_id`, and opens the ticket tab just before the chat's tab. No new session, nothing sent to the agent.

4. **PR notices reach conversations** (PDT-44's rule, spec §14), now that Start makes conversations: the newest
   running agent session of the attempt, terminal or conversation. A conversation is told only when idle (no turn,
   request or queued message), its terminal is closed, no rate limit is hit (from its `usage` rate limits) and it has
   been quiet for 20 s; the message shows as `source: notice` ("Sent automatically").

## Guardrails (spec §14)

Make ticket is a click; Start is a click. The side request runs the user's own CLI with the same env scrubbing as the
conversation, no tools, no retry, and saves nothing.

## Tasks

1. pdt-core: `StartTicket.sessionMode`/`fromSession` + validation; `conversation::draft` (prompt, schema,
   `TicketDraft::from_value` trimming and capping to the ticket limits). TDD.
2. pdt-agents: `AgentAdapter::draft(DraftSpec)`; Claude args (pure, tested) + process; Codex fork + turn.
3. pdt-api: the draft route; Start's session mode and `fromSession`; store `set_session_attempt`.
4. fake-agent: Claude `--fork-session` + `--json-schema`, Codex ephemeral fork + `outputSchema`; API tests.
5. Web: Start dialog, ticket Conversation tab, Make ticket dialog, docs (AGENTS.md, spec status).
