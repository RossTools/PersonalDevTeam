# Review and merge

When an agent has worked on a ticket, you read what it changed, send it feedback, approve and merge, all in PDT. On
GitHub the merge goes through the pull request.

## The Changes view

Open a ticket's **Changes** tab (it shows the change's size, `+120 −8`), or **Changes** in a session's header, **Review
changes** in a branch's menu, or a project's **Worktrees** in **Settings**.

It shows everything on the branch against the base branch, committed or not, including new untracked files, on one
scrolling page like GitHub's **Files changed**. Each file's sticky header has its path (`old → new` for a rename),
`+added −removed`, **Uncommitted** when it has uncommitted changes, and its options.

- The file list on the left jumps to a file and highlights the one you're reading, with its comment count and a tick
  once viewed. The button at its top hides it.
- **Split** or **Unified** sets the layout (narrow windows are always unified).
- In a file's header: **Changes only** or **Whole file**; **Diff** or **Rendered** for Markdown; the arrow collapses it.
- **Viewed** marks a file read and collapses it, until the file changes again (remembered in this browser).
- Generated files (lockfiles, paths marked `linguist-generated` in `.gitattributes`), very large diffs and binary
  files start collapsed: **Load diff** shows one.
- Keys: **]** / **[** next and previous file, **v** mark viewed and move on, **c** comment on the picked lines.
- Leave the page and come back, and it is as you left it: the same file and place in it, **Rendered** where you chose
  it, collapsed files and a comment you were writing. When new commits changed the file, it opens at the file's top.
- The address follows the file you pick, so a reload or copied link opens at that file. Add `&line=12` to point at a
  line (`&side=old` for a removed one).

## Comment and send feedback

1. Click a line number to pick it; **Shift+click** another for a range.
2. Click **+** (or press **c**) and write your comment (Markdown). Comments stay drafts; the top bar counts them ("3
   draft comments").
3. Click **Send feedback**. Every draft goes to the branch's agent in one message, with each comment's file and lines,
   and is recorded on the ticket's **Activity**.

Threads follow their lines as the file changes; if the lines are gone, the thread says "The lines this was on have
changed". **Reply** to a thread, or **Resolve** it.

## Approve

Click **Approve** to approve the branch as it is now; commits pushed afterwards aren't covered.

With **A person approves before an automated merge** on in the project's settings, workflows and agents can only merge
branches you approved.

## Merge

Click **Merge…** (on the Changes view or in the ticket's menu). The dialog says what will happen and what's in the
way.

**With a pull request** (a project on GitHub), the button depends on the repository:

- **Add to merge queue**, if it uses one: the queue tests the change on the latest base branch and merges it.
- **Turn on auto-merge**, if it allows auto-merge: GitHub merges once the required checks pass (at once if they
  already have).
- **Merge when CI passes**, otherwise: PDT merges it itself once CI passes. The badge says **Merging when CI passes**
  meanwhile.
- **Move to Done and clean up once it merges** (on by default): when it merges, the ticket moves to **Done**, its
  sessions and runs stop, and its worktree and workspace go. **Delete the branch** removes the branch too.

If the branch has no pull request yet, the dialog says **No pull request yet**; **Ask the agent to open one**.

**Without a pull request** (a project not on GitHub), PDT merges into the base branch on your machine, with your git
identity: **Merge commit** or **Squash** (remembered per project), plus **Remove the worktree** and **Delete the
branch**.

Your own **Merge…** doesn't wait for anything. What workflows and agents may merge, and whether they wait for CI or
your approval, is project policy: see [Personas and approvals](../automation/personas-and-approvals.md).

## After the merge

PDT checks every minute whether each ticket's branch has merged, whichever way it was merged. When it has:

- if you chose to clean up once it merges, the ticket moves to **Done** and its worktree goes;
- otherwise the ticket shows **Move to Done and remove the worktree**: PDT stops what still runs there (asking first),
  removes the worktree and pulls the base branch into your main checkout.

A worktree with uncommitted changes is never thrown away without asking.

## Pull request and CI status

On GitHub projects, each ticket in progress shows its pull request: a badge on its card and in the Sessions sidebar,
and details in the ticket's **Attempt** section. The badge reads **Draft**, open with its CI result, **Auto-merge on**,
**Merging when CI passes**, **In queue**, **CI failed**, **Conflict**, **Conflict (deferred)**, **Updating**,
**Resolving conflicts**, **Queue failed**, **Merged** or **Closed**; the
details list each CI run and its failed jobs, with links.

PDT checks GitHub about once a minute (and when you open the ticket). If it can't reach GitHub, the badge shows a
warning and the details say when it last checked.

When a pull request runs into trouble, PDT tells the agent working on it, which fixes it and pushes:

- **CI failed**: the agent gets the failed jobs and reads their logs;
- **a conflict** with the base branch: resolved once, at merge time (see [Conflicts](#conflicts));
- **dropped from the merge queue**, or **behind** its base while waiting to merge: PDT brings the branch up to date
  itself first, and the agent gets only what's left.

A pull request that was set to merge is set to merge again after the fix. If no agent is running on the ticket, you
get the news instead, as a ticket comment and an [Inbox](the-inbox.md) item. This is the **PR notices** workflow; turn
it off in **Workflows** to handle these yourself.

## Conflicts

A conflict means the base branch changed the same lines as the ticket's branch. The base keeps moving while a ticket
is worked on, so conflicts are fixed once, when the ticket is next to merge, not each time the base moves.

- **Before the merge** (while the ticket is in Code, Review, QA or Accept), a conflict is shown on the ticket as
  **Conflicts with main (deferred)** and recorded on its **Activity**, but its agent isn't interrupted. The Reviewer
  and QA work on the branch as it is.
- **At the merge**, the Pipeline first brings the branch up to date (**Updating with main…**): PDT merges the base in
  itself and pushes once.
  - No conflicts: it merges, with one CI run.
  - Only generated files conflict (lockfiles, generated code): PDT takes the base's version, runs the command you set
    for them, and commits. See [Generated files at merge time](../automation/run-configuration.md#generated-files-at-merge-time).
  - Real conflicts: only then does an agent get them (**Resolving conflicts (agent)**), with the files, the conflicting
    lines and the tickets merged into the base since. It resolves them, runs the checks and pushes.
- The same happens when the merge queue drops a pull request, or it falls behind while waiting to merge.

The ticket's **Activity** says how each update went: merged cleanly, regenerated, or resolved by the agent. Git
remembers each resolution (`git rerere`), so a conflict resolved once resolves itself the next time.

To have the agent told as soon as a conflict appears instead, set **When to resolve conflicts** to **Immediately** in
the project's settings, under **Pipeline and policies**.

Merging locally, the **Merge…** dialog lists the files under **The merge conflicts**; **Send to agent** asks the agent
to merge the base in and resolve them. Or just tell the agent in its session.

## On a phone

The Changes view is the same page, with sticky file headers. **Jump to file** goes straight to a file. Tap a line
number, tap another to extend the pick, then **+** to comment.
