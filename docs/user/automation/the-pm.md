# The PM

The PM is a project manager for your whole machine: an agent you talk to about the work rather than the code. It sees
every project, its tickets and what's in flight, and it can plan, write and tidy tickets for you.

Open **Project manager** in the rail (or **Ctrl+]** then **m**). A dot on it means the PM has replied and you haven't
looked yet.

## What to ask it

- **Plan a feature**: "Plan dark mode for the web app into tickets." It reads the code, drafts the tickets in its
  reply, and creates them as **Draft**s once you agree, split into parts and linked in order when that helps.
- **Write or refine a ticket**: "Turn my note about slow search into a ticket with a Done when."
- **Tidy the backlog**: "Which tickets are duplicates or out of date?", "Rank the bugs by how much they hurt."
- **Sort ticket proposals**: "Go through the ticket proposals in my Inbox." It reads each one and proposes Accept to
  Todo, Accept to backlog, Duplicate of… or Reject for all of them at once; you confirm them in one step (see
  [Decide Inbox items](#decide-inbox-items)).
- **Say what's going on**: "What's in flight?", "Why is ABC-12 stuck?", "What needs me?" It reads the board, pull
  requests, CI, running agents and the Inbox.
- **Talk to an agent**: "Tell the agent on ABC-12 to rebase before it pushes again." A comment the PM adds to a
  ticket that is being worked on reaches its agent too, with any screenshots or files attached, as yours do
  (**Ticket comments to the agent**, in [Workflows](workflows.md)).
- **Keep an eye on something**: "Tell me when ABC-12's agent reports", "Let me know when the release run finishes."
  See [Watch a ticket or run](#watch-a-ticket-or-run).
- **Run or write a workflow**: "Cut a minor release", "Write a workflow that posts merged tickets to Discord." A
  workflow it writes is a proposal you accept or discard in the workflow's **Versions**.
- **Put a screenshot on a ticket**: paste it into the chat and say "Make a ticket for this bug, with the screenshot."
  The agent that works on the ticket opens the image (see
  [Screenshots and logs](../using-pdt/tickets-and-the-board.md#screenshots-and-logs)).
- **Report a bug in PDT itself**.

It can also read and search your projects' code, use git and search the web. With a plugin on (Jira, Discord…), it
can use that service too: see [Plugins and connections](plugins.md).

For a release, the PM uses the project's enabled release workflow, such as
[Cut a release](workflows.md#cut-a-release). For a project without one, tell it how you release.

The PM runs as the **PM** persona: change its agent, model or instructions in **Personas** (see
[Personas and approvals](personas-and-approvals.md)).

## Conversations

Keep one conversation per topic (a release, a feature, a clean-up). Several can be open at once, each with its own
history. The list shows the open ones, most recently active first; an unread reply is in bold with a dot, and a
working one shows a spinner.

- **New conversation** starts one; its first message names it. **Rename** (in a conversation's menu) changes the name.
- **Close…** when the topic is done: the PM writes a short journal entry (what was done, what's still open) and the
  conversation becomes read-only. The newest ended ones are listed under **Ended**, the rest under **All ended
  conversations**.
- **Reset…** when the topic carries on but the conversation has grown long or muddled: the PM writes its journal
  entry, and a fresh conversation takes its place with the same name, agent, model and permission mode, starting from
  that entry. It runs the PM persona as it is now, with any tools and instructions changed since. **Previous conversation** at its top opens the old history, read-only.

A conversation never ends on its own. When it gets long, the agent condenses its history, writing a journal
checkpoint first so nothing important is lost.

## What it knows when it starts

Each new conversation starts with a briefing: your projects and their tickets by stage, the tickets in progress, the
Inbox, what happened since the last conversation started, the last few journal entries, the other open conversations,
and the pinned notes. So you can ask "what's new?" without explaining anything.

About once a day, a conversation you've written to since is brought up to date: what happened to each ticket (moved,
merged, CI failed…), what waits in the Inbox, and what other conversations noted as still open. It shows as one line,
"PDT brought this conversation up to date", which opens to the full text. The PM then writes a journal checkpoint; on
a day with nothing new, it's asked only for the checkpoint.

### Change when it's brought up to date

Set it in **Settings › This host › PM**, under **Daily checkpoint**:

- **At a set time**: send the updates once a day at a **Time** (this computer's time). Off, PDT looks every hour and
  brings up to date each conversation whose last update was a day or more ago.
- **Days**: the days updates go out, every day unless you pick fewer.
- **Chat channel**: post the daily update to a chat service as well, such as a Discord webhook. It shows when a
  plugin that posts to chat is on.

These settings are the built-in workflow **PM daily checkpoint** (see
[Workflows](workflows.md#the-built-in-workflows)). For more, use **Customize as a workflow →** and change it in
**Workflows**: turn off **Only in use** to update conversations you haven't written to, say. Once you change it
there beyond these settings, **Settings** says the daily checkpoint follows your customized workflow, with **Open**
and **Reset to default**. The same shows when you turn the workflow off, and **Reset to default** turns it back on.

Turned off, conversations still write their journal when you close or reset them, and when their history is
condensed.

## When it asks first

Before the PM creates, edits, stops or links a ticket, messages an agent, or runs or saves a workflow, it asks you. An
approval card in the conversation (and under **Approvals** in the Inbox) says exactly what it's about to do: **Allow**
or **Deny**, and on most agents **Always allow in this session** (Claude asks every time unless it runs in Bypass).
Answer in either place; the PM carries on in the same turn once you do, whichever agent it runs on.

Reading never asks, and nor does writing its own notes.

To let it act on a project's tickets without asking, turn on **The PM acts without asking** in **Settings**, on the
project's page, under **Pipeline and policies**. A plugin's connection can have the same setting for its service.

These always ask, whatever the setting:

- moving a ticket that came from outside PDT (an issue, say) onto the board, or labelling it `agentic`;
- reporting a bug in PDT;
- deciding Inbox items for you (below).

## Decide Inbox items

When ticket proposals pile up, ask the PM to sort them. It proposes a decision for each, and nothing changes until you
confirm: a card in the PM's conversation lists every proposal with its ticket, the PM's decision (and any edits, such
as a new priority or the `agentic` label) and its reason. Untick the rows you don't want, then **Allow** the rest, or
**Deny** them all. The same table is under **Approvals** in the Inbox, and answering in either place answers both. This
always waits for you, whatever the PM's agent and the project's setting, and the PM carries on with what you decided
once you answer.

Each row you allow is decided exactly as if you had clicked it in the Inbox. The ticket's activity shows it as yours,
**proposed by PM**. The rows you untick stay in the Inbox for you to decide. Only the PM can propose these decisions:
the agents working on tickets aren't given a way to decide proposals, theirs or anyone else's.

## Watch a ticket or run

Ask the PM to keep an eye on a ticket or a workflow run, and it hears when something happens instead of checking
again and again: the ticket's agent reports, the ticket is done or closed, its work merges, CI fails, it waits on you,
or the run succeeds, fails or is cancelled. Several things at once arrive as one message, once the PM is free, and it
tells you.

Watching asks nothing: it only tells the PM's own conversation. The watch ends once the ticket or run is finished,
unless you ask for more. A watched ticket or run shows **Watched by PM “…”**, and **×** beside it stops that watch. If
you close the conversation, what it hadn't heard yet comes to your Inbox instead.

While a conversation watches anything, an eye with a count sits at the top of it (and beside it in the list of
conversations). Click it to see each ticket or run it watches, what it listens for, whether the watch ends when it
finishes, and since when; **Stop watching** ends one. Watched tickets have a small eye on the board and in the backlog:
hover it to see who watches.

To watch something yourself, see [Watch a ticket](../using-pdt/tickets-and-the-board.md#watch-a-ticket).

## Notes and the PM's memory

Open **Notes** in the rail (**Ctrl+]** then **o**). **Mine | PM's memory** switches between:

- **Mine**: your own notes and ideas. The PM can find and read them but never changes or deletes them. **Make a
  ticket** turns a note into a draft ticket.
- **PM's memory**: what the PM saves to remember (decisions, how a project works, what you prefer). You can read, edit
  and delete them.

**Pin** a note in the PM's memory to put it in every PM conversation from the start; pinning one of yours only lists
it first. A note is host-wide or belongs to one project, and saves as you type.

## Talk to it from Discord

With the **Discord** plugin on, you can DM the PM from anywhere Discord runs.

1. In the [Discord Developer Portal](https://discord.com/developers/applications), open your bot's application →
   **Bot**, and under **Privileged Gateway Intents** turn on **Message Content Intent**.
2. Add a **Discord bot** connection (or edit yours) and fill in **Your Discord user ID**: in Discord, turn on
   Developer Mode (Settings → Advanced), then right-click your name → **Copy User ID**.
3. Save. The connection shows **DMs with the PM: Connected** once the bot is online; if it can't connect (a wrong
   token, Message Content Intent off), it says why.

Now DM the bot ("What's in flight?"). Your messages go to your current Discord conversation, an ordinary PM
conversation marked **via Discord** in PDT. You see "typing…" while it works, and the reply comes as a DM (a very long
one as a `.md` file), headed with the conversation's name when it isn't the one you last wrote to.

DM commands:

- `/new [topic]` starts a conversation and makes it current.
- `/list` shows the open conversations.
- `/switch` moves to another one (it suggests them as you type).
- `/close` closes the current one; your next message starts a new one.

Approval requests come with **Approve** and **Decline** buttons, and the PM's questions with a button per answer (or
type one). The same request is in PDT: whichever you answer first counts.

Only your Discord account can talk to the PM or press its buttons; other people's DMs and server channels are ignored.
Turn on **Also send Inbox items (stalled, needs input) as DMs** on the connection to get those in Discord too.
