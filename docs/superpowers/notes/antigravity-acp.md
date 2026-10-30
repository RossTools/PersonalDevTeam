# Antigravity's ACP server (PDT-239)

Notes from running Google's Antigravity ACP server 1.4.5 (`agy-acp-server-1.4.5-darwin-arm64.zip`, the ACP registry's
`antigravity-acp` entry) by hand on macOS, signed in on the free tier, 2026-10-30. Reference:
docs/reference/agents.md "Antigravity".

## What it is
- The `agy` CLI (1.4.10) has no ACP mode. Google publishes the ACP server separately, as a zip per platform holding
  `agy_acp_server.par` (a Python zip app, 179 MB) and `localharness_external` (the Go agent harness it starts).
  `--help` lists only `--debug` and `--notices`.
- `initialize` answers `loadSession`, `mcpCapabilities {http, sse}`, `sessionCapabilities {list, resume,
  additionalDirectories}`, `_meta.antigravity {tier, systemPrompt.append}`, and four `authMethods`: `oauth-personal`,
  `oauth-business`, `gemini-api-key` and `agent-platform`.

## Sign-in
- Without one, `session/new` answers `-32000` "Authentication required", with `data.message` naming `authenticate`
  and `auth.type` in `~/.gemini/antigravity-acp/settings.json`.
- It doesn't reuse `agy`'s sign-in. Its source says its keychain account (`antigravity-acp`, service `gemini`) is kept
  apart from the CLI's `antigravity` "so this server never clobbers it".
- `authenticate {methodId: "oauth-personal"}` runs a loopback OAuth flow: it opens the browser
  (`webbrowser.open`), prints the URL to stderr, waits up to 300 s, then records `auth.type: oauth-personal` in its
  settings. Its answer has `_meta.antigravity.tier` ("Antigravity Starter Quota", `free-tier`). Later processes infer
  the sign-in from `settings.json`.

## Sessions
- Modes: `default` (asks), `auto_edit` (edits without asking) and `yolo` (asks nothing). `set_mode plan` is refused
  with `-32602` "Unknown mode id: plan". It has a `/plan` command (an implementation-plan artifact), which PDT doesn't
  use.
- `configOptions`: `model` (free tier: `gemini-3.8-flash` only), `thinking_level` (low/medium/high, high by default)
  and `mode`. They come in the `session/new` answer, and in `config_option_update` only when they change. A model it
  doesn't have is refused with `-32602` and `data.availableModels`.
- `session/load` of an unknown id: `-32002` "Session not found in the current GEMINI_HOME".
- `session/cancel` during a prompt: the prompt answers `stopReason: cancelled` at once.

## A turn (default mode)
- pdt-mcp over HTTP with its bearer header: it sends `server/discover`, then `initialize` (protocol `2025-11-25`),
  `tools/list` and `tools/call`. The call's frames carry `_meta.mcp {tool, server}`, are titled `<server>_<tool>`, and
  ask with Allow Always / Allow / Deny.
- A command (`kind: execute`, title the command, `rawInput.CommandLine`) asks with "Allow Always (risky)". Its result
  is `rawOutput {commandLine, workingDir, exitCode, combinedOutput}`.
- A new file (`kind: edit`, a `diff` content with `newText`) asks with Allow / Deny only. With client `fs` off, the
  server writes the file itself, under a second tool call (`<session>:<n>`, "Running edit_file"). At the end of the
  turn the first one is closed `failed` "Tool call was approved but never executed." In auto_edit, only one card
  appears.
- "Allow Always" decisions live in memory for the session (`_allowed_tool_calls_by_session_id`), keyed on the exact
  command for commands. Writes are never cached.
- Paths are reported with symlinks resolved (`/private/tmp/…`).

## Quirk: skills in the workspace
`session/new` materializes the harness's hidden system skills at `<cwd>/skills/system/run-workflow/SKILL.md`, whatever
the process's own folder is. Removing it doesn't help: the next prompt writes it again. Google's source already fixed
the same bug for `brain/<id>` ("Not the cwd: that wrote `brain/<id>` into the user's repo"). PDT leaves it alone for
now; it shows as an untracked `skills/` folder in a worktree.
