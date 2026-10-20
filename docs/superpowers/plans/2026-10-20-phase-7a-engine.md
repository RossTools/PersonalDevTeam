# Phase 7a plan: events, engine core and node catalog (PDT-76)

Spec: docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md. One PR, built in this order (each step test-first,
`cargo test` green before the next).

1. **Core types (pdt-core `workflow/`).** Document (nodes, edges, settings), references (`{{path}}`, typed when whole),
   conditions, `Tpl<T>` for literal-or-reference fields, structural + text diff, event types and payload structs,
   validation against a catalog view (kinds, versions, ports, reachability). Unit tests for each.
2. **Store.** Migration `0021_workflows.sql`: `events`, `workflows`, `workflow_versions`, `workflow_runs`,
   `workflow_steps`, `workflow_state`, `session_messages`, `connections`. `event_log.rs` (emit with dedupe, list,
   claim unprocessed, prune), `workflows.rs` (CRUD, versions, runs, steps, state, outbox, connections). Timeline mirror
   in `insert_event`; `end_attempt` records `via` and `pr`.
3. **Catalog and engine (pdt-api `workflow/`).** `NodeKind` trait and registry; nodes; engine tick (match, wake,
   advance, limits, retries, error path, sub-workflows, quiet, cancel, retry); dry run; outbox delivery with today's
   rule.
4. **Domain commands.** `ticket.finish` (`POST /api/tickets/{id}/finish`), used by the node; `session.send` via the
   outbox.
5. **Sensors emit.** PR poller emits `ci.*`/`pr.*`; merge detection's `end_attempt` emits `attempt.merged`; old
   notice queue and `finish::on_merged` removed. pdtd's loop runs the engine tick; built-ins installed on start;
   repo workflows synced every minute; pruning hourly.
6. **API routes** (spec §9), connections with the secret store, TS types.
7. **Web.** Banner and Merge… call the finish command; Settings → Workflows (list, enable, recent runs).
8. **Docs.** docs/reference/workflows.md; tickets.md (PR notices, finishing), agents.md and AGENTS.md invariant,
   agent-guide.md (workflows API); README of the API if it lists routes.
9. **Checks**: fmt, clippy, cargo test, web typecheck/test/build; PR labelled `windows` (process and file-system code
   paths move).
