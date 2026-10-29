# Workspaces, projects, worktrees, runs

Spec: docs/superpowers/specs/2026-10-17-workspaces-design.md. Code: pdt-api `workspaces.rs`, `projects.rs`,
`worktrees.rs`, `files.rs`, `folders.rs`, `runs.rs`, `run_manager.rs`; pdt-git; web/src/workspaces, web/src/palette.

## Workspaces and tabs

- Every project has its own workspace (made with it, deleted with it) plus free-form ones. A workspace is a strip of
  tabs, each showing one session, run, ticket or worktree's changes; a session or run has at most one tab (opening it
  elsewhere moves it), and closing that tab stops and removes it (`DELETE /api/tabs/{id}`; the UI asks first while it
  runs; ticket and changes tabs only close).
- New sessions/runs open a tab at the end of their project workspace unless the request has
  `tab: {workspaceId, position}`; a run on the worktree or branch of an active attempt that has a workspace opens in
  that workspace instead (PDT-160). The workspace and tab on screen are in the URL (`/sessions/<workspace>/<tab>`, see
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
  The state is per browser (localStorage `pdt.rail.expanded`). Above the toggle, a System | Light | Dark theme switch
  (PDT-206; icons, stacked while the rail is collapsed, a row when expanded). The theme is per browser (localStorage
  `pdt.theme`, the same setting as the command palette's Theme commands). Open terminals (runs, sessions, the shell
  panel) recolour as soon as the theme or, on System, the OS colour scheme changes (PDT-222).
- On phones (< 768px, PDT-112) there is no rail: a top bar has ☰ on a view's top-level screen and ← (Back, or up to
  the view when the screen was opened from a link) on a detail screen, the title, 🔔 Needs you (the Inbox count, opens
  the Inbox), ⌕ (the command palette) and ⋯ (the screen's actions: the shell's, plus any a screen adds with
  `useTopBarActions`). ☰ opens the drawer (web/src/shell/PhoneDrawer.tsx): the project (the Tickets view's), the views
  in the rail's order with the Inbox count, the workspaces folding open to their tabs (folded, a workspace shows its
  loudest state), the agents, the theme switch (with names; the rail's setting), and Settings with the host's
  connection state. It closes on a tap outside, a swipe left,
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

## Right panel and todos (PDT-125, PDT-143)

- On desktop a panel sits right of the main area (web/src/shell/RightPanel.tsx). Open it is 320px wide and drags
  wider (240–640px; the divider also takes Left/Right); collapsed it is a 40px strip with a checklist icon and the count
  of open todos. The strip, the panel's collapse button, ⌘. (Mac) and Ctrl+] t open and close it. Open/closed and
  width are kept per browser (`pdt.rightPanel.open`, `pdt.rightPanel.width`). It is the todo list only (PDT-143): the
  user's notes have a view of their own ([agents.md](agents.md) "The user's own notes").
- Phones have no panel: ☑ in the top bar opens the same list in a right sheet ("Todos").
- The todo list (web/src/todos): a header ("Todos", the open count, a scope dropdown, the collapse button), the add
  box, then the open todos in groups and the done ones folded into "Completed · n" with Clear. The scope is **All
  projects** or the project on screen (which new todos then join), kept as `pdt.todos.filter`. Opening the panel with
  Ctrl+] t focuses the add box; ⌘. only toggles it.
- Rows (TodoRow.tsx; in the gallery): an 18px round checkbox (web/ui `Checkbox shape="round"`, its border red when
  overdue), the title (click to open the todo, below), the first line of its notes, and a muted meta line: the due day where the group heading doesn't already say
  it (red when overdue; a time today in amber), the repeat rule ("Every Sat", "Mon, Thu"), the linked ticket's key
  (opens it), in All the project key when there's no ticket, and a notes icon. Hairlines separate rows from the title on. On
  hover a row fills and shows a calendar button (the due picker) and ⋯ (Open, Edit, Due date…, Repeat…, Stop repeating,
  Link to a ticket… / Unlink, Project, Delete). Rows drag (with a fine pointer; on touch from a grip handle) or
  Alt+↑/↓ to reorder within their group; a drop from another group does nothing.
- Completing: a ticked row is PATCHed at once but stays in its place for 800 ms, struck through on a faint green
  fill, then moves to Completed; un-ticking it in that time keeps it where it is.
- Due dates (PDT-140): optional, the user's local day (`dueOn`, `YYYY-MM-DD`) with an optional time (`dueTime`,
  `HH:MM`). A todo with a time is overdue once that time passes. Open todos group as **Overdue** (with **Move to
  today**, which moves each overdue todo to today, keeping its time only if that is still ahead), **Today**,
  **Tomorrow**, **Next 7 days** (2–7 days out), **Later** and **No date**, each with its count, in the manual order;
  headings only once something has a date. Rules in web/src/todos/model.ts.
- The due picker (DuePicker.tsx, PDT-143) is one popover anchored to whatever opened it (the row's date, the
  calendar button, ⋯ Due date…, quick add's Date tag). Panel A: a type-a-date box (quick add's parser; Enter applies
  it), quick picks (In an hour: now + 60 min to the nearest 5, with its time, so late at night tomorrow; Today;
  Tomorrow; This weekend, the coming Saturday, or Next weekend on a Saturday or Sunday; Next week, the coming Monday;
  No date when one is set; all but In an hour drop the time), a month calendar (web/ui `Calendar`: Monday first, past
  days can't be picked except the set one, today in amber; a day keeps the time) and two footer pills, Time and
  Repeat, each with × to clear. Panel B: 09:00/12:00/15:00/18:00 slots, a time field and No time. Panel C: Doesn't
  repeat, presets from the chosen day (every day, weekday, week, 2 weeks, month on the nth), the current rule if it
  is none of those, Custom… (the Repeat editor) and what the next one counts from. A repeat on an undated todo dates
  it today. Changes PATCH at once; a quick pick or a calendar day closes the picker.
- Quick add (QuickAdd.tsx, parse.ts, PDT-143): the add box grows when focused. Dates, times and repeats typed into the
  title are picked out on the client as you type (whole words only, preferring the end of the text; `today`,
  `tonight`, `tomorrow`/`tmr`, weekdays and `next fri`, `next week`, `this weekend`, `in N days|weeks|months`, `nov 3`
  / `3 nov`; `9am`, `3:30pm`, `at 15:30`, `noon`, `midnight`; `every day|weekday|week|month`, `every mon, thu`,
  `every 2 weeks`, `daily`, `weekly`, `monthly`; `may`, `sun` and `sat` only with a day number or after on/next/every;
  a time alone is today if still ahead, else tomorrow), highlighted, and shown as tags under the text; × on a tag
  keeps its words in the title for as long as they stay in the text (a day picked by hand over typed date
  words works the same way). Unset ones are dashed tags: Date and Repeat (the picker),
  Project and Ticket. Enter adds the todo with the parsed title (or the whole text if nothing is left), date, time,
  repeat, project and ticket.
- When an open todo comes due (local midnight on its day, or its time) pdtd makes one `todo.due` notification
  ([workflows.md](workflows.md) "Notifications"); opening it opens the panel (the phone's sheet) with that todo
  open (below), switching the list's filter to All if it hid it, so Back finds its row (with a ring). The list uses
  each such request once (AppShell drops it), so reopening the panel later doesn't open it or change the filter again. Done, deleted or re-dated clears it; a new date reminds again.
  pdtd works out the moment in the host's own time zone.
- A todo's notes (PDT-227, TodoView.tsx, useTodoDraft.ts): a todo has a Markdown `body` (empty when none; at most
  64 KB, pdt-core `valid_body`; `body` in POST and PATCH, `""` clears it). A click on a row's title (or ⋯ Open) opens
  the todo in place of the list, with ← Todos back to it; on a phone the sheet goes full screen while one is open. The
  open view has the done box, the title (an emptied title comes back), Due date (the picker), Repeat, Link a ticket
  (or the linked ticket's key, which opens it), Project, ⋯ (Stop repeating, Unlink, Delete) and the notes in the
  Markdown editor (PDT-187's `MarkdownEditor`). Title and notes save the way notes do: 800 ms after typing stops, on
  blur, Enter (title), Esc or Back; a poll never overwrites an unsaved edit. Renaming in place is ⋯ Edit. A row with
  notes shows their first line, muted (Markdown marks dropped), and a notes icon in its meta line. A repeating todo's
  next one keeps the body, and `list_todos` returns it.
- Repeating todos (PDT-141): a todo with a due date can repeat: every N days (daily), every weekday (Mon–Fri), every
  N weeks on chosen days (weekly), or each month on a day (1–31; a shorter month uses its last day, and the next month
  goes back to the rule's day). The rule is `recurrence` (`{rule: {every: "day", interval} | {every: "weekday"} |
  {every: "week", interval, days: [1–7, Monday first]} | {every: "month", day}, from: "due" | "done"}`), stored as
  JSON on the todo and validated and computed in pdt-core `todo.rs`. Checking it off marks it done and makes the next
  todo (same title, notes, time, project, ticket, rule and place in the list), due on the rule's next day: from the due date
  (the default) the series keeps its schedule but skips days already past, so the next one is never due today or
  earlier; from when it's done, it counts from the day it was checked off. A todo makes its next one once (the store
  remembers it as `next_id`, in the same transaction), so two checks at once, or un-checking and checking it again,
  don't make another. The next one gets its own due reminder. Its row shows a repeat icon and the rule; ⋯ has
  Repeat… (the editor: the rule, what the next one counts from, and a first due date for an undated todo) and Stop
  repeating (`recurrence: null`). Deleting an open repeating todo asks: Only this one
  (`DELETE /api/todos/{id}?series=only`, the default: the next one takes its place) or Stop the series
  (`?series=series`). Clearing the due date stops it repeating. Done rows show no rule.
- The server keeps them in `todos` (`GET/POST /api/todos`, `PATCH/DELETE /api/todos/{id}`; GET filters by `project`
  and `done`). `dueOn`/`dueTime` are validated in pdt-core `todo.rs`; in a PATCH `dueOn: ""` clears the date and its
  time (and recurrence), `dueTime: ""` only the time, and a time needs a date. Order is a float `position` (a move takes the midpoint of its new neighbours). A linked ticket is always
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
| `/pm`, `/pm/earlier`, `/pm/<sessionId>` | The PM's current session, the sessions that ended, one of them |
| `/notes`, `/notes/pm`, `/notes/<id>`, `/notes/new` | The Notes view: my notes, the PM's memory, one note, a new one (created by its first save, which replaces the path with its own) |
| `/pm/notes/<id>`, `/pm/notes` | Old paths (ticket descriptions link to them): replaced by `/notes/<id>` and `/notes/pm` |
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
- Editing in the Files view (PDT-157, not on phones): **Edit** turns the file's CodeMirror editable (same
  highlighting; a file with CRLF endings keeps them), with Save (⌘S / Ctrl+S) and Cancel (Done once saved); a Markdown
  file is edited in `MarkdownEditor` instead (styled source with line numbers and list continuation, as notes, PDT-187)
  and gets an Edit | Preview switch. While there are unsaved changes, navigating in the app asks first (the router's
  `useNavigationGuard`), Back and Forward are undone and taken again only once the user says to discard (by the
  history entries' `n`), and closing the page asks the browser's question; Rename and Delete of the edited file
  (or its folders) are held until the edits are saved or cancelled. The file's
  content carries `hash` (SHA-256 of its bytes); `PUT /api/projects/{id}/files/content` `{path, worktreeId?, text,
  baseHash?}` writes only when the file on disk still has `baseHash`, else 409 `file_changed`, and the UI offers
  Reload (the file as it is now) or Show diff (on disk → yours, the review's `CodeDiff`) + Overwrite (saves against
  the hash just read, so it never overwrites blindly either). Without `baseHash` it creates a file that must not exist
  (409 `file_exists`), making its folders. `POST …/files/rename` `{from, to}` moves a file or folder to a free path;
  `DELETE …/files/content?path=` deletes a file (not a folder), after a confirm. New file is the button beside the
  checkout picker and in a row's right-click menu (with Rename and Delete; also in the file's ⋯). Writes refuse `..`,
  a `.git` segment anywhere (submodules too), and any path whose nearest existing folder or itself resolves, through
  symlinks, outside the checkout; binary files and files over 1 MB stay read-only (`not_editable`, 413 `too_large`).
  While editing, a banner names the agent sessions working in that checkout (running, activity `working`, same
  worktree); after a save the toast offers to tell them: `POST …/files/tell` `{sessionId, paths}` queues a user
  message through the `session.send` outbox (it waits until the agent is free). Saved changes are plain uncommitted
  changes: the editor never commits.
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
- Starting a run (`POST /api/projects/{id}/runs`) whose checkout (worktree, or the main checkout on the same branch),
  name and kind already have a finished run restarts that run in its tab and answers 200 instead of making a second
  one (PDT-160); a run still going gets a new one beside it (201). A run by `branch` records the branch's worktree.
- A run with `detach = true` (PDT-32) starts its command outside any PTY (pdt-agents `detached`: Windows job object,
  unix `setsid` process group; stdout/stderr to `<data dir>/runs/logs/<run id>.log`, which the Output terminal follows
  read-only), so it survives pdtd restarts and updates: `resume_runs` reattaches while any of its processes lives
  (`runs.process`) and marks it stopped otherwise.
- Format, variables, host overrides, Windows traps and API: docs/run-config.md. To configure a project, use Settings →
  the project → Runs on this host → Set up with an agent (it starts a session with docs/prompts/set-up-project.md).
