# Personal Dev Team — Workspaces and Git Integration (Phase 3) Design

Status: draft from brainstorming on 2026-10-17, awaiting the user's review. This amends the main spec,
`2026-10-14-personal-dev-team-design.md`: it is Phase 3.

## 1. Purpose

Day-to-day agent work in PDT without herdr. Today PDT shows one session at a time, picked from a list for one
project. After this phase the user keeps their agents, shells and runs open as tabs, grouped into workspaces,
switches between them from the keyboard, and sees at a glance which agents are working, waiting on them, or done.
The same workspaces and tabs appear on the desktop, the laptop and the phone.

There are no split panes: a tab shows one session or run, full size. (The bottom shell panel stays as it is.)

The phase ships as two PRs under this one spec:

- **3a — Workspaces** (§2–§5): workspaces, tabs, the session tree, keyboard, status at a glance, drag to reorder.
- **3b — Git integration** (§6): project keys, clone by URL, GitHub links, adopting and removing worktrees, removing
  projects, the folder view.

### Success criteria

- The user can keep three agents and a shell for one branch open as tabs and switch between them without the mouse.
- A free-form workspace can hold sessions from two projects.
- Opening PDT on another device shows the same workspaces and tabs.
- From any tab the user can see which other tabs and workspaces have an agent that needs them, and jump to it.
- A project can be added by cloning a GitHub URL, and every branch links to GitHub.

## 2. Model (3a)

### 2.1 Workspaces

- A **workspace** has a name, an order, and an ordered list of tabs.
- Two kinds:
  - **Project workspace:** exactly one per project, made when the project is added (and for existing projects by the
    migration). Named after the project; it can't be deleted while the project exists.
  - **Free-form workspace:** made and named by the user. It can hold sessions and runs from any project.
- Both kinds behave the same once made.

### 2.2 Tabs

- A tab shows **one session** (agent or shell terminal, as `SessionView` today) **or one run** (its output and
  controls, as `RunView` today).
- A tab's title is the session's or run's title; renaming the tab renames the session (as today).
- A session or run has at most one tab per workspace, but can have tabs in several workspaces. All viewers share the
  one PTY; it takes the size of whichever view most recently attached or resized (last writer wins, as today).
- Closing a tab never stops the session or run; it stays in the tree. Removing a session or run closes its tabs.

### 2.3 Where tabs come from

- **Launching** a session or starting a run from the tree opens it as a new tab in its project workspace, after the
  current tab if that's the workspace on screen, else at the end, and switches to it.
- **Launching** from the "+" in the tab strip (§4.3) opens the new session in the workspace on screen.
- **Clicking** a session or run in the tree switches to its tab in the workspace on screen if it has one; otherwise
  to its tab in its project workspace, opening one there if needed.
- **Opening elsewhere:** a tab's menu has "Open in workspace…" (pick or make a free-form workspace); a session or run
  can also be dragged from the tree onto a workspace's tab strip.
- The migration opens a tab in each project workspace for every session and active run the project has, grouped by
  place (Repository first, then worktrees) so a branch's tabs sit together.

### 2.4 Shared and per-device state

- Workspaces, tabs and their order are stored in pdtd's database and shared by every device.
- Which workspace and tab a device is looking at, and the tree's collapsed groups, are per-device (browser storage),
  so the phone doesn't yank the desktop around.
- Tab operations are small and independent (open, close, move), so there's no conflict handling beyond "last write
  wins" on order. Clients poll workspaces like sessions (every 2 s).

## 3. Session tree (3a)

The Sessions list column becomes a tree of every project:

```
▾ Personal Dev Team                 ● needs you
  ▾ Repository
      claude  · Working
      shell
  ▾ phase-3-workspaces
      claude  · Needs you •
      codex   · Idle
      ▸ Run: pdt  (ready)
▸ Other project
```

- Project → place (Repository, then worktrees) → sessions and runs. Groups collapse; collapsed state is per device.
- Each group header shows the most urgent activity under it (needs you > working > idle) and the unseen dot.
- Launch buttons (Shell / Claude / Codex, skip-permissions toggle) stay on each place's header; Run stays as today.
- Project settings, the run-branch form and add-project move into the project's header menu.
- Sessions and runs with a tab in the workspace on screen are marked in the tree.

## 4. Workspace view (3a)

### 4.1 Layout

- The work area's header gets the **workspace switcher** (a dropdown listing workspaces with their status, plus
  "New workspace") and the **tab strip** (tabs with status dot and activity, "+" for a new session).
- Below it, the current tab's session or run, as today.

### 4.2 Status at a glance

- Each tab shows its session's status dot, activity and unseen dot, as the session list does today.
- A workspace's status is the most urgent activity of its tabs: needs you (amber) > working (green) > idle (gray),
  same colors as the session activity badge (remote-host spec §5); the unseen dot shows when any tab is unseen.
- The browser tab title shows a count of sessions that need the user (e.g. `(2) PDT`).

### 4.3 New tab

- "+" in the tab strip opens a small chooser: **Claude**, **Codex**, **Shell**, **Run…**, or an existing session or
  run (type to filter). One keypress picks Claude/Codex/Shell (`c`, `x`, `s`).
- The new session starts in the same place (project and worktree) as the current tab's session or run. In an empty
  free-form workspace the chooser asks for the project and place first.
- The skip-permissions toggle is in the chooser too.

### 4.4 Keyboard

A prefix key, then one key, like tmux. The prefix is caught before the terminal sees it; pressing it twice sends it
through to the terminal.

| Keys (after the prefix) | Action |
|---|---|
| `c` | new tab (opens the chooser) |
| `1`–`9` | go to tab N |
| `n` / `p` | next / previous tab |
| `w` | workspace switcher (type to filter, Enter to go) |
| `x` | close the tab (the session keeps running) |
| `r` | rename the tab |
| `a` | jump to the next session that needs you, in any workspace |
| `?` | show these keys |

- **Default prefix: Ctrl+]**, chosen because it is unbound in zsh/readline and not taken by browsers or macOS
  (confirm against current Claude Code and Codex keybindings while building). Ctrl+B (tmux's) is taken by Claude
  Code (background tasks). The prefix is a host setting.
- Ctrl+` keeps toggling the bottom shell panel. Browser-reserved keys (Cmd/Ctrl+T, W, N) are left alone.

### 4.5 Drag to reorder

- Drag tabs to reorder them, or onto another workspace in the switcher to move them there.
- Drag workspaces in the switcher to reorder them.
- Drag a session or run from the tree onto a tab strip to open it there (§2.3).
- Everything dragging does has a keyboard or menu equivalent ("Move to workspace…", "Move left/right").

### 4.6 Phone

- The tab strip scrolls horizontally; swiping left/right on the header moves between tabs. The workspace switcher is
  a full-screen list.
- No drag on the phone; the tab menu has the move actions instead.
- The mobile terminal key bar stays as it is.

## 5. API and storage (3a)

Migration `0008_workspaces.sql` (new tables; existing migrations untouched):

```sql
CREATE TABLE workspaces (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    project_id TEXT UNIQUE REFERENCES projects(id) ON DELETE CASCADE,  -- set for project workspaces
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE workspace_tabs (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id   TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    run_id       TEXT REFERENCES runs(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    CHECK ((session_id IS NULL) <> (run_id IS NULL)),
    UNIQUE (workspace_id, session_id),
    UNIQUE (workspace_id, run_id)
);
```

Routes (errors are `{code, message}`):

| Route | Purpose |
|---|---|
| `GET /api/workspaces` | all workspaces with their tabs, in order |
| `POST /api/workspaces` | new free-form workspace `{name}` |
| `PATCH /api/workspaces/{id}` | rename, move `{name?, position?}` |
| `DELETE /api/workspaces/{id}` | delete a free-form workspace (`409 project_workspace` for a project's) |
| `POST /api/workspaces/{id}/tabs` | open a tab `{sessionId \| runId, position?}`; returns the existing tab if there is one |
| `PATCH /api/tabs/{id}` | reorder or move to another workspace `{position?, workspaceId?}` |
| `DELETE /api/tabs/{id}` | close a tab (its session or run keeps going) |

- `POST /api/sessions` and run start gain an optional `workspaceId` (+ `position`); without one, pdtd opens the tab
  in the project workspace as in §2.3 and returns it with the session or run.
- Tab and workspace status are aggregated by the client from the session and run lists it already polls, so there's
  no new status polling.
- Placement and ordering rules (insert after current, move between workspaces, renumbering positions) are pure
  functions with unit tests, in pdt-core for the server and mirrored in a small TS module for optimistic updates.

## 6. Git integration (3b)

### 6.1 Project keys

- Every project gets a **key**: 2–10 uppercase letters and digits starting with a letter, unique on the host (e.g.
  `PDT`). Phase 4 numbers tickets with it (`PDT-42`).
- Suggested from the name when adding (initials, or the first letters of a one-word name) and editable in project
  settings. Existing projects get one in the migration (deduplicated with a number).
- Shown in the tree and the workspace switcher.

### 6.2 Adding projects

- **Add by path** (today) and **clone by URL**: `POST /api/projects/clone {url, parentDir?, name?, key?}` runs
  `git clone` into `<parentDir>/<repo name>` (default `parentDir` is a new host setting, "Projects folder", default
  `~/work`), then adds the project. Progress streams to the add form; a failed clone removes the half-made folder.
  Uses the host's own git credentials (SSH agent, credential helper); PDT stores none.
- **Remove project:** removes it from PDT (sessions stopped, runs stopped, database rows gone, its workspace
  deleted). Never deletes files. Fixes the stale "PDT configuration" project on the Mac mini.

### 6.3 GitHub links

- If `origin` is a GitHub URL (https or ssh), pdtd derives `owner/repo` and the UI links:
  - the project → the repo,
  - a branch → its tree page, and "Open PR" → `compare/<base>...<branch>` (GitHub redirects to an existing PR's
    compare view; showing PR state needs the API and waits for Phase 6),
  - a run or session's commit → the commit page.
- No GitHub API calls and no token in this phase.

### 6.4 Worktrees

- **Adopt:** when `.worktrees/<branch>` already exists as a git worktree for that branch, creating that worktree or
  running that branch registers it instead of failing with `worktree_path_taken`. A folder that isn't a worktree of
  this repo still fails.
- **Remove worktree:** a tree action. Refuses while it has running sessions or runs; asks before removing a worktree
  with uncommitted changes (`git worktree remove`, `--force` only after the user confirms). Deleting the branch is a
  separate checkbox, off by default. Its tab in the project workspace closes.
- **Sync:** pdtd notices worktrees removed outside PDT (on list) and marks them missing; the tree offers to forget
  them.

### 6.5 Folder view

- The Files icon in the rail opens a read-only folder view for the current place (project root or worktree): a file
  tree honouring `.gitignore`, and a file viewer (CodeMirror, read-only, syntax highlighting) with the file's git
  status. Large (> 1 MB) and binary files show a notice instead.
- `GET /api/projects/{id}/files?worktreeId=&path=` lists a directory; `GET …/files/content?…` returns a file. Paths
  are resolved inside the checkout and refused if they escape it (symlinks included).
- Editing files is out of scope; Phase 6 adds the diff view next to this.

## 7. Testing

- **Placement and ordering (pdt-core, TS mirror):** insert after current, open-or-focus existing tab, move between
  workspaces, position renumbering. Pure functions, TDD.
- **pdt-store / pdt-api:** workspace and tab CRUD, one tab per session/run per workspace, project workspace made with
  a project and removed with it, `409 project_workspace`, launch placement, tabs closed when a session or run is
  removed, the migration seed for existing projects.
- **Web:** tree rendering and status aggregation, tab strip and switcher status, prefix keys (including the
  double-prefix pass-through), the new-tab chooser, drag behaviour through its keyboard and menu equivalents, phone
  tab switching.
- **3b:** key validation and suggestion, clone into a temp dir from a local bare repo (no network), GitHub URL
  parsing (https, ssh, `.git` suffix, non-GitHub), worktree adopt/remove/missing, folder path containment
  (`..`, absolute paths, symlinks out of the checkout), `.gitignore` filtering.
- Everything runs on Windows and macOS; no test touches port 7420, the real data folders, the network or the
  keychain.

## 8. Deferred

- Split panes (decided against: a tab shows one thing).
- Tab types beyond terminals and runs (conversation views come with Phase 5, diff with Phase 6, folder view as a tab
  later).
- Workspace templates and reopening a closed tab.
- Sessions spawning child sessions.
- PR state, checks and reviews from GitHub (Phase 6 / GitHub integration follow-up).
- File editing.
- Confirming Codex `app-server` approval events and MCP configuration (the main spec placed this in Phase 3; it moves
  to Phase 5, where the adapter is built).
