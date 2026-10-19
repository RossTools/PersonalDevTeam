# Phase 5: agent protocols as verified on 2026-10-18

Facts the structured adapters rest on (spec: docs/superpowers/specs/2026-10-18-structured-agents-design.md).
[V] seen in a real run on the Mac mini, [D] from docs or the generated schema only, [?] unknown.
Versions: Claude Code 2.1.302, codex-cli 0.163.0. Devin CLI v3000.11.92 from docs only (not installed here).

## Claude Code: `claude -p` stream-json

Command: `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages
--replay-user-messages --permission-prompts host --permission-prompt-tool stdio --session-id <id>` (or `--resume <id>`).

- **`--permission-prompt-tool stdio` is required** (hidden; the Agent SDK passes it). With `--permission-prompts host`
  alone every prompt is silently denied (`system/permission_denied`). The flag also adds `AskUserQuestion`,
  `EnterPlanMode`, `ExitPlanMode` and `Task*` to init's `tools`. [V]
- One process serves many turns; it exits 0 when stdin closes. [V]
- User turn (stdin): `{"type":"user","message":{"role":"user","content":"text" | [blocks]},"parent_tool_use_id":null,
  "session_id":"<id>"}`. Image block: `{"type":"image","source":{"type":"base64","media_type":"image/png","data":…}}`. [V]
- Sent while a turn runs: if a tool is running it is injected at the next tool boundary (same turn, one `result`);
  otherwise it queues as the next turn. `"priority":"now"` aborts the running tool and runs it as a fresh turn. The
  `--replay-user-messages` echo (`isReplay:true`, `uuid`) arrives when the message is taken, not when sent. No
  queued-ack event; `result.queued_turn_count`. [V]
- Output: `system/init` (re-sent every turn: `tools`, `model`, `permissionMode`, `slash_commands`,
  `terminal_slash_commands`, `skills`, `agents`, `mcp_servers`, `apiKeySource`, `claude_code_version`, `capabilities`);
  `system/status`, `system/compact_boundary`, `system/task_*`; `assistant` — **one event per content block**, blocks
  of one message share `message.id`; thinking text arrives empty; `user` tool results (`tool_result`, structured
  `tool_use_result`); `stream_event` partials (Anthropic message-stream events); `rate_limit_event` (five_hour /
  seven_day utilisation); `result` (`subtype` success / error_during_execution, `usage`, `modelUsage`,
  `total_cost_usd` — a running process total at list price, `permission_denials`, `terminal_reason`). [V]
- Permission request (stdout): `{"type":"control_request","request_id":…,"request":{"subtype":"can_use_tool",
  "tool_name","display_name","input","description","permission_suggestions":[…],"blocked_path","tool_use_id"}}`.
  Answer (stdin): `{"type":"control_response","response":{"subtype":"success","request_id":…,"response":R}}` with R =
  `{"behavior":"allow","updatedInput":{…}}` | `{"behavior":"deny","message":"…"}` (optional `interrupt:true` [D]).
  "Always allow" = allow plus `updatedPermissions:[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":
  "touch *"}],"behavior":"allow","destination":"session"}]` [V]. `destination:"localSettings"` writes
  `<cwd>/.claude/settings.local.json` — PDT must never send it. An edited `updatedInput` runs the edit but the
  model's own `tool_use` still shows the original. Read-only commands don't prompt. [V]
- AskUserQuestion arrives as `can_use_tool` (`requires_user_interaction:true`, `input.questions[{question, header,
  options[{label,description}], multiSelect}]`); answer allow with `updatedInput:{…input, "answers":{"<question>":
  "<label>"}}`. ExitPlanMode arrives the same way with `input:{plan, planFilePath}`; allow leaves plan mode, deny with
  a message makes it re-plan. [V]
- Control requests (stdin `{"type":"control_request","request_id","request":{"subtype":…}}`): `initialize` (returns
  `commands[{name,description,argumentHint}]`, `models[]`, `account{subscriptionType}`, `current_permission_mode`) [V];
  `interrupt` (→ `[Request interrupted by user]`, `result` error_during_execution, process stays up) [V]; `set_model`,
  `set_permission_mode` [V]; `mcp_status`, `get_context_usage` [V]; `rewind_files`, `stop_task`, `rename_session`,
  `get_usage`, `cancel_async_message` [D]. Unknown subtype → `{"subtype":"error",…}`. [V]
- Slash commands as user text: custom commands expand and run; `/context`, `/model` return a `<synthetic>` assistant
  message and `result num_turns:0`; `/compact` works (`compact_boundary`). [V]
- Resume: `--resume <id>` keeps id and context; **nothing is replayed on stdout**. History lives in
  `<config dir>/projects/<cwd key>/<id>.jsonl` (the same file the TUI uses; entries linked by `uuid`/`parentUuid`,
  `entrypoint:"sdk-cli"`). [V]
- **TUI ↔ `-p` round trip (checked 2026-10-18, 5a's first task)**: interactive `claude --resume <id>` on a session made
  by `-p --session-id <id>` opens it with its history shown, keeps the same id and file, and appends its turns with
  `entrypoint:"cli"`; a later `-p --resume <id>` sees the TUI's turns too. The TUI shows its folder-trust prompt in a
  folder never trusted before (`-p` skips it), so the handoff (5c) must expect that prompt in a fresh folder. [V]
- The `control_response` PDT sends for `can_use_tool` is echoed back on stdout (same `request_id`); ignore it. [V]
- **Edits and plans (checked 2026-10-19, 5c, a Haiku `-p` run in plan mode; fixture `plan.jsonl`)**: Edit's
  `tool_use_result` has `structuredPatch: [{oldStart, oldLines, newStart, newLines, lines}]` (lines prefixed ` `/`+`/
  `-`); Write gives `{type: "create", content, structuredPatch: []}` for a new file. Approving ExitPlanMode with
  `updatedPermissions: [{type: "setMode", mode: "acceptEdits", destination: "session"}]` goes straight to Accept edits
  (the following Edit/Write ran without a prompt); `system/status` carries the new `permissionMode` at once. Plan mode
  writes its plan with Write to `~/.claude/plans/<slug>.md` (outside the project). [V]
- **Transcript (`<config dir>/projects/<key>/<id>.jsonl`, fixture `transcript.jsonl`)**: `user` / `assistant` /
  `system` entries mirror the stream frames (`toolUseResult` instead of `tool_use_result`, one `assistant` entry per
  content block), plus bookkeeping types to skip (`attachment`, `queue-operation`, `last-prompt`, `api-request*`,
  `atis-latch`, …); `isMeta` entries (command caveats) and `isSidechain` entries are not the conversation. There is no
  `result`: a turn ends at the next real prompt, at `[Request interrupted by user]`, or (TUI turns) at
  `system/turn_duration {durationMs}`. Local commands appear as `<command-name>/x</command-name>…<command-args>` and
  `<local-command-stdout>…`. [V]
- **Slash commands in `-p` (checked 2026-10-19)**: a custom command (`.claude/commands/hi.md`) is replayed as
  `<command-message>hi</command-message>\n<command-name>/hi</command-name>` (plus `<command-args>`); a local one
  (`/context`) is not replayed at all: an `assistant` message and a `result` follow. [V]
- **Codex schema (codex-cli 0.163.0, `codex app-server generate-json-schema`)**: `thread/compact/start {threadId}` → `{}`;
  `review/start {threadId, target: {type: uncommittedChanges} | {type: baseBranch, branch} | {type: commit, sha} |
  {type: custom, instructions}, delivery?}`; `turn/start` input items include `{type: localImage, path}`;
  `account/read` → `{account: {type: apiKey} | {type: chatgpt, email, planType} | {type: amazonBedrock}}`. [D]
- **The TUI leaves on a typed `/exit`** (written, then Enter 150 ms later so it isn't read as a paste): a fresh TUI in
  a trusted folder exited 0 about 1 s after Enter. [V]
- Windows `claude -p`: not checked by hand (no Windows machine here). The adapter pipes stdio with no PTY, through
  `cmd.exe /d /c` for a `.cmd` shim (flags only, no free text on the command line); the fake-agent tests cover that
  path on the Windows CI.
- Billing: the claude.ai subscription (`apiKeySource:"none"`, `subscriptionType:"Claude Max"`, subscription rate-limit
  windows). [V] Agent SDK docs: "Unless previously approved, Anthropic does not allow third party developers to offer
  claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK." [D]
- `--settings` hooks still fire in `-p` (UserPromptSubmit, PreToolUse, Stop); no Notification hook for host-answered
  prompts. [V]
- Samples: scratch logs from the verification run were not kept in the repo; re-record into the fake-agent fixtures.

### Side requests (checked 2026-10-19, 2.1.302)

`claude -p --resume <id> --fork-session --no-session-persistence --tools "" --output-format json --json-schema '<schema>'`
with the prompt on stdin answers one `result` object whose `structured_output` holds the schema's object (`result`
holds it as text; `num_turns` is 2: the schema is a tool). Nothing is saved: the project folder keeps only the original
`<id>.jsonl`. Works while the conversation's own `-p` process is live. Make ticket uses it (5d).

### `/clear` (checked 2026-10-19, 2.1.302)
- Sent as a user message, `/clear` is not replayed. Claude prints `{"type":"conversation_reset","trigger":"clear","new_conversation_id":A,"uuid":A,"user_message_uuid":…,"session_id":<old>}`, then a `system/init` and a `result` (`local_command: "clear"`, `num_turns: 0`) whose `session_id` is a third id B.
- Every later frame carries B, and the transcript is `projects/*/B.jsonl` (it starts with the `/clear` command entries); no file is named after A. `--resume B` continues the cleared conversation and keeps B.
- So PDT follows the `session_id` of each `init`, never `new_conversation_id` (fixture: crates/pdt-core/tests/fixtures/claude/clear.jsonl). In the TUI, the SessionStart hook (`source: "clear"`) reports B as its `session_id`.

## Codex: `codex app-server`

- Handshake: `initialize {clientInfo:{name,version}, capabilities:{experimentalApi?}}` then the `initialized`
  notification; anything earlier gets "Not initialized"; experimental methods need `experimentalApi:true`.
  `clientInfo.name` becomes the thread's originator and notify's `client`. [V]
- One process serves many threads at once; every notification carries `threadId`. [V] An idle thread with no
  subscriber unloads after 30 min (`thread/closed`). [D]
- `thread/start {cwd, model, approvalPolicy: untrusted|on-request|never, sandbox: read-only|workspace-write|
  danger-full-access, baseInstructions, developerInstructions, config{}}` → `{thread:{id, path, status}}` +
  `thread/started`. MCP servers come from config (`config{}` / `-c`), not a thread param. [V]
- `turn/start {threadId, input:[{type:text,text}|{type:localImage,path}|{type:image,url}|{type:mention,name,path}],
  model?, effort?}` (overrides stick). [V for text; D for images]
- One turn: `thread/status/changed active` → `turn/started` → `item/started|completed` (userMessage; agentMessage
  `phase: commentary|final_answer`; commandExecution `{command, cwd, status, aggregatedOutput, exitCode}` with
  `item/commandExecution/outputDelta`; fileChange `{changes[{path,kind,diff}]}`; reasoning; plan; mcpToolCall;
  webSearch; contextCompaction) → `item/agentMessage/delta` → `thread/tokenUsage/updated` (`modelContextWindow`) →
  `account/rateLimits/updated` → `turn/diff/updated {diff}` (whole-turn unified diff) → `turn/completed {status:
  completed|interrupted|failed}`. [V]
- Approvals are server requests: `item/commandExecution/requestApproval {itemId, command, cwd, reason,
  availableDecisions}` and `item/fileChange/requestApproval {itemId, reason}` (after the item's `item/started`), answered
  `{decision: accept|acceptForSession|decline|cancel}` (cancel also interrupts the turn). Thread `activeFlags`
  shows `waitingOnApproval`; `serverRequest/resolved` follows any answer. Legacy `execCommandApproval` is not sent to
  v2 clients. [V]
- `item/tool/requestUserInput` (questions with options, `isOther`) fires in plan collaboration mode only today
  (experimental); answer `{answers:{<id>:{answers:[label]}}}`; flag `waitingOnUserInput`. [V]
- `turn/interrupt {threadId, turnId}` → `turn/completed interrupted`; a running shell command is not killed (finished
  7 s later). `turn/steer {threadId, expectedTurnId, input}` injects into the running turn; `turn/start` during a turn
  behaves like a steer. Queue (`thread/queue/*`) is experimental. [V]
- History: `thread/resume {threadId, excludeTurns:true}` loads it; `thread/turns/list {threadId, itemsView:"full",
  sortDirection:"asc", cursor}` returns every item (deltas not stored). Thread id = the TUI's session id and the
  rollout file's id. [V]
- Other RPCs: `thread/compact/start`, `review/start {target}`, `thread/shellCommand`, `thread/name/set`,
  `model/list`, `skills/list`, `account/read` [V]; `thread/fork`, `thread/revert`, `thread/goal/*` [D]. No generic
  "run slash command".
- **Shared live thread**: `codex app-server --listen unix://<path>` (WebSocket over a unix socket; path ≤ 108 bytes,
  Codex symlinks long ones). A client there plus the real TUI via `codex resume <id> --remote unix://<path>` share
  thread `<id>` live: each sees the other's turns, both get approval requests, first answer wins. [V] `ws://IP:PORT`
  with `--ws-auth` also exists [D]. Windows transport: [?].
- The user's own shared daemon (`~/.codex/app-server-control/…sock`, `codex agents`) is separate. A plain `codex`
  attaches to it; `codex -c …` ran embedded instead (one observation each). PDT must not use that daemon. [V]
- Auth: the ChatGPT login (`account/read` → `type:"chatgpt"`, plan rate-limit windows). Docs: hosted or commercial use
  of app-server auth is not permitted. [V/D]
- `-c notify=[…]` still fires per completed turn under app-server (also for turns from an attached TUI). [V]

- **5b checks (2026-10-18, codex-cli 0.163.0, macOS)**: `codex app-server --listen ws://127.0.0.1:<port> --ws-auth
  capability-token --ws-token-file <file>` accepts clients sending `Authorization: Bearer <token>`; `/readyz` answers
  when it is listening. The real TUI attaches with `codex resume <thread> --remote ws://127.0.0.1:<port>
  --remote-auth-token-env <VAR>` and shares the thread live with another client (that client saw the TUI's whole
  turn after a `thread/resume {excludeTurns:true}`). So PDT uses loopback ws + a capability token on every OS
  instead of a unix socket: one transport, the same on Windows (plain TCP), and the token file is 0600 in PDT's data
  dir. [V on macOS; Windows: same TCP path, not run by hand]
- Server→client requests have their own JSON-RPC ids (starting at 0, per connection); approvals name the item
  (`itemId`), and `serverRequest/resolved {requestId}` follows the answer. `item/commandExecution/requestApproval`
  carries `command`, `cwd`, `commandActions`, `availableDecisions` (accept, acceptWithExecpolicyAmendment, …). [V]
- `thread/tokenUsage/updated {tokenUsage:{total,last,modelContextWindow}}`; `account/rateLimits/updated
  {rateLimits:{primary,secondary:{usedPercent,windowDurationMins,resetsAt (s)},planType}}` (no threadId). Noise per
  turn: `turn/moderationMetadata`, `thread/readState/changed`, `mcpServer/startupStatus/updated`,
  `remoteControl/status/changed`, `account/updated`. [V]

### Side requests (checked 2026-10-19, 0.163)

`thread/fork {threadId, ephemeral: true, excludeTurns: true, sandbox: "read-only", approvalPolicy: "never"}` returns
`{thread: {id}}` for a fork that is never written to disk (no rollout file); without `excludeTurns` an ephemeral fork
is refused ("ephemeral paginated thread/fork requires `excludeTurns: true`"). It works from a thread's rollout after the
server restarted. `turn/start` on the fork with `outputSchema` ends with an `agentMessage` whose text is the JSON
object; its notifications name the fork's thread id, so they never reach the conversation. Make ticket uses it (5d).

## Devin CLI: `devin acp` (docs only)

- Local agent working in its cwd; macOS/Linux/Windows; `~/.local/bin/devin`. [D] https://docs.devin.ai/cli/index
- `devin acp`: an Agent Client Protocol server (JSON-RPC 2.0, stdio), the route Zed/JetBrains use. Flow: `initialize`
  → `authenticate` (if asked) → `session/new {cwd, mcpServers}` or `session/load` → `session/prompt` (its response ends
  the turn) with `session/update` notifications (message chunks, thoughts, tool_call / tool_call_update, plan, modes,
  available commands); `session/request_permission` (allow once / always / reject; editable commands);
  `elicitation/create` for questions; `session/cancel`; `session/set_mode`; config options for model/effort. [D]
  https://agentclientprotocol.com/protocol/overview, https://docs.devin.ai/cli/changelog/stable
- No documented session id at launch; sessions are locked to one process (TUI and ACP can't hold one at once). [D/inferred]
- Terminal session: `devin [--permission-mode M] [--model X] [-- <prompt>]`, resume `devin -r <id>`; activity from
  Claude-compatible hooks (`.devin/hooks.v1.json`; `session_id` in payloads) or OSC 9/777 with `notify: always`. Devin
  **also loads hooks from `~/.claude/settings*.json` and `.claude/settings*.json`** by default. [D]
- Auth: `devin auth login` (browser; enterprise SSO); token in `~/.local/share/devin/credentials.toml`;
  `WINDSURF_API_KEY` for ACP. Billing: the org's ACUs. [D]
- To check hands-on on the work laptop: org policy allows a custom ACP client; `initialize` capabilities (loadSession,
  images, modes); ACP sessionId = `devin -r` id = hooks' `session_id`; lock behaviour with the TUI open; real
  `request_permission` option ids; cancel's `stopReason`; whether `--config <path>` merges (to add hooks without
  touching user files); whether `~/.claude` hooks fire.
