# Phase 6 — Review: plan

Spec: docs/superpowers/specs/2026-10-19-review-design.md. Branch `pdt/PDT-3-phase-6-review`; one PR carries the spec,
this plan and the code.

**Goal:** the ticket tab's Changes section and a changes tab for any worktree: file list + CodeMirror diff
(split/unified, whole file), line comments as drafts, Send feedback into the coder session, Approve, Merge… (local,
or auto-merge of the PR on GitHub), then Done.

## Tasks

1. **pdt-git `review.rs`** (TDD, temp repos)
   - `review_base(worktree, base, fallback) -> String`: the newer of `merge-base HEAD <base>` and
     `merge-base HEAD origin/<base>`, else `fallback`.
   - `diff_files(worktree, merge_base) -> Vec<DiffEntry>`: `git diff -M --numstat -z <mb>` plus
     `git diff -M --name-status -z <mb>` for statuses, plus untracked files (`ls-files --others --exclude-standard`,
     counted as added lines, binary when they hold a NUL in the first 8 KB); versions from `git diff --raw` blob ids,
     with `git hash-object --stdin-paths` for working-tree files.
   - `file_at(worktree, rev, path) -> Option<Vec<u8>>` (`git show <rev>:<path>`, None when absent) and the working
     file read directly.
   - `merge_preview(repo, worktree, branch, base) -> Preview { commits, conflicts, base_checkout, base_dirty }`
     (`rev-list --count base..branch`, `merge-tree --write-tree --name-only --no-messages`).
   - `merge_local(repo, branch, base, message) -> Result<String /*commit*/, MergeError>`: in the base's checkout
     when one has it (clean; `merge --no-ff -m`; abort on conflict) or with `merge-tree` + `commit-tree` +
     `update-ref <ref> <new> <old>`.
2. **pdt-core `review.rs`**: `ChangedFile`, `Changes`, `FileSides`, `ReviewComment`, `NewReviewComment`,
   `ReviewCommentPatch`, `FeedbackResult`, `MergePreview`, `MergeProblem`, `MergeResult`, `feedback_message` (TDD);
   `pr::merge_method(allowed, queue)` (TDD). `EventKind::{Feedback, Review}`. `Tab.worktree_id`, `TabItem::Changes`.
3. **pdt-store**: migration 0019 (comments; tabs rebuild); `review.rs` (insert/list/get/update/delete/mark_sent);
   tabs accept a worktree.
4. **pdt-github**: `merge_settings(repo, base) -> MergeSettings { squash, merge, rebase, queue }` (GraphQL) and
   `merge_pr(repo, number, method: Option<&str>)` (`gh pr merge <n> --auto [--squash|…] -R repo`); FakeGithub.
5. **pdt-api `review.rs`**: routes in §4 of the spec; `feedback` reuses `conversations::send_message` (as the user)
   and `activity::type_message`; merge ends the attempt through the same code as the merge check (merged event,
   overlap/notice refresh) and records the event. Tabs: `open_tab` accepts `worktreeId`.
6. **Web, @ui**: `SplitPane`, `Comment`/`CommentThread` (+ gallery entries, tests).
7. **Web, review**: add `@codemirror/merge`; move the theme to `files/codeTheme.ts`; `review/queries.ts`,
   `anchors.ts` (TDD), `FileList.tsx`, `CodeDiff.tsx` (+ lazy wrapper), `ReviewBar.tsx`, `MergeDialog.tsx`,
   `ChangesView.tsx`.
8. **Web, wiring**: ticket tab's Changes; changes tab kind in workspaces (tab title "Changes · <branch>", icon);
   BranchChip menu, Settings worktree list and palette entries; ticket menu Merge….
9. **Docs**: AGENTS.md Review paragraph, docs/agent-guide.md (feedback reaches agents as a message), ROADMAP
   pointer untouched; tickets for follow-ups.
10. **Checks**: fmt, clippy, cargo test (commit gen), web typecheck/test/build; screenshots of the Changes view from a
    throwaway dev daemon with the demo seed.
