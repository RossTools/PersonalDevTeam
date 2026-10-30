# Install and updates

How PDT runs on your machine, starts at login, updates itself and rolls back when an update goes wrong. To install it
the first time, see [Install PDT](../getting-started/install.md).

## How PDT runs

PDT is a small server on your machine with a web page in front of it. You use it in your browser, at
`http://127.0.0.1:7420` on the machine itself, or from your phone with [remote access](remote-access.md).

A supervisor keeps the server running: it starts it at login, starts it again if it stops, and installs updates. Your
agents and runs carry on when you close the browser, and resume where they were after PDT restarts.

## The tray app

On macOS and Windows, and on a Linux desktop, PDT shows an icon in the menu bar or system tray. Its menu has:

- **Open**: opens PDT in your browser (on Windows, a left-click on the icon does the same).
- **Check for updates**: checks now, and offers to install an update it finds. While an update is under way it says
  so (**Downloading the update…**, **Installing the update…**).
- **Restart daemon**: restarts PDT's server. Sessions are interrupted and resume when it's back.
- **Version**: the version you're running.
- **Start at login** (macOS and Windows): whether the tray icon comes back when you log in.
- **Quit**: closes the tray icon only. PDT keeps running, and so do your agents.

The tray is optional: PDT runs without it, and opening the app again just opens PDT in your browser.

If the tray says the app is in the wrong place (on macOS, run from a disk image or your Downloads folder), move it to
**Applications** and open it from there: PDT can't start at login or update itself from anywhere else.

## Starting at login

PDT starts when you log in, without opening a browser window:

- **macOS**: as a login item for **Personal Dev Team**. macOS lists it in **System Settings › General › Login Items**, under
  **Allow in the Background**.
- **Windows**: as a startup app, listed in **Settings › Apps › Startup**.
- **Linux**: as a systemd user service, `pdt-host`. It stops when you log out unless you turn on lingering
  (`loginctl enable-linger "$USER"`), which keeps it running from boot. Check it with
  `systemctl --user status pdt-host`; its output is in `journalctl --user -u pdt-host`.

Turning off **Start at login** in the tray only stops the tray icon coming back; PDT itself still starts.

## Updates

PDT checks for a new release every hour. What it does when it finds one is up to you, in **Settings › This host ›
Updates › Install updates**:

- **Install automatically when idle** (the default): it installs the update as soon as no agent is working and no run
  is setting up. A version that failed to install once isn't tried again on its own.
- **Notify only**: a banner at the top of PDT says **Update to v… available**. **Release notes** shows what's new in
  every release since yours; **Update** installs it.
- **Manual (no checks)**: PDT doesn't check. **Check for updates** still checks when you click it.

**Check for updates** sets how often it looks (every 15 minutes to once a day).

When you install an update by hand while an agent is working, PDT says what's busy and asks before going ahead.
Sessions resume afterwards.

### What happens during an update

1. PDT downloads the new version and checks it against the release's checksums.
2. It copies its database, stops, installs the new version and starts it.
3. It checks that the new version starts and answers properly. If it does, the page reloads itself and a banner says
   the update installed.
4. If it doesn't, PDT puts back the version you had and the database copy, and starts that. The banner says **Update
   to v… failed (…) and was rolled back**, with the reason.

Nothing is lost when an update rolls back: you're on the version you had, with your data as it was. **Dismiss** the
banner and carry on. If the same version keeps failing, see [Troubleshooting](../troubleshooting/troubleshooting.md#an-update-rolled-back).

### The GitHub token for updates

PDT reads its releases from GitHub. If it says it can't check (**Couldn't check: …**), paste a GitHub token in
**Settings › This host › Updates › GitHub token** and click **Save token**. Any token that can read public
repositories will do. It's kept in your system's keychain. The same token files bug reports you send from PDT
(**Report a bug…**). **Settings › This host › Danger zone** removes it.

## Release channels

**Settings › This host › Updates › Update channel**:

- **Stable releases**: full releases only. The default.
- **Nightly builds too**: also a build of the latest code every night it changed, to try what's coming. They can
  change the database in ways no stable release understands yet, so once you've installed one you can't go back to an
  older stable release.

Switching back to **Stable releases** never installs an older version: you stay on the nightly build until a newer
stable release comes out, then move to it. PDT never downgrades.

## Uninstall

PDT keeps your data when you uninstall it. To remove that too, see [Your data](your-data.md#remove-everything).

**macOS**: stop PDT and take it out of your login items, then delete the app:

```sh
"/Applications/Personal Dev Team.app/Contents/MacOS/pdt-host" uninstall
```

Then quit the tray icon and move **Personal Dev Team** from **Applications** to the Bin. If you installed it with
[the install command](../getting-started/install.md#install-with-one-command), it's in the **Applications** folder in
your home folder instead:

```sh
"$HOME/Applications/Personal Dev Team.app/Contents/MacOS/pdt-host" uninstall
```

**Windows**: uninstall **Personal Dev Team** in **Settings › Apps › Installed apps**. That stops PDT and removes it
from startup too.

**Linux**: stop the service and remove it, then delete the folder you installed into:

```sh
~/.local/opt/pdt/pdt-host uninstall
rm -r ~/.local/opt/pdt
```

For the AppImage, run `pdt-host uninstall` from the `app` folder in PDT's data folder, then delete the AppImage.
