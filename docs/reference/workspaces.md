# Workspaces, projects, worktrees, runs

Spec: docs/superpowers/specs/2026-10-17-workspaces-design.md. Code: pdt-api `workspaces.rs`, `projects.rs`,
`worktrees.rs`, `files.rs`, `folders.rs`, `runs.rs`, `run_manager.rs`; pdt-git; web/src/workspaces, web/src/palette.

## Workspaces and tabs

- Every project has its own workspace (made with it, deleted with it) plus free-form ones. A workspace is a strip of
  tabs, each showing one session, run, ticket or worktree's changes; a session or run has at most one tab (opening it
  elsewhere moves it), and closing that tab stops and removes it (`DELETE /api/tabs/{id}`; the UI asks first while it
  runs; ticket and changes tabs only close).
- New sessions/runs open a tab at the end of their project workspace unless the request has
  `tab: {workspaceId, position}`. The workspace and tab on screen are per browser (localStorage); workspaces and tabs
  are shared.
- The Sessions sidebar is herdr-style (web/src/workspaces/Sidebar.tsx): workspaces on top, then Tickets (the
  workspaces a ticket's Start made, `Workspace.ticket` from `attempts.workspace_id`: ticket title, key chip that opens
  it, PR badge, agent state; reordered and moved up/down within their own section), agent sessions below; tabs,
  workspaces and agents have right-click menus (long press on touch) drawn in a portal at the pointer.
- Projects are added and configured in the Settings view.
- Shortcuts: Ctrl+] then one key (web/src/workspaces/keys.ts), caught on window in the capture phase; Ctrl+] twice
  reaches the terminal. The command palette (web/src/palette) opens with Ctrl+] k or ⌘K on macOS (never Ctrl+K: the
  terminal's kill-line).
- On phones (< 768px) the list column is a left sheet opened from the workspace name; the rail is Sessions, Tickets,
  Search (palette), Settings (Files is in the palette's Go to).

## Projects and git (workspaces spec §6)

- Projects have a unique key (`PDT`; suggested from the name, backfilled when the store opens).
  `POST /api/projects/clone` clones into the host's Projects folder (default `~/work`) with `GIT_TERMINAL_PROMPT=0`;
  removing a project stops its sessions and runs and never touches files. Projects carry `githubRepo` from `origin`
  (no GitHub API calls).
- A project's base branch (what new worktrees and tickets branch from and are reviewed against) defaults to the
  repository's default branch: `origin/HEAD`'s, else a local `main` or `master`, else the checked-out branch (pdt-git
  `default_branch`). The project's Settings page changes it (`PATCH /api/projects/{id} {baseBranch}`, which must name
  a local branch or one on `origin`); existing attempts keep the base they started with.
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
- `DELETE /api/worktrees/{id}` refuses while busy (`worktree_busy`) or dirty without `force` (`worktree_dirty`); the
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
