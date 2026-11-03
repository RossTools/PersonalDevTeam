# Personas and approvals

A **persona** is an agent set up for one kind of work: which agent and model it runs, what it may do without asking,
which of PDT's tools it gets, and its instructions. Workflow steps run personas: in the
[Pipeline](workflows.md#the-pipeline), the **Coder** builds a ticket, the **Reviewer** reviews it and **QA** tests it.

**Approvals** are each project's rules for what merges without you.

## The built-in personas

Open **Personas** in the rail.

| Persona | Does |
| --- | --- |
| **Refiner** | Turns a draft ticket into one that is ready to build. |
| **Coder** | Builds the ticket on its branch, runs the checks and commits. |
| **Reviewer** | Reviews the change against the ticket, with line comments. |
| **QA** | Runs the project's tests and tries the change. |
| **Triage** | Drafts tickets from issues opened outside PDT. |
| **PR Reviewer** | Reviews pull requests opened outside PDT. |
| **PM** | The [project manager](the-pm.md) you talk to. |

**Triage** and **PR Reviewer** read text other people wrote, so they run locked down: they can read but not change
anything, and have no shell, web or secrets. See [Outside contributions](outside-contributions.md).

You can change or archive the built-ins, but not delete them. When PDT updates a built-in, it changes only the parts
you left as they were. **Load PDT's template** on a persona's page shows PDT's current version, to compare or take.

## Change a persona

Open it in **Personas** and set:

- **Agent**: Claude Code, Codex, Devin or another agent you've added.
- **Model**: blank for the agent's default.
- **Permission mode**: what its sessions may do without asking (**Ask**, **Accept edits**, **Plan**, **Bypass
  (dangerous)**), or **Project or host default** (see [Which permission mode wins](#which-permission-mode-wins)).
- **Tools**: the PDT tools its sessions get (reading the ticket, handing in an outcome, asking you, commenting…). A
  new persona starts with PDT's default set; a persona with none ticked gets no PDT tools.
- **Instructions**: Markdown added to its sessions' instructions, on top of PDT's own and the project's.

Click **Save a new version**, with an optional **Note**. **Versions** lists every save, newest first; tick two to
compare them. A step that already started keeps the instructions it started with, but its PDT tools follow
the newest version: an agent that supports it sees tools added or taken away without restarting.

**New persona** (at the top of the list) makes your own: a **Name**, a **Role** (free text: refiner, coder,
reviewer…), a **Description** and the same setup. Use it in an agent step or the Pipeline's settings.

### Which permission mode wins

For a workflow step's session, PDT takes the first of these that is set:

1. the project's **Agent permission mode for workflow steps** (**Settings**, the project's page, **Agents**);
2. the persona's **Permission mode**;
3. your default for tickets in **Settings › This host › Agents**.

## How a workflow runs an agent

- **Session**: the Coder works in the ticket's own session (the one **Start** opened), so it keeps what it learned
  between passes. The Reviewer and QA each get their own session. Each is a tab in the ticket's workspace named after
  the persona (`Reviewer · ABC-12`).
- **Task**: the first message is the step's prompt and the ticket. On a later pass it gets only why the work is back:
  the other agent's summary and comments, or your note.
- **Outcome**: the agent finishes by handing in one of the step's outcomes with a summary. A Coder can't hand in work
  with uncommitted changes or unpushed commits; PDT tells it what's left.
- **Questions**: an agent that needs you asks in the Inbox under **Questions**; the step waits for your answer.
- **Stalls**: an agent that ends its turn without an outcome or a question, or whose session stops, is stuck. See
  [When an agent is stuck](#when-an-agent-is-stuck).
- **Time limits**: a step that runs too long (four hours, or ten turns without an outcome) stops. It is restarted
  once in a fresh session, then shows in the Inbox: **Resume** restarts it in the same session, with an optional
  note; **Stop** ends the run.

To do a stage yourself, set its **Who …** setting to `user` in the [Pipeline's settings](workflows.md#pipeline-settings).

### Loop limits

Work can go round: the Reviewer sends it back, the Coder fixes it, the Reviewer sends it back again. Each agent step
has a **loop limit** (the Pipeline's **Passes before asking**, 3 by default). When a step would go round once more,
the Inbox shows it under **Loop limit** with each pass's outcome and summary: **Once more** allows one more pass,
**Take over** picks the outcome, **Stop** ends the run.

### When an agent is stuck

The built-in workflow **When an agent is stuck** decides what happens to a step that stalls, reaches its loop limit
or runs out of time. By default:

1. A stalled agent is nudged: "You stopped without an outcome. Finish with submit_outcome, or ask_user if you're
   blocked."
2. If it stops again, it is nudged again, with the step's goal and the end of its last message.
3. Then the step starts again in a fresh session, with its task, the earlier session's last message and the state of
   the branch. The earlier session is stopped.
4. If that stalls too, the step shows under **Stalled** in the Inbox: **Nudge** reminds it again (in your words if
   you like), **Take over** picks the outcome for it.

At a loop limit the Inbox asks you. A step that runs out of time is restarted once in a fresh session, then the Inbox
asks you.

Change it in **Settings › This host › Agents**, under **When an agent is stuck**: how many nudges, the nudge messages
(**Reset to default text** brings one back), whether to restart in a fresh session before asking you, and what
happens at a loop limit (**Ask me** or **Stop the step**) and at a time limit. **Customize as a workflow →** opens the
workflow for anything more; once you change it there beyond these settings, Settings names your workflow, with
**Open** and **Reset to default**.

Turned off, or if the workflow fails or ends without doing anything with the step, PDT handles the step itself: a
stalled agent is nudged once, then the step shows in the Inbox, and loop limits and time limits go straight to the
Inbox.

### How many agents run at once

**Agents running at once** (in **Settings › This host › Agents**) caps how many agents take a turn together. When it's
reached, workflow steps and auto-start wait for a free slot, and the ticket's card says it's queued. Messages you send
an agent yourself never wait.

## Approvals and merging

Set each project's rules in **Settings**, on the project's page, under **Pipeline and policies**. Every merge keeps to
them, whatever a workflow asks.

- **Who may merge**: **Agents, workflows and people**; **Only a person** (your **Merge…** in PDT); or **Nobody in PDT
  (merged elsewhere)**.
- **A person approves before an automated merge**: a workflow's or agent's merge waits until you've approved the
  branch as it is now (**Approve** in the [Changes view](../using-pdt/review-and-merge.md), or **Accept** in the
  Inbox). A new commit needs approving again.
- **Labels that merge without approval**: tickets with one of these don't need it.
- **An automated merge waits for green CI**: a workflow's or agent's merge waits for CI to pass on the latest commit,
  and goes back to the Coder when it fails (on by default).
- **When to resolve conflicts**: **At merge time** (the default) or **Immediately**. See
  [Conflicts](../using-pdt/review-and-merge.md#conflicts).

**Merge through pull requests** (under **Review** on the same page) makes every merge go through the pull request, so
nothing merges on your machine.

### The `agentic` label

`agentic` on a ticket means it may go all the way without you: in the Pipeline it skips **Accept** and merges once the
Reviewer and QA pass it.

If the project has **A person approves before an automated merge** on, add `agentic` to **Labels that merge without
approval** too. Otherwise the merge stops at **Merge blocked**, and **Merge again** counts as your approval.

Only the labels a ticket had when it started count. A ticket that came from outside PDT (an issue, a pull request)
gets `agentic` only from you, never from an agent or the PM.

## When PDT asks you

With the Pipeline and these rules, PDT asks you, in the [Inbox](../using-pdt/the-inbox.md):

- to **accept** each ticket that isn't `agentic`, before it merges;
- to **approve** before an automated merge, when the project says so;
- when a merge is blocked for a reason no agent can fix;
- when a step goes past its loop limit, stalls, or runs out of time;
- when an agent asks a question, or wants permission its permission mode doesn't give;
- before an agent uses a PDT tool that acts for you (creating or editing tickets, messaging another agent, running a
  workflow, filing an issue…) in a session that never asks itself: Codex, Devin, or Claude in **Bypass**. In a
  conversation PDT asks with a card in the conversation (and in the Inbox); from a terminal, in the Inbox. Either way
  the agent carries on once you answer.
  Where you let the PM act without asking, it goes ahead;
- before an agent outside a workflow step posts where others can read it (comments on a pull request or issue, or
  opening a pull request). Workflow steps that do this, such as the Coder opening its pull request, don't ask;
- before the PM changes tickets or messages agents, unless you let it act without asking (see
  [The PM](the-pm.md#when-it-asks-first));
- before a ticket from outside PDT goes on the board.
