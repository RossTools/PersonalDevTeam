# Personal Dev Team — Phase 7e: integrations (triggers, actions, connections)

Status: design for PDT-80, 2026-10-20. Grows 7a's catalog (docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md,
docs/reference/workflows.md) with the triggers, actions and connections that need no agent (vision §4, §12:
docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md). Agent templates and the Hermes cutover are 7f (PDT-84).

7e ships in three parts, each its own ticket and plan, all from this spec:

| Part | Ticket | Sections |
|---|---|---|
| 7e-1: schedule, webhook, GitHub issues and PR comments; command, HTTP, Discord, notify; templates and the four agent-free Hermes jobs | PDT-80 | §2–§5 |
| 7e-2: Jira Cloud and Buildkite (work setup) | PDT-85 | §6 |
| 7e-3: Radarr, Sonarr, Plex; MCP tools as actions; brief archive | PDT-86 | §7 |

Dropped for now (decided with the user 2026-10-20; PDT-91): email search and archive, Google Calendar upsert. The
Hermes jobs that used them run their existing scripts as command steps (§5).

## 1. Shared rules

- **One module per kind** in pdt-api `workflow/nodes/`, one line in `catalog.rs`, config and output types deriving
  serde, schemars and ts-rs, as 7a. Every kind has a test; external services are always fakes in tests (a local HTTP
  server on a port the OS picks, `FakeGithub`), never the real service.
- **Connections hold every URL and credential** (7a §7). A node names its connection (`connection: "discord-alerts"`);
  a URL or token is never written inline in a document, because documents are versioned, diffed and may live in a repo.
  Each connection kind documents its `url`, `config` and what its secret is.
- **Dry runs never call out.** Actions report `would` (method and URL, program and args, the message) and take their
  success port with an empty output.
- **Quiet.** An action whose output has `quiet: true` counts as having done nothing (7a §4.3), so a run whose only
  action said so ends `quiet`. Command and HTTP pass a `quiet` the script or service returns.
- **Sensors emit events; workflows react.** New sensors (GitHub issues, PR comments, schedules, webhooks; Jira and
  Buildkite in 7e-2) only write events with dedupe keys. What happens next (tell the agent, post to Discord, import a
  ticket) is a workflow, built in where PDT needs it to behave out of the box.
- **Sensor baselines** live in `host_settings` under `sensor.<name>.<key>` (no migration): a sensor's first look at a
  source records where it is and emits nothing, so enabling a watcher never floods.

## 2. Triggers

### 2.1 Trigger configs and matching

7a's triggers all share `TriggerConfig` (`projects`, `where`, `runKey`). The new triggers add their own fields, so the
config becomes the shared fields (flattened) plus the kind's own, and `NodeKind` gains one method:

```rust
/// Whether this trigger node fires for `event` (beyond the shared `projects` / `where` checks). Default: yes.
fn accepts(&self, config: &Value, wf: &Workflow, node: &Node, event: &LogEvent) -> bool { true }
```

The engine's matcher and the dry run call it after the shared checks. Schedule and webhook triggers use it to fire only
for their own events.

### 2.2 Schedule (`schedule`)

Config (besides the shared fields):

| Field | Meaning |
|---|---|
| `every` | an interval (`"90s"`, `"15m"`, `"1h"`, `"1d"`), at least 1 minute |
| `cron` | a 5-field cron expression (`"0 7 * * 1-5"`); exactly one of `every` and `cron` |
| `timezone` | an IANA zone for `cron` (default the host's local zone) |
| `catchUp` | after pdtd was down, run once for the latest missed slot (default `true`); `false` skips missed slots |

- **The scheduler** runs in the engine tick: for each enabled workflow's `schedule` node it computes the latest slot at
  or before now. It emits `schedule.fired` (key `schedule:<workflow>:<node>:<slot ms>`, so a slot fires once however
  often the tick runs) with payload `{workflowId, nodeId, slot, slotIso, date}` (`date` is the slot's local
  `YYYY-MM-DD`, handy for run keys: `runKey: "brief:{{trigger.date}}"`).
- **No backlog.** The last slot handled is kept in workflow state under the reserved key `_pdt.schedule.<node>`; a newly
  enabled (or newly saved) schedule records the current slot as its baseline and first fires at the next one. After
  downtime only the latest missed slot fires, and only with `catchUp`. Keys starting `_pdt.` are hidden from
  `{{state}}`, and `state.set` refuses them.
- `accepts`: the event's `workflowId` and `nodeId` are this node's. The workflow's scope still applies; a schedule
  belongs in a `personal` or `host` workflow, or a `project` one (its `projectId` is the workflow's project).
- `every` slots are aligned to the Unix epoch (a `15m` schedule fires at :00, :15, …), so restarts don't drift them.
- Cron parsing: a crate (`croner`, added with `cargo add`), with `chrono-tz` for zones.

### 2.3 GitHub issues (`github.issue.opened`)

- **Which repos.** The sensor (pdt-api `github_issues.rs`, every 60 s with the PR check) polls each repo named in an
  enabled workflow's `github.issue.opened` node (`repos: ["owner/name"]`), plus, for a node with no `repos`, the GitHub
  repo of each project in its scope (a project workflow: its project). A repo needs no PDT project, as Hermes's
  watchers don't.
- **Reading.** `gh api repos/<repo>/issues?state=all&sort=created&direction=desc&per_page=50` (pull requests in the
  answer are skipped), through a new `GithubApi::recent_issues` so `FakeGithub` serves tests.
- **Baseline.** The newest issue number per repo is kept in `sensor.github_issues.<repo>`; the first poll of a repo
  records it and emits nothing. Later polls emit one event per issue with a higher number.
- **Event** `github.issue.opened`, key `github:<repo>:issue:<number>`, payload `{repo, number, title, body, url, author,
  labels, createdAt}`, `projectId` when the repo is a project's. Config adds `repos` and the usual `where`
  (`{{trigger.labels}} contains bug`).

### 2.4 PR comments and reviews (`pr.commented`, `pr.reviewed`)

The PR poller (7a) also reads the comments and reviews of each active attempt's open PR, through `GithubApi`:

- `pr.commented`: a conversation comment or a review comment (on a line). Key `<attempt>:comment:<comment id>`;
  payload `{pr, url, commentId, author, body, path?, line?, inReview, byOwner, isBot, notice}`. `byOwner` is the
  repo's authenticated user (the user themself); `isBot` a `[bot]` login or `type: Bot`.
- `pr.reviewed`: a review submitted. Key `<attempt>:review:<review id>`; payload `{pr, url, reviewId, author, state
  (approved | changes_requested | commented), body, notice}`.
- The first read of a PR records the newest comment and review ids it has (baseline `sensor.pr_comments.<attempt>`),
  so an existing PR's history is not replayed.
- `notice` follows 7a's shape, with a text such as "A review comment on PR #12 from alice on src/a.rs:40: …", so a
  workflow can deliver it unchanged.
- New built-in, **off** by default: **PR comments to the agent**: `pr.commented` and `pr.reviewed` (where not `byOwner`
  and not `isBot`, and not a bare `approved` review) → `session.send` (`text: {{trigger.notice.text}}`, `dedupe` on the
  notice key, `timeline: pr`).

### 2.5 Webhook (`webhook`)

- **Endpoint** `POST /api/hooks/<name>`, where `<name>` is a connection of kind `webhook` (its secret is the shared
  secret). It answers `202 {eventId}` (or `200 {duplicate: true}`) at once; workflows run on the next tick.
- **Reachable** on the local address and, with remote access on, over Tailscale. The Host guard applies as everywhere;
  the identity guard (owner's `Tailscale-User-Login`) does not apply to `/api/hooks/*`, because the senders are other
  machines on the tailnet (tagged servers, a NAS) that have no user identity. The secret is what authenticates them,
  so a request without a valid one is `401 bad_signature` and records nothing. Funnel (public) stays unsupported.
- **The secret** is checked as either `Authorization: Bearer <secret>` or GitHub's `X-Hub-Signature-256: sha256=<HMAC
  of the body>`, compared in constant time. A connection without a secret refuses every request (`409
  webhook_no_secret`).
- **Limits.** Body up to 1 MiB; JSON bodies are parsed, others kept as text.
- **Event** `webhook.received`, key `webhook:<name>:<delivery>`, where delivery is the `X-GitHub-Delivery`,
  `Idempotency-Key` or `X-Request-Id` header, else a new id (no dedupe). Payload `{hook, method, query, headers (the
  `content-type`, `user-agent` and `x-*` ones), body}`.
- **Trigger** config: `hook` (the connection name), plus the shared fields. `accepts`: the event's `hook` is this one.

## 3. Actions

| Kind | Config | Ports | Output |
|---|---|---|---|
| `command` | `program`, `args`, `cwd`, `input`, `env`, `parse` | `out` | `{exitCode, json?, stdout, stderr}` |
| `http.request` | `connection`, `method`, `url`, `headers`, `query`, `body`, `expect` | `out` | `{status, headers, body}` |
| `discord.post` | `connection`, `content`, `username`, `embeds`, `thread` | `out` | `{messageIds}` |
| `notify` | `title`, `body`, `level`, `link` | `out` | `{eventId}` |

Every action also has 7a's `error` port, `retry` and `timeoutMs`.

### 3.1 `command`

- Runs `program` with `args` (no shell; a list, each item a template), JSON `input` written to stdin (then closed),
  `env` added to pdtd's environment. `cwd`: `scratch` (default: a fresh folder under
  `<data dir>/workflow-scratch/<run id>`, removed when the run ends), `project` (the run's project's main checkout),
  `attempt` (the run's attempt's worktree) or an absolute path.
- The node's `timeoutMs` (default 5 min) kills the process tree on expiry (its own process group on Unix,
  `taskkill /T` on Windows), so a hung script never hangs the engine.
- `parse` (`json` default, or `text`): with `json`, stdout must be one JSON value, which becomes `json` (and its
  `quiet`, if an object has one, becomes the step's). A non-zero exit, or unparseable stdout with `parse: json`, is an
  error (`exit 2: <last stderr lines>`), so retry and the error port apply. `stdout` and `stderr` are kept up to 64 KiB
  each (the end of stderr).
- Windows: `program` is found as `Command` finds it (an `.exe` on PATH; a `.cmd` needs `cmd /c`); no console window.
- Consent: a command runs whatever the document says, as the user. Workflows from repo files start disabled (7a), and
  enabling one is the user's consent (vision principle 5); the runs view shows the command line of every step.

### 3.2 `http.request`

- `connection` is optional; with one, `url` may be relative to its `url`, and its secret is sent per its config:
  `auth: bearer` (default), `basic` (`config.username`, secret the password), `header` (`config.header`, the secret its
  value) or `query` (`config.param`).
- `method` (default `GET`), `headers`, `query` (an object), `body` (JSON value → `application/json`, a string → as is),
  `expect` (the status codes that count as success: a list or `"2xx"`, the default; `"any"`).
- Output `body` is parsed JSON when the response says JSON, else text (up to 1 MiB). A status outside `expect` is an
  error naming the status and the start of the body; a network error or timeout is an error. Transport: `reqwest` (as
  pdt-update), native TLS, the system proxy.

### 3.3 `discord.post`

- `connection`: kind `discord_webhook`, its secret the webhook URL (Discord's URL is itself the credential).
- `content` (Markdown); longer than Discord's 2000 characters it is split at line breaks into several messages, in
  order. `username`, `embeds` (Discord's JSON, passed through), `thread` (a thread id to post into).
- A 429 waits `retry_after` (up to the node's timeout) and tries again; other failures are errors.
- Bot-token posting (to reply in channels, for 7f's PM over Discord) is later: a `discord_bot` connection kind.

### 3.4 `notify`

PDT-8 (notifications) is still a draft, so `notify` is the smallest useful version: it writes a `pdt.notified` event
(`{title, body, level: info | warning | error, link?, workflowId, runId}`) to the log. The web app follows these
events (`GET /api/events?type=pdt.notified&after=<seq>`, with the 2 s poll) and shows each as a toast, plus a system
notification through the browser's Notification API when the page is hidden and the user allowed it in Settings. A
`link` that is a ticket key opens the ticket. PDT-8 later decides where notifications live and can read the same
events.

## 4. Templates

7d's templates (`GET /api/workflow-templates`, `POST /api/workflow-templates/{id}/instantiate`, params as
`{{params.<id>}}`, the instantiated workflow disabled; docs/reference/workflows.md "API") are how 7e ships its jobs:
the templates of §5 are more built-in templates in pdt-api `workflow/templates.rs`, listed in the Workflows view's New
workflow. Connection names are `string` params (their description says which kind).

## 5. The agent-free Hermes jobs

| Template | Graph |
|---|---|
| **New GitHub issues to Discord** (params: `repos`, `connection`) | `github.issue.opened` (`repos`) → `discord.post` ("New issue in <repo> #<n>: <title> <url>"). The sensor's baseline is the first run's silence. |
| **Health watchdog** (params: `url` or `command`, `every` default 1m, `connection`) | `schedule` → check (`http.request` with `expect: any`, or `command`) → `condition` healthy (status < 400 / exit 0) → `switch` on `{{state.healthy}}`: unchanged → quiet `end`; changed → `state.set healthy` → `discord.post` ("<name> is down: …" / "<name> recovered"). The first run only records the baseline (`state.set` with `ifMissing`, then quiet). |
| **Scheduled script to Discord** (params: `command`, `args`, `schedule`, `connection`) | `schedule` (run key per slot) → `command` → `condition` `{{steps.run.json.message}}` exists → `discord.post` (`{{steps.run.json.message}}`); else quiet `end`. The script prints `{"message": "…"}` when there is news and `{}` (or `{"quiet": true}`) when not. |

The scheduled-script template covers the two Hermes jobs whose native kinds were dropped: **brief follow-ups** (daily:
the watchlist script) and **email processing** (every 15 min: the email script). Each is the template with its
schedule and script. Moving a job over is: create from the template, dry-run, enable, then turn off the Hermes job once
the PDT one has run cleanly for a while (vision §12).

## 6. Jira Cloud and Buildkite (7e-2, PDT-85)

The work setup: a project whose issues are in Jira and whose CI is Buildkite. The design here fixes the shape; PDT-85's
plan settles the details.

### 6.1 Jira connection and project link

- Connection kind `jira`: `url` `https://<site>.atlassian.net`, `config.email`, secret an API token (basic auth),
  REST v3; comments are written as ADF (plain paragraphs from the text). Polled, no webhooks (no public address).
- A project links to Jira in its host settings (`project_settings`, new columns: the connection, a JQL scope such as
  `project = ABC`, and the done mapping: the transition name, default `Done`). Not in `.pdt/project.toml`: work repos may
  not take a `.pdt/` folder.
- **Links** (migration): `ticket_links_external (ticket_id, system 'jira', connection_id, key, url, unique(system,
  connection_id, key))`. The ticket responses carry `external: [{system, key, url}]`; the Board card and ticket page
  show the key as a link to the issue.

### 6.2 Jira sensor and events

Every 60 s per linked project: JQL `(<scope>) AND updated >= "<last poll - 1m>" ORDER BY updated ASC` with
`expand=changelog` and the comments since then. Baseline: the first poll records the time and emits nothing.

| Event | Key | Payload |
|---|---|---|
| `jira.issue.created` | `jira:<conn>:<key>:created` | `{key, url, summary, description, status, assignee, assignedToMe, reporter, issueType, priority, labels, ticketId?}` |
| `jira.issue.updated` | `jira:<conn>:<key>:updated:<updated>` | the issue fields plus `changed` (field names) |
| `jira.issue.transitioned` | `jira:<conn>:<key>:status:<changelog id>` | `from`, `to` plus the issue fields |
| `jira.issue.commented` | `jira:<conn>:<key>:comment:<comment id>` | `commentId`, `author`, `body` (text from ADF) plus the issue fields |

`ticketId` is set when the issue is linked, so ticket workflows can use it.

### 6.3 Jira actions and the built-in behaviour

| Kind | Config | Ports |
|---|---|---|
| `jira.import` | `connection`, `key`, `project` (PDT project) | `imported`, `exists` |
| `jira.create` | `connection`, `project` (Jira project key), `issueType`, `summary`, `description`, `fields` | `out` |
| `jira.transition` | `connection`, `key`, `to` (a transition or status name) | `out`, `unavailable` |
| `jira.comment` | `connection`, `key`, `body` | `out` |

- `jira.import` makes a Draft ticket from the issue (title the summary, description from ADF, a link) unless a ticket
  already links that issue (`exists`, with its id). Linking is the dedupe, so an issue is never imported twice.
- **Incoming** is a built-in (on; it does nothing until a project links Jira): **Import assigned Jira issues**:
  `jira.issue.created` and `jira.issue.updated` (where `assignedToMe` and no `ticketId`) → `jira.import`.
- **Import by key**: `POST /api/projects/{id}/jira/import {key}` (any issue, assigned or not) calls the same command;
  the ticket list gets "Import from Jira…".
- **Outgoing** is a built-in (on): **Done moves the Jira issue**: `ticket.stage_changed` (to `done`, the ticket has a
  Jira link) → `jira.transition` (the project's done mapping). Nothing else is pushed unless a workflow says so.

### 6.4 Buildkite

- Connection kind `buildkite`: `config.org`, secret an API token (read builds, write builds for retry/trigger). A
  project names its pipeline slug(s) in its host settings.
- **Sensor**: with the PR poller, for each active attempt of a project with Buildkite, read
  `GET /v2/organizations/<org>/pipelines/<p>/builds?branch=<branch>&commit=<head>`, map builds and failed jobs to 7a's
  provider-neutral CI status (merged with GitHub's checks for the badge), and emit `ci.run_failed` / `ci.run_passed`
  with `provider: buildkite`, key `<attempt>:bk_failed:<build numbers>` / `bk_passed`, the same `notice` shape. The PR
  badge, notices and workflows then work as with GitHub Actions. An attempt without a PR still gets CI events
  (Buildkite builds branches).
- **Actions**: `buildkite.retry` (`build` url or number: retries the failed jobs, or rebuilds) and `buildkite.trigger`
  (`pipeline`, `branch`, `commit`, `message`, `env`) → `{number, url}`.

## 7. Media, MCP tools and the brief archive (7e-3, PDT-86)

### 7.1 Radarr, Sonarr, Plex

Connection kinds `radarr`, `sonarr` (url, secret the API key, sent as `X-Api-Key`; `config.rootFolder` and
`config.qualityProfile` as defaults for adds) and `plex` (url, secret the `X-Plex-Token`).

| Kind | Does |
|---|---|
| `radarr.search` | look up films by term → `[{tmdbId, title, year, inLibrary}]` |
| `radarr.add` | add a film by `tmdbId` (profile and folder from the connection unless given), search on add; `exists` port if present |
| `radarr.queue` | what's downloading → `[{title, progress, eta, status}]` |
| `sonarr.search`, `sonarr.add`, `sonarr.queue` | the same for series (`tvdbId`, monitor option) |
| `plex.search` | search the library → `[{title, type, year, ratingKey}]` |
| `plex.recently_added` | the newest items of a library section |
| `plex.refresh` | refresh a library section |

Exposing them to the PM as tools is 7f; they are ordinary action kinds here.

### 7.2 MCP tools as actions

- An MCP server is a connection of kind `mcp`: `config.transport` `stdio` (`command`, `args`, `env`; the secret, if
  any, passed as the env var `config.secretEnv`) or `http` (`url`; the secret as a bearer token). PDT is the MCP client
  (the `rmcp` crate).
- PDT lists each server's tools (`tools/list`, on save and every 10 min, cached in memory). One kind, `mcp.tool`
  (`server`, `tool`, `arguments`), calls a tool; the catalog response also lists every cached tool as a virtual entry
  `mcp:<server>/<tool>` (the tool's description and input schema, `mcp.tool` preset), so the builder and agents see
  each tool as an action. Validation checks `arguments` against the tool's schema when it is cached.
- Output: the tool's structured content if any, else its text content; `isError` is an error.
- A stdio server is started on first use and kept for 5 min idle, one process per server, killed with pdtd.

### 7.3 Brief archive

`brief.archive` (`brief` name, `title`, `body`, `items`, `date`) appends an entry to
`<data dir>/briefs/<brief>/<date>.md` plus an index (`index.jsonl`: date, title, item titles and links);
`brief.read` (`brief`, `since` or `last`) returns recent entries, so 7f's brief agents can avoid repeating themselves.
Files, not tables: they are readable and greppable outside PDT.

## 8. API summary (new)

- `POST /api/hooks/{name}` (webhook; no identity guard, secret required)
- 7e-2: `POST /api/projects/{id}/jira/import`; project settings fields for Jira and Buildkite
- Catalog: the new kinds; 7e-3's virtual MCP entries

## 9. Code layout (7e-1)

- pdt-core `workflow/events.rs`: `schedule.fired`, `github.issue.opened`, `pr.commented`, `pr.reviewed`,
  `webhook.received`, `pdt.notified` and their payloads; `workflow/schedule.rs`: interval and cron slots (pure, TDD).
- pdt-github: `recent_issues`, `pr_comments`, `pr_reviews` (+ `FakeGithub`).
- pdt-api `workflow/nodes/`: `schedule.rs`, `webhook.rs` (trigger), `command.rs`, `http.rs`, `discord.rs`,
  `notify.rs`; `workflow/scheduler.rs`; `github_issues.rs` (sensor); comments in `github.rs`; `hooks.rs` (the
  endpoint); the §5 templates in `workflow/templates.rs`.
- web: the notifications follower (toasts) and its Settings switch.
- Docs: docs/reference/workflows.md (new kinds, sensors, webhooks, templates), host.md (the `/api/hooks` exception to
  the identity guard), agent-guide.md if it lists the catalog.
