# Tickets, parts, Start, pull requests, areas

Phase 4 and later. Spec: docs/superpowers/specs/2026-10-18-tickets-design.md (parts:
2026-10-19-ticket-parts-design.md). Code: pdt-core `ticket.rs`, `rank.rs`, `parts.rs`, `pr.rs`, `overlap.rs`;
pdt-api `tickets.rs`, `start.rs`, `parts.rs`, `github.rs`, `overlap.rs`; pdt-git `merge.rs`; crates/pdt-github;
web/src/tickets. Review and merge are in [review.md](review.md).

## Tickets

- Per-project numbers that never repeat (`projects.next_ticket_number`; key `<project key>-<n>`), stages
  draft/ready/todo/in_progress/done/wont_do (Ready is refined but waiting in the backlog; To do is picked for the
  board), priority, labels (lower-case, no commas), areas (names from the project's `[[area]]` config; below), a
  fractional base-62 `rank` (pdt-core `rank`; a new ticket goes to the top of its stage unless `before`/`after` say
  otherwise), links `blocks`/`relates` (blocks cycles refused with 409 `link_cycle`), and an event timeline (comments
  are events). Search is FTS5 on title/description keyed by `tickets.seq`, plus exact key/number matches.
- API:
  - `GET /api/tickets` (`project`, `q`, `stage`/`priority`/`label` as comma-separated lists, `sort`
    rank|priority|updated|created, `blocked`, `conflicts`, `parent=<id>|none`, `hasParts`, `closedAfter`, keyset `cursor`,
    `limit`; returns `{items, nextCursor, total}`).
  - `POST /api/projects/{id}/tickets`, `GET|PATCH|DELETE /api/tickets/{id}`, `POST /api/tickets/{id}/rank`
    (`{before?, after?, to?: top|bottom}`), `POST /api/tickets/bulk` (all-or-nothing).
  - `POST|DELETE /api/tickets/{id}/links`, `GET /api/tickets/{id}/events`, `POST /api/tickets/{id}/comments`,
    `GET /api/projects/{id}/labels`.
- A workspace tab holds a session, a run, a ticket (`workspace_tabs.ticket_id`) or a worktree's changes, exactly one.
- Web: the Tickets view (rail item, Ctrl+] l) is the backlog (Draft and Ready unless the Stage filter says
  otherwise): a virtualised `DataList` paged by cursor (polls every 2 s, every 15 s past two pages), quick add (Enter
  adds a Draft above the first row when the list is a flat rank order; Shift+Enter opens the full form), filters
  (saved filters per browser in localStorage), bulk bar, rank by drag or Alt+↑/↓; a row opens the peek sheet
  (non-modal on desktop, full screen on phones), Open makes a ticket tab. The Board (Ctrl+] b) shows To do / In
  progress / Done (closed in the last 14 days, `closedAfter`); cards drag between columns on desktop, phones show one
  column at a time.
- scripts/import-roadmap.mjs imported the old docs/ROADMAP.md as tickets PDT-1…17 (done once; kept for reference).

## Parts (PDT-38)

A ticket that takes several PRs is a parent whose parts are child tickets (`tickets.parent_id`, one level; no ticket
types). Plan: docs/superpowers/plans/2026-10-19-pdt-38-ticket-parts.md.

- Rules in pdt-core `parts.rs` (TDD): a parent is in the same project, not itself, not a part, has no active attempt
  and is open, and the part has no parts (409 `invalid_parent`). A parent with parts can't be deleted (409
  `ticket_has_parts`; the FK is SET NULL, not RESTRICT, so removing a project still cascades) or closed while a part
  is open (409 `parts_open`, also in bulk unless the open parts close in the same call).
- A new part goes after the parent's last part (or right after the parent); `blockedByPrevious` links that last part
  as its blocker; `POST /api/tickets/{id}/parts` makes several in one go; `PATCH {parent: id | ""}` attaches or
  detaches; `parted` events on both tickets.
- Parents are never started: Start on one is 409 `ticket_has_parts` unless `next: true`, which starts its next part
  (first open part by rank, not started or merged, no open blockers; else 409 `no_next_part` saying why).
- A part's first prompt has a "Part of" block (the parent's key, title and description, and one line per other part;
  the parent's description is cut first to fit 8 KB). A part's Start moves its parent to In progress (system `moved`)
  if it is not there yet; the parent moves to Done like any ticket once every part is closed. A part's Start works
  like any ticket's (its own workspace, named after the part).
- Every ticket response carries `parent {id,key,title}` and, on parents, `parts` (`PartsProgress`: counts and one
  `PartState` per part, done/merged/review/progress/todo/backlog; computed in pdt-api `parts.rs` with the in-memory PR
  state); full tickets carry `partList`.
- Web: the Board has a lane per parent (the lane header is the parent's card, with `PartsStepper` from web/src/common
  and Start next part), the ticket tab/peek a Parts section, part rows a parent chip, and the ticket menu "Make into
  parts…" (from a Scope/Parts list in the description).

## Start (Phase 4b)

`POST /api/tickets/{id}/start`, crates/pdt-api/src/start.rs. Conversation-specific options (`sessionMode`,
`fromSession`) are in [conversations.md](conversations.md#tickets-5d).

- Checks open blockers and overlaps (`mode: check` → 409 `start_blocked`; `force` starts anyway, recorded on the
  `started` event).
- Makes branch `pdt-<KEY>-<n>-<slug>` (no `/`, which some third-party tools break on; tickets started before
  PDT-74 keep their `pdt/…` branches) and its worktree (adopting one that exists; a new branch starts from
  `origin/<base>`, fetched first, when that is ahead of the local base), an active attempt (one per ticket, 409
  `ticket_active`), moves the ticket to In progress, and starts an agent session (`sessions.attempt_id`; titled with
  the ticket title) whose first prompt is the ticket (`NewSession.initialPrompt`, appended as the last argv element on
  the creating launch only; capped at 8 KB). On Windows, when the agent is a `.cmd` shim run through cmd.exe, the
  prompt goes to `<data dir>/sessions/<id>/prompt.md` and the agent gets a short prompt naming that file.
- Tabs: the ticket tab, then the session tab after it (or both in a new free-form workspace `<KEY>-<n>`). Sessions
  carry `ticket {id, key}` (sidebar and tab chips). Session titles never name the agent (a new session is titled with
  its branch, or the project for the main checkout); tabs, agent rows, the palette and the attempt list show it with
  `SessionKindIcon` (web/src/common).
- `noSession: true` (PDT-72; the Start dialog's "No session") starts no agent session: the branch, worktree, attempt
  and only the ticket's tab (still in its new workspace with `newWorkspace`); the result's `session` is `null`.
- `POST /api/attempts/{id}/abandon` ends an attempt (worktree kept); moving a ticket to Done ends its attempt as
  merged, to Won't do as merged if detected, else abandoned.

## Merge detection and finishing

- pdt-git `merge.rs`, every 60 s in pdtd's sweep loop and on `POST /api/tickets/{id}/check-merge` (the ticket tab and
  peek call it on open), fetching each project's base once per tick (`GIT_TERMINAL_PROMPT=0`, only with an `origin`).
- An attempt is merged when its head (or `attempts.last_head` once the branch is gone) is an ancestor of
  `<base>`/`origin/<base>` or its whole patch is already there (squash: `commit-tree` + `cherry`), both only once a
  check has seen the branch hold commits not in the base (`attempts.own_commits`, sticky; PDT-45: a branch that only
  fast-forwarded to a newer main is not merged), or GitHub says its PR merged at that head (`attempt.pr`; the merge
  queue squashes onto a newer base, which the patch check can miss).
- A merged attempt shows the banner "Move to Done and remove the worktree" (web/src/tickets/actions.ts
  `finishMerged`; in the ticket peek, and above every tab of the ticket's workspace while it is on screen, PDT-71,
  else in the ticket tab): after one question it stops every session and run in the worktree, the attempt's sessions and
  everything in the workspace Start made for the ticket (`attempts.workspace_id`), removes the worktree, deletes that
  workspace's sessions and then the workspace with its tabs (`DELETE /api/workspaces/{id}?closeTabs=true`), the
  ticket's among them.
- Meanwhile it pulls the base branch (PDT-56; `POST /api/projects/{id}/pull {branch?}`, pdt-git `fast_forward`): fetch
  `origin/<base>` and fast-forward only, in whichever checkout has the branch (`git merge --ff-only`, which refuses
  rather than touch uncommitted changes) or just the ref; no origin, no branch, diverged, blocked or a failed fetch
  come back as `outcome` (200), never an error, and the toast says what happened.

## Pull requests and CI (PDT-25)

crates/pdt-github, pdt-api `github.rs`, pure rules in pdt-core `pr.rs`.

- Every 60 s (before the merge check) and when a ticket is opened (`check-merge`), pdtd reads each active attempt's PR
  through the user's `gh` (`PDT_GH_BIN`, else the login shell's PATH, then pdtd's own, then the standard install
  folders; tests get `pdt_github::Unavailable` or `testing::FakeGithub`, and pdtd's own tests set `PDT_GH_BIN` to
  nothing). Only projects whose `origin` is on GitHub.
- One GraphQL query reads state, draft, mergeable, `BEHIND`, auto-merge, merge-queue entry and newest merge-queue
  event; then `gh run list --branch` gives CI on the PR head (the newest run of each workflow; the fine-grained token
  cannot read check runs) and, while queued or just out of the queue, `gh run list --event merge_group` gives the runs
  on `gh-readonly-queue/<base>/pr-<n>-…`; failed job names come from `gh run view --json jobs` (cached per run
  attempt).
- The result is in memory and on every ticket response as `attempt.pr` (Attempt section: `PrDetails`; board card:
  `PrBadge`, web/src/tickets/PullRequest.tsx). An attempt that ended while its PR was last seen open has the PR read
  again each tick for an hour after (no news), and the web shows a merged attempt's still-open PR as Merged (PDT-59:
  git's merge check can end the attempt while GitHub still says queued).
- News: CI failed (per run attempt), a conflict (per head), the merge queue removing the PR without merging it (per
  `RemovedFromMergeQueueEvent`, read in the same query, so a PR queued and dropped between two looks or across a
  restart still counts; not when a person dequeued it, it was queued again or auto-merge is back on), merged (also
  when git's merge check ends the attempt). Each is typed into the attempt's newest running agent session as one
  bracketed paste plus Enter once it is idle and nobody has typed in it for 20 s (a 5 s delivery loop), then recorded
  as a `pr` event (`told`, `sessionId`); with no running agent session it is recorded as not told. The `pr` events'
  `key`s make each notice news once, across restarts too. Conversations: see
  [conversations.md](conversations.md#tickets-5d).

## Areas and overlap (Phase 4c)

Tickets spec §7; the config format is in docs/run-config.md "Areas".

- `[[area]]` entries (`name`, `paths` globs, `exclusive`) in the main checkout's `.pdt/project.toml` plus host
  overrides (`GET /api/projects/{id}/areas`; areas not in the config are ignored). PDT's own .pdt/project.toml
  declares its areas (migrations is the exclusive one).
- A ticket's overlap is always against the project's tickets with an active attempt: shared areas (planned) and, when
  both are active, shared changed files (actual; pdtd's 30 s `refresh_overlaps` runs `pdt_git::changed_files` per
  active attempt and keeps them in memory; new file overlaps add an `overlap` event once per pair and file set). A
  shared exclusive area or a file inside one is a `conflict`, else `overlap`.
- Computed in pdt-api `overlap.rs` (pure rules in pdt-core `overlap.rs`) and applied to every ticket response:
  `overlap`, `overlapWith`, `clearToStart` on summaries, `overlaps` on the full ticket; `GET
  /api/tickets?conflicts=true|false`.
- Start with `mode: check` refuses (`start_blocked`) on blockers or overlaps; Wait is `PATCH {waiting: true}` (top of
  its stage; cleared by Start and by closing).
- The parser skips top-level sections it does not know (listed in `effective.ignored`, noted in the project's
  settings) so a file written for a newer PDT still runs on an older stable; unknown keys inside
  `[[run]]`/`[[test]]`/`[[area]]` are still refused, and so are unknown sections in host overrides. A PDT older than
  PDT-21 still refuses unknown sections, so a new section in this repo's file must wait until the stable has this.
