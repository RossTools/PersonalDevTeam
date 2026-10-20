# Personal Dev Team — Phase 6: Review Design

Status: written by the agent working PDT-3 (2026-10-19) from the main spec §7.3 and the UX spec §8.3; the decisions
below are the agent's, for the user to review on the PR. Plan: docs/superpowers/plans/2026-10-19-phase-6-review.md.

## 1. Goal

A review loop driven by the user, inside PDT: see an attempt's changes, comment on lines, send the comments to the
coding agent, approve, merge. Done when a ticket goes Start → the agent works → review the diff → send feedback → the
agent fixes → merge → Done without leaving PDT.

Not in this phase: agent reviewers and their comments (`add_review_comment`, personas; Phase 7), the pipeline's
Review/Accept stages (Phase 7), GitHub review state (PDT-12), commenting on a PR on GitHub, editing files in the diff.

## 2. Decisions

1. **What the diff is.** For a worktree on branch B: the working tree (committed, staged, unstaged and untracked
   files that are not ignored) against the merge base of `HEAD` and the base branch. The base branch is the attempt's
   `base_branch`, or the project's base branch for a worktree with no attempt. When `origin/<base>` exists its merge
   base is used if it is newer than the local base's (Start branches from `origin/<base>` when that is ahead, so a
   stale local `main` would otherwise drag main's own commits into the diff). With no common ancestor the attempt's
   `base_commit` is used (as `changed_files` does). The main checkout has no Changes view (it has no base to compare).
2. **The diff is drawn in the browser.** The API lists the changed files and serves each file's two texts (at the
   merge base, and in the working tree); `@codemirror/merge` computes and draws the diff, split (`MergeView`) or
   unified (`unifiedMergeView`), with syntax highlighting. Long lines wrap (the split halves are narrow, and threads
   under a line then fit the pane). Unchanged runs collapse (`collapseUnchanged`, 3 lines of margin) and expand on a
   click; **Whole file** turns collapsing off. A thread on a line inside a collapsed run shows once the run expands. A side over 1 MB, or a binary file, is not
   served (`tooLarge` / `binary`) and the view says so.
3. **Comments belong to the worktree.** `review_comments.worktree_id`, deleted with the worktree, so a changes tab on
   any worktree (attempt or not) can be reviewed. A comment anchors to a side (`old` for removed lines, else `new`),
   a line range and the text of those lines when it was made. When the file changes, the browser re-finds the lines
   (same place, else the nearest exact copy of the text); a comment whose lines are gone is **outdated** and listed
   above the file's diff. Threads: a root comment plus replies (`parent_id`, one level). Resolve applies to the
   root. No agent comments yet (the `author` column says `user`; Phase 7 adds personas).
4. **Drafts and Send feedback.** Every new comment or reply is a draft (dashed outline, "Draft" badge) that can be
   edited or deleted. **Send feedback** puts all of the worktree's drafts into one message (§5), sends it to the
   coder session and marks them sent (`sent_at`, `session_id`). Sent comments can't be edited (they are in the
   agent's history); they can be resolved and replied to. Resolved threads collapse.
5. **The coder session.** The newest agent session that can take a message now among the attempt's sessions
   (`sessions.attempt_id`) and the worktree's sessions that belong to no attempt (a chat started there by hand); for a
   worktree with no attempt, among its sessions. A conversation always can: it gets the message as the user's next message (queued if a turn runs;
   a parked or stopped conversation reopens). A terminal session can while its PTY runs, and gets one bracketed paste
   plus Enter. Agent sessions but none that can: `409 session_not_running` (restart it). None at all:
   `409 no_agent_session`. The ticket timeline records a `feedback` event (count, session).
6. **Approve** is a record, not a gate (there is no pipeline yet): a `review` ticket event with `{verdict:
   "approved", head}`. The review bar shows "Approved" while the branch head is still that commit. Only for attempts.
7. **Merge… has two paths.**
   - **The attempt has an open pull request** (`attempt.pr`, PDT-25): merging is GitHub's job. The dialog shows the PR,
     its CI and mergeability, and **Merge on GitHub** runs `gh pr merge <n> --auto` (with `--squash`, `--merge` or
     `--rebase`, the first the repository allows, unless its base branch has a merge queue, which picks the method
     itself). The attempt stays active; the merge check ends it when GitHub merges (as today), and the existing
     merged banner offers Move to Done.
   - **No open PR:** a **local merge** into the base branch with a merge commit (`--no-ff`, message
     `Merge <branch>: <ticket title>`). Made without touching any checkout when no checkout has the base branch
     checked out (`git merge-tree --write-tree`, `commit-tree`, `update-ref` with the old value); otherwise in that
     checkout (`git merge --no-ff`), which must be clean. Refused with an `Alert` (no Merge button) when: the
     worktree has uncommitted changes (commit or discard first), the merge conflicts (the files are listed;
     **Send to agent** sends the agent a message to merge the base in and resolve them), the checkout holding the
     base branch is dirty, or there is nothing to merge (0 commits). A local merge ends the attempt as merged (a
     `merged` event), then the dialog's **Remove the worktree** (on) and **Delete the branch** (off) run the existing
     finish flow (`finishMerged`: stop the attempt's sessions and runs after one question, remove the worktree and the
     ticket's workspace, pull the base) and move the ticket to Done. With Remove the worktree off, the ticket still
     moves to Done and the worktree stays.
   Merge… is offered for active attempts only, from the review bar and the ticket menu.
8. **Viewed** is per browser (localStorage), keyed by worktree, path and a version of the file (git's blob id of
   both sides); a file that changes after it was marked becomes unviewed. The checkbox is in the file's header (as on
   GitHub: a control inside a tree row fights the tree's own keyboard and press handling), the list counts them
   ("4 / 9 viewed") and the tree dims them with a check.
9. **Where Changes opens.** The ticket tab's Changes section (its active attempt's worktree; the last attempt's while
   the worktree still exists). And a **changes tab** for any worktree: a new tab kind (`workspace_tabs.worktree_id`,
   one tab per worktree), opened from the worktree's `BranchChip` menu ("Review changes"), the project's worktree
   list in Settings and the palette ("Review changes of <branch>"). Closing a changes tab just closes it.

## 3. Data

Migration `0019_review.sql`:

```sql
CREATE TABLE review_comments (
    id          TEXT PRIMARY KEY NOT NULL,
    worktree_id TEXT NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
    parent_id   TEXT REFERENCES review_comments(id) ON DELETE CASCADE,  -- a reply's thread root
    path        TEXT NOT NULL,           -- checkout-relative, '/'-separated (a reply copies its root's anchor)
    side        TEXT NOT NULL,           -- old | new
    line        INTEGER NOT NULL,        -- 1-based first line on that side
    end_line    INTEGER NOT NULL,        -- last line (= line for one line)
    line_text   TEXT NOT NULL,           -- the anchored lines when the comment was made
    body        TEXT NOT NULL,
    author      TEXT NOT NULL DEFAULT 'user',
    sent_at     INTEGER,                 -- null: a draft
    session_id  TEXT,                    -- where it was sent (no FK: the session may be removed)
    resolved_at INTEGER,                 -- roots only
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX review_comments_by_worktree ON review_comments (worktree_id, path);
```

plus a rebuild of `workspace_tabs` adding `worktree_id TEXT UNIQUE REFERENCES worktrees(id) ON DELETE CASCADE`, with
the CHECK counting four columns. `EventKind` gains `feedback` and `review`.

## 4. API

All JSON camelCase, types in pdt-core `review.rs` (ts-rs).

- `GET /api/worktrees/{id}/changes` → `Changes { worktreeId, branch, base, mergeBase, head, dirty, files:
  ChangedFile[], attemptId, ticketId, attemptActive, approved }`; `ChangedFile { path, oldPath?, status: added|modified|deleted|renamed, untracked, binary,
  added, removed, version }` (sorted by path; `version` changes when either side does). 404 for an unknown worktree,
  409 `worktree_missing` when its folder is gone.
- `GET /api/worktrees/{id}/changes/file?path=&oldPath=` → `FileSides { path, old: string|null, new: string|null,
  binary, tooLarge }` (`old` null for an added file, `new` null for a deleted one). Paths stay inside the checkout
  (`.git` refused), as the Files view's.
- `GET /api/worktrees/{id}/comments` → `ReviewComment[]` (oldest first).
- `POST /api/worktrees/{id}/comments` `{ path, side, line, endLine?, lineText, body, parentId? }` → the draft (a reply
  takes its root's anchor; replying to a reply attaches to the root).
- `PATCH /api/review-comments/{id}` `{ body?, resolved? }`: `body` only on drafts (409 `comment_sent`); `resolved`
  only on roots.
- `DELETE /api/review-comments/{id}`: drafts only (409 `comment_sent`); deleting a root deletes its replies.
- `POST /api/worktrees/{id}/feedback` → `{ sessionId, sent }` (409 `no_drafts`, `no_agent_session`,
  `session_not_running`).
- `POST /api/attempts/{id}/approve` → the `review` event.
- `GET /api/attempts/{id}/merge` → `MergePreview { method: local|github, branch, base, commits, problem?:
  MergeProblem, conflicts: string[], baseCheckout?, pr?: { number, url, queue } }`; `MergeProblem = inactive | dirty |
  conflict | base_dirty | nothing | pr_not_mergeable | pr_queued`.
- `POST /api/attempts/{id}/merge` → `{ outcome: merged|queued, commit? }` (409 `merge_blocked` with the problem,
  `merge_failed` when git or `gh` failed).
- `POST /api/attempts/{id}/conflicts` → `{ sessionId, sent: 0 }`: Send to agent for a conflicting merge (409
  `no_conflict` otherwise).
- Tabs: `NewTab`/`Tab` gain `worktreeId`.

## 5. The feedback message

pdt-core `review::feedback_message` (TDD), plain text, threads in file then line order:

```
Review feedback on pdt/PDT-3-phase-6-review (3 comments). Address each one, then say briefly what you changed.

1. crates/pdt-git/src/changes.rs:19-21
   > pub fn changed_files(…
   > …
   Name the fallback.

2. web/src/x.tsx:40 (removed lines)
   > const a = 1;
   Why was this removed?

3. Reply on docs/a.md:3 ("Use the API path"):
   Done? It still says the repo path.
```

Quoted lines are capped at 6 per comment (then `> …`); a reply quotes its thread's first comment, cut to 80 chars.
The whole message is capped at 32 KB (later comments become "…and N more comments, see the Changes view").

## 6. Web

- `web/src/review/`: `ChangesView` (review bar, `SplitPane` of the file tree and the diff), `FileList` (a flat
  `TreeView` grouped by folder: status letter, `DiffStat`, comment count, Viewed checkbox), `CodeDiff` (lazy; the
  CodeMirror merge view, line gutter "+", `c` on a selection, thread widgets under their lines), `ReviewBar`,
  `MergeDialog`, `useReview` queries.
- `@ui` gains `SplitPane` (two panes, a draggable divider with keyboard steps, size remembered per id) and
  `CommentThread` / `Comment` (author, time, Markdown body, Draft badge and dashed outline, Edit/Delete on drafts,
  Reply with a `Textarea`, Resolve / Reopen).
- The CodeMirror theme moves from `files/CodeViewer.tsx` into `files/codeTheme.ts`, shared by both.
- Split | Unified is a `SegmentedControl` (remembered per browser; unified below 900px whatever the choice). Keys in
  the diff: `c` comments on the selected lines, `]`/`[` next/previous file, `v` toggles Viewed.
- Phones: a Files button in the review bar swaps the diff for the file list; unified only; the bar wraps.

## 7. Testing

- pdt-git: `review_base`, `diff_files` (numstat with renames, untracked, binary), `file_at`, `merge_preview`,
  `merge_local` (clean, conflict, base checked out clean/dirty, nothing to merge), against temp repos.
- pdt-core: `feedback_message`, `merge_args` (gh method choice).
- pdt-store: comments CRUD, cascade with the worktree, the new tab kind.
- pdt-api: changes and file endpoints, comments rules, feedback into a fake conversation and a terminal, approve,
  merge (local; GitHub via `FakeGithub`).
- Web: anchor relocation (pure), FileList, ReviewBar, comment drafting/sending against `stubApi`, MergeDialog states.
  CodeMirror itself is exercised in jsdom only lightly (mounts, shows a chunk); the rest by hand in Run → pdt.
