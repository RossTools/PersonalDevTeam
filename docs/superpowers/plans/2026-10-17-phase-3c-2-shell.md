# Phase 3c-2 — Shell UX: plan

Spec: docs/superpowers/specs/2026-10-17-design-system-ux-design.md §6 (patterns), §7 "3c-2 — Shell UX". Branch
`phase-3c-2-shell`, based on main after Phase 3c-1 (PR #20) merges: merge main in before starting work.
Mockups: the "PDT UX pass" canvas, 3c row: Sessions, New tab chooser / command palette, Settings as a page, Phone
sidebar sheet (https://claude.ai/artifact/1EKGuHdvGce3SNSFKFgNqY).

**Status (2026-10-18):** done. Notes: the `Sheet` implementation (Base UI drawer, ~31 kB gzipped) loads lazily and is
preloaded when the app is idle, keeping the main bundle at 328 kB gzipped (3c-1: 322); `CommandList` items gained a
`detail` line; the tab strip scrolls the current tab into view (on a phone it shows about one tab).

**Goal:** the shell fixes from the audit, built on the 3c-1 library: a dev instance is recognisable, views share one
header with a ⋯ menu, an ended session says so and offers Restart, Settings is a page, a command palette reaches
everything, the new-tab chooser is one searchable list, and on a phone the sidebar is a sheet. Toasts for failed
actions (§7 item 9) already shipped in 3c-1.

**Decisions made while planning** (the spec is updated to match):
- **Who is "dev":** pdtd reports `profile` (`dev`/`stable`) and `port` in `/api/health`. The badge shows when the
  profile is dev **or** the port isn't the stable port (`brand.rs`), so a Run → pdt instance and an explicit
  `PDT_PORT` both show it. Text: `DEV · <port>`.
- **Palette keys:** Ctrl+] k everywhere, plus ⌘K on macOS (the terminal never needs ⌘K). Not Ctrl+K: that is
  readline's kill-line in the terminal.
- **Add project** leaves the sidebar form: the settings nav gets a "+" next to PROJECTS and the Sessions sidebar
  keeps its "Add project" button; both open the existing `AddProjectDialog`.
- **Ended sessions** keep the terminal's last screen; the bar floats over its bottom edge (no resize of the PTY,
  spec principle 4).
- **Phone rail:** Sessions, Files, Search (palette), Settings — four items, 44px. Settings stays in the rail (the spec's
  "Settings in the sheet footer" is dropped: four items fit).

## Wave 1 — four parallel tasks (disjoint files)

### Task 1 — instance identity (Rust TDD + web)
- `pdt-core`: `InstanceInfo { profile: Profile, port: u16, dev: bool }` (ts-rs exported), `dev = profile == Dev ||
  port != STABLE_PORT`; unit tests for the rule.
- `pdt-api`: `AppState.instance`, set by pdtd from its config; `Health` gains `profile`, `port`, `dev` (ts-rs
  exported). API test: a dev-profile state reports `dev: true`; stable profile on the stable port reports false (no
  real port is bound; the value is just reported).
- `pdtd`: fill `instance` from the resolved profile and port.
- Web: `useInstance()` (query `['health']`, no polling); `InstanceBadge` (Badge, waiting tone outline, mono
  `DEV · 8512`, tooltip "Development instance: data in <dir name>" if cheap, else none); `document.title` gets a
  `[dev] ` prefix (combine with the existing `(n)` attention prefix: `(2) [dev] PDT`). Tests for both.
- Files: crates/pdt-core (new module or brand.rs neighbour), crates/pdt-api/src/routes.rs (+ tests), crates/pdtd/src/main.rs,
  web/src/api/gen (generated), web/src/shell/InstanceBadge.tsx (+ test), web/src/shell/useInstance.ts.

### Task 2 — view header and ended-session bar
- `SessionView`: `ViewHeader` with AgentDot, title (keeps its width; the folder path is no longer shown in the
  header), `BranchChip` for a worktree branch or `ProjectKey` for the main checkout, `AgentLabel`; one primary action
  (Restart when ended/interrupted, else none) and a `MenuButton` ⋯: Compose (running), Stop (running; confirms when
  the agent is working, spec §6.2), Restart, Copy path, Open on GitHub (when known), Remove… (ended; confirms).
- Ended / interrupted: the terminal stays mounted showing its last screen, dimmed (`opacity-60`), with a fixed-height
  bar floating over its bottom edge: "Session ended · Restart · Remove…" (Restart primary) or "Interrupted when the
  daemon stopped · Resume" (agents: "the agent resumes its last conversation"). Today's "Session ended." corner text
  goes.
- `RunView`: same header pattern: run name + BranchChip, state label, URL link when ready; primary Stop (active) or
  Restart; ⋯ Reset data… / Remove… / Copy URL. Setup/Output segmented control stays.
- `FileViewer` header: path (mono, truncates from the left is fine), git status chip, size, ⋯ Copy path.
- Files: web/src/sessions/SessionView.tsx, web/src/runs/RunView.tsx, web/src/files/FolderView.tsx (FileViewer only),
  web/src/sessions/EndedBar.tsx (new), their tests. Don't touch TerminalView except to pass a `dimmed` prop if needed.

### Task 3 — command palette (component, data, keys; not wired into AppShell)
- `paletteItems(ctx)` (pure, TDD): groups and items from the workspaces API and projects:
  - **Go to:** every workspace; every tab (title, workspace, AgentDot); agents without a tab; each project's settings;
    This host.
  - **Actions:** New Claude / Codex / Shell here (current place), Run… (opens the new-tab chooser), Rename tab,
    Close tab, Move tab to workspace… (opens a second step listing workspaces), Next needs you, Theme: System /
    Light / Dark, Keyboard shortcuts.
  - Each item: id, label, group, keywords (project key, branch), shortcut hint (the Ctrl+] key where one exists).
- `CommandPalette` (`Dialog` + `CommandList`, opens with the filter focused, two-step for "Move tab to workspace…",
  Enter runs the item and closes, Escape closes, focus returns).
- `keys.ts`: `k` → `{ type: 'palette' }`, `KEY_HELP` entry; a ⌘K listener helper (`useMetaK`) that only reacts on
  macOS (`navigator.platform`/`userAgentData`) and calls preventDefault.
- Files: web/src/palette/* (new: paletteItems.ts, CommandPalette.tsx, tests), web/src/workspaces/keys.ts (+ test).

### Task 4 — new-tab chooser as one list
- `NewTabChooser` becomes a `Popover` anchored to the tab strip's "+" (a `Sheet` from the bottom on phones) holding:
  the `PlacePicker` chip; a `CommandList` with groups **Start here** (Claude `c`, Codex `x`, Shell `s`, each showing
  its key), **Run** (each configured run and test for the place; then "Run another branch…", which swaps the list for
  `RunBranchForm`), **Bring here** (other sessions and runs: AgentDot, title, their workspace); footer: the
  skip-permissions `Switch` and "Esc to close".
- Typing filters all groups; the one-key shortcuts work only while the filter is empty.
- Keeps today's API calls and placement (after the current tab).
- Files: web/src/workspaces/NewTabChooser.tsx (+ tests in AppShell.test.tsx that cover it — coordinate: only edit the
  chooser's tests), and an `anchor` prop it needs from WorkspaceBar (add the prop; Wave 2 wires it).

## Wave 2 — shell integration (one task, after Wave 1)

### Task 5 — AppShell, sidebar, settings, phone
- **Rail** (`IconRail`, `views.ts`): only views that exist (Sessions, Files, Settings; Search on phones); the
  `InstanceBadge` at the top on desktop, in the header on phones. Remove the phase placeholder notes.
- **Sidebar tidy:** no duplicate view heading in the Sessions view; agent rows' secondary line shows the branch (or
  nothing for the main checkout) and the workspace only when it isn't the current one.
- **Settings as a page:** list column = "This host", PROJECTS (+ button opens `AddProjectDialog`) with key chips; the
  inline `AddProjectForm` leaves the sidebar. `ProjectPage` sections: Project, Worktrees, Runs on this host (the
  `ProjectSettingsPanel` content as a Section, without its own page heading and `-mx-4` wrapper), Danger zone.
- **Palette wiring:** `usePrefixKeys` 'palette' + ⌘K open `CommandPalette`; actions call existing `ws` functions
  (through `runAction`), Run… opens the chooser, Theme uses `useThemePref`.
- **Chooser wiring:** pass the "+" button as its anchor.
- **Phone (< md):** the list column becomes a `Sheet` (left) opened by the workspace name in `WorkspaceBar` (a
  button with a chevron) and closed by choosing a workspace/agent, Escape, a tap outside or a swipe; the work area
  gets the full height. Desktop unchanged.
- Files: web/src/shell/*, web/src/workspaces/Sidebar.tsx, WorkspaceBar.tsx, web/src/projects/ProjectPage.tsx,
  web/src/runs/ProjectSettingsPanel.tsx (heading/wrapper only), tests.

## Task 6 — docs and checks
- CLAUDE.md: `/api/health` fields and the DEV badge, palette keys (Ctrl+] k, ⌘K), phone sidebar sheet.
- Spec §7: mark 3c-2 items done, note the planning decisions above; ROADMAP: 3c done, Phase 4 next.
- fmt, clippy, `cargo test` (commit `web/src/api/gen`), web typecheck/test/build; gallery and demo screenshots
  (desktop + phone, light + dark); Android emulator: sheet opens from the header, long-press menus, palette with the
  on-screen keyboard. PR; the user tests with Run → pdt.
