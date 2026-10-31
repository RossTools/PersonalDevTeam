# Connect agents and GitHub

PDT brings no AI of its own: it runs the coding agents you install, logged in as you. It needs git and at least one
agent. GitHub is optional.

## Claude Code, Codex and Devin

1. Install the agent: [Claude Code](https://code.claude.com/docs/en/setup),
   [Codex](https://developers.openai.com/codex/cli) or [Devin](https://cli.devin.ai).
2. Run it once in a terminal (`claude`, `codex` or `devin`) and log in. PDT uses that login.
3. In **Get started**, open **Agents on this machine** and click **Check again**. The agent shows as **Found**, with
   the path PDT will run.

If PDT doesn't find an agent you installed, click **Set a path by hand…**, pick the agent, enter the full path to its
program and click **Save path** (also in **Settings › This host › Agents › Programs**).

You can install several: choose the default in **Pick a default agent**, and another per ticket when you start it.

## Other agents (ACP)

PDT also runs agents that speak the Agent Client Protocol (ACP). These are built in, and PDT finds them once they're
installed and signed in:

- **Grok**: install Grok Build and run `grok login`.
- **Cursor**: install the Cursor CLI and run `cursor-agent login`.
- **OhMyPi**: install `omp` and run `omp login` (or set up its model in omp another way).
- **Antigravity**: in **Settings › This host › Agents › ACP agents**, click **Install** beside it (PDT downloads
  Google's ACP server and verifies it), then **Sign in** to sign in with your Google account in your browser. This
  sign-in is separate from the `agy` CLI's. **Update** gets a newer version.

To add another, open **Settings › This host › Agents › ACP agents**, click **Add ACP agent**, and give it a name,
the command that starts it in ACP mode, its arguments and any environment variables (`NAME=value`), one per line.
Save, and PDT shows whether it found the command.

ACP agents run as conversations only, not in a terminal, and are offered wherever you pick an agent.

To stop PDT offering an agent you don't use (and counting it as missing), turn it off in **Settings › This host ›
Agents**.

## GitHub

Connect GitHub if your projects are on it: PDT then follows each ticket's pull request and CI, can merge for you, and
tells the agent when CI fails.

1. In **Get started**, open **Connect GitHub** and click **Connect GitHub**.
2. Open the link PDT shows, sign in to GitHub if asked, enter the code and authorize PDT.
3. Back in PDT, the step shows **Connected as** your login.

PDT keeps the token in your system's keychain and never shows it to agents. It asks for the `repo` scope (pull
requests, CI runs and issues in your public and private repositories).

### Other ways to connect

In **Settings › Plugins › GitHub** you can paste a token instead: a classic token with `repo`, or a fine-grained one
with read and write on pull requests, issues and contents, and read on actions and metadata (`gh auth token` prints
the GitHub CLI's). Click **Test** to see who it signs in as, then **Save login token**.

For **GitHub Enterprise Server**, enter your server's API URL (like `https://github.example.com/api/v3`) with a
token. You can connect github.com and Enterprise servers at once; each project uses the account for the host its
`origin` remote is on. More in [GitHub and other forges](../your-machine/github.md).

### Not on GitHub?

Click **Skip, my projects aren't on GitHub**. Everything else works the same; merging a ticket merges its branch into
your base branch locally.

## Next

[Add a first project](first-project.md).
