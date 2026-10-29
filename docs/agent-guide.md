# Working in a PDT project (guide for agents)

PDT serves this guide itself at `GET /api/guides/agents`, so an agent can read it on any host, with or without a copy
of PDT's repository. It explains how an agent session started by PDT finds PDT and uses its tickets, worktrees, runs
and tests. How *this* project wants you to work (branches, checks, reviews) is in the repository's own instructions
and in the project's **agent instructions** from PDT's settings, which PDT adds to every agent session it starts.

## Finding PDT

PDT's instructions for this session name its API, `http://127.0.0.1:<port>/api`, and the session id. Terminal
sessions also have `PDT_HOOK_PORT` (the port) and `PDT_SESSION_ID`. The API is local, takes and returns JSON, needs
no authentication, and reports errors as `{code, message}`.

| Call | Gives |
|---|---|
| `GET /api/sessions/<session id>` | this session: `projectId`, `worktreeId` (null in the main checkout), `cwd`, `ticket` |
| `GET /api/projects` | projects: `id`, `name`, `key`, `repoPath`, `baseBranch` |
| `GET /api/projects/<id>/worktrees` | the project's worktrees: `id`, `branch`, `path` |
| `GET /api/guides` | PDT's guides (this one, the run configuration reference, and the prompts for setting up a project's runs and its agent instructions) |

## PDT's tools (pdt-mcp)

Most sessions PDT starts also get PDT's MCP server, named `pdt` (in Claude its tools are `mcp__pdt__<tool>`). Its
tools know which session is calling, so they need no session, ticket or attempt ids, and they run without asking,
except the PM's `create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `stop_ticket` and `send_to_session`, which the
person confirms per call unless the project lets the PM act. Use them for what they cover; the API above stays available for everything else.

| Tool | Does |
|---|---|
| `get_ticket` | this session's ticket (or another, by key): description, stage, links, attempts, comments and history (each `by` a persona, `user` or `system`, with an agent's `sessionId` and the `fromTicket` it works on), the outcomes earlier workflow steps submitted, and in a workflow step, the outcomes it allows |
| `submit_outcome` | `{outcome, summary, comments?}`: ends this session's workflow step with one of its outcomes; `comments` (`{file, line, endLine?, body}`) become line comments on the changes. `submitted` is refused while the worktree has uncommitted changes to tracked files, commits not pushed to its upstream, or a branch not pushed to `origin`: commit and push first |
| `ask_user` | `{questions: [{question, choices?}]}`: puts questions in the person's Inbox; the answers come back as a message, so end your turn after asking |
| `add_ticket_comment` | `{body, ticket?}`: a comment on the ticket's timeline, by the agent |
| `list_secrets` | the secrets (API keys, tokens, passwords) this session may use: names and descriptions, never values |
| `request_secret` | `{name, purpose}`: asks the person for a secret; they type it into PDT's Inbox, never the chat. You get `stored: true` once it is (at once if it already is), never the value: end your turn after asking |
| `run_with_secrets` | `{command, env, cwd?, timeoutSecs?}`: runs a command line with `env` values `secret:<name>` set to the secrets; the output comes back with each value replaced by `[secret:<name>]` |
| `write_secret_file` | `{path, template}`: writes a file (a `.env`, an `.npmrc`) with `{{secret.<name>}}` filled in, without echoing it |
| `add_review_comment` | `{file, line, endLine?, body}`: a line comment on the attempt's changes |
| `update_ticket` | `{title?, description?}`: rewrites this session's own ticket (the Refiner's tool; a session gets it when its persona lists it) |
| `catalog_search`, `catalog_get`, `workflow_get`, `workflow_validate`, `workflow_dry_run`, `workflow_save` | the workflow catalog and documents, for sessions whose persona has them (the catalog includes each MCP server's tools as `mcp:<server>/<tool>`, used as an `mcp.tool` node); a change to a workflow is a proposal the person accepts, and a new workflow starts disabled |
| `get_overview`, `search_tickets`, `get_session`, `list_workflow_runs` | the PM's view of every project: open tickets per stage, tickets in progress with PR and CI, the Inbox, running sessions; ticket search; a session's state and last messages; workflow runs and their steps |
| `search_notes`, `save_note`, `delete_note`, `write_journal` | the PM's memory: notes (host-wide or per project; pinned ones go in every PM briefing) and the journal entry each PM conversation keeps (checkpoints after a compaction and daily, the last one when the user closes it) |
| `list_todos` | `{project?, done?}`: the person's own todo list (title, notes `body` in Markdown, done, project, linked ticket, due date `dueOn` and time `dueTime`, repeat rule `recurrence`, dates), read-only |
| `create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `stop_ticket`, `send_to_session` | the PM's changes: a new ticket (a draft by default), an edit to any ticket (also its parent; outside the PM, another ticket than your own only by its exact key, and not the title or description of one in progress or done you didn't create; moving one at work back to Draft, Ready or To do stops its work), a link between two tickets (`blocks`, `blockedBy`, `relates`) or its removal, stopping a ticket's work (`{ticket, stage?, removeWorktree?, deleteBranch?}`: cancels its workflow runs, stops its sessions, abandons its attempt and moves it back to To do; `removeWorktree` also discards its worktree), a message to a session (delivered when it is idle, so it can't stop a busy agent); each waits for the person's yes unless the project lets the PM act |

Never ask the person to paste a secret into the chat, and never print, log or commit one: ask with `request_secret`
and use it through `run_with_secrets` or `write_secret_file` (keep files they write out of git). PDT replaces every
known value with `[secret:<name>]` in what it stores and shows.

A session that runs a step of a workflow (a **persona** such as the Reviewer) has a section "Your role" in its
instructions: it finishes the step with `submit_outcome`, never by what it writes, and asks the person with
`ask_user` when only they can unblock it. A turn that ends with neither is nudged once, then shown to the person as
stalled.

## Tickets

A project's tickets have keys `<project key>-<n>` (`APP-12`). Stages: `draft`, `ready` (refined, waiting in the
backlog), `todo` (picked for the board), `in_progress`, `done`, `wont_do`. Priority: `none`, `low`, `medium`,
`high`, `urgent`. Labels are lower-case without commas.

| Call | Does |
|---|---|
| `GET /api/tickets?project=<id>` | the project's tickets, ranked. Also `q` (search, or a key), `stage`, `priority`, `label` (comma-separated lists), `sort` (`rank`, `priority`, `updated`, `created`), `limit`, `cursor`; returns `{items, nextCursor, total}` |
| `GET /api/tickets/<id>` | one ticket with its `description`, links and attempt |
| `POST /api/projects/<id>/tickets` | `{title, description?, stage?, priority?, labels?, areas?, parent?}`; a new ticket is a Draft at the top of its stage (a part, with `parent`, goes after the parent's last part) |
| `PATCH /api/tickets/<id>` | any of `title`, `description`, `stage`, `priority`, `labels`, `areas`, `parent` (a ticket id, `""` to detach), `waiting` |
| `GET /api/tickets/<id>/events` | the ticket's timeline, oldest first (`newestFirst=true` for newest first; `limit`, `cursor`; returns `{items, nextCursor}`). Comments are the items with `kind: "comment"`, their text in `body` |
| `POST /api/tickets/<id>/comments` | `{body}`; your comment shows as an agent's (actor `agent`), not the person's, and is not sent back to you |
| `POST /api/tickets/<id>/links` | `{kind: "blocks" \| "relates", to: <other ticket id>}` (this ticket blocks `to`) |

The person reviews your branch in PDT's Changes view and sends line comments back as one message that starts
"Review feedback on <branch>": each comment names `file:line`, quotes the lines and says what to change. Address each
one, then say briefly what you changed. A message saying a merge into the base conflicts lists the files: merge the base
branch into yours, resolve them, run the checks and commit. `GET /api/worktrees/<id>/changes` lists what PDT counts
as your changes (committed or not, against the merge base with the base branch).

A session started on a ticket gets the rules for that work in its instructions: follow-up work goes in a new ticket
rather than widening the change, and moving the ticket to `done` and removing its worktree are left to PDT (when it
sees the branch merged) or the person.

## Workflows and events

PDT reacts to what happens (CI results, pull request news, merges, comments) through workflows: a trigger on an
event, then conditions, gates, waits and actions. Messages that start with `[PDT]` in your session come from a
workflow the person enabled, for example the built-in "PR notices" (CI failed, a conflict, dropped from the merge
queue, merged); each says what happened and what to do. When the pull request was set to merge, PR notices sends
the failure as a workflow step instead: fix it, push, and finish with `submit_outcome` (PDT turns auto-merge back on). A notice that comes while you are on a step that hands in work (a Coder's pass sent back by review or QA) goes in
with that step's message, after the review and QA findings: address the findings first, then the notice. A session started on a ticket is told in its instructions
which messages to expect, how to act on each, and, when PDT watches the pull request for it, not to watch CI in a
shell.

To read CI yourself, use PDT's forge tools on its MCP server: `ci_runs` (your branch's runs from each of the project's
CI providers, with the failed jobs) and `ci_failed_logs` (the end of each failed job's log; give the run's `provider`
and `runId` from `ci_runs` or the failure message), or the ticket's attempt (`pr.ci`: the head commit's state, URL and
runs, once PDT has seen the pull request). `change_request_view` shows your pull request with its comments and reviews,
`change_request_comment` answers on it (`replyTo` answers a review comment in its thread), and `issue_view` and
`issue_comment` read and comment on issues. `issue_create` files a new issue (from a ticket, which then links it); the
user approves each one. `change_request_open` opens the pull request for your branch once you have
pushed it (a title, a Markdown body, `base` and `draft` optional), and `change_request_merge` asks for its merge again
after the merge queue dropped it and you pushed the fix (only for a pull request already set to merge; the project's
merge policy applies). They use the user's GitHub login in PDT, so you need no GitHub CLI; you still push with plain
`git push`.

Workflows are changed only when the person asks, and then as a **proposal**: `PUT /api/workflows/<id>` with
`{"doc": …, "note": "what and why", "propose": true}`. It is validated like a save but doesn't take effect: the person
reviews its diff in the Workflows view and accepts or discards it. Never turn a workflow on (`PATCH {"enabled": true}`):
that is the person's consent. Check a document with `/check` or `/validate` and try it with `/dry-run` before proposing.

| Call | Gives |
|---|---|
| `GET /api/events?ticket=<id>` | what PDT logged about a ticket (`type`, `payload`), newest first |
| `GET /api/workflows?project=<id>` | the workflows that apply to the project, with `enabled` |
| `GET /api/workflows/catalog` | every trigger, condition and action, with its config schema and ports |
| `GET /api/workflow-runs?ticket=<id>` | the runs about a ticket; `GET /api/workflow-runs/<id>` has each step's input, output and port |
| `POST /api/workflows/validate` · `/dry-run` | `{doc}` checks a document; `{doc or workflowId, event: {type, payload}}` shows what each step would do, changing nothing |
| `GET /api/workflows/<id>/versions` · `/diff?from=&to=` | the versions (`status`: `saved`, `proposed`, `discarded`) and what changed between two |
| `GET /api/workflows/<id>/export` · `POST /api/workflows/export {ids}` | workflows as one export (below), to paste into another PDT |
| `POST /api/workflows/import` | `{text, check: true}` reports what an export needs here; `{text, mapping, targets}` imports it, off |
| `GET /api/workflow-templates` | templates (`params`, a `doc` with `{{params.<id>}}`); `POST …/<id>/instantiate {params, name, scope, projectId}` makes a workflow from one, off |
| `GET /api/personas` | the personas agent steps run (Refiner, Coder, Reviewer, QA, PM and the person's own): agent, model, instructions, permission mode, tools |
| `GET /api/inbox` | what waits on the person: questions, approvals, stalled steps, loop limits and gates |

**Moving a workflow to another PDT.** An export is pretty-printed JSON, the same text copied or downloaded:
`{format: "pdt-workflow", formatVersion: 1, pdtVersion, exportedAt, workflows: [{name, description, scope, project?,
doc}], requires: {connections: [{name, kind, tools}], personas, projects: [{key, name}]}}`. It has the current
documents only (no runs, state, versions or secrets); project ids in a document are `{"$project": "<KEY>"}`, and MCP
servers and webhook hooks are connections of kind `mcp` and `webhook`. Import also takes a bare document. Importing
checks first (`check: true`: each needed name `found`, `missing` or `kind_mismatch`, and same-named workflows here),
then needs every missing connection, persona and project mapped to one here or to `null` (left unmapped) in
`mapping: {connections, personas, projects}`; `targets` (one per workflow) give `scope`/`projectId` and, on a name
clash, `copy: true` (`<name> (imported)`) or `versionOf: <id>`. New workflows start off; the answer's `warnings` say
what is left to set up. With pdt-mcp: `workflow_export` and `workflow_import` (the person confirms it).

## Worktrees

A project with `kind: "personal"` has no repository: its tickets run in a folder of their own, with no branch,
worktree, runs or pull request.

PDT keeps each branch's checkout at `<repo>/.worktrees/<branch>`, with `/` in the branch name replaced by `-` (listed in `.git/info/exclude`, so nothing is
committed for it). Making one through PDT means its sessions and runs know about it:

`POST /api/projects/<id>/worktrees` with `{"branch": "fix/login", "base": "main"}` (`base` defaults to the project's
base branch) makes the branch and its folder and returns the worktree (`id`, `path`). Work there with `cd <path>`.
A folder made with plain `git worktree add` under `.worktrees/` is picked up too.

## Runs and tests

A project's runs (dev servers, apps) and tests come from `.pdt/project.toml` in the checkout and from PDT's host
overrides, which keep the whole configuration out of the repository. `GET /api/guides/run-config` is the reference.

| Call | Does |
|---|---|
| `GET /api/projects/<id>/run-config?worktreeId=<id>` | the configuration: `file`, `overrides`, the merged `effective` (`runs`, `tests`, `areas`), or `error` |
| `POST /api/projects/<id>/runs` | `{name, kind: "run" \| "test", worktreeId?}`; starts it (main checkout without `worktreeId`) in the worktree's ticket workspace; the same run finished earlier in that checkout restarts in its tab (200) instead of adding one. Finishing the ticket removes its runs |
| `GET /api/runs/<id>` | `state` (`setting_up`, `starting`, `ready`, `running`, `passed`, `failed`, `stopped`), `url`, `exitCode`, `error` |
| `POST /api/runs/<id>/stop` · `/restart` | as named |

Poll a run until `state` is `ready` (a web run answers on its port) or `failed`, and a test until `passed` or `failed`.

Clean up what you start outside PDT's runs (a throwaway server, a scratch folder) with commands your agent can judge:
stop a server by its port, and delete literal paths, or use the script the project's instructions name. Never
`rm -rf` a path built from `$(…)` or a variable: the agent can't tell what it deletes, so it stops to ask the user
for approval even when it may otherwise run anything, and an agent step waits until it times out.

## Project settings on this host

`GET /api/projects/<id>/settings` returns `agentCredentials`, `runOverrides` (TOML), `agentInstructions` (Markdown),
`mergeViaPr` and `agentPermissionMode` (the mode of workflow steps' sessions here, over their persona's).
`PUT` the same object to change them: `agentCredentials` and `runOverrides` are always replaced, so send back what you
read; `agentInstructions`, `mergeViaPr` and `agentPermissionMode` are replaced when present (`""` clears it). New agent sessions get the new instructions;
running ones keep what they started with. Change these only when the person asks.
