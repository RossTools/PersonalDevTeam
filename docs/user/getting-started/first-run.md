# The first run

The first time you open PDT it shows **Get started**: five things to do before agents can work on your code. Do them
in any order; each ticks itself off when it's done, wherever you do it (here, in Settings or on another device).

1. **Agents on this machine.** PDT looks for git and for Claude Code, Codex and Devin, and shows where it found each.
   It needs git and at least one agent you're logged in to. See [Connect agents and GitHub](connect.md). When git has
   no name and email to commit merges with, it shows the two commands that set them.
2. **Connect GitHub** (optional), for pull requests, CI and issues. If your projects aren't on GitHub, click **Skip,
   my projects aren't on GitHub**: PDT then merges branches locally.
3. **Add a project**: a folder with a git repository, or one to clone. See [Add a first project](first-project.md).
4. **Pick a default agent**: the agent that works on tickets unless you choose another. Click **Use <agent>**. You can
   change it later in **Settings › This host › Agents**, and per persona.
5. **Choose tickets' permissions**: what an agent working on a ticket may do without asking you. Tickets start in
   **Ask** until you pick another mode:
   - **Ask**: the agent asks before it edits a file or runs a command. An agent working on its own (auto-start, a
     workflow) waits for your answer.
   - **Accept edits**: it edits files in the ticket's worktree freely and asks before running a command.
   - **Bypass (dangerous)**: it never asks. It works on its own branch in its own folder, but can run any command as
     you.

   Click **Use <mode>**. You can still pick another mode each time you start a ticket, change the default in
   **Settings › This host › Agents › Ticket permissions**, or set one for a persona or project.

When all five are done, PDT says **You're set up.** with **Go to the board** and **Open the PM**.

**Dismiss setup** hides the checklist; **Settings › This host › Setup** opens it again.

## Getting around

- The **rail** on the left switches between **Project manager** (the PM), **Sessions** (your projects and the agents
  working in them), **Inbox** (what's waiting for you), **Tickets** (the backlog and the board), **Notes**,
  **Documents**, **Workflows**, **Personas**, **Files** and **Settings**.
- Each project has a **workspace**: a strip of tabs for its sessions, runs and tickets.
- Keyboard shortcuts are **Ctrl+]** then a key: **b** opens the board, **l** the tickets.

## Next

[Connect agents and GitHub](connect.md).
