# Personal Dev Team — The project manager (PM)

Status: design for PDT-81, 2026-10-21. Builds vision §9 and §13 (docs/superpowers/specs/2026-10-20-workflows-and-pm-vision.md)
on Phase 7b's pdt-mcp and personas (docs/superpowers/specs/2026-10-20-phase-7b-agent-steps-design.md).

## 1. What it delivers

- **The PM**: a host-level conversation the user talks to from its own rail item, about every project: triage, "what's
  in flight?", "why is PDT-52 stuck?", drafting tickets and workflows.
- **PM tools** in pdt-mcp: search and read tickets, the overview (board, attempts in flight with PR and CI, inbox,
  running sessions), a session's state, workflow runs, notes and the journal; and the acting tools (create and edit
  tickets, message a session), which **ask first** unless the project's policy lets the PM act.
- **Memory owned by PDT**: a briefing pdtd writes into every new PM session's instructions, a notes store (host-wide
  and per project, SQLite FTS5), and a journal entry per conversation (checkpoints on compaction and daily, the
  last one when the user closes it) that later briefings carry.
- **The PM persona**, a built-in like the 7b ones, so a workflow's `agent` step can run it (triage).

Done when the user can ask the PM about any project's state and get an answer grounded in PDT's data, have it draft a
ticket that it creates once the user confirms, and a new PM session picks up where the last one left off from the
briefing and notes.

## 2. Where the PM lives

Sessions belong to a project (`sessions.project_id`, also every session query and the web's session types). Rather
than make that optional, PDT keeps one **host project** for its own sessions:

- `ProjectKind::Host` (`kind = 'host'`), made by `Store::open` when missing: name "PDT", key `_HOST` (no user key can
  have `_`), folder `<data dir>/pm` (no git), no workspace.
- `list_projects` leaves it out, so the project list, board, scheduler, PR poller, merge detection and workflows never
  see it; `get_project` and `host_project()` find it. Everything that needs git already refuses a project that is not
  a repo (`Project::has_repo`, which Personal projects use since 7b).
- Its sessions are PM sessions: conversations of the PM persona, with no tab (the PM view shows them).

## 3. PM sessions

- `GET /api/pm` → `{current, sessions, rotateAt}`: the current PM session (the newest one in the host project that is
  not ended), and the earlier ones, newest first, each with its journal entry. Ended sessions stay readable.
- `POST /api/pm/sessions {text?}` starts a new PM session (ending the current one, which writes no journal) with
  `text` as its first message. The web sends the first message of a new conversation this way.
- A PM session is a **conversation** of the PM persona's current version: its agent and model, permission mode `ask`
  unless the persona sets one (never the project override: the host project has none), in the work folder (§10; it
  was `<data dir>/pm` before PDT-100).
- **Tools.** It gets pdt-mcp with the PM tool set. PDT-81 also gave Claude `--tools ""` (no built-in tools); PDT-100
  dropped that: the PM has its agent's own tools too (§10).
- **Instructions** (`pdt_core::pm::instructions`): who the PM is and what it may do, the persona's instructions, and
  the briefing (§5) made when the session was created and kept in `<session dir>/briefing.md`, so a restart or resume
  sends the same text.

## 4. Tools

| Tool | Input | Does | Asks? |
|---|---|---|---|
| `get_overview` | `project?` | per project (or one): tickets per board stage (counts) and the open ones in flight (in progress, in review) with attempt, PR and CI; open Inbox items; running agent sessions with their activity | no |
| `search_tickets` | `query?`, `project?`, `stage?`, `limit?` (20) | ticket summaries matching the words (as `GET /api/tickets?q=`) | no |
| `get_ticket` | `ticket?` | (7b) | no |
| `get_session` | `session` | a session's state (agent, status, activity, ticket, persona, step), its open requests and the last messages of its conversation | no |
| `list_workflow_runs` | `workflow?`, `ticket?`, `status?`, `limit?` | runs with their steps' status and errors | no |
| `search_notes` | `query?`, `project?`, `pinned?`, `limit?` | notes and journal entries matching (FTS5), newest first; every pinned note when `query` is empty | no |
| `save_note` | `id?`, `project?`, `title`, `body`, `pinned?` | adds or replaces a note | no |
| `delete_note` | `id` | removes a note | no |
| `write_journal` | `body` | the session's journal entry (§5.3) | no |
| `create_ticket` | `project`, `title`, `description?`, `stage?` (`draft`), `priority?`, `labels?`, `parent?` | `POST /api/tickets`, recorded as the agent's | **yes** |
| `edit_ticket` | `ticket`, `title?`, `description?`, `stage?`, `priority?`, `labels?` | `PATCH /api/tickets/{id}` as the agent's | **yes** |
| `send_to_session` | `session`, `text` | queues a message through the outbox (7a's delivery rule; "[PDT] From the project manager: …") | **yes** |

Plus 7b's `add_ticket_comment`, `ask_user`, `submit_outcome`, the catalog and workflow tools. `workflow_save` already
only proposes a version or saves a disabled workflow, so enabling stays the user's. Connections (Radarr, Sonarr, …)
come with their catalog work (PDT-86).

**Asking.** The asking tools (`ASKING_TOOLS` in pdt-core) are left out of Claude's `--allowedTools`, so Claude asks
through its permission prompt and the user sees PDT's approval card with the tool's arguments (the drafted ticket).
A project's **PM policy** (`project_settings.pm_acts`: `ask`, default, or `act`) loosens it: when an approval request
opens for an asking tool whose target is in a project set to `act` (`create_ticket`'s `project`, `edit_ticket`'s
ticket, `send_to_session`'s session), PDT answers it itself. Agents with no per-tool prompt for MCP tools (Codex,
Devin) get the asking tools refused under `ask`, with a message saying so. A persona in permission mode `bypass`
never asks: that is the user's choice.

## 5. Memory

### 5.1 Briefing

Built by pdtd (pdt-api `pm.rs` gathers, pdt-core `pm::briefing` renders Markdown), never by the model:

- **Projects**: name, key, kind; ticket counts per stage.
- **In flight**: tickets in progress or in review with branch, PR state and CI, and whether they wait on the user.
- **Inbox**: open items (kind, ticket, title).
- **Since the last session**: the event log since the previous PM session started (by type, with the latest few
  lines: tickets moved, attempts merged, CI failed, runs failed), capped.
- **Journal**: the last few journal entries.
- **Pinned notes**: the index (id, project, title); `search_notes` reads them.

Sections are capped (a few dozen lines each) so the briefing stays small.

### 5.2 Notes

`pm_notes`: id, `project_id` (null: host-wide), `kind` (`note` | `journal`), `title`, `body`, `pinned`,
`session_id` (journal: the session that wrote it), timestamps; `pm_notes_fts` is an FTS5 table over title and body
kept in step by triggers. API: `GET /api/pm/notes?q=&project=&kind=`, `POST /api/pm/notes`,
`PATCH /api/pm/notes/{id}`, `DELETE /api/pm/notes/{id}`; the PM view lists, searches, edits, pins and deletes them.

### 5.3 The journal, compaction and closing (PDT-167)

- A PM conversation never ends on its own: Claude Code and Codex compact their own context, so a full context or a
  long life (it used to end at 60 % context or 24 h) is no reason to throw away a working conversation.
- **Journal checkpoints.** When the agent compacts its context (a `compaction` event), PDT queues "[PDT] Your context
  was just compacted. Write a journal checkpoint with `write_journal`… this conversation carries on." through the
  outbox, so it never goes into a running turn (it waits for the turn to end) and never wakes a parked conversation.
  `write_journal` replaces the conversation's one entry each time, so the entry is always the latest picture.
- **Daily update.** When a turn ends and at least `UPDATE_EVERY_MS` (24 h) has passed since the conversation was last
  brought up to date (its briefing, then each update: `pm_sessions.caught_up_at`), and the user wrote to it since,
  PDT queues "[PDT] Since your briefing (… ago)…" (later "Since the last update"): the log's events since then, the
  Inbox, and other conversations' journal entries written since then, ending with a request for a journal checkpoint.
  The turn answering it asks for no other. One checkpoint request or update waits at a time per conversation.
  PDT-211 trims it: no `schedule.fired` or overlap events, one line per ticket (key, title, what happened; at most 20)
  under the counts line, and only each journal entry's title, age and "Still open" section (at most 15 lines). With no
  ticket events, an empty Inbox and no new journal entry it sends only a short checkpoint request, or nothing when the
  conversation's journal is newer than the user's last message. The update, checkpoint and closing requests show in
  the conversation as one collapsed line (`user_message.pdt`) that opens to the full text.
- **Closing.** Only the user ends a conversation (`DELETE /api/pm/sessions/{id}`). PDT drops a waiting checkpoint or
  update and sends "[PDT] The user closed this PM conversation, so it is ending. Write its journal entry with
  `write_journal`… Then end your turn." (`pm_sessions.rotation = 'journal'`, reason `user`). When that turn ends
  (journal written or not), PDT ends the session (`rotation = 'done'`, stopped; a missing journal is recorded as
  "(no journal)").
- **Resetting** (PDT-308, `POST /api/pm/sessions/{id}/reset`). The same as closing, with reason `reset` and the request
  "[PDT] The user is resetting this PM conversation…", but the call waits (up to `RESET_WAIT_MS`, 3 min) for the
  conversation to end; on time-out it drops the request and ends it with "(no journal)". It then starts a fresh
  conversation with the old one's title, agent, model, permission mode, persona, folder and `via`, its row
  `reset_from` the old one, and returns it. Its briefing (written to `briefing.md` before the session starts) leads
  with the old journal under "This conversation before it was reset". `listed_at` keeps both in place in the list: the
  old one while it writes its journal, the new one until it has activity of its own. The reset runs in a task of its
  own (a caller that goes away doesn't stop it), and the old one counts as open while that task runs
  (`AppState::pm_resets`), so it never leaves the list before its successor is in it. One whose reset never finished
  (pdtd stopped mid-way) is an ordinary ended conversation. A reset conversation's journal turn never marks it unread,
  and a reset one with a successor is left out of the ended list (its successor links to it).
- The ended conversation shows, in place of its composer, why it ended (`reason`; `context` and `age` are kept for
  conversations that ended before PDT-167), its journal, and **New conversation**.
- `pm_sessions`: session id, `rotation` (`none` | `journal` | `done`), `reason`, `caught_up_at`, timestamps.

## 6. As a workflow step

The built-in **PM** persona (Claude, permission `ask`, the PM tool set plus `submit_outcome`) can be an `agent` node's
persona. A step session runs in the step's project as any persona does (7b §4.2); asking tools still ask (the approval
waits in the Inbox's Approvals). The built-in template **Triage new GitHub issues** (`builtin:issue-triage`; params
repo, project, persona) runs it on PDT-80's `github.issue.opened`: outcomes `drafted` (a draft ticket made with
`create_ticket`), `duplicate`, `ignored`; `ask_user` when only the user can decide.

## 7. Web

- Rail item **PM** (`Ctrl+] m`, not on the phone rail: the palette's Go to has it). List column: **Current**, earlier
  sessions (date, reason, journal), **Notes**. Work area: the selected session's conversation (`ConversationView`);
  with no current session, a start page with a composer ("Ask the PM…") that starts one.
- Notes page: search, project filter, a list (pinned first), and a form to add, edit, pin or delete.
- Project settings gain **PM may act without asking** (`pmActs`).

## 8. Code layout

- pdt-core `pm.rs` (briefing and update renderers, instructions, the update rule, tool sets), `persona.rs` (PM built-in, new tools,
  `ASKING_TOOLS`), `project.rs` (`Host`), `policy.rs` (`PmActs`).
- pdt-store migration `0026_pm.sql` (`pm_notes`, `pm_notes_fts`, `pm_sessions`, `project_settings.pm_acts`);
  `pm.rs`.
- pdt-api `pm.rs` (routes, sessions, briefing data, checkpoints and updates, auto-answer), `mcp/tools.rs` (the new tools);
  pdt-agents `mcp.rs` (asking tools out of `--allowedTools`; PDT-100: the PM's Codex settings).
- web `pm/` (view, list, notes), rail, palette, project settings.

## 9. Out of scope

Connections as PM tools (PDT-86), Discord server management (PDT-98), talking to the PM over Discord, limits on
non-Claude agents' own tools (PDT-90).

## 10. The PM as a TPM (PDT-100, 2026-10-22)

The PM reads code, researches, writes light designs and helps with setup, like a technical project manager.

- **Work folder.** A new PM session runs in the deepest folder that holds every project's folder (each checkout's
  parent) and the Projects folder (Host settings' `projects_dir`; its default `~/work` counts only while there are no
  projects): pdt-core `pm::work_folder`. When they share nothing below the file system's root it is the home folder,
  else the host project's `<data dir>/pm`. No setting: add a Projects folder to widen it. The PM's own files (briefing,
  instructions, MCP config) stay in its session folder; the host project's folder is unchanged.
- **Its agent's own tools**, with no extra limits and the agent's usual permission prompts (`ask` by default): Claude
  keeps its built-in tools (Read/Grep/Glob, Bash, Edit/Write, WebSearch/WebFetch, subagents); Devin keeps its own.
  Codex has shell and file tools already and also gets live web search (`web_search = "live"`) and the network inside
  its workspace-write sandbox (`sandbox_workspace_write.network_access`, and `networkAccess` on each turn's
  `sandboxPolicy`), so installs and fetches work; writes outside the work folder ask as usual.
- **Instructions.** `pm::instructions` names the work folder, what the agent's own tools are for (reading checkouts
  and worktrees, `git diff <base>...<branch>` for an attempt, the web, commands, installs, files) and PDT's API and
  guides; the briefing gives each project's folder. The persona's instructions add how to use them: read the code
  before answering or estimating, link sources, put designs on the ticket, set up `.pdt/project.toml` from the
  run-config guide, and leave code changes to coding agents (draft a ticket instead).
- **Reaching edited personas.** Before PDT-100 any saved version (even a model change) froze a built-in persona on
  the template it had. Now each field the user left as PDT's last template follows newer ones and each field they
  changed stays theirs, and the persona page can load PDT's current template into the form
  (docs/reference/workflows.md "Personas"). Edited PM personas get the new instructions without the user doing
  anything unless they changed the instructions themselves.
