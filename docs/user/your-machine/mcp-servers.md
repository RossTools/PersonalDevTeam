# MCP servers

Add an MCP server once in PDT, and every agent gets it: Claude Code, Codex and the other agents, in conversations
and terminals.

An MCP server gives agents more tools: search your Slack, read your notes, query a database. Each agent normally keeps
its own list of servers, and a remote server that signs in has to be signed in to once per agent. In PDT you add the
server once, sign in once, and decide which tools agents may use without asking.

## Add a server

1. Open **Settings › MCP servers** and click **Add server**.
2. Give it a **Name**. Agents see a short form of it (Slack's tools become `mcp__slack__search` in Claude Code).
3. Under **Runs as**, pick how it runs:
   - **A program**: a server that runs on this machine. Fill in the **Program** (`npx`, or a full path), its
     **Arguments** (one per line), its **Environment** and, if it needs one, a **Working folder**.
   - **An address**: a remote server. Fill in its **Address** and any **Headers** it needs.
   - **HTTP+SSE**: an older kind of remote server. Agents that can connect to it get it; PDT can't **Test** it.
4. Under **Who gets it**, untick anyone who shouldn't have it: **The PM**, **Ticket and workflow agents**, **Sessions
   you start**. **Only these personas**, **Only these projects** and **Turned off in** narrow it further.
5. Pick what **Its tools** do by default: **Allow**, **Ask** or **Block**.
6. **Save**.

New sessions get the server; sessions already running don't until they restart.

## Keys and tokens

A header or environment value can be **Text** or a **Secret**: pick **Secret** and choose one of your secrets (see
[Secrets](secrets.md)). PDT fills in the value only in each session's launch settings, in a file only you can read. It
never shows it, and each use appears in the secret's **Uses**. A secret limited to some projects or personas is only
filled in for those sessions. A session it can't be filled in for starts without the server.

## Sign in with OAuth

Many remote servers sign you in with your account on the service instead of a key.

1. Edit the server and tick **Signs in with OAuth**. Leave **Client id** empty unless the service makes you register an
   app first; then enter that app's client id (and its **Client secret**, if it has one).
2. **Save**, then click **Sign in** on the server's row.
3. Sign in to the service in the tab that opens. It comes back to PDT and says you're signed in.

The row then says who you're signed in as. PDT keeps the sign-in fresh on its own and gives every agent session a
token, so no agent has to sign in. If the service stops accepting it (you revoked PDT's access, or it expired), the row
says **Sign-in failed**, your Inbox gets **Sign in to … again**, and new sessions start without the server until you
click **Sign in again**. **Sign out** forgets the sign-in.

PDT only signs in through secure (https) addresses, or plain http on your own computer. If a server's sign-in uses
anything else, **Sign in** says so and opens nothing.

A session gets the token that's current when it starts. A session that outlives its token (rare: most services keep a
token for hours) loses the server's tools until it restarts.

## Allow, ask or block each tool

Click **Test** on a server's row. PDT connects to it and lists its tools (and its resources and prompts, if it has
any), or says what went wrong. Set each tool to:

- **Allow**: the agent uses it without asking;
- **Ask**: the agent asks you first, in its usual prompt (or your Inbox). A session that runs without asking runs it;
- **Block**: the agent can't use it.

Claude Code and Codex block tools. With **Block** as a server's default, Claude Code gets the server once its tools
have been listed (by **Test**, or by PDT on its own within a minute or so), so that every tool it doesn't allow stays
blocked; a tool the server adds later asks until PDT lists the tools again. Other agents can't block a single tool: they say so when the session starts, and
the tool stays available to them. A server whose every tool is blocked isn't given to any agent.

## Example: search Slack, never post

1. **Add server**, name it **Slack**, **Runs as** **An address**, **Address** `https://mcp.slack.com/mcp`.
2. Tick **Signs in with OAuth**. If Slack asks for an app, create one in Slack's app settings and enter its client id
   and secret.
3. **Save**, **Sign in**, and sign in to your Slack workspace.
4. **Test**, then set the search tools to **Allow** and the tools that post or send messages to **Block**.

Every agent can now search Slack without asking, and none can post.

## Who never gets them

Sessions that read outside input are locked down and get none of these servers: triage of a new issue, and work on a
pull request from outside you haven't let in yet. See [Outside contributions](../automation/outside-contributions.md).

What a server's tools return (messages, documents) can hold text other people wrote. Agents are told to read it as
information, never as instructions.

## Agents' own MCP servers

The servers you added to an agent yourself still load beside PDT's. To stop that, turn on **Use only PDT's MCP
servers**: Claude Code and Codex then ignore their own servers in PDT's sessions. Other agents can't be told to, so
theirs still load.

## Workflows

Workflows can call the same servers' tools: each tool is an action in the workflow catalog. A server with **Agents**
unticked under **Who gets it** is for workflows only, and its row says **Workflows only**: tick **Agents** to give it to
your agents too. A server added under **Settings › Plugins** or **All connections** starts that way. See [Workflows](../automation/workflows.md).
