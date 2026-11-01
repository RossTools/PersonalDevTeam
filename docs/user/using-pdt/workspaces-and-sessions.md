# Workspaces and sessions

**Sessions** in the rail is where the work happens: agents, shells, dev servers and test runs, each in a tab. They run
in PDT, not your browser, so closing the window or switching devices doesn't stop them.

## Projects and worktrees

A **project** is one repository on your machine. Add one with **Add project** in **Settings** (see
[A first project](../getting-started/first-project.md)); its page there has its name, key, **Base branch** (what new
branches start from and are reviewed against), runs and policies.

A **worktree** is another checkout of the repository on a different branch, in its own folder under `.worktrees/`.
PDT makes one for each ticket it starts and each branch you run, so several agents work at once without touching each
other's files or your main checkout.

The project's **Worktrees** section in **Settings** lists them, with **Review changes** and **Remove worktree**. PDT
asks before removing one with uncommitted changes and won't remove one a ticket is working in. Finishing a merged
ticket removes its worktree for you.

## Workspaces and tabs

A **workspace** is a row of tabs; each tab shows an agent session, a shell, a run, a ticket or a branch's changes.

- Every project has its own workspace. A started ticket can get one (**Open in a new workspace**), and **New
  workspace** in the sidebar makes a free-form one.
- The sidebar lists the workspaces, the **Tickets** being worked on (with pull request and agent state), and your
  **Agents** sessions. Right-click (or long-press) anything for rename, move and close.
- A session's dot and label say what it is doing: **Working**, **Needs you**, **Done** (finished, not looked at yet),
  **Idle**, **Ended**. **Background** (a slowly pulsing green ring) means its turn ended but tasks it started in the
  background still run; hover to see them. It turns **Done** when they finish.
- Sessions a workflow starts (a ticket's Coder, Reviewer and QA, a Triage or PM step) aren't under **Agents**. Find
  them in their tab, the ticket's **Sessions** (under **Attempt**), or **Open session** on the step in the workflow's
  **Runs**.
- Drag tabs to reorder them, or move one to another workspace from its menu. A session or run has only one tab:
  opening it elsewhere (**Bring here**) moves it.
- **Closing a tab stops what's in it and removes it.** A closed session can't be resumed (files it wrote stay). PDT
  asks before closing a running one. Ticket and changes tabs just close.

## Start something new

Click **+** at the end of the tab bar (or **Ctrl+]** then **c**). The **New tab** chooser offers:

- each installed, switched-on agent (**Claude**, **Codex**, **Devin** and any you've added) as a conversation, and the
  first three also as a terminal (**Claude (terminal)**); keys **c**, **t**, **x**, **o**, **d**, **e** start them;
- **Shell** (**s**);
- the project's runs and tests, **Run another branch…**, and **Ticket…** to open a ticket here;
- tabs from other workspaces, to **Bring here**.

The picker at the top sets where it starts (project, and main checkout or worktree); it defaults to the current tab's.

## Conversation or terminal

An agent session is one of:

- **A conversation**: PDT's chat view, with the agent's messages, tool calls folded to a line each ("Ran 3 commands")
  while the agent's text stays outside them, each edit's diff, and a **Changes** summary per turn. Questions and permission requests show as cards with
  **Allow** and **Deny**. Type `@` to mention a file, `/` for the agent's commands; paste or attach images.
- **A terminal**: the agent's own interface, as in your terminal app.

Conversation is the default; change it in **Settings › This host › Agents**, or per session when you start one.
**Show as** in a session's header switches an existing one. Claude and Devin carry the conversation over (PDT asks
before interrupting a turn); Codex opens its terminal alongside on the same thread, and **Close terminal** goes back.

You can type while the agent works; the message goes in when it's ready. When it's waiting on you, a bar over the
message box says what it wants, and **Review** jumps to it.

## Models and permission modes

A conversation's message box has two pickers, both changeable at any time:

- **Model**. **Default model** is the agent's own default, or the one set under **Default models** in **Settings ›
  This host › Agents** (shown as, say, "Default (Opus)").
- **Permission mode**: what the agent may do without asking.
  - **Ask**: asks before anything that changes your machine.
  - **Accept edits**: edits files freely, asks before running commands.
  - **Plan**: reads and plans, changes nothing, and asks you to approve the plan.
  - **Bypass (dangerous)**: never asks.

Terminal sessions have one choice when they start: **Skip permission prompts (dangerous)**.

**Settings › This host › Agents** sets two defaults: one for tickets (bypass, since the agent works on its own branch
in its own folder) and one for other sessions (ask). **Always allow in this session** lasts for that session only; PDT
never changes your agents' own settings.

## Runs: dev servers and tests

A project's **runs** (a dev server, say) and **tests** come from its
[run configuration](../automation/run-configuration.md). Each run gets its own port and data folder, so one per branch
can run side by side.

Start one from **+** in the tab bar (for the current checkout, or **Run another branch…**, which makes the worktree if
needed), or with **Run…** on a ticket's Inbox item.

A run's tab shows its output (**Setup** and **Output** when it has a setup step). The header has its address (click to
open the app), **Stop** and **Restart**; the menu adds **Copy URL**, **Reset data…** (start with an empty data
folder) and **Remove…**. Starting a run that already has a finished tab restarts it there.

## The terminal's keys

- **Shift+Enter** adds a new line instead of sending (in Claude Code, Codex and most shells).
- Selecting text copies it; **Cmd+click** (Mac) or **Ctrl+click** opens a link.
- **Compose** (session menu) writes a long or multi-line prompt and sends it in one go.
- **Ctrl+]** then a key: **c** new tab, **n** / **p** next and previous tab, **1**–**9** tab by number, **a** next
  session that needs you, **k** command palette, **?** all shortcuts. **Ctrl+]** twice sends it to the program. On a
  Mac, **⌘K** also opens the palette.

On a phone or tablet, a key bar above the keyboard adds **Esc**, **Tab**, **Ctrl** (sticky: tap it, then the key),
**^C**, the arrows, **⌃]** and **⌨** (hide or show the keyboard). Drag to scroll back; long-press to select a word and
**Copy** it. The session's **⋯** menu has **Paste**, **Compose** and **Font size**.

## What survives a restart

When PDT restarts (after an update, say):

- **Workspaces and tabs** stay as they were, on every device.
- **Terminal sessions** start again on the same agent conversation; the old screen contents are gone.
- **Conversations** keep their history. A turn that was running is marked interrupted: click **Continue**. Nothing is
  sent to an agent without you (or a workflow you turned on).
- **Background tasks** an agent started (a long test run, a background sub-agent) stop and never report back. PDT
  tells a conversation's agent what stopped so it can restart what it needs: a workflow step's agent at once, your own
  conversations with your next message or **Continue**. Terminal sessions can't be told.
- **Runs** set to keep running on their own (`detach = true`) keep running; others stop until you **Restart** them.
- An unsent message stays in that browser tab.

A session you stopped stays stopped: **Restart** (terminal) or a new message (conversation) brings it back.
