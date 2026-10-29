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
  `detected` / `merged_here` / `automated` / `closed`, `pr`, `finishOnMerge`, `finishDeleteBranch` and the merged `notice`),
  `attempt.abandoned` (every `attempt.*` carries the ticket's `labels` at the time, for `{{trigger.labels}}`
  `contains` `<label>`) and `ticket.overlap_found` (or `ticket.overlap_cleared` for an `overlap` event with
  `cleared`), in the same transaction, except Start's: its `ticket.stage_changed`
  and `attempt.started` are logged once Start has made its session (`announce_start`), so a Start that fails and is
  undone never runs a workflow for an attempt that is gone; an agent step's `outcome` timeline event
  becomes `agent.outcome` (`outcome`, `summary`, `persona`, `personaVersion`, `by`, `sessionId`, `runId`). Phase 7e
  adds the PR poller's `pr.commented` and `pr.reviewed`, the issues sensor's `github.issue.opened`, the scheduler's
  `schedule.fired` and webhooks' `webhook.received` ("Sensors and webhooks"); 7e-2 the
  Jira sensor's `jira.issue.created`, `.updated`, `.transitioned` and `.commented`, and the Buildkite sensor's
  `ci.run_failed` / `ci.run_passed`. Payload types: pdt-core `workflow/events.rs` (PDT's own types in `EVENT_TYPES`; Jira's in its crate, pdt-int-jira `core.rs`, registered by the Jira integration: "Integrations"). CI events carry
  `provider` (`github_actions`, or the CI integration's id: `buildkite`) and the PR's `autoMerge` and `inMergeQueue`; CI and `pr.*` events
  carry `mergeRequested` (the open PR was set to merge and hasn't merged, PDT-104: [tickets.md](tickets.md)) and
  `agentWorking` (an agent step is open on the attempt); CI events also `mergeWaiting` (a workflow's merge waits for
  this CI, PDT-151). The merge check logs `attempt.late_commits` (key `<attempt>:late_commits:<head>`, PDT-151:
  commits pushed after the attempt's PR merged; [tickets.md](tickets.md)). `ticket.stage_changed` carries `external`, the issue in each tracker the
  ticket is linked to by tracker (`{"jira": "ABC-5"}`, `{{trigger.external.jira}}`), and
  `githubIssue` (`owner/name#12`) when it is linked to a GitHub issue.
- PDT-202 ([tickets.md](tickets.md) "Outside input in prompts"): each event type names its outside-text fields
  (`EventType.outside`), fenced in what goes to an agent; `pr.commented` / `pr.reviewed` carry `outside` (the author
  isn't a collaborator on the repository: never sent into a session); the engine logs `triage.skipped` for a triage
  run over the hourly cap, and `session.outside_input` records outside text reaching a session.
- The timeline and the conversation events stay as they were; a `pr` timeline event is now the record of a message a
  workflow delivered (`told`, `sessionId`, plus `workflowId`, `workflowName`, `runId`).
- `GET /api/events?type=&project=&ticket=&attempt=&limit=` (newest first) or `&after=<seq>` (oldest first, to
  follow); `GET /api/events/{id}`.

## Workflows

- A document (`WorkflowDoc`): `name`, `description`, `nodes` (`id`, `kind`, `version`, `config`, `position`, `retry`
  `{max, backoffMs}`, `timeoutMs`, `maxVisits`), `edges` (`from`, `port`, `to`) and `settings` (`maxSteps`,
  `maxRuntimeMs`). Config strings take `{{trigger.x}}`, `{{steps.<node>.x}}`, `{{state.x}}`, `{{run.id}}`; a string
  that is one reference keeps the value's type. `{{event.x}}` is left for a wait's event conditions, and
  `{{secret.x}}` and `{"$secret": "x"}` for the moment the node runs ([secrets.md](secrets.md#workflows)); steps' and
  runs' stored inputs, outputs and errors are redacted of known secrets' values.
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
  - **PR notices** (on; a Pipeline's own refused merge is fixed and reviewed in the Pipeline, PDT-124: see
    "Pipeline"): `pr.merged`, `attempt.merged` (not `via: closed`) → `session.send` (`text:
    {{trigger.notice.text}}`, `dedupe: {{trigger.notice.key}}`, `timeline: pr`). `ci.run_failed` (not
    `mergeWaiting`: a workflow's merge waiting for that CI handles it, PDT-151), `pr.conflict`,
    `pr.queue_left` and `pr.behind` → a condition on `{{trigger.mergeRequested}}` or `{{trigger.mergeRefused}}`
    (PDT-104; PDT-139: a workflow's first `attempt.merge` refused as `fixable` counts, see below): neither → the
    same `session.send`; set to merge but an agent on it (`attempt.agent_working`, read as the step runs, so a
    conflict and the queue drop it caused, logged in one look, get one fix) → the `session.send` too; else
    `state.set` `tries.{{trigger.attemptId}}` `add: 1`, and over 3 (`FIX_TRIES`) → a `ticket.comment` the first time
    (4) → the `session.send`; within 3 → `fix` (agent, Coder, `session: reuse`; the prompt is the notice plus "this
    pull request was set to merge…"; outcomes `submitted`: pushed, `review`: a large fix, committed, not pushed) →
    `submitted` → `remerge` (`attempt.merge` with `again`); `review` → `review` (agent, Reviewer, its own session):
    `approved` → `push` (agent, Coder, reuse: push it; `submitted` → `remerge`), `changes_requested` → `fix`.
    `timeout` / `stopped` of these → a ticket comment. `remerge` `refused` for CI (`ci_pending` / `ci_failed`,
    PDT-151: the push's CI hasn't passed; the merge node records it as set to merge) → a quiet end, as CI passing
    remerges it below and a failure is a new notice; refused otherwise → a ticket comment with the reason.
    `ci.run_passed` where `pr` exists, `mergeRequested`, not `autoMerge`, not `inMergeQueue`, not `agentWorking` →
    `remerge` (only `mergeRequested`: a refused merge alone never turns auto-merge on for an unreviewed fix,
    PDT-124). The `session.send`'s `untold` (no running agent session for the attempt: none, or all `interrupted`,
    PDT-139) → a condition that the notice is not `merged` (`false` → a quiet end) → `inbox.notice` (one Inbox item per
    attempt and notice key, below) → `added` → a `ticket.comment` with the notice text; `skipped` ends there. An
    untold notice does not spend its key (PDT-129), so the same notice again tries the agent again (told if a session
    is there by then) and, still untold, stops at `inbox.notice`'s `skipped`.
    An untouched PR notices follows this template, so an enabled one picks it up on pdtd's next start.
  - **Finish merged tickets** (on): `attempt.merged` (`via` `detected`, or `automated`: a workflow's or an agent's
    local merge, PDT-181; a person's own Merge… finishes the ticket itself) → gate answered ahead by `finishOnMerge`
    (Merge…'s "Move to Done and clean up once it merges", PDT-75), else `unanswered` → quiet end (the merged banner
    asks) → `ticket.finish` (`stopRunning`, `deleteBranch: {{trigger.finishDeleteBranch}}`); `blocked` (uncommitted
    changes) → quiet end.
  - **Commits pushed after a merge** (on, PDT-151): `attempt.late_commits` → `session.send` to the attempt
    (`{{trigger.notice.text}}`: its push came after the merge, push nothing more) and, side by side, a gate in the
    Inbox (`{{trigger.summary}}`, `event`: this attempt's `attempt.merged` or `attempt.abandoned` ends it):
    `open_follow_up` → `ticket.create` (To do, "Follow-up to <key>: commits pushed after PR #n merged", the commits
    and `git cherry-pick <merged head>..<head>` in its description) → a comment naming it → `ticket.finish`
    (`stopRunning`, keeping the branch for the cherry-pick); `discard_and_finish` → a second gate ("Discard …? …
    deletes its branch"): `discard` → `ticket.finish` with `deleteBranch`, `back` → the first gate; `keep_open` →
    quiet end. A blocked finish (uncommitted changes, or a session or run that didn't stop) comments on the ticket. Runs may last 30 days.
  - **Ticket comments to the agent** (on): `ticket.commented` by the user (`actor` = `user`, not an agent's comment
    through the API or a workflow's) with an active attempt → `session.send` (`reopen: true`: with no running agent
    session, the attempt's newest conversation that wasn't stopped, e.g. one a pdtd restart interrupted, reopens to take
    it, PDT-144). A note given with a gate's choice is the user's comment too, but carries `workflowId` and is skipped: that run passes it on itself.
  - **Base moved while a PR waits** (off, PDT-24): `pr.behind` → `session.send`. PR notices now handles `pr.behind`
    too (PDT-104), so with both on the agent hears it twice.
  - **PR comments to the agent** (off, 7e): `pr.commented` and `pr.reviewed` (not `byOwner`, not `isBot`; reviews
    not `approved` and with a body) → `session.send` (`{{trigger.notice.text}}`, dedupe on the notice key, `timeline:
    pr`).
  - Each enabled integration's built-ins (PDT-173: installed when it is enabled, and listed with PDT's): Jira's two.
    A turned-off integration's built-ins (every install from before PDT-173 has Jira's, enabled) are left out of
    `GET /api/workflows`, the engine and the PM's workflow lookup, as they can't run and built-ins can't be deleted;
    they come back as they were when it is turned on (`builtins::hidden`).
  - **Import assigned Jira issues** (on, 7e-2): `jira.issue.created` and `jira.issue.updated` where `assignedToMe`
    and no `ticketId` → `jira.import` (`{{trigger.key}}` into `{{trigger.projectId}}`). Does nothing until a project
    links Jira.
  - **Done moves the Jira issue** (on, 7e-2): `ticket.stage_changed` to `done` with an `external.jira` →
    `jira.transition` (`ticket`: the project's done mapping). Nothing else is pushed to Jira.
  - **PDT bug triage** (PDT-154; installed, on, and offered as a template only where a project's repository is
    `RELEASE_REPO`, PDT's own (PDT-226; `builtins::only_for_pdt`); one installed elsewhere before that stays): `github.issue.opened` on `RELEASE_REPO` where `labels` contains `from-pdt` (the bug reports other
    PDT instances file, PDT-153) and `projectId` exists → an `agent` step (the locked-down Triage persona, PDT-201, a
    new session in that project, no ticket) that triages against the code and the tickets, changing nothing itself, with
    outcomes `accept` (`data` `{priority, labels}`), `duplicate` (`data` `{ticket, duplicateOf}`) and `reject` (the
    summary is the reason, for the reporter). `accept` → `github.import` (the suggested priority and labels but
    `agentic`; an external Draft only the user moves to To do) → a
    `ticket.comment` with the triage notes → `github.comment` with the ticket key. `duplicate` → `github.close`
    `duplicate` of `duplicateOf` (the issue linked to the existing ticket, else the earliest one reporting the same
    bug; a comment names it and the ticket), or `not_planned` naming the ticket when there is none → a
    `ticket.comment` on the existing ticket noting the extra report; no ticket is made. `reject` → `github.close`
    `not_planned` with the reason → `inbox.notice` (no attempt) so the user can overrule it (reopen and import).
  - **Done closes the GitHub issue** (on, PDT-154): `ticket.stage_changed` to `done` (→ `github.close` `completed`,
    commenting "Fixed in <key>; it ships in the next PDT release") or `wont_do` (→ `not_planned`, with a comment),
    both where `githubIssue` exists and with `runKey` `{{trigger.ticketId}}`: once per ticket, so moving it again
    changes nothing on GitHub. The release isn't known when a ticket moves to Done, so the comment doesn't name one.
  - **Cut a release** (off, PDT-194, PDT-214): `manual` (Run now; the PM's `workflow_run` when the user asks for a release)
    with input `projectId` and `bump` (`patch`, `minor`, `major`) or `version` (`X.Y.Z`), and `channel` → `condition`
    (`channel` is `experimental`?; PDT-210). Experimental: `github.release` with `prereleases` (the newest before,
    `exp_before`) → `github.workflow.run` (`release.yml` on `main`, inputs `bump`, `version` and `channel:
    experimental`; no release PR, so no review and no check for an open one) → every 2 minutes (four hours)
    `github.release` with `prereleases` and `newerThan: {{steps.exp_before.tag}}` until the pre-release is published →
    `notify` and an `inbox.notice` (`released-<tag>`, "Experimental release <tag> is published") → end with
    `{tag, url}`. Otherwise (stable, the default) → `github.release` (the
    latest before, `before`) → `github.change_request` on branch `release` (`dryRunPort: none`): `open` (an earlier
    release PR still open, which the Action would refuse to replace) → `inbox.notice` (`release-pr-open-<n>`: "Release
    PR #n is still open": it merges by itself once its checks pass, or fix or close it, then run it again) → end `not_started`; otherwise → `github.workflow.run` (the project's `release.yml` on `main`, inputs `bump` and
    `version`; empty ones are left out) → every 30 s (40 times) `github.change_request` on branch `release` until one
    is `open` (`none`, and the last release's `merged` or `closed` PR, keep looking) → `notify` ("Release PR #n is
    open", informational: it merges by itself) → every 5 minutes (two weeks: the checks with the bundle builds, the
    merge queue, and time for a fix) `github.change_request` with `ci` (number `n`, branch `release`): `open` keeps
    waiting; `failed` (a check failed on its head, or the merge queue removed it for failed checks) →
    `inbox.notice` (key `release-pr-<n>-<ci|queue>-<head sha>`, so once per failure: "Release PR #n failed its
    checks", the run's link and the failed jobs; push a fix to `release` or close it) and a warning `notify` → keeps
    waiting; `closed` or `none` → `notify` (warning: closed without merging, nothing released) → end; `merged` →
    every 2 minutes (four hours)
    `github.release` with `newerThan: {{steps.before.tag}}` until the tag's release is published → `notify` and an
    `inbox.notice` (`released-<tag>`, "Release <tag> is published") → end with `{tag, url}`. The GitHub reads retry 5
    times a minute apart; `maxRuntimeMs` 16 days. PDT never merges or tags anything here: the Action turns on the
    release PR's auto-merge, the merge queue merges it once its checks pass, and the merge's tag publishes the release
    ([SDLC.md](../SDLC.md) "Releases"). To change the notes, edit the changelog fragments before starting it.
- What the enabled workflows will tell an attempt's agent is also in that agent's instructions (pdt-core
  `workflow/news.rs`, [agents.md](agents.md) "Instructions and guides"), so turning one off changes what new sessions
  are told.

## Integrations (PDT-171)

One place declares what an integration adds: a type implementing pdt-api's `Integration` (`integrations/mod.rs`;
what an integration crate builds on is re-exported as `pdt_api::sdk`): its connection kinds (and a check of their
config), event types, node kinds and pdt-mcp tools with their `readOnly`, `asks`, `openWorld`, `destructive` and `pm`
flags (and how to run them, what an asking call acts on, its approval title, credentials to keep out of the
transcript), its lines of the PM's instructions, project settings, whether it is a project's CI provider, its members
of the 60 s poll, its routes (under `/api`, 404 while it is off), its built-in workflows and its templates. PDT's own
pieces are always on: core, forge (the forge tools, PDT-177), webhook (incoming), github (issues), http, mcp, bug_reports (`report_bug`), in pdt-api
`integrations/`. The `Registry` gathers PDT's own and the enabled integrations, in that order, into the catalog,
`tools/list` and `tools/call`, the personas' tools (`GET /api/personas/tools`, the PM's set, which ask), the event
types and `GET /api/connections/kinds`. An integration's tools are the PM's only with its `pm` flag, and in another
persona's set only when the user picks them.

**Integration crates and enabling them** (PDT-173). Each optional integration is a crate, `crates/pdt-int-<id>`:
discord (webhooks, the bot and its tools, the templates that post to Discord), jira (with its sensor, import route and
built-ins), buildkite (its CI sensor), arr (Radarr and Sonarr), scryer (with its tools and GraphQL schema) and plex.
pdtd has a cargo feature per crate (all on by default, so releases have them all; `--no-default-features --features
jira,…` leaves the rest out) and hands those it was built with to `Integrations::new` (pdtd `src/integrations/`).
pdt-core `integration::KNOWN` is the one list naming them outside their crates (id, label, description, category, icon, connection kinds, node-kind
prefixes). Which of them an instance uses is the host value `integrations.enabled` (a JSON list of ids):
Settings → Plugins (the UI calls an integration a plugin, PDT-228; see [Plugins](#plugins-in-settings)). `PUT
/api/integrations {enabled}` sets exactly those (422 for one this build lacks; PDT's own ids are ignored: they are
always on). A disabled integration registers nothing (no connection
kinds, node kinds, triggers, tools, PM instructions, project settings, pollers, routes or built-ins); its connections,
project links and workflows are kept. A connection of a turned-off integration's kind is refused (400, naming it), a
project's settings leave its section out, and a workflow node of its kinds fails validation (and a run) with
`integration_disabled` ("… is part of the Jira integration, which is turned off") or, when this build lacks it,
`integration_missing`, instead of `unknown_kind`; a refused connection's message says which of the two it is. A new install enables none; when the store opens with no
`integrations.enabled` yet (an install from before PDT-173), it enables those already in use: with a connection of one
of their kinds, or a workflow of the user's (not a built-in) with one of their node kinds (pdt-store `host.rs`).
Turning one on installs its built-ins and refreshes the built-in personas on the engine's next tick.

**Breaking change (PDT-173):** `ticket.stage_changed` no longer has `jiraKey`; it has `external.jira`
(`{{trigger.external.jira}}`). The untouched built-in "Done moves the Jira issue" follows its new template, but a copy
of it, or a workflow of the user's, that reads `{{trigger.jiraKey}}` stops matching (the value is missing, so an
`exists` condition is false) with no warning: change it to `{{trigger.external.jira}}`.

An **action** (PDT-172, `integrations/action.rs`) is one capability defined once and offered as both a node kind and a
pdt-mcp tool: one `Action` gives its input type (the node's config and the tool's arguments, one JSON Schema), output
type, the tool's name, flags, title and description, the node's id, title, description and ports, the kind of
connection it uses (a tool call may leave `connection` out to use the only one), a `check` for node configs, an
approval title (also a dry run's `would`), what an asking call acts on, and one async `run`. Where the two differ, the
action shapes each output (`scryer.add` leaves by `added` or `exists` with the title, `scryer_add` answers
`{added, alreadyInLibrary, title}`; the tools cut long Scryer answers to fit). An integration lists its actions in
`Integration::actions`; the registry makes each one's tool (before the integration's other tools) and node kind (after
its other kinds). An action can be a tool only (`scryer_schema`, `discord_get`, `discord_change`, `discord_webhook`).
On actions: Scryer's library, search, queue, add, query and mutate (each `scryer.*` kind and `scryer_*` tool) and
`scryer_schema`, and Discord's tools; `discord.post` and `discord.channel.ensure` are node kinds with no tool. A test
(pdtd `src/integrations/tests.rs`, `snapshot.json`, with every integration on) keeps every tool's `tools/list` entry and flags, the PM's tools and every
kind's spec as they were.

**PM hooks** (PDT-175). An integration also says how the PM treats its tools, so PDT's PM code names none:
`Integration::ask_target` (what an asking call of its non-action tools acts on; an action's comes from
`Action::ask_target`), `approval_title` (the title Claude's approval card, the Inbox and the transcript show; set on
the tool-call and request events as they are stored), `redact_call` (takes credentials an agent wrote out of a call's
input and gives them back, for PDT to take out of its output and raw line too) and `pm_prompt` (Markdown list items for
the PM's instructions, given to a PM session with any of its tools). The `Registry` answers each by the tool's name as
an agent gives it (`mcp__pdt__<tool>`, Codex's `mcp:pdt/<tool>`, or bare; pdt-core `mcp::pdt_tool`). A registry
without an integration gives none of its text, tools or hooks; `pm_snapshot.md` keeps the PM's instructions and the
hooks' answers for a set of calls as they were.

**Project settings** (PDT-174): an integration a project links to declares a `ProjectSettingsSchema` (pdt-core
`integration.rs`: its label, description, the connection kind it links through, and `ConfigField`s, text, choice or
lines, with `default` and `optional`), and may check the result (`check_project_settings`, e.g. Buildkite's pipeline
slugs). `GET /api/integrations/project-settings` lists the enabled integrations' (jira, buildkite); the project's settings page makes its
Integrations section from them. A project's settings are one JSON object per integration in
`project_settings.integration_settings`, keyed by the integration's id: `connection` plus each field (lists as
arrays). `ProjectSettingsSchema::apply` tidies an update: keys left out keep their value, an empty one takes its
default, an unknown key or wrong type is `400`, no connection unlinks.

## Catalog

`GET /api/workflows/catalog` lists every kind: `id`, `version`, `category`, `title`, `description`,
`configSchema` / `outputSchema` (JSON Schema from schemars), `ports`, `dynamicPorts`. One module per kind in
pdt-api `workflow/nodes/` or its integration's crate, registered by its integration ("Integrations");
config types also derive ts-rs (web/src/api/gen). Each event type is a trigger kind: the plain event trigger, unless a
kind claims the type (`NodeKind::event_type`: the schedule, webhook and GitHub issue triggers have configs of their
own). After the kinds come the MCP tools' entries ("MCP tools as actions"), which have a
`preset`.

| Kind | Ports |
|---|---|
| one trigger per event type; `manual` (Run now, sub-workflows) — config `projects`, `where`, `runKey` | `out` |
| `condition` (`conditions`, `match: all \| any`) | `true`, `false` |
| `switch` (`value`, `cases [{port, equals}]`) | each case, `default` |
| `gate` (`question`, `choices`, `ahead [{when, choice}]`, `ask: wait \| skip`, `timeoutMs`, `inbox: gate \| accept`, `pipelineStage`, `approve`, `event {type, also, where, any}`: an event that ends the question, closing its Inbox item) | each choice, `unanswered`, `timeout`, `event` (with `event`) |
| `agent` (`persona`, `prompt`, `outcomes [{id, description}]`, `session: new \| reuse \| own`, `ticket`, `attempt`, `maxTurns`, `maxRuntimeMs`, `nudges`, `backgroundWaitMs`, `loopLimit`, `owner: agent \| user`, `pipelineStage`; below) | each outcome (`dynamicPorts`), `timeout`, `stopped` |
| `wait` (`ms`, `until`, or `event {type, also, where, any}` with `timeoutMs`; `also`: more event types that end it, `any`: with `where`, one of these holds too, PDT-151; `pipelineStage`) | `done`, `event`, `timeout` |
| `workflow.call` (`workflow`, `input`) | `done`, `failed` |
| `end` (`quiet`, `output`) | — |
| `state.set` (`key`, `value`, `ifMissing`; or `add`: a counter, output `value`) | `out` |
| `session.send` (`attempt` or `session`, `text`, `dedupe`, `timeline: none \| pr`, `reopen`) | `sent`, `untold`, `skipped` |
| `ticket.finish` (`ticket`, `stopRunning`, `deleteBranch`) | `finished`, `blocked` |
| `ticket.move` (`ticket`, `stage`, `externalStage`: where an external ticket the user hasn't let in goes instead of To do or In progress; without it such a move fails, PDT-201), `ticket.comment` (`ticket`: its id or key, `body`) | `out` |
| `ticket.create` (`project`, `title`, `description`, `stage` default Draft, `labels`; PDT-151; output `id`, `key`; not run again on its own; source `workflow`, or `external` in a run an outside event started, then never in To do) | `out` |
| `attempt.merge` (`attempt`, `style`, `finishOnMerge`, `deleteBranch`, `again`, `pipelineStage`; Phase 7c) | `merged`, `queued`, `refused` |
| `inbox.notice` (`attempt`, `text`, `key`, `title`; PDT-139): an Inbox item (kind and group `notice`, Unheard notices) on the attempt's ticket, once per attempt and key (`inbox_items.dedupe_key` `notice:<attempt>:<key>`, also once it closed); body `notice` (the trigger's notice kind), `key`, `text`, `pr`, `url`, `workflow`; the default title says what happened and that no agent session is open. Without `attempt` (PDT-154) it is the workflow's notice, in the run's project (and ticket, if any), once per workflow and key (`notice:workflow:<workflow>:<key>`), titled by `title` or the text's first line, open until dismissed | `added`, `skipped` |
| `attempt.agent_working` (`attempt`, `known`; PDT-104): an agent step on the attempt now (bound, or waiting for a slot, in any other run) | `yes`, `no` |
| `schedule` (`every` or `cron` + `timezone`, `catchUp`, `where`, `runKey`; fires on its own `schedule.fired`) | `out` |
| `webhook` (`hook`: a `webhook` connection, `where`, `runKey`; its own `webhook.received`) | `out` |
| `github.issue.opened` (`repos`, else the projects' repos; `projects`, `where`, `runKey`) | `out` |
| `command` (`program` and `args`, or `script` and `shell: sh \| bash \| zsh \| pwsh`; `cwd: scratch \| project \| attempt \| <full path>`, `input`, `env`, `parse: json \| text`) | `out` |
| `http.request` (`connection`, `method`, `url`, `headers`, `query`, `body`, `expect: 2xx \| any \| [codes]`) | `out` |
| `discord.post` (`connection`: `discord_webhook`, `content`, `username`, `embeds`, `thread`) | `out` |
| `discord.channel.ensure` (`connection`: `discord_bot`, `name`, `category`, `topic`) | `out` |
| `notify` (`title`, `body`, `level: info \| warning \| error`, `link`) | `out` |
| `jira.import` (`key`, `project`: a PDT project, `connection`: default the project's) | `imported`, `exists` |
| `jira.create` (`connection`, `project`: a Jira project key, `issueType` default `Task`, `summary`, `description`, `fields`) | `out` |
| `jira.transition` (`connection` + `key`, or `ticket`; `to`: a transition or status, with `ticket` default the project's done mapping) | `out`, `unavailable` |
| `jira.comment` (`connection`, `key`, `body`) | `out` |
| `github.import` (`repo`, `number`, `project`: a PDT project, `priority`, `labels`; PDT-154) | `imported`, `exists` |
| `github.comment` (`repo` + `number`, or `ticket`: its linked issue; `body`) | `out`, `unlinked` |
| `github.close` (`repo` + `number`, or `ticket`; `reason: completed \| not_planned \| duplicate`, `duplicateOf` with `duplicate`, `comment`, `labels`) | `out`, `unlinked` |
| `github.create_issue` (`ticket`, or `project`; `title`, `body`, `labels`; PDT-221) | `filed` |
| `github.workflow.run` (`repo`: default the run's project's, `workflow`: its file, `ref` default `main`, `inputs`; PDT-194) | `out` |
| `github.change_request` (`repo`, `branch` or `number`, `dryRunPort`; PDT-194; output `number`, `url`, `title`, `state`, `merged`, `headSha`) | `open`, `merged`, `closed`, `none` |
| `github.release` (`repo`, `tag` or the latest, `newerThan`, `prereleases`; PDT-194; output `tag`, `name`, `url`, `publishedAt`) | `found`, `none` |
| `buildkite.retry` (`connection`, `build`: a build URL, or a number with `pipeline`) | `out` |
| `buildkite.trigger` (`connection`, `pipeline`, `branch`, `commit` default `HEAD`, `message`, `env`) | `out` |
| `radarr.search` / `sonarr.search` (`connection`, `term`, `limit`) | `out` |
| `radarr.add` (`connection`, `tmdbId`, `qualityProfile`, `rootFolder`, `search`, `minimumAvailability`) / `sonarr.add` (`connection`, `tvdbId`, `qualityProfile`, `rootFolder`, `monitor`, `search`) | `added`, `exists` |
| `radarr.queue` / `sonarr.queue` (`connection`) | `out` |
| `scryer.search` (`connection`, `term`, `facet`, `limit`), `scryer.queue` (`connection`, `limit`), `scryer.library` (`connection`, `facet`, `query`, `monitored`, `limit`, `offset`) | `out` |
| `scryer.add` (`connection`, `facet`, `tvdbId`, `tmdbId`, `imdbId`, `name`, `library`, `monitored`, `monitor`) | `added`, `exists` |
| `scryer.query` (`connection`, `query`, `variables`, `operationName`), `scryer.mutate` (`connection`, `mutation`, `args`, `select`) | `out` |
| `plex.search` (`connection`, `query`, `limit`), `plex.recently_added` (`connection`, `section`, `limit`), `plex.refresh` (`connection`, `section`) | `out` |
| `mcp.tool` (`server`: an `mcp` connection, `tool`, `arguments`) | `out` |
| `brief.archive` (`brief`, `title`, `body`, `items [{title, link, note}]`, `date`), `brief.read` (`brief`, `since`, `last`) | `out` |

Every node also has an `error` port: taken once its retries are spent, if it has an edge; otherwise the run fails.

A trigger kind decides which events it fires for (`NodeKind::accepts`; default: the event is in the workflow's scope),
then the engine checks the shared `projects` and `where`. `schedule` and `webhook` fire only for their own events, and
`github.issue.opened` with `repos` for those repos whatever the scope. `notify` logs no event, so a workflow can't
notify itself forever.

- `command` runs the program without a shell, or a `script` (PDT-250) as `<shell> -c <script>` (`shell` defaults to
  `sh`; `pwsh` for PowerShell). A step has a program or a script, never both (the schema's `oneOf`, and validation says
  so; a script takes no `args`). `{{…}}` references in a script are filled in like any text field: the value pasted in
  as plain text, not quoted for the shell, so a value with a quote or `$(…)` in it can break or change the script. Put
  outside text (titles, comments, anything a person wrote) in `env` (`"TITLE": "{{trigger.title}}"`) and use `"$TITLE"`
  in the script, or read it from `input` on stdin. The step gets `input` as JSON on stdin and `env` added to pdtd's
  environment, and runs in a scratch folder (`<data dir>/workflow-scratch/<run>`, removed when the run ends), the
  run's project's main checkout, its attempt's worktree or a full path. stdout (up to 64 KiB) is parsed as JSON into `json`
  (empty: null); `{"quiet": true}` makes the step quiet. A non-zero exit or unparseable stdout fails it (with the end
  of stderr). The time limit kills the process tree (its own process group on Unix, `taskkill /T` on Windows).
- `http.request`: `url` may be relative to the connection's URL; the connection's secret is sent per its `config.auth`
  (`bearer` default, `basic` with `username`, `header` with `header`, `query` with `param`, `none`), and only to an
  address at or below the connection's URL. Output `{status, ok, headers, body}` (JSON when the response is, else
  text; 1 MiB). A status outside `expect` is an error.
- `discord.post`: the connection's secret is the webhook URL. Content over 2000 characters goes as several messages
  (cut at line breaks), embeds with the first; mentions are never parsed; a 429 waits `retry_after` (up to 4 tries).
  Output `{messageIds}`.
- A failed request (no answer: DNS, TLS, a refused or reset connection) never shows its URL: reqwest's message ends
  with it, and a URL can carry the secret (a Discord webhook's token, a `query` key), so every outbound call's error
  goes through `send_error` (`workflow/nodes/http.rs`), which drops the URL and keeps the causes, before it becomes the
  step's error, the run's failure, a notification or a log line.
- Jira actions (`workflow/nodes/jira.rs`, client in `jira.rs`) sign in with the connection's email and API token
  (basic auth, REST v3); text goes as ADF (a paragraph per blank-line-separated block, line breaks kept).
  `jira.import` makes a Draft ticket (title the summary, description the issue's text) linked to the issue, or leaves
  by `exists` (quiet) when a ticket already links it. `jira.transition` counts an issue already in `to` as done
  (quiet), picks the transition named `to` or the one leading to that status, and otherwise leaves by `unavailable`
  (quiet, with the `available` names); a ticket without a Jira link also leaves by `unavailable`. `jira.create`
  outputs `{key, url}`, `jira.comment` `{key, commentId}`.
- GitHub issue actions (`workflow/nodes/github.rs`, PDT-154) go through the GitHub forge (PDT-176: PDT's GitHub
  login, pdt-github, the access the `github.issue.opened` trigger uses; `FakeForge` in tests), and fail when it has no
  issues; the trigger watches only projects whose forge is GitHub. `number` is a number or text: `12`, `#12`,
  `owner/name#12` or the issue's URL. `github.import` makes a Draft ticket (title the issue's, description its text
  then "Reported by @author in <url>"; a priority or label PDT can't take is left out) linked to the issue as
  `owner/name#12`, or leaves by `exists` (quiet) when a ticket already links it; output `{ticketId, ticketKey, issue,
  url}`. A pull request's number is an error. With `ticket`, `github.comment` and `github.close` act on the ticket's
  linked issue (the repository as its URL spells it) and leave by `unlinked` (quiet) without one. `github.comment`
  outputs `{issue, url}` (comments over 20000 characters are cut). `github.close` reads the issue first: one already
  closed counts as done (quiet, `alreadyClosed`, nothing changed); otherwise it adds `labels` (a POST, so the issue
  keeps its others), posts `comment`, then closes it with REST's `state_reason` (`PATCH /repos/{repo}/issues/{n}`);
  `duplicate` reads `duplicateOf` (a number in the same repository, or `owner/name#n`) for its id and sends it as
  `duplicate_issue_id`, which is GitHub's Close as duplicate. Output `{issue, reason, duplicateOf, commentUrl,
  labels}`. Neither is run again on its own. `github.create_issue` (an action, also the `issue_create` tool) files an
  issue through the project's forge, from `ticket` (which then links it) or in `project`'s repository
  ([github.md](github.md) "Filing issues").
- Release actions (`workflow/nodes/release.rs`, PDT-194) go through the GitHub forge too (its `releases` capability
  for the workflow and release calls). Without `repo` they use the run's project's GitHub repository (its forge; a
  project not on GitHub is an error), so a host-wide workflow runs them in the project its Run now input names
  (`projectId`; dry runs take the event's `projectId` as real runs do). `github.workflow.run` sends
  `workflow_dispatch` with the inputs as text (null and empty ones left out); it is not run again on its own, and
  GitHub doesn't say which run it started. `github.change_request` reads the branch's newest open pull request (else
  its newest) or one by number, and leaves by its state (`none`: quiet). With `ci` (and `branch`, its head; `number`
  then picks the PR, else read by number alone) it reads CI on the head and the merge-queue run as the PR checks do:
  an open PR whose head CI failed, or that the merge queue removed for failed checks with auto-merge off, leaves by
  `failed` (a port only with `ci`), with `failure` (`ci` or `queue`), `ciUrl` and `failedJobs` (a Markdown list). `github.release` reads the latest published
  release or one tag; with `newerThan`, a latest release with that tag, or an older version, is `none` (quiet), so it
  polls for the next one (a pre-release never counts, so with a test tag prefix, whose releases are pre-releases,
  a stable Cut a release never sees one published). With `prereleases` (PDT-210; not with `tag`) it reads the newest
  published release, pre-releases included, as an experimental cut needs. Dry runs call nothing: `github.change_request` takes its `dryRunPort`, else
  `open` for a branch and `merged` for a number, `github.release` `found`.
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
- **Scryer** (PDT-147; pdt-api `scryer.rs`, pdt-core `scryer.rs`; connection `url` Scryer's address with its base
  path, secret an API key (`ska_…`), sent as a bearer token in a POST to `<url>/graphql`; GraphQL `errors` fail the
  step, a 401 says the key is wrong). `facet` is `movie`, `series` or `anime`. Search: `searchMetadata` for one facet,
  else `searchMetadataMulti` (10 per facet by default): `{results [{facet, name, year, tvdbId, tmdbId, imdbId,
  inLibrary, titleId, overview}]}`, `inLibrary` from `titlesByExternalIds` (same facet, a shared id). Add: by at least
  one id; a title of that facet sharing an id takes `exists`; else `metadataMovie` / `metadataSeries` names it and
  fills in its other ids, `library` (a name, slug or id among `libraries(facet)`; Scryer's default when left out)
  is resolved first, and `addTitle` adds it as Scryer's own web does (`monitored` default true; `monitorType`
  `ALL_EPISODES` when monitored, else `NONE`, or a series' `monitor`: `all`, `future`, `missingAndFuture`, `none`;
  season folders for series and anime; anime without specials, with inter-season films; a series' TMDB id kinded
  `series`): `added`, the title as output (`{titleId, facet, name, year, monitored, library, status, episodesOwned,
  episodesTotal, sizeBytes}`). Scryer then searches for a monitored title itself. Queue: `downloadQueuePage` (50 by
  default): `{items [{title, progress, status (`downloading`, `importPending`, …), secondsLeft, client, attention}],
  total}`. Library: `titles` (50 a page by default, at most 300): `{titles, total, hasMore}`.
  Every operation is checked against Scryer's published schema, vendored in
  `crates/pdt-int-scryer/src/schema.graphql` (a unit test with apollo-compiler).
  Any query or mutation (PDT-150; `scryer/graphql.rs`): `scryer.query` runs a query document (no mutations or
  subscriptions) and outputs `{data}`; `scryer.mutate` runs one mutation by name with its `args` and `select` and
  outputs `{mutation, result}`. Both are validated against Scryer's live schema (introspection, kept 10 minutes per
  connection; the bundled one when Scryer doesn't answer it) before anything is sent, and credentials in the output
  are redacted. A mutation's credential is `{"$secret": "<connection>"}`, filled in from that connection's secret.
  Saving checks a query parses and holds only queries, and that a mutation is not one PDT never sends (signing in and
  the user's own credentials); a workflow's mutations don't ask (the user enabled it). The PM's tools on them, and
  which mutations always ask, are in [agents.md](agents.md) "Scryer's whole API".
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
  settings or secret change or after a transport error, stopped after 5 minutes idle (`maintain_workflows`; never
  while a call is in flight) and when pdtd stops. A call that does not answer in time (a second before its step's
  `timeoutMs`, so the error names the server), a listing that takes over 30 s, or a call its step gave up on marks the
  client broken (PDT-130): the next use starts the server again, so a wedged server never fails every later step. A
  stdio server's stderr goes to pdtd's log at debug level.
- **Tools** (`tools/list`) are cached in memory by connection name: listed in the background when pdtd first sees an
  `mcp` connection and when one is saved, and again by `maintain_workflows` once the last try is 10 minutes old while
  the server's client is live (an unused server is not started just to list it). A failed listing keeps the tools from
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

- `run_workflows` (pdtd every 2 s; tests call it): installs the built-ins once,
  emits due schedules ("Sensors and webhooks"), matches unprocessed events to enabled workflows' triggers (scope, `projects`, `where`) and starts a run per match
  unless its run key (default `<node>:<event id>`) already ran, wakes steps waiting for that event, advances runnable
  runs, delivers the outbox, and advances again. Enabled workflows' parsed documents are cached in memory by
  workflow, version and `updated_at` for the schedule and matching passes, and the steps waiting for events are read
  once per batch of events.
- Locks (PDT-128): `AppState.engine_lock` is held only around state changes: the tick's passes (schedules, events,
  slots, outbox, sweep), claiming a step (`running`) and writing its result, and the commands (answer, resume, retry,
  cancel, `submit_outcome`, `ask_user`, a turn's end, Inbox answers). A node's body (an HTTP request, a command,
  a forge call, starting an agent session: actions and agent steps) runs outside it, holding only its run's lock
  (`AppState.run_locks`, shared by the runs of one ticket, so those still go one at a time and one's checks see
  what the other did); flow, gate, wait and condition kinds and `session.send` (quick, and waited on by the
  engine's own passes) run under it. Runnable runs advance in their own tasks (a ticket's runs one after another in
  one task), up to 16 tasks at once; a tick waits for them up to `engine_tick_wait` (pdtd: 1 s; tests: 10 min, so a
  test's tick ends with its runs) and then goes on, and later ticks skip a run whose lock is held. So a hung step
  holds up only its own ticket's runs: other runs, the outbox, timers and schedules go on. Commands on a run's steps
  take its lock first (they never see a step half-run, e.g. an agent step whose session is starting), then the
  engine lock; cancel takes only the engine lock, so it never waits for a hung step, whose result is dropped when it
  comes (a step's result is written only while it is still `running` with the same attempt). The limit: as the run
  lock is the ticket's, a command for one of a ticket's agent steps (`submit_outcome`, `ask_user`, a turn's end, an
  Inbox answer) still waits while another run of that ticket is in a slow action's body, up to that node's timeout. Agent steps start one
  at a time (`agent_step_start`), so runs side by side can't both take the host's last slot.
- `workflow_runs` / `workflow_steps`: a run starts with its trigger's step; each step is written before and after it
  runs (`pending` → `running` → `succeeded` / `waiting` / `failed`), so after a restart a `running` step runs again
  and the rest continue. A step's input is its config with references filled in; its output feeds `{{steps.x}}`.
  Ending a step is one transaction (PDT-131): the step, the steps its port leads to, closing its Inbox items, ending
  its agent binding and waking its run, so a crash never leaves a run with a step done and nothing after it. A run a
  PDT before that left so (nothing left to run or wait for, its last step to end leaving by a port that leads
  somewhere, nothing queued after it) gets those steps queued instead of ending early.
- Running a step again (PDT-131). A step cut short — found `running` after a restart, or dropped by its `timeoutMs`
  — may have done some or all of its work, and runs again only if its kind is safe to run again (`rerun_safe`); the
  node sees `StepCx::rerun`. A restart doesn't use up one of the node's retries (its `attempts` stays). Safe because
  they record or check what they did: `discord.post` saves each piece's message id as it posts
  (`StepCx::save_progress`, kept in the step's `output` while it runs and across its retries and Retry), so it posts
  only the pieces still to go; `ticket.comment` marks its comment with its step (`data.stepId`) and finds it instead
  of commenting again; `attempt.merge` takes `merged` when its attempt already merged (a pull request already set to
  merge is `queued` anyway); `notify`, `session.send`, gates and agent steps are keyed by their step. Not safe, so
  never run again on their own: `command`, `http.request` other than GET, HEAD, PUT, DELETE and OPTIONS,
  `buildkite.trigger`, `buildkite.retry`, `jira.create`, `jira.comment`, `github.comment` and `github.close`. Cut short, such a step fails without its
  retries ("PDT stopped while this step ran… Retry the run to run it once more"; a timeout keeps its own error) and
  takes its error port, else the run fails; Retry on the run is the user's choice to run it once more. A definite
  failure (an error answer) is still retried as the node says.
- Waits (`wait`, gates, deliveries, sub-workflows, retries) keep the step `waiting` with what it waits for (`wait`)
  and `wake_at`; a timer that comes due takes the step's timeout port. A run with only waiting steps is `waiting`,
  and wakes at its first timer or at its runtime limit, whichever comes first, so a wait with no timeout (an answer,
  a sub-run, a session that never comes) can't keep it past `maxRuntimeMs`. Waiting runs with no wake time (left by
  a PDT before PDT-131) are woken once by the tick to get one.
- Event waits look back (PDT-131): events are matched to waiting steps as they are processed, so a step that starts
  waiting for an event (a `wait`, a gate's `event`) first looks for one already logged since its run started that
  fits its type and conditions, is not the run's trigger, and no step of the run took already (a loop's next wait
  waits for the next event). A pull request merged by hand during QA ends the Pipeline's wait for `attempt.merged` at
  once instead of three days later.
- A run ends `succeeded`, `quiet` (an `end` with `quiet`, or no action did anything that wasn't quiet: nobody was
  told), `failed` (`error` names the node) or `cancelled`. Limits: `maxVisits` per node (loop limit, default 25),
  `maxSteps` (200) and `maxRuntimeMs` (7 days) per run, `timeoutMs` per execution (actions default 5 min). The
  runtime limit counts from the run's start, or from when Retry or Resume last set it going (`resumedAt`), as its
  waits' timeouts start again too.
- Deleting a workflow (`DELETE /api/workflows/{id}`) cancels its runs still going (a run of another workflow waiting
  on one through `workflow.call` goes on by `failed`) and closes every open Inbox item of its runs, before the
  workflow and its runs go.
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

`session_messages`, one per step (status `waiting | sending | told | untold`). `outbox.rs` delivers with the PR
notices' old rule: the attempt's newest running agent session (a terminal with a live PTY, or a conversation); a
terminal idle with no typing for 20 s (counted from when PDT started watching it, so a TUI still starting isn't typed
into; focus reports and the terminal's replies to queries are not typing), a conversation `ready_for_notice` (idle,
nothing queued, no open request, no rate limit, terminal closed; a parked one reopens); all of a session's waiting
messages in one message. Workflows' messages (`session.send`: PR notices' CI failed, conflict, queue drop, base moved)
that meet an agent step handing in work (one with a `submitted` outcome: a Coder's) are folded into it (PDT-161,
pdt-core `fold_messages`): in one batch with the step's message (a QA-fail or changes-requested pass waiting for the
Coder) they come after it, under "Also for this pass, once each point above is addressed", never ahead of the
findings; on their own while such a step works they say they are one more item of the step, after what its message
asked. No running agent session → `untold`, except while a pdtd restart is about to resume the
session (`AppState::pending_resume`): its messages wait for it. What the user sent to a session (an Inbox answer, a
Nudge) goes to its conversation even when that is stopped or interrupted (a pdtd restart), reopening it as typing
would. So does a message that `reopens` (`session_messages.reopens`, PDT-144: an agent step's message to its reused
session, and `session.send` with `reopen`, which with no running agent session for its attempt goes to the attempt's
newest conversation that wasn't stopped); unlike the user's, the host's cap still holds it. Any other message is sent to a
conversation only if it is still running when PDT looks under the session's lock (PDT-203): one stopped meanwhile
(a cancelled run's step session, stopped while an agent step's nudge waited) is not reopened, and the message is
`untold`. `dedupe` is per attempt (also against `pr` timeline keys from before the event log); an `untold` message does
not spend its key. Resolving writes the timeline record and resumes the step (`sent` or `untold`).

Honest delivery (PDT-129):
- A terminal's message is told once its paste and Enter went in; writes to a terminal whose process is gone fail
  (pdt-agents `PtyManager::write`/`accepts_input`: as soon as the child's side of the PTY closes, before its exit is
  reported), and the message stays waiting.
- A conversation's message is `sending` (`pending_id`: the conversation's pending message that carries it) until the
  agent echoes it as a `user_message` (Claude's replay, Codex's item; Devin's ACP has none, so its echo is PDT's own
  as the prompt is written), then `told`. Every other conversation message (the user's `POST …/messages`, review
  feedback, an agent step's first message) gets a `sending` row too.
- Lost on the way → `untold` and a `session.undelivered` notification naming the session, why and the text: the agent
  exits unexpectedly with messages it hadn't taken; pdtd restarts with rows still `sending`
  (`recover_conversations`); a conversation leaves a message pending for `Conversations::pending_timeout` (5 min) with
  no turn, no open request and no event (`expire_pending_messages`, every 30 s; it no longer holds the conversation
  busy). The user's Stop, or removing the session, resolves them `untold` without a notification.
- A session that cannot take its messages though it isn't working (a terminal at `unknown` or `needs_you`, untracked,
  or being typed in; a conversation waiting on a request or with its terminal open) gets `AppState::outbox_deadline`
  (30 min, counted from when it was first seen so): then its messages are `untold` with a `session.undelivered`
  notification. A working session (or a conversation in a rate limit) is waited for however long it takes.

Told and untold messages are deleted 30 days after they were resolved (`maintain_workflows`), except an active
attempt's, whose `dedupe` keys still count.

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
Pipeline (below), New GitHub issues to Discord (7e; PDT-226 removed the templates that weren't about development: Health watchdog,
Brief follow-ups and Email processing; workflows made from them stay as they are), Triage new GitHub issues (PDT-81: `github.issue.opened` → an `agent` step of the
locked-down Triage persona by default (PDT-201, [agents.md](agents.md)), `{{params.persona}}`, with outcomes `drafted` (it made a draft ticket in `{{params.project}}` with
`create_ticket`, which asks first unless the project lets the PM act; an external Draft), `duplicate` and `ignored`,
the last quiet), and
the built-in workflows (PDT bug triage only where a project is PDT's own repository). A required text param with no default is checked as
the reference itself. The user's are in `workflow_templates`.

Each template has a `category` for the New workflow gallery (PDT-229): `ciPr` (CI & pull requests), `tickets`, `issues`,
`notifications`, `releases` or `other` (Blank and the user's own); every built-in declares its own (`builtin()`'s
argument, `Builtin::category`). The list (`GET /api/workflow-templates`) adds to each what its card and preview show:
`plugin` (`{id, label, on}`: the first optional integration whose node kinds it uses, else GitHub for a `github.*`
kind; GitHub is always on), `builtinWorkflow` and `added` (one of PDT's built-in workflows, and whether this host has
it), `triggers` and `steps` (node titles, the kind's title when a node has none; ends left out). It also lists the
templates and built-ins of integrations this build has that are turned off, with `plugin.on` false; instantiating one
is `409 integration_disabled`. Node titles come from a catalog of every integration this build has
(`Integrations::everything`), which nothing runs from.

A workflow made from a built-in template records it and its params in its state (`_pdt.template`: `{id, params}`).
On the engine's first tick (with the built-ins), each such workflow still untouched (its current version is the one
instantiating wrote, or one PDT wrote) moves to the template's current document filled with those params (its name
kept; a version authored `builtin`, "updated by PDT to the newer template …"); one the user or an agent saved a
version of is left alone (PDT-124). A Pipeline made before templates were recorded (version 1, noted "from the
template Pipeline", with the Pipeline's triggers and merge) is recognised, and its params are read back from the
document (the value at each place the template has exactly `{{params.<id>}}`; defaults for the rest). A waiting run
goes on with the version it started on.

Connections: kinds the actions know are `discord_webhook` (secret: the webhook URL; one the PM made has
`config.webhookId`, `channelId`, `channel` and `bot`), `discord_bot` (secret: the bot token; `config.guildId`, checked
on save, `applicationId` for the invite link, `pmActs`; `url` empty means Discord's API, tests point it at a fake),
`webhook` (secret: the shared secret), `http` (`url`, `config.auth` and its field; secret: the token or password),
`jira` (`url` the site `https://<site>.atlassian.net`, `config.email`; secret: an API token), `buildkite` (`url`
empty for Buildkite's own API, `config.org`; secret: an API token with read builds, and write builds to retry or start
them), `radarr` and `sonarr` (`url`, `config.qualityProfile`, `config.rootFolder`; secret: the API key), `scryer`
(`url`, `config.pmActs`; secret: an API key), `plex` (`url`;
secret: the token) and `mcp` ("MCP tools as actions"; `GET/POST /api/connections/{id}/tools`, `409 not_mcp` for
another kind). A config field holding a connection's name says its kind in its schema (`x-connection-kind`:
`discord_webhook`, `discord_bot`, `webhook`, `jira`, `buildkite`, `radarr`, `sonarr`, `scryer`, `plex`, `mcp`, `any`), so forms offer a picker. `GET /api/connections/kinds` (every kind the integrations declare, for the Connections page), `GET/POST /api/connections`, `GET/PATCH/DELETE /api/connections/{id}` (`name` unique, `kind`, `url`,
`config`), `PUT/DELETE /api/connections/{id}/secret {secret}` (the OS keychain, account `connection:<id>`; only
`hasSecret` is ever returned). A connection carries `lastError` and `lastErrorAt` while a sensor cannot use it
("Sensors and webhooks"); saving it (or its secret) clears them until the next use.

## Export and import (PDT-155)

Workflows move between PDTs as text (pdt-core `workflow::transfer`, pdt-api `workflow/transfer.rs`). An export is
`{format: "pdt-workflow", formatVersion: 1, pdtVersion, exportedAt, workflows: [{name, description, scope, project?,
doc}], requires}`, pretty-printed, the same whether copied or downloaded: current documents only (no runs, state,
`_pdt.template`, versions, proposals or secrets). Every config string that is a project id here becomes `{"$project":
"<KEY>"}`; a project workflow's own project travels as `project` (its key). `requires` lists what the documents name:
`connections` `{name, kind, tools}` (each field with `x-connection-kind`, so MCP servers and webhook hooks too;
`tools` are an MCP server's tools that `mcp.tool` steps call), `personas` (agent steps) and `projects` `{key, name}`.
Templated names (`{{…}}`) are left out.

`GET /api/workflows/{id}/export`, `POST /api/workflows/export {ids}` (one envelope, in order). `POST
/api/workflows/import {text, check: true}` parses the text (an export, or a bare document as one host workflow) and
answers `{pdtVersion, workflows: [{name, description, scope, project, nodes, existing, issues}], connections,
personas, projects, warnings}`: what the documents need, worked out from the documents themselves (so a bare
document is matched too), each `found`, `missing` or `kind_mismatch` (a connection of that name of another kind; `any`
fits every kind), and the workflows here with the same name. Text that isn't JSON, another `format`, a
`formatVersion` other than 1 or no workflows are `400 invalid_export` saying which. `{text, mapping, targets}`
imports: `mapping` `{connections, personas, projects}` maps each needed name (a project's key) to one here (a
project's id or key) or to null to leave it unmapped; one that is missing and unanswered is `409 needs_mapping` naming
them all. The documents' names are rewritten, placeholders filled (an unmapped project stays its key, so it matches
nothing), each document is validated, and only then are they written: new workflows disabled (authored by the user,
noted "imported from PDT <version>"), or with `versionOf` a new version of that workflow (an agent's is proposed;
never a repo workflow, `409 workflow_in_repo`). A name already taken here is `409 workflow_exists` unless the target
says `copy` (`<name> (imported)`, then `(imported 2)` …). A target's `scope`/`projectId` default to the export's, a
project workflow's project through the mapping. The answer is `{workflows, warnings}`: names left unmapped, webhook
connections without a secret, and `command` steps whose `cwd` is an absolute folder (from the other machine; not
rewritten). The workflow's detail and `/check` / `/validate` add warnings for what a document names that is not here:
`missing_connection`, `connection_kind`, `hook_secret` (a webhook connection with no secret), `missing_persona`,
`missing_project` (a trigger's `projects`, `ticket.create` / `github.import` `project`).

pdt-mcp: `workflow_export {ids | names}` → `{text, workflows, requires}` (read-only); `workflow_import` takes the
import request (`check: true` reports without asking), asks the user, and imports as `agent`.

Web: the workflow's ⋯ menu has Copy export (the text on the clipboard, "Copied") and Export… (the text to select and
copy, with Download, `<name>.pdt-workflow.json`); the Workflows list has Import… and Export… (pick the workflows; all
to start with). Import… is a paste box (Choose file or drop a file fills it), Check, then each workflow's Applies to
and, on a clash, a copy or a new version of one, and a Map to… for each missing name (connections of the kind needed,
personas, projects, or Leave unmapped): Import stays off until each is answered. With warnings it shows them before
opening the first workflow. The workflow page shows the reference warnings in a "Not set up on this host yet" banner.

## Sensors and webhooks (Phase 7e)

- **Schedules** (`workflow/nodes/schedule.rs`, pdt-core `workflow/schedule.rs`): each engine tick, every enabled
  workflow's `schedule` node's latest slot (`every` aligned to the epoch, at least 1 minute; or a 5-field cron in
  `timezone`, default the host's) is compared with its mark in state (`_pdt.schedule.<node>`): a node seen for the first
  time, or a workflow saved or enabled since the mark (a pending proposal doesn't count: it changes nothing that runs
  until accepted, PDT-131), takes the slot as its baseline; a newer slot emits
  `schedule.fired` (key `schedule:<workflow>:<node>:<slot>`, payload `workflowId`, `nodeId`, `slot`, `slotIso`,
  `date`). A slot more than 2 minutes old (pdtd was down) fires only with `catchUp` (default on), and only the latest.
- **Not losing events** (PDT-130): a sensor's mark (the newest issue, comment or time read to) moves only past what
  was logged. An event that cannot be logged (the store failing) leaves the mark where it was, so the next look reads
  it again; event keys make what was already logged harmless.
- **Failing connections** (`connection_health.rs`): when the Jira or Buildkite sensor cannot use its connection (a
  revoked token, the service down, no such connection), the connection gets `lastError` / `lastErrorAt` (shown on its
  row on its plugin's page) and one `connection.failed` notification (dedupe key `connection:<name>`, opening
  Settings → All connections), however many polls fail; the first poll that works clears both. A pass in which any use of
  a connection failed counts as failing. A failing GitHub read (issues, PR comments, PRs) is logged once per read until it
  works again; a PR it can't read keeps its last look with `error` set (`attempt.pr.error`, [tickets.md](tickets.md)).
- **GitHub issues** (`github_issues.rs` `refresh_issues`, pdtd every 60 s): the repos enabled `github.issue.opened`
  nodes name, else the GitHub repos of the projects in their scope, through GitHub's `…/issues` API (pull requests left out;
  100 a page, read back page by page to the last issue seen, up to 10 pages). The newest number logged per repo is
  kept in `host_settings` (`sensor.github_issues.<repo>`); the first look only records it (from one page). Event key `github:<repo>:issue:<n>`, payload `repo`, `number`, `title`, `body`, `url`, `author`, `labels`,
  `createdAt`, `projectId` (when it is a project's repo).
- **PR comments and reviews** (`github.rs`): while an enabled workflow has a `pr.commented` or `pr.reviewed` trigger,
  the PR poller also reads each active attempt's open PR's conversation comments, review comments and reviews (the
  comments and the reviews side by side). The newest ids are kept per attempt (`sensor.pr_comments.<attempt>`),
  written only once every new one is logged; the first look only records them. Keys
  `<attempt>:comment:<id>` / `<attempt>:review:<id>`; payloads carry `author`, `body` (`path`, `line`, `inReview`; a
  review's `state`), `byOwner` (gh's own login), `isBot` and a `notice` (`commented` / `reviewed`).
- **Webhooks** (`hooks.rs`): `POST /api/hooks/<name>` for a `webhook` connection; its secret as `Authorization: Bearer`
  or GitHub's `X-Hub-Signature-256` (constant-time check). `404` no such hook, `409 webhook_no_secret`, `401
  bad_signature`; body up to 1 MiB. The identity guard lets `/api/hooks/*` through (tailnet machines have no user
  login) except over Funnel; the Host guard still applies. `webhook.received` (key `webhook:<name>:<delivery>` from
  `X-GitHub-Delivery`, `Idempotency-Key` or `X-Request-Id`, else new; a repeat answers `200 {duplicate: true}`, else
  `202 {eventId}`), payload `hook`, `method`, `query`, `headers` (`content-type`, `user-agent`, `x-*` but
  credentials: any name with `auth`, `token`, `secret`, `signature`, `password`, `credential`, `cookie`, or ending in
  `key` but `idempotency-key`), `body` (JSON, else text).
- **Jira** (`jira.rs` `refresh_jira`, pdtd every 60 s): each project whose settings link Jira (a `jira` connection,
  a JQL scope, the done transition) is searched with `POST /rest/api/3/search/jql`, `(<scope>) AND updated >= "-<n>m"
  ORDER BY updated ASC` (relative, since JQL dates are in the Jira user's zone; n covers the time since the last look
  plus a minute), with the changelog, up to 5 pages of 100; an issue whose changelog or comments the search cut short
  (over 100) has its newest 100 of each read from the issue. The last look is `sensor.jira.<project>` (`since`, the
  connection and scope; the first look, or a new connection or scope, only records it); a poll that hits the page cap
  moves it to the last issue read rather than to now, so the next poll reads on. Per issue, news since then less a
  minute (keys make re-reads harmless): `jira.issue.created` (key `jira:<project>:<connection id>:<key>:created`, so
  two projects whose scopes overlap on one connection each get the event),
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
- **Notify** (`workflow/nodes/notify.rs`): a `workflow.notify` notification (PDT-108, [Notifications](#notifications)):
  `title`, `body` plus "From <workflow>", `level` as its severity, `link` as its target (a ticket key opens the ticket,
  else the URL); dedupe key `notify:<step>`; output `{notificationId}` (null when the kind is turned down below Inbox).

## Web

### Plugins in Settings

PDT-228 (`web/src/plugins/`). Settings → **Plugins** (`/settings/plugins`; the old `/settings/integrations` goes
there) is the catalog of `GET /api/integrations`: PDT's own plugins (GitHub, Incoming webhooks, HTTP services, MCP
servers: `core`, always on, "Built in" instead of a switch; PDT's core, forge tools and bug reports are not listed),
then each optional integration this build has, then the known ones it lacks. Each has its `category` (`dev`, `chat`,
`media`, `general`: from pdt-core `KNOWN`, or the own integration's `Integration::category`; one without is not a
plugin), `icon` (letters on a colour), `description`, `connectionKinds` (enabled or not), `nodeKinds` (its triggers,
steps and actions' steps), `templates`, `workflows` (its built-ins), `tools`/`pmTools`, `projectSettings` (its project
settings' description) and `projects` (those with settings for it), whether it is `checkable` (has pollers), and a
computed `status`: `off`, `not_built`, `error` (one of its connections has `lastError`: `error` names it,
`errorConnection` is its id; GitHub: its login is rejected), `needs_setup` (an optional one that is on with no
connection of any of its kinds; GitHub: `Forge::login_needed`), else `on`. The page has a search (name and
description), tabs All · On · Needs attention (with its count), category chips with counts, and Cards or List
(localStorage `pdt.plugins.layout`; a phone always lists), grouped by category. A card's switch turns the plugin on
or off; turning on one that comes back `needs_setup` opens the add-connection dialog for its first kind at once
(Later closes it, leaving it in Needs setup). A dashed note leaves room for plugins installed from outside PDT.

A plugin's page (`/settings/plugins/<id>`; a phone's drill-down): its icon, name, category, description, status and
switch; a banner for Needs setup (Add a connection), Error (the error, with Test again, `POST
/api/integrations/{id}/check`, which runs its pollers now and lists the plugins again, and Edit connection) and Not in
this build; Connections (its own, with Add per kind while it is on, Edit and Delete; GitHub's is its login,
`GithubLogin.tsx`, moved here from This host); What it adds (workflow steps, agent and PM tools, templates and
built-ins, project settings with links to the projects using them); and short Setup steps. The Inbox's Connect
GitHub opens GitHub's page (`/settings/plugins/github`; the old `/settings/host/github` goes there).

Settings → **All connections** (`/settings/connections`) is one searchable table of every connection (and GitHub's
login): name, plugin, details, OK or Failing; a row opens its plugin's page.

Connections (`web/src/connections/`): each plugin's page lists its connections (`ConnectionRow.tsx`: a missing
secret and a webhook's address `/api/hooks/<name>` show on its row, and why a sensor's last use of it failed, and since
when; an MCP server's row shows its tools, or why listing them failed, with List again). The dialog asks
what the kind needs (`GET /api/connections/kinds`, from the integrations; `kinds.ts` reads it: its address, and its own settings: Jira's email, Buildkite's organization, Radarr's
default quality profile and root folder, an MCP server's transport, program, arguments, environment and secret
variable; a setting shown is required unless marked optional) and
says where its secret comes from (Discord: the channel's settings → Integrations → Webhooks → New Webhook → Copy
Webhook URL); an incoming webhook can generate its secret. A Discord bot asks for the server's id, the application's id
(which makes the invite link with View Channels, Manage Channels, Send Messages and Manage Webhooks) and whether the PM
changes the server without asking; Scryer asks for its address, an API key and whether the PM adds titles and changes
settings without asking (both rows then show "PM acts without asking"). Saving keeps config keys the dialog doesn't show. The secret is written to the keychain and never shown again
(empty when editing keeps it). A template's `connection` param and a node field with `x-connection-kind` use
`ConnectionPicker`: the connections of that kind, and Add… for a new one.

To post to Discord: add a Discord webhook connection (above), then New workflow → a template that posts (New GitHub
issues to Discord) and pick it as the Discord channel, or add a Post to Discord step and pick it in
the step's panel. Or add a Discord bot connection and ask the PM to set up the channel and its webhook
([agents.md](agents.md#the-pm-pdt-81)).

`notify` steps show as notifications ([Notifications](#notifications)): a toast by default.

The Workflows view (rail; `web/src/workflows/`), which replaced 7a's Settings → This host → Workflows:

- List column (`WorkflowsList`): New workflow, Import, Export, and the workflows grouped This host / Personal / each
  project (on/off dot, Built in, Repo, Proposed). Templates are only in the gallery.
- New workflow (`NewWorkflowDialog`, the gallery; model in `gallery.ts`): search (name, description, plugin) in the
  header; on the left All and the categories that have templates, with how many match, then Built-in; in the middle
  cards (name, description, trigger, plugin; Added / Not added for built-ins), Start from scratch first and then a
  group per category under All, and PDT's built-in workflows in a collapsed group at the end; on the right the picked
  one's preview (When the triggers → its steps, the first eight), its params, Name (blank for Start from scratch),
  Project (this host, a project, or None (personal)), Cancel and Create workflow (it starts off). A template whose
  plugin is off is faded, its preview links to that plugin's page (Settings → Plugins, PDT-228) and Create is disabled; a built-in already
  added notes that this makes a second copy. No match: "No templates match" and Start from scratch. On a phone the
  categories are a row of chips and the preview opens as a sheet over the list.
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
    limits. A field's `x-format` picks the editor for long text (PDT-250): `markdown` (agent prompts, gate questions,
    messages, comments and ticket text: `MarkdownEditor`), `code` with `x-language` (a command's `script` in `sh`; with
    `json`, and for every JSON field, `CodeEditor` holding JSON, kept as typed until it parses) and `multiline` (a
    growing text area); other text is one line until it holds a newline. Markdown and code fields have Expand, which
    opens the same editor over most of the screen (Done keeps the edit, Cancel drops it).
- Runs (`RunsView`): the runs (polled), and a run's graph of the version it ran with the path taken, its steps (input,
  output, error), Retry from the failed step, Resume from a timed-out or stopped agent step, Cancel, and a waiting
  gate's choices.
- Dry run (`DryRunDialog`): the draft against a recent event of its trigger types, a hand-written one (prefilled from
  the trigger's schema) or a manual input; the path, why triggers didn't match, and what each action would do.
- Versions (`VersionsView`): every version with author, status and note, a diff against the current one, Restore,
  Accept / Discard for a proposal.
- Templates (`TemplatePage`): the user's own edit name, description, params and the document; reached from Save as
  template and the gallery's "Edit this template" (built-in ones only by their URL, read-only).
- On a phone (PDT-120) all of it is view-only except turning a workflow on or off: no New workflow, Dry run, Run now,
  workflow ⋯ menu or draft edits (the panel's fields are read-only, Outline rows have no ⋯), and templates have no Use
  or Delete. The top bar's ⋯ has Enable… (the same "Turn on?" dialog, without Dry run first) or Disable. The tabs are
  Outline (the default) · Runs · Graph · Text · Versions; the graph pans and pinch-zooms. Runs is a list of its own;
  a run (`/workflows/<id>/runs/<runId>`) is its steps in order (status, port, error), each opening to its input and
  output, without the graph, Retry, Resume, Cancel or a gate's choices (answer those from the Inbox).

## Agent steps (Phase 7b)

The `agent` node (pdt-api `workflow/nodes/agent.rs`, the work in `workflow/agent.rs`) runs a persona in a session and
waits for the outcome the agent submits through pdt-mcp ([agents.md](agents.md) "pdt-mcp").

- **Session.** `new` (default; Refiner): a fresh conversation of the persona's agent and model in the
  attempt's worktree (else the project's main checkout, or a Personal project's ticket folder), recording the persona
  and version (`sessions.persona_id`, `persona_version`, `step_id`), with no attempt (so PR notices still go to the
  attempt's own session) but linked to the ticket (`sessions.ticket_id`, PDT-105): it is titled `<Persona> · <KEY>`,
  carries the ticket like an attempt's session and is in the ticket's `sessions` (with `persona`, `attemptId` null).
  `reuse` (Coder): the session this node used earlier in the run, else the attempt's agent session, else a new
  one that becomes the attempt's. `own` (Reviewer, QA, Review the fix; PDT-183): the session this node used earlier
  in the run, else a new one like `new`'s: it never falls back to the attempt's session and never becomes it, so a
  Reviewer never lands in the Coder's conversation, and three review rounds make one Reviewer session. A conversation the user didn't stop can be reused even when it isn't running (a
  pdtd restart interrupted it): the step's message reopens it (PDT-144). A terminal only while it runs, or while a
  restart is about to resume it; one that stopped can't take a message, so the step starts a new conversation with
  the whole first message and, on a later pass, why it is back. Every new session a step starts on a ticket opens its tab at the end of the
  attempt's Start workspace (after the ticket's tab and the sessions before it: Coder, Reviewer, QA), else the
  project's workspace (a Refiner before any attempt, or the workspace is gone), as a session the user's Start makes
  would (PDT-103, PDT-105); each pass's new session opens its own tab, and earlier passes' stay like other ended
  sessions. A step with no ticket opens no tab. A reused session gets the step through the outbox
  (never into a running turn), with the step's rules in the message since its instructions were fixed at its start.
  On a later pass in the same session (sent back by Review, QA or a rejection) it gets only why it is back (the
  sending step's outcome and summary, then its line comments as a list of findings, PDT-161; or the user's gate
  choice and note: pdt-core `next_pass_message`), the ask to address each point, and the outcomes, not the task and
  rules again.
- **Closing sessions** (PDT-183). Once a step's outcome is in (the agent's, or the user's Take over), its session
  closes when the turn that handed it in ends (`agent::close_step_session`): a `new` step's is stopped (Finished), an
  `own` step's is parked at once (its process closes; the node's next pass reopens it) instead of after the 15 min
  idle park; a `reuse` step's goes on. When a run ends (succeeded, failed or cancelled), its steps' sessions still
  open are stopped (`agent::run_ended`; a cancelled run's at once, mid-turn too), except the attempt's own session
  (the Coder's) and the session of a timed-out step the run can still be resumed from (a late outcome, PDT-123). A
  session a step took up meanwhile is left alone. Transcripts stay in the session history until the ticket's
  cleanup.
- **Ticket and attempt**: the config's, else the trigger's `ticketId` / `attemptId`, else the ticket's active attempt.
  A step with neither runs in the run's (or workflow's) project. One with no project either (a personal or scheduled
  workflow, PDT-152) runs in the host project as the PM does ([agents.md](agents.md) "The PM"): no ticket, attempt or
  tab, the PM's work folder as its folder, the PM's context and briefing in its instructions with the step's "Your
  role" section in place of the persona's, and its persona's tools (the PM's Scryer tools, say). It is not one of the
  PM's conversations (no rotation or journal). A dry run resolves the step as a real run does: a missing or archived
  persona is its error, and it says when the step would run in the host project (a ticket or attempt id it can't
  find, as in a hand-written event, counts as one that would be there).
- **Permission mode** of a new session: the project's `agentPermissionMode` (project settings), else the persona
  version's, else the host default (`ticketPermissionMode` on a ticket, `sessionPermissionMode` otherwise).
- **Instructions and first message.** The session's instructions get a "Your role: <persona>" section (the persona's
  instructions, the outcomes, and the rule: finish with `submit_outcome`, ask with `ask_user`, a turn with neither
  counts as stalled). The first message is the node's `prompt`, the ticket's key and title, and the outcome list.
- **Binding.** `agent_steps` (one per step: session, persona and version, project, ticket, attempt, outcomes,
  `state` `working | asking | stalled | done`, turns, nudges, the background-task wait) is what pdt-mcp and the Inbox read. The step waits
  (`wait: {agent: …}`) until `maxRuntimeMs` (4 h), then `timeout` (the Inbox has it: [Timeouts and
  Resume](#timeouts-and-resume-pdt-123)); a step re-run after a restart waits again.
- **Outcome.** `submit_outcome` checks the outcome is one of the step's (else an error the agent sees), refuses
  `submitted` (the outcome that hands work on, PDT-161; a timed-out step's late one too) while the attempt's worktree
  has uncommitted changes to tracked files or commits its upstream lacks (pdt-git `unpushed`; the upstream, else
  `origin/<branch>` when that exists; a branch on neither is refused as not pushed when the repository has an
  `origin` remote, and only checked for uncommitted changes when it has none; untracked files don't count; the error names the files and commits: pdt-core `not_handed_in`), adds its line
  comments as review comments on the attempt's worktree (author: the persona), records an `outcome` timeline event
  and completes the step by that port with `{outcome, summary, comments, data, sessionId, persona, personaVersion,
  turns, by}`. `data` (PDT-154) is the object the agent handed in with `submit_outcome` (fields the prompt asks for,
  such as an issue number; `{}` without), kept on the binding (`agent_steps.data`), so later steps read
  `{{steps.<id>.data.<field>}}`. The session stays as it is.
- **Turns.** A conversation's turn end (pdt-api `conversations.rs`) or a terminal going idle (`activity.rs`) is
  reported for a working binding (a reused session's turn before the step's message was delivered doesn't count):
  `turns + 1`; at `maxTurns` (10) the step takes `timeout`; while `nudges` (1) remain PDT sends "[PDT] You ended your
  turn without `submit_outcome`…"; after that the binding is **stalled** and the Inbox has it. The engine tick also
  stalls a working step whose session stopped (not running, or a conversation with no live process, turn or queued
  message) for 30 s, or was removed (its `session_id` cleared). While the step's message to a reused session is still
  waiting or on its way, the step isn't stalled (a conversation reopens to take it, or the cap holds it); once that
  message ends `untold` (lost on the way, or nobody could take it), the step stalls at once with "Its message was never
  delivered: <the message, quoted>" and its Stalled item offers **Resume with this message** (`resend`, primary),
  which sends it again as the user's (reopening a stopped conversation), as well as Nudge and Take over.
- **Background tasks (PDT-142).** A conversation's turn that ends while background tasks the agent started still run
  (Claude's `background_tasks_changed`, kept by `conversations.rs`; a long pytest, a CI watch) isn't counted, nudged
  or stalled: the binding records `background_since` and the tasks' descriptions, and the ticket's agent state is
  `background` ("waiting on background task: …"). Claude takes a finished task up in a turn of its own, which is
  judged as usual. If the tasks all end and no turn starts for a grace period (10 s), the engine tick counts the
  waiting turn as ended without an outcome (nudge, then stall). While they run, only the cap stalls the step:
  `backgroundWaitMs` (30 min, per agent node) after the first such turn end, with "Waiting on background tasks for
  X min: <descriptions>." (never while a turn runs or a message or request is open, and only while the step still
  waits; a binding whose step ended is done instead). A Nudge from the Inbox starts the wait over. Terminal (TUI) sessions report no background
  tasks, so their turns are judged as before; Codex and Devin conversations don't report them either.
- **Late outcomes.** A stalled step whose session submits an outcome after all (it went on by itself) completes as
  usual, and its Stalled item closes.
- **Failed starts.** A step whose session would not start fails (its binding, which has no session, is done; one cut
  short after its session started keeps it and goes on there); a Retry, a node retry or a
  restart that runs the same step again finds a binding with no session (or done without an outcome) and starts over
  instead of waiting for it.
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
diff). Built-ins Refiner, Coder, Reviewer, QA, Triage (locked down for untrusted input, PDT-201, [agents.md](agents.md))
and PM (the project manager, [agents.md](agents.md#the-pm-pdt-81); permission mode `ask`, the PM's tools) (pdt-core `persona.rs`) are installed on the engine's first tick (and
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
- `GET /api/personas/tools` (PDT-110) → every pdt-mcp tool (every integration's, in `tools/list` order) as `{name, does, asks,
  default}`: the persona page's tool list, so a new tool shows there with no web change. Saving keeps every tool the
  persona has, a name the server doesn't list included (shown checked, to uncheck).

## Pipeline, policies and scheduling (Phase 7c)

- **The Pipeline template** (`builtin:pipeline`, pdt-api `workflow/pipeline.rs`): instantiated per project (off until
  enabled); an untouched instance follows newer versions of it (Templates, above). `ticket.refine_requested` → Refine (agent `{{params.refiner}}`, new session, `pipelineStage: refine`):
  `ready` → `ticket.move` to `{{params.afterRefine}}` (To do by default; an external ticket the user hasn't let in goes
  to Ready, `externalStage`, PDT-201); `needs_clarification` → a gate (refine again
  | stop). `attempt.started` → Code (Coder, `session: reuse`, `loopLimit: {{params.loopLimit}}`) → Review (own) → QA
  (own; the Code prompt asks for a changelog entry when the project keeps one, the Review prompt checks it, PDT-193); `changes_requested` and `fail` go back to Code (whose pass lists the Reviewer's or QA's findings first; a PR
  notice for the attempt that comes meanwhile goes in after them, and `submitted` needs the worktree committed and
  pushed, PDT-161: see "Agent steps" and the outbox above); `pass` → a switch on `{{params.accept}}`: `user` → a
  condition (`{{trigger.labels}}` `contains_any` `{{params.autoAcceptLabels}}`): a ticket with one of those labels
  goes straight on, any other to Accept (a gate with `inbox: accept`, `pipelineStage: accept`, `approve: merge`;
  `reject` → Code); `auto` → straight on; then
  a switch on `{{params.merge}}`: `workflow` → `attempt.merge` (`pipelineStage: merge`; `merged` → `ticket.finish`;
  `retry` 3 times, 2 minutes apart, for GitHub not answering (`merge_failed`, `ci_unreadable`);
  `queued` → end, Finish merged tickets finishes it, and PR notices gets it merging again after a failure, PDT-104;
  `refused` → a switch on `{{steps.merge.code}}`: `ci_pending` (PDT-151: CI on the PR's head still running, or not
  started) → Wait for CI (a `wait` for this attempt's `ci.run_passed` `also` `ci.run_failed`, `any`: on the head the
  merge was refused at (`{{steps.merge.headSha}}`, so also one logged between the merge's read and the wait) or with
  `mergeWaiting` (a newer head pushed while it waits); `pipelineStage: merge`, 6 hours) → `event` → `attempt.merge` again, `timeout` → Merge blocked;
  anything else → a condition on `{{steps.merge.fixable}}`, PDT-124). Fixable (a conflict, uncommitted changes,
  failing CI; not recorded as set
  to merge, so nothing merges the fix before its review, even when the loop ends early) → Fix the merge (agent `{{params.coder}}`, `session: reuse`, owner
  `{{params.codeBy}}`, `pipelineStage: code`, `loopLimit`; the prompt has the refusal's reason) → Review the fix (agent
  `{{params.reviewer}}`, `session: own`, `pipelineStage: review`): `approved` → `attempt.merge` again,
  `changes_requested` → Fix again; a later refusal's reason is what Fix hears on its next pass. Not fixable (the
  policy, a draft, no PR, …) → Merge blocked (a gate, Needs you: "Merge blocked on <key>: <reason>", `merge_again`
  (records the approval, then `attempt.merge`) | `stop` (quiet end), `pipelineStage: merge_blocked`, `event`: this
  attempt's `attempt.merged` → end, closing the item; 3 days → `timeout`). `person` → a wait for this attempt's
  `attempt.merged` (`pipelineStage: merge`, `timeoutMs` 3 days); its `timeout` → Still not merged (a gate:
  `keep_waiting` → the wait again | `merge` (records the approval, then `attempt.merge`) | `stop`; ends on the merge
  too). The Inbox item of either gate offers Open PR when the attempt has one. `timeout` and `stopped` add a ticket
  comment and end (a timeout is also in the Inbox, and the run can be resumed: PDT-123). Params: the four personas, `refineBy`/`codeBy`/`reviewBy`/`qaBy` (`agent`
  or `user`: manual), `afterRefine`, `accept`, `autoAcceptLabels` (a list, default `["agentic"]`; the labels the ticket had
  when it started count; a workflow that saved its own list, even `[]`, keeps it, PDT-200), `merge`, `loopLimit`. Runs may last 30 days. A ticket accepted by label has no approval,
  so with `requireApproval` on its merge is refused (Merge blocked asks; Merge again records the approval) unless the
  label is also in `approvalExemptLabels`.
- **Manual stages**: an `agent` node with `owner: user` starts no session. Its binding's state is `manual` and the Inbox
  gets a `manual` item ("Your turn: <node>", group Gates) with the prompt and one `outcome:<id>` choice per outcome;
  answering (with an optional `text`, the summary) completes the step `by: user`.
- **`pipelineStage`** on `agent` and `gate` sets the ticket's stage when the step starts ([tickets.md](tickets.md)).
- **`approve`** on a gate: taking that choice records the user's approval of the run's attempt at its branch head (a
  `review` event naming the workflow), which `requireApproval` needs before an automated merge.
- **`attempt.merge`** runs the Merge command as an automated merger ([review.md](review.md#merge)): a policy refusal
  or anything that stops the merge leaves by `refused` (`code`, `reason`, `fixable`: a conflict, uncommitted
  changes or failing CI (`ci_failed`), which the attempt's agent can clear; `ci_pending` while CI on the pull
  request's head hasn't passed, PDT-151, with `headSha`, the head it judged); `merge_failed` and `ci_unreadable`
  (GitHub couldn't be read for CI) are errors. It reads the PR from GitHub first: one merged already ends the
  attempt as `detected` with its number and leaves by `merged`, and it never merges a branch locally that has or had
  a PR, while GitHub can't be read, or in a project whose attempts merge through PRs (PDT-181, `refused`). Merging `again` refused for CI records the
  attempt as set to merge, so PR notices merges it once CI passes. With `again`
  (PR notices after a fix, PDT-104) it reads the PR from GitHub first: one already on auto-merge or in the queue is
  `queued` (`already: true`); and it sends no `finishOnMerge`, so the first merge's choice stays. A first merge
  (not `again`) refused as `fixable` records that choice anyway and sets `attempts.merge_refused` (PDT-139, sticky):
  the PR's later `ci.run_failed` and `pr.*` events carry `mergeRefused` (while it is open), so PR notices puts the
  Coder on them instead of only telling an agent. It is not "set to merge" (`mergeRequested` stays false, PDT-124);
  once the attempt is set to merge (the remerge after the fix) that replaces it, and a person turning auto-merge off
  clears both, so it stays off (PDT-104).
- **Policies** (project settings `policy`: `mergeBy` `agents | user | external`, `requireApproval`, `requireGreenCi`
  (default on: an automated merge of a pull request waits for green CI on its head, PDT-151),
  `approvalExemptLabels` (tickets with one of them need no approval; normalised like ticket labels, `[]` clears),
  `maxRunningAttempts`, `autoStart` `off | todo | ready`, `autoStartOrder` `priority | rank` (default `priority`: the most urgent
  candidate first, PDT-218), `startDelaySeconds` (auto-start's quiet period, default 120,
  0 at once); `PUT` takes them as top-level fields, `maxRunningAttempts` 0 clears it): Merge and Start enforce them ([review.md](review.md#merge), [tickets.md](tickets.md)). Web: the
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
`timed_out` (group Stalled, PDT-123), `notice` (group Unheard notices, after Loop limit, PDT-139: made by
`inbox.notice`, with no step, so it outlives its run), `secret` (group Questions, `request_secret`, PDT-158:
answered `{secret}` in a masked field, [secrets.md](secrets.md#agents-pdt-mcp)); one
open item per step and kind. A `notice` closes (`resolution` `{settled: true}`) when its cause is gone: the
attempt ended (merged, abandoned) or its PR, as the PR check last saw it, is no longer open or shows the notice
settled (`pdt_core::pr::notice_settled`: a conflict mergeable again, CI passed, back in the queue or on auto-merge,
caught up with its base), checked on every notifications `sync` (`inbox::settle_notices`); and at once on the
attempt's `ci.run_passed` (its CI notices, Buildkite's too) and `pr.merged` (all of them). Dismiss closes one too. Resolved and closed items are deleted 30 days after they ended (`maintain_workflows`);
open ones stay however old. Approvals are
not stored: the open permission requests of live conversations (`approval:<session>:<request>`), answered in their
session. `GET /api/inbox?project=` → `{items, total}`, Questions, Approvals, Stalled, Loop limit, Gates, Ready to
accept, oldest first within each, with ticket key and title and persona name. `POST /api/inbox/{id}/answer` takes
`{choice}` (a gate's choice, with an optional `text`, the user's comment; `nudge` with an optional `text`, `resend` (the undelivered message in `body.message`) or `outcome:<id>`, for a stalled
step; `again`,
`outcome:<id>` or `stop` at a loop limit; `resume` with an optional `text`, the note, or `stop` for a timed-out step)
or `{answers}` / `{text}` for a question (a notice takes no answer); `POST /api/inbox/{id}/dismiss`
closes a question, a secret request or a stalled item (the step keeps waiting) or a notice. A gate answered through
`/api/workflow-runs/{run}/steps/{step}/answer` resolves its item too. When a session is removed (and when the store
opens, for older ones) its open questions close (`resolution` `{sessionGone: true}`) and no item keeps its
`sessionId`, so the Inbox never offers Open session for a session that is gone (PDT-93). **Sessions waiting** (group
`session`, kind `session`, id `session:<session>`, after Approvals) are the live `session.needs_you` notifications
([Notifications](#notifications)): not answerable here (Open session, Enter); viewing the session takes it out. `total`
counts every item, sessions waiting included: the rail badge shows it.

The page title and favicon (PDT-168, PDT-207; web/src/notifications/attention.ts `useAttention`) read the queries the
rail already reads (the notification stream refreshes them). The title is prefixed with the Inbox's open item count
(`(3) …`, the rail badge's number) and nothing else. The favicon gets a dot coloured by the most important thing
waiting: red (`--pdt-failed`) while a session needs the user (its `needs_you` activity or its Inbox item), amber
(`--pdt-waiting`) while the Inbox has any other item, the accent (`--pdt-accent`) while an agent or run has finished
unseen or a PM reply is unread; the dot is ringed in `--pdt-surface` so it reads on light and dark tab bars. With
nothing waiting the plain icon and title come back.

Web: the rail's Inbox (web/src/inbox; the open count on its icon, a dot when Updates are unread, `Ctrl+] i`): the list
has two sections, **Needs you** (the items, grouped by kind) and **Updates** (the notification feed, newest first, unread
in bold, cleared dimmed, Mark all read, Clear all, a Clear button on each row (on hover; always on touch), a link to
Settings → Notifications; selecting one reads it and shows Open, its target, and Clear); opening an item reads its notifications. The work
area shows the item and its decision (answers per question, the approval card, Nudge / Take over, Once more / Take
over / Stop, Resume (with a note) / Stop, a gate's choices, a manual step's outcomes, an unheard notice's text with
Open ticket and Dismiss); `j`/`k` move, Enter takes the primary action,
`o` opens the ticket, `e` dismisses. A gate has a note box: the note goes with the choice (as the user's ticket
comment, so a rejection tells the coder what to change). An item on a ticket with an attempt also offers Open coder
session (the attempt's running agent session), Open changes and Run… (the worktree's configured runs and tests); each
opens its tab right after the ticket's tab in the workspace that has it (its Start workspace; AppShell's
`TicketTabsContext`, `openChanges` with `ticketId`), moving an existing changes tab there (a session or run tab that
exists is shown where it is), and switches to it.

On a phone (PDT-117, web/src/inbox/PhoneInboxList.tsx and PhoneInbox.tsx) `/inbox` is the list: **Needs you** (its
count) | **Updates** (a dot while any is unread), and **Review all (n)** at the bottom opens the first item.
`/inbox/<item>` shows one item (the top bar says "Approval · 2 of 4"): its ticket and step in brief, the kind's content
(it scrolls) and its decisions in a bar fixed at the bottom, 48px, primary on the right (Question: the choices as big
rows, Send; Approval: Deny · Always · Allow; Stalled: Take over (its outcomes in a bottom sheet) · Nudge, with Resume with this message on the right when its message
never arrived; Unheard notice: Dismiss · Open ticket; a loop limit's,
gate's, manual step's or timed-out step's own choices; Session waiting: Open session); more than three choices fold
into **More ▾** (a bottom sheet). A decision shows the next item in place (Back returns to the list), "All clear" after
the last; a horizontal swipe moves between items without answering. ⋯ has Open session, Open ticket and Dismiss.
Merge, and the primary choice under Ready to accept, ask first: the PR and its CI, the diff size and View changes.
An item that fails to render shows "Something went wrong" with Reload and Back to Inbox (an error boundary around
the item, web/src/common/ErrorBoundary.tsx; another around the app's routed views), and logs the error to the console
(PDT-209).

## Notifications

PDT-108 (spec: the ticket). pdtd makes each notification once, in `notifications` (pdt-store `notifications.rs`), and
every client shows it from there. Types and rules: pdt-core `notification.rs`; the Notifier: pdt-api `notifications.rs`.

- **Kinds and Show.** Needs you: `session.needs_you`, `inbox.question`, `inbox.approval`, `inbox.gate` (gates and Ready
  to accept), `inbox.manual`, `inbox.stalled` (a stalled or timed-out step), `inbox.loop_limit`, `inbox.notice` (an unheard
  notice, PDT-139). Updates: `session.turn_ended`, `session.exited`,
  `session.undelivered` (a message for an agent never reached it: [Outbox](#outbox-sessionsend)), `pr.ci_failed`,
  `pr.conflict`, `pr.queue_left`, `pr.merged`, `workflow.failed`, `workflow.notify`, `connection.failed`
  ("Sensors and webhooks"), `host.loop_stale` (a background loop of pdtd stopped finishing passes, [host.md](host.md)),
  `todo.due` (a todo came due, PDT-140: [workspaces.md](workspaces.md) "Right panel and todos").
  Each kind's Show
  is `off`, `dot` (only the target's own marker: no row), `inbox`, `toast` or `desktop`, each including the ones
  before; defaults in `NotificationKind::default_show`. The Inbox's own kinds (`inbox.*`) never go below `inbox`.
  Overrides live in host value `notifications.show` (kind → Show); `GET /api/notifications/settings` lists every kind
  (section, label, when, show, default, allowed), `PATCH` `{kind: show | null}` (null restores the default).
- **Rows.** Only `inbox` and up are stored, with the Show at creation. One live row per `dedupe_key`
  (`session:<id>:needs_you|turn_ended|exited`, `inbox:<item>`, `attempt:<id>:ci_failed|conflict|queue_left|merged`,
  `run:<id>:failed`, `notify:<step>`, `connection:<name>`, `loop:<name>:stale`, `todo:<id>:<dueOn>[T<HH:MM>]`); a cause firing again updates its row
  (title, body, `created_at`, unread again).
  `seq` grows on every change (created, updated, cleared). Pruned after 30 days, except live Needs-you rows.
  The user can clear an Update (PDT-224: `dismissed_at`, also read): it leaves every list and the Inbox's dot for good.
  Its row stays live while its cause lasts, so the same cause doesn't raise it again: state causes find it live and
  make nothing, and a cause firing again updates it out of sight (no stream event, no toast). Once the cause went
  away (the row is cleared) a new one is news again. Needs-you rows can't be cleared this way.
- **State causes** (`sync`, every 2 s and at once on a kick: activity changes, a session viewed, an Inbox answer or
  dismiss, a settings change): a running agent session that needs the user and is unseen (`session.needs_you`, not
  for a conversation with an open approval: that is `inbox.approval`), or idle and unseen for 30 s
  (`session.turn_ended`, only above `dot`); open Inbox items and approvals; stale background loops
  (`host.loop_stale`, severity error, default `toast`); open todos whose due moment has passed (`todo.due`, default
  `toast`, target `{kind: "todo", id}`: one per todo and due date, remembered on the todo as `reminded_key`, so a
  dismissed one doesn't come back and one missed while pdtd was down comes once when it starts). A row whose cause
  went away is cleared: a todo done, deleted or re-dated, a loop's next pass,
  the session left that state or was viewed (`last_viewed_at`), the item resolved or closed, an exited session
  restarted or removed, a conflict's PR mergeable again (or closed), a dropped PR back in the queue. Viewing a session
  reads its rows (`POST /api/sessions/{id}/view`).
- **Event causes** (`notify` where they happen): a terminal agent session's process exiting non-zero without PDT
  killing it (Stop, close, restart: `AppState::kill_pty`/`remove_pty` mark it) or shutdown (`session.exited`); the PR
  poller's and Buildkite's `ci.run_failed`, `pr.conflict`, `pr.queue_left`, `pr.merged` as they are logged
  (`ci.run_passed` clears `ci_failed`; `pr.merged` clears the attempt's other three); a run ending failed
  (`workflow.failed`); `notify` steps; a sensor's connection starting to fail (`connection.failed`, cleared when it
  works again or is saved); a message given up on (`session.undelivered`, severity warning, default `toast`, key
  `undelivered:<message id>`).
- **API.** `GET /api/notifications?section=needs_you|updates&project=&unread=&limit=&before=` → `{items,
  unreadUpdates}` (Needs you lists live rows only); `POST /api/notifications/{id}/read`; `POST
  /api/notifications/read-all {section?, sessionId?, inboxItemId?}`; `POST /api/notifications/{id}/clear` (an Update;
  409 `needs_you` for a Needs-you row) and `POST /api/notifications/clear-all` (every Update); `GET /api/notifications/stream`, Server-Sent
  Events `created`/`updated`/`cleared` with the row (event id = `seq`; `Last-Event-ID` gets what was missed, as
  `created` or `cleared`; it ends when pdtd shuts down).
- **Web** (`web/src/notifications`): AppShell follows the stream (`useNotificationStream`, EventSource) and refreshes
  the Inbox on every row. `presentation` decides a new row's toast (Show `toast`+, page visible, not the session in
  the focused tab or the open Inbox item) or desktop notification (Show `desktop`, page hidden, this device allowed
  them, this tab holds the `navigator.locks` lock so one tab per browser shows them; `tag` = dedupe key, closed when
  the row clears); rows older than 2 minutes on arrival show neither. Clicking opens the target (session, ticket,
  ticket key, Inbox item, workflow, URL, Settings → All connections). Settings → Notifications: This device (Desktop notifications, localStorage
  `pdt.desktopNotifications` and the browser's permission, with Test) and What shows (a Show per kind, Needs you and
  Updates, Reset to defaults).
