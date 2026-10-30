# Cursor's `cursor-agent acp` (PDT-237)

What PDT relies on, checked against Cursor CLI `2026.10.22-27dcf5f` on macOS on 2026-10-29. Facts marked **live** were
checked over stdio. The others were read from the CLI's bundled source (`~/.local/share/cursor-agent/versions/<v>/`,
mostly `7096.index.js`, the ACP mode) and https://cursor.com/docs/cli/acp. Facts marked **QA** come from a live agent
step on a scratch daemon on 2026-10-30, on a Free-plan account with model `default[]` (Auto). Windows is untested.

## Launch and handshake (live)

- `cursor-agent acp` (also installed as `agent acp`) is the ACP server on stdio, logging to stderr. It uses the
  credentials from `agent login` (`authMethods`: `cursor_login`). `session/new` works without an `authenticate` call.
- `initialize` answers `loadSession: true`, `mcpCapabilities: {http, sse}`, `promptCapabilities: {image}` and
  `sessionCapabilities.list`.
- `session/new` answers `modes`: `availableModes` `agent` (full tools), `plan` (read-only planning) and `ask`
  (read-only Q&A), with `currentModeId: "agent"`. It also gives `models` (`default[]` = Auto, then ids such as
  `claude-opus-5-5[context=300k,effort=medium,fast=false]`) and `configOptions` `mode` and `model`.
- `session/set_mode` works (undocumented): it reports `current_mode_update` and answers `{}`. An unknown id fails with
  `-32603` and `data.details` "Invalid mode ID: bogus. Valid modes: agent, plan, ask".
- Models are set with `session/set_config_option` `{configId: "model", value}` (and `session/set_model`). An unknown
  value fails with `-32602` and `data.message` "Invalid model value: …". On the Free plan only Auto (`default[]`)
  runs (**QA**). With no model set, Cursor uses the account's default model (here composer-2.5), which answers
  "Upgrade your plan to continue" as an agent message, and the turn ends `end_turn`.
- Client-passed `mcpServers` (`type: "http"`, with headers) are honoured, although the docs mention only
  `.cursor/mcp.json`. Cursor connects at `session/new`, sending the `Authorization` header, then `initialize`,
  `notifications/initialized`, a GET for the SSE stream, and `tools/list` at the first prompt. No per-session
  `.cursor/mcp.json` is needed. Its MCP clients are keyed by the server's `name` (source).
- `session/load` of an id with nothing saved fails with `-32602` and `data.message` `Session "<id>" not found`. Cursor
  writes `~/.cursor/acp-sessions/<id>/meta.json` at `session/new` but keeps the conversation (`store.db`) only once a
  turn has run.

## Permissions (source)

- `session/request_permission` options are `allow-once` / `allow_once`, `allow-always` / `allow_always` and
  `reject-once` / `reject_once` (id / kind). Its "Allow always" writes Cursor's own allowlist
  (`~/.cursor/cli-config.json` permissions), so PDT never offers it.
- The request's `toolCall.kind`: a write or edit is `edit` (with `diff` content), a delete is `edit`, a shell command is
  `execute` (title the command in backticks), a read is `read`, and an MCP call is `other` (title `<server>: <tool>`,
  content its arguments as JSON). Web fetches and searches are `fetch` and `search`.
- What asks depends on Cursor's own permission settings (`approvalMode` `allowlist` | `unrestricted` | `auto-review`,
  the allowlist, `--force`). PDT never passes `--force`, and keeps Accept edits and Bypass by answering requests
  itself (`acp_agent::enforced_answer`).
- Edits in the workspace never ask (**QA**: an Edit-tool write of `bye.txt` sent a `tool_call` of kind `edit` with a
  diff, and no permission request). Shell commands did ask. PDT can't keep Ask (it doesn't edit the user's Cursor
  settings), so Cursor offers no Ask, and a session in Ask runs as Accept edits, as Devin's does.
- Cursor asks before it reports the call with its `rawInput` (**QA**: the request came at seq 13, the `tool_call` at
  seq 16). The request's own `toolCall.rawInput` already holds `{providerIdentifier: "pdt", toolName}`, so PDT names the
  call from it.

## Tool calls (source)

- `tool_call` has `title`, `kind`, `rawInput` and `locations`. For an MCP call, `rawInput` is
  `{providerIdentifier, toolName, args}`, and `providerIdentifier` is the `mcpServers` name, `pdt` (**QA**). Some of Cursor's own tools give `_toolName` (`task`, `updateTodos`,
  `createPlan`, `askQuestion`).
- A command's output is in `tool_call_update.rawOutput` `{exitCode, stdout, stderr}` with no `content`. An edit's
  completed update carries `diff` content (`path`, `oldText`, `newText`).
- `switch_mode` is a tool kind. In the main session, a switch-mode request is rejected by Cursor itself
  ("Switch mode requires approval").

## Cursor's own requests (docs and source)

- `cursor/ask_question` (blocking): `{toolCallId, title?, questions: [{id, prompt, options: [{id, label}],
  allowMultiple}]}`. The answer is `{outcome: {outcome: "answered", answers: [{questionId, selectedOptionIds}]}}`,
  `{outcome: "skipped", reason}` or `{outcome: "cancelled"}`. There is no free-text answer: PDT sends an answer in the
  user's own words as `skipped` with the words in `reason`. If the client answers `-32601`, Cursor falls back to one
  `request_permission` per single-choice question, with each option as an `allow_once` option.
- `cursor/create_plan` (blocking, plan mode): `{toolCallId, name?, overview?, plan, todos, isProject?, phases?}`, with
  the to-dos also sent as a `plan` update. The answer is `{outcome: {outcome: "accepted", planUri?}}`,
  `{outcome: "rejected", reason?}` or `cancelled`. Cursor saves the plan file itself when no `planUri` comes back. If
  the client answers `-32601`, Cursor saves it and carries on as if it were accepted. Accepting doesn't leave plan mode,
  so PDT sets the mode the user picked and asks Cursor to implement the plan.
- Notifications PDT ignores: `cursor/update_todos`, `cursor/task` (sub-agents), `cursor/generate_image`.

## Prompt replay

- `session/load` replays each turn's prompt as `user_message_chunk` with no id or time, so PDT imports nothing from
  it. A `user_message_chunk` that arrives mid-turn (the ticket's "replays the user's prompt" quirk) is dropped, so the
  prompt isn't shown twice.

## Not yet checked live

`cursor/ask_question` and `cursor/create_plan` over the wire, and resuming a saved Cursor session.
