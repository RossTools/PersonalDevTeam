# Phase 4c — Areas and overlap: plan

Spec: docs/superpowers/specs/2026-10-18-tickets-design.md §3 (areas), §4, §5 (checks, wait), §7, §8, §10 (4c); UX spec
§8.1 "Avoiding conflicting work". Ticket PDT-1, branch `pdt/PDT-1-phase-4c-areas-and-overlap`.

**Goal:** a project declares named areas (paths, some exclusive) in `.pdt/project.toml`; tickets list the areas they
expect to touch; PDT shows planned overlap (shared areas) and actual overlap (files changed by two active attempts) on
rows, cards, the ticket header and side column; Start warns about them and offers Start anyway or Wait; waiting tickets
say when they are clear to start; the backlog filters by overlap and ships a "Ready to start" saved filter.

**Decisions made while planning** (the spec is updated to match):
- **Whose overlap:** a ticket's overlap is always measured against the *other tickets with an active attempt* in its
  project. An active ticket overlaps another active one by shared areas (planned) or shared changed files (actual); an
  open ticket without an active attempt (Draft, Ready, To do, waiting) overlaps an active one by shared areas: what Start
  would warn about. Waiting tickets never hold areas against each other (two waiting tickets would otherwise wait for
  each other forever), and an active ticket does not show overlap with a ticket that is only waiting on it.
- **Levels:** a shared exclusive area, or a shared changed file matching an exclusive area's paths, is a `conflict`;
  any other shared area or file is an `overlap`. Areas not in the current configuration are ignored (and shown struck
  through). Closed tickets have no overlap.
- **Area config** comes from the project's main checkout (`<repo>/.pdt/project.toml`) merged with the host overrides,
  like runs; areas merge by name (override fields replace). Names follow the ticket area rules (1–60 characters, no
  commas); `paths` are globset globs relative to the checkout (`*` stays within a folder, `**` crosses folders), at
  least one; `exclusive` defaults to false. An invalid configuration means no areas for overlap (the areas route and
  run config report the error).
- **Wait** is `PATCH /api/tickets/{id} {waiting: true}` (and `false` to stop waiting), not a Start mode: Start either
  starts or refuses, so its result always has a session. Waiting moves the ticket to the top of its stage. Starting or
  closing a ticket clears `waiting`. A `waiting` event records each change.
- **Clear to start:** a waiting ticket with no open blockers and no overlap has `clearToStart: true`; the client toasts
  "<KEY> is clear to start" once per ticket per browser session.
- **Changed files** are recomputed for every active attempt every 30 s (`git diff --name-only --no-renames
  <merge-base>` against the working tree, plus untracked files), with no HEAD/mtime cache: it is a few git calls per
  active attempt, and a cache keyed on HEAD and the index would miss unstaged edits. Kept in memory (pdtd restart
  recomputes on the first tick).
- **Overlap events:** when two active attempts first share a set of changed files, both tickets get an `overlap`
  timeline event `{ticketId, key, files, level}`; the same pair with the same files is not recorded again (checked
  against the ticket's latest overlap event for that other ticket, so restarts don't repeat it).
- **Filter:** `conflicts=true` lists tickets with any overlap or conflict, `false` those with none. The saved filter
  "Ready to start" is stage Ready, unblocked, `conflicts=false` (a built-in, not removable).

## API contract

Types (ts-rs, camelCase):
- `AreaDef { name, paths: string[], exclusive: boolean }`; `RunConfig` gains `areas: AreaDef[]`.
- `ProjectAreas { areas: AreaDef[], error: string | null }`.
- `TicketSummary` gains `overlapWith: string[]` (the other tickets' keys, conflicts first, then by key) and
  `clearToStart: boolean`; `overlap` is now computed (`none | overlap | conflict`).
- `Ticket` gains `overlaps: TicketOverlap[]`; `TicketOverlap { ticketId, key, title, stage, level: OverlapLevel,
  areas: string[] (shared configured areas), files: string[] (shared changed files, sorted, at most 50),
  filesTotal: number }`, conflicts first.
- `TicketPatch` gains `waiting?: boolean`. `TicketQuery` gains `conflicts?: boolean`.
- `EventKind` gains `waiting` (data `{waiting: bool}`). `overlap` events carry `{ticketId, key, files, level}`.

Routes:
- `GET /api/projects/{id}/areas` → `ProjectAreas`.
- `GET /api/tickets?conflicts=` (as above); every ticket response carries the computed overlap fields.
- `POST /api/tickets/{id}/start` with `mode: check` now also refuses (`409 start_blocked`) when the ticket overlaps an
  active ticket; the message names blockers and overlaps. `force` records `{forced, blockers, overlaps}` on `started`.
- `PATCH /api/tickets/{id} {waiting}`.

## Backend

1. **pdt-core (TDD):** `runconfig` `[[area]]` parsing/merge/validation (`AreaDef`, errors `BadAreaName`,
   `NoAreaPaths`, `BadGlob`, `DuplicateName`); `overlap.rs`: `AreaSet` (compiled globs; `exclusive_match(file)`),
   `compute(areas, tickets) -> HashMap<ticket id, Vec<OverlapWith>>` over `Participant {id, active, areas, files}`;
   `level_of`; the new fields and kinds.
2. **pdt-git:** `changed_files(worktree, base) -> Vec<String>` (merge-base with `base`, falling back to the base
   commit; diff vs the working tree; untracked). Temp-repo tests: committed, staged, unstaged, untracked, deleted,
   renamed (both names), ignored files left out.
3. **pdt-store:** `waiting` in patch (event, top of stage), cleared by `start_attempt` and by closing; `open_area_tickets
   (project)` (open tickets with their areas and whether active); filter `ids_in` / `ids_not_in` for `conflicts`;
   `latest_overlap_event(ticket, other)`.
4. **pdt-api:** `overlap.rs`: the changed-files cache in `AppState`, `refresh_overlaps(&st)` (pdtd runs it every 30 s),
   `index(&st, project?)` → per-ticket overlaps, applied to every ticket response and the list; the areas route; start
   checks; events.
5. **pdtd:** 30 s loop; demo seed gets areas in the Sample App config and on a few tickets.

## Web (one helper, against the generated types)

- `api.getProjectAreas`, `patchTicket` with `waiting`.
- Areas editor: suggestions from the configured areas; areas not in the config struck through with a tooltip; areas in
  the new-ticket form if it has labels.
- `OverlapBadge` (web/ui): amber "Overlaps PDT-51" / red "Conflicts with PDT-51" (+N more), on backlog rows, board
  cards and the ticket header; `Waiting` (clock) and `Clear to start` badges.
- Ticket side column "Overlaps": each other ticket (key opens it, title, stage, level, shared areas, files list).
- Start dialog: an Alert per overlap (shared areas/files); with findings the actions are **Wait** (primary, PATCH
  waiting) and **Start anyway** (secondary; `danger` for conflicts); a waiting ticket offers "Stop waiting".
- Filters: an "Overlap" filter (any / none) and the built-in saved filter "Ready to start". Toast once when a ticket
  in view becomes clear to start.
- Gallery entries and tests for each.

## Checks

fmt, clippy, cargo test (commit web/src/api/gen), web typecheck/test/build; screenshots of rows/cards with badges, the
ticket's Overlaps section and the Start dialog with a conflict. AGENTS.md (areas, overlap), spec decisions, PDT's own
`.pdt/project.toml` areas.
