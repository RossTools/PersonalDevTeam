# Troubleshooting and FAQ

What to check when something goes wrong, and answers to common questions.

## Where the logs are

The logs are in the `logs` folder of PDT's data folder:

- macOS: `~/Library/Application Support/pdt/logs`
- Windows: `%LOCALAPPDATA%\pdt\logs`
- Linux: `~/.local/share/pdt/logs`

`pdtd.<date>.log` is PDT's own log, one a day; `pdt-host.log` covers starting, stopping and updates;
`pdtd-launch.log` holds what PDT printed before its log opened, which is where a failed start shows. See
[Your data](../your-machine/your-data.md#logs).

To tell us about a problem, use **Report a bug…** (in the command palette, or **Settings › This host › Bug
reports**). It can attach the end of the log, with tokens and your machine's names taken out.

## PDT doesn't open

1. Open `http://127.0.0.1:7420` (or your own port, if you changed it).
2. If nothing answers, use **Restart daemon** in the tray menu, or choose **Stop PDT and quit** in the tray and open the app again.
3. Still nothing: read the end of `pdtd-launch.log` and `pdt-host.log`. The usual reasons are that the port is taken
   (below), or another copy of PDT already uses the data folder (**is in use by another pdtd**).

On Linux, `systemctl --user status pdt-host` says whether the service runs, and `journalctl --user -u pdt-host` shows
its output.

## "Can't reach the host"

A red strip above the page says **Can't reach the host** when your browser is offline or PDT stops answering. What is
on screen may be out of date. PDT keeps trying and the strip goes away once it answers; if it stays, see
[PDT doesn't open](#pdt-doesnt-open).

When the Inbox can't load its items (with or without the strip), it says **Can't load the Inbox** rather than that
nothing waits on you.

## The port is in use, or the field is locked

**The log says `port 7420 is already in use`.** Another program has PDT's port. Find it (`lsof -i :7420` on macOS
and Linux, `netstat -ano | findstr 7420` on Windows) and stop it, or give PDT another port by writing it into
`network.json` in PDT's data folder:

```json
{ "port": 7430 }
```

Then **Restart daemon** in the tray, and open `http://127.0.0.1:7430`. The tray finds the new port by itself.

**The Port field in Settings › This host › Network is greyed out, with an env badge.** PDT was started with the
`PDT_PORT` environment variable, which wins over the setting. Change or remove it where you set it (your shell
profile, a service file, a launch script) and start PDT again. `PDT_ALLOWED_HOSTS` locks **Allowed hosts** the same
way.

**You changed the port and nothing happened.** It changes when PDT restarts: click **Restart now** in the banner on
the **Network** screen.

## An agent isn't found

**Settings › This host › Checks**, or **Agents on this machine** in **Get started**, says **not found** or **Missing**:

1. Check the agent runs in a terminal: `claude --version`, `codex --version` or `devin --version`.
2. If it does, PDT may not see your terminal's `PATH` (it starts at login, before your shell profile runs). Find the
   agent (`which claude` on macOS and Linux, `where claude` on Windows) and enter that path in **Settings › This
   host › Agents › Programs**.
3. Click **Run checks again**.

A program path set by an environment variable (`PDT_CLAUDE_PROGRAM` and so on) shows an **env** badge and wins over
the setting.

If the agent shows **Not logged in**, or its sessions fail at once with a login error, log in once in a terminal (run
`claude` and type `/login`, `codex login` or `devin auth login`): PDT uses the agent's own login. If you use an API
key instead, set **Settings › This host › Agents › Credentials** to **Pass through**. **Start** on a ticket warns
before it starts an agent that isn't logged in.

Don't use an agent? Turn it off with its switch under **Programs**, and PDT stops counting it as missing.

## A merge says git doesn't know who you are

git needs a name and email to commit a merge. Set them once in a terminal, then merge again:

```sh
git config --global user.name "Your Name"
git config --global user.email "you@example.com"
```

Nothing was merged and the checkout is as it was. **git identity** in **Settings › This host › Checks** names the projects
without a name and email; in those, git may make up an address from your user and computer names.

## An agent is waiting for approval

An agent asks before running a command or editing a file when its permission mode says so. Its tab shows it's
waiting, and the Inbox lists it under **Needs you**:

- **Approvals**: **Allow**, **Deny**, or **Always allow in this session**, in the Inbox or the conversation.
- **Sessions waiting**: the agent asks in its own terminal. **Open session** and answer there.

If agents ask too often, choose a more permissive mode: for one session in its **Start** dialog or conversation, for
tickets and new sessions in **Settings › This host › Agents** (**Ticket permissions**, **Session permissions**), and
for workflow steps per persona or project. See
[Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#models-and-permission-modes).

## GitHub no longer accepts PDT's login

You see **Reconnect GitHub** in the Inbox, or **GitHub on github.com no longer accepts this login** in **Settings ›
Plugins › GitHub**, and pull requests stop updating. The login was revoked (on GitHub, or by an organization's
policy) or the token expired.

1. Open **Settings › Plugins › GitHub** (**Enter** on the Inbox item takes you there).
2. Click **Reconnect** and enter the code on GitHub, or paste a new token and click **Save login token**.

PDT checks pull requests and CI again at once. If it keeps happening, a pasted token may have a short expiry: make
one that lasts longer. See [GitHub and other forges](../your-machine/github.md).

**An organization's private repositories answer as not found** while others work: the organization hasn't approved
PDT. See [Private repositories of an organization](../your-machine/github.md#private-repositories-of-an-organization).

**Errors say GitHub's rate limit is used up.** PDT waits until the time GitHub gives, then catches up by itself.

## An update rolled back

A red banner says **Update to v… failed (…) and was rolled back**. You're on the version you had, with your data as
it was. The banner gives the reason; `pdt-host.log` has the details.

- PDT won't install that version on its own again, but tries the next one. To retry it, click **Check for updates**
  in **Settings › This host › Updates**, then **Update** in the banner.
- **Windows: `pdtd.exe` is in use.** Something still had PDT's program open (a second copy, or a virus scanner).
  Restart the computer and try again.
- **macOS: the app isn't in Applications.** PDT can't update an app run from a disk image or your Downloads folder.
  Move it to **Applications** and open it from there.
- If every update fails the same way, use **Report a bug…** with the diagnostics included.

## Updates say they can't check

The **Version** row in **Settings › This host › Updates** says what the last check found. When it isn't the version
you're on or a new one:

- **GitHub's limit on checks is used up for now; PDT checks again once it resets (after …)**: nothing is wrong, and
  PDT checks again by itself at that time. To check more often, save a GitHub token under **Advanced › GitHub token**.
- **The saved GitHub token was rejected and ignored: replace or remove it**: the check went on without the token.
  Click **Replace the token** and paste a new one, or remove it under **Danger zone**.
- **could not reach GitHub: …** (after **Check for updates**: **Couldn't check: could not reach GitHub: …**): check
  your network or proxy. PDT tries again at the next check.
- **Couldn't check: …** with another reason (GitHub answered with an error, no release was found): it says what
  GitHub answered. Check **Release repository** if you set one; otherwise try again later.
- **Updates are off: …**, with no **Check for updates** button: this copy of PDT can't update itself, and the rest
  of the line says why. A development copy never updates; the installed app can't update from a disk image or your
  Downloads folder (move it to **Applications** and open it from there).

## macOS keeps asking about the keychain

After an update macOS may ask whether Personal Dev Team can use its keychain items (the GitHub login, the update
token, your secrets). Choose **Always Allow**. Until you answer, PDT can't read them, and says so in Settings.

## Remote access doesn't work

See [Remote access and your phone](../your-machine/remote-access.md#when-the-status-isnt-active).

## FAQ

### Does PDT need an AI account?

PDT brings no AI of its own. It runs the agents you install, signed in as you, so they cost what they cost outside
PDT: your subscription or API usage. Several agents at once use your plan's limits faster; **Agents running at once**
in **Settings › This host › Agents** caps it.

### Is my code sent anywhere?

Not by PDT. Your tickets, conversations and settings stay on your machine; there is no PDT server. The agents send
what they read to their own providers, as they do in a terminal. PDT calls GitHub for its own updates and, when
you've connected it, for your projects' pull requests, CI and issues. Bug reports go out only when you send one, and
show what they include first.

### Does PDT change my agents' settings?

No. What PDT gives an agent (its instructions, its tools, "Always allow" for a command) lasts for that session only;
your agents' own settings and config files are never edited.

### Do agents stop when I close the browser?

No. Agents, runs and workflows carry on, and the Inbox collects what needs you. They pause only when PDT itself stops
(your computer sleeps or restarts, or PDT updates) and resume when it's back.

### Can anyone else on my tailnet get in?

No. PDT lets in only the Tailscale user set as its **Owner**, and the same goes for the tailnet addresses of the runs
it starts. Other users, shared and tagged devices, and Tailscale Funnel are turned away. See [Remote access and your phone](../your-machine/remote-access.md).

### Can I run PDT on a server and use it from my laptop?

Yes. Install it on a Linux server ([Install by hand](../getting-started/install.md#install-by-hand)), turn on
lingering so it runs while you're logged out, and reach it over Tailscale.

### Can I go back to an older version?

No: PDT never installs an older version than the one you run, because an older version may not read the newer one's
data. A failed update rolls back for you. See [Release channels](../your-machine/updates.md#release-channels).
