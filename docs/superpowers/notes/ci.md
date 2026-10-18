# CI layout

Workflow: `.github/workflows/ci.yml`; bundle steps in `.github/actions/bundle`.

Linux does every check that doesn't depend on the OS, split into parallel jobs. macOS and Windows run only what they
can break on their own: clippy (for `cfg` code) and the Rust tests (paths, processes, PTYs).

| Check | Runs on | Does | Pull requests | main / nightly |
|---|---|---|---|---|
| lint (linux) | GitHub Linux | fmt, clippy | unless docs only | always / nightly |
| test (linux) | GitHub Linux | nextest, generated TS types are committed | unless docs only | always / nightly |
| web (linux) | GitHub Linux | web typecheck, vitest, build | unless docs only | always / nightly |
| build (macos-latest) ✱ | GitHub macOS (M1, 3 vCPU) | clippy, nextest, desktop crate clippy + test | unless docs only | always / nightly |
| build (windows-latest) ✱ | GitHub Windows | clippy, nextest | `windows` label, or a change under `windows/`, `*.ps1`, `*.cmd`, `.github/` | always / nightly |
| bundle (macos-latest, app) ✱ | GitHub macOS | release app, signed | `bundle` label, or a change under `desktop/`, `scripts/`, `.github/` | always (release) |
| bundle (windows-latest, nsis) ✱ | GitHub Windows | installer (also the only Windows compile of the desktop crate) | same | always (release) |

The Linux jobs run on `${{ vars.PDT_LINUX_RUNNER || 'ubuntu-latest' }}`. `ubuntu-latest` is 2 vCPU for this
internal repo (vitest takes ~4 min there); a larger runner (org Settings → Actions → Runners → New GitHub-hosted
runner; 8-core arm64 is $0.014/min, never covered by the included minutes) is used by setting the repo variable
`PDT_LINUX_RUNNER` to its name. Unset the variable to go back.

✱ required by the "Main" ruleset. The Linux checks must be added to it too (Settings → Rules → Main → required
status checks), or a PR can merge with them failing.

**Docs-only PRs** (every changed file under `docs/` or ending in `.md`) skip every job but `changes`; a `windows` or
`bundle` label still runs those jobs. No code reads a Markdown file today; if one ever does (an `include_str!` of a
`.md`, release notes from a file), narrow the rule in the `changes` job. Pushes to main always run everything.

A job skipped by its `if` still reports its check, and a skipped check counts as passing, so the job names must stay
the ruleset's required check names. Renaming a job means editing the "Main" ruleset first.

Why: on `windows-latest` (2 vCPU for this internal repo) vitest alone took 6m37s, and the web code is the same on every
OS; Linux minutes are the cheapest. CI no longer runs on the Mac mini, which also runs the agents and the stable PDT
(docs/superpowers/notes/runners.md). The standard macOS and Windows runners are covered by the plan's included
minutes. `PDT_MACOS_RUNNER` (repo variable) moves the macOS checks to another runner, such as `macos-latest-xlarge`
(paid per minute). A Windows-only break is caught on
the next main push or the nightly run instead of on the PR; add the `windows` label to a PR that touches
platform-specific code (`cfg(windows)`, process spawning, paths).

## Merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group) instead of requiring branches to be up
to date with main. A PR joins the queue when its checks pass and auto-merge is on; GitHub then makes
`gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it with the `merge_group` event. The
`changes` job picks jobs for it as for a PR: the files come from comparing the group's base and head (more than 300
files runs everything), the labels from PR `<n>` in the queue branch's name, which is why the group size stays 1.
The required checks report on the queue run the same way they do on a PR (skipped counts as passing), and the PR
merges when they pass. A PR that conflicts with main or fails there leaves the queue.

So each merge runs CI on the PR, in the queue, and on main (the release). The queue runs only once a PR is ready;
all of it runs on GitHub's runners.

## The old Mac mini runner (unused)
CI ran `build (macos-latest)` on a self-hosted runner on the Mac mini until PDT-27; no job targets it now.
- It lives in `~/actions-runner-pdt` (LaunchAgent `actions.runner.ross-game-tools-PersonalDevTeam.*`, labels
  `self-hosted, macOS, ARM64, pdt-mini`) and built under `~/.cache/pdt-ci`.
- To retire it (the user, by hand): `cd ~/actions-runner-pdt && ./svc.sh stop && ./svc.sh uninstall`, remove it under
  repo Settings → Actions → Runners, then delete `~/actions-runner-pdt` and `~/.cache/pdt-ci`.
- While it is registered, never make the repo public.
