# Secrets agents can use but never see (PDT-158)

The user gives agents (the PM included) and workflows secrets (API keys, tokens, passwords) they can *use* but never
*read*. pdt-core `secret.rs` (types, rules, references, the `Redactor`), pdt-store `secrets.rs` (rows and uses),
pdt-api `vault.rs` (keychain, routes, tools, the Inbox answer, workflow filling).

## Where they live

- **Values**: the OS keychain only (`SecretStore`, account `secret:<id>`; `PDT_SECRET_STORE=memory` in tests), and,
  while pdtd runs, the store's `Redactor`. Never in the database, a response, a log or a tool result.
- **Rows** (`secrets`): `name` (unique ignoring case; letters, digits, `_`, `-`, starting with a letter, at most 64),
  `description`, `projects` and `personas` (JSON id lists: who may use it; empty means any), `last_used_at`,
  `last_used_by`. `secret_uses`: every use (`by`: the session's title or `workflow <name>`, `how`: the tool or node
  kind, `session_id`, `run_id`), deleted with the secret.
- **Access** (`Secret::allows`): an agent session may use one when its project is in `projects` and its persona in
  `personas` (a session with no persona only when `personas` is empty). A workflow run only by `projects`: the run's
  project, else the workflow's, else the host project (host and personal workflows; also the PM's project).
- Values are at least 6 characters (shorter ones would be redacted all over ordinary text) and at most 16 KB.

## Redaction

`pdt_core::secret::Redactor` knows every value (as it is and as written inside a JSON string) and replaces it with
`[secret:<name>]`, longest first. One per pdtd, shared by:

- the store (`Store::with_redactor`), on what it writes: conversation events (`body`, `raw`; the event returned and
  broadcast is the redacted one), ticket comments and events, workflow steps (`input`, `output`, `error`, `wait`),
  runs (`trigger`, `output`, `error`), Inbox items (`title`, `body`, `resolution`), outbox messages, notifications
  (`title`, `body`) and the event log's payloads (`emit_event`);
- a conversation's live text deltas (each chunk) and the partial text sent on reconnect;
- every pdt-mcp tool result and error (`mcp/mod.rs`);
- the log writer (`pdtd/src/logging.rs`, the file and stderr), so bug reports' log tail is clean too.

pdtd loads every value from the keychain in the background as it starts (`vault::load_known`); one it can't read then
is read, and known, on first use. Setting a value makes it known at once; deleting a secret forgets it.

Not covered: what the agent itself keeps (Claude's own transcript files, a terminal session's scrollback), output PDT
never sees, and a value split across two live deltas (the stored message is redacted). Stopping a determined agent
from leaking a value through a command it runs is out of scope: agents are told to use the tools below.

## Settings → Secrets

`web/src/secrets/SecretsPage.tsx`, under This host in the Settings list (`/settings/secrets`). Lists each secret's
name, who may use it, its description and last use; Uses shows its audit list; Add and Edit ask for the name (new
only), description, value (masked; empty on Edit keeps it) and ticks for projects (the host project shows as "The
PM") and agents (personas); Delete asks first.

API: `GET /api/secrets`, `POST /api/secrets {name, value, description?, projects?, personas?}` (`201`; `400
invalid_input`, `409 secret_exists`, `502 keychain_failed`), `PATCH /api/secrets/{id} {description?, projects?,
personas?, value?}`, `DELETE /api/secrets/{id}` (`204`; the keychain entry too), `GET /api/secrets/{id}/uses` (newest
200).

## Agents (pdt-mcp)

In the default tool set and the PM's (pdt-core `DEFAULT_TOOLS`; each tool's `pm` flag), never asking:

- `list_secrets {}`: names and descriptions of the secrets this session may use.
- `request_secret {name, purpose}`: one the session may use already answers `{name, stored: true}`; one that exists
  but it may not use is an error; otherwise an Inbox item of kind `secret` (group Questions; `body` `{name, purpose,
  sessionTitle}`) opens, replacing the step's open one, and the agent step is `asking`, as with `ask_user`. The user
  enters the value in a masked field (`POST /api/inbox/{id}/answer {secret}`; `InboxAnswer`'s `Debug` hides it): it
  becomes a secret for the asking project (description: the purpose), or replaces the value of one added meanwhile,
  and the session gets the message "The secret NAME is stored…", never the value (`resolution` `{name, stored}`).
  Dismiss closes it, the step still asking.
- `run_with_secrets {command, env?, cwd?, timeoutSecs?}`: `sh -c` (`cmd /C` on Windows) in the session's folder (a
  relative `cwd` is under it), with `env` values `secret:<name>` replaced by the value. `{{secret.…}}` in the command
  line is refused. Returns `{exitCode, timedOut, stdout, stderr, truncated, secrets}`, each stream capped at 64 KB and
  redacted. Default limit 120 s, at most 1800; the process tree is killed at the limit.
- `write_secret_file {path, template}`: writes the template with `{{secret.<name>}}` filled in (a relative path is
  under the session's folder; owner-only on macOS and Linux); returns `{path, bytes, secrets}`, never the content.

Each named secret must exist and be allowed, else the call fails naming it; each use is recorded.

## Workflows

`{{secret.<name>}}` in a node's config strings and `{"$secret": "<name>"}` as a config value are filled in only as
the node runs (`vault::fill_step`, from the engine): the engine's template pass keeps `secret.` references, the step's
stored `input` keeps them as written, and its output and error are redacted when stored. Only references the node's
**config** holds count: at run time the config is resolved again against the run's context with the secrets added
(`secret::resolve_with_secrets`), in one pass, so text a reference brings in (a trigger's payload, a ticket's text, an
earlier step's output) is passed on as written, never filled and never an error; a `$secret` is filled only where the
config has one. Otherwise anyone who can put text in data a workflow forwards could have a secret sent out.
`{{secret.x}}` must name a secret; a `$secret` naming none is left for the node (the Scryer nodes' connection
references). `agent` steps and `session.send` take no secrets (the step fails). Dry runs fill nothing. Uses are
recorded as `workflow <name>` with the node kind.

Tests: crates/pdt-api/tests/secrets.rs (the done-when: request, masked answer, `run_with_secrets` echo, a pasted
value, comments, `http.request` with both forms, references arriving as data passed on unfilled, access, and a grep of the database files for the value).
