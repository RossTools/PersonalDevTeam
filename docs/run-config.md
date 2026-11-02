## For agents: reading and changing it

PDT serves this guide at `GET /api/guides/run-config`: the user page above, then this part. To have an agent write a
project's configuration, PDT's set-up prompt is `GET /api/guides/set-up-project`.

The address of PDT is `127.0.0.1:7420` for the stable instance. Agent sessions started by PDT have `PDT_HOOK_PORT`
(that PDT's port) and `PDT_SESSION_ID`; `GET /api/sessions/$PDT_SESSION_ID` gives the session's `projectId` and
`worktreeId`.

| Call | Does |
|---|---|
| `GET /api/projects` | projects (`id`, `repoPath`, …) |
| `GET /api/projects/{id}/run-config[?worktreeId=…]` | `file`, `overrides`, the merged `effective` config (`runs`, `tests`, `areas`, `ignored` sections), or `error` |
| `GET /api/projects/{id}/areas` | `{ areas: [{ name, paths, exclusive }], error }` from the main checkout and the overrides |
| `GET /api/projects/{id}/settings` | `{ agentCredentials, effectiveCredentials, runOverrides, agentInstructions, mergeViaPr }` |
| `PUT /api/projects/{id}/settings` | `{ agentCredentials, runOverrides, agentInstructions?, mergeViaPr? }`; replaces `agentCredentials` and `runOverrides`, so send back the `agentCredentials` you read. An empty `runOverrides` clears them. Text that isn't valid TOML, or has an unknown section, is refused (`invalid_run_config`); check `run-config`'s `error` for the rest. `agentInstructions` and `mergeViaPr` are kept when left out (`""` clears the instructions) |
| `GET /api/projects/{id}/commands[?worktreeId=…]` | the checkout's `commands` (`fingerprint`, `uses`, `setup`, `command`, `env`) and the `untrusted` fingerprints, which wait on the user's approval |
| `GET /api/guides` · `/api/guides/{name}` | PDT's built-in guides (Markdown): `agents`, `run-config` (this one), `set-up-project`, `set-up-workflow` |
| `POST /api/projects/{id}/runs` | `{ name, kind: "run" \| "test", worktreeId? }`; without `worktreeId` (or `branch`) it uses the main checkout. Opens a tab in the project's workspace |
| `GET /api/runs/{id}` | `state` (`setting_up`, `starting`, `ready`, `running`, `passed`, `failed`, `stopped`), `url`, `exitCode`, `error` |
| `POST /api/runs/{id}/stop` · `/restart` · `/reset-data` | as named |

A run or test whose command the user hasn't approved is refused with `409 untrusted_commands`, naming the commands.
You can't approve one: ask the user to start it in PDT, or to approve it in the project's Settings. Changing a
command in `.pdt/project.toml` (in your worktree too) makes it new, so the user is asked again.

Notes for writing it:

- A section the running PDT doesn't know is skipped in `.pdt/project.toml` (listed in `effective.ignored`), but
  refused in host overrides, which are written for this PDT.
- A detached run's output is also at `<PDT data dir>/runs/logs/<run id>.log`. PDT sets `PYTHONUNBUFFERED=1` for it
  unless it is set.
- The local forge's CI log for a run is at `GET /api/local-ci/runs/<id>/log` (the CI-failed message gives the URL).
- `[ci] providers` needs PDT 0.3.0 or newer; an older PDT refuses the whole file over the unknown key.
- `[[merge.regenerate]]` needs a PDT newer than 0.5.1; an older one skips `[merge]` (listed in `effective.ignored`).
