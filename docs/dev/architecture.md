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
  `pdt-host stop` (`stop.rs`) stops pdt-host first (launchd bootout, `systemctl --user stop`, or killing the lock
  holder) and then asks pdtd to shut down, leaving the registration. Settings → This host → Startup turns the
  registration off and on (`login.rs`, through pdt-api's `StartAtLogin`, which only pdtd wires to the real one); off
  leaves a `host-login-off` marker in the data folder that `install` and the tray's first-launch registration honour.
- **The tray app** (`desktop/src-tauri`, Tauri v2) is optional and has no window of its own: it starts pdt-host when
  pdtd doesn't answer, opens the UI in the browser and can restart the daemon. "Quit tray" leaves PDT running;
  "Stop PDT and quit" asks first when `/api/host/busy` lists something, then runs `pdt-host stop`.
- **The web UI** (`web/`, React + Vite) talks only to pdtd's `/api`: JSON over HTTP, polled every 2 s, plus
  WebSockets for terminals and conversation events and an SSE stream for notifications. It reloads itself when pdtd
  reports a new build: a production build names its id in index.html's `web-build` meta, health's `webBuild` reports
  the same id, and the page asks again whenever it becomes visible (`useReloadOnNewBuild`). While a draft or a dialog
  holds the reload back, a banner offers it.
- **Agents** (Claude Code, Codex, Devin, ACP agents) are child processes of pdtd. They report activity through hooks
  (`pdtd hook …`, which posts to the pdtd that launched them) and call PDT through pdt-mcp.
- **Remote access** is `tailscale serve` in front of pdtd (pdt-api `remote.rs`, `crates/pdt-tailscale`); requests from
  the tailnet must carry the owner's Tailscale identity. pdtd only removes mappings its instance made; it keeps their
  ports in the store (host value `remote.served`), since `serve --bg` mappings outlive the process. A web run's port
  is mapped to an owner-checking proxy of pdtd's on a free loopback port (`remote_proxy.rs`), never to the run, and
  the proxy stops when the mapping goes; after a restart the mapping leads nowhere until the resumed run maps it again.

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
| `pdt-forge` | The `Forge` trait (change requests, CI, merges, reviews, issues, with declared capabilities), `CiProvider`, the login store, the forges' shared HTTP `transport` (reqwest, `Offline`, recorded answers), their shared REST client (`rest.rs`: `RestClient<F: Flavor>`, rate limit, token renewal, refused logins, paging, ETags) and OAuth device flow and refresh (`oauth.rs`), and `FakeForge` (`testing.rs`). |
| `pdt-github` | The GitHub forge over GitHub's REST and GraphQL APIs (nothing runs `gh`): pull requests, Actions, merges, issues, the OAuth device-flow login, rate limits. The `Forge` methods are in `forge.rs`. |
| `pdt-tailscale` | The `tailscale` CLI (`serve`, `status`), and `fake-tailscale` for tests. |
| `pdt-update` | The GitHub release client: finding a release, verified downloads. Reads without a token unless one is saved; a rejected token falls back to anonymous, a rate limit is a "try later" (`UpdateStatus.retryAt`). |
| `pdt-sheet` | Local sheets on IronCalc: `.xlsx` and `.csv` in and out, cells with computed values, formats, sheets, sort and filter, and IronCalc's sync diffs. |
| `pdt-host` | The supervisor: running pdtd, installing updates, rolling back, login registration. |
| `pdtd` | The daemon binary: config, logging, startup (`main.rs`: the state, the startup resume and the poll), `pdtd hook`, the demo seed (`seed.rs`), and which integrations it is built with (`src/integrations/`). |
| `pdt-int-*` | The optional integrations (discord, jira, buildkite, gitlab, google, arr, scryer, plex), one crate each, built on pdt-api's `sdk` and `Integration` trait. Each is a pdtd cargo feature (all on by default) and is enabled per host in Settings → Plugins. Outside their crates, only pdt-core `integration::KNOWN` and pdtd's features name them; the web shows what they declare: a connection kind's fields (switches, patterns, conditions), invite link (`link`), row summary (`summary`) and status line (`status`, `Integration::connection_status`, `GET /api/connections/{id}/status`), a project settings schema's import by key (`import`, `Integration::import_by_key`, `POST /api/projects/{id}/import/{integration}`), each plugin's label, its accounts' sign-in in the browser (`Integration::sign_in`, pdt-core `integration::SignIn`: the routes under its `api`, which the web's `AccountSignIn` calls), a document provider's own file picker (`DocumentProvider::picker`, `ProviderPicker`: an ES module the provider serves, which the import dialog loads and runs for an account) and a forge's name for a change request (`ForgeLoginView.changeRequestName`). |

Errors from the API are JSON `{code, message}` (pdt-api `error.rs`); clients match on `code`. Every code is a constant
in `error::codes`, with one status each (the module's doc says which status means what), built with `ApiError`'s
constructors (`invalid`, `conflict`, `upstream`, `keychain`, `io`, …); a test fails on a code spelled out in a route.
A missing record is `.or_404("what")` (`error::OrNotFound`), and `st.project(id)`, `st.ticket(id)`, `st.session(id)`
load one by id. API types derive ts-rs and are generated into `web/src/api/gen`.

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
`AppState::storage` keeps the sizes in memory, so a restart measures again) and logs the disks' level changes
(`watch_disks`). The cleanup is the built-in Clean up storage's (below): `storage::auto_cleanup` cleans or removes
finished tickets' worktrees per the project's `project_settings.storage`. A project's build
output is `[storage] build_output` from its main checkout's run configuration, else detected from the files at its
root (`detect_build_output`). Free space comes from `AppState::disk_space` (fs4, installed by pdtd; `AppState::new` reads none, tests set a fake) for the data
folder and each project's folder, grouped by device (`storage::disk_id`) with the least free space any read, at the
thresholds `storage::thresholds` reads from Clean up storage's inputs (the host settings' `low_disk_gb` /
`critical_disk_gb` until it is installed, and once to fill its inputs). Below the low one the Inbox lists a `low_disk`
item.

**Pickup paused** (`pickup_pause.rs`, the host value `pickup.paused`: a reason and the workflow that paused it). While
it is set, `ticket.start` starts nothing, the pickup watch tells nothing, `autoStartAt` is left out and
`pickupPaused` is filled in on each ticket auto-start would take (`scheduler::apply_auto_start`). `pickup.pause` /
`pickup.resume` (`nodes/pickup_pause.rs`) set and clear it, a resume only its own workflow's pause unless `any`;
`DELETE /api/pickup/pause` is the user's Resume.

**Runs.** A run is a `[[run]]` or `[[test]]` from `.pdt/project.toml` (plus host overrides; the user docs' Run configuration page)
started in a checkout, on a port from the host's range. Runs live in PTYs, or detached (`detach = true`) so they
outlive pdtd and are reattached after a restart. On Windows each PTY session's processes are in a kill-on-close job
object only `PtyManager` holds (`pty/job.rs`), so the whole tree ends when pdtd does, however it ends, or when the
session is removed; a process that asks to break away (a detached run) may. When remote access changes the host, the runs (never tests) that use
`$PDT_RUN_HOST` or `$PDT_RUN_URL` restart (`run_manager::restart_host_dependent_runs`); any stop of the run meanwhile
cancels that restart (`AppState::run_stops`), and one that cannot start again fails the run with the reason.

**Tickets.** Numbered per project (`<KEY>-<n>`, never reused), in a stage (draft, ready, todo, in progress, done,
won't do), ordered by a fractional `rank`, linked (`blocks`, `relates`), with a timeline of events (comments are
events). A parent ticket's parts are tickets one level down; the parent is never started itself. Starting a part
moves its parent to In progress (`Store::part_started`); the "Close finished parents" built-in closes and reopens it
as its parts close and reopen (`ticket.close_parent`, the rule in pdt-core `parts::parent_follow`). Tickets from
outside (`source: external`) need the user's admission before agents work on them. `pipelineStage` is the Board column
a workflow step sets. The board's Done window is a per-project host setting (`project_settings.board`, pdt-core
`board::BoardSettings`, `GET`/`PUT /api/projects/{id}/board`).

**Issue links.** A ticket's links to issues in other trackers are `ticket_links_external` rows: the tracker (`system`,
a plugin's id), the connection or repository it is read through, the key and URL, how it was made (`origin`: created,
imported, manual; pdt-core `issue_link`), `close_on_done`, and the issue's title and state as last read. An issue is
linked to one ticket. Each tracker plugin recognizes its own addresses and keys (`Integration::find_issue`; GitHub's and
GitLab's through `issue_links::find_forge_issue` with their page parsers) and reads a link again
(`Integration::read_issue`) when the ticket is viewed and the link is `REFRESH_AFTER_MS` old (`POST
/api/tickets/{id}/issues/refresh`). `ticket.stage_changed` carries every link in `external` (and
`githubIssue`), and those with `close_on_done` in `closing`; the trackers' Done built-ins test `closing`, so a manual
link, off by default, never closes someone else's issue unless the user turns it on (created and imported links always
have it), while the user's own workflows see and act on every link. A step given a ticket acts on its first link
(`Store::external_links`: closing ones first). Plain links (an
address and label) are `ticket_web_links`, kept apart.

**Attempts.** One go at a ticket: its branch, worktree, base branch, workspace and the merge
state. The branch is named from the project's template (pdt-core `branch_name`: the parts table, empty parts dropped
with their separator, git's rules and the 100-character cap; pdt-api `branch_names.rs`: the repository's `[branch]`
over the `project_settings.branch_names` setting over the default `pdt-{key}-{slug}`), and `defaultBranch` in ticket
responses is that name (`overlap::decorate`). Start adds `-2`, `-3`, … when another branch, here or on origin, or
another ticket's worktree has the name, and records `attempts.branch_named` (PDT named it, or was given it). Start
(pdt-api `start.rs`) makes the branch and worktree, the attempt and the first agent session, whose
prompt is pdt-core `start::start_prompt`. An attempt ends as merged or abandoned. Merge detection (pdt-git
`merge.rs`, run by the poll) counts a branch as merged when its head is in the base, its patch is (a squash), or the
forge says its PR merged; it counts only once the branch had commits of its own. Finish (pdt-api `finish.rs`) moves
the ticket to Done and cleans up the worktree, sessions, runs and workspace. Closing a ticket any other way, or
abandoning its attempt, goes through `start::close_attempt` / `stop::wind_down`: the attempt's workflow runs are
cancelled (but the run closing it) and its sessions and runs stopped; Won't do and Abandon also close its open PR and,
when `stop::clean_up_closed` finds nothing to lose (no uncommitted changes, nothing unpushed), remove the worktree, the
branch, the remote branch when it is PDT's (`branch_named`, or `pdt-<KEY>-…` for an attempt from before it was recorded;
origin at the same head) and the workspace, as the
project's `StorageSettings::wont_do` says. A merge found outside PDT stops the attempt's sessions (`WindDown::Merged`)
and leaves the workflow runs to take the merge (the Pipeline's Accept has an `event` port for it).

**Pull requests and CI.** pdt-api `github.rs` reads each attempt's PR, and `ci.rs` each CI provider's runs (GitHub
Actions, GitLab CI, the local forge's command, Buildkite), into memory. What changed (CI failed or passed, a conflict, a drop
from the merge queue, a merge) is logged as events, which the built-in PR notices workflow turns into messages to the
agent. CI on the head rolled up as `cancelled` (nothing failed, running or queued there) is `ci.run_cancelled`, keyed by
its cancelled runs and their attempts, so a re-run cancelled again is news again; runs on older commits never make CI
of the head. The built-in "When CI is cancelled" (`builtins/ci_cancelled.rs`) counts cancels per attempt and head in its
state and re-runs the first one with `ci.rerun_head` (`nodes/ci_cancelled.rs`: reads the PR again, then retries each
cancelled run through its `CiProvider`, or takes `moved_on` when the head moved or its CI is no longer cancelled); a
later cancel, or a re-run that fails, gets `ci.stalled`: a gate in the `stalled` group (Re-run CI, Dismiss) that looks
at the head every minute and leaves by `moved_on` once it moved on, which closes the item. A gate in the `stalled`
group shows its ticket as stalled (`scheduler::ticket_agents`, named by the item's `who`).

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
area is a conflict. A single ticket's response computes only that ticket's overlaps (`compute_for`). The effective
run configuration is cached per checkout (`runconfig::ConfigCache`) while the file's modification time and size and
the project's overrides stay the same, so the 2 s polls don't re-read it; a file modified in the last 2 s is not cached.

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

The web's conversation view folds tool calls into summary rows (`web/src/conversation/blocks.ts`), except pdt-mcp's
actions (`pdtTools.ts` `isCardTool`: tickets created, edited or stopped, links, messages, comments, workflow runs,
watches, issues, documents, notes), which render as cards (`PdtToolCard.tsx`) from the call's input and pdt-mcp's JSON
result (a field a card needs goes in the tool's result); an approval of one shows `PdtProposal.tsx`. A folded run of
ticket lookups (`lookedUp`) names the tickets it read as pills, in `ToolCallRow`'s `tags`, outside its button. Its Markdown gets `MarkdownRefsContext` (web/src/common/markdownRefs.ts): with it, ticket keys
become pills with a hover card (`TicketPill.tsx`, fetched on first hover), `#12` and the forge's PR links PR pills,
inline-code file paths links to the file viewer once the session's folder has them, SHAs muted, tables framed with
status chips. Without it (ticket descriptions, notes) Markdown only links ticket keys.

**pdt-mcp.** PDT's MCP server at `/api/mcp` (pdt-api `mcp/`). Each session gets a token (its id, signed with a key in
the data folder) and the tools of its persona; each tool calls the API route's own function. Tools that need the
user's say are asked in the agent's own prompt where it supports that, or through the Inbox.

A running session's tools follow its persona's current version (its instructions stay those it launched with), so a
tool PDT adds to a built-in persona, through a release or a plugin turned on, reaches sessions already running.
pdt-mcp declares `tools.listChanged` and tells a session `notifications/tools/list_changed` when the list it would get
differs from the one it last listed (`mcp/listen.rs`): the fingerprint of that list is `mcp-tools` in the session's
folder, so a change made while pdtd was down (an update) counts too. It is checked when something may have changed it
(`mcp::tools_may_have_changed`: Settings → Plugins, a persona saved by the user or updated by PDT), and when a session
opens its stream. A session hears it on its `GET /api/mcp` stream, or, when it holds none (a client stops reconnecting
after pdtd restarts), ahead of the answer to its next request that accepts a stream; once per change either way.
Code that changes what `session_tools` gives must call `tools_may_have_changed`.

- **Claude** acts on the notice (a headless `-p` conversation has been seen not to, on some versions). Its
  `--allowedTools` is fixed at launch, so a tool added later asks once before its first call; an asking tool asks as
  before, and a tool taken away is refused by pdt-mcp.
- **Codex** may not act on the notice (it isn't documented to); one that doesn't keeps the tools it listed when its
  thread started. New tools run without asking once it has them (`default_tools_approval_mode = "approve"`).
- **ACP agents** (Devin and the rest) are each the agent's own client; PDT can't tell whether one refreshes. One that
  doesn't keeps its launch list until a new session.

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
the `pm.update` and `pm.checkpoint` nodes (pdt-api `workflow/nodes/pm.rs`) on the schedule Settings → This host → PM
sets (hourly by default); `pm.update` brings
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
conversation's watches to the fresh one (`watches::follow`); a tick during the reset waits for it. The web shows them from
`GET /api/watches` with no filter (every active watch, each with its target's title and stage or status), one shared
query (`useAllWatches`) for the board's and backlog's eyes, the PM list's and the conversation header's watch list.

**Workflows.** Everything that happens is an event in the event log (`events`, keyed so a duplicate is dropped). A
workflow is a versioned graph of nodes (pdt-core `workflow/`): a trigger, conditions, actions, waits, agent steps,
gates. The engine (pdt-api `workflow/engine/`) matches new events to enabled workflows, starts runs, and advances
their steps; node kinds live in `workflow/nodes/` or in an integration's crate. While writes fail (a full disk), the
engine's tick waits, writing a probe row (`Store::probe_writes`) after a backoff of 2 s doubling to 30 s; once one
works, events that could not be logged (kept in memory by `emit_event`) are logged, and unprocessed events start their
runs, each once by its run key. Agent steps start a session with a
persona and wait for its `submit_outcome`; messages to agents go through the outbox (`workflow/outbox.rs`), which
waits for the agent to be idle. Built-in workflows (`workflow/builtins/`) and templates (the Pipeline:
`workflow/pipeline.rs`) are installed by PDT and follow newer versions until the user edits them. A template fills
`{{params.x}}` into its one document, except "Review outside pull requests" (`workflow/outside_prs.rs`), whose
`tickets` param picks one of two node sets, and "After a release" (`workflow/after_release.rs`), which has its
announcement only with a chat connection and its cask bump (`github.homebrew_cask`, `workflow/nodes/homebrew.rs`, files
through `Forge::file` / `create_branch` / `write_file`) only with a tap (both in `templates::filled`); a workflow recorded before a param existed gets its
old behaviour as the value (`outside_prs::upgrade`). Triggers on things
outside PDT poll: new GitHub issues (`github_issues.rs`), outside pull requests (`github_pulls.rs`) and failed runs of a
CI workflow (`github_workflow_runs.rs`: `github.workflow.failed`, read through `CiProvider::workflow_runs` since a
`RunCursor` kept in the workflow's state, so a restart neither misses nor repeats a run). The template "Report failed
workflow runs" follows each such run with `github.workflow.watch` (one gate item, Re-run failed jobs through
`CiProvider::retry`, closed once a later attempt passes; `workflow/nodes/workflow_runs.rs`). A project's policy
(pdt-core `policy.rs`) sets who merges, approvals, the running cap and auto-start; the host caps how many agents run a
turn at once.

**Inbox, session and run events** come from pdt-api `workflow/sensors.rs`. `inbox.created`: the Notifier's pass
hands it every open item, stored or listed (approvals, proposals, low disk), and it logs the ones it hasn't seen,
keyed by id and opening time (items already open at startup are left alone). `inbox.answered`: the answer routes
(the Inbox's, a permission request's, a gate's in the run view). `session.needs_you` and `session.turn_ended`: an
agent session's activity change (`activity.rs` `persist` for terminals, `conversations/live.rs` `refresh_activity`),
each transition once. `session.exited`: a terminal's exit (with `Notifier::take_expected_exit` as `stoppedByPdt`), a
conversation's process dying, or its Stop. `run.failed` and `run.finished`: `finish_run`. None of them goes through
notification levels.

**Stuck agent steps** (`workflow/stuck.rs`). A working step that stalls (`agent/mod.rs` `stall`, after a turn without an
outcome or its session going), reaches its loop limit (`start_step`) or times out (`complete_step` by `timeout`) is
handed to the built-in "When an agent is stuck" (`builtins/stuck_agents.rs`) while it is on: a row in
`agent_escalations` and an `agent.stalled` / `agent.loop_limit` / `agent.timed_out` event, with no Inbox item yet. Its
nodes (`nodes/agent_stuck.rs`: `agent.nudge`, `agent.restart`, `agent.inbox`, `agent.stop`) act on the step, taking
its run's lock (`engine::lock_other_run`, since runs of one ticket share a lock), and end the escalation. A stalled
step restarts in place (a new session bound to the same step); a timed-out one is reopened (`reopen_at`) with a step
whose `wait` says `fresh`. `stuck::sweep` (in the agent sweep) gives a step PDT's own handling (one nudge, then the
Inbox) when the workflow is off, its run for the step ended without acting, or none started within
`START_GRACE_MS`. The workflow's inputs are template params: Settings → Agents reads them back from its document
(`templates::unfill`) and saves a version noted `inputs::SETTINGS_NOTE`, which still follows PDT's newer versions; any
other edit makes it the user's (`InputsBuiltin::customized`). Its reset leaves the workflow on or off, since Settings
has a switch for that.

**Idle tickets** (`workflow/idle.rs`). The poll's idle check is the safety net for tickets stuck in ways nothing else
notices: a ticket in progress (not waiting until clear) with an unfinished workflow run, but no agent step that isn't
done or waits for a slot, no busy session on its attempt, no open Inbox item about it (the idle net's own, `reason:
idle`, aside: open or dismissed, it neither ends the spell nor starts a new one), and a pull request (if any) that
is open with no CI running or queued and neither on auto-merge nor queued. Each spell's start is kept in memory
(`Sensors::idle`); after the Idle tickets input of "When an agent is stuck" (`idleMinutes`, read from its current
document; 0 or the workflow off: no check) it logs `ticket.idle` once per spell (keyed by ticket and start). The
built-in's `ticket.stalled` (`nodes/ticket_idle.rs`) puts a stalled gate in the Inbox and looks again every minute,
leaving by `moved_on` (closing it) once the ticket isn't idle, its own run not counting as unfinished work.

**Notification rules** (`builtins/notification_rules.rs`). Each kind's level and quiet hours live in the built-in's
`notification.levels` nodes (`nodes/notification_levels.rs`: a trigger kind with no event type and no ports, so it
starts no runs). The Notifier reads them with `notifications::rules` as it makes each notification, never through a
run: from the enabled workflow (cached by its id, version and enabled flag), PDT's defaults while it is off, and the
old host value `notifications.show` while it isn't installed yet (the first install copies that value in, as
`notification_rules::migrated`). A levels node with `projects` sets those projects' levels. In quiet hours
`notify_at` makes a toast or desktop notification at `inbox` and keeps it in the host value
`notifications.quiet_held`; the first sync once no quiet hours hold makes one `notifications.quiet_summary`. The
optional rules are branches on `inbox.created` and `run.failed` whose triggers' `where` holds a param, so with a rule
off no run starts. Escalation waits for `inbox.answered` up to `escalateAfter`, then `notify` with `show: desktop` and
`whileOpen` (`inbox::is_open`). Chat posts are `notify` steps with `show: off` and `chat`. `notify` finds the
integration that declares a `NotificationChannel` for the connection's kind and runs its node kind with `connection`
and `content`, so pdt-api never names the plugin. Settings → Notifications (`notification_rules.rs`,
`/api/notification-rules`; the old `PATCH /api/notifications/settings` sets the same levels) reads and writes the
inputs as stuck agents' are; the levels are an `extra` input (a map no param type holds), and its view, which lists
them by kind, is its own. `builtins::WITH_INPUTS` lists the built-ins whose inputs Settings sets, which follow PDT's
newer versions with their inputs kept.

**Install updates** (`builtins/install_updates.rs`). The update loop (`update::run_update_loop`) only checks; what a
check finds is `update.available`, and this built-in installs it: `host.wait_idle` → `host.install_update`
(`nodes/host_update.rs`). The wait looks again every minute (a `retry` wait, keeping `since` in its output) until
`update::busy_reasons` is empty, inside its window (pdt-core `InstallWindow`), or it has been busy inside the window for
`maxWaitMs` (`timeout`, then an install with `force`; `since` is cleared outside the window, so the clock starts at the
first busy look inside it). Given the update's version it ends by `skipped` once that version is installed or another is
on offer, and while the host's policy isn't auto-when-idle, so under Notify only no run waits.
`install_updates::wake` gives the update on offer a run when the policy turns to auto-when-idle or the workflow is
turned on. `host.install_update` with `automatic` refuses a version pdt-host skipped (`update::skipped`), and finds
its version installed when it runs again after the restart. `update.installed` leads to `host.check_health` (after
`settleMs`: the running version and `Loops::health`) and an optional `notify`; failures and `update.failed` are
`inbox.notice` items. Settings → Updates (`install_updates.rs`, `/api/install-updates`) reads and writes the inputs as
stuck agents' are (its `finish` checks the wait; its reset turns the workflow on and calls `wake`); the policy, channel and check interval stay host settings, since the checks use them.

**Clean up storage** (`builtins/clean_up_storage.rs`). Hourly, `storage.clean` (`which: due`: `auto_cleanup`) then
`storage.measure`; `disk.low` and `disk.recovered` go to `storage.measure` too (`nodes/storage.rs`). Critical leads to
`pickup.pause` and `storage.clean` with `which: finished` (each behind a condition on its input); low or ok to
`pickup.resume`, so a disk no longer critical resumes pickup at the next hourly look even without `disk.recovered`.
Its inputs are the thresholds (`lowGb`, `criticalGb`; `storage.measure`'s config, read back by `storage::thresholds`
for the storage loop, the Storage view and the Inbox) and `pausePickup` / `cleanNow`. Settings → Storage is
`/api/storage/cleanup`. The write floor that stops PDT writing to a full disk stays in pdt-store.

**Settings backed by a built-in's inputs.** A behaviour that became a built-in keeps its home in Settings. The shared
pieces: `builtins/inputs.rs` (`InputsBuiltin`: the params, the template, an optional `finish` that writes what the
inputs derive into the filled-in document and `read` that reads it back, `extra` inputs that are no param; `filled`,
`values_of`, `customized`, `refill`), `builtin_settings.rs` (`router(path, &builtin)`: `GET`/`PUT <path>` and
`POST <path>/reset`, which turns the workflow back on; `router_with` and a `Reset` for a reset that leaves it off or
does more; `change` and `reset` for a route module with a view of its own), and on the web `common/BuiltinSettings.tsx` (`BuiltinSettingsSection`: the controls, the customized
or off notice with Open and Reset to default, the Customize as a workflow link; `ChatChannelRow`) and
`workflows/BuiltinSettingsBanner.tsx` (the editor's banner, one line per built-in). The PM daily checkpoint
(`pm_checkpoint.rs`, `/api/pm-checkpoint`, Settings → This host → PM) is built on them: its `finish` makes the schedule
(`every: 1h` on every day, else a cron expression on the days, at the time or each hour) and `pm.update`'s `after`
(`1d`, or `12h` at a set time). A new one is an `InputsBuiltin`, a route module with one `router` call, its line in
`WITH_INPUTS`, a section using `BuiltinSettingsSection`, and its line in `SETTINGS_HOMES`.

**Host and integration events** are logged where each thing is noticed, each once per occurrence (the event key, or a
level remembered in a host value). `release.published`: `forge_releases.rs` in the 60 s poll, for the projects an
enabled trigger covers (`sensor.releases.<project>` holds the tags seen; the first look only records).
`update.available`: `update.rs` `check_now`, keyed by version. `update.installed`: `update::log_installed` at
startup, from pdt-host's `result.json` and the request it took (`request.taken.json`, whose `automatic` pdtd set).
`update.failed`: the same, for a result that failed or rolled back while pdtd runs the version it ran before.
`disk.low` / `disk.recovered`: `storage::watch_disks` in the storage loop, by pdt-core `disk_news` over the levels in
`sensor.disk_levels`. `connection.failed`: `connection_health::report` when a connection starts failing, and
`forge_login::check` for each refused forge account (keyed by its refusal time). `todo.due`: the Notifier's todo pass,
keyed as the reminder. An integration's own (`scryer.download_finished`) come from its poller.

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
still free. Having checked the caps and the overlap rule itself, it sets `ValidStart::pickup_checked`, so Start
doesn't refuse for the project's running cap (a step's cap may be higher) and refuses only conflicts among overlaps
(`start::refusing_overlaps`): choose and Start keep tickets apart by the same rule. Start refusing for now
(`start_blocked`: a blocker, a conflict) is a skip, never a failure: the ticket's `pickup_hold` says why, and the next
run tries it again. A real failure (a git error, say) is skipped until the ticket changes
(`AppState::auto_start_failed`, with the error, which fills `TicketSummary::pickup_failed` while auto-start is on), or
until Try again (`POST /api/tickets/{id}/retry-auto-start`, `scheduler::forget_failed_start`); a failed write (a full
disk) is tried again on the next run. The
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
(Markdown) is in `body.question`, an agent's in `body.questions` (`inbox::Question`: `ask_user` checks each `question`
is one line and its `recommended` one of its `choices`; items stored before structured choices have plain-string
`choices`, which `Choice` still reads), and `inbox::decorate` adds what is computed per request: ticket key and title,
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
The PM's `inbox_decide` (pdt-api `mcp/tools/inbox.rs`, rules in pdt-core `inbox_decide.rs`) proposes decisions on
many of them at once. In the PM's conversation it asks with one of PDT's own cards (`conversations::held::ask` with
`rows`: the `Request` event carries them, and the Inbox's approval item has them as `body.rows`); the web shows them as a
table (`web/src/common/DecideRows.tsx`, in `conversation/DecideCard.tsx` and the Inbox), and the Allow sends the ticked
rows (`AnswerRequest.rows`) to the waiting call, answered from either place. From a terminal it puts one `confirm` item
in the Inbox with `body.rows` and waits on it (`workflow::agent::confirm_wait`); the Allow sends the ticked rows
(`InboxAnswer.rows`), `workflow::agent::answer_confirm` hands them to the waiting call (a call no longer waiting is run
again from the Allow). Either way the call narrows itself to the kept rows and goes on, and each row is applied by
`ticket_proposal::decide` as the user's, with the PM as `proposedBy` on the events.

**Secrets.** Values live in the OS keychain (or, on Linux without a Secret Service, a file only the user can read) and
in memory; agents and workflows refer to them by name (`run_with_secrets`, `write_secret_file`, `{{secret.x}}` in a
node's config). pdt-core `secret::Redactor` replaces any value, as it is and in its JSON-escaped, hex, URL-encoded and
base64 forms (`secret::forms`), with `[secret:<name>]` in everything the store writes, in pdt-mcp results, in
conversation deltas and in the logs. It also knows PDT's own keychain values (connections, sign-ins, forge logins, the
update token) through the `secrets::Redacting` store that wraps `AppState::secrets`, labelled by kind
(`[secret:connection]`). `run_with_secrets` and `write_secret_file` run in pdtd,
outside the agent's sandbox and permission mode, so they are asking tools (out of Claude's `--allowedTools`; the
Inbox for other agents) unless `Session::asks_nothing`, and the PM's "act without asking" never covers them. Each must
name a secret, and pdt-core `secret::checkout_path` plus the on-disk checks in `vault.rs` keep their folder and file
inside the session's folder, out of `.git`, and off symlinks (the file is opened `O_NOFOLLOW`). Code: pdt-api
`vault.rs` and `secrets.rs`, pdt-store `secrets.rs`.

**GitHub.** The forge (`crates/pdt-github`) signs in with the OAuth device flow or a pasted token; logins are stored
in the keychain (pdt-forge `login.rs`, pdt-api `forge_login.rs`). Its REST calls go through pdt-forge's shared
`RestClient` with GitHub's `Flavor` (names, URLs, headers, rate-limit headers, ETags), and its device flow through
pdt-forge `oauth.rs` with GitHub's endpoints, as GitLab's do: a new forge adds a `Flavor` and an `OAuthApp`, not a
copy of the client. A 401 marks the login rejected only after
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
the document is linked to lets the PM act) or the project or ticket they name, and a document's title, content and comments
come fenced as outside input (`Outside`: one fence per call, `outsideText` when it was used; a sheet's content as JSON
in one block). The documents open beside a conversation are `documents_beside` rows (session, document, position, who opened
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
Each stream also checks the file every 2 s, so a change made outside PDT reaches the open views; the check reads the file only when its modified time or length moved since PDT last saw it (or it was modified in the last few seconds). The provider's file reads, xlsx parsing and writes run on blocking threads (`spawn_blocking`), and each live sheet loads under its own lock, so one big workbook holds up only itself. A doc's editor saves
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
client, `IntegrationInfo.signIn.builtIn`). Their only redirect URI is the website's `/oauth/google` page
(`site/public/oauth/google.js`): `state` is `<nonce>.<base64url(origin)>`, the page forwards only to an origin that
looks like a PDT (loopback, private and Tailscale addresses, `localhost`, `*.ts.net`: the same rule as
`builtin::pdt_origin`, which pdtd applies before issuing one) and asks first (with a stronger warning when it isn't
loopback), and the callback takes only a state it issued, traded with its PKCE verifier. The page also starts each
sign-in, so the code goes back only to the PDT that started it: the sign-in answer carries `relay` (that page), the UI's
sign-in window opens it, the page posts `{pdtRelay: 'ready'}` to its opener, which posts back `{pdtRelayTo: url}`;
the page takes it only when the message's origin (which the browser sets) is the state's, keeps the state in its
`localStorage` for 15 minutes, and later forwards only a state it kept, once. Anyone can start a flow with the built-in
client, but a Google link they hand someone else then stops at the page. A state the page didn't keep (no opener or
storage: a proxy isolating it) is never forwarded: the page shows Google's answer as a code to paste
(`pasteCode`, base64url of `state=…&code=…`), which `POST /api/google/sign-in/code` (`builtin::pasted`, then the
callback's own checks in `routes::finish`) takes; the UI offers it for a sign-in through the page
(`SignIn.pasteCode`), and sends the window to the consent page itself when the page never says it is ready. From a
loopback origin (`builtin::loopback_redirect`: `127.0.0.1`, `[::1]`, `localhost`, over http), a build with the
Desktop app clients too (`PDT_GOOGLE_DESKTOP_*`, `builtin::Desktop`) skips the website: Google comes straight back to
`<origin>/api/google/callback` with a plain nonce as the state, no `relay`. The sign-in it keeps says so
(`SignIn.desktop`), since its refresh token only works with the Desktop client. Signing in with no account (`POST /api/google/sign-in`) adds the connection,
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
picker's token; signing in and the picker are for the web app only (its UI token). The plugin declares both: its
sign-in (`Integration::sign_in`), which Settings → Plugins → Google shows, and its picker, `src/picker.js` served at
`/api/google/picker.js`, which the import dialog loads and runs (the module asks for the token through the web app's
UI-token GET); a picked file is imported like any other. The document page embeds the provider's
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

**Web queries.** Each area keeps its query keys, shared queries and invalidation in one module: `<area>/queries.ts`
(`projectKeys`/`useProjects`, `sessionKeys`/`sessionsQuery`, `runKeys`/`runsQuery`, `workspaceKeys`, `hostKeys`/
`useHostSettings`, `workflowKeys`, `ticketKeys`, `documentKeys`, …; worktrees in `common/worktrees.ts`). Screens use
those hooks and keys rather than a literal key and an inline `useQuery`, so every copy of a query shares one key and
one set of options. A write calls the area's `invalidate…` helper, which also marks stale what depends on it
(`invalidateProjects` the workspaces, `invalidateWorktrees` the sessions and workspaces, `savedWorkflow` the list,
the versions and the Settings sections that show a built-in). Poll intervals are the tiers in `common/poll.ts`
(`POLL.live` 2 s for what's on screen, `steady`, `slow`, `paged`, `idle`); what the notification stream pushes polls
with `streamPoll`, slowly while the stream is connected.

**Web menus and opening things.** A ticket's menu, wherever it shows (backlog row, board card, peek, tab, phone top
bar), comes from `ticketMenu` in `tickets/actions.ts`; a screen passes the options it can offer (Watch, Start next
part, Merge…, Create issue…, Abandon…, rank) and a new ticket action is added there once. Opening a ticket, session,
changes tab, workflow, Settings page or plugin goes through `common/open.ts` (the contexts AppShell provides and plain
navigations), and a URL outside PDT through its `openExternal` (`noopener,noreferrer`), not `window.open`. What an
Inbox item opens is `inboxTargets` in `inbox/model.ts`.

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
| Conversations | pdt-api `conversations/`; pdt-core `conversation/`; pdt-agents `adapter/` | `web/src/conversation` |
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
