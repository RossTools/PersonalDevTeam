# Workspaces and sessions

**Sessions** in the rail is where the work happens: agents, shells, dev servers and test runs, each in a tab. They run
in PDT, not in your browser, so closing the window, or opening PDT on another device, doesn't stop them.

## Projects and worktrees

A **project** is one repository on your machine. Add one in **Settings** with **Add project** (see
[A first project](../getting-started/first-project.md)); its page there has its name, key, **Base branch** (what new
branches start from and are reviewed against), its runs, and its policies.

A **worktree** is a second checkout of the same repository on another branch, in its own folder inside the project
(under `.worktrees/`). PDT makes one for each ticket it starts, and for each branch you run, so several agents can
work at once without touching each other's files or your main checkout.

The project's **Worktrees** section in **Settings** lists them, with **Review changes** for each and **Remove
worktree** for ones you're finished with. PDT asks first when a worktree has uncommitted changes, and won't remove one
a ticket is still working in. Finishing a merged ticket removes its worktree for you.

## Workspaces and tabs

A **workspace** is a row of tabs. Each tab shows one thing: an agent session, a shell, a run, a ticket, or a branch's
changes.

- Every project has a workspace of its own. Starting a ticket can give it one too (**Open in a new workspace**), and
  you can make free-form ones with **New workspace** in the sidebar.
- The **Sessions** sidebar lists the workspaces, then the **Tickets** being worked on (with their pull request and
  the agent's state), then every **Agents** session. Right-click anything there (long-press on a touch screen) for its
  menu: rename, move, close.
- **Agents** lists the conversations you started yourself, from a ticket's **Start** or not. The sessions a workflow
  starts (a ticket's Coder, Reviewer and QA, a Triage or PM step) are never there, running or ended, even while a tab
  holds them. Reach them from their tab (PDT opens one for each, and the command palette and **Bring here** find it),
  the ticket's **Sessions** (under **Attempt**), or **Open session** on the step in the workflow's **Runs**. Once
  one's tab is closed, the palette and **Bring here** leave it out (except a running one with no ticket), so open it
  from its ticket or run. A step that needs you still shows in the **Inbox**.
- Drag a tab to reorder it, or move it to another workspace from its menu. A session or run has one tab only: opening
  it somewhere else moves it there.
- **Closing a tab stops what's in it and removes it.** For a session that means its terminal history is gone and it
  can't be resumed; files it wrote stay. PDT asks before closing one that's still running. Ticket and changes tabs just
  close.

## Start something new

Click **+** at the end of the tab bar (or press **Ctrl+]** then **c**). The **New tab** chooser offers:

- **Claude**, **Codex**, **Devin** and any other agents you've added, each as a conversation, and the first three
  also as a terminal (**Claude (terminal)**); a key starts each (**c**, **t**, **x**, **o**, **d**, **e**);
- **Shell** (**s**): a plain terminal;
- the project's runs and tests for this checkout, **Run another branch…**, and **Ticket…** to open a ticket here;
- tabs from other workspaces, to **Bring here**.

The picker at the top says where it starts: which project, and the main checkout or a worktree. It defaults to the tab
you're on. Only agents that are installed and switched on are offered.

## Conversation or terminal

An agent session is either:

- **A conversation**: PDT's chat view. You see the agent's messages, its tool calls folded into one line each ("Ran 3
  commands"), the diff of each file it edits, and a **Changes** summary at the end of each turn. Questions and
  permission requests appear as cards with **Allow** and **Deny**. Type `@` to mention a file, `/` for the agent's
  commands, and paste or attach images.
- **A terminal**: the agent's own interface, exactly as it runs in your terminal app.

Conversations are the default; change that in **Settings › This host › Agents**, or pick per session in the **Start**
dialog or the **New tab** chooser.

To switch an existing session, use **Show as** in its header: **Conversation** or **Terminal**. Claude and Devin
carry the same conversation over (PDT asks to interrupt the agent if it's in the middle of a turn). Codex opens its
terminal alongside the conversation, on the same thread; **Close terminal** goes back.

While an agent works in a conversation you can keep typing: your message goes in when it's ready for it. When the
agent is waiting on you, a bar over the message box says what it wants; **Review** jumps to it.

## Models and permission modes

The message box of a conversation has two pickers:

- **Model**: which model the agent uses. **Default model** is the agent's own default, or the one you set for it in
  **Settings › This host › Agents** under **Default models** (then it reads, say, "Default (Opus)").
- **Permission mode**: what the agent may do without asking.
  - **Ask**: it asks before anything that changes your machine.
  - **Accept edits**: it edits files freely and asks before running commands.
  - **Plan**: it reads and plans, changes nothing, and asks you to approve the plan.
  - **Bypass (dangerous)**: it never asks.

Both can be changed at any time during a conversation. Terminal sessions have one choice, **Skip permission prompts
(dangerous)**, when they start.

The defaults are in **Settings › This host › Agents**: one permission mode for tickets (bypass, since the agent works
on its own branch in its own folder) and one for other sessions (ask). **Always allow in this session** on a request
lasts for that session only; PDT never changes your agents' own settings.

## Runs: dev servers and tests

A project's **runs** (a dev server, say) and **tests** are set up in its run configuration (see
[Run configuration](../automation/run-configuration.md)). Each run gets its own port and data folder, so one per
branch can run side by side.

To start one:

- from **+** in the tab bar: the runs and tests for the checkout you're in, or **Run another branch…** to pick a
  branch (PDT makes its worktree if needed);
- from a ticket's Inbox item, with **Run…**.

A run's tab shows its output, with **Setup** and **Output** when it has a setup step. Its header has the run's
address (click it to open the running app), **Stop** and **Restart**; its menu adds **Copy URL**, **Reset data…**
(start again with an empty data folder) and **Remove…**.
Starting a run that already has a finished tab restarts it there.

## The terminal's keys

In a terminal tab:

- **Shift+Enter** adds a new line (in Claude Code, Codex and most shells) instead of sending.
- Selecting text with the mouse copies it. **Cmd+click** (Mac) or **Ctrl+click** opens a link.
- **Compose** (in the session's menu) writes a long or multi-line prompt and sends it in one go.
- **Ctrl+]** is PDT's shortcut key: press it, then a letter (**c** new tab, **n** / **p** next and previous tab,
  **1**–**9** a tab by number, **a** the next session that needs you, **k** the command palette, **?** all of them).
  Press **Ctrl+]** twice to send it to the program. On a Mac, **⌘K** opens the command palette too.

On a phone or tablet, a key bar sits above the on-screen keyboard with the keys it lacks: **Esc**, **Tab**, **Ctrl**,
**^C**, the arrows, **⌃]** (PDT's shortcut key) and **⌨** (hide or show the keyboard). **Ctrl** is sticky: tap it,
then the key to combine it with. Drag to scroll back; long-press to select a word and **Copy** it. The session's
**⋯** menu has **Paste**, **Compose** and **Font size**.

## What survives a restart

PDT keeps working when your browser closes. When PDT itself restarts (after an update, say):

- **Workspaces and tabs** stay as they were, on every device.
- **Terminal sessions** that were running start again and pick up the same agent conversation. What was on screen
  before is gone; the agent's conversation isn't.
- **Conversations** keep their whole history. A turn that was running is marked interrupted: click **Continue** to
  carry on. Nothing is sent to an agent without you (or a workflow you turned on).
- **Runs** set to run on their own (`detach = true` in the run configuration) keep running; others stop, and
  **Restart** starts them again.
- A message you were typing but hadn't sent stays in that browser tab.

A session you stopped stays stopped: **Restart** (a terminal) or a new message (a conversation) brings it back.
