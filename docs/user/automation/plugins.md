# Plugins and connections

A **plugin** connects PDT to another service: GitHub, Jira, Buildkite, Discord and others. Turning one on adds what it
brings: workflow steps and triggers, templates, tools for the PM and agents, and settings for your projects. Turning
it off takes them away again, and keeps its settings, connections and workflows for later.

A **connection** is how PDT reaches one account or server of that service: its address and a secret (a token, an API
key, a webhook URL). A plugin can have several connections, say two Jira sites.

## The Plugins page

Open **Settings › Plugins**. Each plugin shows its status:

- **Built in**: part of PDT, always there. GitHub, incoming webhooks, HTTP services and MCP servers are built in.
- **On** or **Off**: an optional plugin, with a switch.
- **Needs setup**: on, but with no connection yet.
- **Error**: one of its connections stopped working (a revoked token, the service down).

Search by name, or use **All**, **On** and **Needs attention** (the plugins that need setup or have an error), and the
categories. **Cards** and **List** change the layout.

The optional plugins:

| Plugin | Adds |
| --- | --- |
| **Discord** | Post to Discord channels from workflows, and let the PM manage a Discord server through a bot. |
| **Jira** | Link projects to Jira Cloud or Server / Data Center: issues assigned to you become tickets, and moving a ticket to Done moves its issue. |
| **Buildkite** | CI of tickets' branches from Buildkite pipelines, shown and acted on like the pull request's CI. |
| **Sonarr and Radarr** | Search Radarr and Sonarr, add films and series, and read their download queues from workflows. |
| **Scryer** | Films, series and anime from a Scryer media manager, in workflows and for the PM. |
| **Plex** | Search a Plex server, list what it added lately and refresh its libraries from workflows. |

**GitHub** is built in but has a switch, for a machine whose projects don't use GitHub. It can't be turned off while a
project's pull requests are on GitHub: its page says which.

## Turn on a plugin

1. In **Settings › Plugins**, turn on the plugin's switch.
2. If it needs a connection, the dialog to add one opens. Fill in what it asks for (its address, a token, and any
   settings of its own) and save. **Later** closes it; the plugin stays in **Needs setup** until you add one.
3. If it works per project (Jira, Buildkite), link it to a project: open **Settings**, the project's page, and fill
   in the plugin's part under **Plugins** (which connection, and what it needs there: a Jira search, Buildkite
   pipelines).

A plugin's own page (click it) has:

- its status, and what to do about it: **Add a connection** while it needs setup, or the error with **Test again** and
  **Edit connection**;
- **Connections**: add, edit and delete them;
- **What it adds**: its **Workflow steps**, **Agent and PM tools**, **Templates** and **Project settings** (with the
  projects that use them);
- **Setup**: short steps for getting what it needs from the service.

## Connections and secrets

A connection's secret goes straight into your machine's keychain and is never shown again. When you edit a
connection, leave the secret empty to keep the one it has. Neither agents nor workflows see it: PDT adds it to the
request as it sends it, and only to that connection's address.

The dialog says where each kind's secret comes from. A Discord webhook, for example: in Discord, the channel's
settings › **Integrations** › **Webhooks** › **New Webhook** › **Copy Webhook URL**.

**Settings › All connections** lists every connection in one table, with whether it's working. A row opens its
plugin's page. When a connection starts failing, PDT tells you once in the Inbox's **Updates**.

Other secrets (a password a command needs, a token an agent needs for a script) live in **Settings › Secrets**: see
[Secrets](../your-machine/secrets.md).

### Built-in kinds of connection

- **GitHub**: your GitHub login, which PDT uses for pull requests, CI, issues and merges. See
  [Connect agents and GitHub](../getting-started/connect.md#github).
- **Incoming webhook**: an address another service can call to start a workflow (its row shows the address). Give it
  a secret (**Generate** makes one); callers send it as a bearer token, or sign with it as GitHub's webhooks do.
- **HTTP service**: any web service, by its address and token, for the **HTTP request** step.
- **MCP server**: a program or address that speaks the Model Context Protocol. Its row lists the tools it offers, and
  each one becomes a step you can add in a workflow.

## Use a plugin in a workflow

With a plugin on, its steps and triggers are in the workflow builder's **Add node**, and its templates in **New
workflow**. A step that needs a connection has a picker for it in its panel (with **Add…** for a new one); a template
asks for its connection when you create a workflow from it.

For example, to post new GitHub issues to Discord:

1. Turn on **Discord** and add a **Discord webhook** connection for the channel.
2. In **Workflows**, **New workflow** › **New GitHub issues to Discord**.
3. Fill in **Repository** (`owner/name`), pick the **Discord channel** (the connection), **Create workflow**, and
   turn it on.

Or ask the PM: with a **Discord bot** connection, it can make the channel and its webhook for you, and save the
webhook as a connection.

See [Workflows](workflows.md) for building your own.
