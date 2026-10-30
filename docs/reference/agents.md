# Agent sessions: terminals, instructions, activity, restart, Devin, ACP agents

How PDT launches and resumes Claude, Codex and Devin sessions and the terminals they run in. Conversations (the
structured view) are in [conversations.md](conversations.md). Code: crates/pdt-agents (PTYs, `adapter/`,
`hooks.rs`, `instructions.rs`, `detached.rs`), crates/pdt-api `sessions.rs`, `pty_ws.rs`, `activity.rs`.

## Instructions and guides (PDT-46)

- pdtd serves its guides itself, built in with `include_str!` (pdt-api `guides.rs`): `GET /api/guides` lists them,
  `GET /api/guides/{name}` returns Markdown (`agents` = docs/agent-guide.md, `run-config` = docs/run-config.md,
  `set-up-project` / `set-up-workflow` = docs/prompts/*.md below their `---` line). Guides link to each other by API
  path, never by repo path (a test checks).
- Every agent session (Claude, Codex or Devin; terminal or conversation) gets PDT's instructions: a short context
  block (API address, session, project, checkout, guides, worktrees and runs through the API; pdt-core
  `agent_context`) plus the project's **agent instructions** (`project_settings.agent_instructions`,
  `agentInstructions` in the settings API; kept when a PUT leaves it out, max 16 KB), written to
  `<data dir>/sessions/<id>/instructions.md`.
- A session on an active attempt, or a workflow step's session about one (a new Reviewer or QA session has no attempt
  of its own), outside Personal projects, is told how to read CI by the project's forge (PDT-176, `Forge::agent_notes`;
  none without a forge). GitHub's (PDT-177): PDT's forge tools, `ci_runs` and `ci_failed_logs` (or the attempt's
  `pr.ci`), `change_request_view` and `issue_view`, instead of the GitHub CLI; never the check-runs API, which
  fine-grained tokens can't read (PDT-165). The forge also names how to answer a PR comment
  (`change_request_comment`, `replyTo` for a review comment) in "Messages from PDT". Agents still `git push`
  themselves. The local forge's (PDT-178): commit and don't push; PDT runs the `[ci]` command on each commit,
  merges once it passes, and serves each run's log; with `[push] remote` (PDT-245) it also says PDT pushes the base
  to that remote, so the agent pushes neither.
- The forge tools (pdt-api `integrations/forge.rs`, actions with only a tool face, in the default tool set): they use
  the session's project forge and branch (its attempt's, a step's attempt's, else its worktree's).
  `change_request_view` (number or branch; state, mergeability, comments and reviews), `change_request_comment`
  (`replyTo`: a review comment's id), `ci_runs` (every CI provider's runs of the project, PDT-242, each naming its
  `provider`, with failed jobs; `provider` for one), `ci_failed_logs` (a run's failed jobs with the last 200 lines of
  each log; `provider` and `runId`, a text id or an Actions number; the newest failed run of any provider when no
  `runId`), `issue_view`, `issue_comment`. The PM has
  `issue_view`. `issue_create` (PDT-221, pdt-api `integrations/github.rs`; also in the default set and the PM's) files
  an issue, from a ticket that then links it, and asks before each call ([github.md](github.md) "Filing issues"). PDT-185, for the session's own ticket attempt or worktree branch only (not a step's
  attempt, so a Reviewer or QA session opens and merges nothing): `change_request_open` (`title`, `body`, `base`:
  the attempt's base when left out, `draft`; `Forge::open_change_request`; a branch with an open one gets it back,
  `existing: true`; the attempt's PR is read at once so PDT tracks it) and `change_request_merge` (the attempt's open
  PR set to merge again: only when it was set to merge, `merge_requested` as PR notices reads it, else refused; already
  on auto-merge or queued → `already`; otherwise the automated `attempt.merge` path, policy and CI gate included; a
  refusal for CI not passed yet keeps it set to merge and answers `waitingForCi`; any other refusal is the tool's
  error).
- A session on an active attempt (Start's session) also gets "This session's ticket" (key, API path, branch;
  follow-up work in its own ticket; Done, the worktree and the branch are PDT's or the user's) and, when enabled
  workflows message the attempt's agent, "Messages from PDT" (PDT-83): which events reach it as `[PDT]` messages, how
  to act on each kind, and, for any `ci.*` / `pr.*` / `attempt.merged` one, that PDT watches the pull request so CI
  isn't watched in a shell. The events come from pdt-core `workflow::news::agent_news`: enabled project and
  host-wide workflows whose trigger (and its `projects`) leads by edges, or a `workflow.call`, to a `session.send`
  with `attempt` (trigger conditions aren't read). The text is fixed when the session starts or restarts; a chat that
  Start takes over keeps what it started with.
  - Claude gets `--append-system-prompt-file <that file>`.
  - Codex terminals get `-c developer_instructions=<TOML string>` (through a Windows `.cmd` shim, a pointer to the
    file), Codex conversations `developerInstructions` on `thread/start`/`thread/resume`; the user's own
    `developer_instructions` from config.toml come first, since PDT's value replaces them (pdt-agents
    `instructions.rs`).
  - Devin gets them from a SessionStart hook (below).
- The project's Settings page has the instructions box and "Set up with an agent" / "Write instructions with an
  agent", which start a conversation in the main checkout with the matching prompt.

## pdt-mcp (Phase 7b)

PDT's MCP server (pdt-api `mcp/`; spec docs/superpowers/specs/2026-10-20-phase-7b-agent-steps-design.md §2):
`POST /api/mcp`, Streamable HTTP answered with JSON (`initialize`, `ping`, `tools/list`, `tools/call`; batches;
notifications get 202; `GET`/`DELETE` 405).

- **Identity.** `Authorization: Bearer <session id>.<HMAC-SHA256 of it>` under the host key `<data dir>/mcp.key` (32
  random bytes, owner-only, made on first use). Nothing is stored per session; a token works while its session exists,
  is an agent session and is not `finished` (401 otherwise). A tool call knows its session, and through it the
  project, ticket (the session's attempt's, else its agent step's) and agent step.
- **Tools** (`mcp/tools.rs`): `get_ticket`, `submit_outcome`, `ask_user`, `add_ticket_comment`, `add_review_comment`,
  `list_secrets`, `request_secret`, `run_with_secrets`, `write_secret_file` (the default set; the secret tools are in
  [secrets.md](secrets.md)), and `update_ticket` (Phase 7c: title and description of the session's own ticket, the store's
  `patch_ticket_by` as the agent; the built-in Refiner lists it), `catalog_search`, `catalog_get`, `workflow_get`,
  `workflow_validate`, `workflow_dry_run`, `workflow_save`, `workflow_export` (read-only: the export's text) and
  `workflow_import` (asking: imports an export, new workflows off; `check: true` only reports) for personas that list
  them, and the PM's tools ([The
  PM](#the-pm-pdt-81)). Inputs are the API's request types (schemars schemas) and each tool
  runs the route's own function (ticket `get_one`, the store's comment, review `add_comment`, workflow
  `create_workflow` with author `agent`, so a new workflow starts disabled, and `save` with `propose`, so a change is a
  proposal the user accepts or discards). `tools/list` gives only the session's set (its
  persona version's, else the default); read-only tools carry `readOnlyHint`. Errors the agent can act on are tool
  results with `isError`. Every result and error is redacted of known secrets' values (PDT-158).
- **Per launch, never the user's config** (pdt-agents `mcp.rs`): Claude (conversation and terminal) gets
  `--mcp-config <session dir>/mcp.json` (owner-only; it holds the token) and `--allowedTools mcp__pdt__…` for its set
  less the asking tools (`create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `stop_ticket`, `send_to_session`,
  `workflow_run`, `workflow_import`,
  `discord_change`, `discord_webhook`:
  each tool's `asks` flag, pdt-core `integration::ToolDecl`), which Claude
  asks about per call;
  Codex conversations a thread `config` `mcp_servers.pdt` with `http_headers` and `default_tools_approval_mode =
  "approve"`, Codex terminals the same through `-c` with the token in `PDT_MCP_TOKEN` (`bearer_token_env_var`); Devin
  conversations an ACP `mcpServers` entry, and the mapping allows `mcp__pdt__*` permission requests at once (Devin's
  extensions in pdt-core `conversation/devin.rs`). Devin terminals get no pdt-mcp (their MCP servers live in a file of Devin's own).
- **Instructions.** `agent_context` names the server and the session's tools; a persona's step session also gets
  "Your role: <persona>" ([workflows.md](workflows.md#agent-steps-phase-7b)); a Personal project's session is told it
  works in a folder with no branches.
- Tests: `fake-agent` calls a tool when a message's first line is `mcp: <tool> <json>` (`mcp-list` lists them),
  asking first, as Claude does, for a listed tool `--allowedTools` leaves out; crates/pdt-api/tests/agent_steps.rs and
  pm.rs serve the API on a local port for it.

## The PM (PDT-81)

A host-level project manager to talk to (spec docs/superpowers/specs/2026-10-21-pm-persona-design.md; pdt-api `pm.rs`,
pdt-core `pm.rs`, pdt-store `pm.rs`, web `pm/`).

- **Where it lives.** PM sessions are conversations in the **host project** (`kind: host`, key `_HOST`, folder
  `<data dir>/pm`, no git, no workspace), made on first use and never listed (`list_projects` leaves it out). Each
  runs in the **work folder** (PDT-100, pdt-core `pm::work_folder`): the deepest folder holding every project's
  folder (each checkout's parent) and Host settings' Projects folder (its default `~/work` only while there are no
  projects); the home folder when they share nothing below the root. They have no tab: the PM rail item (`Ctrl+] m`,
  the palette's Go to) shows them, and opening one from the Inbox (whose items name the conversation) goes there. The
  PM item (rail and phone drawer) carries a dot, "Project manager (new reply)", while any open conversation is unseen
  and not working and isn't the one on screen (PDT-145); the PM view marks the conversation it shows seen, as a
  session's tab does.
- **Conversations** (PDT-149). Several PM sessions can be open at once, one per topic, each with its own transcript,
  briefing and journal entry; notes, pinned notes and journal entries are host-wide, so all of them see the same.
  `GET /api/pm/sessions` lists them: each with `session`, `open` (not ended), `title`, `unseen`, `lastActivity` (its
  last event; for an ended one when it ended), rotation (`pm_sessions`: `none` | `journal` | `done`), reason (`context`
  | `age` | `user`) and journal; the open ones first, the most recently active first, then the ended ones, newest
  first. `GET /api/pm` → `{current, sessions}` is the same list with `current` the newest open conversation (for
  clients that know only one). `POST /api/pm/sessions {text?, title?}` starts one beside the open ones (it ends none)
  as the built-in **PM** persona's current version (its agent and model; permission mode `ask` unless the persona
  sets one). Its title is `title`, else its first message's first line (pdt-core `pm::conversation_title`, at most 60
  characters), else "New conversation" until the user's first message names it. `PATCH /api/pm/sessions/{id}
  {title}` renames one. `DELETE /api/pm/sessions/{id}` closes one: one that has said anything is asked for its
  journal through the outbox (rotation `journal`, reason `user`, the message reopens a parked conversation) and ends
  when that turn does; one that never did ends at once with no journal; one already ending is left
  as it is. An ended conversation is read-only. Idle PM conversations park as every conversation does and wake on a
  message; PDT's messages to them keep to the host's cap on agents running a turn.
- **Its agent's own tools** (PDT-100), with the agent's usual permission prompts: read and search checkouts and
  worktrees, git, web search and fetch, commands and installs, file edits, subagents. Claude and Devin keep theirs;
  Codex also gets `web_search = "live"` and network access in its workspace-write sandbox (thread config
  `sandbox_workspace_write.network_access`, `networkAccess` on each turn's `sandboxPolicy`; pdt-agents
  `mcp::CODEX_PM_CONFIG`).
- **PDT's tools.** pdt-mcp with the PM's set (every tool with the `pm` flag, from every integration): `get_overview` (projects with open tickets per
  stage, tickets in progress with branch/PR/CI/agent in words, the Inbox, running agent sessions), `search_tickets`,
  `get_ticket`, `get_session` (state, open requests, last messages),
  `list_workflow_runs`, `search_notes`, `save_note`, `delete_note`, `write_journal`, `list_todos` (the user's todo
  list with due dates, `dueOn`/`dueTime`, and repeat rules, `recurrence`; read-only: the PM never changes it), the catalog and workflow tools
  (`workflow_get` by name searches every project; `workflow_save` needs a project), and the asking tools
  `workflow_run` (`id` or `name`, `project`, `input`: runs an enabled workflow's Run now trigger with `projectId` added
  to the input; a disabled one is refused. The PM starts the built-in "Cut a release" with it, only when the user asks
  for a release, adding `channel: experimental` for an experimental one: [workflows.md](workflows.md) "Workflows",
  PDT-194, PDT-210),
  `create_ticket` (a draft by default; the timeline gets "Created by the PM.", or by the session's persona), `edit_ticket` (closing ends the
  attempt and moving back to Draft, Ready or To do stops its work, as the API; `parent` takes a key or id, `""`
  detaches; another persona given the tool edits its own ticket freely, but another ticket only by its exact key,
  never by id or number, and one in progress or closed keeps its title and description unless that session created
  it, its `create_ticket` note carrying `created` and its session id; the refusal points at `add_ticket_comment`,
  PDT-162), `stop_ticket` (`ticket`, `stage?`, `removeWorktree?`, `deleteBranch?`: `POST /api/tickets/{id}/stop`,
  [tickets.md](tickets.md) "Stopping a ticket's work"; the timeline gets "The PM stopped its work."), `link_tickets` / `unlink_tickets` (`ticket`, `kind`:
  `blocks`, `blockedBy` or `relates`, and `other` in the same project; the store's link rules, so self-links, blocking
  cycles and unknown tickets are errors; the `linked`/`unlinked` and `parted` events are the agent's) and
  `send_to_session` (through the outbox, "[PDT] From the project manager: …"), and `report_bug` (a bug in PDT itself,
  routed as Host settings' Bug reports say: [host.md](host.md) "Bug reports"; it always asks, `AskTarget::Always`, so
  Codex and Devin can't call it).
- **Discord** (PDT-98; crates/pdt-int-discord, only while the Discord integration is enabled: [workflows.md](workflows.md)
  "Integrations"). Through a `discord_bot` connection (`connection`:
  its name, or the only one), with Discord's REST API (no gateway): `discord_get` (the server, its categories,
  channels and webhooks, each webhook with the connection it is saved as), and the asking tools `discord_change`
  (`changes`, in order: `create` a channel or category, `rename`, `move` into a category or out, `archive`: move into
  an Archive category; nothing is deleted; the first failure stops the rest and the result says what was done) and
  `discord_webhook` (a webhook on `channel`, made first with `createChannel`, saved as a `discord_webhook` connection
  called `name` that `discord.post` uses; the URL goes to the keychain and never into the result; a taken name is
  refused before anything is made, and a webhook that can't be saved is removed). The approval card's title is one
  line per change (crates/pdt-int-discord `core::ask_title`, through the integration's `approval_title` hook).
- **Jira** (PDT-258; crates/pdt-int-jira `actions.rs`, only while the Jira integration is enabled). Through a `jira`
  connection (`connection`: its name, or the only one; Cloud or Server / Data Center), the same calls as the `jira.*`
  nodes ([workflows.md](workflows.md) "Jira actions"), each tool's schema its node's config with `connection`
  optional. Reading asks nothing: `jira_check` (who the connection signs in as, the deployment and version),
  `jira_search` (JQL a page at a time; `next` back as `page`), `jira_issue`, `jira_transitions` (each transition's
  screen fields: required, type, choices), `jira_boards`, `jira_sprints` (active sprints and backlog sections) and
  `jira_board` (each issue's `placement` by sprint and status). The asking tools wait for the user's yes unless the
  connection's `pmActs` lets the PM act: `jira_import` (a ticket from an issue, never twice), `jira_create`,
  `jira_edit`, `jira_comment`, `jira_transition` (with its screen's `fields` in the same request; a required one left
  out comes back as an error naming it and its choices, for the PM to ask the user), `jira_sprint` (into a sprint or
  section, or back to the backlog) and `jira_epic`. The approval card's title says what changes ("Jira: move OPS-1 to
  Resolved with Resolution=Fixed"; pdt-int-jira `actions::approval_title`). The token never reaches the agent.
- **Scryer** (PDT-147; crates/pdt-int-scryer `actions.rs`, `core.rs`, only while the Scryer integration is enabled). Through a `scryer` connection (`connection`: its
  name, or the only one), the same calls as the `scryer.*` nodes ([workflows.md](workflows.md)), each tool's schema
  its node's config with `connection` optional: `scryer_library` (the library a page at a time: facet, words in the
  title, monitored), `scryer_search` (metadata by title, each result's ids and whether the library has it),
  `scryer_queue` (what is downloading), and the asking tool `scryer_add` (a title by facet and ids, monitored by
  default; one already in the library is left as it is and the result says `alreadyInLibrary`). The approval card's
  title is pdt-core `scryer::ask_title` ("Scryer: add the movie Dune (tmdb 438631), monitored"), through the
  integration's `approval_title` hook. The API key never
  reaches the agent.
- **Scryer's whole API** (PDT-150; crates/pdt-int-scryer `graphql.rs`, `core.rs`), so the PM can find, read and
  change any setting Scryer's GraphQL schema has (indexers, download clients, seeding profiles, quality, general
  settings, jobs). Every document is checked against Scryer's schema before it is sent (apollo-compiler): the live
  one from introspection, kept per connection for 10 minutes, else the copy PDT bundles
  (`crates/pdt-int-scryer/src/schema.graphql`, asked for again after a minute). Subscriptions are not offered.
  - `scryer_schema` (read): `search` (words in operation and type names and descriptions, a name match first, 30 at
    most), `operation` (one query or mutation: its arguments, the input types and enums they take, its return type,
    and for a mutation whether it asks), `type` (one type's fields, input fields, values or members); with none, the
    API's size. Each answer says whether the schema is `live` or `bundled`.
  - `scryer_query` (read, no asking): a query document with `variables` (and `operationName` for one of several),
    validated, its variables checked against their types, and refused when it holds a mutation or subscription. The
    answer is `{data}`, credentials redacted; past 48 KB the biggest lists are halved until it fits and
    `truncated` lists each cut (`path`, `kept`, `of`) with a hint to select less or page.
  - `scryer_mutate` (asking): `mutation` (its name), `args` (its arguments by name) and `select` (the fields to read
    back; its scalar fields, and those of its object fields, when left out). PDT writes the document (each argument a
    variable of the schema's type), validates it and its variables, and returns `{mutation, result}`, credentials
    redacted. The approval card's title is pdt-core `scryer::mutate_ask_title` ("Scryer: createIndexerConfig
    name=NZBgeek, config=[2 items], isEnabled, providerType=newznab"), credentials redacted. The connection's
    `config.pmActs` answers it, except for a call that always asks (`scryer::mutate_always_asks`, `AskTarget::Always`):
    a destructive or high-impact mutation (`delete*`, `remove*`, `reset*`, `revoke*`, `uninstall*`, `unlink*`,
    `empty*`, `abandon*`, `clear*`, and a list in pdt-core `scryer.rs` such as `startApplicationUpgrade`, restoring a
    backup, users and permissions, plugins and scripts), or one that sends a `$secret`. Signing in, passkeys, two-factor,
    the user's own password and API keys, and first-run setup are never sent (`MutationRule::Denied`).
  - **Credentials.** A value is a credential when its field's name says so (`password`, `apiKey`, `token`, `secret`,
    `passkey`, `cookie`, `credential`, `privateKey`, `authorization`), or it is a value of a `{key, …}` pair whose
    `key` does (Scryer's provider settings, `{key: "apiKey", stringValue}`). A mutation takes one by reference,
    `{"$secret": "<connection>"}` anywhere in `args`: PDT puts that connection's secret there just before sending
    (any kind: an HTTP service connection suits an indexer's API key), so the agent never sees it; a reference to a
    connection without a secret fails before anything is sent, and queries take none. Written out literally, a
    credential is redacted (`[redacted]`) from the conversation's tool-call and request events (input and output)
    and their raw lines (the Scryer integration's `redact_call` hook, for every `scryer_*` tool; conversations.rs
    `integration_hooks`), so from the transcript and the card. What
    Scryer answers is redacted the same way, also any secret PDT sent that it echoes. The log line for a change names
    the connection and the mutation only.
- **Asking.** Claude asks before each asking tool (the approval card shows the arguments, also in the Inbox's
  Approvals). When the request's target project (`create_ticket`'s `project`, `edit_ticket`'s, `link_tickets`', `unlink_tickets`' and `stop_ticket`'s ticket,
  `send_to_session`'s session) has the policy **The PM acts without asking** (`project_settings.pm_acts`, `pmActs` in
  project settings), PDT answers yes itself (conversations.rs → `pm::request_opened`, the target from the registry's
  `ask_target`); for an integration's tools the setting is their connection's `config.pmActs` (a Discord server is in
  no project): the Discord bot's for the Discord tools, the Scryer connection's for `scryer_add` and `scryer_mutate`
  (`AskTarget::Connection` with the connection's kind; `AskTarget::Always`, for the calls that always ask, is never
  answered for the user). Codex and Devin can't ask per tool, so there an asking tool runs at once where the
  project (or the connection) lets the PM act; anywhere else, and for `AskTarget::Always` (`report_bug`,
  `workflow_import`), PDT asks in its Inbox instead (PDT-256, `mcp::tools::confirmable`): the call becomes a `confirm`
  item (group Approvals) with the tool, its arguments, the agent and the session, and the tool answers with an error
  telling the agent to end its turn. The user's **Allow** (from PDT's web client only: without the UI token it is
  `409`) runs the call as given, as the session's call with the user's yes (`Cx::confirmed`), and the agent gets a
  message with its result or error; **Deny** sends it a message saying no. A second click runs nothing.
- **Briefing.** Built by pdtd when the session's instructions are first written and kept in
  `<session dir>/briefing.md` (a restart sends the same text): projects with open tickets per stage and whether the PM
  may act, tickets in progress, the Inbox, events since the previous PM session started (counts and the latest
  lines), the last three journal entries, the other open PM conversations (title, last activity, and their last
  message's first line), and pinned notes (id, project, title). Sections are capped at 30 lines. The
  instructions are pdt-core `pm::instructions`: who the PM is, its work folder, its tools (the asking ones named; its
  agent's own), PDT's API and guides, the persona's instructions, an "Integrations" section with the prompt
  fragment of each enabled integration the session has a tool of (`Integration::pm_prompt`: Discord's and Scryer's),
  the briefing (each project with its folder). The PM persona's own text names no integration.
- **Notes** (`pm_notes`, FTS5 table `pm_notes_fts` kept by triggers): `kind` note | journal | user, host-wide or per
  project, pinned or not. `GET /api/pm/notes?q=&project=&kind=&limit=` (words are quoted prefixes, all required; a
  project gets its own and host-wide notes, pinned first), `POST` (`kind` note, the default, or user; never journal),
  `GET`, `PATCH /api/pm/notes/{id}` (`project: ""` makes it host-wide), `DELETE`.
- **The Notes view** (PDT-143, `web/src/notes`; rail and phone drawer after Tickets, Ctrl+] o, the palette's "Notes")
  holds both kinds the user sees: a list column with New note, a full-text search (`/` focuses it), a **Mine | PM's
  memory** switch (`/notes` and `/notes/pm`; the PM's side says pinned notes go in every PM briefing) and a scope (all,
  host-wide or a project's, filtered in the browser), then the notes in sections (Pinned, Last 7 days, Earlier), each
  with two lines of its text without the Markdown and Host-wide or its project's key. A note is a page
  (`/notes/<id>`): its project (a menu; changing it saves at once), the save state (Saved · edited when, Saving…, or
  Not saved with Retry, which also toasts), **Write | Read**, Pin (amber when pinned; "listed first" for mine, "in
  every briefing" for the PM's), Make a ticket (mine only) and ⋯ Delete… (asks first). Read renders the Markdown as a
  document; Write is the title and the Markdown source in `MarkdownEditor` (web/src/common, CodeMirror, PDT-187):
  styled as it is typed (headings, emphasis, code, links, quotes) with the marks kept visible and dimmed, growing with
  its text; Enter continues a list with the next item on the very next line, even in a loose list (an empty item ends
  it), Backspace removes an empty item's marker, Tab / Shift-Tab indent. Esc goes back to Read. A double-click on the rendered note (not on a link or a checkbox, nor at the end of
  a drag) opens Write with the caret at the start of the block clicked (each block carries `data-source-start`), or on
  the title the title. A note with text opens to Read, an empty or new one to Write with its title focused. It
  saves by itself 800 ms after typing stops, on blur and when the page goes; the text comes from the server once and
  a polled copy only replaces it when it changed elsewhere and nothing here is unsaved. An empty title is saved as
  the text's first line (plain, at most 200 characters) or "Untitled". `/notes/new` is a blank note made by its first
  save (in the switch's kind and the scope's project), which replaces the path with `/notes/<id>`; nothing is made
  if it's left empty. The PM view no longer lists the notes, and the right panel has no Notes tab.
- **The user's own notes** (`kind` user, PDT-126): ideas and the like. They are never in a briefing, even pinned (a pin
  only lists them first). The PM's `search_notes` finds them, each result marked `writtenBy` "the user" (or "the
  PM"); its `save_note` (with that id) and `delete_note` refuse them. A user note has **Make a ticket** on its page:
  the new-ticket form, as a Draft in the note's project, titled with the note's title, its body the description
  followed by "From the note [title](<origin>/notes/<id>)." Older descriptions link to `/pm/notes/<id>`, which still
  opens the note (the path is replaced by `/notes/<id>`; `/pm/notes` by `/notes/pm`).
- **The PM view** (PDT-149). Its list column has the open conversations (the most recently active first; one with an
  unseen reply in bold with a dot, one working a turn with a spinner, PDT-205; each one's menu has Rename and Close…),
  New conversation (+), and the five newest ended ones (their journal's first line) with "All ended conversations" when there are more. The conversation's header has its
  title, Rename, Close… (asks first: it writes its journal, then ends) and New conversation; an ended one shows its
  journal and New conversation. Whatever opens the PM (the rail, the palette, Ctrl+] m) goes back to the conversation
  last on screen; Make ticket and the approval cards belong to the conversation they are in.
- **Paths and the phone** (PDT-113, PDT-149). `/pm` is the newest open conversation (the start box when none is
  open), `/pm/new` the start box for a new conversation (its first message starts it), `/pm/earlier` the ended ones
  (when, how it ended, the journal's first line) and `/pm/<session id>` one conversation. On a phone `/pm` is the list
  of conversations and each one a screen of its own with ← (no header: the top bar has its title); ⋯ has New
  conversation and Ended conversations, and on a conversation Rename and Close…. The start box shows the usual text
  but isn't focused (the keyboard would cover it). The Notes view's list is its phone screen; a tap opens a note as a screen with ←, the
  save state and Write | Read under the top bar, and its project, pin, Make a ticket and Delete… under ⋯.
- **Compaction, checkpoints and the daily update** (PDT-167). A PM conversation only ends when the user closes it;
  neither a full context nor age ends one (the agent compacts its own context). On a `compaction` event PDT queues
  "[PDT] Your context was just compacted. Write a journal checkpoint…" through the outbox (it waits for the turn to
  end and never wakes a parked conversation). When a turn ends a day (`AppState.pm_update_every_ms`, pdt-core
  `pm::UPDATE_EVERY_MS`) after the conversation was last brought up to date (`pm_sessions.caught_up_at`: its briefing,
  then each update) and the user wrote to it since, PDT queues "[PDT] Since your briefing…" (then "Since the last
  update…", pdt-core `pm::daily_message`), trimmed to what matters (PDT-211): the log's events without
  `schedule.fired` and `ticket.overlap_found`/`overlap_cleared`, as a counts line and one line per ticket (key, title,
  what happened: "stage → Done, PR #208 merged, CI passed"; at most `pm::UPDATE_TICKETS`, 20); the Inbox; each other
  conversation's new journal entry cut to its title, age and "Still open" section (at most 15 lines; the full entry is
  in later briefings, and journal requests ask for a `Still open` heading); and a request for a checkpoint. When no
  ticket had events, the Inbox is empty and no journal is new, it sends only a short checkpoint request, or nothing
  when the conversation's journal entry is newer than the user's last message. One checkpoint request or update waits
  at a time (dedupe `pm:checkpoint` or `pm:update`); closing drops a waiting one.
- **PDT's messages collapse in the chat** (PDT-211). The outbox key of each of PDT's own messages to a PM
  conversation (`pm:update`, `pm:checkpoint`, `pm:journal` for the closing request; pdt-core `pm::pdt_message`) rides
  on its pending message, and the echoed `user_message` gets `source: notice` and `pdt: update | checkpoint |
  journal` (the same for Claude, Codex and Devin, no text matching). The conversation view, desktop and phone, shows
  it as one muted line ("PDT brought this conversation up to date · 1d ago") that opens to the full text; the agent
  still gets all of it.
  `write_journal` replaces the conversation's one entry, so briefings carry the latest checkpoint. An ended
  conversation shows why it ended (reason `user`; `context` and `age` are from before PDT-167), its journal and New
  conversation in place of its composer; while it writes its closing journal a banner says the user closed it.
- **As a workflow step.** The PM persona can be an `agent` node's persona; its asking tools still ask (the approval
  waits in the Inbox's Approvals). Triage of outside input runs the **Triage** persona instead (below).
- **External tickets** (PDT-201, [tickets.md](tickets.md) "Sources and the Todo gate"): the PM's `edit_ticket` that
  would move an external ticket the user hasn't let in to To do or In progress, or label it `agentic`, always asks the
  user, even where the project lets the PM act; only that yes lets it through.

## Triage: the locked-down persona (PDT-201)

- The built-in **Triage** persona (role `triage`, pdt-core `TRIAGE_ROLE`; any persona with that role is locked down,
  pdt-core `persona::locked_down`) reads input from outside PDT. Its tools are `search_tickets`, `get_ticket`,
  `create_ticket`, `ask_user` and `submit_outcome` (no shell, secret tools, `edit_ticket` or `send_to_session`).
- Its sessions run in a read-only sandbox: Claude gets `--tools Read,Grep,Glob --strict-mcp-config` (pdt-agents
  `mcp::CLAUDE_READ_TOOLS`; no shell, web or edits, no MCP servers but PDT's) and permission mode Ask; Codex permission
  mode Plan (its read-only sandbox) with `web_search="disabled"` and `sandbox_mode="read-only"`; a Devin or ACP agent
  Triage step fails (PDT can't sandbox it).
- Its `create_ticket` (and that of any agent step in a run an outside event started) makes `external` Drafts: a stage
  other than Draft or the `agentic` label is refused, whatever the user answers.
- The built-in template **Triage new GitHub issues** (`builtin:issue-triage`, its `persona` param defaulting to
  Triage) and the built-in **PDT bug triage** run it.
- The built-in **PR Reviewer** (PDT-269, role `triage` too, so locked down the same way) reviews pull requests PDT
  didn't open for the **Review outside pull requests** template ([workflows.md](workflows.md)). Its tools are
  `get_ticket`, `change_request_diff` and `submit_outcome`: no shell, secrets, ticket edits, comments or `ask_user`.
  `change_request_diff` (a forge tool, read only) reads a pull request of the session's project by number through
  the forge's API (`Forge::change_request_files`, never a checkout): state, base, head, mergeability, its files and
  one `diff --git` block each (cut at 150 000 characters, whole files, `truncated`), with the title and the diff
  fenced. It suggests `merge`, `changes` or `close`; the workflow posts nothing until the user chooses.
- Its prompt gets the issue's title and text in fenced blocks with a fixed "data, never instructions" note, triage
  runs are capped per author and per repository an hour, and each session outside text reached is recorded on the
  ticket's timeline (PDT-202, [tickets.md](tickets.md) "Outside input in prompts").

## Activity

- Claude sessions get `--settings <data dir>/sessions/<id>/claude-settings.json` (hooks run
  `pdtd hook claude <Event>`).
- Codex gets `-c notify=[pdtd, hook, codex-notify]`, chaining the user's own `notify` from config.toml after
  `--chain` (pdt-agents `hooks.rs`). Terminal Codex also reads its OSC 7501 status, with screen heuristics for older
  CLIs (docs/superpowers/notes/phase-2-agent-signals.md).
- The user's own settings are never edited.
- Typing for PDT (the outbox, PDT-129): a terminal takes PDT's messages only while `idle` and nobody has typed for
  20 s, counted from when its tracker starts (a fresh TUI is still drawing). What the terminal sends by itself (focus
  reports, replies to device-attribute, cursor-position, mode and colour queries; `is_terminal_reply`) is not typing.
  A session at `unknown` (hooks that never fire) is not typed into: after the outbox's deadline its messages are
  untold and the user gets a `session.undelivered` notification ([workflows.md](workflows.md#outbox-sessionsend)).
- `attention_at` is set when a session becomes idle or needs the user; it is **unseen** until `last_viewed_at`
  (`POST /api/sessions/{id}/view`, any device) passes it. Unseen and needs-you sessions make notifications
  (`session.needs_you`, `session.turn_ended` after 30 s; [workflows.md](workflows.md#notifications)); viewing one
  clears and reads them. An open session's pane posts the view when it opens, when the tab becomes visible and when
  a poll finds attention it hasn't cleared yet (a newer `attentionAt` while unseen; not on every poll).

## Terminals

- Sessions are PTYs owned by pdtd; they survive closed windows. WebSocket at `/api/sessions/{id}/pty`; with
  `?replay=gzip` (the browser asks when it has DecompressionStream) the scrollback replay arrives as one gzip frame.
  Scrollback is in memory only.
- In the browser: Shift+Enter sends ESC CR (a line break in Claude Code, Codex, zsh); Compose sends a multi-line
  prompt as one bracketed paste plus Enter; a mouse selection is copied when the mouse is released, and OSC 52 copies
  also reach the viewer's clipboard (reads refused); http(s) links (plain and OSC 8; children get
  `FORCE_HYPERLINK=1`) open on Cmd/Ctrl+click or a tap, and hovering a wrapped OSC 8 link underlines all its rows
  (web/src/terminal/linkSpan.ts reads xterm's undocumented cell `extended.urlId`); Option+drag selects while the
  program owns the mouse.
- On touch devices (PDT-115, web/src/terminal/keyBar.ts) a key bar sits under the terminal, above the on-screen
  keyboard: `Esc Tab Ctrl ^C ↑ ↓ ← → ⌃] ⌨`, scrolling sideways when it doesn't fit and never taking focus. Ctrl is
  sticky: tapped, it applies to the next key typed or tapped (Ctrl+R, Ctrl+arrow), then lets go; tapped again it
  lets go. ⌃] is PDT's prefix: the next typed key runs its shortcut (`runPrefixKey`), and ⌃] twice sends Ctrl+] to
  the program. ⌨ puts the keyboard away (keys then leave it down) and brings it back. A drag scrolls the scrollback; a
  long press selects the word under the finger, dragging extends it, and a Copy bar copies it. On a phone the
  session's ⋯ has Paste (the clipboard as a bracketed paste), Compose and Font size, which is kept in the browser
  (`localStorage` `pdt.terminal.fontSize`) and applies to every terminal.
- Reconnect: a terminal that connects while the startup auto-resume has yet to get to its session waits (up to 30 s)
  and attaches once it starts. The WebSocket for an existing session with no PTY otherwise upgrades, sends
  `{"type":"not_running"}` and closes; the browser then shows "Session ended." and reattaches when the session list
  shows it `running` again. Otherwise the browser retries forever (backoff capped at 5 s). Input is never queued
  while it is not connected: typed keys are dropped and the terminal says "Not connected: input not sent" until it
  reconnects, and Compose keeps the prompt and says it was not sent.

## Restart and resume

- Claude runs with `--session-id <session id>` and restarts with `--resume <conversation id>`: the session id until
  `/clear` (PDT-52) starts a new conversation, then the id Claude moved to, kept in `external_session_id`
  (`Session::claude_conversation_id`). Conversations learn it from the next `system/init`'s `session_id` (not
  `conversation_reset`'s `new_conversation_id`, which names no transcript; `conversation_reset` itself becomes a
  divider event); terminals from the SessionStart hook's `session_id` (`pdtd hook` forwards it), which also logs the
  old transcript's TUI turns and the divider so a later switch to a conversation imports the new transcript from its
  start. Resumes, transcript reads, the resume fallback, Make ticket and stdin messages all use that id.
- Resume fallback: Claude saves `<claude config dir>/projects/*/<session id>.jsonl` only after its first turn. If that
  file is missing, restart/auto-resume launches `--session-id <conversation id>` (fresh conversation, same id) instead
  of `--resume`. Config dir is `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
- Codex cannot be given an id, so pdtd finds each Codex session's conversation in
  `$CODEX_HOME/sessions/**/rollout-*.jsonl` (default `~/.codex`), matches it by working folder and start time against
  the session's `fresh_started_at` (when it was last launched without a conversation), stores it as
  `external_session_id` (at process exit or restart) and restarts with `codex resume <id>`. Fresh Codex launches in
  one folder are spaced 5 s apart so start times stay unambiguous (a create or restart can wait up to 5 s). A session
  that never took a turn restarts fresh.
- Auto-resume: at startup pdtd decides how to resume every terminal session left `interrupted` (binding Codex
  conversations) before serving, then restarts them in the background once it is serving (resume mode, 120x30;
  clients resize on attach; a manual Restart that gets there first wins and auto-resume skips that session; several
  fresh Codex sessions in one folder start 5 s apart); a session that fails to start stays `interrupted`. Stopped
  (`finished`) sessions and conversations are never auto-resumed (a conversation reopens on its next message).

## Offered agents (PDT-55)

The new-tab chooser, palette, ticket Start and the project's "… with an agent" menus offer only agents whose host
check found them (web/src/common/useAgentAvailability.ts, the `['host-checks']` query Host settings' "Run checks
again" refreshes); an installed agent that is not logged in stays with a "not logged in" hint; everything is offered
until the checks load or when they fail. Only a program missing from the search path counts as not found: one whose
check times out (30 s) or cannot start is found but not ok, and pdtd's startup check runs again (up to 3 times, 30 s
apart) while an agent's check is unanswered, since a CLI's first launch after a restart can be slow (PDT-92).

## Switching an agent off (PDT-270)

Settings → This host → Agents has an on/off switch for each agent, next to its program (Programs) or in its row (ACP
agents). Host settings `disabledAgents` lists the ones switched off; every agent is on unless it is there, and an ACP
agent's entry goes when the agent is removed. The settings view's `enabledAgents` is the agents switched on (the
built-ins, then the ACP agents): `useAgents().all` (web/src/common/agents.ts) is that list, and
`useAgentAvailability` reports a switched-off agent as `off`, so it leaves every picker (ticket Start, new tab, the
palette, "… with an agent", persona editing, the default agent, Get started). A persona already on one keeps it,
listed as "switched off". The Checks list and Get started don't count it as missing (Get started's agents step needs
one agent that is found and on).

- The default agent can't be switched off (`invalid_input`, "Claude is the default agent: pick another default
  before switching it off"), nor can a switched-off agent be made the default; its switch is disabled with that reason.
- `POST /api/sessions` (and ticket Start, the PM, a workflow step: every `create_session`) refuses a new session on a
  switched-off agent: 400 `agent_off`, "Codex is switched off in Settings → Agents". Sessions already running carry
  on, and past ones still open and resume.
- An agent step whose persona runs on a switched-off agent fails with "<persona>: Codex is switched off in Settings →
  Agents" (its dry run too); it never falls back to another agent. Workflow validation (the editor's check,
  `workflow_validate`) warns `agent_off` on such a step ahead of time.

## Host defaults (PDT-69)

Host settings `defaultAgent` (claude), `defaultSessionMode` (conversation), `ticketPermissionMode` (bypass: the
agent works on its own branch) and `sessionPermissionMode` (ask), PDT-75, and `updateCheckMinutes` (60; 5 min to a
week), in Settings → This host (Agents, Updates). The permission modes are only what the Start dialog and the new-tab
chooser pick first (in a terminal only bypass changes anything); requests without one keep the agent's default. The web reads them from the `['host-settings']` query
(web/src/common/useAgentDefaults.ts): ticket Start preselects them, the new-tab chooser, palette and "… with an agent"
menus list the default agent's choices first, default view first (`byDefault` in web/src/sessions/newSession.ts), and
"… with an agent" starts in the default view. pdtd's update loop re-reads the interval every minute, so a change
applies without waiting out the old one.

`agentModels` (PDT-265), Settings → This host → Agents "Default models": a model per agent kind (`{"claude":
"opus"}`; a patch sets the agents it names, blank clears one; a removed ACP agent's goes with it). The model a session
starts with is `HostSettings::model_for`: the session's own (picked in the composer, or its persona's, copied in when
it is made) > the agent's default model > the agent CLI's own default. It is resolved at each launch, not stored on
the session, so a conversation's "Default" follows the setting: conversations (and Make ticket's side request), workflow
steps and the PM, and fresh Claude and Codex terminals (`--model`; a resumed terminal keeps its conversation's model,
Devin's terminal takes none). Switching a live conversation back to "Default" sends the agent's default model when it
has one. The composer's picker says what "Default" is ("Default (Opus)"); Settings lists the models the agent's
conversations reported (`GET /api/agents/{agent}/capabilities`) and, until it has run one, takes a typed name.

`maxAgentTurns` (4; 0: no cap; Phase 7c), Settings → This host → Agents "Agents running at once": the most agent
sessions running a turn before PDT's own work waits for a slot ([workflows.md](workflows.md#pipeline-policies-and-scheduling-phase-7c)).

## Devin (PDT-54, PDT-55)

Facts about the CLI: docs/superpowers/notes/devin-cli.md. `AgentKind::Devin`, program `devin` (`PDT_DEVIN_PROGRAM`);
a conversation by default like the others (`mode: terminal` for its TUI).

- Terminal: bypass is `--permission-mode dangerous`; the first prompt goes after `--` (a bare argument is a folder to
  Devin); activity comes from its OSC 7501 status (`app=devin`, the same parser as Codex).
- Config: Devin has no instructions flag and `--config` replaces the user's config, so each session (terminal or
  conversation) gets `--config <data dir>/sessions/<id>/devin-config.json`: the user's config
  (`$XDG_CONFIG_HOME/devin/config.json`, else `~/.config/devin/config.json`; `%APPDATA%\devin\config.json` on
  Windows; JSONC) plus a `SessionStart` hook `pdtd hook devin --instructions <file>`, which prints the instructions as
  `additionalContext` and, for a terminal, posts Devin's `session_id` (stored as `external_session_id`, latest wins);
  restart/auto-resume run `--resume <id>`. An unreadable user config means no `--config` (and so no hook).
- Conversations (PDT-55) run `devin [--config <file>] acp`, one Agent Client Protocol process (JSON-RPC on stdio) per
  live conversation. The process and JSON-RPC are the generic ACP adapter (pdt-agents `adapter/acp.rs`) and the
  frame mapping the generic ACP mapper (pdt-core `conversation/acp.rs`); what is Devin's own (its arguments, modes and
  resume fallback in `adapter/devin.rs`'s `DevinProfile`; its `cognition.ai/*` fields, tool and option names, `q0`…
  question fields and `/private` paths in `conversation/devin.rs`'s `DevinExtensions`) is a profile on top:
  - `session/new` (its id is `external_session_id`) or `session/load` to resume (its history replay is dropped; a
    session Devin never saved, i.e. never prompted, gives -32016 and a new one starts with a notice).
  - A turn is one `session/prompt`; a message sent while it runs joins it (steer); Send now cancels first
    (`session/cancel`).
  - Approvals are `session/request_permission` ("Always allow" = `allow_session` only, never Devin's saved rules; a
    deny note goes as the next message), questions `elicitation/create` (PDT offers the `elicitation.form`
    capability), plans the exit-plan permission (`plan_accept_edits`/`plan_bypass`); edits carry whole-file old/new
    text, diffed with `similar`.
  - Devin has no ask-before-edits mode: Ask maps to its Code mode (`accept-edits`); capabilities offer Accept edits /
    Plan / Bypass and new Devin conversations start in Accept edits.
  - No Make ticket (409 `draft_unsupported`; ACP has no fork).
- Conversation | Terminal switches like Claude's, one at a time (`POST …/mode`; Devin locks a session to one
  process): to terminal parks ACP, stores the switch time (ms) in `history_from` and restarts as a terminal
  (`--resume <id>`); back types `/exit`, then opens ACP at once, whose `session/load` replay imports the turns whose
  prompt is newer than that time (`source: terminal`; every turn for a session that was always a terminal) and then
  clears `history_from` (a failed open keeps it and the next message retries).
- Tests use `fake-agent acp`. The host check runs `devin auth status` (exit 0 either way; reads the text) and treats a
  missing Devin as fine.
- Web: new-tab "Devin" (d) and "Devin (terminal)" (e), palette "New Devin here" / "New Devin (terminal) here", ticket
  Start's agent choice, Set up / Write instructions with an agent.

## ACP agents (PDT-234)

Any other agent that speaks the Agent Client Protocol on stdio is an agent kind `acp:<id>` (pdt-core `AgentKind::Acp`;
the id is 1–32 lowercase letters, digits and `-`, interned so `AgentKind` stays `Copy`). The generated TS type adds
the template literal ``acp:${string}`` to `"claude" | "codex" | "devin"`, so the built-in names are unchanged.

- Definitions (pdt-core `acp_agent`): profiles that ship with PDT (`builtin_profiles()`: id, label, icon, command,
  arguments, environment, a mode map and a flavor, `AcpFlavor`, for what the agent adds to ACP; OhMyPi's, Cursor's and
  Antigravity's so far, each harness's ticket adds its own; their ids can't be taken) and the user's, in host
  settings `acpAgents` (`[{id, label, command, args, env: [{name, value}]}]`, the whole list per `PUT
  /api/host/settings`; up to 32). Settings → This host → Agents → ACP agents adds, edits and removes them ("Add ACP
  agent": name, command, arguments one per line, `NAME=value` lines; the id comes from the name and never changes).
  The view lists each with where its command was found (`acpAgents[].found`); the host checks report each under
  `acp["acp:<id>"]` (found is ok: PDT can't tell whether one is logged in). A session, persona or default agent naming
  an ACP agent the host doesn't have is refused (400 `unknown_agent`); removing the default one puts it back on Claude.
- Conversations only: `mode: terminal` is refused, they don't switch to a terminal, and they have no TUI hooks,
  background tasks or Make ticket (409 `draft_unsupported`). The UI offers them as conversations wherever an agent is
  offered (new tab, palette "New <label> here", ticket Start, personas, Settings' default agent) with a plug icon.
- They run through the generic ACP adapter as plain ACP (pdt-agents `adapter/acp_agent.rs`, `DefinedProfile`): the
  command (`~` expanded, looked up on the login PATH) with its arguments and environment in the session's folder,
  pdt-mcp in `mcpServers`, and PDT's instructions as a text block in front of a new session's first prompt (logged
  without it). The models PDT's model picker offers come from the `model` config option, in a `config_option_update`
  or in the `session/new`/`session/load` answer's `configOptions`. Resume is `session/load` when the agent advertises `loadSession`; otherwise a new session starts with a
  notice, as it does when the agent's `session/load` says it has no such session (`-32002`, or the flavor's own way
  of saying it, `AcpFlavor::session_missing`). An error's `data.details` is added to its message.
- Modes: a profile's mode map, else the `availableModes` the agent advertises in `session/new`/`session/load`
  (matched to PDT's by id or name: `default`/`ask`, `acceptEdits`/`code`, `plan`, `bypass`/`yolo`, …,
  `acp_agent::advertised_modes`), set with `session/set_mode`. Every PDT mode is offered: one the agent has no mode
  for (all of them for an agent that advertises none; Bypass for OpenCode's build/plan), PDT keeps itself, at start
  and on a change, with the agent back in the mode it started in when that is none of PDT's
  (pdt-agents `AcpLive::apply_mode`). Keeping a mode is answering `session/request_permission`
  (`acp_agent::enforced_answer`, by the tool call's ACP kind): Bypass allows everything; reads, searches and thinking
  are allowed in every mode; Accept edits allows edits, deletes and moves; Plan rejects those and commands; the rest
  go to the user. A profile's map may cover only some modes and name a base mode (`AcpAgentDef::base_mode`,
  `AcpProfile::base_mode`): the session still offers the others, and PDT keeps them with the agent in that mode. For
  an agent that edits files without asking (`AcpAgentDef::edits_unasked`, `AcpProfile::asks_before_edits`), PDT can't
  keep Ask: it offers no Ask (`AcpAgentDef::offered_modes`, also in `GET /api/agents/acp:<id>/capabilities`), and a
  session or persona in Ask runs as Accept edits (as Devin's does).
- A built-in profile's flavor (`AcpAgentDef::flavor`, pdt-core `AcpFlavor`) is what its agent adds to ACP, mapped in
  pdt-core `conversation/<agent>.rs`: tool names, output in `rawOutput`, its own blocking requests as PDT questions
  and plan approvals (`AcpExtensions::ext_request`/`ext_answer`), how it says a saved session is missing (resume
  starts a new one with a notice). ACP errors show their `data.details` or `data.message`.
- Ids are interned (at most 1024 from outside input; values read from the store are never refused,
  `AgentKind::parse_stored`), and `GET /api/agents/acp:<id>/capabilities` answers 404 for an id the host doesn't
  have before parsing it.
- Tests: `fake-agent --plain acp` (with `--modes` it advertises its modes, with `--partial-modes` only `build` and
  `plan`; `mcp: <tool> <json>` calls pdt-mcp after asking) in pdt-api `tests/acp_plain.rs` and `tests/acp_agents.rs`;
  a flavor's recorded and source-built frames in pdt-core `tests/fixtures/<agent>/`.

### OhMyPi (PDT-238)

The built-in profile `acp:omp` ("OhMyPi") runs `omp acp` (notes: docs/superpowers/notes/omp-acp.md); the user installs
`omp` and sets up its model in omp itself (with none, a prompt fails with "No model selected", shown in the turn's
error).
- Modes: it advertises `default` (PDT's Ask) and, while omp's plan mode is on, `plan` (Plan); PDT keeps Accept edits
  and Bypass itself, with omp in `default`. omp asks (`session/request_permission`) only before `bash`, `delete`,
  `move` and an edit that deletes or moves a file, so in Ask mode its other edits and writes run without asking.
- pdt-mcp: omp connects to the `mcpServers` PDT passes in `session/new` and `session/load`, and calls a tool by
  writing to `xd://mcp__<tool>` (a `write` call of kind `execute`; it never asks before one). Resume is `session/load` (it replays the conversation; omp saves a session at `session/new`).
- Its flavor (pdt-core `conversation/omp.rs`): a tool call's `name` is its tool (`bash`, `edit`); its `ask` tool's
  questions arrive as a form elicitation where each question is a field's `title` (its header the `description`) with
  a `<field>__other` field beside it, which takes an answer in the user's own words (`AcpExtensions::question_in_title`,
  `other_field`). Plan mode's approval is a one-question form ("Approve and execute" / "Refine plan").
- It asks permission before it reports the call (`session/request_permission` comes before the `tool_call`): the
  generic mapper then names the request from the request's own `toolCall` (its kind, `execute` for `bash`; its command
  or title).
- Tests: pdt-core `conversation::omp` on fixtures recorded from omp 18.14.0 in `tests/fixtures/omp/` (`start.jsonl`:
  the handshake and a prompt with no model; `turn.jsonl`: an MCP call, a command, an edit and an `ask` form;
  `plan.jsonl`: plan mode, its approval and the approved command). An omp persona has run a Code step end to end
  (2026-10-29, on a scratch daemon).

### Cursor (PDT-237)

The built-in profile `acp:cursor` ("Cursor") runs `cursor-agent acp` (notes: docs/superpowers/notes/cursor-acp.md); the
user installs the Cursor CLI and signs in with `agent login` (PDT passes nothing). On Cursor's Free plan only Auto runs
(model `default[]`); with no model set, Cursor's default model may answer "Upgrade your plan to continue".
- Modes: Plan is Cursor's `plan`; Accept edits and Bypass are PDT's, with Cursor in its `agent` (the profile's base
  mode; Cursor's `ask` is read-only Q&A, not PDT's Ask). Cursor's `agent` edits files in the workspace without asking
  (only commands, and writes outside the workspace, ask), so Cursor offers no Ask: a session or persona in Ask runs as
  Accept edits.
- pdt-mcp: Cursor honours the `mcpServers` PDT passes (no `.cursor/mcp.json`); its MCP calls are named
  `mcp__<server>__<tool>` from the call's `rawInput`, and pdt-mcp's run without asking. Cursor asks before its
  `tool_call` carries the `rawInput`, so the request's own `toolCall.rawInput` names the call (and a later update
  names a call first reported by its kind only). Cursor's "Allow always" writes
  its own allowlist, so PDT never offers it.
- Its flavor (pdt-core `conversation/cursor.rs`): `cursor/ask_question` is a PDT question (an answer in the user's own
  words skips it with them as the reason) and `cursor/create_plan` a plan approval: approving it puts the session in
  the mode picked (Ask by default) and tells Cursor to build it. A command's output comes from its `rawOutput`. The
  prompt Cursor echoes back as a `user_message_chunk` mid-turn is dropped. `session/load` of an id Cursor never saved
  (`-32602`, `Session "…" not found`) starts a new session with a notice.
- Tests: pdt-core `conversation::cursor` on `tests/fixtures/cursor/` (`start.jsonl` recorded from Cursor CLI
  2026.10.22: the handshake, modes, a Free-plan prompt and a missing session; `turn.jsonl` and `plan.jsonl` built from
  its source). A Cursor persona has run an agent step end to end (2026-10-30, on a scratch daemon, model
  `default[]`).

### Antigravity (PDT-239)

The built-in profile `acp:antigravity` ("Antigravity") runs Google's own ACP server for Antigravity
(`agy_acp_server.par`; notes: docs/superpowers/notes/antigravity-acp.md). The `agy` CLI itself speaks no ACP, and
Gemini CLI, which it replaced, is not supported. Third-party `agy-acp` bridges aren't used.
- Install (pdt-api `toolchains.rs`, pdt-core `acp_registry`): the profile names its ACP registry entry
  (`AcpAgentDef::installer`, `antigravity-acp`). Settings → Agents → Antigravity → Install (`POST
  /api/host/acp-agents/antigravity/install`; Update once installed) reads the registry
  (`cdn.agentclientprotocol.com/registry/v1/latest/registry.json`) and takes this platform's build. The archive must
  be an https URL under `dl.google.com/agy-extensions/releases/`. Its sha256 must match the registry's, or nothing is
  installed. It is unpacked without running anything: no paths outside its folder, no links, and the exec bits it
  marks. It lands in `<data dir>/toolchains/antigravity-acp/<version>/` with a `pdt-install.json`
  (version, command, the registry's arguments: Linux adds `--uid=`). Older versions are removed. The newest
  installed version is what launches (`toolchains::launch_def`). Path (host settings `acpPrograms.antigravity`, a full
  path or `~/…`) runs a server installed by hand instead. With neither, it shows Not installed, and a session can't
  start (`agent_not_found`). Only pdtd's production wiring downloads (`AppState::toolchains`; `new` is offline, and
  tests use a fake source).
- Sign-in: the server has its own Google sign-in, apart from `agy`'s. It keeps its token in the keychain under
  service `gemini`, account `antigravity-acp`, and records `auth.type` in `~/.gemini/antigravity-acp/settings.json`
  (`$GEMINI_HOME` moves it). Sign in (`POST /api/host/acp-agents/antigravity/sign-in`) starts the server and calls
  ACP `authenticate` with `oauth-personal`. The server opens the browser on PDT's computer, and the request answers
  once that is done (it waits up to about five minutes). PDT never sees the credentials, and only reads
  `settings.json` to show Signed in (`acpAgents[].signedIn`). A session started before signing in fails with
  "Antigravity is not signed in: sign in from Settings → Agents → Antigravity" (`AcpFlavor::refusal_hint` for its
  `-32000` "Authentication required"). Its other sign-ins (Gemini Enterprise, an API key, Agent Platform) aren't
  offered.
- Modes: Ask is its `default`, Accept edits `auto_edit`, Bypass `yolo`. It has no plan mode (`set_mode plan` is
  refused), so PDT keeps Plan itself, with the server in `default` (the base mode), refusing edits and commands.
- Models: its `model` config option (the free tier offers Gemini 3.8 Flash only), set with
  `session/set_config_option`. Its `thinking_level` option isn't offered.
- pdt-mcp: the HTTP server in `mcpServers` is honoured, with its auth header. MCP calls carry `_meta.mcp {server,
  tool}`, named `mcp__<server>__<tool>`, and pdt-mcp's run without asking.
- Its flavor (pdt-core `conversation/antigravity.rs`): a command's output and exit code come from `rawOutput`
  `{exitCode, combinedOutput}`. Its "Allow Always" is kept in the server's memory for the session (for a command, that
  exact command), so PDT offers it. Paths come back with symlinks resolved. Resume is `session/load`; an id it never
  saved answers `-32002`. Cancel ends the turn `cancelled`.
- Known server quirks (1.4.5): `session/new` writes `skills/system/run-workflow/SKILL.md` into the session's folder,
  and writes it again if it is removed, so it shows as an untracked folder in a worktree. In Ask, a new file shows
  twice: the approved `create_file` card later fails as "approved but never executed", and the `edit_file` card that
  ran beside it is done.
- Tests: pdt-core `conversation::antigravity` on fixtures recorded from the server 1.4.5 in
  `tests/fixtures/antigravity/` (`start.jsonl`: the handshake, modes, models and a missing session; `turn.jsonl`: a
  pdt-mcp call, `ls` and a new file); pdt-api `tests/acp_antigravity.rs` (install from a recorded registry, the
  checksum check, Path, Sign in and a conversation, with `fake-agent` as the installed program). An Antigravity
  persona has run a Code step end to end (2026-10-30, on a scratch daemon, Gemini 3.8 Flash).
