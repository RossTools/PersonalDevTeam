# Run configuration

Tell PDT how to run and test your project, and any branch can start on its own port with one click: a dev server for
the ticket an agent just finished, next to the one on `main`, without them getting in each other's way.

A project's run configuration lists its **runs** (dev servers, apps), its **tests**, and its **areas** (the parts of
the code that belong together). How to start a run is in
[Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#runs-dev-servers-and-tests).

## Let an agent write it

The quickest way: open **Settings**, the project's page, and under **Runs on this host** click **Set up with an
agent**. Pick an agent: it starts a session in the project that looks at how
the project is built and run, and writes the configuration for you.

## Where it lives

The configuration comes from two places, put together:

- **`.pdt/project.toml`** in the repository. Each branch can have its own, so a branch can change how it runs.
  Commit it when everyone working on the repository uses PDT.
- **Host overrides**, in the project's **Settings** under **Runs on this host**: the same format, kept on this machine
  only. Use them for a repository that isn't yours, or for what shouldn't be committed (a local path, a secret).

An override with the same `name` as an entry in the file changes only the fields it sets; a new name adds an entry.
Overrides can also be the whole configuration on their own. **Repository configuration** on the same page shows what
the file says.

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

- **`name`** (required): what the run is called in the **+** menu.
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

| Variable | Value |
| --- | --- |
| `$PORT` | the run's port (web runs only) |
| `$PDT_RUN_DATA` | the run's data folder (with `data = true`) |
| `$PDT_RUN_URL` | the run's link |
| `$PDT_RUN_HOST` | the address it's reached at: your machine's name on your tailnet when remote access is on, else `127.0.0.1` |
| `$PDT_RUN_BRANCH` | the branch |

PDT fills in these variables (as `$NAME` or `${NAME}`) in `setup`, `command` and `env`; anything else, such as
`$HOME`, is left to the shell. The run uses the same environment and tools as your terminal.

### Runs that outlive PDT

A run normally stops when PDT restarts, which includes its automatic updates. For something you keep open all day (a
desktop app you use from the main checkout, say), set `detach = true`: it starts as a background process, its output
goes to the run's **Output** tab (read-only: it takes no typing), and PDT finds it again after a restart. **Stop** still
stops it.

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

- **`paths`**: patterns from the repository's root. `*` matches within one folder, `**` across folders.
- **`exclusive = true`**: only one ticket in progress may touch it at a time. Sharing it is a conflict, not just a
  warning.

While a ticket is in progress, another in the same area shows an overlap, and **Start** warns you. PDT also flags
files that two active branches both change. See
[Tickets and the board](../using-pdt/tickets-and-the-board.md#links-and-blocking).

Areas are read from the main checkout (and the host overrides), not from the branch being worked on.

## Without GitHub

PDT can stand in for GitHub for a repository with no remote, or one on a plain git server:

```toml
forge = "local"

[ci]
command = "cargo test"
```

- A started ticket's branch gets a change request (**Change #1**) once it has commits, shown wherever a pull request
  would be.
- On each new commit, PDT runs the `[ci]` command in the ticket's folder; exit 0 passes. Failures and conflicts reach
  the agent as they would on GitHub.
- **Merge…** merges the branch into its base on your machine once CI passes.

To push each merge to a remote too:

```toml
[push]
remote   = "origin"
branches = true        # optional: push ticket branches as they move, as a backup
```

PDT pushes with your own git credentials, and moves the local base only once the remote has taken the merge.

`forge = "none"` turns off pull requests and CI for a project altogether. Without a `forge` line, PDT goes by where
the repository's `origin` is.

## Pitfalls

- **Use `$PORT`.** A web server on a fixed port never shows as ready, and two branches would clash.
- **Let the run's address in.** With remote access on, the link uses your machine's tailnet name. Dev servers that
  check the address they're called by (Vite's `server.allowedHosts`, Django's `ALLOWED_HOSTS`) must accept
  `$PDT_RUN_HOST`, or the page loads on this machine but not on your phone.
- **Unset variables stay as written.** `env = { X = "$SOMETHING" }` passes the text `$SOMETHING` when PDT doesn't set
  it.
- **No secrets in the file.** Put them in the host overrides, or have the app read them from your environment.
- **Keep state in `$PDT_RUN_DATA`**, so branches don't share a database and **Reset data…** works.
- **Windows:** `&&` between commands works, but `||`, `;` and `|` don't; prefer a command on your PATH over a path to
  a program.

A file with a mistake (an unknown field, two entries with the same name, a run with no command) doesn't run at all, and
the project's **Settings** say what's wrong.
