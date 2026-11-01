# How we work on PDT

PDT is built inside PDT: agent sessions run on the stable instance (port 7420), each change lives in its own worktree,
and the user tests it in a dev instance of PDT started with **Run**. This page is the process: branches, checks, pull
requests, CI, the merge queue, the changelog and releases. How the code works is in the developer guide
([architecture.md](dev/architecture.md), [invariants.md](dev/invariants.md), [testing.md](dev/testing.md)).

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

Create them **through the stable PDT**, so its sessions group under the branch and Run uses that checkout:

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
scripts/check.sh --only lint   # just some groups (changelog, site, lint, web, test); CI's lint and web jobs do this
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
  `no-changelog` label (for PRs that only change docs, CI or tests; adding it re-runs CI).
- At release, `scripts/changelog.sh <version>` adds a `## <version> (<date>)` section to `CHANGELOG.md` (Breaking,
  Added, Changed, Fixed), prints the notes, and moves the fragments to `.changes/<version>/`.
  `scripts/changelog.sh notes <version>` prints a version's section (the release's notes); `preview` prints the
  unreleased ones.

## CI

`.github/workflows/ci.yml`; shared setup (toolchain, cache, nextest, pnpm) in `.github/actions/setup`, bundle steps in
`.github/actions/bundle`. Linux does every check that doesn't depend on the OS, in parallel jobs; macOS and Windows
run only what they can break on their own (clippy for `cfg` code, the Rust tests).

| Check | Runs on | Does | Pull requests | Merge queue |
|---|---|---|---|---|
| lint (linux) | Namespace `linux-8` | version sync, fmt, clippy (pdtd also without integration features) | unless docs only | unless docs only |
| test (linux) | Namespace `linux-8` | nextest at the stand-in version 9.9.9 (pdtd also without integration features); generated files are committed | unless docs only | unless docs only |
| web (linux) | Namespace `linux-8` | web typecheck, vitest, build | unless docs only | unless docs only |
| build (macos-latest) | Namespace `macos-6` | clippy, nextest, desktop crate clippy and test | `macos` label, or a `.github/` change | unless docs only |
| build (windows-latest) | GitHub Windows | clippy, nextest | `windows` label, or a change to `windows/`, `*.ps1`, `*.cmd`, `.github/` | same |
| site | GitHub `ubuntu-latest` | the install scripts' lint, the website's tests (the docs check included) and build | a change to `site/`, `docs/`, the pnpm lockfile or workspace, `ci.yml` or `pages.yml` | same |
| changelog | GitHub `ubuntu-latest` | a fragment or the `no-changelog` label; the scripts' tests | unless docs only | skipped |
| bundle (…) | GitHub macOS, Windows, `ubuntu-22.04` and `-arm` | the macOS app, the Windows installer, the Linux tarball and AppImage, as workflow artifacts | the release PR only | never |

- **Docs-only PRs** (every changed file under `docs/` or `site/`, or a `*.md`) run only `changes` and `site`. The
  guides pdtd builds in with `include_str!` (`docs/agent-guide.md`, `docs/run-config.md`, `docs/prompts/`,
  `docs/user/automation/run-configuration.md`; `crates/pdt-api/src/guides.rs`) count as code, since Rust tests check
  them. If other code starts reading a Markdown file, add it to that rule in the `changes` job.
- **Tests never depend on the checked-in version**: `test (linux)` sets it to 9.9.9 in its own checkout, so a test
  that hard-codes `0.x.y` fails on its own PR. Compare with `pdt_core::BUILD_VERSION` or `env!("CARGO_PKG_VERSION")`.
- **Main runs nothing**: `ci.yml` has no `push` trigger (the queue has just checked that tree). Only `pages.yml`
  runs on main, redeploying the website when `site/**` or `docs/**` changed.
- **macOS waits for the queue** because Namespace bills macOS at 10× Linux; a macOS-only break shows when the PR
  enters the queue. Label a PR `macos` to see it earlier.
- **Windows runs only when asked**: a Windows-only break in a PR without the `windows` label or Windows files is not
  caught before it merges. Label any path, process or file-system change `windows`. Windows runs start from a cold
  cache (GitHub's runners save rust-cache only from main, which runs no CI).
- **Bundles by hand**: `gh workflow run ci --ref <branch>` (or Actions → ci → Run workflow, "Build bundles") runs
  every check and builds every bundle at `<version>-local`, kept as artifacts for 5 days and published nowhere: use
  it to try a packaging change.
- **Job names are the "Main" ruleset's required checks.** A job skipped by its `if` still reports, and a skipped check
  passes. Renaming a job means editing the ruleset first.

### The merge queue

The "Main" ruleset merges through a merge queue (squash, one PR per group). A PR joins it once its checks pass and
auto-merge is on; GitHub makes `gh-readonly-queue/main/pr-<n>-<sha>` (main plus the PR) and runs `ci.yml` on it
(`merge_group`). The `changes` job picks jobs as for a PR: files from the group's base and head (over 300 runs
everything), labels from PR `<n>` in the branch name, which is why the group size stays 1. The PR merges when the
required checks pass; one that conflicts with main or fails leaves the queue: fix the branch (merge main in for a
conflict), push, and merge again.

### Reviews

The "Main" ruleset requires one approving review from a code owner. `.github/CODEOWNERS` makes the maintainer
(`@rossxwest`) the owner of every file, so an outside PR (a contributor's, Dependabot's) needs the maintainer's
approval before it can join the queue. The maintainer's own PRs, which PDT opens as `rossxwest`, can't be approved by
their author and merge through the ruleset's admin bypass.

### Runners

- **Namespace** (its GitHub App is installed on the repo) runs the Linux and macOS checks: profiles
  `namespace-profile-linux-8` and `namespace-profile-macos-6`, each with a cache volume (`target/`, the cargo
  registry, the pnpm store), saved when a job exits cleanly. Its Linux image has no `SHELL` and no zsh, so `setup`
  with `zsh: true` installs zsh for pdtd's terminal tests.
- **GitHub** runs Windows, the bundles (signing) and the small `changes`, `changelog` and `site` jobs.
- Repo variables `PDT_LINUX_RUNNER` / `PDT_MACOS_RUNNER` override the Namespace runners (`gh variable set
  PDT_LINUX_RUNNER --body ubuntu-latest`; `gh variable delete` to go back); `setup` then uses GitHub's caches.
- `.github/workflows/bench.yml` (by hand only) times the same work on a list of runners, to compare hosts.
- The Mac mini's old self-hosted runner is retired; if it is still listed under Settings → Actions → Runners, remove
  it, and never make the repo public while it is registered.

## Dependabot

`.github/dependabot.yml` checks the Cargo workspace, the pnpm workspace (`web`, `desktop`, `site`) and GitHub Actions
every Monday: one grouped PR of minor and patch updates per ecosystem, and one PR per major update, at most five open
each. `rust-toolchain.toml` is bumped by hand.

- Its PRs carry the `dependencies` and `no-changelog` labels and `deps:` commit messages.
- They are outside PRs: PDT's external-PR workflow has an agent review each, and the user approves and merges it
  through the queue. A major update that needs code changes is fixed on its branch or in a ticket of its own.

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
   queue squashed the PR's commit away), and publishes the GitHub release with `SHA256SUMS` and the version's
   `CHANGELOG.md` section. Re-running a failed job reuses the tag and release and replaces re-uploaded assets.

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
release is published; while an earlier release PR is open it starts nothing and says so.

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
(`MACOS_NOTARY_KEY`, base64 of the `.p8`; `MACOS_NOTARY_KEY_ID`; `MACOS_NOTARY_ISSUER`). A **Developer ID
Application** certificate carries a Team ID, so the keychain's partition list trusts every release (`teamid:<id>`)
and the update token is read without a prompt once allowed. A self-signed one (`scripts/make-signing-cert.sh`, a
stopgap) has no Team ID: the keychain then trusts each build's exact code hash, so every update asks again, and
Gatekeeper warns. Check a host with `security dump-keychain -a`: the item's partition list should show `teamid:`.

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

**By hand**, one at a time (pdtd refuses a data folder another pdtd holds):

```sh
pnpm --dir web build
PDT_PROFILE=dev PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd      # port 7421, data dir pdt-dev
```

For UI work, `pnpm --dir web dev` serves the UI with hot reload on 5173 and proxies `/api` to 7421. For an agent that
needs to look at the app, see [testing.md](dev/testing.md#looking-at-the-app).

**Desktop tray / installer changes:** `cargo build -p pdtd -p pdt-host`, then `PDT_PROFILE=dev cargo run -p
pdt-desktop`. To check the bundle, `scripts/release-stable.sh --build-only` (Windows: `-BuildOnly`); never without it,
never run a built installer, never `pdt-host install`/`uninstall`.

**Upgrade rehearsal** before a release with migrations or plugin changes: `scripts/rehearse-upgrade.sh` runs this
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
