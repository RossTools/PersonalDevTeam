# Connect agents and GitHub

PDT doesn't bring its own AI: it runs the coding agents you install, logged in as you. It needs git and at least one
agent. GitHub is optional.

## Claude Code, Codex and Devin

1. Install the agent with its own instructions: [Claude Code](https://code.claude.com/docs/en/setup),
   [Codex](https://developers.openai.com/codex/cli) or [Devin](https://cli.devin.ai).
2. Open a terminal and run it once (`claude`, `codex` or `devin`), and log in when it asks. PDT uses that login.
3. In **Get started**, open **Agents on this machine** and click **Check again**. The agent shows as **Found**, with
   the path PDT will run.

If you installed an agent somewhere PDT doesn't look, click **Set a path by hand…**, pick the agent, enter the full
path to its program and click **Save path**. The same setting is in **Settings › This host › Agents › Programs**.

You can install more than one. Each ticket can use a different agent, and you choose the default in **Pick a default
agent**.

## Other agents (ACP)

PDT can also run any agent that speaks the Agent Client Protocol (ACP). OhMyPi is built in: install `omp`, set up its
model in omp, and PDT finds it.

Google Antigravity is built in too, through Google's own ACP server. In **Settings › This host › Agents › ACP agents**,
click **Install** beside Antigravity. PDT downloads the server from Google and checks it against the ACP registry
before installing it. Then click **Sign in**: your browser opens to sign in with your Google account. This sign-in is
the server's own: signing in to the `agy` CLI doesn't count. Click **Update** later to get a newer version.

To add another:

1. Open **Settings › This host › Agents › ACP agents** and click **Add ACP agent**.
2. Give it a name, the command that starts it in ACP mode, its arguments (one per line) and any environment variables
   it needs (`NAME=value`, one per line).
3. Save. PDT shows whether it found the command.

ACP agents run as conversations only (not in a terminal). Once added, they're offered wherever you pick an agent.

## Don't need an agent?

You can turn an agent off in **Settings › This host › Agents**, so PDT stops offering it and doesn't count it as
missing.

## GitHub

Connect GitHub if your projects are on it. PDT then follows each ticket's pull request and CI, can merge for you, and
tells the agent when CI fails.

1. In **Get started**, open **Connect GitHub** and click **Connect GitHub**.
2. PDT shows a code and a link. Open the link, sign in to GitHub if asked, enter the code and authorize PDT.
3. Back in PDT, the step shows **Connected as** your login.

PDT keeps the token in your system's keychain and never shows it to agents. It asks for the `repo` scope, which
covers reading and merging pull requests, CI runs and issues in your public and private repositories.

### Other ways to connect

In **Settings › Plugins › GitHub** you can instead paste a token (a classic token with `repo`, or a fine-grained one
with read and write on pull requests, issues and contents, and read on actions and metadata). If you use the GitHub
CLI, `gh auth token` prints the one it has. Click **Test** to see who it signs in as, then **Save login token**.

For **GitHub Enterprise Server**, enter your server's API URL (like `https://github.example.com/api/v3`) with a
token. You can be connected to github.com and Enterprise servers at the same time; each project uses the account for
the host its `origin` remote is on.

### Not on GitHub?

Click **Skip, my projects aren't on GitHub**. Everything else works the same; when you merge a ticket, PDT merges its
branch into your base branch locally.

## Next

[Add a first project](first-project.md).
