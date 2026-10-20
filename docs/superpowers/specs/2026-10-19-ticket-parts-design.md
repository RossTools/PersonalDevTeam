# Personal Dev Team — Tickets that take several PRs (parts) Design

Status: decisions made with the user, 2026-10-19 (PDT-38); ready for a plan. Amends the tickets spec
(docs/superpowers/specs/2026-10-18-tickets-design.md §3, §5, §6, §8; it brings forward swimlanes from §12, grouped by parent only) and notes what Phase 7 inherits.

## 1. Problem

A ticket has one active attempt, one branch, one worktree and one PR. Some tickets need several PRs: PDT-2 (Phase 5)
is planned as 5a, 5b, 5c and 5d. What went wrong there:

- **Done too early.** The attempt was marked merged and the ticket offered "Move to Done and remove the worktree"
  while three PRs were still to come. (It actually fired 60 s after Start, before any PR: the branch had only caught
  up with `origin/main`. That was a separate bug, **PDT-45**, now fixed. With one attempt per ticket, though, the first
  real merge would have done the same.)
- **Extra branches are unknown.** 5b's branch and worktree were made by hand; the ticket, its attempts and the board
  don't know them.
- **Stacked branches hurt.** After 5a was squash-merged, merging main into 5b made every file both touched a conflict.
- **The worktree can't be removed.** The session working the ticket lives in 5a's worktree.
- **Progress lives in comments.** The board can't show which part is in flight, in review or merged.

## 2. Options

| Option | Fits | Costs | Verdict |
|---|---|---|---|
| 1. **Sub-tickets** (`parent`) | Every part gets what a ticket already has: attempt, branch, worktree, PR and CI notices, blockers, areas and overlap, Start checks, the timeline, later Review (Phase 6) and the pipeline (Phase 7) | A parent/child relation; rows and cards show it | **Recommended** |
| 2. Ticket types (epic/story/task) | The same as 1 | A type field that duplicates "has children"; rules per type | Not needed: "has children" does the job |
| 3. Several attempts per ticket, in sequence | Fixes "done too early" and unknown branches | Nothing to plan parts with; every per-ticket feature (blockers, overlap, waiting, Phase 7 stages, loop count) would need a per-attempt copy; no parallel parts | Too small |
| 4. Planned parts inside a ticket (checklist → attempts) | Planning plus a stepper | A second, smaller ticket model inside the ticket (title, state, order, attempt, PR per part); same duplication as 3 | Sub-tickets with a different name |

Options 3 and 4 add per-part state that tickets already have. Each part is a unit of review, CI and merge, which is
exactly what a ticket is in PDT's model. So a part is a ticket, and the multi-PR ticket is its **parent**. Option 4's
stepper is kept as the parent's progress display (§7).

## 3. Decisions (made with the user, 2026-10-19)

1. **Parts are child tickets.** `tickets.parent_id`. One level only: a part can't have parts (a big part is split
   into more parts of the same parent, e.g. 5c-1 and 5c-2). No ticket types.
2. **Parents aren't started.** Start on a ticket with parts is refused (`409 ticket_has_parts`); the parent's work
   happens in its parts. A ticket with an active attempt can't get parts (`409 ticket_active`): finish or abandon it
   first. A parent can keep ended attempts from before it had parts (history; PDT-2 has one).
3. **Order and sequence:** parts are ordered by rank within the parent. A part that must wait for another gets a normal
   `blocks` link; adding a part below another in the Parts list adds that link by default ("Starts after PDT-n",
   untick for a part that can run in parallel). Blockers, Wait and "clear to start" work unchanged.
4. **Planned up front or added as you go:** both. Parts can be added at any time; nothing is fixed when the parent
   starts.
5. **Parent stage follows its parts:** when the first part starts, the parent moves to In progress (a system `moved`
   event). Once every part is Done or Won't do, the parent moves to Done like any other ticket (drag on the board,
   the stage picker, bulk, its lane menu); nothing moves it automatically and there is no special banner. Moving a
   parent to Done or Won't do while any part is open is refused (`409 parts_open`, naming the open parts); the UI
   says why and offers to close the rest as Won't do.
6. **Parts branch off main** after the part before merges (blocks). **Stacking** (start part N+1 on part N's branch
   while N is in review) comes in the second PR, with PDT's restack after the squash (§6); the first PR has no
   stacking.
7. **Sessions belong to parts.** Each part's Start opens its own session in its own worktree; it ends with that part.
   Continuity comes from the ticket: the part's first prompt includes the parent and the other parts' state (§5).
   There is no long-lived "lead" or planning session on the parent, and no session moves from one part to the next:
   a fresh session per part keeps context small, and merged worktrees can always be removed.
8. **Board: one lane per parent.** Parts are cards in their own columns inside their parent's lane; tickets without a
   parent are in a lane at the top. The lane header is how a parent is tracked: key, title, stage, the stepper, and
   Start next part. A parent gets a lane as soon as it is picked (To do), before any of its parts are (§7). Parents
   are never cards themselves, so nothing is shown twice.
9. **Keys:** parts get ordinary numbers (`PDT-46`), shown with the parent as `PDT-2 › PDT-46`. No `PDT-2.1` keys:
   numbers stay unique and search, links and branches stay as they are.

## 4. Model

One migration (`0018_ticket_parts.sql`, the next free number; the migrations area is exclusive):

```sql
ALTER TABLE tickets ADD COLUMN parent_id TEXT REFERENCES tickets(id) ON DELETE RESTRICT;
ALTER TABLE tickets ADD COLUMN workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL;  -- a parent's shared workspace
CREATE INDEX tickets_by_parent ON tickets (parent_id, rank) WHERE parent_id IS NOT NULL;
ALTER TABLE attempts ADD COLUMN stacked_on TEXT REFERENCES attempts(id) ON DELETE SET NULL;  -- PR 2 (§6)
```

Rules (pure, pdt-core, TDD):

- **Valid parent:** same project; not the ticket itself; the parent has no parent; the child has no parts; the parent
  has no active attempt; the parent is open (not Done/Won't do).
- **Delete:** a parent with parts is refused (`409 ticket_has_parts`); deleting a part is as today.
- **Roll-up** (`PartsProgress`): `total`, and counts by part state, in order: `done` (Done/Won't do), `merged`
  (attempt merged, ticket not yet Done), `review` (PR open), `progress` (active attempt, no PR), `todo` (To do),
  `backlog` (Draft/Ready). The stepper shows one segment per part in rank order with that state.
- **Next part:** the first open part by rank that has no open blockers and no attempt. "Start next part" on the parent
  starts that one.
- **Parent's areas** are planning hints: a new part copies them; overlap ignores parents (they have no attempt).

Events: `parted` on both tickets (`{parent, child}` added or removed), the parent's system `moved` when the first part
starts.

## 5. Start, prompts and workspaces

- **Prompt for a part:** today's prompt, plus a short "Part of" block: the parent's key and title, the parent's
  description (cut first when the 8 KB cap is reached), and one line per other part (`PDT-46 5a: Done (#44)`,
  `PDT-47 5b: in review (#50)`, `PDT-48 5d: To do`). That gives the new session what the old long-running one carried.
- **Workspace:** a part's Start with "new workspace" uses the **parent's workspace** (made by the first part's Start,
  named `<parent key>`, holding the parent ticket tab; stored as `tickets.workspace_id` on the parent, while the
  part's `attempts.workspace_id` stays empty), adding the part's ticket and session tabs. The sidebar shows that
  workspace as the parent, with its newest active part's PR. "Move to Done and
  remove the worktree" on a part (`finishMerged`) removes that part's worktree, sessions and tabs, and leaves the
  shared workspace. Closing the parent removes the workspace when it holds only ticket tabs.
- **Merge detection** is unchanged: it runs on parts' attempts. A part merging can only close that part.

## 6. Stacked parts (second PR)

The stacked-branch pain comes from merging main into part N+1 after part N was squash-merged: git sees N's commits and
the squash commit as different changes to the same lines. The fix is to replay only N+1's own commits:
`git rebase --onto origin/main <N's last head> <N+1's branch>`. PDT knows N's last head (`attempts.last_head`).

- **Start, stacked:** when a part's only open blockers have an active attempt (their PR open), the Start dialog offers
  **Stack on PDT-n** next to Wait and Start anyway. The attempt's branch starts from the blocker's branch head;
  `stacked_on` = the blocker's attempt; `base_branch` stays `main` (the PR targets main; merge detection checks main).
- **Changed files and overlap** for a stacked attempt are measured from the blocker's head, not the merge base with
  main, so the part doesn't overlap its own base on every file.
- **The PR:** the agent opens it against main with "Stacked on #n" in the body (the prompt says so); it shows the
  blocker's changes too until that merges.
- **When the blocker merges:** PDT sends the part's agent session a notice (the PR/CI notice path, PDT-25) with the
  exact commands: `git fetch origin && git rebase --onto origin/main <last head> && git push --force-with-lease`, and
  then clears `stacked_on`. PDT doesn't rewrite a branch an agent is working in. A **Restack** button (clean
  worktree, no turn running, no one typing) runs the same commands itself and reports conflicts back to the session.
- A part stacked on a blocker that is abandoned is left as is, with a warning on the part.

## 7. Web

- **Ticket tab / peek of a parent:** a **Parts** section above the timeline: the stepper, then the parts in rank
  order (key, title, stage, PR badge, agent state), drag to reorder, quick add ("Starts after <part above>" ticked),
  **Start next part**. The side column hides Start and the attempt section for parents (ended attempts stay under
  History).
- **Ticket tab / peek of a part:** a parent chip (`PDT-2 › `) in the header; a "Parent" picker in the side column
  (also to make an existing ticket a part, or detach it).
- **Backlog:** rows show the parent chip on parts and `2/4` progress on parents. Filter `parent:` (a ticket, or
  "no parent"). Grouping by parent is deferred.
- **Board: lanes by parent.** The columns stay To do / In progress / Done (last 14 days). Rows are lanes:

  ```
                       To do          In progress          Done
   ─ (no parent) ───────────────────────────────────────────────────
                       PDT-6          PDT-30, PDT-42        PDT-39
   ─ PDT-2 Phase 5 ▮▮▮▯▯ 3/5 · In progress · Start next part ───────
                       PDT-50 5d      PDT-49 5c-2 (PR #57)  PDT-46 5a, PDT-47 5b
   ─ PDT-4 Phase 7 ▯▯▯ 0/3 · To do · Start next part ───────────────
                       (no parts picked yet)
  ```

  - **Which parents get a lane:** a parent in To do or In progress, or one with any part on the board (a part in To do,
    In progress, or Done in the window). Moving a parent to To do is how it is picked; its lane shows at once, even
    when every part is still in the backlog. A parent closed in the Done window keeps its lane until the window ends.
  - **The lane header** is the parent's card: key, title, its rolled-up stage, the stepper (§4 roll-up), the PR badge
    of a part in review, **Start next part**, a menu (Open, Move to Done once every part is closed, Add part), and a
    collapse toggle (collapsed: just the header, remembered per browser). Clicking it opens the parent's peek.
  - **Order:** the "no parent" lane first, then parent lanes by the parents' rank. Lanes can't be dragged; ranking the
    parent in the backlog moves its lane.
  - **Moves:** cards drag between columns within their lane only; changing a part's parent happens in the ticket.
  - **Phones** (one column at a time): the lanes become group headings inside the column, with the stepper and a
    `3/5` count.
  - A board filter by parent shows just that lane.
  - **API:** the board's three column queries already return each part's `parent`; the lane headers come from one
    more query, `GET /api/tickets?hasParts=true&stage=todo,in_progress&closedAfter=…` (with `parts` roll-ups), plus
    the parents of any parts on the board.
- **Make into parts…** (ticket menu): opens a form with one row per part, prefilled from the description's list items
  under a heading named "Scope" or "Parts" if there is one (PDT-2's "Scope (one PR each)" list), each with "Starts
  after the previous one" ticked. Creates the parts in one bulk call.
- **Palette:** "Start next part of <KEY>".

## 8. API

| Route | Change |
|---|---|
| `POST /api/projects/{id}/tickets` | `parent?`; `after?` defaults to the parent's last part; `blockedByPrevious?` adds the `blocks` link |
| `PATCH /api/tickets/{id}` | `parent: id \| null` (rules in §4; `409 invalid_parent` names the broken rule); closing a parent with open parts → `409 parts_open` |
| `GET /api/tickets` | `parent=<id>\|none`, `hasParts=true\|false`; summaries carry `parent {id, key, title}` and `parts: PartsProgress` |
| `GET /api/tickets/{id}` | `parts: [TicketSummary]` in rank order |
| `POST /api/tickets/{id}/start` | refused on a parent (`409 ticket_has_parts`); `next: true` on a parent starts its next part; `stack: true` (PR 2) |
| `POST /api/tickets/{id}/parts` | create several parts in order (`[{title, description?}]`, all-or-nothing) |
| `POST /api/attempts/{id}/restack` | PR 2 (§6) |

## 9. Phase 7 and later

- **The pipeline runs per part** (Refine → Code → Review → QA → Accept for each PR). A parent has no pipeline stage of
  its own; its stage is the roll-up (§3.5).
- **Refine on a parent** may answer with parts: a `submit_outcome("split", parts[])` outcome creates them (the
  Refiner's version of "Make into parts…").
- **Automatic start:** Phase 7 starts the next part when its blocker merges, as it does waiting tickets.

## 10. Converting PDT-2

After the first PR ships to the stable instance, by an agent through the API with the user's go-ahead:
make parts 5a (Done, #44), 5b (Done, #50), 5c part 1 (in review, #52; Start with `branch` adopts its branch),
5c part 2 (its worktree `pdt/PDT-2-5c-composer-extras` exists; Start adopts it) and 5d (To do), each blocked by the one
before. PDT-2's ended attempt stays as history. The old 5a worktree and its session can then be removed.

## 11. Build order

- **PR 1, parts:** migration, pdt-core rules and roll-up (TDD), store, API (parent, parts, roll-up, Start refusal and
  `next`, parent stage, close rules), part prompts, the shared workspace and `finishMerged` change; web Parts section,
  parent chips and progress, Parent picker, board lanes by parent, Make into parts…, palette entry. Then convert
  PDT-2.
- **PR 2, stacking:** `stacked_on`, Start "Stack on", changed files from the blocker's head, the restack notice and
  Restack.
- PDT-45 (catching up with main counted as merged) is fixed (done before this work).

## 12. Testing

- pdt-core: valid-parent rules (each refusal), roll-up states and order, next part, prompt with parts and the cap
  (parent description cut first).
- pdt-store: `parent_id` and the RESTRICT delete, list by parent, bulk part creation all-or-nothing.
- pdt-api: Start refused on parents and `next`; parent to In progress on the first part's start; a parent moves
  to Done by PATCH, bulk or board drag once every part is closed and `parts_open` before; the shared workspace survives a part's finish.
- pdt-git (PR 2): `rebase --onto` after a squash merge applies cleanly where `merge main` conflicts (temp repo);
  changed files from the blocker's head.
- pdt-api: `hasParts` filter.
- Web: Parts section (add, reorder, start next), Make into parts… parsing, chips; board lanes (a picked parent with
  no parts on the board gets a lane, the no-parent lane first, lane order by rank, collapse remembered, drags stay in
  their lane, phone group headings).

## 13. Open questions

None; the user answered them on 2026-10-19 (§3: parent to Done like a normal ticket once every part is closed,
stacking in the second PR, a fresh session per part, one level).
