# Running and updating PDT

How PDT runs on your machine, starts at login, updates itself and how to remove it. To install it, see
[Install PDT](../getting-started/install.md).

## How PDT runs

PDT is a small server on your machine that you use in your browser, at `http://127.0.0.1:7420`, or from your phone
with [remote access](remote-access.md). A supervisor starts it at login, restarts it if it stops and installs
updates. Agents and runs carry on when you close the browser, and resume where they were after PDT restarts.

## The tray app

On macOS, Windows and a Linux desktop, PDT has an icon in the menu bar or system tray. Its menu has:

- **Open**: opens PDT in your browser (on Windows, left-clicking the icon does the same).
- **Check for updates**: checks now and offers to install what it finds.
- **Restart daemon**: restarts PDT's server. Sessions are interrupted and resume when it's back.
- **Version**: the version you're running.
- **Start at login** (macOS and Windows): whether the tray icon comes back when you log in. PDT itself starts
  either way.
- **Quit**: closes the tray icon only. PDT and your agents keep running.

If the tray says the app is in the wrong place (on macOS, a disk image or your Downloads folder), move it to
**Applications** and open it from there: PDT can't start at login or update itself from anywhere else.

## Starting at login

- **macOS**: a login item, **Personal Dev Team**, in **System Settings › General › Login Items** under **Allow in the
  Background**.
- **Windows**: a startup app, in **Settings › Apps › Startup**.
- **Linux**: a systemd user service, `pdt-host`. It stops when you log out unless you run
  `loginctl enable-linger "$USER"` once, which keeps it running from boot. `systemctl --user status pdt-host` shows
  whether it runs; `journalctl --user -u pdt-host` shows its output.

## Updates

PDT checks for a new release every hour (change how often under **Check for updates**, from every 15 minutes to once
a day). Choose what it does in **Settings › This host › Updates › Install updates**:

- **Install automatically when idle** (the default): installs as soon as no agent is working or waiting on
  background tasks and no run is setting up.
- **Notify only**: a banner says **Update to v… available**. **Release notes** shows what's new since your version;
  **Update** installs it.
- **Manual (no checks)**: PDT checks only when you click **Check for updates**.

If you install an update while agents are busy, PDT says what's busy and asks first. Sessions resume afterwards.

Each update is checked against the release's checksums, and PDT copies its database before installing. If the new
version doesn't start properly, PDT puts back the version you had (on Windows, with its entry in **Settings › Apps**)
and the database copy, and a banner says **Update to v… failed (…) and was rolled back** with the reason. Nothing is lost, and PDT doesn't retry that version on its
own. See [An update rolled back](../troubleshooting/troubleshooting.md#an-update-rolled-back).

### The GitHub token for updates

PDT reads its releases from GitHub. If it says **Couldn't check: …**, paste a GitHub token that can read public
repositories in **Settings › This host › Updates › GitHub token** and click **Save token**. It's kept in your
system's keychain, and is also used to file bug reports you send with **Report a bug…**. **Settings › This host ›
Danger zone** removes it.

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
