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
  stalled | yours` (a manual step or a gate) `| queued` (waiting for a slot) `| background` (its turn ended while
  background tasks it started run, PDT-142; `background` lists their descriptions). The backlog shows a Refining badge; the
  Board splits In progress into Code | Review | QA | Accept | Merge once any In progress card has a stage (none:
  Code; `merge_blocked` under Merge, with a Merge blocked badge), and
  each card shows the owner line ("Reviewer · working", "Your turn", "… · waiting for a slot", "Coder · waiting on
  background task: Run pytest").
- **Refine**: `POST /api/tickets/{id}/refine` (`{by: "user"}` from the web; ticket menu and the ticket tab's menu) on
  a Draft or Ready ticket (else 409 `ticket_not_refinable`) sets `pipelineStage: refine` and logs
  `ticket.refine_requested`; 409 `no_refine_workflow` when no enabled workflow for the project has that trigger, and
  `refine_running` while one already refines it.
- scripts/import-roadmap.mjs imported the old docs/ROADMAP.md as tickets PDT-1…17 (done once; kept for reference).

## Sources and the Todo gate (PDT-201)

Outside input (an issue, a bug report from another instance) can carry prompt injection, so a ticket made from it
reaches agents only once the user says so.

- **Source**: every ticket has `source` (`tickets.source`, pdt-core `TicketSource`): `user` (the API and UI; every
  ticket made before PDT-201), `pm` (the PM's `create_ticket` in a conversation), `workflow` (`ticket.create` in a run,
  an agent other than the PM, an agent's `report_bug`) or `external`: `github.import`, `jira.import` and the Jira
  sensor (pdt-store `create_linked_ticket` always makes it `external`), `create_ticket` from a session that triages
  (the Triage persona, or any agent step of a run an outside event started: pdt-core `workflow::events::UNTRUSTED_EVENTS`,
  `github.issue.opened` and `webhook.received`), and `ticket.create` in such a run. Requests can't set it.
  `admitted` (`tickets.admitted_at`) says the user let an external ticket in. The card and the ticket page show a
  badge for every source but `user`: PM, Workflow, External (in the waiting tone until let in, its tooltip says only
  the user can move it on).
- **The gate** (pdt-core `external_gate`, applied by pdt-store to every create, patch, move and bulk change): an
  external ticket not let in yet can't be made in, or moved to, To do or In progress, nor get the `agentic` label
  (pdt-core `AGENTIC_LABEL`), except by the user (`Actor::User`: the UI, the bulk bar, Start) or a PM call the
  user confirmed (`By.confirmed`). Such a change lets it in (`admitted_at`); from then on it moves as any other ticket.
  A triaged ticket's Approve in the Inbox (PDT-225, [workflows.md](workflows.md#inbox)) is the user's move too, sent with
  the UI token.
  Anything else is refused with `403 untrusted_ticket` (a workflow step fails with the reason; an agent's tool call
  gets it): `ticket.move`, `ticket.create` with `stage: todo`, the Refiner, Coder, Reviewer, QA and triage sessions,
  the PM without the user's yes. `ticket.move`'s `externalStage` sends such a ticket somewhere else instead (the
  Pipeline's afterRefine: Ready). Auto-start never starts one (`403 untrusted_ticket` from `start_ticket(auto)`).
  `github.import` drops an `agentic` label triage suggests. A ticket filed as a GitHub issue (PDT-221, pdt-store
  `link_external`) links it but keeps its source: the issue came from the ticket.
- **The PM's yes**: the PM's `edit_ticket` that would let an external ticket in always opens Claude's approval prompt,
  even in a project that lets the PM act ([agents.md](agents.md) "The PM"): `pm::request_opened` holds it (pdt-api
  `untrusted::Grants`) instead of answering for the user, and the user's Approve
  (`POST /api/sessions/{id}/requests/{requestId}`) is good for that one call with those arguments for ten minutes.
  Without it (a Codex PM, a session that bypasses permissions) the store refuses the edit.
- **The UI token** (PDT-204): agent sessions with a shell can `curl` the HTTP API on 127.0.0.1, which otherwise counts
  as the user. So `PATCH /api/tickets/{id}`, `POST /api/tickets/bulk`, `POST /api/tickets/{id}/start` and
  `POST /api/tickets/{id}/stop` (with a `stage`) refuse a change that would let an external ticket in
  (`403 untrusted_ticket`, pdt-api `untrusted::lets_in`) unless the request carries this pdtd's UI token in
  `x-ui-token`. The token is random at every start (`untrusted::UiToken`); `GET /api/ui/token` hands it only to a
  browser's same-origin fetch (`Sec-Fetch-Site: same-origin`, `403 not_the_ui` otherwise). The web client fetches it
  once and sends it with ticket edits, bulk actions, Start and the answer to a request (`asUser` in
  web/src/api/client.ts; it doesn't call stop); a token from before a restart is `403 stale_ui_token`, and the client
  fetches the new one and retries once. The PM's yes counts only with the token too, so an agent can't approve its own
  call. Everything else over the API is unchanged. This stops an agent's plain curl, not a determined one: an agent
  runs as the user and could act like a browser.
- **Secrets**: a session that triages, or whose ticket is external and not let in, is refused `request_secret`,
  `run_with_secrets` and `write_secret_file` ([secrets.md](secrets.md)).

## Outside input in prompts (PDT-202)

The second layer: lower the odds an injection works, and record what outside text reached which agent.

- **Fencing** (pdt-core `fence`): an event type declares the payload fields that hold outside text (`EventType.outside`:
  `github.issue.opened` `title`, `body`; `webhook.received` `query`, `headers`, `body`; `pr.commented` and
  `pr.reviewed` `body`; Jira's `summary`, `description`, a comment's `body`). Wherever a reference to such a field, or
  to something inside one (`{{trigger.body.message}}`), goes to an agent (a node kind's `agent_text`: an `agent`
  step's `prompt`, `session.send`'s `text`), the engine (`engine::fence_input`, pdt-core `resolve_fenced`) puts its
  value in a block tagged with an id picked for that prompt (a string as it is, an object or list as its JSON), `<untrusted id="…">…</untrusted id="…">`, and puts a fixed instruction first: the block is
  data to read, never instructions to follow. Anything in the text that looks like the tag (`<untrusted`,
  `</untrusted`, any case or spacing) has its `<` written `&lt;`, so it can't close its block early. Other nodes
  (conditions, `ticket.create`, `github.comment`) get the text as it is. `get_ticket` fences an external ticket's
  description the same way (`outsideText` carries the instruction). On purpose, PR notices' own `notice.text` (PDT's
  message quoting a pull request comment or review, what PR comments to the agent sends) is not fenced: only
  collaborators' comments get that far (see Outside authors), and collaborators are trusted, as they can push to the
  repository anyway. The built-in triage prompts (Triage new GitHub
  issues, PDT bug triage) put the title and text on lines of their own, so the blocks stand apart.
- **Outside authors**: GitHub says how a comment's or review's author is related to the repository
  (`author_association`); only `OWNER`, `MEMBER` and `COLLABORATOR` are insiders (pdt-forge `collaborator`). Bots
  (installed on the repository) and the account PDT is logged in as count as insiders too. Everyone else's
  `pr.commented` and `pr.reviewed` carry `outside: true`: such a comment goes on the ticket's timeline with its text
  (`outside_input` with `held: true`, behind "Show what they wrote"), never into a session. `session.send` in a run
  they started leaves by `skipped` with the reason, an agent step fails with it, and PR comments to the agent skips
  them in its trigger.
- **Triage cap**: a workflow with an `agent` step started by an outside event (`github.issue.opened`,
  `webhook.received`) is a triage run. One author's issues start at most `triagePerAuthorHour` of them an hour (default
  3), one repository's (or webhook's) `triagePerRepoHour` (default 20; Settings → Host, 0 for no cap), counted in
  events that started a run (pdt-store `events_with_runs_since`). Over the cap the run isn't started: a
  `triage.skipped` event (`repo`, `number`, `url`, `author`, the workflow, `cap`: `author` or `repo`, `limit`), and one
  Inbox notice per repository and hour that lists what it skipped (pdt-store `put_inbox_summary`).
- **Audit**: each time outside text goes into a session, a `session.outside_input` event (`source`: `owner/name#12`,
  `webhook <name>`, a Jira key or the external ticket's link; `url`, `sessionId`, `ticketId`, `runId`, `workflowName`,
  `via`) and, with a ticket, an `outside_input` entry on its timeline. `via`: `prompt` (an agent step's fenced prompt),
  `message` (a delivered `session.send` with a block), `ticket` (a new session on an external ticket: Start, or an
  agent step's new session), `triage` (on the ticket a triage made: its `create_ticket`, or `ticket.create` /
  `github.import` in its run, naming the run's agent sessions).

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
  ticket (then Ready with `ready`) in the project's order, skipping parents, tickets with an active attempt (an abandoned or merged one doesn't count, PDT-159: a ticket
  stopped and moved back to To do starts again once clear), blocked ones, `conflict`
  overlaps and waiting tickets that aren't clear (pdt-core `policy::pick_next`). It starts in a new workspace with the
  host's default agent, view and ticket permission mode, or with no session when an enabled workflow for the project
  has an `attempt.started` trigger (the Pipeline's Coder step starts one, its tab after the ticket's,
  then the Reviewer's and QA's); the `started`
  event has `auto: true`. A failed start is logged and skipped until the ticket changes; a start with its own session
  waits for a slot under the host's cap.
- **Auto-start order** (project settings `autoStartOrder`: `priority`, the default for new and existing projects, or
  `rank`; PDT-218; Web: "Auto-start picks by"): `priority` takes the most urgent candidate first (urgent, high,
  medium, low, none), then backlog rank, then id; `rank` takes them in backlog rank only. Either way To do comes
  before Ready, and the skips above, the quiet period and the failed-start skip still apply. The scheduler queries
  candidates in that order (`TicketSort::Priority` or `Rank`) and `pick_next` picks by it; `get_overview`'s
  `autoStarting` lists To do then Ready, each in that order.
- **Quiet period** (project settings `startDelaySeconds`, default 120, 0 at once, at most a day; PDT-146): auto-start
  takes a ticket only once it has gone that long unchanged, measured from the later of `createdAt` and `updatedAt`
  (pdt-core `policy::settles_at`), so tickets created and then linked to their blockers never start early. Anything
  that bumps `updatedAt` restarts the wait: an edit (title, description, priority, labels, areas, parent), a move, a
  comment, waiting, and a link or unlink, which touches both tickets. A blocker closing does not (the freed ticket can
  start at once). While a candidate (a stage auto-start takes, startable) waits it out, ticket responses carry
  `autoStartAt` (ms; computed in memory, null otherwise) and the card, row and ticket header show "Auto-starts in
  1:30"; pdt-mcp's `get_overview` lists such tickets under `autoStarting` and `search_tickets` gives
  `autoStartsInSeconds`. A person's Start is never delayed.
- A Start whose session cannot be made is undone: the attempt and its timeline events go and the ticket's stage and
  pipeline stage are as they were (the worktree stays, for the next Start to adopt). Workflows only hear of a Start
  (`ticket.stage_changed`, `attempt.started`) once its session exists, so an undone one runs none.
- `noSession: true` (PDT-72; the Start dialog's "No session") starts no agent session: the branch, worktree, attempt
  and only the ticket's tab (still in its new workspace with `newWorkspace`); the result's `session` is `null`.
- `POST /api/attempts/{id}/abandon` ends an attempt (worktree kept); moving a ticket to Done ends its attempt as
  merged, to Won't do as merged if detected, else abandoned, once the move went through (a PATCH or bulk move that is
  refused ends nothing; a bulk move ends each ticket's attempt on its own).
- **Stopping a ticket's work** (PDT-159, pdt-api `stop.rs`): moving a ticket with an active attempt back to Draft,
  Ready or To do (a PATCH, a bulk move, the PM's `edit_ticket`), Abandon and `POST /api/tickets/{id}/stop` cancel the
  ticket's open workflow runs (the attempt's, and the ticket's that started before the stop, so a run the move itself
  triggers goes on), stop the attempt's running sessions (they and their tabs stay) and abandon the attempt, runs
  first so no step starts a session meanwhile. The UI asks first ("Stop ALP-1's work?") when a move would do this.
  `/stop` takes `{stage?, removeWorktree?, deleteBranch?}`: `stage` is `draft`, `ready` or `todo` (else 400
  `invalid_stage`; left out, an in-progress ticket goes to To do); `removeWorktree` also stops and removes the
  sessions in the latest attempt's worktree and workspace and every run of the attempt (as Finish does), removes the worktree (uncommitted changes too; the
  branch with `deleteBranch`) and deletes the workspace Start made (`finish::clean_up`, shared with Finish), so the
  next Start begins clean instead of adopting it. It answers `StopResult` (`abandoned`, `runsCancelled`,
  `sessionsStopped`, `worktreeRemoved`, …). The PM's `stop_ticket` is the same call, asking first.
- **Personal projects** (Phase 7b, [workspaces.md](workspaces.md#projects-and-git-workspaces-spec-6)): Start makes no
  branch or worktree. The attempt has an empty branch and no worktree, and the session runs in the ticket's folder
  `<project folder>/<KEY>-<n>` (made on Start, kept across attempts: it is the work's output); the prompt says to work
  in this folder. Start from a chat is refused there (409 `personal_project`). Merge detection and the PR poller skip
  these attempts; moving the ticket to Done ends the attempt.
- Agent steps record their outcome on the timeline (`outcome`: `{outcome, summary, persona, personaVersion, by,
  sessionId, attemptId, comments, workflowId, workflowName, runId}`, actor `agent`, or `user` when taken over from the
  Inbox) and a step's new session as a `session` event with its persona. Every ticket change and comment an agent makes through
  pdt-mcp (comments, `update_ticket`, `edit_ticket` with its moves, `create_ticket`, `link_tickets`/`unlink_tickets`,
  `stop_ticket`, `report_bug`) has actor `agent` and `data.agent`: `{sessionId, projectId, persona, fromTicket,
  fromTicketId}` (the ticket that session works on, if any; PDT-163). Events carry it as `agent` (read from
  `data.agent`, or from the flat `data.sessionId`/`data.persona` agent comments had before; null otherwise). The
  timeline names such an event "Coder · PDT-34", a button that opens the session; older events show the persona or
  "Agent" as before. `get_ticket` gives each comment and each entry of `history` (the newest 50 created, edited,
  moved, linked, unlinked, parted, waiting, started, merged and abandoned events, oldest first, with their `data`)
  `by` (the persona, `agent` when none is known, `user`, `system`), `sessionId` and `fromTicket`. The built-in
  "Ticket comments to the agent" only forwards the user's.

## Merge detection and finishing

- pdt-git `merge.rs`, every 60 s in pdtd's sweep loop and on `POST /api/tickets/{id}/check-merge` (the ticket tab and
  peek call it on open), fetching each project's base once per tick (`GIT_TERMINAL_PROMPT=0`, only with an `origin`).
- An attempt is merged when its head (or `attempts.last_head` once the branch is gone) is an ancestor of
  `<base>`/`origin/<base>` or its whole patch is already there (squash: `commit-tree` + `cherry`), both only once a
  check has seen the branch hold commits not in the base (`attempts.own_commits`, sticky; PDT-45: a branch that only
  fast-forwarded to a newer main is not merged), or GitHub says its PR merged at that head (`attempt.pr`; the merge
  queue squashes onto a newer base, which the patch check can miss).
- Commits pushed after the merge (PDT-151): when GitHub says the attempt's PR merged at a head older than the
  branch's, the commits since (`<merged head>..<head>`, pdt-git `commits_between`) are in no merged PR, so the attempt
  stays active and Finish merged tickets never runs. The merge check (with `news`: the sweep and `check-merge`, not a
  close) logs `attempt.late_commits` once per branch head (`mergedHead`, `head`, `commits` newest first, `summary`
  "CB-25: 1 commit pushed after PR #308 merged: cd5d0b7 …", `list`, and the `notice` for the agent). The built-in
  "Commits pushed after a merge" ([workflows.md](workflows.md#workflows)) tells the agent and asks the user in the
  Inbox.
- Ending an attempt as merged logs `attempt.merged` (with `via`: `detected` by this check or by a merge that found
  the attempt's PR merged already, `merged_here` by a person's local Merge…, `automated` by a workflow's or an agent's
  local merge, `closed` when the ticket was moved to Done or Won't do) for workflows (docs/reference/workflows.md).
  Finish merged tickets finishes `detected` and `automated` merges (PDT-181); a person's Merge… finishes its own.
- Finishing is one server command, `POST /api/tickets/{id}/finish {stopRunning, discardChanges, deleteBranch,
  keepWorktree}` (pdt-api `finish.rs`, PDT-76). It checks first and changes nothing when something would be lost:
  `outcome: blocked` with `blocked {sessions, runs, dirty}` for sessions or runs still going (unless `stopRunning`)
  or uncommitted changes (unless `discardChanges`). Otherwise it moves the ticket to Done, ends its active attempt (as merged, `via: closed`), cancels the
  ticket's workflow runs still going (but the run finishing it and the runs that called that one; error "cancelled:
  the ticket was finished"), stops every session
  in the worktree, the attempt's sessions and everything in the workspace Start made for the ticket
  (`attempts.workspace_id`), stops and removes every run of the attempt in any state (stopped, passed and failed too:
  in its worktree, on its branch or with a tab in its workspace, PDT-160), removes the worktree (and the branch with `deleteBranch`; `branchNote` when it stayed),
  closes the ticket's tab elsewhere, deletes that workspace with its tabs, and pulls the base branch (`pull`).
  `keepWorktree` only moves to Done (ending the attempt and cancelling the runs as above) and pulls (`outcome: kept`). The route runs it in a task of its own, so it
  completes when the caller goes away first (PDT-97: an agent finishing its own ticket is stopped by it).
  A ticket already closed whose attempt ended and whose worktree is gone is finished already: the command does
  nothing (`finished`), so a second finish after one merge (the Pipeline's Finish, then Finish merged tickets) doesn't
  clean up again or cancel runs (PDT-181). Two finishes at the same moment both still run.
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

crates/pdt-forge (the `Forge` and `CiProvider` traits), crates/pdt-github (the GitHub forge and GitHub Actions),
pdt-api `forge.rs`, `ci.rs` and `github.rs`, pure rules in pdt-core `pr.rs`.

- A project's **forge** (PDT-176) is where its PRs, reviews and issues live: the one `.pdt/project.toml` names
  (`forge = "github"`, `"local"` or `"none"`, docs/run-config.md "Forge"), else the forge whose plugin claims its
  `origin`'s host, else none (no PR, CI or feedback is read and Merge is local). Forges are plugins (PDT-241): each
  registers a `pdt_forge::ForgePlugin` (its kind, the hosts it claims, the `Forge` it opens) in the
  `pdt_forge::ForgeRegistry` on `AppState::forges`, and is used while the integration that declares its kind
  (`Integration::forge_kind`) is enabled. GitHub (`pdt_github::plugin::GithubPlugin`) claims github.com and the GitHub
  Enterprise Server the login is for; the local forge is built in. Everything that reads a forge goes through
  `forge::project_forge` (the PR poller, merges, agents' tools, issue triage, the `github.*` nodes); a flow tied to no
  project (an issue trigger's `repos`, a node's `repo` in a run without a GitHub project) uses the forge's account
  (`AppState::forge_account`). Only the GitHub login uses the GitHub client directly. A forge declares its capabilities (`Forge::capabilities`:
  merge queue, required checks, issues, reviews) and the steps below skip or adapt what it can't do: no merge-queue
  runs or queue setting without a merge queue, no `pr.commented`/`pr.reviewed` without reviews, no issue triage or
  `github.*` issue actions without issues, and without required checks a workflow's or agent's merge always waits
  for green CI (Review "Merge"). Each forge also writes its part of agents' instructions (docs/reference/agents.md).
  GitHub can do all of it.
- A project's **CI providers** (PDT-242, `pdt_forge::CiProvider`, pdt-api `ci.rs`) are apart from its forge: GitHub
  Actions (`pdt_github::GithubActions`, the GitHub plugin's own CI, `ForgePlugin::ci`), the local CI command
  (`local_forge::LocalCi`, the local forge's) and Buildkite (`pdt_int_buildkite::BuildkiteCi`, from
  `Integration::ci_provider` once the project's settings name a connection and pipelines). By default a project has its
  forge's own CI and every CI integration set up for it; `[ci] providers` in `.pdt/project.toml` picks and orders them
  (docs/run-config.md "CI providers"). A provider reads a branch's runs (`runs`), a run's jobs (`jobs`) and a job's log
  (`job_log_tail`), and may retry a run, start a workflow (`trigger`) and read merge-queue runs, as its
  `capabilities` say; `config_paths` says whether the repository has its CI (Actions: files under `.github/workflows`;
  the others: set up, so yes). A run (`pdt_core::pr::CiRun`) names its `provider`, has a text `id` (an Actions run's
  number, a Buildkite build's `pipeline/number`, a local run's number), its `attempt`, `name` (workflow or pipeline),
  `state`, `url`, head, branch and `failedJobs`. CI events logged before PDT-242 were rewritten to that shape (migration
  `…_ci_run_providers.sql`). Tests use `pdt_forge::testing::FakeCi` (`FakeForge::ci` is a fake Actions;
  `testing::fake_github` registers both).
- The **local forge** (PDT-178, pdt-api `local_forge.rs`, store `local_changes` / `local_ci_runs`) is PDT itself, for
  repositories with no remote: change requests, CI and merges with required checks only. Each read of a branch's
  change request (the poller's, Merge's) also moves it on: opens one once the active attempt's branch has commits its
  base lacks (title `<key>: <ticket title>`), follows the head, marks it merged when its commits reach the base some
  other way, stops CI on an older head and starts the `[ci]` command in the worktree on a new one (a run marked
  running that this pdtd isn't running was cut short by a restart: cancelled and run again), and merges one set to
  merge (`Forge::merge`) whose CI passed or that has no `[ci]` (`pdt_git::merge_fast_forward`). Its log is
  `<data dir>/local-ci/<run>.log`, served at `/api/local-ci/runs/{id}/log`, and is each run's `url`. A run that ends
  wakes `run_local_ci_watch`, which reads the attempt again and runs the merge check for it at once. `attempt.pr` has
  `local: true`; notices and the CI gate name a "change request", point at the log with `curl` and say commit, not
  push. A merge in the read that sees CI pass logs `pr.merged` without a `ci.run_passed`, as auto-merge does on GitHub.
- Every 60 s (before the merge check) and when a ticket is opened (`check-merge`), pdtd reads each active attempt's PR
  from its project's forge and its CI from each of the project's CI providers (`github::read_attempt`); GitHub's goes over GitHub's REST and GraphQL APIs with the in-app GitHub login
  ([github.md](github.md); no `gh`). Without a login every read fails as "GitHub is not connected" and the Inbox asks
  the user to connect. Tests get `GithubApi::offline()` (no network, no login) or `pdt_forge::testing::FakeForge`
  registered as GitHub through the registry (`pdt_forge::testing::fake_github`), whose capabilities a test can turn off, and pdtd's own tests set `PDT_GITHUB_OFFLINE=1`. Only projects with a
  forge or a CI provider (a GitHub Enterprise Server's origin counts once the login is for that host).
- **CI** is read on the open PR's head from every provider; a provider that builds branches (`capabilities().branches`:
  Buildkite) is also read without a PR, on `origin/<branch>`, else the local branch (`attempt.ci`). The newest run of
  each provider's workflow rolls up into one status (`CiStatus::of`/`merge`: a failure wins, then running, queued,
  cancelled), the PR badge's `attempt.pr.ci`. A provider whose read fails keeps its last look on that head (logged
  once) and an automated merge refuses (`ci_unreadable`) until it reads again. A failed read of the PR itself keeps
  its last look (`attempt.pr.error`), but the providers that build branches are still read (on that PR's last head,
  else the branch's) and still report: Buildkite doesn't wait for GitHub. Each provider's CI is its own events: `ci.run_failed` (a failure notice naming
  the provider unless it is Actions or the local command, and how to read the log: `ci_failed_logs` with its
  `provider` and `runId`) and `ci.run_passed`, with `provider`; keys `<attempt>:ci_failed:<id>.<attempt>,…` for
  Actions and local CI (as before), `<attempt>:ci_failed:<provider>:<id>.<attempt>,…` for the others.
- On GitHub, one GraphQL query reads state, draft, mergeable, `BEHIND`, auto-merge, merge-queue entry and newest
  merge-queue event; then GitHub Actions' runs API (`…/actions/runs?branch=`) gives CI on the PR head (the newest run of each workflow;
  fine-grained tokens cannot read check runs) and, while queued or just out of the queue, `…/actions/runs?event=
  merge_group` gives the runs on `gh-readonly-queue/<base>/pr-<n>-…`; failed job names come from the run attempt's
  jobs (cached per run attempt). The GETs are conditional (ETags), so an unchanged answer costs no rate limit.
- The PR is the branch's newest open one, else its newest, but a closed or merged PR opened before the attempt started
  (`createdAt`) is not the attempt's: a ticket reopened and started again on its old branch name has no PR (no merge
  from it, no news) until one is opened for it. A PR still open on the branch is the branch's, whoever opened it.
- The result is in memory and on every ticket response as `attempt.pr` (Attempt section: `PrDetails`; board card:
  `PrBadge`, web/src/tickets/PullRequest.tsx). An attempt that ended while its PR was last seen open has the PR read
  again each tick for an hour after (no news), and the web shows a merged attempt's still-open PR as Merged (PDT-59:
  git's merge check can end the attempt while GitHub still says queued).
- `attempt.pr.checkedAt` is when it was last read. When a read fails (GitHub not connected or down) the last look stays
  with `error` set (PDT-130): the board card's badge shows a warning sign and says why in its tooltip, the Attempt
  section "Not updated: <error> (last checked <when>)"; otherwise both say when it was last checked. A failing read is
  logged once until it works again. The failed jobs known are read once a pass (not once an attempt), and whether a
  workflow listens for PR comments once a pass.
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
  the same fix step for the PR's later failures, but a CI pass alone never turns auto-merge on for it; being set to
  merge replaces it, and a person turning auto-merge off clears it with `merge_requested`. A failure while an
  agent already fixes the attempt (checked as the run gets there) is told to that agent instead of a second fix.
  A CI failure notice says when a merge is in flight ("Auto-merge is on for the pull request, so it merges once the
  fix passes", or the merge queue). CI events carry `mergeWaiting` when a workflow's merge waits for that CI (PDT-151:
  a waiting step of an active run of the attempt that has an `attempt.merge` step waits for a `ci.*` event); PR
  notices leaves those failures to that run, so the Coder hears of them once.
  Details: [workflows.md](workflows.md) "Workflows".
- A user's comment on a ticket with an active attempt goes to its agent session the same way (built-in "Ticket comments
  to the agent"; an agent's own comments, actor `agent`, are not sent); Start already puts the earlier comments in the
  first prompt.

## Jira and Buildkite (Phase 7e-2, PDT-85)

For a work project whose issues are in Jira Cloud and whose CI is Buildkite. pdt-core `jira.rs`, `buildkite.rs`;
crates/pdt-int-jira and crates/pdt-int-buildkite (each on only when enabled, [workflows.md](workflows.md)
"Integrations"); the sensors and actions are in [workflows.md](workflows.md) "Sensors and webhooks".

- **The link** is host settings, not `.pdt/project.toml` (work repos may not take a `.pdt/` folder): the project's
  settings page, section Integrations (made from what each integration declares, [workflows.md](workflows.md)
  "Integrations"), names a `jira` connection, a JQL scope (`project = ABC`) and the done transition or status
  (default `Done`), and a `buildkite` connection with the pipelines (slugs, one per line) that build its branches.
  API: `GET/PUT /api/projects/{id}/settings` `integrations` `{"jira": {connection, scope, done}, "buildkite":
  {connection, pipelines}}`; an integration left out is kept, a key left out keeps its value, `null` or a
  `connection` of `""` unlinks, a connection of another kind is `400`. Stored as JSON in
  `project_settings.integration_settings` (PDT-174; 0027's `jira_*` and `buildkite_*` columns were copied in and
  are unused).
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
- **GitHub issues** (PDT-154): a ticket can link a GitHub issue the same way (`system 'github'`, `connection_id` the
  repository `owner/name` and `key` `owner/name#12`, both lower case; `url` the issue's). `github.import` makes such
  tickets (the built-in "PDT bug triage" for the `from-pdt` bug reports on the release repository); the card and the
  ticket page show it as `#12` (titled "GitHub issue owner/name#12", the page linking it), and `get_ticket` lists it
  in `external`. Moving a linked ticket to Done or Won't do closes the issue once (built-in "Done closes the GitHub
  issue", [workflows.md](workflows.md) "Built-in workflows").
- **Buildkite CI** is a CI provider (PDT-242, "Pull requests and CI"): with an open PR at the same head, in
  `attempt.pr.ci` (the PR badge rolls up every provider); without a PR, as `attempt.ci` (board card `CiBadge`, Attempt
  section `CiDetails`). Failures are `ci.run_failed` with `provider: buildkite`, so "PR notices" tells the agent, and
  `ci_failed_logs` reads its failed jobs' logs (`GET …/builds/<n>/jobs/<id>/log`).

## Areas and overlap (Phase 4c)

Tickets spec §7; the config format is in docs/run-config.md "Areas".

- `[[area]]` entries (`name`, `paths` globs, `exclusive`) in the main checkout's `.pdt/project.toml` plus host
  overrides (`GET /api/projects/{id}/areas`; areas not in the config are ignored). PDT's own .pdt/project.toml
  declares its areas (migrations is the exclusive one).
- A ticket's overlap is always against the project's tickets with an active attempt: shared areas (planned) and, when
  both are active, shared changed files (actual; pdtd's 30 s `refresh_overlaps` runs `pdt_git::changed_files` per
  active attempt and keeps them in memory, keeping an attempt's last list when git fails). A shared exclusive area or a
  file inside one is a `conflict`, else `overlap`.
- File overlaps are recorded on both tickets' timelines as `overlap` events (`{ticketId, key, files, level}`, mirrored
  as `ticket.overlap_found`) only when they are news: a new pair, a file not reported for the pair since it began, or
  a worse level (pdt-core `overlap_news`, against the ticket's earlier events folded by the store's
  `overlap_reports`). The same files again, fewer of them, or a file coming back after a revert or rebase add nothing.
  A reported pair that no longer shares a file (one attempt ended, or the files were put back) adds one `overlap`
  event with `cleared: true`, `files: []`, `level: none` (`ticket.overlap_cleared`); a later overlap is a new pair.
  No clear is recorded while either attempt has no file list yet (pdtd just started), nor on done or won't-do tickets.
- Computed in pdt-api `overlap.rs` (pure rules in pdt-core `overlap.rs`) and applied to every ticket response:
  `overlap`, `overlapWith`, `clearToStart` on summaries, `overlaps` on the full ticket; `GET
  /api/tickets?conflicts=true|false`.
- Start with `mode: check` refuses (`start_blocked`) on blockers or overlaps; Wait is `PATCH {waiting: true}` (top of
  its stage; cleared by Start and by closing).
- The parser skips top-level sections it does not know (listed in `effective.ignored`, noted in the project's
  settings) so a file written for a newer PDT still runs on an older stable; unknown keys inside
  `[[run]]`/`[[test]]`/`[[area]]` are still refused, and so are unknown sections in host overrides. A PDT older than
  PDT-21 still refuses unknown sections, so a new section in this repo's file must wait until the stable has this.
