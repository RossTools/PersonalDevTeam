# The first run

The first time you open PDT it shows **Get started**: four things PDT needs before agents can work on your code. Do
them in any order. Each one ticks itself off when it's done, wherever you do it (in the checklist, in Settings, or on
another device).

1. **Agents on this machine.** PDT looks for git and for Claude Code, Codex and Devin, and shows where it found each.
   It needs git and at least one agent. See [Connect agents and GitHub](connect.md).
2. **Connect GitHub** (optional). For pull requests, CI and issues. If your projects aren't on GitHub, click **Skip,
   my projects aren't on GitHub**: PDT then merges branches locally.
3. **Add a project.** A folder with a git repository, or one to clone. See [Add a first project](first-project.md).
4. **Pick a default agent.** The agent that works on tickets unless you choose another. Pick one and click **Use
   <agent>**. You can change it later in **Settings › This host › Agents**, and per persona.

When all four are done, PDT says **You're set up.** with two ways on: **Go to the board** and **Open the PM**.

## Finding it again

- **Dismiss setup** at the bottom hides the checklist.
- **Settings › This host › Setup** opens it again any time.

The checklist lives on your machine, not in the browser, so your phone and other browsers see the same progress.

## Getting around

- The **rail** on the left switches between the main views: **Project manager** (the PM), **Sessions** (your
  projects and the agents working in them), **Inbox** (what's waiting for you), **Tickets** (the backlog and the
  board), **Notes**, **Workflows**, **Personas**, **Files** and **Settings**.
- Each project has a **workspace**: a strip of tabs for its sessions, runs and tickets.
- Keyboard shortcuts start with **Ctrl+]**, then a key: **Ctrl+]** then **b** opens the board, **l** the tickets.

## Next

[Connect agents and GitHub](connect.md).
