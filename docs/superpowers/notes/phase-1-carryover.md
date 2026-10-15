# Phase 1 → Phase 2+ carry-over

Items that Phase 1 reviews deferred. Plan the ones tagged with a phase into that phase.

## Phase 3 (structured agents)
- **Child-process cleanup:**
  - Put session processes in a Windows Job Object, so `cmd.exe`-wrapped shims and their grandchildren die with the session or with pdtd.
- **Scrollback is memory-only;** it is lost when pdtd restarts.

## Phase 4 (review/merge)
- **Worktree removal** happens on merge and must refuse dirty worktrees (Phase 1 decision 3).
- **Orphaned worktree:** a failed DB insert after `git worktree add` leaves a worktree on disk.
- **Branch-name path collision:** `feature/x` and `feature-x` map to the same path, and the second gives a 500 instead of a 4xx.

## Hardening backlog (no phase yet)
- **Git error classification:**
  - `rev-parse` and `check-ref-format` failures all map to UnknownRef/InvalidBranch.
  - `BranchExists` is detected from stderr text.
  - `HEAD` and `@{-1}` pass the pre-check and then give a 500.
  - A base starting with `-` is not guarded with `--end-of-options`.
- **Duplicated lists and lock entries:**
  - `PDT_ENV_VARS` (pdt-core) duplicates pdtd's config variable list with no sync test.
  - `session_locks` entries leak when a session is deleted other than through `remove`.
- **PTY edge cases:**
  - The ConPTY cursor query is answered only if it starts the first read.
  - The spawn holds the manager-wide lock across `openpty` and the process spawn.
- **Web UI:**
  - **Polling errors:** the session list blanks on a transient poll error, and the sessions query (with its 2 s interval) is duplicated in three components.
  - **Duplicate terminal:** a selected shell can show in both the work area and a panel tab at once.
  - **Theme:** the xterm theme is read once at mount.
  - **Input and resize:** `onBinary` is not wired, and a resize is sent on every ResizeObserver tick.
  - **Accessibility:** the accessible name reads "runningShell" (the StatusDot label runs into the title), and the panel tabs lack arrow-key navigation and a tabpanel.
  - **WebSocket handling:** unexpected JSON is not guarded, and handlers stay attached after dispose.
- **Desktop:**
  - A read timeout is reported as "not Personal Dev Team".
  - If the port goes from foreign to free mid-wait, the app never spawns pdtd.
  - The launch log is unbounded and has no start marker.
- **Release scripts:**
  - `Wait-Process` timeouts are swallowed.
  - The scripts POST `/api/shutdown` to whatever answers on 7420 without checking `/api/health`.
- **macOS:** the agent PATH comes from `$SHELL -l` (not `.zshrc`), is captured synchronously at startup (≤ 3 s), and always replaces the caller's PATH.
- **Test gaps:**
  - WebSocket exited-on-attach and lag-close paths.
  - Restart failure and 404 branches.
  - The panel's New shell button.
  - The lifecycle test can't tell graceful marking from the startup sweep.
- **Not yet verified by hand:**
  - The UI in a real browser (xterm rendering and focus, panel tabs, reload redraw).
  - The desktop error dialog exiting with code 1.
  - A non-build-only run of the release scripts.
