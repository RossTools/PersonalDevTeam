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
  below 900px; unchanged runs collapse unless Whole file; lines wrap).

## Comments and feedback

- `review_comments` (migration 0019, deleted with the worktree): `GET|POST /api/worktrees/{id}/comments` (a root
  anchors to `side` old/new, `line`..`endLine` and `lineText`; a reply takes its root's anchor),
  `PATCH|DELETE /api/review-comments/{id}` (body and delete on drafts only, 409 `comment_sent`; `resolved` on roots).
  The browser re-finds moved lines by their text, else lists the thread as outdated (web/src/review/anchors.ts).
- `POST /api/worktrees/{id}/feedback` sends every draft as one message (pdt-core `review::feedback_message`) to the
  newest agent session of the attempt (else of the worktree) that can take it: a conversation's next user message, or
  a bracketed paste into a running terminal (409 `no_drafts`, `no_agent_session`, `session_not_running`); drafts
  become sent and the ticket gets a `feedback` event.
- `POST /api/attempts/{id}/approve` records a `review` event at the branch head (Approved while the head is
  unchanged; 409 `attempt_inactive` once the attempt ended).

## Merge

- `GET /api/attempts/{id}/merge` previews (`problem` names what blocks it, pdt-core `MergeProblem`); `POST` merges.
  Every refusal is 409 `merge_blocked` with a message (inactive, dirty, conflict, base_dirty, nothing,
  pr_not_mergeable, pr_queued, no_pr); a failing gh or git is 409 `merge_failed`.
- With an open PR (`attempt.pr`) it runs `gh pr merge --auto` (pdt-github `merge_pr`; the method from
  `merge_settings`, none with a merge queue; when GitHub says the PR is already clean it merges at once) and the merge
  check ends the attempt later.
- Otherwise a local `--no-ff` merge commit with the user's identity (in the checkout that has the base, which must be
  clean, else `merge-tree` + `commit-tree` + `update-ref`); it ends the attempt as merged, then the dialog runs
  `finishMerged` (Remove the worktree on, Delete the branch off) or just moves the ticket to Done.
- A project whose settings say Merge through pull requests (`project_settings.merge_via_pr`, `mergeViaPr` in the
  settings API, kept when a PUT leaves it out) never merges locally: no open PR is `no_pr`.
- `POST /api/attempts/{id}/ask-agent` asks the coder to fix a `conflict` (merge the base in) or `no_pr` (push and
  open a PR); 409 `nothing_to_ask`, `worktree_missing`.

## Web

The ticket tab's Changes (its section tab shows the diff size; the section last shown is remembered per ticket in
localStorage), the session header's Changes button (`ChangesButton`, with the diff size), and changes tabs
(`workspace_tabs.worktree_id`, `OpenTab.worktreeId`; only close) from a worktree `BranchChip`'s Review changes, the
project's worktree list and the palette. Viewed per browser (localStorage, by version); keys `c` (comment on selected
lines), `]`/`[` (files), `v` (viewed); `@ui` `SplitPane`, `Comment`/`CommentThread`/`CommentComposer`.
