# Testing PDT

How PDT's own tests are run and what they must never touch. Before pushing, run the CI checks listed in
[SDLC.md](../SDLC.md#checks-before-pushing).

## Running them

- **Rust:** `cargo test` (and `cargo clippy --all-targets -- -D warnings`, `cargo fmt`). CI runs `cargo nextest run`:
  every test in its own process, all at once, so a test must not depend on another's state or on running alone.
  The tests are built once into a nextest archive and split across parallel jobs, each in a fresh checkout with the
  archive's `target/` (docs/SDLC.md "Build once, test in partitions"): a test may run in any job, and can rely only on
  the checkout and the test binaries, not on anything else a build wrote.
- **Generated types:** `cargo test` writes the TypeScript types (ts-rs) into `web/src/api/gen`. Commit them with the
  Rust change, or CI's diff check fails.
- **Web:** `pnpm --dir web test`, `typecheck`, `build`. `web/src/lint.test.ts` enforces the design-system rules
  (`web/ui/README.md`). Helpers: `stubApi` and `renderWithQuery` in `web/src/test-utils.tsx`; a test that touches the
  file system needs `// @vitest-environment node`.
- **Site:** `pnpm --filter site test` (link rewriting, and the user docs' check for internal references).
- **Desktop:** `cargo test -p pdt-desktop` needs `pnpm --dir web build` first; plain `cargo test` skips that crate.
- **Core logic is test-first.** Rules live in pdt-core as pure functions, so most of them are unit tests there; the
  routes are tested through the router in `crates/pdt-api/tests/` (helpers in `tests/common`: `call`, and
  `call_as_ui` for routes that need the UI token); the daemon end to end in `crates/pdtd/tests/`.
- **One integration test binary per crate:** a crate's `tests/*.rs` files are modules of its `tests/main.rs` (the
  `integration` target; Cargo.toml sets `autotests = false`), not a binary each. A new file needs `mod <name>;` there;
  a test in each `main.rs` (`test-support/all_listed.rs`) fails when one is left out. Shared helpers (`common`,
  `port_blocks`) are declared once in `main.rs` and imported with `use crate::common;`. A test that changes the
  process environment gets its own `[[test]]` target instead (pdt-api's `checks_env`). Run one file's tests with
  `cargo test -p pdt-api --test integration pm::`.

## Build output

Each worktree has its own `target/`, so its size multiplies by the number of worktrees. To keep it down:

- **Less debug info:** the dev profile (also `cargo test`'s) keeps line tables only for PDT's crates, so backtraces
  still show file and line, and none for dependencies (the workspace `Cargo.toml`). For a debugger that shows
  variables, build with `CARGO_PROFILE_DEV_DEBUG=full`.
- **One test binary per crate** (above): each links all of PDT and its dependencies.
- **A shared compile cache (optional):** install [sccache](https://github.com/mozilla/sccache) (`brew install
  sccache`, `cargo install sccache --locked`) and set `RUSTC_WRAPPER=sccache` in your shell profile, or
  `build.rustc-wrapper = "sccache"` in `~/.cargo/config.toml`. The repository doesn't set it, so builds without
  sccache keep working. Worktrees then share compiled dependencies instead of each building them again. sccache can't
  cache crates built incrementally, which PDT's own crates are in the dev profile: those still build per worktree.
  `CARGO_INCREMENTAL=0` caches them too, and drops `target/debug/incremental`, at the cost of slower rebuilds after
  an edit. Don't share one `CARGO_TARGET_DIR` between worktrees: agents building at once would wait on cargo's lock.
- **Stale artifacts:** `scripts/clean-targets.sh` removes build output cargo hasn't written for 7 days
  (`--days N`) from the main checkout's `target/` and every `.worktrees/*/target`, or from the folders given;
  `--dry-run` lists it. Cargo rebuilds what it needs later (from sccache when set up). A worktree's `cargo clean`
  removes all of its own.

## What tests never touch

| Never | Use instead |
| --- | --- |
| Port 7420, or a bare random port | Ports PDT picks for runs come from a block claimed with `test-support/port_blocks.rs` (one range per test crate, listed in that file) |
| The real data folders (`pdt`, `pdt-dev`) | A temp folder, as `PDT_DATA_DIR` |
| The OS keychain | `PDT_SECRET_STORE=memory` |
| Tailscale | `PDT_TAILSCALE_BIN` pointing at `fake-tailscale` (`crates/pdt-tailscale/src/bin`) or a missing file |
| GitHub, GitLab | `PDT_GITHUB_OFFLINE=1` (takes every forge offline), `GithubApi::offline()`, recorded answers (`pdt_forge::transport::testing::Recorded`), or `FakeForge` (`crates/pdt-forge`) |
| Real agents | `fake-agent` (`crates/pdt-api/src/bin/fake-agent`: Claude, Codex app-server and ACP); `PDT_CLAUDE_PROGRAM`, `PDT_CODEX_PROGRAM`, `PDT_DEVIN_PROGRAM` name the program |
| The machine's free disk space | `AppState::new` reads none (no disk is ever low); set `AppState::disk_space` to a fake |
| A real pdtd under pdt-host | `fake-pdtd` (`crates/pdt-host/src/bin`) |

A new daemon test copies the harness in `crates/pdtd/tests/common`: it drops every inherited `PDT_*` variable, then
sets the dev profile, an explicit port and data folder, in-memory secrets, a missing Tailscale, offline GitHub and
missing agent programs.

## Cross-platform

Tests pass on Linux, macOS and Windows. Build fixture paths from `std::env::temp_dir()` (`/srv/code` is not absolute
on Windows), and label the PR `windows` for any path, process or file-system change so the Windows checks run.

## Snapshots

- `crates/pdtd/src/integrations/tests.rs` pins, in `snapshot.json` and `pm_snapshot.md`, every tool agents and the PM
  get (with their flags), the node kinds' specs and the PM's prompt. A new tool, node kind or prompt change shows up
  there: update the snapshot on purpose and read the diff.
- A test in `crates/pdt-api/src/guides.rs` checks that the guides pdtd serves link to each other by API path, never by
  repository path.
- A test in `crates/pdt-store` refuses a migration named with a number instead of its UTC creation time, and
  duplicate versions.

## Looking at the app

- A dev instance for the user is the repository's **pdt** run (Run → pdt on the branch's worktree).
- For an agent (screenshots, Playwright): build `target/debug/pdtd` and `web/dist`, then
  `scripts/scratch-daemon.sh start` (Windows: `scripts\scratch-daemon.ps1`). It prints `port=`, `dir=`, `url=`; stop
  it with `scripts/scratch-daemon.sh stop <port>`, which also deletes its folder. Set the `PDT_*_PROGRAM` variables to
  a stand-in, or new sessions start the real agents.
- Seeds (`PDT_SEED`, dev profile only, on a database with no projects; `crates/pdtd/src/seed.rs`): `demo` fills in
  sample projects, tickets, sessions and runs; `website` is the made-up Acme Shop project the website's screenshots
  are taken from (`seed/website.rs`, `scripts/site-screenshots.mjs`, site/README.md "Screenshots"). Nothing is
  started by either.
- Never open the stable UI (7420) in a test browser: attaching a terminal resizes the user's real PTY.
