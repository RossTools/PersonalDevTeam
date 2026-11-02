# How PDT is built

The developer guide: what a contributor (a person or a coding agent) needs to find their way around PDT's code. It
covers the processes, the crates, the core model and where each area lives. The rules that are easy to break are in
[invariants.md](invariants.md), the testing rules in [testing.md](testing.md), and how a change goes from branch to
release in [SDLC.md](../SDLC.md). What PDT does for its users is in the user docs; this guide doesn't repeat it.
Write this guide about how the code works today and the rules it keeps, without ticket history or decision logs;
the rules for every kind of doc are in AGENTS.md ("Writing docs"). Designs for new work go on its ticket.

PDT is single-user today, but nothing in it should stop several users, hosting it as a service, or running it from
any machine: one web UI serves the browser, the tray and the phone; a Rust daemon owns all the state and work; the
code is split into crates so a feature lands in one place.

## Processes

```
pdt-host ──starts, watches, updates──▶ pdtd ◀──HTTP + WebSocket── web UI (browser, tray, phone over Tailscale)
                                         ▲
                     agents' hooks ──────┤ (`pdtd hook …`)
                     agents' MCP calls ──┘ (`/api/mcp`)
```

- **pdtd** (`crates/pdtd`) is the daemon and does all the work: one axum server (routes in pdt-api) on
  `127.0.0.1:<port>`, a SQLite database, the agents' terminals and conversations, and the background loops (the
  PR/merge poll every 60 s, the workflow engine every 2 s, overlaps every 30 s, the pickup watch every 10 s,
  notifications, update checks, remote access). A loop that panics ends only that pass. It also serves the built web UI, from
  `PDT_WEB_DIR`, a `web/` folder beside the executable, or the macOS bundle's resources.
- **pdt-host** (`crates/pdt-host`) supervises pdtd: starts it (passing the port from Settings), restarts it, adopts a
  pdtd that is already running, and installs updates: pdtd downloads and verifies a release, pdt-host copies the
  database aside, swaps the install, health-checks the new pdtd and rolls back (database included; on Windows also the
  installer's registry keys, `nsis_registry.rs`) when it fails. It
  is registered to start at login: a LaunchAgent on macOS, an HKCU Run value on Windows, a systemd user unit on Linux.
- **The tray app** (`desktop/src-tauri`, Tauri v2) is optional and has no window of its own: it starts pdt-host when
  pdtd doesn't answer, opens the UI in the browser and can restart the daemon. Quitting it leaves PDT running.
- **The web UI** (`web/`, React + Vite) talks only to pdtd's `/api`: JSON over HTTP, polled every 2 s, plus
  WebSockets for terminals and conversation events and an SSE stream for notifications. It reloads itself when pdtd
  reports a new build.
- **Agents** (Claude Code, Codex, Devin, ACP agents) are child processes of pdtd. They report activity through hooks
  (`pdtd hook …`, which posts to the pdtd that launched them) and call PDT through pdt-mcp.
- **Remote access** is `tailscale serve` in front of pdtd (pdt-api `remote.rs`, `crates/pdt-tailscale`); requests from
  the tailnet must carry the owner's Tailscale identity.

**Instances.** A profile picks the defaults: `stable` (port 7420, data folder `<local data dir>/pdt`) and `dev` (7421,
`pdt-dev`; debug builds default to it). `PDT_PORT` and `PDT_DATA_DIR` override them, and a lock file stops two pdtd
sharing a data folder. Environment variables are read in `crates/pdtd/src/config.rs`. `GET /api/health` reports
name, version, profile, port and build. The data folder holds `pdt.db`, `logs/`, `sessions/<id>/` (per-session files:
instructions, agent settings, MCP config, `attachments/` for images pasted into a conversation), `tickets/<id>/attachments/`
(a ticket's screenshots and text files, metadata in `ticket_attachments`; pdt-api `ticket_attachments.rs`, removed
with the ticket by `prune`), `network.json` (port and allowed hosts, read before the database opens)
and `backups/` (a copy taken before migrations).

**Releases.** The version is the workspace version in `Cargo.toml` (repeated in `tauri.conf.json`); a build without
`PDT_BUILD_VERSION` reports `<version>-local`. Only a supervised stable pdtd updates; a dev instance never does.

## Crates

| Crate | Its job |
| --- | --- |
| `pdt-core` | The domain: types, rules and pure functions, no I/O. Tickets, ranks, parts, policies, overlap, PR and CI state, the workflow model (documents, references, validation, dry runs), agent instructions (`agent_context`), launch command lines, conversation frame mapping, secrets' redaction, brand constants (`brand.rs`). Most unit tests live here. |
| `pdt-store` | SQLite through sqlx: one module per table group, migrations in `migrations/`, data backfills in `Store::open`, failed writes and recovering from them in `health.rs`. |
| `pdt-git` | The git CLI: worktrees, branches, diffs, merge detection, local merges, pulls. |
| `pdt-agents` | Running agents: PTYs (`pty.rs`), detached runs (`detached.rs`), launching (`launch.rs`), hooks, instruction files, MCP wiring, and the adapters in `adapter/` (Claude, Codex, Devin, ACP). |
| `pdt-api` | The axum routes and everything that ties the crates together: `routes.rs` (`AppState`, and the router merging each module's routes), one module per area, the background loops (`background.rs`), the workflow engine and nodes (`workflow/`), pdt-mcp (`mcp/`), the integration SDK (`sdk.rs`, `integrations/`). Test fakes in `src/bin` (`fake-agent`, `fake-mcp`, `fake-command`). |
| `pdt-forge` | The `Forge` trait (change requests, CI, merges, reviews, issues, with declared capabilities), `CiProvider`, the login store, the forges' shared HTTP `transport` (reqwest, `Offline`, recorded answers), and `FakeForge` (`testing.rs`). |
| `pdt-github` | The GitHub forge over GitHub's REST and GraphQL APIs (nothing runs `gh`): pull requests, Actions, merges, issues, the OAuth device-flow login, rate limits. The `Forge` methods are in `forge.rs`. |
| `pdt-tailscale` | The `tailscale` CLI (`serve`, `status`), and `fake-tailscale` for tests. |
| `pdt-update` | The GitHub release client: finding a release, verified downloads. |
| `pdt-sheet` | Local sheets on IronCalc: `.xlsx` and `.csv` in and out, cells with computed values, formats, sheets, sort and filter, and IronCalc's sync diffs. |
| `pdt-host` | The supervisor: running pdtd, installing updates, rolling back, login registration. |
| `pdtd` | The daemon binary: config, logging, startup (`main.rs`: the state, the startup resume and the poll), `pdtd hook`, the demo seed (`seed.rs`), and which integrations it is built with (`src/integrations/`). |
| `pdt-int-*` | The optional integrations (discord, jira, buildkite, gitlab, google, arr, scryer, plex), one crate each, built on pdt-api's `sdk` and `Integration` trait. Each is a pdtd cargo feature (all on by default) and is enabled per host in Settings → Plugins. Outside their crates, only pdt-core `integration::KNOWN` and pdtd's features name them; the web shows what they declare: a connection kind's fields (switches, patterns, conditions), invite link (`link`), row summary (`summary`) and status line (`status`, `Integration::connection_status`, `GET /api/connections/{id}/status`), a project settings schema's import by key (`import`, `Integration::import_by_key`, `POST /api/projects/{id}/import/{integration}`), and each plugin's label. |

Errors from the API are JSON `{code, message}` (pdt-api `error.rs`); clients match on `code`. API types derive ts-rs
and are generated into `web/src/api/gen`.

## The model

**Projects.** A repository and its main checkout, with a short key (`PDT`) used in ticket keys. The base branch
defaults to the repository's default branch. The forge (GitHub, GitLab, the local forge, or none) comes from
`.pdt/project.toml` or from the plugin that claims `origin`'s host; all forge access goes through pdt-api
`forge::project_forge`. Two special kinds: `personal` (a folder with no repository: no worktrees or runs) and the
hidden `host` project (key `_HOST`), which holds the PM's sessions.

**Workspaces and tabs.** A workspace is a strip of tabs. Each project has one; Start makes one per ticket; others are
free-form. A tab holds exactly one session, run, ticket or worktree's changes, and each of those has at most one tab.

**Worktrees.** Each branch PDT works on gets a worktree at `<repo>/.worktrees/<branch, / replaced by ->`
(pdt-core `worktree.rs`). Deleting one is refused while a session, run or active attempt uses it, or while it has
uncommitted changes (unless forced). Storage (pdt-api `storage.rs`, rules in pdt-core `storage.rs`): the `storage`
loop (every 60 s) measures each worktree whose size is missing or 30 minutes old in a task of its own (one at a time;
`AppState::storage` keeps the sizes in memory, so a restart measures again), and hourly runs the automatic cleanup,
which cleans or removes finished tickets' worktrees per the project's `project_settings.storage`. A project's build
output is `[storage] build_output` from its main checkout's run configuration, else detected from the files at its
root (`detect_build_output`). Free space comes from `AppState::disk_space` (fs4, installed by pdtd; `AppState::new` reads none, tests set a fake) for the data
folder and each project's folder, grouped by device (`storage::disk_id`) with the least free space any read; below the host's `low_disk_gb` the Inbox lists a `low_disk` item, below
`critical_disk_gb` `ticket.start` starts nothing, the pickup watch tells nothing and `autoStartAt` is left out.

**Runs.** A run is a `[[run]]` or `[[test]]` from `.pdt/project.toml` (plus host overrides; the user docs' Run configuration page)
started in a checkout, on a port from the host's range. Runs live in PTYs, or detached (`detach = true`) so they
outlive pdtd and are reattached after a restart. On Windows each PTY session's processes are in a kill-on-close job
object only `PtyManager` holds (`pty/job.rs`), so the whole tree ends when pdtd does, however it ends, or when the
session is removed; a process that asks to break away (a detached run) may. When remote access changes the host, the runs (never tests) that use
`$PDT_RUN_HOST` or `$PDT_RUN_URL` restart (`run_manager::restart_host_dependent_runs`); any stop of the run meanwhile
cancels that restart (`AppState::run_stops`), and one that cannot start again fails the run with the reason.

**Tickets.** Numbered per project (`<KEY>-<n>`, never reused), in a stage (draft, ready, todo, in progress, done,
won't do), ordered by a fractional `rank`, linked (`blocks`, `relates`), with a timeline of events (comments are
events). A parent ticket's parts are tickets one level down; the parent is never started itself. Tickets from outside
(`source: external`) need the user's admission before agents work on them. `pipelineStage` is the Board column a
workflow step sets.

**Attempts.** One go at a ticket: its branch (`pdt-<KEY>-<n>-<slug>`), worktree, base branch, workspace and the merge
state. Start (pdt-api `start.rs`) makes the branch and worktree, the attempt and the first agent session, whose
prompt is pdt-core `start::start_prompt`. An attempt ends as merged or abandoned. Merge detection (pdt-git
`merge.rs`, run by the poll) counts a branch as merged when its head is in the base, its patch is (a squash), or the
forge says its PR merged; it counts only once the branch had commits of its own. Finish (pdt-api `finish.rs`) moves
the ticket to Done and cleans up the worktree, sessions, runs and workspace. Closing a ticket any other way, or
abandoning its attempt, goes through `start::close_attempt` / `stop::wind_down`: the attempt's workflow runs are
cancelled (but the run closing it) and its sessions and runs stopped; Won't do and Abandon also close its open PR and,
when `stop::clean_up_closed` finds nothing to lose (no uncommitted changes, nothing unpushed), remove the worktree, the
branch, the remote branch when it is PDT's (`pdt-<KEY>-…`, origin at the same head) and the workspace, as the
project's `StorageSettings::wont_do` says. A merge found outside PDT stops the attempt's sessions (`WindDown::Merged`)
and leaves the workflow runs to take the merge (the Pipeline's Accept has an `event` port for it).

**Pull requests and CI.** pdt-api `github.rs` reads each attempt's PR, and `ci.rs` each CI provider's runs (GitHub
Actions, GitLab CI, the local forge's command, Buildkite), into memory. What changed (CI failed or passed, a conflict, a drop
from the merge queue, a merge) is logged as events, which the built-in PR notices workflow turns into messages to the
agent.

**Conflicts at merge time.** A conflict is fixed once, when the ticket is next to merge. PR notices' first step on a
conflict or a moved base, `attempt.defer_conflict` (`workflow/nodes/conflicts.rs`, the rule is pdt-core
`merge_update::defers`), leaves it for merge time while the ticket's Pipeline stage is before the merge and the
project's `resolve_conflicts` policy says so: it records a `merge_update` ticket event and sets the attempt's in-memory
`mergeUpdate` (kept in `github.rs`'s `PrWatch`, like `pr`). The Pipeline's merge, and PR notices' fix of a pull request
set to merge after a conflict or a queue drop, start with `attempt.update_base`: in the attempt's worktree it fetches
the base, merges it (pdt-git `update.rs`, with `rerere`), takes the base's side of files a `[[merge.regenerate]]` rule
covers and runs the rule, commits, and pushes once; real conflicts leave the merge in progress and go to an agent step
with the files, hunks and merged-since subjects (`merge_update::conflict_message`). Each result is a `merge_update`
event (`how`: deferred, clean, regenerated, conflict, agent). Before Accept, the Pipeline's `attempt.has_changes`
(`workflow/nodes/attempt.rs`) asks whether the attempt has anything to merge (a pull request, commits its base lacks,
uncommitted changes); one without goes through its own Accept and `ticket.finish` (deleting the branch) instead of the
merge, after looking again once Accept is answered.

**Review.** The Changes view diffs the worktree against its merge base with the base branch (pdt-git `review.rs`;
each file says whether it is uncommitted, and generated: `linguist-generated` in `.gitattributes` or a lockfile). The
web view shows every file on one scrolling page (`ChangesView` → a `FileSection` per file): a file's diff is drawn
only once it nears the visible part of the page and dropped again far away (`review/lazy.ts`), keeping its height, and
generated, binary and very large files start behind Load diff.
Line comments are drafts until Send feedback sends them, as one message, to the attempt's agent. Approve records a
review at the branch head. Merge (pdt-api `review::merge_attempt`, shared with the `attempt.merge` workflow node)
checks the project's policy first, then turns on the forge's auto-merge or merge queue, or merges locally when there
is no PR. Where the forge can't hold a merge for its checks (`MergeSettings::auto_merge` false: a GitHub repository
without auto-merge; or GitHub refusing it for checks it doesn't require, `ForgeError::NotReady`), PDT holds it: the
attempt is set to merge (`merge_requested`, shown as the PR's `mergeHeld`), and the PR poller merges it once CI on its
head passes (`review::merge_held`, once per green CI). Merges of one attempt take a lock, so the poller's and PR
notices' merge again never both go through. Merge settings are read once per repository and base every five minutes
(`github::merge_settings`).

**Areas and overlap.** `[[area]]`s in `.pdt/project.toml` name parts of the repository. Tickets in flight are
compared by their areas and their changed files (pdt-core `overlap.rs`, refreshed every 30 s); a shared exclusive
area is a conflict.

**Sessions.** An agent session runs as a terminal (the agent's own TUI in a PTY) or a conversation (PDT's structured
view: Claude's stream-json, a PDT-owned Codex app-server, Devin and other agents over ACP). Conversation events are
stored in `session_events` (in the order received; while writes fail they wait in memory, and a Claude conversation's
gaps are filled in from its transcript when it loads); terminals keep their scrollback in memory, and a shell also tees its output to `sessions/<id>/output.log` in the
data dir (pdt-agents `output_file`, capped and rotated at the scrollback size), which the next process of that session
shows above a divider. Every agent gets PDT's instructions (pdt-core
`agent_context`, plus the project's agent instructions and the ticket), written to the session's folder and passed in
the way each agent takes them. A session belongs to an attempt (the Coder's) or to a ticket (a workflow step's
reviewer or QA). Claude, Codex and Devin terminals are resumed by their conversation ids after a restart;
conversations never are. A built-in ACP agent (OhMyPi, Cursor, Grok, Antigravity) is a profile in pdt-core `acp_agent::builtin_profiles`
(command, mode map, flavor) plus, for what it adds to ACP, a flavor mapper in pdt-core `conversation/<agent>.rs` with
recorded frames in `crates/pdt-core/tests/fixtures/<agent>/`; PDT keeps the session's permission mode by answering
the agent's permission requests itself (`acp_agent::enforced_answer`). A profile with an `installer` (Antigravity) runs
a program PDT installs from the ACP registry (pdt-core `acp_registry`, pdt-api `toolchains.rs`): Settings → Agents →
Install downloads it from the agent's own site, checks the registry's sha256 and unpacks it, without running it, into
`<data dir>/toolchains/<registry id>/<version>/`; Sign in asks the server to `authenticate`, and it keeps the
credentials itself.

**pdt-mcp.** PDT's MCP server at `/api/mcp` (pdt-api `mcp/`). Each session gets a token (its id, signed with a key in
the data folder) and the tools of its persona; each tool calls the API route's own function. Tools that need the
user's say are asked in the agent's own prompt where it supports that, or through the Inbox.

**The user's MCP servers.** `mcp` connections (Settings → MCP servers) reach agents beside pdt-mcp. Their
`config.agents` (pdt-core `mcp_servers::AgentAccess`) says who gets one (the PM, ticket and workflow sessions with an
optional persona list, sessions the user starts, project limits) and each tool's mode (allow, ask, block); a
locked-down session gets none. A connection without `agents` reaches everyone; those from before agents got them were
set to `{enabled: false}` by a migration, and the generic connection dialog writes that too (workflow-only). Claude
leaves out a Block-by-default server until its tools are listed (`ExtraServer::listed`), since it blocks tool by tool. pdt-api `agent_mcp.rs` picks a session's servers and resolves each (header and env
values that name PDT secrets, the connection's own secret, an OAuth bearer token) into a pdt-agents
`mcp::ExtraServer`, which `mcp::server_for` adds to the session's `McpServer`. pdt-agents `mcp.rs` writes each agent's
form: Claude's `--mcp-config` file plus `--allowedTools`/`--disallowedTools` (a server whose tools share one mode is
named whole, `mcp__<name>`), Codex's `mcp_servers.<name>` (a thread's `config`, or a terminal's `-c` inline table with
secrets passed through its environment: `env_vars`, `env_http_headers`), ACP's `mcpServers` filtered by the agent's
`mcpCapabilities`. "Use only PDT's MCP servers" (`HostSettings::mcp_only_pdt`) adds `--strict-mcp-config` and switches
off the servers in the user's own Codex config. The session's instructions get `mcp_servers::context_note`. PDT's
own client (`mcp_client.rs`: workflows' `mcp.tool`, Test) starts servers from the same resolution. OAuth
(`mcp_oauth.rs`): protected resource and authorization server discovery, dynamic client registration or the user's
client id, PKCE, the callback `/api/mcp-oauth/callback` on the origin the UI sent; tokens in the secret store
(`mcp-oauth:<connection id>`) and the redactor, refreshed when due (at launch and every minute), and a refusal marks
the sign-in failed, which lists an `mcp_sign_in` Inbox item. Agents get the token current at launch.

**The PM.** A host-level project manager: conversations in the host project, with the PM persona's tools (those
flagged `pm`) and a briefing built when the session starts (pdt-api `pm.rs`, pdt-core `pm.rs`). Close and Reset end
one after its journal; a reset starts its successor (`pm_sessions.reset_from`) with the same settings, briefed from
that journal. The daily update and checkpoint are not hard-coded: the built-in **PM daily checkpoint** workflow runs
the `pm.update` and `pm.checkpoint` nodes (pdt-api `workflow/nodes/pm.rs`) on an hourly schedule; `pm.update` brings
each running open conversation due (`pm_sessions.caught_up_at`, `pdt_core::pm::update_due`) up to date through the
outbox. Journaling at Close, Reset and compaction stays in `pm.rs`. A conversation can be
held elsewhere too (`pm_sessions.via`, the integration's id, which the web names by its plugin label): the Discord integration passes the user's DMs to one (pdt-int-discord `dm.rs`),
through the SDK's `pm` and `conversations` modules (start, send, close; every conversation's events as they are stored,
`conversations::subscribe_all`; answering a request as the user). Its bot holds a Gateway websocket (`gateway.rs`) in
an integration service (`Integration::services`, started once by pdtd, idle while the plugin is off).

**Watches.** A ticket or workflow run that a PM conversation (its `watch` tool) or the user (the Watch toggle) wants to
hear about (pdt-core `watch.rs`, pdt-store and pdt-api `watches.rs`, table `watches`). Each engine tick reads what
happened to each watched target since its stored cursor: the event log (outcomes, comments, stage changes, merges, CI
failures, queue drops), new Inbox items about it, and a run's status. It notes what the watch asked for, and tells a
burst once it has been quiet for 15 s (at most a minute after the first). A conversation is told through the outbox
(`reopens`, so a parked one wakes); the user, and a closed conversation's leftovers, get a `watch.event`
notification. A `once` watch ends when its target finishes. A PM Reset moves the old
conversation's watches to the fresh one (`watches::follow`); a tick during the reset waits for it.

**Workflows.** Everything that happens is an event in the event log (`events`, keyed so a duplicate is dropped). A
workflow is a versioned graph of nodes (pdt-core `workflow/`): a trigger, conditions, actions, waits, agent steps,
gates. The engine (pdt-api `workflow/engine.rs`) matches new events to enabled workflows, starts runs, and advances
their steps; node kinds live in `workflow/nodes/` or in an integration's crate. While writes fail (a full disk), the
engine's tick waits, writing a probe row (`Store::probe_writes`) after a backoff of 2 s doubling to 30 s; once one
works, events that could not be logged (kept in memory by `emit_event`) are logged, and unprocessed events start their
runs, each once by its run key. Agent steps start a session with a
persona and wait for its `submit_outcome`; messages to agents go through the outbox (`workflow/outbox.rs`), which
waits for the agent to be idle. Built-in workflows (`workflow/builtins/`) and templates (the Pipeline:
`workflow/pipeline.rs`) are installed by PDT and follow newer versions until the user edits them. A template fills
`{{params.x}}` into its one document, except "Review outside pull requests" (`workflow/outside_prs.rs`), whose
`tickets` param picks one of two node sets (`templates::filled`); a workflow recorded before a param existed gets its
old behaviour as the value (`outside_prs::upgrade`). Triggers on things
outside PDT poll: new GitHub issues (`github_issues.rs`), outside pull requests (`github_pulls.rs`) and failed runs of a
CI workflow (`github_workflow_runs.rs`: `github.workflow.failed`, read through `CiProvider::workflow_runs` since a
`RunCursor` kept in the workflow's state, so a restart neither misses nor repeats a run). The template "Report failed
workflow runs" follows each such run with `github.workflow.watch` (one gate item, Re-run failed jobs through
`CiProvider::retry`, closed once a later attempt passes; `workflow/nodes/workflow_runs.rs`). A project's policy
(pdt-core `policy.rs`) sets who merges, approvals, the running cap and auto-start; the host caps how many agents run a
turn at once.

**Auto-start** is the host built-in "Pick up tickets" (`workflow/builtins/pick_up_tickets.rs`): `ticket.startable`, `slot.freed` and a
one-minute `schedule` lead to `tickets.candidates` → `tickets.choose` → `ticket.start` (`workflow/nodes/pickup.rs`;
the rules, pure, in pdt-core `pickup.rs`). A config field left out is the project's auto-start setting, so Settings
edits the built-in's parameters without editing the workflow. The pickup watch (pdt-api `scheduler.rs`, every 10 s)
emits the triggers, for projects with auto-start on: `ticket.startable` once per ticket each time it becomes startable
(`policy::startable` and the start delay over; what it told is kept in `AppState::pickup_watch`, so a restart tells
again), and `slot.freed` when a ticket in flight ended (room under the cap, or a hot area free), its cap rose, or the
host's free agent slots went from none to some. `ticket.start` holds `AppState::pickup_lock` while it checks and starts, so concurrent pickup runs
neither start a ticket twice (the store's one-active-attempt rule is the backstop) nor go over the caps a pick was
chosen under (project, area and label, and the overlap rule, carried on each pick); it takes the first of each project's chosen tickets
still free. Having checked the cap itself, it sets `ValidStart::cap_checked`, so Start doesn't refuse for the
project's running cap (a step's cap may be higher) while still refusing for blockers and overlaps. A failed start is skipped until the ticket
changes (`AppState::auto_start_failed`), unless it failed to write (a full disk), which the next run tries again. The
watch also announces starts left half-done: an active attempt whose `started` timeline event never reached the event
log (pdtd stopped, or the write failed, between `start_attempt` and `announce_start`), once it is two minutes old
(`Store::unannounced_starts`).

**Overlap-aware pickup**: a project's `hot_areas` and `untagged_alongside_hot` (`ProjectPolicy`) become each
candidates project's `OverlapPolicy` (`avoid` when it has hot areas, else `off`); `tickets.choose`'s `overlap`,
`hotAreas`, `perArea` and `untaggedAlongsideHot` override it (`OverlapOverride`). `OverlapPolicy::check` compares a
candidate's areas with the tickets in flight. Choose keeps the overlap reasons it passed over in
`AppState::pickup_holds` (replacing its projects' entries each run, not on a dry run), and the ticket list and detail
fill `TicketSummary::pickup_hold` from it for tickets that could still start, while the hold is under three minutes
old (a pickup that stopped running stops renewing it); the pickup watch drops a project's holds when its auto-start
is off. Agents tag areas through `update_ticket`
(Refiner), `create_ticket` and `edit_ticket` (PM), checked against the project's configured areas
(`ticket::agent_areas`); `get_ticket` lists them as `projectAreas`.

**The Inbox and notifications.** The Inbox (pdt-api and pdt-store `inbox.rs`) holds what waits on the user:
questions, gates, stalled steps, secrets to enter, confirmations. An item's `title` is one line; a gate's question
(Markdown) is in `body.question`, and `inbox::decorate` adds what is computed per request: ticket key and title,
persona name, and a gate's `changeRequest` (its attempt's pull request, else the run trigger's or the ticket's external
link that a forge plugin's `is_change_request_url` accepts, named by its `change_request_name`). Some items are not
stored but listed while their cause lasts: approvals, sessions waiting, Connect GitHub, "PDT can't save changes"
(`inbox::storage`, while `Store::write_failure` says writes fail), and workflow proposals
(`inbox::proposals`: one per workflow with a proposed version, id `proposal:<workflow>:<version>`, answered
by `inbox::answer` with the Versions tab's accept and discard). An item's `group` is what kind of wait it is;
the list shows a few sections (`InboxGroup::section`: Questions, Approvals, Stuck, To act on, Ready to accept) and
`inbox::sort` orders by section, then age. The web lays every kind out the same way (`web/src/inbox/itemView.tsx`
makes one spec per item that the desktop and the phone both render). Notifications (`notifications.rs`) are the
transient kind.

**Ticket proposals.** A ticket an agent session makes (source `agent`, not via Make into parts) is held for the user
while the "Ticket proposals" built-in is on (`workflow::nodes::ticket_proposal::proposals_on`): `create_ticket` and
`POST /api/projects/{id}/tickets` make it in Draft without `agentic` (pdt-core `ticket_proposal::hold`), held
(`tickets.proposed_at`, so `proposed` from the start; cleared once it leaves Draft), with the agent's `reason` on its
`created` event. The built-in's `ticket.created` trigger runs `inbox.ticket_proposal`, which stores a
`ticket_proposal` item; ticket lists leave out tickets with an open one unless asked (`proposed=true`). A held ticket
the workflow never put in the Inbox (turned off, its conditions edited, its run failed) stays listed in Draft. `ticket_proposal::answer` applies the user's Accept (the edits, the `agentic` box), Duplicate of…
(a `relates` link, Won't do) or Reject (Won't do), then tells the proposing session while it runs, else comments on
the ticket it came from.

**Secrets.** Values live in the OS keychain (or, on Linux without a Secret Service, a file only the user can read) and
in memory; agents and workflows refer to them by name (`run_with_secrets`, `write_secret_file`, `{{secret.x}}` in a
node's config). pdt-core `secret::Redactor` replaces any value with `[secret:<name>]` in everything the store writes,
in pdt-mcp results, in conversation deltas and in the logs. Code: pdt-api `vault.rs` and `secrets.rs`, pdt-store
`secrets.rs`.

**GitHub.** The forge (`crates/pdt-github`) signs in with the OAuth device flow or a pasted token; logins are stored
in the keychain (pdt-forge `login.rs`, pdt-api `forge_login.rs`). A 401 marks the login rejected only after
`GET /user` also answers 401; a rate limit blocks calls locally until it resets. The device flow needs an OAuth App's
client ID, `GITHUB_OAUTH_CLIENT_ID` in `brand.rs` (`PDT_GITHUB_CLIENT_ID` overrides it). PDT's app is owned by the
RossTools organization with Device Flow on and token expiry off. To register another (a fork, an Enterprise host):
GitHub → Settings → Developer settings → OAuth Apps → New OAuth App, any callback URL (the device flow never calls
it), tick **Enable Device Flow**, and put its client ID in `brand.rs`; no client secret is needed.

**Documents.** Docs and sheets kept by a provider, linked to projects and tickets. PDT keeps a record of
each (`documents`, `document_projects`, `document_tickets`; pdt-store `documents.rs`) and asks the provider for the
rest. A provider implements pdt-api's `documents::DocumentProvider` (like a forge, it declares
`ProviderCapabilities`: list and read always; search, create, edit, rename, delete, comments and an embed URL when
it can) and comes from `AppState::documents` (PDT's own) or an enabled integration's
`Integration::document_providers`. The provider and local folder of a new document are the project's
(`project_document_settings`), else the host's (`HostSettings::documents_provider`, `documents_dir`), else the first
provider and `<data dir>/documents` (pdt-core `document.rs`, pure). `documents::testing::FakeProvider` keeps
documents in memory for tests; with `PDT_SEED=demo` a dev pdtd registers one as "Sample". The routes are in
`documents/routes.rs`; `get_ticket` lists a ticket's documents and the ticket section of the instructions says to read
its design doc. Edits are provider-neutral `DocumentEdit` operations (pdt-core `document.rs`): a provider maps each to its own API and
answers `Unsupported` for one it can't do; pdt-core `document_ops.rs` applies the text ones to Markdown and the sheet
ones to plain cells, for providers that keep content in memory or as Markdown. The agent tools (`documents_list`,
`doc_read`, `doc_edit`, `sheet_read`, `sheet_edit`, `document_create`, `document_open_beside`, comments) are actions
of the `documents` integration (pdt-api `documents/tools.rs`); the writes ask, with `AskTarget::Document` (a project
the document is linked to lets the PM act) or the project or ticket they name, and comments come fenced as outside
input. The documents open beside a conversation are `documents_beside` rows (session, document, position, who opened
it, when it was focused: the newest focused is the tab shown), served at `/api/sessions/{id}/documents`; the web's
`DocumentsBesideLayout` (`web/src/documents/BesidePanel.tsx`) wraps the agent and PM conversation views, and
`DocumentBody` reads a document again whenever its `updatedAt` moves (an edit through PDT touches it) and lights what
changed.

PDT's own provider is `documents::local::LocalProvider` (`AppState::local`, listed first in `AppState::documents`).
Its reference is the file's full path: a doc is a `.md` (its images in `<name>.assets/` beside it), a sheet a `.xlsx`
read and written by pdt-sheet (IronCalc; files are written to a temp file and renamed). Comments
(`document_comments`, anchored by a `CommentAnchor`: a heading, quoted text, or a sheet's cell or range) and chart
specs (`document_charts`, `ChartSpec`) are in the store. A sheet is live: the provider holds one IronCalc model per
open sheet (loaded on first use, dropped when idle with no view), and every change is numbered. A view loads the
model (`GET …/sheet`, IronCalc's bytes and `x-sheet-seq`), posts its own edits as IronCalc diffs with the number it
has (`POST …/sheet/diffs`), and follows `GET …/events` (SSE): `diffs` (from other views and agents' edits through
`apply`), `reload`, `changed` (a doc saved), `comments`, `charts`. A view that missed a number reloads; diffs sent on
an old number are applied anyway (last write wins) and the answer says `conflict`, so the view reloads and says so.
Each stream also checks the file every 2 s, so a change made outside PDT reaches the open views. A doc's editor saves
with `baseVersion` (a hash of the file) and gets `409 conflict` when it changed since. Charts are drawn in the browser
with Chart.js from `GET …/charts`, which reads their numbers from pdtd's model; they float over the grid at their
anchor cell. The web UI's `@ironcalc/workbook` must be the same version as pdt-sheet's `ironcalc` (they exchange
IronCalc's bytes and diffs; a pdt-sheet test checks), and is loaded only when a sheet opens.

`GET /api/documents/{id}/version` is the provider's version (Google's Drive counter): `DocumentBody` polls it in PDT's
own view, so edits made at the provider (in Google's editor) show too.

Workflows work on documents through the `documents` integration's node kinds (pdt-api `documents/nodes.rs`):
`document.create`, `document.append`, `sheet.append_rows`, `sheet.set_range`, `sheet.add_chart` and
`document.comment`, each a thin layer over the same provider calls as the tools (`apply_edits`, `create_document`),
and its two templates (`documents/templates.rs`). The triggers `document.changed` and `document.comment_added` are
fed by a poller (`documents/watch.rs`, the 60 s pass): it looks only at the documents an enabled trigger watches
(the one it names, else those linked to the workflow's projects), compares the provider's `version` and its comment
ids with what it saw last (host values `sensor.document_version.<id>`, `sensor.document_comments.<id>`; the first look
only records; the state of a document no trigger watches now is deleted, so watching it again starts with a first
look), and logs one event per linked project. Each trigger takes one copy of an event (pdt-core
`document_watch::accepts_copy`), so a document linked to two projects starts a host workflow once. Comments an agent or
a workflow wrote never count: the provider's `actor` on a comment says so where it knows, and the ids of replies PDT
posted are kept (`sensor.document_ours.<id>`) for providers that write them as the user (Google). A comment's text is
an outside field of its event, fenced for agents unless `byUser` (the user wrote it). The sheet steps write text that came from a
reference with a leading `'` when it would read as a formula (pdt-core `document_watch::text_from_refs`); every
provider takes that as text (IronCalc's quote prefix, Google's `USER_ENTERED`, `document_ops`), so only formulas
written into the step's config run.

**Google.** An optional plugin (`crates/pdt-int-google`, pdtd's `google` feature), the `google` document provider.
Each Google account is a `google` connection (the folder for new files, and, for the user's own OAuth client, its ID
in the config, the client secret as its secret, the picker's API key and project number); signing in (`oauth.rs`: the
authorization code flow with PKCE, the `drive.file` scope only) keeps the refresh token and email as a secret of their
own (`connection:<id>:google`, `accounts.rs`), so editing the connection keeps them. A connection with no client ID
uses PDT's own clients (`builtin.rs`), built in from `PDT_GOOGLE_*` (`option_env!`; the release and nightly bundle
jobs pass the repository's secrets through `.github/actions/bundle`; a build without them offers only the user's own
client, `GET /api/google/client`). Their only redirect URI is the website's `/oauth/google` page
(`site/public/oauth/google.js`): `state` is `<nonce>.<base64url(origin)>`, the page forwards only to an origin that
looks like a PDT (loopback, private and Tailscale addresses, `localhost`, `*.ts.net`: the same rule as
`builtin::pdt_origin`, which pdtd applies before issuing one) and asks first, and the callback takes only a state it
issued, traded with its PKCE verifier. Signing in with no account (`POST /api/google/sign-in`) adds the connection,
named after the email, once Google says who it is. There are two clients, after Google's rule for apps that are
partly an agent: the standard one for the user's own calls and the agentic one for agents' document tools.
`pdt_api::sdk::documents::by_agent` (a task-local set around every agent tool call, `AnyAction::call`) tells the
provider an agent is calling, and it then uses the account's agent sign-in (`connection:<id>:google-agent`, its own
consent, `"agent": true` on the sign-in route) and never the user's; taking it away (`DELETE …/agents`) forgets it
without calling Google's revoke, which would revoke every client of the project. An account with its own client has
one sign-in, which agents use too. Deleting the connection runs `Integration::connection_deleted` (for every
built integration with that kind, on or off), where Google revokes the token and forgets the sign-in. A document's reference is
`<connection id>/<file id>`. `api.rs` is the client (an access token per account in memory, renewed once on a 401;
Google's errors as `ProviderError`, a rate limit as `Busy`, which the API answers 429), over pdt-forge's blocking
`Transport` (tests use recorded answers). `drive.rs` has the files, a Doc's Markdown (Drive's export), a Sheet's
cells, comments and versions; `docs.rs` turns Markdown into Docs `batchUpdate` requests (inserted from the last block
at one index, UTF-16 indexes, sections found in `documents.get`'s structure); `sheets.rs` maps sheet edits to
`batchUpdate` requests and `values.batchUpdate` writes (`USER_ENTERED`), in order. A chart's PDT id becomes Google's numeric chart id by
a hash and is kept in the spreadsheet's developer metadata (`pdt.chart`), so a read gives charts back under PDT's ids. `routes.rs` signs in and out
(`/api/google/…`; the callback page is where Google sends the browser back) and gives the browser Google's file
picker's token; signing in and the picker are for the web app only (its UI token). The web app loads the picker
(`web/src/documents/google.tsx`); a picked file is imported like any other. The document page embeds the provider's
`embed_url` (Google's editor, with `authuser`) and falls back to PDT's own reading.

**GitLab.** An optional plugin (`crates/pdt-int-gitlab`, pdtd's `gitlab` feature) over GitLab's REST API (v4), for
gitlab.com and self-hosted servers; nothing runs `glab`. It claims `origin` remotes on gitlab.com and on each host it
has a login for (one account per host, a token with the `api` scope, or the device flow with `PDT_GITLAB_CLIENT_ID`),
and can't be turned off while a project uses it. Merge requests are its change requests, a merge train (when the
project has one) is the base branch's merge queue, and `GitlabCi` (provider id `gitlab_ci`) reads pipelines.
`api.rs` is the client, `forge.rs` the `Forge`, `ci.rs` the CI provider, `plugin.rs` the claim and `login.rs` the
login.

## Adding to the hot lists

Most changes add a route, a client call, a tool, a prompt, a built-in workflow or a screen. Each of those lives in its
own file, and the central list takes at most one line, so tickets built side by side rarely touch the same lines.

| To add | Write it in | The central line |
| --- | --- | --- |
| An API route | The area's module (pdt-api `<area>.rs`), in its `router()` | A new module adds `.merge(crate::<area>::router())` to `routes::router` (sorted) |
| A web API call | `web/src/api/<area>.ts`, in its `<area>Api` object | A new area adds its import and `...<area>Api` to `api` in `web/src/api/client.ts` (sorted). `client.test.ts` fails when two areas define the same call. |
| A pdt-mcp tool | Its group's module in pdt-api `mcp/tools/` (`defs()` and `call()`); shared helpers are in `mcp/tools/mod.rs` | A new group adds itself to `defs()` and `call()` in `mod.rs`. The order of `defs()` is the `tools/list` order. |
| A built-in persona's prompt | pdt-core `persona/<id>.md` (read with `include_str!`) | A new persona adds an entry to `BUILTIN_PERSONAS` in `persona.rs` |
| A built-in workflow | Its own module in pdt-api `workflow/builtins/` | Its `mod` line, its id constant and its line in `BUILTINS` (or the integration's list) in `builtins/mod.rs` |
| A background loop | pdt-api `background.rs` (`start_background`) | |
| A lazy screen or dialog | Its feature folder | One line in `web/src/shell/screens.tsx`, and the place in `AppShell.tsx` that shows it |
| A Settings page of its own | Its feature folder | Its kind in `web/src/shell/route.ts` and `SettingsNav`, and one entry in `web/src/shell/settingsPages.tsx` |

A new top-level view (Inbox, Notes and the like) still means editing `AppShell.tsx`, because its state lives in the
shell. `AppShell`'s tests are split by area (`AppShell.<area>.test.tsx`), with shared fixtures in
`web/src/shell/fixtures.ts`.

## Where to look

| Area | Rust | Web |
| --- | --- | --- |
| Host, settings, updates, remote access | pdt-api `host.rs`, `update.rs`, `remote.rs`, guards (`host_guard.rs`, `identity_guard.rs`, `untrusted.rs`); `crates/pdt-host`; `crates/pdt-update` | `web/src/host`, `web/src/setup` |
| Projects, worktrees, runs | pdt-api `projects.rs`, `worktrees.rs`, `runs.rs`, `run_manager.rs`, `runconfig.rs`; pdt-core `runconfig.rs` | `web/src/projects`, `web/src/runs` |
| Storage and disk space | pdt-api `storage.rs`; pdt-core `storage.rs`; pdt-git `is_disposable` | `web/src/host/StorageSettings.tsx`, `web/src/projects/worktreeStorage.tsx` |
| Workspaces and tabs | pdt-api `workspaces.rs`; pdt-store `workspaces.rs` | `web/src/workspaces`, `web/src/shell` |
| Tickets, Start, parts, merge detection | pdt-api `tickets.rs`, `start.rs`, `stop.rs`, `finish.rs`, `parts.rs`, `scheduler.rs`; pdt-core `ticket.rs`, `start.rs`, `parts.rs`, `policy.rs`, `pickup.rs`; pdt-git `merge.rs` | `web/src/tickets` |
| PRs, CI, forges | pdt-api `github.rs`, `ci.rs`, `forge.rs`, `local_forge.rs`; `crates/pdt-forge`, `crates/pdt-github`, `crates/pdt-int-gitlab` | `web/src/tickets`, `web/src/plugins` |
| Review and merge | pdt-api `review.rs`; pdt-core `review.rs`; pdt-git `review.rs` | `web/src/review` |
| Sessions and terminals | pdt-api `sessions.rs`, `pty_ws.rs`, `activity.rs`; `crates/pdt-agents` | `web/src/sessions`, `web/src/terminal` |
| Conversations | pdt-api `conversations.rs`; pdt-core `conversation/`; pdt-agents `adapter/` | `web/src/conversation` |
| Agent instructions and guides | pdt-core `agent_context.rs`; pdt-api `guides.rs` (serves `docs/agent-guide.md`, the run-config guide (`docs/user/automation/run-configuration.md` then `docs/run-config.md`), `docs/prompts/`) | |
| pdt-mcp and personas | pdt-api `mcp/` (tools in `mcp/tools/`), `personas.rs`; pdt-core `persona.rs` (prompts in `persona/`), `mcp.rs` | `web/src/personas` |
| Workflows, Inbox, notifications | pdt-api `workflow/`, `inbox.rs`, `notifications.rs`; pdt-core `workflow/`; pdt-store `workflows.rs`, `event_log.rs` | `web/src/workflows`, `web/src/inbox`, `web/src/notifications` |
| Integrations | pdt-api `sdk.rs`, `integrations/`; `crates/pdt-int-*`; pdtd `src/integrations/` | `web/src/plugins`, `web/src/connections` |
| Documents | pdt-api `documents/`; pdt-core `document.rs`, `document_ops.rs`; pdt-store `documents.rs`; `crates/pdt-int-google` | `web/src/documents` |
| The PM | pdt-api `pm.rs`; pdt-core `pm.rs`; pdt-store `pm.rs` | `web/src/pm` |
| Watches | pdt-api `watches.rs`; pdt-core `watch.rs`; pdt-store `watches.rs` | `web/src/common/Watch.tsx` |
| Secrets | pdt-api `vault.rs`, `secrets.rs`; pdt-core `secret.rs` | `web/src/secrets` |
| The user's MCP servers | pdt-api `agent_mcp.rs`, `mcp_oauth.rs`, `mcp_servers.rs`, `mcp_client.rs`; pdt-core `mcp_servers.rs`; pdt-agents `mcp.rs` | `web/src/mcp` |
| Database | `crates/pdt-store` (`migrations/`, `migrate.rs`) | |
| Design system | | `web/ui` (`web/ui/README.md`), shared app components in `web/src/common` |
| Website | | `site/` (`site/README.md`) |
