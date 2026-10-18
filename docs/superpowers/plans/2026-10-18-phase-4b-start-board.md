# Phase 4b — Start, attempts and the board: plan

Spec: docs/superpowers/specs/2026-10-18-tickets-design.md §4 (start, abandon), §5, §6, §8 (board, start dialog,
sidebar keys), §9, §10 (4b). Branch `phase-4b-start-board`.

**Goal:** a ticket can be started: PDT makes the branch and worktree, an attempt, and an agent session whose first
prompt is the ticket, and opens the ticket and the session as tabs. PDT notices when the branch is merged and offers
"Move to Done and remove the worktree". The Board shows To do / In progress / Done. Sessions that belong to an attempt
show the ticket key. A script imports docs/ROADMAP.md as tickets (run against the stable instance only with the user's
go-ahead).

**Not in 4b (4c):** areas config, planned/actual overlap, start checks for areas, "Wait until clear". Start checks in
4b are blockers only (`mode: check | force`).

**Decisions made while planning** (the spec is updated to match):
- **Squash merges** are detected by patch identity, not by `git diff base...branch` (that diff still shows the branch's
  changes after a squash): `git commit-tree <head>^{tree} -p <merge-base>` makes one commit with the branch's whole
  change, and `git cherry <base> <that commit>` reports `-` when an equivalent patch is already in the base.
- **Remote merges:** a PR merged on GitHub only reaches the local base after a fetch, so the 60 s check fetches each
  project's base branch (`git fetch origin <base>`, `GIT_TERMINAL_PROMPT=0`, 20 s timeout, failures logged) and tests
  against both `<base>` and `origin/<base>`.
- **Deleted branches:** `attempts.last_head` (migration 0011) remembers the branch head at each check, so a merged
  branch deleted afterwards is still recognised.
- **Windows:** npm-installed agents are `.cmd` shims run through `cmd.exe /c`, which would interpret `&`, `|`, `%`
  and newlines in a ticket. There the prompt is written to `<data dir>/sessions/<id>/prompt.md` and the agent gets a
  short prompt naming the ticket and that file (refused to a key-only prompt if the path has cmd metacharacters).
- **The first prompt is not stored:** it is given on the session's creation only; restarts resume the conversation.
- **Merge check on demand** is `POST /api/tickets/{id}/check-merge` (the ticket tab and peek call it when they open),
  not a side effect of GET.
- **To do stage** (`todo`, between Ready and In progress, added after review): the board shows To do, not Ready, so a
  backlog of Ready tickets does not crowd it. No migration (stage is free text).
- **Session titles** for started sessions: `<Agent> · <ticket title>` (the key shows as a chip beside it); sessions carry `ticket {id, key}` for the sidebar.

## API contract (both helpers code against this)

Types (pdt-core, ts-rs, camelCase):
- `Session` gains `attemptId: string | null` and `ticket: SessionTicket | null` (`{ id, key }`, from the attempt).
- `NewSession` gains `initialPrompt?: string` (≤ 16 KB; agents only; added after the agent's flags; fresh launch only).
- `StartTicket { agent: AgentKind, branch?: string, base?: string, bypassPermissions?: boolean,
  tab?: TabPlacement, newWorkspace?: boolean, mode?: StartMode, cols?: number, rows?: number }`;
  `StartMode = "check" | "force"` (default check).
- `StartResult { ticket: Ticket, attempt: AttemptSummary, session: Session }`.
- `Ticket` gains `defaultBranch: string` (`pdt/<KEY>-<n>-<slug>`) and `sessions: AttemptSession[]`
  (`{ id, title, status, agent, attemptId }`: the sessions of all its attempts, newest first).
- `AttemptSummary` gains `lastHead: string | null`.
- `TicketQuery` gains `closedAfter?: number` (ms; tickets closed at or after it, others closed are excluded).

Routes:
- `POST /api/tickets/{id}/start` (`StartTicket`) → `201 StartResult`. Errors: `409 start_blocked` (message names the
  open blockers by key; only with `mode: check`), `409 ticket_active` (it has an active attempt), `409 ticket_closed`
  (Done / Won't do), `400 invalid_input` (bad branch), `409 worktree_path_taken`. Effects, in order: checks; branch
  (default `defaultBranch`); worktree (existing code; adopts `.worktrees/<branch>`); attempt (state active,
  `base_branch` = `base` or the project's base, `base_commit` = the worktree's); stage → In progress (event `moved`)
  and event `started`; session (`initialPrompt` = the ticket prompt, title `<Agent> · <KEY>-<n>`, `attempt_id` set;
  event `session`); tabs: with `newWorkspace`, a free-form workspace named `<KEY>-<n>` with the ticket tab then the
  session tab; else the ticket tab at `tab` (or the end of the project workspace) and the session tab right after it.
  A failure after the worktree exists rolls back the attempt and stage (the worktree is kept and adopted next time).
- `POST /api/attempts/{id}/abandon` → `200 Ticket`; state abandoned, `ended_at`, event `abandoned`. Worktree kept.
- `POST /api/tickets/{id}/check-merge` → `200 Ticket` (runs merge detection for its active attempt now).
- `PATCH /api/tickets/{id}` with stage Done / Won't do ends the active attempt: `merged` if detection says so, else
  `abandoned` (event either way).
- `GET /api/tickets?closedAfter=` (the board's Done column).

Ticket prompt (pdt-core `ticket_prompt`): `"<KEY>-<n>: <title>\n\n<description>\n\n(Ticket <KEY>-<n> in PDT. Work in
this worktree on branch <branch>.)"`; over 8 KB the description is cut at a char boundary and ends with
`"\n\n[Description cut: read the full ticket in PDT.]"`; no description → no empty paragraph.

## Backend (one helper)

### Task 1 — pdt-core (TDD)
`attempt_branch(key, number, title)` (slug: ascii lower-case, `[a-z0-9-]`, runs of others → `-`, trimmed, ≤ 40 chars,
cut at a `-` when possible; empty slug → `pdt/<KEY>-<n>`), `ticket_prompt`, `StartTicket` validation (branch through
the existing branch-name rules, sizes 20..=500 × 5..=300, defaults 120×30), `NewSession.initial_prompt` validation
(agents only, ≤ 16 KB, not blank), `launch()` (or the API's arg building) appends the prompt last on a fresh launch,
`StartMode`, `SessionTicket`, `AttemptSession`, the new fields.

### Task 2 — pdt-git (tests with temp repos)
`rev_parse(repo, rev) -> Option<String>`, `is_ancestor(repo, a, b) -> bool`, `merge_base(repo, a, b)`,
`patch_in(repo, head, base) -> bool` (commit-tree + cherry, no refs or working tree touched),
`fetch(repo, remote, branch, timeout)` (`GIT_TERMINAL_PROMPT=0`), `is_merged(repo, head, base_commit, bases: &[&str])`
(false when head == base_commit; true if head is an ancestor of any base, or its patch is in any base). Tests: regular
merge, fast-forward, squash merge, squash after main moved on, unmerged branch, a branch with no commits, deleted
branch via last head.

### Task 3 — migration 0011 and store
`0011_attempt_head.sql`: `ALTER TABLE attempts ADD COLUMN last_head TEXT;`. Store: `start_attempt` (one transaction:
attempt row, stage in_progress + `moved` event when it changes, `started` event; `Conflict` on a second active
attempt), `end_attempt(id, state, now)` + event, `set_attempt_head`, `list_active_attempts()` (with project repo path,
base branch, worktree path), `get_attempt`, `undo_start` (for the rollback), sessions: `attempt_id` column written by
`insert_session` and read (with `ticket {id, key}` through attempts → tickets → projects) by every session query;
`Ticket.sessions`, `defaultBranch` (computed in the API or store), `closed_after` filter.

### Task 4 — API and the merge loop
Routes above; `sessions::create` gains the initial prompt (Windows shim rule above; `pdt_agents::launch` gets a
`resolve` variant that says whether it went through cmd); a shared `create_session(st, …) -> Session` used by both
`POST /api/sessions` and Start. `pdt_api::check_merges(&st)`: for each active attempt, fetch (once per project per
tick), detect, update `last_head`, end merged attempts with an event. pdtd calls it in the existing 60 s loop. Tests in
crates/pdt-api/tests/start.rs: start happy path (fake agent program via `PDT_CLAUDE_PROGRAM`-style test hooks the
existing session tests use), blocked/force, active, closed, tabs (both placements), abandon, check-merge after a real
merge in a temp repo, PATCH to Done ending the attempt, sessions listing `ticket`. `cargo test` regenerates
web/src/api/gen; commit it.

## Web (one helper, against the contract; reconciled with the generated types after the backend lands)

### Task 5 — Start
- `api.startTicket(id, body)`, `api.abandonAttempt(id)`, `api.checkMerge(id)`.
- `StartDialog` (web/src/tickets): agent (Claude / Codex, remembered per browser), branch (placeholder
  `defaultBranch`), base (placeholder the project's base), bypass permissions (same wording as the new-session dialog),
  "Open in a new workspace"; open blockers listed with a warning and the primary becomes "Start anyway" (`mode:
  force`); errors in the dialog. On success the session's tab becomes current.
- Entry points: the ticket header's primary action **Start** (Draft/Ready/To do with no active attempt), the peek footer, the
  row and card menus ("Start…"), the palette ("Start <KEY>-<n>" for the open ticket tab; "Start ticket…" picks one).
- Ticket side column **Attempt** section: branch, state badge, its sessions (open; status dot), Abandon (confirm).
- **Merged banner** (ticket tab and peek, when the latest attempt is merged and the ticket is open): "Merged into
  <base>. Move to Done and remove the worktree" → PATCH Done, then DELETE the worktree; `worktree_busy` asks to stop
  the attempt's sessions first (then retries); `worktree_dirty` asks to discard. The ticket tab and peek call
  `checkMerge` once when they open on a ticket with an active attempt.

### Task 6 — Board and keys
- Board segment: three columns To do / In progress / Done (Done: `closedAfter` = 14 days ago, newest first), each its
  own paged query (polling like the backlog), cards with key, title, priority, labels, the attempt branch and its
  sessions' activity dot; click opens the peek; drag a card to another column (desktop, a handle; `text/plain
  pdt:ticket:<id>`) moves its stage; "Move to ›" in the card menu (right-click / long press). Phones: one column at a
  time with a stage picker. Ctrl+] b opens the board.
- Sidebar agent rows and session tab titles show a `TicketKey` chip when `session.ticket` is set (click opens the
  ticket).
- Gallery: board card, start dialog states. Tests for each.

## Task 7 — roadmap import, docs, checks
- `scripts/import-roadmap.mjs` (spec §9): parses docs/ROADMAP.md "Next" and "Follow-ups"; dry run by default (prints
  the tickets); `--apply --base <url> --project <id>` creates them and rewrites the file to a pointer. Not run against
  the stable instance in this PR (needs the user's go-ahead, after the release with 4b is installed).
- AGENTS.md (Start, attempts, merge loop, board keys), spec (decisions above), ROADMAP (4a done, 4b in review).
- fmt, clippy, cargo test (commit gen), web typecheck/test/build; demo screenshots of Start dialog, ticket with an
  attempt, merged banner, board (desktop + phone); PR labelled `windows` (process launch change).
