# Workflows

A workflow lets PDT act for you when something happens: CI fails on a ticket's pull request, a branch merges, a new
issue is opened, a ticket is started. Each one is a small graph: a **trigger** (what happened), then **steps** (what to
do about it), with branches where it decides something or asks you.

PDT ships with a few workflows already on, such as telling an agent when its pull request's CI fails. The biggest one
you can add is the **Pipeline**, which hands each ticket from a coding agent to a reviewing agent and a testing agent,
then to you, then to merge.

Open **Workflows** in the rail. The list on the left has your workflows grouped by where they apply: **This host**
(every project), **Personal**, and each project. A dot shows whether each is on.

## The built-in workflows

These come with PDT. You can turn them off, edit them, or copy them, but not delete them.

| Workflow | What it does | Starts |
| --- | --- | --- |
| **PR notices** | Tells a ticket's agent when its pull request's CI fails, it conflicts with the base branch, it falls out of the merge queue, or it merges. When the pull request was set to merge, the agent fixes it and PDT merges it again. | On |
| **Finish merged tickets** | Moves a ticket to **Done** and cleans up once its branch merges, when you asked for that at **Merge…**. | On |
| **Commits pushed after a merge** | When an agent pushes to a branch that already merged, asks you whether to open a follow-up ticket for those commits. | On |
| **Ticket comments to the agent** | Sends your comments on a ticket to the agent working on it. | On |
| **PR comments to the agent** | Sends other people's comments and reviews on the pull request to the agent. | Off |
| **Base moved while a PR waits** | Tells the agent when the base branch moves while its pull request waits. **PR notices** covers this too. | Off |

Plugins add their own: turning on Jira, say, adds **Import assigned Jira issues** and **Done moves the Jira issue**.
See [Plugins and connections](plugins.md).

What the enabled workflows will tell an agent is also in the agent's instructions, so it knows to expect them.

## Start from a template

1. In **Workflows**, click **New workflow**.
2. Pick a template. Search by name, or browse the categories on the left (**CI & pull requests**, **Tickets**,
   **Issues**, **Notifications**, **Releases**). **Start from scratch** gives you an empty workflow.
3. The panel on the right shows **When** (its triggers) and **What it does** (its steps). Fill in its settings, if it
   has any.
4. Give it a **Name** and pick its **Project**: one project, **This host (every project)**, or **None (personal)**.
5. Click **Create workflow**.

A new workflow starts **off**, so nothing happens until you turn it on.

A template whose plugin is off is shown faded; its preview links to the plugin so you can turn it on first. PDT's
built-in workflows are at the end, in **Built-in**: adding one again makes a second copy.

## The Pipeline

The **Pipeline** (under **Tickets**) is the template most projects want. Add it for a project, turn it on, and every
ticket you start in that project goes through these stages:

1. **Code**: the **Coder** builds the ticket on its branch, runs the checks, commits and pushes, and opens a pull
   request when the project uses them.
2. **Review**: the **Reviewer** reads the change against the ticket. **Changes requested** sends it back to Code with
   the Reviewer's comments; **Approved** moves it on.
3. **QA**: the **QA** agent runs the tests and tries each point of the ticket's "Done when". **Fail** sends it back to
   Code; **Pass** moves it on.
4. **Accept**: the ticket waits for you under **Ready to accept** in the [Inbox](../using-pdt/the-inbox.md). Accept it
   and it merges; reject it with a note and the note goes back to the Coder. A ticket labelled `agentic` skips this
   stage.
5. **Merge**: PDT merges it (on GitHub, by turning on the pull request's auto-merge once CI passes), then moves the
   ticket to **Done**. A merge that fails because of a conflict or failing CI goes back to the Coder to fix and to the
   Reviewer to check the fix; one PDT can't fix (a draft pull request, the project's policy) asks you what to do.

On the board, a project with the Pipeline splits **In progress** into these stages, and each card says who has the
ticket: "Reviewer · working", "Your turn".

The Pipeline also handles **Refine** (in a **Draft** or **Ready** ticket's menu): the **Refiner** rewrites the ticket
into one that is ready to build, then moves it to **To do** (or asks you when it needs something cleared up first).

### Pipeline settings

When you create it, you can change:

- **Refiner persona**, **Coder persona**, **Reviewer persona**, **QA persona**: who does each stage. See
  [Personas and approvals](personas-and-approvals.md).
- **Who refines**, **Who codes**, **Who reviews**, **Who does QA**: `agent`, or `user` to do that stage yourself. A
  stage of yours shows in the Inbox as "Your turn", with the stage's outcomes to pick from when you're done.
- **A refined ticket goes to**: `todo` (on the board) or `ready` (the backlog).
- **Who accepts**: `user` (you, in the Inbox) or `auto` (it merges once QA passes).
- **Labels accepted automatically**: tickets with one of these labels skip Accept. The default is `agentic`.
- **Who merges**: `workflow` (the Pipeline merges after Accept) or `person` (it waits until someone merges, in PDT or on
  GitHub, and asks you after three days).
- **Passes before asking**: how many times the work may go back to Code before PDT asks you whether to go on (3).

After that, each setting lives in the step it belongs to: click the step in the graph to change it.

What a workflow may merge at all is the project's policy, set apart from any workflow: see
[Personas and approvals](personas-and-approvals.md#approvals-and-merging).

## Turn a workflow on or off

The **Enabled** switch at the top of a workflow turns it on. Before it does, PDT shows where it runs and what it can
do (comment on tickets, send messages to agents, merge…), with **Dry run first** if you want to see it work on a real
event without changing anything. **Turn on** confirms.

While it's on, PDT runs it whenever its trigger happens. Turning it off stops new runs; runs already going finish.

## Read a run

A workflow's **Runs** tab lists its runs, newest first. Pick one to see:

- the graph of the version it ran, with the path it took;
- each step, with what it got (its input), what it gave (its output) or the error it stopped with;
- **Open session** on a step that ran an agent, to see what the agent did. A ticket's step sessions are left out of
  the sidebar's **Agents** list; open them here or from the ticket.

A run is **Waiting** while a step waits: for an agent to finish, for you to answer, for CI, or for a time. **Cancel**
stops it.

When a run fails, **Retry from the failed step** runs it again from where it stopped, with the same ticket and data.
When an agent step stopped or ran out of time, **Resume from** that step starts it again in the same session, with an
optional note from you. You can resume a timed-out step from the Inbox too.

Failed runs also show in the Inbox's **Updates**.

## Build or change a workflow

A workflow page has three ways to edit the same draft:

- **Graph**: the steps as boxes joined by arrows. **Add node** adds a step after the selected one; drag from a step's
  port (one of its exits) to another step to join them; **Delete** removes the selected one; **Arrange** tidies the
  layout.
- **Outline**: the same as an indented list, each trigger with its steps beneath, and a branch for each exit. Use the
  **⋯** on a row to add, move or remove a step. This is the default on a phone.
- **Text**: the whole workflow as JSON or TOML.

Click a step to open its panel: its settings, its **Title**, and **When it fails** (**Retries**, **Time limit**,
**Visits per run**, and **On error, go to**: a step to take over instead of failing the run). Problems show on the
step as you edit.

Changes collect in an **Unsaved changes** bar: **Save** (with an optional note) makes a new version, **Discard** drops
them. **Versions** lists every saved version, who saved it, and what changed; **Restore this version** brings an old
one back.

### Triggers

A trigger starts a run. Each kind of event in PDT is one: a ticket moved, started, commented on or merged; CI failed or
passed; a pull request conflicts, left the merge queue or merged; a new GitHub issue or outside pull request; a failed
run of a GitHub Actions workflow. Some have settings of their own:

- **On a schedule**: every so often, or at set times (a cron line, in your time zone).
- **Webhook**: when another service calls PDT's address for a webhook connection.
- **Run now**: only when you click **Run now** (or the PM runs it for you). It can ask for inputs first (below).

Every trigger can narrow which events count: only some projects, or only when a condition holds (say, the ticket has
the label `ui`).

### Steps

The catalog in **Add node** lists every step, grouped. The main ones:

- **Act on tickets**: **Comment on the ticket**, **Move the ticket**, **Create a ticket**, **Finish the ticket** (stop
  its runs, remove its worktree), **Merge the attempt**.
- **Talk to agents**: **Tell the agent** sends a message to the ticket's agent (it waits until the agent is between
  turns); an **Agent step** has a persona do a piece of work (below).
- **Decide**: **If** (yes or no), **Switch** (one exit per value), **Wait** (for a time, or for an event, with a time
  limit), and **Ask the user** (a gate, below).
- **Tell people**: **Notify me** (a notification in PDT), **Put it in my Inbox**, a post to Discord, a comment on a
  GitHub issue.
- **Reach out**: **Run a command**, **HTTP request** (a web service, through a connection), **MCP tool**, or **Run a
  workflow** (another one).
- **Plugins' steps**: Jira, Buildkite, GitHub and the others add theirs while they're on.

A step's settings can use what came before: `{{trigger.ticketKey}}` is the ticket the event is about,
`{{steps.review.summary}}` is what the step called `review` handed in. The settings panel has a picker for these.
Secrets go in as `{{secret.name}}` and are filled in only as the step runs: see
[Secrets](../your-machine/secrets.md).

### Gates

A gate (**Ask the user**) asks you something and waits. Its question and choices show in the Inbox under **Gates** (or **Ready to
accept**), with a box for a note; the note goes on the ticket as your comment, so "no" can say what to change. Each
choice is an exit of its own. A gate can also give up after a time, or close by itself when something else happens
first (the ticket merged meanwhile, say).

### Agent steps and outcomes

An **Agent step** runs a persona (Coder, Reviewer, your own) on the ticket with a prompt you write, and waits for it
to say how it went. You name its **outcomes**, each with a line saying when to pick it (`approved`: the change is
ready; `changes_requested`: it needs work), and each outcome is an exit. The agent ends its work by handing in one of
them, with a summary and, for a review, line comments on the change.

How agent steps pick their session, what happens when an agent stops without an outcome, and the limit on going round
are in [Personas and approvals](personas-and-approvals.md).

### Dry run

**Dry run** runs the workflow as it is in the editor against an event, without sending, moving or changing anything.
Pick a recent event of the trigger's kind, write one, or give a manual input. It shows the path the run would take,
why a trigger didn't match, and what each step would have done.

**Run now** runs the saved version for real, for a workflow with a manual trigger.

### Run now's inputs

A **Run now** trigger can declare the inputs it takes, in its settings under **Inputs**. Each has an **id** (steps read
it as `{{trigger.<id>}}`), a **label**, a **type** (text, number, true/false, or a choice from a list, each with a value
and a label), a **default**, whether it is **required**, and a short **help** line. **Show if** shows a field only while
another one has a value: Bump only for a stable release, say (`""` means the other field is empty).

With inputs declared, **Run now** opens a form with them, defaults filled in, and starts the run with what you pick;
on a phone it is in the workflow's **⋯**. Without any, it runs at once. A **confirm** text is asked before the run
starts (it can name the inputs, `{{trigger.bump}}`); give it a **show if** to ask only for some runs. The outline shows
what Run now asks for.

The same check applies however a run starts (the form, the API, the PM, another workflow): defaults are filled in, a
field that isn't shown is left out, and an input the trigger doesn't declare, or a value of the wrong kind, is refused
with the list of inputs it takes.

A workflow for this host (not one project's) whose steps work on the run's project, such as a GitHub step without a
repository, needs a project to run in: its Run now form asks for **Project**, starting on the project you have open.
A start without one (through the API, say) is refused, saying so.

A step that can't run because of how the workflow is set up (no project to work on, a repository that isn't
`owner/name`, a project that isn't on GitHub) fails the run at once with that message, instead of trying again first.

### Share and copy

- **Save as template…** (in the workflow's **⋯**) keeps a copy to start new workflows from; your templates show in
  **New workflow** under **Yours**.
- **Duplicate** copies it; **Delete…** removes it with its runs.
- **Export…** and **Import…** (at the top of the list) move workflows between PDT hosts as a file.
- A project can keep workflows in its repository, under `.pdt/workflows/`. PDT adds each one, off, marked
  **In the repo**; it can't be edited in PDT, only in the repository.

The PM can write a workflow for you too: what it saves is a proposal you review in **Versions** and **Accept** or
**Discard**.

## Examples

### Tell the agent when CI fails

PDT's **PR notices** already does this. To send your own words instead:

1. **New workflow** › **Tell the agent when CI fails**.
2. Change **Message**, say to "CI failed. Read the failed job's log, fix the cause, run the checks locally and push."
   Keep `{{trigger.notice.text}}` in it to pass on PDT's own notice (which jobs failed and how to read their logs).
3. Pick the project, **Create workflow**, then **Dry run** it against a recent CI failure.
4. Turn it on, and turn **PR notices** off for that project's pull requests if you don't want the agent told twice.

The workflow is two steps: the **CI failed** trigger, and **Tell the agent**, which waits until the agent is between
turns and sends the message once per failure.

### Triage GitHub issues

1. Turn on the **GitHub** plugin and connect your GitHub login (see [Plugins and connections](plugins.md)).
2. **New workflow** › **Triage new GitHub issues**.
3. Fill in **Repository** (`owner/name`) and **Project for its tickets** (the project's key, `ABC`). Leave **Persona**
   as **Triage**.
4. **Create workflow** and turn it on.

From then on, each new issue goes to the **Triage** agent. It looks for a ticket that already covers it, then drafts a
new ticket, marks the issue a duplicate, or leaves it. Each draft waits in the Inbox under **Triaged tickets** for you
to **Approve** (to **To do**) or **Reject** (to **Won't do**). The first time it runs it only notes where the repository
is, so old issues aren't triaged. Issues are written by other people, so triage runs locked down: see
[Outside contributions](outside-contributions.md).

### Report failed workflow runs

PDT already tells a ticket's agent when CI fails on its pull request. Runs nobody is waiting for (a release workflow
started from GitHub's Actions page, a nightly build on a schedule, a deploy after a push) can fail unnoticed. To get
each failed run in the Inbox:

1. Turn on the **GitHub** plugin and connect your GitHub login (see [Plugins and connections](plugins.md)).
2. **New workflow** › **Report failed workflow runs** (under **CI & pull requests**).
3. Fill in **Workflow**: its file (`release.yml`) or its name as GitHub Actions shows it. Leave **Repository** empty
   for the project's own repository, or give another (`owner/name`).
4. To narrow it, fill in **Branch** (`main`) and **Started by** (`schedule, workflow_dispatch` for scheduled and
   manual runs only). **Check every** is how often PDT looks (every 30 minutes unless you change it).
5. Pick the project, **Create workflow** and turn it on.

For example, with **Workflow** `release.yml` and **Started by** `schedule`, a nightly release that fails puts one
item in the Inbox: the run's title, its branch, what started it, its version or tag when it has one, its failed jobs,
and a link to the run. **Re-run failed jobs** runs them again on GitHub as your GitHub login (if your login may not,
the Inbox says so and you re-run them on GitHub). The item goes away by itself once a later attempt of the run passes,
however it was re-run; if the re-run fails too, a new item takes its place. **Dismiss** leaves the run as it is.

The first look only notes where the workflow's runs are, so runs from before you turned it on aren't reported. PDT
keeps its place between looks, so a run that ends while PDT is stopped is reported when it starts again, once.

### Ask before finishing a merged ticket

**Ask before finishing a merged ticket** puts a gate in the Inbox whenever a ticket's branch is found merged: **finish**
moves it to **Done** and removes its worktree, **keep** leaves it as it is.

### Cut a release

**Cut a release** (a template under **Releases**, with the GitHub plugin) runs a project's release workflow and follows
it through to the published release. It works with any release workflow that can be started by hand
(`workflow_dispatch`). Make one for each project you release:

1. **New workflow** › **Cut a release**, in the project.
2. **Release workflow**: its file in `.github/workflows` (`publish.yml`), and the **Branch it runs on** (`main`).
3. **Stable release inputs**: the workflow's inputs for a stable release, as JSON. The default,
   `{"bump": "{{trigger.bump}}", "version": "{{trigger.version}}"}`, passes the bump or the exact version you pick;
   rename them to the workflow's own names. Empty ones are left out.
4. **Release PR branch**: if the workflow opens a release pull request, the branch it opens it from. Then a release
   pull request that is still open stops a new release (the Inbox says so), you are told when the new one is open, and
   a failed check goes to the Inbox while it waits for the merge. Leave it empty when the workflow publishes the
   release itself.
5. **Stable tag pattern**: how to recognise the release it publishes, where `*` is any text (`v*`). Empty: the
   repository's latest release.
6. The other kind of release it makes, if any (a beta, a nightly): its **name**, its **inputs** as JSON
   (`{"stage": "beta"}`) and its **tag pattern** (`v*-beta.*`; pre-releases count). It has no release pull request.
7. **Run now offers first**: the kind the form starts on; the other kind, by default, which is the safe one.
8. **Create workflow** and turn it on.

**Run now** then asks which kind: the other kind, or **Stable** with an **Exact version** or, when that is empty, a
**Bump** (patch, minor or major). A stable release asks you to confirm it first, naming the bump or version. Usually
you ask the [PM](the-pm.md) for a release ("cut a minor release") and it runs this for you, with input like
`{"type": "stable", "bump": "minor"}`.

When the release is published, you get a notification and an Inbox item with its link. PDT itself merges and tags
nothing: your workflow and GitHub do.

Cut a release used to be a built-in workflow. One you turned on or ran stays as your own workflow, as it was; one
you never used is removed. Make a new one from the template to get Run now's form.

### Report a missing scheduled release

GitHub's scheduled workflows are best-effort, and some days a scheduled run never starts. **Report a missing
scheduled release** (under **Releases**) checks each day that your scheduled release came out:

1. **New workflow** › **Report a missing scheduled release**.
2. Fill in **Repository** (`owner/name`), **Release workflow** (its file, `nightly-build.yml`) and **Tag pattern**:
   the tag the day's release gets, with `{date}` for the day (`20261030` or `2026-10-30`), e.g. `v*-beta.{date}*` or
   `snapshot-{date}`. Without `{date}`, any matching release published that day counts.
3. Set **Expected by (hour, UTC)**: when the day's release should be out. It looks then.
4. Leave **Only if the branch changed** on to stay quiet on days the **Branch** has nothing new since the last
   matching release (or the latest release), if your workflow skips those days.
5. **Start it inputs**: the workflow's inputs when you start it from the Inbox, as JSON (`{"channel": "beta"}`), or
   `{}`.
6. **Create workflow** and turn it on.

When the day's release is missing, the Inbox says so, with a link to the workflow's runs: **Start it** runs the
workflow on the branch with those inputs, **Ignore** leaves it. The item goes away by itself once the release is
published.
