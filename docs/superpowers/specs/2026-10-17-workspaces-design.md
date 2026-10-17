# Personal Dev Team — Workspaces and Git Integration (Phase 3) Design

Status: draft from brainstorming on 2026-10-17, awaiting the user's review. This amends the main spec,
`2026-10-14-personal-dev-team-design.md`: it is Phase 3, and it replaces the main spec's bottom panel (§7.1) with
split panes.

## 1. Purpose

Day-to-day agent work in PDT without herdr. Today PDT shows one session at a time, picked from a list for one
project, plus a bottom panel of shells. After this phase the user works the way they do in herdr: several agents and
shells side by side in split panes, grouped into tabs and workspaces, driven from the keyboard, with each tab and
workspace showing at a glance which agents are working, waiting on the user, or done. The same layouts appear on the
desktop, the laptop and the phone.

The phase ships as two PRs under this one spec:

- **3a — Workspaces** (§2–§5): workspaces, tabs, split panes, the session tree, keyboard, status, drag to rearrange.
- **3b — Git integration** (§6): project keys, clone by URL, GitHub links, adopting and removing worktrees, removing
  projects, the folder view.

### Success criteria

- The user can run three agents and a shell for one branch side by side in one tab, and a second branch's agents in
  another tab, without the mouse.
- A free-form workspace can hold sessions from two projects.
- Opening PDT on another device shows the same workspaces, tabs and panes.
- From any tab the user can see which other tabs and workspaces have an agent that needs them.
- A project can be added by cloning a GitHub URL, and every branch links to GitHub.

## 2. Model (3a)

### 2.1 Workspaces

- A **workspace** has a name, an order, and a list of tabs.
- Two kinds:
  - **Project workspace:** exactly one per project, made when the project is added (and for existing projects by the
    migration). Named after the project; it can't be deleted while the project exists.
  - **Free-form workspace:** made and named by the user. It can hold sessions and runs from any project.
- Both kinds behave the same once made: any session or run can be placed in either.

### 2.2 Tabs

- A tab has a title, an order, a layout (§2.3) and the pane that last had focus.
- A project workspace starts with one tab per place sessions live: "Repository" and one per worktree, titled with the
  branch. New worktrees add a tab. The user can rename, reorder, close and add tabs freely afterwards.
- Free-form workspaces start with one empty tab.

### 2.3 Layouts and panes

- A tab's layout is a tree. A node is either a **split** (direction `row` or `column`, two or more children with
  relative sizes) or a **pane**.
- A pane shows one thing:
  - a **session** (agent or shell terminal, as `SessionView` today),
  - a **run** (its output and controls, as `RunView` today),
  - or nothing yet (an empty pane offers the quick-new choices, §4.3).
- A session or run may appear in any number of panes and workspaces. All viewers share the one PTY; the PTY takes the
  size of the pane that most recently had focus or was resized (last writer wins, as today).
- Removing a session or run empties the panes that showed it; the pane stays so the layout doesn't jump.

### 2.4 Where sessions go

- **Launching** from the session tree (§3) places the new session in its project workspace, in the tab for its
  branch (made if missing), as a new pane split off that tab's focused pane, and switches to it.
- **Launching** from a pane (§4.3) puts the session in that pane or a split of it, in whatever workspace that is.
- **Clicking** a session or run in the tree focuses a pane already showing it in the current workspace. If there is
  none, it goes to its project workspace and branch tab, placed in a new split if not already there.
- **Opening elsewhere:** a session's menu has "Open in workspace…" (pick or make a free-form workspace) and the tree
  supports dragging a session or run onto a pane or tab.

### 2.5 Shared and per-device state

- Workspaces, tabs and layouts are stored in pdtd's database and shared by every device.
- Which workspace and tab a device is looking at, the tree's collapsed groups, and the phone's current pane are
  per-device (browser storage), so the phone doesn't yank the desktop around.
- Concurrent edits: each tab has a `version`. A layout save sends the version it was based on; a stale save gets
  `409 layout_conflict` and the client reloads the tab and reapplies the user's last action if it still makes sense
  (otherwise it just shows the new layout). Clients poll workspaces like sessions (every 2 s).

### 2.6 The bottom panel

The bottom panel goes away: a shell is a pane like any other. Ctrl+` now toggles a shell pane in the current tab
(splits one off the focused pane, or focuses it if the tab already has one; a second press returns focus).

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
- The current workspace's sessions are highlighted in the tree.

## 4. Workspace view (3a)

### 4.1 Layout

- The work area gets a header with the **workspace switcher** (a dropdown listing workspaces with their status,
  plus "New workspace") and the **tab strip** (tabs with their status, "+" for a new tab).
- Below it, the tab's panes, separated by draggable dividers. Each pane has a slim title bar (status dot, title,
  activity badge, the session's actions, close-pane, a menu with split/move/open-in-workspace).
- The focused pane has an accent border; keyboard input goes to its terminal.

### 4.2 Status at a glance

- A tab's status is the most urgent activity of the sessions in its panes; a workspace's is the most urgent of its
  tabs. Same order and colors as the session activity badge (§5 of the remote-host spec): needs you (amber) >
  working (green) > idle (gray).
- The unseen dot shows on a tab or workspace when any session in it is unseen.
- The browser tab title shows a count of sessions that need the user (e.g. `(2) PDT`).

### 4.3 Quick new pane

- Split right / split down opens a small chooser in the new pane: **Claude**, **Codex**, **Shell**, **Run…**, or an
  existing session or run (type to filter). One keypress picks Claude/Codex/Shell (`c`, `x`, `s`).
- The new session starts in the same place (project and worktree) as the focused pane's session or run. In an empty
  free-form workspace the chooser asks for the project and place first.
- The skip-permissions toggle is in the chooser too.

### 4.4 Keyboard

A prefix key, then one key, like tmux. The prefix is caught before the terminal sees it; pressing it twice sends it
through to the terminal.

| Keys (after the prefix) | Action |
|---|---|
| `\|` / `-` | split right / split down (opens the chooser) |
| arrows or `h j k l` | focus the pane in that direction |
| `z` | zoom: the focused pane fills the tab (again to restore) |
| `x` | close the pane (the session keeps running) |
| `t` | new tab |
| `1`–`9` | go to tab N |
| `n` / `p` | next / previous tab |
| `w` | workspace switcher (type to filter, Enter to go) |
| `r` | rename the tab |
| `a` | jump to the next session that needs you, in any workspace |
| `?` | show these keys |

- **Default prefix: Ctrl+]**, chosen because it is unbound in zsh/readline and not taken by browsers or macOS
  (confirm against current Claude Code and Codex keybindings while building). Ctrl+B (tmux's) is taken by Claude
  Code (background tasks). The prefix is a host setting.
- Ctrl+` keeps toggling the shell pane (§2.6). Browser-reserved keys (Cmd/Ctrl+T, W, N) are left alone.

### 4.5 Drag to rearrange

- Drag a divider to resize; sizes are saved with the layout.
- Drag a pane by its title bar onto another pane's edge (split there) or centre (swap), or onto a tab (move it to
  that tab).
- Drag tabs to reorder them, or onto another workspace in the switcher to move them.
- Drag a session or run from the tree onto a pane or tab (§2.4).
- Everything dragging does has a keyboard or menu equivalent.

### 4.6 Phone

- One pane at a time, full width. The tab strip scrolls horizontally; under it a pane switcher lists the tab's panes
  with their status. Swiping left/right moves between panes.
- Splitting on the phone adds a pane to the tab (shown in the switcher), so layouts made on the phone look sensible on
  the desktop.
- No drag on the phone; the pane menu has "Move to tab…" instead.
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
    id            TEXT PRIMARY KEY NOT NULL,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    position      INTEGER NOT NULL,
    layout        TEXT NOT NULL,          -- JSON layout tree (§2.3)
    focused_pane  TEXT,
    worktree_id   TEXT REFERENCES worktrees(id) ON DELETE SET NULL,  -- the branch tab a project workspace made
    version       INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
```

The migration makes a project workspace for each existing project, with a Repository tab and one tab per worktree;
each tab's layout holds that place's sessions in a row of panes (empty if none, at most four; more stay in the tree).

- The layout JSON is typed in pdt-core (`Layout`, `LayoutNode`, `PaneContent`) and exported with ts-rs. pdtd
  validates it on save: known pane contents, unique pane ids, split sizes positive, depth ≤ 8, panes ≤ 16 per tab.
  Pane contents pointing at missing sessions or runs are kept and shown as empty.
- Pure layout operations (split, close, move, swap, resize, focus-direction, insert-session-for-launch) live in a
  TypeScript module with unit tests, mirrored in pdt-core only where the server needs them (the migration seed,
  the launch placement in §2.4).

Routes (errors are `{code, message}`):

| Route | Purpose |
|---|---|
| `GET /api/workspaces` | all workspaces with their tabs (layouts included) and aggregated status |
| `POST /api/workspaces` | new free-form workspace `{name}` |
| `PATCH /api/workspaces/{id}` | rename, move `{name?, position?}` |
| `DELETE /api/workspaces/{id}` | delete a free-form workspace (`409 project_workspace` for a project's) |
| `POST /api/workspaces/{id}/tabs` | new tab `{title?}` |
| `PATCH /api/tabs/{id}` | rename, reorder, move to another workspace `{title?, position?, workspaceId?}` |
| `PUT /api/tabs/{id}/layout` | save `{layout, focusedPane, version}` → new version, or `409 layout_conflict` |
| `DELETE /api/tabs/{id}` | close a tab (its sessions keep running) |

`POST /api/sessions` gains an optional `placement` (`{tabId, paneId, split: "right" \| "down" \| "replace"}`); without
one, pdtd places the session as in §2.4 and returns the tab and pane it used. Session activity and unseen come from
the existing session list; the client aggregates tab and workspace status from it, so there's no new polling.

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

- **Layout module (TS):** split/close/move/swap/resize/focus-direction, placement rules of §2.4, validation limits.
  Pure functions, TDD.
- **pdt-core:** layout (de)serialization and validation; the migration seed for existing projects.
- **pdt-store / pdt-api:** workspace and tab CRUD, versioned layout saves and `409 layout_conflict`, project
  workspace made with a project and removed with it, launch placement, tabs made for new worktrees.
- **Web:** tree rendering and aggregation, prefix keys (including the double-prefix pass-through), the quick-new
  chooser, phone single-pane mode, drag behaviour through its keyboard equivalents.
- **3b:** key validation and suggestion, clone into a temp dir from a local bare repo (no network), GitHub URL
  parsing (https, ssh, `.git` suffix, non-GitHub), worktree adopt/remove/missing, folder path containment
  (`..`, absolute paths, symlinks out of the checkout), `.gitignore` filtering.
- Everything runs on Windows and macOS; no test touches port 7420, the real data folders, the network or the
  keychain.

## 8. Deferred

- Pane types beyond terminals and runs (conversation views come with Phase 5, diff with Phase 6, folder view as a pane
  later).
- Workspace templates and restoring a closed tab.
- Sessions spawning child sessions.
- PR state, checks and reviews from GitHub (Phase 6 / GitHub integration follow-up).
- File editing.
- Confirming Codex `app-server` approval events and MCP configuration (the main spec placed this in Phase 3; it moves
  to Phase 5, where the adapter is built).
