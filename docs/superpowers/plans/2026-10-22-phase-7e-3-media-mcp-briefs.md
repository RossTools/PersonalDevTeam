# Phase 7e-3 plan: media connections, MCP tools as actions, brief archive (PDT-86)

Spec: docs/superpowers/specs/2026-10-20-phase-7e-integrations-design.md §1, §7. One PR, no migration, built in this
order (each step test-first, `cargo test` green before the next). External services are always local fakes.

1. **Brief archive.** `nodes/brief.rs`: `brief.archive` (`brief`, `title`, `body`, `items [{title, link, note}]`,
   `date`, default today in the host's zone) appends a Markdown entry to `<data dir>/briefs/<brief>/<date>.md` and a
   line to `index.jsonl` (`{date, at, title, items [{title, link}]}`), serialized in-process; `brief.read` (`brief`,
   `since` or `last`, default the last 7 entries) → `{entries, days [{date, markdown}]}`. Brief names are slugs
   (`[a-z0-9_-]`), dates `YYYY-MM-DD`, so nothing escapes the folder. Dry run of `brief.archive` reports `would`.
2. **Radarr and Sonarr.** `nodes/arr.rs`: one client for both (connection URL + `/api/v3/...`, `X-Api-Key` from the
   secret, the connection's kind checked). `radarr.search` / `sonarr.search` (`term`) → `{results [{tmdbId|tvdbId,
   title, year, inLibrary, overview}]}`; `radarr.add` / `sonarr.add` (`tmdbId|tvdbId`, `qualityProfile`, `rootFolder`,
   `monitor` (Sonarr), `search` default true) with ports `added` / `exists` (the library is checked first); the
   profile is a name or id from the node, else `config.qualityProfile`, else the first profile; the folder likewise,
   else the first root folder. `radarr.queue` / `sonarr.queue` → `{items [{title, release, progress, eta, timeLeft,
   status}]}`. Tests: a fake server answering the v3 paths.
3. **Plex.** `nodes/plex.rs`: `X-Plex-Token` from the secret, `Accept: application/json`. `plex.search` (`query`,
   `limit`) via `/hub/search`; `plex.recently_added` (`section` name or key, else all, `limit`); `plex.refresh`
   (`section`, else all). A section given by name is looked up in `/library/sections`. Fake server tests.
4. **MCP client.** pdt-api `mcp_client.rs` with `rmcp` (client, child process, streamable HTTP over reqwest
   native-tls): an `McpPool` in `AppState`, one live client per `mcp` connection (stdio: `config.command`, `args`,
   `env`, the secret as `config.secretEnv`; http: the connection's `url`, the secret as a bearer token), replaced when
   the connection changes, dropped after 5 min idle or on a transport error, all shut down with pdtd. A tool cache by
   connection name (`tools/list`): refreshed when an `mcp` connection is saved, and every 10 min by
   `maintain_workflows`. `GET /api/connections/{id}/tools` (cached) and `POST` (list now).
5. **`mcp.tool`.** `nodes/mcp_tool.rs` (`server`, `tool`, `arguments`): output the structured content, else the text
   content (parsed when it is JSON); `isError` fails the step; dry run reports `would`. The catalog response (and
   pdt-mcp's `catalog_search` / `catalog_get`) adds a virtual entry `mcp:<server>/<tool>` per cached tool: its title
   and description, `configSchema` with the tool's input schema as `arguments`, and a new `KindSpec.preset`
   (`{kind: "mcp.tool", config: {server, tool, arguments: {}}}`). Validation (validate, check, dry run, workflow
   detail) warns (`mcp_tool`) when a cached server lacks the tool, or `arguments` miss a required property, have one
   the schema refuses, or a literal of the wrong type (references are not checked). Tests: a `fake-mcp` stdio server
   binary in src/bin and an HTTP fake (rmcp's streamable HTTP server is not pulled in: a small axum JSON-RPC answer).
6. **Web.** `newNode` builds preset entries as their kind (with the tool's title); Connections: kinds `radarr`,
   `sonarr`, `plex` and `mcp` (its transport, command, args, env, secret variable), and an MCP connection's tools.
7. **Docs.** docs/reference/workflows.md (kinds, connections, MCP, briefs), agent-guide.md (catalog entries).
8. **Checks**: fmt, clippy, cargo test, web typecheck/test/build; PR labelled `windows` (process spawning, paths).
