# The Inbox

The **Inbox** is everything waiting for you in one list: agents' questions and permission requests, work to accept,
stuck steps, and news. When agents and workflows run on their own, it's the one place to check.

Open **Inbox** in the rail (or **Ctrl+]** then **i**). The number on its icon is how many things wait on you.

## Needs you and Updates

- **Needs you**: things waiting for your answer. Nothing moves on until you deal with them.
- **Updates**: news, newest first, unread in bold (an agent finished its turn, CI failed, a pull request merged, a
  workflow failed). **Mark all read**, **Clear all**, or **Clear** one.

Pick an item to see it and its choices beside the list. Keys: **j** / **k** down and up, **Enter** the main action,
**o** open its ticket, **e** dismiss.

## What lands in Needs you

- **Questions**: an agent asked you something. Pick or write an answer for each and **Send answers**. A request for a
  secret (a token, a password) has a masked field, and the value goes straight to PDT's secret store, never the agent
  (see [Secrets](../your-machine/secrets.md)).
- **Approvals**: an agent wants to run a command or edit a file: **Allow**, **Deny**, or **Always allow in this
  session**. Answering here or in the agent's conversation answers both.
- **Sessions waiting**: a session is waiting on you in its terminal or conversation. **Open session** goes there;
  looking at it clears the item.
- **Stalled**: a workflow step's agent stopped without saying it was done. **Nudge** reminds it (optionally in your
  words); **Take over** finishes the step with the outcome you pick and a note on what you did.
- **Loop limit**: a step has gone round as often as it may (a review that keeps sending work back, say). Allow one
  more round, pick the outcome yourself, or stop.
- **Unheard notices**: pull request trouble (CI failed, a conflict, out of the merge queue) with no agent running to
  hear it (see [Pull request and CI status](review-and-merge.md#pull-request-and-ci-status)). **Open ticket** to deal
  with it; it clears once the pull request is fixed or merged. **Connect GitHub** shows here too while PDT needs a
  GitHub login to follow your pull requests, and **Sign in to … again** while an MCP server's sign-in no longer works
  (see [MCP servers](../your-machine/mcp-servers.md#sign-in-with-oauth)).
- **PDT can't save changes**: its disk is full (with how much is free) or its database can't be written. Workflow
  runs, events and what agents say in conversations wait meanwhile (conversations carry on as normal), and what you
  change says "PDT can't save changes" instead of saving. Free up space; PDT saves again on its own within seconds, starts what waited, and
  the item goes.
- **Low disk space**: a disk with PDT's data or a project is running out of space (with how much is free).
  **Open Storage** shows what each worktree takes; **Clean all finished** removes finished tickets' build output. When
  it is almost full, auto-start waits too. The item goes once there is more space (see
  [Disk space](../your-machine/your-data.md#disk-space)).
- **Triaged tickets**: a workflow drafted a ticket from a new issue. **Approve** moves it to **To do**, **Reject** to
  **Won't do** (moving it on the board does the same).
- **Workflow proposals**: an agent (the PM, say) changed one of your workflows. The item says who proposed it, their
  note and what changed. **Accept** makes it the current version, **Discard** sets it aside, **Open** shows the whole
  change in the workflow's **Versions** tab. The item goes once you decide, or when a newer proposal replaces it.
- **Gates**: a workflow wants your decision, or it's your turn to do a step by hand. The full question is shown
  formatted, with its links. **Open pull request** opens a gate's pull request (**Open merge request** on GitLab). Pick
  a choice; the optional note goes on the ticket as your comment, so a "no" can say what to change.
- **Ready to accept**: review and QA passed a ticket's work and it waits for you before merging. Accepting shows the
  pull request, its CI and the change's size first; a rejection's note goes back to the coding agent.

An item about a ticket in progress also offers **Open coder session**, **Open changes** (the
[Changes view](review-and-merge.md#the-changes-view)) and **Run…**, each opened in the ticket's workspace.

**Dismiss** closes a question, stalled item or notice you don't need. Answered and dismissed items go; open ones stay
however old they are.

## From your phone

The Inbox is a list with **Needs you** and **Updates**. **Review all** goes through the items one at a time, each with
its ticket, what it's about, and its choices in a bar at the bottom (the main one on the right). Answering shows the
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

Desktop notifications are per device: turn on **Desktop notifications** under **This device** on each one and allow
them when the browser asks. **Test** sends one; clicking a notification opens what it's about.

## The tab's title and icon

PDT's browser tab shows when something is waiting:

- the title starts with the Inbox count: `(3) PDT`;
- the icon gets a dot: **red** when an agent session needs you, **amber** when anything else waits in the Inbox,
  **blue** when an agent or run finished and you haven't looked, or the PM replied. An agent still waiting on
  background tasks hasn't finished yet.
