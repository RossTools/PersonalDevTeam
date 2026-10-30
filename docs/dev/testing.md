# Testing PDT

How PDT's own tests are run and what they must never touch. Before pushing, run the CI checks listed in
[SDLC.md](../SDLC.md#checks-before-pushing).

## Running them

- **Rust:** `cargo test` (and `cargo clippy --all-targets -- -D warnings`, `cargo fmt`). CI runs `cargo nextest run`:
  every test in its own process, all at once, so a test must not depend on another's state or on running alone.
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

## What tests never touch

| Never | Use instead |
| --- | --- |
| Port 7420, or a bare random port | Ports PDT picks for runs come from a block claimed with `test-support/port_blocks.rs` (one range per test crate, listed in that file) |
| The real data folders (`pdt`, `pdt-dev`) | A temp folder, as `PDT_DATA_DIR` |
| The OS keychain | `PDT_SECRET_STORE=memory` |
| Tailscale | `PDT_TAILSCALE_BIN` pointing at `fake-tailscale` (`crates/pdt-tailscale/src/bin`) or a missing file |
| GitHub, GitLab | `PDT_GITHUB_OFFLINE=1` (takes every forge offline), `GithubApi::offline()`, recorded answers (`pdt_forge::transport::testing::Recorded`), or `FakeForge` (`crates/pdt-forge`) |
| Real agents | `fake-agent` (`crates/pdt-api/src/bin/fake-agent`: Claude, Codex app-server and ACP); `PDT_CLAUDE_PROGRAM`, `PDT_CODEX_PROGRAM`, `PDT_DEVIN_PROGRAM` name the program |
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
