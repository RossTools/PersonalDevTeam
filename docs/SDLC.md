# How we work on PDT

PDT is built inside PDT: agent sessions run on the stable instance (port 7420), each change lives in its own worktree,
and the user tests it in a dev instance of PDT started with **Run**. This is the day-to-day version of the remote-host
spec's development lifecycle (docs/superpowers/specs/2026-10-15-remote-host-design.md §7).

## The loop

1. **Pick the work.** It comes from the project's tickets in the stable PDT (Tickets → Backlog / Board; `GET /api/tickets`). Larger work gets a spec in
   docs/superpowers/specs and a plan in docs/superpowers/plans first.
2. **Make a worktree** for a new branch (below). Never commit on `main` in the main checkout.
3. **Build it test-first** in the worktree (TDD for core logic). Run the checks before every push (below).
4. **Push and open a PR** against `main`. CI runs the full checks on Linux on every push (Namespace); the macOS checks
   run in the merge queue, and on a PR only with the `macos` label or a `.github/` change; Windows checks and the
   bundles run on a PR only with the `windows` / `bundle` label or when it touches their files, and always on main.
   Docs-only PRs skip CI (docs/superpowers/notes/ci.md). Label a PR `windows` when it touches platform-specific code.
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
7. **Release is automatic.** Every push to `main` publishes `v0.1.<run number>`. The stable instance installs it when
   idle, or when the user clicks Update. Agents never update, restart or stop the stable instance.
8. **Clean up**: stop the branch's runs, then remove the worktree and branch (below). Move the ticket to Done (or let the
   merge banner do it) and add tickets for follow-ups the change turned up. Work that takes several PRs is a parent
   ticket with one part per PR (PDT-38): each part has its own branch, worktree and PR, and its merge closes only that
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
login hold up (docs/reference/host.md "Upgrade rehearsal"). Stop a dev instance on 7421 first, or pass `--port`.

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
