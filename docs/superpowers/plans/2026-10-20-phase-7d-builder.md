# Phase 7d plan: the workflow builder (PDT-79)

Spec: docs/superpowers/specs/2026-10-20-phase-7d-builder-design.md. One PR, in this order (core logic test-first).

1. **Core (pdt-core `workflow/`).** `template.rs`: params, filling `{{params.x}}` (typed when whole), param checks,
   placeholder fill for checking a template. `api.rs`: `VersionStatus`, `Workflow.proposed`,
   `WorkflowVersion.status`, template records. Unit tests.
2. **Store.** Migration `0022_workflow_builder.sql`: `workflow_versions.status`, `workflow_templates`. Version numbers
   from the highest so far; `propose_workflow_version`, `accept_workflow_version`, `discard_workflow_version`;
   `proposed` on reads; template CRUD. Store tests.
3. **API.** `POST /api/workflows/check`; `PUT … {propose}`, `/versions/{v}/accept|discard`; templates
   (`workflow/templates.rs`: built-ins, CRUD, instantiate). Tests in `tests/workflows.rs`. TS types.
4. **web/ui `FlowGraph`** on `@xyflow/react` (pnpm add), tokens for its colours, gallery section, test; lint rule
   keeping xyflow behind `@ui`.
5. **Web model** (`web/src/workflows/model.ts`): add/remove/connect/rename nodes, insert after, move in a chain,
   auto layout, outline tree, triggers and actions summary, schema → form fields. Unit tests.
6. **Web views**: Workflows rail item and list column; workflow page with draft bar, check, Enabled switch with
   consent; Graph, Outline, Text (JSON/TOML via `smol-toml`), node panel; Runs (path, retry, cancel, answer); Dry run
   dialog; Versions (diff, restore, proposals); templates (page, Save as template, New workflow dialog). Remove
   Settings → Workflows. Tests per view.
7. **Docs**: docs/reference/workflows.md, docs/agent-guide.md, the vision's §8 note.
8. **Checks**: fmt, clippy, cargo test, web typecheck/test/build; screenshots of the views on a throwaway dev daemon.
