# Personal Dev Team (PDT)

Instructions for coding agents (Claude Code reads this through CLAUDE.md, Codex reads it directly). Humans: start with README.md.

Personal command center for agentic development: tickets + pipeline board, Claude Code / Codex / Devin sessions, review, terminals.
Spec: docs/superpowers/specs/2026-10-14-personal-dev-team-design.md (feature specs beside it, plans in docs/superpowers/plans). Roadmap: this project's tickets in the stable PDT (key `PDT`; `curl -s '127.0.0.1:7420/api/tickets?project=<id>'`, ranked; the Board shows what is picked and in flight). Add or update tickets through the API when work finishes or new work turns up; docs/ROADMAP.md only points there.

## Workflow (full version: docs/SDLC.md)
- Every change gets its own branch and worktree, created through the stable PDT so its sessions and Run know it: find the project id with `curl -s 127.0.0.1:7420/api/projects` (match `repoPath`), then `POST 127.0.0.1:7420/api/projects/<id>/worktrees` with `{"branch":"..."}`. That makes the branch (from `main`) and the folder `.worktrees/<branch, / replaced by ->`; `cd` in and start. Never commit on `main` in the main checkout.
- A fresh worktree needs `pnpm install --frozen-lockfile` once, and its first cargo build is slow (empty `target/`).
- Before pushing, run the CI checks: fmt, clippy, `cargo test` (commit web/src/api/gen), web typecheck/test/build.
- Every PR adds a changelog fragment, `.changes/unreleased/<TICKET>.md`: front matter `kind: added | changed | fixed | breaking | internal`, then one or two sentences for people who use PDT (what changed for them, not the code). Several per ticket: `<TICKET>-<anything>.md`. Check with `scripts/changelog.sh check`. A PR that only changes docs, CI or tests gets the `no-changelog` label instead, or CI fails (docs/SDLC.md "Changelog").
- CI (docs/superpowers/notes/ci.md): Linux checks on every PR; macOS checks in the merge queue (on a PR only with the `macos` label or a `.github/` change); Windows checks only with the `windows` label or a change to their files; bundles only on the release PR (and by hand, and from a release's tag). A merge to main runs no CI and ships no app (only pages.yml redeploys the website when `site/**` or `docs/**` changed): releases are deliberate (docs/SDLC.md "Releases"). The version is Cargo.toml's workspace version, repeated in tauri.conf.json (`scripts/version.sh check`); only a release PR changes it. Label a PR `windows` for any path, process or file-system change. Docs-only PRs (only `docs/**`, `site/**` and `*.md`, except the guides pdtd builds in) skip every job but `changes` and `site` (the website build, which runs whenever `site/**` or `docs/**` changes; pages.yml deploys it from main, site/README.md). Job names are the ruleset's required checks: don't rename them.
- PR against `main`; the user tests the branch with Run → pdt. When the user says merge, `gh pr merge <n> --auto` (no `--squash`: the merge queue sets the method) instead of watching CI in a shell: once the PR's checks pass it joins main's merge queue, which runs CI again on main plus the PR and squash-merges it. The branch never needs updating with main; a PR the queue drops (a conflict with main or a failed queue run) needs fixing and merging again. Then clean up (docs/SDLC.md "After merge").
- Releases (docs/SDLC.md "Releases"; PDT's prompts no longer teach this, so it lives here): only when the user asks for one. Start it with PDT's "Cut a release" workflow (`workflow_run`, `name` "Cut a release", the project, `input` {"type": "stable", "bump": "patch" | "minor" | "major"} or {"type": "stable", "version": "X.Y.Z"}), never `gh workflow run release.yml` directly. Starting it is the release: the release PR merges itself once its checks pass (nobody reviews it; to change the notes, edit the `.changes/unreleased/` fragments on main first), and the user is told when the PR is open and when the release is published. A nightly (a pre-release of main for hosts on the nightly update channel) is published every night by itself when main changed; for one now give `{"type": "prerelease"}` (no release PR). If Cut a release is off or missing, ask the user to turn it on (or make it from its template, docs/SDLC.md "Releases") in Workflows.
- `gh pr checks` fails with this repo's fine-grained token; use `gh run list --branch <b>`, `gh run view <id> --json jobs`, `gh run watch <id> --exit-status`.
- The stable PDT (7420) runs a release, which can lag main by a version or two: check `curl -s 127.0.0.1:7420/api/health` (`version`) before relying on a new API route or assuming a fix is live.

## Layout
- crates/pdt-core — pure domain types, rules (TDD) + brand constants (no I/O)
- crates/pdt-git — git CLI wrapper
- crates/pdt-store — SQLite (sqlx) + migrations in crates/pdt-store/migrations
- crates/pdt-agents — PTY terminals, detached runs, the agent adapters (`adapter/{claude,codex,devin}.rs`), hooks and instructions
- crates/pdt-api — axum routes; errors are JSON {code, message}; pdt-mcp in `mcp/`; test fakes in src/bin (`fake-agent`)
- crates/pdt-tailscale — tailscale CLI wrapper (serve, status)
- crates/pdt-forge — the `Forge` trait (change requests, CI, merges, reviews, issues; declared capabilities) + `FakeForge`; the forges' HTTP `transport` (reqwest, `Offline`, recorded answers)
- crates/pdt-github — the GitHub forge over GitHub's REST and GraphQL APIs (no `gh`): an attempt's PR, Actions runs, merges, issues; the OAuth device-flow login
- crates/pdt-int-* — the optional integrations (discord, jira, buildkite, gitlab, arr, scryer, plex), one crate each on pdt-api's `sdk` and `Integration` trait; pdtd's cargo features, enabled per instance in Settings → Plugins (docs/dev/architecture.md "Crates"). The only list naming them outside their crates is pdt-core `integration::KNOWN` (and pdtd's features)
- crates/pdt-update — GitHub release client (token, verified downloads)
- crates/pdt-host — the supervisor: runs pdtd, installs updates, rolls back
- crates/pdtd — daemon binary (also `pdtd hook …` for agent hooks; demo seed in src/seed.rs; the integrations it is built with in src/integrations/)
- test-support — shared test helpers (`port_blocks.rs`)
- .pdt/project.toml — this repo's run configuration (dev instance + tests + areas)
- web/ — React + Vite UI; web/ui is the design system (web/ui/README.md): tokens.css is the only place colors live; public components in web/ui/*.tsx exported from `@ui`; shadcn/ui (React Aria base, Nova) sources in web/ui/primitives (ours to edit, never imported by features; add with `pnpm dlx shadcn@latest add <name> -y -o -c web < /dev/null`, which otherwise can hang on an overwrite prompt); app-level shared components in web/src/common
- desktop/src-tauri — Tauri v2 tray app
- site/ — the project website (Astro + Starlight; landing page, and the docs read in place from docs/), on GitHub Pages: site/README.md
- scripts/ — release-stable, macOS signing/notarizing, sidecar staging
- docs/ — user/ (the user docs the website publishes; style notes in its README, no internal references: `pnpm --filter site check-docs`), SDLC.md, run-config.md, agent-guide.md and prompts/ (the last three are served by pdtd: edit them as product text), dev/ (the developer guide: architecture, invariants, testing), superpowers/ (specs, plans, notes)

## Commands
- Rust: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`
- CI runs tests with `cargo nextest run` (every test in its own process, all at once). nextest is not installed on the dev machine; plain `cargo test` is the local check. Tests that start servers on PDT-picked ports claim a port block with `test-support/port_blocks.rs`, never a bare random one.
- TS types are generated by `cargo test` into web/src/api/gen — commit them with the Rust type change, or CI's diff check fails.
- Web: `pnpm --dir web dev | test | typecheck | build`. `cargo test -p pdt-desktop` and the desktop build need `pnpm --dir web build` first; plain `cargo test` skips the desktop crate.
- Component gallery: `pnpm --dir web dev`, then `/gallery.html` (dev only, never built). Screenshots for PRs touching web/ui: with `pnpm --dir web dev` running, `node web/scripts/ui-screenshots.mjs <out dir>` (needs `pnpm --dir web exec playwright-core install chromium` once).
- Run a dev daemon (port 7421, data dir pdt-dev):
  - PowerShell: `$env:PDT_PROFILE='dev'; $env:PDT_SEED='demo'; $env:PDT_WEB_DIR="$PWD\web\dist"; cargo run -p pdtd`
  - Bash (macOS): `PDT_PROFILE=dev PDT_SEED=demo PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd`
  - `pnpm --dir web dev` proxies `/api` to 7421. A bare pdtd serves no UI unless `PDT_WEB_DIR` is set or a `web/` dir sits next to the exe.
- Throwaway dev daemon for an agent to look at (screenshots, Playwright; Run → pdt needs the user): build `target/debug/pdtd` and `web/dist`, then `scripts/scratch-daemon.sh start` (Windows: `scripts\scratch-daemon.ps1 start`). It picks a free port and a scratch folder under `<temp dir>/pdt-scratch/`, starts pdtd with the dev profile, demo seed (`--seed ''` for none), in-memory secrets, no Tailscale and GitHub offline, and prints `port=`, `dir=` and `url=`. Stop it with `scripts/scratch-daemon.sh stop <port>`, which kills it by port and deletes its folder (only folders under `pdt-scratch`; `list` shows them; `start` removes day-old leftovers). It still launches the real `claude`/`codex`/`devin` for new sessions (found in their install location even with PATH changed); set `PDT_CLAUDE_PROGRAM` / `PDT_CODEX_PROGRAM` / `PDT_DEVIN_PROGRAM` to a stand-in script to avoid that. Never `pkill -f target/debug/pdtd` (that kills other worktrees' daemons). Headless Chromium has no GPU, so WebGL/perf numbers there mean little.
- Never `rm -rf` a path built from `$(…)` or a variable (`rm -rf "$(cat /tmp/dir)"`, `rm -rf $DIR`): the agent can't tell what it deletes and stops for the user's approval, even in bypass mode, and an agent step stalls. Delete literal paths, or use `scripts/scratch-daemon.sh stop`.
- Never open the stable UI (7420) in a test browser: attaching a terminal resizes the user's real PTY.
- Logs: `<data dir>/logs/pdtd.<date>.log`, `pdt-host.log`, `pdtd-launch.log`. Data lives in `<local data dir>/pdt` or `pdt-dev`.
- The shell is zsh: quote globs (`grep -r --include='*.ts'`), or zsh fails with "no matches found".

## Web conventions
- Enforced by web/src/lint.test.ts (no raw colours, no arbitrary text sizes or z-index, no `window.confirm`, React Aria / Base UI / primitives only inside web/ui): see web/ui/README.md.
- Colour class names follow shadcn: `bg-primary` (accent blue), `bg-accent` (soft selected/hover fill), `text-muted-foreground` (secondary text), `text-destructive`; status colours `running`/`waiting`/`failed`/`idle` are for state only. Merge classes with `cn` from `@ui/lib/utils` (it knows our theme names).
- Test helpers: `stubApi` / `renderWithQuery` in web/src/test-utils.tsx. Tests that touch the file system need `// @vitest-environment node`.
- The UI polls every 2 s, so components re-render constantly: effects must be idempotent, and never focus from an inline ref callback (use useRef + useEffect). Menus and popovers render in a portal (overflow containers clip them).
- Shortcuts are Ctrl+] then a key (web/src/workspaces/keys.ts); never bind Ctrl+K (the terminal's kill-line). In terminals Shift+Enter sends ESC CR.

## How PDT works (read the developer guide before changing a part)
- docs/dev/architecture.md: the processes (pdt-host, pdtd, the tray, the web UI) and how they talk, the crates, the core model (projects, tickets, attempts, sessions, runs, workflows) and where each area lives.
- docs/dev/invariants.md: the rules that are easy to break, each with where it is enforced. docs/dev/testing.md: how tests run and what they never touch.
- What PDT does for its users is in docs/user (published on the website); run configuration in docs/run-config.md.
- Invariants that are easy to break (the full list: docs/dev/invariants.md):
  - The user's own agent settings and config files are never edited; "Always allow" is session-only for every agent.
  - A session or run has at most one tab, and closing its tab stops and removes it.
  - One active attempt per ticket; parent tickets are never started (their parts are).
  - PR state and file overlaps are computed in memory and added to ticket responses, not stored.
  - Conversations are never auto-resumed as terminals, and nothing is sent to an agent without the user's click or a workflow the user enabled (built-ins such as PR notices are enabled by default and can be turned off). The exception: a PM conversation's own `watch` tells that conversation about what it watches, without asking.
  - Guides served by pdtd link to each other by API path, never by repo path (a test checks).

## Rules
- TDD for core logic. Tests must pass on Windows and macOS: build fixture paths from `std::env::temp_dir()` ("/srv/code" is not absolute on Windows), and label the PR `windows` for any path, process or file-system change.
- Keep to the asked scope; when the user narrows a request mid-way, do exactly the narrower thing.
- When asking the user to test, point them at the branch's run (Run → pdt on the worktree), not the main checkout's.
- Agents never stop, restart or update the stable instance (port 7420; it hosts the agents building this repo), and never call its /api/shutdown. Only pdt-host and the user's Update do. Never run scripts/release-stable.* without -BuildOnly/--build-only, never run or launch a built installer or stable app bundle (a first launch registers it at login), and never run `pdt-host install` or `pdt-host uninstall`.
- A dev instance always gets an explicit PDT_PORT and PDT_DATA_DIR: use this repo's "pdt" run (Run → pdt), or PDT_PROFILE=dev (port 7421, data dir pdt-dev) for a manual one. pdtd refuses a data folder another pdtd holds. Dev instances never update.
- Tests never touch port 7420 or the real data folders, never the real keychain (PDT_SECRET_STORE=memory), never the real Tailscale (`PDT_TAILSCALE_BIN`) or GitHub (`PDT_GITHUB_OFFLINE=1`, `GithubApi::offline()`, recorded answers, `FakeForge`), and use `fake-agent` instead of real agents. Never touch the Mac mini's old self-hosted runner (`~/actions-runner-pdt`).
- Names (product, slug, daemon, ports, release repo, keychain service; also in user-facing UI text) live only in crates/pdt-core/src/brand.rs (+ desktop/src-tauri/tauri.conf.json, desktop/src-tauri/tauri.bundle.conf.json, desktop/src-tauri/windows/hooks.nsh, scripts/release-stable.* and the install scripts site/public/install.{sh,ps1}).
- Never edit an applied migration (sqlx checksums them); add a new one, named by its UTC creation time: `crates/pdt-store/migrations/$(date -u +%Y%m%d%H%M%S)_<what>.sql` (not the next number: `0001`–`0029` are the old numbered ones, and a test fails on a new number or a duplicate version). Nothing lists migrations by hand (`sqlx::migrate!` embeds the folder), and one with an older timestamp than the last applied still applies, so branches merge in any order. Data backfills that run when the store opens (e.g. project keys) go in `Store::open`, not in a migration (tests that build their own migrator skip them).
- `.pdt/project.toml` must stay readable by the stable PDT: a new top-level section waits until the stable release supports it (docs/dev/invariants.md "Code and data").
- On Windows, run cargo from PowerShell, not Git Bash: Git Bash's `link` shadows MSVC `link.exe`.
- The `codex` on PATH may be the Codex desktop app (an old version) shadowing the npm CLI; check `codex --version`.
- New dependencies: add with `cargo add` / `pnpm add` (latest), never versions from memory.
- When behaviour changes, update the user docs (docs/user) and the developer guide (docs/dev) in the same PR.
