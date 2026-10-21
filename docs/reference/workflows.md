# Workflows, events, the engine and the builder (Phases 7a, 7d)

Specs: docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md and 2026-10-20-phase-7d-builder-design.md (vision:
2026-10-20-workflows-and-pm-vision.md).
Code: pdt-core `workflow/` (documents, references, conditions, validation, diffs, event types, templates, API
records), pdt-store `event_log.rs` and `workflows.rs` (migrations 0021, 0022), pdt-api `workflow/` (catalog, engine,
nodes, outbox, built-ins, templates, repo files, routes) and `finish.rs` (`ticket.finish`); web `src/workflows/` and
`web/ui/FlowGraph.tsx`.

## Event log

- Table `events`: `type`, `key` (unique per type: a second event with the same key is dropped), project / ticket /
  attempt / session ids, `source`, JSON `payload`, `processed_at` (matched by the engine). Kept 30 days.
- Writers: the PR poller (`github.rs` `news_events`: `ci.run_failed`, `ci.run_passed`, `pr.conflict`,
  `pr.queue_left`, `pr.merged`, `pr.behind`; key `<attempt>:<notice key>`), and the ticket timeline: the store's
  `insert_event` mirrors `moved`, `comment`, `started`, `merged`, `abandoned` and `overlap` into
  `ticket.stage_changed`, `ticket.commented`, `attempt.started`, `attempt.merged` (key `<attempt>:merged`, with `via`
  `detected` / `merged_here` / `closed`, `pr`, `finishOnMerge`, `finishDeleteBranch` and the merged `notice`),
  `attempt.abandoned` and `ticket.overlap_found`, in the same transaction. Payload types: pdt-core
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
  - **Ticket comments to the agent** (on): `ticket.commented` by the user with an active attempt → `session.send`.
  - **Base moved while a PR waits** (off, PDT-24): `pr.behind` → `session.send`.

## Catalog

`GET /api/workflows/catalog` lists every kind: `id`, `version`, `category`, `title`, `description`,
`configSchema` / `outputSchema` (JSON Schema from schemars), `ports`, `dynamicPorts`. One module per kind in
pdt-api `workflow/nodes/`, registered in `catalog.rs` `catalog()`; config types also derive ts-rs (web/src/api/gen).

| Kind | Ports |
|---|---|
| one trigger per event type; `manual` (Run now, sub-workflows) — config `projects`, `where`, `runKey` | `out` |
| `condition` (`conditions`, `match: all \| any`) | `true`, `false` |
| `switch` (`value`, `cases [{port, equals}]`) | each case, `default` |
| `gate` (`question`, `choices`, `ahead [{when, choice}]`, `ask: wait \| skip`, `timeoutMs`) | each choice, `unanswered`, `timeout` |
| `wait` (`ms`, `until`, or `event {type, where}` with `timeoutMs`) | `done`, `event`, `timeout` |
| `workflow.call` (`workflow`, `input`) | `done`, `failed` |
| `end` (`quiet`, `output`) | — |
| `state.set` (`key`, `value`, `ifMissing`) | `out` |
| `session.send` (`attempt` or `session`, `text`, `dedupe`, `timeline: none \| pr`) | `sent`, `untold`, `skipped` |
| `ticket.finish` (`ticket`, `stopRunning`, `deleteBranch`) | `finished`, `blocked` |
| `ticket.move` (`ticket`, `stage`), `ticket.comment` (`ticket`, `body`) | `out` |

Every node also has an `error` port: taken once its retries are spent, if it has an edge; otherwise the run fails.

## Engine

- `run_workflows` (pdtd every 2 s; tests call it; serialized by `AppState.engine_lock`): installs the built-ins once,
  matches unprocessed events to enabled workflows' triggers (scope, `projects`, `where`) and starts a run per match
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
- Per-workflow state: `workflow_state`, `GET /api/workflows/{id}/state`.

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
Blank, Tell the agent when CI fails, Ask before finishing a merged ticket, Comment when a pull request merges, and
the four built-in workflows. The user's are in `workflow_templates`.

Connections: `GET/POST /api/connections`, `GET/PATCH/DELETE /api/connections/{id}` (`name` unique, `kind`, `url`,
`config`), `PUT/DELETE /api/connections/{id}/secret {secret}` (the OS keychain, account `connection:<id>`; only
`hasSecret` is ever returned).

## Web

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
