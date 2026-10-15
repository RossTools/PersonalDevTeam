# Phase 0 → Phase 1 carry-over

Items that the Phase 0 reviews deferred. The Phase 1 plan should cover them.

## Must address in Phase 1 (self-hosting depends on them)
- **WebSocket Origin check:** PTY sessions must validate `Origin` on WebSocket upgrades, alongside the existing Host allowlist (`pdt-api/src/host_guard.rs`).
- **Stable install procedure (spec §11):** where the released `pdtd` and `web/` live, and the steps to update it (build, stop, swap, start). Consider bundling `pdtd` as a Tauri `externalBin`.
- **Desktop attach health check:** before attaching, `GET /api/health` and confirm `name == PRODUCT_NAME`. Today the app attaches to whatever process is listening on the port.
- **Release logging:** write `pdtd` logs to `<data_dir>/logs`. The desktop app currently sends the daemon's stderr to null, and a release build shows no dialog when startup fails.
- **Graceful shutdown:** handle Ctrl-C / SIGTERM by stopping child PTYs and closing the SQLite pool.
- **Bottom-panel toggle on screen:** add a visible button, so phone and touch users can open terminals. Today only the Ctrl+\` shortcut does it.

## Nice to have
- Accessibility:
  - `aria-current` on the selected project;
  - touch targets of at least 44px on the mobile rail;
  - `required` and `aria-invalid` on form fields;
  - a height constraint on the mobile list column.
- Add IPv6 / bracketed hosts to the default Host allowlist.
- Emit the startup warnings after the bind succeeds.
- Add tests for:
  - `GitMissing`;
  - wrong content-type (415);
  - `Health.version`;
  - the `StatusDot` statuses and the `ghost` button variant.
- Backup filename: use a sub-millisecond timestamp or a suffix to avoid collisions.
- CI: rename the toolchain step, since cargo installs the pinned toolchain lazily.

## Environment notes
- On the dev machine, `codex` on PATH is the Codex desktop app (0.134), which shadows the npm CLI (0.162.1). Fix PATH order before Phase 3, which needs the Codex adapter.
- On Windows, run cargo from PowerShell. In Git Bash, Git's `link` shadows MSVC's `link.exe`.
