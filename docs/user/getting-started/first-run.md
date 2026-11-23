# The first run

The first time you open PDT it shows **Get started**: six things to do before agents can work on your code. Do them
in any order; each ticks itself off when it's done, wherever you do it (here, in Settings or on another device).

1. **Name this instance**: a short name for this PDT, like `studio` or `win-box` (letters, digits, `-`, `_` and
   `.`). Until you pick one it goes by the machine's name. Other instances show it when this one works on their
   projects; see [Remote projects](../your-machine/remote-projects.md). Click **Use this name**; **Settings › This
   host › Name** changes it later.
2. **Agents on this machine.** PDT looks for git and for Claude Code, Codex and Devin, and shows where it found each.
   It needs git and at least one agent you're logged in to. See [Connect agents and GitHub](connect.md). When git has
   no name and email to commit merges with, it shows the two commands that set them.
3. **Connect GitHub** (optional), for pull requests, CI and issues. If your projects aren't on GitHub, click **Skip,
   my projects aren't on GitHub**: PDT then merges branches locally.
4. **Add a project**: a folder with a git repository, or one to clone. See [Add a first project](first-project.md).
5. **Pick a default agent**: the agent that works on tickets unless you choose another. Click **Use <agent>**. You can
   change it later in **Settings › This host › Agents**, and per persona.
6. **Choose tickets' permissions**: what an agent working on a ticket may do without asking you. Tickets start in
   **Ask** until you pick another mode:
   - **Ask**: the agent asks before it edits a file or runs a command. An agent working on its own (auto-start, a
     workflow) waits for your answer.
   - **Accept edits**: it edits files in the ticket's worktree freely and asks before running a command.
   - **Bypass (dangerous)**: it never asks. It works on its own branch in its own folder, but can run any command as
     you.

   Click **Use <mode>**. You can still pick another mode each time you start a ticket, change the default in
   **Settings › This host › Agents › Ticket permissions**, or set one for a persona or project.

When all six are done, PDT says **You're set up.** with **Go to the board** and **Open the PM**.

**Dismiss setup** hides the checklist; **Open** above **Settings › This host › Checks** opens it again.

## Getting around

- The **rail** on the left switches between **Project manager** (the PM), **Sessions** (your projects and the agents
  working in them), **Inbox** (what's waiting for you), **Tickets** (the backlog and the board), **Notes**,
  **Documents**, **Workflows**, **Personas**, **Files** (a checkout's files; see
  [Files](../using-pdt/workspaces-and-sessions.md#files)) and **Settings**.
- Each project has a **workspace**: a strip of tabs for its sessions, runs and tickets.
- Your own [todos](../using-pdt/todos.md) sit in a panel on the right: **Ctrl+]** then **t** (**⌘.** on a Mac).
- Keyboard shortcuts are **Ctrl+]** then a key: **b** opens the board, **l** the tickets.

## Next

[Connect agents and GitHub](connect.md).
