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
  written before logging starts and panics. pdt-host moves `pdt-host.log` and `pdtd-launch.log` to `<name>.1`
  (replacing the one before) once they pass 5 MB.
- Database (`<data dir>/pdt.db`, SQLite in WAL mode, `synchronous=NORMAL`): before applying a pending migration pdtd
  copies it to `<data dir>/backups/pdt-<ms>.db` (`VACUUM INTO`) and keeps the newest 3 copies. Every 10 minutes
  `maintain_workflows` empties the WAL (`wal_checkpoint(TRUNCATE)`), so it cannot grow between restarts while
  readers keep SQLite's own checkpoints from resetting it. Housekeeping prunes events, finished workflow runs,
  resolved outbox messages, ended Inbox items and notifications after 30 days, and conversation frames after 7
  ([workflows.md](workflows.md), [conversations.md](conversations.md)).
- `/api/health` reports `status`, `name`, `version`, `profile` (`dev`/`stable`), `port`, `dev` (dev profile or a
  non-stable port) and `webBuild` (the first 12 hex digits of the served index.html's SHA-256, read per request; null
  without a web dir). The UI shows a `DEV · <port>` badge in the rail and a `[dev]` title prefix when `dev` is true.
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
- pdt-host writes its program path to `<data dir>/pdt-host.program`; a tray whose bundle differs from the running
  pdt-host's offers "switch to this one" (asking at startup when the other copy is misplaced), which runs
  `pdt-host install --replace` (boots out and stops the other pdt-host, bootstraps this bundle's) and restarts pdtd.
