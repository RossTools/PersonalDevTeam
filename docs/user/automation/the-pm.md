# The PM

The PM is a project manager for your whole machine: an agent you talk to about the work rather than the code. It sees
every project, its tickets and what's in flight, and it can plan, write and tidy tickets for you.

Open **Project manager** in the rail (or **Ctrl+]** then **m**). A dot on it means the PM has replied and you haven't
looked yet.

## What to ask it

- **Plan a feature**: "Plan dark mode for the web app into tickets." It reads the code, drafts the tickets in its
  reply, and creates them (as **Draft**s) once you agree, split into parts and linked in order when that helps.
- **Write or refine a ticket**: "Turn my note about slow search into a ticket with a Done when."
- **Tidy the backlog**: "Which tickets are duplicates or out of date?", "Rank the bugs by how much they hurt."
- **Say what's going on**: "What's in flight?", "Why is ABC-12 stuck?", "What needs me?" It reads the board, pull
  requests and CI, running agents and the Inbox.
- **Talk to an agent**: "Tell the agent on ABC-12 to rebase before it pushes again."
- **Keep an eye on something**: "Tell me when ABC-12's agent reports", "Let me know when the release run finishes."
  It watches the ticket or run (see [Watch a ticket or run](#watch-a-ticket-or-run)).
- **Run or write a workflow**: "Run the release workflow", "Write a workflow that posts merged tickets to Discord." A
  workflow it writes is a proposal you accept or discard (see [Workflows](workflows.md#share-and-copy)).
- **Put a screenshot on a ticket**: paste it into the chat and say "Make a ticket for this bug, with the screenshot."
  The image goes on the ticket as an attachment, and the agent that works on it opens it (see
  [Screenshots and logs](../using-pdt/tickets-and-the-board.md#screenshots-and-logs)). Attaching asks no more than the
  change it comes with.
- **Report a bug in PDT itself**.

It can also read and search your projects' code, use git, and search the web, as its agent normally would. With a
plugin on (Jira, Discord…), it can use that service too: see [Plugins and connections](plugins.md).

The PM runs as the **PM** persona: change its agent, model or instructions in **Personas** (see
[Personas and approvals](personas-and-approvals.md)).

## Conversations

Keep one conversation per topic: a release, a feature, a clean-up. Each has its own history, and several can be open at
once.

- **New conversation** starts one. Its first message names it; **Rename** changes the name.
- The list on the left has the open conversations, the most recently active first. One with a reply you haven't read
  is in bold; one that's working shows a spinner.
- **Close…** ends a conversation: the PM first writes a short journal entry (what was done, what's still open), then
  the conversation becomes read-only. The newest ended ones are listed below the open ones, with **All ended
  conversations** for the rest.

A conversation doesn't end on its own. When it gets long, the agent condenses its own history and PDT asks it to write
a journal checkpoint first, so nothing important is lost.

## When it asks first

Before the PM creates, edits, stops or links a ticket, messages an agent, or runs or saves a workflow, it asks you. An
approval card shows in the conversation (and under **Approvals** in the Inbox) with exactly what it's about to do.
**Allow** lets it go ahead; **Deny** tells it no. (With Codex or Devin as the PM's agent, the card is in the Inbox
only.)

Reading never asks, and nor do its own notes: it can look at anything without stopping you.

To let it act on a project's tickets without asking, turn on **The PM acts without asking** in **Settings**, on the
project's page, under **Pipeline and policies**. A plugin's connection can have the same setting, for that service.

Some things always ask, whatever the setting:

- moving a ticket that came from outside PDT (an issue, say) onto the board, or labelling it `agentic`;
- reporting a bug in PDT.

## Watch a ticket or run

Ask the PM to keep an eye on a ticket or a workflow run and it watches it for you: when the ticket's agent reports,
the ticket is done or closed, its work merges, CI fails, or it waits on you (or the run succeeds, fails or is
cancelled), PDT tells that conversation, and the PM tells you. It doesn't keep checking: it hears once it happens.
Several things at once arrive together, and the message waits until the PM has finished what it's doing.

Watching asks nothing: it only tells the PM's own conversation. Unless you ask for more, the watch ends once the ticket
or run is finished. A ticket or run the PM watches says so (**Watched by PM “Release”**), and the **×** beside it stops
that watch. If you close the conversation, what it hadn't heard yet comes to your Inbox instead, and the watch ends.

To watch something yourself, see [Watch a ticket](../using-pdt/tickets-and-the-board.md#watch-a-ticket).

## Talk to it from Discord

With the **Discord** plugin on, you can DM the PM from your phone or anywhere Discord runs.

1. In the [Discord Developer Portal](https://discord.com/developers/applications), open your bot's application →
   **Bot**, and under **Privileged Gateway Intents** turn on **Message Content Intent**.
2. Add a **Discord bot** connection (or edit yours), and fill in **Your Discord user ID**: in Discord, turn on
   Developer Mode (Settings → Advanced), then right-click your name → **Copy User ID**.
3. Save. The connection says **DMs with the PM: Connected** once the bot is online.

Now DM the bot, "What's in flight?" for example. Your messages go to your current Discord conversation, an ordinary PM
conversation marked **via Discord** in PDT's list; the first message starts one. While the PM works you see
"typing…", and its reply arrives as a DM (a long one in several messages, a very long one as a `.md` file). When the
answer comes from a different conversation than the last message, it starts with that conversation's name.

In the DMs, these commands manage conversations:

- `/new [topic]` starts a new conversation and makes it the current one.
- `/list` shows the open conversations.
- `/switch` talks to another one (it suggests them as you type).
- `/close` closes the current one: the PM writes its journal entry, and your next message starts a new conversation.

When the PM asks before doing something, the DM shows what it wants to do with **Approve** and **Decline** buttons.
The same request is in PDT too: whichever you answer first counts, and the other then says so. Questions the PM asks
show their answers as buttons; you can also type an answer.

Only your Discord account can talk to the PM or press its buttons: other people's DMs are ignored, and so are messages
in server channels. Turn on **Also send Inbox items as DMs** on the connection to get the Inbox's needs-you items
(a stalled step, a question) as DMs too.

The bot reconnects by itself when Discord's connection drops or PDT restarts. If it can't (a wrong token, Message
Content Intent off), the connection says why.

## Notes and the PM's memory

Open **Notes** in the rail (**Ctrl+]** then **o**). **Mine | PM's memory** switches between two kinds:

- **Mine**: your own notes and ideas. The PM can find and read them when it searches, but never changes or deletes
  them. **Make a ticket** on a note turns it into a draft ticket.
- **PM's memory**: what the PM saves to remember: decisions, how a project works, what you prefer. You can read, edit
  and delete them.

**Pin** a note in the PM's memory and it's in every PM conversation from the start. Pinning one of your own only lists
it first.

A note is host-wide or belongs to one project. Notes save as you type.

## What it knows when it starts

Each new conversation starts with a briefing: your projects and their tickets by stage, the tickets in progress, the
Inbox, what happened since the last conversation started, the last few journal entries, the other open conversations,
and the pinned notes. So you can ask "what's new?" without explaining anything.

A project with an enabled release workflow (such as Cut a release) is listed with it, and the PM starts that workflow
when you ask for a release. For a project without one, the PM has no release instructions: tell it how you release.

## The daily update

A conversation you keep using doesn't go stale. About once a day, when you've written to it since, PDT brings it up to
date: what happened to each ticket (moved, merged, CI failed…), what waits in the Inbox, and what the other
conversations noted as still open. In the conversation it shows as one quiet line, "PDT brought this conversation up to
date", which opens to the full text.
