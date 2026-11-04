# How we work on PDT

PDT is built inside PDT: the maintainer's agent sessions run on their stable instance (port 7420), each change lives
in its own worktree, and the maintainer tests it in a dev instance of PDT started with **Run**. This page is the
process: branches, checks, pull requests, CI, the merge queue, the changelog and releases. How the code works is in
the developer guide ([architecture.md](dev/architecture.md), [invariants.md](dev/invariants.md),
[testing.md](dev/testing.md)).

**Contributing from outside** (no maintainer PDT, no `PDT` tickets): [CONTRIBUTING.md](../CONTRIBUTING.md) is your
version of this page. The parts below that use the stable PDT on port 7420, its tickets, `gh pr merge`, labels,
releases or the stable data folder are for maintainers; use a plain branch (or `git worktree add`) instead, and name
your changelog fragment after your issue or PR (`GH-45.md`).

## The loop

1. **Pick the work** from the project's tickets in the stable PDT (Tickets → Backlog / Board; `GET /api/tickets`).
   Designs for larger work go on the ticket (its description or comments), never in the repository.
2. **Make a worktree** for a new branch (below). Never commit on `main` in the main checkout.
3. **Build it test-first** (TDD for core logic), add a changelog fragment, and run the checks (below).
4. **Push and open a PR** against `main`. CI runs (below, "CI").
5. **The user tests it**: Run → `pdt` on the branch's worktree (below, "Testing a branch"). Feedback comes back in the
   session; fix the branch and restart the run.
6. **Merge when the user says so**: `gh pr merge <n> --auto`, with no merge-method flag (the queue sets it; `--squash`
   only prints "The merge strategy for main is set by the merge queue") and no `--delete-branch` (it can't delete a
   branch whose worktree exists). Once the PR's checks pass it joins main's merge queue (below), which squash-merges
   it: the PR title becomes the commit subject on `main`, so write it as one. Nobody watches CI in a shell, and the
   branch never needs updating with main.
7. **Merging ships nothing**: a merge to `main` runs no CI and publishes no release (below, "Releases").
8. **Clean up** (below, "After merge") and add tickets for follow-ups. Work that takes several PRs is a parent ticket
   with one part per PR; each part has its own branch, worktree and PR, and **Start next part** on the parent starts
   the next.

## Worktrees

Worktrees live at `.worktrees/<branch>` inside the repo (`/` in a branch name becomes `-`); `/.worktrees/` is in
`.git/info/exclude`. Ticket Start names branches `pdt-<KEY>-<n>-<slug>` (no `/`: some tools break on it); other
branches are short and kebab-case.

Maintainers create them **through the stable PDT**, so its sessions group under the branch and Run uses that
checkout (without a PDT, `git worktree add .worktrees/<branch> -b <branch> origin/main` or just a branch):

```sh
git -C <repo> fetch origin && git -C <repo> merge --ff-only origin/main   # keep the main checkout clean
curl -s 127.0.0.1:7420/api/projects                                        # the project id (repoPath = <repo>)
curl -s -X POST 127.0.0.1:7420/api/projects/<project id>/worktrees \
  -H 'content-type: application/json' -d '{"branch":"<branch>"}'           # optional "base": branch or commit
```

The branch starts at the project's base branch (`main`); `cd` into the returned `path`. A fresh worktree needs
`pnpm install --frozen-lockfile` once, and its first cargo build is slow (empty `target/`). A folder made with plain
`git worktree add` is unknown to PDT, and Run on that branch fails with `worktree_path_taken`: remove it and recreate
it as above, or start a dev instance by hand.

## Checks before pushing

`scripts/check.sh` runs what CI runs, cheap checks first, and stops at the first failure:

```sh
scripts/check.sh               # everything: changelog and version, the docs check and website, lint, web, cargo test
scripts/check.sh --docs        # a docs-only change: the changelog, the docs check, the website's tests and build
scripts/check.sh --no-rust     # skip fmt, clippy and cargo test; --no-web and --no-site skip those groups
scripts/check.sh --only lint   # just some groups (changelog, site, lint, web, test); CI's lint job does this
```

- `cargo test` regenerates `web/src/api/gen` and the website's plugin list (`site/src/data`): commit them, or the
  check fails as CI's would.
- When `desktop/` changed, also run `cargo clippy -p pdt-desktop --all-targets -- -D warnings` and
  `cargo test -p pdt-desktop` (after `pnpm --dir web build`).
- The docs check fails on internal references in the user docs, and on any file under `docs/` in a `superpowers`,
  `specs` or `plans` folder (AGENTS.md "Writing docs").

**Watching CI** is usually unnecessary (auto-merge waits for it). When you need a result, watch in the background,
and use the Actions run: the `gh` token is fine-grained, which can't read check runs, so `gh pr checks` fails.

```sh
gh run list --branch <branch> --limit 1 --json databaseId,status,headSha   # the run for the latest push
gh run watch <run id> --interval 30 --exit-status                          # exits non-zero if CI failed
```

A new push cancels the old run and starts a new one, so look it up again after each push.

## Changelog

Release notes are written for people who use PDT, by whoever makes the change: every PR adds a **fragment**,
`.changes/unreleased/<TICKET>.md` (more for the same ticket: `<TICKET>-<anything>.md`):

```markdown
---
kind: fixed
---
The board no longer jumps back to the top when a ticket moves.
```

- `kind` is `added`, `changed`, `fixed`, `breaking` or `internal` (left out of the release notes: refactors,
  tooling). The text is one or two sentences (or a short list) on what changed for the user, with no tickets, crates
  or file names unless the user sees them.
- `scripts/changelog.sh check` checks the fragments. CI's `changelog` job fails a PR without one unless it has the
  `no-changelog` label (for PRs that only change docs, CI or tests; adding it re-runs CI) or says `[no-changelog]`
  in its description. The description is read when a push or a label starts the run, so after writing it there, push
  again (or re-run the job). It is for contributors from a fork, who can't set labels.
- At release, `scripts/changelog.sh <version>` adds a `## <version> (<date>)` section to `CHANGELOG.md` (Breaking,
  Added, Changed, Fixed), prints the notes, and moves the fragments to `.changes/<version>/`.
  `scripts/changelog.sh notes <version>` prints a version's section (the release's notes); `preview` prints the
  unreleased ones.

## CI

`.github/workflows/ci.yml`; the Rust checks per OS in `.github/workflows/rust.yml`; shared setup (toolchain, cache,
nextest, pnpm, Windows Defender exclusions) in `.github/actions/setup`, bundle steps in `.github/actions/bundle` and
`.github/actions/bundle-macos` (which joins the two macOS builds into the universal app).
Linux does every check that doesn't depend on the OS, in parallel jobs; macOS and Windows run only what they can break
on their own (clippy for `cfg` code, the Rust tests). Every job runs on GitHub's hosted runners unless a repo variable
names another (below, "Runners").

| Check | Does | Pull requests | Merge queue |
|---|---|---|---|
| lint (linux) | version sync, fmt, clippy (pdtd also without integration features) | a Rust change | unless docs only |
| test (linux) | nextest at the stand-in version 9.9.9 (pdtd also without integration features); generated files are committed | a Rust change | unless docs only |
| web (linux) | web typecheck, knip and build; vitest in 4 shards (`PDT_WEB_SHARDS`) | a web change | unless docs only |
| build (macos-latest) | clippy, nextest, desktop crate clippy and test | `macos` label, or a `.github/` change | unless docs only |
| build (windows-latest) | clippy, nextest | `windows` label, or a Rust change to `windows/`, `*.ps1`, `*.cmd`, `.github/` | unless docs only |
| site | the install scripts' lint, the website's tests (the docs check included) and build | a change to `site/`, `docs/`, the pnpm lockfile or workspace, `ci.yml` or `pages.yml` | same |
| secrets | gitleaks over the commits the PR (or the queued group) adds | always | always |
| changelog | a fragment or the `no-changelog` label; the scripts' tests | unless docs only | skipped |
| bundle (…) | the macOS app (each CPU built in its own job, `bundle (macos, <target>)`, then joined into the universal app, signed and checked with `lipo -archs` in `bundle (macos-latest, app)`), the Windows installer, the Linux tarball and AppImage, as workflow artifacts (GitHub's runners always: signing) | the release PR only | never |

- **Docs-only PRs** (every changed file under `docs/` or `site/`, or a `*.md`) run only `changes`, `secrets` and
  `site`. The
  guides pdtd builds in with `include_str!` (`docs/agent-guide.md`, `docs/run-config.md`, `docs/prompts/`,
  `docs/user/automation/run-configuration.md`; `crates/pdt-api/src/guides.rs`) count as code, since Rust tests check
  them, and so do the install scripts in `site/public`. If other code starts reading a file there, add it to
  `builtin` in the `changes` job.
- **Per job on a PR**: a *Rust change* is any code change outside `web/`; a *web change* touches `web/`,
  `package.json`, the pnpm lockfile or workspace, `scripts/check.sh` or `.github/`. A PR that changes only `web/`
  skips the Rust jobs, and one that leaves those alone skips the web jobs. The merge queue runs both whatever
  changed (a few Rust tests read `web/`, such as the brand check).
- **Tests never depend on the checked-in version**: `test (linux)` sets it to 9.9.9 in its own checkout, so a test
  that hard-codes `0.x.y` fails on its own PR. Compare with `pdt_core::BUILD_VERSION` or `env!("CARGO_PKG_VERSION")`.
- **Main runs no checks**: `ci.yml` has no `push` trigger (the queue has just checked that tree). On main,
  `pages.yml` redeploys the website when `site/**` or `docs/**` changed, and `cache.yml` rebuilds the build caches
  when the dependencies change (below, "Caching").
- **macOS waits for the queue** because macOS minutes cost the most (on Namespace, 10× Linux); a macOS-only break
  shows when the PR enters the queue. Label a PR `macos` to see it earlier.
- **Windows waits for the queue** like macOS: a Windows-only break in a PR without the `windows` label or Windows files
  shows when the PR enters the queue. Label any path, process or file-system change `windows` to see it earlier.
- **Bundles by hand**: `gh workflow run ci --ref <branch>` (or Actions → ci → Run workflow, "Build bundles") runs
  every check and builds every bundle at `<version>-local`, kept as artifacts for 5 days and published nowhere: use
  it to try a packaging change.
- **The secret scan** (`secrets`) runs gitleaks' default rules over the PR's new commits, with the config in
  `.gitleaks.toml`. Run it locally with `gitleaks git --log-opts=origin/main..HEAD .` (`brew install gitleaks`). A
  finding that is no secret (a recorded id in a test fixture) goes in `.gitleaks.toml` as an `[[allowlists]]` entry
  with `targetRules` (the finding's rule), `paths` and a `regexes` match for the value, `condition = "AND"`: without
  `targetRules`, a `paths` entry skips the whole file for every rule. A real secret that reached a commit is leaked
  even once removed: revoke it, then rewrite the branch.
- **Job names are the "Main" ruleset's required checks.** A job skipped by its `if` still reports, and a skipped check
  passes. Renaming a job means editing the ruleset first. `test (linux)`, `build (macos-latest)` and
  `build (windows-latest)` only report the result of their OS's `rust (…)` jobs (rust.yml); they fail if those or
  `changes` failed.

### Build once, test in partitions

For each OS, `rust.yml` runs:

- **build**: compiles every test binary once into a nextest archive (`cargo nextest archive`), uploaded as an artifact
  for a day, with cargo's `--timings` report (`cargo-timings-<os>-<tests>`, a week) showing what compiled when. On
  Linux and Windows (`split`) two build jobs run side by side, **build (unit)** (`--lib --bins`) and **build
  (integration)** (`--test '*'`), an archive each: the build is CPU-bound on a 4-CPU runner, so halving each job's
  work shortens it by minutes. Both select the same packages, so the dependencies build with the same features and
  come from the same cache; only the workspace's shared crates (pdt-core up to pdt-api) compile in both. macOS
  builds once (`build (all)`): its minutes cost the most.
- **minimal** (Linux): pdtd's tests without its integration features, a build of its own beside the others.
- **test (1…N)**: each downloads the archives, extracts them into its checkout (so `target/` is where the build had
  it and `env!("CARGO_BIN_EXE_…")` paths hold) and runs one slice of each archive's tests (`--partition slice:i/N`),
  the second archive even when the first fails. They need no Rust toolchain. On Linux each checks the generated files
  its tests wrote.
- **clippy** (macOS and Windows; Linux lints in `lint (linux)`): beside the build, so it adds no wall time; on macOS
  also the desktop crate's clippy and tests. Windows keeps full clippy: its `cfg(windows)` code is in most crates,
  and a separate job costs no waiting.

N is 3 on Linux, 8 on Windows and 2 on macOS; the repo variables `PDT_LINUX_PARTITIONS`, `PDT_MACOS_PARTITIONS` and
`PDT_WINDOWS_PARTITIONS` change it, as a JSON list (`gh variable set PDT_WINDOWS_PARTITIONS --body '[1,2,3,4,5]'`).

The builds are lean: no debug info (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_DEV_STRIP=debuginfo`), no
incremental state (`CARGO_INCREMENTAL=0`), and Windows links with `rust-lld` instead of MSVC's `link.exe`. Linux
already links with Rust's bundled lld. The test builds compile the dependencies (not the workspace's crates) at
`opt-level=1` (`DEPS_CONFIG` in `rust.yml`, part of the cache key): the tests are CPU-bound and take about a quarter
less CPU. Local builds keep `opt-level=0`.

### Caching

- **Only main saves caches; every other run only restores.** PRs and queue runs can read only their own branch's
  caches and main's, so a save anywhere else would be read by no one and fill the repo's 10 GB.
- The bundles (`.github/actions/bundle`) keep their own Rust caches, one per bundle and CPU (`bundle-nsis`,
  `bundle-linux` on each Linux CPU, `bundle-app-aarch64-apple-darwin`, `bundle-app-x86_64-apple-darwin`): release
  builds share nothing with the checks' debug builds. Nightly releases save them (they run on main), along with the
  pnpm store of each runner's OS and CPU; the release PR and a stable release's tag run (in a pull request's scope,
  where a save would reach no one) restore the newest nightly's. `release.yml` sets the same `CARGO_*` variables as
  `ci.yml`, because they are part of the keys.
- On GitHub's runners `setup` restores `Swatinem/rust-cache`, one cache per OS and build (`linux-lint`,
  `linux-build`, `linux-minimal`, `macos-clippy`, `macos-build`, `windows-clippy`, `windows-build`). Each holds the cargo registry and
  the dependencies' build output (no debug info, not the workspace's crates), keyed on `Cargo.lock`, the manifests,
  the toolchain and the OS. When the key misses it restores the newest cache for the same OS and build, and rebuilds
  only what changed.
- The pnpm store (`.github/actions/pnpm`, used by every job that needs Node) is cached per OS, keyed on
  `pnpm-lock.yaml`, saved on main only.
- `cache.yml` saves them: it runs the same builds on main when `Cargo.lock`, a `Cargo.toml`, `pnpm-lock.yaml`, the
  toolchain or the CI files change, weekly (GitHub drops caches unused for 7 days), and by hand (Actions → cache →
  Run workflow). Then `.github/scripts/prune-caches.sh` keeps only main's newest cache of each kind and deletes the
  rest; each nightly runs it too, after saving the bundles' caches. Past 10 GB GitHub evicts the least recently used
  caches, which are the bundles' (read once a night), so pruning is what keeps them. Run it with `--dry-run` to see
  what it would delete.
- sccache isn't used: rust-cache with keys per OS and build already restores every dependency. Only the workspace's
  own crates compile on each run, and a shared compiler cache would need storage outside GitHub's 10 GB (a bucket such
  as Cloudflare R2, with its credentials as secrets); it would mostly speed up the bundles' release builds of the
  workspace's crates.
- Namespace runners keep their cache volume instead (`target/`, the cargo registry, the pnpm store, saved when a job
  exits cleanly); `cache.yml` skips an OS whose runner variable names a Namespace profile.
- Windows: `setup` excludes the checkout, the temp folders and the cargo and rustup homes from Defender's scanning,
  which otherwise slows the build and the tests several times over.

### The merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group). A PR joins it once its checks pass and
auto-merge is on; GitHub makes `gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it
(`merge_group`). The `changes` job picks jobs as for a PR: files from the group's base and head (over 300 runs
everything), labels from PR `<n>` in the branch name, which is why the group size stays 1. The PR merges when the
required checks pass; one that conflicts with main or fails leaves the queue: fix the branch (merge main in for a
conflict), push, and merge again.

### Runners

- **GitHub's hosted runners** (`ubuntu-latest`, `macos-latest`, `windows-latest`) run every job by default: free and
  many at once for a public repo.
- Repo variables `PDT_LINUX_RUNNER`, `PDT_MACOS_RUNNER` and `PDT_WINDOWS_RUNNER` switch an OS's checks (lint, web,
  the Rust jobs) to another runner with no code change: a Namespace profile (`gh variable set PDT_LINUX_RUNNER --body
  namespace-profile-linux-8`, `PDT_MACOS_RUNNER` `namespace-profile-macos-6`) or a larger GitHub runner. `gh variable
  delete` goes back to GitHub's. `setup` picks the cache to match (a runner named `nsc-runner-*` is Namespace's).
- **Namespace** (its GitHub App is installed on the repo): each profile has a cache volume. Its Linux image has no
  `SHELL` and no zsh, so `setup` with `zsh: true` installs zsh for pdtd's terminal tests.
- The bundles and the small `changes`, `changelog`, `site` and reporting jobs always run on GitHub's runners.
- **A pull request from a fork** runs every check on GitHub's hosted runners whatever the variables say (the
  `changes` job picks each OS's runner): untrusted code never spends Namespace minutes or writes to a cache volume
  later runs restore. In the merge queue it runs on the configured runners, after a maintainer has reviewed and
  queued it.
- `.github/workflows/bench.yml` (by hand only) times the same work on a list of runners, to compare hosts.
- The Mac mini's old self-hosted runner is retired; if it is still listed under Settings → Actions → Runners, remove
  it, and never make the repo public while it is registered.

### Outside pull requests

- **Approval**: Settings → Actions → General → "Approval for running fork pull request workflows from
  contributors" must be **Require approval for all external contributors**, so no fork's PR runs CI until a
  maintainer approves that run (GitHub's default asks only first-time contributors).
- **Secrets**: no workflow uses `pull_request_target`, and every workflow's token is `contents: read` unless a job
  widens it. Fork PRs get no secrets; the bundles, the only jobs that sign, run only for this repository's `release`
  branch.
- **Labels**: a contributor can't set `no-changelog`, `windows` or `macos`. They write `[no-changelog]` in the PR's
  description instead (above, "Changelog"); a maintainer adds `windows` or `macos` when the change needs those
  checks.
- **On a contributor's fork**, `pages.yml`, `cache.yml` and `audit.yml`'s daily run skip
  (`github.event.repository.fork`); `ci.yml` checks their PRs here; `release.yml` stops with a message when
  `PDT_RELEASE_TOKEN` is missing.

## Dependabot

`.github/dependabot.yml` checks the Cargo workspace, the pnpm workspace (`web`, `desktop`, `site`) and GitHub Actions
every Monday: one grouped PR of minor and patch updates per ecosystem, and one PR per major update, at most five open
each. `rust-toolchain.toml` is bumped by hand.

- Its PRs carry the `dependencies` and `no-changelog` labels and `deps:` commit messages.
- Third-party actions (anything but GitHub's `actions/*` and the local `./.github/actions`) are pinned to a full
  commit SHA with the tag in a comment, `uses: owner/repo@<sha> # v1.2.3`, so a moved tag can't change what CI or a
  release runs. Dependabot bumps the SHA and the comment together. `scripts/check-action-pins.sh` (in `check.sh`'s
  changelog group and CI's changelog job) fails on one pinned to a tag. To add or bump one by hand, take the commit
  the release tag points to: `git ls-remote --tags https://github.com/<owner>/<repo> 'refs/tags/<tag>*'` (the
  `^{}` line for an annotated tag).
- They are outside PRs: PDT's external-PR workflow has an agent review each, and the user approves and merges it
  through the queue. A major update that needs code changes is fixed on its branch or in a ticket of its own.

### Advisories

`audit.yml` checks the dependencies against published advisories: `cargo deny check advisories` over `Cargo.lock`
(vulnerable, unsound, unmaintained and yanked crates) and `pnpm audit --audit-level high` over `pnpm-lock.yaml`. It
runs daily on `main`, by hand, and on a PR that changes a lockfile or `deny.toml`. It is not a required check: a new
advisory is about the dependencies, not the PR that meets it.

- Run it locally with `cargo deny check advisories` (`cargo install --locked cargo-deny`) and
  `pnpm audit --audit-level high`.
- A failed daily run is new work: update the crate (`cargo update -p <crate>`) or package, or, when there is no fix
  (an unmaintained crate deep in tauri's or keyring's Linux stack), add its id to `[advisories] ignore` in
  `deny.toml` with the reason.

## Releases

Releases are deliberate: merges don't ship. Stable releases happen when the user asks; nightlies every night.

- **Versions are SemVer.** The source of truth is the workspace `version` in `Cargo.toml`, repeated in
  `desktop/src-tauri/tauri.conf.json`; `scripts/version.sh check` fails when they differ. Only a release PR changes
  them.
- **Builds**: a release build reports its version as is; every other build (local, Run, bundles by hand) reports
  `<version>-local`, which never counts as newer than a release.
- **Updates** never install an older version. The stable channel (the default) takes GitHub's latest release; the
  nightly channel also takes nightly pre-releases, whichever is newer. The update banner shows the notes of every
  release in between.

### Releasing

0. **Screenshots** (when the UI changed since the last release): `node scripts/site-screenshots.mjs` (site/README.md
   "Screenshots"), merged before starting.
1. **Start it** with PDT's "Cut a release" workflow (below), or Actions → release → Run workflow on `main` with a
   bump (`patch`, `minor`; `major` from 1.0.0 on) or an exact version. Agents start it only through Cut a release.
   Starting it is the release: nobody reviews the release PR. To change the notes, edit the fragments on `main`
   first. It refuses while another release PR is open.
2. **The release PR**: `.github/workflows/release.yml` sets the version (`Cargo.toml`, `Cargo.lock`,
   `tauri.conf.json`), compiles the fragments into `CHANGELOG.md`, and opens `Release vX.Y.Z` from the `release`
   branch with the notes as its body. CI runs, the bundle jobs build signed and notarized release bundles (so
   packaging breaks show before a tag), and the workflow turns on auto-merge with `PDT_RELEASE_TOKEN`, so it merges
   itself once its checks pass. A failed check leaves it open: push a fix to `release` (and turn auto-merge on again
   once the queue dropped it), or close it, which releases nothing.
3. **Tag and publish**: on merge, the workflow tags `vX.Y.Z` on the merge commit, builds the bundles again from it (the
   queue squashed the PR's commit away), and publishes the GitHub release with `SHA256SUMS` (signed: "Release
   signing") and the version's `CHANGELOG.md` section. Then `check-macos-download` downloads the published
   `.app.tar.gz` on macOS and fails unless Gatekeeper accepts it (`spctl -a -t exec`, a stapled ticket). Re-running a
   failed job reuses the tag and release and replaces re-uploaded assets.

Setup: the `PDT_RELEASE_TOKEN` secret (fine-grained: Contents and Pull requests, read and write) and auto-merge
allowed in the repository's settings; without the token the workflow stops before opening anything. **Dry run**: on
a fork with the same secrets, set the repo variable `PDT_RELEASE_TAG_PREFIX` (`test-v`): tags become
`test-vX.Y.Z` and releases pre-releases, which no PDT installs.

**Cut a release** (this project's copy of PDT's Cut a release template, Settings → Workflows) takes
`{"type": "stable", "bump": "minor"}`, `{"type": "stable", "version": "1.0.0"}` or `{"type": "prerelease"}` (a
nightly now); Run now asks the same in a form. This repository's copy is filled in with: release workflow
`release.yml` on `main`, stable inputs `{"bump": "{{trigger.bump}}", "version": "{{trigger.version}}"}`, release PR
branch `release`, no stable tag pattern, other release `Nightly` with inputs `{"channel": "nightly"}` and tag pattern
`v*-nightly.*`. It tells the user when the release PR is open, puts a failed check in the Inbox, and says when the
release is published; while an earlier release PR is open it starts nothing and says so. While it waits for the
release it follows the release run (the `pull_request` run on `release` at the PR's head, or the nightly's dispatched
run on `main`) and puts a failed or cancelled one in the Inbox with its failed jobs; re-running the failed jobs reuses
the tag, and it still ends with "published".

### Nightly releases

A nightly puts the head of `main` on hosts that opted in, to try a change on real machines before it ships to
everyone.

- **When**: every night a scheduler outside GitHub dispatches the release workflow on `main` with `channel: nightly`
  and `scheduled: true` (this repository's is the maintainer's "PDT nightly" workflow, 08:17 UTC; the workflow has no
  GitHub `schedule:`). With `scheduled`, a head that is already released publishes nothing; a nightly tag at the head
  whose release never got published is built again. By hand: `channel: nightly` without `scheduled` (always builds),
  or Cut a release with `{"type": "prerelease"}`.
- **What it does**: no release PR, nothing committed. It tags the head of `main` `vX.Y.Z-nightly.YYYYMMDD` (`X.Y.Z`
  the next version for the bump, a patch by default; a day's second build `.2`; `scripts/version.sh next-nightly`),
  builds the same signed bundles with that version set in the build's checkout, and publishes a GitHub
  **pre-release** with `SHA256SUMS` and a preview of the unreleased fragments. It then deletes nightlies beyond the
  newest 14 (`scripts/version.sh prune-nightlies`).
- **Who gets it**: hosts with Settings → Updates → Update channel set to nightly. When `X.Y.Z` ships, it sorts above
  its nightlies, so nightly hosts move to it.
- **Migrations**: a nightly can carry migrations no stable release has, and hosts never downgrade, so a host that
  installed one stays on nightlies until a newer stable exists. Merge only migrations you're happy to keep.
- **A failed or missing night**: the maintainer's "PDT nightly release" workflow puts a failed run in the Inbox
  (template "Report failed workflow runs"), and "PDT missing nightly" (template "Report a missing scheduled
  release") says "Tonight's nightly didn't run" when `main` has unreleased commits and no nightly for the day, with
  **Start it**.

### Signing

Every macOS release is signed (`scripts/sign-macos.sh`) with one certificate from the `MACOS_SIGNING_P12` and
`MACOS_SIGNING_P12_PASSWORD` secrets, and notarized (`scripts/notarize-macos.sh`) with an App Store Connect API key
(`MACOS_NOTARY_KEY`, base64 of the `.p8`; `MACOS_NOTARY_KEY_ID`; `MACOS_NOTARY_ISSUER`). A release build (a release,
a nightly, the release PR's bundles) fails in its first step, naming the secret, when any of the three is empty, and
its check step fails unless the app has a stapled ticket and passes `spctl`: a release is never shipped unnotarized,
which macOS refuses to open once downloaded. A **Developer ID
Application** certificate carries a Team ID, so the keychain's partition list trusts every release (`teamid:<id>`)
and the update token is read without a prompt once allowed. A self-signed one (`scripts/make-signing-cert.sh`, a
stopgap) has no Team ID: the keychain then trusts each build's exact code hash, so every update asks again, and
Gatekeeper warns. Check a host with `security dump-keychain -a`: the item's partition list should show `teamid:`.

### Release signing

Each release's `SHA256SUMS` is signed: the `publish` job runs `scripts/release-sign.sh`, which writes
`SHA256SUMS.sig`, an ECDSA P-256 signature (`openssl dgst -sha256 -sign`, DER) over the tag, a newline, then
`SHA256SUMS`. The private key is the `PDT_RELEASE_SIGNING_KEY` secret of the `release-signing` environment, which only
the `publish` job uses, so a leaked `PDT_RELEASE_TOKEN` can't sign. Its public key is pinned as `RELEASE_SIGNING_KEY` in
`crates/pdt-core/src/brand.rs` and as `signing_key` / `$signingKey` in `site/public/install.sh` and `install.ps1`.
Once it is pinned, updates and the install scripts refuse a release without a valid signature. On macOS, pdt-host
also checks that an update's `.app` is signed by `MACOS_TEAM_ID` before swapping it in.

- **No key yet** (`RELEASE_SIGNING_KEY` is `None`): releases go out unsigned, and the `Sign` step warns.
- **Setting it up**: `scripts/release-signing-key.sh <backup file>` on your machine. It makes the key, stores it as
  the environment's secret, writes the private key to the backup file (keep it offline: GitHub never shows it again)
  and prints the lines to pin. Then:
  1. Cut a release so a signed one exists.
  2. Merge the pins, with docs/user saying updates and the one-command install check the release signature
     (`your-machine/updates.md`, `getting-started/install.md`).
  3. Cut the next release. The website serves the new install scripts on merge, and they need a signed latest release.
- **With a key pinned**, a release without the secret, or with another key, fails at `Sign`: hosts would refuse it.
  A dry run on a fork needs its own key pinned on that fork.
- **Changing the key**: hosts check with the key their version pins, so ship a release signed by the old key that
  pins the new one, then change the secret. A lost key leaves hosts unable to update by themselves: they need
  reinstalling.

### Google sign-in

Release and nightly bundles build in PDT's own Google OAuth clients (the GCP project `pdt`): the standard and agentic
Web clients, the standard and agentic Desktop app clients, the file picker's API key and the project's number, from
the `PDT_GOOGLE_CLIENT_ID`, `PDT_GOOGLE_CLIENT_SECRET`, `PDT_GOOGLE_AGENT_CLIENT_ID`,
`PDT_GOOGLE_AGENT_CLIENT_SECRET`, `PDT_GOOGLE_DESKTOP_CLIENT_ID`, `PDT_GOOGLE_DESKTOP_CLIENT_SECRET`,
`PDT_GOOGLE_DESKTOP_AGENT_CLIENT_ID`, `PDT_GOOGLE_DESKTOP_AGENT_CLIENT_SECRET`, `PDT_GOOGLE_PICKER_KEY` and
`PDT_GOOGLE_PROJECT_NUMBER` repository secrets, passed to `.github/actions/bundle` (CI's bundles only when they build a
release). Never commit them. A release build (a release, a nightly, the release PR) warns, naming it, in the job and
its summary when any of the ten is empty (`scripts/google-build-values.sh`): the build ships without that part. A build
without them (a local one, a PR's bundle) offers only the user's own OAuth client;
set the same variables in the environment to build them in locally. The Web clients' only redirect URI is
`https://pdt.dev/oauth/google`, and the consent screen links `https://pdt.dev/privacy`: both are on the website.

The Desktop app clients are for signing in on the same machine: Google sends the browser straight back to PDT on
`127.0.0.1` (or `localhost`) on any port, so they need no redirect URI. A build without them signs in through the
website from the same machine too. A Desktop client's secret isn't confidential by Google's design (it ships in every
copy of the app; PKCE protects its codes), but keep it in the secrets like the others. To make them: in the same
project (so files picked with one client are the others' too, and the consent screen and scopes are shared), under
**Google Auth Platform › Clients**, create two clients of type **Desktop app** (the standard one and the agentic one)
and set their IDs and secrets as the four `PDT_GOOGLE_DESKTOP_*` secrets.

### Moving the release repository

Hosts find the release repository by its numeric id (`RELEASE_REPO_ID` in `crates/pdt-core/src/brand.rs`), which
survives renames and transfers; the name (`RELEASE_REPO`) is the fallback. To move it: ship a release with
`RELEASE_REPO` set to the new `owner/name`, then rename or transfer it, and never create a repository under the old
name (that ends GitHub's redirects). Settings → This host → Updates should then show a recent "Last checked" and no
error; a host that can't find it can be pointed at the new one with the Release repository setting.

## Testing a branch

**Run → pdt (preferred).** In the stable PDT, on the branch's worktree, choose `pdt`. PDT reads `.pdt/project.toml`
from that branch, runs its setup (install, web build, `cargo build -p pdtd`), starts a dev pdtd on a port from the dev
range with the branch's own data folder, and shows the link (also from the phone). Several branches can run side by
side; restart the run after pushing fixes. **Test rust** and **Test web** are the `[[test]]` entries.
`.pdt/project.toml` is this repo's run configuration; its comments explain each setting.

**By hand**, one at a time per data folder (pdtd refuses a data folder another pdtd holds):

```sh
pnpm --dir web build
PDT_PROFILE=dev PDT_PORT=7421 PDT_DATA_DIR=/tmp/pdt-try PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd
```

Without `PDT_PORT` and `PDT_DATA_DIR`, `PDT_PROFILE=dev` uses port 7421 and the `pdt-dev` data folder next to the
installed PDT's; `PDT_SEED=demo` fills an empty one with demo data.

For UI work, `pnpm --dir web dev` serves the UI with hot reload on 5173 and proxies `/api` to 7421. For an agent that
needs to look at the app, see [testing.md](dev/testing.md#looking-at-the-app).

**Desktop tray / installer changes:** `cargo build -p pdtd -p pdt-host`, then `PDT_PROFILE=dev cargo run -p
pdt-desktop`. To check the bundle, `scripts/release-stable.sh --build-only` (Windows: `-BuildOnly`); never without it,
never run a built installer, never `pdt-host install`/`uninstall`.

**Upgrade rehearsal** (maintainers) before a release with migrations or plugin changes: `scripts/rehearse-upgrade.sh` runs this
checkout's pdtd on port 7421 against a copy of the stable data folder (SQLite's online backup, plus `sessions/` and
`pm/`) with `PDT_REHEARSAL=1`: it migrates, installs the built-in workflows and personas and serves the API, but
resumes nothing, starts no loop, keeps secrets in memory and never acts on the copy's repos or connections. It
reports on migrations, integrations, connections, workflows (validate and dry run), tool lists and the GitHub login.
`--keep` keeps the copy, `--from` picks another source, `--report FILE` saves the report, `--port` another port.
Exit 0 when every check passes, 1 when one fails, 2 when it could not run.

## After merge

For a ticket's branch, the ticket's merge banner (**Move to Done and remove the worktree**) does it all once PDT sees
the merge: it stops the worktree's sessions and runs, removes the worktree and the ticket's workspace, moves the
ticket to Done and fast-forwards `main` (the local branch stays). The user clicks it; an agent in that worktree has
nothing to do.

By hand: check it merged (`gh pr view <n> --json state` is `MERGED`), stop the branch's runs, `cd` out of the
worktree, then:

```sh
git -C <repo> fetch --prune origin && git -C <repo> merge --ff-only origin/main
git -C <repo> worktree remove <worktree path>     # or remove it in the project's Settings, so PDT's list stays right
git -C <repo> branch -D <branch>                  # -D: squash merges don't look merged to git
```

GitHub deletes the remote branch on merge. A worktree removed by hand shows as "folder missing" in PDT until it is
removed there too.
