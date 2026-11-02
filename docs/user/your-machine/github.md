# GitHub and other forges

Sign PDT in to GitHub, so it can follow your pull requests and CI, merge for you and let agents read CI logs.

A *forge* is where your repositories live and pull requests happen. PDT works with GitHub (github.com and GitHub
Enterprise Server). A project on anything else works too, without pull requests: PDT merges its branches locally.

## Sign in

Open **Settings › Plugins › GitHub** (or the **Connect GitHub** step in **Get started**).

**With a code** (the easiest way, on github.com):

1. Click **Connect GitHub**.
2. PDT shows a code and a link. Open the link, sign in to GitHub if asked, enter the code and authorize PDT.
3. The page updates by itself and shows **Connected** with your GitHub login.

**With a token** you make yourself:

1. On GitHub, create a token: a classic token with the `repo` scope, or a fine-grained token with read and write on
   **Pull requests**, **Issues** and **Contents**, and read on **Actions** and **Metadata**, for the repositories PDT
   works on. (If you use the GitHub CLI, `gh auth token` prints the token it has.)
2. Paste it under **Or paste a token** and click **Test**: PDT says who it signs in as and its scopes.
3. Click **Save login token**.

Either way the login goes into your system's keychain. Agents never see it, and it never goes into PDT's logs.

### Private repositories of an organization

An organization can block apps it hasn't approved. If PDT is signed in but can't see an organization's private
repositories, ask an owner to approve **Personal Dev Team** in the organization's **Settings › Third-party access ›
OAuth app policy** (GitHub offers to send the request when you authorize PDT). A token you made yourself doesn't need
this.

## GitHub Enterprise Server

1. In **Settings › Plugins › GitHub**, enter your server's API URL under **Host**, like
   `https://github.example.com/api/v3`.
2. Paste a token from that server and click **Save login token**.

To sign in with a code there instead, your server needs an OAuth App of its own with device flow turned on: enter its
client ID under **OAuth App client ID** and click **Connect GitHub**.

You can be signed in to github.com and any number of Enterprise servers at once. Each project uses the login for the
host its `origin` remote is on; a project on a host PDT has no login for works as if it weren't on GitHub.

## What PDT does on your repositories

With a login, PDT:

- follows each ticket's pull request: its state, reviews, CI runs and whether it merged;
- opens pull requests for agents, and lets them read CI's failed logs and reply to review comments;
- merges when you (or a workflow you turned on) say so: through the merge queue or auto-merge when the repository
  uses them, otherwise itself, once CI passes;
- reads, comments on and files issues, when you or a workflow ask it to, and shows the title and state of issues
  [linked to tickets](../using-pdt/tickets-and-the-board.md#issues-in-github-gitlab-and-jira);
- reruns CI or starts a GitHub Actions workflow, when a workflow asks it to.

Agents push their branches with plain `git`, using your own git setup, not PDT's login. Everything else on GitHub goes
through PDT's tools as you, and agents ask before filing an issue.

## Rate limits

GitHub limits how many requests an account makes an hour, and many projects with busy CI can use the limit up. When
that happens, PDT stops calling GitHub until the time GitHub gives, and its errors say **GitHub's rate limit is used up
until HH:MM UTC**. Pull request and CI status catch up by themselves afterwards.

## When GitHub stops accepting the login

If you revoke PDT on GitHub, the token expires or an organization blocks it, PDT stops calling GitHub with it and:

- **Settings › Plugins › GitHub** says **GitHub on github.com no longer accepts this login**;
- the Inbox shows **Reconnect GitHub** under **Needs you**.

Click **Reconnect** (or paste a new token). PDT checks pull requests and issues again straight away.

## Disconnect

**Disconnect…** beside a login removes it from your keychain. PDT stops following pull requests and CI on that host
until you connect again. PDT stays authorized on GitHub until you revoke it there, under **Settings › Applications**.

## Not on GitHub

Click **Skip, my projects aren't on GitHub** in **Get started**, or just don't sign in. Everything but pull requests
works the same: when you merge a ticket, PDT merges its branch into your base branch locally. See
[Run configuration](../automation/run-configuration.md).

If no project uses GitHub, you can turn the GitHub plugin off in **Settings › Plugins**. PDT still reads GitHub for
its own updates.
