# Phase 0 → Phase 1 carry-over

Items that the Phase 0 reviews deferred. The Phase 1 plan should cover them.

## Must address in Phase 1 (self-hosting depends on them)
- **WebSocket Origin check:** PTY sessions must validate `Origin` on WebSocket upgrades, alongside the existing Host allowlist (`pdt-api/src/host_guard.rs`).
  - Done in Phase 1 (Task 7).
- **Stable install procedure (spec §11):** where the released `pdtd` and `web/` live, and the steps to update it (build, stop, swap, start). Consider bundling `pdtd` as a Tauri `externalBin`.
  - Done in Phase 1 (Task 12).
- **Desktop attach health check:** before attaching, `GET /api/health` and confirm `name == PRODUCT_NAME`. Today the app attaches to whatever process is listening on the port.
  - Done in Phase 1 (Task 11).
- **Release logging:** write `pdtd` logs to `<data_dir>/logs`. The desktop app currently sends the daemon's stderr to null, and a release build shows no dialog when startup fails.
  - Done in Phase 1 (Task 8 for daemon logs, Task 11 for the launch log).
- **Graceful shutdown:** handle Ctrl-C / SIGTERM by stopping child PTYs and closing the SQLite pool.
  - Done in Phase 1 (Task 8).
- **Bottom-panel toggle on screen:** add a visible button, so phone and touch users can open terminals. Today only the Ctrl+\` shortcut does it.
  - Done in Phase 1 (Task 10).

## Nice to have
- Accessibility:
  - `aria-current` on the selected project;
  - touch targets of at least 44px on the mobile rail;
  - `required` and `aria-invalid` on form fields;
  - a height constraint on the mobile list column.
  - Done in Phase 1 (Task 10).
- Add IPv6 / bracketed hosts to the default Host allowlist.
  - Done in Phase 1 (Task 7).
- Emit the startup warnings after the bind succeeds.
  - Done in Phase 1 (Task 8).
- Add tests for:
  - `GitMissing`: Still open;
  - wrong content-type (415): Done in Phase 1 (Task 7);
  - `Health.version`: Done in Phase 1 (Task 7);
  - the `StatusDot` statuses and the `ghost` button variant: Done in Phase 1 (Task 12).
- Backup filename: use a sub-millisecond timestamp or a suffix to avoid collisions.
  - Done in Phase 1 (Task 3).
- CI: rename the toolchain step, since cargo installs the pinned toolchain lazily.
  - Done in Phase 1 (Task 12).

## Environment notes
Still open.
- On the dev machine, `codex` on PATH is the Codex desktop app (0.134), which shadows the npm CLI (0.162.1). Fix PATH order before Phase 3, which needs the Codex adapter.
- On Windows, run cargo from PowerShell. In Git Bash, Git's `link` shadows MSVC's `link.exe`.

## Phase 4
- Worktree removal happens on merge and must refuse dirty worktrees (Phase 1 decision 3).
