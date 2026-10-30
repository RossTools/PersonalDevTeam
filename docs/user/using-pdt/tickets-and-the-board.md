# Tickets and the board

A ticket is one piece of work: what should change, why, and how you'll know it's done. You write tickets, put them in
order, and start agents on them; the board shows where each one is.

Open **Tickets** in the rail (or press **Ctrl+]** then **l**). At the top, **Backlog** and **Board** switch between
the two views of the same tickets.

## Stages

Every ticket is in one stage:

| Stage | Means |
| --- | --- |
| **Draft** | Written down, not ready to be worked on yet. New tickets start here. |
| **Ready** | Refined and ready, but not picked yet. |
| **To do** | Picked: on the board, waiting to start. |
| **In progress** | Started: a branch, a worktree and (usually) an agent are working on it. |
| **Done** | Finished, normally because its branch merged. |
| **Won't do** | Closed without doing it. |

The **Backlog** is the list of what's not picked yet: **Draft** and **Ready** tickets, in your order. The **Board** is
what's picked and in flight: **To do**, **In progress** and **Done** (tickets closed in the last two weeks) as columns.

To move a ticket, drag its card to another column on the board, or use **Move to** in its menu (**⋯**). On a ticket's
own page the stage stepper under the title moves it too.

If a project runs the **Pipeline** workflow, **In progress** splits into **Code**, **Review**, **QA**, **Accept** and
**Merge**, and each card says who has it: "Reviewer · working", "Your turn", and so on. See
[Workflows](../automation/workflows.md).

Moving a ticket that is being worked on back to **Draft**, **Ready** or **To do** stops its work: PDT asks first, then
stops the agent and ends the attempt (the worktree stays).

## Write a ticket

1. Click **New ticket**.
2. Pick the **Project**, and give it a **Title** and a **Description** (Markdown; **Preview** shows it rendered). The
   agent that works on the ticket reads exactly this, so say what should change, why, and how to check it.
3. Optionally set the **Stage**, **Priority** and **Labels**.
4. Click **Create ticket**.

For a quick one, type a title in the box at the top of the backlog and press **Enter**: it goes in as a **Draft** at
the top. **Shift+Enter** opens the full form with the title filled in.

Click a ticket to see it in a panel beside the list; **Open** gives it a tab of its own. There you can edit the title
and description, change the details, add links, and comment. Comments go on the ticket's **Activity**, along with
everything else that happened to it: moves, starts, merges, agents' outcomes.

## Screenshots and logs

To show what's wrong, paste or drop a screenshot or a text file (a log, a JSON response, a config) into a ticket's
description or a comment box, or pick one with the paperclip (**Attach files**). PDT uploads it and puts it in the
text: an image shows in the comment, and a click opens it full size; a text file shows as a chip that opens it in a
viewer. Each upload shows its progress, and one that fails says why until you dismiss it.

PDT takes PNG, JPEG, GIF and WebP images up to 10 MB, and text files (`.txt`, `.log`, `.md`, `.json`, `.csv`,
`.yaml`, `.toml` and other plain text) up to 1 MB, at most 50 per ticket. The ticket's **Attachments** list (beside
the description; under **Links** on a phone) shows them all, with **Attach** to add one and a remove button for each.
Deleting the ticket deletes its attachments.

Agents working on the ticket see its attachments and open the images and files they need. QA can put screenshots
of what it tried on the ticket, and [the PM](../automation/the-pm.md) can attach an image you paste into its chat
to a ticket it creates or edits.

## Priority and labels

**Priority** is **Urgent**, **High**, **Medium**, **Low** or **No priority**. It sorts and groups the backlog, and by
default it decides which ticket auto-start takes first (below).

**Labels** are short lower-case tags you make up as you go (`ui`, `bug`, `docs`). Type one in a ticket's **Labels**
to add it; the backlog filters and groups by them.

One label means something to PDT: **`agentic`**. It says the ticket can go all the way without you. On the
**Pipeline**, a ticket labelled `agentic` skips **Accept** and merges once the reviewing and testing agents pass it;
any other ticket waits for you to accept it in the [Inbox](the-inbox.md). Tickets that came from outside PDT (an issue,
say) only get it from you.

## Parts and epics

A ticket too big for one pull request becomes a parent with **parts**: smaller tickets, each started, reviewed and
merged on its own. An epic is just a parent ticket.

- **Add parts** in the parent's **Parts** section: type a title in **New part… Enter to add** and press **Enter**. With
  **Starts after** ticked (the default), the new part is blocked by the one before it, so the parts go in order.
- **Make into parts…** (in the ticket's menu) makes several parts in one go, starting from the list in the
  description's Scope or Parts section if it has one. **Each part starts after the previous one** chains them.
- **Attach an existing ticket** with its **Parent** field.

A parent is never started itself; **Start next part** starts its first open part that nothing blocks. Starting a part
moves its parent to **In progress**, and the parent moves to **Done** once every part is closed. A parent can't be
closed while any of its parts is open.

On the board, each parent gets its own lane, headed by its card with a stepper that shows how far its parts have got.
A part's agent gets the parent's description and the list of the other parts along with its own ticket.

## Links and blocking

A ticket's **Links** section links it to other tickets:

- **Blocked by** / **Blocks**: one has to be done before the other can start.
- **Related**: worth knowing about, no order.

A ticket with an open blocker shows **Blocked**. When you start it, PDT warns you first (below), and auto-start skips
it until the blocker closes.

Tickets that would change the same parts of the code also count. Projects can name their **areas** (folders that
belong together); while one ticket is in progress, another in the same area shows an overlap, and the ticket's
**Overlaps** section lists the shared areas and the files both branches already change. An overlap in an area marked
exclusive (say, database migrations) is a conflict. See [Run configuration](../automation/run-configuration.md).

## Search and filters

- **Search tickets** finds words in titles and descriptions. Type a ticket's key (`ABC-12`) to jump to it.
- The filters narrow the list by **Stage**, **Priority**, **Labels**, **Blocked** and **Overlap**. **Show closed**
  brings in **Done** and **Won't do** tickets. **Clear filters** resets them.
- **Sort** by rank (your order), priority, last updated or created, and **group** by stage, priority or label.
- **Save filter…** keeps the current filters to come back to. **Ready to start** is built in: **Ready** tickets that nothing blocks and nothing
  conflicts with.

In rank order, drag rows to reorder the backlog, or press **Alt+↑** / **Alt+↓**. **Move to top** is in the row menu.

**Select** in a row's menu (or the row checkboxes) picks several tickets: the bar that appears moves, labels,
prioritises or deletes them all at once.

## Start a ticket

**Start** (on the ticket, its card or its row) opens the **Start** dialog:

- **Session**: **Conversation** (a chat view of the agent's work), **Terminal** (the agent's own interface), or **No
  session** (just the branch and worktree, for you to work in).
- **Agent**: which agent works on it.
- **Branch** and **Base branch**: PDT names the branch after the ticket and makes it from the base.
- **Permissions**: what the agent may do without asking you. See
  [Models and permission modes](workspaces-and-sessions.md#models-and-permission-modes).
- **Open in a new workspace**: the ticket and its session get a workspace of their own.

Click **Start**. PDT then:

1. makes the branch and a worktree for it (a separate folder, so several tickets can be worked on at once);
2. moves the ticket to **In progress** and records an attempt (one at a time per ticket);
3. starts the agent in the worktree, with the ticket (its description and comments) as the first message;
4. opens the ticket's tab with the agent's session beside it.

If something stands in the way (an open blocker, an overlap with a ticket in progress, or the project's limit on
tickets running at once), the dialog lists it. **Start anyway** starts regardless; **Wait** marks the ticket as
waiting until it's clear, and it shows **Waiting until clear** until then.

Comments you add while an agent works on the ticket are sent to that agent.

To stop working on a ticket, move it back to **To do** (or **Draft** or **Ready**), or use **Abandon attempt…** in its
menu.

## Auto-start

A project can start tickets by itself. In **Settings**, on the project's page:

- **Auto-start**: **Off**, **To do tickets**, or **To do, then Ready tickets**.
- **Running tickets at once**: how many tickets may be in progress together. Auto-start starts the next one whenever
  the project is under this number (one at a time when there's no cap).
- **Auto-start picks by**: **Priority, then rank** (the most urgent first, then backlog rank) or **Backlog rank** (the top ticket
  first, whatever its priority). Either way **To do** tickets come before **Ready** ones.
- **Quiet period before auto-start**: a ticket is taken only once it has gone this long unchanged (two minutes by
  default), so you have time to link its blockers or finish editing it. Its card shows "Auto-starts in 1:30" meanwhile.

Auto-start skips parents, tickets that are blocked or conflict with work in progress, and tickets from outside PDT you
haven't let in yet. It starts tickets with your default agent, view and permission mode (**Settings › This host ›
Agents**), or hands them to the project's workflow when there is one. Starting a ticket yourself is never delayed.

## On a phone

The top bar's title picks the project and saved filters; **Backlog** and **Board** sit under it, with search and the
filters beside them. The board shows one column at a time: pick it from the chips, or swipe sideways. There's no
dragging: long-press a row or card for its menu (move, rank, select). **＋** adds a ticket. A ticket opens full screen,
with **Overview**, **Conversation** and **Changes**, and the stage's main action (**Start**, **Accept…**, **Open
work**) at the bottom. In **Comment…**, the paperclip attaches a photo or a file.
