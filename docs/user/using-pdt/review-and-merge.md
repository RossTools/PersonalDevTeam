# Review and merge

When an agent has worked on a ticket, you read what it changed, tell it what to fix, approve it, and merge it. All of
it happens in PDT; on GitHub the merge goes through the pull request.

## The Changes view

Open a ticket's **Changes** tab (it shows the size of the change, `+120 −8`). You can also get there from **Changes**
in a session's header, **Review changes** in a branch's menu (click the branch name), or a project's **Worktrees** in
**Settings**.

It shows everything on the branch against the base branch: committed, staged, not yet committed, and new files not
added to git yet (marked **Uncommitted** while there are any). The changed files are listed on the left; the diff of
the one you pick is on the right.

- **Split** or **Unified** lays the diff out side by side or in one column (narrow windows always get unified).
- **Changes only** folds the lines that didn't change; **Whole file** shows them all.
- A Markdown file can be shown as **Diff** or **Rendered** (before and after, side by side).
- Tick **Viewed** on a file to mark it read. It stays ticked in this browser until the file changes again.
- Keys: **]** and **[** for the next and previous file, **v** to mark it viewed, **c** to comment on the picked lines.

## Comment and send feedback

1. Click a line number to pick the line; **Shift+click** another to pick a range.
2. Click **+** beside the picked lines (or press **c**) and write your comment (Markdown). Comments are drafts until
   you send them; the bar above the diff counts them ("3 draft comments").
3. Click **Send feedback**. PDT sends every draft to the agent working on the branch in one message, with the file
   and lines of each, and the agent gets back to work. The comments are recorded on the ticket's **Activity** too.

Threads stay on their lines as the agent changes the file. If the lines a comment was on are gone, the thread says so
("The lines this was on have changed"). **Reply** to a thread, or **Resolve** it once it's dealt with.

## Approve

When you're happy, click **Approve**. The ticket records that you approved the branch as it is now; if the agent
pushes more commits afterwards, the approval no longer counts for them.

A project can require your approval before anything but you merges (**A person approves before an automated merge**
in its settings). Then a workflow or agent can only merge branches you approved.

## Merge

Click **Merge…** (on the Changes view, or in the ticket's menu). The dialog says what will happen, and what's in the
way if anything is.

**With a pull request** (a project on GitHub), the button depends on what the repository allows:

- **Add to merge queue**, when the repository uses one: the queue tests it on top of the latest base branch and
  merges it.
- **Turn on auto-merge**, when the repository allows auto-merge (a setting in its GitHub settings): GitHub merges the
  pull request as soon as its required checks pass. A pull request that can merge right away merges at once.
- **Merge when CI passes**, when the repository has neither: PDT merges the pull request itself, at once if its CI
  passed, otherwise as soon as it passes. Until then the badge says **Merging when CI passes**, and the ticket has a
  comment saying why.
- **Move to Done and clean up once it merges** (on by default) finishes the ticket for you when the merge happens:
  it moves to **Done**, its sessions and runs stop, and its worktree and workspace go. **Delete the branch** removes the
  branch too.

If the project merges through pull requests and the branch has none yet, the dialog says **No pull request yet**:
**Ask the agent to open one** does what it says.

**Without a pull request** (a project not on GitHub), PDT merges the branch into the base branch on your machine,
with your git identity:

- **Merge commit** or **Squash** (one new commit for the whole branch). PDT remembers your pick for the project.
- **Remove the worktree** and **Delete the branch** clean up after the merge.

Your own **Merge…** doesn't wait for anything. What workflows and agents may merge, and whether they wait for green CI
or your approval, is the project's policy: see [Personas and approvals](../automation/personas-and-approvals.md).

## After the merge

PDT checks every minute whether each ticket's branch has merged, however it was merged: through PDT, on GitHub, or by
hand. When it has, the ticket's attempt is marked merged, and:

- if you asked to clean up once it merges, PDT moves the ticket to **Done** and removes the worktree;
- otherwise the ticket shows **Move to Done and remove the worktree**. Click it, and PDT stops what still runs there
  (asking first), removes the worktree, and pulls the base branch so your main checkout is up to date.

A worktree with uncommitted changes is never thrown away without asking.

## Pull request and CI status

On a project on GitHub, each ticket in progress shows its pull request: a badge on its board card and in the
Sessions sidebar, and the details in the ticket's **Attempt** section. The badge says where it stands: **Draft**,
open with its CI result, **Auto-merge on**, **Merging when CI passes**, **In queue**, **CI failed**, **Conflict**, **Queue failed**, **Merged**,
**Closed**. The details list each CI run with its failed jobs, and link to them.

PDT reads them from GitHub about once a minute (and when you open the ticket). If it can't (GitHub isn't connected,
or is down), the badge shows a warning and the details say when it last checked.

When something goes wrong with a pull request, PDT tells the agent working on it, which fixes it and pushes:

- **CI failed**: the agent gets the failed jobs and reads their logs;
- **a conflict** with the base branch: the agent merges the base in and resolves it;
- **dropped from the merge queue** (on a repository with one), or **behind** its base while it waits to merge: the
  agent brings it up to date.

Once the pull request was set to merge, PDT sets it to merge again after the fix (auto-merge, the queue, or its own
merge once CI passes). If no agent is running on the ticket, the news goes to
you instead: a comment on the ticket and an item in the [Inbox](the-inbox.md). The **PR notices** workflow does this;
turn it off in **Workflows** if you'd rather handle it yourself.

## Conflicts

A conflict means the base branch changed the same lines as the ticket's branch.

- With a pull request, the badge shows **Conflict** and PDT tells the agent, as above.
- Merging locally, the **Merge…** dialog lists the files under **The merge conflicts**: **Send to agent** asks the
  agent to merge the base branch in and resolve them.

You can also just tell the agent yourself, in its session.

## On a phone

The changed files are a screen of their own; tap one for its diff, and swipe sideways for the next or previous file.
Tap a line number to pick it, tap another to extend the pick, then **+** to comment.
