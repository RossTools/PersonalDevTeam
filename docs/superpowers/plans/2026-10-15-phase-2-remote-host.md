# Phase 2 — Remote Host Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make each PDT host usable from any device on the tailnet. A host serves itself over HTTPS with Tailscale and admits only its owner. Any branch can be run with an HTTPS preview link. Agent sessions show what they are doing and whether the user has seen it. A small supervisor (`pdt-host`) keeps `pdtd` running and installs releases from GitHub with automatic rollback.

**Architecture:**
- **`pdt-core` (pure):** gains these modules:
  - `host`: per-host settings, credential policy, port picking;
  - `identity`: the owner check;
  - `runconfig`: `.pdt/project.toml` parsing, merging and variable expansion;
  - `run`: run instance types;
  - `activity`: the agent activity and unseen state machine;
  - `release`: versions, assets, `SHA256SUMS`, token warnings;
  - `handoff`: the update request and result files shared by `pdtd` and `pdt-host`.
- **New crates:**
  - `pdt-tailscale`: wraps the `tailscale` CLI. It includes a recording fake, plus a `fake-tailscale` binary for its own tests.
  - `pdt-update`: the GitHub REST client (reqwest), and verified downloads.
  - `pdt-host`: the supervisor binary and its library. It includes a `fake-pdtd` binary for its own tests.
- **`pdt-store`:** gains migrations `0004` (host and project settings), `0005` (runs) and `0006` (session activity).
- **`pdt-api`:** gains these routes:
  - host settings, remote access and identity;
  - run configuration and run instances (each run's PTY WebSocket reuses the Phase 1 pump);
  - the activity hook endpoint and the view endpoint;
  - update status and actions.

  The secret store is a trait: the real implementation uses the OS keychain, and tests use an in-memory fake.
- **`pdtd`:**
  - gains a `pdtd hook …` subcommand;
  - runs the update loop and hands releases off to `pdt-host` through `<data dir>/update/request.json`;
  - restarts active runs at startup and sweeps runs whose branch has gone.
- **Desktop:** becomes a pure view. It starts `pdt-host` instead of `pdtd`, and its gh-based update is removed. Update calls `pdtd`'s API.

**Tech Stack:**
- **Existing:** everything from Phases 0 and 1.
- **New Rust crates** (add with `cargo add`; the latest on 2026-10-15 is shown):
  - `reqwest` 0.13.5, with `default-features = false` and the features `native-tls`, `json`, `stream`, `http2` and `system-proxy`;
  - `keyring` 4.2.0, default features;
  - `toml` 1.1.8;
  - `sha2` 0.11.1;
  - `hex` 0.4.3.
- **No new web packages.**

**Spec:** `docs/superpowers/specs/2026-10-15-remote-host-design.md` is binding. It amends `docs/superpowers/specs/2026-10-14-personal-dev-team-design.md`: read that spec's §3.1, §5.5, §8, §9 and §11 as amended. Also read `docs/superpowers/notes/phase-1-carryover.md`. This plan closes the carry-over item "`PDT_ENV_VARS` duplicates pdtd's config list" (Task 3).

**Branch:** `phase-2-remote-host` (already checked out; the spec is committed there).

## Verified facts (2026-10-15)

Each fact below was checked before it was written into a task. When a step depends on one, the step cites it by its number.

1. **Claude Code 2.1.301** (`claude --version` on the dev machine):
   - **The flag:** `claude --help` lists `--settings <file-or-json>`: "Path to a settings JSON file or a JSON string to load additional settings from".
   - **Precedence:** command-line settings sit between managed and local settings, and list keys merge across levels instead of overriding (code.claude.com/docs/en/settings, "Lists merge instead of overriding").
   - **Real run:** run in a temp folder with no project settings:

     ```
     claude -p "reply with ok" --model haiku --session-id <uuid> --settings <tmp>/settings.json
     ```

     The run fired our `UserPromptSubmit` and `Stop` hooks, alongside the user's own plugin hooks. Neither set replaced the other.
   - **Payloads:**
     - Both had `session_id` (equal to `--session-id`), `transcript_path`, `cwd`, `prompt_id`, `permission_mode` and `hook_event_name`.
     - `UserPromptSubmit` also had `prompt`.
     - `Stop` also had `stop_hook_active:false`, `last_assistant_message`, `background_tasks` and `session_crons`.
   - **Environment:** a third run showed that the hook process inherits the parent's environment: `PDT_SESSION_ID` set before launching `claude` was visible to the hook.
   - **The hook shape that ran:** `{"type":"command","command":"node","args":[...]}`, which is the **exec form**.
2. **Claude hooks reference** (code.claude.com/docs/en/hooks):
   - **Settings shape:** `{"hooks":{"<Event>":[{"matcher":"…","hooks":[{"type":"command","command":"…","args":[…],"timeout":N}]}]}}`.
   - **Matchers:** `UserPromptSubmit` and `Stop` ignore matchers. `PreToolUse` matches tool names; `"*"`, `""` or no matcher means all. `Notification` matches `notification_type`.
   - **Exec form:**
     - With `args`, "`command` is resolved as an executable and spawned directly … no shell".
     - Without `args`, Windows runs the command in Git Bash, or in PowerShell when Git Bash is missing.
     - Exec form needs a real executable, not a `.cmd` shim.
   - **Exit 0 and stdout:**
     - Exit 0 is success.
     - For `UserPromptSubmit`, plain stdout **is added to Claude's context**.
     - `Notification` ignores exit codes.
   - **Default timeout:** 600 s, lowered to 30 s for `UserPromptSubmit`.
   - **Events:**
     - `PermissionRequest` exists: "When a tool call needs a permission decision".
     - `Notification` input has `message`, an optional `title` and `notification_type`.
     - `permission_prompt` fires "after about six seconds". The other types include `idle_prompt` (60 s after a reply), `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input` and `auth_success`.
3. **Claude login check:** `claude auth status` prints JSON with `"loggedIn": true|false` and exits 0. Checked locally.
4. **Codex CLI 0.162.1** (`codex --version`; the binary is `C:\Users\ross\AppData\Local\Programs\OpenAI\Codex\bin\codex.exe`):
   - **`-c`:** `codex --help` and `codex resume --help` both list `-c, --config <key=value>`: "The `value` portion is parsed as TOML. If it fails to parse as TOML, the raw string is used as a literal."
   - **`notify`** (learn.chatgpt.com/docs/config-file/config-advanced; developers.openai.com redirects there):
     - The program gets "a single JSON argument", appended after the configured array.
     - The only event is `agent-turn-complete`.
     - The payload fields are `type`, `thread-id`, `turn-id`, `cwd`, `input-messages` and `last-assistant-message`.
     - `approval-requested` exists only for the TUI's built-in `tui.notifications`, not for `notify`.
   - **The user's existing `notify`:** the dev machine's `~/.codex/config.toml` already sets `notify = [ "…codex-computer-use.exe", "turn-ended" ]`. A `-c notify=…` override replaces it, so PDT chains to it (Decision 11).
   - **Login check:** `codex login status` exits 0 and prints "Logged in using ChatGPT".
   - **Not verified:** that `notify` fires inside a PTY for 0.162.1, and the exact text of its approval screen. Task 1 verifies both on the dev machine.
5. **Tailscale 1.102.2** (`tailscale version` on the dev machine):
   - **`tailscale serve --help`:**
     - `--bg` runs in the background.
     - `--https <port>` exposes HTTPS on that port.
     - Targets may be URLs such as `http://127.0.0.1:3000`.
     - `serve status [--json]` and `serve reset` exist.
   - **Removing a mapping** (tailscale.com/docs/reference/tailscale-cli/serve): "add `off` to the end of the command you used to turn it on … the `<target>` is optional in `off` commands, but all original flags are required". So the removal is `tailscale serve --https=<port> off`. Whether `--bg` must be repeated in that form is **not verified**; the Mac mini checklist checks it (Task 15).
   - **`tailscale status --json`** (dev machine, read-only):
     - The top-level keys include `BackendState` (`"Running"`), `Self`, `User`, `CertDomains` and `MagicDNSSuffix`.
     - `Self.DNSName` ends in a dot (`rosspc.taila6ad31.ts.net.`).
     - `Self.UserID` keys into `User`, whose entries have `LoginName`.
     - `CertDomains` was empty on the dev machine, which suggests HTTPS certificates are not enabled on this tailnet yet. Task 4 reports that state.
   - **Identity headers** (tailscale.com/docs/features/tailscale-serve.md; source `ipn/ipnlocal/serve.go` on `main`):
     - Serve sets `Tailscale-User-Login`, `Tailscale-User-Name`, `Tailscale-User-Profile-Pic` and `Tailscale-Headers-Info` only for untagged tailnet users. "These identity headers are not populated for traffic originating from tagged devices", and Funnel requests get `Tailscale-Funnel-Request: ?1` and no identity.
     - Serve deletes incoming copies of all of these.
     - Serve **always** sets `X-Forwarded-Host`, sets `X-Forwarded-For` to the caller's address, and keeps the original `Host` header (`r.Out.Host = r.In.Host`).
     - Values may be RFC 2047 "Q"-encoded when they contain non-ASCII characters.
6. **GitHub REST** (docs.github.com, API version 2022-11-28):
   - **Latest release:** `GET /repos/{owner}/{repo}/releases/latest` returns "the most recent non-prerelease, non-draft release". Its fields include `tag_name`, `name`, `body`, `published_at` and `assets[]`, each with `id`, `name`, `url`, `browser_download_url`, `size` and `digest`.
   - **Asset download:** `GET /repos/{owner}/{repo}/releases/assets/{asset_id}` with `Accept: application/octet-stream`. "API clients should handle both a 200 or 302 response."
   - **Token expiry:** `GitHub-Authentication-Token-Expiration` is returned for tokens that expire (GitHub changelog, 2021-07-26). Its value is `YYYY-MM-DD HH:MM:SS UTC` or `YYYY-MM-DD HH:MM:SS ±HHMM`, according to the WebKit bug 302492 and Composer PR 11688. Both forms are parsed. The exact live format is **not verified**: we have no fine-grained token to test with.
7. **Crates:**
   - `cargo search` gives `reqwest 0.13.5`, `keyring 4.2.0`, `toml 1.1.8+spec-1.1.0`, `sha2 0.11.1` and `hex 0.4.3`.
   - **reqwest:** reqwest 0.13's default TLS is rustls with **aws-lc-rs**, which builds C code. This plan uses `native-tls` instead (SChannel on Windows, Security.framework on macOS): no C toolchain, and the OS trust store, which corporate TLS inspection on the work laptop needs.
   - **keyring:**
     - keyring 4's default `v1` feature enables `windows-native-keyring-store` (Credential Manager) and `apple-native-keyring-store/keychain`.
     - Its API is `keyring::v1::Entry::new(service, user) -> Result<Entry>`, with `set_password`, `get_password` and `delete_credential`.
     - It returns `keyring::v1::Error::NoEntry` when nothing is stored.
     - `Error` is `#[non_exhaustive]`.
8. **Tauri NSIS hooks** (v2.tauri.app/distribute/windows-installer):
   - `bundle.windows.nsis.installerHooks: "./windows/hooks.nsh"`.
   - The macros are `NSIS_HOOK_PREINSTALL`, `NSIS_HOOK_POSTINSTALL`, `NSIS_HOOK_PREUNINSTALL` and `NSIS_HOOK_POSTUNINSTALL`.
9. **cargo on this machine** is at `%USERPROFILE%\.cargo\bin\cargo.exe`. It may be missing from a non-interactive PowerShell's `PATH`; if `cargo` is not found, call it by that path.

## Decisions this plan makes

The spec leaves these open, or the facts above force them. Each is recorded so it can be reviewed and reversed.

1. **A request through a proxy without an identity is refused.** The spec says a request without identity headers is local. But Tailscale sends no identity for tagged devices or Funnel (fact 5), and that traffic would then be trusted. So:
   - a request carrying `X-Forwarded-For`, `X-Forwarded-Host` or `Tailscale-Funnel-Request` but no `Tailscale-User-Login` gets 403 `forbidden_identity`;
   - truly local requests carry none of these headers and stay trusted.
2. **The owner must be set before remote access can be turned on.** Turning remote access on with no effective owner gives 400 `owner_required`. Settings offers the login of the host's own Tailscale user (`Self.UserID` → `User[...].LoginName`).
   - `PDT_OWNER_LOGIN`, when set, overrides the stored owner. Settings shows it as read-only.
   - Owner comparison trims whitespace and ignores ASCII case.
3. **Allowlist entries:**
   - Remote access adds the bare DNS name (`box.tailnet.ts.net`) to the Host/Origin allowlist. Serve keeps the caller's `Host` header, which has no port on 443.
   - The allowlist becomes a shared, mutable `AllowedHosts`.
4. **Scrubbing by prefix:** every inherited variable starting with `PDT_` is removed from every child process. This replaces the list in `pdt-core`; `PDT_PROFILE`, `PDT_PORT`, `PDT_DATA_DIR`, `PDT_WEB_DIR` and `PDT_ALLOWED_HOSTS` remain covered. PDT then sets its own variables: `PDT_SESSION_ID`, `PDT_HOOK_PORT`, `PDT_RUN_*` and `PDT_OWNER_LOGIN`.
5. **Run terminals are not sessions.** Each run step has a PTY with the id `run:<run id>:setup` or `run:<run id>:command`.
   - It is served at `/api/runs/{id}/pty/{step}` by the same pump as sessions.
   - Runs never appear in the session list, and the session auto-resume never touches them.
6. **Run states:** `setting_up`, `starting`, `ready`, `failed` and `stopped` come from the spec. Tests add `running` and `passed`.
   - A run without `web` is `ready` once its command has started.
   - A web run is `ready` once a TCP connect to `127.0.0.1:<port>` succeeds.
   - When a run's command exits on its own, the run is `failed`, with the exit code. When it was asked to stop, it is `stopped`.
7. **How run commands execute:**
   - **Expansion:** PDT expands `$PORT`, `${PORT}`, `$PDT_RUN_DATA`, `$PDT_RUN_URL`, `$PDT_RUN_HOST`, `$PDT_RUN_BRANCH` and `$PDT_OWNER_LOGIN` in `setup`, `command` and `env` values before running. The same TOML then works on both OSes. Other `$…` text is left for the shell.
   - **Windows:** the step is written to `<data dir>/runs/<run id>/<step>.cmd` and run with `%ComSpec% /d /c <file>`, which avoids PTY argument quoting.
   - **macOS:** `/bin/sh -c <command>`, with the login `PATH` (Phase 1's `login_path`).
8. **Where runs run:**
   - A run targets the main checkout, an existing worktree, or a branch.
   - A branch with no worktree gets one. A local branch is checked out with `git worktree add <path> <branch>`. A branch that exists only on `origin` becomes a local tracking branch.
   - That covers same-repo PR branches. Fork PRs wait for the git integration phase.
   - Run configuration is read from the checkout being run, so a branch's own `.pdt/project.toml` applies.
9. **Run lifecycle details:**
   - **Restart:** runs restart on the user's Restart click. They do not restart automatically when a branch changes.
   - **Gone branches:** runs stop when their worktree folder or branch ref disappears. A sweep checks every 60 s. Squash merges can't be detected from git, and the merge flow removes the worktree anyway.
   - **Concurrency limit:** the limit counts runs only, not tests. Going over it gives 409 `too_many_runs`, naming the oldest active run. The UI confirms, then retries with `stopRunId`.
10. **Activity mapping:**
    - **Claude:**
      - `UserPromptSubmit` and `PreToolUse` mean `working`.
      - `PermissionRequest` means `needs_you` (it fires at once, where `Notification:permission_prompt` waits about 6 s).
      - `Notification` with `permission_prompt`, `elicitation_dialog`, `elicitation_url_dialog` or `agent_needs_input` means `needs_you`. `idle_prompt` means `idle`. Other notification types change nothing.
      - `Stop` means `idle`.
    - **Codex:**
      - `agent-turn-complete` means `idle`.
      - An Enter typed into the terminal means `working`.
      - An approval screen means `needs_you`.
    - **Fallback to `unknown`:**
      - `working` with no PTY output for 120 s (the agents' spinners repaint constantly while busy).
      - A Claude Enter followed by no hook within 20 s while output flowed.
    - **Starting:** a session starts `idle`.
    - **Attention:** a change *into* `idle` or `needs_you` from a different activity records `attention_at`. The session is unseen while `attention_at > last_viewed_at`.
11. **Hook commands:**
    - **Exec form:** the hooks use exec form with the absolute path of the running `pdtd` (`std::env::current_exe()`). That avoids shells and quoting on both OSes (fact 2).
    - **What the hook does:** it reads at most 1 MiB of stdin and posts to `127.0.0.1:$PDT_HOOK_PORT`, with 2 s timeouts. It never prints to stdout, because `UserPromptSubmit` stdout would reach Claude's context, and it always exits 0.
    - **Codex chain:** if `$CODEX_HOME/config.toml` already has a top-level `notify`, PDT's `notify` passes `--chain <their command…> --`. `pdtd hook codex-notify` then runs their command with the same JSON, so the user's notifier keeps working.
12. **The per-session Claude settings file** is `<data dir>/sessions/<session id>/claude-settings.json`. It is rewritten on every launch and deleted with the session.
13. **Updates:**
    - **Download:**
      - Downloads disable automatic redirects.
      - The asset request carries the token. Its 302 `Location` is fetched **without** an `Authorization` header.
      - Nothing is handed off unless `SHA256SUMS` lists the asset and the hash matches.
    - **Asset names:** CI renames assets so their names contain no spaces before it computes `SHA256SUMS`. GitHub would rename them on upload, and the names in the checksums file must match the uploaded ones.
    - **Supervision:** updates are offered only when `pdtd` runs under `pdt-host`. `pdt-host` sets `PDT_SUPERVISED=1` for its `pdtd`. A dev instance never updates, even if the keychain holds a token.
    - **Where tokens live:** the keychain account is `update-token`, under the service `brand::KEYCHAIN_SERVICE`. Tests and the `pdtd` lifecycle tests use `PDT_SECRET_STORE=memory`.
    - **Busy list:** a busy Update returns 409 `host_busy`, and the message lists what is busy. The UI shows that message in a confirm dialog and retries with `force: true`.
14. **`pdt-host` layout and behaviour:**
    - **Where it runs:** `pdt-host` runs from a copy at `<data dir>/host/pdt-host[.exe]`, never from the install folder, so installers can replace the install folder.
    - **Commands:** `pdt-host run --install-dir <dir>`; `pdt-host install --install-dir <dir>`, which copies the binary, registers it and starts it; and `pdt-host uninstall`.
    - **Registration:**
      - **macOS:** a LaunchAgent plist with `RunAtLoad` and `KeepAlive`, loaded with `launchctl bootstrap gui/<uid>`.
      - **Windows:** a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written with `reg.exe`. That is per-user and needs no admin rights. `schtasks /SC ONLOGON` may need elevation, which is not verified, so it is not used.
    - **Keeping the previous version:**
      - **Windows:** the install folder is copied to `<data dir>/update/previous/` before the silent NSIS install. Rollback copies it back.
      - **macOS:** the new bundle is swapped in by rename. The old bundle stays as `<bundle>.previous`, and rollback renames it back.
    - **Watching `pdtd`:** `pdt-host` watches `pdtd` by `/api/health`, and through the child handle when it started `pdtd` itself. After a self-replacement, the new `pdt-host` adopts a `pdtd` that is already running.
15. **Versions:**
    - `BUILD_VERSION` defaults to `0.1.0-local`.
    - `0.1.0-local` is older than every release, so a local build always offers the newest release.
    - The Tauri config keeps `0.1.0`.
16. **Tray:**
    - **Start:** the tray starts `pdt-host run` if `/api/health` does not answer.
    - **Quit:** Quit exits only the tray; `pdtd` keeps running under `pdt-host`.
    - **Restart:** "Restart daemon" posts `/api/shutdown`, and `pdt-host` brings `pdtd` back.
    - **Update:** Update posts `/api/host/update/install`.

## Global Constraints

- **CI:** Windows and macOS CI both pass: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, the web typecheck, tests and build, and the desktop clippy and tests. Tests assert with `contains` around PTY output (ConPTY adds escape sequences).
- **Toolchain:** Rust `1.99.0`, edition `2024`. On Windows, run cargo from **PowerShell**, never Git Bash.
- **New dependencies:** add them with `cargo add` or `pnpm add`, which picks the latest version. Never type a version from memory. The versions in the Tech Stack are what `cargo search` showed on 2026-10-15.
- **Names:** names live only in `crates/pdt-core/src/brand.rs`, `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/tauri.bundle.conf.json` and `scripts/release-stable.*`. The new names `HOST_BIN`, `HOST_LABEL`, `HOST_AUTOSTART_NAME`, `KEYCHAIN_SERVICE` and `PROJECT_CONFIG_FILE` go in `brand.rs`.
- **Never touch the stable instance:**
  - Nothing in a test or dev run may touch port 7420 or the real data folders. Use port 0 or a free port, temp folders, and `PDT_PROFILE=dev` for manual dev daemons.
  - Any test that starts `pdtd` or `pdt-host` removes every inherited `PDT_*` variable.
  - Never run `scripts/release-stable.*` without `-BuildOnly` / `--build-only`.
  - Never run a built installer, never `pdt-host install` on the dev machine, and never POST `/api/shutdown` to port 7420.
- **Secrets:** secrets live only in the OS keychain or credential manager (`keyring`), never in a file, the database, logs or API responses. Tests use the in-memory store.
- **No outbound traffic** except the GitHub update check and downloads, and only when a token is stored. There is no telemetry, and no host-to-host traffic. Tailscale is a local CLI.
- **Never touch the user's agent settings:** never write the user's `~/.claude/settings.json`, `~/.codex/config.toml` or any project's `.claude/` files. Per-session settings live in PDT's data folder.
- **Errors:** library crates use `thiserror`, and only the binaries (`pdtd`, `pdt-host`) use `anyhow`. API errors are always `{ "code": string, "message": string }`.
- **Listening:** `pdtd` listens only on `127.0.0.1`.
- **Colors** come only from `web/ui/tokens.css`, through the Tailwind theme names in `web/src/index.css`.
- **Timestamps** are `i64` Unix milliseconds, exported to TypeScript as `number`.
- **Generated TypeScript types:** `cargo test` regenerates `web/src/api/gen`. Commit the results.
- **Migrations:** never edit `0001`–`0003`; add new files.
- **TDD** applies to `pdt-core`, `pdt-store`, `pdt-git`, `pdt-tailscale`, `pdt-update`, `pdt-agents`, `pdt-api` and the `pdt-host` library.

## Review Focus

1. **A tailnet request with no identity headers is refused.** This covers a tagged device, Funnel, or any request that came through a proxy, and the response is 403 `forbidden_identity`. Such a request must not be trusted as local. Pinned in Task 5 (`forwarded_without_identity_is_refused`).
2. **Stopping a web run frees its port**, including the grandchild server under `cmd.exe` or `/bin/sh`, so that the next run (or a restart) can bind it on both OSes. Pinned in Task 7 (`stopping_a_web_run_frees_its_port`).
3. **A hook never disturbs the agent:**
   - it prints nothing on stdout (stdout from `UserPromptSubmit` would be injected into Claude's context);
   - it exits 0 quickly when `pdtd` is down or the session is unknown.

   Pinned in Task 9 (`hook_is_silent_and_exits_0_when_pdtd_is_down`).
4. **The update token never leaves api.github.com,** and a bad or missing checksum leaves nothing for `pdt-host`. The asset redirect is followed without `Authorization`, and no `request.json` is written. Pinned in Task 11 (`asset_redirect_drops_the_token`, `bad_checksum_hands_nothing_off`).
5. **A `pdtd` restart with active runs** restarts them on their old port, and a run whose port was taken meanwhile gets a new one. A resumed run that moves maps only its new port; the old port is never unserved (it may be a mapping this process did not make, such as stable's live run). Pinned in Task 7 (`active_runs_resume_on_restart`, `resume_moves_to_a_free_port_when_the_old_one_is_taken`).

## File structure

| Path | Responsibility |
|---|---|
| `crates/pdt-core/src/host.rs` | `HostSettings`, patch and validation, `UpdatePolicy`, `CredentialPolicy`, `PortRange`, `pick_port` |
| `crates/pdt-core/src/identity.rs` | Classify a request (local, owner, other), and the owner check |
| `crates/pdt-core/src/runconfig.rs` | Parse `.pdt/project.toml`, merge host overrides, validate, expand variables, build the run environment |
| `crates/pdt-core/src/run.rs` | `Run`, `RunKind`, `RunState`, `NewRun`, run path helpers |
| `crates/pdt-core/src/activity.rs` | `Activity`, `ActivityEvent`, `HookReport`, transitions, unseen rule, ANSI stripping and approval matching |
| `crates/pdt-core/src/release.rs` | `Version` (with local builds), `HostOs`, asset choice, `SHA256SUMS`, token warning |
| `crates/pdt-core/src/handoff.rs` | `UpdateRequest`, `UpdateResult`, the file names under `<data>/update/` |
| `crates/pdt-tailscale/` | The `TailscaleApi` trait, `TailscaleCli`, status parsing, `RecordingTailscale`, and the `fake-tailscale` binary |
| `crates/pdt-update/` | `GitHubClient` (latest release, token expiry, asset download with manual redirect), `fetch_verified` |
| `crates/pdt-host/` | The supervisor library (backoff, health, update install, rollback, self-replacement, registration), the `pdt-host` and `fake-pdtd` binaries |
| `crates/pdt-store/migrations/0004_host_settings.sql`, `0005_runs.sql`, `0006_session_activity.sql` | Schema |
| `crates/pdt-store/src/host.rs`, `runs.rs` | Host and project settings, runs |
| `crates/pdt-agents/src/hooks.rs` | The Claude settings JSON, the Codex `notify` argument, reading the user's Codex `notify` |
| `crates/pdt-api/src/secrets.rs` | `SecretStore`, `MemorySecretStore`, `KeyringStore` |
| `crates/pdt-api/src/host.rs` | Host settings, the token, whoami and checks routes |
| `crates/pdt-api/src/remote.rs` | The remote-access state machine (Tailscale serve, allowlist) |
| `crates/pdt-api/src/identity_guard.rs` | The identity middleware |
| `crates/pdt-api/src/project_settings.rs` | The project credential policy and run overrides |
| `crates/pdt-api/src/runs.rs`, `run_manager.rs` | Run routes, and the run lifecycle driver |
| `crates/pdt-api/src/activity.rs` | Hook endpoint, view endpoint, output and input heuristics |
| `crates/pdt-api/src/update.rs` | Update service, loop, routes |
| `crates/pdtd/src/hook.rs` | The `pdtd hook …` subcommand |
| `web/src/host/` | Settings → This host, the first-run checklist, the update banner |
| `web/src/runs/` | Run list, Run menu, run logs, run-config editor |
| `desktop/src-tauri/src/` | The tray (rewritten `main.rs`; `update.rs` deleted; `daemon.rs` starts `pdt-host`) |
| `.pdt/project.toml` | PDT's own run configuration |
| `docs/superpowers/notes/phase-2-agent-signals.md` | Task 1's findings |
| `docs/superpowers/notes/phase-2-mac-mini-checklist.md` | The hand-test steps for the Mac mini (hermes-agent) |

---

### Task 1: Verify agent signals against the installed CLIs

The spec (§5.1) requires the plan's first task to check the activity signals against the installed Claude Code and Codex 0.162.1. Claude's hooks were checked while this plan was written (facts 1 and 2). This task re-checks them quickly and does the Codex work: `notify` inside a PTY, and the text of an approval screen. Its output is a notes file and a fixture that Task 9's tests use.

**Files:**
- Create:
  - `docs/superpowers/notes/phase-2-agent-signals.md`
  - `crates/pdt-agents/tests/fixtures/codex-approval.txt` (raw PTY bytes)
  - `scratch/` (outside the repo: use your scratchpad dir)

**Interfaces:**
- Produces:
  - **The fixture:** `crates/pdt-agents/tests/fixtures/codex-approval.txt` holds raw PTY output captured while Codex 0.162.1 showed an approval prompt. Task 9's `approval_screen_is_recognised_in_a_real_capture` reads it.
  - **The notes file** records:
    - the Claude version and Codex version;
    - whether `notify` fired in a PTY, and its argv;
    - the approval prompt's visible lines;
    - the strings chosen for `CODEX_APPROVAL_MARKERS` (Task 9).

- [ ] **Step 1: Record versions**

Run (PowerShell): `claude --version; codex --version; (Get-Command codex).Source`
Expected: Claude ≥ 2.1.301; `codex-cli 0.162.1`. If `codex` resolves to the old desktop-app copy (CLAUDE.md), use the npm CLI's full path for the rest of this task and note it.

- [ ] **Step 2: Capture Codex `notify` in a real PTY**

Create a scratch folder (not in the repo) containing a git repo with one file, and a recorder script `rec.js`:

```js
// Appends its argv (one JSON line per call) to rec.log beside this script.
const fs = require('fs'), path = require('path');
fs.appendFileSync(path.join(__dirname, 'rec.log'), JSON.stringify(process.argv.slice(2)) + '\n');
```

From that folder, start Codex in a terminal (Windows Terminal or a PDT shell session on the dev daemon; **never** a session on port 7420 for this). Use a `-c notify` override that points at the recorder. The notify value is TOML, so on Windows write the path with forward slashes or doubled backslashes:

```
codex -c "notify=[\"node\",\"C:/path/to/scratch/rec.js\",\"--chain-test\"]"
```

Ask: `reply with the word ok`. Wait for the turn to finish, then quit Codex (Ctrl+C twice).
Expected: `rec.log` holds one line such as `["--chain-test","{\"type\":\"agent-turn-complete\",\"thread-id\":…}"]`. That shows the JSON is appended **after** the configured args, as one argument. Copy the line, with long text trimmed, into the notes file.

- [ ] **Step 3: Capture an approval screen**

In the same folder, start Codex with the default approval mode (if the user's config sets `approval_policy = "never"`, add `-c approval_policy="untrusted"` for this run). Ask: `run the shell command: git status`.

When the approval prompt appears, capture the raw screen bytes: run Codex inside a PDT dev-daemon shell session and save that session's scrollback. The easy path is to open `ws://127.0.0.1:7421/api/sessions/<id>/pty` with a small node script that writes every binary frame to `codex-approval.txt` until you press Ctrl+C. Then answer "No" (Esc) in Codex and quit.

Store the capture at `crates/pdt-agents/tests/fixtures/codex-approval.txt`. It is raw bytes, so add `crates/pdt-agents/tests/fixtures/*.txt -text` to `.gitattributes`.

- [ ] **Step 4: Write the notes**

`docs/superpowers/notes/phase-2-agent-signals.md`:

```markdown
# Phase 2: agent signals as verified on <date>

- Claude Code <version>. Hooks from `--settings` fire alongside the user's own hooks. Events seen: UserPromptSubmit,
  PreToolUse, PermissionRequest, Notification (<types seen>), Stop. Hook processes inherit the session environment.
  Exec form (`command` + `args`) works on Windows.
- Codex <version>. `-c notify=[…]` replaces the configured notify; the payload is appended as the last argv
  element: <trimmed example>. Fired once per completed turn inside a PTY: <yes/no>.
- Codex approval screen: visible lines <copy the lines>. Markers used by PDT (whitespace-insensitive, case-insensitive):
  <list>. Capture: crates/pdt-agents/tests/fixtures/codex-approval.txt.
```

Fill in every `<…>` from what you observed. Start the marker list from these candidates; keep the ones that appear on the captured screen, and add any other exact line it shows:
- `Would you like to run the following command?`
- `Would you like to make the following edits?`
- `Yes, proceed`
- `No, and tell Codex what to do differently`

If none of them appear, use the screen's own question and option lines.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/notes/phase-2-agent-signals.md crates/pdt-agents/tests/fixtures/codex-approval.txt .gitattributes
git commit -m "docs: record Claude hook and Codex notify/approval signals for Phase 2"
```

---
### Task 2: Host settings, the secret store and the settings API

**Files:**
- Create:
  - `crates/pdt-core/src/host.rs`
  - `crates/pdt-store/migrations/0004_host_settings.sql`
  - `crates/pdt-store/src/host.rs`
  - `crates/pdt-api/src/secrets.rs`
  - `crates/pdt-api/src/host.rs`
  - `crates/pdt-api/tests/host.rs`
- Modify:
  - `crates/pdt-core/src/lib.rs`, `crates/pdt-core/src/brand.rs`
  - `crates/pdt-store/src/lib.rs`, `crates/pdt-store/Cargo.toml`
  - `crates/pdt-api/src/lib.rs`, `crates/pdt-api/src/routes.rs`, `crates/pdt-api/src/pty_ws.rs` (test helper), `crates/pdt-api/Cargo.toml`
  - `crates/pdt-api/tests/{api,sessions,pty_ws,worktrees}.rs` (the `AppState::new` call)
  - `crates/pdtd/src/config.rs`, `crates/pdtd/src/main.rs`, `crates/pdtd/tests/lifecycle.rs`

**Interfaces:**
- Produces (`pdt_core::host`):
  - Enums:
    - `UpdatePolicy { AutoWhenIdle, Notify, Manual }`, serialized `auto_when_idle`, `notify` and `manual`.
    - `CredentialPolicy { Subscription, PassThrough }`, serialized `subscription` and `pass_through`. It has `as_str()` and `parse()`.
  - Structs:
    - `PortRange { start: u16, end: u16 }`.
    - `HostSettings { remote_access, owner_login: Option<String>, update_policy, run_ports: PortRange, max_concurrent_runs: u32, agent_credentials }`. It implements `Default` and `apply(&HostSettingsPatch) -> Result<HostSettings, HostSettingsError>`.
    - `HostSettingsPatch`: every field is optional, and `ownerLogin: ""` clears the owner.
  - Constants: `DEFAULT_RUN_PORTS`, `DEFAULT_MAX_RUNS = 4`.
- Produces (`pdt_core::brand`): `KEYCHAIN_SERVICE`.
- Produces (`pdt_store`):
  - `ProjectSettings { run_overrides: Option<String>, agent_credentials: Option<CredentialPolicy> }`.
  - `Store::get_host_settings`, `put_host_settings`, `get_project_settings`, `put_project_settings`.
- Produces (`pdt_api`):
  - `secrets::{SecretStore, MemorySecretStore, KeyringStore, UPDATE_TOKEN}`.
  - `AppState::new(store, data_dir: PathBuf)`. New public fields: `data_dir`, `secrets: Arc<dyn SecretStore>`, `owner_from_env: Option<String>`. New crate-private field: `settings_lock: Arc<tokio::sync::Mutex<()>>`.
  - `host::HostSettingsView { settings, ownerFromEnv, tokenPresent }`.
  - Routes `GET /api/host/settings` and `PUT /api/host/settings`.
  - Crate helpers `host::view(&AppState)` and `host::token_present(&AppState)`.
- Produces (`pdtd`): `Config.owner_login` (from `PDT_OWNER_LOGIN`) and `Config.memory_secrets` (from `PDT_SECRET_STORE=memory`).

- [ ] **Step 1: Write the failing `pdt-core` tests**

Add `pub mod host;` to `crates/pdt-core/src/lib.rs`, after `pub mod brand;`. Create `crates/pdt-core/src/host.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn patch() -> HostSettingsPatch {
        HostSettingsPatch::default()
    }

    #[test]
    fn defaults_match_the_spec() {
        let s = HostSettings::default();
        assert!(!s.remote_access);
        assert_eq!(s.owner_login, None);
        assert_eq!(s.update_policy, UpdatePolicy::Notify);
        assert_eq!(s.run_ports, PortRange { start: 8400, end: 8499 });
        assert_eq!(s.max_concurrent_runs, 4);
        assert_eq!(s.agent_credentials, CredentialPolicy::Subscription);
    }

    #[test]
    fn missing_fields_take_defaults_when_reading_old_json() {
        let s: HostSettings = serde_json::from_str(r#"{"remoteAccess":true}"#).unwrap();
        assert!(s.remote_access);
        assert_eq!(s.max_concurrent_runs, 4);
    }

    #[test]
    fn wire_names() {
        assert_eq!(serde_json::to_string(&UpdatePolicy::AutoWhenIdle).unwrap(), r#""auto_when_idle""#);
        assert_eq!(serde_json::to_string(&CredentialPolicy::PassThrough).unwrap(), r#""pass_through""#);
        for c in [CredentialPolicy::Subscription, CredentialPolicy::PassThrough] {
            assert_eq!(CredentialPolicy::parse(c.as_str()), Some(c));
        }
        assert_eq!(CredentialPolicy::parse("api"), None);
    }

    #[test]
    fn patch_applies_only_given_fields() {
        let s = HostSettings::default();
        let p = HostSettingsPatch {
            update_policy: Some(UpdatePolicy::Manual),
            owner_login: Some("  me@example.com ".into()),
            ..patch()
        };
        let out = s.apply(&p).unwrap();
        assert_eq!(out.update_policy, UpdatePolicy::Manual);
        assert_eq!(out.owner_login.as_deref(), Some("me@example.com"));
        assert_eq!(out.run_ports, s.run_ports);
        let cleared = out.apply(&HostSettingsPatch { owner_login: Some("  ".into()), ..patch() }).unwrap();
        assert_eq!(cleared.owner_login, None);
    }

    #[test]
    fn patch_validation() {
        let s = HostSettings::default();
        let ports = |start, end| HostSettingsPatch { run_ports: Some(PortRange { start, end }), ..patch() };
        assert_eq!(s.apply(&ports(9000, 8999)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(80, 90)), Err(HostSettingsError::BadPortRange));
        assert_eq!(s.apply(&ports(10000, 11000)), Err(HostSettingsError::BadPortRange), "1001 ports");
        assert!(s.apply(&ports(10000, 10999)).is_ok());
        let runs = |n| HostSettingsPatch { max_concurrent_runs: Some(n), ..patch() };
        assert_eq!(s.apply(&runs(0)), Err(HostSettingsError::BadMaxRuns));
        assert_eq!(s.apply(&runs(33)), Err(HostSettingsError::BadMaxRuns));
        assert!(s.apply(&runs(32)).is_ok());
        let owner = |o: &str| HostSettingsPatch { owner_login: Some(o.into()), ..patch() };
        assert_eq!(s.apply(&owner("two words")), Err(HostSettingsError::BadOwner));
        assert_eq!(s.apply(&owner("a\u{7}b")), Err(HostSettingsError::BadOwner));
    }
}
```

Run: `cargo test -p pdt-core host`
Expected: compile errors (the types are not defined yet).

- [ ] **Step 2: Implement `host.rs`, above the tests**

```rust
//! Per-host settings (spec §6). Pure: pdt-store stores them, pdt-api edits them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdatePolicy {
    AutoWhenIdle,
    Notify,
    Manual,
}

/// Whether agent sessions get the host's API-key variables (spec §6 "Agent credentials").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialPolicy {
    /// ANTHROPIC_API_KEY and OPENAI_API_KEY are removed, so agents use the logged-in subscription.
    Subscription,
    /// Agents get the host's environment unchanged (API keys, Bedrock/Vertex variables, proxies).
    PassThrough,
}

impl CredentialPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialPolicy::Subscription => "subscription",
            CredentialPolicy::PassThrough => "pass_through",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [CredentialPolicy::Subscription, CredentialPolicy::PassThrough].into_iter().find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

pub const DEFAULT_RUN_PORTS: PortRange = PortRange { start: 8400, end: 8499 };
pub const DEFAULT_MAX_RUNS: u32 = 4;
const MAX_RUNS_LIMIT: u32 = 32;
const MAX_PORTS_IN_RANGE: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct HostSettings {
    pub remote_access: bool,
    /// The Tailscale login allowed to use this host remotely. `PDT_OWNER_LOGIN` overrides it.
    pub owner_login: Option<String>,
    pub update_policy: UpdatePolicy,
    pub run_ports: PortRange,
    pub max_concurrent_runs: u32,
    pub agent_credentials: CredentialPolicy,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            remote_access: false,
            owner_login: None,
            update_policy: UpdatePolicy::Notify,
            run_ports: DEFAULT_RUN_PORTS,
            max_concurrent_runs: DEFAULT_MAX_RUNS,
            agent_credentials: CredentialPolicy::Subscription,
        }
    }
}

/// A partial update. `owner_login: Some("")` (or blank) clears the owner.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsPatch {
    #[serde(default)]
    #[ts(optional)]
    pub remote_access: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub owner_login: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub update_policy: Option<UpdatePolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub run_ports: Option<PortRange>,
    #[serde(default)]
    #[ts(optional)]
    pub max_concurrent_runs: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub agent_credentials: Option<CredentialPolicy>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HostSettingsError {
    #[error("the run port range must lie within 1024–65535, start at or below its end, and hold at most 1000 ports")]
    BadPortRange,
    #[error("the maximum number of concurrent runs must be between 1 and 32")]
    BadMaxRuns,
    #[error("the owner login must not contain spaces or control characters")]
    BadOwner,
}

impl HostSettings {
    pub fn apply(&self, p: &HostSettingsPatch) -> Result<HostSettings, HostSettingsError> {
        let mut s = self.clone();
        if let Some(v) = p.remote_access {
            s.remote_access = v;
        }
        if let Some(raw) = &p.owner_login {
            let login = raw.trim();
            if login.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(HostSettingsError::BadOwner);
            }
            s.owner_login = (!login.is_empty()).then(|| login.to_string());
        }
        if let Some(v) = p.update_policy {
            s.update_policy = v;
        }
        if let Some(r) = p.run_ports {
            let size = u32::from(r.end).saturating_sub(u32::from(r.start)) + 1;
            if r.start < 1024 || r.start > r.end || size > MAX_PORTS_IN_RANGE {
                return Err(HostSettingsError::BadPortRange);
            }
            s.run_ports = r;
        }
        if let Some(n) = p.max_concurrent_runs {
            if !(1..=MAX_RUNS_LIMIT).contains(&n) {
                return Err(HostSettingsError::BadMaxRuns);
            }
            s.max_concurrent_runs = n;
        }
        if let Some(c) = p.agent_credentials {
            s.agent_credentials = c;
        }
        Ok(s)
    }
}
```

Add this to `brand.rs`, under `RELEASE_REPO`:

```rust
/// Service name under which secrets (the update token) are stored in the OS keychain / credential manager.
pub const KEYCHAIN_SERVICE: &str = "dev.personaldevteam";
```

Run: `cargo test -p pdt-core host`
Expected: PASS (5 tests).

- [ ] **Step 3: Add the migration and write the failing store tests**

`crates/pdt-store/migrations/0004_host_settings.sql`:

```sql
-- Per-host settings (spec §6) as JSON under a key; today only key 'host'. Never synced between hosts.
CREATE TABLE host_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- Per-project host overrides: run configuration overrides (TOML text) and the agent credential policy.
CREATE TABLE project_settings (
    project_id        TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_overrides     TEXT,
    agent_credentials TEXT
);
```

Run (PowerShell): `cargo add serde_json -p pdt-store`. Cargo uses the workspace entry.

Create `crates/pdt-store/src/host.rs` with only the tests, and add `mod host;` and `pub use host::ProjectSettings;` to `lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::worktrees::tests::store_with_project;
    use crate::{ProjectSettings, StoreError};
    use pdt_core::host::{CredentialPolicy, HostSettings, UpdatePolicy};

    #[tokio::test]
    async fn host_settings_default_then_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.get_host_settings().await.unwrap(), HostSettings::default());
        let mut s = HostSettings::default();
        s.update_policy = UpdatePolicy::AutoWhenIdle;
        s.owner_login = Some("me@example.com".into());
        store.put_host_settings(&s).await.unwrap();
        store.put_host_settings(&s).await.unwrap(); // upsert
        assert_eq!(store.get_host_settings().await.unwrap(), s);
    }

    #[tokio::test]
    async fn corrupt_host_settings_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        sqlx::query("INSERT INTO host_settings (key, value) VALUES ('host', 'not json')")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(matches!(store.get_host_settings().await, Err(StoreError::Corrupt(_))));
    }

    #[tokio::test]
    async fn project_settings_roundtrip_and_cascade() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        let s = ProjectSettings {
            run_overrides: Some("[[run]]\nname = \"web\"\n".into()),
            agent_credentials: Some(CredentialPolicy::PassThrough),
        };
        store.put_project_settings("p", &s).await.unwrap();
        assert_eq!(store.get_project_settings("p").await.unwrap(), s);
        store.put_project_settings("p", &ProjectSettings::default()).await.unwrap();
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        assert!(store.put_project_settings("missing", &s).await.is_err(), "foreign key enforced");
    }
}
```

Run: `cargo test -p pdt-store host`
Expected: compile errors.

- [ ] **Step 4: Implement the store methods, above the tests**

```rust
use crate::{Store, StoreError};
use pdt_core::host::{CredentialPolicy, HostSettings};

const HOST_KEY: &str = "host";

/// A project's settings on this host (overrides of the repo's run configuration, credential policy).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectSettings {
    /// TOML in the `.pdt/project.toml` schema, merged over the repo's file (spec §4.1).
    pub run_overrides: Option<String>,
    /// None means "use the host's policy".
    pub agent_credentials: Option<CredentialPolicy>,
}

impl Store {
    pub async fn get_host_settings(&self) -> Result<HostSettings, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM host_settings WHERE key = ?")
            .bind(HOST_KEY)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            None => Ok(HostSettings::default()),
            Some((v,)) => serde_json::from_str(&v).map_err(|e| StoreError::Corrupt(format!("host settings: {e}"))),
        }
    }

    pub async fn put_host_settings(&self, s: &HostSettings) -> Result<(), StoreError> {
        let value = serde_json::to_string(s).expect("host settings serialize");
        sqlx::query(
            "INSERT INTO host_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(HOST_KEY)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_project_settings(&self, project_id: &str) -> Result<ProjectSettings, StoreError> {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT run_overrides, agent_credentials FROM project_settings WHERE project_id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some((run_overrides, creds)) = row else { return Ok(ProjectSettings::default()) };
        let agent_credentials = match creds {
            Some(c) => Some(
                CredentialPolicy::parse(&c).ok_or_else(|| StoreError::Corrupt(format!("credential policy {c:?}")))?,
            ),
            None => None,
        };
        Ok(ProjectSettings { run_overrides, agent_credentials })
    }

    pub async fn put_project_settings(&self, project_id: &str, s: &ProjectSettings) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO project_settings (project_id, run_overrides, agent_credentials) VALUES (?, ?, ?) \
             ON CONFLICT(project_id) DO UPDATE SET run_overrides = excluded.run_overrides, \
             agent_credentials = excluded.agent_credentials",
        )
        .bind(project_id)
        .bind(&s.run_overrides)
        .bind(s.agent_credentials.map(CredentialPolicy::as_str))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

Run: `cargo test -p pdt-store`
Expected: PASS, including every earlier test with 0004 applied.

- [ ] **Step 5: Add the secret store**

Run (PowerShell): `cargo add keyring -p pdt-api` and `cargo add thiserror -p pdt-api`.

`crates/pdt-api/src/secrets.rs`:

```rust
//! Secrets (the update token) live only in the OS keychain / credential manager (spec §3.2). Never logged, never
//! returned by the API, never written to a file or the database.

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain account holding the GitHub update token.
pub const UPDATE_TOKEN: &str = "update-token";

#[derive(Debug, thiserror::Error)]
#[error("the OS keychain failed: {0}")]
pub struct SecretError(pub String);

/// Blocking: call from `spawn_blocking` (macOS may show a keychain prompt).
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError>;
    /// Deleting a secret that does not exist succeeds.
    fn delete(&self, name: &str) -> Result<(), SecretError>;
}

/// For tests and `PDT_SECRET_STORE=memory`.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
        Ok(())
    }
}

/// Windows Credential Manager / macOS login keychain, through `keyring` (v1 API, verified fact 7).
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: &str) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, name: &str) -> Result<keyring::v1::Entry, SecretError> {
        keyring::v1::Entry::new(&self.service, name).map_err(|e| SecretError(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::v1::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.entry(name)?.set_password(value).map_err(|e| SecretError(e.to_string()))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemorySecretStore::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    /// Touches the real keychain: run by hand only (`cargo test -p pdt-api keyring -- --ignored`).
    #[test]
    #[ignore]
    fn keyring_store_roundtrip() {
        let s = KeyringStore::new("dev.personaldevteam.test");
        s.set("probe", "v").unwrap();
        assert_eq!(s.get("probe").unwrap().as_deref(), Some("v"));
        s.delete("probe").unwrap();
        assert_eq!(s.get("probe").unwrap(), None);
    }
}
```

The keychain test is ignored on purpose: CI must not touch a real keychain. If `keyring::v1::Entry` does not resolve, look up the re-export path in `cargo doc -p keyring`. Use `keyring::Entry` and `keyring::Error` if they exist at the root; fact 7 documents the names.

- [ ] **Step 6: Extend `AppState` and update every call site**

In `crates/pdt-api/src/routes.rs`, add these fields to `AppState`:

```rust
    /// pdtd's data directory (runs, per-session agent settings, update downloads).
    pub data_dir: std::path::PathBuf,
    /// Where secrets live. pdtd sets the keychain store; `new` uses an in-memory one (tests).
    pub secrets: std::sync::Arc<dyn crate::secrets::SecretStore>,
    /// `PDT_OWNER_LOGIN`: when set, it is the owner and Settings shows it read-only.
    pub owner_from_env: Option<String>,
    /// Serializes read-modify-write of host settings and the side effects applied with them.
    pub(crate) settings_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
```

Change the constructor to `pub fn new(store: Store, data_dir: std::path::PathBuf) -> Self`. It sets `data_dir`, `secrets: std::sync::Arc::new(crate::secrets::MemorySecretStore::default())`, `owner_from_env: None` and `settings_lock: Default::default()`.

Update the call sites:
- `src/pty_ws.rs` test: `AppState::new(Store::open(&tmp.path().join("pdt.db")).await.unwrap(), tmp.path().join("data"))`.
- `tests/api.rs`: `router(AppState::new(store, tmp.join("data")), web_dir)`.
- `tests/pty_ws.rs`, `tests/sessions.rs` and `tests/worktrees.rs`: `AppState::new(store, tmp.join("data"))`, using each helper's `tmp` path.

In `crates/pdt-api/src/lib.rs`, add `mod host;` and `pub mod secrets;`.

- [ ] **Step 7: Write the failing API tests**

`crates/pdt-api/tests/host.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn setup(tmp: &std::path::Path) -> (axum::Router, AppState) {
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.join("data"));
    (router(state.clone(), None), state)
}

#[tokio::test]
async fn settings_start_at_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (s, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["remoteAccess"], false);
    assert_eq!(v["settings"]["updatePolicy"], "notify");
    assert_eq!(v["settings"]["runPorts"], json!({"start": 8400, "end": 8499}));
    assert_eq!(v["settings"]["maxConcurrentRuns"], 4);
    assert_eq!(v["settings"]["agentCredentials"], "subscription");
    assert_eq!(v["ownerFromEnv"], Value::Null);
    assert_eq!(v["tokenPresent"], false);
}

#[tokio::test]
async fn settings_patch_persists_and_validates() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    let (s, v) = call(
        &app,
        "PUT",
        "/api/host/settings",
        Some(json!({"updatePolicy": "manual", "maxConcurrentRuns": 2, "ownerLogin": " me@example.com "})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"]["updatePolicy"], "manual");
    assert_eq!(v["settings"]["ownerLogin"], "me@example.com");
    assert_eq!(state.store.get_host_settings().await.unwrap().max_concurrent_runs, 2);

    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 0}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, _) = call(&app, "PUT", "/api/host/settings", Some(json!({"updatePolicy": "sometimes"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(state.store.get_host_settings().await.unwrap().max_concurrent_runs, 2, "unchanged");
}

#[tokio::test]
async fn token_presence_comes_from_the_secret_store() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state) = setup(tmp.path()).await;
    state.secrets.set(pdt_api::secrets::UPDATE_TOKEN, "github_pat_x").unwrap();
    let (_, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(v["tokenPresent"], true);
    assert!(!v.to_string().contains("github_pat_x"), "the token itself is never returned");
}
```

Run: `cargo test -p pdt-api --test host`
Expected: FAIL (`/api/host/settings` is a 404).

- [ ] **Step 8: Implement the routes**

`crates/pdt-api/src/host.rs`:

```rust
use crate::error::ApiError;
use crate::routes::AppState;
use crate::secrets::UPDATE_TOKEN;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::host::{HostSettings, HostSettingsError, HostSettingsPatch};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostSettingsView {
    pub settings: HostSettings,
    /// `PDT_OWNER_LOGIN`, which overrides `settings.ownerLogin` when set.
    pub owner_from_env: Option<String>,
    pub token_present: bool,
}

impl From<HostSettingsError> for ApiError {
    fn from(e: HostSettingsError) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string())
    }
}

pub(crate) async fn token_present(st: &AppState) -> Result<bool, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.get(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map(|t| t.is_some())
        .map_err(ApiError::internal)
}

pub(crate) async fn view(st: &AppState) -> Result<HostSettingsView, ApiError> {
    Ok(HostSettingsView {
        settings: st.store.get_host_settings().await?,
        owner_from_env: st.owner_from_env.clone(),
        token_present: token_present(st).await?,
    })
}

pub(crate) async fn get_settings(State(st): State<AppState>) -> Result<Json<HostSettingsView>, ApiError> {
    Ok(Json(view(&st).await?))
}

pub(crate) async fn put_settings(
    State(st): State<AppState>,
    payload: Result<Json<HostSettingsPatch>, JsonRejection>,
) -> Result<Json<HostSettingsView>, ApiError> {
    let Json(patch) = payload?;
    let _guard = st.settings_lock.lock().await;
    let next = st.store.get_host_settings().await?.apply(&patch)?;
    st.store.put_host_settings(&next).await?;
    drop(_guard);
    Ok(Json(view(&st).await?))
}
```

In `router`, add:

```rust
        .route("/host/settings", get(crate::host::get_settings).put(crate::host::put_settings))
```

Run: `cargo test -p pdt-api`
Expected: PASS, including the existing suites with the new constructor. `web/src/api/gen` gains `HostSettings.ts`, `HostSettingsPatch.ts`, `HostSettingsView.ts`, `PortRange.ts`, `UpdatePolicy.ts` and `CredentialPolicy.ts`.

- [ ] **Step 9: `pdtd` passes the owner and picks the secret store**

In `crates/pdtd/src/config.rs`, add these fields to `Config`:

```rust
    /// `PDT_OWNER_LOGIN`: the Tailscale login allowed in (dev instances get it from their run configuration).
    pub owner_login: Option<String>,
    /// `PDT_SECRET_STORE=memory`: keep secrets in memory (tests); otherwise the OS keychain.
    pub memory_secrets: bool,
```

Set them in `from_lookup`, and include them in the returned struct:

```rust
        let owner_login = lookup("PDT_OWNER_LOGIN").map(|v| v.trim().to_string());
        let memory_secrets = lookup("PDT_SECRET_STORE").as_deref() == Some("memory");
```

Add a test:

```rust
    #[test]
    fn owner_and_secret_store_from_env() {
        let c = Config::from_lookup(lookup(&[]), Some("/d".into()), None).unwrap();
        assert_eq!((c.owner_login, c.memory_secrets), (None, false));
        let c = Config::from_lookup(
            lookup(&[("PDT_OWNER_LOGIN", " me@example.com "), ("PDT_SECRET_STORE", "memory")]),
            Some("/d".into()),
            None,
        )
        .unwrap();
        assert_eq!((c.owner_login.as_deref(), c.memory_secrets), (Some("me@example.com"), true));
    }
```

In `main.rs`, replace `let state = AppState::new(store.clone());` with:

```rust
    let mut state = AppState::new(store.clone(), cfg.data_dir.clone());
    state.owner_from_env = cfg.owner_login.clone();
    state.secrets = if cfg.memory_secrets {
        std::sync::Arc::new(pdt_api::secrets::MemorySecretStore::default())
    } else {
        std::sync::Arc::new(pdt_api::secrets::KeyringStore::new(pdt_core::brand::KEYCHAIN_SERVICE))
    };
```

In `crates/pdtd/tests/lifecycle.rs`, in `start()`, add `.env("PDT_SECRET_STORE", "memory")` after the `PDT_DATA_DIR` line.

Run: `cargo test -p pdtd`
Expected: PASS.

- [ ] **Step 10: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green. `git status --porcelain -- web/src/api/gen` lists only the new type files.

```bash
git add crates web/src/api/gen
git commit -m "feat: per-host settings, project settings storage and keychain secret store"
```

---

### Task 3: Agent credential policy and `PDT_*` prefix scrubbing

**Files:**
- Create: `crates/pdt-api/src/project_settings.rs`, `crates/pdt-api/tests/project_settings.rs`
- Modify:
  - `crates/pdt-core/src/session.rs`, `crates/pdt-core/src/host.rs`
  - `crates/pdt-api/src/sessions.rs`, `crates/pdt-api/src/routes.rs`, `crates/pdt-api/src/lib.rs`
  - `docs/superpowers/notes/phase-1-carryover.md`

**Interfaces:**
- Consumes: `CredentialPolicy`, `HostSettings.agent_credentials` and `Store::{get,put}_project_settings` (Task 2).
- Produces:
  - `pdt_core::session::is_scrubbed(kind, name, creds: CredentialPolicy) -> bool`. Every name that starts with `PDT_` is scrubbed.
  - `pdt_core::host::effective_credentials(host, project: Option<CredentialPolicy>) -> CredentialPolicy`.
  - `pdt_api::project_settings`:
    - `ProjectSettingsView { agentCredentials: Option<CredentialPolicy>, effectiveCredentials, runOverrides: Option<String> }`.
    - `ProjectSettingsInput { agentCredentials: Option<CredentialPolicy>, runOverrides: Option<String> }`.
    - Routes `GET /api/projects/{id}/settings` and `PUT /api/projects/{id}/settings`. PUT replaces both fields.
  - `pdt_api::sessions::credentials_for(&AppState, project_id) -> Result<CredentialPolicy, ApiError>` (crate-private).

- [ ] **Step 1: Update the failing tests in `pdt-core`**

Replace `scrub_rules` in `crates/pdt-core/src/session.rs` with:

```rust
    #[test]
    fn scrub_rules() {
        use crate::host::CredentialPolicy::{PassThrough, Subscription};
        for kind in [SessionKind::Shell, SessionKind::AgentTui] {
            for creds in [Subscription, PassThrough] {
                for v in [
                    "PDT_PROFILE",
                    "PDT_PORT",
                    "PDT_DATA_DIR",
                    "PDT_WEB_DIR",
                    "PDT_ALLOWED_HOSTS",
                    "PDT_OWNER_LOGIN",
                    "PDT_SESSION_ID",
                    "PDT_HOOK_PORT",
                    "PDT_RUN_DATA",
                    "PDT_ANYTHING_NEW",
                ] {
                    assert!(is_scrubbed(kind, v, creds), "{kind:?}/{creds:?} keeps {v}");
                }
                for v in ["NO_COLOR", "FORCE_COLOR", "CLAUDECODE", "CLAUDE_PID", "CLAUDE_CODE_ENTRYPOINT"] {
                    assert!(is_scrubbed(kind, v, creds), "{kind:?} keeps {v}");
                }
                for v in ["PATH", "TERM", "PDT", "XPDT_PORT", "CLAUDE_CONFIG_DIR", "CLAUDE_CODE_USE_BEDROCK"] {
                    assert!(!is_scrubbed(kind, v, creds), "{kind:?} drops {v}");
                }
            }
        }
        assert!(is_scrubbed(SessionKind::AgentTui, "ANTHROPIC_API_KEY", Subscription));
        assert!(is_scrubbed(SessionKind::AgentTui, "OPENAI_API_KEY", Subscription));
        assert!(!is_scrubbed(SessionKind::AgentTui, "ANTHROPIC_API_KEY", PassThrough));
        assert!(!is_scrubbed(SessionKind::AgentTui, "OPENAI_API_KEY", PassThrough));
        assert!(!is_scrubbed(SessionKind::Shell, "ANTHROPIC_API_KEY", Subscription));
    }
```

Add to the tests in `crates/pdt-core/src/host.rs`:

```rust
    #[test]
    fn project_override_wins() {
        use CredentialPolicy::*;
        assert_eq!(effective_credentials(Subscription, None), Subscription);
        assert_eq!(effective_credentials(Subscription, Some(PassThrough)), PassThrough);
        assert_eq!(effective_credentials(PassThrough, Some(Subscription)), Subscription);
    }
```

Run: `cargo test -p pdt-core`
Expected: compile errors (`is_scrubbed` takes two arguments, and `effective_credentials` does not exist).

- [ ] **Step 2: Implement**

In `session.rs`, delete the `PDT_ENV_VARS` constant. Then replace `is_scrubbed` with:

```rust
/// pdtd's own configuration lives in `PDT_*` variables; none is ever passed to a child, so a dev pdtd started inside a
/// stable-hosted terminal cannot inherit the stable port, data dir or owner. PDT sets its own variables afterwards.
const PDT_ENV_PREFIX: &str = "PDT_";

/// Whether the inherited environment variable `name` must be removed from a child of this kind.
pub fn is_scrubbed(kind: SessionKind, name: &str, creds: crate::host::CredentialPolicy) -> bool {
    name.starts_with(PDT_ENV_PREFIX)
        || LEAKED_ENV_VARS.contains(&name)
        || (kind == SessionKind::AgentTui
            && creds == crate::host::CredentialPolicy::Subscription
            && API_KEY_ENV_VARS.contains(&name))
}
```

Change the doc comment on `API_KEY_ENV_VARS` to: `/// Removed for agents under the subscription policy, so they bill the user's subscription (spec §5.5; remote-host §6).`

In `host.rs`:

```rust
/// The project's own choice when it has one, else the host's.
pub fn effective_credentials(host: CredentialPolicy, project: Option<CredentialPolicy>) -> CredentialPolicy {
    project.unwrap_or(host)
}
```

Run: `cargo test -p pdt-core`
Expected: PASS.

- [ ] **Step 3: Thread the policy into session launches**

In `crates/pdt-api/src/sessions.rs`, add:

```rust
/// The credential policy for agents in this project (project override, else host setting).
pub(crate) async fn credentials_for(
    st: &AppState,
    project_id: &str,
) -> Result<pdt_core::host::CredentialPolicy, ApiError> {
    let host = st.store.get_host_settings().await?.agent_credentials;
    let project = st.store.get_project_settings(project_id).await?.agent_credentials;
    Ok(pdt_core::host::effective_credentials(host, project))
}
```

Then:
- Give `start_pty` a last parameter `creds: pdt_core::host::CredentialPolicy`, and use `.filter(|k| is_scrubbed(s.kind, k, creds))` inside it.
- In `create`, compute `let creds = credentials_for(&st, &session.project_id).await?;` right before `start_pty`, and pass it.
- In `restart_locked`, compute `let creds = credentials_for(st, &session.project_id).await?;` after loading the session, and pass it.

- [ ] **Step 4: Write the failing project-settings API test**

`crates/pdt-api/tests/project_settings.rs`. Copy the imports and the `call` helper from `tests/host.rs` verbatim, then add:

```rust
async fn setup(tmp: &std::path::Path) -> (axum::Router, String) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(
        std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success()
    );
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = pdt_api::router(pdt_api::AppState::new(store, tmp.join("data")), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    (app, p["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn project_credentials_override_the_host() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/settings");
    let (s, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["agentCredentials"], Value::Null);
    assert_eq!(v["effectiveCredentials"], "subscription");

    let body = |c: Value| Some(json!({"agentCredentials": c, "runOverrides": null}));
    let (s, v) = call(&app, "PUT", &uri, body(json!("pass_through"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["effectiveCredentials"], "pass_through");

    call(&app, "PUT", "/api/host/settings", Some(json!({"agentCredentials": "pass_through"}))).await;
    let (_, v) = call(&app, "PUT", &uri, body(json!("subscription"))).await;
    assert_eq!(v["effectiveCredentials"], "subscription", "a project can tighten the host policy too");
    let (_, v) = call(&app, "PUT", &uri, body(Value::Null)).await;
    assert_eq!(v["effectiveCredentials"], "pass_through", "null falls back to the host");

    let (s, _) = call(&app, "GET", "/api/projects/nope/settings", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
```

Run: `cargo test -p pdt-api --test project_settings`
Expected: FAIL (404).

- [ ] **Step 5: Implement the routes**

`crates/pdt-api/src/project_settings.rs`:

```rust
use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use pdt_core::host::{CredentialPolicy, effective_credentials};
use pdt_store::ProjectSettings;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsView {
    /// The project's own choice; null means it follows the host setting.
    pub agent_credentials: Option<CredentialPolicy>,
    pub effective_credentials: CredentialPolicy,
    /// Host overrides of the repo's `.pdt/project.toml`, as TOML text.
    pub run_overrides: Option<String>,
}

/// Replaces both fields.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSettingsInput {
    pub agent_credentials: Option<CredentialPolicy>,
    pub run_overrides: Option<String>,
}

pub(crate) async fn view(st: &AppState, project_id: &str) -> Result<ProjectSettingsView, ApiError> {
    st.store.get_project(project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let p = st.store.get_project_settings(project_id).await?;
    let host = st.store.get_host_settings().await?.agent_credentials;
    Ok(ProjectSettingsView {
        agent_credentials: p.agent_credentials,
        effective_credentials: effective_credentials(host, p.agent_credentials),
        run_overrides: p.run_overrides,
    })
}

pub(crate) async fn get_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSettingsView>, ApiError> {
    Ok(Json(view(&st, &id).await?))
}

pub(crate) async fn put_settings(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ProjectSettingsInput>, JsonRejection>,
) -> Result<Json<ProjectSettingsView>, ApiError> {
    let Json(input) = payload?;
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let run_overrides = input.run_overrides.filter(|t| !t.trim().is_empty());
    st.store
        .put_project_settings(&id, &ProjectSettings { run_overrides, agent_credentials: input.agent_credentials })
        .await?;
    Ok(Json(view(&st, &id).await?))
}
```

Register `mod project_settings;` in `lib.rs`. In `router`, add the route `.route("/projects/{id}/settings", get(crate::project_settings::get_settings).put(crate::project_settings::put_settings))`. Task 6 adds validation of `runOverrides`.

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 6: Close the carry-over item, run the full check and commit**

In `docs/superpowers/notes/phase-1-carryover.md`, under "Duplicated lists and lock entries", replace the `PDT_ENV_VARS` bullet with: `Done in Phase 2 (Task 3): every PDT_* variable is scrubbed by prefix.`

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add crates web/src/api/gen docs/superpowers/notes/phase-1-carryover.md
git commit -m "feat: per-host and per-project agent credential policy; scrub PDT_* by prefix"
```

---

### Task 4: Tailscale wrapper and the remote-access toggle

**Files:**
- Create:
  - `crates/pdt-tailscale/Cargo.toml`, `crates/pdt-tailscale/src/lib.rs`
  - `crates/pdt-tailscale/src/bin/fake-tailscale.rs`
  - `crates/pdt-tailscale/tests/cli.rs`
  - `crates/pdt-api/src/remote.rs`
  - `crates/pdt-api/tests/remote.rs`
- Modify:
  - `Cargo.toml` (workspace: dependency entry and `default-members`)
  - `crates/pdt-api/Cargo.toml`, `crates/pdt-api/src/{host_guard,routes,host,lib}.rs`
  - `crates/pdtd/src/main.rs`

**Interfaces:**
- Consumes: `HostSettings.remote_access` and `owner_login`, `AppState.settings_lock`, `host::view` (Task 2).
- Produces (`pdt_tailscale`):
  - The trait `TailscaleApi: Send + Sync { status() -> Result<TsStatus, TsError>; serve(https_port: u16, target_port: u16) -> Result<(), TsError>; unserve(https_port: u16) -> Result<(), TsError> }`. It is blocking.
  - `TsStatus { dns_name: String /* no trailing dot */, self_login: Option<String>, https_enabled: bool }`.
  - `TsError { NotInstalled, NotRunning(String), HttpsDisabled, Command(String), BadStatus(String) }`.
  - `TailscaleCli::{new(program), detect(login_path: Option<&str>), with_env(k, v)}`.
  - `parse_status(&str)`, `serve_args`, `unserve_args`.
  - `testing::RecordingTailscale::{new(Result<TsStatus, TsError>), calls() -> Vec<String>, set_status(..)}`.
- Produces (`pdt_api`):
  - `AllowedHosts` (shared, mutable) with `new`, `contains`, `add`, `remove` and `snapshot`. `with_host_guard(router, impl Into<AllowedHosts>)`.
  - New public `AppState` fields:
    - `listen_port: u16`;
    - `allowed_hosts: AllowedHosts`;
    - `tailscale: Arc<dyn TailscaleApi>`;
    - `remote: Arc<std::sync::Mutex<RemoteStatus>>`.
  - `remote::RemoteStatus { enabled, active, url, dnsName, error, tailscaleLogin }`.
  - `pub async fn apply_remote_access(&AppState) -> RemoteStatus` (exported).
  - `HostSettingsView.remote: RemoteStatus`.
  - Routes `GET /api/host/remote` (cached) and `POST /api/host/remote/refresh` (re-applies).
  - Error 400 `owner_required`.
  - The crate hook `remote::extra_ports(&AppState) -> Vec<u16>`. It returns empty here; Task 7 fills it with web-run ports.

- [ ] **Step 1: Create the crate and write the failing unit tests**

Create `crates/pdt-tailscale/Cargo.toml`:

```toml
[package]
name = "pdt-tailscale"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true
```

Run (PowerShell, repo root): `cargo add serde_json thiserror -p pdt-tailscale`, then `cargo add which -p pdt-tailscale`, then `cargo add tempfile --dev -p pdt-tailscale`.

In the root `Cargo.toml`:
- add `"crates/pdt-tailscale"` to `default-members`;
- add `pdt-tailscale = { path = "crates/pdt-tailscale" }` under `[workspace.dependencies]`.

`crates/pdt-tailscale/src/lib.rs`, tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
      "BackendState": "Running",
      "Self": { "DNSName": "box.tail1234.ts.net.", "UserID": 42, "HostName": "box" },
      "User": { "42": { "ID": 42, "LoginName": "me@example.com", "DisplayName": "Me" } },
      "CertDomains": ["box.tail1234.ts.net"],
      "MagicDNSSuffix": "tail1234.ts.net"
    }"#;

    #[test]
    fn parses_running_status() {
        let s = parse_status(STATUS).unwrap();
        assert_eq!(s.dns_name, "box.tail1234.ts.net");
        assert_eq!(s.self_login.as_deref(), Some("me@example.com"));
        assert!(s.https_enabled);
    }

    #[test]
    fn https_disabled_when_cert_domains_empty_or_null() {
        let empty = STATUS.replace(r#"["box.tail1234.ts.net"]"#, "[]");
        assert!(!parse_status(&empty).unwrap().https_enabled);
        let null = STATUS.replace(r#"["box.tail1234.ts.net"]"#, "null");
        assert!(!parse_status(&null).unwrap().https_enabled);
    }

    #[test]
    fn not_running_and_garbage() {
        let stopped = STATUS.replace(r#""Running""#, r#""NeedsLogin""#);
        assert!(matches!(parse_status(&stopped), Err(TsError::NotRunning(s)) if s.contains("NeedsLogin")));
        assert!(matches!(parse_status("nope"), Err(TsError::BadStatus(_))));
        let no_dns = STATUS.replace(r#""box.tail1234.ts.net.""#, r#""""#);
        assert!(matches!(parse_status(&no_dns), Err(TsError::BadStatus(_))));
    }

    #[test]
    fn missing_user_entry_is_no_login() {
        let other = STATUS.replace(r#""42": {"#, r#""7": {"#);
        assert_eq!(parse_status(&other).unwrap().self_login, None);
    }

    #[test]
    fn command_lines() {
        assert_eq!(serve_args(443, 7420), ["serve", "--bg", "--https=443", "http://127.0.0.1:7420"]);
        assert_eq!(unserve_args(8401), ["serve", "--https=8401", "off"]);
    }

    #[test]
    fn classifies_cli_failures() {
        assert!(matches!(
            classify_failure("failed to connect to local Tailscale daemon for /localapi/v0/status; not running?"),
            TsError::NotRunning(_)
        ));
        assert!(matches!(classify_failure("Tailscale is stopped."), TsError::NotRunning(_)));
        assert!(matches!(classify_failure("something else"), TsError::Command(s) if s == "something else"));
    }

    #[test]
    fn recording_fake_records_calls() {
        let f = testing::RecordingTailscale::new(Ok(parse_status(STATUS).unwrap()));
        f.serve(443, 7421).unwrap();
        f.unserve(443).unwrap();
        assert_eq!(f.calls(), ["serve --bg --https=443 http://127.0.0.1:7421", "serve --https=443 off"]);
        f.set_status(Err(TsError::NotInstalled));
        assert!(matches!(f.status(), Err(TsError::NotInstalled)));
    }
}
```

Run: `cargo test -p pdt-tailscale`
Expected: compile errors.

- [ ] **Step 2: Implement the library, above the tests**

```rust
//! Thin wrapper over the `tailscale` CLI (verified fact 5). Blocking; call from `spawn_blocking` in async code.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsStatus {
    /// `Self.DNSName` without its trailing dot, e.g. `box.tail1234.ts.net`.
    pub dns_name: String,
    /// The login of the user this machine is logged in as (offered as the host owner).
    pub self_login: Option<String>,
    /// HTTPS certificates are enabled for the tailnet (`CertDomains` is non-empty); `serve --https` needs them.
    pub https_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TsError {
    #[error("tailscale CLI not found")]
    NotInstalled,
    #[error("Tailscale is not running ({0})")]
    NotRunning(String),
    #[error("HTTPS certificates are not enabled for this tailnet (Tailscale admin console → DNS → Enable HTTPS)")]
    HttpsDisabled,
    #[error("tailscale failed: {0}")]
    Command(String),
    #[error("unexpected tailscale status output: {0}")]
    BadStatus(String),
}

pub trait TailscaleApi: Send + Sync {
    fn status(&self) -> Result<TsStatus, TsError>;
    /// Maps `https://<machine>:<https_port>/` to `http://127.0.0.1:<target_port>` in the background (persists).
    fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError>;
    /// Removes the mapping on `https_port`.
    fn unserve(&self, https_port: u16) -> Result<(), TsError>;
}

pub fn serve_args(https_port: u16, target_port: u16) -> Vec<String> {
    vec![
        "serve".into(),
        "--bg".into(),
        format!("--https={https_port}"),
        format!("http://127.0.0.1:{target_port}"),
    ]
}

/// "add `off` to the end … the target is optional, but all original flags are required" (fact 5).
pub fn unserve_args(https_port: u16) -> Vec<String> {
    vec!["serve".into(), format!("--https={https_port}"), "off".into()]
}

pub fn parse_status(json: &str) -> Result<TsStatus, TsError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| TsError::BadStatus(e.to_string()))?;
    let state = v["BackendState"].as_str().unwrap_or("");
    if state != "Running" {
        return Err(TsError::NotRunning(format!("state {state}")));
    }
    let dns = v["Self"]["DNSName"].as_str().unwrap_or("").trim_end_matches('.').to_string();
    if dns.is_empty() {
        return Err(TsError::BadStatus("Self.DNSName is empty (is MagicDNS enabled?)".into()));
    }
    let self_login = match &v["Self"]["UserID"] {
        serde_json::Value::Null => None,
        id => {
            let key = id.as_u64().map(|n| n.to_string()).or_else(|| id.as_str().map(String::from));
            key.and_then(|k| v["User"][&k]["LoginName"].as_str().map(String::from))
        }
    };
    let https_enabled = v["CertDomains"].as_array().is_some_and(|a| !a.is_empty());
    Ok(TsStatus { dns_name: dns, self_login, https_enabled })
}

fn classify_failure(stderr: &str) -> TsError {
    let lower = stderr.to_lowercase();
    if lower.contains("failed to connect") || lower.contains("not running") || lower.contains("is stopped") {
        TsError::NotRunning(stderr.trim().to_string())
    } else {
        TsError::Command(stderr.trim().to_string())
    }
}

pub struct TailscaleCli {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
}

impl TailscaleCli {
    pub fn new(program: PathBuf) -> Self {
        Self { program: Some(program), env: Vec::new() }
    }

    /// Finds the CLI on `login_path` (or pdtd's PATH), then in the standard install locations.
    pub fn detect(login_path: Option<&str>) -> Self {
        let paths = login_path.map(std::ffi::OsString::from).or_else(|| std::env::var_os("PATH"));
        let on_path = std::env::current_dir().ok().and_then(|cwd| which::which_in("tailscale", paths, cwd).ok());
        let program = on_path.or_else(|| candidates().into_iter().find(|p| p.is_file()));
        Self { program, env: Vec::new() }
    }

    /// Sets a variable for every invocation (tests point the fake CLI at its files this way).
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn run(&self, args: &[String]) -> Result<String, TsError> {
        let program = self.program.as_ref().ok_or(TsError::NotInstalled)?;
        let mut cmd = Command::new(program);
        cmd.args(args).envs(self.env.iter().map(|(k, v)| (k, v)));
        no_window(&mut cmd);
        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound { TsError::NotInstalled } else { TsError::Command(e.to_string()) }
        })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if err.is_empty() {
                err = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
            Err(classify_failure(&err))
        }
    }
}

fn candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe")]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
            PathBuf::from("/opt/homebrew/bin/tailscale"),
            PathBuf::from("/usr/local/bin/tailscale"),
        ]
    } else {
        vec![PathBuf::from("/usr/bin/tailscale")]
    }
}

fn no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

impl TailscaleApi for TailscaleCli {
    fn status(&self) -> Result<TsStatus, TsError> {
        parse_status(&self.run(&["status".into(), "--json".into()])?)
    }
    fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError> {
        self.run(&serve_args(https_port, target_port)).map(|_| ())
    }
    fn unserve(&self, https_port: u16) -> Result<(), TsError> {
        self.run(&unserve_args(https_port)).map(|_| ())
    }
}

pub mod testing {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory TailscaleApi that records `serve` calls as command lines, for other crates' tests.
    pub struct RecordingTailscale {
        status: Mutex<Result<TsStatus, TsError>>,
        calls: Mutex<Vec<String>>,
    }

    impl RecordingTailscale {
        pub fn new(status: Result<TsStatus, TsError>) -> Self {
            Self { status: Mutex::new(status), calls: Mutex::new(Vec::new()) }
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        pub fn set_status(&self, status: Result<TsStatus, TsError>) {
            *self.status.lock().unwrap() = status;
        }
    }

    impl TailscaleApi for RecordingTailscale {
        fn status(&self) -> Result<TsStatus, TsError> {
            self.status.lock().unwrap().clone()
        }
        fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError> {
            self.calls.lock().unwrap().push(serve_args(https_port, target_port).join(" "));
            Ok(())
        }
        fn unserve(&self, https_port: u16) -> Result<(), TsError> {
            self.calls.lock().unwrap().push(unserve_args(https_port).join(" "));
            Ok(())
        }
    }
}
```

Run: `cargo test -p pdt-tailscale --lib`
Expected: PASS.

- [ ] **Step 3: Add the fake CLI and its integration test**

`crates/pdt-tailscale/src/bin/fake-tailscale.rs`:

```rust
//! Test double for the `tailscale` CLI. Appends each argv (space-joined) to $FAKE_TAILSCALE_LOG; answers
//! `status --json` with the contents of $FAKE_TAILSCALE_STATUS; fails like a stopped daemon when
//! $FAKE_TAILSCALE_DOWN is set.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(log) = std::env::var("FAKE_TAILSCALE_LOG") {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(log).expect("log");
        writeln!(f, "{}", args.join(" ")).expect("write log");
    }
    if std::env::var_os("FAKE_TAILSCALE_DOWN").is_some() {
        eprintln!("failed to connect to local Tailscale daemon for /localapi/v0/status; not running?");
        std::process::exit(1);
    }
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status", "--json"] => {
            let file = std::env::var("FAKE_TAILSCALE_STATUS").expect("FAKE_TAILSCALE_STATUS");
            print!("{}", std::fs::read_to_string(file).expect("status file"));
        }
        ["serve", ..] => {}
        other => {
            eprintln!("fake-tailscale: unexpected arguments {other:?}");
            std::process::exit(2);
        }
    }
}
```

`crates/pdt-tailscale/tests/cli.rs`:

```rust
use pdt_tailscale::{TailscaleApi, TailscaleCli, TsError};
use std::path::PathBuf;

fn fake(tmp: &std::path::Path) -> (TailscaleCli, PathBuf) {
    let status = tmp.join("status.json");
    std::fs::write(
        &status,
        r#"{"BackendState":"Running","Self":{"DNSName":"box.t.ts.net.","UserID":1},
            "User":{"1":{"LoginName":"me@example.com"}},"CertDomains":["box.t.ts.net"]}"#,
    )
    .unwrap();
    let log = tmp.join("calls.log");
    let cli = TailscaleCli::new(PathBuf::from(env!("CARGO_BIN_EXE_fake-tailscale")))
        .with_env("FAKE_TAILSCALE_STATUS", &status.to_string_lossy())
        .with_env("FAKE_TAILSCALE_LOG", &log.to_string_lossy());
    (cli, log)
}

#[test]
fn status_serve_and_unserve_go_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let (cli, log) = fake(tmp.path());
    let s = cli.status().unwrap();
    assert_eq!((s.dns_name.as_str(), s.self_login.as_deref()), ("box.t.ts.net", Some("me@example.com")));
    cli.serve(443, 7421).unwrap();
    cli.unserve(443).unwrap();
    let calls = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        ["status --json", "serve --bg --https=443 http://127.0.0.1:7421", "serve --https=443 off"]
    );
}

#[test]
fn a_stopped_daemon_and_a_missing_cli_are_typed() {
    let tmp = tempfile::tempdir().unwrap();
    let (cli, _) = fake(tmp.path());
    let down = cli.with_env("FAKE_TAILSCALE_DOWN", "1");
    assert!(matches!(down.status(), Err(TsError::NotRunning(_))));
    let missing = TailscaleCli::new(tmp.path().join("no-such-tailscale"));
    assert!(matches!(missing.status(), Err(TsError::NotInstalled)));
}
```

Run: `cargo test -p pdt-tailscale`
Expected: PASS.

- [ ] **Step 4: Make the allowlist shared and mutable**

Replace the guard setup in `crates/pdt-api/src/host_guard.rs` with this. The middleware body (the Host check and the Origin check) is unchanged except for the lookups:

```rust
/// The Host/Origin allowlist, shared so remote access can add the tailnet name at runtime. Lower-case `host[:port]`.
#[derive(Clone, Default)]
pub struct AllowedHosts(Arc<std::sync::RwLock<Vec<String>>>);

impl AllowedHosts {
    pub fn new(hosts: Vec<String>) -> Self {
        Self(Arc::new(std::sync::RwLock::new(hosts.into_iter().map(|h| h.to_ascii_lowercase()).collect())))
    }
    pub fn contains(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.0.read().unwrap_or_else(|e| e.into_inner()).iter().any(|h| *h == host)
    }
    pub fn add(&self, host: &str) {
        let host = host.to_ascii_lowercase();
        let mut v = self.0.write().unwrap_or_else(|e| e.into_inner());
        if !v.contains(&host) {
            v.push(host);
        }
    }
    pub fn remove(&self, host: &str) {
        let host = host.to_ascii_lowercase();
        self.0.write().unwrap_or_else(|e| e.into_inner()).retain(|h| *h != host);
    }
    pub fn snapshot(&self) -> Vec<String> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl From<Vec<String>> for AllowedHosts {
    fn from(v: Vec<String>) -> Self {
        AllowedHosts::new(v)
    }
}

pub fn with_host_guard(router: Router, allowed: impl Into<AllowedHosts>) -> Router {
    let allowed: AllowedHosts = allowed.into();
    router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let allowed = allowed.clone();
        async move {
            let headers = req.headers();
            if let Some(host) = headers.get(header::HOST) {
                let host = String::from_utf8_lossy(host.as_bytes()).into_owned();
                if !allowed.contains(&host) {
                    return ApiError::new(StatusCode::FORBIDDEN, "forbidden_host", format!("host {host} is not allowed"))
                        .into_response();
                }
            }
            if let Some(origin) = headers.get(header::ORIGIN) {
                let origin = String::from_utf8_lossy(origin.as_bytes()).into_owned();
                if !origin_allowed(&origin, &allowed) {
                    return ApiError::new(
                        StatusCode::FORBIDDEN,
                        "forbidden_origin",
                        format!("origin {origin} is not allowed"),
                    )
                    .into_response();
                }
            }
            let res: Response = next.run(req).await;
            res
        }
    }))
}

/// An Origin is allowed when it is http(s) and its `host[:port]` is an allowed Host value.
fn origin_allowed(origin: &str, allowed: &AllowedHosts) -> bool {
    let lower = origin.to_ascii_lowercase();
    let Some(authority) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) else {
        return false;
    };
    !authority.is_empty() && !authority.contains('/') && allowed.contains(authority)
}
```

Export it: `pub use host_guard::{AllowedHosts, with_host_guard};` in `lib.rs`. The existing tests pass `Vec<String>` and still compile.

- [ ] **Step 5: Add the `AppState` fields and the remote module skeleton**

Run (PowerShell): `cargo add pdt-tailscale -p pdt-api`.

Add to `AppState`:

```rust
    /// The port pdtd listens on (0 in tests that do not serve). Tailscale maps 443 to it.
    pub listen_port: u16,
    /// The live Host/Origin allowlist (the guard and remote access share it).
    pub allowed_hosts: crate::host_guard::AllowedHosts,
    pub tailscale: std::sync::Arc<dyn pdt_tailscale::TailscaleApi>,
    /// The last result of applying the remote-access setting.
    pub remote: std::sync::Arc<std::sync::Mutex<crate::remote::RemoteStatus>>,
```

In `new`:
- `listen_port: 0`;
- `allowed_hosts: Default::default()`;
- `remote: Default::default()`;
- for `tailscale`, construct `platform` first and then use `std::sync::Arc::new(pdt_tailscale::TailscaleCli::detect(platform.login_path.as_deref()))`.

Write `crates/pdt-api/tests/remote.rs`. It needs the same `call` helper as `tests/host.rs`, plus:

```rust
use pdt_api::{AllowedHosts, AppState, router, with_host_guard};
use pdt_tailscale::testing::RecordingTailscale;
use pdt_tailscale::{TsError, TsStatus};
use std::sync::Arc;

fn ts(https: bool) -> TsStatus {
    TsStatus { dns_name: "box.t.ts.net".into(), self_login: Some("me@example.com".into()), https_enabled: https }
}

async fn setup(tmp: &std::path::Path, fake: Arc<RecordingTailscale>) -> (axum::Router, AppState) {
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.listen_port = 7999;
    state.allowed_hosts = AllowedHosts::new(vec!["127.0.0.1:7999".into()]);
    state.tailscale = fake;
    let app = with_host_guard(router(state.clone(), None), state.allowed_hosts.clone());
    (app, state)
}

/// `call` sends no Host header; this one sends `Host: box.t.ts.net` like `tailscale serve` does.
async fn get_as_tailnet_host(app: &axum::Router) -> StatusCode {
    let req = Request::builder().uri("/api/health").header("host", "box.t.ts.net").body(Body::empty()).unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn enabling_needs_an_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, _) = setup(tmp.path(), fake.clone()).await;
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("owner_required")), "{v}");
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn enabling_serves_443_and_allows_the_tailnet_name() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(true))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);
    let (s, v) = call(
        &app,
        "PUT",
        "/api/host/settings",
        Some(json!({"remoteAccess": true, "ownerLogin": "me@example.com"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["active"], true);
    assert_eq!(v["remote"]["url"], "https://box.t.ts.net/");
    assert_eq!(v["remote"]["tailscaleLogin"], "me@example.com");
    assert_eq!(fake.calls(), ["serve --bg --https=443 http://127.0.0.1:7999"]);
    assert!(state.allowed_hosts.contains("box.t.ts.net"));
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::OK);

    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"remoteAccess": false}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remote"]["active"], false);
    assert_eq!(fake.calls().last().unwrap(), "serve --https=443 off");
    assert_eq!(get_as_tailnet_host(&app).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tailscale_problems_are_reported_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(RecordingTailscale::new(Ok(ts(false))));
    let (app, state) = setup(tmp.path(), fake.clone()).await;
    let body = json!({"remoteAccess": true, "ownerLogin": "me@example.com"});
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["remote"]["enabled"].clone(), v["remote"]["active"].clone()), (json!(true), json!(false)));
    assert!(v["remote"]["error"].as_str().unwrap().contains("HTTPS"), "{v}");
    assert!(!state.allowed_hosts.contains("box.t.ts.net"));

    fake.set_status(Err(TsError::NotInstalled));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert!(v["error"].as_str().unwrap().contains("not found"), "{v}");
    fake.set_status(Ok(ts(true)));
    let (_, v) = call(&app, "POST", "/api/host/remote/refresh", None).await;
    assert_eq!(v["active"], true, "retry succeeds once Tailscale is fixed");
    let (_, v) = call(&app, "GET", "/api/host/remote", None).await;
    assert_eq!(v["active"], true);
}
```

Also import `axum::body::Body`, `axum::http::Request` and `tower::ServiceExt`, as `tests/host.rs` does.

Run: `cargo test -p pdt-api --test remote`
Expected: compile errors or FAIL (no `remote` module yet).

- [ ] **Step 6: Implement remote access**

`crates/pdt-api/src/remote.rs`:

```rust
//! Remote access (spec §2): `tailscale serve` maps https://<machine>.<tailnet>.ts.net/ to pdtd, and the tailnet name
//! joins the Host/Origin allowlist. The setting is the truth; this makes reality match it and reports what happened.

use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::State;
use pdt_tailscale::TsError;
use serde::Serialize;
use ts_rs::TS;

pub const HOST_HTTPS_PORT: u16 = 443;

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteStatus {
    /// The setting.
    pub enabled: bool,
    /// The mapping is in place and the tailnet name is allowed.
    pub active: bool,
    pub url: Option<String>,
    pub dns_name: Option<String>,
    /// Why it is not active (e.g. "Tailscale is not running", "tailscale CLI not found").
    pub error: Option<String>,
    /// The login this machine's Tailscale is signed in as (offered as the owner).
    pub tailscale_login: Option<String>,
}

async fn blocking<T: Send + 'static>(
    st: &AppState,
    f: impl FnOnce(&dyn pdt_tailscale::TailscaleApi) -> Result<T, TsError> + Send + 'static,
) -> Result<T, TsError> {
    let ts = st.tailscale.clone();
    tokio::task::spawn_blocking(move || f(ts.as_ref())).await.unwrap_or_else(|e| Err(TsError::Command(e.to_string())))
}

/// Ports of other things to map alongside pdtd (web runs, Task 7).
pub(crate) async fn extra_ports(_st: &AppState) -> Vec<u16> {
    Vec::new()
}

/// Applies the stored setting: serve or unserve, update the allowlist, and record the outcome.
pub async fn apply_remote_access(st: &AppState) -> RemoteStatus {
    let enabled = st.store.get_host_settings().await.map(|s| s.remote_access).unwrap_or(false);
    let status = blocking(st, |t| t.status()).await;
    let previous_dns = st.remote.lock().unwrap_or_else(|e| e.into_inner()).dns_name.clone();
    let mut out = RemoteStatus {
        enabled,
        tailscale_login: status.as_ref().ok().and_then(|s| s.self_login.clone()),
        ..Default::default()
    };
    if enabled {
        match status {
            Err(e) => out.error = Some(e.to_string()),
            Ok(s) if !s.https_enabled => out.error = Some(TsError::HttpsDisabled.to_string()),
            Ok(s) => {
                let port = st.listen_port;
                match blocking(st, move |t| t.serve(HOST_HTTPS_PORT, port)).await {
                    Err(e) => out.error = Some(e.to_string()),
                    Ok(()) => {
                        for p in extra_ports(st).await {
                            if let Err(e) = blocking(st, move |t| t.serve(p, p)).await {
                                tracing::warn!(port = p, error = %e, "could not map a run port");
                            }
                        }
                        if let Some(old) = previous_dns.filter(|d| *d != s.dns_name) {
                            st.allowed_hosts.remove(&old);
                        }
                        st.allowed_hosts.add(&s.dns_name);
                        out.active = true;
                        out.url = Some(format!("https://{}/", s.dns_name));
                        out.dns_name = Some(s.dns_name);
                    }
                }
            }
        }
    } else {
        if let Some(dns) = &previous_dns {
            st.allowed_hosts.remove(dns);
        }
        if let Ok(s) = &status {
            st.allowed_hosts.remove(&s.dns_name);
            // Removing a mapping that is not there fails harmlessly; log it only.
            if let Err(e) = blocking(st, |t| t.unserve(HOST_HTTPS_PORT)).await {
                tracing::debug!(error = %e, "unserve 443");
            }
            for p in extra_ports(st).await {
                let _ = blocking(st, move |t| t.unserve(p)).await;
            }
        }
    }
    *st.remote.lock().unwrap_or_else(|e| e.into_inner()) = out.clone();
    out
}

pub(crate) async fn get_remote(State(st): State<AppState>) -> Json<RemoteStatus> {
    Json(st.remote.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

pub(crate) async fn refresh(State(st): State<AppState>) -> Result<Json<RemoteStatus>, ApiError> {
    let _guard = st.settings_lock.lock().await;
    Ok(Json(apply_remote_access(&st).await))
}
```

In `host.rs`:
- add `pub remote: crate::remote::RemoteStatus` to `HostSettingsView`, and fill it from `st.remote` in `view`;
- in `put_settings`, after `apply(&patch)?` and before storing, add:

```rust
    let effective_owner = st.owner_from_env.clone().or(next.owner_login.clone());
    if next.remote_access && effective_owner.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "owner_required",
            "set the owner (a Tailscale login) before turning on remote access",
        ));
    }
```

- After `put_host_settings`, while still holding the guard, re-apply when the request touched remote access: `if patch.remote_access.is_some() { crate::remote::apply_remote_access(&st).await; }`.

Register `pub mod remote;` and export `pub use remote::{RemoteStatus, apply_remote_access};` in `lib.rs`. Add the routes:

```rust
        .route("/host/remote", get(crate::remote::get_remote))
        .route("/host/remote/refresh", axum::routing::post(crate::remote::refresh))
```

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 7: Wire `pdtd`**

In `crates/pdtd/src/main.rs`, after building `state` (Task 2):

```rust
    state.listen_port = cfg.port;
    state.allowed_hosts = pdt_api::AllowedHosts::new(cfg.allowed_hosts.clone());
```

Then build the app with `with_host_guard(router(state.clone(), cfg.web_dir.clone()), state.allowed_hosts.clone())`. Once serving is about to start, re-apply the remote setting in the background. `serve --bg` mappings persist across restarts, so this re-adds the allowlist entry and reports state:

```rust
    {
        let st = state.clone();
        tokio::spawn(async move {
            let r = pdt_api::apply_remote_access(&st).await;
            tracing::info!(enabled = r.enabled, active = r.active, error = ?r.error, "remote access applied");
        });
    }
```

Run: `cargo test -p pdtd`
Expected: PASS. Remote access is off by default, so no tailscale call happens in the lifecycle test.

- [ ] **Step 8: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add Cargo.toml Cargo.lock crates web/src/api/gen
git commit -m "feat: remote access through tailscale serve, with a live Host/Origin allowlist"
```

---

### Task 5: Identity guard

**Files:**
- Create:
  - `crates/pdt-core/src/identity.rs`
  - `crates/pdt-api/src/identity_guard.rs`
  - `crates/pdt-api/tests/identity.rs`
- Modify:
  - `crates/pdt-core/src/lib.rs`
  - `crates/pdt-api/src/{routes,host,lib}.rs`
  - `crates/pdtd/src/main.rs`

**Interfaces:**
- Consumes: `AppState.owner_from_env`, `HostSettings.owner_login` and `AppState.settings_lock` (Task 2).
- Produces (`pdt_core::identity`):
  - `Caller { Local, Tailnet { login: String }, Unidentified }`.
  - `IdentityHeaders<'a> { login: Option<&'a str>, forwarded: bool, funnel: bool }`.
  - `classify(IdentityHeaders) -> Caller`.
  - `check(&Caller, owner: Option<&str>) -> Result<(), Denied>`.
  - `same_login(a, b) -> bool`.
  - `decode_header_value(&str) -> String` (RFC 2047 Q).
- Produces (`pdt_api`):
  - The identity middleware, applied inside `router()` to everything, including the static UI.
  - `AppState.owner_cache: Arc<RwLock<Option<String>>>`, and `AppState::refresh_owner(&self)`. pdtd calls `refresh_owner` at startup; `PUT /api/host/settings` calls it too.
  - `GET /api/host/whoami` → `WhoAmI { caller: "local" | "tailnet", login: Option<String> }`.
  - Error 403 `forbidden_identity`.

- [ ] **Step 1: Write the failing `pdt-core` tests**

Add `pub mod identity;` to `pdt-core/src/lib.rs`. Create `crates/pdt-core/src/identity.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn h(login: Option<&str>, forwarded: bool, funnel: bool) -> IdentityHeaders<'_> {
        IdentityHeaders { login, forwarded, funnel }
    }

    #[test]
    fn classification() {
        assert_eq!(classify(h(None, false, false)), Caller::Local);
        assert_eq!(classify(h(Some("me@x.com"), true, false)), Caller::Tailnet { login: "me@x.com".into() });
        assert_eq!(classify(h(None, true, false)), Caller::Unidentified, "tagged device through serve");
        assert_eq!(classify(h(Some("  "), true, false)), Caller::Unidentified);
        assert_eq!(classify(h(Some("me@x.com"), true, true)), Caller::Unidentified, "funnel is never trusted");
        assert_eq!(classify(h(None, false, true)), Caller::Unidentified);
    }

    #[test]
    fn owner_check() {
        let me = Caller::Tailnet { login: "Me@X.com".into() };
        assert_eq!(check(&Caller::Local, None), Ok(()));
        assert_eq!(check(&me, Some(" me@x.com ")), Ok(()), "case and spaces do not matter");
        assert_eq!(check(&me, Some("other@x.com")), Err(Denied::NotOwner("Me@X.com".into())));
        assert_eq!(check(&me, None), Err(Denied::NoOwner));
        assert_eq!(check(&Caller::Unidentified, Some("me@x.com")), Err(Denied::Unidentified));
    }

    #[test]
    fn rfc2047_q_values_are_decoded() {
        assert_eq!(decode_header_value("me@example.com"), "me@example.com");
        assert_eq!(decode_header_value("=?utf-8?q?Ren=C3=A9_D?="), "René D");
        assert_eq!(decode_header_value("=?UTF-8?Q?a=3Db?="), "a=b");
        assert_eq!(decode_header_value("=?utf-8?b?bm90IHE=?="), "=?utf-8?b?bm90IHE=?=", "only Q is decoded");
        assert_eq!(decode_header_value("=?utf-8?q?bad=ZZ?="), "=?utf-8?q?bad=ZZ?=", "malformed stays raw");
    }

    #[test]
    fn denial_messages_are_readable() {
        assert!(Denied::NotOwner("a@b".into()).to_string().contains("a@b"));
        assert!(Denied::Unidentified.to_string().contains("tagged"));
    }
}
```

Run: `cargo test -p pdt-core identity`
Expected: compile errors.

- [ ] **Step 2: Implement, above the tests**

```rust
//! Who may use this host (spec §2, decision 1). `tailscale serve` adds `Tailscale-User-Login` for untagged tailnet
//! users and always adds `X-Forwarded-For`/`X-Forwarded-Host`; pdtd listens only on loopback, so a request with none
//! of these came from this machine.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// From this machine (no proxy headers).
    Local,
    /// Through `tailscale serve`, from a tailnet user.
    Tailnet { login: String },
    /// Through a proxy without a user identity: a tagged device, Funnel, or something else.
    Unidentified,
}

#[derive(Debug, Clone, Copy)]
pub struct IdentityHeaders<'a> {
    /// `Tailscale-User-Login`, already decoded.
    pub login: Option<&'a str>,
    /// `X-Forwarded-For` or `X-Forwarded-Host` is present.
    pub forwarded: bool,
    /// `Tailscale-Funnel-Request` is present.
    pub funnel: bool,
}

pub fn classify(h: IdentityHeaders<'_>) -> Caller {
    if h.funnel {
        return Caller::Unidentified;
    }
    match h.login.map(str::trim).filter(|l| !l.is_empty()) {
        Some(login) => Caller::Tailnet { login: login.to_string() },
        None if h.forwarded => Caller::Unidentified,
        None if h.login.is_some() => Caller::Unidentified,
        None => Caller::Local,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("this host has no owner yet; set one in Settings on the host itself")]
    NoOwner,
    #[error("{0} is not this host's owner")]
    NotOwner(String),
    #[error("the request has no Tailscale user identity (tagged devices and Funnel are not allowed)")]
    Unidentified,
}

pub fn same_login(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

pub fn check(caller: &Caller, owner: Option<&str>) -> Result<(), Denied> {
    match caller {
        Caller::Local => Ok(()),
        Caller::Unidentified => Err(Denied::Unidentified),
        Caller::Tailnet { login } => match owner {
            None => Err(Denied::NoOwner),
            Some(o) if same_login(o, login) => Ok(()),
            Some(_) => Err(Denied::NotOwner(login.clone())),
        },
    }
}

/// Decodes an RFC 2047 "Q"-encoded UTF-8 value (`=?utf-8?q?…?=`), which Tailscale may use for non-ASCII values
/// (fact 5). Anything else, or anything malformed, is returned unchanged.
pub fn decode_header_value(raw: &str) -> String {
    let decoded = (|| {
        let inner = raw.strip_prefix("=?")?.strip_suffix("?=")?;
        let mut parts = inner.splitn(3, '?');
        let (charset, enc, text) = (parts.next()?, parts.next()?, parts.next()?);
        if !charset.eq_ignore_ascii_case("utf-8") || !enc.eq_ignore_ascii_case("q") {
            return None;
        }
        let mut bytes = Vec::with_capacity(text.len());
        let mut it = text.bytes();
        while let Some(b) = it.next() {
            match b {
                b'_' => bytes.push(b' '),
                b'=' => {
                    let hex = [it.next()?, it.next()?];
                    bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
                }
                b => bytes.push(b),
            }
        }
        String::from_utf8(bytes).ok()
    })();
    decoded.unwrap_or_else(|| raw.to_string())
}
```

Run: `cargo test -p pdt-core identity`
Expected: PASS.

- [ ] **Step 3: Write the failing API tests**

`crates/pdt-api/tests/identity.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn req(app: &axum::Router, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut r = Request::builder().uri(uri);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let res = app.clone().oneshot(r.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn put(app: &axum::Router, body: Value) {
    let r = Request::builder()
        .method("PUT")
        .uri("/api/host/settings")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    assert_eq!(app.clone().oneshot(r).await.unwrap().status(), StatusCode::OK);
}

async fn app(tmp: &std::path::Path, env_owner: Option<&str>, web_dir: Option<std::path::PathBuf>) -> axum::Router {
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.join("data"));
    state.owner_from_env = env_owner.map(String::from);
    state.refresh_owner().await;
    router(state, web_dir)
}

const ME: (&str, &str) = ("tailscale-user-login", "me@example.com");
const FWD: (&str, &str) = ("x-forwarded-for", "100.64.0.7");

#[tokio::test]
async fn local_requests_pass_and_whoami_says_local() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None, None).await;
    let (s, v) = req(&app, "/api/host/whoami", &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({"caller": "local", "login": null}));
}

#[tokio::test]
async fn only_the_owner_gets_in_remotely() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), None, None).await;
    let (s, v) = req(&app, "/api/health", &[ME, FWD]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")), "no owner yet");
    put(&app, json!({"ownerLogin": "Me@Example.com"})).await;
    assert_eq!(req(&app, "/api/health", &[ME, FWD]).await.0, StatusCode::OK);
    let (_, v) = req(&app, "/api/host/whoami", &[ME, FWD]).await;
    assert_eq!(v, json!({"caller": "tailnet", "login": "me@example.com"}));
    let (s, v) = req(&app, "/api/health", &[("tailscale-user-login", "eve@example.com"), FWD]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")));
    assert!(v["message"].as_str().unwrap().contains("eve@example.com"));
}

#[tokio::test]
async fn forwarded_without_identity_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(tmp.path(), Some("me@example.com"), None).await;
    for headers in [
        vec![FWD],
        vec![("x-forwarded-host", "box.t.ts.net")],
        vec![ME, FWD, ("tailscale-funnel-request", "?1")],
    ] {
        let (s, v) = req(&app, "/api/health", &headers).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")), "{headers:?}");
    }
}

#[tokio::test]
async fn env_owner_overrides_the_setting_and_guards_the_ui() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>ui</html>").unwrap();
    let app = app(tmp.path(), Some("me@example.com"), Some(web)).await;
    put(&app, json!({"ownerLogin": "someone-else@example.com"})).await;
    assert_eq!(req(&app, "/api/health", &[ME, FWD]).await.0, StatusCode::OK, "env wins");
    let (s, _) = req(&app, "/", &[("tailscale-user-login", "eve@example.com"), FWD]).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the static UI is guarded too");
    assert_eq!(req(&app, "/", &[]).await.0, StatusCode::OK);
}
```

Run: `cargo test -p pdt-api --test identity`
Expected: compile errors (`refresh_owner` is missing).

- [ ] **Step 4: Implement the guard**

Add to `AppState`:
- the field `pub(crate) owner_cache: std::sync::Arc<std::sync::RwLock<Option<String>>>` (Default in `new`);
- the methods:

```rust
    /// Reloads the effective owner (`PDT_OWNER_LOGIN`, else the stored setting) used by the identity guard.
    pub async fn refresh_owner(&self) {
        let stored = self.store.get_host_settings().await.ok().and_then(|s| s.owner_login);
        *self.owner_cache.write().unwrap_or_else(|e| e.into_inner()) = self.owner_from_env.clone().or(stored);
    }

    pub(crate) fn owner(&self) -> Option<String> {
        self.owner_cache.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
```

`crates/pdt-api/src/identity_guard.rs`:

```rust
use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pdt_core::identity::{Caller, IdentityHeaders, check, classify, decode_header_value};
use serde::Serialize;
use ts_rs::TS;

pub(crate) fn caller(headers: &HeaderMap) -> Caller {
    let login =
        headers.get("tailscale-user-login").map(|v| decode_header_value(&String::from_utf8_lossy(v.as_bytes())));
    classify(IdentityHeaders {
        login: login.as_deref(),
        forwarded: headers.contains_key("x-forwarded-for") || headers.contains_key("x-forwarded-host"),
        funnel: headers.contains_key("tailscale-funnel-request"),
    })
}

pub(crate) async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    match check(&caller(req.headers()), st.owner().as_deref()) {
        Ok(()) => next.run(req).await,
        Err(denied) => {
            tracing::warn!(reason = %denied, path = %req.uri().path(), "refused a remote request");
            ApiError::new(StatusCode::FORBIDDEN, "forbidden_identity", denied.to_string()).into_response()
        }
    }
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct WhoAmI {
    /// "local" or "tailnet".
    pub caller: String,
    pub login: Option<String>,
}

pub(crate) async fn whoami(headers: HeaderMap) -> Json<WhoAmI> {
    Json(match caller(&headers) {
        Caller::Tailnet { login } => WhoAmI { caller: "tailnet".into(), login: Some(login) },
        _ => WhoAmI { caller: "local".into(), login: None },
    })
}
```

In `router()`:
- add the route `.route("/host/whoami", get(crate::identity_guard::whoami))`;
- capture `let guard_state = state.clone();` before `.with_state(state)`;
- after the `match web_dir` block produces `app`, return `app.layer(axum::middleware::from_fn_with_state(guard_state, crate::identity_guard::guard))`. Make the `match` an expression assigned to `let app = …;`.

In `host.rs` `put_settings`, call `st.refresh_owner().await;` right after `put_host_settings`.

In `pdtd` `main.rs`, call `state.refresh_owner().await;` after setting `owner_from_env`.

Register `mod identity_guard;` in `lib.rs`.

Run: `cargo test -p pdt-api`
Expected: PASS, including all earlier suites (local requests carry none of the proxy headers).

- [ ] **Step 5: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add crates web/src/api/gen
git commit -m "feat: identity guard: only the owner's Tailscale login gets in remotely"
```

---

### Task 6: Run configuration (`.pdt/project.toml` + host overrides)

**Files:**
- Create:
  - `crates/pdt-core/src/runconfig.rs`
  - `crates/pdt-api/src/runconfig.rs`
  - `crates/pdt-api/tests/runconfig.rs`
- Modify:
  - `crates/pdt-core/src/lib.rs`, `crates/pdt-core/src/brand.rs`, `crates/pdt-core/Cargo.toml`
  - `crates/pdt-api/src/{routes,lib,project_settings}.rs`

**Interfaces:**
- Consumes: `ProjectSettings.run_overrides` and `PUT /api/projects/{id}/settings` (Task 3).
- Produces (`pdt_core::brand`): `PROJECT_CONFIG_FILE = ".pdt/project.toml"`.
- Produces (`pdt_core::runconfig`):
  - `ConfigFile { runs: Vec<EntryFile>, tests: Vec<EntryFile> }`.
  - `EntryFile { name, setup, command, web, data, env }`, every field except `name` optional.
  - `Source { File, Overrides }`.
  - `parse(&str, Source) -> Result<ConfigFile, RunConfigError>`.
  - `merge(Option<ConfigFile>, Option<ConfigFile>) -> Result<RunConfig, RunConfigError>`.
  - `RunConfig { runs: Vec<RunDef>, tests: Vec<RunDef> }`.
  - `RunDef { name, setup: Option<String>, command: String, web: bool, data: bool, env: BTreeMap<String, String> }`.
  - `RunVars { port: Option<u16>, data_dir: Option<String>, url: Option<String>, host: String, branch: String, owner_login: Option<String> }`, with `pairs()`.
  - `expand(text, &[(&str, String)]) -> String`.
  - `run_env(&RunDef, &RunVars) -> Vec<(String, String)>`.
- Produces (`pdt_api::runconfig`):
  - `RunConfigView { filePath, file: Option<String>, overrides: Option<String>, effective: Option<RunConfig>, error: Option<String> }`.
  - `GET /api/projects/{id}/run-config?worktreeId=`.
  - `pub(crate) async fn load_effective(st, project_id, checkout: &Path) -> Result<RunConfig, ApiError>`.
  - `pub(crate) async fn checkout_dir(st, project, worktree_id: Option<&str>) -> Result<(PathBuf, String /*branch*/), ApiError>`.
  - Error 400 `invalid_run_config` from the project-settings PUT and from run starts.

- [ ] **Step 1: Add the dependency and write the failing tests**

Run (PowerShell): `cargo add toml -p pdt-core`. Add `pub mod runconfig;` to `pdt-core/src/lib.rs`. Add to `brand.rs`:

```rust
/// The optional run configuration file inside a project's repository (remote-host spec §4.1).
pub const PROJECT_CONFIG_FILE: &str = ".pdt/project.toml";
```

`crates/pdt-core/src/runconfig.rs`, tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
[[run]]
name    = "web"
setup   = "pnpm install"
command = "pnpm dev --port $PORT"
web     = true
data    = true
env     = { API_URL = "http://127.0.0.1:$PORT/api", MODE = "dev" }

[[test]]
name    = "unit"
command = "pnpm test"
"#;

    fn file() -> ConfigFile {
        parse(FILE, Source::File).unwrap()
    }

    #[test]
    fn parses_the_spec_example() {
        let c = merge(Some(file()), None).unwrap();
        let web = &c.runs[0];
        assert_eq!((web.name.as_str(), web.web, web.data), ("web", true, true));
        assert_eq!(web.setup.as_deref(), Some("pnpm install"));
        assert_eq!(web.env["MODE"], "dev");
        assert_eq!(c.tests[0].command, "pnpm test");
        assert!(!c.tests[0].web);
    }

    #[test]
    fn overrides_merge_field_by_field_and_by_name() {
        let o = parse(
            r#"
[[run]]
name = "web"
command = "pnpm dev --host 0.0.0.0 --port $PORT"
env = { MODE = "test", EXTRA = "1" }

[[run]]
name = "cli"
command = "cargo run"
"#,
            Source::Overrides,
        )
        .unwrap();
        let c = merge(Some(file()), Some(o)).unwrap();
        let web = &c.runs[0];
        assert_eq!(web.command, "pnpm dev --host 0.0.0.0 --port $PORT");
        assert_eq!(web.setup.as_deref(), Some("pnpm install"), "kept from the file");
        assert!(web.web, "kept from the file");
        assert_eq!(web.env["MODE"], "test");
        assert_eq!(web.env["EXTRA"], "1");
        assert_eq!(web.env["API_URL"], "http://127.0.0.1:$PORT/api");
        assert_eq!(c.runs[1].name, "cli");
        assert!(!c.runs[1].web && !c.runs[1].data);
    }

    #[test]
    fn overrides_alone_can_supply_everything() {
        let o = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\n", Source::Overrides).unwrap();
        assert_eq!(merge(None, Some(o)).unwrap().runs.len(), 1);
        assert_eq!(merge(None, None).unwrap(), RunConfig::default());
    }

    #[test]
    fn errors_are_specific() {
        assert!(matches!(parse("[[run]\n", Source::File), Err(RunConfigError::Parse { source: Source::File, .. })));
        assert!(matches!(
            parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nwebb = true\n", Source::Overrides),
            Err(RunConfigError::Parse { source: Source::Overrides, .. })
        ));
        let no_cmd = parse("[[run]]\nname = \"a\"\n", Source::File).unwrap();
        assert_eq!(merge(Some(no_cmd), None), Err(RunConfigError::MissingCommand("a".into())));
        let blank = parse("[[run]]\nname = \" \"\ncommand = \"x\"\n", Source::File).unwrap();
        assert_eq!(merge(Some(blank), None), Err(RunConfigError::EmptyName));
        let dup = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\n[[run]]\nname = \"a\"\ncommand = \"y\"\n", Source::File)
            .unwrap();
        assert_eq!(merge(Some(dup), None), Err(RunConfigError::DuplicateName("a".into())));
        let bad_env = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nenv = { \"A=B\" = \"1\" }\n", Source::File).unwrap();
        assert_eq!(merge(Some(bad_env), None), Err(RunConfigError::BadEnvName("A=B".into())));
        let web_test = parse("[[test]]\nname = \"t\"\ncommand = \"x\"\nweb = true\n", Source::File).unwrap();
        assert_eq!(merge(Some(web_test), None), Err(RunConfigError::WebTest("t".into())));
    }

    fn vars() -> RunVars {
        RunVars {
            port: Some(8401),
            data_dir: Some("/data/runs/p/main/web".into()),
            url: Some("https://box.t.ts.net:8401/".into()),
            host: "box.t.ts.net".into(),
            branch: "feature/x".into(),
            owner_login: Some("me@example.com".into()),
        }
    }

    #[test]
    fn expansion_replaces_only_known_names() {
        let p = vars().pairs();
        assert_eq!(expand("pnpm dev --port $PORT", &p), "pnpm dev --port 8401");
        assert_eq!(expand("${PORT}x $PORTX $HOME $$ $", &p), "8401x $PORTX $HOME $$ $");
        assert_eq!(expand("$PDT_RUN_HOST:$PORT", &p), "box.t.ts.net:8401");
        assert_eq!(expand("$PDT_OWNER_LOGIN/$PDT_RUN_BRANCH", &p), "me@example.com/feature/x");
        assert_eq!(expand("${PDT_RUN_DATA}/db", &p), "/data/runs/p/main/web/db");
        assert_eq!(expand("${NOPE", &p), "${NOPE");
    }

    #[test]
    fn run_env_sets_pdt_vars_then_explicit_env_wins() {
        let c = merge(Some(file()), None).unwrap();
        let env = run_env(&c.runs[0], &vars());
        let get = |k: &str| env.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PORT"), Some("8401"));
        assert_eq!(get("PDT_RUN_URL"), Some("https://box.t.ts.net:8401/"));
        assert_eq!(get("PDT_RUN_HOST"), Some("box.t.ts.net"));
        assert_eq!(get("PDT_RUN_BRANCH"), Some("feature/x"));
        assert_eq!(get("PDT_RUN_DATA"), Some("/data/runs/p/main/web"));
        assert_eq!(get("PDT_OWNER_LOGIN"), Some("me@example.com"));
        assert_eq!(get("API_URL"), Some("http://127.0.0.1:8401/api"));
        let mut v = vars();
        v.port = None;
        v.data_dir = None;
        v.url = None;
        v.owner_login = None;
        let env = run_env(&c.tests[0], &v);
        assert!(env.iter().all(|(k, _)| k != "PORT" && k != "PDT_RUN_DATA" && k != "PDT_OWNER_LOGIN"));
    }

    #[test]
    fn explicit_env_overrides_a_pdt_variable() {
        let o = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nenv = { PDT_RUN_HOST = \"custom\" }\n", Source::File).unwrap();
        let c = merge(Some(o), None).unwrap();
        let env = run_env(&c.runs[0], &vars());
        assert_eq!(env.iter().rev().find(|(k, _)| k == "PDT_RUN_HOST").unwrap().1, "custom");
    }
}
```

Run: `cargo test -p pdt-core runconfig`
Expected: compile errors.

- [ ] **Step 2: Implement, above the tests**

```rust
//! Run configurations (remote-host spec §4.1): the repo's optional `.pdt/project.toml`, merged field by field with
//! the host's overrides (same schema, matched by `name`). Pure.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default, rename = "run")]
    pub runs: Vec<EntryFile>,
    #[serde(default, rename = "test")]
    pub tests: Vec<EntryFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    pub name: String,
    pub setup: Option<String>,
    pub command: Option<String>,
    pub web: Option<bool>,
    pub data: Option<bool>,
    pub env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunDef {
    pub name: String,
    pub setup: Option<String>,
    pub command: String,
    pub web: bool,
    pub data: bool,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunConfig {
    pub runs: Vec<RunDef>,
    pub tests: Vec<RunDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    File,
    Overrides,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::File => ".pdt/project.toml",
            Source::Overrides => "the host overrides",
        })
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RunConfigError {
    #[error("{source} is not valid: {message}")]
    Parse { source: Source, message: String },
    #[error("\"{0}\" has no command")]
    MissingCommand(String),
    #[error("every run and test needs a name")]
    EmptyName,
    #[error("the name \"{0}\" is used twice")]
    DuplicateName(String),
    #[error("\"{0}\" is not a valid environment variable name")]
    BadEnvName(String),
    #[error("test \"{0}\" cannot be a web run")]
    WebTest(String),
}

pub fn parse(text: &str, source: Source) -> Result<ConfigFile, RunConfigError> {
    toml::from_str(text).map_err(|e| RunConfigError::Parse { source, message: e.to_string() })
}

fn merge_list(base: Vec<EntryFile>, over: Vec<EntryFile>) -> Result<Vec<EntryFile>, RunConfigError> {
    let mut out: Vec<EntryFile> = Vec::new();
    for e in base.into_iter() {
        if out.iter().any(|o| o.name.trim() == e.name.trim()) {
            return Err(RunConfigError::DuplicateName(e.name.trim().into()));
        }
        out.push(e);
    }
    let mut seen_over: Vec<String> = Vec::new();
    for o in over {
        let name = o.name.trim().to_string();
        if seen_over.contains(&name) {
            return Err(RunConfigError::DuplicateName(name));
        }
        seen_over.push(name.clone());
        match out.iter_mut().find(|b| b.name.trim() == name) {
            Some(b) => {
                b.setup = o.setup.or(b.setup.take());
                b.command = o.command.or(b.command.take());
                b.web = o.web.or(b.web);
                b.data = o.data.or(b.data);
                if let Some(env) = o.env {
                    b.env.get_or_insert_with(BTreeMap::new).extend(env);
                }
            }
            None => out.push(o),
        }
    }
    Ok(out)
}

fn valid_env_name(n: &str) -> bool {
    let mut chars = n.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn finish(e: EntryFile, test: bool) -> Result<RunDef, RunConfigError> {
    let name = e.name.trim().to_string();
    if name.is_empty() {
        return Err(RunConfigError::EmptyName);
    }
    let command = e
        .command
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| RunConfigError::MissingCommand(name.clone()))?;
    let env = e.env.unwrap_or_default();
    if let Some(bad) = env.keys().find(|k| !valid_env_name(k)) {
        return Err(RunConfigError::BadEnvName(bad.clone()));
    }
    let web = e.web.unwrap_or(false);
    if test && web {
        return Err(RunConfigError::WebTest(name));
    }
    let setup = e.setup.filter(|s| !s.trim().is_empty());
    Ok(RunDef { name, setup, command, web, data: e.data.unwrap_or(false), env })
}

/// The effective configuration: `file` with `overrides` merged over it (either may be absent).
pub fn merge(file: Option<ConfigFile>, overrides: Option<ConfigFile>) -> Result<RunConfig, RunConfigError> {
    let file = file.unwrap_or_default();
    let over = overrides.unwrap_or_default();
    let runs = merge_list(file.runs, over.runs)?.into_iter().map(|e| finish(e, false)).collect::<Result<_, _>>()?;
    let tests = merge_list(file.tests, over.tests)?.into_iter().map(|e| finish(e, true)).collect::<Result<_, _>>()?;
    Ok(RunConfig { runs, tests })
}

/// The values PDT provides to a run (spec §4.2 "Variables PDT sets").
#[derive(Debug, Clone, PartialEq)]
pub struct RunVars {
    pub port: Option<u16>,
    pub data_dir: Option<String>,
    pub url: Option<String>,
    /// The tailnet DNS name when remote access is active, else `127.0.0.1`.
    pub host: String,
    pub branch: String,
    pub owner_login: Option<String>,
}

impl RunVars {
    /// (name, value) for every variable that has a value, in a fixed order.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if let Some(p) = self.port {
            v.push(("PORT", p.to_string()));
        }
        if let Some(d) = &self.data_dir {
            v.push(("PDT_RUN_DATA", d.clone()));
        }
        if let Some(u) = &self.url {
            v.push(("PDT_RUN_URL", u.clone()));
        }
        v.push(("PDT_RUN_HOST", self.host.clone()));
        v.push(("PDT_RUN_BRANCH", self.branch.clone()));
        if let Some(o) = &self.owner_login {
            v.push(("PDT_OWNER_LOGIN", o.clone()));
        }
        v
    }
}

/// Replaces `$NAME` and `${NAME}` for the names in `vars`; anything else (other variables, a lone `$`) is left for
/// the shell, so the same text works under cmd.exe and sh.
pub fn expand(text: &str, vars: &[(&str, String)]) -> String {
    let lookup = |n: &str| vars.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str());
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (name, consumed) = if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => ("", 0),
            }
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], end)
        };
        match lookup(name).filter(|_| !name.is_empty()) {
            Some(v) => {
                out.push_str(v);
                rest = &after[consumed..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The variables a run's processes get: PDT's own, then the definition's `env` (expanded), which wins.
pub fn run_env(def: &RunDef, vars: &RunVars) -> Vec<(String, String)> {
    let pairs = vars.pairs();
    let mut env: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    for (k, v) in &def.env {
        env.retain(|(n, _)| n != k);
        env.push((k.clone(), expand(v, &pairs)));
    }
    env
}
```

Run: `cargo test -p pdt-core runconfig`
Expected: PASS.

- [ ] **Step 3: Write the failing API test**

`crates/pdt-api/tests/runconfig.rs`. Copy the imports and the `call` helper from `tests/host.rs`, then add:

```rust
fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

async fn setup(tmp: &std::path::Path) -> (axum::Router, String, std::path::PathBuf) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".pdt").join("project.toml"), "[[run]]\nname = \"web\"\ncommand = \"serve $PORT\"\nweb = true\n")
        .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let app = pdt_api::router(pdt_api::AppState::new(store, tmp.join("data")), None);
    let (_, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    (app, p["id"].as_str().unwrap().to_string(), repo)
}

#[tokio::test]
async fn effective_config_merges_file_and_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, _) = setup(tmp.path()).await;
    let uri = format!("/api/projects/{pid}/run-config");
    let (s, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["file"].as_str().unwrap().contains("serve $PORT"));
    assert!(v["filePath"].as_str().unwrap().ends_with("project.toml"));
    assert_eq!(v["effective"]["runs"][0]["web"], true);
    assert_eq!(v["error"], Value::Null);

    let overrides = "[[run]]\nname = \"web\"\ncommand = \"serve --fast $PORT\"\n";
    let (s, _) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": overrides})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "GET", &uri, None).await;
    assert_eq!(v["effective"]["runs"][0]["command"], "serve --fast $PORT");
    assert_eq!(v["overrides"], overrides);
}

#[tokio::test]
async fn invalid_overrides_are_rejected_and_bad_files_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/projects/{pid}/settings"),
        Some(json!({"agentCredentials": null, "runOverrides": "[[run]\n"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_run_config")), "{v}");
    std::fs::write(repo.join(".pdt").join("project.toml"), "[[run]]\nname = \"x\"\n").unwrap();
    let (s, v) = call(&app, "GET", &format!("/api/projects/{pid}/run-config"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["effective"], Value::Null);
    assert!(v["error"].as_str().unwrap().contains("no command"), "{v}");
}

#[tokio::test]
async fn a_project_without_a_file_has_an_empty_config() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, pid, repo) = setup(tmp.path()).await;
    std::fs::remove_file(repo.join(".pdt").join("project.toml")).unwrap();
    let (_, v) = call(&app, "GET", &format!("/api/projects/{pid}/run-config"), None).await;
    assert_eq!((v["file"].clone(), v["effective"].clone()), (Value::Null, json!({"runs": [], "tests": []})));
}
```

Run: `cargo test -p pdt-api --test runconfig`
Expected: FAIL (404).

- [ ] **Step 4: Implement the API module**

`crates/pdt-api/src/runconfig.rs`:

```rust
use crate::error::ApiError;
use crate::routes::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::brand::PROJECT_CONFIG_FILE;
use pdt_core::project::Project;
use pdt_core::runconfig::{RunConfig, RunConfigError, Source, merge, parse};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use ts_rs::TS;

impl From<RunConfigError> for ApiError {
    fn from(e: RunConfigError) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_run_config", e.to_string())
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunConfigView {
    pub file_path: String,
    /// The file's text, or null when the checkout has none.
    pub file: Option<String>,
    pub overrides: Option<String>,
    /// Null when the file or the overrides are invalid (see `error`).
    pub effective: Option<RunConfig>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Where {
    worktree_id: Option<String>,
}

/// The folder a run in this place uses, and its branch: the worktree, or the project's main checkout.
pub(crate) async fn checkout_dir(
    st: &AppState,
    project: &Project,
    worktree_id: Option<&str>,
) -> Result<(PathBuf, String), ApiError> {
    match worktree_id {
        Some(wid) => {
            let wt = st.store.get_worktree(wid).await?.filter(|w| w.project_id == project.id).ok_or_else(|| {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", "worktree does not belong to this project")
            })?;
            Ok((PathBuf::from(wt.path), wt.branch))
        }
        None => {
            let repo = PathBuf::from(&project.repo_path);
            let r = repo.clone();
            let info =
                tokio::task::spawn_blocking(move || pdt_git::resolve_repo(&r)).await.map_err(ApiError::internal)??;
            Ok((repo, info.current_branch.unwrap_or_else(|| project.base_branch.clone())))
        }
    }
}

fn read_file(checkout: &FsPath) -> Result<Option<String>, ApiError> {
    match std::fs::read_to_string(checkout.join(PROJECT_CONFIG_FILE)) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ApiError::internal(e)),
    }
}

fn effective(file: Option<&str>, overrides: Option<&str>) -> Result<RunConfig, RunConfigError> {
    let f = file.map(|t| parse(t, Source::File)).transpose()?;
    let o = overrides.map(|t| parse(t, Source::Overrides)).transpose()?;
    merge(f, o)
}

/// The effective configuration for a run in `checkout`.
pub(crate) async fn load_effective(st: &AppState, project_id: &str, checkout: &FsPath) -> Result<RunConfig, ApiError> {
    let overrides = st.store.get_project_settings(project_id).await?.run_overrides;
    Ok(effective(read_file(checkout)?.as_deref(), overrides.as_deref())?)
}

/// Checks override text before it is stored.
pub(crate) fn validate_overrides(text: &str) -> Result<(), ApiError> {
    parse(text, Source::Overrides)?;
    Ok(())
}

pub(crate) async fn get_config(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(w): Query<Where>,
) -> Result<Json<RunConfigView>, ApiError> {
    let project = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let (dir, _) = checkout_dir(&st, &project, w.worktree_id.as_deref()).await?;
    let file = read_file(&dir)?;
    let overrides = st.store.get_project_settings(&id).await?.run_overrides;
    let (effective, error) = match effective(file.as_deref(), overrides.as_deref()) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(Json(RunConfigView {
        file_path: dir.join(PROJECT_CONFIG_FILE).to_string_lossy().into_owned(),
        file,
        overrides,
        effective,
        error,
    }))
}
```

In `project_settings::put_settings`, before storing, add: `if let Some(t) = &run_overrides { crate::runconfig::validate_overrides(t)?; }`.

Register `mod runconfig;` in `lib.rs`, and add `.route("/projects/{id}/run-config", get(crate::runconfig::get_config))`.

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 5: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add crates Cargo.lock web/src/api/gen
git commit -m "feat: run configurations from .pdt/project.toml merged with host overrides"
```

---

### Task 7: Run instances (backend)

**Files:**
- Create:
  - `crates/pdt-core/src/run.rs`
  - `crates/pdt-store/migrations/0005_runs.sql`
  - `crates/pdt-store/src/runs.rs`
  - `crates/pdt-api/src/runs.rs`, `crates/pdt-api/src/run_manager.rs`
  - `crates/pdt-api/tests/runs.rs`
- Modify:
  - `crates/pdt-core/src/lib.rs`
  - `crates/pdt-git/src/lib.rs`
  - `crates/pdt-agents/src/pty.rs` (tree kill on Windows)
  - `crates/pdt-store/src/lib.rs`
  - `crates/pdt-api/src/{routes,lib,pty_ws,remote}.rs`
  - `crates/pdtd/src/main.rs`, `crates/pdtd/tests/lifecycle.rs`

**Interfaces:**
- Consumes:
  - `RunConfig`, `RunDef`, `RunVars`, `run_env`, `expand`, `load_effective` and `checkout_dir` (Task 6);
  - `HostSettings.run_ports` and `max_concurrent_runs` (Task 2);
  - `AppState.remote`, `AppState.tailscale` and the `remote::extra_ports` hook (Task 4);
  - `AppState::owner()` (Task 5).
- Produces (`pdt_core::run`):
  - `RunKind { Run, Test }` and `RunState { SettingUp, Starting, Ready, Running, Passed, Failed, Stopped }`. Each has `as_str()` and `parse()`, and `RunState` has `is_active()`.
  - `RunStep { Setup, Command }`.
  - `Run { id, projectId, worktreeId, branch, name, kind, state, port, url, dataDir, cwd, exitCode, error, createdAt, updatedAt }`.
  - `NewRun { worktreeId?, branch?, name, kind, stopRunId? }`.
  - `pty_id(run_id, step)`, `run_data_dir(data_root, project_id, branch, name)`.
  - `pick_port(range, taken, prefer, is_free) -> Option<u16>`.
  - `StepLaunch { program, args, script: Option<String> }` and `step_launch(windows, comspec, command, script_path)`.
- Produces (`pdt_store`): `insert_run`, `get_run`, `list_runs(project_id)`, `list_active_runs()`, `set_run_state(id, state)`, `finish_run(id, state, exit_code, error, at)`, `set_run_port(id, port, url)`, `delete_run`.
- Produces (`pdt_git`):
  - `BranchInfo { name, local: bool }` and `list_branches(repo) -> Result<Vec<BranchInfo>, GitError>`.
  - `add_worktree_for_branch(repo, path, branch) -> Result<String /*head sha*/, GitError>`.
  - `branch_exists(repo, branch) -> bool`.
- Produces (`pdt_api`):
  - Routes:
    - `GET /api/projects/{id}/runs`, `POST /api/projects/{id}/runs` (body `NewRun`), `GET /api/projects/{id}/branches`;
    - `GET /api/runs` (active runs on the host);
    - `GET /api/runs/{id}` and `DELETE /api/runs/{id}`;
    - `POST /api/runs/{id}/stop`, `POST /api/runs/{id}/restart`, `POST /api/runs/{id}/reset-data`;
    - `GET /api/runs/{id}/pty/{step}` (WebSocket).
  - Errors: 409 `too_many_runs`, 409 `run_active`, 404 `unknown_run`, 503 `no_free_port`.
  - `pub async fn resume_runs(&AppState) -> usize` and `pub async fn sweep_gone_runs(&AppState)`.
  - `AppState.run_controls` (crate-private).
  - `PtyManager::kill` kills the whole process tree on Windows (`taskkill /T /F`).

- [ ] **Step 1: Write the failing `pdt-core` tests**

Add `pub mod run;` to `pdt-core/src/lib.rs`. Create `crates/pdt-core/src/run.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::PortRange;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn enum_strings_match_serde() {
        for s in [
            RunState::SettingUp,
            RunState::Starting,
            RunState::Ready,
            RunState::Running,
            RunState::Passed,
            RunState::Failed,
            RunState::Stopped,
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_str()));
            assert_eq!(RunState::parse(s.as_str()), Some(s));
        }
        assert_eq!(RunState::SettingUp.as_str(), "setting_up");
        for k in [RunKind::Run, RunKind::Test] {
            assert_eq!(RunKind::parse(k.as_str()), Some(k));
        }
        let active: Vec<_> = [RunState::SettingUp, RunState::Starting, RunState::Ready, RunState::Running]
            .into_iter()
            .filter(|s| s.is_active())
            .collect();
        assert_eq!(active.len(), 4);
        assert!(![RunState::Passed, RunState::Failed, RunState::Stopped].iter().any(|s| s.is_active()));
    }

    #[test]
    fn ids_and_paths() {
        assert_eq!(pty_id("r1", RunStep::Setup), "run:r1:setup");
        assert_eq!(pty_id("r1", RunStep::Command), "run:r1:command");
        let d = run_data_dir(Path::new("data"), "p", "feature/x", "web");
        assert_eq!(d, Path::new("data").join("runs").join("p").join("feature-x").join("web"));
    }

    #[test]
    fn port_picking() {
        let range = PortRange { start: 8400, end: 8403 };
        let taken: BTreeSet<u16> = [8400].into();
        assert_eq!(pick_port(range, &taken, None, |_| true), Some(8401));
        assert_eq!(pick_port(range, &taken, Some(8403), |_| true), Some(8403), "the preferred port when free");
        assert_eq!(pick_port(range, &taken, Some(8400), |_| true), Some(8401), "taken by another run");
        assert_eq!(pick_port(range, &taken, Some(9000), |_| true), Some(8401), "outside the range");
        assert_eq!(pick_port(range, &taken, None, |p| p == 8403), Some(8403), "bound by something else");
        assert_eq!(pick_port(range, &taken, None, |_| false), None);
    }

    #[test]
    fn step_launch_per_platform() {
        let w = step_launch(true, "C:\\Windows\\system32\\cmd.exe", "pnpm dev && echo ok", Path::new("C:\\d\\s.cmd"));
        assert_eq!(w.program, "C:\\Windows\\system32\\cmd.exe");
        assert_eq!(w.args, ["/d", "/c", "C:\\d\\s.cmd"]);
        assert_eq!(w.script.as_deref(), Some("@echo off\r\npnpm dev && echo ok\r\nexit /b %ERRORLEVEL%\r\n"));
        let u = step_launch(false, "", "pnpm dev && echo ok", Path::new("/unused"));
        assert_eq!((u.program.as_str(), u.args.as_slice()), ("/bin/sh", &["-c".to_string(), "pnpm dev && echo ok".into()][..]));
        assert_eq!(u.script, None);
    }
}
```

Run: `cargo test -p pdt-core run::`
Expected: compile errors.

- [ ] **Step 2: Implement `run.rs`, above the tests**

```rust
//! Run instances (remote-host spec §4.2). Pure types and rules; pdt-api drives the processes.

use crate::host::PortRange;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunKind {
    Run,
    Test,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Run => "run",
            RunKind::Test => "test",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RunKind::Run, RunKind::Test].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunState {
    SettingUp,
    Starting,
    /// A web run whose port answers, or a non-web run whose command started.
    Ready,
    /// A test that is executing.
    Running,
    /// A test that exited 0.
    Passed,
    Failed,
    Stopped,
}

const ALL_STATES: [RunState; 7] = [
    RunState::SettingUp,
    RunState::Starting,
    RunState::Ready,
    RunState::Running,
    RunState::Passed,
    RunState::Failed,
    RunState::Stopped,
];

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::SettingUp => "setting_up",
            RunState::Starting => "starting",
            RunState::Ready => "ready",
            RunState::Running => "running",
            RunState::Passed => "passed",
            RunState::Failed => "failed",
            RunState::Stopped => "stopped",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        ALL_STATES.into_iter().find(|k| k.as_str() == s)
    }
    /// Has (or is about to have) a process.
    pub fn is_active(self) -> bool {
        matches!(self, RunState::SettingUp | RunState::Starting | RunState::Ready | RunState::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStep {
    Setup,
    Command,
}

impl RunStep {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStep::Setup => "setup",
            RunStep::Command => "command",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [RunStep::Setup, RunStep::Command].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    /// None means the project's main checkout.
    pub worktree_id: Option<String>,
    pub branch: String,
    /// The `[[run]]`/`[[test]]` name in the effective configuration.
    pub name: String,
    pub kind: RunKind,
    pub state: RunState,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub data_dir: Option<String>,
    /// Working directory, native separators.
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewRun {
    /// Run in this worktree. With neither this nor `branch`, the run uses the main checkout.
    #[serde(default)]
    #[ts(optional)]
    pub worktree_id: Option<String>,
    /// Run this branch; a branch without a worktree gets one.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    pub name: String,
    pub kind: RunKind,
    /// Confirmed by the user: stop this run to stay within the host's maximum.
    #[serde(default)]
    #[ts(optional)]
    pub stop_run_id: Option<String>,
}

pub fn pty_id(run_id: &str, step: RunStep) -> String {
    format!("run:{run_id}:{}", step.as_str())
}

/// Kept per project, branch and run name (spec §4.2 "Data").
pub fn run_data_dir(data_root: &Path, project_id: &str, branch: &str, name: &str) -> PathBuf {
    data_root.join("runs").join(project_id).join(branch.replace('/', "-")).join(name.replace(['/', '\\'], "-"))
}

/// A port in `range` that no active run holds and `is_free` accepts; `prefer` first when it qualifies.
pub fn pick_port(
    range: PortRange,
    taken: &BTreeSet<u16>,
    prefer: Option<u16>,
    mut is_free: impl FnMut(u16) -> bool,
) -> Option<u16> {
    let ok = |p: u16, f: &mut dyn FnMut(u16) -> bool| (range.start..=range.end).contains(&p) && !taken.contains(&p) && f(p);
    if let Some(p) = prefer
        && ok(p, &mut is_free)
    {
        return Some(p);
    }
    (range.start..=range.end).find(|&p| ok(p, &mut is_free))
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepLaunch {
    pub program: String,
    pub args: Vec<String>,
    /// Windows: the text to write to `script_path` before launching.
    pub script: Option<String>,
}

/// How to run a step's command line: through a `.cmd` file on Windows (no PTY argument quoting), `sh -c` elsewhere.
pub fn step_launch(windows: bool, comspec: &str, command: &str, script_path: &Path) -> StepLaunch {
    if windows {
        StepLaunch {
            program: comspec.to_string(),
            args: vec!["/d".into(), "/c".into(), script_path.to_string_lossy().into_owned()],
            script: Some(format!("@echo off\r\n{command}\r\nexit /b %ERRORLEVEL%\r\n")),
        }
    } else {
        StepLaunch { program: "/bin/sh".into(), args: vec!["-c".into(), command.into()], script: None }
    }
}
```

Run: `cargo test -p pdt-core`
Expected: PASS.

- [ ] **Step 3: Store runs (TDD)**

`crates/pdt-store/migrations/0005_runs.sql`:

```sql
CREATE TABLE runs (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    worktree_id TEXT REFERENCES worktrees(id) ON DELETE SET NULL,
    branch      TEXT NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    state       TEXT NOT NULL,
    port        INTEGER,
    url         TEXT,
    data_dir    TEXT,
    cwd         TEXT NOT NULL,
    exit_code   INTEGER,
    error       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX runs_by_project ON runs (project_id, created_at);
-- Two active runs never share a port.
CREATE UNIQUE INDEX runs_by_active_port ON runs (port)
    WHERE port IS NOT NULL AND state IN ('setting_up', 'starting', 'ready', 'running');
```

Write `crates/pdt-store/src/runs.rs` with these tests at the bottom, and add `mod runs;` to `lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::worktrees::tests::store_with_project;
    use pdt_core::run::{Run, RunKind, RunState};

    pub(crate) fn run(id: &str, port: Option<u16>, at: i64) -> Run {
        Run {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            branch: "main".into(),
            name: "web".into(),
            kind: RunKind::Run,
            state: RunState::SettingUp,
            port,
            url: port.map(|p| format!("http://127.0.0.1:{p}/")),
            data_dir: Some("/d".into()),
            cwd: "/r".into(),
            exit_code: None,
            error: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[tokio::test]
    async fn runs_roundtrip_and_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_run(&run("b", Some(8401), 2)).await.unwrap();
        store.insert_run(&run("a", None, 1)).await.unwrap();
        let ids: Vec<_> = store.list_runs("p").await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(store.get_run("b").await.unwrap(), Some(run("b", Some(8401), 2)));
        assert!(store.set_run_state("b", RunState::Ready, 5).await.unwrap());
        assert!(store.finish_run("b", RunState::Failed, Some(3), Some("boom"), 9).await.unwrap());
        let b = store.get_run("b").await.unwrap().unwrap();
        assert_eq!((b.state, b.exit_code, b.error.as_deref(), b.updated_at), (RunState::Failed, Some(3), Some("boom"), 9));
        assert_eq!(store.list_active_runs().await.unwrap().len(), 1, "only a is active");
        store.set_run_port("a", Some(8402), Some("http://127.0.0.1:8402/")).await.unwrap();
        assert_eq!(store.get_run("a").await.unwrap().unwrap().port, Some(8402));
        assert!(store.delete_run("a").await.unwrap());
        assert!(!store.delete_run("a").await.unwrap());
    }

    #[tokio::test]
    async fn active_runs_cannot_share_a_port() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_run(&run("a", Some(8401), 1)).await.unwrap();
        assert!(store.insert_run(&run("b", Some(8401), 2)).await.is_err());
        store.finish_run("a", RunState::Stopped, None, None, 3).await.unwrap();
        store.insert_run(&run("b", Some(8401), 2)).await.unwrap();
    }
}
```

Run: `cargo test -p pdt-store runs`
Expected: compile errors. Then implement, above the tests:

```rust
use crate::{Store, StoreError};
use pdt_core::run::{Run, RunKind, RunState};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "id, project_id, worktree_id, branch, name, kind, state, port, url, data_dir, cwd, exit_code, \
                       error, created_at, updated_at";
const ACTIVE: &str = "('setting_up', 'starting', 'ready', 'running')";

fn to_run(r: &SqliteRow) -> Result<Run, StoreError> {
    let kind: String = r.try_get("kind")?;
    let state: String = r.try_get("state")?;
    let port: Option<i64> = r.try_get("port")?;
    Ok(Run {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        worktree_id: r.try_get("worktree_id")?,
        branch: r.try_get("branch")?,
        name: r.try_get("name")?,
        kind: RunKind::parse(&kind).ok_or_else(|| StoreError::Corrupt(format!("run kind {kind:?}")))?,
        state: RunState::parse(&state).ok_or_else(|| StoreError::Corrupt(format!("run state {state:?}")))?,
        port: port.and_then(|p| u16::try_from(p).ok()),
        url: r.try_get("url")?,
        data_dir: r.try_get("data_dir")?,
        cwd: r.try_get("cwd")?,
        exit_code: r.try_get("exit_code")?,
        error: r.try_get("error")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

impl Store {
    pub async fn insert_run(&self, r: &Run) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO runs (id, project_id, worktree_id, branch, name, kind, state, port, url, data_dir, cwd, \
             exit_code, error, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.id)
        .bind(&r.project_id)
        .bind(&r.worktree_id)
        .bind(&r.branch)
        .bind(&r.name)
        .bind(r.kind.as_str())
        .bind(r.state.as_str())
        .bind(r.port.map(i64::from))
        .bind(&r.url)
        .bind(&r.data_dir)
        .bind(&r.cwd)
        .bind(r.exit_code)
        .bind(&r.error)
        .bind(r.created_at)
        .bind(r.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_run(&self, id: &str) -> Result<Option<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_run).transpose()
    }

    pub async fn list_runs(&self, project_id: &str) -> Result<Vec<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE project_id = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Every active run on this host, oldest first.
    pub async fn list_active_runs(&self) -> Result<Vec<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE state IN {ACTIVE} ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Moves to another state, clearing the previous exit code and error.
    pub async fn set_run_state(&self, id: &str, state: RunState, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET state = ?, exit_code = NULL, error = NULL, updated_at = ? WHERE id = ?")
            .bind(state.as_str())
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn finish_run(
        &self,
        id: &str,
        state: RunState,
        exit_code: Option<i32>,
        error: Option<&str>,
        at: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET state = ?, exit_code = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(state.as_str())
            .bind(exit_code)
            .bind(error)
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn set_run_port(&self, id: &str, port: Option<u16>, url: Option<&str>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET port = ?, url = ? WHERE id = ?")
            .bind(port.map(i64::from))
            .bind(url)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_run(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM runs WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}
```

Run: `cargo test -p pdt-store`
Expected: PASS.

- [ ] **Step 4: Git branch helpers (TDD)**

Add these tests to `crates/pdt-git/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn branches_local_and_remote_only() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        repo_with_commit(&origin);
        git_ok(&origin, &["branch", "feature/remote"]);
        let clone = tmp.path().join("clone");
        let ok = Command::new("git").args(["clone", "-q"]).arg(&origin).arg(&clone).status().unwrap().success();
        assert!(ok);
        git_ok(&clone, &["branch", "local-only"]);
        let b = list_branches(&clone).unwrap();
        let find = |n: &str| b.iter().find(|x| x.name == n).cloned();
        assert_eq!(find("main").map(|x| x.local), Some(true));
        assert_eq!(find("local-only").map(|x| x.local), Some(true));
        assert_eq!(find("feature/remote").map(|x| x.local), Some(false));
        assert!(find("HEAD").is_none(), "origin/HEAD is not a branch");
        assert!(branch_exists(&clone, "local-only"));
        assert!(!branch_exists(&clone, "feature/remote"), "remote-only is not local");
    }

    #[test]
    fn worktree_for_an_existing_or_remote_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        repo_with_commit(&origin);
        git_ok(&origin, &["branch", "feature/remote"]);
        let clone = tmp.path().join("clone");
        assert!(Command::new("git").args(["clone", "-q"]).arg(&origin).arg(&clone).status().unwrap().success());
        git_ok(&clone, &["branch", "existing"]);
        let a = clone.join(".worktrees").join("existing");
        let sha = add_worktree_for_branch(&clone, &a, "existing").unwrap();
        assert_eq!(sha, git_ok(&clone, &["rev-parse", "existing"]));
        assert_eq!(resolve_repo(&a).unwrap().current_branch.as_deref(), Some("existing"));
        let b = clone.join(".worktrees").join("feature-remote");
        add_worktree_for_branch(&clone, &b, "feature/remote").unwrap();
        assert_eq!(resolve_repo(&b).unwrap().current_branch.as_deref(), Some("feature/remote"));
        assert!(branch_exists(&clone, "feature/remote"), "now a local tracking branch");
        let c = clone.join(".worktrees").join("nope");
        assert!(matches!(add_worktree_for_branch(&clone, &c, "nope"), Err(GitError::UnknownRef(_))));
    }
```

Run: `cargo test -p pdt-git`
Expected: compile errors. Implement, above the tests:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    /// False when the branch exists only as `origin/<name>`.
    pub local: bool,
}

/// Local branches, then `origin` branches that have no local branch of the same name.
pub fn list_branches(repo: &Path) -> Result<Vec<BranchInfo>, GitError> {
    let out = match git_with_env(
        repo,
        &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes/origin"],
        &[],
    )? {
        Ok(s) => s,
        Err(f) => return Err(classify_failure(repo, f)),
    };
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in out.lines() {
        if let Some(n) = line.strip_prefix("refs/heads/") {
            local.push(n.to_string());
        } else if let Some(n) = line.strip_prefix("refs/remotes/origin/")
            && n != "HEAD"
        {
            remote.push(n.to_string());
        }
    }
    let mut v: Vec<BranchInfo> = local.iter().map(|n| BranchInfo { name: n.clone(), local: true }).collect();
    v.extend(remote.into_iter().filter(|n| !local.contains(n)).map(|name| BranchInfo { name, local: false }));
    Ok(v)
}

/// Whether `refs/heads/<branch>` exists.
pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    let r = format!("refs/heads/{branch}");
    matches!(git_with_env(repo, &["rev-parse", "--verify", "--quiet", &r], &[]), Ok(Ok(s)) if !s.is_empty())
}

/// Checks out an existing branch in a new worktree at `path`. A branch that exists only on `origin` becomes a local
/// branch tracking it. Returns the branch's head commit.
pub fn add_worktree_for_branch(repo: &Path, path: &Path, branch: &str) -> Result<String, GitError> {
    check_branch_name(repo, branch)?;
    if path.exists() {
        return Err(GitError::Command(format!("worktree path already exists: {}", path.display())));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| GitError::Command(format!("creating {}: {e}", parent.display())))?;
    }
    let path_str = path.to_string_lossy();
    let remote = format!("origin/{branch}");
    let args: Vec<&str> = if branch_exists(repo, branch) {
        vec!["worktree", "add", &path_str, branch]
    } else {
        let r = format!("refs/remotes/{remote}");
        if !matches!(git_with_env(repo, &["rev-parse", "--verify", "--quiet", &r], &[])?, Ok(s) if !s.is_empty()) {
            return Err(GitError::UnknownRef(branch.to_string()));
        }
        vec!["worktree", "add", "--track", "-b", branch, &path_str, &remote]
    };
    if let Err(f) = git_with_env(repo, &args, &[])? {
        return Err(GitError::Command(f.stderr));
    }
    match git_with_env(path, &["rev-parse", "HEAD"], &[])? {
        Ok(sha) => Ok(sha),
        Err(f) => Err(GitError::Command(f.stderr)),
    }
}
```

Run: `cargo test -p pdt-git`
Expected: PASS.

- [ ] **Step 5: Tree kill on Windows**

In `crates/pdt-agents/src/pty.rs`, in `Handle::kill`, replace the Windows branch:

```rust
        if cfg!(windows) {
            // Kill the whole tree: a run's `cmd.exe /c script.cmd` (or an npm shim) leaves its server or agent
            // running otherwise, holding ports and files. portable-pty 0.9.0's own Windows killer also inverts
            // TerminateProcess's result, so its result says nothing; killing our own child only fails if it exited.
            if let Some(pid) = self.pid {
                let mut cmd = std::process::Command::new("taskkill");
                cmd.args(["/T", "/F", "/PID", &pid.to_string()])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                }
                let _ = cmd.status();
            }
            return Ok(());
        }
```

Then remove the `#[cfg_attr(not(unix), allow(dead_code))]` on `pid`; it is used on both platforms now.

Run: `cargo test -p pdt-agents`
Expected: PASS (`kill_ends_the_process`, `shutdown_kills_everything`).

- [ ] **Step 6: Share the PTY WebSocket pump**

In `crates/pdt-api/src/pty_ws.rs`, split `connect` into the session check, followed by a reusable upgrade:

```rust
pub(crate) async fn connect(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    upgrade(st, id, ws)
}

/// Upgrades to the PTY protocol for terminal `pty_id` (a session id, or a run step's id).
pub(crate) fn upgrade(
    st: AppState,
    pty_id: String,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let ws = ws.map_err(|_| {
        ApiError::new(StatusCode::BAD_REQUEST, "websocket_required", "this endpoint needs a WebSocket upgrade")
    })?;
    let ptys = st.ptys.clone();
    let id = pty_id;
    Ok(match classify(&st, &id) {
        Outcome::Attach(attachment) => ws.on_upgrade(move |socket| pump(socket, ptys, id, attachment)),
        Outcome::Wait => ws.on_upgrade(move |socket| wait_for_resume(socket, st, id)),
        Outcome::NotRunning => ws.on_upgrade(not_running),
    })
}
```

Run: `cargo test -p pdt-api --test pty_ws`
Expected: PASS (unchanged behaviour).

- [ ] **Step 7: Write the failing run API tests**

`crates/pdt-api/tests/runs.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_tailscale::testing::RecordingTailscale;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A web server in node (CI has node for pnpm): answers every request with the run's branch.
const CONFIG: &str = r#"
[[run]]
name = "web"
setup = "node -e \"require('fs').writeFileSync(require('path').join(process.env.PDT_RUN_DATA, 'setup.txt'), String(process.env.PORT))\""
command = "node -e \"require('http').createServer((q, s) => s.end(process.env.PDT_RUN_BRANCH)).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true
data = true

[[run]]
name = "broken-setup"
setup = "node -e \"process.exit(3)\""
command = "node -e \"setTimeout(() => {}, 60000)\""

[[test]]
name = "ok"
command = "node -e \"process.exit(0)\""

[[test]]
name = "bad"
command = "node -e \"process.exit(1)\""
"#;

struct Env {
    app: axum::Router,
    state: AppState,
    pid: String,
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

/// A free range of 40 ports starting at a port the OS just handed out.
fn free_range() -> (u16, u16) {
    let p = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().min(65000);
    (p, p + 39)
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".pdt").join("project.toml"), CONFIG).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (start, end) = free_range();
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"runPorts": {"start": start, "end": end}}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    Env { app, state, pid: p["id"].as_str().unwrap().into(), repo, _tmp: tmp }
}

async fn start(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", &format!("/api/projects/{}/runs", env.pid), Some(body)).await
}

async fn wait_state(app: &axum::Router, id: &str, want: &[&str]) -> Value {
    for _ in 0..600 {
        let (_, r) = call(app, "GET", &format!("/api/runs/{id}"), None).await;
        if want.contains(&r["state"].as_str().unwrap_or("")) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, r) = call(app, "GET", &format!("/api/runs/{id}"), None).await;
    panic!("run {id} never reached {want:?}: {r}");
}

async fn http_get(port: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf
}

#[tokio::test]
async fn web_run_becomes_ready_and_serves_its_branch() {
    let env = setup().await;
    let (s, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["branch"], "main");
    let id = r["id"].as_str().unwrap();
    let r = wait_state(&env.app, id, &["ready", "failed"]).await;
    assert_eq!(r["state"], "ready", "{r}");
    let port = r["port"].as_u64().unwrap() as u16;
    assert_eq!(r["url"], format!("http://127.0.0.1:{port}/"));
    assert!(http_get(port).await.ends_with("main"));
    let data = PathBuf::from(r["dataDir"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(data.join("setup.txt")).unwrap(), port.to_string(), "setup saw $PORT");
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/runs", env.pid), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, active) = call(&env.app, "GET", "/api/runs", None).await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, id, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn stopping_a_web_run_frees_its_port() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    let port = r["port"].as_u64().unwrap() as u16;
    let (s, _) = call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let r = wait_state(&env.app, &id, &["stopped", "failed"]).await;
    assert_eq!(r["state"], "stopped", "{r}");
    let mut freed = false;
    for _ in 0..100 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(freed, "the node server (a grandchild of the shell) still holds port {port}");
    let (s, v) = call(&env.app, "POST", &format!("/api/runs/{id}/restart"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    assert_eq!(r["state"], "ready", "restarts after a stop: {r}");
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn failing_setup_fails_the_run() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "broken-setup", "kind": "run"})).await;
    let r = wait_state(&env.app, r["id"].as_str().unwrap(), &["failed", "ready"]).await;
    assert_eq!((r["state"].as_str(), r["exitCode"].as_i64()), (Some("failed"), Some(3)), "{r}");
    assert!(r["error"].as_str().unwrap().contains("setup"));
    assert_eq!(r["port"], Value::Null, "not a web run");
}

#[tokio::test]
async fn tests_pass_and_fail() {
    let env = setup().await;
    let (_, ok) = start(&env, json!({"name": "ok", "kind": "test"})).await;
    let (_, bad) = start(&env, json!({"name": "bad", "kind": "test"})).await;
    let ok = wait_state(&env.app, ok["id"].as_str().unwrap(), &["passed", "failed"]).await;
    let bad = wait_state(&env.app, bad["id"].as_str().unwrap(), &["passed", "failed"]).await;
    assert_eq!(ok["state"], "passed");
    assert_eq!((bad["state"].as_str(), bad["exitCode"].as_i64()), (Some("failed"), Some(1)));
    let (s, v) = start(&env, json!({"name": "nope", "kind": "test"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_run")));
}

#[tokio::test]
async fn the_run_limit_asks_before_stopping_another_run() {
    let env = setup().await;
    call(&env.app, "PUT", "/api/host/settings", Some(json!({"maxConcurrentRuns": 1}))).await;
    let (_, first) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let first = first["id"].as_str().unwrap().to_string();
    wait_state(&env.app, &first, &["ready"]).await;
    let (s, v) = start(&env, json!({"name": "broken-setup", "kind": "run"})).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("too_many_runs")), "{v}");
    let (s, v) = start(&env, json!({"name": "ok", "kind": "test"})).await;
    assert_eq!(s, StatusCode::CREATED, "tests do not count: {v}");
    let (s, v) = start(&env, json!({"name": "broken-setup", "kind": "run", "stopRunId": first})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    wait_state(&env.app, &first, &["stopped"]).await;
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn remote_runs_get_https_links_and_mappings() {
    let mut env = setup().await;
    let fake = Arc::new(RecordingTailscale::new(Ok(pdt_tailscale::TsStatus {
        dns_name: "box.t.ts.net".into(),
        self_login: None,
        https_enabled: true,
    })));
    env.state.tailscale = fake.clone();
    *env.state.remote.lock().unwrap() = pdt_api::RemoteStatus {
        enabled: true,
        active: true,
        dns_name: Some("box.t.ts.net".into()),
        ..Default::default()
    };
    env.app = router(env.state.clone(), None);
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready", "failed"]).await;
    let port = r["port"].as_u64().unwrap();
    assert_eq!(r["url"], format!("https://box.t.ts.net:{port}/"));
    assert!(fake.calls().contains(&format!("serve --bg --https={port} http://127.0.0.1:{port}")), "{:?}", fake.calls());
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    assert_eq!(fake.calls().last().unwrap(), &format!("serve --https={port} off"));
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn a_branch_without_a_worktree_gets_one() {
    let env = setup().await;
    git(&env.repo, &["branch", "feature/x"]);
    let (_, branches) = call(&env.app, "GET", &format!("/api/projects/{}/branches", env.pid), None).await;
    assert!(branches.as_array().unwrap().iter().any(|b| b["name"] == "feature/x"), "{branches}");
    let (s, r) = start(&env, json!({"branch": "feature/x", "name": "ok", "kind": "test"})).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["branch"], "feature/x");
    assert!(r["worktreeId"].is_string());
    let (_, wts) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert_eq!(wts.as_array().unwrap().len(), 1);
    wait_state(&env.app, r["id"].as_str().unwrap(), &["passed"]).await;
    let (s, r2) = start(&env, json!({"branch": "feature/x", "name": "ok", "kind": "test"})).await;
    assert_eq!((s, &r2["worktreeId"]), (StatusCode::CREATED, &r["worktreeId"]), "reuses the worktree");
    let (s, r3) = start(&env, json!({"branch": "main", "name": "ok", "kind": "test"})).await;
    assert_eq!((s, &r3["worktreeId"]), (StatusCode::CREATED, &Value::Null), "the main checkout's branch");
}

#[tokio::test]
async fn data_reset_and_removal_need_a_stopped_run() {
    let env = setup().await;
    let (_, r) = start(&env, json!({"name": "web", "kind": "run"})).await;
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_state(&env.app, &id, &["ready"]).await;
    let data = PathBuf::from(r["dataDir"].as_str().unwrap());
    let (s, v) = call(&env.app, "POST", &format!("/api/runs/{id}/reset-data"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("run_active")));
    let (s, _) = call(&env.app, "DELETE", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    call(&env.app, "POST", &format!("/api/runs/{id}/stop"), None).await;
    wait_state(&env.app, &id, &["stopped"]).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/runs/{id}/reset-data"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(data.is_dir() && std::fs::read_dir(&data).unwrap().next().is_none(), "emptied, still there");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&env.app, "GET", &format!("/api/runs/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
```

Run: `cargo test -p pdt-api --test runs`
Expected: compile errors or FAIL (no routes yet).

- [ ] **Step 8: Implement the run manager**

Add to `AppState`:

```rust
    /// The driver of each active run in this daemon run (stop requests, completion).
    pub(crate) run_controls:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<crate::run_manager::RunControl>>>>,
```

It is initialised with `Default::default()`.

`crates/pdt-api/src/run_manager.rs`:

```rust
//! Drives run instances (spec §4.2): setup, command, readiness, Tailscale mapping, stop, resume after a restart.

use crate::routes::{AppState, now_ms};
use pdt_agents::SpawnSpec;
use pdt_core::host::CredentialPolicy;
use pdt_core::run::{Run, RunKind, RunState, RunStep, pick_port, pty_id, step_launch};
use pdt_core::runconfig::{RunDef, RunVars, expand, run_env};
use pdt_core::session::{SessionKind, is_scrubbed};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

const PROBE_EVERY: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(crate) struct RunControl {
    stop: AtomicBool,
    /// Why the run was stopped by PDT rather than the user (e.g. its branch is gone).
    reason: std::sync::Mutex<Option<String>>,
    finished: AtomicBool,
    done: tokio::sync::Notify,
}

impl RunControl {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

pub(crate) fn port_is_free(p: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()
}

fn remote_host(st: &AppState) -> Option<String> {
    let r = st.remote.lock().unwrap_or_else(|e| e.into_inner());
    r.active.then(|| r.dns_name.clone()).flatten()
}

/// The link for a web run on `port`: the tailnet when remote access is active, else loopback.
pub(crate) fn run_url(st: &AppState, port: u16) -> String {
    match remote_host(st) {
        Some(h) => format!("https://{h}:{port}/"),
        None => format!("http://127.0.0.1:{port}/"),
    }
}

fn vars(st: &AppState, run: &Run) -> RunVars {
    RunVars {
        port: run.port,
        data_dir: run.data_dir.clone(),
        url: run.url.clone(),
        host: remote_host(st).unwrap_or_else(|| "127.0.0.1".into()),
        branch: run.branch.clone(),
        owner_login: st.owner(),
    }
}

/// A free port for a new or resumed web run, avoiding every other active run's port.
pub(crate) async fn allocate_port(st: &AppState, prefer: Option<u16>, except_run: Option<&str>) -> Option<u16> {
    let range = st.store.get_host_settings().await.ok()?.run_ports;
    let taken: BTreeSet<u16> = st
        .store
        .list_active_runs()
        .await
        .ok()?
        .into_iter()
        .filter(|r| Some(r.id.as_str()) != except_run)
        .filter_map(|r| r.port)
        .collect();
    tokio::task::spawn_blocking(move || pick_port(range, &taken, prefer, port_is_free)).await.ok()?
}

fn spawn_step(
    st: &AppState,
    run: &Run,
    step: RunStep,
    command: &str,
    env: &[(String, String)],
) -> Result<oneshot::Receiver<i32>, String> {
    let id = pty_id(&run.id, step);
    st.ptys.remove(&id);
    let script = st.data_dir.join("runs").join("scripts").join(&run.id).join(format!("{}.cmd", step.as_str()));
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
    let l = step_launch(st.platform.windows, &comspec, command, &script);
    if let Some(text) = &l.script {
        std::fs::create_dir_all(script.parent().expect("script dir")).map_err(|e| e.to_string())?;
        std::fs::write(&script, text).map_err(|e| format!("writing {}: {e}", script.display()))?;
    }
    let spec = SpawnSpec {
        id,
        program: PathBuf::from(l.program),
        args: l.args,
        cwd: PathBuf::from(&run.cwd),
        env_remove: std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| is_scrubbed(SessionKind::Shell, k, CredentialPolicy::PassThrough))
            .collect(),
        env: env.to_vec(),
        cols: 120,
        rows: 30,
    };
    st.ptys.spawn(spec).map_err(|e| e.to_string())
}

async fn serve_port(st: &AppState, port: u16, on: bool) {
    let ts = st.tailscale.clone();
    let r = tokio::task::spawn_blocking(move || if on { ts.serve(port, port) } else { ts.unserve(port) }).await;
    if let Ok(Err(e)) = r {
        tracing::warn!(port, on, error = %e, "tailscale mapping for a run failed");
    }
}

type Outcome = (RunState, Option<i32>, Option<String>);

async fn drive(st: &AppState, ctl: &RunControl, run: &Run, def: &RunDef) -> Outcome {
    let v = vars(st, run);
    let pairs = v.pairs();
    let mut env = vec![("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".into(), "truecolor".into())];
    if let Some(path) = &st.platform.login_path {
        env.push(("PATH".into(), path.clone()));
    }
    env.extend(run_env(def, &v));
    if let Some(dir) = &run.data_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Some(setup) = &def.setup {
        let _ = st.store.set_run_state(&run.id, RunState::SettingUp, now_ms()).await;
        let rx = match spawn_step(st, run, RunStep::Setup, &expand(setup, &pairs), &env) {
            Ok(rx) => rx,
            Err(e) => return (RunState::Failed, None, Some(format!("setup could not start: {e}"))),
        };
        let code = rx.await.unwrap_or(-1);
        if ctl.stopping() {
            return (RunState::Stopped, None, None);
        }
        if code != 0 {
            return (RunState::Failed, Some(code), Some(format!("setup exited with code {code}")));
        }
    }
    if ctl.stopping() {
        return (RunState::Stopped, None, None);
    }
    let first = if run.kind == RunKind::Test { RunState::Running } else { RunState::Starting };
    let _ = st.store.set_run_state(&run.id, first, now_ms()).await;
    let mut exit = match spawn_step(st, run, RunStep::Command, &expand(&def.command, &pairs), &env) {
        Ok(rx) => rx,
        Err(e) => return (RunState::Failed, None, Some(format!("the command could not start: {e}"))),
    };
    let mapped = match (def.web, run.port) {
        (true, Some(p)) if remote_host(st).is_some() => {
            serve_port(st, p, true).await;
            Some(p)
        }
        _ => None,
    };
    let mut ready = run.kind == RunKind::Test;
    if run.kind == RunKind::Run && !def.web {
        let _ = st.store.set_run_state(&run.id, RunState::Ready, now_ms()).await;
        ready = true;
    }
    let code = loop {
        tokio::select! {
            c = &mut exit => break c.unwrap_or(-1),
            _ = tokio::time::sleep(PROBE_EVERY), if !ready => {
                if let Some(p) = run.port
                    && tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok()
                {
                    ready = true;
                    let _ = st.store.set_run_state(&run.id, RunState::Ready, now_ms()).await;
                }
            }
        }
    };
    if let Some(p) = mapped {
        serve_port(st, p, false).await;
    }
    if ctl.stopping() {
        let reason = ctl.reason.lock().unwrap_or_else(|e| e.into_inner()).clone();
        return (RunState::Stopped, Some(code), reason);
    }
    match run.kind {
        RunKind::Test if code == 0 => (RunState::Passed, Some(0), None),
        RunKind::Test => (RunState::Failed, Some(code), Some(format!("tests failed (exit code {code})"))),
        RunKind::Run => (RunState::Failed, Some(code), Some(format!("the command exited with code {code}"))),
    }
}

/// Starts driving `run` in the background. The row must already be stored.
pub(crate) fn start(st: &AppState, run: Run, def: RunDef) {
    let ctl = Arc::new(RunControl::default());
    st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).insert(run.id.clone(), ctl.clone());
    let st = st.clone();
    tokio::spawn(async move {
        let (state, code, error) = drive(&st, &ctl, &run, &def).await;
        // During shutdown the row stays active, so the next pdtd resumes it.
        if !st.shutting_down.load(Ordering::SeqCst)
            && let Err(e) = st.store.finish_run(&run.id, state, code, error.as_deref(), now_ms()).await
        {
            tracing::warn!(run = %run.id, error = %e, "could not record the run's outcome");
        }
        ctl.finished.store(true, Ordering::SeqCst);
        ctl.done.notify_waiters();
        let mut controls = st.run_controls.lock().unwrap_or_else(|e| e.into_inner());
        if controls.get(&run.id).is_some_and(|c| Arc::ptr_eq(c, &ctl)) {
            controls.remove(&run.id);
        }
    });
}

/// Asks a run to stop and waits (up to 20 s) for its driver to finish. A run with no driver (left active by a
/// previous daemon and not resumed yet) is marked stopped directly.
pub(crate) async fn stop(st: &AppState, id: &str, reason: Option<String>) {
    let ctl = st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
    match ctl {
        Some(ctl) => {
            *ctl.reason.lock().unwrap_or_else(|e| e.into_inner()) = reason;
            ctl.stop.store(true, Ordering::SeqCst);
            for step in [RunStep::Setup, RunStep::Command] {
                let _ = st.ptys.kill(&pty_id(id, step));
            }
            let wait = async {
                while !ctl.finished.load(Ordering::SeqCst) {
                    let notified = ctl.done.notified();
                    if ctl.finished.load(Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(20), wait).await;
        }
        None => {
            if let Ok(Some(r)) = st.store.get_run(id).await
                && r.state.is_active()
            {
                let _ = st.store.finish_run(id, RunState::Stopped, None, reason.as_deref(), now_ms()).await;
            }
        }
    }
}

/// Restarts the runs a previous pdtd left active (spec §4.2 "Restarts"). Returns how many started.
pub async fn resume_runs(st: &AppState) -> usize {
    let Ok(active) = st.store.list_active_runs().await else { return 0 };
    let mut started = 0;
    for mut run in active {
        if st.shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let def = match crate::runs::definition_for(st, &run).await {
            Ok(d) => d,
            Err(msg) => {
                let _ = st.store.finish_run(&run.id, RunState::Failed, None, Some(&msg), now_ms()).await;
                continue;
            }
        };
        if def.web {
            match allocate_port(st, run.port, Some(&run.id)).await {
                None => {
                    let msg = "no free port in the host's run port range";
                    let _ = st.store.finish_run(&run.id, RunState::Failed, None, Some(msg), now_ms()).await;
                    continue;
                }
                Some(p) => {
                    if let Some(old) = run.port.filter(|o| *o != p)
                        && remote_host(st).is_some()
                    {
                        serve_port(st, old, false).await;
                    }
                    let url = run_url(st, p);
                    let _ = st.store.set_run_port(&run.id, Some(p), Some(&url)).await;
                    run.port = Some(p);
                    run.url = Some(url);
                }
            }
        }
        start(st, run, def);
        started += 1;
    }
    started
}

/// Stops active runs whose worktree folder or branch has gone (spec §4.2: merged or deleted branches).
pub async fn sweep_gone_runs(st: &AppState) {
    let Ok(active) = st.store.list_active_runs().await else { return };
    for run in active {
        let Some(wid) = &run.worktree_id else { continue };
        let wt = st.store.get_worktree(wid).await.ok().flatten();
        let project = st.store.get_project(&run.project_id).await.ok().flatten();
        let gone = match (wt, project) {
            (Some(wt), Some(p)) => {
                let (repo, branch, path) = (PathBuf::from(p.repo_path), run.branch.clone(), PathBuf::from(wt.path));
                tokio::task::spawn_blocking(move || !path.is_dir() || !pdt_git::branch_exists(&repo, &branch))
                    .await
                    .unwrap_or(false)
            }
            _ => true,
        };
        if gone {
            tracing::info!(run = %run.id, branch = %run.branch, "stopping a run whose branch or worktree is gone");
            stop(st, &run.id, Some("its branch or worktree is gone".into())).await;
        }
    }
}
```

In `remote.rs`, replace `extra_ports` with:

```rust
/// Web runs' ports, mapped alongside pdtd while remote access is active.
pub(crate) async fn extra_ports(st: &AppState) -> Vec<u16> {
    st.store.list_active_runs().await.map(|v| v.into_iter().filter_map(|r| r.port).collect()).unwrap_or_default()
}
```

- [ ] **Step 9: Implement the run routes**

`crates/pdt-api/src/runs.rs`:

```rust
use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::run_manager;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use pdt_core::run::{NewRun, Run, RunKind, RunState, RunStep, pty_id, run_data_dir};
use pdt_core::runconfig::RunDef;
use pdt_core::worktree::{WORKTREES_DIR, Worktree, worktree_path};
use serde::Serialize;
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Branch {
    pub name: String,
    pub local: bool,
}

async fn load(st: &AppState, id: &str) -> Result<Run, ApiError> {
    st.store.get_run(id).await?.ok_or_else(|| ApiError::not_found("run"))
}

fn conflict(code: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, msg)
}

/// The definition named by `run` in the configuration of its own checkout. Err is a message for the run's error.
pub(crate) async fn definition_for(st: &AppState, run: &Run) -> Result<RunDef, String> {
    let cfg = crate::runconfig::load_effective(st, &run.project_id, std::path::Path::new(&run.cwd))
        .await
        .map_err(|e| format!("the run configuration is invalid: {}", e.message()))?;
    let list = if run.kind == RunKind::Test { cfg.tests } else { cfg.runs };
    list.into_iter().find(|d| d.name == run.name).ok_or_else(|| format!("the configuration has no \"{}\"", run.name))
}

pub(crate) async fn list(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Run>>, ApiError> {
    st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    Ok(Json(st.store.list_runs(&id).await?))
}

pub(crate) async fn list_active(State(st): State<AppState>) -> Result<Json<Vec<Run>>, ApiError> {
    Ok(Json(st.store.list_active_runs().await?))
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Run>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn branches(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<Branch>>, ApiError> {
    let p = st.store.get_project(&id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let repo = PathBuf::from(p.repo_path);
    let list = tokio::task::spawn_blocking(move || pdt_git::list_branches(&repo)).await.map_err(ApiError::internal)??;
    Ok(Json(list.into_iter().map(|b| Branch { name: b.name, local: b.local }).collect()))
}

/// Resolves where a new run goes: an existing worktree, a branch (creating its worktree), or the main checkout.
async fn place(st: &AppState, project: &pdt_core::project::Project, input: &NewRun) -> Result<Option<String>, ApiError> {
    if let Some(w) = &input.worktree_id {
        return Ok(Some(w.clone()));
    }
    let Some(branch) = input.branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) else { return Ok(None) };
    if let Some(wt) = st.store.list_worktrees(&project.id).await?.into_iter().find(|w| w.branch == branch) {
        return Ok(Some(wt.id));
    }
    let (_, main_branch) = crate::runconfig::checkout_dir(st, project, None).await?;
    if main_branch == branch {
        return Ok(None);
    }
    let repo = PathBuf::from(&project.repo_path);
    let path = worktree_path(&repo, branch);
    let (b, target) = (branch.to_string(), path.clone());
    let base_commit = tokio::task::spawn_blocking(move || {
        pdt_git::ensure_excluded(&repo, &format!("/{WORKTREES_DIR}/"))?;
        pdt_git::add_worktree_for_branch(&repo, &target, &b)
    })
    .await
    .map_err(ApiError::internal)??;
    let wt = Worktree {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id.clone(),
        branch: branch.to_string(),
        path: path.to_string_lossy().into_owned(),
        base_commit,
        created_at: now_ms(),
    };
    st.store.insert_worktree(&wt).await?;
    Ok(Some(wt.id))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    Path(project_id): Path<String>,
    payload: Result<Json<NewRun>, JsonRejection>,
) -> Result<(StatusCode, Json<Run>), ApiError> {
    let Json(input) = payload?;
    let project = st.store.get_project(&project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    let worktree_id = place(&st, &project, &input).await?;
    let (cwd, branch) = crate::runconfig::checkout_dir(&st, &project, worktree_id.as_deref()).await?;
    let cfg = crate::runconfig::load_effective(&st, &project.id, &cwd).await?;
    let list = if input.kind == RunKind::Test { &cfg.tests } else { &cfg.runs };
    let def = list.iter().find(|d| d.name == input.name).cloned().ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, "unknown_run", format!("the configuration has no \"{}\"", input.name))
    })?;
    if input.kind == RunKind::Run {
        let settings = st.store.get_host_settings().await?;
        let active: Vec<Run> =
            st.store.list_active_runs().await?.into_iter().filter(|r| r.kind == RunKind::Run).collect();
        if active.len() as u32 >= settings.max_concurrent_runs {
            match input.stop_run_id.as_deref().filter(|id| active.iter().any(|r| r.id == *id)) {
                Some(id) => run_manager::stop(&st, id, None).await,
                None => {
                    let oldest = &active[0];
                    return Err(conflict(
                        "too_many_runs",
                        format!(
                            "{} runs are active (the maximum is {}). Stop \"{} · {}\" to start this one?",
                            active.len(),
                            settings.max_concurrent_runs,
                            oldest.name,
                            oldest.branch
                        ),
                    ));
                }
            }
        }
    }
    let port = if def.web {
        Some(run_manager::allocate_port(&st, None, None).await.ok_or_else(|| {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no_free_port", "no free port in the host's run port range")
        })?)
    } else {
        None
    };
    let data_dir =
        def.data.then(|| run_data_dir(&st.data_dir, &project.id, &branch, &def.name).to_string_lossy().into_owned());
    let now = now_ms();
    let run = Run {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: project.id,
        worktree_id,
        branch,
        name: def.name.clone(),
        kind: input.kind,
        state: if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting },
        port,
        url: port.map(|p| run_manager::run_url(&st, p)),
        data_dir,
        cwd: cwd.to_string_lossy().into_owned(),
        exit_code: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_run(&run).await?;
    run_manager::start(&st, run.clone(), def);
    Ok((StatusCode::CREATED, Json(run)))
}

pub(crate) async fn stop(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    load(&st, &id).await?;
    run_manager::stop(&st, &id, None).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Run>, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        run_manager::stop(&st, &id, None).await;
    }
    let def = definition_for(&st, &run).await.map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_run_config", m))?;
    let mut run = load(&st, &id).await?;
    if def.web {
        let p = run_manager::allocate_port(&st, run.port, Some(&id)).await.ok_or_else(|| {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no_free_port", "no free port in the host's run port range")
        })?;
        let url = run_manager::run_url(&st, p);
        st.store.set_run_port(&id, Some(p), Some(&url)).await?;
        run.port = Some(p);
        run.url = Some(url);
    }
    let first = if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting };
    st.store.set_run_state(&id, first, now_ms()).await?;
    run.state = first;
    run_manager::start(&st, run.clone(), def);
    Ok(Json(run))
}

pub(crate) async fn reset_data(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(conflict("run_active", "stop the run before resetting its data"));
    }
    if let Some(dir) = run.data_dir.map(PathBuf::from) {
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            std::fs::create_dir_all(&dir)
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let run = load(&st, &id).await?;
    if run.state.is_active() {
        return Err(conflict("run_active", "stop the run before removing it"));
    }
    for step in [RunStep::Setup, RunStep::Command] {
        st.ptys.remove(&pty_id(&id, step));
    }
    let _ = std::fs::remove_dir_all(st.data_dir.join("runs").join("scripts").join(&id));
    st.store.delete_run(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn pty(
    State(st): State<AppState>,
    Path((id, step)): Path<(String, String)>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    load(&st, &id).await?;
    let step = RunStep::parse(&step).ok_or_else(|| ApiError::not_found("run step"))?;
    crate::pty_ws::upgrade(st, pty_id(&id, step), ws)
}
```

`ApiError` needs a `message()` accessor: add `pub fn message(&self) -> &str { &self.message }` in `error.rs`.

Register `mod runs; mod run_manager;` and export `pub use run_manager::{resume_runs, sweep_gone_runs};` in `lib.rs`. Add the routes:

```rust
        .route("/projects/{id}/runs", get(crate::runs::list).post(crate::runs::create))
        .route("/projects/{id}/branches", get(crate::runs::branches))
        .route("/runs", get(crate::runs::list_active))
        .route("/runs/{id}", get(crate::runs::get_one).delete(crate::runs::remove))
        .route("/runs/{id}/stop", axum::routing::post(crate::runs::stop))
        .route("/runs/{id}/restart", axum::routing::post(crate::runs::restart))
        .route("/runs/{id}/reset-data", axum::routing::post(crate::runs::reset_data))
        .route("/runs/{id}/pty/{step}", get(crate::runs::pty))
```

Run: `cargo test -p pdt-api --test runs`
Expected: PASS on Windows. If `stopping_a_web_run_frees_its_port` fails, check first that `taskkill` reached the tree (Step 5), before you change the test.

- [ ] **Step 10: Resume and sweep in `pdtd`, with lifecycle tests**

In `crates/pdtd/src/main.rs`, next to the session resume (after the remote-access task), add:

```rust
    {
        let st = state.clone();
        tokio::spawn(async move {
            let resumed = pdt_api::resume_runs(&st).await;
            if resumed > 0 {
                tracing::info!(runs = resumed, "resumed runs from the previous daemon");
            }
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                pdt_api::sweep_gone_runs(&st).await;
            }
        });
    }
```

Add to `crates/pdtd/tests/lifecycle.rs`:

```rust
const RUN_CONFIG: &str = r#"
[[run]]
name = "web"
command = "node -e \"require('http').createServer((q, s) => s.end('ok')).listen(Number(process.env.PORT), '127.0.0.1')\""
web = true
"#;

fn wait_run(port: u16, id: &str, want: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, r) = http(port, "GET", &format!("/api/runs/{id}"), None);
        if r["state"] == want {
            return r;
        }
        assert!(Instant::now() < deadline, "run never became {want}: {r}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A project whose repo has RUN_CONFIG, a run port range starting at a free port, and a started web run.
fn project_with_web_run(port: u16, tmp: &Path) -> (String, u16) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    std::fs::write(repo.join(".pdt").join("project.toml"), RUN_CONFIG).unwrap();
    let base = free_port().min(65000);
    let body = serde_json::json!({"runPorts": {"start": base, "end": base + 39}}).to_string();
    assert_eq!(http(port, "PUT", "/api/host/settings", Some(&body)).0, 200);
    let body = serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string();
    let (_, p) = http(port, "POST", "/api/projects", Some(&body));
    let body = serde_json::json!({"name": "web", "kind": "run"}).to_string();
    let (s, r) = http(port, "POST", &format!("/api/projects/{}/runs", p["id"].as_str().unwrap()), Some(&body));
    assert_eq!(s, 201, "{r}");
    let id = r["id"].as_str().unwrap().to_string();
    let r = wait_run(port, &id, "ready");
    (id, r["port"].as_u64().unwrap() as u16)
}

#[test]
fn active_runs_resume_on_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let mut d = start(port, &data);
    let (id, run_port) = project_with_web_run(port, tmp.path());
    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    wait_exit(&mut d, Duration::from_secs(20));
    let _d = start(port, &data);
    let r = wait_run(port, &id, "ready");
    assert_eq!(r["port"].as_u64(), Some(run_port as u64), "same port when it is still free");
}

#[test]
fn resume_moves_to_a_free_port_when_the_old_one_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let port = free_port();
    let mut d = start(port, &data);
    let (id, run_port) = project_with_web_run(port, tmp.path());
    assert_eq!(http(port, "POST", "/api/shutdown", None).0, 202);
    wait_exit(&mut d, Duration::from_secs(20));
    let mut squatter = None;
    for _ in 0..100 {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", run_port)) {
            squatter = Some(l);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _squatter = squatter.expect("the old run's port was freed by the shutdown");
    let _d = start(port, &data);
    let r = wait_run(port, &id, "ready");
    assert_ne!(r["port"].as_u64(), Some(run_port as u64));
}
```

Run: `cargo test -p pdtd`
Expected: PASS.

- [ ] **Step 11: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add crates web/src/api/gen
git commit -m "feat: run instances with setup, readiness, HTTPS mapping, limits, resume and sweep"
```

---

### Task 8: Runs UI

**Files:**
- Create:
  - `web/src/runs/useStartRun.ts`
  - `web/src/runs/RunsSection.tsx`, `web/src/runs/RunBranchForm.tsx`, `web/src/runs/RunView.tsx`
  - `web/src/runs/ProjectSettingsPanel.tsx`, `web/src/runs/runState.ts`
  - `web/src/runs/runs.test.tsx`
- Modify:
  - `web/src/api/client.ts`
  - `web/src/terminal/ptySocket.ts`, `web/src/terminal/ptySocket.test.ts`, `web/src/terminal/TerminalView.tsx`
  - `web/src/sessions/SessionList.tsx`, `web/src/shell/AppShell.tsx`
  - `web/src/test-utils.tsx`

**Interfaces:**
- Consumes: the Task 7 routes. Generated types: `Run`, `RunKind`, `RunState`, `NewRun`, `Branch`, `RunConfigView`, `RunConfig`, `RunDef`, `ProjectSettingsView`, `ProjectSettingsInput` and `CredentialPolicy`.
- Produces:
  - `api.listRuns`, `listActiveRuns`, `startRun`, `stopRun`, `restartRun`, `resetRunData`, `deleteRun`, `runConfig`, `listBranches`, `projectSettings` and `saveProjectSettings`.
  - `runPtyUrl(runId, step)`.
  - `TerminalView`'s new optional `url` prop.
  - `Reply` in `test-utils` (stub non-200 responses).
  - `SessionList`'s new optional props `selectedRunId`, `onSelectRun(id)` and `onOpenSettings()`.

- [ ] **Step 1: Client, URL helper and test utility**

Append to `web/src/api/client.ts`. Add the imports first:

```ts
import type { Branch } from './gen/Branch';
import type { NewRun } from './gen/NewRun';
import type { ProjectSettingsInput } from './gen/ProjectSettingsInput';
import type { ProjectSettingsView } from './gen/ProjectSettingsView';
import type { Run } from './gen/Run';
import type { RunConfigView } from './gen/RunConfigView';
```

Then add these entries inside `api`:

```ts
  listRuns: (projectId: string) => request<Run[]>(`/api/projects/${enc(projectId)}/runs`),
  listActiveRuns: () => request<Run[]>('/api/runs'),
  startRun: (projectId: string, r: NewRun) =>
    request<Run>(`/api/projects/${enc(projectId)}/runs`, { method: 'POST', body: JSON.stringify(r) }),
  stopRun: (id: string) => request<void>(`/api/runs/${enc(id)}/stop`, { method: 'POST' }),
  restartRun: (id: string) => request<Run>(`/api/runs/${enc(id)}/restart`, { method: 'POST' }),
  resetRunData: (id: string) => request<void>(`/api/runs/${enc(id)}/reset-data`, { method: 'POST' }),
  deleteRun: (id: string) => request<void>(`/api/runs/${enc(id)}`, { method: 'DELETE' }),
  runConfig: (projectId: string, worktreeId: string | null) =>
    request<RunConfigView>(
      `/api/projects/${enc(projectId)}/run-config${worktreeId ? `?worktreeId=${enc(worktreeId)}` : ''}`,
    ),
  listBranches: (projectId: string) => request<Branch[]>(`/api/projects/${enc(projectId)}/branches`),
  projectSettings: (projectId: string) => request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`),
  saveProjectSettings: (projectId: string, s: ProjectSettingsInput) =>
    request<ProjectSettingsView>(`/api/projects/${enc(projectId)}/settings`, { method: 'PUT', body: JSON.stringify(s) }),
```

In `web/src/terminal/ptySocket.ts`, below `ptyUrl`:

```ts
export function runPtyUrl(
  runId: string,
  step: 'setup' | 'command',
  loc: Pick<Location, 'protocol' | 'host'> = window.location,
): string {
  const scheme = loc.protocol === 'https:' ? 'wss' : 'ws';
  return `${scheme}://${loc.host}/api/runs/${encodeURIComponent(runId)}/pty/${step}`;
}
```

Add to `ptySocket.test.ts`:

```ts
describe('runPtyUrl', () => {
  it('builds ws and wss URLs for a run step', () => {
    expect(runPtyUrl('r 1', 'setup', { protocol: 'http:', host: 'h:1' })).toBe('ws://h:1/api/runs/r%201/pty/setup');
    expect(runPtyUrl('r', 'command', { protocol: 'https:', host: 'box.t.ts.net' })).toBe(
      'wss://box.t.ts.net/api/runs/r/pty/command',
    );
  });
});
```

Import `runPtyUrl` there.

In `TerminalView.tsx`:
- add the prop `url?: string`, documented as `/** WebSocket URL; defaults to the session's PTY URL. */`;
- use `new PtySocket(url ?? ptyUrl(sessionId), …)`;
- change the effect dependencies to `[sessionId, url]`.

In `web/src/test-utils.tsx`, add a non-200 reply:

```ts
/** A stub route value that answers with a specific status and JSON body. */
export class Reply {
  constructor(public status: number, public body: unknown) {}
}
```

In `stubApi`, before the `value === undefined` check, add:

```ts
    if (value instanceof Reply) return new Response(JSON.stringify(value.body), { status: value.status });
```

Run: `pnpm --dir web test -- ptySocket`
Expected: PASS.

- [ ] **Step 2: Write the failing UI tests**

`web/src/runs/runs.test.tsx`:

```tsx
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { ProjectSettingsPanel } from './ProjectSettingsPanel';
import { RunBranchForm } from './RunBranchForm';
import { RunsSection } from './RunsSection';
import { RunView } from './RunView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ url }: { url?: string }) => <div data-testid="terminal-view">{url}</div>,
}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const run = (over: Partial<Run>): Run => ({
  id: 'r1', projectId: 'p', worktreeId: null, branch: 'main', name: 'web', kind: 'run', state: 'ready',
  port: 8401, url: 'https://box.t.ts.net:8401/', dataDir: '/d', cwd: '/r', exitCode: null, error: null,
  createdAt: 1, updatedAt: 1, ...over,
});
const config = {
  filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
  effective: {
    runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }],
    tests: [{ name: 'unit', setup: null, command: 'y', web: false, data: false, env: {} }],
  },
};

describe('RunsSection', () => {
  it('starts a configured run or test and selects it', async () => {
    const calls = stubApi({
      'GET /api/projects/p/runs': [run({ id: 'old', state: 'stopped' })],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': (c) => run({ id: 'new', ...(c.body as object) }),
    });
    const onSelectRun = vi.fn();
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={onSelectRun} />);
    expect(await screen.findByRole('button', { name: /web · stopped/ })).toBeInTheDocument();
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in Repository' }), 'test:unit');
    await waitFor(() => expect(onSelectRun).toHaveBeenCalledWith('new'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ name: 'unit', kind: 'test' });
  });

  it('asks before stopping the oldest run when the host is at its limit', async () => {
    let posts = 0;
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config': config,
      'GET /api/runs': [run({ id: 'oldest', name: 'web', branch: 'b1' }), run({ id: 'newer', branch: 'b2' })],
      'POST /api/projects/p/runs': (c) =>
        ++posts === 1
          ? new Reply(409, { code: 'too_many_runs', message: 'Stop "web · b1" to start this one?' })
          : run({ id: 'new', ...(c.body as object) }),
    });
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<RunsSection project={project} worktreeId="w" label="feature/x" selectedRunId={null} onSelectRun={() => {}} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in feature/x' }), 'run:web');
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    expect(confirm).toHaveBeenCalledWith('Stop "web · b1" to start this one?');
    expect(calls.filter((c) => c.method === 'POST')[1].body).toEqual({ worktreeId: 'w', name: 'web', kind: 'run', stopRunId: 'oldest' });
  });

  it('does nothing more when the user declines', async () => {
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': new Reply(409, { code: 'too_many_runs', message: 'Stop?' }),
    });
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={() => {}} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in Repository' }), 'run:web');
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows a configuration error instead of the menu', async () => {
    stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config': { ...config, effective: null, error: '"x" has no command' },
    });
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={() => {}} />);
    expect(await screen.findByText(/has no command/)).toBeInTheDocument();
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
  });
});

describe('RunBranchForm', () => {
  it('runs a branch that has no worktree yet', async () => {
    const calls = stubApi({
      'GET /api/projects/p/branches': [{ name: 'main', local: true }, { name: 'feature/y', local: false }],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': (c) => run({ id: 'n', ...(c.body as object) }),
    });
    const onSelectRun = vi.fn();
    renderWithQuery(<RunBranchForm project={project} onSelectRun={onSelectRun} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Branch' }), 'feature/y');
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'What to run' }), 'run:web');
    await userEvent.click(screen.getByRole('button', { name: 'Run branch' }));
    await waitFor(() => expect(onSelectRun).toHaveBeenCalledWith('n'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ branch: 'feature/y', name: 'web', kind: 'run' });
  });
});

describe('RunView', () => {
  it('links a ready web run and stops it', async () => {
    const calls = stubApi({ 'POST /api/runs/r1/stop': undefined, 'GET /api/projects/p/runs': [] });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    expect(screen.getByRole('link', { name: 'https://box.t.ts.net:8401/' })).toHaveAttribute('target', '_blank');
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/command');
    await userEvent.click(screen.getByRole('tab', { name: 'Setup' }));
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/setup');
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/runs/r1/stop')).toBe(true));
  });

  it('shows why a run failed and offers restart, reset data and remove', async () => {
    const calls = stubApi({
      'POST /api/runs/r1/restart': run({}),
      'POST /api/runs/r1/reset-data': undefined,
      'DELETE /api/runs/r1': undefined,
      'GET /api/projects/p/runs': [],
    });
    const onRemoved = vi.fn();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<RunView run={run({ state: 'failed', error: 'setup exited with code 3', exitCode: 3 })} onRemoved={onRemoved} />);
    expect(screen.getByRole('alert')).toHaveTextContent('setup exited with code 3');
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset data' }));
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual(
      expect.arrayContaining(['POST /api/runs/r1/reset-data', 'POST /api/runs/r1/restart', 'DELETE /api/runs/r1']),
    );
  });
});

describe('ProjectSettingsPanel', () => {
  it('saves run overrides and the credential policy, and shows validation errors', async () => {
    let attempt = 0;
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null },
      'GET /api/projects/p/run-config': { ...config, file: '[[run]]\nname = "web"\n' },
      'PUT /api/projects/p/settings': (c) =>
        ++attempt === 1
          ? new Reply(400, { code: 'invalid_run_config', message: 'the host overrides is not valid: x' })
          : { agentCredentials: 'pass_through', effectiveCredentials: 'pass_through', runOverrides: (c.body as { runOverrides: string }).runOverrides },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText(/name = "web"/)).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Host overrides (TOML)' }), 'bad');
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Agent credentials' }), 'pass_through');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('not valid');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT')[1].body).toEqual({ agentCredentials: 'pass_through', runOverrides: 'bad' });
    const status = await screen.findByRole('status');
    expect(within(status).getByText('Saved')).toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web test -- runs`
Expected: FAIL (the modules do not exist).

- [ ] **Step 3: Implement the components**

`web/src/runs/runState.ts`:

```ts
import type { Status } from '@ui';
import type { RunState } from '../api/gen/RunState';

export function runDot(state: RunState): Status {
  switch (state) {
    case 'setting_up':
    case 'starting': return 'waiting';
    case 'ready':
    case 'running':
    case 'passed': return 'running';
    case 'failed': return 'failed';
    case 'stopped': return 'idle';
  }
}

export const runLabel = (state: RunState) => state.replace('_', ' ');
export const isActive = (state: RunState) => ['setting_up', 'starting', 'ready', 'running'].includes(state);
```

`web/src/runs/useStartRun.ts`:

```ts
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api, ApiError } from '../api/client';
import type { NewRun } from '../api/gen/NewRun';
import type { Run } from '../api/gen/Run';

/** Starts a run. At the host's run limit it asks (window.confirm, with the server's message) before stopping the
 * oldest active run; declining resolves to null without an error. */
export function useStartRun(projectId: string, onStarted: (runId: string) => void) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (r: NewRun): Promise<Run | null> => {
      try {
        return await api.startRun(projectId, r);
      } catch (e) {
        if (!(e instanceof ApiError) || e.code !== 'too_many_runs') throw e;
        if (!window.confirm(e.message)) return null;
        const oldest = (await api.listActiveRuns()).find((x) => x.kind === 'run');
        if (!oldest) throw e;
        return api.startRun(projectId, { ...r, stopRunId: oldest.id });
      }
    },
    onSuccess: (run) => {
      void qc.invalidateQueries({ queryKey: ['runs', projectId] });
      void qc.invalidateQueries({ queryKey: ['worktrees', projectId] });
      if (run) onStarted(run.id);
    },
  });
}
```

`web/src/runs/RunsSection.tsx`:

```tsx
import { useQuery } from '@tanstack/react-query';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { RunKind } from '../api/gen/RunKind';
import { runDot, runLabel } from './runState';
import { useStartRun } from './useStartRun';

/** The runs of one place (main checkout or worktree) and a menu to start one from its configuration. */
export function RunsSection({ project, worktreeId, label, selectedRunId, onSelectRun }: {
  project: Project;
  worktreeId: string | null;
  label: string;
  selectedRunId: string | null;
  onSelectRun: (id: string) => void;
}) {
  const runs = useQuery({ queryKey: ['runs', project.id], queryFn: () => api.listRuns(project.id), refetchInterval: 2000 });
  const config = useQuery({
    queryKey: ['run-config', project.id, worktreeId],
    queryFn: () => api.runConfig(project.id, worktreeId),
  });
  const start = useStartRun(project.id, onSelectRun);
  const mine = (runs.data ?? []).filter((r) => r.worktreeId === worktreeId);
  const eff = config.data?.effective;
  const choices = eff ? [...eff.runs.map((d) => `run:${d.name}`), ...eff.tests.map((d) => `test:${d.name}`)] : [];

  return (
    <div className="flex flex-col gap-0.5">
      {config.data?.error && <p className="px-3 text-[11px] text-failed">{config.data.error}</p>}
      {choices.length > 0 && (
        <select
          aria-label={`Run in ${label}`}
          value=""
          disabled={start.isPending}
          onChange={(e) => {
            const [kind, ...name] = e.target.value.split(':');
            start.mutate({ ...(worktreeId ? { worktreeId } : {}), name: name.join(':'), kind: kind as RunKind });
          }}
          className="mx-3 rounded-pdt bg-surface border border-border text-xs text-muted px-1 py-0.5"
        >
          <option value="">Run…</option>
          {choices.map((c) => (
            <option key={c} value={c}>{c.startsWith('test:') ? `Test ${c.slice(5)}` : c.slice(4)}</option>
          ))}
        </select>
      )}
      <ul className="flex flex-col gap-0.5">
        {mine.map((r) => (
          <li key={r.id}>
            <button
              type="button"
              aria-current={r.id === selectedRunId ? 'true' : undefined}
              onClick={() => onSelectRun(r.id)}
              className={`w-full flex items-center gap-2 text-left px-3 py-1 rounded-pdt text-sm ${r.id === selectedRunId ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
            >
              <StatusDot status={runDot(r.state)} />
              <span className="truncate">{r.kind === 'test' ? 'Test ' : ''}{r.name} · {runLabel(r.state)}</span>
            </button>
          </li>
        ))}
      </ul>
      {start.isError && <p role="alert" className="px-3 text-xs text-failed">{start.error.message}</p>}
    </div>
  );
}
```

`web/src/runs/RunBranchForm.tsx`:

```tsx
import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { RunKind } from '../api/gen/RunKind';
import { useStartRun } from './useStartRun';

/** Runs any branch, local or on origin; the server creates its worktree when it has none. */
export function RunBranchForm({ project, onSelectRun }: { project: Project; onSelectRun: (id: string) => void }) {
  const branches = useQuery({ queryKey: ['branches', project.id], queryFn: () => api.listBranches(project.id) });
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const [branch, setBranch] = useState('');
  const [what, setWhat] = useState('');
  const start = useStartRun(project.id, onSelectRun);
  const eff = config.data?.effective;
  const choices = eff ? [...eff.runs.map((d) => `run:${d.name}`), ...eff.tests.map((d) => `test:${d.name}`)] : [];
  if (!choices.length) return null;
  const select = 'rounded-pdt bg-surface border border-border text-xs px-1 py-0.5';

  return (
    <form
      className="flex flex-wrap items-center gap-1 px-2 py-1"
      onSubmit={(e) => {
        e.preventDefault();
        const [kind, ...name] = what.split(':');
        start.mutate({ branch, name: name.join(':'), kind: kind as RunKind });
      }}
    >
      <select aria-label="Branch" value={branch} onChange={(e) => setBranch(e.target.value)} className={select}>
        <option value="">Branch…</option>
        {(branches.data ?? []).map((b) => (
          <option key={b.name} value={b.name}>{b.local ? b.name : `${b.name} (origin)`}</option>
        ))}
      </select>
      <select aria-label="What to run" value={what} onChange={(e) => setWhat(e.target.value)} className={select}>
        <option value="">Run…</option>
        {choices.map((c) => <option key={c} value={c}>{c.startsWith('test:') ? `Test ${c.slice(5)}` : c.slice(4)}</option>)}
      </select>
      <Button type="submit" variant="ghost" aria-label="Run branch" disabled={!branch || !what || start.isPending}>Run</Button>
      {start.isError && <p role="alert" className="w-full text-xs text-failed">{start.error.message}</p>}
    </form>
  );
}
```

`web/src/runs/RunView.tsx`:

```tsx
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, StatusDot } from '@ui';
import { api } from '../api/client';
import type { Run } from '../api/gen/Run';
import { TerminalView } from '../terminal/TerminalView';
import { runPtyUrl } from '../terminal/ptySocket';
import { isActive, runDot, runLabel } from './runState';

export function RunView({ run, polledAt, onRemoved }: { run: Run; polledAt?: number; onRemoved: () => void }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['runs', run.projectId] });
  const [step, setStep] = useState<'setup' | 'command'>(run.state === 'setting_up' ? 'setup' : 'command');
  const stop = useMutation({ mutationFn: () => api.stopRun(run.id), onSuccess: refresh });
  const restart = useMutation({ mutationFn: () => api.restartRun(run.id), onSuccess: refresh });
  const reset = useMutation({ mutationFn: () => api.resetRunData(run.id), onSuccess: refresh });
  const remove = useMutation({
    mutationFn: () => api.deleteRun(run.id),
    onSuccess: () => {
      void refresh();
      onRemoved();
    },
  });
  const active = isActive(run.state);
  const busy = stop.isPending || restart.isPending || reset.isPending || remove.isPending;
  const error = stop.error ?? restart.error ?? reset.error ?? remove.error;
  const tab = (s: 'setup' | 'command', label: string) => (
    <button
      type="button"
      role="tab"
      aria-selected={step === s}
      onClick={() => setStep(s)}
      className={`rounded-pdt px-2 py-0.5 ${step === s ? 'bg-accent-soft text-fg' : 'text-muted hover:bg-surface-2'}`}
    >
      {label}
    </button>
  );

  return (
    <div className="flex flex-1 flex-col min-h-0">
      <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5 text-sm">
        <StatusDot status={runDot(run.state)} />
        <span className="font-medium">{run.kind === 'test' ? 'Test ' : ''}{run.name} · {run.branch}</span>
        <span className="text-xs text-muted">{runLabel(run.state)}{run.exitCode !== null ? ` (exit ${run.exitCode})` : ''}</span>
        {run.url && run.state === 'ready' ? (
          <a href={run.url} target="_blank" rel="noreferrer" className="text-xs text-accent underline">{run.url}</a>
        ) : (
          run.url && <span className="text-xs text-muted">{run.url}</span>
        )}
        <span className="flex-1" />
        {active && <Button variant="ghost" disabled={busy} onClick={() => stop.mutate()}>Stop</Button>}
        <Button variant="ghost" disabled={busy} onClick={() => restart.mutate()}>Restart</Button>
        {!active && run.dataDir && (
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => { if (window.confirm('Delete this run’s data folder contents?')) reset.mutate(); }}
          >
            Reset data
          </Button>
        )}
        {!active && <Button variant="ghost" disabled={busy} onClick={() => remove.mutate()}>Remove</Button>}
      </header>
      {run.error && <p role="alert" className="px-3 py-1 text-xs text-failed">{run.error}</p>}
      {error && <p role="alert" className="px-3 py-1 text-xs text-failed">{error.message}</p>}
      <div role="tablist" aria-label="Run output" className="flex gap-1 px-2 py-1 border-b border-border text-xs">
        {tab('setup', 'Setup')}
        {tab('command', 'Output')}
      </div>
      <TerminalView
        key={`${run.id}:${step}`}
        sessionId={`${run.id}:${step}`}
        url={runPtyUrl(run.id, step)}
        sessionStatus={active ? 'running' : 'finished'}
        polledAt={polledAt}
      />
    </div>
  );
}
```

`web/src/runs/ProjectSettingsPanel.tsx`:

```tsx
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { Project } from '../api/gen/Project';

/** Per-project host settings: overrides of the repo's .pdt/project.toml, and the agent credential policy. */
export function ProjectSettingsPanel({ project }: { project: Project }) {
  const qc = useQueryClient();
  const settings = useQuery({ queryKey: ['project-settings', project.id], queryFn: () => api.projectSettings(project.id) });
  const config = useQuery({ queryKey: ['run-config', project.id, null], queryFn: () => api.runConfig(project.id, null) });
  const [overrides, setOverrides] = useState('');
  const [creds, setCreds] = useState<CredentialPolicy | ''>('');
  useEffect(() => {
    if (!settings.data) return;
    setOverrides(settings.data.runOverrides ?? '');
    setCreds(settings.data.agentCredentials ?? '');
  }, [settings.data]);
  const save = useMutation({
    mutationFn: () =>
      api.saveProjectSettings(project.id, { agentCredentials: creds || null, runOverrides: overrides.trim() ? overrides : null }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['project-settings', project.id] });
      void qc.invalidateQueries({ queryKey: ['run-config', project.id] });
    },
  });

  return (
    <div className="flex flex-col gap-3 p-4 text-sm overflow-auto">
      <h2 className="text-base font-semibold">{project.name}: settings on this host</h2>
      <section aria-label="Repository configuration" className="flex flex-col gap-1">
        <h3 className="text-xs uppercase tracking-wider text-muted">{config.data?.filePath ?? '.pdt/project.toml'}</h3>
        <pre className="rounded-pdt bg-surface border border-border p-2 text-xs font-mono whitespace-pre-wrap">
          {config.data?.file ?? 'This repository has no run configuration file.'}
        </pre>
        {config.data?.error && <p className="text-xs text-failed">{config.data.error}</p>}
      </section>
      <label className="flex flex-col gap-1">
        <span className="text-xs uppercase tracking-wider text-muted">Host overrides (TOML)</span>
        <textarea
          aria-label="Host overrides (TOML)"
          value={overrides}
          onChange={(e) => setOverrides(e.target.value)}
          rows={10}
          spellCheck={false}
          className="rounded-pdt bg-surface border border-border p-2 font-mono text-xs text-fg"
        />
      </label>
      <label className="flex items-center gap-2">
        <span>Agent credentials</span>
        <select
          aria-label="Agent credentials"
          value={creds}
          onChange={(e) => setCreds(e.target.value as CredentialPolicy | '')}
          className="rounded-pdt bg-surface border border-border px-1 py-0.5"
        >
          <option value="">Use the host setting ({settings.data?.effectiveCredentials ?? '…'})</option>
          <option value="subscription">Subscription (remove API keys)</option>
          <option value="pass_through">Pass through</option>
        </select>
      </label>
      <div className="flex items-center gap-2">
        <Button variant="primary" disabled={save.isPending} onClick={() => save.mutate()}>Save</Button>
        <span role="status" className="text-xs text-muted">{save.isSuccess ? 'Saved' : ''}</span>
      </div>
      {save.isError && <p role="alert" className="text-xs text-failed">{save.error.message}</p>}
    </div>
  );
}
```

Run: `pnpm --dir web test -- runs`
Expected: PASS.

- [ ] **Step 4: Wire into the session list and the shell**

In `SessionList.tsx`:
- Add the optional props `selectedRunId?: string | null`, `onSelectRun?: (id: string) => void` and `onOpenSettings?: () => void`.
- Inside each group's `<section>`, after the `<ul>` of sessions, add: `{onSelectRun && <RunsSection project={project} worktreeId={g.id} label={g.label} selectedRunId={selectedRunId ?? null} onSelectRun={onSelectRun} />}`.
- After the groups, add: `{onSelectRun && <RunBranchForm project={project} onSelectRun={onSelectRun} />}`.
- If `onOpenSettings` is given, add a ghost button "Project settings" at the top of the list.

In `AppShell.tsx`:
- Replace `sessionId` with a selection:

```tsx
type Selection = { kind: 'session'; id: string } | { kind: 'run'; id: string } | { kind: 'settings' } | null;
const [selection, setSelection] = useState<Selection>(null);
```

- Derive `sessionId = selection?.kind === 'session' ? selection.id : null` and `runId = selection?.kind === 'run' ? selection.id : null`.
- Add a runs query `useQuery({ queryKey: ['runs', projectId], queryFn: () => api.listRuns(projectId!), enabled: projectId !== null, refetchInterval: 2000 })`. Find `run` by id.
- Pass `selectedRunId={runId}`, `onSelectRun={(id) => setSelection({ kind: 'run', id })}` and `onOpenSettings={() => setSelection({ kind: 'settings' })}` to `SessionList`. Make `onSelect` set `{ kind: 'session', id }`.
- In the main area, render the first that applies:
  - `RunView` for a run, with `onRemoved={() => setSelection(null)}`;
  - `ProjectSettingsPanel` for settings;
  - the existing `SessionView`.
- `selectProject` resets the selection to `null`.

Run: `pnpm --dir web typecheck; pnpm --dir web test`
Expected: PASS. If the existing `AppShell` and sessions tests stubbed only the old routes, add `GET /api/projects/<id>/runs: []`, `GET /api/projects/<id>/run-config: {filePath:'', file:null, overrides:null, effective:{runs:[],tests:[]}, error:null}` and `GET /api/projects/<id>/branches: []` to their stubs.

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat(web): runs: start from the configuration or any branch, logs, link, stop/restart/reset"
```

---

### Task 9: Agent activity and unseen (backend)

**Files:**
- Create:
  - `crates/pdt-core/src/activity.rs`
  - `crates/pdt-store/migrations/0006_session_activity.sql`
  - `crates/pdt-agents/src/hooks.rs`
  - `crates/pdt-agents/tests/codex_screen.rs`
  - `crates/pdt-api/src/activity.rs`
  - `crates/pdt-api/tests/activity.rs`
  - `crates/pdtd/src/hook.rs`
  - `crates/pdtd/tests/common/mod.rs`, `crates/pdtd/tests/hooks.rs`
- Modify:
  - `crates/pdt-core/src/{lib,session}.rs`
  - `crates/pdt-store/src/sessions.rs`
  - `crates/pdt-agents/src/lib.rs`, `crates/pdt-agents/Cargo.toml`
  - `crates/pdt-api/src/{routes,sessions,pty_ws,lib}.rs`
  - `crates/pdtd/src/{main,config}.rs`, `crates/pdtd/tests/lifecycle.rs`
  - `web/src/sessions/sessions.test.tsx` (fixture fields)

**Interfaces:**
- Consumes: `AppState.listen_port` (Task 4) and `AppState.data_dir` (Task 2). Task 1's fixture and marker list.
- Produces (`pdt_core::activity`):
  - Types:
    - `Activity { Working, NeedsYou, Idle, Unknown }`, with `as_str()` and `parse()`.
    - `HookReport`, tagged by `source`: `{source: "claude", event, notificationType?}` or `{source: "codex", event}`.
    - `Signal { Busy, NeedsYou, TurnEnded, Quiet, HookMissing }`.
    - `Watch { last_output_ms, enter_at_ms, hook_since_enter, output_since_enter }`.
  - Functions:
    - `HookReport::signal()`;
    - `next(Activity, Signal) -> Activity`;
    - `needs_attention(prev, next) -> bool`;
    - `is_unseen(attention_at, last_viewed_at) -> bool`;
    - `tick(Activity, &Watch, expects_hooks, now) -> Option<Signal>`;
    - `strip_ansi(&[u8]) -> String`, `squash(&str) -> String` and `screen_shows(&str, &[&str]) -> bool`.
  - Constants: `CODEX_APPROVAL_MARKERS`, `QUIET_AFTER_MS = 120_000`, `HOOK_GRACE_MS = 20_000`.
- Produces (`Session`): new fields `activity: Option<Activity>`, `attentionAt`, `lastViewedAt` and `unseen: bool`.
- Produces (`pdt_store`): `update_activity(id, Option<Activity>, attention_at: Option<i64>)` and `mark_viewed(id, at)`.
- Produces (`pdt_agents::hooks`): `claude_settings(pdtd) -> Value`, `write_claude_settings(dir, pdtd)`, `codex_notify_args(pdtd, chain)`, `user_codex_notify(codex_home, pdtd)` and `toml_string`.
- Produces (`pdt_api`):
  - Routes `POST /api/sessions/{id}/activity` (body `HookReport` → 204; 409 `not_an_agent`) and `POST /api/sessions/{id}/view` (→ 204).
  - `pub async fn run_activity_ticker(AppState)`.
  - `AppState.hook_program: PathBuf`.
- Produces (`pdtd`):
  - The `pdtd hook claude <Event>` and `pdtd hook codex-notify [--chain <cmd…> --] <json>` subcommands.
  - `PDT_CLAUDE_PROGRAM` and `PDT_CODEX_PROGRAM` override the agent programs; they are used by tests, and by users with unusual installs.

- [ ] **Step 1: Write the failing `pdt-core` tests**

Add `pub mod activity;` to `pdt-core/src/lib.rs`. Create `crates/pdt-core/src/activity.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use Activity::*;

    fn claude(event: &str, nt: Option<&str>) -> HookReport {
        HookReport::Claude { event: event.into(), notification_type: nt.map(Into::into) }
    }

    #[test]
    fn wire_format() {
        let r: HookReport =
            serde_json::from_str(r#"{"source":"claude","event":"Notification","notificationType":"permission_prompt"}"#)
                .unwrap();
        assert_eq!(r, claude("Notification", Some("permission_prompt")));
        let r: HookReport = serde_json::from_str(r#"{"source":"codex","event":"agent-turn-complete"}"#).unwrap();
        assert_eq!(r, HookReport::Codex { event: "agent-turn-complete".into() });
        assert_eq!(serde_json::to_string(&NeedsYou).unwrap(), r#""needs_you""#);
        for a in [Working, NeedsYou, Idle, Unknown] {
            assert_eq!(Activity::parse(a.as_str()), Some(a));
        }
    }

    #[test]
    fn claude_events_map_to_signals() {
        assert_eq!(claude("UserPromptSubmit", None).signal(), Some(Signal::Busy));
        assert_eq!(claude("PreToolUse", None).signal(), Some(Signal::Busy));
        assert_eq!(claude("PermissionRequest", None).signal(), Some(Signal::NeedsYou));
        for nt in ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"] {
            assert_eq!(claude("Notification", Some(nt)).signal(), Some(Signal::NeedsYou), "{nt}");
        }
        assert_eq!(claude("Notification", Some("idle_prompt")).signal(), Some(Signal::TurnEnded));
        assert_eq!(claude("Notification", Some("auth_success")).signal(), None);
        assert_eq!(claude("Notification", None).signal(), None);
        assert_eq!(claude("Stop", None).signal(), Some(Signal::TurnEnded));
        assert_eq!(claude("SessionStart", None).signal(), None);
        assert_eq!(HookReport::Codex { event: "agent-turn-complete".into() }.signal(), Some(Signal::TurnEnded));
        assert_eq!(HookReport::Codex { event: "something-new".into() }.signal(), None);
    }

    #[test]
    fn transitions() {
        assert_eq!(next(Idle, Signal::Busy), Working);
        assert_eq!(next(Working, Signal::NeedsYou), NeedsYou);
        assert_eq!(next(NeedsYou, Signal::Busy), Working);
        assert_eq!(next(Working, Signal::TurnEnded), Idle);
        assert_eq!(next(Working, Signal::Quiet), Unknown);
        assert_eq!(next(Idle, Signal::Quiet), Idle, "quiet only matters while working");
        assert_eq!(next(Idle, Signal::HookMissing), Unknown, "hooks broke: say so");
        assert_eq!(next(Unknown, Signal::TurnEnded), Idle);
    }

    #[test]
    fn attention_and_unseen() {
        assert!(needs_attention(Working, Idle));
        assert!(needs_attention(Unknown, Idle));
        assert!(needs_attention(Idle, NeedsYou));
        assert!(!needs_attention(Idle, Idle));
        assert!(!needs_attention(Idle, Working));
        assert!(!needs_attention(NeedsYou, NeedsYou));
        assert!(!is_unseen(None, None));
        assert!(is_unseen(Some(10), None));
        assert!(is_unseen(Some(10), Some(5)));
        assert!(!is_unseen(Some(10), Some(10)), "viewed at the same moment counts as seen");
        assert!(!is_unseen(Some(10), Some(11)));
    }

    #[test]
    fn ticks() {
        let w = |last_output_ms, enter_at_ms, hook_since_enter, output_since_enter| Watch {
            last_output_ms,
            enter_at_ms,
            hook_since_enter,
            output_since_enter,
        };
        assert_eq!(tick(Working, &w(0, None, false, false), true, QUIET_AFTER_MS), Some(Signal::Quiet));
        assert_eq!(tick(Working, &w(1, None, false, false), true, QUIET_AFTER_MS), None);
        assert_eq!(tick(Idle, &w(0, None, false, false), true, QUIET_AFTER_MS * 10), None);
        let missing = w(HOOK_GRACE_MS, Some(0), false, true);
        assert_eq!(tick(Idle, &missing, true, HOOK_GRACE_MS), Some(Signal::HookMissing));
        assert_eq!(tick(Idle, &missing, false, HOOK_GRACE_MS), None, "Codex has no per-prompt hook");
        assert_eq!(tick(Idle, &w(HOOK_GRACE_MS, Some(0), true, true), true, HOOK_GRACE_MS), None, "a hook came");
        assert_eq!(tick(Idle, &w(0, Some(0), false, false), true, HOOK_GRACE_MS), None, "no output: nothing happened");
    }

    #[test]
    fn screen_text() {
        let raw = b"\x1b[2J\x1b[1;1H\x1b[1mWould\x1b[0m you\x1b[3C like to \x1b]0;title\x07run the following command?\r\n";
        let text = strip_ansi(raw);
        assert!(!text.contains('\x1b'));
        assert!(screen_shows(&squash(&text), &["Would you like to run the following command?"]));
        assert!(!screen_shows(&squash("nothing here"), CODEX_APPROVAL_MARKERS));
        assert_eq!(squash(" A b\tC\n"), "abc");
    }
}
```

Run: `cargo test -p pdt-core activity`
Expected: compile errors.

- [ ] **Step 2: Implement, above the tests**

```rust
//! Agent activity (remote-host spec §5): what an agent terminal is doing, and whether the user has seen it since it
//! last needed them. Pure; pdt-api feeds it hook reports and terminal input/output.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Activity {
    Working,
    NeedsYou,
    Idle,
    Unknown,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Working => "working",
            Activity::NeedsYou => "needs_you",
            Activity::Idle => "idle",
            Activity::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Activity::Working, Activity::NeedsYou, Activity::Idle, Activity::Unknown].into_iter().find(|a| a.as_str() == s)
    }
}

/// What `pdtd hook` posts to `/api/sessions/{id}/activity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "source", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum HookReport {
    Claude {
        event: String,
        #[serde(default)]
        #[ts(optional)]
        notification_type: Option<String>,
    },
    Codex {
        event: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Busy,
    NeedsYou,
    TurnEnded,
    /// No output for `QUIET_AFTER_MS` while working.
    Quiet,
    /// Claude took input and printed, but no hook arrived within `HOOK_GRACE_MS`: detection is broken.
    HookMissing,
}

const NEEDS_YOU_NOTIFICATIONS: [&str; 4] =
    ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"];

impl HookReport {
    pub fn signal(&self) -> Option<Signal> {
        match self {
            HookReport::Claude { event, notification_type } => match event.as_str() {
                "UserPromptSubmit" | "PreToolUse" => Some(Signal::Busy),
                "PermissionRequest" => Some(Signal::NeedsYou),
                "Stop" => Some(Signal::TurnEnded),
                "Notification" => match notification_type.as_deref() {
                    Some(t) if NEEDS_YOU_NOTIFICATIONS.contains(&t) => Some(Signal::NeedsYou),
                    Some("idle_prompt") => Some(Signal::TurnEnded),
                    _ => None,
                },
                _ => None,
            },
            HookReport::Codex { event } => (event == "agent-turn-complete").then_some(Signal::TurnEnded),
        }
    }
}

pub fn next(current: Activity, s: Signal) -> Activity {
    match s {
        Signal::Busy => Activity::Working,
        Signal::NeedsYou => Activity::NeedsYou,
        Signal::TurnEnded => Activity::Idle,
        Signal::Quiet if current == Activity::Working => Activity::Unknown,
        Signal::Quiet => current,
        Signal::HookMissing => Activity::Unknown,
    }
}

/// Becoming idle or needing the user (from anything else) asks for attention.
pub fn needs_attention(prev: Activity, next: Activity) -> bool {
    prev != next && matches!(next, Activity::Idle | Activity::NeedsYou)
}

pub fn is_unseen(attention_at: Option<i64>, last_viewed_at: Option<i64>) -> bool {
    attention_at.is_some_and(|a| last_viewed_at.is_none_or(|v| a > v))
}

pub const QUIET_AFTER_MS: i64 = 120_000;
pub const HOOK_GRACE_MS: i64 = 20_000;

/// Per-session timing facts the heuristics need.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Watch {
    pub last_output_ms: i64,
    /// When the user last pressed Enter (Claude only), until a hook arrives.
    pub enter_at_ms: Option<i64>,
    pub hook_since_enter: bool,
    pub output_since_enter: bool,
}

/// The time-based signal due now, if any. `expects_hooks` is true for Claude.
pub fn tick(activity: Activity, w: &Watch, expects_hooks: bool, now: i64) -> Option<Signal> {
    if expects_hooks
        && let Some(at) = w.enter_at_ms
        && !w.hook_since_enter
        && w.output_since_enter
        && now - at >= HOOK_GRACE_MS
    {
        return Some(Signal::HookMissing);
    }
    (activity == Activity::Working && now - w.last_output_ms >= QUIET_AFTER_MS).then_some(Signal::Quiet)
}

/// Lines of Codex's approval prompt, as verified in Task 1 (docs/superpowers/notes/phase-2-agent-signals.md).
/// Compared whitespace- and case-insensitively, because the TUI redraws with cursor moves instead of spaces.
pub const CODEX_APPROVAL_MARKERS: &[&str] = &[
    "Would you like to run the following command?",
    "Would you like to make the following edits?",
    "Yes, proceed",
];

/// Terminal output without escape sequences (CSI, OSC, two-byte ESC) or control characters other than newlines.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        i += 1;
                        while i < bytes.len() && bytes[i] != 0x07 && !(bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\')) {
                            i += 1;
                        }
                        i += if bytes.get(i) == Some(&0x1b) { 2 } else { 1 };
                    }
                    Some(_) => i += 1,
                    None => {}
                }
            }
            b'\n' => {
                out.push(b'\n');
                i += 1;
            }
            c if c < 0x20 || c == 0x7f => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lower-case, without whitespace.
pub fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Whether squashed screen text contains any marker.
pub fn screen_shows(squashed: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| squashed.contains(&squash(m)))
}
```

**Before running the tests:** replace the entries of `CODEX_APPROVAL_MARKERS` with the marker list recorded in `docs/superpowers/notes/phase-2-agent-signals.md` (Task 1). Keep a line that the unit test `screen_text` uses only if Task 1 confirmed it; otherwise change the test's marker literal to one of the confirmed lines.

Run: `cargo test -p pdt-core activity`
Expected: PASS.

- [ ] **Step 3: The real-capture test**

`crates/pdt-agents/tests/codex_screen.rs`:

```rust
use pdt_core::activity::{CODEX_APPROVAL_MARKERS, screen_shows, squash, strip_ansi};

#[test]
fn approval_screen_is_recognised_in_a_real_capture() {
    let raw = include_bytes!("fixtures/codex-approval.txt");
    assert!(screen_shows(&squash(&strip_ansi(raw)), CODEX_APPROVAL_MARKERS), "markers missing from the Task 1 capture");
}
```

Run: `cargo test -p pdt-agents --test codex_screen`
Expected: PASS. If it fails, fix the markers rather than the test.

- [ ] **Step 4: Store the activity (TDD)**

`crates/pdt-store/migrations/0006_session_activity.sql`:

```sql
-- Agent activity (remote-host spec §5): NULL for shells and ended sessions.
ALTER TABLE sessions ADD COLUMN activity TEXT;
-- When the session last became idle or needs_you (ms); unseen while it is later than last_viewed_at.
ALTER TABLE sessions ADD COLUMN attention_at INTEGER;
ALTER TABLE sessions ADD COLUMN last_viewed_at INTEGER;
```

In `pdt-core/src/session.rs`, add to `Session`:

```rust
    /// Agents only; None for shells and ended sessions.
    pub activity: Option<crate::activity::Activity>,
    #[ts(type = "number | null")]
    pub attention_at: Option<i64>,
    #[ts(type = "number | null")]
    pub last_viewed_at: Option<i64>,
    /// Computed: became idle or needs_you after the user last viewed it (on any device).
    pub unseen: bool,
```

Add these tests to `pdt-store/src/sessions.rs` (the `session()` helper gets `activity: None, attention_at: None, last_viewed_at: None, unseen: false`):

```rust
    #[tokio::test]
    async fn activity_attention_and_viewing() {
        use pdt_core::activity::Activity;
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.activity, s.unseen), (Some(Activity::Working), false));
        store.update_activity("a", Some(Activity::Idle), Some(50)).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.activity, s.attention_at, s.unseen), (Some(Activity::Idle), Some(50), true));
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().attention_at, Some(50), "kept until the next attention");
        assert!(store.mark_viewed("a", 60).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.last_viewed_at, s.unseen), (Some(60), false));
        store.update_activity("a", None, None).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().activity, None);
        assert!(!store.mark_viewed("zz", 1).await.unwrap());
    }
```

Run: `cargo test -p pdt-store`
Expected: compile errors. Then implement:
- Add `activity, attention_at, last_viewed_at` to `COLUMNS`.
- In `to_session`, parse `activity` with `Activity::parse`, returning `Corrupt` on unknown values. Read the two timestamps, and set `unseen: pdt_core::activity::is_unseen(attention_at, last_viewed_at)`.
- In `insert_session`, add the three columns.
- Add:

```rust
    /// Sets the activity; `attention_at` (when Some) records that it now needs the user's attention.
    pub async fn update_activity(
        &self,
        id: &str,
        activity: Option<pdt_core::activity::Activity>,
        attention_at: Option<i64>,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET activity = ?, attention_at = COALESCE(?, attention_at) WHERE id = ?")
            .bind(activity.map(pdt_core::activity::Activity::as_str))
            .bind(attention_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn mark_viewed(&self, id: &str, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET last_viewed_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
```

In `pdt-api/src/sessions.rs` `create`, initialise the new fields: `activity: None, attention_at: None, last_viewed_at: None, unseen: false`.

Run: `cargo test -p pdt-store; cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 5: Hook settings and the Codex notify argument (TDD)**

Run (PowerShell): `cargo add toml -p pdt-agents`. Add `pub mod hooks;` to `pdt-agents/src/lib.rs`.

`crates/pdt-agents/src/hooks.rs`:

```rust
//! Per-session agent hook configuration (remote-host spec §5.1, decisions 11–12). The user's own settings are never
//! touched: Claude gets `--settings <file>`, Codex gets `-c notify=[…]`.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const CLAUDE_HOOK_EVENTS: [&str; 5] = ["UserPromptSubmit", "PreToolUse", "PermissionRequest", "Notification", "Stop"];
pub const CLAUDE_SETTINGS_FILE: &str = "claude-settings.json";

/// Exec-form command hooks (no shell, verified fact 2) running `<pdtd> hook claude <Event>`.
pub fn claude_settings(pdtd: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for ev in CLAUDE_HOOK_EVENTS {
        hooks.insert(
            ev.into(),
            json!([{ "hooks": [{
                "type": "command",
                "command": pdtd.to_string_lossy(),
                "args": ["hook", "claude", ev],
                "timeout": 10
            }] }]),
        );
    }
    json!({ "hooks": hooks })
}

/// Writes `<dir>/claude-settings.json` and returns its path.
pub fn write_claude_settings(dir: &Path, pdtd: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(CLAUDE_SETTINGS_FILE);
    std::fs::write(&path, serde_json::to_vec_pretty(&claude_settings(pdtd)).expect("json"))?;
    Ok(path)
}

/// A TOML basic string.
pub fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `-c notify=[<pdtd>, "hook", "codex-notify", ("--chain", <user's notify…>, "--")]`; Codex appends its JSON.
pub fn codex_notify_args(pdtd: &Path, chain: &[String]) -> Vec<String> {
    let mut items: Vec<String> = vec![pdtd.to_string_lossy().into_owned(), "hook".into(), "codex-notify".into()];
    if !chain.is_empty() {
        items.push("--chain".into());
        items.extend(chain.iter().cloned());
        items.push("--".into());
    }
    let list = items.iter().map(|i| toml_string(i)).collect::<Vec<_>>().join(",");
    vec!["-c".into(), format!("notify=[{list}]")]
}

/// The user's own top-level `notify` from `<codex_home>/config.toml`, to chain to (never our own hook).
pub fn user_codex_notify(codex_home: &Path, pdtd: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let items: Vec<String> =
        table.get("notify")?.as_array()?.iter().map(|v| v.as_str().map(String::from)).collect::<Option<_>>()?;
    let ours = items.len() > 1 && items[1] == "hook" && Path::new(&items[0]) == pdtd;
    (!items.is_empty() && !ours).then_some(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_settings_use_exec_form_for_every_event() {
        let exe = Path::new(if cfg!(windows) { r"C:\Program Files\P D T\pdtd.exe" } else { "/Applications/P D T.app/Contents/MacOS/pdtd" });
        let v = claude_settings(exe);
        for ev in CLAUDE_HOOK_EVENTS {
            let h = &v["hooks"][ev][0]["hooks"][0];
            assert_eq!(h["type"], "command");
            assert_eq!(h["command"], exe.to_string_lossy().as_ref());
            assert_eq!(h["args"], json!(["hook", "claude", ev]));
            assert!(v["hooks"][ev][0].get("matcher").is_none(), "no matcher: every tool and notification");
        }
        let dir = tempfile::tempdir().unwrap();
        let p = write_claude_settings(&dir.path().join("s1"), exe).unwrap();
        let back: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn notify_argument_is_valid_toml_with_windows_paths() {
        let args = codex_notify_args(Path::new(r#"C:\Users\o"neil\pdtd.exe"#), &[]);
        assert_eq!(args[0], "-c");
        let parsed: toml::Table = toml::from_str(&args[1]).unwrap();
        let list: Vec<&str> = parsed["notify"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(list, [r#"C:\Users\o"neil\pdtd.exe"#, "hook", "codex-notify"]);
        let chained = codex_notify_args(Path::new("/x/pdtd"), &["notifier".into(), "turn-ended".into()]);
        let parsed: toml::Table = toml::from_str(&chained[1]).unwrap();
        let list: Vec<&str> = parsed["notify"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(list, ["/x/pdtd", "hook", "codex-notify", "--chain", "notifier", "turn-ended", "--"]);
    }

    #[test]
    fn reads_the_users_notify_but_never_our_own() {
        let dir = tempfile::tempdir().unwrap();
        let pdtd = Path::new("/x/pdtd");
        assert_eq!(user_codex_notify(dir.path(), pdtd), None, "no config");
        std::fs::write(dir.path().join("config.toml"), "model = \"o3\"\nnotify = [\"C:\\\\n.exe\", \"turn-ended\"]\n[tui]\n")
            .unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), Some(vec![r"C:\n.exe".to_string(), "turn-ended".into()]));
        std::fs::write(dir.path().join("config.toml"), "notify = [\"/x/pdtd\", \"hook\", \"codex-notify\"]\n").unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), None);
        std::fs::write(dir.path().join("config.toml"), "notify = \"oops\"\n").unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), None);
    }
}
```

Run: `cargo test -p pdt-agents hooks`
Expected: PASS.

- [ ] **Step 6: Write the failing API tests**

`crates/pdt-api/tests/activity.rs`. Copy the imports and the `call` helper from `tests/host.rs`, then add:

```rust
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

async fn setup(tmp: &std::path::Path) -> (axum::Router, pdt_api::AppState) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(
        std::process::Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success()
    );
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let state = pdt_api::AppState::new(store.clone(), tmp.join("data"));
    let app = pdt_api::router(state.clone(), None);
    let (_, p) = call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    for (id, kind, agent) in [("c1", SessionKind::AgentTui, Some(AgentKind::Claude)), ("sh", SessionKind::Shell, None)] {
        store
            .insert_session(&Session {
                id: id.into(),
                project_id: p["id"].as_str().unwrap().into(),
                worktree_id: None,
                kind,
                agent,
                title: "t".into(),
                cwd: repo.to_string_lossy().into(),
                status: SessionStatus::Running,
                exit_code: None,
                created_at: 1,
                ended_at: None,
                external_session_id: None,
                fresh_started_at: None,
                activity: agent.map(|_| pdt_core::activity::Activity::Idle),
                attention_at: None,
                last_viewed_at: None,
                unseen: false,
            })
            .await
            .unwrap();
    }
    (app, state)
}

async fn report(app: &axum::Router, id: &str, body: Value) -> StatusCode {
    call(app, "POST", &format!("/api/sessions/{id}/activity"), Some(body)).await.0
}

async fn session(app: &axum::Router, id: &str) -> Value {
    call(app, "GET", &format!("/api/sessions/{id}"), None).await.1
}

#[tokio::test]
async fn hook_reports_drive_activity_and_unseen() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    assert_eq!(report(&app, "c1", json!({"source": "claude", "event": "UserPromptSubmit"})).await, StatusCode::NO_CONTENT);
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("working"), Some(false)));
    report(&app, "c1", json!({"source": "claude", "event": "Stop"})).await;
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("idle"), Some(true)));
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert_eq!(call(&app, "POST", "/api/sessions/c1/view", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(session(&app, "c1").await["unseen"], false, "seen on every device");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    report(&app, "c1", json!({"source": "claude", "event": "Notification", "notificationType": "permission_prompt"})).await;
    let s = session(&app, "c1").await;
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("needs_you"), Some(true)));
    report(&app, "c1", json!({"source": "claude", "event": "Notification", "notificationType": "auth_success"})).await;
    assert_eq!(session(&app, "c1").await["activity"], "needs_you", "ignored notification types change nothing");
    assert_eq!(report(&app, "c1", json!({"source": "claude", "event": "FutureEvent"})).await, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn reports_need_an_agent_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _) = setup(tmp.path()).await;
    let (s, v) = call(&app, "POST", "/api/sessions/sh/activity", Some(json!({"source": "codex", "event": "x"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_an_agent")));
    assert_eq!(report(&app, "nope", json!({"source": "codex", "event": "x"})).await, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "POST", "/api/sessions/nope/view", None).await.0, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/api/sessions/c1/activity", Some(json!({"source": "nobody"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}
```

Run: `cargo test -p pdt-api --test activity`
Expected: FAIL (404).

- [ ] **Step 7: Implement the API side**

Add to `AppState`:

```rust
    /// The program agent hooks run (`<this pdtd> hook …`).
    pub hook_program: std::path::PathBuf,
    pub(crate) activity: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, crate::activity::Tracker>>>,
```

Initialise them with `hook_program: std::env::current_exe().unwrap_or_default()` and `activity: Default::default()`.

`crates/pdt-api/src/activity.rs`:

```rust
//! Agent activity (remote-host spec §5): hook reports from `pdtd hook`, terminal input and output heuristics, and
//! the unseen flag.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::PtyEvent;
use pdt_core::activity::{
    Activity, CODEX_APPROVAL_MARKERS, HookReport, Signal, Watch, needs_attention, next, screen_shows, squash,
    strip_ansi, tick,
};
use pdt_core::session::{AgentKind, Session, SessionKind};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast::error::RecvError;

const SCREEN_TAIL: usize = 16 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Tracker {
    agent: AgentKind,
    activity: Activity,
    watch: Watch,
    /// Squashed recent output (Codex), searched for the approval prompt.
    screen: String,
    generation: u64,
}

/// Applies a signal to a session: from its tracker when it has one (running in this daemon), else from the stored row.
pub(crate) async fn apply(st: &AppState, id: &str, sig: Signal) -> Result<(), ApiError> {
    let tracked = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        map.get_mut(id).map(|t| {
            let prev = t.activity;
            t.activity = next(prev, sig);
            (prev, t.activity)
        })
    };
    let (prev, now) = match tracked {
        Some(p) => p,
        None => {
            let Some(s) = st.store.get_session(id).await? else { return Ok(()) };
            let prev = s.activity.unwrap_or(Activity::Idle);
            (prev, next(prev, sig))
        }
    };
    if prev != now {
        st.store.update_activity(id, Some(now), needs_attention(prev, now).then(now_ms)).await?;
    }
    Ok(())
}

/// Starts tracking an agent session whose terminal just started: idle, watching its output until it exits.
pub(crate) async fn track(st: &AppState, s: &Session) {
    let Some(agent) = s.agent.filter(|_| s.kind == SessionKind::AgentTui) else { return };
    let Ok(att) = st.ptys.attach(&s.id) else { return };
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
    st.activity.lock().unwrap_or_else(|e| e.into_inner()).insert(
        s.id.clone(),
        Tracker {
            agent,
            activity: Activity::Idle,
            watch: Watch { last_output_ms: now_ms(), ..Default::default() },
            screen: String::new(),
            generation,
        },
    );
    let _ = st.store.update_activity(&s.id, Some(Activity::Idle), None).await;
    let (st, id) = (st.clone(), s.id.clone());
    tokio::spawn(async move {
        let mut events = att.events;
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(bytes)) => on_output(&st, &id, &bytes).await,
                Ok(PtyEvent::Exit(_)) | Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            }
        }
        let mine = {
            let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
            let mine = map.get(&id).is_some_and(|t| t.generation == generation);
            if mine {
                map.remove(&id);
            }
            mine
        };
        if mine {
            let _ = st.store.update_activity(&id, None, None).await;
        }
    });
}

async fn on_output(st: &AppState, id: &str, bytes: &[u8]) {
    let sig = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        t.watch.last_output_ms = now_ms();
        if t.watch.enter_at_ms.is_some() {
            t.watch.output_since_enter = true;
        }
        if t.agent != AgentKind::Codex || t.activity == Activity::NeedsYou {
            return;
        }
        t.screen.push_str(&squash(&strip_ansi(bytes)));
        if t.screen.len() > SCREEN_TAIL {
            let mut cut = t.screen.len() - SCREEN_TAIL;
            while !t.screen.is_char_boundary(cut) {
                cut += 1;
            }
            t.screen.drain(..cut);
        }
        if !screen_shows(&t.screen, CODEX_APPROVAL_MARKERS) {
            return;
        }
        t.screen.clear();
        Signal::NeedsYou
    };
    let _ = apply(st, id, sig).await;
}

/// Called with every chunk the user types into a terminal (no-op for terminals without a tracker).
pub(crate) fn on_input(st: &AppState, id: &str, bytes: &[u8]) {
    let enter = bytes.contains(&b'\r');
    let sig = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        match t.agent {
            AgentKind::Codex if enter || t.activity == Activity::NeedsYou => {
                t.screen.clear();
                Some(Signal::Busy)
            }
            AgentKind::Claude if enter => {
                t.watch.enter_at_ms = Some(now_ms());
                t.watch.hook_since_enter = false;
                t.watch.output_since_enter = false;
                None
            }
            _ => None,
        }
    };
    if let Some(sig) = sig {
        let (st, id) = (st.clone(), id.to_string());
        tokio::spawn(async move {
            let _ = apply(&st, &id, sig).await;
        });
    }
}

/// Applies time-based signals (`tick`) to every tracked session every 5 s, until shutdown.
pub async fn run_activity_ticker(st: AppState) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        every.tick().await;
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let due: Vec<(String, Signal)> = {
            let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            map.iter_mut()
                .filter_map(|(id, t)| {
                    let sig = tick(t.activity, &t.watch, t.agent == AgentKind::Claude, now)?;
                    if sig == Signal::HookMissing {
                        t.watch.enter_at_ms = None;
                    }
                    Some((id.clone(), sig))
                })
                .collect()
        };
        for (id, sig) in due {
            let _ = apply(&st, &id, sig).await;
        }
    }
}

pub(crate) async fn post_report(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<HookReport>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(report) = payload?;
    let s = st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    if s.kind != SessionKind::AgentTui {
        return Err(ApiError::new(StatusCode::CONFLICT, "not_an_agent", "only agent sessions report activity"));
    }
    if let Some(t) = st.activity.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
        t.watch.hook_since_enter = true;
        t.watch.enter_at_ms = None;
    }
    if let Some(sig) = report.signal() {
        apply(&st, &id, sig).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn post_view(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.mark_viewed(&id, now_ms()).await? {
        return Err(ApiError::not_found("session"));
    }
    Ok(StatusCode::NO_CONTENT)
}
```

Wire it:
- **Routes:** `.route("/sessions/{id}/activity", axum::routing::post(crate::activity::post_report))` and `.route("/sessions/{id}/view", axum::routing::post(crate::activity::post_view))`. Add `mod activity;` and `pub use activity::run_activity_ticker;` in `lib.rs`.
- **`pty_ws.rs`:**
  - Change `pump(socket, ptys, id, att)` to `pump(socket, st: AppState, id, att)`, and use `st.ptys` inside it.
  - In the `Message::Binary` input branch, call `crate::activity::on_input(&st, &id, &bytes);` before `st.ptys.write`.
  - Update both callers: `upgrade` and `wait_for_resume`.
- **`sessions.rs` `start_pty`:**
  1. Build the launch arguments **before** `resolve`:

     ```rust
     let mut launch_args = l.args.clone();
     if s.kind == SessionKind::AgentTui {
         match s.agent {
             Some(AgentKind::Claude) => {
                 let dir = st.data_dir.join("sessions").join(&s.id);
                 let file = pdt_agents::hooks::write_claude_settings(&dir, &st.hook_program)
                     .map_err(|e| PtyError::Spawn(format!("writing the session's hook settings: {e}")))?;
                 launch_args.extend(["--settings".to_string(), file.to_string_lossy().into_owned()]);
             }
             Some(AgentKind::Codex) => {
                 let chain = st
                     .platform
                     .codex_home
                     .as_deref()
                     .and_then(|h| pdt_agents::hooks::user_codex_notify(h, &st.hook_program))
                     .unwrap_or_default();
                 let mut a = pdt_agents::hooks::codex_notify_args(&st.hook_program, &chain);
                 a.extend(launch_args);
                 launch_args = a;
             }
             None => {}
         }
     }
     let (program, args) = pdt_agents::launch::resolve(&l.program, &launch_args, login_path)?;
     ```

  2. After the scrubbed `env` is built, for agent sessions add `("PDT_SESSION_ID", s.id)` and `("PDT_HOOK_PORT", st.listen_port.to_string())`. These are set after scrubbing, so inherited values never leak.
  3. `start_pty` is synchronous and `track` is async. In both callers (`create` and `restart_locked`), after a successful `start_pty`, call `crate::activity::track(&st, &session).await;`.
- **Removing a session:** `remove` also deletes `st.data_dir.join("sessions").join(&id)`, best-effort.

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 8: The `pdtd hook` subcommand (TDD)**

Move the helpers (`Daemon`, `free_port`, `start`, `http` and `wait_exit`) out of `crates/pdtd/tests/lifecycle.rs` into `crates/pdtd/tests/common/mod.rs`. Change `start` to `start_with(port, data, extra_env: &[(&str, &str)])`, keep `start(port, data)` as a wrapper, and keep `PDT_SECRET_STORE=memory`. Add `#![allow(dead_code)]` at the top of `common/mod.rs`. In `lifecycle.rs`, add `mod common; use common::*;`.

`crates/pdtd/tests/hooks.rs`:

```rust
mod common;
use common::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hook(args: &[&str], env: &[(&str, &str)], stdin: &str) -> (std::process::Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pdtd"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd
        .arg("hook")
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let t = Instant::now();
    let out = child.wait_with_output().unwrap();
    (out, t.elapsed())
}

#[test]
fn hook_is_silent_and_exits_0_when_pdtd_is_down() {
    let dead = free_port().to_string();
    for (args, env) in [
        (vec!["claude", "UserPromptSubmit"], vec![("PDT_SESSION_ID", "s"), ("PDT_HOOK_PORT", dead.as_str())]),
        (vec!["claude", "Stop"], vec![]),
        (vec!["codex-notify", r#"{"type":"agent-turn-complete"}"#], vec![("PDT_SESSION_ID", "s"), ("PDT_HOOK_PORT", dead.as_str())]),
        (vec!["nonsense"], vec![]),
        (vec![], vec![]),
    ] {
        let (out, took) = hook(&args, &env, r#"{"prompt":"hello"}"#);
        assert!(out.status.success(), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty(), "{args:?} printed {:?}", String::from_utf8_lossy(&out.stdout));
        assert!(took < Duration::from_secs(5), "{args:?} took {took:?}");
    }
}

#[test]
fn codex_notify_runs_the_users_chained_notifier() {
    let tmp = tempfile::tempdir().unwrap();
    let out_file = tmp.path().join("chained.txt");
    let script = "require('fs').writeFileSync(process.argv[1], process.argv[2])";
    let file = out_file.to_string_lossy();
    let payload = r#"{"type":"agent-turn-complete","thread-id":"t"}"#;
    let (out, _) = hook(&["codex-notify", "--chain", "node", "-e", script, &file, "--", payload], &[], "");
    assert!(out.status.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !out_file.exists() {
        assert!(Instant::now() < deadline, "the chained notifier never ran");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(std::fs::read_to_string(&out_file).unwrap(), payload);
}

/// A stand-in for `claude` that records its arguments and PDT_SESSION_ID, then waits.
fn fake_claude(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join("claude.cmd");
        std::fs::write(
            &p,
            "@echo off\r\necho %*> \"%~dp0args.txt\"\r\necho %PDT_SESSION_ID%> \"%~dp0session.txt\"\r\nping -n 120 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        p
    } else {
        let p = dir.join("claude");
        std::fs::write(
            &p,
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/args.txt\"\necho \"$PDT_SESSION_ID\" > \"$(dirname \"$0\")/session.txt\"\nexec sleep 120\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}

#[test]
fn hook_events_change_a_sessions_activity() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = fake_claude(&bin);
    let data = tmp.path().join("data");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git").args(["init", "-q", "-b", "main"]).arg(&repo).status().unwrap().success());
    let port = free_port();
    let _d = start_with(port, &data, &[("PDT_CLAUDE_PROGRAM", &claude.to_string_lossy())]);
    let (_, p) = http(port, "POST", "/api/projects", Some(&serde_json::json!({"name": "P", "repoPath": repo.to_string_lossy()}).to_string()));
    let body = serde_json::json!({"projectId": p["id"], "kind": "agent_tui", "agent": "claude", "cols": 80, "rows": 24});
    let (s, sess) = http(port, "POST", "/api/sessions", Some(&body.to_string()));
    assert_eq!(s, 201, "{sess}");
    let id = sess["id"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(20);
    while !bin.join("session.txt").exists() {
        assert!(Instant::now() < deadline, "the fake claude never started");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(bin.join("session.txt")).unwrap().trim(), id, "PDT_SESSION_ID is set");
    let args = std::fs::read_to_string(bin.join("args.txt")).unwrap();
    assert!(args.contains("--settings"), "{args}");
    let settings_file = data.join("sessions").join(&id).join("claude-settings.json");
    let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&settings_file).unwrap()).unwrap();
    let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(Path::new(command).is_file(), "the hook runs this pdtd: {command}");

    let port_s = port.to_string();
    let env = [("PDT_SESSION_ID", id.as_str()), ("PDT_HOOK_PORT", port_s.as_str())];
    let activity = || http(port, "GET", &format!("/api/sessions/{id}"), None).1;
    assert_eq!(activity()["activity"], "idle");
    assert!(hook(&["claude", "UserPromptSubmit"], &env, r#"{"prompt":"x"}"#).0.status.success());
    assert_eq!(activity()["activity"], "working");
    hook(&["claude", "Stop"], &env, "{}");
    let s = activity();
    assert_eq!((s["activity"].as_str(), s["unseen"].as_bool()), (Some("idle"), Some(true)));
    assert_eq!(http(port, "POST", &format!("/api/sessions/{id}/view"), None).0, 204);
    assert_eq!(activity()["unseen"], false);
    hook(&["claude", "Notification"], &env, r#"{"notification_type":"permission_prompt","message":"m"}"#);
    assert_eq!(activity()["activity"], "needs_you");
}
```

Run: `cargo test -p pdtd --test hooks`
Expected: FAIL (`hook` is not a subcommand yet; the daemon starts instead and the test times out or fails).

Implement `crates/pdtd/src/hook.rs`:

```rust
//! `pdtd hook …` (remote-host spec §5.1, decision 11): reports an agent hook event to the pdtd that owns the session
//! (`PDT_SESSION_ID`, `PDT_HOOK_PORT`). It never writes to stdout (Claude adds UserPromptSubmit stdout to the
//! conversation) and always exits 0, quickly, whatever happens.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const STDIN_LIMIT: u64 = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(2);

pub fn run(args: &[String], stdin: &mut dyn Read, env: &dyn Fn(&str) -> Option<String>) -> i32 {
    let report = match args.first().map(String::as_str) {
        Some("claude") => {
            let Some(event) = args.get(1) else { return 0 };
            let mut buf = Vec::new();
            let _ = stdin.take(STDIN_LIMIT).read_to_end(&mut buf);
            let input: serde_json::Value = serde_json::from_slice(&buf).unwrap_or_default();
            serde_json::json!({
                "source": "claude",
                "event": event,
                "notificationType": input.get("notification_type").and_then(|v| v.as_str()),
            })
        }
        Some("codex-notify") => {
            let (chain, payload) = split_chain(&args[1..]);
            if let Some(p) = payload
                && !chain.is_empty()
            {
                run_chain(&chain, p);
            }
            let event = payload
                .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
                .unwrap_or_default();
            serde_json::json!({ "source": "codex", "event": event })
        }
        _ => return 0,
    };
    if let (Some(id), Some(port)) = (env("PDT_SESSION_ID"), env("PDT_HOOK_PORT").and_then(|p| p.parse::<u16>().ok())) {
        let _ = post(port, &format!("/api/sessions/{id}/activity"), &report.to_string());
    }
    0
}

/// `[--chain <cmd…> --] <payload>` → (cmd, payload). Codex appends the payload as the last argument.
fn split_chain(args: &[String]) -> (Vec<String>, Option<&str>) {
    let payload = args.last().map(String::as_str);
    if args.first().map(String::as_str) != Some("--chain") || args.len() < 2 {
        return (Vec::new(), payload);
    }
    match args[..args.len() - 1].iter().rposition(|a| a == "--") {
        Some(end) => (args[1..end].to_vec(), payload),
        None => (Vec::new(), payload),
    }
}

/// Starts the user's own notifier with the same payload, without waiting for it.
fn run_chain(chain: &[String], payload: &str) {
    let mut cmd = std::process::Command::new(&chain[0]);
    cmd.args(&chain[1..])
        .arg(payload)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = cmd.spawn();
}

fn post(port: u16, path: &str, body: &str) -> std::io::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    let req = format!(
        "POST {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes())?;
    let mut sink = [0u8; 1024];
    let _ = s.read(&mut sink);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn chain_parsing() {
        assert_eq!(split_chain(&s(&["{}"])), (vec![], Some("{}")));
        assert_eq!(split_chain(&s(&["--chain", "n.exe", "turn-ended", "--", "{}"])), (s(&["n.exe", "turn-ended"]), Some("{}")));
        assert_eq!(split_chain(&s(&["--chain", "a", "--", "b", "--", "{}"])), (s(&["a", "--", "b"]), Some("{}")));
        assert_eq!(split_chain(&s(&["--chain", "a", "{}"])), (vec![], Some("{}")));
        assert_eq!(split_chain(&[]), (vec![], None));
    }

    #[test]
    fn unknown_or_incomplete_input_exits_0_without_network() {
        let env = |_: &str| None;
        assert_eq!(run(&s(&["claude"]), &mut std::io::empty(), &env), 0);
        assert_eq!(run(&s(&["what"]), &mut std::io::empty(), &env), 0);
        assert_eq!(run(&s(&["claude", "Stop"]), &mut "not json".as_bytes(), &env), 0);
    }
}
```

In `crates/pdtd/src/main.rs`:
- add `mod hook;`;
- replace `#[tokio::main] async fn main()` with:

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("hook") {
        std::process::exit(hook::run(&args[2..], &mut std::io::stdin(), &|k| std::env::var(k).ok()));
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(run()) {
        tracing::error!("{e:#}");
        eprintln!("{DAEMON_BIN}: {e:#}");
        std::process::exit(2);
    }
}
```

In `config.rs`, add `claude_program` and `codex_program` (both `Option<String>`) from `PDT_CLAUDE_PROGRAM` and `PDT_CODEX_PROGRAM`, plus a config test. In `run()`, after constructing the state, override `state.platform.claude_program` and `state.platform.codex_program` when they are set. Spawn the ticker: `tokio::spawn(pdt_api::run_activity_ticker(state.clone()));`.

Run: `cargo test -p pdtd`
Expected: PASS.

- [ ] **Step 9: Update the web fixtures, full check and commit**

In `web/src/sessions/sessions.test.tsx`, add `activity: null, attentionAt: null, lastViewedAt: null, unseen: false` to the `session()` defaults. Make the same change to the `shell` object in `AppShell.test.tsx`.

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test; pnpm --dir web typecheck; pnpm --dir web test`
Expected: green.

```bash
git add crates web
git commit -m "feat: agent activity from Claude hooks, Codex notify and terminal heuristics; unseen tracking"
```

---

### Task 10: Activity and unseen in the UI

**Files:**
- Create: `web/src/sessions/ActivityBadge.tsx`, `web/src/sessions/useViewTracking.ts`, `web/src/sessions/activity.test.tsx`
- Modify: `web/src/api/client.ts`, `web/src/sessions/SessionList.tsx`, `web/src/sessions/SessionView.tsx`

**Interfaces:**
- Consumes: `Session.activity` and `Session.unseen`, plus `POST /api/sessions/{id}/view` (Task 9).
- Produces:
  - `api.viewSession(id)`;
  - `<ActivityBadge activity unseen />`;
  - `useViewTracking(sessionId, polledAt)`: posts a view on mount, on every poll, and whenever the tab becomes visible, but only while `document.visibilityState === 'visible'`.

- [ ] **Step 1: Write the failing tests**

`web/src/sessions/activity.test.tsx`:

```tsx
import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { ActivityBadge } from './ActivityBadge';
import { SessionList } from './SessionList';
import { useViewTracking } from './useViewTracking';

vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => <div /> }));
afterEach(() => vi.unstubAllGlobals());

const base: Session = {
  id: 's1', projectId: 'p', worktreeId: null, kind: 'agent_tui', agent: 'claude', title: 'Claude · PDT', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false,
};

describe('ActivityBadge', () => {
  it('names every activity and marks unseen ones', () => {
    const { rerender } = render(<ActivityBadge activity="working" unseen={false} />);
    expect(screen.getByText('Working')).toBeInTheDocument();
    rerender(<ActivityBadge activity="needs_you" unseen />);
    expect(screen.getByText('Needs you')).toBeInTheDocument();
    expect(screen.getByLabelText('unseen')).toBeInTheDocument();
    rerender(<ActivityBadge activity="idle" unseen={false} />);
    expect(screen.queryByLabelText('unseen')).not.toBeInTheDocument();
    rerender(<ActivityBadge activity="unknown" unseen={false} />);
    expect(screen.getByText('Unknown')).toBeInTheDocument();
    rerender(<ActivityBadge activity={null} unseen={false} />);
    expect(screen.queryByText(/./)).not.toBeInTheDocument();
  });
});

describe('SessionList activity', () => {
  it('distinguishes idle-and-unseen from idle-and-seen', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/sessions': [base, { ...base, id: 's2', title: 'Codex · PDT', agent: 'codex', unseen: true }],
    });
    renderWithQuery(<SessionList project={{ id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1 }} selectedId={null} onSelect={() => {}} />);
    const seen = await screen.findByRole('button', { name: /Claude · PDT/ });
    const unseen = screen.getByRole('button', { name: /Codex · PDT/ });
    expect(seen).not.toHaveTextContent('unseen');
    expect(unseen).toHaveAccessibleName(/unseen/);
  });
});

function Probe({ id, polledAt }: { id: string; polledAt: number }) {
  useViewTracking(id, polledAt);
  return null;
}

describe('useViewTracking', () => {
  it('posts views while the tab is visible, including on each poll', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/view': undefined });
    let visibility = 'visible';
    vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility as DocumentVisibilityState);
    const { rerender } = renderWithQuery(<Probe id="s1" polledAt={1} />);
    await waitFor(() => expect(calls).toHaveLength(1));
    rerender(<Probe id="s1" polledAt={2} />);
    await waitFor(() => expect(calls).toHaveLength(2));
    visibility = 'hidden';
    rerender(<Probe id="s1" polledAt={3} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toHaveLength(2);
    visibility = 'visible';
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    await waitFor(() => expect(calls).toHaveLength(3));
  });
});
```

(`renderWithQuery` returns RTL's `render` result, so `rerender` re-renders inside the same provider. If the helper does not pass `rerender` through, wrap with `QueryClientProvider` in the test.)

Run: `pnpm --dir web test -- activity`
Expected: FAIL.

- [ ] **Step 2: Implement**

Add to the `api` object in `client.ts`: `viewSession: (id: string) => request<void>(`/api/sessions/${enc(id)}/view`, { method: 'POST' }),`.

`web/src/sessions/ActivityBadge.tsx`:

```tsx
import type { Activity } from '../api/gen/Activity';

const LABEL: Record<Activity, string> = { working: 'Working', needs_you: 'Needs you', idle: 'Idle', unknown: 'Unknown' };
const TONE: Record<Activity, string> = {
  working: 'text-running',
  needs_you: 'text-waiting font-semibold',
  idle: 'text-muted',
  unknown: 'text-muted italic',
};

/** An agent's activity; `unseen` adds an accent dot (it became idle or needs you since anyone last looked). */
export function ActivityBadge({ activity, unseen }: { activity: Activity | null; unseen: boolean }) {
  if (!activity) return null;
  return (
    <span className={`ml-auto flex items-center gap-1 text-[11px] ${TONE[activity]}`}>
      {unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-accent" />}
      {LABEL[activity]}
    </span>
  );
}
```

`web/src/sessions/useViewTracking.ts`:

```ts
import { useEffect } from 'react';
import { api } from '../api/client';

/** Tells the daemon the user is looking at this session (clears "unseen" on every device). It posts on mount, on
 * every poll (so something that happens while it is on screen is seen at once), and when the tab becomes visible,
 * but only while the tab is visible. */
export function useViewTracking(sessionId: string, polledAt?: number) {
  useEffect(() => {
    if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
  }, [sessionId, polledAt]);
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void api.viewSession(sessionId).catch(() => {});
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [sessionId]);
}
```

In `SessionList.tsx`, render `<ActivityBadge activity={s.activity} unseen={s.unseen} />` inside each session button, after the title. Bold the title when `s.unseen`.

In `SessionView.tsx`, call `useViewTracking(session.id, polledAt)` when `session.kind === 'agent_tui'`. Hooks cannot be conditional, so pass the id always and let the server ignore shells: the view endpoint accepts any session. Also show `<ActivityBadge>` in the header.

Run: `pnpm --dir web typecheck; pnpm --dir web test`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add web
git commit -m "feat(web): agent activity badges and shared unseen state"
```

---

### Task 11: Update checking, verified downloads and the hand-off to `pdt-host`

**Files:**
- Create:
  - `crates/pdt-core/src/release.rs`, `crates/pdt-core/src/handoff.rs`
  - `crates/pdt-update/Cargo.toml`, `crates/pdt-update/src/lib.rs`, `crates/pdt-update/tests/github.rs`
  - `crates/pdt-api/src/update.rs`, `crates/pdt-api/tests/update.rs`
- Modify:
  - `Cargo.toml` (workspace member, default member, dependency)
  - `crates/pdt-core/src/lib.rs` (and `BUILD_VERSION`)
  - `crates/pdt-api/{Cargo.toml,src/routes.rs,src/lib.rs}`
  - `crates/pdtd/src/{main,config}.rs`
  - `.github/workflows/ci.yml`

**Interfaces:**
- Consumes:
  - `SecretStore` and `UPDATE_TOKEN` (Task 2);
  - `HostSettings.update_policy` (Task 2);
  - `Session.activity` (Task 9);
  - `RunState::SettingUp` (Task 7);
  - `AppState.shutdown`.
- Produces (`pdt_core::release`):
  - `Version { major, minor, patch, local }`, with `parse_version(&str)`, `is_newer_than` and `Display`.
  - `HostOs { Windows, Mac, Other }`, with `current()`. `asset_suffix(os)` and `select_asset(&[&str], os)`.
  - `SUMS_ASSET = "SHA256SUMS"`, `parse_sums(&str) -> BTreeMap<String, String>`.
  - `TokenState`, tagged by `state`: `missing`, `valid{expiresAt?}`, `expires_soon{expiresAt}` or `expired`. `token_state(present, expires_at, rejected, now)`.
- Produces (`pdt_core::handoff`):
  - `UpdateRequest { version, tag, asset: PathBuf, fromVersion, requestedAt }`.
  - `UpdateOutcome { Installed, RolledBack, Failed }`.
  - `UpdateResult { outcome, fromVersion, toVersion, message, log: Option<String>, at }`.
  - `update_dir`, `request_path`, `result_path` and `downloads_dir` (each takes `data_dir`).
- Produces (`pdt_update`):
  - `GITHUB_API`.
  - `GitHubClient::new(api_base, repo)`, with `latest(token) -> Result<Latest, UpdateError>` and `download(token, &Asset, dest)`.
  - `Latest { release: Release, token_expires_at: Option<i64> }` and `Release { tag, version, notes, published_at, assets: Vec<Asset> }`.
  - `fetch_verified(&client, token, &release, HostOs, dir) -> Result<PathBuf, UpdateError>`.
  - `parse_expiry(&str) -> Option<i64>`.
- Produces (`pdt_api`):
  - `UpdateService::new(api_base, repo, supervised, current)`, `AppState.update: Arc<UpdateService>`, and `UpdateStatus`, `UpdateState` and `ReleaseSummary`.
  - Routes:
    - `GET /api/host/update`;
    - `POST /api/host/update/check`;
    - `POST /api/host/update/install` (body `{force: bool}`);
    - `POST /api/host/update/dismiss`;
    - `PUT /api/host/token` (body `{token}`) and `DELETE /api/host/token`.
  - Errors: 409 `host_busy`, 409 `updates_disabled`, 409 `no_update`, 409 `no_token`, 502 `update_failed`, 400 `invalid_token`.
  - `pub async fn run_update_loop(AppState)`.
- Produces (`pdtd`): `Config.supervised` (`PDT_SUPERVISED=1`). It loads `<data>/update/result.json` at startup.

- [ ] **Step 1: Versions, assets, checksums and token state (TDD)**

Add `pub mod release;` and `pub mod handoff;` to `pdt-core/src/lib.rs`. Change `BUILD_VERSION` (decision 15):

```rust
/// The version this build reports: `PDT_BUILD_VERSION` at compile time (CI release builds set it to the release
/// version, e.g. `0.1.42`), otherwise `<crate version>-local`, which never counts as newer than a release.
pub const BUILD_VERSION: &str = match option_env!("PDT_BUILD_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-local"),
};
```

`crates/pdt-core/src/release.rs`, tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_local_builds() {
        let v = |s| parse_version(s).unwrap();
        assert_eq!(v("v0.1.42"), Version { major: 0, minor: 1, patch: 42, local: false });
        assert_eq!(v("0.1.0-local"), Version { major: 0, minor: 1, patch: 0, local: true });
        assert_eq!(v("0.1.3+abc"), Version { major: 0, minor: 1, patch: 3, local: false });
        for bad in ["", "v", "1.2", "1.2.3.4", "0.1.0-rc.1", "latest", "0.x.1"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(v("0.1.10").is_newer_than(&v("0.1.9")));
        assert!(!v("0.1.9").is_newer_than(&v("0.1.9")));
        assert!(v("0.1.1").is_newer_than(&v("0.1.0-local")), "any release beats a local build");
        assert!(v("0.1.1").is_newer_than(&v("0.9.9-local")));
        assert!(!v("0.9.9-local").is_newer_than(&v("0.1.1")), "a local build never counts as newer");
        assert_eq!(v("0.1.0-local").to_string(), "0.1.0-local");
        assert_eq!(v("v0.1.7").to_string(), "0.1.7");
    }

    #[test]
    fn asset_selection_matches_ci_names() {
        let names = ["Personal.Dev.Team_0.1.7_x64-setup.exe", "Personal.Dev.Team.app.tar.gz", "SHA256SUMS"];
        assert_eq!(select_asset(&names, HostOs::Windows), Some("Personal.Dev.Team_0.1.7_x64-setup.exe"));
        assert_eq!(select_asset(&names, HostOs::Mac), Some("Personal.Dev.Team.app.tar.gz"));
        assert_eq!(select_asset(&names, HostOs::Other), None);
        assert_eq!(select_asset(&["-setup.exe"], HostOs::Windows), None);
    }

    #[test]
    fn sums_file() {
        let text = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  a-setup.exe\n\
                    0000000000000000000000000000000000000000000000000000000000000000 *b.app.tar.gz\n\
                    garbage line\n";
        let s = parse_sums(text);
        assert_eq!(s["a-setup.exe"], "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
        assert_eq!(s["b.app.tar.gz"], "0".repeat(64));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn token_warnings() {
        let day = 86_400_000;
        assert_eq!(token_state(false, None, false, 0), TokenState::Missing);
        assert_eq!(token_state(true, None, false, 0), TokenState::Valid { expires_at: None });
        assert_eq!(token_state(true, Some(30 * day), false, 0), TokenState::Valid { expires_at: Some(30 * day) });
        assert_eq!(token_state(true, Some(14 * day), false, 0), TokenState::ExpiresSoon { expires_at: 14 * day });
        assert_eq!(token_state(true, Some(10), false, 20), TokenState::Expired);
        assert_eq!(token_state(true, Some(30 * day), true, 0), TokenState::Expired, "rejected means expired or revoked");
        assert_eq!(serde_json::to_string(&TokenState::ExpiresSoon { expires_at: 5 }).unwrap(), r#"{"state":"expires_soon","expiresAt":5}"#);
    }
}
```

Implement, above the tests:

```rust
//! Releases and updates (remote-host spec §3): versions, assets, checksums, token warnings. Pure.

use serde::Serialize;
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// A local build (`X.Y.Z-local`): older than every release.
    pub local: bool,
}

impl Version {
    pub fn is_newer_than(&self, current: &Version) -> bool {
        if self.local {
            return false;
        }
        current.local || (self.major, self.minor, self.patch) > (current.major, current.minor, current.patch)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}{}", self.major, self.minor, self.patch, if self.local { "-local" } else { "" })
    }
}

/// `X.Y.Z`, `vX.Y.Z` or `X.Y.Z-local`, with an optional `+build` suffix. Other pre-releases are not ours.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s.split('+').next()?;
    let (core, local) = match s.split_once('-') {
        Some((c, "local")) => (c, true),
        Some(_) => return None,
        None => (s, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()??, local };
    parts.next().is_none().then_some(v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    Mac,
    Other,
}

impl HostOs {
    pub fn current() -> HostOs {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::Mac
        } else {
            HostOs::Other
        }
    }
}

/// The release asset each platform installs: the NSIS installer, or the tarred `.app` bundle.
pub fn asset_suffix(os: HostOs) -> Option<&'static str> {
    match os {
        HostOs::Windows => Some("-setup.exe"),
        HostOs::Mac => Some(".app.tar.gz"),
        HostOs::Other => None,
    }
}

pub fn select_asset<'a>(names: &[&'a str], os: HostOs) -> Option<&'a str> {
    let suffix = asset_suffix(os)?;
    names.iter().copied().find(|n| n.len() > suffix.len() && n.ends_with(suffix))
}

pub const SUMS_ASSET: &str = "SHA256SUMS";

/// `sha256sum` output: `<64 hex>  <name>` or `<64 hex> *<name>` → name → lower-case hex. Other lines are ignored.
pub fn parse_sums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, rest) = l.split_once(' ')?;
            let name = rest.trim_start_matches(' ').trim_start_matches('*').trim_end();
            (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub const TOKEN_WARN_MS: i64 = 14 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TokenState {
    Missing,
    Valid {
        #[ts(type = "number | null")]
        expires_at: Option<i64>,
    },
    ExpiresSoon {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// Expired, or GitHub rejected it (revoked).
    Expired,
}

/// Warns `TOKEN_WARN_MS` (14 days) ahead of expiry (spec §3.2).
pub fn token_state(present: bool, expires_at: Option<i64>, rejected: bool, now: i64) -> TokenState {
    match (present, expires_at) {
        (false, _) => TokenState::Missing,
        _ if rejected => TokenState::Expired,
        (true, Some(t)) if t <= now => TokenState::Expired,
        (true, Some(t)) if t - now <= TOKEN_WARN_MS => TokenState::ExpiresSoon { expires_at: t },
        (true, t) => TokenState::Valid { expires_at: t },
    }
}
```

`crates/pdt-core/src/handoff.rs`:

```rust
//! The files pdtd and pdt-host use to hand an update over (remote-host spec §3.4). Pure: types and paths only.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ts_rs::TS;

pub fn update_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("update")
}
/// Written by pdtd just before it shuts down for an update; taken (renamed) by pdt-host.
pub fn request_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("request.json")
}
/// Written by pdt-host after an install attempt; shown by pdtd until dismissed.
pub fn result_path(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("result.json")
}
pub fn downloads_dir(data_dir: &Path) -> PathBuf {
    update_dir(data_dir).join("downloads")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub version: String,
    pub tag: String,
    /// The verified installer (Windows) or `.app.tar.gz` (macOS).
    pub asset: PathBuf,
    pub from_version: String,
    pub requested_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateOutcome {
    Installed,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateResult {
    pub outcome: UpdateOutcome,
    pub from_version: String,
    pub to_version: String,
    pub message: String,
    /// The update log's path, when there is one.
    pub log: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_the_data_dir() {
        let d = Path::new("data");
        assert_eq!(request_path(d), d.join("update").join("request.json"));
        assert_eq!(result_path(d), d.join("update").join("result.json"));
        assert_eq!(downloads_dir(d), d.join("update").join("downloads"));
    }

    #[test]
    fn request_roundtrips_as_camel_case_json() {
        let r = UpdateRequest {
            version: "0.1.7".into(),
            tag: "v0.1.7".into(),
            asset: PathBuf::from("a.exe"),
            from_version: "0.1.6".into(),
            requested_at: 1,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"fromVersion\":\"0.1.6\""), "{j}");
        assert_eq!(serde_json::from_str::<UpdateRequest>(&j).unwrap(), r);
    }
}
```

Run: `cargo test -p pdt-core`
Expected: PASS. Note that `TokenState` derives `Serialize` only.

- [ ] **Step 2: Create `pdt-update` and write its failing tests**

`crates/pdt-update/Cargo.toml`:

```toml
[package]
name = "pdt-update"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true
```

Run (PowerShell):
- `cargo add pdt-core thiserror serde serde_json jiff hex sha2 futures-util -p pdt-update`
- `cargo add tokio -p pdt-update --features fs,io-util`
- `cargo add reqwest -p pdt-update --no-default-features --features native-tls,json,stream,http2,system-proxy`
- `cargo add axum tempfile -p pdt-update --dev`
- `cargo add tokio -p pdt-update --dev --features full`

Root `Cargo.toml`:
- add `"crates/pdt-update"` to `default-members`;
- add `pdt-update = { path = "crates/pdt-update" }` under `[workspace.dependencies]`.

`crates/pdt-update/tests/github.rs`:

```rust
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pdt_core::release::HostOs;
use pdt_update::{GitHubClient, UpdateError, fetch_verified, parse_expiry};
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
struct Fake {
    base: Arc<std::sync::OnceLock<String>>,
    sums: Arc<std::sync::Mutex<String>>,
    token_leaked: Arc<AtomicBool>,
}

const WIN: &[u8] = b"windows installer bytes";
const MAC: &[u8] = b"mac bundle bytes";

fn sha(b: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(b))
}

fn good_sums() -> String {
    format!("{}  Personal.Dev.Team_0.1.7_x64-setup.exe\n{}  Personal.Dev.Team.app.tar.gz\n", sha(WIN), sha(MAC))
}

fn authed(h: &HeaderMap) -> bool {
    h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some("Bearer good")
}

async fn latest(h: HeaderMap) -> Response {
    if !authed(&h) {
        return (StatusCode::UNAUTHORIZED, r#"{"message":"Bad credentials"}"#).into_response();
    }
    let body = serde_json::json!({
        "tag_name": "v0.1.7", "body": "Release notes", "published_at": "2026-10-15T00:00:00Z",
        "assets": [
            {"id": 1, "name": "Personal.Dev.Team_0.1.7_x64-setup.exe", "size": WIN.len()},
            {"id": 2, "name": "Personal.Dev.Team.app.tar.gz", "size": MAC.len()},
            {"id": 3, "name": "SHA256SUMS", "size": 1}
        ]
    });
    ([("github-authentication-token-expiration", "2030-01-02 03:04:05 UTC")], axum::Json(body)).into_response()
}

async fn asset(State(f): State<Fake>, Path(id): Path<u64>, h: HeaderMap) -> Response {
    if !authed(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if h.get(header::ACCEPT).and_then(|v| v.to_str().ok()) != Some("application/octet-stream") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if id == 3 {
        return f.sums.lock().unwrap().clone().into_response();
    }
    let to = format!("{}/storage/{id}", f.base.get().unwrap());
    (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
}

async fn storage(State(f): State<Fake>, Path(id): Path<u64>, h: HeaderMap) -> Response {
    if h.contains_key(header::AUTHORIZATION) {
        f.token_leaked.store(true, Ordering::SeqCst);
        return StatusCode::BAD_REQUEST.into_response();
    }
    (if id == 1 { WIN } else { MAC }).to_vec().into_response()
}

async fn serve() -> (GitHubClient, Fake) {
    let fake = Fake {
        base: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(good_sums())),
        token_leaked: Default::default(),
    };
    let app = Router::new()
        .route("/repos/o/r/releases/latest", get(latest))
        .route("/repos/o/r/releases/assets/{id}", get(asset))
        .route("/storage/{id}", get(storage))
        .with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    fake.base.set(base.clone()).unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (GitHubClient::new(&base, "o/r"), fake)
}

#[tokio::test]
async fn latest_release_and_token_expiry() {
    let (c, _) = serve().await;
    let l = c.latest("good").await.unwrap();
    assert_eq!((l.release.tag.as_str(), l.release.version.to_string()), ("v0.1.7", "0.1.7".to_string()));
    assert_eq!(l.release.notes, "Release notes");
    assert_eq!(l.release.assets.len(), 3);
    assert_eq!(l.token_expires_at, parse_expiry("2030-01-02 03:04:05 UTC"));
    assert!(l.token_expires_at.is_some());
}

#[tokio::test]
async fn a_rejected_token_is_unauthorized() {
    let (c, _) = serve().await;
    assert_eq!(c.latest("expired").await.unwrap_err(), UpdateError::Unauthorized);
}

#[tokio::test]
async fn asset_redirect_drops_the_token() {
    let (c, fake) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    for (os, bytes) in [(HostOs::Windows, WIN), (HostOs::Mac, MAC)] {
        let path = fetch_verified(&c, "good", &release, os, dir.path()).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert!(!fake.token_leaked.load(Ordering::SeqCst), "the storage host received the GitHub token");
}

#[tokio::test]
async fn bad_or_missing_checksums_are_rejected() {
    let (c, fake) = serve().await;
    let release = c.latest("good").await.unwrap().release;
    let dir = tempfile::tempdir().unwrap();
    *fake.sums.lock().unwrap() = good_sums().replace(&sha(WIN), &"0".repeat(64));
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(ref n) if n.contains("setup.exe")), "{e:?}");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!leftovers.iter().any(|n| n.to_string_lossy().ends_with("setup.exe")), "{leftovers:?}");
    *fake.sums.lock().unwrap() = String::new();
    let e = fetch_verified(&c, "good", &release, HostOs::Windows, dir.path()).await.unwrap_err();
    assert!(matches!(e, UpdateError::Checksum(_)), "an unlisted asset is rejected: {e:?}");
    let mut no_sums = release.clone();
    no_sums.assets.retain(|a| a.name != "SHA256SUMS");
    assert_eq!(fetch_verified(&c, "good", &no_sums, HostOs::Windows, dir.path()).await.unwrap_err(), UpdateError::NoChecksums);
}

#[test]
fn expiry_header_formats() {
    let utc = parse_expiry("2030-01-02 03:04:05 UTC").unwrap();
    assert_eq!(parse_expiry("2030-01-02 05:04:05 +0200"), Some(utc));
    assert_eq!(parse_expiry("2030-01-01 22:04:05 -0500"), Some(utc));
    assert_eq!(parse_expiry("soon"), None);
}
```

Run: `cargo test -p pdt-update`
Expected: compile errors.

- [ ] **Step 3: Implement `pdt-update`**

`crates/pdt-update/src/lib.rs`:

```rust
//! The GitHub REST client for updates (remote-host spec §3.2–3.3; verified fact 6). The token goes only to the API
//! host: asset redirects are followed by hand, without it (decision 13).

use futures_util::StreamExt;
use pdt_core::release::{HostOs, SUMS_ASSET, Version, parse_sums, parse_version, select_asset};
use reqwest::header::{ACCEPT, LOCATION};
use serde::Deserialize;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const GITHUB_API: &str = "https://api.github.com";
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub notes: String,
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Latest {
    pub release: Release,
    /// From `GitHub-Authentication-Token-Expiration` (ms); None for tokens without an expiry.
    pub token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UpdateError {
    #[error("the update token was rejected (expired or revoked)")]
    Unauthorized,
    #[error("no release was found, or the token cannot see the release repository")]
    NotFound,
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub answered {0}: {1}")]
    Http(u16, String),
    #[error("the latest release is not usable: {0}")]
    BadRelease(String),
    #[error("the release has no installer for this platform")]
    NoAsset,
    #[error("the release has no SHA256SUMS file")]
    NoChecksums,
    #[error("the checksum of {0} does not match SHA256SUMS")]
    Checksum(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    id: u64,
    name: String,
    size: u64,
}

/// `YYYY-MM-DD HH:MM:SS UTC` or `YYYY-MM-DD HH:MM:SS ±HHMM` → Unix ms.
pub fn parse_expiry(header: &str) -> Option<i64> {
    let h = header.trim();
    let h = match h.strip_suffix(" UTC") {
        Some(t) => format!("{t} +0000"),
        None => h.to_string(),
    };
    jiff::Timestamp::strptime("%Y-%m-%d %H:%M:%S %z", &h).ok().map(|t| t.as_millisecond())
}

fn net(e: reqwest::Error) -> UpdateError {
    UpdateError::Network(e.to_string())
}

async fn check(res: reqwest::Response) -> Result<reqwest::Response, UpdateError> {
    match res.status().as_u16() {
        200..=299 => Ok(res),
        401 => Err(UpdateError::Unauthorized),
        404 => Err(UpdateError::NotFound),
        code => Err(UpdateError::Http(code, res.text().await.unwrap_or_default().chars().take(200).collect())),
    }
}

pub struct GitHubClient {
    http: reqwest::Client,
    api: String,
    repo: String,
}

impl GitHubClient {
    pub fn new(api_base: &str, repo: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("{}-updater", pdt_core::brand::SLUG))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(900))
            .build()
            .expect("an HTTP client");
        Self { http, api: api_base.trim_end_matches('/').to_string(), repo: repo.to_string() }
    }

    fn api_get(&self, path: &str, token: &str, accept: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}/repos/{}/{path}", self.api, self.repo))
            .bearer_auth(token)
            .header(ACCEPT, accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    pub async fn latest(&self, token: &str) -> Result<Latest, UpdateError> {
        let res = self.api_get("releases/latest", token, "application/vnd.github+json").send().await.map_err(net)?;
        let token_expires_at = res
            .headers()
            .get("github-authentication-token-expiration")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_expiry);
        let text = check(res).await?.text().await.map_err(net)?;
        let r: ReleaseJson = serde_json::from_str(&text).map_err(|e| UpdateError::BadRelease(e.to_string()))?;
        let version = parse_version(&r.tag_name)
            .filter(|v| !v.local)
            .ok_or_else(|| UpdateError::BadRelease(format!("tag {:?} is not vX.Y.Z", r.tag_name)))?;
        Ok(Latest {
            release: Release {
                tag: r.tag_name,
                version,
                notes: r.body.unwrap_or_default(),
                published_at: r.published_at,
                assets: r.assets.into_iter().map(|a| Asset { id: a.id, name: a.name, size: a.size }).collect(),
            },
            token_expires_at,
        })
    }

    /// Streams an asset to `dest`. The API answers 200 or a redirect (fact 6); redirects are followed without the token.
    pub async fn download(&self, token: &str, asset: &Asset, dest: &Path) -> Result<(), UpdateError> {
        let mut res = self
            .api_get(&format!("releases/assets/{}", asset.id), token, "application/octet-stream")
            .send()
            .await
            .map_err(net)?;
        for _ in 0..MAX_REDIRECTS {
            if !res.status().is_redirection() {
                break;
            }
            let loc = res
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| UpdateError::BadRelease("a redirect without a Location".into()))?;
            let next = res.url().join(loc).map_err(|e| UpdateError::BadRelease(e.to_string()))?;
            res = self.http.get(next).header(ACCEPT, "application/octet-stream").send().await.map_err(net)?;
        }
        let res = check(res).await?;
        let io = |e: std::io::Error| UpdateError::Io(format!("writing {}: {e}", dest.display()));
        let mut file = tokio::fs::File::create(dest).await.map_err(io)?;
        let mut body = res.bytes_stream();
        while let Some(chunk) = body.next().await {
            file.write_all(&chunk.map_err(net)?).await.map_err(io)?;
        }
        file.flush().await.map_err(io)?;
        Ok(())
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Downloads this platform's asset of `release` into `dir` and checks it against the release's SHA256SUMS. Returns
/// the verified file; on any failure nothing is left under the asset's name.
pub async fn fetch_verified(
    client: &GitHubClient,
    token: &str,
    release: &Release,
    os: HostOs,
    dir: &Path,
) -> Result<PathBuf, UpdateError> {
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    let name = select_asset(&names, os).ok_or(UpdateError::NoAsset)?;
    let asset = release.assets.iter().find(|a| a.name == name).expect("selected from the list");
    let sums_asset = release.assets.iter().find(|a| a.name == SUMS_ASSET).ok_or(UpdateError::NoChecksums)?;
    tokio::fs::create_dir_all(dir).await.map_err(|e| UpdateError::Io(e.to_string()))?;
    let sums_file = dir.join(SUMS_ASSET);
    client.download(token, sums_asset, &sums_file).await?;
    let sums = parse_sums(&tokio::fs::read_to_string(&sums_file).await.map_err(|e| UpdateError::Io(e.to_string()))?);
    let expected = sums.get(name).cloned().ok_or_else(|| UpdateError::Checksum(name.to_string()))?;
    let part = dir.join(format!("{name}.part"));
    let fin = dir.join(name);
    let _ = tokio::fs::remove_file(&fin).await;
    client.download(token, asset, &part).await?;
    let p = part.clone();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&p))
        .await
        .map_err(|e| UpdateError::Io(e.to_string()))?
        .map_err(|e| UpdateError::Io(e.to_string()))?;
    if actual != expected {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(UpdateError::Checksum(name.to_string()));
    }
    tokio::fs::rename(&part, &fin).await.map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(fin)
}
```

Run: `cargo test -p pdt-update`
Expected: PASS.

- [ ] **Step 4: Write the failing service tests**

Run (PowerShell): `cargo add pdt-update -p pdt-api`, then `cargo add sha2 hex -p pdt-api --dev`.

`crates/pdt-api/tests/update.rs`. Copy the imports and the `call` helper from `tests/host.rs`. Then add a fake GitHub that answers asset downloads directly with 200; the redirect path is covered in `pdt-update`.

```rust
use pdt_core::activity::Activity;
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const WIN: &[u8] = b"win";
const MAC: &[u8] = b"mac";

#[derive(Clone)]
struct Fake {
    hits: Arc<AtomicUsize>,
    sums: Arc<std::sync::Mutex<String>>,
}

async fn fake_github() -> (String, Fake) {
    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    let sha = |b: &[u8]| hex::encode(sha2::Sha256::digest(b));
    let fake = Fake {
        hits: Default::default(),
        sums: Arc::new(std::sync::Mutex::new(format!("{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n", sha(WIN), sha(MAC)))),
    };
    let ok = |h: &HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer good");
    let app = axum::Router::new()
        .route(
            "/repos/o/r/releases/latest",
            axum::routing::get(move |State(f): State<Fake>, h: HeaderMap| async move {
                f.hits.fetch_add(1, Ordering::SeqCst);
                if !ok(&h) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                axum::Json(json!({"tag_name": "v0.1.7", "body": "notes", "published_at": null, "assets": [
                    {"id": 1, "name": "P_0.1.7_x64-setup.exe", "size": 3},
                    {"id": 2, "name": "P.app.tar.gz", "size": 3},
                    {"id": 3, "name": "SHA256SUMS", "size": 1}]}))
                .into_response()
            }),
        )
        .route(
            "/repos/o/r/releases/assets/{id}",
            axum::routing::get(|State(f): State<Fake>, Path(id): Path<u64>| async move {
                match id {
                    1 => WIN.to_vec().into_response(),
                    2 => MAC.to_vec().into_response(),
                    _ => f.sums.lock().unwrap().clone().into_response(),
                }
            }),
        )
        .with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, fake)
}

async fn setup(tmp: &std::path::Path, supervised: bool) -> (axum::Router, pdt_api::AppState, Fake) {
    let (base, fake) = fake_github().await;
    let store = pdt_store::Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = pdt_api::AppState::new(store, tmp.join("data"));
    state.update = Arc::new(pdt_api::UpdateService::new(&base, "o/r", supervised, "0.1.0-local"));
    (pdt_api::router(state.clone(), None), state, fake)
}

#[tokio::test]
async fn unsupervised_pdtd_never_updates() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), false).await;
    state.secrets.set(pdt_api::secrets::UPDATE_TOKEN, "good").unwrap();
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "disabled");
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("updates_disabled")));
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_token_means_no_network_traffic() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, _, fake) = setup(tmp.path(), true).await;
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!((v["state"].as_str(), v["token"]["state"].as_str()), (Some("idle"), Some("missing")));
    assert_eq!(fake.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn busy_hosts_ask_first_then_the_release_is_handed_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let (s, _) = call(&app, "PUT", "/api/host/token", Some(json!({"token": "good"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "POST", "/api/host/update/check", None).await;
    assert_eq!(v["state"], "available", "{v}");
    assert_eq!(v["latest"]["version"], "0.1.7");
    assert_eq!(v["latest"]["notes"], "notes");

    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
    let (_, p) = call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    state
        .store
        .insert_session(&Session {
            id: "busy".into(),
            project_id: p["id"].as_str().unwrap().into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            title: "Claude · feature".into(),
            cwd: repo.to_string_lossy().into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: Some(Activity::Working),
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
        })
        .await
        .unwrap();
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": false}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("host_busy")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Claude · feature"));
    assert!(!pdt_core::handoff::request_path(&tmp.path().join("data")).exists());

    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "installing");
    let req: pdt_core::handoff::UpdateRequest =
        serde_json::from_slice(&std::fs::read(pdt_core::handoff::request_path(&tmp.path().join("data"))).unwrap()).unwrap();
    assert_eq!((req.version.as_str(), req.from_version.as_str()), ("0.1.7", "0.1.0-local"));
    assert_eq!(std::fs::read(&req.asset).unwrap(), if cfg!(windows) { WIN } else { MAC });
    tokio::time::timeout(std::time::Duration::from_secs(1), state.shutdown.notified()).await.expect("pdtd asked to stop");
}

#[tokio::test]
async fn bad_checksum_hands_nothing_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, fake) = setup(tmp.path(), true).await;
    state.secrets.set(pdt_api::secrets::UPDATE_TOKEN, "good").unwrap();
    *fake.sums.lock().unwrap() = format!("{}  P_0.1.7_x64-setup.exe\n{}  P.app.tar.gz\n", "0".repeat(64), "0".repeat(64));
    call(&app, "POST", "/api/host/update/check", None).await;
    let (s, v) = call(&app, "POST", "/api/host/update/install", Some(json!({"force": true}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("update_failed")), "{v}");
    assert!(!pdt_core::handoff::request_path(&tmp.path().join("data")).exists());
    let (_, v) = call(&app, "GET", "/api/host/update", None).await;
    assert_eq!(v["state"], "available", "can be retried");
}

#[tokio::test]
async fn tokens_are_checked_stored_and_never_echoed() {
    let tmp = tempfile::tempdir().unwrap();
    let (app, state, _) = setup(tmp.path(), true).await;
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": "expired"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_token")));
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap(), None);
    let (s, v) = call(&app, "PUT", "/api/host/token", Some(json!({"token": " good "}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains("good"), "{v}");
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap().as_deref(), Some("good"));
    assert_eq!(call(&app, "DELETE", "/api/host/token", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(state.secrets.get(pdt_api::secrets::UPDATE_TOKEN).unwrap(), None);
}
```

Run: `cargo test -p pdt-api --test update`
Expected: compile errors (no `UpdateService` yet).

- [ ] **Step 5: Implement the update service**

`crates/pdt-api/src/update.rs`:

```rust
//! Update checks and installs (remote-host spec §3.3–3.4). pdtd checks hourly (unless the policy is manual) and on
//! request, downloads and verifies a release, writes `update/request.json` and shuts down; pdt-host installs it.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::secrets::UPDATE_TOKEN;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use pdt_core::activity::Activity;
use pdt_core::handoff::{UpdateRequest, UpdateResult, downloads_dir, request_path, result_path};
use pdt_core::host::UpdatePolicy;
use pdt_core::release::{HostOs, TokenState, parse_version, token_state};
use pdt_core::run::RunState;
use pdt_core::session::SessionStatus;
use pdt_update::{GitHubClient, Release, UpdateError, fetch_verified};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateState {
    Disabled,
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseSummary {
    pub version: String,
    pub tag: String,
    pub notes: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub current: String,
    pub state: UpdateState,
    pub disabled_reason: Option<String>,
    pub policy: UpdatePolicy,
    pub latest: Option<ReleaseSummary>,
    #[ts(type = "number | null")]
    pub last_checked_at: Option<i64>,
    pub error: Option<String>,
    pub token: TokenState,
    pub last_result: Option<UpdateResult>,
}

pub struct UpdateService {
    client: GitHubClient,
    supervised: bool,
    current: String,
    status: Mutex<UpdateStatus>,
    release: Mutex<Option<Release>>,
    running: tokio::sync::Mutex<()>,
}

impl UpdateService {
    pub fn new(api_base: &str, repo: &str, supervised: bool, current: &str) -> Self {
        Self {
            client: GitHubClient::new(api_base, repo),
            supervised,
            current: current.into(),
            status: Mutex::new(UpdateStatus {
                current: current.into(),
                state: if supervised { UpdateState::Idle } else { UpdateState::Disabled },
                disabled_reason: (!supervised)
                    .then(|| "updates are installed by pdt-host, and this pdtd is not running under it".into()),
                policy: UpdatePolicy::Notify,
                latest: None,
                last_checked_at: None,
                error: None,
                token: TokenState::Missing,
                last_result: None,
            }),
            release: Mutex::new(None),
            running: tokio::sync::Mutex::new(()),
        }
    }

    pub fn set_last_result(&self, r: Option<UpdateResult>) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).last_result = r;
    }

    fn snapshot(&self) -> UpdateStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn edit(&self, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        s.clone()
    }
}

async fn token(st: &AppState) -> Result<Option<String>, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.get(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

async fn refresh_policy(st: &AppState) -> UpdatePolicy {
    let p = st.store.get_host_settings().await.map(|s| s.update_policy).unwrap_or(UpdatePolicy::Notify);
    st.update.edit(|s| s.policy = p);
    p
}

/// Asks GitHub for the latest release (only with a token). Never fails: problems land in `error` / `token`.
pub(crate) async fn check(st: &AppState) -> UpdateStatus {
    let svc = &st.update;
    refresh_policy(st).await;
    if !svc.supervised {
        return svc.snapshot();
    }
    let tok = match token(st).await {
        Ok(Some(t)) => t,
        Ok(None) => return svc.edit(|s| { s.token = TokenState::Missing; s.state = UpdateState::Idle; s.error = None; }),
        Err(e) => return svc.edit(|s| s.error = Some(e.message().to_string())),
    };
    svc.edit(|s| s.state = UpdateState::Checking);
    let result = svc.client.latest(&tok).await;
    let now = now_ms();
    match result {
        Ok(l) => {
            let newer = parse_version(&svc.current).is_none_or(|c| l.release.version.is_newer_than(&c));
            let summary = ReleaseSummary {
                version: l.release.version.to_string(),
                tag: l.release.tag.clone(),
                notes: l.release.notes.clone(),
                published_at: l.release.published_at.clone(),
            };
            *svc.release.lock().unwrap_or_else(|e| e.into_inner()) = newer.then(|| l.release.clone());
            svc.edit(|s| {
                s.token = token_state(true, l.token_expires_at, false, now);
                s.last_checked_at = Some(now);
                s.error = None;
                s.latest = newer.then_some(summary);
                s.state = if newer { UpdateState::Available } else { UpdateState::Idle };
            })
        }
        Err(e) => svc.edit(|s| {
            if e == UpdateError::Unauthorized {
                s.token = TokenState::Expired;
            }
            s.last_checked_at = Some(now);
            s.error = Some(e.to_string());
            s.state = if s.latest.is_some() { UpdateState::Available } else { UpdateState::Idle };
        }),
    }
}

/// What makes the host busy (spec §3.3): agents working or of unknown activity, runs setting up.
pub(crate) async fn busy_reasons(st: &AppState) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(sessions) = st.store.list_sessions_by_status(SessionStatus::Running).await {
        for s in sessions {
            match s.activity {
                Some(Activity::Working) => out.push(format!("{} is working", s.title)),
                Some(Activity::Unknown) => out.push(format!("{} may be working (activity unknown)", s.title)),
                _ => {}
            }
        }
    }
    if let Ok(runs) = st.store.list_active_runs().await {
        out.extend(runs.into_iter().filter(|r| r.state == RunState::SettingUp).map(|r| format!("run {} · {} is setting up", r.name, r.branch)));
    }
    out
}

fn conflict(code: &'static str, msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, msg)
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Downloads and verifies the available release, hands it to pdt-host and asks pdtd to stop.
pub(crate) async fn install(st: &AppState, force: bool) -> Result<UpdateStatus, ApiError> {
    let svc = &st.update;
    if !svc.supervised {
        return Err(conflict("updates_disabled", svc.snapshot().disabled_reason.unwrap_or_default()));
    }
    let _guard = svc.running.try_lock().map_err(|_| conflict("update_in_progress", "an update is already in progress"))?;
    let release = svc
        .release
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| conflict("no_update", "no update is available; check for updates first"))?;
    if !force {
        let busy = busy_reasons(st).await;
        if !busy.is_empty() {
            return Err(conflict(
                "host_busy",
                format!("Busy: {}. Update anyway? Sessions resume afterwards.", busy.join("; ")),
            ));
        }
    }
    let tok = token(st).await?.ok_or_else(|| conflict("no_token", "no update token is stored"))?;
    svc.edit(|s| { s.state = UpdateState::Downloading; s.error = None; });
    let dir = downloads_dir(&st.data_dir).join(&release.tag);
    let asset = match fetch_verified(&svc.client, &tok, &release, HostOs::current(), &dir).await {
        Ok(p) => p,
        Err(e) => {
            svc.edit(|s| { s.state = UpdateState::Available; s.error = Some(e.to_string()); });
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, "update_failed", e.to_string()));
        }
    };
    let req = UpdateRequest {
        version: release.version.to_string(),
        tag: release.tag.clone(),
        asset,
        from_version: svc.current.clone(),
        requested_at: now_ms(),
    };
    write_atomic(&request_path(&st.data_dir), &serde_json::to_vec_pretty(&req).expect("json")).map_err(ApiError::internal)?;
    tracing::info!(version = %req.version, "update downloaded and verified; handing over to pdt-host");
    let status = svc.edit(|s| s.state = UpdateState::Installing);
    st.shutdown.notify_one();
    Ok(status)
}

/// Hourly checks, and the auto-when-idle install (spec §3.3). Runs only under pdt-host.
pub async fn run_update_loop(st: AppState) {
    if !st.update.supervised {
        return;
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let policy = refresh_policy(&st).await;
        if policy != UpdatePolicy::Manual {
            let s = check(&st).await;
            if s.state == UpdateState::Available && policy == UpdatePolicy::AutoWhenIdle && busy_reasons(&st).await.is_empty() {
                if let Err(e) = install(&st, false).await {
                    tracing::warn!(error = ?e, "automatic update did not start");
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

pub(crate) async fn get_status(State(st): State<AppState>) -> Json<UpdateStatus> {
    refresh_policy(&st).await;
    Json(st.update.snapshot())
}

pub(crate) async fn post_check(State(st): State<AppState>) -> Json<UpdateStatus> {
    Json(check(&st).await)
}

#[derive(Deserialize)]
pub(crate) struct InstallBody {
    #[serde(default)]
    force: bool,
}

pub(crate) async fn post_install(
    State(st): State<AppState>,
    payload: Result<Json<InstallBody>, JsonRejection>,
) -> Result<Json<UpdateStatus>, ApiError> {
    let Json(b) = payload?;
    Ok(Json(install(&st, b.force).await?))
}

pub(crate) async fn post_dismiss(State(st): State<AppState>) -> StatusCode {
    let _ = std::fs::remove_file(result_path(&st.data_dir));
    st.update.set_last_result(None);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub(crate) struct TokenBody {
    token: String,
}

/// Stores the token after GitHub accepts it (a 401 is refused; other problems are reported but the token is kept).
pub(crate) async fn put_token(
    State(st): State<AppState>,
    payload: Result<Json<TokenBody>, JsonRejection>,
) -> Result<Json<UpdateStatus>, ApiError> {
    let Json(b) = payload?;
    let tok = b.token.trim().to_string();
    if tok.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", "the token is empty"));
    }
    if st.update.client.latest(&tok).await == Err(UpdateError::Unauthorized) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", "GitHub rejected this token"));
    }
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.set(UPDATE_TOKEN, &tok))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(check(&st).await))
}

pub(crate) async fn delete_token(State(st): State<AppState>) -> Result<StatusCode, ApiError> {
    let secrets = st.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.delete(UPDATE_TOKEN))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    *st.update.release.lock().unwrap_or_else(|e| e.into_inner()) = None;
    st.update.edit(|s| { s.token = TokenState::Missing; s.latest = None; s.state = if st.update.supervised { UpdateState::Idle } else { UpdateState::Disabled }; });
    Ok(StatusCode::NO_CONTENT)
}
```

Wire it:
- `AppState` gets `pub update: std::sync::Arc<crate::update::UpdateService>`. It is initialised in `new` with `UpdateService::new(pdt_update::GITHUB_API, pdt_core::brand::RELEASE_REPO, false, pdt_core::BUILD_VERSION)`.
- `lib.rs` gets `pub mod update; pub use update::{UpdateService, run_update_loop};`.
- `ApiError` already has `message()` (Task 7). Mark the fields of `UpdateService` used outside the module (`client`, `release`, `supervised`) `pub(crate)`.
- Routes:

```rust
        .route("/host/update", get(crate::update::get_status))
        .route("/host/update/check", axum::routing::post(crate::update::post_check))
        .route("/host/update/install", axum::routing::post(crate::update::post_install))
        .route("/host/update/dismiss", axum::routing::post(crate::update::post_dismiss))
        .route("/host/token", axum::routing::put(crate::update::put_token).delete(crate::update::delete_token))
```

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 6: `pdtd` wiring**

In `config.rs`, add `pub supervised: bool` (`lookup("PDT_SUPERVISED").as_deref() == Some("1")`) and a test. In `main.rs`, after the state is built:

```rust
    state.update = std::sync::Arc::new(pdt_api::UpdateService::new(
        pdt_update::GITHUB_API,
        pdt_core::brand::RELEASE_REPO,
        cfg.supervised,
        pdt_core::BUILD_VERSION,
    ));
    if let Ok(bytes) = std::fs::read(pdt_core::handoff::result_path(&cfg.data_dir))
        && let Ok(r) = serde_json::from_slice::<pdt_core::handoff::UpdateResult>(&bytes)
    {
        tracing::info!(outcome = ?r.outcome, to = %r.to_version, "last update result");
        state.update.set_last_result(Some(r));
    }
```

Run `cargo add pdt-update serde_json -p pdtd`. After serving starts, add `tokio::spawn(pdt_api::run_update_loop(state.clone()));`.

Update the `RELEASE_REPO` doc comment in `brand.rs` to: `/// GitHub repository (owner/name) whose releases hosts update from, through the GitHub REST API and a token.`

Run: `cargo test -p pdtd`
Expected: PASS.

- [ ] **Step 7: CI publishes `SHA256SUMS` with space-free names**

In `.github/workflows/ci.yml`, in the `release` job, add this step between `download-artifact` and "Create the release":

```yaml
      # GitHub renames uploaded assets with spaces; rename first so SHA256SUMS lists the names hosts will see.
      - name: Checksums
        run: |
          cd assets
          for f in *; do n="${f// /.}"; if [ "$f" != "$n" ]; then mv "$f" "$n"; fi; done
          sha256sum * > SHA256SUMS
          cat SHA256SUMS
```

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock crates .github/workflows/ci.yml web/src/api/gen
git commit -m "feat: token-based update checks, verified downloads and the pdt-host hand-off"
```

---

### Task 12: `pdt-host` supervisor: restart, install, rollback, self-replacement

**Files:**
- Create:
  - `crates/pdt-host/Cargo.toml`
  - `crates/pdt-host/src/{lib,config,health,supervise,install,selfupdate,log}.rs`
  - `crates/pdt-host/src/main.rs`
  - `crates/pdt-host/src/bin/fake-pdtd.rs`
  - `crates/pdt-host/tests/supervise.rs`
- Modify:
  - `Cargo.toml` (workspace dependency and `default-members`)
  - `crates/pdt-core/src/brand.rs` (`HOST_BIN`)

**Interfaces:**
- Consumes: `UpdateRequest`, `UpdateResult`, `UpdateOutcome`, `request_path` and `result_path` (Task 11). pdtd honours `PDT_SUPERVISED=1` (Task 11).
- Produces (`pdt_host`):
  - `config::HostConfig { install_dir, data_dir, port, dev, health_timeout, backoff, poll }`, with `HostConfig::from_env(install_dir)` and `pdtd_path()`.
  - `supervise::Backoff { initial, max, reset_after }` with `delay(failures)`; `supervise::run(&HostConfig, &dyn Installer, &AtomicBool, after_install: &dyn Fn())`; `supervise::take_request(data_dir)`; `supervise::spawn_pdtd(&HostConfig)`.
  - `health::{Probe { Free, Ours { version }, Foreign(String) }, probe(port), classify_health(&[u8]), request_shutdown(port)}`.
  - `install::Installer` (trait: `install(asset)`, `rollback()`), with `CopyDirInstaller`, `NsisInstaller`, `BundleInstaller`, `nsis_args(dir)` and `installer_for(&HostConfig)`.
  - `selfupdate::{needs_replace(running, bundled), replace(running, bundled), clean_old(running), old_path(running)}`.
- Produces (`pdt_core::brand`): `HOST_BIN = "pdt-host"`.
- Produces (binary): `pdt-host run [--install-dir <dir>]`. Task 13 adds `install` and `uninstall`.

- [ ] **Step 1: Create the crate**

`crates/pdt-host/Cargo.toml`:

```toml
[package]
name = "pdt-host"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "pdt-host"
path = "src/main.rs"
```

Run (PowerShell):
- `cargo add pdt-core serde_json sha2 hex dirs anyhow jiff -p pdt-host`
- `cargo add tempfile --dev -p pdt-host`

Root `Cargo.toml`:
- add `"crates/pdt-host"` to `default-members`;
- add `pdt-host = { path = "crates/pdt-host" }` under `[workspace.dependencies]`.

Add `pub const HOST_BIN: &str = "pdt-host";` to `brand.rs`, with the doc comment `/// The supervisor that runs on each host: starts pdtd, installs updates, rolls back (remote-host spec §3.1).` Also extend the module doc's list of rename touch points with "and HOST_* below".

`src/lib.rs`:

```rust
//! pdt-host: keeps pdtd running on a host, installs updates pdtd hands it, rolls back a bad one (remote-host spec §3).
//! Deliberately small and synchronous, so it rarely needs to change itself.

pub mod config;
pub mod health;
pub mod install;
pub mod log;
pub mod selfupdate;
pub mod supervise;
```

- [ ] **Step 2: Configuration, health probe and log (TDD)**

`src/config.rs`:

```rust
use pdt_core::brand::{DAEMON_BIN, Profile};
use std::path::PathBuf;
use std::time::Duration;

use crate::supervise::Backoff;

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Where the installed app lives: pdtd (and the bundled pdt-host) are here.
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub dev: bool,
    /// How long a (new) pdtd gets to answer /api/health (spec §3.4: 60 s).
    pub health_timeout: Duration,
    pub backoff: Backoff,
    /// How often a running pdtd is checked.
    pub poll: Duration,
}

impl HostConfig {
    /// The same rules as pdtd's config (`PDT_PROFILE`, `PDT_PORT`, `PDT_DATA_DIR`), so both agree on the port and data dir.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        default_data_root: Option<PathBuf>,
        install_dir: PathBuf,
    ) -> Result<HostConfig, String> {
        let lookup = |k: &str| lookup(k).filter(|v| !v.trim().is_empty());
        let profile = Profile::from_env_value(lookup("PDT_PROFILE").as_deref());
        let port = match lookup("PDT_PORT") {
            Some(p) => p.trim().parse().map_err(|_| format!("PDT_PORT is not a valid port: {p:?}"))?,
            None => profile.port(),
        };
        let data_dir = match lookup("PDT_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => default_data_root.ok_or("could not determine a data directory; set PDT_DATA_DIR")?.join(profile.data_dir_name()),
        };
        Ok(HostConfig {
            install_dir,
            data_dir,
            port,
            dev: profile == Profile::Dev,
            health_timeout: Duration::from_secs(60),
            backoff: Backoff::default(),
            poll: Duration::from_secs(2),
        })
    }

    pub fn from_env(install_dir: PathBuf) -> Result<HostConfig, String> {
        Self::from_lookup(|k| std::env::var(k).ok(), dirs::data_local_dir(), install_dir)
    }

    pub fn pdtd_path(&self) -> PathBuf {
        self.install_dir.join(format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn matches_pdtd_defaults() {
        let c = HostConfig::from_lookup(lookup(&[]), Some("/d".into()), "/app".into()).unwrap();
        assert_eq!((c.port, c.data_dir.clone(), c.dev), (7420, PathBuf::from("/d").join("pdt"), false));
        let c = HostConfig::from_lookup(lookup(&[("PDT_PROFILE", "dev")]), Some("/d".into()), "/app".into()).unwrap();
        assert_eq!((c.port, c.dev), (7421, true));
        let c = HostConfig::from_lookup(lookup(&[("PDT_PORT", "9000"), ("PDT_DATA_DIR", "/x")]), None, "/app".into()).unwrap();
        assert_eq!((c.port, c.data_dir), (9000, PathBuf::from("/x")));
        assert!(HostConfig::from_lookup(lookup(&[("PDT_PORT", "x")]), Some("/d".into()), "/app".into()).is_err());
        assert!(c.pdtd_path().starts_with("/app"));
    }
}
```

`src/health.rs`. This is the desktop's probe, moved here so the tray can reuse it (Task 13) and extended to read the version:

```rust
use pdt_core::brand::PRODUCT_NAME;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Free,
    Ours { version: String },
    Foreign(String),
}

pub fn probe(port: u16) -> Probe {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return Probe::Free };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    if let Err(e) = s.write_all(format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()) {
        return Probe::Foreign(e.to_string());
    }
    let mut buf = Vec::new();
    if let Err(e) = s.take(64 * 1024).read_to_end(&mut buf) {
        return Probe::Foreign(e.to_string());
    }
    classify_health(&buf)
}

pub fn classify_health(raw: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Probe::Foreign("it did not answer like a web server".into());
    };
    let status = head.lines().next().unwrap_or("");
    if !status.starts_with("HTTP/1.") || status.split_whitespace().nth(1) != Some("200") {
        return Probe::Foreign(format!("its health check answered {status:?}"));
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v["name"] == PRODUCT_NAME => Probe::Ours { version: v["version"].as_str().unwrap_or("").to_string() },
        Ok(v) => Probe::Foreign(format!("it identifies as {}", v["name"])),
        Err(_) => Probe::Foreign("its health check is not JSON".into()),
    }
}

/// POST /api/shutdown (callers check the port is ours first).
pub fn request_shutdown(port: u16) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(format!("POST /api/shutdown HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.take(64 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next().unwrap_or("");
    match line.split_whitespace().nth(1) {
        Some(c) if c.starts_with('2') => Ok(()),
        _ => Err(format!("its shutdown request answered {line:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_health_and_reads_the_version() {
        let ok = b"HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\",\"name\":\"Personal Dev Team\",\"version\":\"0.1.7\"}";
        assert_eq!(classify_health(ok), Probe::Ours { version: "0.1.7".into() });
        assert!(matches!(classify_health(b"HTTP/1.1 500 X\r\n\r\n"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"HTTP/1.1 200 OK\r\n\r\n{\"name\":\"x\"}"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"junk"), Probe::Foreign(_)));
    }

    #[test]
    fn a_closed_port_is_free() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(probe(port), Probe::Free);
    }
}
```

`src/log.rs`:

```rust
use crate::config::HostConfig;
use std::io::Write;
use std::path::PathBuf;

pub fn log_path(cfg: &HostConfig) -> PathBuf {
    cfg.data_dir.join("logs").join(format!("{}.log", pdt_core::brand::HOST_BIN))
}

/// Appends a timestamped line to `<data>/logs/pdt-host.log` and stderr. Never fails.
pub fn log(cfg: &HostConfig, msg: &str) {
    let line = format!("{} {msg}", jiff::Timestamp::now());
    eprintln!("{line}");
    let path = log_path(cfg);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::File::options().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
```

Run: `cargo test -p pdt-host --lib`
Expected: these compile once Steps 3–5 add the other modules. Write Steps 3–5 first, then run.

- [ ] **Step 3: Installers (TDD for the pure parts)**

`src/install.rs`:

```rust
//! Installing a release and rolling it back (spec §3.4 steps 3–4): the silent NSIS installer on Windows, a bundle swap
//! on macOS. The previous version is kept: a copy of the install folder (Windows) or the old bundle (macOS).

use crate::config::HostConfig;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub trait Installer {
    fn install(&self, asset: &Path) -> Result<(), String>;
    fn rollback(&self) -> Result<(), String>;
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

/// Makes `dst` an exact copy of `src`.
pub fn replace_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        for e in std::fs::read_dir(dst)? {
            let p = e?.path();
            if p.is_dir() { std::fs::remove_dir_all(&p)? } else { std::fs::remove_file(&p)? }
        }
    }
    copy_dir(src, dst)
}

/// Tests (and the basis of the Windows backup): the asset is a folder whose contents are copied over `target`.
pub struct CopyDirInstaller {
    pub target: PathBuf,
    pub backup: PathBuf,
}

impl Installer for CopyDirInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        replace_dir(&self.target, &self.backup).map_err(|e| format!("backing up {}: {e}", self.target.display()))?;
        copy_dir(asset, &self.target).map_err(|e| format!("copying the new version: {e}"))
    }
    fn rollback(&self) -> Result<(), String> {
        replace_dir(&self.backup, &self.target).map_err(|e| format!("restoring the previous version: {e}"))
    }
}

/// NSIS: `/S` silent, `/D=` the install folder (must be last and unquoted).
pub fn nsis_args(install_dir: &Path) -> Vec<String> {
    vec!["/S".into(), format!("/D={}", install_dir.display())]
}

pub struct NsisInstaller {
    pub install_dir: PathBuf,
    pub backup: PathBuf,
}

impl Installer for NsisInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        let tray = stop_tray(&self.install_dir);
        replace_dir(&self.install_dir, &self.backup).map_err(|e| format!("backing up the install folder: {e}"))?;
        let status = Command::new(asset)
            .args(nsis_args(&self.install_dir))
            .stdin(Stdio::null())
            .status()
            .map_err(|e| format!("running the installer: {e}"))?;
        if tray {
            start_tray(&self.install_dir);
        }
        if status.success() { Ok(()) } else { Err(format!("the installer failed ({status})")) }
    }
    fn rollback(&self) -> Result<(), String> {
        let tray = stop_tray(&self.install_dir);
        let r = replace_dir(&self.backup, &self.install_dir).map_err(|e| format!("restoring the previous version: {e}"));
        if tray {
            start_tray(&self.install_dir);
        }
        r
    }
}

/// The bundle that contains `exe_dir` (`…/X.app/Contents/MacOS` → `…/X.app`).
pub fn bundle_of(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

pub fn previous_bundle(bundle: &Path) -> PathBuf {
    let mut s = bundle.as_os_str().to_owned();
    s.push(".previous");
    PathBuf::from(s)
}

pub struct BundleInstaller {
    pub bundle: PathBuf,
}

impl Installer for BundleInstaller {
    fn install(&self, asset: &Path) -> Result<(), String> {
        let parent = self.bundle.parent().ok_or("the bundle has no parent folder")?;
        let staging = parent.join(".pdt-update-staging");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let ok = Command::new("tar").arg("-xzf").arg(asset).arg("-C").arg(&staging).status().map_err(|e| e.to_string())?;
        if !ok.success() {
            return Err("could not extract the update".into());
        }
        let new = std::fs::read_dir(&staging)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or("the archive has no .app bundle")?;
        let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&new).status();
        let tray = stop_tray(&self.bundle);
        let prev = previous_bundle(&self.bundle);
        let _ = std::fs::remove_dir_all(&prev);
        std::fs::rename(&self.bundle, &prev).map_err(|e| format!("moving the old bundle aside: {e}"))?;
        if let Err(e) = std::fs::rename(&new, &self.bundle) {
            let _ = std::fs::rename(&prev, &self.bundle);
            return Err(format!("moving the new bundle into place: {e}"));
        }
        let _ = std::fs::remove_dir_all(&staging);
        if tray {
            start_tray(&self.bundle);
        }
        Ok(())
    }
    fn rollback(&self) -> Result<(), String> {
        let prev = previous_bundle(&self.bundle);
        if !prev.exists() {
            return Err("there is no previous bundle to restore".into());
        }
        let tray = stop_tray(&self.bundle);
        std::fs::remove_dir_all(&self.bundle).map_err(|e| e.to_string())?;
        std::fs::rename(&prev, &self.bundle).map_err(|e| e.to_string())?;
        if tray {
            start_tray(&self.bundle);
        }
        Ok(())
    }
}

pub fn installer_for(cfg: &HostConfig) -> Result<Box<dyn Installer>, String> {
    if cfg!(windows) {
        Ok(Box::new(NsisInstaller {
            install_dir: cfg.install_dir.clone(),
            backup: pdt_core::handoff::update_dir(&cfg.data_dir).join("previous"),
        }))
    } else if cfg!(target_os = "macos") {
        let bundle = bundle_of(&cfg.install_dir).ok_or("pdt-host is not installed from an .app bundle")?;
        Ok(Box::new(BundleInstaller { bundle }))
    } else {
        Err("updates are not supported on this platform".into())
    }
}

/// Stops the tray app running from `place` (it locks files on Windows and runs stale code on macOS). pdtd has already
/// exited and pdt-host runs from the data folder, so everything else there is the tray. Returns whether one ran.
fn stop_tray(place: &Path) -> bool {
    let daemon = pdt_core::brand::DAEMON_BIN;
    if cfg!(windows) {
        let script = format!(
            "$p = Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{}\\', [StringComparison]::OrdinalIgnoreCase) -and $_.ProcessName -notin @('{daemon}', 'uninstall') }}; if ($p) {{ $p | Stop-Process -Force; 'yes' }}",
            place.display().to_string().replace('\'', "''")
        );
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("yes"))
            .unwrap_or(false)
    } else {
        let pattern = format!("{}/Contents/MacOS/", place.display());
        let found = Command::new("pgrep").args(["-f", &pattern]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false);
        if found {
            let _ = Command::new("pkill").args(["-f", &pattern]).status();
        }
        found
    }
}

fn start_tray(place: &Path) {
    if cfg!(windows) {
        let daemon = pdt_core::brand::DAEMON_BIN;
        let host = pdt_core::brand::HOST_BIN;
        let app = std::fs::read_dir(place).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).find(|p| {
                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                    && p.file_stem().is_some_and(|s| ![daemon, host, "uninstall"].iter().any(|n| s.eq_ignore_ascii_case(n)))
            })
        });
        if let Some(app) = app {
            let mut cmd = Command::new(app);
            for (k, _) in std::env::vars() {
                if k.starts_with("PDT_") {
                    cmd.env_remove(k);
                }
            }
            let _ = cmd.spawn();
        }
    } else {
        let _ = Command::new("open").arg(place).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsis_command_line() {
        let d = Path::new(r"C:\Users\me\AppData\Local\Personal Dev Team");
        assert_eq!(nsis_args(d), ["/S", r"/D=C:\Users\me\AppData\Local\Personal Dev Team"]);
    }

    #[test]
    fn bundle_paths() {
        let exe_dir = Path::new("/Users/me/Applications/Personal Dev Team.app/Contents/MacOS");
        let b = bundle_of(exe_dir).unwrap();
        assert_eq!(b, Path::new("/Users/me/Applications/Personal Dev Team.app"));
        assert_eq!(previous_bundle(&b), Path::new("/Users/me/Applications/Personal Dev Team.app.previous"));
        assert_eq!(bundle_of(Path::new("/usr/local/bin")), None);
    }

    #[test]
    fn copy_installer_backs_up_and_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let (target, backup, asset) = (tmp.path().join("t"), tmp.path().join("b"), tmp.path().join("a"));
        std::fs::create_dir_all(target.join("sub")).unwrap();
        std::fs::write(target.join("v.txt"), "old").unwrap();
        std::fs::write(target.join("sub").join("x"), "1").unwrap();
        std::fs::create_dir_all(&asset).unwrap();
        std::fs::write(asset.join("v.txt"), "new").unwrap();
        let i = CopyDirInstaller { target: target.clone(), backup };
        i.install(&asset).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "new");
        i.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(target.join("v.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(target.join("sub").join("x")).unwrap(), "1");
    }
}
```

- [ ] **Step 4: Self-replacement**

`src/selfupdate.rs`:

```rust
//! Spec §3.4 step 6: when a release changes pdt-host itself, it replaces its own copy last and restarts.

use sha2::Digest;
use std::path::{Path, PathBuf};

fn sha(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(hex::encode(h.finalize()))
}

/// The bundled copy differs from the one running.
pub fn needs_replace(running: &Path, bundled: &Path) -> bool {
    match (sha(running), sha(bundled)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

pub fn old_path(running: &Path) -> PathBuf {
    running.with_extension(if cfg!(windows) { "old.exe" } else { "old" })
}

/// Swaps in `bundled` at `running`'s path. Windows can rename a running exe but not overwrite it; unix can rename over it.
pub fn replace(running: &Path, bundled: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        let old = old_path(running);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(running, &old)?;
        if let Err(e) = std::fs::copy(bundled, running) {
            let _ = std::fs::rename(&old, running);
            return Err(e);
        }
        Ok(())
    } else {
        let new = running.with_extension("new");
        std::fs::copy(bundled, &new)?;
        std::fs::rename(&new, running)
    }
}

/// Removes the previous copy left by `replace` (best effort; called at startup).
pub fn clean_old(running: &Path) {
    let _ = std::fs::remove_file(old_path(running));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_when_different() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("host.bin"), tmp.path().join("bundled.bin"));
        std::fs::write(&a, "v1").unwrap();
        std::fs::write(&b, "v1").unwrap();
        assert!(!needs_replace(&a, &b));
        std::fs::write(&b, "v2").unwrap();
        assert!(needs_replace(&a, &b));
        replace(&a, &b).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v2");
        clean_old(&a);
        assert!(!old_path(&a).exists());
        assert!(!needs_replace(&a, &tmp.path().join("missing")));
    }
}
```

- [ ] **Step 5: The supervisor loop**

`src/supervise.rs`:

```rust
//! The supervision loop (spec §3.1, §3.4): start pdtd (or adopt one that is running), restart it with backoff when it
//! exits, and install the update it hands over, rolling back when the new version fails its health check.

use crate::config::HostConfig;
use crate::health::{Probe, probe};
use crate::install::Installer;
use crate::log::{log, log_path};
use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, request_path, result_path};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    /// A pdtd that ran this long resets the failure count.
    pub reset_after: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { initial: Duration::from_secs(1), max: Duration::from_secs(60), reset_after: Duration::from_secs(300) }
    }
}

impl Backoff {
    /// 0 after no failures; then initial, 2×, 4×… capped at `max`.
    pub fn delay(&self, failures: u32) -> Duration {
        if failures == 0 {
            return Duration::ZERO;
        }
        self.initial.saturating_mul(1u32 << (failures - 1).min(16)).min(self.max)
    }
}

/// Takes the pending update request, renaming the file so a crash loop never installs it twice.
pub fn take_request(data_dir: &Path) -> Option<UpdateRequest> {
    let path = request_path(data_dir);
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::rename(&path, path.with_extension("taken.json"));
    serde_json::from_slice(&bytes).ok()
}

pub fn spawn_pdtd(cfg: &HostConfig) -> std::io::Result<Child> {
    let launch_log = cfg.data_dir.join("logs").join(format!("{}-launch.log", pdt_core::brand::DAEMON_BIN));
    let _ = std::fs::create_dir_all(launch_log.parent().expect("logs dir"));
    let out = std::fs::File::options().create(true).append(true).open(&launch_log)?;
    let mut cmd = Command::new(cfg.pdtd_path());
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("PDT_PORT", cfg.port.to_string())
        .env("PDT_DATA_DIR", &cfg.data_dir)
        .env("PDT_SUPERVISED", "1")
        .current_dir(&cfg.install_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out.try_clone()?))
        .stderr(Stdio::from(out));
    if cfg.dev {
        cmd.env("PDT_PROFILE", "dev");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    }
    #[cfg(unix)]
    {
        // Own process group: pdtd survives pdt-host restarting itself (launchd kills the job's process group).
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
}

fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50).min(end - Instant::now()));
    }
}

/// Waits for our pdtd to answer health (with `version` when given). Fails early if the child exits.
fn wait_healthy(cfg: &HostConfig, child: &mut Option<Child>, version: Option<&str>, stop: &AtomicBool) -> bool {
    let end = Instant::now() + cfg.health_timeout;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        if let Probe::Ours { version: v } = probe(cfg.port)
            && version.is_none_or(|want| want == v)
        {
            return true;
        }
        if let Some(c) = child
            && c.try_wait().ok().flatten().is_some()
        {
            *child = None;
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn kill(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Returns once pdtd has gone: our child exited, or (adopted) it stopped answering three times in a row.
fn watch(cfg: &HostConfig, child: &mut Option<Child>, stop: &AtomicBool) {
    let mut misses = 0;
    while !stop.load(Ordering::SeqCst) {
        if let Some(c) = child {
            if c.try_wait().ok().flatten().is_some() {
                *child = None;
                return;
            }
        } else if matches!(probe(cfg.port), Probe::Ours { .. }) {
            misses = 0;
        } else {
            misses += 1;
            if misses >= 3 {
                return;
            }
        }
        sleep_unless_stopped(cfg.poll, stop);
    }
}

fn write_result(cfg: &HostConfig, r: &UpdateResult) {
    let path = result_path(&cfg.data_dir);
    let _ = std::fs::create_dir_all(path.parent().expect("update dir"));
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(r).expect("json"));
}

fn handle_update(cfg: &HostConfig, installer: &dyn Installer, child: &mut Option<Child>, req: UpdateRequest, stop: &AtomicBool) -> UpdateOutcome {
    // pdtd shut itself down after writing the request; make sure it is gone before files are replaced.
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let exited = child.as_mut().is_none_or(|c| c.try_wait().ok().flatten().is_some());
        if exited && probe(cfg.port) == Probe::Free {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    kill(child);
    log(cfg, &format!("installing {} (from {})", req.version, req.from_version));
    let result = |outcome, message: String| UpdateResult {
        outcome,
        from_version: req.from_version.clone(),
        to_version: req.version.clone(),
        message,
        log: Some(log_path(cfg).to_string_lossy().into_owned()),
        at: jiff::Timestamp::now().as_millisecond(),
    };
    let r = match installer.install(&req.asset) {
        Err(e) => {
            log(cfg, &format!("install failed: {e}"));
            result(UpdateOutcome::Failed, format!("Update to v{} failed: {e}", req.version))
        }
        Ok(()) => {
            *child = spawn_pdtd(cfg).map_err(|e| log(cfg, &format!("could not start the new pdtd: {e}"))).ok();
            if wait_healthy(cfg, child, Some(&req.version), stop) {
                log(cfg, &format!("v{} is healthy", req.version));
                result(UpdateOutcome::Installed, format!("Updated to v{}", req.version))
            } else {
                log(cfg, &format!("v{} failed its health check; rolling back", req.version));
                kill(child);
                match installer.rollback() {
                    Ok(()) => result(
                        UpdateOutcome::RolledBack,
                        format!("Update to v{} failed and was rolled back", req.version),
                    ),
                    Err(e) => {
                        log(cfg, &format!("rollback failed: {e}"));
                        result(UpdateOutcome::Failed, format!("Update to v{} failed, and so did the rollback: {e}", req.version))
                    }
                }
            }
        }
    };
    write_result(cfg, &r);
    r.outcome
}

/// Runs until `stop` is set (tests); in production it runs for the life of the process. `after_install` is called
/// after a successful install (pdt-host's main uses it to replace itself last).
pub fn run(cfg: &HostConfig, installer: &dyn Installer, stop: &AtomicBool, after_install: &dyn Fn()) {
    let mut failures = 0u32;
    let mut child: Option<Child> = None;
    while !stop.load(Ordering::SeqCst) {
        if let Some(req) = take_request(&cfg.data_dir) {
            if handle_update(cfg, installer, &mut child, req, stop) == UpdateOutcome::Installed {
                after_install();
            }
            failures = 0;
            if child.is_none() {
                continue;
            }
        } else {
            match probe(cfg.port) {
                Probe::Ours { .. } => {}
                Probe::Foreign(why) => {
                    failures += 1;
                    log(cfg, &format!("port {} is held by something else ({why})", cfg.port));
                    sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                    continue;
                }
                Probe::Free => {
                    child = match spawn_pdtd(cfg) {
                        Ok(c) => Some(c),
                        Err(e) => {
                            failures += 1;
                            log(cfg, &format!("could not start pdtd: {e}"));
                            sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                            continue;
                        }
                    };
                    if !wait_healthy(cfg, &mut child, None, stop) {
                        kill(&mut child);
                        failures += 1;
                        log(cfg, "pdtd did not become healthy");
                        sleep_unless_stopped(cfg.backoff.delay(failures), stop);
                        continue;
                    }
                }
            }
        }
        let started = Instant::now();
        watch(cfg, &mut child, stop);
        if stop.load(Ordering::SeqCst) || request_path(&cfg.data_dir).exists() {
            continue;
        }
        if started.elapsed() >= cfg.backoff.reset_after {
            failures = 0;
        }
        failures += 1;
        let d = cfg.backoff.delay(failures);
        log(cfg, &format!("pdtd exited; restarting in {d:?}"));
        sleep_unless_stopped(d, stop);
    }
    kill(&mut child);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let b = Backoff { initial: Duration::from_secs(1), max: Duration::from_secs(60), reset_after: Duration::ZERO };
        let d: Vec<u64> = (0..9).map(|f| b.delay(f).as_secs()).collect();
        assert_eq!(d, [0, 1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(b.delay(1000), Duration::from_secs(60));
    }

    #[test]
    fn a_request_is_taken_once() {
        let tmp = tempfile::tempdir().unwrap();
        let req = UpdateRequest {
            version: "0.1.7".into(),
            tag: "v0.1.7".into(),
            asset: "a".into(),
            from_version: "0.1.6".into(),
            requested_at: 1,
        };
        let p = request_path(tmp.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::to_vec(&req).unwrap()).unwrap();
        assert_eq!(take_request(tmp.path()), Some(req));
        assert_eq!(take_request(tmp.path()), None);
    }
}
```

Tests pass `&|| {}` as `after_install`.

- [ ] **Step 6: The `fake-pdtd` binary and the integration tests**

`src/bin/fake-pdtd.rs`:

```rust
//! A stand-in pdtd for pdt-host's tests. Reads `fake.json` next to its own exe: {"version": "...", "mode": "healthy" |
//! "crash" | "crash_once" | "unhealthy"}. Serves /api/health on $PDT_PORT, exits on POST /api/shutdown, and records
//! $PDT_SUPERVISED in `env.txt` beside itself.
use std::io::{Read, Write};

fn main() {
    let exe = std::env::current_exe().expect("exe");
    let dir = exe.parent().expect("dir").to_path_buf();
    let cfg: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("fake.json")).expect("fake.json")).expect("json");
    let version = cfg["version"].as_str().unwrap_or("0.0.0").to_string();
    let mode = cfg["mode"].as_str().unwrap_or("healthy").to_string();
    let _ = std::fs::write(dir.join("env.txt"), std::env::var("PDT_SUPERVISED").unwrap_or_default());
    let crash = mode == "crash" || (mode == "crash_once" && !dir.join("crashed-once").exists());
    if crash {
        let _ = std::fs::write(dir.join("crashed-once"), "");
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::process::exit(1);
    }
    let port: u16 = std::env::var("PDT_PORT").expect("PDT_PORT").parse().expect("port");
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        if req.starts_with("POST /api/shutdown") {
            let _ = s.write_all(b"HTTP/1.0 202 Accepted\r\nContent-Length: 0\r\n\r\n");
            let _ = s.flush();
            std::process::exit(0);
        }
        let reply = if req.starts_with("GET /api/health") && mode != "unhealthy" {
            let body = serde_json::json!({"status": "ok", "name": pdt_core::brand::PRODUCT_NAME, "version": version}).to_string();
            format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
        } else {
            "HTTP/1.0 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_string()
        };
        let _ = s.write_all(reply.as_bytes());
    }
}
```

`crates/pdt-host/tests/supervise.rs`:

```rust
use pdt_core::handoff::{UpdateOutcome, UpdateRequest, UpdateResult, request_path, result_path};
use pdt_host::config::HostConfig;
use pdt_host::health::{Probe, probe, request_shutdown};
use pdt_host::install::CopyDirInstaller;
use pdt_host::supervise::{Backoff, run};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A folder holding the fake pdtd (as `pdtd[.exe]`) and its `fake.json`.
fn fake_install(dir: &Path, version: &str, mode: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = dir.join(format!("pdtd{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_fake-pdtd"), &exe).unwrap();
    std::fs::write(dir.join("fake.json"), serde_json::json!({"version": version, "mode": mode}).to_string()).unwrap();
    dir.to_path_buf()
}

fn config(install: &Path, data: &Path, port: u16, health: Duration) -> HostConfig {
    HostConfig {
        install_dir: install.to_path_buf(),
        data_dir: data.to_path_buf(),
        port,
        dev: true,
        health_timeout: health,
        backoff: Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(500),
            reset_after: Duration::from_secs(5),
        },
        poll: Duration::from_millis(200),
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn supervise(cfg: HostConfig, backup: PathBuf) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let handle = std::thread::spawn(move || {
        let installer = CopyDirInstaller { target: cfg.install_dir.clone(), backup };
        run(&cfg, &installer, &s, &|| {});
    });
    Running { stop, handle: Some(handle) }
}

fn wait_version(port: u16, want: &str, within: Duration) {
    let end = Instant::now() + within;
    loop {
        if probe(port) == (Probe::Ours { version: want.into() }) {
            return;
        }
        assert!(Instant::now() < end, "never saw v{want} on port {port} (last: {:?})", probe(port));
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn read_result(data: &Path) -> UpdateResult {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(b) = std::fs::read(result_path(data))
            && let Ok(r) = serde_json::from_slice(&b)
        {
            return r;
        }
        assert!(Instant::now() < end, "no update result was written");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn request(data: &Path, version: &str, asset: &Path) {
    let req = UpdateRequest {
        version: version.into(),
        tag: format!("v{version}"),
        asset: asset.to_path_buf(),
        from_version: "0.1.5".into(),
        requested_at: 1,
    };
    let p = request_path(data);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_vec(&req).unwrap()).unwrap();
}

#[test]
fn starts_pdtd_supervised_and_restarts_it_after_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "crash_once");
    let port = free_port();
    let _h = supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    assert!(install.join("crashed-once").exists(), "the first start crashed");
    assert_eq!(std::fs::read_to_string(install.join("env.txt")).unwrap(), "1", "PDT_SUPERVISED is set");
    // A clean exit (not an update) is restarted too.
    request_shutdown(port).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    wait_version(port, "0.1.5", Duration::from_secs(20));
}

#[test]
fn installs_an_update_and_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "healthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port, Duration::from_secs(10)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    wait_version(port, "0.1.6", Duration::from_secs(30));
    let r = read_result(&data);
    assert_eq!((r.outcome, r.to_version.as_str()), (UpdateOutcome::Installed, "0.1.6"));
    assert!(!request_path(&data).exists(), "the request was taken");
}

#[test]
fn rolls_back_when_the_new_version_fails_its_health_check() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let asset = fake_install(&tmp.path().join("release"), "0.1.6", "unhealthy");
    let data = tmp.path().join("data");
    let port = free_port();
    let _h = supervise(config(&install, &data, port, Duration::from_secs(3)), tmp.path().join("backup"));
    wait_version(port, "0.1.5", Duration::from_secs(20));
    request(&data, "0.1.6", &asset);
    request_shutdown(port).unwrap();
    let r = read_result(&data);
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{}", r.message);
    assert!(r.message.contains("0.1.6"));
    wait_version(port, "0.1.5", Duration::from_secs(30));
}

#[test]
fn adopts_a_running_pdtd_and_takes_over_when_it_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let install = fake_install(&tmp.path().join("app"), "0.1.5", "healthy");
    let port = free_port();
    let mut outside = std::process::Command::new(install.join(format!("pdtd{}", std::env::consts::EXE_SUFFIX)))
        .env("PDT_PORT", port.to_string())
        .spawn()
        .unwrap();
    wait_version(port, "0.1.5", Duration::from_secs(10));
    let _h = supervise(config(&install, &tmp.path().join("data"), port, Duration::from_secs(10)), tmp.path().join("backup"));
    std::thread::sleep(Duration::from_secs(1));
    request_shutdown(port).unwrap();
    let _ = outside.wait();
    wait_version(port, "0.1.5", Duration::from_secs(20));
}
```

- [ ] **Step 7: The `pdt-host` binary (`run`)**

`src/main.rs`:

```rust
use anyhow::{Context, bail};
use pdt_host::config::HostConfig;
use pdt_host::log::log;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn usage() -> ! {
    eprintln!("usage: {} run [--install-dir <dir>]", pdt_core::brand::HOST_BIN);
    std::process::exit(2);
}

/// `--install-dir <dir>`, else the folder this executable is in.
fn install_dir(args: &[String]) -> anyhow::Result<PathBuf> {
    match args.iter().position(|a| a == "--install-dir") {
        Some(i) => Ok(PathBuf::from(args.get(i + 1).context("--install-dir needs a value")?)),
        None => Ok(std::env::current_exe()?.parent().context("no exe folder")?.to_path_buf()),
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let cfg = HostConfig::from_env(install_dir(args)?).map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&cfg.data_dir)?;
    let lock_path = cfg.data_dir.join(format!("{}.lock", pdt_core::brand::HOST_BIN));
    let lock = std::fs::File::options().create(true).truncate(false).write(true).open(&lock_path)?;
    if lock.try_lock().is_err() {
        log(&cfg, "another pdt-host is already supervising this data folder; exiting");
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    pdt_host::selfupdate::clean_old(&exe);
    if !cfg.pdtd_path().is_file() {
        bail!("{} not found", cfg.pdtd_path().display());
    }
    log(&cfg, &format!("supervising {} on port {}", cfg.pdtd_path().display(), cfg.port));
    let installer = pdt_host::install::installer_for(&cfg).map_err(anyhow::Error::msg)?;
    let stop = AtomicBool::new(false);
    let bundled = cfg.install_dir.join(format!("{}{}", pdt_core::brand::HOST_BIN, std::env::consts::EXE_SUFFIX));
    let after_install = || {
        if exe != bundled && pdt_host::selfupdate::needs_replace(&exe, &bundled) {
            log(&cfg, "the release changes pdt-host itself; replacing and restarting");
            match pdt_host::selfupdate::replace(&exe, &bundled) {
                Ok(()) => relaunch(&exe, args),
                Err(e) => log(&cfg, &format!("could not replace pdt-host: {e}")),
            }
        }
    };
    pdt_host::supervise::run(&cfg, installer.as_ref(), &stop, &after_install);
    drop(lock);
    Ok(())
}

/// Starts the replaced pdt-host and exits. Under launchd, exiting is enough: KeepAlive starts the new copy. pdtd keeps
/// running; the new pdt-host adopts it.
fn relaunch(exe: &std::path::Path, args: &[String]) -> ! {
    let under_launchd = std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == pdt_core::brand::HOST_LABEL);
    if !under_launchd {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("run").args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000 | 0x0000_0200);
        }
        let _ = cmd.spawn();
    }
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("{}: {e:#}", pdt_core::brand::HOST_BIN);
        std::process::exit(1);
    }
}
```

The lock lets a second `run` exit at once. The new copy started by `relaunch` would hit the lock and exit if the old process still held it, so `relaunch` exits right after spawning. If the new copy still finds the lock held, it exits; launchd or the next tray or logon start brings it back.

`HOST_LABEL` is added to `brand.rs` in Task 13. Add it now as `pub const HOST_LABEL: &str = "dev.personaldevteam.host";`, with the doc comment `/// launchd label of the per-user LaunchAgent that runs pdt-host (macOS).`

Run: `cargo test -p pdt-host`
Expected: PASS on both OSes. These tests start real processes; allow them up to a minute each.

- [ ] **Step 8: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: green.

```bash
git add Cargo.toml Cargo.lock crates
git commit -m "feat: pdt-host supervisor: restart with backoff, install updates, health check, rollback"
```

---

### Task 13: Registering `pdt-host`, bundling it, and the tray as a view

**Files:**
- Create:
  - `crates/pdt-host/src/register.rs`
  - `desktop/src-tauri/windows/hooks.nsh`
  - `desktop/src-tauri/src/api.rs`
- Rename: `scripts/stage-pdtd.mjs` → `scripts/stage-sidecars.mjs`
- Modify:
  - `crates/pdt-core/src/brand.rs`
  - `crates/pdt-host/src/{lib,main}.rs`
  - `desktop/src-tauri/tauri.bundle.conf.json`
  - `desktop/src-tauri/Cargo.toml`
  - `desktop/src-tauri/src/{main,daemon}.rs`
  - `.github/workflows/ci.yml`
  - `scripts/release-stable.ps1`, `scripts/release-stable.sh`
- Delete: `desktop/src-tauri/src/update.rs`, `desktop/src-tauri/src/platform.rs`

**Interfaces:**
- Consumes:
  - `pdt_host::{config::HostConfig, health::{probe, Probe, request_shutdown}}` (Task 12);
  - `GET /api/host/update` and `POST /api/host/update/{check,install}` (Task 11).
- Produces (`pdt_core::brand`): `HOST_LABEL` (added in Task 12) and `HOST_AUTOSTART_NAME = "Personal Dev Team host"`.
- Produces (`pdt_host::register`):
  - `installed_host_path(data_dir)`, `launch_agent_path(home)`, `launchd_plist(program, install_dir, log)` and `run_key_command(program, install_dir)`;
  - `install(&HostConfig) -> Result<(), String>` and `uninstall() -> Result<(), String>`.
- Produces (binary): `pdt-host install [--install-dir <dir>]` and `pdt-host uninstall`.
- Produces (desktop):
  - the tray starts `pdt-host run` (sidecar next to its exe) when nothing answers;
  - its Update item follows `/api/host/update`;
  - Quit leaves `pdtd` running.

- [ ] **Step 1: Registration (TDD for the generated text)**

Add to `brand.rs`:

```rust
/// Name of the per-user autostart entry for pdt-host (Windows `HKCU\…\Run` value).
pub const HOST_AUTOSTART_NAME: &str = "Personal Dev Team host";
```

`crates/pdt-host/src/register.rs`:

```rust
//! Starting pdt-host at login (spec §3.1): a per-user launchd LaunchAgent on macOS (RunAtLoad, KeepAlive), and a
//! per-user `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value on Windows (decision 14; no admin needed).
//! pdt-host runs from a copy in the data folder, so installers can replace the install folder.

use crate::config::HostConfig;
use pdt_core::brand::{HOST_AUTOSTART_NAME, HOST_BIN, HOST_LABEL};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn installed_host_path(data_dir: &Path) -> PathBuf {
    data_dir.join("host").join(format!("{HOST_BIN}{}", std::env::consts::EXE_SUFFIX))
}

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{HOST_LABEL}.plist"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn launchd_plist(program: &Path, install_dir: &Path, log: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{program}</string>
    <string>run</string>
    <string>--install-dir</string>
    <string>{dir}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = xml(HOST_LABEL),
        program = xml(&program.to_string_lossy()),
        dir = xml(&install_dir.to_string_lossy()),
        log = xml(&log.to_string_lossy()),
    )
}

pub fn run_key_command(program: &Path, install_dir: &Path) -> String {
    format!("\"{}\" run --install-dir \"{}\"", program.display(), install_dir.display())
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Whether another pdt-host holds the data folder's lock.
fn host_running(data_dir: &Path) -> bool {
    let path = data_dir.join(format!("{HOST_BIN}.lock"));
    let Ok(f) = std::fs::File::options().write(true).open(path) else { return false };
    matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Copies this executable into the data folder (unless a running pdt-host owns it; it replaces itself after updates),
/// registers it to start at login, and starts it if it is not running.
pub fn install(cfg: &HostConfig) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = installed_host_path(&cfg.data_dir);
    let running = host_running(&cfg.data_dir);
    if !running {
        std::fs::create_dir_all(target.parent().expect("host dir")).map_err(|e| e.to_string())?;
        if me != target {
            std::fs::copy(&me, &target).map_err(|e| format!("copying {} to {}: {e}", me.display(), target.display()))?;
        }
    }
    if cfg!(target_os = "macos") {
        let home = std::env::home_dir().ok_or("no home folder")?;
        let plist = launch_agent_path(&home);
        let log = cfg.data_dir.join("logs").join(format!("{HOST_BIN}.launchd.log"));
        let _ = std::fs::create_dir_all(log.parent().expect("logs"));
        std::fs::create_dir_all(plist.parent().expect("LaunchAgents")).map_err(|e| e.to_string())?;
        std::fs::write(&plist, launchd_plist(&target, &cfg.install_dir, &log)).map_err(|e| e.to_string())?;
        if !running {
            let uid = String::from_utf8_lossy(&Command::new("id").arg("-u").output().map_err(|e| e.to_string())?.stdout)
                .trim()
                .to_string();
            let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{HOST_LABEL}")]).status();
            let ok = Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{uid}")])
                .arg(&plist)
                .status()
                .map_err(|e| e.to_string())?;
            if !ok.success() {
                return Err(format!("launchctl bootstrap failed ({ok})"));
            }
        }
    } else if cfg!(windows) {
        let ok = Command::new("reg")
            .args(["add", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/t", "REG_SZ", "/d", &run_key_command(&target, &cfg.install_dir), "/f"])
            .status()
            .map_err(|e| e.to_string())?;
        if !ok.success() {
            return Err(format!("reg add failed ({ok})"));
        }
        if !running {
            let mut cmd = Command::new(&target);
            cmd.args(["run", "--install-dir"]).arg(&cfg.install_dir);
            for (k, _) in std::env::vars() {
                if k.starts_with("PDT_") {
                    cmd.env_remove(k);
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000 | 0x0000_0200);
            }
            cmd.spawn().map_err(|e| format!("starting pdt-host: {e}"))?;
        }
    } else {
        return Err("registration is supported on macOS and Windows only".into());
    }
    Ok(())
}

/// Removes the login registration. A running pdt-host keeps running until logout.
pub fn uninstall() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let home = std::env::home_dir().ok_or("no home folder")?;
        let uid = String::from_utf8_lossy(&Command::new("id").arg("-u").output().map_err(|e| e.to_string())?.stdout)
            .trim()
            .to_string();
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{HOST_LABEL}")]).status();
        let _ = std::fs::remove_file(launch_agent_path(&home));
    } else if cfg!(windows) {
        let _ = Command::new("reg").args(["delete", RUN_KEY, "/v", HOST_AUTOSTART_NAME, "/f"]).status();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_agent_runs_the_data_folder_copy_at_load_and_keeps_it_alive() {
        let p = launchd_plist(
            Path::new("/Users/me/Library/Application Support/pdt/host/pdt-host"),
            Path::new("/Users/me/Applications/A & B.app/Contents/MacOS"),
            Path::new("/tmp/h.log"),
        );
        assert!(p.contains("<key>Label</key><string>dev.personaldevteam.host</string>"));
        assert!(p.contains("<string>/Users/me/Library/Application Support/pdt/host/pdt-host</string>"));
        assert!(p.contains("<string>/Users/me/Applications/A &amp; B.app/Contents/MacOS</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>") && p.contains("<key>KeepAlive</key><true/>"));
        assert_eq!(
            launch_agent_path(Path::new("/Users/me")),
            Path::new("/Users/me/Library/LaunchAgents/dev.personaldevteam.host.plist")
        );
    }

    #[test]
    fn run_key_quotes_paths() {
        let c = run_key_command(Path::new(r"C:\Users\me\AppData\Local\pdt\host\pdt-host.exe"), Path::new(r"C:\Program Files\P D T"));
        assert_eq!(c, r#""C:\Users\me\AppData\Local\pdt\host\pdt-host.exe" run --install-dir "C:\Program Files\P D T""#);
        assert!(installed_host_path(Path::new("d")).starts_with(Path::new("d").join("host")));
    }
}
```

Add `pub mod register;` to `lib.rs`. In `main.rs`, add these subcommands:

```rust
        Some("install") => install_dir(&args[1..])
            .and_then(|d| HostConfig::from_env(d).map_err(anyhow::Error::msg))
            .and_then(|cfg| pdt_host::register::install(&cfg).map_err(anyhow::Error::msg)),
        Some("uninstall") => pdt_host::register::uninstall().map_err(anyhow::Error::msg),
```

Update `usage()` to list `install [--install-dir <dir>]` and `uninstall`.

Run: `cargo test -p pdt-host`
Expected: PASS. **Do not run `pdt-host install` on the dev machine**: it registers a login item for the stable data folder.

- [ ] **Step 2: Bundle `pdt-host` as a sidecar and register it from the installer**

`git mv scripts/stage-pdtd.mjs scripts/stage-sidecars.mjs`, then make its body:

```js
// Copies target/release/{pdtd,pdt-host} to desktop/src-tauri/binaries/<name>-<host triple>, the names Tauri's
// externalBin expects.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const host = execFileSync('rustc', ['-vV'], { cwd: root, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]?.trim();
if (!host) throw new Error('could not read the host triple from `rustc -vV`');
const ext = process.platform === 'win32' ? '.exe' : '';
const out = join(root, 'desktop', 'src-tauri', 'binaries');
mkdirSync(out, { recursive: true });
for (const name of ['pdtd', 'pdt-host']) {
  copyFileSync(join(root, 'target', 'release', `${name}${ext}`), join(out, `${name}-${host}${ext}`));
  console.log(`staged ${name} for ${host}`);
}
```

`desktop/src-tauri/tauri.bundle.conf.json`:

```json
{
  "bundle": {
    "externalBin": ["binaries/pdtd", "binaries/pdt-host"],
    "resources": { "../../web/dist/": "web/" },
    "windows": { "nsis": { "installMode": "currentUser", "installerHooks": "./windows/hooks.nsh" } }
  }
}
```

`desktop/src-tauri/windows/hooks.nsh` (fact 8):

```nsis
; Registers pdt-host to start at login and starts it (it starts pdtd). During an update pdt-host is already running
; from the data folder and only refreshes the registration.
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\pdt-host.exe" install --install-dir "$INSTDIR"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$INSTDIR\pdt-host.exe" uninstall'
!macroend
```

In `.github/workflows/ci.yml`, in the `bundle` job, replace `cargo build --release -p pdtd` with `cargo build --release -p pdtd -p pdt-host`, and replace `node scripts/stage-pdtd.mjs` with `node scripts/stage-sidecars.mjs`. `PDT_BUILD_VERSION` already applies to the whole job, so both binaries report the release version.

- [ ] **Step 3: Release scripts hand local builds to `pdt-host`**

Both scripts keep their build steps; change them to `cargo build --release -p pdtd -p pdt-host` and `node scripts/stage-sidecars.mjs`. Replace their install half:

`scripts/release-stable.ps1`, after `if ($BuildOnly) { return }`:

```powershell
$dataDir = Join-Path $env:LOCALAPPDATA 'pdt'
$status = $null
try { $status = Invoke-RestMethod -Uri "http://127.0.0.1:$port/api/host/update" -TimeoutSec 5 } catch {}
if ($status -and $status.state -ne 'disabled') {
    # pdt-host supervises the stable pdtd: hand the installer over like a GitHub release (with health check and rollback).
    $drop = Join-Path $dataDir 'update\downloads\local'
    New-Item -ItemType Directory -Force $drop | Out-Null
    Copy-Item $setup.FullName (Join-Path $drop $setup.Name) -Force
    # A local build reports 0.1.0-local (BUILD_VERSION); pdt-host's health check expects exactly that.
    $request = @{ version = '0.1.0-local'; tag = 'local'; asset = (Join-Path $drop $setup.Name);
        fromVersion = $status.current; requestedAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() } | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $dataDir 'update\request.json'), $request)
    Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" | Out-Null
    Write-Host "Handed the installer to pdt-host; see $dataDir\logs\pdt-host.log and $dataDir\update\result.json."
    return
}
```

Keep the existing path (stop tray, stop pdtd, silent install, launch the app) after that block for first installs. The installer's POSTINSTALL hook now registers and starts `pdt-host`.

`scripts/release-stable.sh`, after `$build_only && exit 0`:

```bash
data_dir="$HOME/Library/Application Support/pdt"
status="$(curl -s --max-time 5 "http://127.0.0.1:$port/api/host/update" || true)"
if [[ -n "$status" && "$status" != *'"state":"disabled"'* ]]; then
  # pdt-host supervises the stable pdtd: hand the bundle over like a GitHub release (health check, rollback).
  drop="$data_dir/update/downloads/local"
  mkdir -p "$drop"
  (cd "$(dirname "$bundle")" && tar -czf "$drop/$product.app.tar.gz" "$product.app")
  now="$(($(date +%s) * 1000))"
  printf '{"version":"0.1.0-local","tag":"local","asset":"%s","fromVersion":"local","requestedAt":%s}\n' \
    "$drop/$product.app.tar.gz" "$now" > "$data_dir/update/request.json"
  curl -fsS -X POST "http://127.0.0.1:$port/api/shutdown" >/dev/null
  echo "Handed the bundle to pdt-host; see $data_dir/logs/pdt-host.log and $data_dir/update/result.json."
  exit 0
fi
```

After a first manual install on macOS, the tray registers `pdt-host` (Step 4).

- [ ] **Step 4: The tray becomes a view**

Delete `desktop/src-tauri/src/update.rs` and its `mod update;`. Then:
- run (PowerShell) `cargo add pdt-host -p pdt-desktop`;
- remove `tempfile` from the desktop's dev-dependencies if nothing else uses it.

`desktop/src-tauri/src/api.rs`:

```rust
//! Tiny loopback JSON client for the tray (no extra dependencies).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub fn call(port: u16, method: &str, path: &str, body: Option<&str>) -> Result<(u16, serde_json::Value), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(900)));
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("not an HTTP response")?;
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("no status")?;
    Ok((status, serde_json::from_str(body).unwrap_or(serde_json::Value::Null)))
}

/// The Update menu item for a `/api/host/update` status: (label, enabled).
pub fn update_label(status: &serde_json::Value) -> (String, bool) {
    match status["state"].as_str() {
        Some("available") => (format!("Update to v{}…", status["latest"]["version"].as_str().unwrap_or("?")), true),
        Some("checking") => ("Checking for updates…".into(), false),
        Some("downloading") => ("Downloading the update…".into(), false),
        Some("installing") => ("Installing the update…".into(), false),
        Some("disabled") => ("Updates are off for this app".into(), false),
        _ if status["error"].is_string() => ("Update check failed — see Settings".into(), true),
        _ => ("Check for updates".into(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels() {
        assert_eq!(update_label(&json!({"state": "available", "latest": {"version": "0.1.9"}})), ("Update to v0.1.9…".into(), true));
        assert_eq!(update_label(&json!({"state": "installing"})).1, false);
        assert_eq!(update_label(&json!({"state": "disabled"})).1, false);
        assert_eq!(update_label(&json!({"state": "idle", "error": "x"})).1, true);
        assert_eq!(update_label(&json!({"state": "idle", "error": null})), ("Check for updates".into(), true));
    }
}
```

In `daemon.rs`:
- Delete `Probe`, `probe` and `classify_health`, and use `pdt_host::health::{probe, Probe}` instead (`Probe::Ours { .. }` matches where the old code had `Probe::Ours`). Keep `ensure_daemon`, `desktop_profile`, `launch_log_path` and `with_log_tail`.
- Delete `stop_daemon`, `lock_released`, `daemon_exited` and `pdtd_path`, along with their tests.
- Replace `spawn_pdtd` with:

```rust
fn host_path() -> PathBuf {
    let file = format!("{}{}", pdt_core::brand::HOST_BIN, std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

/// Starts pdt-host (which starts and supervises pdtd), detached; it keeps running if the app exits. A pdt-host that is
/// already running (login item) makes this one exit at once.
pub fn spawn_host(profile: Profile) -> std::io::Result<()> {
    let mut cmd = Command::new(host_path());
    cmd.arg("run");
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        cmd.arg("--install-dir").arg(dir);
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    if profile == Profile::Dev {
        cmd.env("PDT_PROFILE", "dev");
    }
    let log = launch_log_path(profile).and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::File::options().create(true).append(true).open(p).ok()
    });
    let (out, err) = match log.and_then(|f| Some((f.try_clone().ok()?, f))) {
        Some((a, b)) => (Stdio::from(a), Stdio::from(b)),
        None => (Stdio::null(), Stdio::null()),
    };
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(|_| ())
}

/// macOS: the installed stable app registers pdt-host's LaunchAgent once (there is no installer to do it).
pub fn ensure_registered(profile: Profile) {
    if !cfg!(target_os = "macos") || profile != Profile::Stable {
        return;
    }
    let Some(home) = std::env::home_dir() else { return };
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from));
    let installed_bundle = exe_dir.as_deref().and_then(pdt_host::install::bundle_of).is_some();
    if installed_bundle && !pdt_host::register::launch_agent_path(&home).exists() {
        let mut cmd = Command::new(host_path());
        cmd.arg("install");
        if let Some(d) = exe_dir {
            cmd.arg("--install-dir").arg(d);
        }
        let _ = cmd.status();
    }
}
```

The `daemon.rs` tests that remain (`ensure_daemon` with stub probes, `debug_builds_default_to_dev` and `log_tail_is_appended`) switch to `Probe::Ours { version: "x".into() }` where they returned `Probe::Ours`.

In `daemon.rs` `ensure_daemon`, compare with `matches!(last, Probe::Ours { .. })` instead of `== Probe::Ours`.

Replace `desktop/src-tauri/src/main.rs` with the code below, and delete `platform.rs` (nothing uses it any more). The tray now has Open, Update, Restart daemon, Version and Quit. Quit exits only the tray (decision 16).

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The desktop app: an optional tray view of the host (remote-host spec §3.1). pdt-host keeps pdtd running; the tray
//! starts pdt-host when nothing answers, opens the web UI, shows the version and drives pdtd's update API.
mod api;
mod daemon;

use pdt_core::brand::{PRODUCT_NAME, Profile};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// pdt-host gives a first start up to 60 s; leave a margin.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(70);
const REFRESH_EVERY: Duration = Duration::from_secs(60);

struct Tray {
    profile: Profile,
    port: u16,
    restart_item: MenuItem<Wry>,
    update_item: MenuItem<Wry>,
    version_item: MenuItem<Wry>,
    /// Set while startup or a restart runs, so they never overlap.
    busy: AtomicBool,
    ready: AtomicBool,
}

impl Tray {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
    fn begin(&self) -> bool {
        self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
    fn end(&self) {
        self.busy.store(false, Ordering::SeqCst);
    }
}

fn main() {
    let profile = daemon::desktop_profile(std::env::var("PDT_PROFILE").ok().as_deref(), cfg!(debug_assertions));
    let mut context = tauri::generate_context!();
    if profile == Profile::Dev {
        let config = context.config_mut();
        config.identifier = format!("{}.dev", config.identifier);
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_ui(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            setup(app, profile)?;
            Ok(())
        })
        .build(context)
        .expect("error while building the desktop app");
    app.run(|_app, event| match event {
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_ui(_app),
        _ => {}
    });
}

fn setup(app: &mut tauri::App, profile: Profile) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let port = profile.port();
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let update_item = MenuItem::with_id(app, "update", "Checking for updates…", false, None::<&str>)?;
    let restart_item = MenuItem::with_id(app, "restart", "Restart daemon", false, None::<&str>)?;
    let version_item = MenuItem::with_id(app, "version", "Version …", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &update_item,
            &restart_item,
            &PredefinedMenuItem::separator(app)?,
            &version_item,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let suffix = if profile == Profile::Dev { " (dev)" } else { "" };
    let icon = app.default_window_icon().cloned().ok_or("the app has no icon")?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(format!("{PRODUCT_NAME}{suffix} — port {port}"))
        .menu(&menu)
        .show_menu_on_left_click(!cfg!(windows))
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if cfg!(windows)
                && let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                    event
            {
                open_ui(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(Tray {
        profile,
        port,
        restart_item,
        update_item,
        version_item,
        busy: AtomicBool::new(true),
        ready: AtomicBool::new(false),
    });

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let tray = handle.state::<Tray>();
        daemon::ensure_registered(profile);
        match daemon::ensure_daemon(port, pdt_host::health::probe, || daemon::spawn_host(profile), STARTUP_TIMEOUT) {
            Ok(()) => {
                let _ = tray.restart_item.set_enabled(true);
                tray.ready.store(true, Ordering::SeqCst);
                tray.end();
                open_ui(&handle);
                loop {
                    refresh(&handle);
                    std::thread::sleep(REFRESH_EVERY);
                }
            }
            Err(e) => {
                let h = handle.clone();
                error_dialog(&handle, daemon::with_log_tail(e, profile)).show(move |_| h.exit(1));
            }
        }
    });
    Ok(())
}

/// Updates the Update and Version items from pdtd.
fn refresh(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if let Ok((200, s)) = api::call(tray.port, "GET", "/api/host/update", None) {
        let (text, enabled) = api::update_label(&s);
        let _ = tray.update_item.set_text(text);
        let _ = tray.update_item.set_enabled(enabled);
    }
    if let Ok((200, h)) = api::call(tray.port, "GET", "/api/health", None) {
        let _ = tray.version_item.set_text(format!("Version {}", h["version"].as_str().unwrap_or("?")));
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "open" => open_ui(app),
        "update" => update_clicked(app),
        "restart" => restart_daemon(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn error_dialog(app: &AppHandle, message: String) -> tauri_plugin_dialog::MessageDialogBuilder<Wry> {
    app.dialog().message(message).kind(MessageDialogKind::Error).title(PRODUCT_NAME)
}

fn ask(app: &AppHandle, message: &str, ok: &str) -> bool {
    app.dialog()
        .message(message)
        .title(PRODUCT_NAME)
        .buttons(MessageDialogButtons::OkCancelCustom(ok.into(), "Cancel".into()))
        .blocking_show()
}

fn open_ui(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    if !tray.ready.load(Ordering::SeqCst) {
        return;
    }
    let url = tray.url();
    if let Err(e) = app.opener().open_url(url.clone(), None::<&str>) {
        error_dialog(app, format!("Could not open {url}: {e}")).show(|_| {});
    }
}

/// Asks pdtd to stop; pdt-host starts it again. Waits until it answers.
fn restart_daemon(app: &AppHandle) {
    let tray = app.state::<Tray>();
    if !tray.begin() {
        return;
    }
    let _ = tray.restart_item.set_enabled(false);
    let app = app.clone();
    std::thread::spawn(move || {
        let tray = app.state::<Tray>();
        let result = pdt_host::health::request_shutdown(tray.port).and_then(|()| {
            std::thread::sleep(Duration::from_secs(2));
            daemon::ensure_daemon(tray.port, pdt_host::health::probe, || Ok(()), STARTUP_TIMEOUT)
        });
        if let Err(e) = result {
            error_dialog(&app, format!("The daemon did not come back: {}", daemon::with_log_tail(e, tray.profile)))
                .show(|_| {});
        }
        let _ = tray.restart_item.set_enabled(true);
        tray.end();
    });
}

/// Update: install the available release (asking first, and again if the host is busy), or check for one.
fn update_clicked(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let port = app.state::<Tray>().port;
        let status = api::call(port, "GET", "/api/host/update", None).map(|(_, v)| v).unwrap_or_default();
        if status["state"] != "available" {
            let _ = api::call(port, "POST", "/api/host/update/check", None);
            refresh(&app);
            return;
        }
        let version = status["latest"]["version"].as_str().unwrap_or("?").to_string();
        if !ask(&app, &format!("Update to v{version}? Sessions resume afterwards."), "Update") {
            return;
        }
        let mut force = false;
        loop {
            let body = serde_json::json!({ "force": force }).to_string();
            match api::call(port, "POST", "/api/host/update/install", Some(&body)) {
                Ok((200, _)) => break,
                Ok((409, v)) if v["code"] == "host_busy" && !force => {
                    if !ask(&app, v["message"].as_str().unwrap_or("The host is busy. Update anyway?"), "Update") {
                        return;
                    }
                    force = true;
                }
                Ok((_, v)) => {
                    error_dialog(&app, format!("The update failed: {}", v["message"].as_str().unwrap_or("unknown error")))
                        .show(|_| {});
                    break;
                }
                Err(e) => {
                    error_dialog(&app, format!("The update failed: {e}")).show(|_| {});
                    break;
                }
            }
        }
        refresh(&app);
    });
}
```

`ensure_daemon`'s spawn callback has the type `impl FnOnce() -> std::io::Result<()>`, so `|| Ok(())` type-checks. `request_shutdown` returns `Result<(), String>`, and so does `ensure_daemon`, so the `and_then` chain composes.

Run (PowerShell):
- `pnpm --dir web build`
- `cargo build -p pdtd -p pdt-host`
- `cargo clippy -p pdt-desktop --all-targets -- -D warnings`
- `cargo test -p pdt-desktop`

Expected: PASS.

- [ ] **Step 5: Build-only check of the installer**

Run (PowerShell): `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-stable.ps1 -BuildOnly`
Expected: a `*-setup.exe` is built.

Inspect the generated NSIS script under `target\release\nsis\x64\` and confirm:
- it includes the hook macros;
- `pdt-host.exe` sits next to the app exe.

**Do not run the installer or the script without `-BuildOnly`.**

- [ ] **Step 6: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test; cargo clippy -p pdt-desktop --all-targets -- -D warnings; cargo test -p pdt-desktop`
Expected: green.

```bash
git add -A crates desktop scripts .github
git commit -m "feat: pdt-host login registration and sidecar; tray starts pdt-host and drives pdtd's update API"
```

---

### Task 14: Settings → This host, the first-run checklist and the update banner

**Files:**
- Create:
  - `crates/pdt-api/src/checks.rs`, `crates/pdt-api/tests/checks.rs`
  - `web/src/host/checklist.ts`
  - `web/src/host/HostSettingsPage.tsx`, `web/src/host/FirstRunChecklist.tsx`, `web/src/host/UpdateBanner.tsx`
  - `web/src/host/host.test.tsx`
- Modify:
  - `crates/pdt-api/src/{routes,lib}.rs`, `crates/pdtd/src/main.rs`
  - `web/src/api/client.ts`
  - `web/src/shell/{views.ts,AppShell.tsx,AppShell.test.tsx}`

**Interfaces:**
- Consumes:
  - `HostSettingsView` with `remote` (Tasks 2 and 4), `WhoAmI` (Task 5), `UpdateStatus` and `TokenState` (Task 11);
  - the host and token routes, `AppState.platform`, `AppState.tailscale`.
- Produces (`pdt_api`):
  - `ToolCheck { found, ok, detail }` and `HostChecks { claude, codex, git, tailscale, checkedAt }`;
  - `GET /api/host/checks` (runs them the first time) and `POST /api/host/checks/refresh`;
  - `pub async fn run_host_checks(&AppState) -> HostChecks`.
- Produces (web):
  - `checklist(settings, update, checks) -> ChecklistItem[]`;
  - `<HostSettingsPage/>`, `<FirstRunChecklist/>` and `<UpdateBanner/>`;
  - a `settings` rail view.

- [ ] **Step 1: Login and tool checks (TDD)**

`crates/pdt-api/tests/checks.rs`:

```rust
use pdt_api::{AppState, run_host_checks};
use pdt_tailscale::testing::RecordingTailscale;
use std::sync::Arc;

/// A fake program printing `out` and exiting with `code`.
fn fake(dir: &std::path::Path, name: &str, out: &str, code: i32) -> String {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, format!("@echo off\r\necho {out}\r\nexit /b {code}\r\n")).unwrap();
        p.to_string_lossy().into()
    } else {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\necho '{out}'\nexit {code}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.to_string_lossy().into()
    }
}

#[tokio::test]
async fn reports_logins_and_missing_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    st.platform.claude_program = fake(tmp.path(), "claude", r#"{"loggedIn": false, "authMethod": "none"}"#, 0);
    st.platform.codex_program = fake(tmp.path(), "codex", "Logged in using ChatGPT", 0);
    st.tailscale = Arc::new(RecordingTailscale::new(Err(pdt_tailscale::TsError::NotInstalled)));
    let c = run_host_checks(&st).await;
    assert!(c.claude.found && !c.claude.ok, "{:?}", c.claude);
    assert!(c.codex.found && c.codex.ok, "{:?}", c.codex);
    assert!(c.git.found && c.git.ok, "{:?}", c.git);
    assert!(!c.tailscale.ok && c.tailscale.detail.contains("not found"));

    st.platform.claude_program = tmp.path().join("no-such-claude").to_string_lossy().into();
    st.platform.codex_program = fake(tmp.path(), "codex2", "Not logged in", 1);
    let c = run_host_checks(&st).await;
    assert!(!c.claude.found && !c.claude.ok);
    assert!(c.codex.found && !c.codex.ok);
}
```

Run: `cargo test -p pdt-api --test checks`
Expected: compile errors.

`crates/pdt-api/src/checks.rs`:

```rust
//! The Doctor-style checks the first-run checklist needs (spec §3.1 "Login check", §6 "First run").

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::time::Duration;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ToolCheck {
    pub found: bool,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostChecks {
    pub claude: ToolCheck,
    pub codex: ToolCheck,
    pub git: ToolCheck,
    pub tailscale: ToolCheck,
    #[ts(type = "number")]
    pub checked_at: i64,
}

/// Runs `program args` (resolved like a session's program) with a 15 s limit: Err = not found / not runnable.
async fn run(st: &AppState, program: &str, args: &[&str]) -> Result<(bool, String), String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (exe, args) = pdt_agents::launch::resolve(program, &args, st.platform.login_path.as_deref()).map_err(|e| e.to_string())?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    if let Some(p) = &st.platform.login_path {
        cmd.env("PATH", p);
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") || k == "CLAUDECODE" {
            cmd.env_remove(k);
        }
    }
    let out = tokio::time::timeout(Duration::from_secs(15), cmd.output())
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text.trim().to_string()))
}

/// `claude auth status` prints JSON with `loggedIn` (verified fact 3).
pub(crate) fn claude_logged_in(stdout: &str) -> Option<bool> {
    let start = stdout.find('{')?;
    serde_json::from_str::<serde_json::Value>(&stdout[start..]).ok()?.get("loggedIn")?.as_bool()
}

pub async fn run_host_checks(st: &AppState) -> HostChecks {
    let claude = match run(st, &st.platform.claude_program, &["auth", "status"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((_, out)) => match claude_logged_in(&out) {
            Some(true) => ToolCheck { found: true, ok: true, detail: "logged in".into() },
            Some(false) => ToolCheck { found: true, ok: false, detail: "not logged in: run `claude` and log in".into() },
            None => ToolCheck { found: true, ok: false, detail: format!("could not read `claude auth status`: {out}") },
        },
    };
    let codex = match run(st, &st.platform.codex_program, &["login", "status"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((true, out)) => ToolCheck { found: true, ok: true, detail: out },
        Ok((false, out)) => ToolCheck { found: true, ok: false, detail: format!("not logged in: run `codex login` ({out})") },
    };
    let git = match run(st, "git", &["--version"]).await {
        Err(e) => ToolCheck { found: false, ok: false, detail: e },
        Ok((ok, out)) => ToolCheck { found: true, ok, detail: out },
    };
    let ts = st.tailscale.clone();
    let tailscale = match tokio::task::spawn_blocking(move || ts.status()).await {
        Ok(Ok(s)) => ToolCheck { found: true, ok: true, detail: s.dns_name },
        Ok(Err(e)) => ToolCheck { found: !matches!(e, pdt_tailscale::TsError::NotInstalled), ok: false, detail: e.to_string() },
        Err(e) => ToolCheck { found: false, ok: false, detail: e.to_string() },
    };
    let c = HostChecks { claude, codex, git, tailscale, checked_at: now_ms() };
    *st.checks.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    c
}

pub(crate) async fn get_checks(State(st): State<AppState>) -> Result<Json<HostChecks>, ApiError> {
    let cached = st.checks.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(Json(match cached {
        Some(c) => c,
        None => run_host_checks(&st).await,
    }))
}

pub(crate) async fn refresh_checks(State(st): State<AppState>) -> Json<HostChecks> {
    Json(run_host_checks(&st).await)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_claude_auth_status() {
        assert_eq!(super::claude_logged_in("{\n \"loggedIn\": true, \"authMethod\": \"claude.ai\"\n}"), Some(true));
        assert_eq!(super::claude_logged_in("warning\n{\"loggedIn\": false}"), Some(false));
        assert_eq!(super::claude_logged_in("Logged in"), None);
    }
}
```

Wire it:
- Add `pub(crate) checks: std::sync::Arc<std::sync::Mutex<Option<crate::checks::HostChecks>>>` to `AppState` (Default).
- In `lib.rs`: `mod checks; pub use checks::{HostChecks, run_host_checks};`.
- Add the routes `.route("/host/checks", get(crate::checks::get_checks))` and `.route("/host/checks/refresh", axum::routing::post(crate::checks::refresh_checks))`.
- In `pdtd` `main.rs`, after serving starts, run the checks in the background and warn (spec §3.1):

```rust
    {
        let st = state.clone();
        tokio::spawn(async move {
            let c = pdt_api::run_host_checks(&st).await;
            for (name, check) in [("claude", &c.claude), ("codex", &c.codex)] {
                if !check.ok {
                    tracing::warn!(tool = name, detail = %check.detail, "agent CLI is missing or not logged in for this user");
                }
            }
        });
    }
```

Run: `cargo test -p pdt-api`
Expected: PASS.

- [ ] **Step 2: Web client additions**

Add these imports and entries to `web/src/api/client.ts`:

```ts
import type { HostChecks } from './gen/HostChecks';
import type { HostSettingsPatch } from './gen/HostSettingsPatch';
import type { HostSettingsView } from './gen/HostSettingsView';
import type { RemoteStatus } from './gen/RemoteStatus';
import type { UpdateStatus } from './gen/UpdateStatus';
import type { WhoAmI } from './gen/WhoAmI';
```

```ts
  hostSettings: () => request<HostSettingsView>('/api/host/settings'),
  saveHostSettings: (p: HostSettingsPatch) =>
    request<HostSettingsView>('/api/host/settings', { method: 'PUT', body: JSON.stringify(p) }),
  refreshRemote: () => request<RemoteStatus>('/api/host/remote/refresh', { method: 'POST' }),
  whoami: () => request<WhoAmI>('/api/host/whoami'),
  updateStatus: () => request<UpdateStatus>('/api/host/update'),
  checkUpdate: () => request<UpdateStatus>('/api/host/update/check', { method: 'POST' }),
  installUpdate: (force: boolean) =>
    request<UpdateStatus>('/api/host/update/install', { method: 'POST', body: JSON.stringify({ force }) }),
  dismissUpdateResult: () => request<void>('/api/host/update/dismiss', { method: 'POST' }),
  saveToken: (token: string) => request<UpdateStatus>('/api/host/token', { method: 'PUT', body: JSON.stringify({ token }) }),
  deleteToken: () => request<void>('/api/host/token', { method: 'DELETE' }),
  hostChecks: () => request<HostChecks>('/api/host/checks'),
  refreshChecks: () => request<HostChecks>('/api/host/checks/refresh', { method: 'POST' }),
```

- [ ] **Step 3: Write the failing UI tests**

`web/src/host/host.test.tsx`:

```tsx
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { checklist } from './checklist';
import { HostSettingsPage } from './HostSettingsPage';
import { UpdateBanner } from './UpdateBanner';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const settingsView = (over: object = {}) => ({
  settings: {
    remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 },
    maxConcurrentRuns: 4, agentCredentials: 'subscription',
  },
  ownerFromEnv: null,
  tokenPresent: false,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: 'me@example.com' },
  ...over,
});
const update = (over: object = {}) => ({
  current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
  error: null, token: { state: 'missing' }, lastResult: null, ...over,
});
const tool = (ok: boolean) => ({ found: true, ok, detail: ok ? 'ok' : 'not logged in' });
const checks = (ok: boolean) => ({ claude: tool(ok), codex: tool(ok), git: tool(true), tailscale: tool(true), checkedAt: 1 });

describe('checklist', () => {
  it('lists what is missing on a fresh host', () => {
    const items = checklist(settingsView() as never, update() as never, checks(false) as never);
    expect(items.map((i) => i.id)).toEqual(['owner', 'remote', 'token', 'claude', 'codex']);
  });

  it('is empty when everything is set up', () => {
    const s = settingsView({
      tokenPresent: true,
      settings: { ...settingsView().settings, remoteAccess: true, ownerLogin: 'me@example.com' },
      remote: { ...settingsView().remote, enabled: true, active: true },
    });
    expect(checklist(s as never, update({ token: { state: 'valid', expiresAt: null } }) as never, checks(true) as never)).toEqual([]);
  });

  it('flags an expiring token and a remote access error', () => {
    const s = settingsView({
      tokenPresent: true,
      settings: { ...settingsView().settings, remoteAccess: true, ownerLogin: 'me@example.com' },
      remote: { ...settingsView().remote, enabled: true, active: false, error: 'Tailscale is not running' },
    });
    const items = checklist(s as never, update({ token: { state: 'expires_soon', expiresAt: 1 } }) as never, checks(true) as never);
    expect(items.map((i) => i.id)).toEqual(['remote', 'token']);
    expect(items[0].text).toContain('Tailscale is not running');
  });
});

describe('HostSettingsPage', () => {
  it('sets the owner from this device, then turns on remote access', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Use me@example.com' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Remote access' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ ownerLogin: 'me@example.com' }, { remoteAccess: true }]);
  });

  it('shows why remote access could not be turned on', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'PUT /api/host/settings': new Reply(400, { code: 'owner_required', message: 'set the owner first' }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Remote access' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('set the owner first');
  });

  it('stores a token without keeping it on screen', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'PUT /api/host/token': update({ token: { state: 'valid', expiresAt: null } }),
    });
    renderWithQuery(<HostSettingsPage />);
    const field = await screen.findByLabelText('GitHub token');
    expect(field).toHaveAttribute('type', 'password');
    await userEvent.type(field, 'github_pat_abc');
    await userEvent.click(screen.getByRole('button', { name: 'Save token' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/host/token')?.body).toEqual({ token: 'github_pat_abc' }));
    await waitFor(() => expect(field).toHaveValue(''));
  });
});

describe('UpdateBanner', () => {
  it('confirms before updating a busy host', async () => {
    let n = 0;
    const calls = stubApi({
      'GET /api/host/update': update({ state: 'available', latest: { version: '0.1.7', tag: 'v0.1.7', notes: 'Fixes', publishedAt: null } }),
      'POST /api/host/update/install': () =>
        ++n === 1 ? new Reply(409, { code: 'host_busy', message: 'Busy: Claude is working. Update anyway?' }) : update({ state: 'installing' }),
    });
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByText(/Update to v0.1.7 available/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Update' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    expect(confirm).toHaveBeenCalledWith('Busy: Claude is working. Update anyway?');
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ force: false }, { force: true }]);
  });

  it('reports a rollback with its log and can be dismissed', async () => {
    const calls = stubApi({
      'GET /api/host/update': update({
        lastResult: { outcome: 'rolled_back', fromVersion: '0.1.6', toVersion: '0.1.7', message: 'Update to v0.1.7 failed and was rolled back', log: '/d/logs/pdt-host.log', at: 1 },
      }),
      'POST /api/host/update/dismiss': undefined,
    });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('alert')).toHaveTextContent('failed and was rolled back');
    expect(screen.getByText('/d/logs/pdt-host.log')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/host/update/dismiss')).toBe(true));
  });
});
```

Run: `pnpm --dir web test -- host`
Expected: FAIL.

- [ ] **Step 4: Implement**

`web/src/host/checklist.ts`:

```ts
import type { HostChecks } from '../api/gen/HostChecks';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { UpdateStatus } from '../api/gen/UpdateStatus';

export interface ChecklistItem { id: string; text: string }

/** What a host still needs (spec §6 "First run"), in the order to fix it. */
export function checklist(s: HostSettingsView, u: UpdateStatus | undefined, c: HostChecks | undefined): ChecklistItem[] {
  const items: ChecklistItem[] = [];
  if (!s.ownerFromEnv && !s.settings.ownerLogin) items.push({ id: 'owner', text: 'Set the owner (your Tailscale login).' });
  if (!s.settings.remoteAccess) items.push({ id: 'remote', text: 'Remote access is off.' });
  else if (s.remote.error) items.push({ id: 'remote', text: `Remote access is not working: ${s.remote.error}` });
  if (!s.tokenPresent) items.push({ id: 'token', text: 'No update token: this host only updates manually.' });
  else if (u?.token.state === 'expires_soon') items.push({ id: 'token', text: 'The update token expires soon; replace it.' });
  else if (u?.token.state === 'expired') items.push({ id: 'token', text: 'The update token expired or was revoked; replace it.' });
  if (c && !c.claude.ok) items.push({ id: 'claude', text: `Claude Code: ${c.claude.detail}` });
  if (c && !c.codex.ok) items.push({ id: 'codex', text: `Codex: ${c.codex.detail}` });
  return items;
}
```

`web/src/host/FirstRunChecklist.tsx`:

```tsx
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { checklist } from './checklist';

export function FirstRunChecklist({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const settings = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks });
  if (!settings.data) return null;
  const items = checklist(settings.data, update.data, checks.data);
  if (!items.length) return null;
  return (
    <section aria-label="Set up this host" className="m-2 rounded-pdt border border-border bg-surface p-2 text-xs">
      <h3 className="font-semibold mb-1">Set up this host</h3>
      <ul className="list-disc pl-4 text-muted">
        {items.map((i) => <li key={i.id}>{i.text}</li>)}
      </ul>
      {onOpenSettings && (
        <button type="button" onClick={onOpenSettings} className="mt-1 text-accent underline">Open settings</button>
      )}
    </section>
  );
}
```

`web/src/host/UpdateBanner.tsx`:

```tsx
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button } from '@ui';
import { api, ApiError } from '../api/client';

export function UpdateBanner() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus, refetchInterval: 60_000 });
  const refresh = () => qc.invalidateQueries({ queryKey: ['host-update'] });
  const install = useMutation({
    mutationFn: async () => {
      try {
        return await api.installUpdate(false);
      } catch (e) {
        if (e instanceof ApiError && e.code === 'host_busy' && window.confirm(e.message)) return api.installUpdate(true);
        throw e;
      }
    },
    onSettled: refresh,
  });
  const dismiss = useMutation({ mutationFn: api.dismissUpdateResult, onSettled: refresh });
  const s = status.data;
  if (!s) return null;
  const r = s.lastResult;
  return (
    <>
      {r && (
        <div role={r.outcome === 'installed' ? 'status' : 'alert'} className={`flex flex-wrap items-center gap-2 border-b border-border px-3 py-1 text-xs ${r.outcome === 'installed' ? 'text-running' : 'text-failed'}`}>
          <span>{r.message}</span>
          {r.outcome !== 'installed' && r.log && <code className="text-muted">{r.log}</code>}
          <Button variant="ghost" onClick={() => dismiss.mutate()}>Dismiss</Button>
        </div>
      )}
      {s.state === 'available' && s.latest && (
        <div className="flex flex-wrap items-center gap-2 border-b border-border bg-accent-soft px-3 py-1 text-xs">
          <span className="font-medium">Update to v{s.latest.version} available</span>
          {s.latest.notes && (
            <details className="text-muted">
              <summary>Release notes</summary>
              <pre className="whitespace-pre-wrap font-sans">{s.latest.notes}</pre>
            </details>
          )}
          <Button variant="primary" disabled={install.isPending} onClick={() => install.mutate()}>Update</Button>
          {install.isError && !(install.error instanceof ApiError && install.error.code === 'host_busy') && (
            <span className="text-failed">{install.error.message}</span>
          )}
        </div>
      )}
      {(s.state === 'downloading' || s.state === 'installing') && (
        <div role="status" className="border-b border-border px-3 py-1 text-xs text-waiting">
          {s.state === 'downloading' ? 'Downloading the update…' : 'Installing the update; this page reconnects when the host is back.'}
        </div>
      )}
    </>
  );
}
```

`web/src/host/HostSettingsPage.tsx`:

```tsx
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { Button } from '@ui';
import { api } from '../api/client';
import type { CredentialPolicy } from '../api/gen/CredentialPolicy';
import type { HostSettingsPatch } from '../api/gen/HostSettingsPatch';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';

const field = 'rounded-pdt bg-surface border border-border px-2 py-1 text-sm';
const h = 'text-xs uppercase tracking-wider text-muted';

export function HostSettingsPage() {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const update = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus });
  const checks = useQuery({ queryKey: ['host-checks'], queryFn: api.hostChecks });
  const save = useMutation({
    mutationFn: (p: HostSettingsPatch) => api.saveHostSettings(p),
    onSuccess: (v) => qc.setQueryData(['host-settings'], v),
  });
  const token = useMutation({
    mutationFn: (t: string) => api.saveToken(t),
    onSuccess: (u) => {
      qc.setQueryData(['host-update'], u);
      void qc.invalidateQueries({ queryKey: ['host-settings'] });
      setTokenText('');
    },
  });
  const removeToken = useMutation({ mutationFn: api.deleteToken, onSuccess: () => void qc.invalidateQueries() });
  const refreshChecks = useMutation({ mutationFn: api.refreshChecks, onSuccess: (c) => qc.setQueryData(['host-checks'], c) });
  const [tokenText, setTokenText] = useState('');
  const [owner, setOwner] = useState('');
  const [ports, setPorts] = useState({ start: '', end: '' });
  const v = view.data;
  useEffect(() => {
    if (!v) return;
    setOwner(v.settings.ownerLogin ?? '');
    setPorts({ start: String(v.settings.runPorts.start), end: String(v.settings.runPorts.end) });
  }, [v]);
  if (!v) return <p className="p-4 text-sm text-muted">Loading…</p>;
  const s = v.settings;
  const error = save.error ?? token.error ?? removeToken.error;
  const tok = update.data?.token;

  return (
    <div className="flex flex-col gap-5 p-4 text-sm overflow-auto max-w-2xl">
      <h2 className="text-base font-semibold">This host</h2>
      {error && <p role="alert" className="text-xs text-failed">{error.message}</p>}

      <section aria-label="Remote access" className="flex flex-col gap-2">
        <h3 className={h}>Remote access</h3>
        <label className="flex items-center gap-2">
          <input type="checkbox" aria-label="Remote access" checked={s.remoteAccess} onChange={(e) => save.mutate({ remoteAccess: e.target.checked })} />
          <span>Serve this host on the tailnet over HTTPS</span>
        </label>
        <p className="text-xs text-muted">
          {v.remote.active && v.remote.url ? <a className="text-accent underline" href={v.remote.url}>{v.remote.url}</a> : v.remote.error ?? (s.remoteAccess ? 'Not active' : 'Off')}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-2">
            <span>Owner</span>
            <input aria-label="Owner login" className={field} value={v.ownerFromEnv ?? owner} disabled={!!v.ownerFromEnv} onChange={(e) => setOwner(e.target.value)} />
          </label>
          {!v.ownerFromEnv && <Button variant="ghost" onClick={() => save.mutate({ ownerLogin: owner })}>Save owner</Button>}
          {!v.ownerFromEnv && v.remote.tailscaleLogin && v.remote.tailscaleLogin !== s.ownerLogin && (
            <Button variant="secondary" onClick={() => save.mutate({ ownerLogin: v.remote.tailscaleLogin! })}>Use {v.remote.tailscaleLogin}</Button>
          )}
          {v.ownerFromEnv && <span className="text-xs text-muted">Set by PDT_OWNER_LOGIN</span>}
        </div>
        <Button variant="ghost" onClick={() => void api.refreshRemote().then(() => qc.invalidateQueries({ queryKey: ['host-settings'] }))}>Retry</Button>
      </section>

      <section aria-label="Updates" className="flex flex-col gap-2">
        <h3 className={h}>Updates</h3>
        <p className="text-xs text-muted">Version {update.data?.current ?? '…'}{update.data?.disabledReason ? ` · ${update.data.disabledReason}` : ''}</p>
        <label className="flex items-center gap-2">
          <span>Policy</span>
          <select aria-label="Update policy" className={field} value={s.updatePolicy} onChange={(e) => save.mutate({ updatePolicy: e.target.value as UpdatePolicy })}>
            <option value="auto_when_idle">Install automatically when idle</option>
            <option value="notify">Notify only</option>
            <option value="manual">Manual (no checks)</option>
          </select>
        </label>
        <p className="text-xs text-muted">
          Token: {tok?.state === 'missing' || !v.tokenPresent ? 'none' : tok?.state === 'expires_soon' ? `expires ${new Date(tok.expiresAt).toLocaleDateString()}` : tok?.state === 'expired' ? 'expired or revoked' : 'stored in the OS keychain'}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <input aria-label="GitHub token" type="password" autoComplete="off" className={field} value={tokenText} onChange={(e) => setTokenText(e.target.value)} placeholder="github_pat_…" />
          <Button variant="secondary" disabled={!tokenText.trim() || token.isPending} onClick={() => token.mutate(tokenText)}>Save token</Button>
          {v.tokenPresent && <Button variant="ghost" onClick={() => removeToken.mutate()}>Remove token</Button>}
          <Button variant="ghost" onClick={() => void api.checkUpdate().then((u) => qc.setQueryData(['host-update'], u))}>Check now</Button>
        </div>
      </section>

      <section aria-label="Runs" className="flex flex-col gap-2">
        <h3 className={h}>Runs</h3>
        <div className="flex flex-wrap items-center gap-2">
          <span>Ports</span>
          <input aria-label="First run port" className={`${field} w-24`} value={ports.start} onChange={(e) => setPorts({ ...ports, start: e.target.value })} />
          <span>–</span>
          <input aria-label="Last run port" className={`${field} w-24`} value={ports.end} onChange={(e) => setPorts({ ...ports, end: e.target.value })} />
          <Button variant="ghost" onClick={() => save.mutate({ runPorts: { start: Number(ports.start), end: Number(ports.end) } })}>Save ports</Button>
        </div>
        <label className="flex items-center gap-2">
          <span>Max concurrent runs</span>
          <input aria-label="Max concurrent runs" type="number" min={1} max={32} className={`${field} w-20`} defaultValue={s.maxConcurrentRuns} onBlur={(e) => save.mutate({ maxConcurrentRuns: Number(e.target.value) })} />
        </label>
      </section>

      <section aria-label="Agent credentials" className="flex flex-col gap-2">
        <h3 className={h}>Agent credentials</h3>
        <select aria-label="Host agent credentials" className={field} value={s.agentCredentials} onChange={(e) => save.mutate({ agentCredentials: e.target.value as CredentialPolicy })}>
          <option value="subscription">Subscription (remove ANTHROPIC_API_KEY and OPENAI_API_KEY)</option>
          <option value="pass_through">Pass through (API keys, Bedrock/Vertex, proxies)</option>
        </select>
      </section>

      <section aria-label="Checks" className="flex flex-col gap-1">
        <h3 className={h}>Checks</h3>
        {checks.data && (['claude', 'codex', 'git', 'tailscale'] as const).map((k) => (
          <p key={k} className={checks.data[k].ok ? 'text-running' : 'text-failed'}>
            {k}: {checks.data[k].ok ? 'OK' : 'needs attention'} <span className="text-muted">{checks.data[k].detail}</span>
          </p>
        ))}
        <Button variant="ghost" disabled={refreshChecks.isPending} onClick={() => refreshChecks.mutate()}>Run checks again</Button>
      </section>
    </div>
  );
}
```

In `views.ts`, add `'settings'` to `ViewId` and `{ id: 'settings', label: 'Settings', icon: Settings }` to `VIEWS`, importing `Settings` from `lucide-react`. In `AppShell.tsx`:
- render `<UpdateBanner />` at the top of `<main>`;
- in the `settings` view, the list column shows `<FirstRunChecklist />` and the main area shows `<HostSettingsPage />`;
- in the `sessions` view, show `<FirstRunChecklist onOpenSettings={() => setView('settings')} />` above `ProjectsList`;
- `PHASE_NOTE` excludes `'settings'` as well.

Update `AppShell.test.tsx`:
- add `'Settings'` to the rail names it checks;
- stub `GET /api/host/update`, `GET /api/host/settings` and `GET /api/host/checks` with the fixtures from `host.test.tsx`, since the shell now always renders the banner and the checklist. Unstubbed routes answer 404, so queries only error; the stub keeps the existing tests clean.

Run: `pnpm --dir web typecheck; pnpm --dir web test`
Expected: PASS.

- [ ] **Step 5: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test; pnpm --dir web typecheck; pnpm --dir web test; pnpm --dir web build`
Expected: green.

```bash
git add crates web
git commit -m "feat: Settings → This host, first-run checklist, login checks and the update banner"
```

---

### Task 15: PDT's own run configuration, the CLAUDE.md rules and the Mac mini checklist

**Files:**
- Create:
  - `.pdt/project.toml`
  - `docs/superpowers/notes/phase-2-mac-mini-checklist.md`
  - `docs/superpowers/notes/phase-2-carryover.md`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: the run configuration (Task 6) and `PDT_OWNER_LOGIN` (Task 5). PDT sets `PDT_OWNER_LOGIN` for every run when the host has an owner (Task 6, `RunVars::pairs`).
- Produces: a `pdt` run and `rust`/`web` tests for this repository, plus the hand-test procedure for the Mac mini.

- [ ] **Step 1: PDT's `.pdt/project.toml`**

```toml
# PDT's own run configuration (remote-host spec §4.3): a dev instance of PDT on its own port and data folder, reachable
# at the run's link. PDT passes PDT_OWNER_LOGIN itself, so the dev instance admits the host's owner.
[[run]]
name    = "pdt"
setup   = "pnpm install --frozen-lockfile && pnpm --dir web build && cargo build -p pdtd"
command = "cargo run -p pdtd"
web     = true
data    = true
env     = { PDT_PORT = "$PORT", PDT_DATA_DIR = "$PDT_RUN_DATA", PDT_WEB_DIR = "web/dist", PDT_ALLOWED_HOSTS = "$PDT_RUN_HOST:$PORT", PDT_SECRET_STORE = "memory" }

[[test]]
name    = "rust"
command = "cargo test"

[[test]]
name    = "web"
command = "pnpm --dir web test"
```

`PDT_SECRET_STORE=memory` keeps dev instances out of the keychain. They are not supervised, so they never update anyway.

Verify it parses and merges: add the following test to `crates/pdt-core/src/runconfig.rs` `mod tests`. It reads the real file through `include_str!`:

```rust
    #[test]
    fn pdts_own_configuration_is_valid() {
        let c = merge(Some(parse(include_str!("../../../.pdt/project.toml"), Source::File).unwrap()), None).unwrap();
        let pdt = &c.runs[0];
        assert!(pdt.web && pdt.data);
        assert_eq!(pdt.env["PDT_PORT"], "$PORT");
        assert_eq!(pdt.env["PDT_DATA_DIR"], "$PDT_RUN_DATA");
        assert_eq!(c.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["rust", "web"]);
    }
```

Run: `cargo test -p pdt-core pdts_own`
Expected: PASS.

- [ ] **Step 2: Update `CLAUDE.md`**

**Layout:** add these lines:
- `- crates/pdt-tailscale — tailscale CLI wrapper (serve, status)`
- `- crates/pdt-update — GitHub release client (token, verified downloads)`
- `- crates/pdt-host — the supervisor: runs pdtd, installs updates, rolls back`
- `- .pdt/project.toml — this repo's run configuration (dev instance + tests)`

**Running:** add these bullets:
- `pdt-host runs on each host (launchd LaunchAgent on macOS, HKCU Run key on Windows) and supervises pdtd; it installs releases pdtd hands over in <data dir>/update/request.json and rolls back if the new pdtd fails /api/health within 60 s. Logs: <data dir>/logs/pdt-host.log.`
- `Remote access (Settings → This host) runs tailscale serve for the daemon (443) and for web runs (their own port); remote requests must carry the owner's Tailscale-User-Login. PDT_OWNER_LOGIN sets the owner for dev instances.`
- `Runs: "Run" on a worktree/branch starts the .pdt/project.toml run on a port from the host's range with its own data folder; this repo's "pdt" run is a dev instance of PDT.`
- `Agent activity: Claude sessions get --settings <data dir>/sessions/<id>/claude-settings.json (hooks run "pdtd hook claude <Event>"); Codex gets -c notify=[pdtd, hook, codex-notify]. The user's own settings are never edited.`
- `Desktop app in dev: cargo build -p pdtd -p pdt-host, then cargo run -p pdt-desktop with PDT_PROFILE=dev (the tray starts pdt-host from target/debug).`

**Rules:** replace the existing "Never run or restart the *stable* pdtd…" and "Never run scripts/release-stable.*…" lines with these (spec §7):

```markdown
- Agents never stop, restart or update the stable instance, and never call its /api/shutdown (port 7420). Only pdt-host
  and the user's Update do. Never run scripts/release-stable.* without -BuildOnly/--build-only, never run a built
  installer, and never run `pdt-host install` or `pdt-host uninstall`.
- A dev instance always gets an explicit PDT_PORT and PDT_DATA_DIR: use this repo's "pdt" run (Run → pdt), or
  PDT_PROFILE=dev for a manual one. pdtd refuses a data folder another pdtd holds.
- Tests never touch port 7420 or the real data folders, and never the real keychain (PDT_SECRET_STORE=memory).
```

- [ ] **Step 3: Write the Mac mini hand-test checklist**

`docs/superpowers/notes/phase-2-mac-mini-checklist.md`, written for hermes-agent on the Mac mini. Every step is a command or an observation with an expected result. Record results inline as `PASS`/`FAIL: <what happened>` and commit the file back.

```markdown
# Phase 2 hand tests on the Mac mini

For hermes-agent. Run each step in order on the Mac mini, as the user who is logged in automatically. Record PASS or
FAIL (with what you saw) after each step and commit this file on the branch when done. Do not change anything not
named here. "The owner" is the user's Tailscale login (ask the user if unsure). Device B is a second machine on the
tailnet (the Windows desktop or the phone), operated by the user; ask them to do the B steps and report back.

## 0. Preconditions
1. `tailscale status --json | /usr/bin/python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["BackendState"], d["Self"]["DNSName"], d["CertDomains"])'`
   Expected: `Running <name>.<tailnet>.ts.net. [...]` with a non-empty CertDomains list. If it is empty, ask the user to
   enable HTTPS certificates in the Tailscale admin console (DNS page) and repeat.
2. `claude auth status` → `"loggedIn": true`; `codex login status` → exit 0. If not, ask the user to log in.

## 1. Install and the login item
1. Install the latest release: download the `.app.tar.gz` from the release page (the user provides it or a token),
   `tar -xzf` it, move `Personal Dev Team.app` to `~/Applications`, open it once.
   Expected: the browser opens http://127.0.0.1:7420/; `ls ~/Library/LaunchAgents/dev.personaldevteam.host.plist`
   exists; `launchctl print gui/$(id -u)/dev.personaldevteam.host | grep state` shows `running`.
2. `pgrep -fl pdt-host; pgrep -fl "MacOS/pdtd"` → one of each; pdt-host runs from
   `~/Library/Application Support/pdt/host/pdt-host`.
3. Quit the tray (menu → Quit). `curl -s http://127.0.0.1:7420/api/health` still answers (pdtd keeps running).
4. `kill -9 $(pgrep -f "MacOS/pdtd")`; within 10 s `curl -s http://127.0.0.1:7420/api/health` answers again, and
   `tail -5 ~/Library/Application\ Support/pdt/logs/pdt-host.log` shows "pdtd exited; restarting".
5. Log out and back in (or ask the user to). Expected: health answers within a minute without opening the app.
6. Reboot (ask the user first). Expected: after auto-login, health answers within two minutes.

## 2. Remote access and identity
1. In Settings → This host: click "Use <owner>", then turn on Remote access. Expected: status shows
   `https://<machine>.<tailnet>.ts.net/`; `tailscale serve status` lists `https://<machine>…` → `http://127.0.0.1:7420`.
2. Check whether `tailscale serve --https=443 off` without `--bg` removes the mapping: turn Remote access off in
   Settings and run `tailscale serve status`. Expected: no 443 mapping. Record the result: it settles whether the
   off form needs `--bg` (verified fact 5 left this open). Turn it back on.
3. Device B (owner): open the URL. Expected: PDT loads; a Claude session starts and its terminal works.
4. Device B with a different tailnet login (or a shared-device user who is not the owner): open the URL.
   Expected: 403 `forbidden_identity` naming that login.
5. From a tagged device if one exists (e.g. a server with `tag:`): `curl -s https://<machine>.<tailnet>.ts.net/api/health`.
   Expected: 403 `forbidden_identity` ("no Tailscale user identity").
6. On the Mac mini itself: `curl -s http://127.0.0.1:7420/api/host/whoami` → `{"caller":"local",…}`.

## 3. Runs
1. Add this repository as a project (if not already), open a worktree for a branch, choose Run → pdt.
   Expected: setup log streams; state goes setting_up → starting → ready; the link is
   `https://<machine>.<tailnet>.ts.net:84xx/`; `tailscale serve status` lists that port.
2. Device B: open the run's link. Expected: a second PDT (the dev instance) loads and admits the owner.
3. Stop the run. Expected: state stopped; the port's mapping is gone from `tailscale serve status`;
   `lsof -iTCP:<port> -sTCP:LISTEN` prints nothing.
4. Start two runs on two branches; both get different ports and work side by side.

## 4. Activity
1. Start a Claude session, type a prompt. Expected: "Working" while it runs, then "Idle" with the unseen dot on device B;
   viewing it on the Mac mini clears the dot on device B within a few seconds.
2. Ask Claude to run a shell command that needs approval. Expected: "Needs you" within ~1 s.
3. Same with Codex: Enter → Working; an approval prompt → Needs you; turn end → Idle.

## 5. Update and rollback
1. Settings → This host: paste the fine-grained token; policy "notify". Expected: token "stored in the OS keychain"
   (Keychain Access shows an item for service `dev.personaldevteam`); no expiry warning unless < 14 days.
2. Merge any small PR so CI publishes v0.1.N+1 (ask the user). Within an hour, or after "Check now", every device shows
   "Update to v0.1.N+1 available" with the notes.
3. With a Claude session working, click Update. Expected: a confirm listing the busy session. Confirm.
   Expected: the page reconnects; `curl -s http://127.0.0.1:7420/api/health` reports the new version; the session
   resumed; the banner says "Updated to v0.1.N+1"; `ls ~/Applications` shows `Personal Dev Team.app.previous`.
4. Forced rollback: on the Mac mini, with the user's permission, prepare a release that fails its health check (ask the
   user to publish one, or simulate: copy the current `.app.tar.gz` to a temp folder, replace `Contents/MacOS/pdtd`
   inside it with a script `#!/bin/sh\nsleep 600`, re-tar it, and write `~/Library/Application Support/pdt/update/request.json`
   with `{"version":"9.9.9","tag":"v9.9.9","asset":"<that tar.gz>","fromVersion":"<current>","requestedAt":0}`, then
   `curl -X POST http://127.0.0.1:7420/api/shutdown` — this is the one sanctioned shutdown in this checklist).
   Expected: within ~70 s the previous version answers health again; the banner shows "Update to v9.9.9 failed and was
   rolled back" with the log path.
```

- [ ] **Step 4: Write the carry-over notes**

`docs/superpowers/notes/phase-2-carryover.md`:

```markdown
# Phase 2 → later carry-over

- **Verify by hand (Mac mini checklist):** `tailscale serve --https=<port> off` without `--bg`; the live format of
  GitHub-Authentication-Token-Expiration; the Tauri NSIS installer's behaviour with the tray running during a silent
  update; launchd KeepAlive restarting pdt-host after self-replacement.
- **Codex activity is screen-read** (approval markers in pdt-core::activity); Phase 3's app-server adapter replaces it.
- **Runs restart only on request** when their branch changes; dev servers with hot reload pick up changes themselves.
- **Fork PRs** cannot be run until the git integration phase (only branches on origin).
- **Owner identity** compares Tailscale-User-Login case-insensitively; RFC 2047 values are decoded (Q only).
- **Windows rollback** restores a copy of the install folder; the uninstall registry entry keeps the newer version.
- **Tagged-device access** is refused; app-capability headers (Tailscale ≥ 1.92) could allow chosen tags later.
```

- [ ] **Step 5: Full check and commit**

Run: `cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test; pnpm --dir web typecheck; pnpm --dir web test; pnpm --dir web build; cargo clippy -p pdt-desktop --all-targets -- -D warnings; cargo test -p pdt-desktop`
Expected: all green, and `git status --porcelain -- web/src/api/gen` is empty.

```bash
git add .pdt CLAUDE.md docs crates/pdt-core/src/runconfig.rs
git commit -m "docs: PDT's run configuration, Phase 2 rules in CLAUDE.md, Mac mini hand-test checklist"
```

Push the branch and check that CI is green on Windows and macOS: `git push -u origin phase-2-remote-host`, then `gh run list --branch phase-2-remote-host`.
