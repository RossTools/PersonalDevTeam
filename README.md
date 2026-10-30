# Personal Dev Team (PDT)

A personal command center for building software with AI coding agents. PDT runs Claude Code, Codex and Devin sessions in
terminals that outlive the browser window, keeps each change in its own git worktree, and starts a dev build of any
branch with one click so you can try it. It runs as a daemon on your machine, and the same web UI works from a phone
or another computer over Tailscale.

PDT is built inside PDT: the agents working on this repo run in the stable instance and test their branches as dev
instances of it.

> Personal Dev Team is a working name; it changes before any public release.

## What it does today

- **Agent sessions and terminals.** Claude Code, Codex, Devin and shell sessions run in PTYs owned by the daemon. They survive
  closed windows and daemon restarts (agents resume their conversation), and show their activity in the sidebar.
- **Workspaces and tabs.** Each project has a workspace, a strip of tabs showing sessions and runs; you can add your
  own. Keyboard shortcuts start with `Ctrl+]`.
- **Git integration.** Clone by URL, one worktree per branch under `.worktrees/`, GitHub links, a Files view.
- **Runs.** A project describes how to run and test itself in `.pdt/project.toml`. Run on a branch starts it on its
  own port with its own data folder and gives you the link.
- **Remote access.** Optional `tailscale serve` mapping, restricted to the owner's Tailscale login.
- **Conversations.** Agent sessions can also run as a structured chat (messages, tool calls, approval cards, diffs,
  images), switchable to the agent's own terminal UI.
- **Tickets and a board.** A backlog and a To do / In progress / Done board per project, with parent tickets split into
  parts. Start on a ticket makes its branch and worktree and starts an agent on it; PDT tracks the PR and CI and
  tells the agent when CI fails or the merge queue drops it.
- **Review.** A Changes view of a branch's diff with line comments sent back to the agent, Approve, and Merge (locally
  or through the PR's auto-merge).
- **Updates.** The `pdt-host` supervisor keeps the daemon running, installs new GitHub releases when idle, health
  checks them and rolls back on failure. An optional tray app opens the UI.

Coming next (tracked as tickets in PDT itself): a pipeline of agent personas that hand work to each other.

## Install

Releases are published on GitHub when they are cut (`vX.Y.Z`; merges to `main` don't ship): `*-setup.exe` for
Windows, `*.app.tar.gz` for macOS, and `pdt-linux-<arch>.tar.gz` (headless) or `pdt-linux-<arch>.AppImage` for Linux
(x86_64, aarch64). Run the installer on Windows; on macOS unpack the app into Applications and open it once; on Linux
run the AppImage, or unpack the tarball and run `pdt/pdt-host install` (a systemd user service; see
docs/reference/host.md "Linux" for starting at boot with `loginctl enable-linger`). From then on
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
| `crates/pdt-agents` | PTY terminals, agent launching and the conversation adapters |
| `crates/pdt-core` | domain types and brand names |
| `crates/pdt-git`, `pdt-tailscale`, `pdt-github`, `pdt-update` | git and Tailscale CLI wrappers, GitHub API and GitHub release clients |
| `web/` | React + Vite UI; `web/ui` is the design system |
| `desktop/src-tauri` | the optional tray app (Tauri v2) |
| `.pdt/project.toml` | how PDT runs and tests this repo |

## Docs

- [docs/user](docs/user): the user docs (Getting started and the rest), published on the website
- [SECURITY.md](SECURITY.md): how to report a vulnerability privately (never in a public issue)
- [AGENTS.md](AGENTS.md): the working guide for coding agents
- [docs/reference](docs/reference): how each part of PDT behaves (host, agent sessions, conversations, tickets,
  review, workspaces)
- [docs/SDLC.md](docs/SDLC.md): how a change goes from worktree to release
- [docs/ROADMAP.md](docs/ROADMAP.md): where the roadmap lives now (PDT's own tickets)
- [docs/run-config.md](docs/run-config.md): how to configure Run and Test for a project; [docs/prompts/set-up-project.md](docs/prompts/set-up-project.md) is a prompt that has an agent do it (built in: Settings → project → Set up with an agent)
- [docs/agent-guide.md](docs/agent-guide.md): how agents use PDT (tickets, worktrees, runs). PDT serves it and the other guides at `/api/guides`, and a project's agent instructions live in its Settings, so a repo needs no PDT files
- [docs/superpowers/specs](docs/superpowers/specs): designs; [plans](docs/superpowers/plans): build plans;
  [notes](docs/superpowers/notes): CI layout, known limits, protocol facts, incident notes
