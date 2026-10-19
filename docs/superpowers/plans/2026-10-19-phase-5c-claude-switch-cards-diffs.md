# Phase 5c (part 1): Claude's terminal switch, cards and diffs

Spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md (§6, §7, §8.2, §8.3, §9, §13 5c). Protocol facts:
docs/superpowers/notes/phase-5-agent-protocols.md. 5c is two PRs; this is the first. The second adds slash commands,
images, `@` files and the usage / rate-limit footer.

## Checked first (2026-10-19, Claude Code 2.1.302, a `-p` run in plan mode)

- Edit's `tool_use_result` carries `structuredPatch` (`[{oldStart, oldLines, newStart, newLines, lines}]`, lines
  prefixed ` `/`+`/`-`); Write gives `{type: "create", content, structuredPatch: []}`. The transcript keeps the same
  object as `toolUseResult`.
- Approving ExitPlanMode with `updatedPermissions: [{type: "setMode", mode: "acceptEdits", destination: "session"}]`
  leaves plan mode straight into Accept edits (no prompt for the following Edit/Write); `system/status` reports the new
  `permissionMode` at once.
- Plan mode writes its plan file with Write under `~/.claude/plans/`: turn diffs keep only files inside the session's
  folder.
- Transcript entries (`user`, `assistant`, `system`, plus bookkeeping types to skip) mirror the stream frames; no
  `result`, so a turn ends at the next real prompt (or `system/turn_duration` in TUI turns).

## Tasks

1. **Diffs (pdt-core `conversation/claude.rs`, TDD).** `ClaudeMapper::with_cwd`. Edit/MultiEdit/Write tool calls get
   `diff` (unified, path relative to the folder): from `structuredPatch`, or the created file's content, else from the
   input (no line numbers). Requests for those tools carry the input's diff (`Request.diff`). Each turn ends with a
   `turn_diff` (item `diff:<turn>`) of its successful edits inside the folder, one section per file.
2. **Plan approval mode.** `Answer::Plan` gains `mode?: PermissionMode`: Claude adds a session `setMode`. `system/status`
   with `permissionMode` updates the session's mode live.
3. **History import (§8.3, TDD).** `claude::history_events(transcript, after_uuid)`: entries after the anchor become
   events (`source: terminal`), through the same mapper; usage rows skipped; `last_uuid(transcript)`.
4. **Switch (§8.2).** `POST /api/sessions/{id}/mode {mode, cols?, rows?, interrupt?}` for Claude:
   - to terminal: 409 `turn_running` unless `interrupt`; park the `-p` process, add a notice, record `history_from`
     (the transcript's last uuid), set `mode = terminal` and start the TUI as a restart would (`--resume`, hooks).
   - to conversation: type `/exit` into the TUI, kill it after 5 s, import the turns after `history_from` (all of
     them when PDT's log is empty: old terminal sessions become conversations), set `mode = conversation`.
   - Codex keeps its live shared terminal (5b); its terminal-mode sessions are not switched yet.
5. **Continue after restart (§9).** The restart notice goes; the last turn, when interrupted, offers **Continue**
   (sends "Continue where you left off.").
6. **Web.** `@ui` `DiffViewer` (unified, line-number gutter, per-file headers); tool rows show "+12 −3" and the diff;
   approval cards for edits show the diff; the turn footer's **Changes** opens the turn's diff; QuestionCard options
   show their descriptions and take an "Other…" answer; PlanCard offers "Approve, accept edits" / "Approve" / "Keep
   planning"; the Conversation | Terminal switch for Claude (asks before interrupting a turn).
7. **Docs.** AGENTS.md, protocol notes, spec open questions.

## Tests

- pdt-core: fixture `tests/fixtures/claude/plan.jsonl` (the run above, paths replaced) and a transcript fixture.
- pdt-api: switch round trip with `fake-agent` (the test writes the transcript Claude would; fake-agent exits on
  `/exit`), `turn_running` refusal, import of an old terminal session.
- web: DiffViewer, cards, Changes, Continue, switch.
