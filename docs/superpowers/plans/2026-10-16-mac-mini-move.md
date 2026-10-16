# Move to the Mac mini — runbook

**Goal:** the Mac mini runs the stable PDT that develops PDT. The user works in it from the Windows desktop, the
laptop and the phone. The Windows desktop stays a second host for Windows-only work and Windows testing.

**Who does what:**
- **hermes** (hermes-agent on the Mac mini) runs the Mac mini steps that aren't marked otherwise.
- **USER** steps install, stop, update or roll back a stable app, or create a credential. Only the user does those.
  Agents never touch a stable instance (CLAUDE.md).
- Record PASS/FAIL with what you saw next to each step, in a copy of this file on the branch `mac-mini-move-results`.
  Open a PR from it when done.

**Starting state:**
- Phase 2 is merged (#3). CI publishes `v0.1.N` on every push to main; N is 33 or later.
- The Windows stable install predates pdt-host, so it is updated by hand once.
- The Mac mini already has repo access and some of the toolchain. Tailscale HTTPS certificates are enabled.

## 1. The update token (USER)
1. Create a fine-grained GitHub token with resource owner `ross-game-tools`, repository `PersonalDevTeam` only, and
   permissions **Contents: read** (Metadata: read is added automatically).
   - Choose an expiry; PDT warns 14 days before it expires.
   - If the org requires approval for fine-grained tokens, approve it.
2. Keep it to paste into each host's Settings. One token can serve both hosts. Never commit it or put it in a file.

## 2. Windows desktop: the one manual update (USER)
The installed app predates pdt-host, so it can't update itself yet.
1. Wait for the release: `gh release list --repo ross-game-tools/PersonalDevTeam --limit 1` shows `v0.1.N` (N ≥ 33).
2. Download the installer:
   `gh release download v0.1.N --repo ross-game-tools/PersonalDevTeam -p "*-setup.exe" -D $env:TEMP\pdt-release`
3. Quit the tray (menu → Quit), then stop the old daemon: `Stop-Process -Name pdtd`. Sessions come back as
   interrupted and auto-resume after the update.
4. Run the installer. It registers pdt-host at login (HKCU Run) and starts it.
5. Check:
   - `curl.exe -s http://127.0.0.1:7420/api/health` reports `0.1.N`.
   - `Get-Process pdt-host | Select-Object Path` points into `%LOCALAPPDATA%\pdt\host\`.
   - The old sessions resumed.
6. Settings → This host:
   - Click "Use <owner>".
   - Paste the token.
   - Update policy: "notify" for now. This host is used less, so updates are applied by clicking.
   - Remote access: on, if you want to reach this desktop from elsewhere.
7. Then follow `docs/superpowers/notes/phase-2-windows-checklist.md` once the next release exists (step 6 below
   makes one).

## 3. Mac mini: toolchain (hermes)
Inventory first and report to the user; install only user-level tools yourself. Ask the user before installing
anything system-wide (Xcode CLT, Homebrew packages, a Node installer).

| Tool | Check | Need |
|---|---|---|
| Xcode CLT | `xcode-select -p` | a path |
| git | `git --version` | 2.x |
| Rust | `rustup --version`, `cargo --version` | rustup present; `rust-toolchain.toml` pins 1.99.0, and rustup installs it on first `cargo` in the repo |
| Node | `node --version` | v24 (what CI uses) |
| pnpm | `pnpm --version` | 12.11.2 (`packageManager` in `package.json`; `corepack enable` provides it) |
| Tailscale | `tailscale status --json` → `BackendState` Running, `CertDomains` non-empty | — |
| Claude Code | `claude auth status` → loggedIn | user logs in if not |
| Codex CLI | `codex --version` (the npm CLI, not an old app build), `codex login status` | user logs in if not |
| gh (optional) | `gh auth status` | handy for releases |

## 4. Mac mini: install stable PDT (USER, hermes checks)
Follow `docs/superpowers/notes/phase-2-mac-mini-checklist.md` §0 and §1.1–§1.2:
1. Download and unpack `v0.1.N`'s `.app.tar.gz`.
2. Move it to `~/Applications` and open it once.
3. hermes checks:
   - the LaunchAgent is running;
   - http://127.0.0.1:7420/ answers;
   - pdt-host runs from `~/Library/Application Support/pdt/host/`.

## 5. Mac mini: configure (USER in the browser, or hermes)
1. In Settings → This host:
   - Click "Use <owner>".
   - Remote access: on. The status shows `https://<mac-mini>.<tailnet>.ts.net/`.
   - Paste the token.
   - Update policy: **Install automatically when idle**.
2. From the Windows desktop browser, open `https://<mac-mini>.<tailnet>.ts.net/`. PDT loads and admits you.
3. Clone the repo where it will live, e.g. `git clone https://github.com/ross-game-tools/PersonalDevTeam ~/work/pdt`.
   Run `cargo --version` in it once so rustup fetches 1.99.0. Add it as a project in PDT.
4. Run → pdt on `main`. The run reaches ready, and its `https://…:84xx/` link opens a dev PDT from the desktop.
   This first run is a slow full build.

## 6. Prove the loop (USER + hermes)
1. Run the rest of the Mac mini checklist: §1.3–§1.6 (survives quit, kill, logout, reboot), §2 (remote access,
   identity), §3 (runs), §4 (activity). Record results.
2. **First self-hosted change:** from the desktop browser, start a Claude session in the Mac mini's PDT on a new
   branch and do a small roadmap follow-up there, end to end: change, tests, push, PR, CI, merge. The CI speed-up
   (docs/ROADMAP.md) is a good candidate. Test its dev instance through Run → pdt from the desktop.
3. That merge publishes `v0.1.N+1`. Expected:
   - The Mac mini (auto when idle) updates itself once no session is busy.
   - The Windows desktop (notify) shows "Update to v0.1.N+1 available".
   - Then run checklist §5 (Mac) and the Windows checklist (USER clicks Update on Windows).
4. Optional: try the phone (monitor and nudge). Phone layout work is a roadmap follow-up; note what hurts.

## 7. Done when
- The Mac mini is reachable from the desktop, laptop and phone. It survives a reboot unattended, and updates itself
  after a merge.
- A change to PDT was made, tested on a dev instance and merged entirely through the Mac mini's PDT.
- The Windows desktop updates by a click.
- Checklist results are committed on `mac-mini-move-results` with a PR. Failures become roadmap items or fixes.
- docs/ROADMAP.md moves "Move PDT to the Mac mini" to Done, and the next item (Phase 3 brainstorm) becomes Now.

## If something fails
- **pdt-host or pdtd won't start:** read `~/Library/Application Support/pdt/logs/` (pdt-host.log, pdtd-launch.log,
  pdtd.<date>.log).
- **Remote URL 403:** the owner login is wrong (Settings → This host), or the request came from a tagged device.
- **Run stuck in setting_up:** the setup log in the run view; usually a missing tool from §3.
- **Update failed and rolled back:** the banner names the log. A failed version isn't retried automatically; click
  Update to retry after a fix.
- Report anything unexpected to the user rather than working around it on the stable instance.
