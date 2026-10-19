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

Windows bundles are Authenticode-signed when the Artifact Signing secrets and variables are set, and a main release
fails without them (docs/superpowers/notes/windows-signing.md).

## Merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group) instead of requiring branches to be up
to date with main. A PR joins the queue when its checks pass and auto-merge is on; GitHub then makes
`gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it with the `merge_group` event. The
`changes` job picks jobs for it as for a PR: the files come from comparing the group's base and head (more than 300
files runs everything), the labels from PR `<n>` in the queue branch's name, which is why the group size stays 1.
The required checks report on the queue run the same way they do on a PR (skipped counts as passing), and the PR
merges when they pass. A PR that conflicts with main or fails there leaves the queue.

So each merge runs CI on the PR, in the queue, and on main (the release). The queue runs only once a PR is ready,
and the Mac mini's warm build keeps the macOS check about a minute.

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
