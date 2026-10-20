# Personal Dev Team — Workflows, automation and the PM: Vision

Status: direction agreed with the user, 2026-10-20. This is the long-term shape, not a build spec: each phase below
gets its own spec and plan. Amends the main spec §6 (docs/superpowers/specs/2026-10-14-personal-dev-team-design.md:
the fixed `.pdt/workflow.toml` stage machine becomes the first template on a general engine) and re-scopes Phase 7
(PDT-4).

## 1. Purpose

- **Nothing is missed.** CI results, PR comments, merge-queue drops, new issues, a push to main: PDT notices them, in
  code, every time. An agent is never the one who has to notice.
- **Workflows are configured, not hard-coded.** n8n-style: triggers, conditions, actions, agent steps and human gates,
  wired into workflows the user (or an agent helping the user) builds, edits and sees as a graph or as a list.
- **One PDT, several ways of working.** The personal flow (agents code, review and merge) and a strict work flow (agents
  never merge, external reviewers, required approvals) are the same engine with different workflows and policies.
- **It replaces other automation.** The trigger and action catalog grows until PDT can take over the jobs Hermes does
  today (§12), most of which have nothing to do with a code project. Adding a trigger or an action has to be cheap (§4).
- **A project manager to talk to.** A top-level PM agent handles triage, ticket drafting and "what's going on?" across
  every project (§9).

## 2. Principles

1. **PDT is the control plane; agents are workers.** pdtd observes, decides transitions and keeps state. Agents do the
   work inside a step and report back.
2. **Transitions come from typed outcomes, never from agent text.** An agent step ends with `submit_outcome(...)` from
   the outcomes that step allows; anything else is an error the agent sees. A step that ends without one is *stalled*
   (main spec §6.3).
3. **Events are durable and idempotent.** Every observation is an event in an append-only log with a dedupe key (as
   `pr` events have today). Reactions run at least once and dedupe; a workflow run's state is in SQLite, so a pdtd
   restart or update resumes it where it was.
4. **Policy is configuration.** "Agents may merge", "auto-merge when green", "a person must approve", loop limits,
   concurrency caps: per-project settings and workflow choices, not branches in code.
5. **Consent is explicit and on record.** Enabling a workflow is the user's consent to what it does. Every automated
   message, ticket change or merge records the workflow and run that did it, and shows it ("from workflow *CI
   failed → tell coder*"). The current invariant ("nothing is sent to an agent without the user's click, PR notices
   aside") becomes "… without the user's click or a workflow the user enabled" when the engine ships.
6. **The UI reads state; workflows change state.** A workflow never drives the UI. It changes tickets, attempts,
   sessions and inbox items through the same domain commands the UI and API use, and the UI shows the result. A step
   that needs the user is a **human gate** (§3), which the UI renders as an inbox item, banner or approval card.
7. **Same commands for everyone.** A workflow action, an API call, an MCP tool call by an agent and a button in the UI
   run the same domain command with the same checks (one active attempt per ticket, parents never started, …).
   Workflows get no back door.
8. **Deterministic first, agents where judgement is needed.** Routing, filtering, waiting and retrying are engine nodes.
   An agent step is used for work that needs judgement (triage, refine, code, review), and its output is typed.

## 3. Architecture

```
 sensors (code)          event log (SQLite)        workflow engine              domain commands
 ───────────────         ──────────────────        ───────────────              ───────────────
 PR / CI poller   ─┐                              ┌ match triggers              move ticket, start attempt,
 merge check      ─┤                              │ durable runs, step state    send to session, create inbox
 overlap refresh  ─┼──►  typed, keyed events  ──► │ conditions, waits, timers ─►item, comment on PR, merge, …
 GitHub issues    ─┤      (pr.ci_failed,          │ agent steps (typed outcome)        │
 schedule / cron  ─┤       attempt.merged, …)     │ human gates                        ▼
 webhook / HTTP   ─┤                              └ loop limits, retries          state (tickets, attempts,
 session activity ─┘                                                              sessions, inbox) ──► UI
```

- **Sensors** stay in Rust: polling intervals, GraphQL queries and git checks are not something to draw. Their change
  is that they emit events instead of acting. Today's PR poller already computes "news"; it becomes the event source.
- **Event log**: one table, typed payloads, dedupe keys, retention. The ticket timeline and the session events become
  views over it where that fits (decided in the engine spec).
- **Workflow runs**: one row per run, one per step execution (input, output, status, attempt count, error), so a run
  is inspectable and replayable in the UI.
- **Node kinds** (each has a catalog entry, §4):
  - *Trigger*: an event type with a filter, a schedule, a webhook, or manual ("Run now").
  - *Condition / switch*: branch on event or step data, project policy or ticket fields.
  - *Action*: a domain command or an external call.
  - *Agent step*: runs a persona in a session (new or existing, per the session rules of main spec §6.2) with a
    declared outcome set; its outcome selects the next edge.
  - *Human gate*: creates an inbox item with the choices it allows and waits; the choice selects the next edge.
  - *Wait / timer*: until an event, a time, or a condition becomes true ("until CI finishes", "20 s idle").
  - *Loop limit, retry, error handler*: per edge or per node.
  - *Sub-workflow*: call another workflow (how templates compose).
- **Engine features every workflow gets** (the patterns Hermes's jobs already rely on, §12):
  - *Run keys*: a trigger can set an idempotency key (`feed:ai-updates:weekly:<date>`); a second run with the same key
    is skipped, so a retried or doubled schedule never makes two of anything.
  - *Workflow state*: a small key-value store per workflow for baselines and "last seen" values ("issue numbers already
    posted", "watchdog was healthy"). A first run can record a baseline silently. Watchers then need no state files of
    their own.
  - *Quiet by default*: a step can produce "nothing to report", and the run ends without notifying anyone. Runs still
    show in the runs view.
  - *Limits*: max runtime and max turns on agent steps, timeouts on actions, retry with backoff, and a failure path
    (notify, inbox item) instead of a silent error.

## 4. Extensibility: the node catalog

Expect many triggers and actions. Each is a self-describing entry in a catalog, so adding one doesn't touch the
engine, the builder or the agent tools:

- **One registry.** Every node kind registers an id (`github.pr.ci_failed`, `ticket.move`, `session.send`), a version,
  a title and description, a config schema, and typed input and output schemas. Built-in kinds implement one Rust
  trait in their own module.
- **Everything is generated from the schema.** The builder's config forms, validation, the data picker ("use
  `trigger.pr.number` here"), the TS types (ts-rs, as today) and the agent-facing catalog all come from it. A new kind
  is one module plus a test.
- **Discoverable.** `GET /api/workflows/catalog` (and a pdt-mcp tool) lists every kind with its schemas, so an agent can
  build a valid workflow without reading code.
- **Versioned kinds.** A kind's config schema can change; saved workflows record the version they were written
  against and are migrated or flagged, never silently broken.
- **Extension points beyond built-ins**, in roughly this order:
  1. *Command* action: run a program with JSON on stdin, JSON on stdout (in a worktree or a scratch dir), with a
     timeout.
  2. *HTTP request* action and *webhook* trigger (webhooks only on the local or Tailscale address, with a secret).
  3. *MCP tools as actions*: any MCP server the user configures in PDT exposes its tools as action kinds, schemas
     included. This is the widest door: Slack, Linear, Jira and so on without PDT code.
  4. *Plugin node kinds*: an external process that registers its own triggers and actions over a small JSON-RPC
     protocol. Only if 1–3 leave real gaps.
- **Connections.** An external service is a named connection: base URL plus credentials in the secret store
  (Radarr, Sonarr, Plex, a Discord webhook, Google). Workflows reference the connection by name, never a URL or key
  inline. The same connection is offered to agent sessions as tools (§9), so a workflow and the PM use one setup.
- **Outputs are action kinds too.** Notify (PDT notification, PDT-8), post to Discord, create a calendar event, send
  an email, append to a feed archive. A workflow's delivery is just its last step, swappable per workflow.

## 5. Workflow definitions

- **Data, not code.** A workflow is a versioned document: nodes, edges, node configs, layout (positions for the
  graph) and metadata. Every save is a new version with a diff, as personas have (main spec §6.4); a run records the
  version it ran.
- **Where they live.** Host-first: stored by pdtd per project (or host-wide), because work repos may not allow a
  `.pdt/` folder. A project can also keep workflows in `.pdt/workflows/` in the repo; PDT shows where each came from.
  (The repo format must keep to the `.pdt/project.toml` compatibility rule in AGENTS.md.)
- **Templates.** Built-in templates ship with PDT (the pipeline, PR/CI reactions, issue triage). The user saves their
  own as templates, with parameters ("which persona reviews", "auto-merge yes/no"). Instantiating a template makes an
  ordinary workflow the user can edit.
- **Scope.** A workflow is enabled per project, host-wide with a project filter, or **personal**: not about any code
  project (a weekly brief, a media request, a package tracker). Personal workflows still create tracked work items
  and runs (§12 "Agent tasks without a repo").
- **Test before enabling.** Validate (schemas, unreachable nodes, outcomes with no edge), dry-run against a recorded or
  hand-written event (no side effects, shows what each node would do), then enable.

## 6. The builder: graph view and non-graph view

Both are writable and both are in scope. They edit the same document, so a change in one shows in the other.

- **Graph view** (n8n-style; React Flow / xyflow, wrapped in web/ui): add nodes from the catalog, drag edges between
  ports (an agent step or gate has one port per outcome), configure a node in a side panel built from its schema, see
  validation errors on the node. In a run, the same graph shows the path taken, each step's status, input and output.
- **Non-graph view**: an outline of the workflow as ordered steps with nested branches (closer to GitHub Actions or
  Zapier). Faster for linear workflows, keyboard-friendly, and usable on a phone where a graph isn't.
- **Text view**: the document as TOML/JSON with validation, for bulk edits and diffs.
- **Agent editing**: an agent (usually the PM) creates and edits workflows through the API/MCP with the same validation
  and dry-run; its changes arrive as a new version the user reviews (with a diff) and enables.
- **Runs view**: every run with its trigger, status and steps; failed runs can be retried from the failed step.

## 7. What moves onto the engine

Reactions move; sensors and core rules don't.

| Today (hard-coded) | Becomes |
|---|---|
| PR notices: CI failed, conflict, dropped from the merge queue, merged → typed into the attempt's session | Built-in template "PR reactions" (`pr.*` triggers → `session.send` with the same idle-and-20 s delivery rule) |
| Merged attempt → "Move to Done and remove the worktree" banner | `attempt.merged` → human gate (rendered as that banner) → finish (stop sessions, remove worktree, pull base) |
| PDT-24: main moved while a PR waits | `base.pushed` trigger → update or re-queue action, per policy |
| Phase 7 pipeline (Refine → Code → Review → QA → Accept) | Built-in template "Pipeline" (§8) |
| Merge detection, PR/CI polling, overlap, one attempt per ticket | Stay in code (sensors and domain rules) |

The delivery rule for messages into sessions (idle, nobody typing, never during a rate limit, terminal vs.
conversation) becomes the `session.send` action, shared with PDT-53's cross-session messages.

## 8. Phase 7 on the engine (re-scope of PDT-4)

Phase 7 builds the engine core and expresses the pipeline as its first template, instead of a fixed stage machine
that would be rebuilt later. Roughly:

- **7a — Events and engine core:** event log, sensors emitting events, workflow documents and versions, durable runs,
  the node kinds of §3, the catalog (§4) with the built-in triggers and actions the PR reactions and pipeline need.
  Port the PR reactions and the merged banner.
- **7b — Agent steps:** pdt-mcp (§13; `get_ticket`, `submit_outcome`, `ask_user`, comments), personas and their versions,
  stalled handling, loop limits, the Inbox (gates, questions, stalled, loop limit).
- **7c — Pipeline template:** the main spec §6 stages as a workflow; the Board splits In progress into its stages;
  scheduling and concurrency caps.
- **7d — Builder:** graph view, non-graph view, text view, runs view, dry-run, templates.
- **7e — More of the catalog:** command, HTTP and webhook, GitHub issues and PR comments, schedule; then MCP tools as
  actions.

7d can start once 7a's document format is fixed; it doesn't wait for 7b/7c. Each part gets its own spec and plan.

## 9. The PM persona

A host-level agent the user talks to: triage, drafting tickets, "what's in flight?", "why is PDT-52 stuck?",
helping write workflows.

- **Top level.** Its own rail item and conversation, not tied to a project or worktree; it works across every
  project, with a project filter when asked.
- **Tools** (pdt-mcp, §13; the PM's tool set, and no shell): search, read, draft and edit tickets; the board, inbox and attempts; sessions and their state;
  PRs and CI; send a message to a session (PDT-53); the workflow catalog, documents, dry-run and runs.
- **Asks before acting.** It drafts tickets and workflow changes and asks before creating or enabling them, by
  default. That is a policy the user can loosen per project.
- **Also a workflow step.** Triage is an agent step with the PM persona: `issue.opened → PM triage → outcome: draft
  ticket | ignore | ask user`.
- **Sessions are disposable; memory belongs to PDT.**
  - The database is the main memory: tickets, events, runs. The PM queries it rather than keeping copies.
  - Every new PM session starts with a **briefing built by pdtd, not by the model**: board and inbox state, attempts in
    flight, events since the last session, and the index of pinned notes. Cheap, current and never drifts.
  - A small **notes store** (per project and host-wide) holds what the database can't know: preferences, decisions,
    people and rules at work. Searched through a tool (SQLite FTS5), not loaded whole.
  - **Rotation:** a new session on a context threshold or daily; the ending session writes a short journal entry that
    the next briefing includes. Old sessions stay readable.
  - None of this edits the agent's own memory or config files (AGENTS.md invariant).

## 10. Policies

Per project, read by workflows and enforced by the domain commands (so a workflow can't route around them):

- who may merge (agents, the user only, nobody: external), auto-merge;
- required gates (a person approves before merge, before start);
- loop limit, running-attempt cap, host concurrency cap;
- what the PM may do without asking.

A strict work project might be: agents never merge, PR review by external reviewers is a `pr.review_approved` wait,
and the user's Accept is a gate.

## 11. Open items (see also §12)

- **Event log vs. existing tables:** whether ticket events, `pr` events and session events move into one log or stay
  separate with the log beside them (engine spec).
- **External events without a public address:** polling is the default; webhooks only via the local or Tailscale
  address. Is that enough for work (Jira, Slack)?
- **Phones:** the non-graph view and the Inbox are the phone UI for workflows; the graph view may be read-only there.
- **Ticket changes:** rewrite PDT-4 to §8 and add tickets for 7a–7e and the PM, after the user agrees the list.

## 12. Replacing Hermes

Hermes runs the user's personal automation today; its development use stopped when PDT started. Its jobs are almost
all "a script on a schedule, silent unless there is news, sometimes handing a task to an agent persona". That is the
engine's shape, so each kind of job maps to a template:

| Kind of Hermes job | Schedule | As a PDT workflow |
|---|---|---|
| New GitHub issues on a repo → Discord; the first run records a silent baseline | every few minutes | `github.issue.opened` trigger (repo filter; the sensor keeps the baseline in workflow state) → Discord post. Later: → PM triage → draft ticket |
| Briefs written by an agent (news and updates on a topic, with a lookback window) | weekly or daily | schedule (run key per date) → agent task (researcher persona, max runtime and turns; reads the brief's archive to avoid repeats) → post to the brief's Discord channel + archive |
| Follow-ups on a brief's output (filter its items against a watchlist, dedupe, add to a calendar) | daily | schedule → command (the existing script), or condition + `calendar.upsert_event` → notify only on changes |
| Email processing (match and link related emails, post a summary, archive) | every 15 min | schedule → command, or email search + match + Discord post + email archive kinds |
| A daily review of a code project's recent changes by an architect persona | daily | schedule (run key per day) → agent task in that project's checkout → PM inbox, or a draft ticket when it recommends something |
| A health watchdog for a local service | every minute | schedule → command → workflow state "healthy?" → Discord only on a change (failure or recovery) |
| Syncing sessions into an external memory store | hourly | Not ported: the PM's memory (§9) is PDT's own. Possibly an export action later |

The **media skills** (Radarr, Sonarr, Plex on the local network) are not workflows: they are what the PM can do when
asked ("add this film", "what's downloading?"). In PDT they are connections (§4) with action kinds
(`radarr.search`, `radarr.add`, `sonarr.queue`, …), which also become PM tools. A workflow can use them too
("when a show's new season is announced, add it").

What this adds to the plan:

- **Agent tasks without a repo go in a Personal project** (decided 2026-10-20): a project with no repo, whose tickets
  run in a scratch folder. A brief or a triage run is then an ordinary ticket with its attempts, events and output, so
  the board, the timeline and the PM's queries work unchanged. Start makes no branch or worktree for such a project;
  the details go in the 7a/7b specs.
- **First catalog for 7e**, from the table: schedule (cron and interval), `github.issue.opened`, command, HTTP,
  Discord post (webhook first), Google Calendar upsert, email search/archive, brief archive; then Radarr/Sonarr/Plex
  connections.
- **Moving over** one job at a time, Hermes's running alongside until the PDT workflow has run cleanly for a while.
  The existing scripts can run unchanged as command steps on day one; replacing them with native kinds is optional.
- **Talking to the PM over Discord** (as the user does with Hermes) is wanted eventually (decided 2026-10-20). Until
  then PDT's UI over Tailscale is the way in from a phone (PDT-7). Discord would be a channel for the PM's
  conversation plus a Discord trigger, so it fits the same catalog.

## 13. How agents act on PDT: MCP over the API, guides for knowledge

Decided 2026-10-20: agents act on PDT through **pdt-mcp**, a thin layer over the HTTP API. The guides served at
`/api/guides` stay the way agents learn how PDT works. A skill that wraps the API (curl through a shell) is not the
main path.

Why MCP for actions:

- **Identity.** Each session gets its own endpoint token, so `submit_outcome` and `ask_user` know which session,
  attempt and ticket they belong to. With curl the agent would pass its own session id, and a mistake reaches the
  wrong ticket.
- **No shell needed.** The PM, Reviewer and triage personas can run with PDT tools only and no Bash. "May comment and
  submit an outcome, nothing else" is enforceable with tools and close to impossible with a shell. This matters most
  for strict projects (§10).
- **Per-tool permissions.** The agent's own permission system can allow `get_ticket` and ask before `merge`. With
  curl everything is one Bash permission.
- **Typed at the boundary.** Arguments are checked against the tool's schema and errors come back typed, which fits
  principle 2.
- **Every agent.** Claude, Codex and Devin all speak MCP, and its config is passed per launch (`--mcp-config` and the
  Codex/Devin equivalents), so the user's agent config is never edited. Skills are agent-specific and need installing
  where the agent looks for them.
- **Readable conversations.** Tool calls show in the conversation view as named calls with arguments, not curl output.

Rules:

- **The HTTP API is the one source of truth.** pdt-mcp tools are generated from the same handlers and schemas as the
  API (as the TS types are), so the two can't drift. Anything a tool does, the API does too.
- **Served by pdtd** as an HTTP MCP endpoint (`/api/mcp`, token per session), not a process per session: nothing to
  spawn or supervise, and fewer Windows process issues. Tokens end with the session.
- **Few, coarse tools.** Every tool's schema costs context on every turn. The node catalog is searched
  (`catalog_search`, `catalog_get`), never one tool per node kind; workflows are edited as documents
  (`workflow_get`, `workflow_validate`, `workflow_dry_run`, `workflow_save`), not node by node.
- **Tool sets per persona.** A persona version (main spec §6.4) names its tool set: the Coder gets
  outcome/ask/comment tools, the PM gets ticket, board, session, connection and workflow tools, a triage step gets
  read tools and `submit_outcome`. The engine only offers a step the tools its persona allows.
- **Guides stay the knowledge layer.** They explain PDT and point at the tools. Coding agents that already have a
  shell (a session in a worktree) can keep using the API with curl, as they do today.
- **Later:** the same endpoint with a user token lets agents outside PDT (Claude Desktop, say) use PDT, with the
  owner-only remote rules of docs/reference/host.md.
