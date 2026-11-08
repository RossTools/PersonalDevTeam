# The Inbox

The **Inbox** is everything waiting for you in one list: agents' questions and permission requests, work to accept,
stuck steps, and news. When agents and workflows run on their own, it's the one place to check.

Open **Inbox** in the rail (or **Ctrl+]** then **i**). The number on its icon is how many things wait on you.

## Needs you and Updates

- **Needs you**: things waiting for your answer. Nothing moves on until you deal with them.
- **Updates**: news, newest first, unread in bold (an agent finished its turn, CI failed, a pull request merged, a
  workflow failed). **Mark all read**, **Clear all**, or **Clear** one.

Each row shows the kind's icon, the item's title, its ticket and who asks, and how long it has waited. Pick one to see
it beside the list, laid out the same for every kind:

- at the top, the kind, the ticket, who asks and when, the title, and links to the **Ticket**, the agent's **session**,
  its **Changes** (with lines added and removed: the [Changes view](review-and-merge.md#the-changes-view)), its pull
  request or CI log, and **Run…**, each opened in the ticket's workspace;
- one line on why it waits on you, then what it's about: the agent's questions, a command, a step's passes, a notice;
- at the bottom, a note box where the item takes one, then the main action, the other choices, and **Dismiss**,
  **Stop** or **Won't do** on the right.

Keys: **j** / **k** down and up, **Enter** the main action, **o** open its ticket, **e** dismiss, **1**–**9** pick a
question's choice, **y** / **n** allow or deny an approval.

## What lands in Needs you

Items are listed in five sections, most urgent first.

### Questions

An agent asked you something. The title is the question in one line; what the agent found and why it asks shows
under it as the agent's message. Each question is a card: each choice has its label and what it means, with its pros
and cons. The choice the agent recommends is marked **Recommended**, says why, and is already picked: nothing is sent
until you press **Send answers**. Pick another choice (or press its number), or pick **Something else…** and write
your own. **Send answers** says how many you've answered. A request for a secret (a
token, a password) has a masked field, and the value goes straight to PDT's secret store, never the agent (see
[Secrets](../your-machine/secrets.md)). A question an agent asks in its conversation is listed here too, titled with
the question, and shows the same cards: answering it here or in the conversation answers both. Where the agent lets you
pick several choices, they are tick boxes, and what you write under **Something else…** is added to them.

### Approvals

An agent wants to run a command or edit a file, shown as it would run: **Allow once**, **Always allow**, or **Deny**
(with an optional reason). Answering here or in the agent's conversation answers both. A call to one of PDT's own tools
from an agent that can't ask in its own prompt shows here too, with what it would create or change: in a conversation it is also a card in
the conversation. From a terminal session it is only here. Either way the agent waits for your answer and carries on
once you give it, with your reason if you deny it. If you take a day or more, the agent stops waiting and the approval stays open: your answer then
reaches the agent as a message, with the result of the call you allowed. If the call holds a secret's value, the
approval shows the secret's name in its place and says so; allowed only after the agent stopped waiting from a terminal
session, it runs nothing and the agent is asked to make the call again. When the PM proposes decisions on
your ticket proposals, they wait here and in the PM's conversation as one table: untick the rows you don't want, then
**Allow** the rest (**y**), or **Deny** them all (**n**) (see
[Decide Inbox items](../automation/the-pm.md#decide-inbox-items)).

### Stuck

- **Stalled**: a workflow step's agent stopped without saying it was done, and the nudges and fresh session of
  [When an agent is stuck](../automation/personas-and-approvals.md#when-an-agent-is-stuck) didn't get it going.
  **Nudge** reminds it (optionally in your words); **Take over** finishes the step with the outcome you pick.
- **Loop limit**: a step has gone round as often as it may (a review that keeps sending work back, say). Its passes are
  listed with each one's outcome. Allow one more round, pick the outcome yourself, or stop.
- **Stalled tickets**: nothing moves a ticket on. Its pull request's CI was cancelled and re-running it once didn't
  help (**Re-run CI** runs it again), or the ticket has waited on nothing for a while. The pull request is a link away,
  and the item goes by itself once something happens on the ticket again.
- **Timed out**: a step ran out of time. **Resume** runs it again (with an optional note), **Stop** leaves it.

### To act on

- **Unheard notices**: pull request trouble (CI failed, a conflict, out of the merge queue) with no agent running to
  hear it (see [Pull request and CI status](review-and-merge.md#pull-request-and-ci-status)). **Open ticket** to deal
  with it; it clears once the pull request is fixed or merged.
- **Sessions waiting**: a session is waiting on you in its terminal or conversation. **Open session** goes there;
  looking at it clears the item.
- **Connect GitHub** while PDT needs a GitHub login to follow your pull requests, and **Sign in to … again** while an
  MCP server's sign-in no longer works (see [MCP servers](../your-machine/mcp-servers.md#sign-in-with-oauth)).
- **PDT can't save changes**: its disk is full (with how much is free) or its database can't be written. Workflow
  runs, events and what agents say in conversations wait meanwhile (conversations carry on as normal), and what you
  change says "PDT can't save changes" instead of saving. Free up space; PDT saves again on its own within seconds,
  starts what waited, and the item goes.
- **Low disk space**: a disk with PDT's data or a project is running out of space (with how much is free).
  **Open Storage** shows what each worktree takes; **Clean all finished** removes finished tickets' build output. When
  it is almost full, pickup pauses too (auto-start starts no new tickets). The item goes once there is more space (see
  [Disk space](../your-machine/your-data.md#disk-space)).
- **Triaged tickets**: a workflow drafted a ticket from a new issue. **Approve** moves it to **To do**, **Reject** to
  **Won't do** (moving it on the board does the same).
- **Ticket proposals**: an agent filed a ticket while it worked (a follow-up a Coder or QA found). It waits in
  **Draft**, off the board, until you decide. The item shows the ticket's title and description, which agent proposed
  it, the ticket it was working on (with a link) and the agent's reason. Choose:
  - **Accept to Todo** or **Accept to backlog** (Ready). Tick **Label it agentic** first if agents may pick it up
    (it's unticked to start with). **Edit** changes the title, description, priority and labels before you accept.
  - **Duplicate of…**: pick the ticket it repeats. It goes to **Won't do**, linked to that ticket.
  - **Reject**: it goes to **Won't do**.

  The optional reason goes to the agent with your decision: in its session if it's still running, otherwise as a
  comment on the ticket it was working on. Tickets you or the PM make, and parts from **Make into parts**, aren't
  proposals. To let agents' tickets go straight to the board, turn off the **Ticket proposals** workflow (see
  [Workflows](../automation/workflows.md#turn-a-workflow-on-or-off)). With many waiting, the PM can propose a
  decision for each and you confirm them together (see [Decide Inbox items](../automation/the-pm.md#decide-inbox-items)).
- **Workflow proposals**: an agent (the PM, say) changed one of your workflows. The item shows their note and what
  changed. **Accept** makes it the current version, **Discard** sets it aside, **Versions** shows the whole change in
  the workflow's **Versions** tab. The item goes once you decide, or when a newer proposal replaces it.

### Ready to accept

- Review and QA passed a ticket's work and it waits for you before merging. Accepting shows the pull request, its CI
  and the change's size first; a rejection's note goes back to the coding agent.
- Gates: a workflow wants your decision, or it's your turn to do a step by hand. The full question is shown formatted,
  with its links, and the links row has its pull request (merge request on GitLab). Pick a choice; the optional note
  goes on the ticket as your comment, so a "no" can say what to change.

**Dismiss** closes a question, stalled item or notice you don't need. Answered and dismissed items go; open ones stay
however old they are.

## From your phone

The Inbox is a list with **Needs you** and **Updates**. **Review all** goes through the items one at a time, each with
the same layout as on a computer, and its choices in a bar at the bottom (the main one on the right). Answering shows the
next; swipe sideways to skip; "All clear" means you're done. The bell in the top bar shows the count and opens the
Inbox from anywhere.

To reach PDT from your phone, see [Remote access](../your-machine/remote-access.md).

## Notifications

In **Settings › Notifications**, each kind of notification shows as:

- **Off** or **Dot only** (a marker on the session or ticket);
- **Inbox**: a row in the Inbox;
- **Inbox and toast**: also a pop-up while PDT is open;
- **Desktop notification**: also a system notification while PDT is in the background.

Each level includes the ones before it. Things that wait on you never go below **Inbox**.

The same page has rules you can turn on:

- **Escalate what waits**: something still unanswered in the Inbox after a while (2 hours unless you change it) comes
  back as a desktop notification, and goes to your chat channel if you set one.
- **Quiet hours**: between two times (22:00 to 07:00 unless you change them), nothing pops up. What would have been a
  toast or a desktop notification waits in the Inbox, and one summary comes when quiet hours end.
- **Chat channel**: questions, approvals and failed workflow runs are posted to a chat channel, such as a Discord
  webhook. You can pick which of the three are posted. It's there while a plugin that posts notifications (Discord) is
  on.

All of this is the **Notification rules** workflow, and the page sets it for you. To go further (different levels for
one project, a rule of your own), use **Customize as a workflow →**. Once you change the workflow beyond these settings,
the page says **Notifications follow your customized workflow** and offers **Open** and **Reset to default**. Turn the
workflow off and every kind shows at PDT's default level, with none of the rules.

Desktop notifications are per device: turn on **Desktop notifications** under **This device** on each one and allow
them when the browser asks. **Test** sends one; clicking a notification opens what it's about.

## The tab's title and icon

PDT's browser tab shows when something is waiting:

- the title starts with the Inbox count: `(3) PDT`;
- the icon gets a dot: **red** when an agent session needs you, **amber** when anything else waits in the Inbox,
  **blue** when an agent or run finished and you haven't looked, or the PM replied. An agent still waiting on
  background tasks hasn't finished yet.
