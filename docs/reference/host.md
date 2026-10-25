# Host: daemon, desktop tray, releases, updates, remote access

How pdtd, pdt-host and the tray app run on a machine. Reference for work on crates/pdtd, crates/pdt-host,
crates/pdt-update, crates/pdt-tailscale and desktop/; AGENTS.md has the rules that apply to every change.

## The daemon

- A bare pdtd serves no UI unless `PDT_WEB_DIR` is set or a `web/` dir sits next to the exe (or `../Resources/web`,
  the macOS bundle layout; crates/pdtd/src/config.rs).
- Data lives in `<local data dir>/pdt` (stable) or `pdt-dev`; a `pdtd.lock` file prevents two daemons sharing a dir
  (pdt-host keeps its own `pdt-host.lock`).
- Logs: `<data dir>/logs/pdtd.<date>.log` (daily, 14 kept) and `pdt-host.log`; a spawned pdt-host's and pdtd's
  startup output goes to `<data dir>/logs/pdtd-launch.log`; on macOS launchd's output goes to `pdt-host.launchd.log`.
  pdtd logs to stderr too unless pdt-host started it (`PDT_SUPERVISED=1`), so `pdtd-launch.log` holds only what is
  written before logging starts and panics (a panic hook also logs every panic's message and place to
  `pdtd.<date>.log`). pdt-host moves `pdt-host.log` and `pdtd-launch.log` to `<name>.1`
  (replacing the one before) once they pass 5 MB.
- Database (`<data dir>/pdt.db`, SQLite in WAL mode, `synchronous=NORMAL`): before applying a pending migration pdtd
  copies it to `<data dir>/backups/pdt-<ms>.db` (`VACUUM INTO`) and keeps the newest 3 copies. Every 10 minutes
  `maintain_workflows` empties the WAL (`wal_checkpoint(TRUNCATE)`), so it cannot grow between restarts while
  readers keep SQLite's own checkpoints from resetting it. Housekeeping prunes events, finished workflow runs,
  resolved outbox messages, ended Inbox items and notifications after 30 days, and conversation frames after 7
  ([workflows.md](workflows.md), [conversations.md](conversations.md)).
- Background loops (PDT-127, pdt-api `loops.rs`): `poll` (every 60 s, after the startup resume of runs and sessions:
  gone runs, PRs and CI, Buildkite, merges, watched issues, Jira, workflow upkeep, in that order), `engine` (2 s),
  `overlaps` (30 s), `activity` (5 s), `notifier` (2 s), `scheduler` (10 s), `conversations` (30 s). Each pass runs in
  its own task: a panic ends that pass only, logged at error level, and the loop goes on. Each member of `poll` is
  limited to 2 minutes (aborted and logged past it), so one hung member never holds up merge detection or pruning;
  the Jira and Buildkite sensors' calls time out after 30 s each, and the shared HTTP client (nodes, sensors, MCP)
  gives up connecting after 10 s. A loop is stale when its last finished pass (or its start) is more than five
  periods ago, at least 5 minutes (`poll`: 20 minutes; pdt-core `liveness.rs`); the notifier then raises a
  `host.loop_stale` notification per loop ([workflows.md](workflows.md#notifications)), cleared by its next pass. The
  update loop is restarted a minute after a panic. A run's driver that panics fails the run, so its row and Stop
  never hang.
- `/api/health` reports `status`, `name`, `version`, `profile` (`dev`/`stable`), `port`, `dev` (dev profile or a
  non-stable port), `webBuild` (the first 12 hex digits of the served index.html's SHA-256, read per request; null
  without a web dir) and `loops` (each background loop's `name`, `everyMs`, `startedAt`, `lastPassAt`,
  `staleAfterMs`, `panics` and `stale`; `status` stays `ok` with a stale loop, so pdt-host does not restart pdtd). The UI shows a `DEV · <port>` badge in the rail and a `[dev]` title prefix when `dev` is true.
  It polls health every 5 s and reloads when `webBuild` changes from the one it loaded with (a dev instance restarted
  with a rebuilt web/dist; every dev build reports version 0.1.0), held back while a modal is open or a focused text
  field holds text (web/src/host/useReloadOnNewBuild.ts).
- Demo data: `PDT_SEED=demo` (set by the "pdt" run) makes a dev-profile pdtd with no projects create two demo git repos
  in `<data dir>/demo` (branches, a worktree with uncommitted work) and register them with ended sessions, past runs
  and Sample App tickets (ten, plus an "Accounts" parent with three of them as parts) (crates/pdtd/src/seed.rs).
  Nothing is started; a database with any project is left alone; ignored outside the dev profile.

## Remote access and the Host guard

- `PDT_ALLOWED_HOSTS` (comma-separated) adds Host header values accepted beyond `127.0.0.1:<port>`, `localhost:<port>`
  and `[::1]:<port>` (DNS-rebinding guard). The dev profile also accepts the Vite dev server (`localhost:5173`,
  `127.0.0.1:5173`) in its Host/Origin allowlist.
- Remote access (Host settings) maps the daemon over `tailscale serve --bg`: the stable instance on https 443, any
  other port (dev, explicit `PDT_PORT`) on https `<its own port>`. With it off, pdtd runs no `tailscale serve` command
  (read-only `tailscale status` still runs for the checks); turning it off removes only the mappings that process
  made. `PDT_TAILSCALE_BIN` sets the tailscale CLI path (tests point it at `fake-tailscale` or a missing file so they
  never touch the real Tailscale); otherwise it is found on PATH or in the standard install location.
- pdtd applies the setting once at startup, then the `remote` loop (`run_remote_watch`, pdt-api remote.rs; in
  /api/health's `loops`) keeps it in step with Tailscale, which often comes up after pdtd at login (PDT-148). While
  the setting is on but remote access is not active (Tailscale stopped or not logged in, the CLI timing out) it applies
  the setting again after 30 s, doubling to at most 5 min; while active it asks `tailscale status` every 3 min and
  applies again only when Tailscale stopped answering, HTTPS went off or the DNS name changed (the old name leaves the
  allowlist). With the setting off it runs no tailscale command. `POST /api/host/remote/refresh` still applies at once.
- Remote requests must carry the owner's Tailscale-User-Login (case-insensitive); tagged/Funnel/proxied requests
  without it get 403 `forbidden_identity`. The exception is `/api/hooks/*` (workflow webhooks, Phase 7e): any request but a
  Funnel one gets through, and the hook's secret authenticates it ([workflows.md](workflows.md) "Sensors and
  webhooks"). `PDT_OWNER_LOGIN` sets the owner for dev instances (PDT passes it to every
  run).
- Speed: pdtd sets TCP_NODELAY on every connection (keystroke echoes are tiny frames), compresses HTTP responses
  (gzip/br, quality 4), serves `/assets/*` (content-hashed) with `Cache-Control: immutable` and index.html with
  `no-cache`.

## Desktop tray

The desktop app is an optional tray view of the host (decision 16; no window, no Dock icon on macOS, single instance
per profile; a second launch just opens the UI). pdt-host, not the tray, keeps pdtd running.

- At startup the tray starts pdt-host (`pdt-host run`: on Windows the `<data dir>/host` copy if present, else the one
  beside its exe; on macOS always the bundle's) when /api/health doesn't answer, then opens the UI (not when pdt-host
  started it after an update). On a first launch of the installed stable app on macOS it registers the LaunchAgent
  itself (`pdt-host install`, desktop/src-tauri/src/daemon.rs).
- Start at login (PDT-148): the stable app registers itself to start at login (with `--no-open`, so a login opens no
  browser window) at every start, unless the "Start at login" item was turned off (that leaves
  `<data dir>/tray-login-off`). See "pdt-host at login". Not offered for dev, outside an app bundle on macOS, or from a
  misplaced bundle.
- Open (also left-click on Windows) opens `http://127.0.0.1:<port>/` in the default browser. Restart daemon posts
  /api/shutdown and pdt-host starts pdtd again. The version item shows pdtd's version. Quit exits only the tray;
  pdt-host and pdtd keep running.
- In dev: `cargo build -p pdtd -p pdt-host`, then `cargo run -p pdt-desktop` with `PDT_PROFILE=dev` and `PDT_WEB_DIR`
  set (debug builds default to the dev profile when `PDT_PROFILE` is unset). `cargo test -p pdt-desktop` and the
  desktop build need `pnpm --dir web build` first; plain `cargo test` skips the desktop crate.

## Releases and the stable app

- Every push to main publishes GitHub release `v0.1.<run number>` of ross-game-tools/PersonalDevTeam
  (`*-setup.exe`, `*.app.tar.gz`), built with that version (`PDT_BUILD_VERSION`, read at compile time). PR and local
  builds keep the checked-in 0.1.0.
- scripts/release-stable.ps1 (Windows) / scripts/release-stable.sh (macOS) build the installer (Tauri; pdtd and
  pdt-host are sidecars via desktop/src-tauri/tauri.bundle.conf.json). When pdt-host supervises the stable pdtd they
  hand the build to it as an update; otherwise they stop the stable tray app and pdtd, install and launch.
  `-BuildOnly` / `--build-only` only builds (the only way agents run them).
- macOS releases are signed with one certificate (scripts/sign-macos.sh, CI secrets `MACOS_SIGNING_P12`/`_PASSWORD`;
  notarized by scripts/notarize-macos.sh when `MACOS_NOTARY_KEY`/`_KEY_ID`/`_ISSUER` are set). Today it is
  self-signed, which has no Team ID, so the keychain partition list trusts each build by code hash and asks again after
  every update; a Developer ID certificate fixes that (docs/superpowers/notes/developer-id.md).

## Updates

- The tray's Update item follows pdtd's update API (`GET /api/host/update`; a click checks, or installs via
  `POST /api/host/update/install`, asking again with `force` when the host is busy).
- pdtd checks GitHub releases every `updateCheckMinutes` (Host settings → Updates, hourly by default; re-read every
  minute) and when asked, with the token stored in the OS keychain (account `update-token` for the stable data
  dir, `update-token:<hash>` for any other, so a dev pdtd never sees stable's token) and hands the verified download
  to pdt-host, which installs it, health-checks the new version and rolls back on failure. pdt-host copies the
  database to `update/db-before-update` before installing and restores it on rollback (an old version cannot open a
  database a newer one migrated).
- Keychain calls time out after 5 s and the startup token read runs in the background, so a keychain prompt never
  blocks serving.
- Updates are offered only when pdt-host supervises a stable pdtd; dev instances never update (and a dev pdt-host
  never installs).
- The web UI reloads itself when `/api/host/update` reports a different `current` version than it loaded against
  (polled every 2 s while downloading/installing, else every 60 s and on window focus); index.html is `no-cache`, so
  the reload gets the new bundle.

## Bug reports (PDT-153)

"Report a bug" (the palette's Actions, the Keyboard shortcuts dialog, Host settings → Bug reports; pdt-api
`bug_reports.rs`, pdt-core `bug_report.rs`, web `host/BugReportDialog.tsx`) files a report about PDT itself.

- **Where it goes.** Host settings' `bugReports`: `auto` (default), `github`, `local` or `off`. `auto` is `github`,
  except when a project here has `origin` on the release repository (`RELEASE_REPO`; the update client's repository):
  the instance PDT is developed on, where it is `local`. `GET /api/bug-reports/preview` says which (`setting`,
  `route`, `repo`, `project`, `tokenPresent`) and returns the diagnostics as they would be sent; the form says where
  the report goes before it is sent.
- **Filing.** `POST /api/bug-reports {title, description?, includeDiagnostics?, nonce?}`. `github`: an issue on the
  release repository with labels `bug` and `from-pdt`, filed with the update token (pdt-update `create_issue`; the
  token goes only to the API host and is never logged) → `{kind: "issue", number, url}`. `local`: a Draft ticket
  labelled `bug` in that project (diagnostics in its description), with no GitHub request → `{kind: "ticket", key, id,
  projectId}`. `off` → 409 `bug_reports_off`; `local` with no such project → 409 `no_bug_project`. The same `nonce`
  within a minute returns the first report instead of filing another (the form sends one per form; pdt-mcp's
  `report_bug` uses the session and title).
- **Errors.** No token → 409 `no_token`; GitHub rejecting it → 409 `token_rejected`; a 403/404 (no Issues: write) →
  409 `token_no_issue_access`; each names Host settings → Updates, and the form offers a Host settings button. GitHub
  unreachable → 502 `github_unreachable` ("Try again").
- **Diagnostics** (opt-in, shown in the form): version, profile (and whether it is a development instance), OS and
  architecture, supervised by pdt-host, update state and token state, and the last 50 lines of the newest
  `logs/pdtd.<date>.log`. No ticket or project content. The log lines and update error are redacted
  (`bug_report::redact`): the update token and owner login by value, token-shaped words (`ghp_`, `github_pat_`,
  `sk-`…), values after secret-looking keys (`token=`, `"apiKey":`, `Authorization: Bearer …`), long random-looking
  words, the home folder (as `~`) and other users' (`/Users/[user]`), and this machine's names (hostname, its short
  form, the tailnet name, any `*.ts.net`) as `[host]`.

## pdt-host at login

- `pdt-host install` registers a LaunchAgent that runs the app bundle's pdt-host (macOS), or copies itself to
  `<data dir>/host` and registers that copy in an HKCU Run value (Windows; the NSIS hooks in
  desktop/src-tauri/windows/hooks.nsh run it).
- macOS never copies or rewrites the registered program (PDT-40: SentinelOne flags a login item whose program changes
  as "persistence deception"); after an update pdt-host just restarts from the new bundle. An older install's agent
  naming `<data dir>/host/pdt-host` is rewritten by pdt-host at startup, which then execs the bundle's pdt-host
  (`--from-copy`); the old copy is removed once launchd starts the bundle's directly (next login) or on
  install/uninstall.
- A bundle macOS runs from an App Translocation folder, a folder pdt-host cannot write to (e.g. a disk image) or that
  has gone is misplaced (PDT-51, pdt-host `placement.rs`): it is never registered at login, `installer_for` refuses it
  and pdt-host passes the reason to pdtd as `PDT_UPDATES_UNSUPPORTED` (updates then show as off with it); the tray says
  to move the app to Applications.
- The tray app has its own login item (PDT-148, pdt-host `tray_login.rs`): a LaunchAgent
  `~/Library/LaunchAgents/<TRAY_LABEL>.plist` running the bundle's tray exe with `--no-open` (RunAtLoad, no KeepAlive,
  so Quit stays quit; only written, never bootstrapped: launchd reads it at the next login), or an HKCU Run value
  `<TRAY_AUTOSTART_NAME>` (Windows). The tray writes it at startup and removes it when "Start at login" is turned off;
  `pdt-host uninstall` removes it too.
- pdt-host writes its program path to `<data dir>/pdt-host.program`; a tray whose bundle differs from the running
  pdt-host's offers "switch to this one" (asking at startup when the other copy is misplaced), which runs
  `pdt-host install --replace` (boots out and stops the other pdt-host, bootstraps this bundle's) and restarts pdtd.
