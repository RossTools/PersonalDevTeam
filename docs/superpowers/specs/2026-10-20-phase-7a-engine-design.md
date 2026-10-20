# Personal Dev Team — Phase 7a: events, workflow engine core and node catalog

Status: design for PDT-76, 2026-10-20. Builds §2–§5, §7 and §8 of the vision
(docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md) and settles its open item "event log vs. existing
tables". Agent steps (7b), the pipeline (7c) and the builder UI (7d) are out of scope; their needs shape the format.

## 1. What 7a delivers

- An **event log**: typed, keyed, deduplicated events with retention, fed by the sensors and by the domain's own
  state changes.
- **Workflow documents**: versioned (every save a version, with a diff), stored by pdtd, optionally read from
  `.pdt/workflows/` in a project's repo; project, host-wide or personal scope; enabled per workflow.
- A **durable engine**: run and step rows in SQLite, at-least-once execution, resume after a restart, retry from a
  failed step, run keys, per-workflow state, quiet runs, timeouts and limits.
- A **node catalog**: one registry of self-describing node kinds (id, version, config schema, output schema, ports),
  served at `GET /api/workflows/catalog`; configs are validated against it, and the TS types come from the same Rust
  types.
- **Connections**: named external services, credentials in the secret store.
- **Domain commands** shared by the UI, API and workflows: `ticket.finish` (one server command for the merged banner,
  Merge…'s finish-on-merge and workflows) and `session.send` (the PR notices' delivery rule).
- **Built-in workflows** that replace the hard-coded reactions: PR notices, finishing merged tickets, comments to the
  agent, and (off by default) "main moved while a PR waits".
- Validate and dry-run a document against a recorded or hand-written event.

Done when the PR notices and finishing a merged ticket run as enabled built-in workflows with today's behaviour, their
runs are inspectable through the API, and a new trigger or action is one module plus a test.

## 2. Events

### 2.1 The log and the existing tables (decided)

| Table | Stays | Relation to the log |
|---|---|---|
| `ticket_events` (timeline) | Yes: the user-facing history of a ticket | Domain changes recorded there are **mirrored into the log in the same transaction** (one choke point: the store's timeline insert). A `pr` timeline event is now the record of a message a workflow delivered (or could not). |
| `session_events` (conversations) | Yes: an agent's transcript | Not mirrored: transcripts are not domain events (session activity triggers can come later). |
| PR watch (memory) | Yes: the badge's view of a PR | The poller turns news into events instead of queueing notices. |

So the log is beside the existing tables, not a replacement, and nothing the UI reads moves.

### 2.2 Shape

`events`: `seq` (order), `id`, `type` (`pr.conflict`), `key` (dedupe: unique; a second event with the same key is
dropped), `project_id`, `ticket_id`, `attempt_id`, `session_id`, `source` (`pr_poller`, `tickets`, `api`, …),
`payload` (JSON), `created_at`, `processed_at` (the engine matched it to triggers). Retention: 30 days, then pruned
(runs keep a copy of their trigger event).

### 2.3 Types in 7a

CI events are provider-neutral (`provider: "github_actions"` today; Buildkite in 7e feeds the same types).

| Type | Source | Key | Payload (besides ticket/attempt ids, key, branch, base) |
|---|---|---|---|
| `ci.run_failed` | PR poller | `<attempt>:ci_failed:<run.attempt,…>` | `provider`, `pr`, `headSha`, `url`, `runs`, `jobs`, `notice` |
| `ci.run_passed` | PR poller | `<attempt>:ci_passed:<run.attempt,…>` | `provider`, `pr`, `headSha`, `url`, `runs` |
| `pr.conflict` | PR poller | `<attempt>:conflict:<head>` | `pr`, `url`, `headSha`, `notice` |
| `pr.queue_left` | PR poller | `<attempt>:queue_removed:<event id>` | `pr`, `url`, `reason`, `jobs`, `notice` |
| `pr.merged` | PR poller | `<attempt>:merged` | `pr`, `url`, `notice` |
| `pr.behind` | PR poller (open, auto-merge on, not in a merge queue, behind its base) | `<attempt>:behind:<head>` | `pr`, `url`, `headSha`, `notice` |
| `attempt.started` | timeline `started` | the timeline event id | `sessionId` |
| `attempt.merged` | timeline `merged` | `<attempt>:merged` (as `pr.merged`, so the notice is one) | `via` (`detected`, `merged_here`, `closed`), `actor`, `pr`, `finishOnMerge`, `finishDeleteBranch`, `notice` |
| `attempt.abandoned` | timeline `abandoned` | the timeline event id | `actor` |
| `ticket.stage_changed` | timeline `moved` | the timeline event id | `from`, `to`, `actor` |
| `ticket.commented` | timeline `comment` | the timeline event id | `body`, `attemptId` (the active attempt, if any) |
| `ticket.overlap_found` | timeline `overlap` | the timeline event id | `otherTicketId`, `otherKey`, `files`, `level` |

`notice` is the message today's code types into the agent (`{kind, key, pr, url, jobs, text}`), so the built-in
templates can deliver exactly today's text with `{{trigger.notice.text}}`, and a user's workflow can write its own
from the other fields.

`GET /api/events` (filters: `type`, `project`, `ticket`, `attempt`, `after`, `limit`) and `GET /api/events/{id}` read
the log; `POST /api/events` is not offered (sensors and domain commands are the only writers), but a dry run takes a
hand-written event (§6).

## 3. Workflow documents

### 3.1 Format

```json
{
  "name": "PR notices",
  "description": "…",
  "nodes": [
    {"id": "ci", "kind": "ci.run_failed", "version": 1, "config": {"where": []}, "position": {"x": 0, "y": 0}},
    {"id": "tell", "kind": "session.send", "version": 1,
     "config": {"attempt": "{{trigger.attemptId}}", "text": "{{trigger.notice.text}}",
                "dedupe": "{{trigger.notice.key}}", "timeline": "pr"},
     "retry": {"max": 2, "backoffMs": 5000}, "timeoutMs": 60000, "maxVisits": 10}
  ],
  "edges": [{"from": "ci", "port": "out", "to": "tell"}],
  "settings": {"maxSteps": 200, "maxRuntimeMs": 86400000}
}
```

- A node's `version` is the catalog kind's version it was written against. An older version is migrated by the kind
  when the run loads it (and flagged by validation); a newer one is an error ("written for a newer PDT").
- `position` is the graph layout (7d); optional.
- Every node can have `retry`, `timeoutMs`, `maxVisits` (loop limit, default 25) and an `error` edge (§4.3).
- Several trigger nodes may lead into one graph (one workflow for all PR notices).

### 3.2 Data references

Config strings may hold `{{path}}` references into the run's context: `trigger` (the event payload plus `type`,
`eventId`, `projectId`, `ticketId`, `attemptId`), `steps.<node id>` (that node's latest output), `state.<key>`
(workflow state), `run` (`id`, `workflowId`, `dryRun`). A string that is exactly one reference keeps the value's JSON
type (`"{{trigger.finishOnMerge}}"` is a boolean); otherwise values are interpolated as text, missing ones as empty.
Non-string config fields accept either a literal or a reference.

Conditions (trigger filters, `condition`, `switch`) are data, not an expression language:
`{"left": "{{trigger.via}}", "op": "eq", "right": "detected"}` with ops `eq`, `ne`, `in`, `not_in`, `exists`,
`missing`, `truthy`, `falsy`, `gt`, `lt`, `contains`, `starts_with`.

### 3.3 Storage, versions, scope

- `workflows`: id, name, description, `scope` (`project` | `host` | `personal`), `project_id` (project scope),
  `enabled`, `builtin` (the template id it came from), `source` (`host` | `repo`), `repo_file`, `version` (current),
  timestamps. `workflow_versions`: (workflow, version) → doc, author (`user`, `builtin`, `repo`, `agent`), note,
  time. Every save is a new version; `GET /api/workflows/{id}/diff?from=&to=` gives a node/edge summary and a unified
  diff of the two documents. A run records the version it ran.
- **Host-first.** Workflows live in pdtd's database. A project may also keep documents in its repo as
  `.pdt/workflows/<name>.json` (main checkout); pdtd reads them on start and every minute, each file one workflow with
  `source: repo` (read-only in PDT: edit the file; a changed file is a new version authored `repo`). A repo workflow
  starts **disabled**: enabling stays a host-side choice (consent, vision principle 5). Separate files don't touch
  `.pdt/project.toml`, so its compatibility rule holds.
- **Scope.** `project`: the project's events only. `host`: every project's, narrowed by a trigger's `projects`
  filter. `personal`: no project (manual and later schedule triggers; the Personal project lands with 7b).
- **Built-ins** ship in pdtd as templates and are installed host-wide on start. A built-in the user never edited is
  updated to a newer template (a new version authored `builtin`); one the user edited is left alone. Built-ins can be
  disabled, not deleted.

## 4. Engine

### 4.1 Runs and steps

- `workflow_runs`: id, workflow, version, `status` (`running`, `waiting`, `succeeded`, `quiet`, `failed`,
  `cancelled`), `run_key` (unique per workflow), trigger event id and a copy of the trigger, project/ticket/attempt
  (for listing), parent run and step (sub-workflows), `wake_at`, error, output, timestamps.
- `workflow_steps`: id, run, `seq`, node id, `status` (`pending`, `running`, `waiting`, `succeeded`, `failed`,
  `skipped`), input (the resolved config), output, `port` (the edge taken), attempts, error, `wake_at`, `wait`
  (what it waits for), timestamps.
- `workflow_state`: (workflow, key) → JSON value.

### 4.2 Execution

The engine is one tick (every 2 s in pdtd; tests call it): match new events to enabled triggers (a run per matching
trigger node, skipped when its run key exists), wake waiting steps whose event, answer or time came, then advance every
runnable run step by step. A run starts with its trigger node's step (output: the event), and each finished step
queues the nodes its port's edges lead to. Every step is written before and after it runs, so a pdtd restart re-runs
a step that was running (at least once; actions are idempotent or dedupe) and continues the rest.

A step's outcome is one of: **next** (a port and an output), **wait** (until a time, an event, an answer or a
delivery; with the port to take on timeout), **end** (the run ends `succeeded` or `quiet`), or an **error**.

### 4.3 Engine features

- **Run keys**: a trigger's `runKey` (default: the event id); a second run with the same key is skipped.
- **Workflow state**: `state.set` / `state.get` nodes and `{{state.x}}`; `state.set` with `ifMissing` records a
  baseline once.
- **Quiet runs**: an `end` node with `quiet: true` (or a run whose `session.send` found nothing to send) ends
  `quiet`: nobody is told, and the run is still listed.
- **Limits**: per-node `timeoutMs` (actions), `retry` (`max`, `backoffMs`, doubling), `maxVisits` (a loop limit:
  more visits fail the step), and per-workflow `maxSteps` and `maxRuntimeMs`.
- **Error path**: a node with an edge from its `error` port takes it (output `{error}`) once retries are spent;
  without one, the run fails.
- **Retry from a failed step**: `POST /api/workflow-runs/{id}/retry` resets the failed step and resumes the run.
- **Cancel**: `POST /api/workflow-runs/{id}/cancel`.
- **Attribution**: every automated change names its workflow and run (the `pr` timeline event's `workflowId`,
  `workflowName`, `runId`; ticket moves by `ticket.finish` the same).

### 4.4 Node kinds in 7a

| Kind | Category | Ports | Notes |
|---|---|---|---|
| one per event type (§2.3) | trigger | `out` | config `projects`, `where` (conditions), `runKey` |
| `manual` | trigger | `out` | "Run now": `POST /api/workflows/{id}/run` with an input |
| `condition` | condition | `true`, `false` | `all` / `any` of conditions |
| `switch` | condition | each case's port, `default` | `value` and cases `{port, equals}` |
| `gate` | human gate | each choice, `timeout` | `question`, `choices`, `answer` (answered ahead of time when it resolves to a choice), `ask` (`wait` for an answer through the API, or take the `unanswered` port at once), `timeoutMs` |
| `wait` | wait | `done`, `event`, `timeout` | `ms`, `until`, or `event` (type + conditions) with `timeoutMs` |
| `workflow.call` | sub-workflow | `done`, `failed` | runs another workflow with an input and waits for it; its output is the child's |
| `end` | flow | — | `quiet`, `output` |
| `state.set` | action | `out` | `key`, `value`, `ifMissing` |
| `session.send` | action | `sent`, `untold`, `skipped` | §5.2 |
| `ticket.finish` | action | `finished`, `blocked` | §5.1 |
| `ticket.move` | action | `out` | `ticket`, `stage` |
| `ticket.comment` | action | `out` | `ticket`, `body` (a system comment) |

Agent steps come in 7b. Answering a gate: `POST /api/workflow-runs/{run}/steps/{step}/answer {choice}` (the Inbox
renders gates in 7b).

### 4.5 The catalog

Each kind is one Rust module implementing one trait: `spec()` (id, version, category, title, description, config
schema, output schema, ports), `ports(config)` for dynamic ports, `run(cx, input)`, `dry_run(cx, input)` and
`migrate(from, config)`. Config and output types are Rust structs deriving serde, `schemars` (the JSON Schema served by
the catalog and used for forms in 7d) and ts-rs (the TS types in web/src/api/gen). Validation deserializes each node's
config into its kind's type. Adding a kind: one module, one line in the registry, a test.

## 5. Domain commands

### 5.1 `ticket.finish`

`POST /api/tickets/{id}/finish {stopRunning, discardChanges, deleteBranch, keepWorktree}` →
`{outcome: finished | blocked | kept, blocked?: {sessions, runs, dirty}, worktreeRemoved, workspaceRemoved,
branchNote?, pull?}`. It checks first and changes nothing when blocked: running sessions or runs (unless
`stopRunning`) or uncommitted changes (unless `discardChanges`). Otherwise it moves the ticket to Done, stops and
removes the worktree's and workspace's sessions and runs (the attempt's elsewhere only stop), removes the worktree
(and the branch with `deleteBranch`), closes the ticket's tab elsewhere, deletes the workspace Start made and
fast-forwards the base branch. `keepWorktree` only moves to Done and pulls. The merged banner and Merge… call it (and
ask the user on `blocked`, then call again); the `ticket.finish` node calls it with `stopRunning` and never
`discardChanges`, so a dirty worktree is always left for the banner (as PDT-75).

### 5.2 `session.send`

Config: `attempt` (the newest running agent session of that attempt; terminal with a live PTY, or a conversation),
or `session`; `text`; `dedupe` (per attempt: a key already sent is `skipped`); `timeline` (`pr` records today's `pr`
timeline event with the trigger's `notice`; `none`). The step puts the message in a durable **outbox**
(`session_messages`) and waits. The delivery rule is today's: a terminal must be idle with no typing for 20 s, a
conversation idle with no open request, no queued message, no rate limit and its terminal closed; all of a session's
waiting messages go in one message; a message whose attempt has no running agent session is recorded as not told
(`untold`). PDT-53's cross-session messages will use the same outbox.

## 6. Validate and dry run

`POST /api/workflows/validate {doc}` and the result on every save: unknown kinds, newer or older kind versions,
invalid configs (the kind's type), duplicate node ids, edges from unknown ports or to unknown nodes, no trigger,
unreachable nodes, and ports with no edge (a warning: the run ends there). Saving an invalid document is refused
(`422 workflow_invalid`); warnings are returned.

`POST /api/workflows/dry-run {doc | workflowId, event: {type, payload} | eventId}` runs the graph in memory: triggers
match (or say why not), conditions and switches evaluate, actions report what they would do (`would`) without doing
it, waits and gates take their first path. The response lists each step with its input, port and output.

## 7. Connections

`connections`: id, unique name, kind (free text in 7a, e.g. `discord_webhook`), base URL, non-secret config JSON,
timestamps. The secret (token, key, webhook URL) is set with `PUT /api/connections/{id}/secret`, kept in the OS
keychain under `connection:<id>` and never returned (`hasSecret`). Nodes get a connection by name through the step
context; 7e's HTTP, Discord and media kinds are the first users.

## 8. Built-in workflows

| Built-in | Default | Graph |
|---|---|---|
| PR notices | on | `ci.run_failed`, `pr.conflict`, `pr.queue_left`, `pr.merged`, `attempt.merged` (where `via` ≠ `closed`) → `session.send` (`timeline: pr`, dedupe on the notice key) |
| Finish merged tickets | on | `attempt.merged` (where `via` = `detected`) → `gate` answered ahead by `{{trigger.finishOnMerge}}` (else `unanswered` → `end` quiet: the banner asks) → `ticket.finish` (`stopRunning`, `deleteBranch` from the trigger) → `blocked` → `end` quiet |
| Ticket comments to the agent | on | `ticket.commented` (where `attemptId` exists) → `session.send` |
| Main moved while a PR waits (PDT-24) | off | `pr.behind` → `session.send` ("merge origin/<base> in and push") |

With the first two enabled, the PR notices and finishing behave as before; turning them off stops them, which is the
consent the updated invariant names: "nothing is sent to an agent without the user's click or a workflow the user
enabled".

## 9. API summary

- `GET /api/workflows/catalog`
- `GET /api/workflows?project=&scope=`, `POST /api/workflows`, `GET|PUT|PATCH|DELETE /api/workflows/{id}`,
  `GET /api/workflows/{id}/versions`, `GET /api/workflows/{id}/versions/{v}`, `GET /api/workflows/{id}/diff`,
  `GET /api/workflows/{id}/state`, `POST /api/workflows/{id}/run`
- `POST /api/workflows/validate`, `POST /api/workflows/dry-run`
- `GET /api/workflow-runs?workflow=&ticket=&status=&limit=`, `GET /api/workflow-runs/{id}` (with steps),
  `POST /api/workflow-runs/{id}/retry`, `POST /api/workflow-runs/{id}/cancel`,
  `POST /api/workflow-runs/{id}/steps/{step}/answer`
- `GET /api/events`, `GET /api/events/{id}`
- `GET|POST /api/connections`, `PATCH|DELETE /api/connections/{id}`, `PUT|DELETE /api/connections/{id}/secret`
- `POST /api/tickets/{id}/finish`

A small Settings → Workflows page lists the workflows with their enabled switch and recent runs, so the built-ins can
be turned off before the builder (7d) exists.

## 10. Code layout

- pdt-core `workflow/`: document, conditions and references, validation, diff, event types and payloads, API types.
- pdt-store `event_log.rs`, `workflows.rs`: the tables above; the timeline mirror in `tickets.rs`.
- pdt-api `workflow/`: `engine.rs` (tick, runs, steps, dry run), `catalog.rs` (trait and registry), `nodes/*.rs`
  (one module per kind), `builtins.rs`, `routes.rs`, `outbox.rs` (`session.send` delivery); `finish.rs` becomes the
  `ticket.finish` command; `github.rs` keeps the poller and emits events.
