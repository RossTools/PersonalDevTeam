# Prompt: set up a project's runs in PDT

PDT has this prompt built in: **Set up with an agent** in a project's Settings (Runs on this host) starts a session
with it, and `GET /api/guides/set-up-project` serves it. By hand: start a Claude Code or Codex session from PDT in the
project (its main checkout or a worktree), and paste everything below the line. It finds the project through PDT
itself, so nothing needs filling in. The reference it follows is
[../user/automation/run-configuration.md](../user/automation/run-configuration.md) (`GET /api/guides/run-config`).

---

Set up this project so PDT (Personal Dev Team, which started this session) can run and test it: a **Run** that
starts the app for development, and a **Test** for each test suite.

**1. Find PDT and this project.** PDT's instructions for this session name its API (`http://127.0.0.1:<port>/api`)
and this session's id; terminal sessions also have `$PDT_HOOK_PORT` and `$PDT_SESSION_ID`.
`GET /api/sessions/<session id>` gives this session's `projectId` and `worktreeId`. (If you have neither, this session
didn't come from PDT: use port 7420 and match `repoPath` in `GET /api/projects` to the repo root.) The full reference
is `GET /api/guides/run-config`. Read `GET /api/projects/<projectId>/run-config?worktreeId=<worktreeId>` (leave out
`worktreeId` when null): any existing `.pdt/project.toml` (`file`), host overrides (`overrides`), and the merged
`effective` or an `error`. If a working configuration already exists, say so and ask what to change.

**2. Learn how the project runs.** Read the README and contributing docs, and the build files: package.json scripts,
Cargo.toml, pyproject/requirements, Makefile, justfile, docker-compose, Procfile, `.env.example`, CI workflows (CI
shows the real install and test commands). Work out:
- the install or build step needed before starting (this becomes `setup`, which runs before every start, so it must
  be quick when nothing changed: `pnpm install --frozen-lockfile`, not a clean reinstall);
- the dev server command, and how to give it a port and a listen address (flag or env var);
- whether it checks the Host header (Vite, Django, webpack-dev-server, Rails …) and how to allow another host;
- what state it writes (database file, uploads, cache) and whether that can be pointed at a folder;
- each test suite's command;
- services it needs that PDT doesn't start (Postgres, Redis …). Don't start or install them; note them for me.

**3. Write the configuration.** The format:

```toml
[[run]]
name    = "app"                     # a folder name too: no / \ : < > | " ? *
setup   = "..."                     # optional
command = "..."                     # must listen on $PORT
web     = true                      # PDT assigns $PORT and shows the link when the port answers
data    = true                      # PDT provides $PDT_RUN_DATA, a folder kept per branch
detach  = true                      # optional: keeps running when PDT restarts or updates (no input)
env     = { NAME = "value" }        # optional

[[test]]
name    = "unit"
command = "..."                     # passes on exit 0
```

PDT sets `PORT` and `PDT_RUN_URL` (web runs), `PDT_RUN_DATA` (with `data`), `PDT_RUN_HOST` (the hostname the link
uses: the tailnet name when remote access is on, else 127.0.0.1) and `PDT_RUN_BRANCH`, and expands only those, as
`$NAME` or `${NAME}`, in `setup`, `command` and `env`. Steps run in the checkout with `sh -c` (macOS/Linux) or a
generated `.cmd` (Windows), with the host's normal environment. Rules:
- The server must listen on `$PORT`. Bind it to 127.0.0.1 where the framework allows.
- If the server checks the Host header, allow `$PDT_RUN_HOST` (through a flag or env var; don't edit the project's
  own config files for this unless I agree).
- Point app state at `$PDT_RUN_DATA` when the app supports it, and set `data = true`.
- Set `detach = true` only for a long-lived app I use day to day (a desktop app on the main checkout) that must
  not be killed when PDT updates. Its output goes to a log, and it takes no typed input.
- Never put secrets in it, and don't forward a variable PDT doesn't set through `env` (`X = "$SOMETHING"` passes
  the literal text).
- Prefer plain commands on PATH, joined with `&&`; no `||`, `;` or pipes (they don't work on Windows).

**Where to save it:** as **host overrides** unless I say otherwise. They live only in PDT, so the repo isn't
touched: `GET /api/projects/<projectId>/settings`, then
`PUT /api/projects/<projectId>/settings` with `{"agentCredentials": <the value you read>, "runOverrides": "<the TOML>"}`
(the PUT replaces both fields). If I'd rather commit it, write `.pdt/project.toml` instead, on a branch, not on
the base branch.

**4. Check it.**
- `GET .../run-config` again: `error` must be null and `effective` must show your entries.
- Try each step yourself in a shell first, from the checkout, with `PORT` set to a free port and `PDT_RUN_DATA` set
  to a temporary folder. Check that the server answers on that port, then stop it.
- Then start it through PDT: `POST /api/projects/<projectId>/runs` with `{"name": "app", "kind": "run"}` (plus
  `"worktreeId"` when this session has one). Poll `GET /api/runs/<runId>` until `state` is `ready` or `failed`
  (setup can take minutes). When ready, `curl` its `url`. If it failed, report `error` and `exitCode`; the output is in
  the run's tab in PDT. Leave it running, so I can look at it.
- Start each test the same way (`"kind": "test"`) and report `passed` or `failed`. A failing test suite isn't
  something to fix here; just report it.

**5. Report** in a few lines: the configuration you saved and where, what you checked and the result, the link of the
running app, and anything I have to do myself (services to start, secrets to provide, a Host setting you didn't
change).
