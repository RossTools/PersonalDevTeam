# A first ticket

This walks one ticket from an idea to merged code: write it, start an agent on it, watch it work, review the result
and merge it.

## 1. Write the ticket

1. Open **Tickets** in the rail and click **New ticket**.
2. Pick the **Project**, give it a **Title**, and describe what you want in the description: what should change, why,
   and how you'll know it's done. The agent gets exactly this, so write it as you would for a colleague.
3. Click **Create ticket**.

A new ticket is a **Draft**. When it's ready to be worked on, move it to **To do** (drag it, or **Move to** in its
menu). **Ready** is for tickets that are refined but not picked yet. The **Board** shows **To do**, **In progress**
and **Done** side by side.

To add a ticket fast, type a title in the box at the top of the backlog and press **Enter**; **Shift+Enter** opens the
full form.

## 2. Start it

Click **Start** on the ticket. The **Start** dialog shows what will happen and lets you change it:

- **Agent**: your default agent, or another.
- **Session**: **Conversation** (a chat view of the agent's work, with its tool calls and diffs) or **Terminal** (the
  agent's own interface).
- **Branch** and **Base branch**: PDT names the branch after the ticket and makes it from your base branch.
- **Permissions**: what the agent may do without asking you.

Click **Start**. PDT makes the branch and a worktree for it, moves the ticket to **In progress**, and starts the
agent there with the ticket as its first message. The ticket opens in a tab, with the agent's session beside it.

If the ticket is blocked by another open ticket, or would change the same files as one in progress, the dialog says
so first. You can **Start anyway**, or **Wait** until it's clear.

## 3. Watch the agent work

The agent works in its own folder, so you can carry on with something else. Its activity shows in the **Sessions**
sidebar and on the ticket's card.

- When it needs you (a question, or permission to run something), it waits, and the session shows what it's asking.
  Answer in the conversation or approve the request.
- You can send it a message at any time: more detail, a correction, or "stop and explain".
- When it's done, it says so and the session goes idle.

On a project that uses pull requests, the agent pushes its branch and opens one. PDT then follows the pull request and
its CI, and if CI fails it tells the agent, which fixes it.

## 4. Review the changes

Open the ticket's **Changes** tab. It shows everything on the branch against the base branch: committed or not.

1. Read through the diff. To comment, click a line number (Shift+click another to take in a range), then click
   **+** on the picked lines and write your comment.
2. When you've left comments, click **Send feedback**. PDT sends them all to the agent in one message, and the agent
   gets back to work.
3. Repeat until you're happy, then click **Approve**.

## 5. Merge it

Click **Merge…**. What happens depends on the project:

- **With a GitHub pull request**, PDT turns on auto-merge (or adds it to the merge queue): GitHub merges once the
  required checks pass.
- **Without one** (a project not on GitHub, or GitHub skipped), PDT merges the branch into your base branch locally.

Once the branch is merged, PDT notices, moves the ticket to **Done** and tidies up the attempt. That's the whole loop.

## Let the agents review it too

Projects can run the **Pipeline**: after the coding agent submits its work, a reviewing agent and a testing agent go
over it and send it back with their findings until it passes, and then PDT asks you to accept it in the **Inbox**.
You still decide what merges. See [Automation](../automation/workflows.md).

## Next

[Where to go next](next-steps.md).
