# Personal Dev Team — Remote Host (Phase 2) Design

Status: approved in brainstorming on 2026-10-15. This amends the main spec,
`2026-10-14-personal-dev-team-design.md`: it becomes the new Phase 2, and the main spec's §3.1, §5.5, §9, §11
and §13 now point here.

## 1. Purpose

The user's main PDT host becomes a Mac mini they usually aren't sitting at. They use it from a Windows desktop, a
laptop and a phone. Once this phase lands, PDT is developed inside PDT on the Mac mini, and the user tests each
branch remotely from whichever device they're on.

PDT runs as several **independent hosts**:

| Host | Role | Network | Agent accounts |
|---|---|---|---|
| Mac mini | Main host; PDT develops itself here; mostly web apps | Tailscale | Personal subscriptions |
| Windows desktop | Second host for work that belongs on Windows | Tailscale | Personal subscriptions |
| Work laptop | Self-contained install for the user's corporate job | No Tailscale (not allowed) | Corporate accounts |

Hosts never talk to each other or share data. Switching hosts means opening a different URL. Nothing in this phase
is specific to PDT developing itself: self-hosting is one use of the generic features below.

### Success criteria

- From any device on the tailnet, the user can open the Mac mini's PDT over HTTPS, start agent sessions, run any
  branch of a project and open that run's web page.
- A merge to main reaches the Mac mini's stable instance without anyone at the Mac. It installs when no agent is
  busy, or when the user clicks Update from any device. Sessions resume afterwards, and a bad release rolls back by
  itself.
- The work laptop runs the same build with corporate agent credentials, no Tailscale and no outbound traffic from PDT
  except an optional update check.

## 2. Remote access and authentication

- `pdtd` keeps listening only on `127.0.0.1`. **Remote access** is a per-host setting, off by default.
- When it's on:
  - **Serving:** `pdtd` runs `tailscale serve --bg --https=443 http://127.0.0.1:<port>`, so the host appears at
    `https://<machine>.<tailnet>.ts.net/`.
  - **Allowlist:** it reads its own DNS name from `tailscale status --json` (`Self.DNSName`) and adds it to the
    Host/Origin allowlist.
  - **Removal:** it removes its mappings when the setting is turned off.
  - **Status:** Settings shows on or off, the URL and errors (for example "Tailscale is not running", or
    "tailscale CLI not found").
- **Who is allowed:** `tailscale serve` adds the caller's identity headers (`Tailscale-User-Login`,
  `Tailscale-User-Name`) when it proxies a request.
  - A request **without** these headers is local. It came from the host itself, because `pdtd` only listens on
    loopback, and it is trusted as before.
  - A request **with** them is allowed only if `Tailscale-User-Login` equals the host's **owner** setting.
    Everything else gets 403 `{code: "forbidden_identity"}`.
  - On first setup, PDT offers to set the owner to the identity of the device being used.
- This replaces the main spec's device tokens. Tokens may return later for remote access without Tailscale.
- **Accepted:** any local process on a host can use the API without authenticating, as before. Agents and runs on
  that host are local.

## 3. Host supervisor and updates

### 3.1 `pdt-host`

- **What it is:** a small new program, and the thing that actually runs on each host. Its name lives in `brand.rs`
  like the others.
- **What it does:**
  - starts `pdtd`, and restarts it with backoff if it exits unexpectedly;
  - performs updates and rollbacks (§3.4);
  - stays deliberately small, so it rarely changes itself.
- **How it starts:**
  - **macOS:** a per-user launchd agent (`~/Library/LaunchAgents`, `RunAtLoad` and `KeepAlive` on). It must be
    per-user: Claude Code and Codex keep their logins in the user's account (macOS Keychain), so PDT runs in the
    user's login session. The Mac mini logs in automatically after a reboot; hermes-agent already relies on this.
    The agent runs `pdt-host` from the app bundle, never a copy: a login item whose program is rewritten after every
    update is what malware hiding in a persistence item looks like, and endpoint security (SentinelOne) flags it
    (PDT-40).
  - **Windows:** a per-user logon task.
  - **Registration:** installers register these.
- **The tray app** becomes an optional view of the host: Open (default browser), status, Update, Quit. Its code
  for starting the daemon moves into `pdt-host`. Hosts don't need a tray. The tray's `gh`-based Update, shipped in
  PR #2, is removed.
- **Login check:** at startup PDT warns if `claude` or `codex` aren't logged in for the user it runs as.

### 3.2 Update source: a GitHub token

- **The one way to update:** a read-only, fine-grained GitHub token, limited to the release repo
  (`RELEASE_REPO` in `brand.rs`).
  - **Storage:** the user pastes it once per host in Settings. It is stored in the OS keychain or credential
    manager, never in a file or the database.
  - **Access:** PDT calls the GitHub REST API directly to read the latest release and download its assets.
  - **Org permission:** the org must allow fine-grained tokens; the owner approves the token.
- **No token** means the host only updates manually, from a downloaded installer.
- **Expiry:** PDT reads the token's expiry from GitHub's API responses
  (`github-authentication-token-expiration`). It warns 14 days ahead, and shows an error once the token has
  expired or been revoked.

### 3.3 Checking and policy

- **Checks:** `pdtd` checks hourly and when asked. Every device's web UI shows "Update to v0.1.N available", with
  the release notes and an **Update** button.
- **Checksums:** CI also publishes `SHA256SUMS` in each release. A download whose checksum doesn't match is
  rejected.
- **Policy** (per host):
  - **auto when idle:** installs once no agent session is `working` or `unknown` (§5) and no run is setting up.
  - **notify:** banner only.
  - **manual:** no checks.
- **Busy hosts:** clicking Update while something is busy lists what's busy and asks to confirm.

### 3.4 Installing and rollback

1. `pdtd` downloads and verifies the release into its data folder.
2. `pdtd` hands the release to `pdt-host`, for example through a request file in the data folder, and shuts down
   gracefully. Running sessions are marked `interrupted`.
3. `pdt-host` installs the release: the silent NSIS installer on Windows, a bundle swap on macOS. It keeps the
   previous version's installer or bundle.
4. `pdt-host` starts the new `pdtd` and waits up to 60 s for `/api/health`. If that check fails, it reinstalls the
   previous version and starts it. The UI then shows "Update to v0.1.N failed and was rolled back", with the log.
5. Sessions auto-resume (Phase 1 behaviour).
6. If the release changes `pdt-host` itself, `pdt-host` restarts last: on Windows it first replaces its copy in the
   data folder; on macOS the new bundle already holds the new `pdt-host`.

**Versions:** release builds report `0.1.<run>`. Local builds report `0.1.0-local` and never count as newer than a
release.

## 4. Run configurations

### 4.1 Configuration

- **Where it lives:** an optional `.pdt/project.toml` in the repo, plus **host overrides** stored in PDT's
  settings for the project.
  - Overrides are merged field by field, and can supply the whole configuration. The user's work repos never
    contain the file.
  - Runs and tests are matched by `name` when merging.

```toml
[[run]]
name    = "web"
setup   = "pnpm install"                  # runs before each start; optional
command = "pnpm dev --port $PORT"
web     = true                            # PDT assigns $PORT and serves it over HTTPS
data    = true                            # PDT provides $PDT_RUN_DATA, kept per branch
env     = { API_URL = "http://127.0.0.1:$PORT/api" }

[[test]]
name    = "unit"
command = "pnpm test"
```

### 4.2 Run instances

- **Starting one:** "Run" on a branch, worktree or PR starts a run instance.
  - **Checkout:** a branch without a worktree gets one in `.worktrees/`.
  - **Terminals:** setup and command run in PDT terminals (Phase 1 PTYs), so their logs survive reloads.
- **States:** `setting_up`, `starting`, `ready`, `failed`, `stopped`. A `web` run is `ready` once its port
  answers.
- **Variables PDT sets:** `PORT`, `PDT_RUN_DATA`, `PDT_RUN_URL`, `PDT_RUN_HOST` (the tailnet DNS name),
  `PDT_RUN_BRANCH`, and `PDT_OWNER_LOGIN` (the host's owner).
  - Inherited `PDT_*` variables and the leaked variables from Phase 1 are still removed.
  - Variables set explicitly in `env` are passed through.
- **Web runs:**
  - **Port:** PDT picks a free port from the host's range (default 8400–8499).
  - **Tailscale:** if remote access is on, it maps the port with `tailscale serve --bg --https=<port>`, so the
    link is `https://<machine>.<tailnet>.ts.net:<port>/`. Otherwise the link is `http://127.0.0.1:<port>/`.
  - **Cleanup:** mappings are removed when the run stops.
- **Data:** the folder is kept per project, branch and run name. A "Reset data" action clears it.
- **Lifecycle:**
  - **Stopping:** runs stop on request, when their branch is merged or deleted, or when the host's maximum number of
    concurrent runs (default 4) would be exceeded. The user confirms first in that last case.
  - **Restarts:** a run interrupted by a `pdtd` restart or update restarts by itself.
- **Not a web app:** a run with `web = false` is a process with logs, such as a CLI. On a machine the user is
  sitting at, it can also launch a desktop app.
- **Tests:** they run the same way, in a terminal on the worktree, with pass or fail shown. Agents can run them as
  plain commands.
- **Accepted:** runs use the host's real toolchains, with the same trust as agent terminals.

### 4.3 PDT's own configuration

PDT's `.pdt/project.toml` defines a run:
- **Setup:** builds the web UI and `pdtd`.
- **Command:** starts `target/debug/pdtd`.
- **Environment:**
  - `PDT_PORT=$PORT` and `PDT_DATA_DIR=$PDT_RUN_DATA`;
  - `PDT_ALLOWED_HOSTS` set to the run's host and port;
  - the owner login passed through (a new `pdtd` setting, `PDT_OWNER_LOGIN`), so a dev instance accepts the user.

## 5. Agent activity state

### 5.1 Activity

Each `claude` and `codex` terminal session has an **activity**:

| Activity | Meaning |
|---|---|
| `working` | The agent is busy |
| `needs_you` | A permission prompt or a question is waiting for the user |
| `idle` | The agent finished its turn |
| `unknown` | PDT can't tell |

Shells have no activity.

- **Claude:**
  - **How:** PDT launches each session with `--settings <per-session file>` that adds hooks for that session only.
    The user's own settings files are never touched.
  - **Reporting:** the hooks run `pdtd hook <event>`, which reads `PDT_SESSION_ID` and `PDT_HOOK_PORT` and reports
    over loopback. `pdtd` sets these two after it removes inherited `PDT_*` variables, so a session always reports to
    the `pdtd` that owns it.
  - **Mapping:** `UserPromptSubmit` and `PreToolUse` mean `working`; `Notification` means `needs_you`; `Stop` means
    `idle`.
- **Codex:**
  - `-c notify=[…]` for the session reports turn completion, which means `idle`.
  - `working` is inferred from input and recent output.
  - Approval prompts are recognised from the screen text, which means `needs_you`.
  - Phase 3's structured adapter replaces the screen reading later.
- **Fallback:** if detection breaks after a CLI update, activity falls back to `unknown`. Auto-update treats
  `unknown` as busy.
- **Verification:** the plan's first task checks these behaviours against the installed versions (Claude Code on the
  dev machine, Codex 0.162.1).

### 5.2 Unseen

- **The flag:** each session has an **unseen** flag. It is set when the session becomes `idle` or `needs_you` after
  the user last viewed it.
- **Viewing:** it's cleared when a client shows that session's terminal while the browser tab is visible.
- **Shared:** the server records "last viewed", so viewing on one device marks the session seen on all of them.
- **Display:** the session list distinguishes idle and unseen from idle and seen, and the same for `needs_you`.
  Later notifications fire only for unseen sessions.

## 6. Per-host settings

They're stored in each host's database, edited in **Settings → This host**, and never synced.

| Setting | Values (default) |
|---|---|
| Remote access | on / off (off) |
| Owner Tailscale login | a login (unset) |
| Update policy | auto when idle / notify / manual (notify) |
| Update token | in the keychain (none) |
| Run port range | 8400–8499 |
| Max concurrent runs | 4 |
| Agent credentials | subscription / pass through (subscription) |

- **Agent credentials:**
  - **subscription** keeps today's behaviour: `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are removed from agent
    sessions.
  - **pass through** gives agents the host's environment unchanged: API keys, Bedrock and Vertex variables,
    corporate proxies.
  - A project can override the host setting either way.
- **Outbound traffic:** PDT's only outbound traffic is the update check, and only with a token. There is no
  telemetry and no host-to-host traffic.
- **First run:** a first-run checklist lists what is missing. Examples: remote access off, no update token, owner
  unset, Claude or Codex not logged in.

## 7. Development lifecycle (PDT on the Mac mini)

1. The user starts an agent session on a branch from any device. It works in `.worktrees/<branch>` on the Mac mini.
2. The agent runs the tests. CI runs them again on its PR.
3. The user clicks **Run** on the branch, gets a dev instance of PDT on its own port and data, and opens the link.
   Several branches can run side by side.
4. The user tests and gives feedback in the session. The agent fixes the branch, and the run restarts on the new
   code.
5. The user says "merge". The PR is squash-merged once its CI is green, the run stops, and the worktree is removed.
6. CI publishes `v0.1.N`. The stable instance installs it when idle, or on Update, and its sessions resume.

**Self-hosting safety rules:**
- Agents never stop, restart or update the stable instance, and never call its `/api/shutdown`. Only `pdt-host` and
  Update do.
- A dev instance always gets an explicit `PDT_PORT` and `PDT_DATA_DIR`. `pdtd` refuses a data folder that another
  `pdtd` holds.
- Rollback (§3.4) limits a bad merge to one failed update.

These replace the CLAUDE.md rules about port 7420 once this phase ships. Until then, the existing rules apply.

## 8. Testing

- **Unit tests:**
  - merging the config file with host overrides;
  - port allocation;
  - parsing releases and versions, checksums, and token expiry;
  - the activity and unseen state machine;
  - the identity check;
  - the credential-passing rules.
- **Integration tests:**
  - a fake `tailscale` CLI, recording the `serve` calls and returning status JSON;
  - a fake GitHub API: a release, an expired token, a bad checksum;
  - start and stop of a run with a fake web server;
  - `pdt-host` restarting a crashed `pdtd`, and rolling back to the previous version when a fake new `pdtd` fails its
    health check;
  - `pdtd hook` events changing a session's activity.
- **By hand on the Mac mini:**
  - `tailscale serve` for the host and a run;
  - the identity check, from another device and from a tailnet login that isn't the owner;
  - the launchd agent surviving a logout or login and a reboot;
  - a real update from v0.1.N to N+1, and a forced rollback.

  The plan writes this checklist as steps an agent on the Mac mini (hermes-agent) can follow.
- **CI:** Windows and macOS, as today.

## 9. Deferred

- **Phone UI:** the single-column layout and touch terminals. This is a parallel workstream, independent of this
  phase.
- **Notifications** for `needs_you` and unseen sessions.
- **Automatic per-PR runs:** a per-project option to start a run automatically for each PR.
- **A multi-host UI:** one interface for several hosts.
- **Device tokens:** for remote access without Tailscale.
- **Replacing hermes-agent:** its own brainstorm, after the agentic phases.
- **Devin** as an agent on the work laptop: a terminal session type first, an adapter later.
- **Workspaces:** herdr-style workspaces, tabs and split panes, which is the next phase and gets its own brainstorm.
