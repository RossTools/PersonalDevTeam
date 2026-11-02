# Plugins and connections

A **plugin** connects PDT to another service: GitHub, Jira, Buildkite, Discord and others. Turning one on adds its
workflow steps and triggers, templates, tools for the PM and agents, and project settings. Turning it off removes them
and keeps its settings, connections and workflows for later.

A **connection** is how PDT reaches one account or server of that service: its address and a secret (a token, an API
key, a webhook URL). A plugin can have several connections, say two Jira sites.

## The Plugins page

Open **Settings › Plugins**. Each plugin shows its status:

- **Built in**: always there. GitHub, incoming webhooks, HTTP services and MCP servers are built in.
- **On** or **Off**: an optional plugin, with a switch.
- **Needs setup**: on, but with no connection yet.
- **Error**: one of its connections stopped working (a revoked token, the service down).

Search by name, or filter with **All**, **On**, **Needs attention** and the categories. **Cards** and **List** change
the layout.

The optional plugins:

| Plugin | Adds |
| --- | --- |
| **Discord** | Post to Discord channels from workflows, let the PM manage a Discord server through a bot, and talk to the PM in the bot's DMs ([Talk to it from Discord](the-pm.md#talk-to-it-from-discord)). |
| **Jira** | Link projects to Jira Cloud or Server / Data Center: issues assigned to you become tickets, any issue can be [linked to a ticket](../using-pdt/tickets-and-the-board.md#issues-in-github-gitlab-and-jira), and moving a ticket to Done moves its issue. |
| **Buildkite** | CI of tickets' branches from Buildkite pipelines, shown and acted on like the pull request's CI. |
| **Google** | Google Docs and Sheets as documents: pick files from your Drive, make new ones, and let agents edit them and answer their comments ([Google Docs and Sheets](../using-pdt/documents.md#google-docs-and-sheets)). |
| **Sonarr and Radarr** | Search Radarr and Sonarr, add films and series, and read their download queues from workflows. |
| **Scryer** | Films, series and anime from a Scryer media manager, in workflows and for the PM. |
| **Plex** | Search a Plex server, list what it added lately and refresh its libraries from workflows. |

**GitHub** is built in but has a switch, for a machine whose projects don't use GitHub. It can't be turned off while a
project's pull requests are on GitHub; its page says which.

## Turn on a plugin

1. In **Settings › Plugins**, turn on the plugin's switch.
2. If it needs a connection, a dialog opens. Fill in its address, a token and any settings of its own, and save.
   **Later** closes it; the plugin stays in **Needs setup** until you add one.
3. For a plugin that works per project (Jira, Buildkite), open the project's page in **Settings** and fill in its part
   under **Plugins**: which connection, and what it needs there (a Jira search, Buildkite pipelines).

Click a plugin to open its page:

- its status, with **Add a connection** while it needs setup, or the error with **Test again** and **Edit
  connection**;
- **Connections**: add, edit and delete them;
- **What it adds**: its **Workflow steps**, **Agent and PM tools**, **Templates** and **Project settings** (with the
  projects that use them);
- **Setup**: how to get what it needs from the service.

## Connections and secrets

A connection's secret goes into your machine's keychain and is never shown again. When you edit a connection, leave
the secret empty to keep the one it has. Agents and workflows never see it: PDT adds it to each request as it sends
it, and only to that connection's address.

The dialog says where each kind's secret comes from. A Discord webhook, for example: in Discord, the channel's
settings › **Integrations** › **Webhooks** › **New Webhook** › **Copy Webhook URL**.

**Settings › All connections** lists every connection, with whether it's working; click a row to open its plugin's
page. When a connection starts failing, PDT tells you once in the Inbox's **Updates**.

Secrets that aren't a connection's (a password a command needs, a token for an agent's script) go in
[Secrets](../your-machine/secrets.md).

### Built-in kinds of connection

- **GitHub**: your GitHub login, for pull requests, CI, issues and merges. See
  [GitHub and other forges](../your-machine/github.md).
- **Incoming webhook**: an address another service can call to start a workflow (its row shows the address). Give it
  a secret (**Generate** makes one); callers send it as a bearer token, or sign with it as GitHub's webhooks do.
- **HTTP service**: any web service, by its address and token, for the **HTTP request** step.
- **MCP server**: a program or address that speaks the Model Context Protocol. Its row lists the tools it offers, and
  each one becomes a workflow step.

## Use a plugin in a workflow

With a plugin on, its steps and triggers are in the workflow builder's **Add node**, and its templates in **New
workflow**. A step that needs a connection has a picker for it in its panel (**Add…** makes a new one); a template
asks for its connection when you create a workflow from it.

For example, to post new GitHub issues to Discord:

1. Turn on **Discord** and add a **Discord webhook** connection for the channel.
2. In **Workflows**, **New workflow** › **New GitHub issues to Discord**.
3. Fill in **Repository** (`owner/name`), pick the **Discord channel** (the connection), click **Create workflow**,
   and turn it on.

Or, with a **Discord bot** connection, ask the PM: it can make the channel and its webhook and save the webhook as a
connection.

See [Workflows](workflows.md) for building your own.
