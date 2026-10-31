# Tickets and the board

A ticket is one piece of work: what should change, why, and how you'll know it's done. You write tickets, put them in
order and start agents on them; the board shows where each one is.

Open **Tickets** in the rail (or **Ctrl+]** then **l**). **Backlog** and **Board** at the top switch between two views
of the same tickets.

## Stages

| Stage | Means |
| --- | --- |
| **Draft** | Written down, not ready yet. New tickets start here. |
| **Ready** | Refined and ready, not picked yet. |
| **To do** | Picked: on the board, waiting to start. |
| **In progress** | A branch, a worktree and (usually) an agent are working on it. |
| **Done** | Finished, normally because its branch merged. |
| **Won't do** | Closed without doing it. |

The **Backlog** lists **Draft** and **Ready** tickets in your order. The **Board** has columns for **To do**, **In
progress** and **Done** (closed in the last two weeks).

Move a ticket by dragging its card, with **Move to** in its menu (**⋯**), or with the stage stepper on its page.
Moving a ticket that's being worked on back to **Draft**, **Ready** or **To do** stops its agent and ends the attempt
(PDT asks first; the worktree stays).

With the **Pipeline** workflow, **In progress** splits into **Code**, **Review**, **QA**, **Accept** and **Merge**, and
each card says who has it ("Reviewer · working", "Your turn"). See [Workflows](../automation/workflows.md).

## Write a ticket

1. Click **New ticket**.
2. Pick the **Project** and write a **Title** and **Description** (Markdown; **Preview** renders it). The agent reads
   exactly this, so say what should change, why, and how to check it.
3. Optionally set **Stage**, **Priority** and **Labels**, then click **Create ticket**.

For a quick one, type a title in the box at the top of the backlog and press **Enter** (it goes in as a **Draft** at
the top); **Shift+Enter** opens the full form instead.

Click a ticket to open it beside the list, or **Open** to give it its own tab. Its **Activity** holds your comments and
everything that happened to it: moves, starts, merges, agents' outcomes.

## Screenshots and logs

Paste or drop a screenshot or text file (a log, a JSON response, a config) into a description or comment, or use the
paperclip (**Attach files**). Images show inline and open full size; text files show as a chip that opens a viewer.

Limits: PNG, JPEG, GIF and WebP images up to 10 MB; plain-text files (`.txt`, `.log`, `.md`, `.json`, `.csv`,
`.yaml`, `.toml` and similar) up to 1 MB; 50 per ticket. The ticket's **Attachments** list shows them all, with
**Attach** and a remove button. Deleting the ticket deletes them.

Agents working on the ticket can open its attachments, QA can attach screenshots of what it tried, and
[the PM](../automation/the-pm.md) can attach an image you paste into its chat.

## Priority and labels

**Priority** (**Urgent**, **High**, **Medium**, **Low**, **No priority**) sorts and groups the backlog and, by default,
decides what [auto-start](#auto-start) takes first.

**Labels** are lower-case tags you make up (`ui`, `bug`, `docs`); type one in **Labels** to add it. One label means
something to PDT: **`agentic`**, for a ticket that can go all the way without you. On the **Pipeline**, an `agentic`
ticket skips **Accept** and merges once review and QA pass it; others wait for you in the [Inbox](the-inbox.md).
Tickets that came from outside PDT (an issue, say) only get it from you.

## Parts and epics

A ticket too big for one pull request becomes a parent (an epic) with **parts**, each started, reviewed and merged on
its own.

- **Add a part** in the parent's **Parts** section: type in **New part… Enter to add** and press **Enter**. With
  **Starts after** ticked (the default), each part is blocked by the one before.
- **Make into parts…** (ticket menu) makes several at once, from the description's Scope or Parts list if it has
  one. **Each part starts after the previous one** chains them.
- Or set an existing ticket's **Parent** field.

A parent is never started itself: **Start next part** starts its first unblocked open part. The parent moves to **In
progress** with its first part and to **Done** when every part is closed, and can't be closed while a part is open. On
the board each parent gets its own lane, with a stepper showing progress. A part's agent also gets the parent's
description and the list of other parts.

## Links and blocking

A ticket's **Links** section has:

- **Blocked by** / **Blocks**: one must be done before the other starts.
- **Related**: no order.

A ticket with an open blocker shows **Blocked**; starting it warns you, and auto-start skips it.

Tickets that change the same code count too. If a project names its **areas** (folders that belong together), a ticket
in the same area as one in progress shows an overlap, and its **Overlaps** section lists the shared areas and files
both branches change. An overlap in an exclusive area (database migrations, say) is a conflict. See
[Run configuration](../automation/run-configuration.md).

## Search and filters

- **Search tickets** matches titles and descriptions; type a key (`ABC-12`) to jump to that ticket.
- Filter by **Stage**, **Priority**, **Labels**, **Blocked** and **Overlap**; **Show closed** adds **Done** and
  **Won't do**; **Clear filters** resets.
- Sort by rank, priority, updated or created; group by stage, priority or label.
- **Save filter…** keeps the current filters. The built-in **Ready to start** shows **Ready** tickets nothing blocks or
  conflicts with.

In rank order, drag rows (or **Alt+↑** / **Alt+↓**) to reorder; **Move to top** is in the row menu. **Select** (or the
row checkboxes) picks several tickets to move, label, prioritise or delete together.

## Start a ticket

**Start** (on the ticket, card or row) opens the **Start** dialog:

- **Session**: **Conversation** (a chat view), **Terminal** (the agent's own interface) or **No session** (just the
  branch and worktree, for you).
- **Agent**, **Branch** and **Base branch** (PDT names the branch after the ticket).
- **Permissions**: see [Models and permission modes](workspaces-and-sessions.md#models-and-permission-modes).
- **Open in a new workspace**: give the ticket and session a workspace of their own.

Click **Start**. PDT makes the branch and a [worktree](workspaces-and-sessions.md#projects-and-worktrees), moves the
ticket to **In progress** (one attempt at a time per ticket), starts the agent there with the ticket and its comments
as the first message, and opens the ticket's tab with the session beside it.

If an open blocker, an overlap or the project's limit on running tickets stands in the way, the dialog says so:
**Start anyway**, or **Wait** (the ticket shows **Waiting until clear** and starts when it is).

Comments you add while an agent works are sent to it. To stop, move the ticket back to **To do** (or **Draft** or
**Ready**), or use **Abandon attempt…** in its menu.

## Auto-start

In **Settings**, on the project's page:

- **Auto-start**: **Off**, **To do tickets**, or **To do, then Ready tickets**.
- **Running tickets at once**: auto-start starts the next ticket whenever fewer than this many are in progress (one
  at a time with no cap).
- **Auto-start picks by**: **Priority, then rank** or **Backlog rank**. **To do** always comes before **Ready**.
- **Quiet period before auto-start**: a ticket must go this long unchanged first (two minutes by default), so you can
  finish editing it. Its card shows "Auto-starts in 1:30".

Auto-start skips parents, blocked or conflicting tickets, and outside tickets you haven't let in. It uses your default
agent, view and permission mode (**Settings › This host › Agents**), or the project's workflow if it has one. Starting
a ticket yourself is never delayed.

## On a phone

The top bar's title picks the project and saved filters, with **Backlog**, **Board**, search and filters below. The
board shows one column at a time: pick it from the chips or swipe. Long-press a row or card for its menu instead of
dragging; **＋** adds a ticket. A ticket opens full screen with **Overview**, **Conversation** and **Changes**, and its
main action (**Start**, **Accept…**, **Open work**) at the bottom. In **Comment…**, the paperclip attaches a photo or
file.
