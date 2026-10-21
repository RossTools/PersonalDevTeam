# Workflows, events, the engine, the builder, agent steps and integrations (Phases 7a, 7b, 7d, 7e)

Specs: docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md, 2026-10-20-phase-7d-builder-design.md and, for
agent steps, personas, pdt-mcp and the Inbox, 2026-10-20-phase-7b-agent-steps-design.md; for schedules, webhooks,
GitHub issues and PR comments, command, HTTP, Discord and notify, 2026-10-20-phase-7e-integrations-design.md (vision:
2026-10-20-workflows-and-pm-vision.md).
Code: pdt-core `workflow/` (documents, references, conditions, validation, diffs, event types, templates, API
records, what reaches an agent, agent steps), `persona.rs`, `inbox.rs`; pdt-store `event_log.rs`, `workflows.rs`,
`personas.rs` and `inbox.rs` (migrations 0021, 0022, 0023); pdt-api `workflow/` (catalog, engine, nodes, outbox,
built-ins, templates, repo files, routes, agent steps), `personas.rs`, `inbox.rs`, `finish.rs` (`ticket.finish`),
`github_issues.rs` (the issues sensor and its trigger) and `hooks.rs` (incoming webhooks);
web `src/workflows/`, `src/inbox/`, `src/personas/` and `web/ui/FlowGraph.tsx`.

## Event log

- Table `events`: `type`, `key` (unique per type: a second event with the same key is dropped), project / ticket /
  attempt / session ids, `source`, JSON `payload`, `processed_at` (matched by the engine). Kept 30 days.
- Writers: the PR poller (`github.rs` `news_events`: `ci.run_failed`, `ci.run_passed`, `pr.conflict`,
  `pr.queue_left`, `pr.merged`, `pr.behind`; key `<attempt>:<notice key>`), and the ticket timeline: the store's
  `insert_event` mirrors `moved`, `comment`, `started`, `merged`, `abandoned` and `overlap` into
  `ticket.stage_changed`, `ticket.commented`, `attempt.started`, `attempt.merged` (key `<attempt>:merged`, with `via`
  `detected` / `merged_here` / `closed`, `pr`, `finishOnMerge`, `finishDeleteBranch` and the merged `notice`),
  `attempt.abandoned` and `ticket.overlap_found`, in the same transaction; an agent step's `outcome` timeline event
  becomes `agent.outcome` (`outcome`, `summary`, `persona`, `personaVersion`, `by`, `sessionId`, `runId`). Phase 7e
  adds the PR poller's `pr.commented` and `pr.reviewed`, the issues sensor's `github.issue.opened`, the scheduler's
  `schedule.fired`, webhooks' `webhook.received` and `notify`'s `pdt.notified` ("Sensors and webhooks"). Payload types: pdt-core
  `workflow/events.rs` (`EVENT_TYPES`). CI events carry `provider` (`github_actions`).
- The timeline and the conversation events stay as they were; a `pr` timeline event is now the record of a message a
  workflow delivered (`told`, `sessionId`, plus `workflowId`, `workflowName`, `runId`).
- `GET /api/events?type=&project=&ticket=&attempt=&limit=` (newest first) or `&after=<seq>` (oldest first, to
  follow); `GET /api/events/{id}`.

## Workflows

- A document (`WorkflowDoc`): `name`, `description`, `nodes` (`id`, `kind`, `version`, `config`, `position`, `retry`
  `{max, backoffMs}`, `timeoutMs`, `maxVisits`), `edges` (`from`, `port`, `to`) and `settings` (`maxSteps`,
  `maxRuntimeMs`). Config strings take `{{trigger.x}}`, `{{steps.<node>.x}}`, `{{state.x}}`, `{{run.id}}`; a string
  that is one reference keeps the value's type. `{{event.x}}` is left for a wait's event conditions.
- Rows: `workflows` (scope `project` | `host` | `personal`, `enabled`, `builtin`, `source` `host` | `repo`,
  `repo_file`, `version`), `workflow_versions` (every save; author `user` / `builtin` / `repo` / `agent`; `status`
  `saved` / `proposed` / `discarded`). A version's number is the highest so far plus one.
- Proposals (7d): `PUT {doc, note, propose: true}` saves a version authored `agent` that doesn't become current; a
  newer proposal discards the older one. `Workflow.proposed` is the pending one. `POST /versions/{v}/accept` makes it
  current (validated again), `/discard` sets it aside (`409 not_proposed` for anything that isn't pending). Repo
  workflows refuse proposals like saves. Restoring an old version is a save of its document.
- Repo files: `.pdt/workflows/<name>.json` in the main checkout, read by `maintain_workflows` (every 60 s with the
  merge check): new files become disabled project workflows, a changed file a new version, a removed file removes the
  workflow. They can't be edited or deleted through the API (`409 workflow_in_repo`).
- Built-ins (`builtins.rs`), installed host-wide on the engine's first tick; an untouched one follows newer templates,
  an edited one is left alone; they can be disabled, not deleted (`409 builtin_workflow`):
  - **PR notices** (on): `ci.run_failed`, `pr.conflict`, `pr.queue_left`, `pr.merged`, `attempt.merged` (not `via:
    closed`) → `session.send` (`text: {{trigger.notice.text}}`, `dedupe: {{trigger.notice.key}}`, `timeline: pr`).
  - **Finish merged tickets** (on): `attempt.merged` (`via: detected`) → gate answered ahead by `finishOnMerge`
    (Merge…'s "Move to Done and clean up once it merges", PDT-75), else `unanswered` → quiet end (the merged banner
    asks) → `ticket.finish` (`stopRunning`, `deleteBranch: {{trigger.finishDeleteBranch}}`); `blocked` (uncommitted
    changes) → quiet end.
  - **Ticket comments to the agent** (on): `ticket.commented` by the user (`actor` = `user`, not an agent's comment
    through the API or a workflow's) with an active attempt → `session.send`.
  - **Base moved while a PR waits** (off, PDT-24): `pr.behind` → `session.send`.
  - **PR comments to the agent** (off, 7e): `pr.commented` and `pr.reviewed` (not `byOwner`, not `isBot`; reviews
    not `approved` and with a body) → `session.send` (`{{trigger.notice.text}}`, dedupe on the notice key, `timeline:
    pr`).
- What the enabled workflows will tell an attempt's agent is also in that agent's instructions (pdt-core
  `workflow/news.rs`, [agents.md](agents.md) "Instructions and guides"), so turning one off changes what new sessions
  are told.

## Catalog

`GET /api/workflows/catalog` lists every kind: `id`, `version`, `category`, `title`, `description`,
`configSchema` / `outputSchema` (JSON Schema from schemars), `ports`, `dynamicPorts`. One module per kind in
pdt-api `workflow/nodes/`, registered in `catalog.rs` `catalog()`; config types also derive ts-rs (web/src/api/gen).

| Kind | Ports |
|---|---|
| one trigger per event type; `manual` (Run now, sub-workflows) — config `projects`, `where`, `runKey` | `out` |
| `condition` (`conditions`, `match: all \| any`) | `true`, `false` |
| `switch` (`value`, `cases [{port, equals}]`) | each case, `default` |
| `gate` (`question`, `choices`, `ahead [{when, choice}]`, `ask: wait \| skip`, `timeoutMs`, `inbox: gate \| accept`) | each choice, `unanswered`, `timeout` |
| `agent` (`persona`, `prompt`, `outcomes [{id, description}]`, `session: new \| reuse`, `ticket`, `attempt`, `maxTurns`, `maxRuntimeMs`, `nudges`, `loopLimit`; below) | each outcome (`dynamicPorts`), `timeout`, `stopped` |
| `wait` (`ms`, `until`, or `event {type, where}` with `timeoutMs`) | `done`, `event`, `timeout` |
| `workflow.call` (`workflow`, `input`) | `done`, `failed` |
| `end` (`quiet`, `output`) | — |
| `state.set` (`key`, `value`, `ifMissing`) | `out` |
| `session.send` (`attempt` or `session`, `text`, `dedupe`, `timeline: none \| pr`) | `sent`, `untold`, `skipped` |
| `ticket.finish` (`ticket`, `stopRunning`, `deleteBranch`) | `finished`, `blocked` |
| `ticket.move` (`ticket`, `stage`), `ticket.comment` (`ticket`, `body`) | `out` |
| `schedule` (`every` or `cron` + `timezone`, `catchUp`, `where`, `runKey`; fires on its own `schedule.fired`) | `out` |
| `webhook` (`hook`: a `webhook` connection, `where`, `runKey`; its own `webhook.received`) | `out` |
| `github.issue.opened` (`repos`, else the projects' repos; `projects`, `where`, `runKey`) | `out` |
| `command` (`program`, `args`, `cwd: scratch \| project \| attempt \| <full path>`, `input`, `env`, `parse: json \| text`) | `out` |
| `http.request` (`connection`, `method`, `url`, `headers`, `query`, `body`, `expect: 2xx \| any \| [codes]`) | `out` |
| `discord.post` (`connection`: `discord_webhook`, `content`, `username`, `embeds`, `thread`) | `out` |
| `notify` (`title`, `body`, `level: info \| warning \| error`, `link`) | `out` |

Every node also has an `error` port: taken once its retries are spent, if it has an edge; otherwise the run fails.

A trigger kind decides which events it fires for (`NodeKind::accepts`; default: the event is in the workflow's scope),
then the engine checks the shared `projects` and `where`. `schedule` and `webhook` fire only for their own events, and
`github.issue.opened` with `repos` for those repos whatever the scope. `pdt.notified` has no trigger (a workflow could
notify itself forever).

- `command` runs the program without a shell (`sh -c` / `cmd /c` for one), `input` as JSON on stdin, `env` added to
  pdtd's environment, in a scratch folder (`<data dir>/workflow-scratch/<run>`, removed when the run ends), the run's
  project's main checkout, its attempt's worktree or a full path. stdout (up to 64 KiB) is parsed as JSON into `json`
  (empty: null); `{"quiet": true}` makes the step quiet. A non-zero exit or unparseable stdout fails it (with the end
  of stderr). The time limit kills the process tree (its own process group on Unix, `taskkill /T` on Windows).
- `http.request`: `url` may be relative to the connection's URL; the connection's secret is sent per its `config.auth`
  (`bearer` default, `basic` with `username`, `header` with `header`, `query` with `param`, `none`), and only to an
  address at or below the connection's URL. Output `{status, ok, headers, body}` (JSON when the response is, else
  text; 1 MiB). A status outside `expect` is an error.
- `discord.post`: the connection's secret is the webhook URL. Content over 2000 characters goes as several messages
  (cut at line breaks), embeds with the first; mentions are never parsed; a 429 waits `retry_after` (up to 4 tries).
  Output `{messageIds}`.
- Dry runs never run a command or call out: they report `would`.

## Engine

- `run_workflows` (pdtd every 2 s; tests call it; serialized by `AppState.engine_lock`): installs the built-ins once,
  emits due schedules ("Sensors and webhooks"), matches unprocessed events to enabled workflows' triggers (scope, `projects`, `where`) and starts a run per match
  unless its run key (default `<node>:<event id>`) already ran, wakes steps waiting for that event, advances runnable
  runs, delivers the outbox, and advances again.
- `workflow_runs` / `workflow_steps`: a run starts with its trigger's step; each step is written before and after it
  runs (`pending` → `running` → `succeeded` / `waiting` / `failed`), so after a restart a `running` step runs again
  and the rest continue. A step's input is its config with references filled in; its output feeds `{{steps.x}}`.
- Waits (`wait`, gates, deliveries, sub-workflows, retries) keep the step `waiting` with what it waits for (`wait`)
  and `wake_at`; a timer that comes due takes the step's timeout port. A run with only waiting steps is `waiting`.
- A run ends `succeeded`, `quiet` (an `end` with `quiet`, or no action did anything that wasn't quiet: nobody was
  told), `failed` (`error` names the node) or `cancelled`. Limits: `maxVisits` per node (loop limit, default 25),
  `maxSteps` (200) and `maxRuntimeMs` (7 days) per run, `timeoutMs` per execution (actions default 5 min).
- `POST /api/workflow-runs/{id}/retry` (a failed run, from its failed step), `/cancel`, and
  `/steps/{step}/answer {choice}` (a waiting gate). `GET /api/workflow-runs?workflow=&ticket=&status=&limit=`,
  `GET /api/workflow-runs/{id}` (with steps).
- Per-workflow state: `workflow_state`, `GET /api/workflows/{id}/state`. Keys starting `_pdt.` are PDT's own (a
  schedule's last slot): `{{state}}` leaves them out and `state.set` refuses them.

## Outbox (`session.send`)

`session_messages`, one per step. `outbox.rs` delivers with the PR notices' old rule: the attempt's newest running
agent session (a terminal with a live PTY, or a conversation); a terminal idle with no typing for 20 s, a conversation
`ready_for_notice` (idle, nothing queued, no open request, no rate limit, terminal closed; a parked one reopens); all
of a session's waiting messages in one message; no running agent session → `untold`. `dedupe` is per attempt (also
against `pr` timeline keys from before the event log). Resolving writes the timeline record and resumes the step.

## API

`GET/POST /api/workflows` (`?project=`: its own, host-wide and personal ones), `GET/PUT/PATCH/DELETE
/api/workflows/{id}` (PUT `{doc, note}` saves a version; PATCH `{enabled}`; invalid documents are `422
workflow_invalid`), `/versions`, `/versions/{v}`, `/diff?from=&to=` (nodes and edges added, removed, changed, and a
unified diff), `/state`, `/run {input, runKey}`; `POST /api/workflows/validate {doc}` (issues: errors and warnings);
`POST /api/workflows/dry-run {doc | workflowId, event {type, payload} | eventId}` (in memory: which triggers match,
each step's input, port and output; actions report `would`).

`POST /api/workflows/check {doc}` → `{issues, ports}`: validation plus each node's ports as its config gives them (a
gate's choices, a switch's cases; `error` not listed). The builder calls it after every change.

Templates: `GET/POST /api/workflow-templates`, `GET/PUT/DELETE /api/workflow-templates/{id}`,
`POST /api/workflow-templates/{id}/instantiate {params, name, scope, projectId}` (a new workflow, always disabled).
A template is `{name, description, params, doc}`; a param `{id, title, description, type: string | text | number |
boolean | choice, choices, default, required}`; the document uses `{{params.<id>}}` (a string that is exactly one
reference takes the value with its type; other references stay). Saving checks the params and validates the document
with defaults or placeholders filled in. Built-in templates (`builtin:<id>`, read-only, `409 builtin_template`):
Blank, Tell the agent when CI fails, Ask before finishing a merged ticket, Comment when a pull request merges, the
Hermes jobs of 7e (New GitHub issues to Discord; Health watchdog: a URL on a schedule, Discord only on a change, the
first check silent; Brief follow-ups and Email processing: a script on a schedule whose `{"message": …}` goes to
Discord), and the built-in workflows. A required text param with no default is checked as the reference itself. The user's are in `workflow_templates`.

Connections: `GET/POST /api/connections`, `GET/PATCH/DELETE /api/connections/{id}` (`name` unique, `kind`, `url`,
`config`), `PUT/DELETE /api/connections/{id}/secret {secret}` (the OS keychain, account `connection:<id>`; only
`hasSecret` is ever returned).

## Sensors and webhooks (Phase 7e)

- **Schedules** (`workflow/nodes/schedule.rs`, pdt-core `workflow/schedule.rs`): each engine tick, every enabled
  workflow's `schedule` node's latest slot (`every` aligned to the epoch, at least 1 minute; or a 5-field cron in
  `timezone`, default the host's) is compared with its mark in state (`_pdt.schedule.<node>`): a node seen for the first
  time, or a workflow saved or enabled since the mark, takes the slot as its baseline; a newer slot emits
  `schedule.fired` (key `schedule:<workflow>:<node>:<slot>`, payload `workflowId`, `nodeId`, `slot`, `slotIso`,
  `date`). A slot more than 2 minutes old (pdtd was down) fires only with `catchUp` (default on), and only the latest.
- **GitHub issues** (`github_issues.rs` `refresh_issues`, pdtd every 60 s): the repos enabled `github.issue.opened`
  nodes name, else the GitHub repos of the projects in their scope, through `gh api …/issues` (pull requests left out).
  The newest number per repo is kept in `host_settings` (`sensor.github_issues.<repo>`); the first look only records
  it. Event key `github:<repo>:issue:<n>`, payload `repo`, `number`, `title`, `body`, `url`, `author`, `labels`,
  `createdAt`, `projectId` (when it is a project's repo).
- **PR comments and reviews** (`github.rs`): while an enabled workflow has a `pr.commented` or `pr.reviewed` trigger,
  the PR poller also reads each active attempt's open PR's conversation comments, review comments and reviews. The
  newest ids are kept per attempt (`sensor.pr_comments.<attempt>`); the first look only records them. Keys
  `<attempt>:comment:<id>` / `<attempt>:review:<id>`; payloads carry `author`, `body` (`path`, `line`, `inReview`; a
  review's `state`), `byOwner` (gh's own login), `isBot` and a `notice` (`commented` / `reviewed`).
- **Webhooks** (`hooks.rs`): `POST /api/hooks/<name>` for a `webhook` connection; its secret as `Authorization: Bearer`
  or GitHub's `X-Hub-Signature-256` (constant-time check). `404` no such hook, `409 webhook_no_secret`, `401
  bad_signature`; body up to 1 MiB. The identity guard lets `/api/hooks/*` through (tailnet machines have no user
  login) except over Funnel; the Host guard still applies. `webhook.received` (key `webhook:<name>:<delivery>` from
  `X-GitHub-Delivery`, `Idempotency-Key` or `X-Request-Id`, else new; a repeat answers `200 {duplicate: true}`, else
  `202 {eventId}`), payload `hook`, `method`, `query`, `headers` (`content-type`, `user-agent`, `x-*` but the
  signature), `body` (JSON, else text).
- **Notify** (`workflow/nodes/notify.rs`): `pdt.notified` (`title`, `body`, `level`, `link`, `workflowId`,
  `workflowName`, `runId`; key per step).

## Web

Notifications (`web/src/common/useWorkflowNotifications.ts`, in AppShell): follows `pdt.notified` from the page's load
(`/api/events?type=pdt.notified&after=<seq>` every 2 s) and shows each as a toast ("Open": a ticket key opens the
ticket, else the URL); with Settings → This host → Notifications → Desktop notifications on (per browser, after the
browser's permission), also as a system notification while the page is hidden.

The Workflows view (rail; `web/src/workflows/`), which replaced 7a's Settings → This host → Workflows:

- List column (`WorkflowsList`): New workflow (template, params, name, where it applies; it starts off), the
  workflows grouped This host / Personal / each project (on/off dot, Built in, Repo, Proposed), then the templates.
- A workflow (`WorkflowPage`): header with the Enabled switch (turning on asks, listing its triggers and actions, and
  offers Dry run first), Dry run, Run now (with a manual trigger; runs the saved version), ⋯ (Save as template,
  Duplicate, Delete…); a banner for a pending proposal; and Graph | Outline | Text | Runs | Versions.
- Graph, Outline and Text edit one draft (`useDraft`: undo/redo, never replaced by polling while dirty; "Unsaved
  changes" bar with a note, Discard and Save, refused while the draft has errors). Every change is checked
  (`useCheck`), so issues show on nodes and ports follow the config. Repo workflows are read-only.
  - Graph (`GraphView` on `web/ui/FlowGraph`, React Flow): Add node (the catalog, wired from the selected node's first
    open port), drag a port to a node to connect, drag to move (saved as `position`), Delete/Backspace removes,
    Arrange lays everything out again. Nodes without a position get a layered layout that avoids placed ones.
  - Outline (`OutlineView`): each trigger, then its steps; a node with several ports nests a branch per port; a node
    reached again is "Continues at …". Up/Down move between rows, Enter opens the panel; ⋯ adds after, moves up/down in
    a chain, removes (joining one way in to one way out). Default on phones, with the panel in a sheet.
  - Text (`TextView`): JSON or TOML (no nulls) in CodeMirror; applied as you type once it parses.
  - The side panel (`NodePanel` + `SchemaForm`) is generated from the kind's config schema: text, numbers, switches,
    choices, lists, objects, optional and `Tpl` fields (a value or a `{{reference}}`, with a picker of trigger fields and
    earlier steps), JSON for anything else; plus id (rename rewrites edges and `{{steps.<id>}}`), title, retries,
    time limit, loop limit and "On error, go to". With no node selected it edits the workflow's name, description and
    limits.
- Runs (`RunsView`): the runs (polled), and a run's graph of the version it ran with the path taken, its steps (input,
  output, error), Retry from the failed step, Cancel, and a waiting gate's choices.
- Dry run (`DryRunDialog`): the draft against a recent event of its trigger types, a hand-written one (prefilled from
  the trigger's schema) or a manual input; the path, why triggers didn't match, and what each action would do.
- Versions (`VersionsView`): every version with author, status and note, a diff against the current one, Restore,
  Accept / Discard for a proposal.
- Templates (`TemplatePage`): built-in ones read-only; the user's edit name, description, params and the document.

## Agent steps (Phase 7b)

The `agent` node (pdt-api `workflow/nodes/agent.rs`, the work in `workflow/agent.rs`) runs a persona in a session and
waits for the outcome the agent submits through pdt-mcp ([agents.md](agents.md) "pdt-mcp").

- **Session.** `new` (default; Reviewer, QA, Refiner): a fresh conversation of the persona's agent and model in the
  attempt's worktree (else the project's main checkout, or a Personal project's ticket folder), recording the persona
  and version (`sessions.persona_id`, `persona_version`, `step_id`), with no tab and no attempt (so PR notices still go
  to the attempt's own session). `reuse` (Coder): the session this node used earlier in the run, else the attempt's
  running agent session, else a new one that becomes the attempt's; a reused session gets the step through the outbox
  (never into a running turn), with the step's rules in the message since its instructions were fixed at its start.
- **Ticket and attempt**: the config's, else the trigger's `ticketId` / `attemptId`, else the ticket's active attempt.
  A step with neither runs in the run's (or workflow's) project.
- **Permission mode** of a new session: the project's `agentPermissionMode` (project settings), else the persona
  version's, else the host default (`ticketPermissionMode` on a ticket, `sessionPermissionMode` otherwise).
- **Instructions and first message.** The session's instructions get a "Your role: <persona>" section (the persona's
  instructions, the outcomes, and the rule: finish with `submit_outcome`, ask with `ask_user`, a turn with neither
  counts as stalled). The first message is the node's `prompt`, the ticket's key and title, and the outcome list.
- **Binding.** `agent_steps` (one per step: session, persona and version, project, ticket, attempt, outcomes,
  `state` `working | asking | stalled | done`, turns, nudges) is what pdt-mcp and the Inbox read. The step waits
  (`wait: {agent: …}`) until `maxRuntimeMs` (4 h), then `timeout`; a step re-run after a restart waits again.
- **Outcome.** `submit_outcome` checks the outcome is one of the step's (else an error the agent sees), adds its line
  comments as review comments on the attempt's worktree (author: the persona), records an `outcome` timeline event
  and completes the step by that port with `{outcome, summary, comments, sessionId, persona, personaVersion, turns,
  by}`. The session stays as it is.
- **Turns.** A conversation's turn end (pdt-api `conversations.rs`) or a terminal going idle (`activity.rs`) is
  reported for a working binding (a reused session's turn before the step's message was delivered doesn't count):
  `turns + 1`; at `maxTurns` (10) the step takes `timeout`; while `nudges` (1) remain PDT sends "[PDT] You ended your
  turn without `submit_outcome`…"; after that the binding is **stalled** and the Inbox has it. The engine tick also
  stalls a working step whose session stopped (not running, or a conversation with no live process, turn or queued
  message) for 30 s.
- **Questions.** `ask_user` makes an Inbox question (a new one replaces the step's open one) and the binding
  `asking`, so its turn's end isn't a stall; the answer goes back through the outbox as "[PDT] The user answered your
  questions: …" and the binding works again. A session with no step can ask too.
- **Loop limit.** When the node is about to run more than `loopLimit` (3; 0: none) times again in one run, it waits
  with a `loop_limit` Inbox item instead (the earlier passes' outcomes and summaries): **Once more** runs it (that
  pass only), **Take over** completes it with an outcome (`by: user`), **Stop** takes `stopped`.
- When a step completes or its run ends, its open Inbox items close and its binding is done.

## Personas

`personas` / `persona_versions` (pdt-store `personas.rs`, pdt-api `personas.rs`): a name, role and description, and
versions of agent, model, instructions, permission mode (null: project override, else host default) and tools (pdt-mcp
tools; empty: the default five). Every save (`PUT /api/personas/{id}`) is a new version; `PATCH` changes name, role,
description, `archived`; `GET /versions`, `/versions/{v}`, `/diff?from=&to=` (the fields that changed and a unified
diff). Built-ins Refiner, Coder, Reviewer and QA (pdt-core `persona.rs`) are installed on the engine's first tick (and
on `GET /api/personas`), follow newer templates until edited, and can be archived, not deleted (`409
builtin_persona`); a persona a workflow names can't be deleted (`409 persona_in_use`).

## Inbox

`inbox_items` (pdt-store `inbox.rs`, pdt-api `inbox.rs`): `question` (`ask_user`), `gate` (a waiting `gate` node;
group `gate` or `accept` from its `inbox`), `stalled`, `loop_limit`; one open item per step and kind. Approvals are
not stored: the open permission requests of live conversations (`approval:<session>:<request>`), answered in their
session. `GET /api/inbox?project=` → `{items, total}`, Questions, Approvals, Stalled, Loop limit, Gates, Ready to
accept, oldest first within each, with ticket key and title and persona name. `POST /api/inbox/{id}/answer` takes
`{choice}` (a gate's choice; `nudge` with an optional `text`, or `outcome:<id>`, for a stalled step; `again`,
`outcome:<id>` or `stop` at a loop limit) or `{answers}` / `{text}` for a question; `POST /api/inbox/{id}/dismiss`
closes a question or stalled item (the step keeps waiting). A gate answered through
`/api/workflow-runs/{run}/steps/{step}/answer` resolves its item too.

Web: the rail's Inbox (web/src/inbox; the open count on its icon, `Ctrl+] i`): the list groups items by kind; the work
area shows the item and its decision (answers per question, the approval card, Nudge / Take over, Once more / Take
over / Stop, a gate's choices and Open changes for Ready to accept); `j`/`k` move, Enter takes the primary action,
`o` opens the ticket, `e` dismisses.
