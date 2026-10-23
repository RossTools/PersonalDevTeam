# Phase 7e-2 plan: Jira Cloud and Buildkite (PDT-85)

Spec: docs/superpowers/specs/2026-10-20-phase-7e-integrations-design.md §6 (and §1's shared rules). One PR, one
migration, built in this order (each step test-first, `cargo test` green before the next). The details the spec left to
this plan are settled in "Decisions".

## Decisions

- **Connections.** `jira`: `url` the site (`https://<site>.atlassian.net`), `config.email`, secret an API token, sent
  as basic auth (email, token). `buildkite`: `url` the API (default `https://api.buildkite.com`; tests point it at a
  fake), `config.org`, secret an API token (bearer). Both reuse 7e-1's connection store and keychain.
- **Project link** (host-only, `project_settings`): `jira_connection` (a connection name, as nodes name them),
  `jira_scope` (JQL, e.g. `project = ABC`), `jira_done` (the done transition or status, default `Done`);
  `buildkite_connection`, `buildkite_pipelines` (JSON list of slugs). The API's settings view and input gain
  `jira` / `buildkite` objects; a `connection` of `""` unlinks.
- **Links**: `ticket_links_external (ticket_id, system, connection_id, key, url, created_at)`, unique
  `(system, connection_id, key)`, cascade on ticket delete. `TicketSummary.external: [{system, key, url}]`, filled in
  where labels and areas are, so lists, the Board and the ticket page all have it. `ticket.stage_changed` gains
  `jiraKey` (the ticket's Jira key, if linked) so the outgoing built-in can filter on its trigger.
- **Jira search** uses Jira Cloud's current endpoint, `POST /rest/api/3/search/jql` (`jql`, `fields`,
  `expand: "changelog"`, `nextPageToken`), up to 5 pages of 100 per poll. The window is relative
  (`updated >= "-<n>m"`, n = minutes since the last poll + 1), because JQL dates are in the Jira user's zone.
  `assignedToMe` compares the assignee's `accountId` with `GET /rest/api/3/myself` (cached per connection).
- **Jira events.** Baseline `sensor.jira.<project id>` = `{since}` (ms); the first poll records it and emits nothing.
  Then per issue: `created` when its `created` ≥ since; `updated` when changelog items after since name fields
  (`changed`), `transitioned` per changelog history after since with a `status` item; `commented` per comment created
  after since (with `byMe`). Event keys from the spec make re-reads harmless. Payloads carry `projectId`, so the plain
  event triggers (shared `projects` / `where`) work, scoped as usual.
- **Jira actions.** `jira.import` (`key`, `project`, `connection` default the project's) → `imported` / `exists`;
  `jira.create`; `jira.transition` takes `connection` + `key`, or `ticket` (its link; `to` then defaults to the
  project's done mapping) → `out` / `unavailable` (no such transition from the issue's status); `jira.comment`. Import
  makes the ticket and its link in one transaction, so a race can't import twice.
- **Built-ins** (both on): *Import assigned Jira issues* (`jira.issue.created` + `jira.issue.updated` where
  `assignedToMe` and no `ticketId` → `jira.import`), *Done moves the Jira issue* (`ticket.stage_changed` where
  `to = done` and `jiraKey` exists → `jira.transition` with `ticket`).
- **Import by key**: `POST /api/projects/{id}/jira/import {key}` → `{ticket, existed}`; Tickets → "Import from Jira…"
  when the project links Jira.
- **Buildkite sensor** (`refresh_buildkite`, in the 60 s loop after the PR check): for each active attempt of a
  project with Buildkite, the head is the PR's head, else `origin/<branch>`, else the local branch. Builds of each
  pipeline for that branch and commit become `CiRun`s (`workflow` the pipeline, `id` the build number, `attempt`
  1 + the jobs' retries so a retried build is news again, failed jobs named). Kept in memory per attempt: merged into
  `attempt.pr.ci` when there is a PR (the badge shows both providers), else `attempt.ci`. Events `ci.run_failed` /
  `ci.run_passed` with `provider: buildkite`, keys `<attempt>:bk_failed:<ids>` / `bk_passed:<ids>`, a CI-failed notice
  naming the build, so the built-in PR notices tell the agent.
- **Buildkite actions.** `buildkite.retry` (`connection`, `build`: a build URL, or a number with `pipeline`): retries
  the failed jobs, else rebuilds → `{number, url}`. `buildkite.trigger` (`connection`, `pipeline`, `branch`,
  `commit` default `HEAD`, `message`, `env`) → `{number, url}`.

## Steps

1. **Core.** pdt-core `jira.rs` (pure): ADF → text and text → ADF; an issue from REST v3 JSON; the events of an issue
   since a time (created / updated / transitioned / commented). `buildkite.rs`: build JSON → `CiRun`s; the CI notice.
   `CiStatus::merge`. Event types and payloads in `workflow/events.rs`; `ExternalLink` in `ticket.rs`.
2. **Store.** Migration `0027_external_links.sql`; `ProjectSettings` Jira and Buildkite fields; links (create a
   linked ticket in one transaction, find by key, list per ticket), `external` on summaries, `jiraKey` on
   `ticket.stage_changed`.
3. **Jira in pdt-api.** `jira.rs`: the client, the sensor, the import command and route; `nodes/jira.rs` with the four
   actions; the two built-ins; project settings fields. Tests (`tests/jira.rs`) against a fake Jira server: baseline,
   each event, import (assigned, by key, twice), the built-ins end to end, transition and its `unavailable` port,
   comment ADF, create, dry runs.
4. **Buildkite in pdt-api.** `buildkite.rs`: client and sensor; `nodes/buildkite.rs` with retry and trigger; tests
   (`tests/buildkite.rs`) with a fake Buildkite: events once per build state, a retry is news, a PR's badge merges
   both providers, an attempt without a PR, the actions.
5. **Web.** Connection kinds `jira` and `buildkite` (their config fields in the dialog); project settings sections
   for Jira and Buildkite; "Import from Jira…" on the tickets view; the Jira key on Board cards and the ticket page;
   CI without a PR on the attempt. TS types regenerated.
6. **Docs.** docs/reference/workflows.md (kinds, sensors, built-ins), tickets.md (Jira links, import, Buildkite CI).
7. **Checks**: fmt, clippy, cargo test, web typecheck/test/build.
