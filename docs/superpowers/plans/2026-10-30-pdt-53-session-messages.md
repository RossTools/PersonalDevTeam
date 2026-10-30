# PDT-53: messages between sessions — plan for the first slice

Spec: docs/superpowers/specs/2026-10-30-session-messages-design.md. One PR, after the user agrees the spec (its §11).
The slice is the whole of spec §§3–9: `send_to_session` for every agent session, with sender, record, guardrails and
the guide. Spec §10 stays out.

0. **Check first**: how Claude Code treats an MCP tool left out of `--allowedTools` under `bypassPermissions` (a
   workflow step's mode). If it runs it without asking, `mcp::tools::confirmable` must hold every agent's call with
   `ask_confirm`, not only non-Claude ones (spec §7). Write down what was found in the PR.
1. **Core (TDD)**, pdt-core:
   - `conversation::MessageFrom {session_id, persona, ticket_key, title}` and `AgentEvent::UserMessage.from`
     (optional, `serde(default)`, skipped when none); a test that an event without it, and one with it, round-trip.
   - `session_message::header(from, text) -> String`: the `[PDT] Message from session …` text (spec §6), PM variant.
   - `session_message::limits(sent: &[Sent], now) -> Result<(), Refusal>`: 10 an hour per sender, 6 per pair since the
     user last wrote to either; `MAX_TEXT = 16_000`. Tests for each limit, the reset, and the edges.
   - `ProjectPolicy.agents_message` (default false); `pm::ask_target` for `send_to_session` with `to`.
   - `ToolDecl` for `send_to_session`: drop `.pm()`'s exclusivity: put it in `DEFAULT_TOOLS` (every persona; personas
     that list their tools get it added when they list the default set) and keep `.asks()`.
2. **Store**: migration `crates/pdt-store/migrations/<UTC timestamp>_session_message_from.sql`: `session_messages`
   `from_session TEXT` and an index on `(from_session, created_at)`; project settings `agents_message INTEGER NOT NULL
   DEFAULT 0`. `SessionMessage.from_session`; `messages_between(a, b, since)` and `messages_from(a, since)` for the
   limits; the time the user last wrote to a session (from conversation events with `source` `pdt` and terminal
   activity reports, or a new `last_user_message_at` on sessions if that is simpler: decide in the code).
3. **API**, pdt-api:
   - `send_to_session`: `to` (session id or ticket key; `session` as alias), the checks of spec §7 in order (exists and
     takes messages, not itself, size, untrusted sender always asks, limits, then `confirmable` with the project
     setting), queues with `from_session` and the header; the result names the receiver and the message id.
   - `pm::acts_without_asking`: `agentsMessage` for non-PM senders (both projects when they differ).
   - Outbox: pass `from` to `conversations::send_notice` so the event carries it; on resolve, the receiver's ticket
     gets a `message` event (pdt-core `EventKind::Message`, with its TS type); an `untold` message from a session
     tells the sender when it still runs (not counted, never reported back).
   - Project settings route: `agentsMessage`.
   - Integration tests (`fake-agent`, `notice_quiet` zero): A messages B by session and by ticket key; B's events
     show `from`; the timeline event; refused cases (finished, itself, too long, the pair limit and its reset by a
     user message); an untrusted sender asks even with the setting on; untold tells the sender.
4. **Web**: `MessageList` shows "From <persona> · <ticket>" (linked to the session) for a user message with `from`;
   the ticket timeline renders `message` events; Project settings has the "Agents may message each other without
   asking" toggle. Tests beside each.
5. **Docs**: docs/agent-guide.md "Messaging another session" and the tool table (spec §9); docs/user (sessions: a
   message from another session, the project setting); docs/dev/invariants.md "Talking to agents" (the third case,
   the limits) and architecture.md's outbox paragraph. Changelog fragment `.changes/unreleased/PDT-53.md`
   (`kind: added`).
6. **Checks**: fmt, clippy, `cargo test` (commit `web/src/api/gen`), web typecheck/test/build. Label the PR `windows`
   only if a path or process change turns up (none is expected).
