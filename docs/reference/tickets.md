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
  - A comment's actor: PDT's UI sends `{body, by: "user"}`; a comment without `by` (any other caller, i.e. an agent's
    curl) is `agent`, shown as "Agent" in the timeline; `by: "system"` is refused (400). Workflows write `system`
    comments. So an agent's comment never passes for the user's, and workflows that react to the user's comments
    (below) don't fire on it.
- A workspace tab holds a session, a run, a ticket (`workspace_tabs.ticket_id`) or a worktree's changes, exactly one.
  A ticket tab's place is its active attempt's worktree (else the main checkout): a new session or run from it (the
  new-tab chooser, palette, Run) and the folder view default to the ticket's branch.
- Web: the Tickets view (rail item, Ctrl+] l) is the backlog (Draft and Ready unless the Stage filter says
  otherwise): a virtualised `DataList` paged by cursor (polls every 2 s, every 15 s past two pages), quick add (Enter
  adds a Draft above the first row when the list is a flat rank order; Shift+Enter opens the full form), filters
  (saved filters per browser in localStorage), bulk bar, rank by drag or Alt+↑/↓; a row opens the peek sheet
  (non-modal on desktop, closed by a click elsewhere; closing it or showing another ticket asks first over an unsaved
  description edit; on phones the full ticket screen below, whose ← is Back), Open (which also asks) makes a ticket tab. A description edit is measured against the
  text it started from: a change made meanwhile (an agent's rewrite) shows a warning, with Load the new version. The Board (Ctrl+] b) shows To do / In
  progress / Done (closed in the last 14 days, `closedAfter`); cards drag between columns on desktop, phones show one
  column at a time.
- Phones (PDT-118): the top bar's title (`Tickets · <KEY> ▾`) opens the projects and saved filters; under it
  Backlog | Board, ⌕ (shows the search box) and ⚲ (the filters in a bottom sheet, with Save filter; the button shows
  how many are set). Rows are two lines: key and title, then the agent's state, the pipeline stage (or stage),
  badges, labels and priority. No drag on touch: rows rank and cards move from their long-press menu. The row menu's
  Select starts bulk mode (checkboxes, a tap toggles, `BulkActionBar` docked at the bottom), which ends once nothing
  is selected. A floating ＋ (the app's only floating button) opens New ticket; Import from Jira is under ⋯. The
  Board picks its column from scrolling chips with counts (the desktop's columns, pipeline stages included) or a
  swipe sideways (not from the screen's left edge, the system Back gesture).
- Phones, the ticket screen (PDT-119, `/tickets/:key`, in place of the peek): the title (tap to edit), stage ·
  priority · the agent's state with the badges, the stage stepper (scrolls sideways), then Overview | Conversation |
  Changes. The Overview is one column: Details, Attempt (or Sessions), Links and Overlaps fold at the top with one-line
  summaries, then Description (✎ edits in place), Parts and Activity. While the title or description is edited the
  top bar shows Cancel and Save in place of ← and its buttons. The bottom bar has Comment… (the composer in a bottom
  sheet) and the stage's primary action: Draft/Ready → Refine · Start, To do → Start, In progress → Accept… while the
  ticket's Accept gate waits in the Inbox (the Inbox's merge confirmation: PR, CI, diff size) else Open work (the
  session working on it, else the attempt's newest), Done and Won't do → none. Move to, Priority, Start next part,
  Make into parts…, Merge…, Abandon attempt… and Delete… are under ⋯.
- **Pipeline stage** (Phase 7c): `pipelineStage` (`refine | code | review | qa | accept | merge | merge_blocked`,
  `tickets.pipeline_stage`)
  is where a pipeline workflow has the ticket: `refine` while Draft or Ready, the others while In progress. Agent
  steps, gates, waits and `attempt.merge` with a `pipelineStage` set it when they start; any change of stage clears it. Every ticket response
  also carries `agent` (`TicketAgent`, computed in memory by pdt-api `scheduler::ticket_agents`): who owns its
  workflow step now (`persona`, `sessionId`, `busy` while its session runs a turn) and `state` `working | asking |
  stalled | yours` (a manual step or a gate) `| queued` (waiting for a slot). The backlog shows a Refining badge; the
  Board splits In progress into Code | Review | QA | Accept | Merge once any In progress card has a stage (none:
  Code; `merge_blocked` under Merge, with a Merge blocked badge), and
  each card shows the owner line ("Reviewer · working", "Your turn", "… · waiting for a slot").
- **Refine**: `POST /api/tickets/{id}/refine` (`{by: "user"}` from the web; ticket menu and the ticket tab's menu) on
  a Draft or Ready ticket (else 409 `ticket_not_refinable`) sets `pipelineStage: refine` and logs
  `ticket.refine_requested`; 409 `no_refine_workflow` when no enabled workflow for the project has that trigger, and
  `refine_running` while one already refines it.
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
  and Start next part; the lane of tickets that are not parts stops at half the screen, each column scrolling on its
  own, so a long Done doesn't push the lanes off screen), the ticket tab/peek a Parts section, part rows a parent chip, and the ticket menu "Make into
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
  the creating launch only; capped at 8 KB). The prompt (pdt-core `start_prompt`) is the key and title, the
  description, the ticket's comments (oldest first, quoted), the "Part of" block for a part, and the branch line
  (which also says to keep to this ticket: anything unrelated gets its own ticket, after asking the user).
  Comments get only the room left: none when a description had to be cut, else the oldest are dropped first (with a
  note saying how many) and a newest comment too long for the room is cut. On Windows, when the agent is a `.cmd` shim run through cmd.exe, the
  prompt goes to `<data dir>/sessions/<id>/prompt.md` and the agent gets a short prompt naming that file.
- Tabs: the ticket tab, then the session tab after it (or both in a new free-form workspace `<KEY>-<n>`). Sessions
  carry `ticket {id, key}` (sidebar and tab chips): their attempt's, else the ticket a workflow step started them on
  (`sessions.ticket_id`: a Reviewer, QA or Refiner, which has no attempt). A ticket's `sessions` lists both kinds,
  newest first (`attemptId` null for a step's, `persona` for any step's); the overview's Attempt section lists them
  (a Sessions section before any attempt) and the board card's dot rolls up the attempt's and the steps'. Session titles never name the agent (a new session is titled with
  its branch, or the project for the main checkout); tabs, agent rows, the palette and the attempt list show it with
  `SessionKindIcon` (web/src/common).
- The project's running-attempt cap (project settings `maxRunningAttempts`, Phase 7c): with that many active
  attempts, `mode: check` refuses (`start_blocked`, "would go over the project's cap …"); `force` starts anyway
  (`overCap` on the `started` event).
- **Auto-start** (project settings `autoStart`: `off`, `todo`, `ready`; pdt-api `scheduler.rs`, every 10 s in pdtd,
  `schedule_tickets` in tests): while a project is under its cap (one when it has none) it starts its next To do
  ticket by rank (then Ready with `ready`), skipping parents, tickets that ever had an attempt, blocked ones, `conflict`
  overlaps and waiting tickets that aren't clear (pdt-core `policy::pick_next`). It starts in a new workspace with the
  host's default agent, view and ticket permission mode, or with no session when an enabled workflow for the project
  has an `attempt.started` trigger (the Pipeline's Coder step starts one, its tab after the ticket's,
  then the Reviewer's and QA's); the `started`
  event has `auto: true`. A failed start is logged and skipped until the ticket changes; a start with its own session
  waits for a slot under the host's cap.
- A Start whose session cannot be made is undone: the attempt and its timeline events go and the ticket's stage and
  pipeline stage are as they were (the worktree stays, for the next Start to adopt). Workflows only hear of a Start
  (`ticket.stage_changed`, `attempt.started`) once its session exists, so an undone one runs none.
- `noSession: true` (PDT-72; the Start dialog's "No session") starts no agent session: the branch, worktree, attempt
  and only the ticket's tab (still in its new workspace with `newWorkspace`); the result's `session` is `null`.
- `POST /api/attempts/{id}/abandon` ends an attempt (worktree kept); moving a ticket to Done ends its attempt as
  merged, to Won't do as merged if detected, else abandoned, once the move went through (a PATCH or bulk move that is
  refused ends nothing; a bulk move ends each ticket's attempt on its own).
- **Personal projects** (Phase 7b, [workspaces.md](workspaces.md#projects-and-git-workspaces-spec-6)): Start makes no
  branch or worktree. The attempt has an empty branch and no worktree, and the session runs in the ticket's folder
  `<project folder>/<KEY>-<n>` (made on Start, kept across attempts: it is the work's output); the prompt says to work
  in this folder. Start from a chat is refused there (409 `personal_project`). Merge detection and the PR poller skip
  these attempts; moving the ticket to Done ends the attempt.
- Agent steps record their outcome on the timeline (`outcome`: `{outcome, summary, persona, personaVersion, by,
  sessionId, attemptId, comments, workflowId, workflowName, runId}`, actor `agent`, or `user` when taken over from the
  Inbox) and a step's new session as a `session` event with its persona. Comments an agent adds through pdt-mcp have
  actor `agent` (`data.sessionId`, `data.persona`); the built-in "Ticket comments to the agent" only forwards the
  user's.

## Merge detection and finishing

- pdt-git `merge.rs`, every 60 s in pdtd's sweep loop and on `POST /api/tickets/{id}/check-merge` (the ticket tab and
  peek call it on open), fetching each project's base once per tick (`GIT_TERMINAL_PROMPT=0`, only with an `origin`).
- An attempt is merged when its head (or `attempts.last_head` once the branch is gone) is an ancestor of
  `<base>`/`origin/<base>` or its whole patch is already there (squash: `commit-tree` + `cherry`), both only once a
  check has seen the branch hold commits not in the base (`attempts.own_commits`, sticky; PDT-45: a branch that only
  fast-forwarded to a newer main is not merged), or GitHub says its PR merged at that head (`attempt.pr`; the merge
  queue squashes onto a newer base, which the patch check can miss).
- Ending an attempt as merged logs `attempt.merged` (with `via`: `detected` by this check, `merged_here` by a local
  Merge…, `closed` when the ticket was moved to Done or Won't do) for workflows (docs/reference/workflows.md).
- Finishing is one server command, `POST /api/tickets/{id}/finish {stopRunning, discardChanges, deleteBranch,
  keepWorktree}` (pdt-api `finish.rs`, PDT-76). It checks first and changes nothing when something would be lost:
  `outcome: blocked` with `blocked {sessions, runs, dirty}` for sessions or runs still going (unless `stopRunning`)
  or uncommitted changes (unless `discardChanges`). Otherwise it moves the ticket to Done, ends its active attempt (as merged, `via: closed`), cancels the
  ticket's workflow runs still going (but the run finishing it and the runs that called that one; error "cancelled:
  the ticket was finished"), stops every session and run
  in the worktree, the attempt's sessions and everything in the workspace Start made for the ticket
  (`attempts.workspace_id`), removes the worktree (and the branch with `deleteBranch`; `branchNote` when it stayed),
  closes the ticket's tab elsewhere, deletes that workspace with its tabs, and pulls the base branch (`pull`).
  `keepWorktree` only moves to Done (ending the attempt and cancelling the runs as above) and pulls (`outcome: kept`). The route runs it in a task of its own, so it
  completes when the caller goes away first (PDT-97: an agent finishing its own ticket is stopped by it).
- A merged attempt shows the banner "Move to Done and remove the worktree" (once Done with the worktree still there,
  kept or left behind, "Remove the worktree"; web/src/tickets/actions.ts
  `finishMerged`; in the ticket peek, and above every tab of the ticket's workspace while it is on screen, PDT-71,
  else in the ticket tab). It calls the command, asks once to stop what still runs and once to discard uncommitted
  changes when the command says so, and calls it again; declining moves the ticket to Done and keeps the rest.
- Merge… on a pull request can ask to finish once it merges (`attempts.finish_on_merge`, PDT-75): the built-in
  workflow "Finish merged tickets" then calls the command (stopping what runs, never discarding changes; a dirty
  worktree leaves the banner for the user). Its runs are listed at `GET /api/workflow-runs?ticket=<id>`, and the
  `moved` event names the workflow and run.
- The pull (PDT-56; also `POST /api/projects/{id}/pull {branch?}`, pdt-git `fast_forward`): fetch
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
- The PR is the branch's newest open one, else its newest, but a closed or merged PR opened before the attempt started
  (`createdAt`) is not the attempt's: a ticket reopened and started again on its old branch name has no PR (no merge
  from it, no news) until one is opened for it. A PR still open on the branch is the branch's, whoever opened it.
- The result is in memory and on every ticket response as `attempt.pr` (Attempt section: `PrDetails`; board card:
  `PrBadge`, web/src/tickets/PullRequest.tsx). An attempt that ended while its PR was last seen open has the PR read
  again each tick for an hour after (no news), and the web shows a merged attempt's still-open PR as Merged (PDT-59:
  git's merge check can end the attempt while GitHub still says queued).
- News is logged as events (PDT-76; docs/reference/workflows.md): CI failed (`ci.run_failed`, per set of failed run
  attempts), CI passed (`ci.run_passed`), a conflict (`pr.conflict`, per head), the merge queue removing the PR
  without merging it (`pr.queue_left`, per `RemovedFromMergeQueueEvent`, read in the same query, so a PR queued and
  dropped between two looks or across a restart still counts; not when a person dequeued it, it was queued again or
  auto-merge is back on), merged (`pr.merged`, and `attempt.merged` when the merge check ends the attempt), and behind
  its base while it waits to merge with auto-merge on and no merge queue (`pr.behind`). Each carries the `notice` PDT
  would type. The built-in workflow "PR notices" (on by default) sends it with `session.send`: typed into the
  attempt's newest running agent session as one bracketed paste plus Enter once it is idle and nobody has typed in it
  for 20 s, then recorded as a `pr` event (`told`, `sessionId`, `workflowId`, `runId`); with no running agent session
  it is recorded as not told, and a CI failure, conflict, queue drop or falling behind goes to the user instead
  (PDT-139): a ticket comment with the notice and an Unheard notices item in the Inbox, once per attempt and notice
  key, which closes once the PR is fixed (mergeable, CI passed, back in the queue, caught up), merged or closed, or the
  attempt ends ([workflows.md](workflows.md#inbox)). Each notice key is sent once per attempt, across restarts too. Turning the workflow off
  stops the notices. Conversations: see [conversations.md](conversations.md#tickets-5d).
- A PR that was set to merge (PDT-104) is handled further: its CI failure, conflict, queue drop or falling behind
  becomes an agent step (the attempt's Coder session, or a new one when it has ended) that fixes it and pushes, with a
  Reviewer first when the agent calls the fix large; then auto-merge goes back on (`attempt.merge` again), and a CI
  pass with auto-merge off turns it on too. After 3 tries per attempt PR notices comments on the ticket and only tells
  the agent again. "Set to merge" (`mergeRequested` on the events) is kept per attempt
  (`attempts.merge_requested`): Merge… / `attempt.merge` turning auto-merge on (never a refused merge, PDT-124), or the poller seeing auto-merge or the
  merge queue; a merge queue drop counts too, a person dequeuing it does not, and a person turning auto-merge off
  (on at the last look, off now, not dropped by the queue) clears it, so it isn't turned back on. A workflow's merge
  refused for a conflict or uncommitted changes (`attempts.merge_refused`, `mergeRefused` on the events, PDT-139) gets
  the same fix step for the PR's later failures, but a CI pass alone never turns auto-merge on for it. A failure while an
  agent already fixes the attempt (checked as the run gets there) is told to that agent instead of a second fix.
  Details: [workflows.md](workflows.md) "Workflows".
- A user's comment on a ticket with an active attempt goes to its agent session the same way (built-in "Ticket comments
  to the agent"; an agent's own comments, actor `agent`, are not sent); Start already puts the earlier comments in the
  first prompt.

## Jira and Buildkite (Phase 7e-2, PDT-85)

For a work project whose issues are in Jira Cloud and whose CI is Buildkite. pdt-core `jira.rs`, `buildkite.rs`;
pdt-api `jira.rs`, `buildkite.rs`; the sensors and actions are in [workflows.md](workflows.md) "Sensors and webhooks".

- **The link** is host settings, not `.pdt/project.toml` (work repos may not take a `.pdt/` folder): the project's
  settings page, section Jira and Buildkite, names a `jira` connection, a JQL scope (`project = ABC`) and the done
  transition or status (default `Done`), and a `buildkite` connection with the pipelines (slugs) that build its
  branches. API: `GET/PUT /api/projects/{id}/settings` `jira` `{connection, scope, done}` and `buildkite`
  `{connection, pipelines}`; left out keeps them, a `connection` of `""` unlinks, a connection of another kind is
  `400`.
- **Linked tickets**: `ticket_links_external (ticket_id, system 'jira', connection_id, key, url)`, unique per
  `(system, connection_id, key)`, gone with the ticket. Every ticket response has `external: [{system, key, url}]`;
  the Board card shows the key, the ticket page links it to the issue.
- **Incoming**: the built-in "Import assigned Jira issues" makes a Draft ticket of each new or updated issue in the
  scope that is assigned to the user and has no ticket. Linking is the dedupe (the ticket and its link are made in one
  transaction), so an issue is never imported twice. Not a sync: later changes in Jira only become events.
- **Import by key**: Tickets → Import from Jira… (shown when the project links Jira), or `POST
  /api/projects/{id}/jira/import {key}`: any issue, assigned or not → `201 {ticket, existed: false}`, or `200 {ticket,
  existed: true}` with the ticket that already links it; `400` for a key not like `ABC-123`, `404
  jira_issue_not_found`, `409 jira_not_linked`, `502 jira_failed`.
- **Outgoing**: moving a linked ticket to Done moves its issue (built-in "Done moves the Jira issue", the project's
  done mapping). Other changes stay in PDT unless a workflow says otherwise (`jira.comment`, `jira.transition`).
- **Buildkite CI** shows on the attempt like GitHub's: with an open PR at the same head, in `attempt.pr.ci` (the PR
  badge rolls up both providers); without a PR, as `attempt.ci` (board card `CiBadge`, Attempt section `CiDetails`).
  Failures are `ci.run_failed` with `provider: buildkite`, so "PR notices" tells the agent.

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
