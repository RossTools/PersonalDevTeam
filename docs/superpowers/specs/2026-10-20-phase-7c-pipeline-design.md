# Personal Dev Team — Phase 7c: the pipeline template, policies, Board stages and scheduling

Status: design for PDT-78, 2026-10-20. Builds §8 and §10 of the vision
(docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md) and main spec §6.1–§6.2 on the 7a engine, 7b's agent
steps and Inbox, and 7d's templates (docs/reference/workflows.md). The concurrency cap is structured agents spec §16.

## 1. What 7c delivers

- **The Pipeline template** (`builtin:pipeline`): Draft → Refine → Ready → Code → Review → QA → Accept → Done as a
  workflow of agent steps and gates, with parameters: the persona of each stage, which stages a person does instead
  (manual), where a refined ticket goes, whether a person accepts, and whether the workflow or a person merges.
- **Policies per project**: who may merge (agents and workflows, a person only, nobody in PDT), whether an automated
  merge needs a person's approval, a cap on running attempts, and auto-start. The domain commands (Merge, Start)
  enforce them, so no workflow routes around them.
- **Board stages**: a ticket in the pipeline carries its stage (`pipelineStage`); the Board splits In progress into
  Code, Review, QA and Accept, and cards show the persona that owns the ticket now and its live state.
- **Scheduling**: auto-start takes To do (and optionally Ready) tickets in rank order, skipping blocked and conflicting
  ones, up to the project's cap; a host cap on agents running a turn holds PDT's own work ("Waiting for a slot").

Done when a Draft ticket goes through refine, code, review and QA by agents with the user only answering questions and
accepting the merge, and a strict project (agents never merge, a person approves) is only different template
parameters and policies.

## 2. Pipeline stages on tickets

- `tickets.pipeline_stage`: `refine | code | review | qa | accept`, or null. It refines the ticket's stage: `refine`
  while the ticket is Draft (or Ready, refined again), the others while it is In progress. Every response carries it
  as `pipelineStage` (pdt-core `ticket::PipelineStage`).
- Who sets it: the `agent` and `gate` nodes gain an optional `pipelineStage` config; when such a step starts (also
  while it waits for a slot, §6.2) the ticket's stage becomes that. The Refine command (§3.3) sets `refine`.
- Any change of the ticket's stage clears it (the store's move): Start moves it to In progress and the Coder step
  then sets `code`, and a ticket that leaves the pipeline never shows a stale stage.

## 3. The Pipeline template

### 3.1 Parameters

| Param | Type | Default | Meaning |
|---|---|---|---|
| `refiner`, `coder`, `reviewer`, `qa` | string | the built-in personas | the persona of each stage |
| `refineBy`, `codeBy`, `reviewBy`, `qaBy` | choice `agent \| user` | `agent` | `user`: the stage is manual (§3.4) |
| `afterRefine` | choice `ready \| todo` | `todo` | where a refined ticket goes (To do: on the board, picked for auto-start) |
| `accept` | choice `user \| auto` | `user` | `user`: the Accept gate; `auto`: merge once QA passes |
| `merge` | choice `workflow \| person` | `workflow` | `workflow`: the run merges after Accept (a PR's auto-merge, or a local merge); `person`: it waits for someone to merge |
| `loopLimit` | number | 3 | passes of Code before the Inbox asks (main spec §6.2) |

Instantiated per project (scope `project`), disabled until the user enables it, like every template.

### 3.2 The document

Two triggers, so a ticket the user starts by hand goes through review too:

```
refine_requested ─► refine (agent: Refiner, new session, pipelineStage refine)
                      ready ─────────────► to_queue (ticket.move {{params.afterRefine}})
                      needs_clarification ► clarify (gate: "answer in the ticket, then refine again";
                                                     refine_again → refine, stop → end)

attempt.started ──► code (agent: Coder, session reuse, pipelineStage code, loopLimit {{params.loopLimit}})
                      submitted ► review (agent: Reviewer, new, pipelineStage review)
                                    approved ► qa (agent: QA, new, pipelineStage qa)
                                                 pass ► accept_mode (switch {{params.accept}})
                                                          user ► accept (gate inbox: accept, pipelineStage accept,
                                                                         choices merge | reject, approve: merge)
                                                                   merge ► merge_mode (switch {{params.merge}})
                                                                   reject ► code
                                                          auto ► merge_mode
                                                 fail ► code
                                    changes_requested ► code
merge_mode: workflow ► merge (attempt.merge, finishOnMerge)
                         merged ► finish (ticket.finish) · queued ► end (Finish merged tickets finishes it)
                         refused ► wait_merge
            person ► wait_merge (wait: attempt.merged for this attempt) ► end
```

- The prompts say what each pass is for; earlier outcomes reach later passes through `get_ticket` (7b §4.5), so a
  Coder sent back by Review or QA sees their summaries and line comments.
- `timeout` and `stopped` ports end the run with a ticket comment saying where it stopped; `error` is the engine's.
- The run's `maxRuntimeMs` is 30 days (a ticket can sit in Accept for a while).
- A strict project: `merge: person` (and policy `mergeBy: user`, §4). Agents still refine, code, review and QA; a
  person merges in PDT or on GitHub and the run ends when `attempt.merged` arrives.

### 3.3 Refine

- `POST /api/tickets/{id}/refine`: the ticket must be Draft or Ready (else 409 `ticket_not_refinable`), and an enabled
  workflow for its project must have a `ticket.refine_requested` trigger (else 409 `no_refine_workflow`, "enable the
  Pipeline in Workflows"). A ticket already in `refine` with a running or waiting run is 409 `refine_running`. It sets
  `pipelineStage: refine` and logs `ticket.refine_requested` (payload: project, ticket id, key, title, stage).
- Web: **Refine** in the ticket menu (cards, backlog rows, the peek) and the ticket tab's menu.
- The Refiner gets a new pdt-mcp tool, `update_ticket {title?, description?}` (`PATCH /api/tickets/{id}` as the
  agent, on this session's ticket only), so it rewrites the ticket instead of only commenting. The built-in Refiner's
  tools are the default set plus `update_ticket`.

### 3.4 Manual stages

The `agent` node gains `owner: agent | user` (default `agent`, a reference allowed). With `user` it starts no session:
the binding's state is `manual` and the Inbox gets a **Your turn** item (kind `manual`, group Gates) with the step's
prompt and one choice per outcome. Answering it completes the step by that outcome (`by: user`), with the user's text
as the summary. Loop limits, `pipelineStage` and the timeline's `outcome` event work as for agents.

## 4. Policies

Per project, in project settings (`GET/PUT /api/projects/{id}/settings`), pure rules in pdt-core `policy.rs`:

| Policy | Values (default) | Enforced by |
|---|---|---|
| `mergeBy` | `agents` (agents, workflows and people) · `user` (people only) · `external` (nobody in PDT) (`agents`) | Merge (`POST /api/attempts/{id}/merge`, the `attempt.merge` node): 409 `merge_not_allowed` |
| `requireApproval` | bool (false) | an automated merge needs a person's approval at the branch's current head (Approve, or a gate whose `approve` choice was taken): 409 `approval_required` |
| `maxRunningAttempts` | number or null (null) | Start: over the cap is a `start_blocked` finding (`force` starts anyway); auto-start never goes over it (1 when unset) |
| `autoStart` | `off` · `todo` · `ready` (`off`) | the scheduler (§6.1) |

- **Who is merging.** The web sends `{by: "user"}` with Merge; anything else (an agent's curl, a workflow, pdt-mcp) is
  automated, as with comments (docs/reference/tickets.md). A person's merge is its own approval.
- **Auto-merge.** A merge through a pull request always turns on GitHub's auto-merge (merge when green, or the merge
  queue). "Merge without a person accepting" is the template's `accept: auto`; `requireApproval` is the policy that
  stops it.
- **Approve before start** is auto-start's `todo`: only tickets a person moved to To do start on their own.
- Permission modes stay as 7b made them (project override, persona, host default).

## 5. The `attempt.merge` node and approvals

- `attempt.merge` (action): `attempt` (default `{{trigger.attemptId}}`), `style` (`merge | squash`, for a local
  merge), `finishOnMerge` (true), `deleteBranch` (false). Ports `merged` (merged locally; the attempt ended),
  `queued` (auto-merge on for its pull request; Finish merged tickets finishes it), `refused` (policy, or what stops
  the merge: `code` and `reason` in the output). It runs the Merge command as an automated merger.
- The `gate` node gains `approve` (a choice): taking it records the user's approval of the run's attempt at its
  branch head (the same `review` timeline event as Approve), so `requireApproval` passes for the merge that follows.

## 6. Scheduling

### 6.1 Auto-start

- pdtd's scheduler (every 10 s; `schedule_tickets` for tests) looks at projects whose `autoStart` is not `off`. While
  the project's active attempts are under its cap (`maxRunningAttempts`, else 1), it starts the next candidate.
- Candidates, in rank order: To do, then (with `ready`) Ready. It skips parents, tickets that ever had an attempt (a
  person restarts those), tickets blocked by an open ticket, tickets with a `conflict` overlap, and waiting tickets
  that are not clear to start (pdt-core `policy::pick_next`, TDD).
- It starts through the Start command (mode `check` with blockers and conflicts already excluded; plain overlaps are
  allowed), in a new workspace named after the ticket. When an enabled workflow for the project has an
  `attempt.started` trigger (the pipeline does), it starts with no session and the workflow's Coder step starts one;
  otherwise it starts an agent session of the host's default agent and permission mode, as Start does. The `started`
  event records `auto: true`.
- A start that fails is logged and the ticket is skipped until pdtd restarts or the ticket changes.

### 6.2 The host cap on agents running a turn

- Host setting `maxAgentTurns` (default 4; 0: no cap). Busy agents: conversations with a turn running or a message
  queued, and agent terminals whose activity is `working`; idle ones don't count.
- It holds PDT's own work only: an agent step's new session waits (`wait: {slot: true}`, its ticket's card says
  "Waiting for a slot"), and the outbox holds a message that would start a turn in an idle session. The engine tick
  admits waiting steps oldest first as slots free. What the user does is never held: their own messages, and what
  they send from the Inbox (answers, nudges; `session_messages.by_user`).

## 7. Board

- The In progress column splits into **Code | Review | QA | Accept** when any In progress card on the board has a
  pipeline stage; a card without one shows under Code. Dropping a card on any of them moves it to In progress (the
  workflow owns the stage). Phones get the four in the column picker.
- **Cards** show the owner and its state from `TicketSummary.agent` (computed in memory like PR state): `{persona,
  state, sessionId, busy}`, `state` one of `working`, `asking` (a question waits on the user), `stalled`, `yours` (a
  manual step or a gate), `queued` (waiting for a slot). The line reads "Reviewer · working" with its `AgentDot`, "Your
  turn" for a gate, "Waiting for a slot". Draft and Ready tickets in `refine` show a Refining badge in the backlog.

## 8. API and data summary

- Migration `0024_pipeline.sql`: `tickets.pipeline_stage`; `project_settings.merge_by`, `require_approval`,
  `max_running_attempts`, `auto_start`; `session_messages.by_user`. Host settings gain `maxAgentTurns`.
- `POST /api/tickets/{id}/refine`; `TicketSummary.pipelineStage`, `.agent`; project settings gain the four policies;
  Merge takes `by`; event type `ticket.refine_requested`; nodes `attempt.merge`, and `pipelineStage` on `agent` and
  `gate`, `owner` on `agent`, `approve` on `gate`; Inbox kind `manual`; pdt-mcp `update_ticket`;
  `builtin:pipeline`.

## 9. Code layout

- pdt-core: `policy.rs` (policies, `may_merge`, `pick_next`), `ticket.rs` (`PipelineStage`, `TicketAgent`),
  `workflow/events.rs` (`ticket.refine_requested`), `inbox.rs` (`manual`), `persona.rs` (`update_ticket`, Refiner's
  tools).
- pdt-store: migration 0024, tickets (`set_pipeline_stage`, clearing on move, active attempt counts), project and host
  settings, outbox `by_user`, steps waiting for a slot.
- pdt-api: `scheduler.rs` (auto-start, busy agents, slot admission), `review.rs` (Merge as a function with the
  policy), `start.rs` (Start as a function; the cap), `tickets.rs` (Refine), `overlap.rs` decoration (`agent`),
  `workflow/nodes/merge.rs`, `nodes/agent.rs` / `gate.rs` (new config), `workflow/agent.rs` (manual steps, slots),
  `workflow/outbox.rs` (the cap), `workflow/templates.rs` (`pipeline`), `mcp/tools.rs` (`update_ticket`).
- web: Board stages and the card's agent line, Refine, the Refining badge, project settings "Pipeline and policies",
  host settings "Agents running at once", Merge sends `by: "user"`, the Inbox's Your turn item.

## 10. Out of scope

External reviewers as a `pr.review_approved` wait (needs a review sensor), the PM persona, PDT-24's base-moved
policy, a per-stage workflow per project in `.pdt/workflows`, and auto-refining new drafts (a `ticket.created`
trigger can do that later).
