# Workspaces, projects, worktrees, runs

Spec: docs/superpowers/specs/2026-10-17-workspaces-design.md. Code: pdt-api `workspaces.rs`, `projects.rs`,
`worktrees.rs`, `files.rs`, `folders.rs`, `runs.rs`, `run_manager.rs`; pdt-git; web/src/workspaces, web/src/palette.

## Workspaces and tabs

- Every project has its own workspace (made with it, deleted with it) plus free-form ones. A workspace is a strip of
  tabs, each showing one session, run, ticket or worktree's changes; a session or run has at most one tab (opening it
  elsewhere moves it), and closing that tab stops and removes it (`DELETE /api/tabs/{id}`; the UI asks first while it
  runs; ticket and changes tabs only close).
- New sessions/runs open a tab at the end of their project workspace unless the request has
  `tab: {workspaceId, position}`. The workspace and tab on screen are in the URL (`/sessions/<workspace>/<tab>`, see
  "URLs" below), and per browser (localStorage) for when the URL doesn't say; workspaces and tabs are shared.
- The Sessions sidebar is herdr-style (web/src/workspaces/Sidebar.tsx): workspaces on top, then Tickets (the
  workspaces a ticket's Start made, `Workspace.ticket` from `attempts.workspace_id`: ticket title, key chip that opens
  it, PR badge, agent state; reordered and moved up/down within their own section), agent sessions below; tabs,
  workspaces and agents have right-click menus (long press on touch) drawn in a portal at the pointer.
- Projects are added and configured in the Settings view.
- Shortcuts: Ctrl+] then one key (web/src/workspaces/keys.ts), caught on window in the capture phase; Ctrl+] twice
  reaches the terminal. The command palette (web/src/palette) opens with Ctrl+] k or ⌘K on macOS (never Ctrl+K: the
  terminal's kill-line).
- The desktop rail (web/src/shell/IconRail.tsx) is icons only, with tooltips on the right; the toggle at its bottom
  expands it to show each view's name beside its icon (badges beside the names, no tooltips) and collapses it again.
  The state is per browser (localStorage `pdt.rail.expanded`).
- On phones (< 768px, PDT-112) there is no rail: a top bar has ☰ on a view's top-level screen and ← (Back, or up to
  the view when the screen was opened from a link) on a detail screen, the title, 🔔 Needs you (the Inbox count, opens
  the Inbox), ⌕ (the command palette) and ⋯ (the screen's actions: the shell's, plus any a screen adds with
  `useTopBarActions`). ☰ opens the drawer (web/src/shell/PhoneDrawer.tsx): the project (the Tickets view's), the views
  in the rail's order with the Inbox count, the workspaces folding open to their tabs (folded, a workspace shows its
  loudest state), the agents, and Settings with the host's connection state. It closes on a tap outside, a swipe left,
  Escape, choosing, or Back (it adds a history entry, which choosing replaces). Sessions, Inbox, Workflows, Personas,
  Files and Settings show their list column as their top-level screen (`/sessions` is the workspaces and agents);
  choosing an item pushes its route. The PM's earlier sessions are under its ⋯, the Tickets' projects and filters
  under its title. ← from a workflow run goes up to the workflow's runs, and from a file or folder to the folder it
  is in.
- Workflows, Personas and Files are view-only on phones (PDT-120): Workflows as in [workflows.md](workflows.md); a
  persona shows its setup (role, agent, model, permission mode, tools, instructions rendered) and versions, with no
  New persona, forms, Archive or Delete; Files drills down one folder per screen (`/files/src/`), the checkout picker
  on the top one, and a file opens read-only with its lines wrapped.
- Sessions on a phone (PDT-114): `/sessions` is the Sidebar full screen with 56px rows and no dragging (long press
  opens the same menus); its ⋯ has New workspace and New conversation (the host's default agent, in the drawer's
  project or the first, main checkout). A workspace opens on its last tab; its ⋯ has New tab, Tab overview, Open ticket
  (a ticket workspace: its ticket screen), Rename (in the top bar) and Close workspace (both for free-form and ticket
  workspaces; closing asks, moves the tabs back to their projects, and goes Back), and the terminal panel. The tab
  strip swipes between tabs and has ⧉ (no ‹ ›): the tab overview (web/src/workspaces/TabOverview.tsx), a grid of
  cards with the tab's state and a live snippet: an open question (the Inbox's question or approval for the session,
  else the conversation's open request), else the agent's last line (`GET /api/sessions/{id}/events?last=20`), and
  the branch's `+added −removed`. A tap opens the tab, a long press has the tab's menu, Back closes it. A session's
  header is one line (state, title, branch chip) with Changes, Conversation/Terminal, Paste, Compose, Font size (a
  terminal's, [agents.md](agents.md) "Terminals"), Stop, Restart, Make ticket and Remove under its ⋯; a run's is one line with its state, Stop and Restart under ⋯.

## Right panel and todos (PDT-125)

- On desktop a panel sits right of the main area (web/src/shell/RightPanel.tsx). Open it is 320px wide and drags
  wider (240–640px; the divider also takes Left/Right); collapsed it is a 40px strip with a checklist icon and the count
  of open todos. The strip, the panel's collapse button, ⌘. (Mac) and Ctrl+] t open and close it. Open/closed and
  width are kept per browser (`pdt.rightPanel.open`, `pdt.rightPanel.width`). It has tabs, starting with Todos.
- Phones have no panel: ☑ in the top bar opens the same content in a right sheet.
- The todo list (web/src/todos): "Add a todo…" (Enter adds it at the top), a filter (All, or the project on screen,
  which new todos then join; kept as `pdt.todos.filter`), rows with a checkbox, the title (click to edit), a drag
  handle (also Alt+↑/↓) and ⋯ (Edit, Link to a ticket…, Unlink, Project, Delete), chips for the project key and the
  linked ticket (which opens it). Done todos fold into "Done (n)" with Clear done.
- The server keeps them in `todos` (`GET/POST /api/todos`, `PATCH/DELETE /api/todos/{id}`; GET filters by `project`
  and `done`). Order is a float `position` (a move takes the midpoint of its new neighbours). A linked ticket is always
  in the todo's project: linking one sets the project, moving the todo to another project drops the link, and deleting
  the ticket or project only removes the link. The PM reads them with `list_todos` ([agents.md](agents.md) "The PM").

## URLs

What is on screen is in the URL path (web/src/shell/route.ts, router.ts), so browser Back/Forward walk through what
was opened and a reload (or a link) opens the same screen. Selecting a view, a workspace or tab, a ticket peek, an
Inbox item, a PM session, a workflow tab or run, a persona, a file or a Settings page pushes a history entry (a click
that switches view and selects in one go makes one entry); fill-ins (the tab after a closed one, `/sessions` naming
its tab, the Inbox picking an item) replace it.

| Path | Shows |
|---|---|
| `/` | The PM on phones, Sessions otherwise |
| `/pm`, `/pm/<sessionId>`, `/pm/notes` | The PM's current session, an earlier one, the notes |
| `/sessions`, `/sessions/<workspaceId>/<tabId>` | A workspace and tab (`/sessions` alone: the browser's last choice) |
| `/inbox`, `/inbox/<itemId>` | The Inbox, an item |
| `/tickets` (`?view=board`), `/tickets/<key>` | Backlog (or Board), with the ticket peeked at (by key; an id is replaced by the key) |
| `/workflows/<id>/(outline\|runs\|graph\|text\|versions)`, `/workflows/<id>/runs/<runId>`, `/workflows/templates/<id>` | A workflow's tab, a run, a template |
| `/personas/<id>` (`new`: the create form) | A persona |
| `/files/<path>` (`?project=<key>&worktree=<id>`) | A file, in the checkout named (else the current tab's); a path ending in `/` is a folder (its own screen on a phone) |
| `/settings`, `/settings/host(/<section>)`, `/settings/notifications`, `/settings/connections`, `/settings/projects/<key>(/<section>)` | Settings: the index (This host on desktop, the list of pages on a phone, with Add project) and its pages; a section (its heading, `danger-zone`) is scrolled to on desktop, and is a screen of its own on a phone, where This host's and a project's pages are the list of their sections; ← goes from a section to its page, then to the index |

- Switching to a view shows it as it was last left (its last path); Tickets opens in the browser's last Backlog or
  Board. localStorage keeps only what a path doesn't say.
- pdtd serves index.html for every path outside `/api` and the built files; the Vite dev server does the same for
  the paths above (web/vite.config.ts).
- Ticket keys of the host's projects in Markdown (conversations, ticket descriptions) link to `/tickets/<key>`; clicks
  on links to the app's own paths navigate in place (modified clicks open a new tab).

## Projects and git (workspaces spec §6)

- A **Personal project** (Phase 7b; `kind: personal`, `POST /api/projects {name, key?, kind: "personal"}`) has no
  repository: its folder is `<data dir>/personal/<project id>` (its `repoPath`; `baseBranch` empty). Worktrees, runs
  and tests, pull and base-branch changes answer 409 `personal_project`, branches are empty, the Files view lists the
  plain folder, and its tickets start in folders ([tickets.md](tickets.md#start-phase-4b)). Web: Add project's
  "Personal (no repo)", and the project page and Start dialog leave out what needs git.
- The **host project** (PDT-81; `kind: host`, key `_HOST`, folder `<data dir>/pm`) holds the PM's sessions
  ([agents.md](agents.md#the-pm-pdt-81)), which run in the work folder that holds the projects' checkouts. PDT makes
  it on first use; it is never listed, has no workspace, and `POST /api/projects` refuses `kind: host`.
- Projects have a unique key (`PDT`; suggested from the name, backfilled when the store opens).
  `POST /api/projects/clone` clones into the host's Projects folder (default `~/work`) with `GIT_TERMINAL_PROMPT=0`;
  removing a project stops its sessions and runs and never touches files. Projects carry `githubRepo` from `origin`
  (no GitHub API calls).
- A project's base branch (what new worktrees and tickets branch from and are reviewed against) defaults to the
  repository's default branch: `origin/HEAD`'s, else a local `main` or `master`, else the checked-out branch (pdt-git
  `default_branch`). The project's Settings page changes it (`PATCH /api/projects/{id} {baseBranch}`, which must name
  a local branch or one on `origin`); existing attempts keep the base they started with. Its branch picker, like
  the run-a-branch picker, lists `main` (then `master`, then the current base) first and filters as you type
  (`Select searchable`; web/src/common/branchOptions.ts).
- The Files view reads checkouts through `/api/projects/{id}/files` (git's file list, `.gitignore` honoured; paths
  stay inside the checkout, `.git` refused). A Markdown file shows rendered (`Markdown`, as ticket text) with a
  Rendered | Source switch.
- Folder picker (PDT-5): `GET /api/host/folders` (`path` absolute or `~…`, empty = home; `hidden`; `nearest` walks up
  to a folder that exists) lists one folder's subfolders, never files, in the host's path form whatever the browser's
  OS (pure rules in pdt-core `host_path`, tested for both styles), marking git checkouts and projects;
  `POST /api/host/folders` `{parent, name}` makes one folder. Web: `FolderField` (web/src/common; path input +
  Browse… dialog around `@ui` `FolderPicker`) in Add project, Clone → Into folder and Host settings → Projects folder;
  it writes `~/…` (`~\…` on Windows), which Add project accepts too.

## Worktrees

- Worktrees live inside the repo at `.worktrees/<branch with / replaced by ->` (listed in .git/info/exclude; pdt-core
  `worktree.rs`), so `feature/x` lives in `.worktrees/feature-x`. Folders there made with plain git are adopted.
- `DELETE /api/worktrees/{id}` refuses while busy (`worktree_busy`), while a ticket's active attempt works in it
  (`worktree_attempt`: finish the ticket or abandon the attempt first) or dirty without `force` (`worktree_dirty`); the
  project's Settings page and the merge banner use it.
- A folder there that is not a git worktree for that branch makes Run on that branch fail with
  `worktree_path_taken` (also a branch whose folder name another branch already took, e.g. `a/b` and `a-b`).

## Runs

- "Run" on a worktree/branch starts a `.pdt/project.toml` run on a port from the host's range (stable 8400–8499, dev
  profile 8500–8599) with its own data folder; this repo's "pdt" run is a dev instance of PDT. Steps run in the
  checkout (`sh -c` on macOS, a generated .cmd on Windows); only `$NAME`/`${NAME}` of PDT's variables are expanded.
- A run with `detach = true` (PDT-32) starts its command outside any PTY (pdt-agents `detached`: Windows job object,
  unix `setsid` process group; stdout/stderr to `<data dir>/runs/logs/<run id>.log`, which the Output terminal follows
  read-only), so it survives pdtd restarts and updates: `resume_runs` reattaches while any of its processes lives
  (`runs.process`) and marks it stopped otherwise.
- Format, variables, host overrides, Windows traps and API: docs/run-config.md. To configure a project, use Settings →
  the project → Runs on this host → Set up with an agent (it starts a session with docs/prompts/set-up-project.md).
