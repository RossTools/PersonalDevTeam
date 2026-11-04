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
- **Nothing from a release runs before it is verified.** pdt-update's `fetch_verified` trusts a release's
  `SHA256SUMS` only once `SHA256SUMS.sig` verifies against `brand::RELEASE_SIGNING_KEY` (when one is pinned), over the
  release's tag and `SHA256SUMS`, and the asset only once it matches. On macOS pdt-host also runs `codesign --verify`
  against `brand::MACOS_TEAM_ID` on the extracted `.app` before it removes quarantine and swaps it in. The install
  scripts pin the same key (a test checks) and check the same signature. Pinning a key or changing it:
  [SDLC.md](../SDLC.md) "Release signing".
- **CI and the release build run third-party actions only by commit SHA.** A tag can be moved to other code, which
  would build or upload a different release whose checksums still match. `scripts/check-action-pins.sh` enforces it;
  [SDLC.md](../SDLC.md) "Dependabot" says how to pin one.

## Talking to agents

- **Nothing is sent to an agent without the user's click or a workflow the user enabled.** Built-ins such as PR
  notices are enabled by default and can be turned off. Messages from workflows go through the outbox
  (pdt-api `workflow/outbox.rs`), which waits until a terminal has been idle with nobody typing, or a conversation is
  idle with nothing open; a message that can't be delivered ends `untold` and notifies the user. A session whose
  state is unknown (its hooks never fired) is never typed into.
  **The one exception is a watch** (pdt-api `watches.rs`): a PM conversation that set one with its `watch` tool is
  told about its target without asking anyone, since it only tells the conversation that asked. Its messages still
  go through the outbox, and a closed conversation's go to the user's Inbox instead.
- **Conversations are never auto-resumed**, and never as terminals. After a pdtd restart their running turn ends as
  interrupted; the UI offers Continue. Only interrupted terminal sessions are resumed by themselves. Background
  tasks a restart stopped (kept per session, `sessions.background_tasks`, moved to `stopped_background` by
  `recover_conversations`) are told to the agent only along with the next message the user or a workflow sends
  (pdt-api `conversations/mod.rs` `tell_stopped_background`); a workflow step waiting on them gets that message at
  once, as an automatic nudge (`workflow/agent/recovery.rs` `restarted_while_waiting`). Terminals don't report
  background tasks, so their agents aren't told.
- **Text from outside is fenced.** Issue and PR text (titles too, a linked issue's on any ticket: `get_ticket`,
  `link_issue`), documents' titles, content and comments (the document tools, `get_ticket`'s `documents`), webhook
  payloads and other outside fields (`EventType.outside`) are fenced when they reach an agent (pdt-core `fence.rs`, pdt-api `untrusted.rs`), and sessions
  that read untrusted input run locked down (`persona::locked_down`: read-only tools, no secrets). Bots' comments and
  reviews are outside text whatever their role on the repository (they relay others' text). A sub-workflow's run
  inherits its `workflow.call` chain's first run (`untrusted::origin`, `StepCx::origin`): when that run's trigger
  carries outside text, all of the child's `{{trigger…}}` is fenced (`fence::ALL`), its sessions are locked down and
  outsiders' text is refused as in the caller; the triage cap counts a workflow whose calls reach an agent step
  (`engine::reaches_agent`). New code that reads "was this run started by outside input" goes through
  `untrusted::untrusted_run` / `run_source`, never the run's own trigger alone.
- **What every user's agents read is about their project, not PDT's development.** The built-in personas'
  instructions, the PM's prompt, the tools' descriptions and schemas (doc comments on their argument types too), the
  agent context and the served guides name no ticket keys of PDT's own (examples use `ABC-12`), and nothing of PDT's own
  changelog, release workflow, nightlies or repository. Release guidance reaches the PM only through its briefing, for
  a project with an enabled release workflow. PDT's own process lives in AGENTS.md and docs/SDLC.md. pdtd's
  `prompts_and_guides_carry_no_pdt_development_assumptions` checks it (with an allow list).
- **Routes that act as the user need the UI token** (`x-ui-token`, new at each start; pdt-api `untrusted.rs`): admitting
  an external ticket, filing issues, the PM's approvals, approving a run configuration's commands, deciding a ticket
  proposal, Google's sign-in and picker token. This stops accidents (an agent calling the REST API approving its own
  prompt or proposal), not a process that means to: `GET /api/ui/token` hands the token to any request that sends
  `Sec-Fetch-Site: same-origin`, which a local process can (see "Requests" for the trust model). A new route that acts
  as the user still checks the token, and nothing may treat it as proof a person clicked.
- **A run configuration's commands run only once the user approved them** (pdt-core `command_trust.rs`, pdt-api
  `command_trust.rs`): runs and tests (`runs::create`, `restart`, and `run_manager`'s restarts), the local forge's
  `[ci] command` (it waits, with an Inbox notice) and `[[merge.regenerate]]` rules (read from the base branch's
  committed file, never the worktree's). An approval is the command's fingerprint (setup, command, env), kept in
  `trusted_commands`; only the UI token approves (`POST …/commands/trust`, or overrides saved from the web app).
  Projects from before carry a `legacy` mark that trusts the main checkout's commands once. A new place that runs a
  configured command checks it first.
- **Making tickets over the HTTP API records the user only with the UI token** (pdt-api `tickets::creator`, also
  parts): without it the caller is an agent, the session its `x-agent-session` header names (persona, the ticket it
  works on), and the ticket's source is `agent` (`pm`, `external` for a PM or triage session). The web client sends
  the token when it creates tickets and parts; a new route that makes tickets uses `creator` too.
- **Only the user decides a ticket proposal** (pdt-core `ticket_proposal.rs`): a held ticket is `proposed` from the
  transaction that makes it (`tickets.proposed_at`), before its Inbox item exists. The store refuses an agent's move
  of it out of Draft (closing it is allowed) or its `agentic` label; without the UI token, PATCH, bulk actions and
  Start refuse the same (`untrusted::decides_proposal`), and auto-start never starts it; the Inbox answer needs the UI
  token. Any move out of Draft releases it and closes the open `ticket_proposal` item; deleting the ticket removes it.
  Lists hide a ticket only while that item is open, so a held ticket with no item is never out of the user's reach.
- **The PM's `inbox_decide` never decides without the user's Allow** (pdt-api `mcp/tools/inbox.rs`): it is
  `asks_in_inbox` (pdt-core `ToolDecl::inbox_only`), so it is in Claude's `--allowedTools`, has no `ask_target` (no
  project policy answers it) and always asks: in a conversation with a card of PDT's own (`conversations::held`) whose
  `Request` event carries `rows` (the Inbox's approval item has them as `body.rows`), from a terminal with a `confirm`
  item with `body.rows`; the call waits on either. Only the user's Allow (the UI token: `held::answer` refuses an
  agent's yes) applies it, narrowed to the rows they kept (`AnswerRequest.rows` or `InboxAnswer.rows`, checked before
  the request is answered, so a wrong pick leaves it open). It refuses non-PM sessions even when a
  persona lists it. Each row goes through `ticket_proposal::decide` as `By::user_via(<the PM>)`: the user's events,
  with the PM as `proposedBy`.
- **An asking tool asks somewhere the user sees** (pdt-api `mcp::tools::confirmable`): Claude's permission prompt
  only while the session isn't in Bypass (`Session::asks_nothing`); otherwise, as for Codex and Devin, unless it is a
  PM session and the project lets the PM act (`pm::acts_without_asking`, which `pm::request_opened` uses too), a conversation gets an approval card of PDT's own (`conversations::held`: a `Request` event
  whose id starts `pdt-ask-`, which the Inbox lists as its approval item) and the tool call waits on it; a terminal gets
  an Inbox `confirm` item and the call waits on that (`workflow::agent::confirm_wait`: `answer_confirm` hands the answer
  to the waiting call, and only runs the call and messages the session itself when no call waits any more).
  `answer_request` answers PDT's own requests itself, never the agent's process, and `pm::request_opened` never answers
  them. The PM's `add_ticket_comment` on a ticket with an active attempt asks the same way whatever its agent
  (`mcp::tools::confirm_pm_comment`; Claude's prompt never covers it): "Ticket comments to the agent" sends the PM's
  comments to the attempt's agent, so it is a message to that agent, as `send_to_session` is. Agents get a budget for PDT's tools so the wait outlives their own default (pdt-agents
  `mcp::PDT_TOOL_TIMEOUT_SECS`): Codex as pdt-mcp's `tool_timeout_sec`, Claude as `MCP_TOOL_TIMEOUT` (every Claude
  process PDT starts, unless pdtd's environment has the user's own; Claude has no per-server setting, so it covers
  every MCP server in the session, which only matters to a call that waits). ACP agents get none: ACP's `mcpServers`
  has no timeout. The wait itself ends a little sooner (`AppState::ask_wait`) with a "still waiting" result rather
  than the agent's timeout: the Inbox item takes `answer_confirm`'s old way, and a conversation's card is marked
  detached, so a turn's end keeps it, and its answer runs the call and messages the session (`held::answered_late`).
  A `ToolDecl::public` tool (posts on the forge) asks in the Inbox outside an active agent step. A new path that runs an asking or public tool goes through `confirmable` (`AnyAction::call` does for actions).
- **Claude's prompt for an asking tool can't be switched off by a settings file** (pdt-agents `mcp::claude_permissions`):
  `confirmable` trusts Claude to have asked, so every Claude session not launched in Bypass gets an ask rule for each of
  its asking tools (`mcp__pdt__<tool>`) in the `--settings` file PDT passes (a terminal's hook settings file, a
  conversation's `claude-permissions.json`; Claude reads one `--settings`). Claude weighs ask rules before allow rules
  from every scope and prompts for them in every mode, so an allow rule in the user's, the project's or the worktree's
  `.claude/settings*.json`, or a mode switched in the terminal, doesn't skip the prompt. Leaving a tool out of
  `--allowedTools` alone is not enough. `mcp::server_for` records each launch's rules (`AppState::claude_ask_rules`),
  and `confirmable` trusts Claude's prompt only for a tool the running process has a rule for: a conversation's mode
  changes without a relaunch, so one launched in Bypass and switched out of it has none, and PDT asks itself. "Always
  allow" isn't offered on Claude's prompt for PDT's tools (an ask rule beats a session allow rule).
- **Tool lists fail closed** (pdt-api `mcp::session_tools`, pdt-core `PersonaSetup::tool_set`): an empty persona tool
  list is no tools, and a persona version that can't be read gives none. Only a session with no persona gets
  `DEFAULT_TOOLS`.
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
- **Cleaning build output never removes code or work.** A build output path is removed only when git ignores it and
  tracks nothing in it (pdt-git `is_disposable`, checked per path by pdt-api `storage::clean_checkout`), and never
  while a session or run works in the worktree. The automatic cleanup touches only worktrees of closed tickets with no
  active attempt, and removes a worktree only through `remove_worktree` without force or deleting the branch.
- **PDT's git never takes optional locks** (`GIT_OPTIONAL_LOCKS=0` on every git pdt-git runs). A read such as
  `git status` would otherwise refresh the index under `index.lock`, failing a git command the agent runs in the same
  worktree at that moment (the background size measuring reads worktrees agents work in).
- **PDT's git never runs the repository's own programs or reads a value as an option.** Every git pdt-git runs has
  `core.fsmonitor=false` (a repository's `core.fsmonitor` is a command `git status` runs), PDT's own commits
  (`PDT_COMMIT_CONFIG`) run no hooks, and a branch, base or commit from a request or a forge goes after
  `--end-of-options`. Bases are checked before they are stored: a project's base names a branch of its repository
  (pdt-api `projects::valid_base`), a Start's base a commit, and a forge's head SHA is used only when it is a full
  commit id (`is_commit_id`).
- **Finish never discards uncommitted changes** unless asked (`discardChanges`), and runs in its own task so it
  completes when the agent that called it is stopped.
- **Workflows hear of a Start only once its session exists** (`announce_start`).
- **Only the user's and the PM's ticket comments go to the agent** (`ticket.commented`'s `author`: `user`, else the
  writing session's persona), never a workflow's (`workflowId`) or the ticket's own agents', so an agent never hears
  back what it wrote; the API refuses `by: "system"`. A comment's attachments' local paths are filled in by pdt-api
  as runs see the event (`trigger_value`), since the store doesn't know the data folder.
- **Hiding a session from the lists never hides it from attention.** The Agents list (the sidebar, the phone drawer
  and `/sessions`, all `AgentsSection` in web `workspaces/Sidebar.tsx`) leaves out every session a workflow step
  started (`startedByStep`, web `sessions/status.ts`), tab or not. Bring here and the palette read
  `useWorkspaces.allInfos`, which leaves out workflow step sessions for a ticket and ended ones with no ticket unless
  a tab holds them (`hiddenStepSession`), so a step's tab can still be found without old steps piling up. The favicon
  and title read `everyInfo`, every session, so a hidden step that needs the user still shows.
- **A project's polled session and run lists stay small, yet hold every tab's item.** The web asks for
  `?ended=RECENT_ENDED` (web `sessions/queries.ts`); the store (`list_recent_sessions`, `list_recent_runs`) always
  adds every running session or active run and every one a workspace tab holds, so a tab never loses its item and
  nothing that can need the user drops out. Only ended ones without a tab are capped (workflow steps leave many).
- **A session in the background is never news.** A running agent that is idle with background tasks (`Session.background`,
  the store's `background_tasks` column, kept in step with the conversation's) shows Background (web
  `common/AgentState.tsx` `inBackground`), and its unseen flag is held back (`sessionUnseen`, read by `sessionInfo`),
  so no Done, unseen dot or favicon. Clearing the last task while the agent is idle sets `attention_at`
  (`set_session_background`), so it turns Done (unseen) then, even if the user looked meanwhile.

## Workflows

- **Everything runs at least once, maybe more.** Events are deduplicated by their key, runs by their run key. A new
  node kind with side effects either records its progress or sets `rerun_safe = false` (pdt-api
  `workflow/catalog.rs`), so a step cut short by a crash isn't repeated blindly.
- **A dry run changes nothing.** The engine calls the same `NodeKind::run` with `StepCx::dry_run` set; a kind with
  side effects checks it before doing them and returns `Outcome::would(port, what)`, and the dry run fills in the
  fields later steps read from the kind's output schema.
- **A node kind's config changes incompatibly only with a version bump and a migration.** Stored documents keep each
  node's `version`; the engine and validation bring an older config up to date as they read it (`NodeKind::migrate`
  in pdt-api `workflow/catalog.rs`; e.g. `github.nightly` 1 → 2 in `workflow/nodes/release.rs`), so a user's
  workflows run unchanged after an update. Never rewrite their documents for it.
- **Never hold `engine_lock` across a slow node body.** Action and agent bodies run outside it; a flow, gate, wait or
  condition kind that does slow work (git, like `attempt.has_changes`) opts out with `NodeKind::in_engine_lock`. A
  result is written only if the step is still `running` with the same attempt.
- **Never hold a global lock across git or the network.** Ticket Start fetches the base before it takes its
  project's start lock (`AppState::ticket_start_locks`), which covers only the checks, the worktree and the rows.
- **Secrets are filled in only when a node runs**, only from the node's own config, never from text that arrived as
  data. Stored step inputs, outputs and errors are redacted. Errors from outbound HTTP go through `send_error`, which
  drops the URL (it can carry a secret). An integration calls its service's JSON API through pdt-api
  `sdk::http::call_json_with` (a timeout, an answer cap, its auth, the service's error message, 404 as none), not its
  own send-and-check.
- **Every HTTP client names itself with `brand::user_agent`** (the build's version) and connects within pdt-core
  `outbound::CONNECT_TIMEOUT`; downloads end within `outbound::DOWNLOAD_TIMEOUT`. Discord's bot calls keep the
  User-Agent Discord requires.
- **Outside text never reaches a shell as code.** A `command` step's `script` refuses `{{…}}` references (only a
  template's `params` may stay); they go through `env`. `check` refuses them on save, and `NodeKind::check_at_run`
  (pdt-api `workflow/catalog.rs`, called by the engine before it resolves a step's input) stops a workflow saved before
  the rule. A rule that guards against injection belongs in `check_at_run` too, since stored documents aren't
  revalidated.
- **Event payloads are an API.** Users' workflows read them as `{{trigger.x}}`; renaming a field breaks them silently.
- **A workflow never reacts to its own runs by default.** The events in pdt-core `events::OWN_RUN_EVENTS` (`run.*`,
  `inbox.*`, `session.needs_you`, `session.turn_ended`, `session.exited`, `agent.stalled`, `agent.loop_limit`,
  `agent.timed_out`) carry `workflowId`: the workflow whose run
  caused them (the run, its agent step's session, its Inbox item). `EventTrigger::accepts` skips them for that
  workflow unless the trigger sets `includeOwn`. A new event a run can cause about itself joins the list and carries
  `workflowId`. Runs started by `run.*` events count a `chain`, and one `MAX_RUN_CHAIN` deep logs no `run.*` event,
  so two workflows reporting each other's runs stop.
- **Notifications never wait on a run.** The Notification rules' levels and quiet hours are read from its document as
  each notification is made (`notifications::rules`); only escalation and chat posts run as workflow steps. With the
  workflow off or every workflow off, every kind still notifies at its default level.
- **A stuck agent step is never left without anyone told.** A step handed to "When an agent is stuck" has no Inbox
  item until the workflow acts; `stuck::sweep` falls back to PDT's own handling when it is off, its run ends without
  acting, or no run starts in time. Code that ends or moves a step on takes its escalation (`take_escalation`).
- **Only Install updates installs by itself.** The update loop checks and logs `update.available`; it never installs.
  An automatic install goes through `host.install_update` with `automatic`, which keeps to the host's policy and never
  retries a version pdt-host skipped. With the workflow off, nothing installs without the user.
- **Only Clean up storage cleans by itself, and only a pause pauses pickup.** The storage loop measures and logs
  `disk.low` / `disk.recovered`; it never cleans or pauses. Auto-start checks `pickup_pause::paused`, never the disk
  level, so with Clean up storage off a full disk only warns. A pause stays over a restart until a `pickup.resume` or
  the user resumes it.
- **`_pdt.*` state keys are PDT's own.**
- **Every manual start checks the Run now trigger's declared inputs.** Run now, `POST /api/workflows/{id}/run`, the
  PM's `workflow_run`, a `workflow.call` and a manual dry run all go through `ManualConfig::input` (pdt-api
  `workflow/nodes/trigger.rs`, the rules in pdt-core `workflow/inputs.rs`), which fills defaults, drops hidden fields
  and refuses anything undeclared. The web form (`web/src/workflows/runInputs.ts`) mirrors the rules for display
  only; the server's check is the one that counts. Never start a manual run around it.
- **A failed write is not a step's failure.** A write the database can't make (a full disk, an I/O error, a lock held
  too long) is a `StoreError::Write`, whose message starts with `WRITE_FAILED` ("PDT can't save changes"); text that
  carries it (`pdt_store::is_write_failure`) makes `fail_step` wait and retry the step without using up one of its
  retries, an event whose run could not start stay unprocessed, and a step's result that could not be written wait in
  `UnsavedResults` (the body is not run again). Pass the store error to a node's error through `workflow::err`
  (whose text keeps `WRITE_FAILED`, and says when the disk is full), never replace it with your own words. A route
  answers it with 503 `write_failed`, saying `WRITE_FAILED` and whether the disk is full, not SQLite's text
  (`ApiError::write_failed`: from a `StoreError::Write`, a full disk or read-only volume in `ApiError::io`, or
  `ApiError::from_text` given an engine error's text; `ApiError::internal` doesn't look at the text), and the router's `note_write_failures`
  records it, so the Inbox says so whoever wrote; a `BEGIN` that can't start (`Store::write_tx`) is recorded too.
- **An event is marked processed only after it was matched against everything.** `match_events` returns before
  marking any event when it can't read the new events, the enabled workflows (`enabled_with_docs` gives `Err`) or the
  steps waiting for events: an empty list from a failed read would drop the events for good. Never
  `unwrap_or_default()` those reads. A workflow whose document is corrupt is left out with a warning instead, so one
  bad row doesn't hold up every event.
- **A failed read is never "not there".** A database, keychain or file read that fails is logged and answered as a
  failure (`ApiError::io_404` gives 404 only for a missing file; pdt-mcp answers 503 `read_failed`, never the 401
  Bearer challenge that sends a client to sign in; a document provider's `Failed`), and nothing is dropped or resolved
  for good on it: the outbox keeps a message whose step it can't read for the next pass. Never `.ok()?` such a read.
- **Conversation events are never skipped.** `conversations/live.rs` `append` applies an event to the runtime state as
  it comes (a message the agent took leaves `pending`, a turn's leftover streamed text goes), then stores it through
  the conversation's `unstored` queue: one that can't be written waits there with every event after it, is retried
  with a backoff and stored in the order received, and only a stored event goes to viewers. What reads the log (a
  turn's end, the PM's hooks) runs once the event is stored. A Claude conversation's log is checked for gaps against
  Claude's transcript the first time it loads in a pdtd (`repair_gaps`, by item id; only messages, tool calls and
  reasoning, since turn markers of turns the agent starts on its own have other ids live).
- **An agent's output is read with backpressure.** An adapter sends into an `EventSink` (pdt-agents
  `adapter/sink.rs`) without waiting, and its reader awaits `sink.ready()` before the next frame, so a conversation
  whose store lags pauses reading the agent's pipe (Codex: the whole app-server connection). Read lines with
  `adapter/lines.rs` `CappedLines` (an overlong line is skipped), never `lines()`. The `unstored` queue gives up its
  oldest events past a count and a byte bound; PTY input waiting for a terminal that doesn't read it is bounded too.
- **A step's setup error is never retried.** An error that waiting won't fix (no project, a bad repository) is made
  with `setup_error` (pdt-api `workflow/catalog.rs`; `repo_for_step` in `nodes/release.rs`), and `fail_step` skips the
  node's retries for it. A kind that works on the run's project says so in `NodeKind::needs_project`, so a manual start
  of a workflow without a project is refused and Run now asks for one (`WorkflowDetail::needs_project`).
- **Built-in workflows, templates and personas follow newer versions only while the user hasn't edited them.** Repo
  workflows (`.pdt/workflows/`) are read-only through the API; imported and instantiated workflows start disabled.
- **Enabling a workflow is consent to the version it has then.** A changed repo file of an enabled workflow is a
  proposal (pdt-api `workflow/repo.rs`), never a saved version, so a merge can't swap in new commands that run on the
  next trigger; only a disabled one takes the file's version directly.

## Secrets and GitHub

- **Agents and workflows use secrets by name; values never go in the chat.** Values never go in the database, a
  response, a log or a tool result; `pdt_core::secret::Redactor` replaces them (and their hex, base64 and URL-encoded
  forms) wherever the store, pdt-mcp, conversations and the logs write. It is a safety net, not a wall: a command run
  with a secret can print it some other way, so the docs promise "never pasted into the chat", not "never seen".
- **Every keychain value PDT reads or writes reaches the redactor.** `AppState::secrets` is wrapped in
  `secrets::Redacting` (pdtd's `main.rs`, `AppState::new`); a value kept outside the keychain (Google's access tokens,
  MCP sign-ins' refreshed tokens) is given to the redactor where it is made.
- **Redact before cutting.** Output cut to a size is redacted first and then loses any end that could start a value
  (`Redactor::string_cut`), so a cut never leaves part of one.
- **A bug report bound for GitHub goes through `bug_report::redact` with the user's own names.** pdt-api
  `bug_reports::redactions` gathers the token, the owner's login, the home folder, the machine's names and each
  project's name, key, folder, origin `owner/repo` and worktree branches and folders (not the release repository's
  project); the log tail and the update error always get it, and an agent's `report_bug` fields do on the GitHub
  route. A new place that logs a user's name for something is covered only if it is one of these.
- **A Codex terminal's secrets stay out of its shell commands**: each name `codex_tui_env` passes a secret in is in
  the `shell_environment_policy.exclude` it gets (pdt-agents `mcp.rs`), after the user's own patterns.
- **The secret tools are no way round the agent's sandbox.** `run_with_secrets` and `write_secret_file` run in pdtd, so
  they ask (unless the session asks before nothing), must name a secret, and stay in the session's folder, out of
  `.git` and off symlinks (pdt-api `vault.rs` `session_path`). Don't mark them non-asking or let the PM act for them.
- **Tokens are never returned by the API or logged**, and follow a redirect only to the same host.
- **An MCP server's sign-in addresses are https (or loopback http) before PDT uses them**: its authorization
  server and every endpoint in that server's metadata pass `pdt_core::mcp_servers::endpoint_ok`, and the metadata's
  `issuer` matches (`AuthServer::check`, in pdt-api `mcp_oauth.rs` `discover`), because the browser opens the sign-in
  address in PDT's own page and PDT sends the token address a client secret. The UI also opens only http(s) addresses.
- **A dev instance never reads stable's keychain entries**: keys outside the stable data folder are scoped by the data
  folder (pdt-api `secrets.rs`).
- **On Linux the secret backend sticks to its data folder** (keychain or file); it never switches.
- **Agents never attach secrets.** A file an agent attaches by path (`files` on `add_ticket_comment`,
  `create_ticket`, `edit_ticket`) is resolved and must be inside the session's folder, the temporary folder or the
  data folder (`ticket_attachment::outside_roots`), and passes `ticket_attachment::path_refusal`: no dotfiles,
  no key or credential folders, nothing in the data folder but conversation and ticket attachments. Both compare
  without case on macOS and Windows. A session that triages outside input attaches nothing (`ticket_attachments::read_files`). Attachments are
  never downloaded from outside issues or PRs, and a text attachment on an external ticket reaches agents fenced
  (`get_ticket`).
- **The Files view never leaves its checkout or touches `.git`.** Paths are cleaned (`files::clean_path`: no `..`,
  no `.git` however spelt, `.git.` and `GIT~1` included), checked, and then read and written only through a handle
  on the checkout (`files::checkout_dir`, cap-std), never by plain path, so a folder swapped for a symlink after
  the check still can't lead out. The review diff reads the same way.
- **Nothing runs `gh` or `glab`.** Forges go through their crates over HTTP (`pdt_forge::transport`); agents use
  pdt-mcp's forge tools.

## Requests

- **Anything running as the user on this machine can act as the user in PDT**, by choice: agents, terminals, runs and
  any other local program. The UI token and `from_ui` checks only guard against accidents (above); they are not a
  boundary. The boundaries are the Tailscale owner check for remote requests, the Host and Origin checks against web
  pages (DNS rebinding, CSRF), and fencing of outside input sent to agents ("Talking to agents"). Don't build on the UI
  token as if it kept a local process out, and don't weaken the real boundaries.
- **The Host guard and owner-only checks cover every route** (pdt-api `host_guard.rs`, `identity_guard.rs`): only
  loopback hosts and the allowed hosts; remote requests carry the owner's Tailscale identity. Hooks
  (`/api/hooks/*`) authenticate with their own secret instead. Don't add a route outside the guards.
- **A browser can't change anything from another page.** The host guard refuses an `Origin` that isn't an allowed
  host, and a change (not GET, HEAD or OPTIONS) whose `Sec-Fetch-Site` is there but isn't `same-origin` or `none`. A
  route that reads its body as raw bytes and parses JSON from it reads it through `host_guard::optional_json`, which
  wants `application/json` for a non-empty body (as axum's `Json` does), so a form or `text/plain` post, which needs
  no preflight, can't carry it.
- **Nothing is served on the tailnet without the owner check.** `tailscale serve` maps only pdtd's own port and
  web runs' owner-checking proxies (pdt-api `remote_proxy.rs`); never map a port straight to a run or another process.
- **Every response carries the security headers** (pdt-api `security_headers.rs`, the router's outer layers): no
  framing (`X-Frame-Options: DENY`, `frame-ancestors 'none'`), `nosniff`, no referrer, and for the web UI a CSP with
  script and frames only from PDT itself and the exact origins the enabled plugins name (`Integration::web_sources`:
  Google's picker script and its Docs frame), connections only to PDT. Keep the UI free of inline scripts and `eval`;
  a new kind of resource it loads (a worker) needs the policy widened there, and a plugin's page script or frame goes
  in its `web_sources`, never a scheme or wildcard. A response with its own CSP keeps it.
- **An error's message never carries another program's raw text** (an API error's `message`, a step's error, an
  Inbox item): the web shows it as it is, in screenshots and bug reports. A database, git, spawn, keychain or file
  error goes through its `From` into `ApiError` (or `ApiError::io(doing, e)`), which logs the full text and says what
  failed: git's failing line and a keychain's first line by `error::short_text` (absolute paths cut to their last
  part), a database error `DB_UNREADABLE`, the rest `internal`. Workflow nodes use `workflow::err`, which does the
  same (`workflow_errors_go_through_err` checks). Text that is the user's own input (a path they typed, a workflow's
  bad input) and a forge's or agent's answer meant for them stay as they are.
- **A run's link is set under `settings_lock`, together with registering its driver** (`run_manager::relaunch`,
  `runs::create`). Applying remote access holds the same lock while it relinks and restarts the running runs, so a
  run started meanwhile never keeps the other setting's link.

## Code and data

- **Transactions go through sqlx's `Transaction`** (`Store::write_tx`, `pool.begin`), never a hand-written `BEGIN`.
  The pool's `after_release` closes any connection still in a transaction (pdt-store `health.rs`): after a full disk
  SQLite rolls a transaction back itself, sqlx's own ROLLBACK then fails, and the connection would refuse every later
  `BEGIN`.
- **Processes start off the runtime's threads.** Creating a process (and scanning the search path for it) can take a
  second or more on a loaded Windows host, and tokio's `Command::spawn` does it inline in the poll. Async code starts
  every process with pdt-agents `launch::spawn_child` (agents, workflow command and conflict steps, local CI,
  `run_with_secrets`, sign-in), never `Command::spawn` itself; a library that spawns its own child (rmcp's
  `TokioChildProcess` for stdio MCP servers) is spawned inside `spawn_blocking` and handed over. Search-path lookups
  go through `launch::find_launch` or `spawn_blocking`, and so do `PtyManager::spawn`/`remove`/`kill`
  (`AppState::kill_pty`/`remove_pty`). Killing a tree from async code or a `Drop` uses `proc::kill_group` (Windows'
  `taskkill` starts on a thread of its own). The same goes for folders that grow with use: Claude's transcript is
  looked up in its working folder's project first (`launch::claude_transcript`), and only scanned for on a miss, on
  the blocking pool. `PtyManager` holds its map lock only to look
  up, reserve an id or insert, never while a PTY opens, a process starts or one is killed, so attaching, writing and
  resizing other terminals never wait on them. The id reserved while a process starts is released on every way out
  (a `Reservation` drop guard; a panic while starting becomes a spawn error), or that session could not start again
  until pdtd restarts.
- **Never edit an applied migration** (sqlx checksums them); add a new one named by its UTC creation time. Data
  backfills that run when the store opens go in `Store::open`.
- **A changed default in `HostSettings` changes it for every install that never stored the field** (no stored row, or
  one saved by an older version): a stored row reads over the profile's defaults (pdt-store `get_host_settings`).
  When existing installs must keep the old value, a migration stores it for them (tickets' permission mode:
  `*_ticket_permissions_keep_bypass.sql`, which counts an install with a project or stored settings as existing).
- **`.pdt/project.toml` must stay readable by the stable release.** Unknown top-level sections are skipped, but unknown
  keys inside `[[run]]`, `[[test]]` and `[[area]]` are refused, so a new section waits until stable supports it.
- **Names live only in `crates/pdt-core/src/brand.rs`** (plus the Tauri configs, the Windows installer hooks and the
  release scripts): product name, slug, daemon, ports, release repository, keychain service, also in UI text.
- **PDT never special-cases its own repository or development process; PDT's own automation lives in the
  maintainer's workflows.** Outside tests, only updates, bug reports and the release repository setting name the
  release repository (`brand::RELEASE_REPO`; a test in pdt-core `brand.rs` lists where). Nothing is installed or
  offered by what a project's repository is. The built-ins that once automated PDT's development are kept on the
  hosts that had them as their own workflows (pdt-api `workflow/builtins/mod.rs`, `RETIRED`); never ship such a built-in
  again. Cut a release is a template (filled in per project), not a built-in.
- **IronCalc is one version on both sides**: pdt-sheet's `ironcalc` (pinned with `=`) and web's
  `@ironcalc/workbook` exchange IronCalc's bytes and diffs, so they are updated together (pdt-sheet's
  `the_web_ui_runs_the_same_ironcalc` checks).
- **IronCalc never sees input past pdt-sheet's limits** (`limits.rs`): its formula parser and evaluator recurse
  without a depth limit (an overflow aborts pdtd), and its importer trusts the package. Every new way a formula or a
  file reaches IronCalc goes through `check_formula` or `check_xlsx` first, and pdtd's runtime threads keep a stack
  (pdtd `main.rs`, `STACK`) that holds the deepest formula the limits allow. A view's diffs aren't checked (their types
  are IronCalc's own); they rely on the stack.
- **Guides served by pdtd link to each other by API path**, never by repository path (a test checks).
- **The web doesn't test for an integration's id or name**: what a plugin adds to the UI comes from what it declares
  (its connection kinds, project settings schema, label, account sign-in, a document provider's picker, a forge's
  name for a change request), so turning it off removes it. Add a declaration rather than a check on `kind === '…'`;
  `git grep -i -E 'google|gitlab' web/src` finds only generated types, fixtures and tests (and Google's ACP agent, which is an agent, not a plugin).
- **New tools and node kinds change the integrations snapshot** (`crates/pdtd/src/integrations/`): update it on
  purpose.
- **The UI re-renders every 2 s**: effects must be idempotent, and never focus from an inline ref callback.
- **The shell re-renders only when a poll brings something new** (web `shell/AppShell.tsx`): the `useWorkspaces` and
  `useBacklog` results are memoized, the views it passes them to (Sidebar, WorkspaceBar, SessionView, RunView,
  BottomPanel) are `memo`, and its handlers are `useCallback`. Keep new props to those views stable, and read a fast-
  changing query in the shell through a `select` that keeps only what it shows (`usePmSummary`).
- **What a view shows when it is left comes back** (web `common/viewState.ts`): keep a view's choices (a file, a
  toggle, a scroll, a draft) with `useViewState` / `useRememberedScroll`, not `useState`, under a name that includes
  the ids of what it shows. The shell scopes them by workspace tab (or by view); a closed tab's are dropped.
- **A failed mutation is reported once.** The app's QueryClient (web `common/queryClient.ts`) toasts a mutation's error
  unless it has its own `onError` or `meta: { errorShown: true }`. Give a `useMutation` that renders its error inline
  (or awaits `mutateAsync` in a save status) that meta, and a hook that returns its mutation's error the same, so the
  error doesn't show twice; leave the rest to the fallback toast.
