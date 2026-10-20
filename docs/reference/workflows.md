# Workflows, events and the engine (Phase 7a)

Spec: docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md (vision: 2026-10-20-workflows-and-pm-vision.md).
Code: pdt-core `workflow/` (documents, references, conditions, validation, diffs, event types, API records),
pdt-store `event_log.rs` and `workflows.rs` (migration 0021), pdt-api `workflow/` (catalog, engine, nodes, outbox,
built-ins, repo files, routes) and `finish.rs` (`ticket.finish`).

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
  `repo_file`, `version`), `workflow_versions` (every save; author `user` / `builtin` / `repo` / `agent`).
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

Connections: `GET/POST /api/connections`, `GET/PATCH/DELETE /api/connections/{id}` (`name` unique, `kind`, `url`,
`config`), `PUT/DELETE /api/connections/{id}/secret {secret}` (the OS keychain, account `connection:<id>`; only
`hasSecret` is ever returned).

## Web

Settings → This host → Workflows (web/src/host/WorkflowsSection.tsx): every workflow with its switch, badges (Built
in, In the repo) and last run. The builder is Phase 7d.
