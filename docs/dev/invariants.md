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
  **The one exception is a watch** (pdt-api `watches.rs`): a PM conversation that set one with its `watch` tool is
  told about its target without asking anyone, since it only tells the conversation that asked. Its messages still
  go through the outbox, and a closed conversation's go to the user's Inbox instead.
- **Conversations are never auto-resumed**, and never as terminals. After a pdtd restart their running turn ends as
  interrupted; the UI offers Continue. Only interrupted terminal sessions are resumed by themselves. Background
  tasks a restart stopped (kept per session, `sessions.background_tasks`, moved to `stopped_background` by
  `recover_conversations`) are told to the agent only along with the next message the user or a workflow sends
  (pdt-api `conversations.rs` `tell_stopped_background`); a workflow step waiting on them gets that message at once,
  as an automatic nudge (`workflow/agent.rs` `restarted_while_waiting`). Terminals don't report background tasks, so
  their agents aren't told.
- **Text from outside is fenced.** Issue and PR text (titles too), webhook payloads and other outside fields
  (`EventType.outside`) are fenced when they reach an agent (pdt-core `fence.rs`, pdt-api `untrusted.rs`), and sessions
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
  project policy answers it) and always opens a `confirm` item with `body.rows`; only that item's Allow, which needs the
  UI token, runs it with `cx.confirmed`, narrowed to the rows the user kept. It refuses non-PM sessions even when a
  persona lists it. Each row goes through `ticket_proposal::decide` as `By::user_via(<the PM>)`: the user's events,
  with the PM as `proposedBy`.
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
- **A session in the background is never news.** A running agent that is idle with background tasks (`Session.background`,
  the store's `background_tasks` column, kept in step with the conversation's) shows Background (web
  `common/AgentState.tsx` `inBackground`), and its unseen flag is held back (`sessionUnseen`, read by `sessionInfo`),
  so no Done, unseen dot or favicon. Clearing the last task while the agent is idle sets `attention_at`
  (`set_session_background`), so it turns Done (unseen) then, even if the user looked meanwhile.

## Workflows

- **Everything runs at least once, maybe more.** Events are deduplicated by their key, runs by their run key. A new
  node kind with side effects either records its progress or sets `rerun_safe = false` (pdt-api
  `workflow/catalog.rs`), so a step cut short by a crash isn't repeated blindly.
- **A node kind's config changes incompatibly only with a version bump and a migration.** Stored documents keep each
  node's `version`; the engine and validation bring an older config up to date as they read it (`NodeKind::migrate`
  in pdt-api `workflow/catalog.rs`; e.g. `github.nightly` 1 → 2 in `workflow/nodes/release.rs`), so a user's
  workflows run unchanged after an update. Never rewrite their documents for it.
- **Never hold `engine_lock` across a slow node body.** Node bodies run outside it; a result is written only if the
  step is still `running` with the same attempt.
- **Secrets are filled in only when a node runs**, only from the node's own config, never from text that arrived as
  data. Stored step inputs, outputs and errors are redacted. Errors from outbound HTTP go through `send_error`, which
  drops the URL (it can carry a secret).
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
  `UnsavedResults` (the body is not run again). Keep the store error's text in a node's error (`e.to_string()`, or
  in a `format!`), never replace it with your own words. A route answers it with 503 `write_failed` and its message
  (`ApiError::write_failed`, also from `ApiError::internal` given its text), and the router's `note_write_failures`
  records it, so the Inbox says so whoever wrote; a `BEGIN` that can't start (`Store::write_tx`) is recorded too.
- **An event is marked processed only after it was matched against everything.** `match_events` returns before
  marking any event when it can't read the new events, the enabled workflows (`enabled_with_docs` gives `Err`) or the
  steps waiting for events: an empty list from a failed read would drop the events for good. Never
  `unwrap_or_default()` those reads. A workflow whose document is corrupt is left out with a warning instead, so one
  bad row doesn't hold up every event.
- **Conversation events are never skipped.** `conversations.rs` `append` applies an event to the runtime state as
  it comes (a message the agent took leaves `pending`, a turn's leftover streamed text goes), then stores it through
  the conversation's `unstored` queue: one that can't be written waits there with every event after it, is retried
  with a backoff and stored in the order received, and only a stored event goes to viewers. What reads the log (a
  turn's end, the PM's hooks) runs once the event is stored. A Claude conversation's log is checked for gaps against
  Claude's transcript the first time it loads in a pdtd (`repair_gaps`, by item id; only messages, tool calls and
  reasoning, since turn markers of turns the agent starts on its own have other ids live).
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

- **Agents and workflows use secrets but never see them.** Values never go in the database, a response, a log or a
  tool result; `pdt_core::secret::Redactor` replaces them wherever the store, pdt-mcp, conversations and the logs
  write.
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
  `create_ticket`, `edit_ticket`) is resolved and checked by `pdt_core::ticket_attachment::path_refusal`: no dotfiles,
  no key or credential folders, nothing in the data folder but conversation and ticket attachments. A session that triages outside input attaches nothing (`ticket_attachments::read_files`). Attachments are
  never downloaded from outside issues or PRs, and a text attachment on an external ticket reaches agents fenced
  (`get_ticket`).
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
- **Every response carries the security headers** (pdt-api `security_headers.rs`, the router's outer layers): no
  framing (`X-Frame-Options: DENY`, `frame-ancestors 'none'`), `nosniff`, no referrer, and for the web UI a CSP with
  script only from PDT's own bundle. Keep the UI free of inline scripts and `eval`; a new kind of resource it loads
  (a worker, another frame source) needs the policy widened there. A response with its own CSP keeps it.
- **A run's link is set under `settings_lock`, together with registering its driver** (`run_manager::relaunch`,
  `runs::create`). Applying remote access holds the same lock while it relinks and restarts the running runs, so a
  run started meanwhile never keeps the other setting's link.

## Code and data

- **Transactions go through sqlx's `Transaction`** (`Store::write_tx`, `pool.begin`), never a hand-written `BEGIN`.
  The pool's `after_release` closes any connection still in a transaction (pdt-store `health.rs`): after a full disk
  SQLite rolls a transaction back itself, sqlx's own ROLLBACK then fails, and the connection would refuse every later
  `BEGIN`.
- **Never edit an applied migration** (sqlx checksums them); add a new one named by its UTC creation time. Data
  backfills that run when the store opens go in `Store::open`.
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
- **Guides served by pdtd link to each other by API path**, never by repository path (a test checks).
- **The web doesn't test for an integration's id or name**: what a plugin adds to the UI comes from what it declares
  (its connection kinds, project settings schema, label, account sign-in, a document provider's picker, a forge's
  name for a change request), so turning it off removes it. Add a declaration rather than a check on `kind === '…'`;
  `git grep -i -E 'google|gitlab' web/src` finds only generated types, fixtures and tests (and Google's ACP agent, which is an agent, not a plugin).
- **New tools and node kinds change the integrations snapshot** (`crates/pdtd/src/integrations/`): update it on
  purpose.
- **The UI re-renders every 2 s**: effects must be idempotent, and never focus from an inline ref callback.
- **What a view shows when it is left comes back** (web `common/viewState.ts`): keep a view's choices (a file, a
  toggle, a scroll, a draft) with `useViewState` / `useRememberedScroll`, not `useState`, under a name that includes
  the ids of what it shows. The shell scopes them by workspace tab (or by view); a closed tab's are dropped.
- **A failed mutation is reported once.** The app's QueryClient (web `common/queryClient.ts`) toasts a mutation's error
  unless it has its own `onError` or `meta: { errorShown: true }`. Give a `useMutation` that renders its error inline
  (or awaits `mutateAsync` in a save status) that meta, and a hook that returns its mutation's error the same, so the
  error doesn't show twice; leave the rest to the fallback toast.
