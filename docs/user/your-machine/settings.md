# Settings

What each section of Settings changes, for this machine and for each project.

Open **Settings** in the rail (on a phone, in the **☰** menu). The list on the left has three parts:
**This host** (the machine running PDT), a few pages of their own (**Notifications**, **Plugins**, **All
connections**, **Secrets**, **MCP servers**), and each project. **Search settings** at the top finds a section by its name or any
setting on it.

Every setting saves as you change it: a moment of **Saved** beside it says so. Tokens and secrets have their own
**Save** button.

## This host

### Updates

How PDT updates itself. See [Updates](updates.md#updates) for what each choice does.

- **Version**: the version you run, with **Check for updates**, and what the last check found.
- **Install updates**: **Install automatically when idle**, **Notify only** or **Manual (no checks)**.
- **Installing updates**: a time window, whether to wait for agents or install anyway after a while, stable releases
  only, the check after installing and telling you what changed. These set the **Install updates** workflow. See
  [Installing updates by themselves](updates.md#installing-updates-by-themselves).
- **Update channel**: **Stable releases**, or **Nightly builds too**.
- **Check for updates**: how often PDT looks.
- **Release repository**: where updates come from. Leave it empty.
- **GitHub token**: the token PDT reads releases and files bug reports with, kept in your keychain.

### Agents

- **Default agent**: the agent a ticket's **Start** picks first, and listed first everywhere else.
- **Default view**: whether a new session opens as a **Conversation** or the agent's own **Terminal**.
- **Ticket permissions** and **Session permissions**: the permission mode tickets and new sessions start with. See
  [Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#models-and-permission-modes).
- **Agents running at once**: the most agents working at the same time before PDT's own work (workflow steps,
  auto-start) waits for a slot. What you send yourself never waits.
- **Triage per author** and **Triage per repository**: the most new issues triaged an hour, from one author or one
  repository. See [Outside contributions](../automation/outside-contributions.md).
- **Credentials**: **Subscription (remove API keys)** starts agents without `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`,
  so they use your subscription login; **Pass through** keeps them (for API keys, Bedrock, Vertex or a proxy). A
  project can choose differently.
- **When an agent is stuck**: how many times a stalled agent is nudged, the nudge messages, whether it is restarted
  in a fresh session before you are asked, and what happens at a loop limit or a time limit. See
  [When an agent is stuck](../automation/personas-and-approvals.md#when-an-agent-is-stuck).
- **Default models**: the model each agent starts with, unless the session or its persona picks another.
- **Programs**: where each agent's program is, a switch to turn the agent off, and whether PDT found it. Leave a path
  empty to use the one on your `PATH`.
- **ACP agents**: other agents that speak the Agent Client Protocol. See
  [Connect agents and GitHub](../getting-started/connect.md#other-agents-acp).

### PM

- **Daily checkpoint**: when the PM's conversations are brought up to date: at a set time or each hour, on which
  days, and a chat channel to post the update to. These set the **PM daily checkpoint** workflow. See
  [Change when it's brought up to date](../automation/the-pm.md#change-when-its-brought-up-to-date).

### Network

Where PDT listens. Changes apply when PDT restarts: a banner offers **Restart now**, and **Network** shows a
**restart** badge until then. Sessions are interrupted and resume when PDT is back.

- **Port**: PDT's port, 7420 unless you change it (if something else needs 7420, say). After a restart on a new
  port, the page reopens itself there.
- **Allowed hosts**: other names you open PDT by on this machine (an alias in your hosts file, say), with `:port` when
  needed. `localhost` and your tailnet name are always allowed; PDT refuses a page opened by any other name, so that
  websites can't reach it through your browser.

A setting marked **env** is set by an environment variable PDT was started with (`PDT_PORT`, `PDT_ALLOWED_HOSTS`),
which wins over the setting, so you can't change it here. See
[Troubleshooting](../troubleshooting/troubleshooting.md#the-port-is-in-use-or-the-field-is-locked).

### Remote access

**Serve on the tailnet**, its **Status** and address, and the **Owner**. See
[Remote access and your phone](remote-access.md).

### Runs

- **Ports**: the range runs (dev servers, tests) get their ports from.
- **Max concurrent runs**: how many runs may run at once.

See [Run configuration](../automation/run-configuration.md).

### Storage

Free space on each disk PDT uses, and what each project's worktrees take, with their build output apart.

- **Clean** or **Remove…** a worktree, or **Clean all finished** tickets' worktrees at once.
- **Low-disk warning**: below this much free space (20 GB unless you change it), the Inbox warns you.
- **Pause auto-start below**: below this (5 GB), auto-start starts no new tickets.

See [Disk space](your-data.md#disk-space).

### Projects folder

Where a repository you add by **Clone URL** is cloned to (`~/work` unless you change it).

### Documents

The **Default provider** new [documents](../using-pdt/documents.md) go to (the first one PDT has, unless you pick
one), and the **Local documents folder** where documents kept on this machine are saved (PDT's data folder unless you
change it). A project can set its own.

### Appearance

**Theme**: **System**, **Dark** or **Light**. It's kept in this browser, so your phone can differ from your computer.

### Bug reports

**Send reports to**: where **Report a bug…** sends a report about PDT: **Automatic** (a GitHub issue on PDT's
repository), **GitHub issue**, **Local ticket** (a draft ticket here, nothing sent anywhere) or **Off**. The report
form shows what it includes before you send it, and its logs are cleaned of secrets, tokens and your machine's names.

### Checks

Whether PDT found the programs it runs: git, the agents and Tailscale, with where it found them or what's wrong.
**Run checks again** after installing something.

### Setup

**Open Get started** brings back the first-run checklist.

### Danger zone

**Remove the GitHub token** deletes the update token from your keychain. It always asks first.

## Notifications

How each kind of news reaches you: off, a dot, the Inbox, a toast, or a desktop notification, and **Desktop
notifications** for this device. You can also escalate what waits unanswered, set quiet hours, and post questions,
approvals and failed runs to a chat channel. These set the **Notification rules** workflow. See
[The Inbox](../using-pdt/the-inbox.md#notifications).

## Plugins and All connections

Turn plugins on and off and manage their connections; the GitHub plugin is where you sign in to GitHub. See
[Plugins and connections](../automation/plugins.md) and [GitHub and other forges](github.md).

## Secrets

Keys and tokens that agents and workflows use without seeing them. See [Secrets](secrets.md).

## MCP servers

MCP servers every agent gets, signed in once, with each tool allowed, asked or blocked. See
[MCP servers](mcp-servers.md).

## A project's settings

Each project has its own sections. A personal project (one without a repository) has fewer.

- **General**: its **Name**, its **Key** (the letters in its ticket numbers), the **Base branch** work starts from,
  and its **Repository folder**.
- **Worktrees**: the checkouts PDT made for its branches, each with its ticket and size. Open one's changes,
  **Clean** its build output or remove it. **Clean build output after** and **Remove worktree after** set the
  [automatic cleanup](your-data.md#disk-space) of finished tickets' worktrees.
- **Runs on this host**: what **Run** and **Test** start, from the repository's file plus overrides kept on this
  machine. See [Run configuration](../automation/run-configuration.md).
- **Agent instructions**: extra instructions for every agent in this project, on top of the repository's own
  `CLAUDE.md` or `AGENTS.md`. Kept on this machine, never written to the repository.
- **Agents**: **Agent credentials** for this project, and the **Agent permission mode for workflow steps**.
- **Review**: **Merge through pull requests**: **Merge…** only turns on auto-merge for the pull request, and never
  merges on your machine.
- **Pipeline and policies**: who may merge, approvals, how many tickets run at once, auto-start, and whether the PM
  acts without asking. See [Workflows](../automation/workflows.md#pipeline-settings) and
  [Personas and approvals](../automation/personas-and-approvals.md#approvals-and-merging).
- **Plugins**: which Jira project, Buildkite pipeline or other service this project's issues and CI live in.
- **Documents**: the [documents](../using-pdt/documents.md) linked to the project, with **Add**, and its own
  **Default provider** and **Local documents folder** (blank: this host's).
- **Danger zone**: **Remove this project**. PDT forgets it and stops its sessions and runs; the files on disk stay.
