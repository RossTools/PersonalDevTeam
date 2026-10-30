# CI layout

Workflow: `.github/workflows/ci.yml`; shared setup (toolchain, cache, nextest, pnpm) in `.github/actions/setup`, bundle
steps in `.github/actions/bundle`.

Linux does every check that doesn't depend on the OS, split into parallel jobs. macOS and Windows run only what they
can break on their own: clippy (for `cfg` code) and the Rust tests (paths, processes, PTYs).

| Check | Runs on | Does | Pull requests | Merge queue | Run by hand |
|---|---|---|---|---|---|
| lint (linux) | Namespace `linux-8` | version sync (`scripts/version.sh check`), fmt, clippy (pdtd also with no integration features) | unless docs only | unless docs only | always |
| test (linux) | Namespace `linux-8` | nextest at the stand-in version 9.9.9 (pdtd also with no integration features), generated TS types are committed | unless docs only | unless docs only | always |
| web (linux) | Namespace `linux-8` | web typecheck, vitest, build | unless docs only | unless docs only | always |
| build (macos-latest) | Namespace `macos-6` | clippy, nextest, desktop crate clippy + test | `macos` label, or a change under `.github/` | unless docs only | always |
| build (windows-latest) | GitHub Windows | clippy, nextest | `windows` label, or a change under `windows/`, `*.ps1`, `*.cmd`, `.github/` | same | always |
| bundle (macos-latest, app) | GitHub macOS | the app, signed when the secret is there (on the release PR: signed and notarized); a workflow artifact | the release PR only | never | with "Build bundles" |
| bundle (windows-latest, nsis) | GitHub Windows | the installer (also the only Windows compile of the desktop crate); a workflow artifact | the release PR only | never | with "Build bundles" |
| site | GitHub `ubuntu-latest` | the website (site/): link-rewriting tests, `astro build` | a change under `site/` or `docs/`, the pnpm lockfile or workspace, `ci.yml` or `pages.yml` | same | always |
| bundle (linux, x86_64) / bundle (linux, aarch64) | GitHub `ubuntu-22.04` / `ubuntu-22.04-arm` (native, no cross-compiling) | the headless tarball and the AppImage (also the only Linux compile of the desktop crate); a workflow artifact | the release PR only | never | with "Build bundles" |

**Tests never depend on the checked-in version** (PDT-216): `test (linux)` first sets the version to 9.9.9
(`scripts/version.sh set 9.9.9`, in its own checkout only), so a test that hard-codes `0.x.y` fails on its own PR rather
than first on the release PR that bumps it. Tests compare with `pdt_core::BUILD_VERSION` or `env!("CARGO_PKG_VERSION")`,
or pass their own version (e.g. `UpdateService::new(…, "0.1.0-local")`). The release PR's `test (linux)` runs at its real
version, and macOS and Windows always do, so no extra run is needed. The web tests never see the version (it reaches
the UI only through `/api/health`, which they stub).

**Main runs nothing** (PDT-192): `ci.yml` has no `push` trigger. The merge queue (group size 1, squash) has just run
the checks on the tree that lands on main, and releases are deliberate, so a merge builds no bundle and publishes no
release (docs/SDLC.md "Releases"). A push that bypasses the queue (an admin bypass) is not checked on main. Before
PDT-192 every push to main ran the Windows checks and the bundles and published `v0.1.<run number>`.

**Bundles by hand:** `gh workflow run ci --ref <branch>` (or Actions → ci → Run workflow, "Build bundles" ticked) runs
every check and builds the macOS app, the Windows installer and the Linux tarball and AppImage (both CPUs) at the
checked-in version, reporting `<version>-local`. They are uploaded as workflow artifacts (`bundle-app`, `bundle-nsis`,
`bundle-linux-x86_64`, `bundle-linux-aarch64`, kept 5 days) and published
nowhere: use it to try a packaging change (`desktop/`, `scripts/`, `.github/actions/bundle`) before a release. The
bundle action's `release: "true"` input is for releases: it reports the version as is, requires the signing
certificate and notarizes.

**Releases** (PDT-194; docs/SDLC.md "Releasing"): `.github/workflows/release.yml`. Run by hand on main, it opens the
release PR from the `release` branch (version bumped, CHANGELOG.md compiled) and turns on its auto-merge with
`PDT_RELEASE_TOKEN` (PDT-214), so it merges by itself once its checks pass. `changes` notices that PR (head branch
`release` in this repository) and runs the bundle jobs on it as release builds (signed and notarized, the version as
is), so a packaging break blocks the release PR rather than the tag; the merge queue doesn't build them again. When
the release PR merges, release.yml's `tag` job tags the merge commit and the same workflow builds the bundles from
it (its own jobs, not ci.yml's checks) and publishes the release. The tag is made with the workflow's token, so it
starts no other workflow. The PR artifacts are not reused: the queue squash-merges, so the tagged commit is never
the commit they were built from. On its 08:00 UTC schedule, or run with `channel: nightly` (PDT-283), its `nightly` job
instead tags the head of main `vX.Y.Z-nightly.YYYYMMDD` (the schedule skips a head that is released already), and
the same bundle jobs build that commit with the version set in their checkout (the bundle action's `version` input;
nothing is committed) for `publish` to make a GitHub pre-release and delete the nightlies beyond the newest 14. Each
job has its own concurrency group, so stable and nightly runs never cancel or hold up each other.

**Windows cache:** GitHub's runners save rust-cache only from main, which no longer runs CI, so Windows check runs
start from a cold cache unless a dispatch on main warms it.

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
- **GitHub** runs Windows (checks and installer), the macOS bundle (signing) and the small `changes` and `changelog`
  jobs; the standard runners are covered by the plan's included minutes.
- Repo variables `PDT_LINUX_RUNNER` / `PDT_MACOS_RUNNER` override the Namespace runners, e.g. `ubuntu-latest` /
  `macos-latest` if Namespace is down or out of credit (`gh variable set PDT_LINUX_RUNNER --body ubuntu-latest`, and
  `gh variable delete` to go back). `setup` notices the runner (`nsc-runner-*` names) and uses rust-cache and
  setup-node's pnpm cache on GitHub's runners instead.
- Comparing hosts: `.github/workflows/bench.yml` times the same work on any list of runners.

Every job in the table but `site` is a required check of the "Main" ruleset. `site` is new (PDT-198): add it to the
ruleset once it has run on main, so a PR that breaks the website can't merge.

**The website** (site/README.md): `.github/workflows/pages.yml` builds `site/` and deploys it to GitHub Pages on every
push to main that touches `site/**` or `docs/**`, and by hand. It is the one workflow a merge to main runs; it builds
and ships nothing of the app.

**changelog** (GitHub `ubuntu-latest`, pull requests only, not docs-only ones): the PR adds a fragment under
`.changes/unreleased/` or has the `no-changelog` label; every unreleased fragment is valid; the compile script's
tests pass (docs/SDLC.md "Changelog"). The merge queue and main skip it. Make it a required check of the "Main"
ruleset (Settings → Rules) so a PR without a fragment can't merge.

**Docs-only PRs** (every changed file under `docs/` or `site/`, or ending in `.md`) skip every job but `changes` and
`site` (which runs when `site/` or `docs/` changed); a `windows`,
`macos` label still runs those jobs. The guides pdtd builds in with `include_str!` (`docs/agent-guide.md`,
`docs/run-config.md`, `docs/prompts/*`; crates/pdt-api/src/guides.rs) count as code, since their tests check them.
If other code starts reading a Markdown file, add it to that rule in the `changes` job.

A job skipped by its `if` still reports its check, and a skipped check counts as passing, so the job names must stay
the ruleset's required check names. Renaming a job means editing the "Main" ruleset first.

Why: on GitHub's 2-vCPU runners (this internal repo's size) vitest alone took ~3 min on Linux and 6m37s on Windows,
and the web code is the same on every OS. Namespace ran the Linux checks in 1.8 min (vs 6.4) and macOS in 1.4 (vs
2.7–3.9). CI no longer runs on the Mac mini, which also runs the agents and the stable PDT
(docs/superpowers/notes/runners.md). A Windows-only break in a PR without the label and without Windows-specific
files is not caught before it merges (main runs nothing); add the `windows` label to a PR that touches platform-specific code
(`cfg(windows)`, process spawning, paths).

## Merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group) instead of requiring branches to be up
to date with main. A PR joins the queue when its checks pass and auto-merge is on; GitHub then makes
`gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it with the `merge_group` event. The
`changes` job picks jobs for it as for a PR: the files come from comparing the group's base and head (more than 300
files runs everything), the labels from PR `<n>` in the queue branch's name, which is why the group size stays 1.
The required checks report on the queue run the same way they do on a PR (skipped counts as passing), and the PR
merges when they pass. A PR that conflicts with main or fails there leaves the queue.

So each merge runs the Linux checks on every PR push and in the queue, macOS in the queue, and nothing on main. The queue runs only once a PR is ready.

## The old Mac mini runner (retired)
CI ran `build (macos-latest)` on a self-hosted runner on the Mac mini until PDT-27; no job targets it now and its
folders (`~/actions-runner-pdt`, `~/.cache/pdt-ci`) are gone. If it is still listed under repo Settings → Actions →
Runners, remove it there; while it is registered, never make the repo public.
