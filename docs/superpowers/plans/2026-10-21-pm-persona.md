# PDT-81: the project manager — plan

Spec: docs/superpowers/specs/2026-10-21-pm-persona-design.md. One PR.

1. **Core (TDD)**: `ProjectKind::Host` and `Project::has_repo`; `ProjectPolicy.pm_acts`; persona tools (the new PM
   tools in `ALL_TOOLS`, `ASKING_TOOLS`, `PM_TOOLS`) and the built-in PM persona; `pm.rs`: briefing renderer, PM
   instructions, rotation rule, the project an asking tool's input targets.
2. **Store**: migration `0026_pm.sql` (`pm_notes` + FTS5 table and triggers, `pm_sessions`,
   `project_settings.pm_acts`); the host project made in `Store::open` and left out of `list_projects`; `pm.rs`
   (notes CRUD and search, journal, PM session rotation rows).
3. **Agents**: `McpServer.ask` (left out of `--allowedTools`) and `McpServer.only_pdt_tools` (`--tools ""`).
4. **API**: `pm.rs` (`GET /api/pm`, `POST /api/pm/sessions`, notes routes, briefing gathering, instructions, rotation
   on turn end, auto-answer of asking tools under `pm_acts`); the new pdt-mcp tools; project settings `pmActs`.
   Integration tests with `fake-agent`.
5. **Web**: `pm/` (PmView, PmList, NotesPage, usePm), the rail item and `Ctrl+] m`, palette Go to, project settings
   toggle. Tests.
6. **Docs**: docs/reference/agents.md (PM, tools), workflows.md (PM persona), tickets.md/workspaces.md (host project),
   the agents guide's pdt-mcp tool list.
