# Phase 3a — Workspaces: plan

Spec: docs/superpowers/specs/2026-10-17-workspaces-design.md §1–§5 and §7. Branch `phase-3-workspaces`.

**Goal:** workspaces (one per project plus free-form ones) holding tabs, each tab one session or run, stored in pdtd
and shared by every device; a session tree of every project; prefix-key shortcuts; status at a glance; drag to
reorder; phone tab switching.

**Decisions made while planning** (the spec is updated to match):
- The client already polls sessions and runs per project (`['sessions', pid]`, `['runs', pid]`); the tree and tabs
  reuse those queries for every project (`useQueries`), so there are no new list endpoints and existing
  invalidations keep working.
- The shortcut prefix is fixed at Ctrl+] for now; a host setting comes later if it ever clashes.
- No optimistic updates: tab actions call the API and refetch `['workspaces']`.

## Backend

### Task 1 — pdt-core `workspace` module (TDD)
- Types (ts-rs exported, camelCase): `Workspace { id, name, projectId: Option, position, createdAt, tabs: Vec<Tab> }`,
  `Tab { id, workspaceId, sessionId: Option, runId: Option, position }`, `NewWorkspace { name }`,
  `WorkspacePatch { name?, position? }`, `OpenTab { sessionId?, runId?, position? }`, `TabPatch { workspaceId?,
  position? }`, `TabPlacement { workspaceId, position? }`.
- Validation: workspace names trimmed, 1–60 chars; `OpenTab` needs exactly one of session/run.
- Pure `place_at(ids, id, index) -> Vec<String>`: removes `id` if present, inserts at `index` clamped to the end.
  Tests: insert, move forward/back, clamp, absent id.

### Task 2 — migration `0008_workspaces.sql` + store (TDD)
- Tables as in spec §5 (`workspace_tabs.session_id` / `run_id` UNIQUE, CHECK exactly one, cascades).
- Seed in SQL: a project workspace per project (`position` by project `created_at`); tabs for every session and
  every active run, ordered Repository first, then worktrees by `created_at`, then item `created_at`
  (`ROW_NUMBER()`). Ids from `lower(hex(randomblob(16)))`.
- Store (`workspaces.rs`): `list_workspaces` (with tabs, ordered), `insert_workspace`, `rename_workspace`,
  `move_workspace(id, index)`, `delete_workspace` (free-form only: moves its tabs to the end of their project
  workspaces in one transaction), `project_workspace(project_id)`, `open_tab(workspace_id, item, index?)` (moves an
  existing tab), `move_tab(tab_id, workspace_id?, index?)`, `close_tab`. Positions are rewritten 0..n with
  `place_at` inside a transaction.
- `insert_project` also makes the project's workspace (same transaction).
- Tests: seed from a database at 0007 with projects/worktrees/sessions/runs; project workspace made with a project
  and gone with it; one tab per item (opening elsewhere moves it); delete free-form moves tabs home; tab removed with
  its session/run; position renumbering.

### Task 3 — API (TDD, crates/pdt-api/tests/workspaces.rs)
- Routes from spec §5. `409 project_workspace` when deleting a project workspace; `404` for unknown ids;
  `400 invalid_input` for a tab naming both/neither item or an item that doesn't exist.
- `NewSession.tab` / `NewRun.tab` (`TabPlacement`, optional): after the row is inserted, open a tab there, else at
  the end of the project workspace. A bad placement is checked before anything is created.
- Tests: CRUD, placement defaults and explicit placement, project workspace on project create.

## Web

### Task 4 — data and pure helpers (TDD)
- `workspaces/status.ts`: `worstActivity(list)` (needs_you > working > idle > unknown), item lookup by tab.
- `workspaces/useWorkspaceData.ts`: workspaces query (poll 2 s), all projects' sessions/runs via `useQueries`,
  `itemFor(tab)`, `tabFor(sessionId | runId)`.
- `workspaces/useCurrent.ts`: current workspace and tab ids per device (localStorage, try/catch), falling back to
  the first workspace/tab when the stored one is gone.

### Task 5 — tab strip, switcher, tab content
- `WorkspaceBar`: switcher (dropdown with status, rename, delete free-form, new workspace) and tab strip (status dot,
  title, activity badge, unseen dot, close, menu with Move to workspace… / Move left / Move right, "+").
- Main area renders the current tab's `SessionView` or `RunView`; empty workspace shows a hint and the "+" chooser.
- `NewTabChooser`: Claude / Codex / Shell (`c` `x` `s`), skip-permissions toggle, Run… (RunMenu for the place), and
  a filter list of existing sessions/runs. Place = current tab's item; otherwise a project/place picker.
- Browser title `(<n needing you>) PDT`.

### Task 6 — session tree
- Every project, collapsible (per-device), status on project and place headers; the existing `SessionList` and
  `RunsSection` render inside each project. Clicking an item focuses its tab (opening one in its project workspace
  if it has none). Launching opens the tab after the current one when the project workspace is on screen.
  Free-form workspace name shown on items that live in one.
- Project settings opens in the main area until a tab is picked; the bottom panel uses the current tab's project.

### Task 7 — keyboard, drag, phone
- `useWorkspaceKeys`: window capture-phase keydown for Ctrl+] then one key (spec §4.4); `Ctrl+]` twice sends `\x1d`
  to the current terminal (TerminalView registers a sender for its session); `?` opens a key help dialog.
- Native HTML drag and drop: tabs reorder within the strip, drop onto a workspace in the switcher, tree items drop
  onto the strip.
- Phone (< md): tab strip scrolls, horizontal swipe on the bar changes tab, switcher is full width.

### Task 8 — docs and checks
- Update the spec for the planning decisions, CLAUDE.md (workspaces line), ROADMAP.
- fmt, clippy, `cargo test` (commit `web/src/api/gen`), web typecheck/test/build. PR; user tests with Run → pdt.
