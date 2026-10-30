# Invariants that are easy to break

Rules the code relies on that a reasonable-looking change can quietly break. Each says where it is enforced, so a
change near it can keep it. The map of the code is in [architecture.md](architecture.md).

## The user's machine

- **The user's own agent settings and config files are never edited.** Claude gets per-session `--settings` and
  `--mcp-config` files, Codex `-c` flags and thread config, Devin a copy of its config with PDT's hook added. Codex's
  `notify` and `developer_instructions` are chained or prepended, never replaced (pdt-agents `hooks.rs`, `mcp.rs`,
  `adapter/`).
- **"Always allow" is session-only for every agent** (Claude `destination: "session"`, Codex `acceptForSession`,
  ACP `allow_session`), including switching mode after a plan. An agent's own option that saves a rule beyond the
  session (Cursor's "Allow always", Grok's `always-allow`) is never offered or picked; one kept for the session only
  (Antigravity's "Allow Always", in its server's memory) may be.
- **Children never inherit pdtd's `PDT_*` variables** (PDT sets the session's own afterwards), so a dev pdtd started
  in a stable-hosted terminal can't pick up the stable port or data folder. Agents also lose
  `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` under the subscription policy (pdt-core `session.rs`). Claude conversations
  never get `--bare` (it drops the plan login).
- **The stable instance (port 7420) is the user's.** Agents never stop, restart or update it, never call its
  `/api/shutdown`, and tests never touch it or the real data folders ([testing.md](testing.md)). Only pdt-host and the
  user's Update do. A dev instance never updates; updates never install an older version.

## Talking to agents

- **Nothing is sent to an agent without the user's click or a workflow the user enabled.** Built-ins such as PR
  notices are enabled by default and can be turned off. Messages from workflows go through the outbox
  (pdt-api `workflow/outbox.rs`), which waits until a terminal has been idle with nobody typing, or a conversation is
  idle with nothing open; a message that can't be delivered ends `untold` and notifies the user. A session whose
  state is unknown (its hooks never fired) is never typed into.
- **Conversations are never auto-resumed**, and never as terminals. After a pdtd restart their running turn ends as
  interrupted; the UI offers Continue. Only interrupted terminal sessions are resumed by themselves.
- **Text from outside is fenced.** Issue and PR text, webhook payloads and other outside fields (`EventType.outside`)
  are fenced when they reach an agent (pdt-core `fence.rs`, pdt-api `untrusted.rs`), and sessions that read untrusted
  input run locked down (`persona::locked_down`: read-only tools, no secrets).
- **Routes that act as the user need the UI token** (`x-ui-token`, new at each start; pdt-api `untrusted.rs`): admitting
  an external ticket, filing issues, the PM's approvals. An agent's `curl` can't do them.
- **Fresh Codex launches in one folder stay at least 5 s apart**, or PDT can't tell their conversations apart.

## Tabs, tickets and attempts

- **A session, run, ticket or worktree's changes has at most one tab**, and closing a session's or run's tab stops and
  removes it (unique columns on `workspace_tabs`).
- **One active attempt per ticket** (the `one_active_attempt` index; 409 `ticket_active`).
- **Parent tickets are never started**; their parts are (409 `ticket_has_parts`).
- **PR state, CI, file overlaps and parts progress are computed in memory** and added to ticket responses, never
  stored. A restart forgets them until the next poll.
- **A branch counts as merged only after it had commits of its own** (`own_commits`, sticky), so a branch that only
  fast-forwarded to the base is not merged.
- **An automated merge never merges locally when a PR may exist**, and needs green CI from every provider and the
  policy's approval; a person's merge skips those gates (pdt-api `review::merge_attempt`, pdt-core `policy.rs`).
  Where the forge can't hold a merge (no auto-merge), a person's merge waits for green CI too, and PDT's own merge once
  it passes is that person's request: it skips the policy, as the forge's auto-merge would (`review::merge_held`).
- **Finish never discards uncommitted changes** unless asked (`discardChanges`), and runs in its own task so it
  completes when the agent that called it is stopped.
- **Workflows hear of a Start only once its session exists** (`announce_start`).
- **Only the user's ticket comments go to the agent**; the API refuses `by: "system"`.
- **Hiding a session from the lists never hides it from attention.** The session lists (Agents, the phone drawer,
  Bring here, the palette) read `useWorkspaces.allInfos`, which leaves out workflow step sessions for a ticket and
  ended ones with no ticket unless a tab holds them (`hiddenStepSession`, web `sessions/status.ts`); the favicon and
  title read `everyInfo`, every session, so a hidden step that needs the user still shows.

## Workflows

- **Everything runs at least once, maybe more.** Events are deduplicated by their key, runs by their run key. A new
  node kind with side effects either records its progress or sets `rerun_safe = false` (pdt-api
  `workflow/catalog.rs`), so a step cut short by a crash isn't repeated blindly.
- **Never hold `engine_lock` across a slow node body.** Node bodies run outside it; a result is written only if the
  step is still `running` with the same attempt.
- **Secrets are filled in only when a node runs**, only from the node's own config, never from text that arrived as
  data. Stored step inputs, outputs and errors are redacted. Errors from outbound HTTP go through `send_error`, which
  drops the URL (it can carry a secret).
- **Event payloads are an API.** Users' workflows read them as `{{trigger.x}}`; renaming a field breaks them silently.
- **`_pdt.*` state keys are PDT's own.**
- **Built-in workflows, templates and personas follow newer versions only while the user hasn't edited them.** Repo
  workflows (`.pdt/workflows/`) are read-only through the API; imported and instantiated workflows start disabled.

## Secrets and GitHub

- **Agents and workflows use secrets but never see them.** Values never go in the database, a response, a log or a
  tool result; `pdt_core::secret::Redactor` replaces them wherever the store, pdt-mcp, conversations and the logs
  write.
- **Tokens are never returned by the API or logged**, and follow a redirect only to the same host.
- **A dev instance never reads stable's keychain entries**: keys outside the stable data folder are scoped by the data
  folder (pdt-api `secrets.rs`).
- **On Linux the secret backend sticks to its data folder** (keychain or file); it never switches.
- **Nothing runs `gh` or `glab`.** Forges go through their crates over HTTP (`pdt_forge::transport`); agents use
  pdt-mcp's forge tools.

## Requests

- **The Host guard and owner-only checks cover every route** (pdt-api `host_guard.rs`, `identity_guard.rs`): only
  loopback hosts and the allowed hosts; remote requests carry the owner's Tailscale identity. Hooks
  (`/api/hooks/*`) authenticate with their own secret instead. Don't add a route outside the guards.

## Code and data

- **Never edit an applied migration** (sqlx checksums them); add a new one named by its UTC creation time. Data
  backfills that run when the store opens go in `Store::open`.
- **`.pdt/project.toml` must stay readable by the stable release.** Unknown top-level sections are skipped, but unknown
  keys inside `[[run]]`, `[[test]]` and `[[area]]` are refused, so a new section waits until stable supports it.
- **Names live only in `crates/pdt-core/src/brand.rs`** (plus the Tauri configs, the Windows installer hooks and the
  release scripts): product name, slug, daemon, ports, release repository, keychain service, also in UI text.
- **Guides served by pdtd link to each other by API path**, never by repository path (a test checks).
- **New tools and node kinds change the integrations snapshot** (`crates/pdtd/src/integrations/`): update it on
  purpose.
- **The UI re-renders every 2 s**: effects must be idempotent, and never focus from an inline ref callback.
