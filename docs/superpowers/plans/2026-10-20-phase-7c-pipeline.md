# Phase 7c plan: pipeline template, policies, Board stages and scheduling (PDT-78)

Spec: docs/superpowers/specs/2026-10-20-phase-7c-pipeline-design.md. One PR, built in this order (core logic
test-first, `cargo test` green before the next step).

1. **Core (pdt-core).** `policy.rs` (`MergeBy`, `AutoStart`, `ProjectPolicy`, `may_merge`, `pick_next`);
   `ticket::PipelineStage`, `TicketAgent`; `ticket.refine_requested` event type; Inbox kind `manual`;
   `update_ticket` tool and the Refiner's tools; `maxAgentTurns` host setting. Unit tests for each rule.
2. **Store.** Migration `0024_pipeline.sql`; pipeline stage (set, cleared on move), policy columns, host setting,
   `session_messages.by_user`, active attempts per project, steps waiting for a slot.
3. **Commands.** Merge and Start as functions taking who asks; the merge policy and approvals; the running-attempt
   cap in Start; `POST /api/tickets/{id}/refine`; `update_ticket` in pdt-mcp.
4. **Engine.** `attempt.merge` node; `pipelineStage` on agent and gate nodes, `owner: user` manual steps, gate
   `approve`; slot waits and admission; the outbox cap.
5. **Scheduler.** `scheduler.rs` (auto-start every 10 s in pdtd), busy-agent count.
6. **Template.** `builtin:pipeline` with its params; an engine test that drives a ticket from Refine to merge with
   the steps' outcomes submitted through pdt-mcp, and one for the strict parameters.
7. **Ticket responses.** `pipelineStage` and `agent` on every ticket.
8. **Web.** Board stages and card agent line; Refine; Refining badge; project settings policies; host cap; Merge
   `by: "user"`; Inbox Your turn.
9. **Docs.** docs/reference/workflows.md (template, nodes, cap), tickets.md (pipeline stage, Refine, auto-start,
   cap), review.md (merge policy), host.md (cap), agent-guide.md (`update_ticket`), AGENTS.md pointer.
10. **Checks**: fmt, clippy, cargo test, web typecheck/test/build.
