# Personal Dev Team — Phase 7b: agent steps, pdt-mcp, personas and the Inbox

Status: design for PDT-77, 2026-10-20. Builds §3 (agent steps, human gates), §9 (tools only), §10 (permission
modes), §12 (Personal project) and §13 (pdt-mcp) of the vision
(docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md) on the 7a engine
(docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md), and main spec §6.2–§6.4. The pipeline template and
Board stages (7c), the builder (7d) and the PM persona's conversation (vision §9) are out of scope; their needs shape
this.

## 1. What 7b delivers

- **pdt-mcp**: an MCP endpoint on pdtd (`/api/mcp`, Streamable HTTP, a token per session), whose tools run the same
  domain functions and take the same input types as the API. Every agent session PDT launches gets it per launch
  (never through the user's config).
- **Personas**: versioned agent setups (agent, model, instructions, permission mode, tool set), with built-in
  Refiner, Coder, Reviewer and QA, and a Personas view with version history and diffs.
- **The `agent` node**: runs a persona in a new or reused session with a declared outcome set; `submit_outcome` picks
  the port; max runtime and max turns; stalled handling (Nudge / Take over) and a loop limit.
- **The Inbox**: one list of what waits on the user (questions, approvals, gates, stalled steps, loop limits, ready to
  accept), with a rail item, `Ctrl+] i` and `j`/`k` triage.
- **Personal projects**: projects with no repo whose tickets run in a scratch folder.

Done when a workflow step can start a persona session that answers through `submit_outcome` or `ask_user`, the
answer moves the workflow on, and the user handles questions and gates from the Inbox.

## 2. pdt-mcp

### 2.1 Endpoint and identity

- `POST /api/mcp`: MCP's Streamable HTTP transport, answered with `application/json` (no SSE stream: PDT never
  starts a request towards the agent). `GET` and `DELETE` answer 405. Methods: `initialize` (protocol versions
  `2025-11-25`, `2025-06-18`, `2025-03-26`; the client's if known, else the newest), `notifications/initialized`,
  `ping`, `tools/list`, `tools/call`. No `Mcp-Session-Id`: the token is the identity.
- **Token**: `Authorization: Bearer <session id>.<mac>`, the MAC an HMAC-SHA256 of the session id under a host key
  (`<data dir>/mcp.key`, 32 random bytes made on first use, owner-only). Nothing is stored per session; a token works
  while its session exists and is not `finished` (a stopped conversation that reopens is running again), so it survives pdtd restarts and session
  resumes, and ends with the session. A wrong or ended token is HTTP 401.
- **Context**: each call knows its session, and through it the project, the attempt and ticket (the session's
  attempt, or the agent step's ticket, §4.4), and the agent step waiting on the session, if any.
- Tool errors are MCP tool results with `isError: true` and a sentence the agent can act on ("`approve` is not an
  outcome of this step; use one of: approved, changes_requested"). Malformed requests are JSON-RPC errors.

### 2.2 Tools

Few and coarse (vision §13). Inputs are the API's request types where one exists (the same Rust struct, its JSON
Schema from schemars), and each tool calls the domain function the matching route calls, so the two can't drift.

| Tool | Input | Does (same as) |
|---|---|---|
| `get_ticket` | `ticket?` (key or id; default: this session's) | the ticket (`GET /api/tickets/{id}`) plus its comments, the earlier agent outcomes on it (§4.5), and, for a step session, the step: persona, outcomes allowed, what the step asks |
| `submit_outcome` | `outcome`, `summary`, `comments?` `[{file, line, endLine?, body}]` | ends this session's agent step with that outcome (§4.3); comments become review comments on the attempt's worktree by the persona |
| `ask_user` | `questions` `[{question, choices?}]` | an Inbox question (§6); the answers come back as a message. In a step, the step waits for the user instead of stalling |
| `add_ticket_comment` | `body`, `ticket?` | `POST /api/tickets/{id}/comments`, as the agent (`actor: agent`, with the session and persona) |
| `add_review_comment` | `file`, `line`, `endLine?`, `body` | a review comment on the attempt's worktree (`POST /api/worktrees/{id}/comments`), authored by the persona or `agent`, already sent |
| `catalog_search` | `query?`, `category?` | node kinds matching the words (`GET /api/workflows/catalog`, id, title, category, description only) |
| `catalog_get` | `id` | one kind with its schemas and ports |
| `workflow_get` | `id` or `name` | the workflow and its current document (`GET /api/workflows/{id}`) |
| `workflow_validate` | `doc` | `POST /api/workflows/validate` |
| `workflow_dry_run` | `doc` or `workflowId`, `event` | `POST /api/workflows/dry-run` |
| `workflow_save` | `id?`, `doc`, `note?` | a new version authored `agent` (`PUT`), or a new **disabled** workflow (`POST`); enabling stays the user's |

**Tool sets.** A persona version names its tools (§3). A session without a persona gets the **default set**:
`get_ticket`, `submit_outcome`, `ask_user`, `add_ticket_comment`, `add_review_comment`. `tools/list` returns only the
session's set and `tools/call` refuses the rest. `submit_outcome` outside a step says so ("no workflow step is
waiting for an outcome from this session").

### 2.3 Passing it to the agents

The endpoint is `http://127.0.0.1:<port>/api/mcp` (the server is named `pdt`), added per launch and never written
into the user's config:

| Agent | Conversation | Terminal |
|---|---|---|
| Claude | `--mcp-config <session dir>/mcp.json` (`{"mcpServers": {"pdt": {"type": "http", url, headers}}}`) and `--allowedTools mcp__pdt__<tool>,…` for the session's set, so PDT's tools never prompt | the same two flags |
| Codex | `thread/start` / `thread/resume` `config: {"mcp_servers.pdt": {url, http_headers}}` | `-c mcp_servers.pdt.url=…` and `-c mcp_servers.pdt.bearer_token_env_var="PDT_MCP_TOKEN"` with the token in the PTY's environment |
| Devin | ACP `session/new` / `session/load` `mcpServers: [{type: "http", name: "pdt", url, headers}]` (Devin's ACP offers `mcpCapabilities.http`) | not in 7b: `--config` would have to carry it and Devin keeps MCP servers in a separate file. Devin terminals keep the API through curl |

PDT's instructions (agent_context) gain a short "PDT tools" paragraph naming the server and its tools when the session
has them; the guides stay the knowledge layer (`GET /api/guides/agents` gains a pdt-mcp section).

## 3. Personas

- `personas`: id, unique `name`, `role` (free text: refiner, coder, reviewer, qa, …), `description`, `builtin` (the
  template id), `version` (current), `archived`, timestamps. `persona_versions`: (persona, version) → `agent`,
  `model` (null: the agent's default), `instructions` (Markdown), `permissionMode` (null: not set), `tools` (tool
  names), `author` (`user`, `builtin`, `agent`), `note`, time. Every save is a new version; a session records the
  persona and version it ran with (`sessions.persona_id`, `persona_version`).
- **Built-ins**, installed on start like built-in workflows (an untouched one follows newer templates, an edited one is
  left alone; they can be archived, not deleted):

  | Persona | Agent | Permission | Tools | Instructions (gist) |
  |---|---|---|---|---|
  | Refiner | Claude | plan | default set | turn the ticket into something ready to build: scope, done-when, open questions to `ask_user`; outcomes `ready`, `needs_clarification` |
  | Coder | Claude | (host default) | default set | build the ticket on its branch, run the checks, commit; outcome `submitted` |
  | Reviewer | Claude | plan | default set | review the attempt's diff against the ticket; line comments in `submit_outcome`; `approved`, `changes_requested` |
  | QA | Claude | (host default) | default set | run the project's tests and runs, try the change; `pass`, `fail` |

- **Permission mode** of a persona's session: the project's override (`project_settings.agent_permission_mode`, e.g. a
  strict project that never bypasses), else the persona version's, else the host default (`ticketPermissionMode` for a
  session on a ticket, `sessionPermissionMode` otherwise, PDT-75).
- API: `GET/POST /api/personas`, `GET/PUT/PATCH/DELETE /api/personas/{id}` (PUT `{version fields, note}` saves a
  version; PATCH `{name, role, description, archived}`; DELETE refuses built-ins and personas a workflow names),
  `GET /api/personas/{id}/versions`, `/versions/{v}`, `/diff?from=&to=` (fields changed and a unified diff of the
  instructions).

## 4. The agent step

### 4.1 The `agent` node

| Config | Default | Meaning |
|---|---|---|
| `persona` | — | a persona name or id (references allowed) |
| `prompt` | — | the task, with `{{…}}` references; PDT adds the ticket and the outcome rules (§4.2) |
| `outcomes` | — | `[{id, description}]`; one port each (`dynamicPorts`, so the builder shows them generically) |
| `session` | `new` | `new`: a fresh session for each pass (Reviewer, QA, Refiner). `reuse`: the session this node used earlier in the run, else the attempt's running agent session, else a new one that becomes the attempt's (Coder, main spec §6.2) |
| `ticket` / `attempt` | `{{trigger.ticketId}}` / `{{trigger.attemptId}}` | what the step works on (either may be empty: a step about nothing in particular runs in the project's main checkout) |
| `maxTurns` | 10 | turns the session may end without an outcome, nudges included |
| `maxRuntimeMs` | 4 h | then `timeout` |
| `nudges` | 1 | automatic nudges before the step counts as stalled |
| `loopLimit` | 3 | how many times the node may run again in one run before the Inbox asks (§5.2); 0 = no limit |

Ports: each outcome, `timeout` (max runtime or max turns), `stopped` (the user stopped it at the loop limit), and
every node's `error`. Its output: `{outcome, summary, comments, sessionId, persona, personaVersion, turns, by}`
(`by`: `agent` or `user` when taken over).

### 4.2 Running it

1. Resolve the persona's current version and the session (§4.1 `session`). A new session is a conversation of the
   persona's agent and model, with the permission mode of §3, in the attempt's worktree (else the project's main
   checkout, or the ticket's scratch folder in a Personal project, §7). It records the persona, version and step.
   It gets no tab: the Inbox, the ticket and the run link to it.
2. The session's instructions get a **step section** after PDT's usual context: the persona's instructions, the
   outcomes with their descriptions, and the rule: finish by calling `submit_outcome` with one of them; call
   `ask_user` when only the user can unblock you; ending a turn without either counts as stalled.
3. The first message is the node's `prompt`, followed by the ticket's key and title and the outcome list. A reused
   session gets it as a message through the outbox (7a's delivery rule), so it never interrupts a running turn.
4. The step waits (`wait: {agent: …}`, until `maxRuntimeMs`). The binding (`agent_steps`: step, run, node, session,
   persona and version, project, ticket, attempt, outcomes, `state`, turns, nudges) is what pdt-mcp and the Inbox
   read.

### 4.3 Outcomes

`submit_outcome` from the bound session: an outcome not in the step's list is an error the agent sees; otherwise
the step completes by that port (the engine's `complete_step`), the binding ends (`done`), review comments are
added, and the ticket's timeline gets an `outcome` event (`{outcome, summary, persona, version, workflowId,
workflowName, runId, sessionId}`; mirrored into the log as `agent.outcome`). The session stays as it is: a reused
Coder session keeps its context for the next pass.

### 4.4 Turns, stalled and nudges

- A conversation's turn end (and a terminal's activity going idle, for a reused terminal session) is reported to the
  engine for sessions with a working binding. A turn that ends with no outcome and no open question counts a turn:
  while `nudges` remain, PDT sends a nudge ("[PDT] You ended your turn without `submit_outcome`. Finish the step by
  calling it with one of: …, or `ask_user` if you need the user."); after that the binding is **stalled** and an Inbox
  item offers **Nudge** (the same message, or the user's own text) and **Take over** (the user picks the outcome; the
  step leaves by it with `by: user`).
- `maxTurns` turns without an outcome take `timeout`. A session that ends (stopped, crashed, pdtd restarted while it
  ran) while its step works also counts as stalled, found by the engine tick.

### 4.5 Earlier outcomes

`get_ticket` lists the outcomes earlier agent steps submitted for the ticket (persona, outcome, summary, comments,
time), so a fresh Reviewer or a Coder on its second pass sees what came before (main spec §6.2).

## 5. Gates and limits

### 5.1 Gates in the Inbox

A `gate` node that waits (`ask: wait`) creates an Inbox item with its question and choices. Answering it there, or
through 7a's `POST /api/workflow-runs/{run}/steps/{step}/answer` (7d's runs view), resolves both. Gate config gains
`inbox: gate | accept` (default `gate`): `accept` lists the item under **Ready to accept**, with Open changes beside
its choices (7c's Accept stage uses it).

### 5.2 Loop limit

When an `agent` node is about to run for the `loopLimit + 1`-th time again in one run, it doesn't start the persona:
it waits with an Inbox **loop limit** item showing the earlier passes' outcomes and summaries, and the choices **Once
more** (runs it, and allows one more pass), **Take over** (pick an outcome) and **Stop** (`stopped`). The engine's
`maxVisits` (default 25) still applies to every node as a hard cap.

## 6. The Inbox

- `inbox_items`: id, `kind` (`question`, `gate`, `stalled`, `loop_limit`), `group` (`question`, `gate`, `stalled`,
  `loop_limit`, `accept`), `status` (`open`, `resolved`, `closed`: its step or run went away), project, ticket,
  attempt, session, run, step, persona, `title`, `body` (questions, choices, outcomes, history), `resolution`,
  timestamps. One open item per (step, kind).
- **Approvals** are not stored: the open permission requests of running conversations are listed from their events
  (`approval`, linking the session; answered through `POST /api/sessions/{id}/requests/{request}`).
- `GET /api/inbox?project=` → `{items, counts}`, most urgent group first: Questions, Approvals, Stalled, Loop limit,
  Gates, Ready to accept; oldest first within a group. Each item carries its ticket key and title, persona name and
  the choices it offers.
- `POST /api/inbox/{id}/answer`: `{choice}` for gates, stalled (`nudge`, `outcome:<id>`) and loop limits (`again`,
  `outcome:<id>`, `stop`); `{answers: [..]}` or `{text}` for questions (sent to the session through the outbox as
  "[PDT] The user answered your questions: …"); `text` with `nudge` replaces the nudge message. `POST
  /api/inbox/{id}/dismiss` closes a question or stalled item without acting (the step keeps waiting).
- A run that ends or is cancelled closes its items; an answered gate resolves its item wherever it was answered.

### 6.1 The view (UX spec §8.4)

Rail item **Inbox** with the open count as a badge, `Ctrl+] i`. The list column groups items by kind (each row:
`TicketKey`, title, kind, age, persona); the work area shows the item: questions with a `Composer` each and Send
answers; approvals with the `ApprovalCard`; stalled with Nudge / Take over (then the outcomes) and Open session;
loop limit with the earlier passes and Once more / Take over / Stop; gates and Ready to accept with their choices
(and Open changes). Keys: `j`/`k` move, Enter takes the primary action, `o` opens the ticket, `e` dismisses where it
can; resolving moves to the next item. On a phone the Inbox is in the rail, one item per screen.

## 7. Personal projects

- A project with `kind: personal` has no repo: `POST /api/projects {name, kind: "personal"}` makes its folder
  `<data dir>/personal/<project id>` (its `repoPath`, no git; `baseBranch` empty).
- **Start** on its tickets makes no branch or worktree: the attempt's folder is `<project folder>/<ticket key>`
  (kept across attempts: it is the work's output), the attempt has no worktree and an empty branch, and the session
  runs there. Finishing moves the ticket to Done and stops its sessions; the folder stays.
- Everything that needs git skips or refuses it: merge detection, the PR poller, overlap, `.pdt/workflows`, runs and
  tests (`409 personal_project`), worktrees, pull, Changes and review. The web hides those parts (Start shows no
  branch, the project page no git), and Add project offers "Personal (no repo)".

## 8. API summary

- `POST /api/mcp`
- `GET/POST /api/personas`, `GET/PUT/PATCH/DELETE /api/personas/{id}`, `GET /api/personas/{id}/versions`,
  `/versions/{v}`, `/diff`
- `GET /api/inbox`, `POST /api/inbox/{id}/answer`, `POST /api/inbox/{id}/dismiss`
- `GET /api/workflow-runs/{id}` steps gain their agent binding; the catalog gains `agent`
- Projects: `kind`; project settings: `agentPermissionMode`; sessions: `personaId`, `personaVersion`, `stepId`

## 9. Code layout

- pdt-core `persona.rs` (types, validation, built-in templates, diff), `inbox.rs` (item types, grouping),
  `mcp.rs` (JSON-RPC and MCP message types), `workflow/agent.rs` (the agent node's config and step texts);
  `agent_context` gains the tools paragraph and step section.
- pdt-store migration `0023_agent_steps.sql`: `personas`, `persona_versions`, `agent_steps`, `inbox_items`,
  `sessions.persona_id/persona_version/step_id`, `projects.kind`, `project_settings.agent_permission_mode`;
  `personas.rs`, `inbox.rs`.
- pdt-api `mcp/` (`mod.rs` transport and auth, `tools.rs` the registry), `personas.rs`, `inbox.rs`,
  `workflow/nodes/agent.rs`, `workflow/agent.rs` (bindings, turn ends, nudges, stalls, loop limits);
  pdt-agents: `OpenSpec.mcp` and the per-agent flags.
- web: `inbox/` (view, rail item, keys), `personas/` (list, form, versions and diff), Personal project in Add
  project, Start and the project page.

## 10. Out of scope

The pipeline template and Board stages (7c), the PM's conversation and memory (vision §9), board/session/connection
tools for the PM, MCP for agents outside PDT (a user token), pdt-mcp in Devin terminals, restricting an agent's own
built-in tools per persona.
