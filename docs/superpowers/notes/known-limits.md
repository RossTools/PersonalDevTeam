# Known limits

Gaps and rough edges that are known and not (yet) tickets. Gathered from the phase 0–2 carry-over notes and the tray
release notes (PDT-70); the finished items were dropped (they are in git history). When one gets fixed or becomes a
ticket, remove it here.

## Sessions and terminals

- **Scrollback is memory only** (a 512 KB buffer per PTY, crates/pdt-agents/src/pty.rs); a pdtd restart loses it.
  Conversations are stored (`session_events`).
- **Windows: sessions are not in a job object.** Only detached runs are (pdt-agents `detached.rs`); sessions kill
  their process tree with `taskkill /T /F`, so a grandchild that outlives pdtd is not cleaned up.
- **Codex terminal activity** uses OSC 7501 when the CLI sends it, with screen and Enter heuristics for older CLIs
  (crates/pdt-api/src/activity.rs; docs/superpowers/notes/phase-2-agent-signals.md).
- PTY edge cases: the ConPTY cursor query is answered only if it starts the first read; the spawn holds the
  manager-wide lock across `openpty` and the process spawn (PDT-48).

## Git and worktrees

- **Orphaned worktree:** a failed DB insert after `git worktree add` leaves the worktree on disk
  (crates/pdt-api/src/worktrees.rs).
- Git error classification: `rev-parse` and `check-ref-format` failures all map to UnknownRef/InvalidBranch;
  `BranchExists` is detected from stderr text; `HEAD` and `@{-1}` pass the pre-check and then give a 500; a base
  starting with `-` is not guarded with `--end-of-options` on the worktree-add path.
- No test covers the git-not-installed error (`GitMissing`).

## Runs and remote access

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
  without the config naming it is not detected (restart it by hand). Gaps in that restart: a Stop clicked while it is
  stopping the run is overridden (the run starts again); if the config is now invalid or no port is free, the run
  stays stopped with the reason only in the log; toggling on then off within milliseconds can leave the old link
  until a manual restart; test runs that name those variables restart too.
- **Windows run steps** go through a generated `.cmd` (limits in docs/run-config.md "Traps"). Readiness probes
  127.0.0.1 and ::1.
- **Fork PRs** cannot be run (Run takes branches of the repository).
- **Owner identity** compares Tailscale-User-Login case-insensitively; RFC 2047 values are decoded (Q only).
- **Tagged-device access** is refused; app-capability headers (Tailscale ≥ 1.92) could allow chosen tags later.
- Never checked by hand: `tailscale serve --https=<port> off` without `--bg`; that serve preserves the Host header;
  the live format of GitHub-Authentication-Token-Expiration.

## Store

- **SQLite pool: no ping before acquire.** On Windows with few cores (CI, or a test run pinned to one core), sqlx's
  ping of an idle connection before handing it out could stall a checkout for over 10 s while the pool had only idle
  connections. The store opens its pool with `test_before_acquire(false)` (crates/pdt-store/src/lib.rs); a local
  SQLite connection doesn't go stale.
- **Windows CI: a request's task can go unpolled for 10–14 s** under the full nextest load (PDT-47; investigated in
  PDT-42): not SQLite, not a blocked worker; suspected a lost I/O-readiness or timer wakeup in tokio/mio on Windows.
  pdtd logs a request still running after 2 s, and one the client dropped (`pdt_api::slow`).

## Tray, releases and updates

- **Local builds report 0.1.0,** so a locally installed stable app always sees the newest GitHub release as newer and
  offers it; accepting replaces the local build with the CI build.
- **Skipped release numbers.** A main CI run still queued when a newer push arrives is superseded by it, so that
  commit gets no release and its number is skipped.
- **macOS releases are arm64 only** (built on `macos-latest`); Intel Macs get no working update.
- **Windows rollback** restores a copy of the install folder; the uninstall registry entry keeps the newer version.
- **Single-instance race (Windows, tauri-plugin-single-instance).** A second launch in the first few hundred ms finds
  the mutex but no window and keeps running: two tray icons sharing one pdtd.
- **Release scripts:** `release-stable.sh` passes all tray pids to one `kill -0` loop, which ends as soon as any one
  has exited; `release-stable.ps1` swallows `Wait-Process` timeouts.
- **Self-signed macOS builds** make the keychain ask again after every update (docs/superpowers/notes/developer-id.md).

## Web UI (from the Phase 1 review; unverified since)

- The xterm theme is read once at mount.
- `onBinary` is not wired, and a resize is sent on every ResizeObserver tick.
- Panel tabs lack arrow-key navigation and a tabpanel.
- Terminal WebSocket handlers: unexpected JSON is not guarded, and handlers stay attached after dispose.
