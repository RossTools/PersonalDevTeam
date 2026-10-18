# Personal Dev Team (PDT)

A personal command center for building software with AI coding agents. PDT runs Claude Code and Codex sessions in
terminals that outlive the browser window, keeps each change in its own git worktree, and starts a dev build of any
branch with one click so you can try it. It runs as a daemon on your machine, and the same web UI works from a phone
or another computer over Tailscale.

PDT is built inside PDT: the agents working on this repo run in the stable instance and test their branches as dev
instances of it.

> Personal Dev Team is a working name; it changes before any public release.

## What it does today

- **Agent sessions and terminals.** Claude Code, Codex and shell sessions run in PTYs owned by the daemon. They survive
  closed windows and daemon restarts (agents resume their conversation), and show their activity in the sidebar.
- **Workspaces and tabs.** Each project has a workspace, a strip of tabs showing sessions and runs; you can add your
  own. Keyboard shortcuts start with `Ctrl+]`.
- **Git integration.** Clone by URL, one worktree per branch under `.worktrees/`, GitHub links, a Files view.
- **Runs.** A project describes how to run and test itself in `.pdt/project.toml`. Run on a branch starts it on its
  own port with its own data folder and gives you the link.
- **Remote access.** Optional `tailscale serve` mapping, restricted to the owner's Tailscale login.
- **Updates.** The `pdt-host` supervisor keeps the daemon running, installs new GitHub releases when idle, health
  checks them and rolls back on failure. An optional tray app opens the UI.

Coming next (docs/ROADMAP.md): tickets and a board, structured agent views, code review, and a pipeline of agent
personas that hand work to each other.

## Install

Every push to `main` publishes a release on GitHub (`v0.1.<n>`): `*-setup.exe` for Windows and `*.app.tar.gz` for
macOS. Run the installer on Windows; on macOS unpack the app into Applications and open it once. From then on
`pdt-host` starts at login and keeps `pdtd` running on http://127.0.0.1:7420. Updates install
themselves after that. The repo is private, so release downloads need a GitHub token (stored in the OS keychain from
Host settings).

## Develop

You need Rust (version pinned in `rust-toolchain.toml`), Node with pnpm, and git. Then:

```sh
pnpm install --frozen-lockfile
pnpm --dir web build
PDT_PROFILE=dev PDT_SEED=demo PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd   # http://127.0.0.1:7421, demo data
```

On Windows use PowerShell (see AGENTS.md). For UI work, `pnpm --dir web dev` serves the UI with hot reload on
http://localhost:5173 and proxies `/api` to the dev daemon.

Checks (CI runs the same):

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                      # also regenerates web/src/api/gen; commit it
pnpm --dir web typecheck && pnpm --dir web test && pnpm --dir web build
```

A dev instance never touches the stable one: it uses its own port and data folder (`pdt-dev`), and keeps secrets in
memory when `PDT_SECRET_STORE=memory`.

## Layout

| Path | What |
|---|---|
| `crates/pdtd` | the daemon |
| `crates/pdt-host` | the supervisor: runs pdtd, installs updates, rolls back |
| `crates/pdt-api` | HTTP and WebSocket routes (axum) |
| `crates/pdt-store` | SQLite store and migrations |
| `crates/pdt-agents` | PTY terminals and agent launching |
| `crates/pdt-core` | domain types and brand names |
| `crates/pdt-git`, `pdt-tailscale`, `pdt-update` | git, Tailscale and GitHub release wrappers |
| `web/` | React + Vite UI; `web/ui` is the design system |
| `desktop/src-tauri` | the optional tray app (Tauri v2) |
| `.pdt/project.toml` | how PDT runs and tests this repo |

## Docs

- [AGENTS.md](AGENTS.md): the working guide for coding agents (and the most complete description of how PDT behaves)
- [docs/SDLC.md](docs/SDLC.md): how a change goes from worktree to release
- [docs/ROADMAP.md](docs/ROADMAP.md): what is done and what comes next
- [docs/run-config.md](docs/run-config.md): how to configure Run and Test for a project; [docs/prompts/set-up-project.md](docs/prompts/set-up-project.md) is a prompt that has an agent do it
- [docs/superpowers/specs](docs/superpowers/specs): designs; [plans](docs/superpowers/plans): build plans;
  [notes](docs/superpowers/notes): CI layout, carry-overs, incident notes
