# How we work on PDT

PDT is built inside PDT: agent sessions run on the stable instance (port 7420), each change lives in its own worktree,
and the user tests it in a dev instance of PDT started with **Run**. This is the day-to-day version of the remote-host
spec's development lifecycle (docs/superpowers/specs/2026-10-15-remote-host-design.md §7).

## The loop

1. **Pick the work.** It comes from the project's tickets in the stable PDT (Tickets → Backlog / Board; `GET /api/tickets`). Larger work gets a spec in
   docs/superpowers/specs and a plan in docs/superpowers/plans first.
2. **Make a worktree** for a new branch (below). Never commit on `main` in the main checkout.
3. **Build it test-first** in the worktree (TDD for core logic). Run the checks before every push (below).
   Add a changelog fragment for the change (below, "Changelog").
4. **Push and open a PR** against `main`. CI runs the full checks on Linux on every push (Namespace); the macOS checks
   run in the merge queue, and on a PR only with the `macos` label or a `.github/` change; Windows checks run only
   with the `windows` label or when the PR touches their files. The bundles run on no PR but the release PR (below, "Releases").
   Docs-only PRs (docs, the website under `site/`, Markdown) skip CI but the website build (docs/superpowers/notes/ci.md). Label a PR `windows` when it touches platform-specific code.
5. **The user tests it**: Run → `pdt` on the branch gives a dev instance on its own port and data folder (below).
   Feedback comes back in the session; the agent fixes the branch and the run is restarted on the new code.
6. **Merge when the user says so**, with auto-merge: `gh pr merge <n> --auto`, with no merge-method flag (the queue
   sets the method; `--squash` only prints "The merge strategy for main is set by the merge queue"). Once the PR's required
   checks pass it joins main's **merge queue**: GitHub builds main plus the PR on a temporary branch, runs CI on it
   and squash-merges the PR when that passes, so nobody sits in a shell watching CI and nobody updates the branch
   with main. PRs are squash-merged (the only merge mode on the repo); the PR title becomes the commit subject on
   `main`, so write it as one. The queue drops a PR that conflicts with main or fails its queue run: fix the branch
   (merge main in for a conflict), push, and merge again. Don't pass `--delete-branch`: it can't delete a branch
   whose worktree still exists.
7. **Merging ships nothing.** A merge to `main` builds no bundle and publishes no release; releases are deliberate
   (below, "Releases"). When one is published, the stable instance installs it when idle, or when the user clicks
   Update. Agents never update, restart or stop the stable instance.
8. **Clean up**: stop the branch's runs, then remove the worktree and branch (below). Move the ticket to Done (or let the
   merge banner do it) and add tickets for follow-ups the change turned up. Work that takes several PRs is a parent
   ticket with one part per PR: each part has its own branch, worktree and PR, and its merge closes only that
   part; start the next one with Start next part on the parent, which moves to Done once every part is closed.

## Worktrees

Worktrees live at `.worktrees/<branch>` inside the repo (`/` in a branch name becomes `-`); `/.worktrees/` is in
`.git/info/exclude`. Ticket Start names branches `pdt-<KEY>-<n>-<slug>` (no `/`: some third-party tools break on it); other branch names
are short and kebab-case, e.g. `sdlc-docs`.

Create them **through the stable PDT**, not with `git worktree add`, so PDT knows about them: its sessions group under
the branch and Run uses that checkout. Start from an up-to-date `main`:

```sh
git -C <repo> fetch origin && git -C <repo> merge --ff-only origin/main   # main checkout is clean; keep it that way
curl -s 127.0.0.1:7420/api/projects                                        # find the project id (repoPath = <repo>)
curl -s -X POST 127.0.0.1:7420/api/projects/<project id>/worktrees \
  -H 'content-type: application/json' -d '{"branch":"<branch>"}'           # optional "base": branch or commit
```

The new branch starts at the project's base branch (`main`). Then `cd` into the returned `path` and work there.

A folder made with plain `git worktree add` is unknown to PDT, and Run on that branch then fails with
`worktree_path_taken`. If that happens, either remove it and recreate it as above, or skip Run and start a dev
instance by hand (below).

## Checks before pushing

Run in the worktree, all of them (CI runs the same set):

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                      # also regenerates web/src/api/gen; commit any changes there
pnpm --dir web typecheck
pnpm --dir web test
pnpm --dir web build
cargo clippy -p pdt-desktop --all-targets -- -D warnings   # only if desktop/ changed; needs the web build
cargo test -p pdt-desktop                                  # likewise
```

**Watching CI:** usually unnecessary (auto-merge waits for it). When you do need a result, run the watch in the
background (or with Monitor), not as a long foreground wait, and use the Actions run, not `gh pr checks`. The `gh` token here is fine-grained, and fine-grained tokens
cannot read check runs (there is no Checks permission for them), so `gh pr checks` fails with "Resource not
accessible by personal access token".

```sh
gh run list --branch <branch> --limit 1 --json databaseId,status,headSha   # the run for the latest push
gh run watch <run id> --interval 30 --exit-status                          # exits non-zero if CI failed
```

A new push starts a new run (the old one is cancelled), so look the run up again after each push.

A fresh worktree needs `pnpm install --frozen-lockfile` once. Its `target/` starts empty, so the first build is slow.
CI runs `cargo nextest run`; nextest isn't installed on the dev machine, and plain `cargo test` covers the same tests.
Tests never touch port 7420, the real data folders or the real keychain.

## Changelog

Release notes are written for people who use PDT, by whoever makes the change, when they make it: every PR adds a
**fragment**, `.changes/unreleased/<TICKET>.md` (a second note for the same ticket: `<TICKET>-<anything>.md`):

```markdown
---
kind: fixed
---
The board no longer jumps back to the top when a ticket moves.
```

- `kind` is `added`, `changed`, `fixed`, `breaking` or `internal`. The text is one or two sentences (or a short
  Markdown list) saying what changed for the user, not how the code changed; it says nothing about tickets, crates or
  file names unless the user sees them. `internal` is for changes users never notice (refactors, tooling) and is left
  out of the release notes.
- `scripts/changelog.sh check` checks the fragments (name, front matter, kind, text).
- **CI** (the `changelog` job, pull requests only) fails a PR that adds no fragment unless it has the `no-changelog`
  label, which is for PRs that only change docs, CI or tests. Adding the label re-runs CI. Docs-only PRs skip the
  job, and the release PR passes it without one (it compiles them). It also runs the scripts' tests
  (`scripts/changelog.test.sh`, `scripts/version.test.sh`, `scripts/release-tag.test.sh`).
- In the Pipeline, the Coder writes the fragment (this repo's AGENTS.md says how) and the Reviewer checks that it is
  written for users and accurate.
- **At release**, `scripts/changelog.sh <version>` adds a `## <version> (<date>)` section to the top of
  `CHANGELOG.md`, grouped Breaking, Added, Changed, Fixed (Internal left out), prints the same notes without the
  heading to stdout (`scripts/changelog.sh 0.2.0 > notes.md` for the release notes), and moves the fragments to
  `.changes/<version>/`. It refuses an existing version folder or an invalid fragment, changing nothing.
  The release workflow runs it on the release PR (below, "Releasing"), and publishes `scripts/changelog.sh notes
  <version>` (that version's section, as compiled and perhaps edited since) as the release's notes.

## Dependabot

`.github/dependabot.yml` has Dependabot check three ecosystems every Monday: the Cargo workspace, the pnpm workspace
(the root `pnpm-lock.yaml`, covering `web`, `desktop` and `site`) and GitHub Actions (the workflows and the composite
actions in `.github/actions/`). For each it opens one grouped PR with that week's minor and patch updates, and a PR of
its own for each major update, at most five open per ecosystem. Security updates, once turned on in the repo's
settings, arrive the same way. `rust-toolchain.toml` is still bumped by hand.

- Its PRs carry the `dependencies` and `no-changelog` labels (dependency bumps need no release note) and commit
  messages starting `deps:`.
- They are outside PRs, not Pipeline attempts: PDT handles them with the external-PR workflow, an agent
  reviews each, and the user approves and merges it through the merge queue like any other PR. A major update may
  need code changes; make those on the Dependabot branch or in a ticket of their own.
- GitHub Actions PRs change `.github/`, so they run the macOS and Windows checks too.

## Releases

Releases are deliberate: merges don't ship. A push to `main` runs no CI at all (the merge queue has just checked that
tree), builds no bundle and publishes nothing. Nightly releases (below) are built from `main` once a night (and by
hand), never per merge.

- **Versions are SemVer**, `X.Y.Z`. The source of truth is the workspace `version` in `Cargo.toml`;
  `desktop/src-tauri/tauri.conf.json` carries the same one. `scripts/version.sh` prints it and `scripts/version.sh
  check` fails when the two differ (CI runs it in `lint (linux)`). Both change only in a release PR. The first
  deliberate release is 0.2.0, after the per-merge `v0.1.<run number>` releases.
- **Builds:** a release build reports the version as is; every other build (local, Run, a bundle built by hand)
  reports `<version>-local`, which never counts as newer than a release.
- **Updates** compare versions by SemVer and never install an older version than the one running. A host on the
  stable update channel (the default) reads GitHub's latest release (never a draft or pre-release); one on the
  nightly channel also takes nightly pre-releases (`vX.Y.Z-nightly.YYYYMMDD`), whichever is newest. The update
  banner shows the notes of every release between the installed version and the new one.
- **Bundles** (the signed macOS app, the Windows installer, the Linux tarball and AppImage) are built only for a
  release (on the release PR and from the tag), or by hand to try a packaging change: run the `ci` workflow on the
  branch with "Build bundles" (`gh workflow run ci --ref <branch>`). Those are kept as workflow artifacts for a few
  days and published nowhere.

### Releasing

The user decides when to release stable versions; only nightlies have a schedule. `.github/workflows/release.yml` does it in two halves:

0. **Screenshots** (when the UI changed since the last release): regenerate the website's screenshots with
   `node scripts/site-screenshots.mjs` (site/README.md "Screenshots") and merge them before starting, so the site
   shows what the release ships.

1. **Start it**: Actions → release → Run workflow on `main`, with the bump (`patch`, `minor`; `major` only from 1.0.0
   on) or an exact version (`gh workflow run release.yml -f bump=minor`, or `-f version=1.0.0`). Or ask the PM, which
   runs PDT's "Cut a release" workflow (below). Starting it is the deliberate act: nobody reviews the release PR. To
   change the notes, edit the changelog fragments (`.changes/unreleased/`) on `main` before starting it. It refuses
   while another release PR is open.
2. **The release PR**: the workflow sets the version in `Cargo.toml`, `Cargo.lock` and `tauri.conf.json`
   (`scripts/version.sh next` / `set`), compiles the fragments into `CHANGELOG.md`'s new section
   (`scripts/changelog.sh <version>`), and opens `Release vX.Y.Z` from the `release` branch with those notes as its
   body. It is a normal PR: CI runs, and on this PR only the bundle jobs build the release bundles (signed and
   notarized macOS app, Windows installer, Linux packages), so packaging problems show before anything is tagged;
   the `changelog` check lets it through without a fragment. The workflow turns on its auto-merge (with
   `PDT_RELEASE_TOKEN`; the job fails if it can't), so it joins the merge queue and merges by itself once its checks
   pass. A failed check leaves it open and unmerged: push a fix to the `release` branch (after the merge queue removes
   it, turn auto-merge on again), or close it, which releases nothing.
3. **Tag and publish**: when it merges, the workflow tags `vX.Y.Z` on the merge commit, builds the bundles again from
   that commit (the PR's were built from a commit the queue squashed away), and publishes the GitHub release with the
   bundles, `SHA256SUMS` and the version's `CHANGELOG.md` section as its notes (`scripts/changelog.sh notes
   <version>`). Hosts see it on their next update check. A failed job can be re-run from the Actions page: the tag and
   the release are reused, and re-uploaded assets replace the old ones.

Setup: the `PDT_RELEASE_TOKEN` secret (required), a token that may push branches, open pull requests and turn on
their auto-merge here (fine-grained: Contents and Pull requests, read and write), and auto-merge allowed in the
repository's settings. Without the token the workflow stops before opening anything: with the workflow's own token
GitHub would start no CI on the release PR and its merge would tag nothing. The bundles use the macOS signing and
notarizing secrets.

**Dry run**: on a fork with the same secrets, set the repo variable `PDT_RELEASE_TAG_PREFIX` to a test prefix
(`test-v`) and run it end to end: the tag becomes `test-vX.Y.Z` and the release a pre-release, which no PDT installs.
"Cut a release" doesn't count pre-releases either, so in such a dry run it waits at the last step and never says the
release is published.

**"Cut a release"** (a built-in PDT workflow, off by default; Settings → Workflows): the PM starts it (only when the
user asks for a release) with this repository's release workflow as its input: `{"workflow": "release.yml",
"inputs": {"bump": "minor"}, "releaseBranch": "release"}` (or `"version": "1.0.0"` in `inputs`; no `tag`: the
repository's latest release, which nightlies never are). It
runs the release workflow, tells the user when the release PR is open (nothing to do), puts a failed check in the
Inbox with the failed jobs (the user fixes it on `release` or closes the PR), and tells the user when the release is
published. While an earlier release PR is still open it starts nothing and says so in the Inbox: that one merges by
itself once its checks pass, or needs fixing or closing. For a nightly now, leave `releaseBranch` out and give
`"inputs": {"channel": "nightly"}, "tag": "v*-nightly.*"`: no release PR, and the Inbox says when the pre-release is
published. Nothing in Cut a release is PDT's own: any project gives its own workflow, inputs, branch and tag
pattern (docs/user "Workflows").

### Moving the release repository

Hosts reach the release repository by its numeric id (`RELEASE_REPO_ID` in `crates/pdt-core/src/brand.rs`), which
survives renames and transfers; the name (`RELEASE_REPO`) is the fallback. To move it: ship a release with
`RELEASE_REPO` set to the new `owner/name` first (the id stays), then rename or transfer it, and never create a
repository under the old name again (that ends GitHub's redirects). Check a host afterwards: Settings → This host →
Updates shows a recent "Last checked" and no error. A host that can't find it can be pointed at the new repository
with the Release repository setting.

### Nightly releases

A nightly release puts the head of `main` in the hands of hosts that opted in, between stable releases: to try a
change on real machines before it ships to everyone, or to get a fix to one host by the next morning. Nightlies
replaced the experimental releases cut by hand (`vX.Y.Z-exp.N`); hosts on an experimental build move to the
next nightly.

- **When**: the release workflow's schedule runs it every day at 08:17 UTC (off the hour: GitHub's scheduled runs
  are best-effort, and top-of-the-hour ones are often delayed or dropped). When the head of `main` is already
  released (the last nightly, or the stable release just merged, points at it) it publishes nothing and the run ends
  green; a nightly tag at the head whose release never got published (that night failed) is built again.
- **Cut one by hand**: the release workflow with channel `nightly` (`gh workflow run release.yml -f channel=nightly`,
  with `-f bump=minor` or `-f version=X.Y.Z` when the next release isn't a patch), or ask the PM for a nightly
  release (Cut a release with `"inputs": {"channel": "nightly"}`, above). A manual run always builds, even when `main` hasn't changed.
- **What it does**: no release PR, and nothing is committed to `main`. It picks `X.Y.Z-nightly.YYYYMMDD`: `X.Y.Z` is
  the version the next release would have for the bump (a patch on the schedule), the date is UTC, and a day's
  second build is `.2`, then `.3`… (`scripts/version.sh next-nightly`). It tags the head of `main` with it, builds
  the same signed and notarized bundles as a stable release with that version set only in the build's checkout, and
  publishes a GitHub **pre-release** with `SHA256SUMS`. Its notes are "Nightly build of main at `<sha>`" and a preview
  of the unreleased fragments (`scripts/changelog.sh preview`, which leaves them in `.changes/unreleased/` for the
  stable release).
- **Retention**: after publishing, it deletes the nightlies (release and tag) beyond the newest 14 by version
  (`scripts/version.sh prune-nightlies`). Stable releases and their tags are never touched, nor the old `-exp.N`
  builds or another prefix's tags.
- **Who gets it**: only hosts with Settings → Updates → Update channel set to nightly (a host set to experimental is
  on nightly). Stable hosts ignore it. When `X.Y.Z` ships as a stable release, it sorts above its nightlies (which
  sort above its `-exp.N` builds), so nightly hosts move to it.
- **Migrations**: a nightly build can carry database migrations that no stable release has yet. A host never
  downgrades, so one that installs a nightly stays on nightlies (or newer) until a newer stable release exists, even
  when it goes back to the stable channel. Merge to `main` only migrations you're happy to keep.
- **A failed night**: on a host that builds PDT, the built-in "PDT nightly release" workflow reads the release
  workflow's newest run on `main` every day at 11:00 UTC and puts a failed one in the Inbox ("The nightly release
  failed", with its failed jobs), once per run.
- **A night that never ran**: GitHub sometimes never starts a scheduled run at all, so there is no failed run to
  report. On a host that builds PDT, the built-in "PDT missing nightly" workflow looks at 10:00 UTC: when `main` has
  commits that the newest nightly and the latest stable release lack, and no nightly for the day (UTC) is published,
  the Inbox says "Tonight's nightly didn't run", once a day, with a link to the release workflow's runs and **Start
  it** (runs the release workflow with channel `nightly` as your GitHub login; when that fails, the Inbox says so and
  links GitHub's Run workflow). It looks again every half hour, and the item closes by itself once a nightly for the
  day is published (after about a day it gives up). Days `main` didn't change say nothing.
- **Re-runs**: re-running the failed jobs reuses the tag; re-running every job of a manual run makes the day's next
  build. A run that failed before publishing leaves its tag behind (the next scheduled run on the same commit builds
  it again), so the day's numbers may skip one.
- **Dry run**: with `PDT_RELEASE_TAG_PREFIX` set (`test-v`), the nightlies are `test-vX.Y.Z-nightly.YYYYMMDD`, and
  retention keeps that prefix's newest 14.

## Testing a branch in a dev instance

**Run → pdt (preferred).** In the stable PDT, on the branch's worktree (or "Run branch" with the branch name), choose
`pdt`. PDT reads `.pdt/project.toml` from that branch, runs its setup (install, web build, `cargo build -p pdtd`),
then starts a dev pdtd on a port from the dev range with the branch's own data folder, and shows the link. It works
from the phone over Tailscale too. Several branches can run side by side. After pushing fixes, restart the run so it
rebuilds. The same menu offers **Test rust** and **Test web** (the `[[test]]` entries).

`.pdt/project.toml` is the run configuration for this repo; change it in a PR like any other code. Its comments
explain each setting. Windows run-step limits: docs/run-config.md "Traps".

**By hand** (when Run is not an option):

```sh
pnpm --dir web build
PDT_PROFILE=dev PDT_WEB_DIR=$PWD/web/dist cargo run -p pdtd      # port 7421, data dir pdt-dev
```

Only one manual dev instance at a time (pdtd refuses a data folder another pdtd holds). For UI work,
`pnpm --dir web dev` serves the UI with hot reload on 5173 and proxies `/api` to 7421.

**Desktop tray / installer changes:** `cargo build -p pdtd -p pdt-host`, then `PDT_PROFILE=dev cargo run -p
pdt-desktop`. To check the bundle, `scripts/release-stable.sh --build-only` (Windows: `-BuildOnly`). Never run the
script without it, never run a built installer, never run `pdt-host install`/`uninstall`.

**Upgrade rehearsal:** before a release with migrations or plugin changes reaches the stable instance,
`scripts/rehearse-upgrade.sh` runs this checkout's pdtd on port 7421 against a read-only copy of the stable data dir
and reports whether migrations, integrations, connections, workflows (validate and dry run), tool lists and the GitHub
login hold up. It copies the database with SQLite's online backup (safe while stable runs), plus `sessions/` and `pm/`,
into a temp folder (`--keep` keeps it; `--from` picks another source; `--report FILE` saves the Markdown report), and
starts pdtd with `PDT_REHEARSAL=1`: it migrates, installs the built-in workflows and personas and serves the API, but
resumes nothing, starts no background loop, keeps secrets in memory and never acts on the copy's repos or
connections. Exit status 0 when every check passes, 1 when one fails, 2 when it could not run. Stop a dev instance on
7421 first, or pass `--port`.

## After merge

For a ticket's branch, the ticket's merge banner ("Move to Done and remove the worktree") does all of this once PDT
sees the merge: it stops the worktree's sessions and runs, removes the worktree and the ticket's workspace, moves the
ticket to Done and fast-forwards `main` (the local branch stays; `git branch -D` it if wanted). The user clicks it;
an agent working in that worktree has nothing to do.

By hand: check it merged (`gh pr view <n> --json state` says `MERGED`), stop the branch's runs in PDT, and `cd` out of the
worktree (a shell whose folder is removed fails every command after). Then, in this order:

```sh
git -C <repo> fetch --prune origin && git -C <repo> merge --ff-only origin/main
git -C <repo> worktree remove <worktree path>
git -C <repo> branch -D <branch>                  # -D: squash merges don't look merged to git
```

GitHub deletes the remote branch when the PR merges (the repo setting is on); `fetch --prune` drops its local
tracking ref.

Removing it from the project's Settings page (`DELETE /api/worktrees/{id}`) does the worktree step and keeps PDT's list
right. A worktree removed by hand shows as "folder missing" in PDT until it is removed there too.

## Safety rules (from AGENTS.md)

- Never stop, restart or update the stable instance (7420) or call its `/api/shutdown`. Creating worktrees and
  starting runs through its API is fine.
- Every dev instance gets an explicit `PDT_PORT` and `PDT_DATA_DIR` (Run does this; `PDT_PROFILE=dev` does it by
  hand).
- Never edit an applied migration; add a new one, named by its UTC creation time:
  `crates/pdt-store/migrations/$(date -u +%Y%m%d%H%M%S)_<what>.sql`. Never the next number (`0001`–`0029` are the
  old numbered ones): two branches would pick the same one. `cargo test -p pdt-store` fails on a misnamed migration or
  a duplicate version.
