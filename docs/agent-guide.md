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
| `POST /api/tickets/<id>/comments` | `{body}` |
| `POST /api/tickets/<id>/links` | `{kind: "blocks" \| "relates", to: <other ticket id>}` (this ticket blocks `to`) |

The person reviews your branch in PDT's Changes view and sends line comments back as one message that starts
"Review feedback on <branch>": each comment names `file:line`, quotes the lines and says what to change. Address each
one, then say briefly what you changed. A message saying a merge into the base conflicts lists the files: merge the base
branch into yours, resolve them, run the checks and commit. `GET /api/worktrees/<id>/changes` lists what PDT counts
as your changes (committed or not, against the merge base with the base branch).

When work you were given as a ticket turns up follow-up work, add it as a new ticket rather than widening the change,
and comment on the ticket with anything the person should know. Move a ticket to `done` only when the person says the
work is finished (PDT also does it when it sees the branch merged).

## Workflows and events

PDT reacts to what happens (CI results, pull request news, merges, comments) through workflows: a trigger on an
event, then conditions, gates, waits and actions. Messages that start with `[PDT]` in your session come from one,
for example the built-in "PR notices" (CI failed, a conflict, dropped from the merge queue, merged). Workflows are
changed only when the person asks.

| Call | Gives |
|---|---|
| `GET /api/events?ticket=<id>` | what PDT logged about a ticket (`type`, `payload`), newest first |
| `GET /api/workflows?project=<id>` | the workflows that apply to the project, with `enabled` |
| `GET /api/workflows/catalog` | every trigger, condition and action, with its config schema and ports |
| `GET /api/workflow-runs?ticket=<id>` | the runs about a ticket; `GET /api/workflow-runs/<id>` has each step's input, output and port |
| `POST /api/workflows/validate` · `/dry-run` | `{doc}` checks a document; `{doc or workflowId, event: {type, payload}}` shows what each step would do, changing nothing |

## Worktrees

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

`GET /api/projects/<id>/settings` returns `agentCredentials`, `runOverrides` (TOML), `agentInstructions` (Markdown) and `mergeViaPr`.
`PUT` the same object to change them: `agentCredentials` and `runOverrides` are always replaced, so send back what you
read; `agentInstructions` and `mergeViaPr` are replaced when present (`""` clears it). New agent sessions get the new instructions;
running ones keep what they started with. Change these only when the person asks.
