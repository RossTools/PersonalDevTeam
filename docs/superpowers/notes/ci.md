# CI layout

Workflow: `.github/workflows/ci.yml`; shared steps in `.github/actions/checks` and `.github/actions/bundle`.

| Check (required on main) | Runs on | Pull requests | main / nightly |
|---|---|---|---|
| build (macos-latest) | Mac mini, self-hosted (`pdt-mini`) | unless docs only | always / nightly |
| build (windows-latest) | GitHub Windows | `windows` label, or a change under `windows/`, `*.ps1`, `*.cmd`, `.github/` | always / nightly |
| bundle (macos-latest, app) | GitHub macOS | `bundle` label, or a change under `desktop/`, `scripts/`, `.github/` | always (release) |
| bundle (windows-latest, nsis) | GitHub Windows | same | always (release) |

**Docs-only PRs** (every changed file under `docs/` or ending in `.md`) skip every job but `changes`; a `windows` or
`bundle` label still runs those jobs. No code reads a Markdown file today; if one ever does (an `include_str!` of a
`.md`, release notes from a file), narrow the rule in the `changes` job. Pushes to main always run everything.

A job skipped by its `if` still reports its check, and a skipped check counts as passing, so the job names must stay
the ruleset's required check names. Renaming a job means editing the "Main" ruleset first.

Why: hosted runners start cold (1.2 GB cache restore, full workspace rebuild) and Windows is twice as slow as macOS. On
the Mac mini the build output stays warm; a small change re-checks in about a minute. A Windows-only break is caught on
the next main push or the nightly run instead of on the PR; add the `windows` label to a PR that touches
platform-specific code (`cfg(windows)`, process spawning, paths).

## Keeping auto-merge PRs up to date

The "Main" ruleset requires a PR to be up to date with main, so a merge leaves every other auto-merge PR "behind".
`.github/workflows/update-prs.yml` runs on each push to main: for every open auto-merge PR from this repo that main
has moved past, it calls GitHub's update-branch (merging main in, with `expected_head_sha` so it never races a push)
and then dispatches `ci.yml` on the branch with `pr=<n>`. The dispatch is needed because a push made with
`GITHUB_TOKEN` starts no `pull_request` run; with `pr` set, the `changes` job reads that PR's files and labels, so
the dispatched run picks the same jobs a PR run would, and its checks land on the new head. A PR whose update
conflicts is skipped with a warning in the run summary.

## The Mac mini runner
- Lives in `~/actions-runner-pdt`, runs as the user (LaunchAgent `actions.runner.ross-game-tools-PersonalDevTeam.*`,
  installed with `./svc.sh install`), labels `self-hosted, macOS, ARM64, pdt-mini`. One job at a time.
- Builds with its own `CARGO_HOME` and `CARGO_TARGET_DIR` under `~/.cache/pdt-ci` (never the user's or a worktree's),
  6 build jobs and 6 test threads to leave room for agents; the target dir is wiped when it passes 30 GB.
- Tests already never touch port 7420, the real data folders, the keychain or Tailscale, so CI cannot disturb the
  stable PDT running on the same machine.
- Releases are never built here: bundles and signing run on GitHub's runners (`sign-macos.sh` edits the keychain
  search list while it runs).
- While the Mac mini is off or the runner is stopped, `build (macos-latest)` waits in the queue, and so do merges and
  releases. Status: `cd ~/actions-runner-pdt && ./svc.sh status`; restart: `./svc.sh stop && ./svc.sh start`.
- The repo is private: only people with access can open PRs that run on it. Never make the repo public with this
  runner attached.
