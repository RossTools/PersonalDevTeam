# Phase 2: agent signals as verified on 2026-10-15

## Versions
- Claude Code 2.1.301. Codex `codex-cli 0.162.1` (`(Get-Command codex).Source` is
  `C:\Users\ross\AppData\Local\Programs\OpenAI\Codex\bin\codex.exe`, so it is the right binary; no shadowing).

## Claude Code
- Re-checked with `claude -p ... --model haiku --settings <file>` where the settings file lists hooks in exec form
  (`"command": "node", "args": ["<script>", "<Event>"]`). Seen firing alongside the user's own hooks:
  UserPromptSubmit, PreToolUse, Stop. Exec form works on Windows. Hook processes inherit the session environment
  (a variable set in the launching shell was visible in the hook; `CLAUDE_PROJECT_DIR` is also set).
- Hook stdin JSON always has `session_id`, `transcript_path`, `cwd`, `prompt_id`, `permission_mode`,
  `hook_event_name`; PreToolUse adds `tool_name`, `tool_input`, `tool_use_id`; Stop adds `last_assistant_message`.
- PermissionRequest and Notification (types) were NOT re-exercised in this task (they need an interactive permission
  prompt); they rest on the facts checked when the plan was written.

## Codex notify
- `-c 'notify=["node","<script>","--chain-test"]'` replaces the configured notify (the user's own notify program
  did not run). The JSON payload is appended as the LAST argv element, one argument:
  `["--chain-test","{\"type\":\"agent-turn-complete\",\"thread-id\":\"01a1...\",\"turn-id\":\"01a1...\",\"cwd\":\"C:\\...\\w\",\"client\":\"codex-tui\",\"input-messages\":[\"reply with the word ok\"],\"last-assistant-message\":\"ok\"}"]`
- Fired inside a real ConPTY: yes.
- CAUTION: it fired TWICE for one user turn. The second call is for Codex's internal title-generation thread: it has
  a different `thread-id`, `input-messages` is the long "Generate a concise, single-line task title..." instruction
  and `last-assistant-message` is `{"title":"Reply ok"}`. Task 9 must not treat every notify call as a user-turn
  completion (e.g. ignore calls whose `input-messages` start with "Generate a concise, single-line task title", or
  key on the first call per turn).

## Codex approval screen
- Reached with `-a on-request -s read-only`, prompt `run the shell command: git status`. (`approval_policy = "untrusted"`
  is rejected by 0.162.1: "no longer supported"; use `-a`.) Here the approval came from a sandbox start failure
  ("May I run git status outside the sandbox after the shell process failed to start?"), but the screen is the
  ordinary command-approval screen.
- Visible lines:
  ```
  Would you like to run the following command?
  Environment: local
  Reason: May I run git status outside the sandbox after the shell process failed to start?
  $ git status
  › 1. Yes, proceed (y)
    2. Yes, and don't ask again for commands that start with `git status` (p)
    3. No, and tell Codex what to do differently (esc)
  Press enter to confirm or esc to cancel
  ```
  While it was showing, the terminal title (OSC 0) was `[ ! ] Action Required | <thread> | <folder>`.
- ConPTY output is a screen diff, not text: spaces are `ESC[1C` cursor moves, and characters already on screen in a
  cell are skipped. In the capture the question line is `Wou ESC[1C d you like to run the following command?`
  (the `l` is missing), so "Would you like to run" does NOT match even after stripping escapes. Strip CSI/OSC
  sequences, drop all whitespace, lowercase, then match.
- `CODEX_APPROVAL_MARKERS` (verified present in the capture after that normalisation):
  - `to run the following command?`
  - `Yes, proceed`
  - `No, and tell Codex what to do differently`
  - `Press enter to confirm or esc to cancel`
- Not verified on a real screen (candidate only, do not rely on it): `Would you like to make the following edits?`
- Capture: crates/pdt-agents/tests/fixtures/codex-approval.txt (raw bytes from the second prompt onwards, ends with
  the approval screen up; 9026 bytes).
- Other notes: a first run in an untrusted folder shows a "Trust this folder?" screen; it saves the decision to
  `config.toml`. This task avoided touching the user's config by running with `CODEX_HOME` pointing at a scratch copy.