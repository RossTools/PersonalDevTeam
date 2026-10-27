# CI layout

Workflow: `.github/workflows/ci.yml`; shared setup (toolchain, cache, nextest, pnpm) in `.github/actions/setup`, bundle
steps in `.github/actions/bundle`.

Linux does every check that doesn't depend on the OS, split into parallel jobs. macOS and Windows run only what they
can break on their own: clippy (for `cfg` code) and the Rust tests (paths, processes, PTYs).

| Check | Runs on | Does | Pull requests | Merge queue | main |
|---|---|---|---|---|---|
| lint (linux) | Namespace `linux-8` | fmt, clippy (pdtd also with no integration features) | unless docs only | unless docs only | never |
| test (linux) | Namespace `linux-8` | nextest (pdtd also with no integration features), generated TS types are committed | unless docs only | unless docs only | never |
| web (linux) | Namespace `linux-8` | web typecheck, vitest, build | unless docs only | unless docs only | never |
| build (macos-latest) | Namespace `macos-6` | clippy, nextest, desktop crate clippy + test | `macos` label, or a change under `.github/` | unless docs only | never |
| build (windows-latest) | GitHub Windows | clippy, nextest | `windows` label, or a change under `windows/`, `*.ps1`, `*.cmd`, `.github/` | same | always |
| bundle (macos-latest, app) | GitHub macOS | release app, signed | `bundle` label, or a change under `desktop/`, `scripts/`, `.github/` | same | always (release) |
| bundle (windows-latest, nsis) | GitHub Windows | installer (also the only Windows compile of the desktop crate) | same | same | always (release) |

**Why main skips the Linux and macOS checks:** the merge queue (group size 1, squash) has just run them on the tree
that lands on main, so running them again on the push only cost money. Windows checks still run on main, the one
place they always run. `release` uses `always()` and refuses only failed or cancelled needs, since the skipped checks
would otherwise skip it. A push that bypasses the queue (an admin bypass) is not checked on main.

**Why macOS waits for the queue:** Namespace bills macOS at 10× Linux per vCPU, so `macos-6` costs 60 units a minute
and `linux-8` 8. In October 2026, when it ran on every PR push, in the queue and on main, it was 80% of a ~$600/month
projection. Now it runs once per merge. A macOS-only break shows up when the PR enters the queue (the queue drops the
PR, which never reaches main); add the `macos` label to a PR that touches macOS-specific code to see it earlier.

### Runners

- **Namespace** (namespace.so; its GitHub App is installed on this repo) runs the Linux and macOS checks: runner
  profiles `namespace-profile-linux-8` (8 vCPU x64, Ubuntu) and `namespace-profile-macos-6` (M4 Pro), each with a
  cache volume. A cache volume keeps `target/`, the cargo registry and the pnpm store between runs, like the Mac mini's
  warm build dir did, so most runs rebuild only what changed. A job saves its changes only when it exits cleanly, and
  the saved copy reaches other machines in the background. Paid per unit-minute: vCPUs × minutes × a platform
  multiplier (Linux 1, macOS 10), $0.0015 a unit on pay-as-you-go; the Team plan ($100/month) includes 100,000.
- Namespace's Linux image is not GitHub's: no `SHELL` and no zsh. pdtd's terminal tests start `/bin/zsh` when no
  shell is given, so `setup` with `zsh: true` installs it (with an empty `~/.zshrc` and `skip_global_compinit=1`).
- **GitHub** runs Windows (checks and installer), the macOS release bundle (signing) and the small `changes` and
  `release` jobs; the standard runners are covered by the plan's included minutes.
- Repo variables `PDT_LINUX_RUNNER` / `PDT_MACOS_RUNNER` override the Namespace runners, e.g. `ubuntu-latest` /
  `macos-latest` if Namespace is down or out of credit (`gh variable set PDT_LINUX_RUNNER --body ubuntu-latest`, and
  `gh variable delete` to go back). `setup` notices the runner (`nsc-runner-*` names) and uses rust-cache and
  setup-node's pnpm cache on GitHub's runners instead.
- Comparing hosts: `.github/workflows/bench.yml` times the same work on any list of runners.

Every job in the table is a required check of the "Main" ruleset.

**Docs-only PRs** (every changed file under `docs/` or ending in `.md`) skip every job but `changes`; a `windows`,
`macos` or `bundle` label still runs those jobs. The guides pdtd builds in with `include_str!` (`docs/agent-guide.md`,
`docs/run-config.md`, `docs/prompts/*`; crates/pdt-api/src/guides.rs) count as code, since their tests check them.
If other code starts reading a Markdown file, add it to that rule in the `changes` job.

A job skipped by its `if` still reports its check, and a skipped check counts as passing, so the job names must stay
the ruleset's required check names. Renaming a job means editing the "Main" ruleset first.

Why: on GitHub's 2-vCPU runners (this internal repo's size) vitest alone took ~3 min on Linux and 6m37s on Windows,
and the web code is the same on every OS. Namespace ran the Linux checks in 1.8 min (vs 6.4) and macOS in 1.4 (vs
2.7–3.9). CI no longer runs on the Mac mini, which also runs the agents and the stable PDT
(docs/superpowers/notes/runners.md). A Windows-only break is caught on the push to main that merges it instead of
on the PR (and that push publishes no release); add the `windows` label to a PR that touches platform-specific code
(`cfg(windows)`, process spawning, paths).

## Merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group) instead of requiring branches to be up
to date with main. A PR joins the queue when its checks pass and auto-merge is on; GitHub then makes
`gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it with the `merge_group` event. The
`changes` job picks jobs for it as for a PR: the files come from comparing the group's base and head (more than 300
files runs everything), the labels from PR `<n>` in the queue branch's name, which is why the group size stays 1.
The required checks report on the queue run the same way they do on a PR (skipped counts as passing), and the PR
merges when they pass. A PR that conflicts with main or fails there leaves the queue.

So each merge runs the Linux checks on every PR push and in the queue, macOS in the queue, and on main only Windows
and the release. The queue runs only once a PR is ready.

## The old Mac mini runner (retired)
CI ran `build (macos-latest)` on a self-hosted runner on the Mac mini until PDT-27; no job targets it now and its
folders (`~/actions-runner-pdt`, `~/.cache/pdt-ci`) are gone. If it is still listed under repo Settings → Actions →
Runners, remove it there; while it is registered, never make the repo public.
