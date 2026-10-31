# Run configuration

Tell PDT how to run and test your project, and any branch can start on its own port with one click: a dev server for
the ticket an agent just finished, next to the one on `main`, without them getting in each other's way.

A project's run configuration lists its **runs** (dev servers, apps), its **tests**, its **areas** (the parts of the
code that belong together) and, optionally, its **forge** and **CI**. To start a run, see **Runs, dev servers and
tests** in Workspaces and sessions.

## Let an agent write it

Open **Settings**, the project's page, and under **Runs on this host** click **Set up with an agent**. Pick an agent:
it starts a session in the project that looks at how the project is built and run, and writes the configuration for
you.

## Where it lives

The configuration comes from two places, put together:

- **`.pdt/project.toml`** in the checkout being run. Each branch can have its own, so a branch can change how it
  runs. Commit it when everyone working on the repository uses PDT.
- **Host overrides**, in the project's **Settings** under **Runs on this host**: the same format, kept on this machine
  only. Use them for a repository that isn't yours, or for what shouldn't be committed (a local path, a secret).

An override with the same `name` as an entry in the file changes only the fields it sets (`env` tables are combined);
a new name adds an entry. Overrides can also be the whole configuration on their own. **Repository configuration** on
the same page shows what the file says.

A file with a mistake (an unknown field in an entry, two entries with the same name, an entry with no command, an
environment variable name that isn't letters, digits and `_`) doesn't run at all, and the project's **Settings** say
what's wrong. A whole section PDT doesn't know (one added by a newer PDT) is skipped and noted in **Settings**, so the
rest still runs.

## Runs

```toml
[[run]]
name    = "web"
setup   = "pnpm install"
command = "pnpm dev --port $PORT --host 127.0.0.1"
web     = true
data    = true
env     = { DATABASE_URL = "sqlite://$PDT_RUN_DATA/app.db" }
```

- **`name`** (required): what the run is called in the **+** menu. It is also a folder name, so no `/ \ : < > | " ? *`.
- **`command`** (required): what starts it, in the checkout's folder.
- **`setup`**: a step before every start, such as installing dependencies. It runs each time, so keep it quick when
  nothing changed.
- **`web = true`**: it's a web server. PDT picks a free port, passes it as `$PORT`, and shows the run's link once
  something answers on it.
- **`data = true`**: PDT gives the run a folder of its own as `$PDT_RUN_DATA`, kept per branch. **Reset data…** on the
  run empties it.
- **`env`**: environment variables for the run.
- **`detach = true`**: keeps it running when PDT restarts or updates (below).

A project can have as many runs as it needs: a web app and its API, say, each with its own port.

### What a run gets

Each step runs in the checkout's folder with your machine's environment (the same tools as your terminal), through
`sh -c` on macOS and Linux and a `.cmd` file on Windows. Its output shows in the run's tab.

| Variable | Value |
| --- | --- |
| `$PORT` | the run's port (web runs only), from this machine's range (8400–8499; 8500–8599 on a dev instance) |
| `$PDT_RUN_DATA` | the run's data folder (with `data = true`) |
| `$PDT_RUN_URL` | the run's link |
| `$PDT_RUN_HOST` | the address it's reached at: your machine's name on your tailnet when remote access is on, else `127.0.0.1` |
| `$PDT_RUN_BRANCH` | the branch |
| `$PDT_OWNER_LOGIN` | your Tailscale login, when remote access has one |

PDT fills in these variables (as `$NAME` or `${NAME}`) in `setup`, `command` and `env`; anything else, such as
`$HOME`, is left to the shell. A web run is ready once something answers on its port (on `127.0.0.1` or `::1`);
another run as soon as its command starts.

### Runs that outlive PDT

A run normally stops when PDT restarts, which includes its automatic updates. For something you keep open all day (a
desktop app you use from the main checkout, say), set `detach = true`:

- `setup` runs as usual; the command starts as a background process, outside any terminal.
- Its output goes to the run's **Output** tab, read-only (it takes no typing), and is cleared on each start.
- The run counts as running while any process it started is, so an app that restarts itself is still tracked.
- After a restart PDT finds it again instead of starting it twice. **Stop**, **Restart**, a deleted branch and
  removing the project end every process it started.

A detached run keeps going through updates until you stop it, so use it only where you need it.

## Tests

```toml
[[test]]
name    = "unit"
command = "pnpm test"
```

A test passes when its command exits with 0. Tests take `setup` and `env` like runs, but not `web` or `detach`. Start
one from **+** in the tab bar, or **Run…** on a ticket's Inbox item.

## Areas

Areas tell PDT which parts of the code belong together, so it can warn you before two tickets change the same thing.

```toml
[[area]]
name      = "migrations"
paths     = ["db/migrations/**"]
exclusive = true

[[area]]
name  = "web"
paths = ["web/**"]
```

- **`name`**: 1–60 characters, no commas.
- **`paths`**: patterns from the repository's root. `*` matches within one folder, `**` across folders.
- **`exclusive = true`**: only one ticket in progress may touch it at a time. Sharing it is a conflict, not just a
  warning.

While a ticket is in progress, another in the same area shows an overlap, and **Start** warns you. PDT also flags
files that two active branches both change. Areas are read from the main checkout (and the host overrides), not from
the branch being worked on.

## Forge

The forge is where a project's pull requests, CI, reviews and issues live. Without a `forge` line, PDT goes by where
the repository's `origin` is: GitHub for github.com and each GitHub Enterprise server you've signed in to, GitLab (with
its plugin on) for gitlab.com and each GitLab server you've signed in to, and none otherwise.

```toml
forge = "github"       # or "gitlab", "local" (below), or "none"
```

`forge = "none"` turns off pull requests, CI and issues for the project; merges happen on your machine. The forge is
read from the main checkout (and the host overrides, which win). A forge whose plugin is off or missing leaves the
choice to `origin`.

### Without GitHub: the local forge

PDT can be the forge itself, for a repository with no remote or one on a plain git server:

```toml
forge = "local"

[ci]
command = "cargo test"  # optional
```

- **Change requests:** a started ticket's branch gets one (**Change #1**) once it has commits, shown wherever a pull
  request would be.
- **CI:** on each new commit, PDT runs the `[ci]` command in the ticket's folder (with `CI=true`); exit 0 passes. A
  newer commit stops the run on the older one, and a run longer than two hours fails. Failures and conflicts reach
  the agent as they would on GitHub.
- **Merging:** **Merge…** merges the branch into its base on your machine once CI passes (a fast-forward when the base
  hasn't moved, else a merge commit). A conflict goes back to the agent. Approval and review rules apply as usual.
- There is no merge queue, issues or forge reviews (PDT's own review still works), and agents are told to commit but
  not push.

To push each merge to a remote too (any git server, a bare repository on a share):

```toml
[push]
remote   = "origin"
branches = true        # optional: push ticket branches as they move, as a backup
```

- PDT merges, pushes the merge to the remote's base, and moves the local base only once the remote has taken it.
- When the remote has moved on, PDT brings its base into the ticket's branch, runs CI again and retries the merge. A
  conflict goes back to the agent.
- What PDT can't fix (the two bases have diverged, uncommitted changes in the worktree, no access) is a notice in the
  Inbox.
- PDT pushes with your own git credentials (ssh keys, a credential helper) and can't answer a password prompt.
  **Runs on this host** shows where merges go.

## CI providers

A project's CI can come from its forge (GitHub Actions, GitLab CI, the local forge's `[ci]` command) and from CI
plugins set up in its settings (Buildkite). PDT reads all of them unless you pick, by id, in order:

```toml
[ci]
providers = ["github_actions", "buildkite"]   # github_actions, gitlab_ci, local, buildkite
```

The pull request's CI badge rolls up every listed provider, each one's failures reach the agent, and an automated
merge waits until each has passed. An empty list means no CI; the host overrides' list replaces the file's.

## Pitfalls

- **Use `$PORT`.** A web server on a fixed port never shows as ready, and two branches would clash. Pass the port by
  flag or environment variable, whichever your framework reads.
- **Let the run's address in.** With remote access on, the link is `https://$PDT_RUN_HOST:$PORT/`. Dev servers that
  check the address they're called by (Vite's `server.allowedHosts`, Django's `ALLOWED_HOSTS`, webpack's
  `allowedHosts`) must accept `$PDT_RUN_HOST`, or the page loads on this machine but not on your phone.
- **Unset variables stay as written.** `env = { X = "$SOMETHING" }` passes the text `$SOMETHING` when PDT doesn't set
  it. Leave such a variable out instead.
- **No secrets in the file.** Put them in the host overrides, or have the app read them from your environment.
- **Keep state in `$PDT_RUN_DATA`**, so branches don't share a database and **Reset data…** works.
- **Windows:** `&&` between commands works, but `||`, `;` and `|` don't; `%` is expanded twice; prefer a command on
  your PATH (`cargo run -p app`) over a path to a program.
