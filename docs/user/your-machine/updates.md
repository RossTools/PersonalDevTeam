# Running and updating PDT

How PDT runs on your machine, starts at login, updates itself and how to remove it. To install it, see
[Install PDT](../getting-started/install.md).

## How PDT runs

PDT is a small server on your machine that you use in your browser, at `http://127.0.0.1:7420`, or from your phone
with [remote access](remote-access.md). A supervisor, `pdt-host`, runs it in the background: it starts PDT at login,
restarts it if it stops and installs updates. Agents and runs carry on when you close the browser, and resume where
they were after PDT restarts.

The tray icon is a separate, optional app. Closing it leaves PDT running; stopping PDT is its own menu item.

## The tray app

On macOS, Windows and a Linux desktop, PDT has an icon in the menu bar or system tray. Its menu has:

- **Open**: opens PDT in your browser (on Windows, left-clicking the icon does the same).
- **Check for updates**: checks now and offers to install what it finds.
- **Restart daemon**: restarts PDT's server. Sessions are interrupted and resume when it's back.
- **Version**: the version you're running.
- **Start at login** (macOS and Windows): whether the tray icon comes back when you log in. PDT itself starts
  either way.
- **Quit tray (PDT keeps running)**: closes the tray icon only. PDT and your agents keep running.
- **Stop PDT and quit**: stops PDT and closes the tray icon. When agents are working, waiting on background tasks,
  or an update is installing, it lists them and asks first. Agents are interrupted as by an update and resume when
  PDT starts again: at your next login, or when you open the app.

If the tray says the app is in the wrong place (on macOS, a disk image or your Downloads folder), move it to
**Applications** and open it from there: PDT can't start at login or update itself from anywhere else.

## Starting at login

PDT starts at login unless you turn off **Settings › This host › Startup › Start PDT at login**. Turning it off
leaves PDT running until you stop it or log out; turning it on again doesn't restart it. The tray icon's own
**Start at login** is separate.

Where it's registered:

- **macOS**: a login item, **Personal Dev Team**, in **System Settings › General › Login Items** under **Allow in the
  Background**.
- **Windows**: a startup app, in **Settings › Apps › Startup**.
- **Linux**: a systemd user service, `pdt-host`. It stops when you log out unless you run
  `loginctl enable-linger "$USER"` once, which keeps it running from boot. `systemctl --user status pdt-host` shows
  whether it runs; `journalctl --user -u pdt-host` shows its output.

## Stopping PDT

**Stop PDT and quit** in the tray, or in a terminal:

```sh
# macOS (the one-line install puts the app in ~/Applications instead)
"/Applications/Personal Dev Team.app/Contents/MacOS/pdt-host" stop
# Linux (the tarball; for the AppImage, the app folder in PDT's data folder)
~/.local/opt/pdt/pdt-host stop
```

On Windows: `"%LOCALAPPDATA%\pdt\host\pdt-host.exe" stop`.

Stopping lasts until your next login (or until you open the app). To keep PDT from starting at login, turn off
**Start PDT at login**; to remove it, [uninstall it](#uninstall).

## Updates

PDT checks for a new release every hour (change how often under **Check for updates**, from every 15 minutes to once
a day). Choose what it does in **Settings › This host › Updates › Install updates**:

- **Install automatically when idle** (the default): installs as soon as no agent is working or waiting on
  background tasks and no run is setting up.
- **Notify only**: a banner says **Update to v… available**. **Release notes** shows what's new since your version;
  **Update** installs it.
- **Manual (no checks)**: PDT checks only when you click **Check for updates**.

If you install an update while agents are busy, PDT says what's busy and asks first. Sessions resume afterwards.

Once PDT runs the new version, every open page reloads itself, including one on your phone as soon as you come back
to it. A page where you're typing a message or have a dialog open waits: a banner says **PDT was updated**, with
**Reload**, and the page reloads by itself once you send or clear the message.

### Installing updates by themselves

With **Install automatically when idle**, the built-in **Install updates** workflow does the installing. Set it in
**Settings › This host › Updates › Installing updates**:

- **Only in a time window**: updates install by themselves only between two times, such as 02:00 to 06:00 (your
  computer's time). Outside it, an update waits for the window.
- **While agents work**: **Wait for them to finish** (the default), or install anyway after a few hours. With a time
  window, only the time agents work inside it counts. Installing anyway stops what's running; sessions resume
  afterwards.
- **Stable releases only**: on the nightly channel, a nightly build doesn't install by itself. Install it from the
  banner.
- **Check PDT after installing** (on by default): a few minutes after PDT starts on the new version, it checks that it
  runs that version and its background work keeps going.
- **Tell me what changed**: a notification linking to the release notes once PDT runs the new version, and a post to
  a chat channel such as Discord if you pick one.

An update that fails to download or install, or a failed check after installing, goes to your Inbox with what went
wrong. Switching to **Install automatically when idle** while an update waits installs that one too.

To add steps of your own, click **Customize as a workflow →**. Once you change the workflow beyond these settings,
Settings says **Updates follow your customized workflow** and offers **Open** and **Reset to default**. Turned off, no
update installs by itself.

Each update is checked against the release's checksums, and PDT copies its database before installing. If the new
version doesn't start properly, PDT puts back the version you had (on Windows, with its entry in **Settings › Apps**)
and the database copy, and a banner says **Update to v… failed (…) and was rolled back** with the reason. Nothing is lost, and PDT doesn't retry that version on its
own. See [An update rolled back](../troubleshooting/troubleshooting.md#an-update-rolled-back).

### The GitHub token for updates

PDT reads its releases from GitHub without a token: you don't need one to get updates.

A token is only for a private repository or a fork, or if you check so often that GitHub's limit for checks without
one runs out. When it does, PDT says so and checks again once the limit resets. Paste a token in
**Settings › This host › Updates › Advanced › GitHub token** and click **Save token**. It's kept in your system's
keychain. **Settings › This host › Danger zone** removes it.

If GitHub rejects the saved token (it expired or was revoked), PDT keeps updating without it and says the token was
ignored, so you can replace or remove it.

## Release channels

Choose in **Settings › This host › Updates › Update channel**:

- **Stable releases** (the default): full releases only.
- **Nightly builds too**: also a build of the latest code each night it changed. A nightly build can change your data
  in ways no stable release reads yet, so once you've installed one you can't go back to an older stable release.

PDT never installs an older version than the one you run. Switching back to **Stable releases** keeps you on the
nightly build until a newer stable release comes out.

## Uninstall

Uninstalling keeps your data. To remove that too, see [Your data](your-data.md#remove-everything).

**macOS**: stop PDT and remove it from your login items, then quit the tray icon and move **Personal Dev Team** to
the Bin:

```sh
"/Applications/Personal Dev Team.app/Contents/MacOS/pdt-host" uninstall
```

If you installed with the one-line command, the app is in the **Applications** folder in your home folder:

```sh
"$HOME/Applications/Personal Dev Team.app/Contents/MacOS/pdt-host" uninstall
```

**Windows**: uninstall **Personal Dev Team** in **Settings › Apps › Installed apps**. That also stops PDT and removes
it from startup.

**Linux**: remove the service, then delete the folder you installed into:

```sh
~/.local/opt/pdt/pdt-host uninstall
rm -r ~/.local/opt/pdt
```

For the AppImage, run `pdt-host uninstall` from the `app` folder in PDT's data folder, then delete the AppImage.
