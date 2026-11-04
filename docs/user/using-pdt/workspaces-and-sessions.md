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
  **Idle**, **Ended**, **Interrupted** (PDT restarted while it ran and it wasn't resumed). **Background** (a slowly
  pulsing green ring) means its turn ended but tasks it started in the background still run; hover to see them. It turns
  **Done** when they finish.
- Sessions a workflow starts (a ticket's Coder, Reviewer and QA, a Triage or PM step) aren't under **Agents**. Find
  them in their tab, the ticket's **Sessions** (under **Attempt**), or **Open session** on the step in the workflow's
  **Runs**.
- Drag tabs to reorder them, or move one to another workspace from its menu. A session or run has only one tab:
  opening it elsewhere (**Bring here**) moves it.
- **Closing a tab stops what's in it and removes it.** A closed session can't be resumed (files it wrote stay). PDT
  asks before closing a running one. Ticket and changes tabs just close.
- **A tab is as you left it.** Go to another view and come back, and its ticket section, scroll, open file and
  toggles are where they were. The Backlog, documents, files, workflows and Settings do the same. This lasts while
  the browser tab is open; closing a tab forgets it.

## Files

**Files** shows a checkout's files as git sees them (ignored files are left out): pick the project or worktree at the
top, open folders in the tree, and click a file to view it. Text files have **Edit**; **⌘S** (**Ctrl+S**) saves.

### Search in files

Type in **Search in files** above the tree to search the text of the checkout's files. Ignored and binary files are
left out.

- **Match case**, **Whole word** and **Regular expression** are the buttons in the search box.
- **Files to search** narrows the search: globs separated by commas, such as `docs/**/*.md` or `docs`. A name
  without a `/` matches at any depth; start one with `!` to leave those files out (`!*.log`).
- Matches are grouped by file, with each file's number of matching lines; click a file's name to fold it. Click a
  match to open the file at that line, the match marked.
- A search shows at most 2,000 matching lines, 50 per file, and stops after 10 seconds; the line above the results
  says when it shows only the first ones.
- **Escape** or the **×** clears the search and brings back the tree.

### Find a file by name

Open the command palette (**Ctrl+]** then **k**, or **⌘K** on a Mac) and type at least two letters of a file's name.
Matching files are listed under **Files**, after everything else, each with its folder and its checkout (a ticket's
key or a branch for a worktree, else the project). It looks in the worktree of the tab on screen, else the current
project; with no project open, in every project. **Enter** opens the file in **Files**.

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

- **A conversation**: PDT's chat view, with the agent's messages, tool calls folded to a line each ("Ran 3 commands";
  PDT's own actions show as cards, below) while the agent's text stays outside them, each edit's diff, and a **Changes** summary per turn. Questions and permission requests show as cards with
  **Allow** and **Deny**. Type `@` to mention a file, `/` for the agent's commands; paste or attach images.
- **A terminal**: the agent's own interface, as in your terminal app.

Conversation is the default; change it in **Settings › This host › Agents**, or per session when you start one.
**Show as** in a session's header switches an existing one. Claude and Devin carry the conversation over (PDT asks
before interrupting a turn); Codex opens its terminal alongside on the same thread, and **Close terminal** goes back.

You can type while the agent works; the message goes in when it's ready. When it's waiting on you, a bar over the
message box says what it wants, and **Review** jumps to it.

In a conversation, ticket keys show as pills: hover one (or tab to it) for its title, stage, priority, labels, who
has it, its branch and its pull request with CI; click it to open the ticket. Pull request numbers (`#12`) link to the
pull request, file paths of the project open in the file viewer, and status words in tables (**CI failed**, **ready
to accept**) are coloured. What an agent does through PDT shows as a card rather than a folded line: the tickets it
created, what it changed on a ticket (from → to), links, messages to other sessions, comments, workflow runs it
started, watches (with **Stop watching**), work it stopped on a ticket, issues it opened or linked, bugs it reported,
documents it created, edited or linked (with **Open beside**), and notes it saved or deleted. When one of these waits
for your yes, its card shows what would be created or changed. Calls that only read tickets stay folded, and their line
names the tickets they read as pills.

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

A run keeps going when its branch gets new commits: **Restart** it to pick them up (a dev server that reloads itself
picks them up anyway). A pull request from a fork has no branch in your repository, so fetch it into a branch of your
own before running it.

## The terminal's keys

- **Shift+Enter** adds a new line instead of sending (in Claude Code, Codex and most shells).
- Selecting text copies it; **Cmd+click** (Mac) or **Ctrl+click** opens a link.
- When a program in the terminal copies something to your clipboard (Claude Code does when you copy), a toast
  shows the start of what it copied.
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
- **Terminal sessions** start again on the same agent conversation. A shell starts again below what it showed before,
  under a **restarted · earlier output above** line.
- **Conversations** keep their history. A turn that was running is marked interrupted: click **Continue**. Nothing is
  sent to an agent without you (or a workflow you turned on).
- **Background tasks** an agent started (a long test run, a background sub-agent) stop and never report back. PDT
  tells a conversation's agent what stopped so it can restart what it needs: a workflow step's agent at once, your own
  conversations with your next message or **Continue**. Terminal sessions can't be told.
- **Runs** set to keep running on their own (`detach = true`) keep running; others stop until you **Restart** them.
- An unsent message stays in that browser tab.

A session you stopped stays stopped: **Restart** (terminal) or a new message (conversation) brings it back.
