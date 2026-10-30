# Settings

What each section of Settings changes, for this machine and for each project.

Open **Settings** in the rail (on a phone, in the **☰** menu). The list on the left has three parts:
**This host** (the machine running PDT), a few pages of their own (**Notifications**, **Plugins**, **All
connections**, **Secrets**), and each project. **Search settings** at the top finds a section by its name or any
setting on it.

Every setting saves as you change it: a moment of **Saved** beside it says so. Tokens and secrets have their own
**Save** button.

## This host

### Updates

How PDT updates itself. See [Install and updates](install-and-updates.md#updates) for what each choice does.

- **Version**: the version you run, with **Check for updates**, and what the last check found.
- **Install updates**: **Install automatically when idle**, **Notify only** or **Manual (no checks)**.
- **Update channel**: **Stable releases**, or **Nightly builds too**.
- **Check for updates**: how often PDT looks.
- **Release repository**: where updates come from. Leave it empty; it's there in case PDT's releases ever move
  somewhere PDT can't follow.
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
- **Default models**: the model each agent starts with, unless the session or its persona picks another.
- **Programs**: where each agent's program is, a switch to turn the agent off, and whether PDT found it. Leave a path
  empty to use the one on your `PATH`.
- **ACP agents**: other agents that speak the Agent Client Protocol. See
  [Connect agents and GitHub](../getting-started/connect.md#other-agents-acp).

### Network

Where PDT listens. Changes apply when PDT restarts: a banner offers **Restart now**, and **Network** shows a
**restart** badge until then. Sessions are interrupted and resume when PDT is back.

- **Port**: PDT's port, 7420 unless you change it. Change it if something else needs 7420. After a restart on a new
  port, the page reopens itself there.
- **Allowed hosts**: other names you open PDT by on this machine (an alias in your hosts file, say), with `:port` when
  needed. `localhost` and your tailnet name are always allowed.

A setting marked **env** is set by an environment variable PDT was started with (`PDT_PORT`, `PDT_ALLOWED_HOSTS`),
which wins over the setting, so you can't change it here. See
[Troubleshooting](../troubleshooting/troubleshooting.md#the-port-is-in-use-or-the-field-is-locked).

### Remote access

Reach PDT from your phone over Tailscale: **Serve on the tailnet**, its **Status** and address, and the **Owner**,
the only Tailscale user let in. See [Remote access and your phone](remote-access.md).

### Runs

- **Ports**: the range runs (dev servers, tests) get their ports from.
- **Max concurrent runs**: how many runs may run at once.

See [Run configuration](../automation/run-configuration.md).

### Projects folder

Where a repository you add by **Clone URL** is cloned to (`~/work` unless you change it).

### Appearance

**Theme**: **System**, **Dark** or **Light**. It's kept in this browser, so your phone can differ from your computer.

### Bug reports

Where **Report a bug…** sends a report about PDT: **Automatic** (a GitHub issue on PDT's repository), **GitHub issue**,
**Local ticket** (a draft ticket here, nothing sent anywhere) or **Off**. The report form shows what it includes
before you send it, and logs are cleaned of secrets, tokens and your machine's names.

### Checks

Whether PDT found the programs it runs: git, the agents and Tailscale, with where it found them or what's wrong.
**Run checks again** after installing something.

### Setup

**Open Get started** brings back the first-run checklist.

### Danger zone

**Remove the GitHub token** deletes the update token from your keychain. It always asks first.

## Notifications

How each kind of news reaches you: off, a dot, the Inbox, a toast, or a desktop notification, and **Desktop
notifications** for this device. See [The Inbox](../using-pdt/the-inbox.md#notifications).

## Plugins and All connections

Turn PDT's plugins on and off (GitHub, Jira, Buildkite, Discord and others), and manage their connections. The GitHub
plugin is also where you sign in to GitHub. See [Plugins and connections](../automation/plugins.md) and
[GitHub and other forges](github.md).

## Secrets

Keys and tokens agents and workflows can use without seeing them. See [Secrets](secrets.md).

## A project's settings

Each project has its own sections. A personal project (one without a repository) has fewer.

- **General**: its **Name**, its **Key** (the letters in its ticket numbers), the **Base branch** work starts from,
  and its **Repository folder**.
- **Worktrees**: the checkouts PDT made for its branches. Remove one you no longer need, or open its changes.
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
- **Danger zone**: **Remove this project**. PDT forgets it and stops its sessions and runs; the files on disk stay.
