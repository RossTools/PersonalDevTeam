# Phase 2 hand tests on the Mac mini

For hermes-agent. Run each step in order on the Mac mini, as the user who is logged in automatically. Record PASS or
FAIL (with what you saw) after each step and commit this file on the branch when done. Do not change anything not
named here. "The owner" is the user's Tailscale login (ask the user if unsure). Device B is a second machine on the
tailnet (the Windows desktop or the phone), operated by the user; ask them to do the B steps and report back.

**The stable instance is never touched by an agent.** Any step marked **USER does this** stops, kills, shuts down,
installs or updates the stable pdtd or the stable app: agents never touch the stable instance, so you ask the user to
do it, then you observe and record the result. Everything else you may run yourself. Dev instances (runs) are fine to
start and stop.

## 0. Preconditions
1. `tailscale status --json | /usr/bin/python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["BackendState"], d["Self"]["DNSName"], d["CertDomains"])'`
   Expected: `Running <name>.<tailnet>.ts.net. [...]` with a non-empty CertDomains list (HTTPS certificates enabled).
   If it is empty, ask the user to enable HTTPS certificates in the Tailscale admin console (DNS page) and repeat.
2. Certificate names are public (certificate transparency logs). `tailscale status --json | /usr/bin/python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["Self"]["DNSName"]); [print(p["DNSName"]) for p in d["Peer"].values()]'`
   Show the user the machine names and the tailnet name; **USER confirms** none reveals anything private before
   remote access is turned on (the name of this machine ends up in a public log). Do not continue until they say yes.
3. `claude auth status` → `"loggedIn": true`; `codex login status` → exit 0. If not, ask the user to log in.
4. Note the stable instance's version: `curl -s http://127.0.0.1:7420/api/health`. Reading health is allowed.

## 1. Install and the login item
1. **USER does this** (installs the stable app): install the latest release: download the `.app.tar.gz` from the
   release page, `tar -xzf` it, move `Personal Dev Team.app` to `~/Applications`, open it once.
   Then you check. Expected: `ls ~/Library/LaunchAgents/dev.personaldevteam.host.plist` exists;
   `launchctl print gui/$(id -u)/dev.personaldevteam.host | grep state` shows `running`;
   http://127.0.0.1:7420/ answers.
2. `pgrep -fl pdt-host; pgrep -fl "MacOS/pdtd"` → one of each; pdt-host runs from
   `~/Library/Application Support/pdt/host/pdt-host`.
3. **USER does this** (quits the tray; the stable app): Quit the tray (menu → Quit). Then you check:
   `curl -s http://127.0.0.1:7420/api/health` still answers (pdtd keeps running).
4. **USER does this** (kills the stable pdtd): `kill -9 $(pgrep -f "MacOS/pdtd")`. You time it: within 10 s
   `curl -s http://127.0.0.1:7420/api/health` answers again, and
   `tail -5 ~/Library/Application\ Support/pdt/logs/pdt-host.log` shows "pdtd exited; restarting".
5. **USER does this** (logs out; stops everything): log out and back in. Expected, observed by you afterwards
   (over ssh or the user's report): launchd agent survives logout/login; health answers within a minute without
   opening the app, and `launchctl print gui/$(id -u)/dev.personaldevteam.host | grep state` shows `running`.
6. **USER does this** (reboot): reboot. Expected: after auto-login the launchd agent starts by itself, health
   answers within two minutes, `pgrep -fl pdt-host` shows pdt-host.

## 2. Remote access and identity
1. In Settings → This host (the user or you in the browser): click "Use <owner>", then turn on Remote access.
   Expected: status shows `https://<machine>.<tailnet>.ts.net/`; `tailscale serve status` lists
   `https://<machine>…` → `http://127.0.0.1:7420`. Only the stable instance claims 443.
2. Check whether `tailscale serve --https=443 off` works: turn Remote access off in Settings and run
   `tailscale serve status`. Expected: no 443 mapping. Record the result: it settles whether the off form needs
   `--bg` (verified fact 5 left this open). If it fails, also try `tailscale serve --bg --https=443 off` and record
   which form worked. Turn Remote access back on.
3. The Host header is preserved by serve (the DNS-rebinding guard depends on it): from device B open the URL; PDT must
   load rather than answer 403 for the Host. Also `tail -20 ~/Library/Application\ Support/pdt/logs/pdtd.*.log`
   shows no rejected-host lines for it. Record what you saw.
4. Device B (owner): open the URL. Expected: PDT loads; a Claude session starts and its terminal works.
   `curl -s https://<machine>.<tailnet>.ts.net/api/host/whoami` from device B → `{"caller":"tailnet","login":"<owner>"}`.
5. Device B with a different tailnet login (or a shared-device user who is not the owner): open the URL.
   Expected: 403 `forbidden_identity` naming that login (the refusal for a non-owner).
6. From a tagged device if one exists (e.g. a server with `tag:`): `curl -s https://<machine>.<tailnet>.ts.net/api/health`.
   Expected: 403 `forbidden_identity` ("no Tailscale user identity").
7. On the Mac mini itself: `curl -s http://127.0.0.1:7420/api/host/whoami` → `{"caller":"local",…}`.

## 3. Runs (dev instances)
Port ranges: the stable instance's runs use 8400–8499; the dev profile's runs use 8500–8599. A run started from the
stable instance gets a port in 8400–8499.
1. Add this repository as a project (if not already), open a worktree for a branch, choose Run → pdt.
   Expected: setup log streams; state goes setting_up → starting → ready; the link is
   `https://<machine>.<tailnet>.ts.net:84xx/`; `tailscale serve status` lists that port.
2. The dev instance runs on its own https port, never 443: `tailscale serve status` still shows 443 → 7420 (stable)
   and a separate `:84xx` → `http://127.0.0.1:84xx`.
3. Device B: open the run's link. Expected: a second PDT (the dev instance) loads and admits the owner. A non-owner
   login gets 403 `forbidden_identity` there too.
4. Token per data dir: with the dev instance open in the browser, Settings → This host shows no update token stored,
   even though the stable instance has one (section 5). `security find-generic-password -s dev.personaldevteam` lists
   the stable account `update-token`; a dev instance would use `update-token:<hash>`. The dev instance never sees
   stable's token.
5. Stop the run. Expected: state stopped; the port's mapping is gone from `tailscale serve status`;
   `lsof -iTCP:<port> -sTCP:LISTEN` prints nothing; 443 → 7420 is untouched.
6. Start two runs on two branches; both get different ports and work side by side.
7. Start a run, then **USER does this** (restarts the stable pdtd, which resumes the run): restart the daemon from the
   tray. Expected: the run comes back (on the same port unless taken); no other mapping in `tailscale serve status`
   disappeared.

## 4. Activity
1. Start a Claude session, type a prompt. Expected: "Working" while it runs, then "Idle" with the unseen dot on device B;
   viewing it on the Mac mini clears the dot on device B within a few seconds.
2. Ask Claude to run a shell command that needs approval. Expected: "Needs you" within ~1 s.
3. Same with Codex: Enter → Working; an approval prompt → Needs you; turn end → Idle.

## 5. Update and rollback
Every step that makes the stable instance update, restart or roll back is **USER does this**; you observe.
1. Settings → This host: paste the fine-grained token (the user gives it to you or does this); policy "notify".
   Expected: token "stored in the OS keychain" (Keychain Access shows an item for service `dev.personaldevteam`);
   no expiry warning unless < 14 days.
2. Merge any small PR so CI publishes v0.1.N+1 (ask the user). Within an hour, or after "Check now", every device shows
   "Update to v0.1.N+1 available" with the notes.
3. **USER does this** (updates the stable app): with a Claude session working, click Update. Expected: a confirm
   listing the busy session. Confirm. Expected, observed by you: the page reconnects;
   `curl -s http://127.0.0.1:7420/api/health` reports the new version (v0.1.N → v0.1.N+1); the session
   resumed; the banner says "Updated to v0.1.N+1"; `ls ~/Applications` shows `Personal Dev Team.app.previous`.
4. **USER does this** (forces a rollback of the stable app; agents never shut down the stable instance): prepare a
   release that fails its health check (publish one, or simulate: copy the current `.app.tar.gz` to a temp folder,
   replace `Contents/MacOS/pdtd` inside it with a script `#!/bin/sh\nsleep 600`, re-tar it, and write
   `~/Library/Application Support/pdt/update/request.json` with
   `{"version":"9.9.9","tag":"v9.9.9","asset":"<that tar.gz>","fromVersion":"<current>","requestedAt":0}`, then
   `curl -X POST http://127.0.0.1:7420/api/shutdown`). You observe: within ~70 s the previous version answers
   health again; the banner shows "Update to v9.9.9 failed and was rolled back" with the log path;
   `tail ~/Library/Application\ Support/pdt/logs/pdt-host.log` says why. Also check the same version is not
   offered again automatically.
