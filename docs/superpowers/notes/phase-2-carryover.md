# Phase 2 → later carry-over

- **Verify by hand (Mac mini and Windows checklists):** `tailscale serve --https=<port> off` without `--bg`; that serve preserves the
  Host header; the live format of GitHub-Authentication-Token-Expiration; the Tauri NSIS installer's behaviour with the
  tray running during a silent update; launchd KeepAlive restarting pdt-host after self-replacement.
- **Codex activity uses OSC 7501 when available**, with screen and Enter heuristics for older CLIs.
  Phase 5's app-server adapter replaces the remaining fallback.
- **Runs restart only on request** when their branch changes; dev servers with hot reload pick up changes themselves.
- **A resumed run that moves maps only its new port**; its old port is never unserved, so a mapping can linger to a
  port nobody serves (harmless; `tailscale serve reset` clears it).
- **A dev instance and the stable one share the tailnet mapping for a run port:** `tailscale serve` has one mapping
  per https port per machine. A dev pdtd (a run of PDT itself) with remote access on maps its own runs' ports too, so
  turning remote access off in one instance unmaps the port the other instance's link uses (the run keeps running;
  toggling remote access on again, or restarting the run, maps it again).
- **Toggling remote access restarts web runs that use `$PDT_RUN_HOST` or `$PDT_RUN_URL`** (in setup, command or
  `env`, e.g. PDT's own run with `PDT_ALLOWED_HOSTS`), on the same port when free, so they pick up the new host.
  Other runs keep running and only their link changes; a program that reads `PDT_RUN_HOST` from its environment
  without the config naming it is not detected (restart it by hand).
  Known gaps in that restart: a Stop clicked while it is stopping the run is overridden (the run starts again); if
  the config is now invalid or no port is free, the run stays stopped with the reason only in the log; toggling on
  then off within milliseconds can leave the old link until a manual restart; test runs that name those variables
  restart too.
- **Windows run steps** go through a generated `.cmd`. The command is split on ` && ` (also inside quotes, so avoid it
  in quoted arguments) into one `call <part>` line per part, each followed by a check that ends the script with the
  failing exit code: shims (pnpm.cmd) after a `&&` work and the exit code is kept. Limits: no `if`/`for` at the start
  of a part, a first word that is a path needs backslashes (so `.pdt/project.toml` uses `cargo run -q -p pdtd` rather
  than `target/debug/pdtd`), and `%` is expanded a second time. Other operators (`||`, `;`, `|`) are not split.
  Readiness probes 127.0.0.1 and ::1.
- **Fork PRs** cannot be run until the git integration phase (only branches on origin).
- **Owner identity** compares Tailscale-User-Login case-insensitively; RFC 2047 values are decoded (Q only).
- **Windows rollback** restores a copy of the install folder; the uninstall registry entry keeps the newer version.
- **Tagged-device access** is refused; app-capability headers (Tailscale ≥ 1.92) could allow chosen tags later.
- **SQLite pool: no ping before acquire.** On Windows with few cores (CI, or a test run pinned to one core with
  `ProcessorAffinity = 1`), sqlx 0.9's ping of an idle connection before handing it out could stall a checkout for
  longer than 10 s while the pool had only idle connections: one request hung (here `GET /api/sessions/{id}` in
  `hook_events_change_a_sessions_activity`) while `/api/health` kept answering. The store now opens its pool with
  `test_before_acquire(false)`; a local SQLite connection doesn't go stale.
- **Windows CI: a request's task can go unpolled for 10–14 s (PDT-42, open).** Under the full nextest load on the
  4-core Windows runner, now and then one HTTP request stalls (any route: `GET /api/sessions/{id}`, the hook's activity
  POST, `/view`, lifecycle tests' POSTs) while other connections are served in ~1 ms. Probes (instrumented branch, ~60
  suite runs) showed: the stall is before the handler's first store call, so not SQLite; the task's own 2 s timer does
  not fire either, so the task is not polled at all; and no task poll on any worker took over 250 ms (tokio's
  `on_before/after_task_poll` hooks, checked to catch a 400 ms block), so no worker thread is blocked. Not reproduced on
  macOS. Next suspects: a lost I/O-readiness or timer wakeup on Windows (tokio/mio). pdtd now logs a request still
  running after 2 s, and one the client dropped (`pdt_api::slow`); pdtd tests print the daemon's logs on a timeout.
