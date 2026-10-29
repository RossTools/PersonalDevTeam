# Devin for Terminal (PDT-54, PDT-55)

Facts PDT relies on, checked against `devin 3000.11.92` on macOS on 2026-10-19 (bundled docs in
`~/.local/share/devin/cli/_versions/current/share/devin/docs`, and live sessions in a scratch repo). Windows is untested.

## Install and login

- Official installer: `curl -fsSL https://cli.devin.ai/install.sh | bash` puts the binary under
  `~/.local/share/devin/cli/_versions/<version>` with a link at `~/.local/bin/devin` (already in PDT's macOS install
  folders) and updates itself. There is also a Homebrew cask (`devin-cli`).
- On the dev Mac the cask's binary hung before running any code (stuck in `_dyld_start`, 0 CPU): macOS had wedged on
  the quarantined first launch, and every launch from that path hung, even after `xattr -c` and a reinstall. A copy
  anywhere else ran. The curl installer avoided it.
- `devin auth status` exits 0 whether or not you are logged in; logged out it prints `Not logged in.`, logged in
  `Logged in (via Devin).`. PDT's host check reads the text. Devin is optional: a missing one is not a warning.

## Launch

- `devin [OPTIONS] [PATH]... [-- <PROMPT>...]`: a bare argument is a folder to work in (`… is neither a known
  subcommand nor an existing path`), so PDT passes the first prompt after `--`, last.
- Bypass: `--permission-mode dangerous` (aliases `yolo`, `bypass`).
- Resume: `--resume <id>` (`-r`). Ids are word pairs like `cool-abstract`.
- Devin saves a session only at its first prompt (`sessions.created_at` is the prompt time). Resuming an id it never
  saved does not fail: it quietly starts a new session with a new id.
- No flag for instructions. Devin reads AGENTS.md, CLAUDE.md, `.devin/rules` etc. from the repo.
- `devin acp` is an Agent Client Protocol server (JSON-RPC on stdio); PDT drives Devin conversations through it (see
  "Conversations: `devin acp`" below).

## Activity: OSC 7501

Devin's TUI emits the same program-status sequence as Codex, with `app=devin`:

```
ESC ] 7501 ; state=idle:app=devin BEL
ESC ] 7501 ; state=working:app=devin:msg=<base64> BEL
ESC ] 7501 ; state=blocked:kind=permission:app=devin:msg=<base64> BEL   (approval prompt)
ESC ] 7501 ; state=done:app=devin:msg=<base64> BEL                      (turn ended)
```

PDT reads them with pdt-core `program_status(bytes, "devin")`; no hooks are needed for activity. It also sets the
window title (`OSC 0`/`30`, `devin: <title>`) and shell-integration marks (`OSC 133`).

## Hooks and `--config`

- Devin runs Claude-format hooks (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
  `PermissionRequest`, `Stop`, `PostCompaction`, `SessionEnd`) from `.devin/hooks.v1.json`, `.devin/config*.json`,
  `.claude/settings*.json` in the project and `~/.config/devin/config.json`, `~/.claude.json`, `~/.claude/settings*.json`
  for the user. No flag or environment variable adds a hooks file.
- `--config <path>` *replaces* the user config file (`~/.config/devin/config.json`; `%APPDATA%\devin\config.json` on
  Windows), which holds the user's settings, including `devin.org_id`, and is where Devin saves settings back
  ("always allow", theme). It is JSON with comments.
- A hook's `command` runs through a shell; an `args` list is ignored. So PDT quotes the program itself.
- Every payload has `session_id` (stable for the session) and `hook_event_name`. `SessionStart` has `source`
  (`startup` or `resume`) and fires at launch, before the first prompt, with the id the session will be saved under;
  a resumed session reports its own id again.
- A `SessionStart` hook that prints `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"…"}}`
  adds that text to the model's context (verified: the model repeated a word given only there).

## What PDT does

Each Devin terminal gets `--config <data dir>/sessions/<id>/devin-config.json`: the user's own config (parsed as JSONC)
with one `SessionStart` hook appended after the user's own: `<pdtd> hook devin --instructions <session dir>/instructions.md`.
`pdtd hook devin` posts `{source: devin, event, sessionId}` to the session's activity route, which records the id as
`external_session_id` (the latest report wins, since a resumed-but-unsaved session comes back with a new id), and on
`SessionStart` prints the instructions as context. Restart and auto-resume launch `--resume <that id>`.

Checked end to end on a throwaway dev pdtd with the real Devin: the first answer quoted the PDT session id from the
instructions, the conversation id was recorded, activity went idle → working → idle, and after kill + restart Devin
resumed the conversation and recalled the earlier request. A new folder shows Devin's own trust dialog first.

If the user's config cannot be read or parsed, Devin starts without `--config` (no PDT hook, so no resume id and no
instructions) rather than on a copy that lost the user's settings.

Known cost: settings Devin saves during a PDT session go to PDT's copy, not the user's file; the next launch copies the
user's file again. The user chose this over no hooks (no exact resume, no PDT instructions).

## Conversations: `devin acp` (PDT-55)

Checked against `devin 3000.11.92` on macOS on 2026-10-19 by driving `devin acp` from a script in a scratch repo
(recorded frames: crates/pdt-core/tests/fixtures/devin) and then through PDT with the real Devin. Mapping: pdt-core
`conversation/acp.rs` with Devin's extensions in `conversation/devin.rs`; process: pdt-agents `adapter/acp.rs` with
Devin's profile in `adapter/devin.rs`; stand-in for tests: `fake-agent acp`.

### Transport and start

- `devin [--config <file>] acp`: JSON-RPC 2.0, one message per line on stdout; logs go to stderr (and
  `~/.local/share/devin/cli/logs/`). Closing stdin ends it. `--config` works as for the TUI (a global option before
  `acp`), and the `SessionStart` hook runs under ACP too (`source: startup`, `session_id` = the ACP session id); its
  `additionalContext` reaches the model (verified with a secret word). So PDT hands Devin its instructions the same
  way as in a terminal. PDT does not set `PDT_SESSION_ID` for a conversation, so the hook only prints them.
- `initialize {protocolVersion: 1, clientCapabilities}` → `agentCapabilities.loadSession: true`,
  `promptCapabilities.image: true`, `sessionCapabilities` (list, delete, close; no fork), `authMethods`
  (`devin-browser`), `agentInfo.version` `0.0.0-dev` (useless). PDT offers no `fs` or `terminal` (Devin uses its own
  tools) and `elicitation: {form: {}}` (needed for questions).
- `session/new {cwd, mcpServers: []}` → `{sessionId, modes, configOptions}`; ids are word pairs (`dazed-dragon`), the
  same ids `devin --resume` takes. Before the answer Devin sends `config_option_update`, `current_mode_update` and
  `available_commands_update` for the new session.
- Devin saves a session only at its first prompt: `session/load` (or `session/prompt`) of one never prompted fails
  with `-32016 Session not found` (`data["cognition.ai/errorKind"] = "session_not_found"`). PDT then starts a new
  session and says so.
- `session/load {sessionId, cwd, mcpServers}` replays the whole conversation as `session/update`s (user and agent
  chunks, tool calls) before it answers `{modes, configOptions}`. PDT has logged those turns already, so it drops
  everything but settings and commands until the answer.

### Modes and models

- Modes (`session/set_mode {modeId}`, or `session/set_config_option {configId: "mode", value}`): `accept-edits`
  ("Code", the default: edits in the folder run without asking, commands Devin does not judge safe ask), `smart`,
  `ask` (read-only Q&A), `plan`, `bypass`. There is no mode that asks before edits; the CLI's `auto` is not offered
  over ACP (setting it is silently ignored). PDT maps Ask and Accept edits to `accept-edits`, Plan to `plan`, Bypass to
  `bypass`, and offers only Accept edits / Plan / Bypass for Devin. Devin reports every change
  (`current_mode_update`, also after a plan is approved).
- Models: the `model` config option (`currentValue`, `options[{value, name}]`); set with
  `session/set_config_option {configId: "model", value}`. There is no "default" value to go back to.

### A turn

- `session/prompt {sessionId, prompt: [ContentBlock]}` (text; images as `{type: image, mimeType, data}`). Its answer
  `{stopReason: end_turn | cancelled | max_tokens | max_turn_requests | refusal, usage}` ends the turn. No user-message
  echo, no message or turn ids.
- A `session/prompt` sent while a turn runs is taken into that turn (`cognition.ai/midTurnPrompts`): both calls are
  answered together when it ends. PDT shows it as a steered message; "Send now" cancels first.
- `session/update` kinds: `agent_message_chunk` / `agent_thought_chunk` (text deltas), `tool_call`, `tool_call_update`,
  `usage_update {used, size}` (sent twice: once more with `_meta["cognition.ai/subagent_context"]`, ignored),
  `session_info_update {title}`, `config_option_update`, `current_mode_update`, `available_commands_update`. Devin also
  sends its own notifications (`_cognition.ai/thinking_complete`, `turn_stats`, `agent_stopped`,
  `mcp/serversChanged`), which PDT ignores.
- Tool calls: `toolCallId`, `title` ("Read file", "Ran touch"), `kind` (read, edit, execute, switch_mode), `rawInput`
  (`command`, `file_path`, …), `_meta["cognition.ai/inferenceToolName"]` (read, edit, write, exec, write_plan,
  exit_plan_mode, ask_user_question). Updates carry `status` (pending/in_progress/completed/failed) and `content`:
  `{type: diff, path (absolute), oldText (absent for a new file), newText}` with the whole file, or
  `{type: content, content: {type: text}}`, where a command's output is resent in full each time.
  `_meta.terminal_exit.exit_code` gives a command's exit code. Paths are canonical (`/private/tmp/…` for `/tmp/…`).
- `session/cancel {sessionId}` (a notification): the running tool call fails (`cognition.ai/canceled`), the prompt
  answers `stopReason: cancelled`, and a late `completed` update for the cancelled call can follow (PDT keeps it
  failed).

### Requests from Devin

- `session/request_permission {toolCall: {toolCallId}, options: [{optionId, name, kind}]}`. For a command:
  `allow_once`, `allow_session` ("this session", kind `allow_always`), `allow_always` (saved for the repo),
  `allow_always_global` (saved for every project), `switch_bypass`, `reject_once`. Answer
  `{outcome: {outcome: "selected", optionId}}` (or `{outcome: "cancelled"}` when cancelling). PDT's "Always allow" is
  `allow_session` only, never a saved rule. A rejection carries no message: PDT sends the user's note as the next
  prompt (taken into the turn). The rejected call fails with `_meta["cognition.ai/rejected"]`.
- Plans (plan mode): Devin writes the plan with `write_plan` (a diff to `~/.devin/plans/plan-*.md`; `rawInput.content`
  is the plan without its front matter), then an `exit_plan_mode` tool call (`_meta["cognition.ai/isExitPlan"]`)
  and a permission request with `plan_accept_edits`, `plan_bypass`, `reject_once`.
- Questions (`ask_user_question`): `elicitation/create {mode: "form", message, requestedSchema}` with one string field
  per question (`q0`, `q1`, …; `title` = header, `description` = question, `oneOf: [{const, title}]` = options;
  multi-select fields are arrays) and `_meta["cognition.ai/allowOther"]`. Answer
  `{action: "accept", content: {q0: "Blue"}}`, `{action: "decline"}` or `{action: "cancel"}`. Without the client's
  `elicitation` capability Devin cannot ask.

### Switching to the terminal and back

Checked with the real Devin: `devin --resume <id>` opens a session ACP made (the ids are the same), `/exit` then Enter
quits the TUI in about a second, and the next `session/load` replays the turns taken there. Every replayed prompt
carries `_meta["cognition.ai/clientMessageId"]` and `_meta["cognition.ai/timestamp"]` (RFC 3339), so PDT stores the
switch time in `history_from` and imports the replayed turns whose prompt is newer (pdt-core `iso_ms`). A session is
locked to one process, so the TUI and the ACP server never run at once: switching parks one before starting the other.
A folder Devin has not seen shows its trust dialog in the terminal first.

### Not done

- Make ticket: ACP has no fork, so a side request would land in the conversation. The route answers 409
  `draft_unsupported` for Devin and the web hides it.
- Windows is untested.
