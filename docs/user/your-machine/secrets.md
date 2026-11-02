# Secrets

Give agents and workflows the keys and tokens they need without them ever seeing the values.

An agent sometimes needs an API key, a token or a password: to publish a package, call a service, or log in while it
tests. Pasted into the conversation, it would end up in the transcript, the logs and the model's context. A
**secret** is kept in your machine's keychain instead: agents and workflows use it by name, and PDT fills in the value
only where it's needed.

## Add a secret

1. Open **Settings › Secrets** and click **Add secret**.
2. Give it a **Name** (letters, digits, `_` and `-`, starting with a letter: `NPM_TOKEN`).
3. Write a **Description**: what it's for. Agents see this, never the value.
4. Paste the **Value** (at least 6 characters).
5. Under **Projects** and **Agents**, tick who may use it, or leave **Every project** and **Every agent**. "Agents"
   here are personas (the Coder, the Reviewer…); **The PM (personal workflows)** is the PM and your workflows that
   belong to no project.
6. **Save**.

The value is never shown again, to you or anyone. **Edit** changes the description and who may use it, or replaces the
value (leave **Value** empty to keep it). **Delete** removes the secret and its value from the keychain.

## When an agent asks for one

An agent that needs a secret you haven't added asks for it: a question in your Inbox, with what it's for and a masked
field. Type or paste the value and send it. It becomes a secret for that project, and the agent is told it's stored,
never what it is. See [The Inbox](../using-pdt/the-inbox.md).

## How agents use them

Agents never get the value. PDT gives them three tools instead:

- **list secrets**: the names and descriptions of the secrets they may use;
- **run with secrets**: runs a command with secrets in its environment (say, `NPM_TOKEN` set to the secret's value).
  The command's output comes back with any secret value replaced by `[secret:NAME]`;
- **write a secret file**: writes a file from a template with the secrets filled in (a `.npmrc` or `.env`, say),
  readable only by you. The agent gets back where it wrote it and how big it is, not its content.

Running a command or writing a file this way happens in PDT, outside the agent's own sandbox, so PDT asks you first:
the agent's permission prompt, or an **Allow** / **Deny** item in your Inbox for agents that don't ask themselves. It
doesn't ask a session you started with permissions bypassed. Either way, the command must use a secret, and it runs
and writes only in the session's own folder: never in `.git`, and never through a link to somewhere else.

Agents reading outside input (triaging a new issue, or working on a ticket from outside you haven't let in yet) can't
use secrets at all. See [Outside contributions](../automation/outside-contributions.md).

## How workflows use them

In a workflow step, write `{{secret.NAME}}` where the value goes (a header, a URL, a request body). PDT fills it in
only as the step runs; the step's record keeps the reference, not the value. Only references written in the step
itself are filled in: one that arrives in data (an issue's text, say) is passed on as written. Agent steps can't take
secrets this way; the agent uses its tools instead. See [Workflows](../automation/workflows.md#steps).

## What's kept out

PDT replaces every secret's value with `[secret:NAME]` everywhere it stores or shows text: conversations, ticket
comments, workflow runs, the Inbox, notifications, its own logs, and bug reports' log excerpts.

PDT can't clean what it doesn't hold: the agent's own transcript files, a terminal session's scrollback, and what a
command does with the value once it has it. A shell's last output is also kept in PDT's data folder, readable only by
you, so it can be shown again after PDT restarts; it is deleted with the session. Agents are told never to print a
secret; still, give an agent only the secrets its work needs.

## Who used a secret

Each secret shows when it was last used and by whom. **Uses** lists every use: which session or workflow, and how
(which tool or step).

## Where values are kept

- **macOS**: the login keychain.
- **Windows**: Credential Manager.
- **Linux**: the Secret Service (GNOME Keyring or KWallet) when there is one. On a machine without one (a server with
  no desktop session, usually), PDT keeps values in a file in its data folder that only you can read. The top of
  **Settings › Secrets** says which: **Secrets: system keyring** or **Secrets: file in the data folder**, with its
  path. PDT keeps to the one it chose the first time.

PDT's GitHub login and its update token are kept the same way. None of them is in PDT's database, so a copy of its
data folder holds no secrets (except the Linux file above).
