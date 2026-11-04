# A first ticket

Take one ticket from an idea to merged code: write it, start an agent on it, watch it work, review the result and
merge it.

## 1. Write the ticket

1. Open **Tickets** in the rail and click **New ticket**.
2. Give it a **Title** (with more than one project, pick its **Project** first) and describe what should change,
   why, and how you'll know it's done. The agent gets exactly this, so write it as you would for a colleague.
3. Click **Create ticket**.

A new ticket is a **Draft**, in the backlog. When it's ready to be worked on, move it to **To do** (drag it, or
**Move to** in its menu); **Ready** is for tickets that are refined but not picked yet. The **Board** shows **To do**,
**In progress** and **Done** side by side, so a ticket you make there starts in **To do** (pick another **Stage** in the
form if you like). When a new ticket lands where you aren't looking, PDT says where it went.

To add a ticket fast, type a title in the box at the top of the backlog and press **Enter**; **Shift+Enter** opens the
full form.

## 2. Start it

Click **Start** on the ticket. The **Start** dialog lets you change:

- **Agent**: your default agent, or another.
- **Session**: **Conversation** (a chat view with the agent's tool calls and diffs), **Terminal** (the agent's own
  interface) or **No session** (only the branch and worktree).
- **Branch** and **Base branch**: named after the ticket, made from your base branch.
- **Permissions**: what the agent may do without asking you.

Click **Start**. PDT makes the branch and its worktree, moves the ticket to **In progress** and starts the agent with
the ticket as its first message. The ticket opens in a tab, with the agent's session beside it.

If the ticket is blocked by another open ticket, or would change the same files as one in progress, the dialog says
so: **Start anyway**, or **Wait** until it's clear.

## 3. Watch the agent work

The agent works in its own folder, so you can do something else. Its activity shows in the **Sessions** sidebar and
on the ticket's card.

- When it needs you (a question, or permission to run something), the session shows what it's asking. Answer in the
  conversation or approve the request.
- You can send it a message at any time: more detail, a correction, or "stop and explain".
- When it's done, it says so and the session goes idle.

On a project with pull requests, the agent pushes its branch and opens one. PDT follows its CI and tells the agent
when it fails.

## 4. Review the changes

Open the ticket's **Changes** tab: everything on the branch against the base branch, committed or not.

1. To comment, click a line number (Shift+click another for a range), click **+** on the picked lines and write.
2. Click **Send feedback**. PDT sends all your comments to the agent in one message, and it gets back to work.
3. Repeat until you're happy, then click **Approve**.

## 5. Merge it

Click **Merge…**:

- **With a GitHub pull request**, PDT turns on auto-merge (or joins the merge queue), and GitHub merges once the
  required checks pass.
- **Without one**, PDT merges the branch into your base branch locally.

Once the branch is merged, PDT moves the ticket to **Done** and tidies up.

## Let the agents review it too

With the **Pipeline** workflow, a reviewing agent and a testing agent go over the coding agent's work and send it
back until it passes; then PDT asks you to accept it in the **Inbox**. You still decide what merges. See
[Workflows](../automation/workflows.md).

## Next

[Where to go next](next-steps.md).
