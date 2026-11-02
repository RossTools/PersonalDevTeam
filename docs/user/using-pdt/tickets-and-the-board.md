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
progress** and **Done** (closed in the last two weeks). To change how far back **Done** reaches, open the board's view
menu (**Done: last 14 days**, at the top) and pick the last day, 3, 7, 14 or 30 days. Each project keeps its own
choice on this computer; the board across all projects shows two weeks.

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

A ticket you didn't make carries a badge saying who did: **PM**, **Workflow**, **External**, or the persona of the
agent that filed it (**Coder**, say). Hover over an agent's badge to see the ticket it was working on: "Created by
Coder · from APP-12".

A ticket an agent filed while it worked is a **proposal** at first: it waits in your [Inbox](the-inbox.md#to-act-on),
not on the board or in the backlog, until you accept it. Opened from the Inbox, it shows a **Proposal** badge.

## Screenshots and logs

Paste or drop a screenshot or text file (a log, a JSON response, a config) into a description or comment, or use the
paperclip (**Attach files**). Images show inline and open full size; text files show as a chip that opens a viewer.

Limits: PNG, JPEG, GIF and WebP images up to 10 MB; plain-text files (`.txt`, `.log`, `.md`, `.json`, `.csv`,
`.yaml`, `.toml` and similar) up to 1 MB; 50 per ticket. The ticket's **Attachments** list shows them all, with
**Attach** and a remove button. Deleting the ticket deletes them.

Agents working on the ticket can open its attachments, QA can attach screenshots of what it tried, and
[the PM](../automation/the-pm.md) can attach an image you paste into its chat.

## Design docs and other documents

A ticket's **Documents** section lists the docs and sheets linked to it. **Create design doc** makes one from a short
template and links it; **Add** links one PDT already has, makes a new one or imports one. Agents working on the ticket
read its design doc first. See [Documents](documents.md).

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
progress** with its first part, and can't be closed while a part is open. On the board each parent gets its own lane,
with a stepper showing progress: a dot per part, or one bar when there are many parts or little room (hover it for
the counts). Each part's card shows its own pull request. A closed parent's lane stays as long as **Done** shows it. A
part's agent also gets the parent's description and the list of other parts.

When the last open part reaches **Done** or **Won't do**, the parent closes too: to **Done**, or to **Won't do** if
every part was **Won't do**. It gets an "All parts closed" comment and you get a notification. A parent that has work
of its own (an attempt or a running agent session) stays open, and the notification says so. Moving a part out of
**Done** or **Won't do** moves its closed parent back to **In progress**. This is the **Close finished parents**
[workflow](../automation/workflows.md); turn it off to close parents yourself.

## Links and blocking

A ticket's **Links** section has:

- **Blocked by** / **Blocks**: one must be done before the other starts.
- **Related**: no order.

A ticket with an open blocker shows **Blocked**; starting it warns you, and auto-start skips it.

### Issues in GitHub, GitLab and Jira

To tie a ticket to an issue that is already in a tracker, click **Link issue…** in its **Links** section
and paste the issue's address or key: `owner/name#12` (or `#12` for the project's own repository) on GitHub or
GitLab, `ABC-123` on Jira. The tracker's plugin must be on, and PDT reads the issue through your sign-in or the
project's connection, so private issues work too.

A linked issue shows beside the ticket's key, on its board card and in the backlog, with the tracker's icon, and in
**Links** › **Issues** with its title and whether it is open or closed. Opening the ticket reads them again when they
are a few minutes old. **⋯** › **Unlink** removes the link; nothing changes in the tracker. An issue can be linked to
one ticket at a time.

Moving a ticket to **Done** closes the issues PDT filed from it or imported it from (Jira moves its issue to the
project's done status). An issue you linked by hand is left as it is unless you tick **Close it when this ticket is
done** under it. GitLab issues are never closed this way.

An address no tracker recognizes (a design, some docs) can be added as a plain link instead: give it a label, or leave
it empty to show the site's name. Plain links are listed under **Other links**.

The PM links issues too, when you ask it: it asks before each one unless the project lets it act.

Tickets that change the same code count too. If a project names its **areas** (folders that belong together), a ticket
in the same area as one in progress shows an overlap, and its **Overlaps** section lists the shared areas and files
both branches change. An overlap in an exclusive area (database migrations, say) is a conflict. See
[Run configuration](../automation/run-configuration.md).

A ticket's areas are in its details, where you can change them. When the Refiner refines a ticket, or the PM drafts
one, it picks the areas from the project's list by what the ticket describes and the code it will touch.

## Watch a ticket

To hear about a ticket without opening it, click the eye (**Watch**) at the top of its page (on a phone, **Watch** in
the **⋯** menu). You're told in the Inbox's **Updates**, and on the desktop if you allow notifications, when the ticket
is done or closed, its work merges, CI fails or the merge queue drops it, or something about it waits on you. The watch
ends once the ticket is finished; click the eye again to stop sooner.

The ticket's **Watching** section lists who watches it: you, and any PM conversation you asked to keep an eye on it.
**×** beside one stops that watch. A workflow run has the same eye, on its page under the workflow's **Runs**.

## Search and filters

- **Search tickets** matches titles and descriptions; type a key (`ABC-12`) to jump to that ticket.
- Filter by **Stage**, **Priority**, **Labels**, **Blocked** and **Overlap**; **Show closed** adds **Done** and
  **Won't do**; **Clear filters** resets.
- Sort by rank, priority, updated or created; group by stage, priority or label.
- **Save filter…** keeps the current filters. The built-in **Ready to start** shows **Ready** tickets nothing blocks or
  conflicts with.

In rank order, drag rows (or **Alt+↑** / **Alt+↓**) to reorder; **Move to top** is in the row menu. **Select** (or the
row checkboxes) picks several tickets to move, label, prioritise or delete together. Tickets an action moves out of
the list leave the selection; **Clear** or **Esc** clears it.

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

### Won't do and Abandon

Moving a ticket at work to **Won't do**, or **Abandon attempt…**, ends its work:

- its sessions, terminals and runs stop, and its workflow runs are cancelled;
- its open pull request is closed, with a comment saying why (a merged one is left alone);
- its worktree, its branch (and the copy PDT pushed for it) and its workspace are removed, unless they hold work
  that isn't committed or pushed. Then everything is kept, the ticket's timeline says why, and **Clean up…** on the
  ticket's attempt removes it anyway.

The ticket's timeline lists what was stopped, closed and removed. To keep worktrees for a while, or always, change
**On Won't do** in the project's **Worktrees** settings (see [Disk space](../your-machine/your-data.md#disk-space)).
Starting the ticket again later begins afresh.

Moving a ticket to **Done** stops its sessions and runs too; its worktree is left to the automatic cleanup. When a
ticket's branch merges while it waits at **Accept**, the question goes away, its sessions stop, and the merged banner
offers to finish it.

## Auto-start

In **Settings**, on the project's page:

- **Auto-start**: **Off**, **To do tickets**, or **To do, then Ready tickets**.
- **Running tickets at once**: auto-start starts the next ticket whenever fewer than this many are in progress (one
  at a time with no cap).
- **Auto-start picks by**: **Priority, then rank** or **Backlog rank**. **To do** always comes before **Ready**.
- **Quiet period before auto-start**: a ticket must go this long unchanged first (two minutes by default), so you can
  finish editing it. Its card shows "Auto-starts in 1:30".
- **Hot areas**: areas where two tickets in progress at once often collide (shared API routes, prompts, docs). While a
  ticket in a hot area is in progress, auto-start doesn't start another ticket in that area. It takes the next ticket
  that doesn't collide. The waiting ticket's card says which ticket and area it waits on ("Waiting: overlaps …"),
  and it starts once that work is done.
- **Tickets without areas run alongside hot ones**: a ticket without areas might touch anything. On (the default), it
  starts like any other. Off, it waits while a ticket in a hot area is in progress, and a ticket in a hot area waits
  while one without areas is in progress.

If auto-start can't start a ticket for a reason that won't pass by itself (a git error, say), its card shows
**Auto-start failed** and the ticket says why. Auto-start leaves it until you change the ticket, or until you click **Try
again** on the ticket (or **Try auto-start again** in its menu).

Auto-start skips parents, blocked or conflicting tickets, and outside tickets you haven't let in. It uses your default
agent, view and permission mode (**Settings › This host › Agents**), or the project's workflow if it has one. Starting
a ticket yourself is never delayed.

Auto-start is the built-in **Pick up tickets** workflow, and these settings are its settings. To pick up tickets your
own way (only some labels, Ready tickets only at night), edit or copy it: see
[Pick up tickets](../automation/workflows.md#pick-up-tickets).

While pickup is paused, auto-start starts nothing new, and each ticket it would take shows **Pickup paused** with why.
PDT pauses it while a disk it uses is almost full (see [Disk space](../your-machine/your-data.md#disk-space)); a
workflow of yours can pause it too.

## On a phone

The top bar's title picks the project and saved filters, with **Backlog**, **Board**, search and filters below. The
board shows one column at a time: pick it from the chips or swipe. Long-press a row or card for its menu instead of
dragging; **＋** adds a ticket. A ticket opens full screen with **Overview**, **Conversation** and **Changes**, and its
main action (**Start**, **Accept…**, **Open work**) at the bottom. In **Comment…**, the paperclip attaches a photo or
file.
