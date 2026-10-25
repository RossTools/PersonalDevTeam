# Review and merge

Phase 6, PDT-3. Spec: docs/superpowers/specs/2026-10-19-review-design.md; plan:
docs/superpowers/plans/2026-10-19-phase-6-review.md. Code: pdt-git `review.rs`, pdt-core `review.rs`, pdt-api
`review.rs`, pdt-github `merge_pr`, web/src/review.

## Changes

- A worktree's changes are its working tree (committed, staged, unstaged, untracked) against the merge base of `HEAD`
  with the base branch (the attempt's, else the project's; `origin/<base>`'s when newer).
- `GET /api/worktrees/{id}/changes` lists files (status, +/−, `version` = both sides' blob ids) with the attempt,
  `approved` and `dirty`; `GET …/changes/file?path=&oldPath=` serves both texts (≤ 1 MB each, not binary).
- The browser draws the diff with `@codemirror/merge` (web/src/review `CodeDiff`, lazy; split or unified, unified
  below 900px; unchanged runs collapse unless Whole file, except a run that holds a thread; lines wrap).
- A Markdown file has Diff | Rendered (PDT-75): rendered shows Before | After side by side in the split layout, else
  the file as it is now (or was, when deleted); comments are made in the diff.

## Comments and feedback

- `review_comments` (migration 0019, deleted with the worktree): `GET|POST /api/worktrees/{id}/comments` (a root
  anchors to `side` old/new, `line`..`endLine` and `lineText`; a reply takes its root's anchor),
  `PATCH|DELETE /api/review-comments/{id}` (body and delete on drafts only, 409 `comment_sent`; `resolved` on roots).
  The browser re-finds moved lines by their text, else lists the thread as outdated (web/src/review/anchors.ts).
- `POST /api/worktrees/{id}/feedback` sends every draft as one message (pdt-core `review::feedback_message`) to the
  newest agent session of the attempt (else of the worktree) that can take it: a conversation's next user message, or
  a bracketed paste into a running terminal (409 `no_drafts`, `no_agent_session`, `session_not_running`); drafts
  become sent and the ticket gets a `feedback` event whose body keeps the sent comments as Markdown
  (`review::feedback_record`; the comments themselves go with the worktree; the timeline folds them under Show the
  comments).
- `POST /api/attempts/{id}/approve` records a `review` event at the branch head (Approved while the head is
  unchanged; 409 `attempt_inactive` once the attempt ended). A workflow gate's `approve` choice (the Pipeline's
  Accept) records the same event, as the user's, naming the workflow and run.

## Merge

- `GET /api/attempts/{id}/merge` previews (`problem` names what blocks it, pdt-core `MergeProblem`); `POST` merges.
  `POST` is the Merge command (pdt-api `review::merge_attempt`, shared with the workflow node `attempt.merge`), and
  the project's policy comes first (Phase 7c, pdt-core `policy::may_merge`): PDT's Merge… sends `by: "user"`, a
  person's merge; anything else is automated. `mergeBy: user` refuses automated merges and `external` every merge
  (409 `merge_not_allowed`); `requireApproval` refuses an automated merge unless a person approved the branch's
  current head (409 `approval_required`; a person's own merge needs none), except for a ticket with one of the
  policy's `approvalExemptLabels` (project settings: "Labels that merge without approval"). A local merge's `merged` event is the
  user's for a person's merge, else `system`.
  Every refusal is 409 `merge_blocked` with a message (inactive, dirty, conflict, base_dirty, nothing,
  pr_not_mergeable (the message says whether it is a draft or conflicts), pr_queued, no_pr); a failing gh or git is
  409 `merge_failed`. A refusal for a conflict or uncommitted changes in the worktree is `fixable` to the
  `attempt.merge` node (PDT-124). A refusal never records the attempt as set to merge: then PR notices would turn
  auto-merge on when CI passes, also for a fix no one reviewed (the Pipeline fixes and reviews its own). An automated
  merge whose pull request is already on auto-merge or in the queue counts as queued.
- Green CI (PDT-151, policy `requireGreenCi`, on by default; project settings: "An automated merge waits for green
  CI"): an automated merge of a pull request reads its PR from GitHub again and needs CI on its head to have passed
  (GitHub Actions and Buildkite together, as the badge shows it; pdt-core `pr::ci_gate`), whatever GitHub's branch
  protection requires (`gh pr merge --auto` merges at once where nothing is required). A failed or cancelled run is
  409 `ci_failed` (`fixable`, the reason names the failed jobs and the run); runs still going, or none yet while the
  repository has CI (`.github/workflows` files at the head or on the branch, or a Buildkite pipeline for the project),
  or a read from GitHub that failed, `ci_pending`. A repository with no CI merges as before, and a ticket comment says it merged without waiting for CI.
  A person's merge doesn't wait.
- With an open PR (`attempt.pr`) it runs `gh pr merge --auto` and records the attempt as set to merge
  (`attempts.merge_requested`, PDT-104: PR notices gets it merging again after a failure) (pdt-github `merge_pr`; the method from
  `merge_settings`, none with a merge queue; when GitHub says the PR is already clean it merges at once) and the merge
  check ends the attempt later. The dialog's "Move to Done and clean up once it merges" (on by default; Delete the
  branch off) sends `{finishOnMerge, deleteBranch}` (left out: what an earlier merge asked stays), kept on the attempt (`attempts.finish_on_merge`,
  `finish_delete_branch`, PDT-75): when the merge check ends the attempt as merged, pdt-api `finish.rs` does what
  `finishMerged` does without its questions (Done; the worktree's and the Start workspace's sessions and runs
  stopped and removed, the attempt's other sessions stopped; the worktree, the workspace with its tabs; the base
  pulled). A worktree with uncommitted changes is never discarded: then nothing happens and the merged banner stays.
- Otherwise a local merge with the user's identity: a `--no-ff` merge commit, or with `{style: "squash"}` one
  single-parent commit (`<KEY>: <title>`; PDT-65), in the checkout that has the base (which must be clean;
  `merge --squash` + `commit` for a squash), else `merge-tree` + `commit-tree` + `update-ref`. The dialog's Merge
  commit | Squash is remembered per project in localStorage. It ends the attempt as merged, then the dialog runs
  `finishMerged` (Remove the worktree on, Delete the branch off) or just moves the ticket to Done.
- A project whose settings say Merge through pull requests (`project_settings.merge_via_pr`, `mergeViaPr` in the
  settings API, kept when a PUT leaves it out) never merges locally: no open PR is `no_pr`.
- `POST /api/attempts/{id}/ask-agent` asks the coder to fix a `conflict` (merge the base in) or `no_pr` (push and
  open a PR); 409 `nothing_to_ask`, `worktree_missing`.

## Web

The ticket tab's Changes (its section tab shows the diff size; the section last shown is remembered per ticket in
localStorage), the session header's Changes button (`ChangesButton`, with the diff size), and changes tabs
(`workspace_tabs.worktree_id`, `OpenTab.worktreeId`; only close) from a worktree `BranchChip`'s Review changes, the
project's worktree list and the palette. The session header's Changes button and branch chip open the tab in the
workspace on screen, after the session's tab (moving it there if another workspace has it); the others show it
wherever it is, or open it in the project's workspace. Viewed per browser (localStorage, by version). A click on a line number picks
the line, Shift+click extends the pick, and "+" on a picked line comments on all of them; keys `c` (comment on picked
or selected lines), `]`/`[` (files), `v` (viewed); `@ui` `SplitPane`, `Comment`/`CommentThread`/`CommentComposer`.

On a phone (PDT-116; the ticket's Changes and a changes tab alike) the changed files are a screen of their own (name,
folder, comments, viewed, +/−, status), and a file opens its unified diff as the next screen, which adds a history
entry: Back (the browser's, the top bar's ←) or All files returns to the list. A swipe left or right (or ‹ ›) moves to
the next or previous file, stopping at either end; swipes that start in a text box or a thread don't count. Threads
show under their lines; a tap on a line number picks it, a tap on another extends the pick, a tap inside drops it,
and "+" (shown on the picked lines) opens the new comment in a bottom sheet with the lines quoted.
