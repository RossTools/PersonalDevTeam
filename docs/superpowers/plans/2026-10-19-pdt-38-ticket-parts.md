# PDT-38 PR 1 — Ticket parts: plan

Spec: docs/superpowers/specs/2026-10-19-ticket-parts-design.md (§3 decisions, §4 model, §5 start/prompts/workspaces,
§7 web, §8 API, §11 PR 1). Branch: this one, `pdt/PDT-38-tickets-that-take-several-prs-sub`; PR #58 carries
the spec, this plan and the code (merging the spec alone would end PDT-38's only attempt: the problem this fixes).

**Goal:** a ticket can have parts (child tickets, one level). Parents can't be started; "Start next part" starts the
first open, unblocked part. A parent moves to In progress when its first part starts and to Done like any ticket once
every part is closed (refused before). A part's first prompt carries the parent and the other parts. Parts share the
parent's workspace. The board has a lane per parent; the backlog and ticket views show parents, parts and progress.
"Make into parts…" turns a ticket's Scope list into parts. Then PDT-2 is converted (with the user's go-ahead).

**Not in this PR (PR 2):** stacking (`attempts.stacked_on`, Stack on, changed files from the blocker's head, restack
notice and button). Backlog grouping by parent.

**Decisions made while planning** (the spec is updated to match):
- **Migration number:** `0018_ticket_parts.sql` (main has 0016 and 0017 now).
- **The shared workspace belongs to the parent:** `tickets.workspace_id` (the parent's workspace, made by the first
  part's Start with "new workspace"). Parts' attempts leave `attempts.workspace_id` empty, so finishing a part never
  deletes the shared workspace. `Workspace.ticket` for that workspace is the parent, with the attempt and PR of its
  newest active part (`WorkspaceTicket.attemptId`, `attemptState` become optional; new `parts: PartsProgress`).
- **Closing a parent removes its workspace** server-side (PATCH, bulk and board drag all go through it) when the
  workspace holds only ticket tabs; with a session or run tab it is left alone (the web sees it in the workspace
  list).
- **Name clash:** web/src/tickets/TicketParts.tsx already means "pieces of the Overview". The new UI goes in
  `PartsSection.tsx` and `BoardLanes.tsx`; the roll-up type is `PartsProgress`. No renames of existing files.
- **Roll-up is computed in pdt-api** (`parts.rs`), like overlap and PR status, because `review` needs the in-memory PR
  state; the store returns parent ids and each part's stage and attempt state.

## API contract

Types (pdt-core `ticket.rs`, ts-rs, camelCase):
- `TicketParent { id, key, title }`.
- `PartsProgress { total, done, merged, review, progress, todo, backlog, segments: PartState[] }`;
  `PartState = "done" | "merged" | "review" | "progress" | "todo" | "backlog"` (one per part, rank order).
- `TicketSummary` gains `parent: TicketParent | null` and `parts: PartsProgress | null` (null when no parts).
- `Ticket` gains `partList: TicketSummary[]` (rank order; empty for a leaf). (`parts` is the roll-up on the summary.)
- `NewTicket` gains `parent?: string`, `blockedByPrevious?: boolean` (with `parent`: a `blocks` link from the
  parent's last part by rank, if any). With `parent` and no `before`/`after`, the part goes after the parent's last part.
- `TicketPatch` gains `parent?: string | null` (null detaches).
- `TicketQuery` gains `parent?: string` (`<id>` or `none`) and `hasParts?: boolean`.
- `StartTicket` gains `next?: boolean` (on a parent: start its next part).
- `NewParts { parts: { title, description? }[], blockedByPrevious?: boolean }` → `{ items: TicketSummary[] }`.
- `EventKind` gains `parted` (data `{parent, child, removed?}`, on both tickets).

Errors: `409 ticket_has_parts` (Start on a parent without `next`, delete a parent), `409 invalid_parent` (message
names the rule), `409 parts_open` (close a parent with open parts; message lists their keys), `409 no_next_part`
(`next` with nothing startable; message says why: all closed, all blocked, or all started), `409 ticket_active` (give
parts to a ticket with an active attempt).

Routes:
- `POST /api/projects/{id}/tickets`, `PATCH /api/tickets/{id}`: as above. PATCH to Done/Won't do on a parent with open
  parts → `parts_open` before anything changes. Bulk close is all-or-nothing: any parent with open parts fails it.
- `POST /api/tickets/{id}/parts` (`NewParts`), all-or-nothing, in order after the existing parts.
- `GET /api/tickets` with `parent`, `hasParts`. `GET /api/tickets/{id}` with `partList`.
- `POST /api/tickets/{id}/start` refused on a parent unless `next`; with `next` it resolves the part and runs the
  normal Start on it (same checks, `mode`, `start_blocked`), returning that part's `StartResult`.
- `DELETE /api/tickets/{id}` on a parent → `ticket_has_parts`.

## Tasks

### 1. pdt-core: rules (TDD) — crates/pdt-core/src/ticket.rs, new `parts.rs`
- `valid_parent(child, parent, parent_has_active_attempt, child_has_parts) -> Result<(), ParentRule>`: same project,
  not itself, parent has no parent, child has no parts, parent has no active attempt, parent open. One test per rule.
- `part_state(stage, attempt_state, pr_open) -> PartState` and `progress(parts) -> PartsProgress` (order kept).
- `next_part(parts) -> Result<&Part, NoNextPart>`: first open part by rank, no open blockers, no attempt; the reason
  when none.
- `can_close_parent(parts) -> Result<(), Vec<key>>`.
- Types and ts exports above; `TicketQuery::validate` for `parent`/`hasParts`; `NewTicket`/`TicketPatch` validation.

### 2. pdt-core: part prompt (TDD) — crates/pdt-core/src/start.rs
- `ticket_prompt` takes an optional `PartContext { parent_key, parent_title, parent_description, others: [(key,
  title, state, pr)] }` and adds a "Part of" block before the tail. The 8 KB cap cuts the parent's description first,
  then the ticket's (with the existing note). Tests: block present, order, parent text cut first, leaf prompt unchanged.

### 3. Migration and store — crates/pdt-store
- `0018_ticket_parts.sql`: `tickets.parent_id` (REFERENCES tickets ON DELETE RESTRICT), `tickets.workspace_id`
  (REFERENCES workspaces ON DELETE SET NULL), `tickets_by_parent` index. Check RESTRICT works with the store's
  `foreign_keys` pragma.
- Row mapping: `parent` on summaries (join for key/title); raw part rows for the roll-up (`part_rows(parent_ids)`:
  id, key, title, stage, rank, active attempt id/state, open-blocker count) in one query per page.
- `create_ticket` with parent (validate in the transaction; rank after the last part; optional blocks link; `parted`
  events); `patch_ticket` parent set/clear; `create_parts` (one transaction); `delete_ticket` refuses parents;
  `patch_ticket`/`bulk_tickets` refuse closing a parent with open parts (check inside the transaction).
- Filters: `parent = ?` / `parent_id IS NULL`, `hasParts` (EXISTS).
- `set_ticket_workspace`, and `Workspace.ticket` for a parent's workspace (from `tickets.workspace_id`, attempt from
  its newest active part).
- Tests: rules enforced in the store too (races), RESTRICT delete, part ordering, bulk all-or-nothing with a parent,
  filters, workspace ticket.

### 4. pdt-api
- `parts.rs`: `apply_summary`/`apply` fill `parts` from part rows + `github::watched_pr`; called from the same places
  as overlap and github decoration (list, reply, board queries, workspaces).
- `tickets.rs`: `parent`, `parts` route, errors mapped (`invalid_parent`, `parts_open`, `ticket_has_parts`).
- `start.rs`: refuse parents; `next`; when the started ticket has a parent: build the `PartContext`; move the parent
  from Draft/Ready/To do to In progress (system `moved` event) on its first part's start; with `newWorkspace`, use the
  parent's workspace (make it the first time, named `<parent key>`, with the parent's ticket tab first; store it with
  `set_ticket_workspace`) and leave `attempts.workspace_id` empty.
- Closing a parent (PATCH/bulk): after the stage change, delete its workspace with its tabs when it has only ticket
  tabs.
- Tests (fake-agent as the agent): Start refused on a parent; `next` starts the right part, respects blockers, and
  `no_next_part` reasons; parent to In progress once; prompt carries the parent; two parts' Starts share one
  workspace; finishing a part (stage Done) leaves the workspace; closing the parent removes it; `parts_open`.

### 5. Web — web/src/tickets, web/src/workspaces, web/src/palette
- `model.ts`/`queries.ts`: part states, labels and colours (status tokens only), query keys for parts.
- `PartsSection.tsx` (ticket tab and peek, above the timeline, for parents and for tickets with no attempt): the
  stepper (`PartsStepper` in web/src/common, gallery entry), part rows (key, title, stage, PR badge, agent dot),
  drag/Alt+↑↓ reorder (rank `before`/`after` within the parent), quick add with "Starts after <part above>", Start
  next part (opens the Start dialog for that part, or toasts the `no_next_part` reason).
- Ticket header: `PDT-2 ›` parent chip on parts; side column "Parent" picker (TicketPicker filtered to open tickets
  without a parent, not itself; Detach); Start hidden for parents (the attempt section shows only ended attempts).
- Closing a parent with open parts: the `parts_open` error opens a dialog listing them with "Close them as Won't do
  and move <KEY> to Done" (bulk Won't do, then the move).
- `BoardLanes.tsx`: lanes per §7 (no-parent lane first, parent lanes by rank, header with stepper, stage, PR badge,
  Start next part, menu, collapse in localStorage behind try/catch); cards drag between columns inside their lane
  only; phones: group headings in the one column. Lane headers from `GET /api/tickets?hasParts=true&stage=todo,
  in_progress` plus closed parents in the Done window plus parents of parts on the board.
- Backlog rows: parent chip on parts, `2/4` on parents; `parent:` filter.
- `finishMerged` on a part: also close the part's ticket tab (its attempt has no workspace of its own).
- "Make into parts…" (ticket menu): a dialog with rows prefilled from the description's list items under a heading
  containing "Scope" or "Parts" (pure parser in model.ts, tested), editable, "Starts after the previous one" ticked;
  `POST …/parts`.
- Sidebar Tickets section: a parent's workspace row shows the parent with `parts` progress and the active part's PR.
- Palette: "Start next part of <KEY>".
- Tests: PartsSection (add, reorder, start next, reasons), parent chip/picker, parts_open dialog, lanes (picked parent
  with nothing on the board, ordering, collapse, drag stays in lane, phone headings), scope parser, finishMerged on a
  part, palette entry. Screenshots for the new `@ui`/common pieces.

### 6. Docs
- AGENTS.md "Tickets" bullet: parts (parent_id, rules, lanes, next, shared workspace). Spec §4 migration number and the
  workspace decision. docs/SDLC.md step 8: a part's merge closes only the part.

### 7. Checks, PR, convert PDT-2
- The full check list (fmt, clippy, cargo test + gen types, web typecheck/test/build). Label `windows` only if a
  path/process change creeps in (none planned).
- After it ships to the stable instance and the user says go: make PDT-2's parts through the API (spec §10): 5a Done
  (#44), 5b Done (#50), 5c part 1 Done (#52) or as it stands, 5c part 2, 5d To do, each blocked by the one before;
  attach running work with Start `branch` (adopts existing worktrees); remove the old PDT-2 worktree and its session
  with the user's OK.
