# Working in a PDT project (guide for agents)

How an agent session started by PDT finds PDT and uses its tickets, worktrees, runs and tests. How *this* project
wants you to work (branches, checks, reviews) is in the repository's own instructions and in the project's **agent
instructions**, which PDT adds to every agent session it starts there.

## Finding PDT

PDT's instructions for this session name its API, `http://127.0.0.1:<port>/api`, the session id and the project's
id. Terminal sessions also have `PDT_HOOK_PORT` (the port) and `PDT_SESSION_ID`. The API is local, takes and returns
JSON (send bodies with `Content-Type: application/json`), needs no authentication, and reports errors as
`{code, message}`.

| Call | Gives |
|---|---|
| `GET /api/sessions/<session id>` | this session: `projectId`, `worktreeId` (null in the main checkout), `cwd`, `ticket` |
| `GET /api/projects` | projects: `id`, `name`, `key`, `kind`, `repoPath`, `baseBranch` |
| `GET /api/projects/<id>/worktrees` | the project's worktrees: `id`, `branch`, `path` |
| `GET /api/guides` | PDT's guides: this one, `run-config` (how runs and tests are configured), and the prompts `set-up-project` and `set-up-workflow` |

## PDT's tools (pdt-mcp)

Most sessions PDT starts also get PDT's MCP server, `pdt` (in Claude: `mcp__pdt__<tool>`). Its tools know which
session is calling, so they need no session, ticket or attempt ids; each tool's description has the details. Use them
for what they cover, and the API for the rest. Which tools a session has depends on its persona.

| Tool | Does |
|---|---|
| `get_ticket` | this session's ticket (or another, by key): description, stage, links, attempts, comments and history, `attachments` (each with a local `path`), `documents` (each `{id, title, kind, provider, url, read}`; see "Documents"), earlier steps' outcomes, and in a workflow step the outcomes it allows |
| `submit_outcome` | `{outcome, summary, comments?, data?}`: ends this session's workflow step; `comments` (`{file, line, endLine?, body}`) become line comments on the changes, `data` holds fields the step's prompt asks for. `submitted` is refused while the worktree has uncommitted changes to tracked files, or commits or a branch not pushed: commit and push first |
| `ask_user` | `{questions: [{question, context?, choices?, recommended?, why?}]}`: questions in the person's Inbox; the answers come back as a message, so end your turn. `question` is one line (at most 120 characters); the background goes in `context` (Markdown), each choice is `{label, description?}` (the description in Markdown: what it means, its pros and cons), and `recommended` is the label of the choice you recommend, pre-selected for the person, with `why` in one line |
| `add_ticket_comment` | `{body, ticket?, files?}`: a comment on the ticket's timeline, by the agent, with files attached |
| `add_review_comment` | `{file, line, endLine?, body}`: a line comment on the attempt's changes |
| `update_ticket` | `{title?, description?}`: rewrites this session's own ticket |
| `list_secrets`, `request_secret`, `run_with_secrets`, `write_secret_file` | secrets (below) |
| `ci_runs`, `ci_failed_logs`, `change_request_view`, `change_request_diff`, `change_request_comment`, `change_request_open`, `change_request_merge`, `issue_view`, `issue_comment`, `issue_create` | the project's forge and CI (see "CI and pull requests") |
| `catalog_search`, `catalog_get`, `workflow_get`, `workflow_validate`, `workflow_dry_run`, `workflow_save`, `workflow_run`, `workflow_export`, `workflow_import` | workflows: the node catalog (each MCP server's tools are in it as `mcp:<server>/<tool>`), documents, runs and transfer. `workflow_save` proposes a new version for the person to accept, or saves a new workflow, which starts disabled |
| `get_overview`, `search_tickets`, `get_session`, `list_workflow_runs`, `list_todos` | the PM's view: every project's open tickets, work in flight with PR and CI, the Inbox and running sessions; ticket search; a session's state and last messages; workflow runs; the person's todo list (read-only) |
| `search_notes`, `save_note`, `delete_note`, `write_journal` | the PM's memory: notes (pinned ones go in every briefing) and each PM conversation's journal entry |
| `documents_list`, `document_get`, `doc_read`, `sheet_read`, `comments_list` | documents (see "Documents"): list them; one's details and open comments; a doc as Markdown (`heading` reads one section); a sheet's sheets, charts and cells (`sheet`, `range`); comments, fenced as outside input |
| `doc_edit`, `sheet_edit`, `document_create`, `document_link`, `comment_reply`, `comment_resolve`, `document_open_beside` | changes to documents: `{document, edits}` (`doc_edit` answers with a diff, `sheet_edit` with new charts' ids); a new doc or sheet, linked to the session's own ticket unless you name a ticket or project; a link (or `unlink`); a comment's reply or resolve; opening one in a tab beside this conversation, for the person to watch your edits land |
| `watch`, `unwatch`, `list_watches` | the PM's watches: `watch {ticket? \| run?, events?, once?}` sends this conversation a `[PDT] Watched …` message (once it is idle) when the target is `finished`, `failed`, gets an agent's `outcome` or a `comment`, or `needs_you` (default: finished, failed, needs_you). `once` (the default) ends the watch when the target finishes and refuses one already finished. Use it instead of polling |
| `create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `stop_ticket`, `send_to_session` | changes to tickets (a new one is a draft by default; moving one at work back to Draft, Ready or To do stops its work; Won't do also closes its pull request and removes its worktree and branch when nothing would be lost; `stop_ticket` cancels its runs and sessions and abandons its attempt) and a message to a session (delivered when it is idle) |
| `link_issue`, `unlink_issue` | a ticket's links to issues already in a tracker (GitHub, GitLab, Jira), by address or key (`owner/name#12`, `ABC-123`); Done leaves a linked issue alone unless `closeOnDone`. An address no tracker recognizes becomes a plain link (`label`). `create_ticket`'s `external` links them as it makes the ticket |

Tools that act beyond this session (`create_ticket`, `edit_ticket`, `link_tickets`, `unlink_tickets`, `link_issue`,
`unlink_issue`, `stop_ticket`, `send_to_session`, `workflow_run`, `workflow_import`, `issue_create`, and the document writes `doc_edit`, `sheet_edit`,
`document_create`, `document_link`, `comment_reply`, `comment_resolve`) wait for the person's yes (except the PM's, where the
project lets the PM act, on its tickets, sessions and documents; other personas always wait). In Claude that is its permission prompt. Elsewhere
the call waits while PDT asks the person (in the conversation, or in the Inbox from a terminal), then runs or tells you
they said no. Don't end your turn while it waits.

**Secrets.** Never ask the person to paste a secret into the chat, and never print, log or commit one. `list_secrets`
gives the names this session may use; `request_secret` `{name, purpose}` asks the person to type one into PDT's Inbox
(you get `stored: true`, never the value: end your turn after asking). Use them through `run_with_secrets`
`{command, env, cwd?, timeoutSecs?}` (an `env` value `secret:<name>` is the secret, and at least one must be; the
output comes back with values replaced by `[secret:<name>]`) or `write_secret_file` `{path, template}`
(`{{secret.<name>}}` filled in, at least one; nothing echoed; keep such files out of git). Both work only inside your
session's folder (never in `.git`, never through a symlink out of it), and the person allows each call unless you run
with permissions bypassed. For commands and files without a secret, use your own tools.

**Workflow steps.** A session that runs a step of a workflow (a **persona** such as the Reviewer) has a section "Your
role" in its instructions: it finishes the step with `submit_outcome`, never by what it writes, and asks with
`ask_user` when only the person can unblock it. A turn that ends with neither is nudged, then shown to the person as
stalled.

## Tickets

A project's tickets have keys `<project key>-<n>` (`APP-12`). Stages: `draft`, `ready` (refined, waiting in the
backlog), `todo` (picked for the board), `in_progress`, `done`, `wont_do`. Priority: `none`, `low`, `medium`,
`high`, `urgent`. Labels are lower-case without commas.

| Call | Does |
|---|---|
| `GET /api/tickets?project=<id>` | the project's tickets, ranked. Also `q` (words, or a key), `stage`, `priority`, `label` (comma-separated lists), `parent`, `blocked`, `sort` (`rank`, `priority`, `updated`, `created`), `limit`, `cursor`; returns `{items, nextCursor, total}` |
| `GET /api/tickets/<id or key>` | one ticket with its `description`, links and attempt |
| `POST /api/projects/<id>/tickets` | `{title, description?, stage?, priority?, labels?, areas?, parent?, reason?}`: a Draft (unless `stage`) at the top of its stage; with `parent`, a part after the parent's last part. Send your session id in an `X-Agent-Session` header, so the ticket shows who made it (your persona and the ticket you work on). From an agent it is usually a proposal (below): say why in `reason` |
| `PATCH /api/tickets/<id>` | any of `title`, `description`, `stage`, `priority`, `labels`, `areas` (lists replace the whole set), `parent` (a ticket id, `""` to detach), `waiting` |
| `GET /api/tickets/<id>/events` | the timeline, oldest first (`newestFirst=true`, `limit`, `cursor`; returns `{items, nextCursor}`). Comments have `kind: "comment"` and their text in `body` |
| `POST /api/tickets/<id>/comments` | `{body}`: a comment by the agent (not sent back to you) |
| `POST /api/tickets/<id>/links` | `{kind: "blocks" \| "relates", to: <other ticket id>}` (this ticket blocks `to`); `DELETE` with the same body removes it |

A session started on a ticket gets the rules for that work in its instructions: follow-up work goes in a new ticket,
and moving the ticket to `done` or removing its worktree is left to PDT (once the branch is merged) or the person.

A ticket an agent makes (`create_ticket` outside the PM, or the call above) is a **proposal** while the project's
"Ticket proposals" workflow is on: it is made in Draft without the `agentic` label, stays off the board, and waits in
the person's Inbox with your `reason` until they accept it (to To do or the backlog), mark it a duplicate or reject it.
You can't move it or label it `agentic` yourself. You hear the decision in your session if it is still running,
otherwise as a comment on the ticket you were working on. `create_ticket`'s result says `"proposal": true` when it is
one.

### Attachments

A ticket holds images (PNG, JPEG, GIF, WebP, up to 10 MB) and UTF-8 text files (`.txt`, `.log`, `.md`, `.json`,
`.csv`, `.yaml`, `.toml` …, up to 1 MB), at most 50. `get_ticket` lists them with a local `path`: open the ones that
matter (Claude Code with its Read tool, Codex with `view_image`). On a ticket from outside PDT a text file comes as
`text`, fenced as outside input.

To attach files, give absolute paths in `files` (`add_ticket_comment`, `create_ticket`, `edit_ticket`); `pasted:1` is
the newest image the person pasted into this conversation (`pasted:2` the one before). Files come from your
session's folder or the temporary folder (save screenshots there); anything else is refused, as are dotfiles
(`.env`), key and credential folders (`.ssh`, `.aws`, keychains) and PDT's own data, and a call with one bad file
attaches none.

| Call | Does |
|---|---|
| `GET /api/tickets/<id>/attachments` | the attachments, oldest first |
| `POST /api/tickets/<id>/attachments?name=<file name>` | the body is the file (its bytes decide the type); returns the attachment |
| `GET` · `DELETE /api/tickets/<id>/attachments/<aid>` | the file (text as `text/plain`) · removes it |

### Documents

Design docs, plans and spreadsheets live outside the repo as documents, linked to projects and tickets. `get_ticket`
lists the ticket's documents: read its design doc before you start. When you're asked for a spec, a plan or a design,
write it as a document linked to the ticket (`document_create`), never as a file in the repository. A doc reads as
Markdown, a sheet as its sheets of cells (each with its `value` and, if it has one, its `formula`) and charts. A
`document` is given by id or exact title.

Edits are a list of operations, applied in order; an operation the document's provider can't do fails with a message
saying so, and the rest after it don't run:

- Docs: `{op: "replace_section", heading, markdown}` (the body under a heading, up to the next one of its level),
  `{op: "insert_after_heading", heading, markdown}`, `{op: "append_text", markdown}`, `{op: "replace_string", find,
  replace, all?}` (the text must appear once unless `all`), `{op: "table_append_row", heading?, table?, cells}` and
  `{op: "table_set_cell", heading?, table?, row, column, value}` (the `table`th table from 1, or the first under
  `heading`; `row` counts body rows from 1, `column` is a header's text or a number), `{op: "replace_text", markdown}`
  (the whole document).
- Sheets: `{op: "set_cells", sheet, cell: "B3", values: [["1", "=A1*2"]]}`, `{op: "add_sheet", name}`,
  `{op: "rename_sheet", sheet, name}`, `{op: "delete_sheet", sheet}`, `{op: "format_range", sheet, range: "A1:C1",
  format: {bold?, italic?, underline?, color?, background?, size?, numberFormat?, horizontal?, vertical?, wrap?}}`,
  `{op: "conditional_format", sheet, range, when, format}`, `{op: "add_chart", sheet, id?, chart: {kind: "column" |
  "bar" | "line" | "area" | "pie" | "scatter", data: "A1:C10", title?, anchor?}}` (the data's first column is the
  categories, its first row the series' names), `{op: "update_chart", id, chart}`, `{op: "delete_chart", id}`,
  `{op: "sort", sheet, range, by: [{column: "B", descending?}], header?}`, `{op: "filter", sheet, range, column,
  when}`, `{op: "clear_filter", sheet}`, `{op: "set_borders", sheet, range, style?: "solid" | "thick" | "dashed" |
  "dotted" | "double" | "none", color?, inner?}`, `{op: "column_width", sheet, columns: "B:D", width}` (pixels),
  `{op: "freeze", sheet, rows?, columns?}`, `{op: "validate", sheet, range, rule, strict?}` (a `rule` is
  `{type: "one_of", values}`, `{type: "checkbox"}`, `{type: "date"}` or `{type: "matches", when}`). A `when` is `{type: "greater_than" | "less_than" | "equal_to" |
  "not_equal_to", value}`, `{type: "between", min, max}`, `{type: "text_contains", text}`, `{type: "empty"}`,
  `{type: "not_empty"}` or `{type: "formula", formula}`.

A `local` document is a file PDT keeps (a `.md`, or a `.xlsx` computed by a real spreadsheet engine: formats,
conditional formats, borders, column widths, frozen panes, sort and filter all work; `validate` doesn't yet, and
`sort`/`filter` with `range` `""` take the sheet's used cells). Change it through PDT, so a person with it open sees
your change at once; a sheet's list of edits is all or none.

| Call | Does |
|---|---|
| `GET /api/documents?ticket=<key>&project=<key>&kind=doc\|sheet&q=<words>&limit=<n>` | documents, the most recently changed first, with their `projects` and `tickets` |
| `GET /api/documents/<id>/content` | `{content, embedUrl}`: `content` is `{kind: "doc", markdown}` or `{kind: "sheet", sheets: [{name, rows, charts}]}` |
| `POST /api/documents/<id>/edits` | `{edits: [...]}` (the operations above): `{version, charts}` |
| `GET /api/documents/<id>/version` | `{version}`: it changes whenever the document does, edits made at the provider too |
| `GET /api/documents/<id>/charts` | a local sheet's charts, each `{sheet, chart, labels, series: [{name, values}]}` with the numbers it draws |
| `GET /api/documents/<id>/comments` | its comments with their replies and `anchor` (`{on: "heading", heading}`, `{on: "text", quote}` or `{on: "cell", sheet, range}`); `POST …/comments` `{body, anchor}` adds one, `POST …/comments/<cid>/replies` `{body}` replies, `POST …/comments/<cid>/resolve` resolves, `…/reopen` opens it again |
| `GET /api/documents/<id>/export?format=md\|xlsx\|csv[&sheet=]` | the document as a file |
| `POST /api/tickets/<id>/design-doc` | makes the ticket's design doc from the template, linked to it |
| `GET /api/sessions/<id>/documents` | the documents open beside a conversation: `{tabs: [{document, by}], active}`; `POST` `{documentId}` opens one (or shows its tab), `PUT` `{documentIds}` orders them, `DELETE …/documents/<doc>` closes one |

### Review feedback

The person reviews your branch in PDT's Changes view and sends line comments back as one message starting "Review
feedback on <branch>": each names `file:line`, quotes the lines and says what to change. Address each one, then say
briefly what you changed. `GET /api/worktrees/<id>/changes` lists what PDT counts as your changes (committed or not,
against the merge base with the base branch).

## Workflows and events

PDT reacts to events (CI results, pull request news, merges, comments) through workflows: a trigger, then
conditions, gates, waits and actions. Messages starting `[PDT]` in your session come from a workflow the person
enabled, such as the built-in "PR notices" (CI failed, a conflict, dropped from the merge queue, merged) or a user's
comment on your ticket; each says what happened and what to do. When the pull request was set to merge, a failure
comes as a workflow step instead: do what its prompt says and finish with `submit_outcome`. Your instructions say
which messages to expect and, when PDT watches the pull request for you, not to watch CI in a shell.

Change a workflow only when the person asks, and then as a **proposal**: `PUT /api/workflows/<id>` with
`{"doc": …, "note": "what and why", "propose": true}` (or `workflow_save`). It is validated but doesn't take effect
until the person accepts it in the Workflows view. Never turn a workflow on (`PATCH {"enabled": true}`): that is the
person's consent. Check a document with `/validate` and try it with `/dry-run` before proposing.

| Call | Gives |
|---|---|
| `GET /api/events?ticket=<id>` | what PDT logged about a ticket (`type`, `payload`), newest first |
| `GET /api/workflows?project=<id>` | the workflows that apply to the project, with `enabled` |
| `GET /api/workflows/catalog` | every trigger, condition and action, with its config schema and ports |
| `GET /api/workflow-runs?ticket=<id>` | the runs about a ticket; `GET /api/workflow-runs/<id>` has each step's input, output and port |
| `POST /api/workflows/validate` · `/dry-run` | `{doc}` checks a document; `{doc or workflowId, event: {type, payload}}` shows what each step would do, changing nothing |
| `GET /api/workflows/<id>/versions` · `/diff?from=&to=` | the versions (`status`: `saved`, `proposed`, `discarded`) and what changed between two |
| `GET /api/workflows/<id>/export` · `POST /api/workflows/export {ids}` | workflows as one export text (`format: "pdt-workflow"`; project ids become `{"$project": "<KEY>"}`; `requires` lists the connections, personas and projects they need), to import into another PDT |
| `POST /api/workflows/import` | `{text, check: true}` reports what each needed name is here (`found`, `missing`, `kind_mismatch`) and same-named workflows; `{text, mapping, targets}` imports, with `mapping: {connections, personas, projects}` (missing names to local ones, or `null`) and one `target` per workflow (`scope`/`projectId`, and on a name clash `copy: true` or `versionOf: <id>`). Imported workflows start off |
| `GET /api/workflow-templates` | templates (`params`, a `doc` with `{{params.<id>}}`); `POST …/<id>/instantiate {params, name?, scope?, projectId?}` makes a workflow from one, off |
| `GET /api/personas` | the personas agent steps run (Refiner, Coder, Reviewer, QA, Triage, PR Reviewer, PM and the person's own): agent, model, instructions, permission mode, tools |
| `GET /api/inbox` | what waits on the person: questions, approvals, gates, manual steps, stalled and timed-out steps, loop limits |

### CI and pull requests

Use PDT's forge tools rather than a forge CLI; they act as the person's forge login in PDT (you still push with plain
`git push`):
- `ci_runs`: your branch's recent runs from each of the project's CI providers, with the failed jobs.
  `ci_failed_logs`: the end of each failed job's log (give `provider` and `runId` from `ci_runs` or the failure
  message, or nothing for the newest failure). The ticket's attempt has the same in `pr.ci` (or `ci` without a pull
  request).
- `change_request_open`: opens the pull request for your pushed branch (`title`, `body`, `base?`, `draft?`).
  `change_request_view`: it, with comments and reviews. `change_request_comment`: answers on it (`replyTo` answers a
  review comment in its thread). `change_request_diff`: a pull request's diff by number.
- Re-running CI: re-run only the newest run on the pull request's head commit (match its commit to the head), or
  push. Re-running an older commit's jobs cancels the head's run when CI keeps one run per branch, and the pull
  request then waits on nothing. PDT re-runs a head whose CI was cancelled once by itself, then puts the ticket in the
  Inbox as stalled.
- `change_request_merge`: asks for the merge again after it dropped out (the merge queue, failed CI) and you pushed
  the fix; only for a pull request that was already set to merge. The project's merge policy applies.
- `issue_view`, `issue_comment`: read and comment on issues. `issue_create`: files one (with `ticket`, from that
  ticket, which then links it).

## Worktrees

A project with `kind: "personal"` has no repository: its tickets run in a folder of their own, with no branch,
worktree, runs or pull request.

PDT keeps each branch's checkout at `<repo>/.worktrees/<branch>`, with `/` in the branch name replaced by `-` (listed
in `.git/info/exclude`). Make one through PDT so its sessions and runs know about it:
`POST /api/projects/<id>/worktrees` with `{"branch": "fix/login", "base": "main"}` (`base` defaults to the project's
base branch) makes the branch and folder and returns the worktree (`id`, `path`); then `cd <path>`. A worktree already
made there with `git worktree add` on that branch is taken over instead.

## Runs and tests

A project's runs (dev servers, apps) and tests come from `.pdt/project.toml` in the checkout and from PDT's host
overrides, which keep the whole configuration out of the repository. `GET /api/guides/run-config` is the reference.

| Call | Does |
|---|---|
| `GET /api/projects/<id>/run-config?worktreeId=<id>` | the configuration: `file`, `overrides`, the merged `effective` (`runs`, `tests`, `areas`), or `error` |
| `POST /api/projects/<id>/runs` | `{name, kind: "run" \| "test", worktreeId?}`: starts it (in the main checkout without `worktreeId`); the same run finished earlier in that checkout restarts (200) instead of adding one. A command the user hasn't approved is `409 untrusted_commands`: you can't approve it; ask the user to start the run in PDT |
| `GET /api/runs/<id>` | `state` (`setting_up`, `starting`, `ready`, `running`, `passed`, `failed`, `stopped`), `url`, `exitCode`, `error` |
| `POST /api/runs/<id>/stop` · `/restart` | as named |

Poll a run until `state` is `ready` (a web run answers on its port) or `failed`, and a test until `passed` or
`failed`.

Clean up what you start outside PDT's runs (a throwaway server, a scratch folder) with commands your agent can judge:
stop a server by its port, and delete literal paths. Never `rm -rf` a path built from `$(…)` or a variable: the agent
can't tell what it deletes, so it stops for the person's approval even when it may otherwise run anything.

## Project settings on this host

`GET /api/projects/<id>/settings` returns `agentCredentials`, `runOverrides` (TOML), `agentInstructions` (Markdown),
`mergeViaPr`, `agentPermissionMode` (the mode of workflow steps' sessions here, over their persona's) and more.
`PUT /api/projects/<id>/settings` changes them: `agentCredentials` and `runOverrides` are always replaced, so send
back what you read; `agentInstructions`, `mergeViaPr` and `agentPermissionMode` only when present (`""` clears).
New agent sessions get new instructions; running ones keep theirs. Change these only when the person asks.
