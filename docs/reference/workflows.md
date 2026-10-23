# Workflows, events, the engine, the builder, agent steps, the pipeline and integrations (Phases 7a–7e)

Specs: docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md, 2026-10-20-phase-7d-builder-design.md, for
agent steps, personas, pdt-mcp and the Inbox, 2026-10-20-phase-7b-agent-steps-design.md, for the Pipeline
template, policies, Board stages and scheduling, 2026-10-20-phase-7c-pipeline-design.md, and for schedules,
webhooks, GitHub issues and PR comments, command, HTTP, Discord and notify, Radarr, Sonarr, Plex, MCP tools and the
brief archive, 2026-10-20-phase-7e-integrations-design.md (vision: 2026-10-20-workflows-and-pm-vision.md).
Code: pdt-core `workflow/` (documents, references, conditions, validation, diffs, event types, templates, API
records, what reaches an agent, agent steps), `persona.rs`, `inbox.rs`; pdt-store `event_log.rs`, `workflows.rs`,
`personas.rs` and `inbox.rs` (migrations 0021–0024); pdt-api `workflow/` (catalog, engine, nodes, outbox,
built-ins, templates and the Pipeline, repo files, routes, agent steps), `personas.rs`, `inbox.rs`, `scheduler.rs`,
`finish.rs` (`ticket.finish`), `github_issues.rs` (the issues sensor and its trigger), `hooks.rs` (incoming
webhooks) and `mcp_client.rs` (MCP servers as a client); pdt-core `policy.rs`;
web `src/workflows/`, `src/inbox/`, `src/personas/` and `web/ui/FlowGraph.tsx`.

## Event log

- Table `events`: `type`, `key` (unique per type: a second event with the same key is dropped), project / ticket /
  attempt / session ids, `source`, JSON `payload`, `processed_at` (matched by the engine). Kept 30 days.
- Writers: the PR poller (`github.rs` `news_events`: `ci.run_failed`, `ci.run_passed`, `pr.conflict`,
  `pr.queue_left`, `pr.merged`, `pr.behind`; key `<attempt>:<notice key>`), and the ticket timeline: the store's
  `insert_event` mirrors `moved`, `comment`, `started`, `merged`, `abandoned` and `overlap` into
  `ticket.stage_changed`, `ticket.commented`, `attempt.started`, `attempt.merged` (key `<attempt>:merged`, with `via`
  `detected` / `merged_here` / `closed`, `pr`, `finishOnMerge`, `finishDeleteBranch` and the merged `notice`),
  `attempt.abandoned` (every `attempt.*` carries the ticket's `labels` at the time, for `{{trigger.labels}}`
  `contains` `<label>`) and `ticket.overlap_found`, in the same transaction; an agent step's `outcome` timeline event
  becomes `agent.outcome` (`outcome`, `summary`, `persona`, `personaVersion`, `by`, `sessionId`, `runId`). Phase 7e
  adds the PR poller's `pr.commented` and `pr.reviewed`, the issues sensor's `github.issue.opened`, the scheduler's
  `schedule.fired`, webhooks' `webhook.received` and `notify`'s `pdt.notified` ("Sensors and webhooks"); 7e-2 the
  Jira sensor's `jira.issue.created`, `.updated`, `.transitioned` and `.commented`, and the Buildkite sensor's
  `ci.run_failed` / `ci.run_passed`. Payload types: pdt-core `workflow/events.rs` (`EVENT_TYPES`). CI events carry
  `provider` (`github_actions` or `buildkite`) and the PR's `autoMerge` and `inMergeQueue`; CI and `pr.*` events
  carry `mergeRequested` (the open PR was set to merge and hasn't merged, PDT-104: [tickets.md](tickets.md)) and
  `agentWorking` (an agent step is open on the attempt). `ticket.stage_changed` carries `jiraKey` when the ticket is linked to
  a Jira issue.
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
  - **PR notices** (on): `pr.merged`, `attempt.merged` (not `via: closed`) → `session.send` (`text:
    {{trigger.notice.text}}`, `dedupe: {{trigger.notice.key}}`, `timeline: pr`). `ci.run_failed`, `pr.conflict`,
    `pr.queue_left` and `pr.behind` → a condition on `{{trigger.mergeRequested}}` (PDT-104): not set to merge → the
    same `session.send`; set to merge but an agent on it (`attempt.agent_working`, read as the step runs, so a
    conflict and the queue drop it caused, logged in one look, get one fix) → the `session.send` too; else
    `state.set` `tries.{{trigger.attemptId}}` `add: 1`, and over 3 (`FIX_TRIES`) → a `ticket.comment` the first time
    (4) → the `session.send`; within 3 → `fix` (agent, Coder, `session: reuse`; the prompt is the notice plus "this
    pull request was set to merge…"; outcomes `submitted`: pushed, `review`: a large fix, committed, not pushed) →
    `submitted` → `remerge` (`attempt.merge` with `again`); `review` → `review` (agent, Reviewer, new session):
    `approved` → `push` (agent, Coder, reuse: push it; `submitted` → `remerge`), `changes_requested` → `fix`.
    `timeout` / `stopped` of these → a ticket comment. `remerge` `refused` → a ticket comment with the reason.
    `ci.run_passed` where `pr` exists, `mergeRequested`, not `autoMerge`, not `inMergeQueue`, not `agentWorking` →
    `remerge`. An untouched PR notices follows this template, so an enabled one picks it up on pdtd's next start.
  - **Finish merged tickets** (on): `attempt.merged` (`via: detected`) → gate answered ahead by `finishOnMerge`
    (Merge…'s "Move to Done and clean up once it merges", PDT-75), else `unanswered` → quiet end (the merged banner
    asks) → `ticket.finish` (`stopRunning`, `deleteBranch: {{trigger.finishDeleteBranch}}`); `blocked` (uncommitted
    changes) → quiet end.
  - **Ticket comments to the agent** (on): `ticket.commented` by the user (`actor` = `user`, not an agent's comment
    through the API or a workflow's) with an active attempt → `session.send`. A note given with a gate's choice is the
    user's comment too, but carries `workflowId` and is skipped: that run passes it on itself.
  - **Base moved while a PR waits** (off, PDT-24): `pr.behind` → `session.send`. PR notices now handles `pr.behind`
    too (PDT-104), so with both on the agent hears it twice.
  - **PR comments to the agent** (off, 7e): `pr.commented` and `pr.reviewed` (not `byOwner`, not `isBot`; reviews
    not `approved` and with a body) → `session.send` (`{{trigger.notice.text}}`, dedupe on the notice key, `timeline:
    pr`).
  - **Import assigned Jira issues** (on, 7e-2): `jira.issue.created` and `jira.issue.updated` where `assignedToMe`
    and no `ticketId` → `jira.import` (`{{trigger.key}}` into `{{trigger.projectId}}`). Does nothing until a project
    links Jira.
  - **Done moves the Jira issue** (on, 7e-2): `ticket.stage_changed` to `done` with a `jiraKey` →
    `jira.transition` (`ticket`: the project's done mapping). Nothing else is pushed to Jira.
- What the enabled workflows will tell an attempt's agent is also in that agent's instructions (pdt-core
  `workflow/news.rs`, [agents.md](agents.md) "Instructions and guides"), so turning one off changes what new sessions
  are told.

## Catalog

`GET /api/workflows/catalog` lists every kind: `id`, `version`, `category`, `title`, `description`,
`configSchema` / `outputSchema` (JSON Schema from schemars), `ports`, `dynamicPorts`. One module per kind in
pdt-api `workflow/nodes/` (Radarr and Sonarr share `arr.rs`), registered in `catalog.rs` `catalog()`; config types also
derive ts-rs (web/src/api/gen). After the kinds come the MCP tools' entries ("MCP tools as actions"), which have a
`preset`.

| Kind | Ports |
|---|---|
| one trigger per event type; `manual` (Run now, sub-workflows) — config `projects`, `where`, `runKey` | `out` |
| `condition` (`conditions`, `match: all \| any`) | `true`, `false` |
| `switch` (`value`, `cases [{port, equals}]`) | each case, `default` |
| `gate` (`question`, `choices`, `ahead [{when, choice}]`, `ask: wait \| skip`, `timeoutMs`, `inbox: gate \| accept`, `pipelineStage`, `approve`) | each choice, `unanswered`, `timeout` |
| `agent` (`persona`, `prompt`, `outcomes [{id, description}]`, `session: new \| reuse`, `ticket`, `attempt`, `maxTurns`, `maxRuntimeMs`, `nudges`, `loopLimit`, `owner: agent \| user`, `pipelineStage`; below) | each outcome (`dynamicPorts`), `timeout`, `stopped` |
| `wait` (`ms`, `until`, or `event {type, where}` with `timeoutMs`) | `done`, `event`, `timeout` |
| `workflow.call` (`workflow`, `input`) | `done`, `failed` |
| `end` (`quiet`, `output`) | — |
| `state.set` (`key`, `value`, `ifMissing`; or `add`: a counter, output `value`) | `out` |
| `session.send` (`attempt` or `session`, `text`, `dedupe`, `timeline: none \| pr`) | `sent`, `untold`, `skipped` |
| `ticket.finish` (`ticket`, `stopRunning`, `deleteBranch`) | `finished`, `blocked` |
| `ticket.move` (`ticket`, `stage`), `ticket.comment` (`ticket`, `body`) | `out` |
| `attempt.merge` (`attempt`, `style`, `finishOnMerge`, `deleteBranch`, `again`; Phase 7c) | `merged`, `queued`, `refused` |
| `attempt.agent_working` (`attempt`, `known`; PDT-104): an agent step on the attempt now (bound, or waiting for a slot, in any other run) | `yes`, `no` |
| `schedule` (`every` or `cron` + `timezone`, `catchUp`, `where`, `runKey`; fires on its own `schedule.fired`) | `out` |
| `webhook` (`hook`: a `webhook` connection, `where`, `runKey`; its own `webhook.received`) | `out` |
| `github.issue.opened` (`repos`, else the projects' repos; `projects`, `where`, `runKey`) | `out` |
| `command` (`program`, `args`, `cwd: scratch \| project \| attempt \| <full path>`, `input`, `env`, `parse: json \| text`) | `out` |
| `http.request` (`connection`, `method`, `url`, `headers`, `query`, `body`, `expect: 2xx \| any \| [codes]`) | `out` |
| `discord.post` (`connection`: `discord_webhook`, `content`, `username`, `embeds`, `thread`) | `out` |
| `discord.channel.ensure` (`connection`: `discord_bot`, `name`, `category`, `topic`) | `out` |
| `notify` (`title`, `body`, `level: info \| warning \| error`, `link`) | `out` |
| `jira.import` (`key`, `project`: a PDT project, `connection`: default the project's) | `imported`, `exists` |
| `jira.create` (`connection`, `project`: a Jira project key, `issueType` default `Task`, `summary`, `description`, `fields`) | `out` |
| `jira.transition` (`connection` + `key`, or `ticket`; `to`: a transition or status, with `ticket` default the project's done mapping) | `out`, `unavailable` |
| `jira.comment` (`connection`, `key`, `body`) | `out` |
| `buildkite.retry` (`connection`, `build`: a build URL, or a number with `pipeline`) | `out` |
| `buildkite.trigger` (`connection`, `pipeline`, `branch`, `commit` default `HEAD`, `message`, `env`) | `out` |
| `radarr.search` / `sonarr.search` (`connection`, `term`, `limit`) | `out` |
| `radarr.add` (`connection`, `tmdbId`, `qualityProfile`, `rootFolder`, `search`, `minimumAvailability`) / `sonarr.add` (`connection`, `tvdbId`, `qualityProfile`, `rootFolder`, `monitor`, `search`) | `added`, `exists` |
| `radarr.queue` / `sonarr.queue` (`connection`) | `out` |
| `plex.search` (`connection`, `query`, `limit`), `plex.recently_added` (`connection`, `section`, `limit`), `plex.refresh` (`connection`, `section`) | `out` |
| `mcp.tool` (`server`: an `mcp` connection, `tool`, `arguments`) | `out` |
| `brief.archive` (`brief`, `title`, `body`, `items [{title, link, note}]`, `date`), `brief.read` (`brief`, `since`, `last`) | `out` |

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
- Jira actions (`workflow/nodes/jira.rs`, client in `jira.rs`) sign in with the connection's email and API token
  (basic auth, REST v3); text goes as ADF (a paragraph per blank-line-separated block, line breaks kept).
  `jira.import` makes a Draft ticket (title the summary, description the issue's text) linked to the issue, or leaves
  by `exists` (quiet) when a ticket already links it. `jira.transition` counts an issue already in `to` as done
  (quiet), picks the transition named `to` or the one leading to that status, and otherwise leaves by `unavailable`
  (quiet, with the `available` names); a ticket without a Jira link also leaves by `unavailable`. `jira.create`
  outputs `{key, url}`, `jira.comment` `{key, commentId}`.
- Buildkite actions (`workflow/nodes/buildkite.rs`, client in `buildkite.rs`; bearer token, the connection's `url`
  or `https://api.buildkite.com`, `config.org`): `buildkite.retry` retries the build's failed retryable jobs (output
  `retried`), else rebuilds it (`rebuilt`, the new build); `buildkite.trigger` starts a build (message default
  "Started by <workflow>"). Both output `{number, url}`.
- **Radarr and Sonarr** (`arr.rs`; connection `url` the app's address, secret its API key, sent as `X-Api-Key` to
  `/api/v3/…`). Search: `{results [{tmdbId | tvdbId, title, year, inLibrary, id, overview}]}` from `…/lookup` (10 by
  default). Add: if the library has that id (`GET /movie?tmdbId=` / `/series?tvdbId=`, filtered again for versions that
  ignore it) it takes `exists`; else it looks the film or series up, sets the quality profile (the node's name or id,
  else the connection's `config.qualityProfile`, else the first profile), the root folder (node, `config.rootFolder`,
  else the first), `monitored`, Radarr's `minimumAvailability` (default `released`) and `searchForMovie`, or Sonarr's
  `monitor` (default `all`), `seasonFolder` and `searchForMissingEpisodes` (`search`, default true), and posts it:
  `added`, the film or series as output. Queue: `{items [{title (film and year, or series and SxxEyy), release,
  progress (percent), eta, timeLeft, status}]}`.
- **Plex** (`plex.rs`; `url` the server, secret the `X-Plex-Token`, JSON asked for). Search: `/hub/search`, the hubs'
  items flattened: `{items [{title, type, year, ratingKey, section, show, addedAt}]}`. Recently added:
  `/library/sections/<key>/recentlyAdded` (or `/library/recentlyAdded`), `limit` default 20. Refresh:
  `/library/sections/<key>/refresh` (or `all`) → `{section}`. A section is its key or its title (looked up in
  `/library/sections`).
- **Brief archive** (`brief.rs`): `brief.archive` appends to `<data dir>/briefs/<brief>/<date>.md` (a `# <brief> —
  <date>` heading on a new day, then `## <title or time>`, the body, the items as a list `[title](link) — note`) and a
  line `{date, at, title, items [{title, link}]}` to the brief's `index.jsonl`; `date` defaults to today in the host's
  zone. `brief.read` returns `{entries, days [{date, markdown}]}`: the index entries on or after `since` and/or the
  newest `last` (default 7 without `since`), oldest first, and their days' Markdown (256 KiB at most). Brief names are
  `[a-z0-9_-]`, dates `YYYY-MM-DD`, so nothing leaves the folder.
- `discord.channel.ensure` (pdt-api `discord.rs`): makes a text channel through a bot connection unless the server has
  one called that (names as Discord makes them: lower case, dashes); a new one goes in `category` (made when
  missing). Output `{channelId, name, created}`.
- Dry runs never run a command or call out: they report `would` (the Radarr, Sonarr, Plex and MCP reads too;
  `brief.read` reads its files as usual).

### MCP tools as actions (Phase 7e-3)

An MCP server is a connection of kind `mcp` (pdt-api `mcp_client.rs`, the `rmcp` client): `config.transport` `stdio`
(`command`, `args` a list, `env` an object, `cwd`; the secret, if any, in the env var `config.secretEnv`; no console
window on Windows, where a `.cmd` such as `npx` needs `cmd /c`) or `http` (the connection's `url`, streamable HTTP, the
secret as a bearer token); without `transport`, a URL and no command means `http`.

- **Clients** (`McpPool` in `AppState`): one per server, started on first use (a 30 s limit on the handshake; a stdio
  server that fails says its last stderr lines), shared by concurrent calls, started again when the connection's
  settings or secret change or after a transport error, stopped after 5 minutes idle (`maintain_workflows`) and when
  pdtd stops. A stdio server's stderr goes to pdtd's log at debug level.
- **Tools** (`tools/list`) are cached in memory by connection name: listed in the background when an `mcp` connection
  is saved, and again by `maintain_workflows` once the last try is 10 minutes old. A failed listing keeps the tools from
  before and records the error. `GET /api/connections/{id}/tools` (the cache: `{server, tools [{name, title,
  description, inputSchema, outputSchema}], error, listedAt, triedAt}`, null before the first listing), `POST` lists
  now. Deleting or renaming a connection drops its cache.
- **`mcp.tool`** calls `tools/call`; its output is the tool's `structuredContent` if it has any, else its text content
  (joined; parsed when it is a JSON object or list). A result with `isError` fails the step with its text.
- **Catalog entries.** `GET /api/workflows/catalog` (and pdt-mcp's `catalog_search` / `catalog_get`) add one entry per
  cached tool: id `mcp:<server>/<tool>`, its title (or name) and the server, its description, `mcp.tool`'s config
  schema with the tool's input schema as `arguments`, its output schema, and `preset: {kind: "mcp.tool", config:
  {server, tool, arguments: {}}}`. Adding one in the builder makes that `mcp.tool` node (titled after the tool, its
  required arguments started), and the node panel shows the tool's arguments.
- **Validation** (validate, check, dry run and a workflow's issues) warns (`mcp_tool`) when a server whose tools are
  cached lacks the node's tool, or the `arguments` miss a required property, have one the schema refuses
  (`additionalProperties: false`) or a literal of the wrong type at the top level. References are checked when the step
  runs (by the server). Warnings don't block saving.

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
- `POST /api/workflow-runs/{id}/retry` (a failed run, from its failed step), `/resume` (a run whose latest agent step
  ended by `timeout` or `stopped`, from that step: [Timeouts and Resume](#timeouts-and-resume-pdt-123); `409
  cannot_resume` otherwise; `GET` gives that step as `resumeFrom`), `/cancel` (also closes the run's Inbox items), and
  `/steps/{step}/answer {choice, text?}` (a waiting gate; the user's `text` goes on the run's ticket as their comment,
naming the workflow, run and choice, so "Ticket comments to the agent" forwards it, and is in the gate's output). `GET /api/workflow-runs?workflow=&ticket=&status=&limit=`,
  `GET /api/workflow-runs/{id}` (with steps).
- Per-workflow state: `workflow_state`, `GET /api/workflows/{id}/state`. `state.set` with `add` counts (none or not a
  number is 0; a dry run reads the count but doesn't write it). Keys starting `_pdt.` are PDT's own (a
  schedule's last slot): `{{state}}` leaves them out and `state.set` refuses them.

## Outbox (`session.send`)

`session_messages`, one per step. `outbox.rs` delivers with the PR notices' old rule: the attempt's newest running
agent session (a terminal with a live PTY, or a conversation); a terminal idle with no typing for 20 s, a conversation
`ready_for_notice` (idle, nothing queued, no open request, no rate limit, terminal closed; a parked one reopens); all
of a session's waiting messages in one message; no running agent session → `untold`. What the user sent to a
session (an Inbox answer, a Nudge) goes to its conversation even when that is stopped or interrupted (a pdtd restart),
reopening it as typing would. `dedupe` is per attempt (also
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
boolean | choice | connection | list, choices, default, required, connectionKind}` (`connection`: a connection's name,
of `connectionKind` when given; `list`: a list of text, given as a JSON list or one comma-separated string); the document uses `{{params.<id>}}` (a string that is exactly one
reference takes the value with its type; other references stay). Saving checks the params and validates the document
with defaults or placeholders filled in. Built-in templates (`builtin:<id>`, read-only, `409 builtin_template`):
Blank, Tell the agent when CI fails, Ask before finishing a merged ticket, Comment when a pull request merges, the
Pipeline (below), the Hermes jobs of 7e (New GitHub issues to Discord; Health watchdog: a URL on a schedule, Discord
only on a change, the first check silent; Brief follow-ups and Email processing: a script on a schedule whose
`{"message": …}` goes to Discord), Triage new GitHub issues (PDT-81: `github.issue.opened` → an `agent` step of the
PM persona, `{{params.persona}}`, with outcomes `drafted` (it made a draft ticket in `{{params.project}}` with
`create_ticket`, which asks first unless the project lets the PM act), `duplicate` and `ignored`, the last quiet), and
the built-in workflows. A required text param with no default is checked as
the reference itself. The user's are in `workflow_templates`.

Connections: kinds the actions know are `discord_webhook` (secret: the webhook URL; one the PM made has
`config.webhookId`, `channelId`, `channel` and `bot`), `discord_bot` (secret: the bot token; `config.guildId`, checked
on save, `applicationId` for the invite link, `pmActs`; `url` empty means Discord's API, tests point it at a fake),
`webhook` (secret: the shared secret), `http` (`url`, `config.auth` and its field; secret: the token or password),
`jira` (`url` the site `https://<site>.atlassian.net`, `config.email`; secret: an API token), `buildkite` (`url`
empty for Buildkite's own API, `config.org`; secret: an API token with read builds, and write builds to retry or start
them), `radarr` and `sonarr` (`url`, `config.qualityProfile`, `config.rootFolder`; secret: the API key), `plex` (`url`;
secret: the token) and `mcp` ("MCP tools as actions"; `GET/POST /api/connections/{id}/tools`, `409 not_mcp` for
another kind). A config field holding a connection's name says its kind in its schema (`x-connection-kind`:
`discord_webhook`, `discord_bot`, `webhook`, `jira`, `buildkite`, `radarr`, `sonarr`, `plex`, `mcp`, `any`), so forms offer a picker. `GET/POST /api/connections`, `GET/PATCH/DELETE /api/connections/{id}` (`name` unique, `kind`, `url`,
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
- **Jira** (`jira.rs` `refresh_jira`, pdtd every 60 s): each project whose settings link Jira (a `jira` connection,
  a JQL scope, the done transition) is searched with `POST /rest/api/3/search/jql`, `(<scope>) AND updated >= "-<n>m"
  ORDER BY updated ASC` (relative, since JQL dates are in the Jira user's zone; n covers the time since the last look
  plus a minute), with the changelog, up to 5 pages of 100. The last look is `sensor.jira.<project>` (`since`, the
  connection and scope; the first look, or a new connection or scope, only records it). Per issue, news since then
  less a minute (keys make re-reads harmless): `jira.issue.created` (key `jira:<connection id>:<key>:created`),
  `jira.issue.updated` (the fields changelog entries changed, `changed`; key `…:updated:<updated>`; not for an issue
  created in the window), `jira.issue.transitioned` (`from`, `to`; key `…:status:<history id>`) and
  `jira.issue.commented` (`commentId`, `author`, `body` as text, `byMe`; key `…:comment:<id>`). Each carries the
  issue (`projectId`, `connection`, `key`, `url`, `summary`, `description` as text, `status`, `assignee`,
  `assignedToMe` against `GET /rest/api/3/myself`, `reporter`, `issueType`, `priority`, `labels`) and `ticketId` when
  a ticket links it.
- **Buildkite** (`buildkite.rs` `refresh_buildkite`, pdtd every 60 s after the PR check): for each active attempt of
  a project whose settings name a `buildkite` connection and pipelines, the head is its open PR's head, else
  `origin/<branch>`, else the local branch. Each pipeline's builds for that branch and commit become CI runs (pdt-core
  `buildkite.rs`: `workflow` the pipeline, `id` the build number, `attempt` 1 plus the jobs' retries, so a retried
  build is news again; `failing` counts as running, `blocked` as queued; failed jobs are the hard-failed script jobs),
  rolled up per pipeline as GitHub's are. Kept in memory: merged into `attempt.pr.ci` while the PR is open at that
  head (one badge for both providers), else `attempt.ci`. Events `ci.run_failed` (key `<attempt>:bk_failed:<pipeline>
  #<number>.<attempt>,…`, a CI-failed `notice` naming the build) and `ci.run_passed` (`…:bk_passed:…`), `provider:
  buildkite`, `pr` when there is one, so the built-in PR notices tell the agent.
- **Notify** (`workflow/nodes/notify.rs`): `pdt.notified` (`title`, `body`, `level`, `link`, `workflowId`,
  `workflowName`, `runId`; key per step).

## Web

Connections (`web/src/connections/`): Settings → Connections (its own page, under This host in the Settings list)
has a section per kind (Discord webhooks, Discord bots, Incoming webhooks, Jira Cloud sites, Buildkite organizations,
HTTP services, Radarr, Sonarr, Plex servers, MCP servers, and Other for kinds no action uses), each saying how to set
one up, with Add, and each connection's Edit and Delete (a missing secret and a webhook's address `/api/hooks/<name>`
show on its row; an MCP server's row shows its tools, or why listing them failed, with List again). The dialog asks
what the kind needs (`kinds.ts`: its address, and its own settings: Jira's email, Buildkite's organization, Radarr's
default quality profile and root folder, an MCP server's transport, program, arguments, environment and secret
variable; a setting shown is required unless marked optional) and
says where its secret comes from (Discord: the channel's settings → Integrations → Webhooks → New Webhook → Copy
Webhook URL); an incoming webhook can generate its secret. A Discord bot asks for the server's id, the application's id
(which makes the invite link with View Channels, Manage Channels, Send Messages and Manage Webhooks) and whether the PM
changes the server without asking. Saving keeps config keys the dialog doesn't show. The secret is written to the keychain and never shown again
(empty when editing keeps it). A template's `connection` param and a node field with `x-connection-kind` use
`ConnectionPicker`: the connections of that kind, and Add… for a new one.

To post to Discord: add a Discord webhook connection (above), then New workflow → a template that posts (New GitHub
issues to Discord, Health watchdog, …) and pick it as the Discord channel, or add a Post to Discord step and pick it in
the step's panel. Or add a Discord bot connection and ask the PM to set up the channel and its webhook
([agents.md](agents.md#the-pm-pdt-81)).

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
  output, error), Retry from the failed step, Resume from a timed-out or stopped agent step, Cancel, and a waiting
  gate's choices.
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
  and version (`sessions.persona_id`, `persona_version`, `step_id`), with no attempt (so PR notices still go to the
  attempt's own session) but linked to the ticket (`sessions.ticket_id`, PDT-105): it is titled `<Persona> · <KEY>`,
  carries the ticket like an attempt's session and is in the ticket's `sessions` (with `persona`, `attemptId` null).
  `reuse` (Coder): the session this node used earlier in the run, else the attempt's running agent session, else a new
  one that becomes the attempt's. Every new session a step starts on a ticket opens its tab at the end of the
  attempt's Start workspace (after the ticket's tab and the sessions before it: Coder, Reviewer, QA), else the
  project's workspace (a Refiner before any attempt, or the workspace is gone), as a session the user's Start makes
  would (PDT-103, PDT-105); each pass's new session opens its own tab, and earlier passes' stay like other ended
  sessions. A step with no ticket opens no tab. A reused session gets the step through the outbox
  (never into a running turn), with the step's rules in the message since its instructions were fixed at its start.
  On a later pass in the same session (sent back by Review, QA or a rejection) it gets only why it is back (the
  sending step's outcome and summary, or the user's gate choice and note: pdt-core `next_pass_message`) and the
  outcomes, not the task and rules again.
- **Ticket and attempt**: the config's, else the trigger's `ticketId` / `attemptId`, else the ticket's active attempt.
  A step with neither runs in the run's (or workflow's) project.
- **Permission mode** of a new session: the project's `agentPermissionMode` (project settings), else the persona
  version's, else the host default (`ticketPermissionMode` on a ticket, `sessionPermissionMode` otherwise).
- **Instructions and first message.** The session's instructions get a "Your role: <persona>" section (the persona's
  instructions, the outcomes, and the rule: finish with `submit_outcome`, ask with `ask_user`, a turn with neither
  counts as stalled). The first message is the node's `prompt`, the ticket's key and title, and the outcome list.
- **Binding.** `agent_steps` (one per step: session, persona and version, project, ticket, attempt, outcomes,
  `state` `working | asking | stalled | done`, turns, nudges) is what pdt-mcp and the Inbox read. The step waits
  (`wait: {agent: …}`) until `maxRuntimeMs` (4 h), then `timeout` (the Inbox has it: [Timeouts and
  Resume](#timeouts-and-resume-pdt-123)); a step re-run after a restart waits again.
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
  questions: …" and the binding works again. A session with no step can ask too. Removing the asking session closes
  its question (nothing can answer it) and the engine tick stalls its step, so it can be taken over.
- **Loop limit.** When the node is about to run more than `loopLimit` (3; 0: none) times again in one run, it waits
  with a `loop_limit` Inbox item instead (the earlier passes' outcomes and summaries): **Once more** runs it (that
  pass only), **Take over** completes it with an outcome (`by: user`), **Stop** takes `stopped`.
- When a step completes or its run ends, its open Inbox items close and its binding is done.

### Timeouts and Resume (PDT-123)

- An agent step that takes `timeout` (its `maxRuntimeMs`, or `maxTurns` turns without an outcome) opens a
  `timed_out` Inbox item (group Stalled): the ticket, the step's node and workflow, why (`body.why`), its session
  (Open session, while it exists), and **Resume** (primary, with an optional note) and **Stop**. The step has ended,
  so the item stays open after its run does (the Pipeline's timeout path ends the run), until it is answered, the run
  is resumed or cancelled.
- A run can be resumed from its latest agent step when that one ended by `timeout` or `stopped` (pdt-core
  `resumable_step`; not once a later agent step ran, nor a finished sub-workflow's run). **Resume** (the Inbox item,
  or Workflows → Runs "Resume from <node>", `POST /api/workflow-runs/{id}/resume`) skips what the run still had to
  do after that step, runs the node again as a new step of the same run (same trigger, so the same ticket, attempt,
  branch, worktree and PR; never a new attempt) without counting against its loop limit, and resolves the step's
  item. The new step goes on in the timed-out step's session while that is there (a conversation, which reopens if
  stopped, or a running terminal), whatever the node's `session`, with "[PDT] The user resumed your step after it
  stopped: …" (pdt-core `resume_message`; the user's note first) sent as the user's; with no session left it starts
  as a first pass would (a `reuse` node finds the attempt's session). **Stop** resolves the item and cancels the run
  if it hasn't ended; the run can still be resumed from Runs.
- **Late outcome**: a `submit_outcome` from a session with no open step, whose newest step timed out and is still
  the run's resumable step, is accepted: the run is taken back to that step as for Resume and goes on from the
  outcome's port (a new step of the node, ended by the outcome, with the usual timeline record), and the timed-out
  item is resolved. Otherwise it is refused as before ("no workflow step is waiting…").

## Personas

`personas` / `persona_versions` (pdt-store `personas.rs`, pdt-api `personas.rs`): a name, role and description, and
versions of agent, model, instructions, permission mode (null: project override, else host default) and tools (pdt-mcp
tools; empty: the default five). Every save (`PUT /api/personas/{id}`) is a new version; `PATCH` changes name, role,
description, `archived`; `GET /versions`, `/versions/{v}`, `/diff?from=&to=` (the fields that changed and a unified
diff). Built-ins Refiner, Coder, Reviewer, QA and PM (the project manager, [agents.md](agents.md#the-pm-pdt-81);
permission mode `ask`, the PM's tools) (pdt-core `persona.rs`) are installed on the engine's first tick (and
on `GET /api/personas`) and can be archived, not deleted (`409 builtin_persona`); a persona a workflow names can't be
deleted (`409 persona_in_use`).
- **Newer templates** (PDT-100): `personas.builtin_template` records the template PDT applied last (until recorded,
  the newest version PDT wrote). Once per daemon run, when the template changed, each field still as that template
  (agent, model, permission mode, tools as the set the persona gets, instructions as a whole) follows the new one and
  each field the user changed stays theirs (pdt-core `follow_template`): a new `builtin` version noted "updated by
  PDT" (plus "; kept your …" naming kept fields whose template moved on), none when nothing would change.
- `GET /api/personas/{id}/template` (`404 not_builtin` otherwise) → `{setup, diff}`: the current template and the
  current version's diff to it. The persona page shows it for a built-in that differs (the fields, **Show the
  changes**) and **Load PDT's template** fills the setup form with it to review and save as a version.
- `GET /api/personas/tools` (PDT-110) → every pdt-mcp tool (pdt-core `ALL_TOOLS`, in order) as `{name, does, asks,
  default}`: the persona page's tool list, so a new tool shows there with no web change. Saving keeps every tool the
  persona has, a name the server doesn't list included (shown checked, to uncheck).

## Pipeline, policies and scheduling (Phase 7c)

- **The Pipeline template** (`builtin:pipeline`, pdt-api `workflow/pipeline.rs`): instantiated per project (off until
  enabled). `ticket.refine_requested` → Refine (agent `{{params.refiner}}`, new session, `pipelineStage: refine`):
  `ready` → `ticket.move` to `{{params.afterRefine}}` (To do by default); `needs_clarification` → a gate (refine again
  | stop). `attempt.started` → Code (Coder, `session: reuse`, `loopLimit: {{params.loopLimit}}`) → Review (new) → QA
  (new); `changes_requested` and `fail` go back to Code; `pass` → a switch on `{{params.accept}}`: `user` → a
  condition (`{{trigger.labels}}` `contains_any` `{{params.autoAcceptLabels}}`): a ticket with one of those labels
  goes straight on, any other to Accept (a gate with `inbox: accept`, `pipelineStage: accept`, `approve: merge`;
  `reject` → Code); `auto` → straight on; then
  a switch on `{{params.merge}}`: `workflow` → `attempt.merge` (`merged` → `ticket.finish`; `queued` → end, Finish
  merged tickets finishes it, and PR notices gets it merging again after a failure, PDT-104; `refused` → wait), `person` → a wait for `attempt.merged` of this attempt. `timeout` and
  `stopped` add a ticket comment and end (a timeout is also in the Inbox, and the run can be resumed: PDT-123). Params: the four personas, `refineBy`/`codeBy`/`reviewBy`/`qaBy` (`agent`
  or `user`: manual), `afterRefine`, `accept`, `autoAcceptLabels` (a list, default none; the labels the ticket had
  when it started count), `merge`, `loopLimit`. Runs may last 30 days. A ticket accepted by label has no approval,
  so with `requireApproval` on its merge is refused (the run waits for someone to merge) unless the label is also
  in `approvalExemptLabels`.
- **Manual stages**: an `agent` node with `owner: user` starts no session. Its binding's state is `manual` and the Inbox
  gets a `manual` item ("Your turn: <node>", group Gates) with the prompt and one `outcome:<id>` choice per outcome;
  answering (with an optional `text`, the summary) completes the step `by: user`.
- **`pipelineStage`** on `agent` and `gate` sets the ticket's stage when the step starts ([tickets.md](tickets.md)).
- **`approve`** on a gate: taking that choice records the user's approval of the run's attempt at its branch head (a
  `review` event naming the workflow), which `requireApproval` needs before an automated merge.
- **`attempt.merge`** runs the Merge command as an automated merger ([review.md](review.md#merge)): a policy refusal
  or anything that stops the merge leaves by `refused` (`code`, `reason`); `merge_failed` is an error. With `again`
  (PR notices after a fix, PDT-104) it reads the PR from GitHub first: one already on auto-merge or in the queue is
  `queued` (`already: true`); and it sends no `finishOnMerge`, so the first merge's choice stays.
- **Policies** (project settings `policy`: `mergeBy` `agents | user | external`, `requireApproval`,
  `approvalExemptLabels` (tickets with one of them need no approval; normalised like ticket labels, `[]` clears),
  `maxRunningAttempts`, `autoStart` `off | todo | ready`; `PUT` takes them as top-level fields, `maxRunningAttempts`
  0 clears it): Merge and Start enforce them ([review.md](review.md#merge), [tickets.md](tickets.md)). Web: the
  project page's "Pipeline and policies".
- **Host cap on agents running a turn** (`maxAgentTurns`, default 4, 0 none): busy sessions are conversations with a
  turn running or a message queued, and agent terminals whose activity is `working`. An agent step that would start
  a new session over the cap waits (`wait: {slot: true, persona}`; its ticket's `agent.state` is `queued`); each
  engine tick admits waiting steps oldest first into the free slots (`scheduler::admit_waiting`). The outbox holds a
  message that would start a turn in an idle session while no slot is free, unless the user sent it
  (`session_messages.by_user`: Inbox answers and nudges). Auto-start's own sessions wait for a slot too.
- **Refine** and **auto-start**: [tickets.md](tickets.md).

## Inbox

`inbox_items` (pdt-store `inbox.rs`, pdt-api `inbox.rs`): `question` (`ask_user`), `gate` (a waiting `gate` node;
group `gate` or `accept` from its `inbox`), `stalled`, `loop_limit`, `manual` (an `owner: user` step, Phase 7c),
`timed_out` (group Stalled, PDT-123); one
open item per step and kind. Approvals are
not stored: the open permission requests of live conversations (`approval:<session>:<request>`), answered in their
session. `GET /api/inbox?project=` → `{items, total}`, Questions, Approvals, Stalled, Loop limit, Gates, Ready to
accept, oldest first within each, with ticket key and title and persona name. `POST /api/inbox/{id}/answer` takes
`{choice}` (a gate's choice, with an optional `text`, the user's comment; `nudge` with an optional `text`, or `outcome:<id>`, for a stalled step; `again`,
`outcome:<id>` or `stop` at a loop limit; `resume` with an optional `text`, the note, or `stop` for a timed-out step)
or `{answers}` / `{text}` for a question; `POST /api/inbox/{id}/dismiss`
closes a question or stalled item (the step keeps waiting). A gate answered through
`/api/workflow-runs/{run}/steps/{step}/answer` resolves its item too. When a session is removed (and when the store
opens, for older ones) its open questions close (`resolution` `{sessionGone: true}`) and no item keeps its
`sessionId`, so the Inbox never offers Open session for a session that is gone (PDT-93).

Web: the rail's Inbox (web/src/inbox; the open count on its icon, `Ctrl+] i`): the list groups items by kind; the work
area shows the item and its decision (answers per question, the approval card, Nudge / Take over, Once more / Take
over / Stop, Resume (with a note) / Stop, a gate's choices, a manual step's outcomes); `j`/`k` move, Enter takes the primary action,
`o` opens the ticket, `e` dismisses. A gate has a note box: the note goes with the choice (as the user's ticket
comment, so a rejection tells the coder what to change). An item on a ticket with an attempt also offers Open coder
session (the attempt's running agent session), Open changes and Run… (the worktree's configured runs and tests); each
opens its tab right after the ticket's tab in the workspace that has it (its Start workspace; AppShell's
`TicketTabsContext`, `openChanges` with `ticketId`), moving an existing changes tab there (a session or run tab that
exists is shown where it is), and switches to it.
