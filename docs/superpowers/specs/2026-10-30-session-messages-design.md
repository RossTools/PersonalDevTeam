# Personal Dev Team — Messages between sessions

Status: design for PDT-53, 2026-10-30, for the user to agree. Generalises the PM's `send_to_session` (PDT-81) and the
workflow outbox's `session.send` (Phase 7a spec §5.2, PDT-129, PDT-144) into one way for an agent session, the user or
PDT to message another session, with a sender, a record and limits. Plan for the first slice:
docs/superpowers/plans/2026-10-30-pdt-53-session-messages.md.

## 1. Why, and what is already there

The ticket started from a workaround: an agent cleaning up the board had to tell another session that its PR failed in
the merge queue, so it attached to `/api/sessions/{id}/pty` and typed a bracketed paste and Enter by hand. That works
for terminals only, has no sender or record, ignores whether the target is busy or being typed in, and an attach that
sends a resize changes the user's real PTY.

Since the ticket was written PDT has built most of the plumbing:

- **The outbox** (pdt-api `workflow/outbox.rs`, table `session_messages`): messages for a session or for an attempt's
  newest running agent session wait until a terminal is idle with nobody typing for 20 s, or a conversation is idle
  with no open request, no queued message, no rate limit and its terminal closed. A session's waiting messages go in
  one message. A conversation's message is `told` only once the agent echoes it; one that is lost, or waits 30 minutes
  on a session that can't take it, ends `untold` and the user gets a `session.undelivered` notification. PDT's own
  messages wait while the host's cap on agents running a turn is reached.
- **Workflows** send through it with `session.send` (PR and CI notices, ticket comments, agent steps' next pass).
- **The PM** sends through it with the pdt-mcp tool `send_to_session` (`{session, text}`), which asks the user first
  unless the project lets the PM act, refuses a finished or non-agent session and itself, and prefixes the text
  `[PDT] From the project manager:`. Only the PM persona has it.
- **Provenance in conversations**: a user message has a `source` (`pdt`, `terminal`, `steer`, `notice`); the view says
  "Sent automatically: news about the pull request" under `notice` ones.
- **Discovery**: `get_ticket` (any ticket, by key) lists the ticket's sessions with their persona and status; the PM
  also has `get_overview` and `get_session`.

What is missing is the sender: a session other than the PM can't message anyone, a message doesn't say which session
sent it (only its text does, for the PM), nothing limits two agents messaging each other in a loop, and the guide
doesn't tell agents what to use instead of the PTY socket.

## 2. What it delivers

- Every agent session PDT starts can message another session with pdt-mcp's `send_to_session`, addressed by session id
  or by ticket key, through the outbox, with its sender recorded and shown.
- The receiving agent and the user see who sent it: "From Coder · PDT-52" in the conversation, and the text the agent
  reads starts with a header naming the sender and how to reply.
- Each message is on record: the outbox row says who sent it and whether it was told, and the receiver's ticket gets a
  timeline event.
- Guardrails: the user's yes or the project's setting, limits on loops and volume, a size cap, no messages from
  sessions that read outsiders' input without the user's yes.
- The agents guide documents it and says never to type into a session through its PTY.

Done when an agent in one ticket's session can tell another ticket's agent something, the receiver sees and answers it
the same way, the user can see both messages and who sent them, and two agents can't keep each other busy without the
user.

## 3. Who can send

| Sender | How | Recorded as |
| --- | --- | --- |
| An agent session PDT started | pdt-mcp `send_to_session` (it knows the calling session) | `{kind: "session", sessionId}` |
| A workflow (PR notices, comments, steps) | `session.send`, unchanged | the run and step, as today |
| The user | the session's own composer or terminal, a ticket comment (the built-in workflow tells the agent), an Inbox answer | the user, as today |

Decisions:

- **Agents send through pdt-mcp only, not a REST route.** The MCP server knows which session calls it; a REST caller
  on localhost could claim to be any session. The ticket's `POST /api/sessions/{id}/notes` is not needed: every
  sender above already has its path. A session PDT did not start (no pdt-mcp) can't send; it can comment on a ticket
  or ask the user.
- **No "Send to session…" in the UI for now.** The user already reaches any session through its composer, its
  terminal, or its ticket's comments. A forward action ("send this message to another session") can follow if the
  user misses it.
- **The PM keeps its tool**, with the same name and arguments; it becomes the general tool, and the PM's header is the
  general one with the PM as sender.

## 4. Addressing and discovery

`send_to_session` takes `to`, which is one of:

- a **session id**: that session;
- a **ticket key** (`PDT-52`): the ticket's active attempt's newest running agent session, as notices are addressed
  (the outbox's attempt addressing). A ticket with no active attempt is refused at once ("PDT-52 has no work in
  progress").

The old `session` field stays as an alias of `to` for the PM's prompts. Branches and PR numbers are not addresses:
`search_tickets`/`get_ticket` find the ticket for them, and a branch is one ticket's attempt anyway.

Discovery stays with the tools a session has: `get_ticket` on any key lists the ticket's sessions (id, persona,
status), and its result already names the attempt's branch and PR. The PM keeps `get_overview` and `get_session`; other
personas don't get them (they show every project).

## 5. Delivery

Exactly the outbox's rules (§1), with these choices:

- **Queue, never steer.** A message waits for the receiver's turn to end; it never joins a running turn. An agent that
  needs to stop another one can't (as for the PM today); it asks the user.
- **Rate limits and the host's cap** hold agents' messages, as they hold PDT's own.
- **Finished sessions are refused** when sent to (as `send_to_session` does today). **Parked, interrupted or
  stopped-but-not-finished conversations are not reopened** by an agent's message (`reopens: false`): only the user's
  messages and the workflow cases of PDT-144 reopen. The message waits; after the outbox deadline it ends `untold`,
  with the usual notification to the user.
- **Addressed by ticket**, the receiver is picked at delivery, not when sending: a step's next session gets it if the
  one that was running ended.
- **Terminals** get the text typed in by the outbox (bracketed paste and Enter, after the quiet time); conversations
  get it as a user message with `source: notice` and the sender in `from` (§6).
- **Batching**: waiting messages from several senders still go in one message; each keeps its own header.

## 6. Provenance

The text the agent reads starts with a header PDT writes (pdt-core, so it is tested):

```
[PDT] Message from session 01a1… (Coder · PDT-52 "Fix the board's drag"). Not from the user.
Reply with send_to_session {"to": "01a1…"} if it asks for an answer.

<text>
```

A PM sender reads "from the project manager (session 01a1…)". The `[PDT]` prefix keeps agents' existing rule (messages
that start with `[PDT]` come from PDT, not the user).

In a conversation the event carries the sender as data, beside the text: `AgentEvent::UserMessage` gains an optional
`from: Option<MessageFrom>` (`{sessionId, persona?, ticketKey?, title?}`), `#[serde(default)]` and skipped when empty,
like `pdt`: an older pdtd that a rollback brings back reads the events, and an older web client ignores it. No new
`MessageSource` variant for the same reason (an unknown variant would fail to parse after a rollback). The view shows
"From Coder · PDT-52" under the message, linking to the sender's session, instead of the notices' line.

Records:

- `session_messages` gains `from_session` (the sending session's id; null for workflows and the user). The outbox
  already records the receiver and `told`/`untold`.
- When the receiver works on a ticket, its timeline gets a `message` event (`by` the sender's persona, with
  `sessionId` and `fromTicket` as agents' comments have, and the text, the receiver and whether it was told), written
  when the message is resolved, as notices write their `pr` event. The sender's own ticket is not written to (its
  conversation already shows the tool call).
- An `untold` message also tells the sender, when it is still running, with a short `[PDT]` message ("Your message to
  … was not delivered: …"); the user's notification is unchanged. This status message is PDT's, not the session's, so
  it is never counted against the limits and never answered with a delivery report.

## 7. Guardrails

**The user's say.** Invariant "Nothing is sent to an agent without the user's click or a workflow the user enabled"
gets a third case: a session's message the user allowed. `send_to_session` stays an asking tool (`ToolDecl::asks`):

- by default each call asks, as the PM's do today (Claude's permission prompt, PDT's approval card for other agents);
- a project setting, **"Agents may message each other without asking"** (`agentsMessage`, off by default), lets
  calls whose receiver is in that project go through, as `pmActs` does for the PM. A message across projects asks
  unless both projects allow it.
- A workflow step's session in a permission mode that skips Claude's prompts (`bypassPermissions`) must still ask:
  the plan's first task checks how Claude treats a tool that isn't in `--allowedTools` in that mode; if it doesn't ask,
  PDT holds the call with its own approval card (`ask_confirm`) for every agent, not only non-Claude ones.

**Untrusted input.** A session that reads input from outside PDT (triage of outsiders' issues, a ticket that came
from outside and is held back: `untrusted::triaging`/`held_back`, PDT-201) always asks, whatever the project setting:
an outsider's text could otherwise steer one of the user's other agents. Messages from workflows keep PDT-202's rule
(an outsider's comment is never sent into a session).

**Loops and volume** (a pure rule in pdt-core, counted from `session_messages`):

- per sender, at most 10 messages an hour;
- per pair of sessions (either direction), at most 6 messages since the user last wrote to either of them, so two
  agents answering each other stop after three rounds and the user decides whether they go on (writing to either
  session resets it);
- refused calls say why, so the agent can ask the user instead (`ask_user`).

**Size.** At most 16 000 characters of text (a long log belongs in a ticket comment or a file the receiver can read).

**Not to itself**, nor to a session that isn't an agent session (as today).

**Remote callers.** Nothing new is reachable remotely: the tool is on pdt-mcp, which only PDT's sessions call, and the
identity guard already covers every route.

## 8. Replies

One-way messages with an address to reply to; no threads, no separate mailbox. The receiver answers with the same tool
to the session the header names, under the same limits. The user's Inbox stays the user's (questions, approvals,
undelivered messages); agents' messages to each other are not Inbox items.

## 9. The agents guide

docs/agent-guide.md (served at `/api/guides/agents`) gets a section "Messaging another session": find the session with
`get_ticket`, send with `send_to_session` by ticket key or session id, expect it to wait for the receiver to be idle,
answer messages headed `[PDT] Message from session …` the same way, and never attach to `/api/sessions/{id}/pty` or
type into another session. The tool table loses "the PM's" for `send_to_session`.

## 10. Later, not in this design

- A UI action to forward a message or a selection to another session.
- Branch and PR as addresses; a `find_sessions` tool for personas other than the PM.
- Steering a busy session, or stopping it, from another agent.
- Threads (reply-to ids) and a view of all messages between sessions.
- Refusing `/pty` attaches that don't come from PDT's web client (the workaround's other half).

## 11. Decisions for the user

1. Agents' messages ask by default, with a per-project setting to let them through (§7) — or should they go through by
   default, as an exception like PR notices?
2. Agents send only through pdt-mcp; no REST route, no UI "Send to session…" for now (§3).
3. An agent's message never reopens a parked or stopped conversation; it waits and ends `untold` (§5).
4. The limits: 10 an hour per sender, 6 per pair between user messages, 16 000 characters (§7).
