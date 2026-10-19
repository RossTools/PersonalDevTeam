# Run configuration

How PDT runs and tests a project: what **Run** and **Test** on a branch or worktree do. Design: remote-host spec §4
(docs/superpowers/specs/2026-10-15-remote-host-design.md). Code: crates/pdt-core/src/runconfig.rs.

To have an agent write the configuration for you, paste [prompts/set-up-project.md](prompts/set-up-project.md) into
an agent session started from PDT in that project.

## Where it lives

A project's configuration comes from two places, merged:

- **`.pdt/project.toml`** in the checkout being run (so each branch can have its own). Commit it when the repo is
  yours and everyone working on it uses PDT.
- **Host overrides**: the same TOML, stored in PDT for this project only, in the project's Settings page. Nothing goes
  in the repo, so this is the way to configure repos you don't own.

Overrides are merged over the file entry by entry, matched by `name`: a field set in the override wins, `env` tables
are combined, and an entry with a new name is added. Overrides can supply the whole configuration on their own.

## Format

```toml
[[run]]
name    = "web"                                   # required; also a folder name (no / \ : < > | " ? *)
setup   = "pnpm install"                          # optional; runs before every start, so keep it quick when nothing changed
command = "pnpm dev --port $PORT --host 127.0.0.1" # required
web     = true                                    # PDT picks $PORT and shows the link once the port answers
data    = true                                    # PDT provides $PDT_RUN_DATA, a folder kept per project, branch and run
env     = { DATABASE_URL = "sqlite://$PDT_RUN_DATA/app.db" }

[[test]]
name    = "unit"
command = "pnpm test"                             # passes when it exits 0; tests cannot be `web`
```

Unknown keys inside an entry are refused, and so are duplicate names, an entry without a command, and env names that
are not `[A-Za-z_][A-Za-z0-9_]*`. A top-level section PDT does not know (say, one added by a newer PDT) is skipped
instead: it is logged when a run reads the file, listed in `effective.ignored`, and noted in the project's settings,
so a file written for a newer PDT still runs on an older one. Host overrides are written for this PDT, so saving them
with an unknown section is refused.

### Areas

The same file names the parts of the project, so PDT can warn when two agents would change the same code (tickets
spec §7). A ticket lists the areas it expects to touch; Start warns when another active ticket shares one, and PDT
flags files changed by two active branches.

```toml
[[area]]
name      = "migrations"                          # required; 1–60 characters, no commas
paths     = ["crates/pdt-store/migrations/**"]    # required; globs from the checkout root: * stays in a folder, ** crosses
exclusive = true                                  # only one active ticket at a time (sharing it is a conflict)
```

Areas are read from the project's **main checkout** (not the branch being worked on) plus the host overrides, merged
by name like runs. An invalid area makes the whole configuration invalid, as a bad run does.

## What a run gets

- **Working folder:** the checkout (the worktree, or the main checkout). Relative paths start there.
- **Shell:** each step runs with `sh -c` on macOS and Linux and in a generated `.cmd` on Windows, with the host's
  environment (the same toolchains as agent terminals). Setup and command run in PDT terminals, so their output
  shows in the run's tab and survives reloads.
- **Variables:**

  | Variable | Value |
  |---|---|
  | `PORT` | the run's port (`web = true` only), from the host's range (stable 8400–8499, dev 8500–8599) |
  | `PDT_RUN_DATA` | the run's data folder (`data = true` only); Reset data empties it |
  | `PDT_RUN_URL` | the run's link |
  | `PDT_RUN_HOST` | the tailnet DNS name when remote access is on, else `127.0.0.1` |
  | `PDT_RUN_BRANCH` | the branch |
  | `PDT_OWNER_LOGIN` | the host owner's Tailscale login, when set |

- **Expansion:** PDT replaces `$NAME` and `${NAME}` for its own variables only, in `setup`, `command` and `env`.
  Anything else (`$HOME`, `%PATH%`) is left for the shell.
- **Ready:** a web run is ready once something answers on `127.0.0.1:$PORT` or `[::1]:$PORT`; a non-web run once its
  command has started.

## Traps

- **Use `$PORT`.** A web run whose server listens on its own fixed port never becomes ready, and two branches would
  clash. Pass the port by flag or env var, whichever the framework reads.
- **Allow the run's host.** With remote access on, the link is `https://$PDT_RUN_HOST:$PORT/`. Dev servers that check
  the Host header (Vite's `server.allowedHosts`, Django's `ALLOWED_HOSTS`, webpack-dev-server's `allowedHosts`) must
  accept `$PDT_RUN_HOST`, or the page loads locally but is refused from the phone.
- **Unset variables stay literal.** `env = { X = "$SOMETHING" }` passes the text `$SOMETHING` when PDT doesn't set
  it. Leave out a variable rather than forwarding one that may be unset.
- **No secrets in the file.** Put them in overrides (they stay on the host) or have the app read them from the
  environment the host already has.
- **Keep state in `$PDT_RUN_DATA`.** Databases, uploads and caches the app writes belong there, so branches don't
  share them and Reset data works.
- **Windows:** `&&` between steps works, `||`, `;` and `|` are not split, a first word that is a path needs
  backslashes (prefer a command on PATH, like `cargo run -p x`), and `%` is expanded twice.
  Details: docs/superpowers/notes/phase-2-carryover.md.

## API

The address of PDT is `127.0.0.1:7420` for the stable instance. Agent sessions started by PDT have
`PDT_HOOK_PORT` (that PDT's port) and `PDT_SESSION_ID`; `GET /api/sessions/$PDT_SESSION_ID` gives the session's
`projectId` and `worktreeId`.

| Call | Does |
|---|---|
| `GET /api/projects` | projects (`id`, `repoPath`, …) |
| `GET /api/projects/{id}/run-config[?worktreeId=…]` | `file`, `overrides`, the merged `effective` config (`runs`, `tests`, `areas`, `ignored` sections), or `error` |
| `GET /api/projects/{id}/areas` | `{ areas: [{ name, paths, exclusive }], error }` from the main checkout and the overrides |
| `GET /api/projects/{id}/settings` | `{ agentCredentials, effectiveCredentials, runOverrides }` |
| `PUT /api/projects/{id}/settings` | `{ agentCredentials, runOverrides }`; replaces **both**, so send back the `agentCredentials` you read. An empty `runOverrides` clears them. Text that isn't valid TOML, or has an unknown section, is refused (`invalid_run_config`); check `run-config`'s `error` for the rest |
| `POST /api/projects/{id}/runs` | `{ name, kind: "run" \| "test", worktreeId? }`; without `worktreeId` (or `branch`) it uses the main checkout. Opens a tab in the project's workspace |
| `GET /api/runs/{id}` | `state` (`setting_up`, `starting`, `ready`, `running`, `passed`, `failed`, `stopped`), `url`, `exitCode`, `error` |
| `POST /api/runs/{id}/stop` · `/restart` · `/reset-data` | as named |
