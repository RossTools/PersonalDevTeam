# The Inbox

The **Inbox** is everything waiting for you, in one list: agents' questions and permission requests, work to accept,
steps that got stuck, and news worth knowing. When agents and workflows run on their own, it's the one place to check.

Open **Inbox** in the rail (or **Ctrl+]** then **i**). The number on its icon is how many things wait on you.

## Needs you and Updates

The Inbox has two lists:

- **Needs you**: things that wait for your answer. Nothing moves on until you deal with them.
- **Updates**: news, newest first, unread in bold: an agent finished its turn, CI failed, a pull request merged, a
  workflow failed. **Mark all read** and **Clear all** tidy it; **Clear** removes one.

Pick an item to see it and its choices beside the list. Keys: **j** / **k** move down and up, **Enter** takes the main
action, **o** opens the item's ticket, **e** dismisses it.

## What lands in Needs you

**Needs you** groups its items by kind:

- **Questions**: an agent asked you something. Pick an answer for each question (or write your own) and **Send
  answers**. A request for a secret (a token, a password) is a question too, with a masked field: the value goes
  straight to PDT's secret store, never to the agent. See [Secrets](../your-machine/secrets.md).
- **Approvals**: an agent wants permission to run a command or edit a file. **Allow**, **Deny**, or **Always allow in
  this session**. The same request shows in the agent's conversation; answering either place answers both.
- **Sessions waiting**: an agent session is waiting on you in its own terminal or conversation. **Open session** takes
  you there; looking at the session takes it off the list.
- **Stalled**: a workflow step's agent stopped without saying it was done. **Nudge** sends it a reminder (with your
  own words if you like); **Take over** finishes the step for it, with the outcome you pick and a note on what you did.
- **Loop limit**: a step has gone round as many times as it may (a review that keeps sending the work back, say). Let
  it go round once more, pick the outcome yourself, or stop.
- **Unheard notices**: news an agent should have heard but couldn't, because none was running: a pull request's CI
  failed, it conflicts with the base branch, or it fell out of the merge queue. **Open ticket** to deal with it; the
  item goes away by itself once the pull request is fixed or merged. **Connect GitHub** shows here too while PDT needs
  a GitHub login to follow your pull requests: **Enter** opens the GitHub settings.
- **Triaged tickets**: a workflow drafted a ticket from a new issue. **Approve** moves it to **To do**, **Reject** to
  **Won't do**. Moving it on the board yourself does the same.
- **Gates**: a workflow asks you to decide before it goes on, or it's your turn to do a step by hand. Pick a choice;
  the optional note goes on the ticket as your comment, so a "no" can say what to change.
- **Ready to accept**: the reviewing and testing agents passed a ticket's work, and it waits for you before it
  merges. Accepting it shows the pull request, its CI and the size of the change first; reject it with a note, and the
  note goes back to the coding agent.

An item about a ticket in progress also offers **Open coder session** (the agent working on it), **Open changes** (the
[Changes view](review-and-merge.md#the-changes-view)) and **Run…** (one of its runs or tests), each opened in the
ticket's workspace.

**Dismiss** closes a question, a stalled item or a notice you don't need. Answered and dismissed items disappear;
open ones stay however old they are.

## From your phone

On a phone the Inbox is a list with **Needs you** and **Updates**. **Review all** goes through the items one at a time:
each shows its ticket, what it's about, and its choices in a bar at the bottom, with the main one on the right.
Answering one shows the next; swipe sideways to skip; "All clear" means you're done. The bell in the top bar shows the
count and opens the Inbox from anywhere.

To reach PDT from your phone at all, see [Remote access](../your-machine/remote-access.md).

## Notifications

Each kind of notification shows as you choose in **Settings › Notifications**:

- **Off** or **Dot only** (just the marker on the session or ticket);
- **Inbox**: a row in the Inbox;
- **Inbox and toast**: also a pop-up in the corner while PDT is open;
- **Desktop notification**: also a system notification while PDT is in the background.

Each level includes the ones before it. Things that wait on you never go below **Inbox**, so you can always find them.

Desktop notifications are per device: turn on **Desktop notifications** under **This device** on each computer or
phone you want them on, and allow them when the browser asks. **Test** sends one. Clicking a notification opens what
it's about.

## The tab's title and icon

PDT's browser tab tells you when something is waiting, even when it's in the background:

- the title starts with the Inbox count: `(3) PDT`;
- the icon gets a dot: **red** when an agent session needs you, **amber** when anything else waits in the Inbox, and
  **blue** when an agent or run finished and you haven't looked yet, or the project manager replied.

With nothing waiting, the title and icon go back to normal.
