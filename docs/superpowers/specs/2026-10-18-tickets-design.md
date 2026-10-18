# Personal Dev Team — Tickets, Backlog and Board (Phase 4) Design

Status: draft, 2026-10-18, awaiting the user's review. UX: docs/superpowers/specs/2026-10-17-design-system-ux-design.md
§8.0–§8.1 (backlog, board, ticket tab, Start, links and blockers, avoiding conflicting work) and the "PDT UX pass"
mockups (Backlog, Board, Ticket, Start dialog). This spec covers the model, the API, Start, attempts, links, areas and
overlap detection, and the build order. It amends the main spec §4 (domain model) and §11 (Phase 4).

## 1. Purpose

PDT's own backlog lives in PDT. A project has tickets the user writes, ranks and pulls into work; Start turns a ticket
into a branch, a worktree and an agent session that already knows the ticket; PDT shows which tickets block others and
which active tickets would change the same code, so two agents don't make conflicting changes.

### Success criteria

- The roadmap's items are PDT tickets (imported once), ranked as listed; `docs/ROADMAP.md` points to PDT.
- Writing a ticket and starting an agent on it takes under a minute: quick add, Start, the agent is working.
- With 1,000 tickets the backlog scrolls, searches and filters without a visible wait.
- Starting a ticket whose blocker isn't done, or whose area is held by another active ticket, warns first.
- Two active branches changing the same file are flagged on both tickets within a minute.

## 2. Decisions (made with the user, 2026-10-18)

- **Stages:** Draft, Ready, In progress, Done, plus **Won't do** (closed, hidden like Done). Phase 7 splits In progress
  into the pipeline stages.
- **Start** launches Claude or Codex in the new worktree with the ticket as its **first prompt**.
- **Attempts end** when PDT sees the branch merged into the base branch (git only; no GitHub API) — it marks the
  attempt merged and offers "Move to Done and remove the worktree" — or when the ticket is moved to Done/Won't do or
  the attempt is abandoned.
- **Roadmap:** imported once when Phase 4 ships; `docs/ROADMAP.md` becomes a pointer.
- From the UX spec: backlog and board share the Tickets view (D6); the board shows Ready / In progress / Done (D3);
  a ticket is a tab (D5); links and areas with planned and actual overlap (D7).

## 3. Model

New tables in one migration, `0010_tickets.sql` (existing migrations untouched).

```sql
CREATE TABLE tickets (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    number      INTEGER NOT NULL,             -- PDT-42: the project key + number; numbers are never reused
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',     -- markdown
    stage       TEXT NOT NULL,                -- draft | ready | in_progress | done | wont_do
    priority    TEXT NOT NULL DEFAULT 'none', -- none | low | medium | high | urgent
    rank        TEXT NOT NULL,                -- fractional index: lexicographic order = backlog order
    waiting     INTEGER NOT NULL DEFAULT 0,   -- Start chose "Wait until clear" (reasons are computed, §7)
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    closed_at   INTEGER,                      -- set when moved to done / wont_do
    UNIQUE (project_id, number)
);
CREATE INDEX tickets_by_rank ON tickets (project_id, rank);
CREATE TABLE ticket_labels (ticket_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE, label TEXT NOT NULL,
    PRIMARY KEY (ticket_id, label));
CREATE TABLE ticket_areas  (ticket_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE, area TEXT NOT NULL,
    PRIMARY KEY (ticket_id, area));
CREATE TABLE ticket_links (
    from_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,   -- blocks: from blocks to
    to_id   TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind    TEXT NOT NULL,                                            -- blocks | relates
    created_at INTEGER NOT NULL,
    PRIMARY KEY (from_id, to_id, kind), CHECK (from_id <> to_id));
CREATE TABLE attempts (
    id          TEXT PRIMARY KEY NOT NULL,
    ticket_id   TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE SET NULL,
    branch      TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    state       TEXT NOT NULL,                -- active | merged | abandoned
    created_at  INTEGER NOT NULL,
    ended_at    INTEGER);
CREATE UNIQUE INDEX one_active_attempt ON attempts (ticket_id) WHERE state = 'active';
CREATE TABLE ticket_events (                  -- the ticket's timeline; comments are events
    id         TEXT PRIMARY KEY NOT NULL,
    ticket_id  TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,                 -- created | edited | moved | linked | unlinked | started | session |
                                              -- merged | abandoned | unblocked | overlap | comment
    actor      TEXT NOT NULL,                 -- user | system
    body       TEXT,                          -- comment text, or a short description
    data       TEXT,                          -- JSON details (from/to stage, link, files…)
    created_at INTEGER NOT NULL);
CREATE INDEX ticket_events_by_ticket ON ticket_events (ticket_id, created_at);
CREATE VIRTUAL TABLE tickets_fts USING fts5(title, description, content='tickets', content_rowid='rowid');
-- + the standard fts5 sync triggers on insert/update/delete
ALTER TABLE sessions ADD COLUMN attempt_id TEXT REFERENCES attempts(id) ON DELETE SET NULL;
ALTER TABLE workspace_tabs ADD COLUMN ticket_id TEXT REFERENCES tickets(id) ON DELETE CASCADE;
```

- **Numbers:** `MAX(number)+1` for the project inside the insert's transaction; a deleted ticket's number isn't
  reused (keep `projects.next_ticket_number` if deletes make MAX unsafe — decided while planning).
- **Workspace tabs** gain a ticket kind: exactly one of session, run, ticket (the CHECK is rebuilt with the table in the
  migration, as SQLite requires). One tab per ticket, like sessions and runs.
- **Rank** (pdt-core, TDD): fractional indexing over a base-62 alphabet — `between(a, b)` returns a key strictly
  between two keys (either may be absent); new tickets go to the top of their stage's list; moving a ticket writes one
  row. Keys grow only when the same gap is split repeatedly; a rebalance (rewrite a project's ranks evenly, one
  transaction) runs when a key passes 40 characters.
- **Labels** are free text per ticket, lower-cased and trimmed; the project's label list is the distinct set.
- **Areas** are declared in `.pdt/project.toml` (and host overrides), parsed with the run configuration:

  ```toml
  [[area]]
  name = "migrations"
  paths = ["crates/pdt-store/migrations/**"]
  exclusive = true
  ```

  `ticket_areas` stores names; an area missing from the current config is shown struck through and ignored for
  overlap.

## 4. API

Errors are `{code, message}`; lists page with an opaque cursor. Types are ts-rs exported.

| Route | Purpose |
|---|---|
| `GET /api/tickets?project=&q=&stage=&priority=&label=&blocked=&conflicts=&attempt=&sort=&cursor=&limit=` | the backlog: filtered, searched (FTS5 on title/description, plus `KEY-123` and number matches), sorted by `rank` (default), `priority`, `updated` or `created`; `{items, nextCursor, total}`. Items carry key, title, stage, priority, labels, areas, rank, attempt summary (branch, state, the attempt sessions' rolled-up activity), `blockedBy` count and `overlap` level |
| `POST /api/projects/{id}/tickets` | create `{title, description?, stage? (draft), priority?, labels?, areas?, rank? \| before?/after?}` |
| `GET /api/tickets/{id}` | full ticket: description, links (with the other ticket's key, title, stage), areas, attempt(s), overlaps (§7) |
| `PATCH /api/tickets/{id}` | edit title, description, priority, labels, areas, stage (moving to done/wont_do ends the active attempt, §6) |
| `DELETE /api/tickets/{id}` | delete (refused with `409 ticket_active` while it has an active attempt) |
| `POST /api/tickets/{id}/rank` | `{before?: id, after?: id}` (or `top`/`bottom` within its stage) |
| `POST /api/tickets/bulk` | `{ids, action: stage \| priority \| addLabel \| removeLabel \| top \| delete, value?}`; all-or-nothing |
| `POST /api/tickets/{id}/links` / `DELETE …/links` | `{to, kind}`; a `blocks` link that would close a cycle → `409 link_cycle` naming it |
| `GET /api/tickets/{id}/events` / `POST …/comments` | timeline (paged) / add a comment `{body}` |
| `POST /api/tickets/{id}/start` | §5 |
| `POST /api/attempts/{id}/abandon` | end the active attempt (worktree kept; removing it is the project page's job) |
| `GET /api/projects/{id}/labels` | distinct labels, for pickers |
| `GET /api/projects/{id}/areas` | the configured areas and their paths |

The client polls the list it shows (2 s, like sessions) and invalidates on its own writes; there is no websocket feed
in Phase 4.

## 5. Start

`POST /api/tickets/{id}/start {agent: claude|codex, branch?, base?, bypassPermissions?, tab?, newWorkspace?, mode:
check|force|wait}`

1. **Checks** (pdt-core, pure): blockers not done; planned conflicts (shared exclusive area with an active ticket) and
   overlaps (shared normal area). With `mode: check` (the dialog's first call) any finding returns `409 start_blocked`
   with the reasons and starts nothing; `force` starts anyway (the timeline records it); `wait` sets `waiting` and
   starts nothing.
2. **Attempt:** branch defaults to `pdt/<KEY>-<n>-<slug>` (slug from the title, ascii, ≤ 40); the worktree is made
   with the existing worktree code (adopting `.worktrees/<branch>` if it exists); one active attempt per ticket
   (`409 ticket_active` otherwise); stage moves to In progress.
3. **Session:** an interactive agent session in the worktree with `attempt_id` set and the ticket as its **first
   prompt** — `claude [flags] "<prompt>"` / `codex [flags] "<prompt>"` — only on a fresh launch (restart/resume never
   repeats it). Prompt: `"<KEY>-<n>: <title>\n\n<description>\n\n(Ticket <KEY>-<n> in PDT. Work in this worktree on
   branch <branch>.)"`, capped at 8 KB (longer descriptions are cut with a note to read the ticket in PDT).
   `NewSession` gains `initialPrompt?`, added to the agent's argv after its flags; Windows quoting goes through the
   existing argv path (no shell).
4. **Tabs:** a ticket tab (after the current tab, or in a new free-form workspace named `<KEY>-<n>` when
   `newWorkspace`), then the session's tab after it. Returns `{ticket, attempt, session, tabs}`.

`waiting` tickets: when their last reason clears (blocker done, conflicting attempt ended) the list marks them
"clear to start" and the client toasts once; Phase 7 starts them itself.

## 6. Attempts and merge detection

- An attempt ends when its ticket moves to Done or Won't do (state `merged` if its branch is merged, else
  `abandoned`), when the user abandons it, or when merge detection fires.
- **Merge detection** (pdt-git): every 60 s, and when the ticket or project page is opened, pdtd checks each active
  attempt: merged if `git merge-base --is-ancestor <branch> <base>` (a regular merge) or, for squash merges, the
  branch has no changes left against the base (`git diff --quiet <base>...<branch>` is empty and the branch has
  commits). Also merged when the branch is gone locally but its last known head is an ancestor of the base.
- When merged: attempt → `merged`, timeline event, and the ticket shows a banner "Merged into main — Move to Done and
  remove the worktree" (one click: stage Done + `DELETE /api/worktrees/{id}` with the usual busy/dirty handling). The
  ticket never moves to Done without the user.

## 7. Blockers, areas and overlap

- **Blocked:** a ticket with a `blocks` link from a ticket that isn't Done (Won't do counts as done for blocking).
- **Planned overlap:** two tickets that are active (an active attempt) or waiting and share an area; `conflict` when
  the area is exclusive, else `overlap`.
- **Actual overlap:** pdtd computes each active attempt's changed files — `git diff --name-only <merge-base>` plus
  uncommitted and untracked files in the worktree — every 30 s (cheap: only active attempts, a few at a time; cached by
  HEAD + index mtime). Any file in two attempts' sets is an overlap; a file matching an exclusive area's globs is a
  conflict. Stored in memory; new overlaps write a timeline event once per pair and file set.
- The list and ticket payloads carry `overlap: none|overlap|conflict` and, on the ticket, the other tickets with the
  shared areas or files.
- Pure functions in pdt-core (TDD): blocked status, cycle check, planned overlap, glob matching (`globset`), the
  start checks.

## 8. Web

As designed in the UX spec §8.1 and the mockups; components already exist in `web/ui` (List with virtualisation,
Sheet, CommandList, Menu, Dialog, Field…). New: `Markdown` (react-markdown + remark-gfm, sanitised; code blocks with the
CodeViewer palette), `Timeline`, `StageStepper` (four stages), `TicketKey`, `TicketPicker` (CommandList over tickets),
`FilterBar`, `BulkActionBar`, `AreaChips`, `OverlapBadge`, `DataList` (the backlog's virtualised list with rank drag,
grouping and column options).

- **Tickets view** (rail: Tickets, between Sessions and Files; Ctrl+] l backlog, Ctrl+] b board): Backlog | Board.
- **Backlog:** the paged list (§4), quick add, filters (saved filters in localStorage for Phase 4), bulk actions, rank
  drag (handle; Alt+↑/↓), group by stage/priority/label.
- **Board:** Ready / In progress / Done (last two weeks), drag between columns (desktop) or "Move to ›" (menu).
- **Peek sheet** and **ticket tab** (Overview: header with StageStepper, description editor with preview, timeline with
  comments, side column with links, areas, overlaps, attempt and sessions; Conversation/Changes tabs shown disabled
  with "Phase 5/6").
- **Start dialog** with the checks, Start anyway / Wait until clear.
- **Palette:** tickets by key and title (Go to), "New ticket", "Start <KEY>-<n>".
- Sidebar agent rows and tab titles show the ticket key when a session belongs to an attempt.

## 9. Roadmap import

After 4b ships: a one-off script (`scripts/import-roadmap.mjs`, kept for reference) reads `docs/ROADMAP.md`, creates one
ticket per "Next" and "Follow-ups" item through the API (title = the bold lead, description = the rest; Next items
Ready in order, follow-ups Draft), and replaces the file with a short pointer to PDT. Run once by an agent against the
stable instance's API, with the user's go-ahead.

## 10. Build order

Three PRs:

- **4a — tickets core (backend + backlog):** migration, pdt-core rank/links/blocked (TDD), store, API (tickets, rank,
  bulk, links, comments, events, labels, search), ts types; web: Tickets view with the backlog (list, quick add,
  filters, bulk, rank), peek sheet, ticket tab Overview (markdown, timeline, links), palette entries.
- **4b — Start and the board:** `initialPrompt` on sessions, Start (attempt, worktree, session, tabs), attempts,
  merge detection and the "merged" banner, the board, ticket keys in the sidebar and tabs. Then the roadmap import.
- **4c — areas and overlap:** areas config, ticket areas, planned and actual overlap, start checks with wait, overlap
  badges and filters.

## 11. Testing

- pdt-core (TDD): rank `between` and rebalance; cycle detection; blocked; planned overlap; glob matching; start checks;
  branch slug; prompt building and cap.
- pdt-store: numbering (no reuse), FTS search, paging and sorting, bulk all-or-nothing, cascade deletes, one active
  attempt, tab kind CHECK.
- pdt-git: merge detection for merge commits, squash merges (temp repos), deleted branches; changed files (committed,
  uncommitted, untracked).
- pdt-api: every route; start check/force/wait; link cycles; `ticket_active`; tickets in tabs.
- Web: backlog paging/filter/bulk/rank by keyboard, quick add, peek, ticket editing, Start dialog paths, board moves,
  palette entries; touch (long-press menus, no drag on phones).
- Windows + macOS; agent launches in tests use stand-in programs (`PDT_CLAUDE_PROGRAM`/`PDT_CODEX_PROGRAM`).

## 12. Deferred

- Assignees, due dates, estimates, epics/sub-tasks, sprints, swimlanes (main spec: out of Milestone 1).
- Server-side saved filters and sharing them across devices.
- GitHub PR state on tickets (the GitHub integration follow-up).
- Automatic start of waiting tickets (Phase 7).
- Real-time push (websocket) for ticket lists.
