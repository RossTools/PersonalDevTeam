# OhMyPi's `omp acp` (PDT-238)

Facts PDT relies on, checked against `omp 18.14.0` (Homebrew) on macOS on 2026-10-29: live over stdio where marked
**live**, otherwise read from its source (https://github.com/can1357/oh-my-pi, `packages/coding-agent/src/modes/acp/`
and `src/session/acp-permission-gate.ts`, at `338d2ce`). Windows is untested.

## Launch and handshake (live)

- `omp acp` is the ACP server on stdio. Its credentials and model come from `~/.omp` (`authMethods`: "Use existing
  local credentials"); PDT passes nothing. With no model set up, `session/new` works and `session/prompt` fails
  `-32603 Internal error` with `data.details` "No model selected. …" (PDT adds the details to the error it shows).
- `initialize` answers `loadSession: true`, `mcpCapabilities: {http, sse}`, `promptCapabilities: {image,
  embeddedContext}`, and `sessionCapabilities` `list`, `fork`, `resume`, `close`.
- `session/new` answers `modes`: `availableModes` `default` ("Standard ACP headless mode") and `plan` (only while omp's
  plan setting is on), `currentModeId: "default"`, plus `configOptions` `mode` and `thinking`. No model option.
- Client-passed `mcpServers` (stdio) are honoured: omp starts the server, `initialize`s it and lists its tools at
  `session/new` (checked with a probe server). No project MCP file is needed.
- `session/load` of a saved id works and answers `modes` again; omp saves a session file at `session/new`, before any
  prompt. An unknown id fails `-32603` with details "ACP session not found: <id>".

## Permissions (source)

- Only `bash`, `delete`, `move`, and an `edit` that deletes or moves a file ask (`session/request_permission`;
  `PERMISSION_REQUIRED_TOOLS`). Plain edits, `write` and MCP tools never ask, in any mode.
- Options: `allow_once`, `allow_always`, `reject_once`, `reject_always` (ids = kinds). "Always" is remembered by omp for
  the process; PDT never offers it (plain ACP's rule).
- The request's `toolCall.kind` is set only for `bash` (`execute`); the others' kind is the earlier `tool_call`'s
  (`delete`, `move`, `edit`).

## Tool calls (source)

- `tool_call` carries a non-standard `name` (the tool: `bash`, `edit`, `read`, `ask`) beside `kind` (`read`, `edit`
  for write/edit, `delete`, `move`, `execute`, `search`, `fetch`, `think` for todo, else `other`).
- A command's content starts with `$ <command>`; its output follows as a further content block. Edits carry `diff`
  content (`path`, `oldText`, `newText`).
- `todo` results also arrive as `plan` updates.

## Questions and plan approval (source)

- Only for a client advertising `elicitation.form` (PDT does). Without it, `ask` gets no answer and plan approval is
  automatic.
- `ask` → `elicitation/create` `mode: form`, `message` the question (or "Answer N questions"), one field per question
  `q<n>`: `title` the question, `description` its header, `oneOf` (single) or `items.anyOf` (multi) of `{const, title,
  description}`, and `q<n>__other` (string, title "Other (type your own)") for the user's own words; no `required`. A
  single-choice answer is read only when `q<n>__other` is empty.
- Plan mode's approval → `elicitation/create` with one required field `value`, `enum: ["Approve and execute", "Refine
  plan"]`, the plan's first 12 lines in `message`. Anything but approve keeps plan mode.
