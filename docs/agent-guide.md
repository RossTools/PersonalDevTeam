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
except the PM's `create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets` and `send_to_session`, which the
person confirms per call unless the project lets the PM act. Use them for what they cover; the API above stays available for everything else.

| Tool | Does |
|---|---|
| `get_ticket` | this session's ticket (or another, by key): description, stage, links, attempts, comments, the outcomes earlier workflow steps submitted, and in a workflow step, the outcomes it allows |
| `submit_outcome` | `{outcome, summary, comments?}`: ends this session's workflow step with one of its outcomes; `comments` (`{file, line, endLine?, body}`) become line comments on the changes |
| `ask_user` | `{questions: [{question, choices?}]}`: puts questions in the person's Inbox; the answers come back as a message, so end your turn after asking |
| `add_ticket_comment` | `{body, ticket?}`: a comment on the ticket's timeline, by the agent |
| `add_review_comment` | `{file, line, endLine?, body}`: a line comment on the attempt's changes |
| `update_ticket` | `{title?, description?}`: rewrites this session's own ticket (the Refiner's tool; a session gets it when its persona lists it) |
| `catalog_search`, `catalog_get`, `workflow_get`, `workflow_validate`, `workflow_dry_run`, `workflow_save` | the workflow catalog and documents, for sessions whose persona has them (the catalog includes each MCP server's tools as `mcp:<server>/<tool>`, used as an `mcp.tool` node); a change to a workflow is a proposal the person accepts, and a new workflow starts disabled |
| `get_overview`, `search_tickets`, `get_session`, `list_workflow_runs` | the PM's view of every project: open tickets per stage, tickets in progress with PR and CI, the Inbox, running sessions; ticket search; a session's state and last messages; workflow runs and their steps |
| `search_notes`, `save_note`, `delete_note`, `write_journal` | the PM's memory: notes (host-wide or per project; pinned ones go in every PM briefing) and the journal entry a PM session writes as it ends |
| `list_todos` | `{project?, done?}`: the person's own todo list (title, done, project, linked ticket, due date `dueOn` and time `dueTime`, repeat rule `recurrence`, dates), read-only |
| `create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `send_to_session` | the PM's changes: a new ticket (a draft by default), an edit to any ticket (also its parent), a link between two tickets (`blocks`, `blockedBy`, `relates`) or its removal, a message to a session; each waits for the person's yes unless the project lets the PM act |

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
the failure as a workflow step instead: fix it, push, and finish with `submit_outcome` (PDT turns auto-merge back on). A session started on a ticket is told in its instructions
which messages to expect, how to act on each, and, when PDT watches the pull request for it, not to watch CI in a
shell. Workflows are changed only when the person asks, and then as a **proposal**: `PUT /api/workflows/<id>` with
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
| `GET /api/workflow-templates` | templates (`params`, a `doc` with `{{params.<id>}}`); `POST …/<id>/instantiate {params, name, scope, projectId}` makes a workflow from one, off |
| `GET /api/personas` | the personas agent steps run (Refiner, Coder, Reviewer, QA, PM and the person's own): agent, model, instructions, permission mode, tools |
| `GET /api/inbox` | what waits on the person: questions, approvals, stalled steps, loop limits and gates |

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
| `POST /api/projects/<id>/runs` | `{name, kind: "run" \| "test", worktreeId?}`; starts it (main checkout without `worktreeId`) |
| `GET /api/runs/<id>` | `state` (`setting_up`, `starting`, `ready`, `running`, `passed`, `failed`, `stopped`), `url`, `exitCode`, `error` |
| `POST /api/runs/<id>/stop` · `/restart` | as named |

Poll a run until `state` is `ready` (a web run answers on its port) or `failed`, and a test until `passed` or `failed`.

## Project settings on this host

`GET /api/projects/<id>/settings` returns `agentCredentials`, `runOverrides` (TOML), `agentInstructions` (Markdown),
`mergeViaPr` and `agentPermissionMode` (the mode of workflow steps' sessions here, over their persona's).
`PUT` the same object to change them: `agentCredentials` and `runOverrides` are always replaced, so send back what you
read; `agentInstructions`, `mergeViaPr` and `agentPermissionMode` are replaced when present (`""` clears it). New agent sessions get the new instructions;
running ones keep what they started with. Change these only when the person asks.
