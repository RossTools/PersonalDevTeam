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
| **PR notices** | Tells a ticket's agent when its pull request's CI fails, it conflicts with the base branch, it falls out of the merge queue, or it merges. When the pull request was set to merge, the agent fixes it and PDT merges it again. | On |
| **Finish merged tickets** | Moves a ticket to **Done** and cleans up once its branch merges, when you asked for that at **Merge…**. | On |
| **Commits pushed after a merge** | Asks you whether to open a follow-up ticket when an agent pushes to a branch that already merged. | On |
| **Ticket comments to the agent** | Sends your comments on a ticket to the agent working on it. | On |
| **PR comments to the agent** | Sends other people's comments and reviews on the pull request to the agent. | Off |
| **Base moved while a PR waits** | Tells the agent when the base branch moves while its pull request waits (**PR notices** covers this too). | Off |

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
2. Pick a template: search, or browse **CI & pull requests**, **Tickets**, **Issues**, **Notifications** and
   **Releases**. **Start from scratch** gives you an empty one; your own templates are under **Yours**, and copies of
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
5. **Merge**: PDT merges it (on GitHub, by turning on the pull request's auto-merge once CI passes) and moves the
   ticket to **Done**. A conflict or failing CI goes back to the Coder, then the Reviewer. A merge PDT can't fix (a
   draft pull request, the project's policy) asks you.

On the board, **In progress** splits into these stages, and each card says who has the ticket ("Reviewer · working",
"Your turn").

The Pipeline also handles **Refine** (in a **Draft** or **Ready** ticket's menu): the **Refiner** rewrites the ticket
so it's ready to build and moves it to **To do**, or asks you when something needs clearing up.

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
  on GitHub, and asks you after three days).
- **Passes before asking**: how many times work may go back to Code before PDT asks whether to go on (default 3; see
  [Loop limits](personas-and-approvals.md#loop-limits)).

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
   and passes over a ticket that failed to start until it changes. The run shows why it passed over each one.
3. **Start the ticket** starts the first one still free, with the agent, permission mode and view you give it (your
   defaults otherwise), handed to the project's workflow (the Pipeline) or to an agent session of its own.

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
  them in the repository, not in PDT.

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
run. Some have settings of their own:

- **On a schedule**: every so often, or at set times (a cron line, in your time zone).
- **Webhook**: when another service calls PDT's address for a webhook connection.
- **Run now**: only when you click **Run now** (or the PM runs it for you). It can ask for inputs first.

Any trigger can be narrowed to some projects, or to when a condition holds (the ticket has the label `ui`, say).

### Steps

**Add node** lists every step, grouped:

- **Act on tickets**: **Comment on the ticket**, **Move the ticket**, **Create a ticket**, **Finish the ticket** (stop
  its runs, remove its worktree), **Merge the attempt**, and **Find tickets to start**, **Choose tickets to start** and
  **Start the ticket** (see [Pick up tickets](#pick-up-tickets)).
- **Talk to agents**: **Tell the agent** messages the ticket's agent once it's between turns; an **Agent step** has a
  persona do a piece of work.
- **Decide**: **If** (yes or no), **Switch** (one exit per value), **Wait** (for a time, or an event with a time
  limit), **Ask the user** (a gate).
- **Tell people**: **Notify me**, **Put it in my Inbox**, a Discord post, a comment on a GitHub issue.
- **Reach out**: **Run a command**, **HTTP request** (through a connection), **MCP tool**, **Run a workflow**.
- Plugins (Jira, Buildkite, GitHub…) add their own steps while they're on.

A step's settings can use what came before: `{{trigger.ticketKey}}` is the event's ticket, `{{steps.review.summary}}`
is what the step called `review` handed in; the settings panel has a picker for these. Secrets go in as
`{{secret.name}}` and are filled in only as the step runs (see [Secrets](../your-machine/secrets.md)).

A step that can't run as set up (no project to work on, a repository that isn't `owner/name`) fails the run at once
with that message.

### Gates

**Ask the user** asks you something and waits. The question shows in the Inbox under **Gates** (or **Ready to
accept**) with a box for a note, which goes on the ticket as your comment. Each choice is an exit. A gate can give up
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
