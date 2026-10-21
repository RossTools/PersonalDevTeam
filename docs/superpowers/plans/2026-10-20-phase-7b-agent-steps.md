# Phase 7b plan: agent steps, pdt-mcp, personas and the Inbox (PDT-77)

Spec: docs/superpowers/specs/2026-10-20-phase-7b-agent-steps-design.md. One PR, built in this order (each step
test-first, `cargo test` green before the next).

1. **Core types (pdt-core).** `persona.rs` (persona, version, input validation, built-in templates, version diff),
   `inbox.rs` (kinds, groups, order, choices), `mcp.rs` (JSON-RPC request/response, tool result), `workflow/agent.rs`
   (agent node config, step texts: first message, nudge, answers); `Actor::Agent`; `EventKind::Outcome` and its
   `agent.outcome` mirror; `agent_context` tools paragraph and step section. Unit tests for each.
2. **Store.** Migration `0023_agent_steps.sql`; `personas.rs` (CRUD, versions, built-in install), `inbox.rs`
   (items, open/resolve/close), agent step bindings, the new session, project and settings columns.
3. **pdt-mcp (pdt-api `mcp/`).** Host key and tokens; the JSON-RPC endpoint; the tool registry with schemas from the
   API types; each tool on the shared domain function (refactor the route handlers it shares into functions taking
   `&AppState`). Tests through the router with `fake-agent`-free HTTP calls.
4. **Agents get it.** `OpenSpec.mcp`; Claude (`--mcp-config`, `--allowedTools`), Codex (thread `config`, TUI `-c` and
   env), Devin ACP `mcpServers`; terminals for Claude and Codex. Unit tests on the arguments.
5. **Personas API** and built-ins; permission mode resolution (project, persona, host).
6. **Agent node and engine.** `nodes/agent.rs`; `workflow/agent.rs` (session pick or create, first message, binding,
   `submit_outcome`, `ask_user`, turn ends from conversations and terminal activity, nudges, stalls, max turns, loop
   limit, take over); gate items; closing items when runs end. Engine tests with `fake-agent` conversations.
7. **Inbox API** (list with approvals, answer, dismiss).
8. **Personal projects.** `kind`, folder, Start without git, guards on git routes and the sensors.
9. **Web.** Inbox (rail, `Ctrl+] i`, list, work area, `j`/`k`/Enter/`o`/`e`), Personas view (list, form, versions,
   diff), Personal project in Add project, Start and the project page.
10. **Docs.** docs/reference/workflows.md (agent node, gates, Inbox), agents.md (pdt-mcp, personas), tickets.md
    (Personal projects, outcome events), agent-guide.md (pdt-mcp), AGENTS.md "How PDT works" pointers.
11. **Checks**: fmt, clippy, cargo test, web typecheck/test/build; PR labelled `windows` (process and file-system
    paths: session dirs, the personal folder, the host key).
