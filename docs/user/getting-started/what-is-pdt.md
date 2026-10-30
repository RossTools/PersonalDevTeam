# What PDT is

PDT (Personal Dev Team) is a dev team that lives on your machine. You write tickets; coding agents such as Claude
Code, Codex and Devin build them, each in its own branch and folder; you review the work and decide what ships.

## What you get

- **A board for your work.** Tickets move from **Draft** through **To do** and **In progress** to **Done**. Big
  tickets split into parts.
- **Agents that work in parallel without stepping on each other.** **Start** on a ticket makes a branch and a git
  worktree for it and starts an agent there with the ticket as its brief. Several tickets can be in flight at once.
- **Sessions that outlive your browser.** Agents run on your machine, not in a browser tab. Close the window, come
  back later or check from your phone: the work carries on.
- **Review built in.** See a branch's changes, comment on lines and send the comments back to the agent, approve,
  then merge, through a pull request or locally.
- **Pull requests and CI watched for you.** When CI fails or a pull request needs attention, PDT tells the agent
  working on it, so you don't have to relay it.
- **Automation when you want it.** Workflows can hand a ticket from a coding agent to a reviewing agent and a
  testing agent, then ask you to accept it. The PM, an agent for your whole machine, plans work and keeps tickets
  tidy when you ask it to.
- **One click to try a branch.** **Run** starts any branch's build on its own port with its own data, so you can try
  the change before it merges.

## How it fits together

PDT runs as a small service on your computer and you use it in a browser at `http://127.0.0.1:7420`. On macOS and
Windows a tray app opens it for you. It uses the agents and git you already have, logged in as you, and keeps its
data on your machine. With Tailscale you can reach it from your phone or another computer, and only you can.

PDT never changes your agents' own settings or config files.

## Next

[Install PDT](install.md).
