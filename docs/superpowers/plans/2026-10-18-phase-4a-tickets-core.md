# Phase 4a — Tickets core and the backlog: plan

Spec: docs/superpowers/specs/2026-10-18-tickets-design.md §3, §4, §8 (backlog, peek, ticket tab), §10 (4a), §11.
UX: docs/superpowers/specs/2026-10-17-design-system-ux-design.md §8.0–§8.1 and the "PDT UX pass" mockups (Backlog,
Ticket). Branch `phase-4-tickets`.

**Goal:** tickets exist: write them (quick add, full form), rank them, search and filter a backlog that stays fast at
1,000+ tickets, select and act on many at once, link them (blocks / relates), comment, and open one as a tab or in a
side sheet. No Start, attempts, board or overlap yet (4b, 4c) — the migration still creates `attempts` and the
`attempt_id`/`ticket_id` columns so 4b and 4c add no schema churn for the core tables.

**Decisions made while planning** (the spec is updated to match):
- **Numbers never repeat:** `projects.next_ticket_number INTEGER NOT NULL DEFAULT 1`, bumped in the insert's
  transaction.
- **Areas** in 4a are only stored on tickets (free names); 4c adds the config and overlap.
- **Markdown:** `react-markdown` + `remark-gfm` + `rehype-sanitize` (add with `pnpm add`).
- **Saved filters:** per browser (localStorage) in Phase 4.
- **Board** placeholder: the Tickets view's Board segment shows "Arrives with Start (4b)" — the backlog is the 4a view.

## Backend (one helper, in order)

### Task 1 — pdt-core (TDD)
- `ticket` module: `Ticket`, `TicketSummary` (list row), `TicketStage` (draft|ready|in_progress|done|wont_do; `is_closed`),
  `Priority` (none|low|medium|high|urgent), `NewTicket`, `TicketPatch`, `TicketLink {to, kind}`, `LinkKind`
  (blocks|relates), `TicketEvent`, `EventKind`, `TicketQuery` (filters, sort, cursor, limit), `TicketPage`,
  `BulkAction`; all ts-rs exported, camelCase. Key = `<project key>-<number>`.
- `rank` module: base-62 fractional index — `between(a: Option<&str>, b: Option<&str>) -> String` (strictly between;
  panics/err when a >= b), `needs_rebalance(key)` (> 40 chars), `spread(n)` for a rebalance. Property-style tests:
  random insert sequences stay sorted; repeated splitting of one gap.
- `links` module: `would_cycle(edges, from, to)` for `blocks`; `blocked(ticket, edges, stage_of)`.
- `ticket_text`: label normalisation (trim, lower-case, 1–30 chars), title rules (1–200), slug (for 4b, now: ascii,
  ≤ 40), key parsing (`PDT-12` / `pdt-12` / `12`).

### Task 2 — migration and store
- `0010_tickets.sql` exactly as spec §3 plus `projects.next_ticket_number`, the FTS5 table and its triggers, and the
  `workspace_tabs` rebuild with `ticket_id` and the exactly-one CHECK (copy rows; keep indexes and uniques; foreign
  keys off/on around the rebuild as SQLite requires).
- `pdt-store/src/tickets.rs`: create (number + rank at top of its stage + `created` event, one transaction), get,
  query (filters: project, stage set, priority set, labels (all of), q via FTS5 `MATCH` with prefix terms, plus exact
  key/number; sort rank|priority|updated|created; keyset cursor; total count), patch (events for stage changes and
  edits; `closed_at`), delete (refuse with an active attempt), rank move (`before`/`after`, rebalance when needed),
  bulk (one transaction, all-or-nothing), labels list, links add/remove (cycle check), events list (paged), comment add.
- Workspace tabs: `open_tab` accepts a ticket; listing returns `ticketId`; deleting a ticket removes its tab.
- Tests with temp databases: everything in spec §11 "pdt-store" that applies to 4a; a 1,000-ticket query test that
  checks paging and sorting stay correct.

### Task 3 — API
- Routes from spec §4 except `start` and `abandon` (4b) and `areas` (4c): tickets list/create/get/patch/delete,
  rank, bulk, links, events, comments, labels; `POST /api/workspaces/{id}/tabs` accepts `{ticketId}`.
- Errors: `400 invalid_input`, `404 not_found`, `409 link_cycle` (message names the cycle by keys), `409
  ticket_active`.
- Tests in crates/pdt-api/tests/tickets.rs (no port 7420, temp data dirs).
- `cargo test` regenerates web/src/api/gen; commit it.

## Web components (one helper, in parallel with the backend; no API calls)
- `web/src/common/Markdown.tsx` (react-markdown + remark-gfm + rehype-sanitize; links open in a new tab; code blocks
  styled with the CodeViewer palette tokens; no raw HTML).
- `Timeline` (web/ui: items with actor, time, icon, body; relative times), `StageStepper` (web/src/common: the four
  stages, current one marked, closed states), `TicketKey` (web/src/common: mono chip, optional link/onClick),
  `PriorityIcon` (web/src/common: icon + label for screen readers), `FilterBar` (web/ui: search input with `/` focus
  hint + chip slots + trailing slot), `BulkActionBar` (web/ui: fixed height, "N selected", actions slot, Clear),
  `DataList` (web/ui: virtualised list built on React Aria GridList/`Virtualizer` with optional checkboxes
  (multi-select, Shift ranges), rank drag with a handle (text/plain `pdt:ticket:<id>`), Alt+↑/↓ move callbacks,
  optional group headers with counts, an `onEndReached` for paging, column visibility; must stay smooth with 1,000
  rows in the gallery).
- Gallery sections and tests for each; lint rules apply.

## Web screens (after both helpers above)

### Task 4 — Tickets view: backlog
- Rail: **Tickets** between Sessions and Files (phones: replaces nothing — the phone rail becomes Sessions, Tickets,
  Search, Settings; Files moves into the palette's Go to). Keys: Ctrl+] l → Tickets (backlog).
- List column: projects with keys and open-ticket counts, "All projects"; saved filters (localStorage) with "Save
  filter…".
- Work area: header (title, Backlog | Board segmented — Board shows the 4b placeholder, search with `/`, New ticket),
  FilterBar (stage, priority, label chips; group, sort, Show closed), DataList of tickets (rank handle, checkbox, key,
  title, stage badge, priority, labels, updated), quick-add row at the top (Enter adds to Draft at the top; Shift+Enter
  opens the full form), BulkActionBar when rows are selected (Move to Ready, Priority ▾, Label ▾, Move to top,
  Delete…), infinite paging via the cursor, empty states.
- Clicking a row opens the **peek Sheet** (right): key, title (inline edit), stage, priority, labels, description
  (Markdown, Edit), links, last few timeline entries, actions Open (ticket tab), Move to ›, Delete….
- Polling: the visible page refetches every 2 s (TanStack Query), keeping scroll and selection.

### Task 5 — ticket tab and palette
- Tabs can hold a ticket: `WorkspaceBar`/`AppShell` render `TicketView` for a ticket tab (title `PDT-42 · title`).
- `TicketView` Overview: ViewHeader (TicketKey, inline-edit title, StageStepper, priority, labels, ⋯: Move to stage,
  Delete…), body: description (Markdown with Edit/Preview), Timeline with comment Composer; side column: Details
  (stage, priority, labels), Links (blocked by / blocks / related, "Link ticket…" via a TicketPicker CommandList,
  remove), Areas (free chips for now). Conversation and Changes tabs disabled ("Phase 5" / "Phase 6").
- Palette: Go to tickets by key/title (searching the API), "New ticket", "Open ticket…". New-tab chooser: "Ticket…"
  opens a ticket here.

### Task 6 — docs and checks
- AGENTS.md (tickets, keys, API, Tickets view keys), spec updates (planning decisions above), ROADMAP (4a in review).
- fmt, clippy, cargo test (commit gen), web typecheck/test/build; demo screenshots (desktop + phone, light + dark) of
  backlog with 1,000 seeded tickets (a dev-only seed flag or a script), peek, ticket tab; Android check (long-press
  menus, sheet, quick add with the keyboard). PR.
