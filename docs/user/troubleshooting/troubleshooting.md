# Troubleshooting

What to check when PDT won't start, an agent isn't found, GitHub isn't connected or an update fails.

## Where the logs are

Most answers start in the logs. They're in the `logs` folder of PDT's data folder:

- macOS: `~/Library/Application Support/pdt/logs`
- Windows: `%LOCALAPPDATA%\pdt\logs`
- Linux: `~/.local/share/pdt/logs`

`pdtd.<date>.log` is PDT's own log, one a day; `pdt-host.log` covers starting, stopping and updates;
`pdtd-launch.log` holds what PDT printed before its log opened, which is where a failed start shows. See
[Your data](../your-machine/your-data.md#logs).

To tell us about a problem, use **Report a bug…** (in the command palette, or **Settings › This host › Bug
reports**). It can attach the end of the log for you, with tokens and your machine's names taken out.

## PDT doesn't open

1. Open `http://127.0.0.1:7420` in your browser (or your own port, if you changed it).
2. If nothing answers, use **Restart daemon** in the tray menu, or quit the tray and open the app again.
3. Still nothing: read the end of `pdtd-launch.log` and `pdt-host.log`. The two usual reasons are below: the port is
   taken, or another copy of PDT already uses the data folder (**is in use by another pdtd**).

On Linux, `systemctl --user status pdt-host` says whether the service runs, and `journalctl --user -u pdt-host` shows
its output.

## The port is in use, or the field is locked

**PDT won't start, and the log says `port 7420 is already in use`.** Another program has PDT's port. Find it
(`lsof -i :7420` on macOS and Linux, `netstat -ano | findstr 7420` on Windows) and stop it, or give PDT another port:
since PDT can't start to show you Settings, write the port into `network.json` in its data folder:

```json
{ "port": 7430 }
```

then restart PDT (**Restart daemon** in the tray). The tray finds the new port by itself; open
`http://127.0.0.1:7430`.

**The Port field in Settings › This host › Network is greyed out, with an env badge.** PDT was started with the
`PDT_PORT` environment variable, which wins over the setting. Change or remove the variable where you set it (your
shell profile, a service file, a launch script) and start PDT again. `PDT_ALLOWED_HOSTS` locks **Allowed hosts** the
same way.

**You changed the port and nothing happened.** The port changes when PDT restarts: click **Restart now** in the
banner on the **Network** screen.

## An agent isn't found

**Settings › This host › Checks**, or **Agents on this machine** in **Get started**, says **not found** or **Missing**:

1. Check the agent runs in a terminal: `claude --version`, `codex --version` or `devin --version`.
2. If it does, PDT may not see the same `PATH` as your terminal (PDT starts at login, before your shell profile
   runs). Find where the agent is (`which claude` on macOS and Linux, `where claude` on Windows) and enter that path in
   **Settings › This host › Agents › Programs**.
3. **Run checks again**.

A program path set by an environment variable (`PDT_CLAUDE_PROGRAM` and so on) shows with an **env** badge and wins
over the setting.

If the agent is found but its sessions fail at once, run it in a terminal and log in: PDT uses the agent's own login.

Don't use an agent at all? Turn it off with its switch under **Programs**, and PDT stops counting it as missing.

## An agent is waiting for approval

An agent asks before running a command or editing a file when its permission mode says so. Its tab shows that it's
waiting, and the Inbox lists it under **Needs you**:

- **Approvals**: **Allow**, **Deny**, or **Always allow in this session**, right in the Inbox or in the conversation.
- **Sessions waiting**: the agent asks in its own terminal. **Open session** and answer there.

If agents ask too often, choose a more permissive mode: for one session in its **Start** dialog or conversation, for
tickets and new sessions in **Settings › This host › Agents** (**Ticket permissions**, **Session permissions**), and
for workflow steps per persona or per project. See
[Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#models-and-permission-modes).

"Always allow" lasts for that session only: PDT never changes your agents' own settings files.

## GitHub no longer accepts PDT's login

You see **Reconnect GitHub** in the Inbox, or **GitHub on github.com no longer accepts this login** in **Settings ›
Plugins › GitHub**, and pull requests stop updating. GitHub turned PDT's login down: it was revoked (on GitHub, or by
an organization's policy), or the token expired.

1. Open **Settings › Plugins › GitHub** (**Enter** on the Inbox item takes you there).
2. Click **Reconnect** and enter the code on GitHub, or paste a new token and click **Save login token**.

PDT checks pull requests and CI again at once. If it happens again soon, a pasted token may have a short expiry:
make one that lasts longer. See [GitHub and other forges](../your-machine/github.md).

**PDT can't see an organization's private repositories** (they answer as not found) while others work: the
organization hasn't approved PDT. See
[Private repositories of an organization](../your-machine/github.md#private-repositories-of-an-organization).

**Errors say GitHub's rate limit is used up.** PDT waits until the time GitHub gives, then catches up by itself.

## An update rolled back

A red banner says **Update to v… failed (…) and was rolled back**. You're on the version you had, with your data as it
was: nothing is lost. The banner gives the reason; `pdt-host.log` has the details.

- PDT won't install the same version on its own again. When a newer one comes out it tries that. To try the same one
  again, use **Check for updates** in **Settings › This host › Updates**, then **Update** in the banner.
- **Windows: `pdtd.exe` is in use.** Something still had PDT's program open (a second copy, or a virus scanner
  holding it). Restart the computer and try again.
- **macOS: the app isn't in Applications.** PDT can't update an app run from a disk image or your Downloads folder.
  Move it to **Applications** and open it from there.
- If every update fails the same way, use **Report a bug…** with the diagnostics included.

## Updates say they can't check

**Settings › This host › Updates** says **Couldn't check: …**:

- **No token, or the token was rejected**: paste a GitHub token under **GitHub token** and **Save token**.
- **Network**: PDT couldn't reach GitHub. It tries again at the next check.
- **Updates are off**: the app runs from a place it can't update itself in (see above), or this is a development
  copy of PDT, which never updates.

## macOS keeps asking about the keychain

After an update macOS may ask whether Personal Dev Team can use its keychain items (the GitHub login, the update
token, your secrets). Choose **Always Allow**. Until you answer, PDT can't read them, and says so in Settings.

## Remote access doesn't work

See [Remote access and your phone](../your-machine/remote-access.md#when-the-status-isnt-active).
