# Prompt: write a project's agent instructions in PDT

PDT has this prompt built in: **Write with an agent** under a project's Agent instructions in Settings starts a
session with it, and `GET /api/guides/set-up-workflow` serves it. It writes the project's **agent instructions**,
which PDT keeps on the host and gives to every agent session in the project, so a repository you cannot (or would
rather not) commit an AGENTS.md or CLAUDE.md to still gets a workflow.

---

Help me write the **agent instructions** for this project in PDT (Personal Dev Team, which started this session). PDT
keeps them on this host, not in the repository, and adds them to every Claude Code and Codex session it starts in the
project, after the repository's own CLAUDE.md / AGENTS.md. They are how I want agents to work here.

**1. Find PDT and this project.** PDT's instructions for this session name its API (`http://127.0.0.1:<port>/api`)
and this session's id. `GET /api/sessions/<session id>` gives the `projectId`. Read `GET /api/guides/agents` (what
PDT offers agents: tickets, worktrees, runs and tests) and `GET /api/projects/<projectId>/settings`: if
`agentInstructions` is already set, show it to me and ask what to change.

**2. Learn what the repository already says.** Read its README, contributing docs, any CLAUDE.md / AGENTS.md /
.cursorrules, the CI workflows and the build files, so the instructions don't repeat or contradict them. Note the real
check commands (format, lint, test, build) from CI.

**3. Ask me** (in one short list, with your suggested answer for each, taken from what you read):
- Branches: should every change get its own branch and worktree through PDT (`POST /api/projects/<id>/worktrees`),
  and what are branches named? Is committing on the base branch ever fine?
- Checks to run before pushing or saying a change is done.
- Pull requests: who opens them, against which branch, title and description conventions, labels, reviewers.
  Anything that must never happen (force-pushing, merging, pushing to the base branch).
- Tickets: should agents add follow-up work as PDT tickets, comment on the ticket when they finish, or leave the
  ticket alone?
- Anything about this codebase an agent keeps getting wrong.

**4. Write and save them.** Short and direct, as a Markdown list grouped under a few headings, only what is specific
to this project and not already in the repository's own instructions (at most a page; PDT's own context, such as its
API address and this project's id, is added separately). Show me the text, and once I agree, save it:
`PUT /api/projects/<projectId>/settings` with `{"agentCredentials": <the value you read>, "runOverrides": <the value
you read>, "agentInstructions": "<the text>"}` (the PUT replaces `agentCredentials` and `runOverrides`, so send back
what you read). Then `GET` the settings again to check it was saved.

**5. Report** in a line or two: what you saved, and that new agent sessions in this project get it (sessions already
running keep what they started with).
