# Workflows

A workflow lets PDT act for you when something happens: CI fails on a ticket's pull request, a branch merges, a new
issue is opened. Each one is a **trigger** (what happened) followed by **steps** (what to do), with branches where it
decides something or asks you.

Open **Workflows** in the rail. The list groups your workflows by where they apply: **This host** (every project),
**Personal**, and each project. A dot shows whether each is on.

## The built-in workflows

These come with PDT. You can turn them off, edit them or copy them, but not delete them.

| Workflow | What it does | Starts |
| --- | --- | --- |
| **Pick up tickets** | Auto-start: starts each project's next ticket, with the project's auto-start settings. See [Pick up tickets](#pick-up-tickets). | On |
| **PR notices** | Tells a ticket's agent when its pull request's CI fails, it conflicts with the base branch, it falls out of the merge queue, or it merges. A conflict before the merge waits for merge time. When the pull request was set to merge, PDT brings it up to date with its base, the agent fixes what's left, and PDT merges it again. | On |
| **Finish merged tickets** | Moves a ticket to **Done** and cleans up once its branch merges, when you asked for that at **Merge…**. | On |
| **Commits pushed after a merge** | Asks you whether to open a follow-up ticket when an agent pushes to a branch that already merged. | On |
| **Ticket comments to the agent** | Sends your comments and the PM's on a ticket to the agent working on it, with where to find the files they attach. Other agents' comments stay on the ticket. | On |
| **PR comments to the agent** | Sends other people's comments and reviews on the pull request to the agent. | Off |
| **Base moved while a PR waits** | Tells the agent when the base branch moves while its pull request waits (**PR notices** covers this too). | Off |
| **Ticket proposals** | Holds a ticket an agent files while it works (not the PM's, and not parts from **Make into parts**) in **Draft**, off the board, and puts it in your Inbox to accept, mark as a duplicate or reject. See [Ticket proposals](../using-pdt/the-inbox.md#to-act-on). Turn it off and agents' tickets go straight to the board. | On |
| **Close finished parents** | Closes a parent when its last part closes (to **Won't do** if every part was **Won't do**), with a comment and a notification, unless it has an attempt or a running agent session of its own. Moving a part out of **Done** or **Won't do** moves its closed parent back to **In progress**. See [Parts and epics](../using-pdt/tickets-and-the-board.md#parts-and-epics). | On |
| **When an agent is stuck** | Nudges an agent step that stops without an outcome, restarts it in a fresh session, then asks you in the Inbox; asks you at a loop limit; restarts a step that ran out of time once, then asks you. Set it in **Settings › This host › Agents**. See [When an agent is stuck](personas-and-approvals.md#when-an-agent-is-stuck). | On |
| **Notification rules** | How far each kind of notification reaches you (off, a dot, the Inbox, a toast, a desktop notification), and quiet hours. You can turn on escalating what waits unanswered and posting questions, approvals and failed runs to a chat channel. Set it in **Settings › Notifications**. See [Notifications](../using-pdt/the-inbox.md#notifications). Turned off, every kind shows at its default level. | On |
| **Install updates** | With updates set to install automatically when idle, waits until no agent is working, then installs the update and checks PDT once it restarts. You can set a time window, install anyway after a while, and be told what changed. A failed update or check goes to your Inbox. Set it in **Settings › This host › Updates**. See [Installing updates by themselves](../your-machine/updates.md#installing-updates-by-themselves). Turned off, updates install only when you say. | On |
| **Clean up storage** | Every hour, cleans finished tickets' worktrees as each project's **Worktrees** settings say. When a disk PDT uses is almost full, pauses pickup and cleans every finished ticket's build output at once; once there is space again, pickup resumes. Set the thresholds and what happens in **Settings › This host › Storage**. See [Disk space](../your-machine/your-data.md#disk-space). Turned off, finished worktrees are cleaned only when you say, and nothing pauses. | On |
| **PM daily checkpoint** | Brings the PM's open conversations up to date once a day and has each write a journal checkpoint. You can set a time, the days and a chat channel to post the update to in **Settings › This host › PM**. See [Change when it's brought up to date](the-pm.md#change-when-its-brought-up-to-date). | On |

Plugins add their own while they're on: Jira, say, adds **Import assigned Jira issues** and **Done moves the Jira
issue**. See [Plugins and connections](plugins.md).

To tell the agent about CI failures in your own words, add the **Tell the agent when CI fails** template (keep
`{{trigger.notice.text}}` in its **Message** to pass on which jobs failed) and turn **PR notices** off for that
project. **Ask before finishing a merged ticket** puts a gate in the Inbox instead of finishing merged tickets
straight away: **finish** or **keep**.

## Turn a workflow on or off

1. Open the workflow and click the **Enabled** switch at the top.
2. PDT shows where it runs and what it can do (comment on tickets, message agents, merge…). Click **Dry run first** to
   watch it handle a real event without changing anything, or **Turn on**.

PDT then runs it whenever its trigger happens. Turning it off stops new runs; runs already going finish.

## Add a workflow from a template

1. In **Workflows**, click **New workflow**.
2. Pick a template: search, or browse **CI & pull requests**, **Tickets**, **Issues**, **Notifications**,
   **Releases** and **Documents**. **Start from scratch** gives you an empty one; your own templates are under **Yours**, and copies of
   the built-ins under **Built-in**.
3. Fill in its settings, if it has any. **When** and **What it does** preview its triggers and steps.
4. Give it a **Name** and a **Project**: one project, **This host (every project)** or **None (personal)**.
5. Click **Create workflow**, then turn it on. A new workflow starts off.

A template whose plugin is off is faded; its preview links to the plugin so you can turn it on.

To have an agent write one for you, ask [the PM](the-pm.md) ("Write a workflow that posts merged tickets to
Discord"). It saves a proposal, which waits in the Inbox under **Workflow proposals** and in a banner on the workflow
(**Proposal waiting** in the list): **Review** shows the whole change in **Versions**, then **Accept** or **Discard**
it, on a phone too. Accepting replaces changes you haven't saved, so PDT asks first.

## The Pipeline

The **Pipeline** template (under **Tickets**) is the one most projects want. Add it for a project, turn it on, and
every ticket you start there goes through these stages:

1. **Code**: the **Coder** builds the ticket on its branch, runs the checks, commits, pushes, and opens a pull request
   when the project uses them.
2. **Review**: the **Reviewer** reads the change against the ticket. **Changes requested** sends it back to Code with
   the comments; **Approved** moves it on.
3. **QA**: the **QA** agent runs the tests and tries each point of the ticket's "Done when". **Fail** sends it back to
   Code; **Pass** moves it on.
4. **Accept**: the ticket waits for you under **Ready to accept** in the [Inbox](../using-pdt/the-inbox.md). Accept it
   and it merges; reject it with a note and the note goes to the Coder. Tickets labelled `agentic` skip this stage
   (see [The `agentic` label](personas-and-approvals.md#the-agentic-label)).
5. **Merge**: PDT brings the branch up to date with its base (only real conflicts go to the Coder; see
   [Conflicts](../using-pdt/review-and-merge.md#conflicts)), merges it (on GitHub, by turning on the pull request's
   auto-merge once CI passes) and moves the ticket to **Done**. Failing CI goes back to the Coder, then the Reviewer.
   A merge PDT can't fix (a draft pull request, the project's policy) asks you.

A ticket that needed no change (the Coder says "No changes", and its branch has no commits) has nothing to merge.
Once QA passes it is accepted as above: tickets that skip Accept go straight to **Done**; the rest wait at Accept,
where **Done** moves them there. Either way its sessions stop and its worktree and branch are removed.

On the board, **In progress** splits into these stages, and each card says who has the ticket ("Reviewer · working",
"Your turn").

The Pipeline also handles **Refine** (in a **Draft** or **Ready** ticket's menu): the **Refiner** rewrites the ticket
so it's ready to build, sets its areas, and moves it to **To do**, or asks you when something needs clearing up.

How each agent works inside a stage is in [Personas and approvals](personas-and-approvals.md#how-a-workflow-runs-an-agent).

### Pipeline settings

You set these when you create it; afterwards each lives in its step (click the step in the graph).

- **Refiner persona**, **Coder persona**, **Reviewer persona**, **QA persona**: who does each stage. See
  [Personas and approvals](personas-and-approvals.md).
- **Who refines**, **Who codes**, **Who reviews**, **Who does QA**: `agent`, or `user` to do it yourself. Your stage
  shows in the Inbox as "Your turn", with its outcomes to pick from when you're done.
- **A refined ticket goes to**: `todo` (the board) or `ready` (the backlog).
- **Who accepts**: `user` (you, in the Inbox) or `auto` (it merges once QA passes).
- **Labels accepted automatically**: tickets with one of these skip Accept (default `agentic`).
- **Who merges**: `workflow` (the Pipeline merges after Accept) or `person` (it waits for someone to merge, in PDT or
  on GitHub, and asks you after the **Merge wait**).
- **Passes before asking**: how many times work may go back to Code before PDT asks whether to go on (default 3; see
  [Loop limits](personas-and-approvals.md#loop-limits)).
- **Merge wait**: how long a merge waits for someone, to merge it or to clear what blocks it, before the Inbox asks
  again (default 3 days).
- **CI wait**: how long a merge waits for CI on the pull request to finish before the Inbox asks (default 6 hours).
- **Code time limit**: how long the Code step may run before the Pipeline stops there (default 8 hours).

Pipelines you added earlier keep these defaults.

What any workflow may merge is the project's policy, set apart from workflows: see
[Approvals and merging](personas-and-approvals.md#approvals-and-merging).

## Pick up tickets

**Pick up tickets** is auto-start. It runs when a ticket can start (it has gone the quiet period unchanged, or its
last blocker closed), when there's room for another (a ticket in flight merged or was abandoned, the project's cap
rose, an agent slot freed up), and every minute. It has three steps:

1. **Find tickets to start** lists the project's tickets that could start, in order: the stages, labels, priority and
   areas to take, whether to leave out blocked, conflicting and parent tickets, and the order (`priority`, `rank` or
   `age`).
2. **Choose tickets to start** keeps to the caps (per project, and optionally per area or label), the quiet period,
   and passes over a ticket that failed to start until it changes (or you click **Try again** on it). It also keeps tickets apart from work in progress,
   by area (**Overlap**):
   - `avoid` (the default when the project has hot areas): no ticket in a hot area while another ticket in that area
     is in progress.
   - `serialize`: at most **Per area** tickets in progress in each area, hot or not.
   - `off`: no rule of its own (conflicting tickets are still left out in the first step).

   **Hot areas** and **Untagged alongside hot** default to the project's settings. The run shows why it passed over
   each ticket, and a ticket waiting on overlapping work says why on its card.
3. **Start the ticket** starts the first one still free, with the agent, permission mode and view you give it (your
   defaults otherwise), handed to the project's workflow (the Pipeline) or to an agent session of its own. It starts a
   ticket that shares an area with work in progress when the second step allowed it, and refuses only a conflict.
   A ticket it can't start yet (a conflict, a blocker) waits for the next run.

A setting left empty is the project's auto-start setting (**Settings**, on the project's page), so with the project's
auto-start off it starts nothing. Set one in the step and it applies whatever the project says. **Dry run** shows what
it would start without starting it.

### Only `agentic` tickets

1. Open **Pick up tickets**, then **Duplicate** in its **⋯**, and set the copy's **Project** to the project.
2. In **Find tickets to start**, set **Labels** to `agentic`.
3. Turn the copy on, and turn the built-in off (or leave the project's other tickets to it by setting **Exclude
   labels** to `agentic` in the built-in, so the two don't share them).

### Ready tickets only at night

1. **Duplicate** **Pick up tickets** for the project.
2. Delete its triggers but **Every minute**, and change that one to a cron line: `*/10 0-6 * * *` (every ten minutes
   from midnight to 7 a.m.).
3. In **Find tickets to start**, set **Stages** to `ready`. In **Choose tickets to start**, set **Cap** (say `3`) for a
   different cap at night.
4. Turn it on. The built-in still takes **To do** tickets during the day.

## Edit a workflow

A workflow page edits one draft three ways:

- **Graph**: steps as boxes joined by arrows. **Add node** adds a step after the selected one; drag from a step's
  port (one of its exits) to another step to join them; **Delete** removes one; **Arrange** tidies the layout.
- **Outline**: an indented list, each trigger with its steps and a branch per exit; use a row's **⋯** to add, move or
  remove a step. The default on a phone.
- **Text**: the whole workflow as JSON or TOML.

Click a step to change its settings, its **Title**, and **When it fails**: **Retries**, **Time limit**, **Visits per
run**, and **On error, go to** (a step that takes over instead of failing the run). Problems show on the step as you
edit.

Changes collect in the **Unsaved changes** bar: **Save** (with an optional note) makes a new version, **Discard**
drops them. **Versions** lists every saved version with who saved it and what changed; **Restore this version** brings
one back.

### Try it before turning it on

- **Dry run** runs the draft against an event without sending, moving or changing anything. Pick a recent event of
  the trigger's kind, write one, or give manual inputs. It shows the path the run would take, why a trigger didn't
  match, and what each step would have done.
- **Run now** runs the saved version for real, for a workflow with a **Run now** trigger.

### Share and copy

- **Save as template…** (in the workflow's **⋯**) adds it to **New workflow** under **Yours**.
- **Duplicate** copies it; **Delete…** removes it with its runs.
- **Export…** and **Import…** (at the top of the list) move workflows between PDT hosts as a file.
- Workflows kept in a project's repository, under `.pdt/workflows/`, are added off and marked **In the repo**. Edit
  them in the repository, not in PDT. Once you turn one on, a change to its file doesn't run until you accept it: it
  waits as a proposal from the repo under **Workflow proposals** in the Inbox. A change you discard stays discarded
  until the file changes again.

## Read a run

A workflow's **Runs** tab lists its runs, newest first. Open one to see the graph of the version it ran with the path
it took, and each step's input, output or error. **Open session** on an agent step shows what the agent did (step
sessions aren't in the sidebar's **Agents** list; open them here or from the ticket).

- A run is **Waiting** while a step waits for an agent, for you, for CI or for a time. **Cancel** stops it.
- **Retry from the failed step** runs a failed run again from where it stopped, with the same ticket and data.
- **Resume from** restarts an agent step that stopped or timed out, in the same session, with an optional note.

Failed runs also show in the Inbox's **Updates**. To hear when one run finishes, fails or waits on you, click the eye
(**Watch**) beside its status, or ask the PM to watch it.

## Reference

### Triggers

Every kind of event in PDT is a trigger: a ticket moved, started, commented on or merged; CI failed or passed; a pull
request conflicts, left the merge queue or merged; a new GitHub issue or outside pull request; a failed GitHub Actions
run; a new release, a PDT update, low disk space, a failing connection or a todo coming due. These ones react to changes to tickets:

- **Ticket created**: says who made it (you, the PM, an agent, a workflow, or outside input). An agent's ticket names
  its persona, the ticket it was working on and the reason it gave. Parts made with **Make into parts** are marked as
  parts.
- **Ticket unblocked**: the last open ticket blocking it reached Done or Won't do, or its link was removed.
- **Ticket labels changed**: the labels added and removed.
- **Ticket priority changed**: the old priority and the new one.
- **Ticket parent changed**: it was made a part of another ticket, moved to another parent, or detached.

These ones react to [documents](../using-pdt/documents.md):

- **Document changed**: its content changed, in PDT or at its provider (Google, say).
- **New comment on a document**: someone commented on it or replied in a comment's thread. It carries who wrote it,
  the text, and the text it's on. Your own comments count; agents' and workflows' don't, so an agent answering
  comments never answers itself. What other people wrote reaches agents marked as outside input.

Each watches the document you name, or every document linked to the workflow's project (for **This host**, every
document linked to a project). PDT looks every minute; the first look only notes where a document is, so turning a
workflow on, or on again, never replays old changes or comments. A document linked to several projects starts each workflow once.

These ones react to the Inbox, agent sessions and workflow runs:

- **Inbox item opened**: something new waits on you (a question, a gate, an approval, a stalled step, a proposal). It
  carries the item's kind, title, ticket, session, persona and project. A session waiting on you is **Session needs
  you** instead.
- **Inbox item answered**: the choice or answer, who gave it (you, or an agent through the API) and how long the item
  waited.
- **Session needs you**: an agent session waits on a permission prompt or a question.
- **Session turn ended**: an agent session finished a turn. This fires on every turn of every session, so narrow it
  with a condition or a project.
- **Session exited**: an agent session's process ended, with its exit code and whether PDT (or you) stopped it.
- **Workflow run failed**: the workflow, the step that failed and its error.
- **Workflow run finished**: a run ended without failing (succeeded, quiet or cancelled), with the step it ended at
  and how long it took.
- **Agent stalled**, **Agent step at its loop limit**, **Agent step timed out**: a workflow's agent step is stuck. Each
  carries why, the step, its persona, session and ticket, the step's goal, the end of the agent's last message, and
  how many nudges and fresh sessions it already had.

Each session event says whether a workflow's agent step runs the session or it's your own, and ends with the last
few lines the agent wrote. A workflow never reacts to its own runs, their Inbox items or their agent steps' sessions,
so one that reports failed runs never reports itself. To react to them too, turn on **Include own** on the trigger.
Runs that start each other through run events stop after three in a row.

These ones react to releases, PDT itself and what it looks after:

- **Release published**: a release or pre-release appeared on a project's repository (GitHub or GitLab). It carries
  the tag, name, link, notes (and `summary`, their start), its files and whether it's a pre-release. PDT looks every minute at the projects a
  turned-on trigger covers; the first look only notes the releases already there.
- **PDT update available**: a newer version of PDT is out on this computer's update channel, once per version.
- **PDT updated**: PDT started on a version it just updated to, and says whether it updated by itself (when idle)
  or because you asked.
- **PDT update failed**: an update failed, or the new version didn't start and PDT went back to the one before. It
  carries both versions and what went wrong.
- **Disk space low**: a disk PDT uses fell below the low-space level in Settings, or below the almost-full one. It
  carries the free space, the level, and how much cleaning finished tickets' build output would free. It fires
  when the level drops, not on every check.
- **Disk space recovered**: a disk that was low has enough free space again.
- **Connection failing**: a connection, or your sign-in to a forge such as GitHub, stopped working (a revoked token,
  the service down). It fires when it starts failing, not on every failed use.
- **Todo due**: one of your todos comes due, with its title, notes and when it was due (or that it's overdue, if PDT
  was off then).
- **Scryer download finished** (with the Scryer plugin): Scryer finished a download and added it to your library,
  with the title, its type, quality and folder. PDT looks every minute while a trigger is on.

Some triggers have settings of their own:

- **On a schedule**: every so often, or at set times (a cron line, in your time zone).
- **Webhook**: when another service calls PDT's address for a webhook connection.
- **Run now**: only when you click **Run now** (or the PM runs it for you). It can ask for inputs first.

Any trigger can be narrowed to some projects, or to when a condition holds (the ticket has the label `ui`, say).

### Steps

**Add node** lists every step, grouped:

- **Act on tickets**: **Comment on the ticket**, **Move the ticket**, **Create a ticket**, **Finish the ticket** (stop
  its runs, remove its worktree), **Update with the base**, **Merge the attempt**, and **Find tickets to start**, **Choose tickets to start** and
  **Start the ticket** (see [Pick up tickets](#pick-up-tickets)).
- **Talk to agents**: **Tell the agent** messages the ticket's agent once it's between turns; an **Agent step** has a
  persona do a piece of work. For a stuck agent step: **Nudge the agent**, **Restart in a fresh session**, **Ask me in
  the Inbox** and **Stop the step**.
- **Decide**: **If** (yes or no), **Switch** (one exit per value), **Wait** (for a time, or an event with a time
  limit), **Ask the user** (a gate).
- **Look after PDT**: **Wait until PDT is idle** (no agent working, inside a time window if you set one, or until a
  time limit), **Install the update** and **Check PDT is healthy**. **Check disk space** goes on by how full the
  fullest disk PDT uses is (enough space, low, almost full). **Clean up worktrees** frees space in finished tickets'
  worktrees: those whose project's delays have passed, or every finished one now (by stage and age), removing build
  output or the whole worktree when nothing would be lost. **Pause pickup** stops auto-start, with a reason the Board
  shows, until **Resume pickup** in the same workflow (or you) resumes it.
- **Tell people**: **Notify me**, **Put it in my Inbox**, a Discord post, a comment on a GitHub issue. **Notify me**
  can set how far it reaches you (from a dot to a desktop notification, or only the chat channel), post to a chat
  channel too, and do nothing once a given Inbox item is answered.
- **Show levels** holds how far each kind of notification reaches you, and quiet hours. It starts no runs: PDT reads it
  from the **Notification rules** workflow as it makes each notification. Add another with **projects** set to give
  those projects their own levels.
- **The PM**: **PM daily update** brings the PM's open conversations up to date; **PM journal checkpoint** asks them
  to write their journal entry.
- **Work on documents**: **Create a document** (a doc or a sheet, linked to a ticket or the project), **Add to a
  document** (Markdown at its end or under a heading), **Add rows to a sheet** (under its last row), **Set cells in a
  sheet**, **Add a chart to a sheet**, and **Comment on a document** (a reply in a comment's thread, or a new comment).
  Each names its document by title or id, and a dry run says what it would write. In a sheet, a value you type
  starting with `=` is a formula; text filled in from the event (a ticket's title, say) is always written as text.
- **Reach out**: **Run a command**, **HTTP request** (through a connection), **MCP tool**, **Run a workflow**.
- Plugins (Jira, Buildkite, GitHub…) add their own steps while they're on.

A step's settings can use what came before: `{{trigger.ticketKey}}` is the event's ticket, `{{steps.review.summary}}`
is what the step called `review` handed in; the settings panel has a picker for these. Secrets go in as
`{{secret.name}}` and are filled in only as the step runs (see [Secrets](../your-machine/secrets.md)).

A **Run a command** script can't use these directly: text from an issue or a webhook would run as part of the script.
Put each one in the step's **env** (`"TITLE": "{{trigger.title}}"`) and use `"$TITLE"` in the script (`$env:TITLE` in
PowerShell); a workflow whose script still uses them fails at that step until you move them. A program's **args**
take them as they are; put `--` before them when a value could start with `-`.

A step that can't run as set up (no project to work on, a repository that isn't `owner/name`) fails the run at once
with that message.

### Gates

**Ask the user** asks you something and waits. The question shows in the Inbox under **Ready to accept** with a
box for a note, which goes on the ticket as your comment. Each choice is an exit. A gate can give up
after a time, or close by itself when something else happens first (the ticket merged, say).

### Agent steps and outcomes

An **Agent step** runs a persona on the ticket with a prompt you write, and waits for it to hand in an **outcome**.
You name the outcomes, each with a line saying when to pick it (`approved`: the change is ready;
`changes_requested`: it needs work); each outcome is an exit. The agent hands one in with a summary and, for a
review, line comments. Sessions, stalls and time limits are in
[How a workflow runs an agent](personas-and-approvals.md#how-a-workflow-runs-an-agent).

### Run now inputs

A **Run now** trigger can declare inputs under **Inputs**. Each has an **id** (steps read it as `{{trigger.<id>}}`), a
**label**, a **type** (text, number, true/false, or a choice from a list), a **default**, whether it's **required**,
and a **help** line. **Show if** shows a field only while another has a value (`""` means empty).

**Run now** then opens a form with the defaults filled in (on a phone, in the workflow's **⋯**); without inputs it
runs at once. A **confirm** text, which can name inputs (`{{trigger.bump}}`) and have its own **show if**, is asked
before the run starts. However a run starts (the form, the PM, another workflow), inputs it doesn't declare or of the
wrong kind are refused.

A **This host** workflow whose steps work on a project asks for a **Project** in its form, starting on the one you
have open.

## Examples

### Triage GitHub issues

1. Turn on the **GitHub** plugin and connect your GitHub login (see [Plugins and connections](plugins.md)).
2. **New workflow** › **Triage new GitHub issues**.
3. Fill in **Repository** (`owner/name`) and **Project for its tickets** (the project's key, `ABC`). Leave
   **Persona** as **Triage**.
4. **Create workflow** and turn it on.

Each new issue goes to the **Triage** agent, which drafts a ticket, marks the issue a duplicate of one, or leaves it.
Drafts wait in the Inbox under **Triaged tickets**: **Approve** sends one to **To do**, **Reject** to **Won't do**.
Issues opened before you turned it on aren't triaged. Triage runs locked down: see
[Outside contributions](outside-contributions.md).

### Log merged tickets to a sheet

1. Make a spreadsheet in **Documents** (`Merged tickets`), with a header row if you like: Ticket, Title, Branch, Pull
   request, How.
2. **New workflow** › **Log each merged ticket to a sheet** (under **Documents**).
3. **Spreadsheet**: its title or id. Pick the project, **Create workflow** and turn it on.

Each ticket whose work merges adds a row under the last one: its key, title, branch, pull request and how it merged.

### Answer comments on a project's documents

1. **New workflow** › **Reply to new comments on a project's documents with an agent** (under **Documents**), in
   the project.
2. **Persona**: who answers (**Reviewer** by default). **Create workflow** and turn it on.

When someone comments on a document linked to the project (a ticket's design doc, a plan), the agent reads the
document and the comment and writes a reply, which the workflow posts in the comment's thread; the agent can also
decide it needs none. Comments made before you turned it on are left alone.

### Report failed workflow runs

For GitHub Actions runs nobody is watching (a release started by hand, a nightly, a deploy):

1. Turn on the **GitHub** plugin and connect your GitHub login.
2. **New workflow** › **Report failed workflow runs** (under **CI & pull requests**).
3. **Workflow**: its file (`release.yml`) or its name in GitHub Actions. Leave **Repository** empty for the project's
   own, or give `owner/name`.
4. Optionally narrow it with **Branch** (`main`) and **Started by** (`schedule, workflow_dispatch`). **Check every**
   sets how often PDT looks (30 minutes).
5. Pick the project, **Create workflow** and turn it on.

Each failed run puts one item in the Inbox: its title, branch, trigger, version or tag, failed jobs and a link.
**Re-run failed jobs** re-runs them on GitHub as your login; **Dismiss** leaves it. The item clears itself once a
later attempt passes; a failed re-run replaces it. Runs from before you turned it on aren't reported, and a run that
ends while PDT is stopped is reported once when it starts again.

### Cut a release

**Cut a release** (under **Releases**, needs the GitHub plugin) starts a project's release workflow (any GitHub
Actions workflow you can start by hand) and follows it to the published release. Make one per project:

1. **New workflow** › **Cut a release**, in the project.
2. **Release workflow**: its file in `.github/workflows` (`publish.yml`), and the **Branch it runs on** (`main`).
3. **Stable release inputs**: the workflow's inputs for a stable release, as JSON. The default,
   `{"bump": "{{trigger.bump}}", "version": "{{trigger.version}}"}`, passes the bump or exact version you pick; rename
   the keys to your workflow's input names. Empty values are left out.
4. **Release PR branch**: if the workflow opens a release pull request, its branch. Then an open release pull request
   blocks a new release, you're told when the new one opens, and a failed check goes to the Inbox. Leave it empty if
   the workflow publishes directly.
5. **Stable tag pattern**: how to recognise the release, `*` matching any text (`v*`). Empty: the repository's latest
   release.
6. Your other kind of release, if any (a beta, a nightly): its **name**, **inputs** as JSON (`{"stage": "beta"}`) and
   **tag pattern** (`v*-beta.*`).
7. **Run now offers first**: which kind the form starts on (the other kind by default, the safe one).
8. **Create workflow** and turn it on.

**Run now** asks which kind: the other kind, or **Stable** with an **Exact version** or a **Bump** (patch, minor,
major), which you confirm. Or ask the [PM](the-pm.md) ("cut a minor release") and it runs this for you. When the
release is published you get a notification and an Inbox item with its link. Your workflow and GitHub do the merging
and tagging, not PDT.

### After a release

**After a release** (under **Releases**) announces each new release of a project in a chat channel and bumps its
Homebrew cask. Each part is optional: fill in the ones you want.

1. **New workflow** › **After a release**, in the project.
2. **Stable releases only** (on): pre-releases are left alone. Turn it off to announce and bump them too.
3. **Announce in**: a chat connection (a Discord webhook, which posts to its channel). It posts the release's name,
   the start of its notes and its link. Leave it empty for no announcement.
4. **Homebrew tap** (`owner/homebrew-tap`, on GitHub) and **Cask** (its file is `Casks/<cask>.rb`): PDT downloads the
   release's macOS file, then sets the cask's `version` and `sha256` and opens a pull request on the tap. Leave the
   tap empty for no cask bump.
5. Optionally **macOS file**, a pattern for which of the release's files the cask installs (`*.dmg`,
   `*.app.tar.gz`; by default the first `.dmg`, else `.app.tar.gz`, else `.app.zip`), **Push instead of a pull
   request** to commit to the tap straight away, and the **Tap branch** (its default branch when empty).
6. **Create workflow**, check it with **Dry run**, and turn it on.

The cask needs one `version "…"` line and one `sha256 "…"` line: one file for every Mac. A `url` that spells out the
version gets the new one too; one with `#{version}` needs nothing. If the cask is already at the release, nothing
changes. Your GitHub login needs push access to the tap.

For PDT's own releases, set **macOS file** to `*.app.tar.gz` (the universal app) and point the cask's `url` at
`…/releases/download/v#{version}/<that file's name>`.

### Report a missing scheduled release

GitHub's scheduled workflows sometimes never start. **Report a missing scheduled release** (under **Releases**)
checks each day that yours came out:

1. **New workflow** › **Report a missing scheduled release**.
2. Fill in **Repository** (`owner/name`), **Release workflow** (`nightly-build.yml`) and **Tag pattern**, with `{date}`
   for the day (`20261030` or `2026-10-30`): `v*-beta.{date}*`, `snapshot-{date}`. Without `{date}`, any matching
   release that day counts.
3. **Expected by (hour, UTC)**: when it looks.
4. **Only if the branch changed** (on): stay quiet on days the **Branch** has nothing new since the last matching
   release.
5. **Start it inputs**: the workflow's inputs as JSON (`{"channel": "beta"}`), or `{}`.
6. **Create workflow** and turn it on.

A missing release puts an item in the Inbox with a link to the workflow's runs: **Start it** runs the workflow with
those inputs, **Ignore** leaves it. It clears itself once the release is published.
