# Contributing to PDT

Thanks for helping. Bugs and ideas: open an issue (or use **Report a bug…** in PDT). Security problems: see
[SECURITY.md](SECURITY.md), never a public issue. For a larger change, open an issue first so we can agree on the
approach before you build it.

This page is what you need from a fresh clone to a pull request. The full process (CI, the merge queue, releases) is
in [docs/SDLC.md](docs/SDLC.md), how the code works in the [developer guide](docs/dev/architecture.md), and
[AGENTS.md](AGENTS.md) is the guide coding agents follow (a good summary for people too).

## Prerequisites

- **Rust through [rustup](https://rustup.rs)**. `rust-toolchain.toml` pins the version, and rustup installs it (with
  rustfmt and clippy) on the first build; a distribution's packaged `cargo` won't.
- **Node 24** (the version CI uses) and **pnpm 12**. `package.json` names the exact pnpm, so `corepack enable` is the
  easy way to get it. On older Node, `scripts/check.sh` skips the website build with a one-line notice.
- **git**.
- **Linux**: **zsh** (pdtd's terminal tests start it). Only to build the desktop tray app (`desktop/`), the Tauri
  system packages: on Debian or Ubuntu `build-essential file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev
  libayatana-appindicator3-dev librsvg2-dev`.
- **Windows**: the MSVC build tools (Visual Studio's "Desktop development with C++"); run cargo from PowerShell.
- **Disk and time**: a full `scripts/check.sh` from a fresh clone builds everything, takes a while the first time and
  leaves around **12 GB** in `target/`. [sccache](docs/dev/testing.md#build-output) shares compiled dependencies
  between checkouts, and `scripts/clean-targets.sh` removes stale build output.

## Making a change

1. **Fork** the repository and clone your fork.
2. **Make a branch** off `main` (short, kebab-case). Several at once: `git worktree add ../pdt-<branch> -b <branch>`.
3. **Install** once per checkout: `pnpm install --frozen-lockfile`.
4. **Build it test-first** for core logic (Rust tests next to the code, integration tests in each crate's `tests/`;
   [docs/dev/testing.md](docs/dev/testing.md)). Read [docs/dev/invariants.md](docs/dev/invariants.md) before changing
   a part: it lists the rules that are easy to break.
5. **Add a changelog fragment** (below).
6. **Run the checks**: `scripts/check.sh` runs what CI runs (`--docs` for a docs-only change). `cargo test`
   regenerates `web/src/api/gen` and `site/src/data`: commit them.
7. **Push and open a pull request** against `main`. The PR title becomes the commit subject on `main`, so write it as
   one line saying what changed. Fill in the template; "Fixes #45" links the issue.

When behaviour changes, update the user docs (`docs/user`) and the developer guide (`docs/dev`) in the same PR
(AGENTS.md "Writing docs" has the style rules).

### Changelog fragment

Every PR adds `.changes/unreleased/<name>.md`, one or two sentences for people who use PDT:

```markdown
---
kind: fixed
---
The board no longer jumps back to the top when a ticket moves.
```

`kind` is `added`, `changed`, `fixed`, `breaking` or `internal`. Maintainers name the file after their ticket
(`PDT-123.md`); you don't have one, so name it after your issue or pull request: `GH-45.md` (a second one:
`GH-45-<anything>.md`). `scripts/changelog.sh check` checks it.

A PR that only changes docs, CI or tests needs no fragment, but the `no-changelog` label instead; labels are added by
a maintainer, so say so in the PR. A maintainer also adds `windows` (paths, processes, the file system) or `macos` to
run those platforms' checks on your PR.

## CI on your pull request

GitHub holds the CI run of a first-time contributor's pull request until a maintainer approves it. CI then runs the
checks that fit what you changed (Linux lint, tests, web and website; macOS and Windows when labelled), the secret
scan and the changelog check ([docs/SDLC.md "CI"](docs/SDLC.md#ci)). Once a maintainer approves the PR, the merge
queue runs the full set and squash-merges it.

## Trying your branch

```sh
pnpm --dir web build
PDT_PROFILE=dev PDT_PORT=7421 PDT_DATA_DIR=/tmp/pdt-try PDT_SEED=demo PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd
```

Then open http://127.0.0.1:7421. `PDT_SEED=demo` fills an empty data folder with demo projects and tickets; delete the
folder to start again. Keep the port and data folder away from an installed PDT (port 7420 and its own data folder).
For UI work, `pnpm --dir web dev` serves the UI with hot reload on 5173 and proxies `/api` to 7421.

If you run PDT yourself, you can also add your clone as a project and try a branch with **Run → pdt**:
`.pdt/project.toml` is this repository's run configuration.
