# FAQ

Answers to common questions about PDT, agents, costs and privacy.

## Does PDT need its own AI, or an AI account?

PDT brings no AI of its own. It runs the coding agents you install (Claude Code, Codex, Devin, or any ACP agent),
signed in as you, so they cost what they cost outside PDT: your subscription or your API usage. Running several
agents at once uses your plan's limits faster; **Agents running at once** in **Settings › This host › Agents** caps
it.

## Is my code sent anywhere?

Not by PDT. Your tickets, conversations and settings stay on your machine, and there's no PDT server to send them
to. The agents send what they read to their own providers, exactly as they do when you run them in a terminal.

PDT calls GitHub for its own updates, and, when you've connected it, for your projects' pull requests, CI and issues.
Bug reports go out only when you send one, and show what they include first.

## Do I need GitHub?

No. Without it, PDT merges a ticket's branch into your base branch on your machine, and everything else works the
same. With it, you get pull requests, CI status, merge queues and issues. See
[GitHub and other forges](../your-machine/github.md).

## Does PDT change my agents' settings?

No. PDT never edits your agents' own settings or config files. What it gives an agent (its instructions, its tools,
"Always allow" for a command) lasts for that session only.

## Do agents stop when I close the browser?

No. The browser is only a window onto PDT. Agents, runs and workflows carry on, and the Inbox collects what needs you.
They pause only when PDT itself stops (when your computer sleeps or restarts, or PDT updates), and resume when it's
back.

## Can I use PDT from my phone?

Yes, over Tailscale. See [Remote access and your phone](../your-machine/remote-access.md).

## Can anyone else on my tailnet get in?

No. PDT lets in only the Tailscale user you set as its **Owner**. Other users, shared and tagged devices, and Tailscale
Funnel are turned away.

## Can several agents work at the same time?

Yes. Each ticket gets its own branch and its own folder (a git worktree), so agents don't trip over each other. PDT
warns you when two tickets are likely to change the same files.

## Can I run PDT on a server and use it from my laptop?

Yes. Install the Linux server version on the server (see [Install PDT](../getting-started/install.md#linux)), turn on
lingering so it runs without you logged in, and reach it over Tailscale from your laptop and phone.

## Does PDT work on Windows and Linux?

Yes: macOS, Windows, and Linux (x86_64 and ARM, glibc 2.35 or newer). Differences between them are called out where
they matter, mostly in [Install and updates](../your-machine/install-and-updates.md).

## How do I get a feature or fix sooner?

Switch **Settings › This host › Updates › Update channel** to **Experimental builds too** to get builds between
releases. See [Release channels](../your-machine/install-and-updates.md#release-channels) for the catch.

## Can I go back to an older version?

Not by updating: PDT never installs an older version than the one you run, because an older version may not read the
newer one's data. If an update fails, PDT rolls it back for you.

## Where do I report a bug or ask for a feature?

**Report a bug…** in PDT (in the command palette, or **Settings › This host › Bug reports**), or see
[Contributing](../contributing/contributing.md).
