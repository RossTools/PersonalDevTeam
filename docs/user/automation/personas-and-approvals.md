# Personas and approvals

A **persona** is an agent set up for one kind of work: which agent and model it runs, what it may do without asking,
which of PDT's tools it gets, and its instructions. Workflow steps run personas: in the
[Pipeline](workflows.md#the-pipeline), the **Coder** builds a ticket, the **Reviewer** reviews it and **QA** tests it.

**Approvals** are the project's rules for what merges without you: who may merge, whether a person has to approve
first, and which tickets may go all the way on their own.

## The built-in personas

Open **Personas** in the rail.

| Persona | Does |
| --- | --- |
| **Refiner** | Turns a draft ticket into one that is ready to build. |
| **Coder** | Builds the ticket on its branch, runs the checks and commits. |
| **Reviewer** | Reviews the change against the ticket, with line comments. |
| **QA** | Runs the project's tests and tries the change. |
| **Triage** | Reads issues from outside PDT and drafts tickets for them, locked down. |
| **PR Reviewer** | Reviews pull requests from outside PDT, locked down. |
| **PM** | The [project manager](the-pm.md) you talk to. |

**Triage** and **PR Reviewer** read text other people wrote, so they run locked down: they can read but not change
anything, and have no shell, web or secrets. See [Outside contributions](outside-contributions.md).

You can change the built-ins or archive them, but not delete them. When a newer PDT improves a built-in persona, it
takes the new version only for what you left as it was; what you changed stays yours. **Load PDT's template** on its
page shows PDT's current version, to compare and take it if you want.

## Change a persona

A persona's page has:

- **Agent**: Claude Code, Codex, Devin or another agent you've added.
- **Model**: leave it blank for the agent's own default.
- **Permission mode**: what its sessions may do without asking (**Ask**, **Accept edits**, **Plan**, **Bypass
  (dangerous)**). **Project or host default** leaves it to the project's setting, else your default for tickets in
  **Settings › This host › Agents**.
- **Tools**: PDT's tools its sessions get (reading the ticket, handing in an outcome, asking you, commenting…). The
  first five are the default set.
- **Instructions**: Markdown added to its sessions' instructions, on top of PDT's own and the project's.

Click **Save a new version**, with an optional **Note** saying what changed. Every save is a version: **Versions**
lists them, newest first; tick two to see what changed between them. Steps that already started keep the version they
started with.

**New persona** (at the top of the list) makes one of your own: a **Name**, a **Role** (free text: refiner, coder,
reviewer…), a **Description**, and its setup. Use it in a workflow's agent step, or in the Pipeline's settings.

### Which permission mode wins

For a workflow step's session, PDT takes the first of these that is set:

1. the project's **Agent permission mode for workflow steps** (**Settings**, the project's page, **Agents**);
2. the persona's **Permission mode**;
3. your default for tickets in **Settings › This host › Agents**.

## How a workflow runs an agent

- **Its session.** The Coder works in the ticket's own agent session (the one **Start** opened), so it keeps what it
  learned between passes. The Reviewer and QA each get a session of their own, so a review is never mixed into the
  Coder's conversation. Each opens as a tab in the ticket's workspace, and the persona's name is in the tab's title
  (`Reviewer · ABC-12`).
- **Its task.** Its first message is the step's prompt and the ticket. On a later pass (the Reviewer sent the work
  back, say) it gets only why it's back: the other agent's summary and comments, or your note.
- **Its outcome.** The agent ends its work by handing in one of the step's outcomes with a summary. A Coder can't hand
  in its work while it has uncommitted changes or commits it hasn't pushed: PDT tells it what's left.
- **Questions.** An agent that needs you asks in the Inbox under **Questions**; the step waits for your answer.
- **Stalls.** If an agent ends its turn without an outcome or a question, PDT reminds it once. If it stops again, the
  step shows under **Stalled** in the Inbox: **Nudge** reminds it again (with your own words if you like), **Take over**
  finishes the step for it with the outcome you pick.
- **Time limits.** A step that runs too long (four hours, or ten turns without an outcome) stops and shows in the Inbox
  too. **Resume** starts it again in the same session, with an optional note; **Stop** ends the run.

You can do a stage yourself: in the Pipeline's settings, set **Who reviews** (say) to `user`. The step then shows in
the Inbox as "Your turn", and you pick its outcome when you're done.

### Loop limits

Work can go round: the Reviewer sends it back to the Coder, the Coder fixes it, the Reviewer sends it back again. To
stop that going on forever, each agent step has a **loop limit** (the Pipeline's **Passes before asking**, 3 by
default). When a step is about to go round once more than that, the Inbox shows it under **Loop limit**, with each
pass's outcome and summary:

- **Once more** lets it go round one more time;
- **Take over** picks the outcome for it;
- **Stop** ends the run.

### How many agents run at once

**Agents running at once** (in **Settings › This host › Agents**) caps how many agents run a turn together. When it's
reached, workflow steps and auto-start wait for a free slot, and the ticket's card says it's queued. What you send an
agent yourself never waits.

## Approvals and merging

Each project has its rules in **Settings**, on the project's page, under **Pipeline and policies**. Merges keep to
them whatever a workflow asks.

- **Who may merge**: **Agents, workflows and people**; **Only a person** (your **Merge…** in PDT); or **Nobody in PDT
  (merged elsewhere)**.
- **A person approves before an automated merge**: a workflow's or an agent's merge waits until you've approved the
  branch as it is now: **Approve** in the [Changes view](../using-pdt/review-and-merge.md), or **Accept** in the Inbox.
  A new commit after your approval needs approving again.
- **Labels that merge without approval**: tickets with one of these labels don't need it.
- **An automated merge waits for green CI**: a workflow's or an agent's merge waits for CI to pass on the pull
  request's latest commit, and goes back to the Coder when it fails (on by default).

**Merge through pull requests** (under **Review** on the same page) makes every merge go through the pull request, so
nothing is merged on your machine.

### The `agentic` label

`agentic` on a ticket says it may go all the way without you. In the Pipeline it skips **Accept**: once the Reviewer
and QA pass it, it merges. Every other ticket waits for you to accept it.

If the project also has **A person approves before an automated merge** on, add `agentic` to **Labels that merge
without approval** too. Otherwise the merge stops and asks you (**Merge blocked**), and **Merge again** counts as your
approval.

PDT only counts the labels a ticket had when it started. A ticket that came from outside PDT (an issue, a pull request)
gets `agentic` only from you: an agent or the PM can't add it.

## When PDT asks you

With the Pipeline and these rules, PDT asks you:

- to **accept** each ticket that isn't `agentic`, before it merges;
- to **approve** before an automated merge, when the project says so;
- when a merge is blocked for a reason no agent can fix;
- when a step goes round more than its loop limit, stalls, or runs out of time;
- when an agent asks a question, or wants permission for something its permission mode doesn't allow;
- before the PM changes tickets or messages agents, unless you let it act without asking (see
  [The PM](the-pm.md#when-it-asks-first));
- before a ticket from outside PDT goes on the board.

Everything waits for you in the [Inbox](../using-pdt/the-inbox.md).
