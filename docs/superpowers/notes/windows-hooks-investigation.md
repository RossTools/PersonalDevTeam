# Windows-only hang: `hook_events_change_a_sessions_activity`

Investigation brief for a Windows agent. The test passes on macOS/Linux and on `main`, but fails
deterministically on the `windows-latest` CI runner. Reproduce it locally on Windows and root-cause it.
(Temporary note — delete once the bug is fixed.)

## Setup
- Branch: `mobile-key-bar-and-bypass-permissions` (you're likely already on it).
- IMPORTANT (CLAUDE.md): run cargo from **PowerShell, not Git Bash** — Git Bash's `link` shadows MSVC
  `link.exe`. Do not touch the stable instance (port 7420) or real data dirs. Tests already use
  `PDT_SECRET_STORE=memory` and a fake tailscale.

## The failing test
`hook_events_change_a_sessions_activity` in `crates/pdtd/tests/hooks.rs`.

Reproduce (PowerShell):

```powershell
$env:RUST_BACKTRACE=1
cargo test -p pdtd --test hooks hook_events_change_a_sessions_activity -- --nocapture
```

Run ~5x to confirm determinism.

## Symptom
The test fires hooks via a child `pdtd hook claude <Event>` process and asserts the session's `activity`
after each:

- UserPromptSubmit → `working` (passes)
- Stop → `idle`, unseen=true (passes)
- view → unseen=false (passes)
- Notification `{"notification_type":"permission_prompt"}` → `needs_you`  ← **FAILS, Windows only**

At `needs_you`, `GET /api/sessions/{id}` returns an **empty/`null` body for the full 10s poll**, then times
out. So pdtd stops responding to that GET right after the Notification hook. Earlier GETs in the same run
work. No pdtd panic appears in captured test output.

## Already ruled out (don't redo)
- Not a timing lag: the test polls 10s and still gets null.
- Not the fake-claude stub: the test reaches this step, so session.txt/args.txt were written. The stub only
  writes when `PDT_SESSION_ID` is set, then runs `ping -n 120`.
- SQLite WAL is already on (`crates/pdt-store/src/lib.rs`: journal_mode=Wal, foreign_keys, max_connections=8).
  **No `busy_timeout` is set.**
- The `st.activity` `std::sync::Mutex` guards in `apply()`, `on_output()`, `on_input()`,
  `run_activity_ticker()` are scoped and dropped before any `.await`.
- The branch's feature (a `bypass_permissions` column + an on-screen key bar) does NOT touch the
  hook/activity path.

## Prime suspects
1. Hang/deadlock in the Notification → activity path. File: `crates/pdt-api/src/activity.rs`:
   - `post_report` (hook HTTP endpoint, ~line 253): does it hold `st.activity` (std Mutex) across
     `apply(...).await`, or hold the per-session async `order` lock in a deadlock-prone way?
   - `apply()` (~line 85) holds a per-session async mutex (`order_lock` + `order.lock().await`). Check
     contention with the GET path or another hook.
   - `get_one`/`load`/`get_session` in `crates/pdt-api/src/sessions.rs`: does the GET path share any lock
     with the hook path?
2. SQLite: despite WAL, a missing `busy_timeout` can stall / return `SQLITE_BUSY` on Windows (stricter file
   locking). Try `.busy_timeout(Duration::from_secs(5))` (or `.pragma("busy_timeout","5000")`) in
   `crates/pdt-store/src/lib.rs`; also check for a WAL checkpoint stall.
3. Tokio runtime starvation: check pdtd's runtime flavor / worker count in `crates/pdtd/src`. A blocking op
   or std Mutex held across an await could stall the whole server on a small pool.

## Decisive diagnostics
- When the `needs_you` GET returns null, immediately probe `GET /api/health`. If health **also** hangs →
  global runtime stall/deadlock. If only the session GET hangs → handler/row specific.
- Dump the daemon log on failure: the test daemon logs to `<data dir>/logs/pdtd.<date>.log` (data dir = the
  test tempdir; see `crates/pdtd/tests/common/mod.rs`, `start_with`). Look for errors / SQLITE_BUSY.
- Check whether the spawned pdtd process is alive and whether it pegs a CPU (deadlock vs busy-loop) during
  the hang.
- The test HTTP client is `http()` in `crates/pdtd/tests/common/mod.rs` (HTTP/1.0 + read_to_end, 10s
  timeout); `null` = empty read or failed connect. The health-vs-session probe disambiguates a TcpStream
  quirk from a real hang.

## Deliverable
- Root cause with evidence (logs + the health-vs-session probe).
- A fix committed to the `mobile-key-bar-and-bypass-permissions` branch.
- `cargo test -p pdtd --test hooks` green on Windows, run ~5x. Also `cargo clippy --all-targets -- -D
  warnings` and `cargo fmt --check`.
- Prefer a real pdtd robustness fix (lock-ordering or SQLite `busy_timeout`) over weakening the test. If only
  a test-side fix is viable, explain why. Report the root cause and the diff. Then delete this note.
