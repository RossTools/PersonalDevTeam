# Host: daemon, desktop tray, releases, updates, remote access

How pdtd, pdt-host and the tray app run on a machine. Reference for work on crates/pdtd, crates/pdt-host,
crates/pdt-update, crates/pdt-tailscale and desktop/; AGENTS.md has the rules that apply to every change.

## The daemon

- pdtd is built with a cargo feature per integration (discord, jira, buildkite, arr, scryer, plex; all on by default,
  so release builds have them all) and logs which it has and which this instance enabled at start (Settings →
  Integrations, the host value `integrations.enabled`; [workflows.md](workflows.md) "Integrations").
- A bare pdtd serves no UI unless `PDT_WEB_DIR` is set or a `web/` dir sits next to the exe (or `../Resources/web`,
  the macOS bundle layout; crates/pdtd/src/config.rs).
- Data lives in `<local data dir>/pdt` (stable) or `pdt-dev` (`dirs::data_local_dir()`: `~/Library/Application
  Support` on macOS, `%LOCALAPPDATA%` on Windows, `$XDG_DATA_HOME` or `~/.local/share` on Linux); a `pdtd.lock` file prevents two daemons sharing a dir
  (pdt-host keeps its own `pdt-host.lock`).
- Logs: `<data dir>/logs/pdtd.<date>.log` (daily, 14 kept) and `pdt-host.log`; a spawned pdt-host's and pdtd's
  startup output goes to `<data dir>/logs/pdtd-launch.log`; on macOS launchd's output goes to `pdt-host.launchd.log`.
  pdtd logs to stderr too unless pdt-host started it (`PDT_SUPERVISED=1`), so `pdtd-launch.log` holds only what is
  written before logging starts and panics (a panic hook also logs every panic's message and place to
  `pdtd.<date>.log`). pdt-host moves `pdt-host.log` and `pdtd-launch.log` to `<name>.1`
  (replacing the one before) once they pass 5 MB.
- Database (`<data dir>/pdt.db`, SQLite in WAL mode, `synchronous=NORMAL`): before applying a pending migration pdtd
  copies it to `<data dir>/backups/pdt-<ms>.db` (`VACUUM INTO`) and keeps the newest 3 copies, so a migration that
  rewrites data (PDT-174's move of the Jira and Buildkite settings) can be undone by hand. Every 10 minutes
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
  with a rebuilt web/dist; every dev build reports the checked-in version), held back while a modal is open or a focused text
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
- Icons (PDT-199): the app icon set in desktop/src-tauri/icons is generated from desktop/app-icon.svg
  (`pnpm --dir desktop tauri icon <path to app-icon.svg> -o <scratch dir>`, then copy over the files tauri.conf.json
  lists and icon.png). On macOS the tray shows icons/tray-template.png instead, a template image (the system tints it
  for light and dark menu bars) rendered from desktop/tray-template.svg at 60×36 (18 pt high at 2×); elsewhere it shows
  the app icon. The web UI's favicon is web/public/favicon.svg.
- Open (also left-click on Windows) opens `http://127.0.0.1:<port>/` in the default browser. Restart daemon posts
  /api/shutdown and pdt-host starts pdtd again. The version item shows pdtd's version. Quit exits only the tray;
  pdt-host and pdtd keep running.
- In dev: `cargo build -p pdtd -p pdt-host`, then `cargo run -p pdt-desktop` with `PDT_PROFILE=dev` and `PDT_WEB_DIR`
  set (debug builds default to the dev profile when `PDT_PROFILE` is unset). `cargo test -p pdt-desktop` and the
  desktop build need `pnpm --dir web build` first; plain `cargo test` skips the desktop crate.

## Releases and the stable app

- Releases are deliberate; merges don't ship (docs/SDLC.md "Releases"). A merge to main builds and publishes nothing.
  A release is GitHub release `vX.Y.Z` (SemVer, no pre-releases) of the release repository (`RELEASE_REPO` in
  brand.rs), with `*-setup.exe`, `*.app.tar.gz`, `pdt-linux-<arch>.tar.gz` and `pdt-linux-<arch>.AppImage`
  (`x86_64`, `aarch64`) and `SHA256SUMS` listing them all. Each host installs its own (pdt-core `select_asset`: on
  Linux by exact name for its CPU and package, see "Linux"). Its version is the workspace version in
  `Cargo.toml`, which `desktop/src-tauri/tauri.conf.json` repeats (`scripts/version.sh check`, run in CI); it changes
  only in a release PR. A release build reports it as is (`PDT_BUILD_VERSION`, read at compile time); every other
  build reports `<version>-local`. Until 0.2.0, every push to main published `v0.1.<run number>`, with its head
  commit's message as notes; installs on those move to 0.2.0 like to any newer release.
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
- Versions compare by SemVer (major, then minor, then patch: 0.2.0 is newer than 0.1.840; an experimental build
  `X.Y.Z-exp.N` sorts after `X.Y.Z-exp.M` for M below N and before `X.Y.Z` itself). A check reads the newest release
  on the host's update channel (below) and offers it only when it is newer than the running version: an older or
  equal release is never installed (no downgrade).
- Update channel (PDT-210; Host settings → Updates → Update channel, `updateChannel` in host settings, `stable` by
  default):
  - `stable` reads `releases/latest`, which is never a draft or pre-release (a tag `vX.Y.Z-exp.N` there is refused).
  - `experimental` reads the first page of `releases` (100, newest first) and takes the newest by SemVer of the full
    releases and the experimental pre-releases (`vX.Y.Z-exp.N`, published as GitHub pre-releases by the release
    workflow; docs/SDLC.md "Experimental releases"). Drafts and other pre-releases (a fork's `test-v…`) never count.
    Because `0.2.1` sorts above `0.2.1-exp.3`, an experimental host moves to the stable release when it ships.
  - Switching back to `stable` never downgrades: a host on an experimental build stays on it until a newer stable
    release exists, and Settings says so ("stays on the experimental build … until a newer stable release exists").
    Settings checks again as soon as the channel changes, so an update the other channel offered doesn't linger.
  - `/api/host/update` has `channel`, `currentExperimental` (the running version is an `-exp.N` build) and
    `latest.experimental`; the Version row in Settings, its "available" badge and the update banner show
    "Experimental" for them.
- Release notes (PDT-192): when a check finds a newer release, pdtd lists the repository's releases (`releases`,
  newest first, until it reaches the installed version; 10 pages of 100 at most) and `latest.releases` in
  `/api/host/update` holds every published release after the installed version up to the new one, newest first
  (`version`, `tag`, `experimental`, `notes`, `publishedAt`; drafts and pre-releases left out, except the new release
  itself when it is an experimental build: its notes preview everything unreleased, so its earlier `-exp.N` builds
  would only repeat them). The list is cached for that pair of
  versions. A local build, or a failed call (not cached, not an update `error`), leaves it null. The update banner's
  Release notes show each of those releases with its notes (Markdown), and fall back to the commit list below, then
  the new release's own `notes`.
- Commits (PDT-169): when a check finds a newer release, pdtd also asks GitHub to compare the installed version's
  tag (`v0.1.741`, or the commit of a dev build `0.1.741+g<sha>`) with the new tag, and `latest.changes` in
  `/api/host/update` lists every commit between them, newest first (`subject`, `title`, `ticket` key, `pr`, `prUrl`,
  `sha`). The comparison is cached for that pair of versions; each answer looks the ticket keys up on this instance
  (`ticketHere`). Without release notes, the update banner's Release notes show "N changes since v…" with ticket links (to tickets here)
  and PR links; GitHub sends at most 250 commits, so a longer range has `moreUrl` (its compare page, "…and N more").
  When the comparison fails (the tag is gone, a local build, GitHub errs) `changes` is null and the release's own
  `notes` show as before; a failure is not cached and does not set the update `error`.
- The release repository (PDT-195): pdtd reaches it by its numeric id first (`RELEASE_REPO_ID` in brand.rs,
  `/repositories/<id>/…`, which survives renames, org renames and transfers), then by `RELEASE_REPO`'s
  `/repos/<owner>/<name>/…` when the id path fails (404 or another refusal; never after a rejected token or a network
  error). API requests follow redirects (301, 302, 307, 308) only while they stay on the API host (`api.github.com`,
  or the Enterprise API host), with the token, for 3 hops at most; a redirect to another host is not followed and the
  check fails with "GitHub redirected the request…". Each hop is logged with its old and new location (no query
  strings). A permanent redirect (301/308) to another repository path is remembered as where it moved
  (`releaseRepo.movedTo` in `/api/host/update`) and tried first until pdtd restarts. Asset downloads keep their own
  rule: no API redirect is followed with the token; the storage redirect is followed without it. Bug reports use the
  same client, so they reach a moved repository the same way.
- Emergency override, for a move redirects cannot cover (someone claimed the old name): Host settings → Updates →
  Release repository (`releaseRepo`, an `owner/name`; blank is the built-in one), or `PDT_RELEASE_REPO` in pdtd's
  environment, which wins over the setting. An override is reached by name only (the id belongs to the built-in
  repository). `/api/host/update` `releaseRepo` says which repository is used (`repo`, `id`, `movedTo`,
  `overriddenBy`: `setting` or `environment`), and Settings shows it.
- Keychain calls time out after 5 s and the startup token read runs in the background, so a keychain prompt never
  blocks serving. Calls run side by side, each with its own limit (`secrets::TimeLimited`, PDT-215): only a call that
  really timed out and is still running makes further calls for the same secret fail at once (and, once 4 are stuck,
  every call), so concurrent reads never fail for being busy. The error names the secret ("did not answer in time for
  the update token", the GitHub login, the connection or secret by name). What pdtd read is kept in memory
  (`secrets::Cached`) and replaced or forgotten when pdtd sets or deletes it (a reconnect, a sign-out), so pollers
  (Settings, the update banner, connections' sensors) do not ask the keychain each time; a failed read is not kept.
  The update banner and Settings clear a keychain error on the next successful read.
- Updates are offered only when pdt-host supervises a stable pdtd; dev instances never update (and a dev pdt-host
  never installs).
- The web UI reloads itself when `/api/host/update` reports a different `current` version than it loaded against
  (polled every 2 s while downloading/installing, else every 60 s and on window focus); index.html is `no-cache`, so
  the reload gets the new bundle.
- A page that still runs the old bundle (a phone tab woken before that check runs) can't load a lazy view's chunk
  the new build no longer has: Vite's `vite:preloadError` reloads the page then, at most once every 30 s
  (web/src/common/staleBuild.ts, PDT-209). Otherwise the view's error boundary says a newer version is out, with
  Reload.

### Renaming or moving the release repository

GitHub redirects a renamed or transferred repository's API requests, and every install since PDT-195 follows them
and reaches the repository by its id anyway. Older installs (0.1.778 and before) follow nothing and need a manual
install. To move it:

1. Ship a release that knows the new location **before** the rename: `RELEASE_REPO` set to the new `owner/name`
   (`RELEASE_REPO_ID` stays: the id never changes), and let the installs take it.
2. Rename or transfer the repository.
3. Never create a repository under the old name again: that ends GitHub's redirects for it.
4. Check one install's update status afterwards (Settings → This host → Updates: a recent "Last checked" and no
   error; `releaseRepo` in `GET /api/host/update`). An install that cannot find it can be pointed at the new
   repository with the Release repository setting.

## Upgrade rehearsal (PDT-179)

Before installing a release with migrations or plugin changes, `scripts/rehearse-upgrade.sh` runs this checkout's
pdtd (built with `cargo build -p pdtd`, or `--bin`) against a copy of a real data dir and prints a pass/fail report
(Markdown; `--report FILE` saves it). Exit status 0 when every check passes, 1 when one fails, 2 when it could not run.

- The copy: the source (`--from`, the stable data dir by default) is only read. Its database is copied with SQLite's
  online backup (consistent while the stable instance runs), plus `sessions/` and `pm/`, into a temp folder that is
  removed afterwards (`--keep` keeps it and the instance's log).
- The instance: a dev pdtd on port 7421 (`--port`; it refuses 7420 and a port in use) with `PDT_REHEARSAL=1`.
  Rehearsal mode needs the dev profile and an explicit `PDT_DATA_DIR` other than the real `pdt`/`pdt-dev` folders; it
  migrates the copy, installs the built-in workflows and personas for what is enabled and serves the API, and does
  nothing else: no run or session resumes, no background loop starts (poll, workflow engine, scheduler, notifier,
  remote access, host checks, updates), and secrets are kept in memory, so the keychain is never read and the copy's
  repos, processes and connections are never acted on. The script also drops every `PDT_*` variable of its shell and
  points the agent programs and Tailscale at nothing.
- The checks: every migration in `crates/pdt-store/migrations` applied; every integration a connection or a user
  workflow uses is enabled (an install that never chose gets the ones it uses, `integrations.enabled`); every
  connection loads, and each with a secret is reported as "needs secret" (its value is in the keychain); every
  workflow validates (`POST /api/workflows/validate`, as `workflow_validate`) and each trigger dry-runs on the newest
  of its last 20 logged events that matches it (`schedule` and `manual` on made-up ones; a type with no logged event
  on an empty made-up one, reported as untested when it fails); each enabled integration's tools (`GET
  /api/integrations` `tools`/`pmTools`) are in the agents' list (`GET /api/personas/tools`) and the PM persona's; with
  no token, `GET /api/github/login` is not connected (Settings offers Connect GitHub); the UI is served when
  `web/dist` is built.

## Bug reports (PDT-153)

"Report a bug" (the palette's Actions, the Keyboard shortcuts dialog, Host settings → Bug reports; pdt-api
`bug_reports.rs`, pdt-core `bug_report.rs`, web `host/BugReportDialog.tsx`) files a report about PDT itself.

- **Where it goes.** Host settings' `bugReports`: `auto` (default), `github`, `local` or `off`. `auto` is `github`,
  except when a project here has `origin` on the release repository (`RELEASE_REPO`; the update client's repository):
  the instance PDT is developed on, where it is `local`. `GET /api/bug-reports/preview` says which (`setting`,
  `route`, `repo`, `project`, `tokenPresent`) and returns the diagnostics as they would be sent; the form says where
  the report goes before it is sent.
- **Filing.** `POST /api/bug-reports {title, description?, expected?, steps?, includeDiagnostics?, nonce?}`. The body
  (`bug_report::body`) has the sections of the GitHub bug form (`.github/ISSUE_TEMPLATE/bug.yml`; a test keeps the
  labels in step): `### What happened` (`description`), `### What you expected`, `### How to reproduce`, `### PDT
  version`, `### OS` (only with the diagnostics, else `_Not shared_`; an empty one is `_No response_`, as GitHub
  writes it), then the diagnostics. The repository's issue forms don't add `from-pdt`, so triage sees only reports
  filed from PDT. `github`: an issue on the
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

- `pdt-host install` registers a LaunchAgent that runs the app bundle's pdt-host (macOS), copies itself to
  `<data dir>/host` and registers that copy in an HKCU Run value (Windows; the NSIS hooks in
  desktop/src-tauri/windows/hooks.nsh run it), or writes and enables a systemd user service running the install
  folder's pdt-host (Linux, below).
- macOS never copies or rewrites the registered program (PDT-40: SentinelOne flags a login item whose program changes
  as "persistence deception"); after an update pdt-host just restarts from the new bundle. An older install's agent
  naming `<data dir>/host/pdt-host` is rewritten by pdt-host at startup, which then execs the bundle's pdt-host
  (`--from-copy`); the old copy is removed once launchd starts the bundle's directly (next login) or on
  install/uninstall.
- macOS Local Network permission (PDT-156): since macOS 15 a program needs the user's permission to connect to an
  address on the local network (private ranges, link-local, `.local`), or the connection fails with "No route to
  host" (os error 65). launchd's own exemption is for daemons, not LaunchAgents (Apple's TN3179), so the agent names
  the app bundle in `AssociatedBundleIdentifiers` (`APP_BUNDLE_ID`): pdt-host and the pdtd it starts count as the app,
  macOS asks once with the app's `NSLocalNetworkUsageDescription` (desktop/src-tauri/Info.plist, merged into the
  bundle's by Tauri) and lists "Personal Dev Team" in System Settings › Privacy & Security › Local Network. An older
  agent without the key is rewritten at startup like the one naming the copy (launchd reads it at the next login). The
  choice is recorded against the app's code signature, so updates signed with the same certificate and bundle id keep
  it. pdtd brings the alert up while the user is at the Mac: at startup and when a connection is saved it connects a
  UDP socket (no packet sent) to the first connection address on the local network (pdt-api `local_network.rs`); with
  no local connection nothing asks. A request blocked by it (EHOSTUNREACH to a local host on macOS) fails with "macOS
  Local Network permission is blocking Personal Dev Team: System Settings › …" instead of the raw error
  (`send_error`), in the connection's `lastError`, steps and the MCP tools (`scryer_*`).
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

## Linux (PDT-197)

Two release assets per CPU (`x86_64`, `aarch64`; built on native runners, Ubuntu 22.04, so glibc 2.35 or newer):
`pdt-linux-<arch>.tar.gz`, headless, and `pdt-linux-<arch>.AppImage`, the desktop app. Both run pdt-host and pdtd
from a plain install folder holding `pdtd`, `pdt-host` and `web/` (pdt-host `linux.rs`). There is no `.deb`, `.rpm`,
Flatpak or Snap, and the tray has no "Start at login" on Linux.

- **Install, headless:** unpack the tarball where it can stay, then register it:
  `mkdir -p ~/.local/opt && tar -xzf pdt-linux-x86_64.tar.gz -C ~/.local/opt && ~/.local/opt/pdt/pdt-host install`.
  The folder's parent must be writable by the user (an update replaces the folder); `/opt` owned by root works, but
  then updates are refused (`installer_for`) and pdtd shows them as off.
- **Install, desktop:** run the AppImage. The stable app, when no unit exists yet, runs its own `pdt-host install
  --appimage <file>` (desktop daemon.rs): pdtd, pdt-host and the web UI are copied out of the mounted AppImage to
  `<data dir>/app` (with `appimage.path` naming the AppImage), and that folder is registered. A mounted AppImage goes
  away when its process exits, so nothing runs from it; the tray then starts pdt-host from `<data dir>/app` too.
- **Service:** `~/.config/systemd/user/pdt-host.service` (`$XDG_CONFIG_HOME`), `ExecStart=<folder>/pdt-host run
  --install-dir <folder>`, `Restart=always`, `KillMode=process` (stopping or restarting pdt-host leaves pdtd and its
  agents running; the next pdt-host adopts pdtd, as on macOS), `Environment=PDT_HOST_SERVICE=systemd`,
  `WantedBy=default.target`. `install` runs `systemctl --user daemon-reload`, then `enable --now` (just `enable` when a
  pdt-host already runs; `--replace` stops it first). A pdt-host that finds another one holding the data folder exits
  with status 3, which the unit does not restart (`RestartPreventExitStatus`). Its output goes to the journal
  (`journalctl --user -u pdt-host`); the logs are in the data folder as elsewhere.
- **Linger:** a user's systemd starts at their first login and stops at their last logout. To run PDT from boot on a
  headless machine: `loginctl enable-linger "$USER"` (once; `install` prints it). Check with `loginctl show-user
  "$USER" -p Linger`.
- **Data:** `$XDG_DATA_HOME/pdt`, i.e. `~/.local/share/pdt` (dev: `pdt-dev`).
- **Secrets:** the Secret Service (gnome-keyring, KWallet) when pdtd can reach one over D-Bus the first time the data
  folder starts; otherwise `<data dir>/secrets.json`, readable only by the user (0600), and Settings → Secrets says
  "Secrets: file in the data folder". The choice sticks to the data folder: it never switches between restarts
  ([secrets.md](secrets.md)). A headless machine without a desktop session normally has no Secret Service.
- **Update:** pdtd picks the asset for its CPU, the AppImage when its folder holds `appimage.path`, else the tarball,
  and hands it to pdt-host (`LinuxInstaller`): the tarball is extracted (or the AppImage extracted with
  `--appimage-extract`, which needs no FUSE) into `.pdt-update-staging` beside the install folder, the folder is
  moved to `<folder>.previous` and the new one moved into place; for the AppImage the AppImage file is swapped the same
  way (`<file>.previous`). Health check, watch and rollback are the same as elsewhere; a release that changes pdt-host
  makes it exit, and systemd starts the new one. A running tray keeps its old version until it is started again.
- **Uninstall:** `<folder>/pdt-host uninstall` (stops pdt-host and pdtd, `systemctl --user disable --now`, removes the
  unit), then delete the install folder (for the AppImage, `<data dir>/app` and the AppImage). The data folder is left.
