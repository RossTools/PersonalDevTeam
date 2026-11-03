# Personal Dev Team (PDT)

Instructions for coding agents (Claude Code reads this through CLAUDE.md, Codex reads it directly). Humans: start with README.md.

Personal command center for agentic development: tickets and a pipeline board, Claude Code / Codex / Devin sessions,
review, terminals.

Where things are written down (one home each; link, don't repeat):

| What | Where |
| --- | --- |
| The process: branches, checks, PRs, CI, merge queue, changelog, releases | docs/SDLC.md |
| How the code works: processes, crates, the model, where each area lives | docs/dev/architecture.md |
| Rules the code relies on that are easy to break | docs/dev/invariants.md (read before changing a part) |
| How tests run and what they never touch | docs/dev/testing.md |
| What PDT does for its users | docs/user (the website's docs) |
| Guides pdtd serves to every user's agents (edit as product text) | docs/agent-guide.md, docs/run-config.md (after docs/user/automation/run-configuration.md), docs/prompts/ |
| The design system | web/ui/README.md |
| The website | site/README.md |
| The roadmap and designs | this project's tickets in the stable PDT (key `PDT`; `curl -s '127.0.0.1:7420/api/tickets?project=<id>'`, ranked) |

Add or update tickets through the API when work finishes or new work turns up.

**Not the maintainer?** The parts that use the stable PDT on port 7420, its `PDT` tickets, `gh pr merge` and labels
describe the maintainer's own setup. If `curl -s 127.0.0.1:7420/api/projects` doesn't list this repository, follow
CONTRIBUTING.md instead: a plain branch (or `git worktree add`), a changelog fragment named after the issue or PR
(`GH-45.md`), and no calls to a PDT that isn't there.

## Writing docs
- **User docs (`docs/user`)**: what the user does and how PDT behaves, task-first and concise (style notes: docs/user/README.md).
  - **Never** ticket keys (`PDT-123`), code paths, API routes, internal history ("before …", "we decided …"), or reasoning, unless the reason is core to how something works.
  - The docs check (`pnpm --filter site check-docs`, in `scripts/check.sh`) catches some of this; the rest is on you.
- **Developer guide (`docs/dev`)**: how the code works and the invariants. No ticket history or decision logs.
- **Ticket keys and reasoning go in** the PR description, commit messages, code comments where useful, and the ticket itself.
- **Changelog fragments**: one or two sentences for users, no ticket keys in the text.
- **No design specs or plans in the repo**: no `docs/superpowers/`, no `docs/**/specs` or `plans` folders (the docs
  check fails on them). Designs go on the ticket (its description or comments).
- When behaviour changes, update docs/user and docs/dev in the same PR.

## Workflow (docs/SDLC.md has the full version)
- **A branch and worktree per change, made through the stable PDT** (outside contributors: CONTRIBUTING.md): find
  the project id with `curl -s 127.0.0.1:7420/api/projects` (match `repoPath`), then
  `POST 127.0.0.1:7420/api/projects/<id>/worktrees` with `{"branch":"..."}`; `cd` into
  `.worktrees/<branch, / replaced by ->`. Never commit on `main` in the main checkout.
- **A fresh worktree** needs `pnpm install --frozen-lockfile` once; its first cargo build is slow.
- **Before pushing**: `scripts/check.sh` (what CI runs); docs-only changes `scripts/check.sh --docs`. Commit
  `web/src/api/gen` (and `site/src/data`) when `cargo test` changes them.
- **Every PR adds a changelog fragment**, `.changes/unreleased/<TICKET>.md` (no ticket:
  `GH-<issue or PR>.md`; `kind: added | changed | fixed | breaking | internal`, then one or two sentences for users),
  or carries the `no-changelog` label when it only changes docs, CI or tests. `scripts/changelog.sh check`.
- **Labels**: `windows` for any path, process or file-system change; `macos` to run the macOS checks on the PR.
  Don't rename CI jobs (they are the ruleset's required checks). Only a release PR changes the version.
- **PR against `main`**; the user tests the branch with Run → pdt. When the user says merge, `gh pr merge <n> --auto`
  (no `--squash`): the merge queue squash-merges it once checks pass. A PR the queue drops needs fixing and merging
  again. Then clean up (docs/SDLC.md "After merge").
- **Releases only when the user asks**, through PDT's "Cut a release" workflow (`workflow_run`, inputs in docs/SDLC.md
  "Releasing"), never `gh workflow run release.yml`. If it is off or missing, ask the user to turn it on in Workflows.
- `gh pr checks` fails with this repo's fine-grained token; use `gh run list --branch <b>` and `gh run view <id>`.
- The stable PDT (7420) runs a release that can lag main: check `curl -s 127.0.0.1:7420/api/health` (`version`)
  before relying on a new API route or a fix.

## Layout
Adding a route, API client call, MCP tool, persona prompt, built-in workflow or screen: each goes in its own file,
plus at most one line in a central list (docs/dev/architecture.md "Adding to the hot lists").
- crates/pdt-core — pure domain types and rules (TDD), brand constants (no I/O)
- crates/pdt-git — git CLI wrapper
- crates/pdt-store — SQLite (sqlx), migrations in crates/pdt-store/migrations
- crates/pdt-agents — PTY terminals, detached runs, the agent adapters (`adapter/`), hooks and instructions
- crates/pdt-api — axum routes (errors are JSON `{code, message}`), pdt-mcp in `mcp/`, the workflow engine, test fakes in src/bin (`fake-agent`)
- crates/pdt-forge — the `Forge` trait, `FakeForge`, the forges' HTTP transport; crates/pdt-github — the GitHub forge (no `gh`)
- crates/pdt-int-* — the optional integrations, one crate each; pdtd cargo features, enabled in Settings → Plugins.
  Outside their crates only pdt-core `integration::KNOWN` and pdtd's features name them
- crates/pdt-tailscale, crates/pdt-update — tailscale CLI wrapper; GitHub release client
- crates/pdt-sheet — local sheets on IronCalc (`.xlsx`/`.csv`, cells, sort/filter, live diffs); same IronCalc version as web's `@ironcalc/workbook`
- crates/pdt-host — the supervisor (runs pdtd, installs updates, rolls back); crates/pdtd — the daemon (also `pdtd hook …`; demo seed in src/seed.rs)
- test-support — shared test helpers (`port_blocks.rs`)
- .pdt/project.toml — this repo's run configuration (the `pdt` dev instance, tests, areas)
- web/ — React + Vite UI; web/ui the design system (`@ui`; shadcn sources in web/ui/primitives, never imported by
  features; add with `pnpm dlx shadcn@latest add <name> -y -o -c web < /dev/null`); app-level shared components in web/src/common
- desktop/src-tauri — Tauri v2 tray app; site/ — the website (Astro + Starlight, reads docs/ in place); scripts/ — checks, release, signing

## Commands
- Rust: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`. CI runs `cargo nextest run` (each test
  in its own process, from archives split across parallel jobs; runners and caches in docs/SDLC.md "CI"); nextest
  isn't installed locally. `cargo test` writes the TS types into web/src/api/gen.
- Integration tests: one binary per crate, `tests/main.rs` with a `mod` per `tests/*.rs` file (add new files there;
  `--test integration <file>::` runs one). Build output per worktree, sccache, and `scripts/clean-targets.sh` (removes
  build output cargo hasn't written for 7 days across worktrees): docs/dev/testing.md "Build output".
- Web: `pnpm --dir web dev | test | typecheck | knip | build` (knip: unused exports, files and dependencies). `cargo test -p pdt-desktop` needs `pnpm --dir web build` first.
- Component gallery: `pnpm --dir web dev`, then `/gallery.html`. PR screenshots for web/ui: `node web/scripts/ui-screenshots.mjs <out dir>`
  (needs `pnpm --dir web exec playwright-core install chromium` once).
- Dev daemon (port 7421, data dir pdt-dev): `PDT_PROFILE=dev PDT_SEED=demo PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd`
  (PowerShell: set the same as `$env:` variables). `pnpm --dir web dev` proxies `/api` to 7421.
- Throwaway daemon for an agent to look at (screenshots, Playwright): build `target/debug/pdtd` and `web/dist`, then
  `scripts/scratch-daemon.sh start` (Windows: `scripts\scratch-daemon.ps1 start`); it prints `port=`, `dir=`, `url=`.
  Stop it with `scripts/scratch-daemon.sh stop <port>`. Set `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM` /
  `PDT_DEVIN_PROGRAM` to a stand-in, or new sessions start the real agents (docs/dev/testing.md "Looking at the app").
- Logs: `<data dir>/logs/pdtd.<date>.log`, `pdt-host.log`, `pdtd-launch.log`. Data lives in `<local data dir>/pdt` or `pdt-dev`.
- The shell is zsh: quote globs (`grep -r --include='*.ts'`).

## Rules
- **The stable instance (7420) is the user's**: never stop, restart or update it, never call its /api/shutdown, never
  open its UI in a test browser (attaching resizes the user's real PTY). Never run scripts/release-stable.* without
  `--build-only`/`-BuildOnly`, never run a built installer or stable app bundle, never `pdt-host install`/`uninstall`.
- **A dev instance always gets an explicit PDT_PORT and PDT_DATA_DIR** (Run → pdt, or `PDT_PROFILE=dev`).
- **Tests never touch** port 7420, the real data folders, the keychain, Tailscale, GitHub or real agents
  (docs/dev/testing.md). Servers on PDT-picked ports claim a block with `test-support/port_blocks.rs`.
- **Never `pkill -f target/debug/pdtd`** (it kills other worktrees' daemons), and **never `rm -rf` a path built from
  `$(…)` or a variable**: the agent stops for approval. Delete literal paths, or `scripts/scratch-daemon.sh stop`.
- **TDD for core logic.** Tests pass on Windows and macOS: fixture paths from `std::env::temp_dir()`.
- **Migrations**: never edit an applied one; add `crates/pdt-store/migrations/$(date -u +%Y%m%d%H%M%S)_<what>.sql`
  (not the next number). Backfills on open go in `Store::open`.
- **`.pdt/project.toml` stays readable by the stable PDT**: a new top-level section waits until stable supports it.
- **Names** (product, slug, daemon, ports, release repo, keychain service, also in UI text) live only in
  crates/pdt-core/src/brand.rs (and the files docs/dev/invariants.md lists).
- **Web**: web/src/lint.test.ts enforces the design-system rules (web/ui/README.md). The UI polls every 2 s: effects
  must be idempotent, never focus from an inline ref callback. Menus and popovers render in a portal. Shortcuts are Ctrl+] then a key; never bind Ctrl+K (in
  terminals it is kill-line; Shift+Enter sends ESC CR). Test helpers: `stubApi` / `renderWithQuery` in web/src/test-utils.tsx.
- **New dependencies** with `cargo add` / `pnpm add` (latest), never versions from memory.
- **Keep to the asked scope**; when the user narrows a request, do exactly the narrower thing. When asking the user to
  test, point them at the branch's run, not the main checkout's.
- On Windows, run cargo from PowerShell (Git Bash's `link` shadows MSVC's). The `codex` on PATH may be the old desktop
  app; check `codex --version`. Never touch the Mac mini's old self-hosted runner (`~/actions-runner-pdt`).
