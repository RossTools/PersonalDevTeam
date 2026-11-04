# Install PDT

PDT runs on macOS (Apple silicon or Intel), Windows and Linux (`x86_64` or `aarch64`, glibc 2.35 or newer, such as Ubuntu
22.04 or later). You also need [git](https://git-scm.com/downloads) and at least one coding agent, before or after
installing PDT: [the first run](first-run.md) checks for them.

## Install with one command

The command downloads the latest release for your system, checks it against the release's checksums (and installs
nothing if they don't match), installs it without admin rights and starts it. If PDT is already installed, it changes
nothing.

**macOS and Linux**, in a terminal:

```sh
curl -fsSL https://pdt.dev/install.sh | sh
```

- On macOS it installs **Personal Dev Team** into the **Applications** folder in your home folder and opens it, which
  adds it to your login items.
- On Linux it installs into `~/.local/opt/pdt` as a systemd user service that starts now and when you log in. On a
  machine without a desktop it offers to keep PDT running after you log out and start it at boot.

**Windows**, in PowerShell:

```powershell
irm https://pdt.dev/install.ps1 | iex
```

It runs the installer silently, for you only. PDT starts now and when you log in.

### Options

On macOS and Linux, put options after `sh -s --`:

```sh
curl -fsSL https://pdt.dev/install.sh | sh -s -- --channel nightly
```

- `--channel nightly`: the newest [nightly build](../your-machine/updates.md#release-channels). To keep getting
  nightly builds, choose them in PDT's update settings too.
- `--version 0.3.0`: that exact release.
- `--force`: install again over an existing install.
- `--yes`: answer yes to its question (on Linux, whether to keep PDT running after you log out).

On Windows, set environment variables first, in the same PowerShell window:

```powershell
$env:PDT_CHANNEL = 'nightly'   # or: $env:PDT_VERSION = '0.3.0'; $env:PDT_FORCE = '1'
irm https://pdt.dev/install.ps1 | iex
```

With `PDT_FORCE`, a PDT that is running is stopped first, and the script checks that the PDT it starts afterwards is
the version it installed.

`PDT_CHANNEL`, `PDT_VERSION` and `PDT_FORCE` work on macOS and Linux too, where `PDT_YES` also answers the script's
questions. You don't need a GitHub token.
Set `GITHUB_TOKEN` only to install from a private fork, or if GitHub says its rate limit is used up.

## Install by hand

Download the file for your system from the [releases page](https://github.com/RossTools/PersonalDevTeam/releases).

**macOS**:

1. Download the file ending in `.app.tar.gz`.
2. Unpack it and move **Personal Dev Team** into **Applications** (not your Downloads folder or a disk image: PDT
   can't update itself from there).
3. Open it. PDT starts, opens in your browser and adds itself to the menu bar and your login items.

If macOS asks whether Personal Dev Team can find devices on your local network, allow it if you'll connect PDT to
anything on your network. You can change it later in **System Settings › Privacy & Security › Local Network**.

**Windows**: download the file ending in `-setup.exe` and run it. PDT starts, opens in your browser and adds itself
to the system tray and your startup apps.

**Linux desktop**: download `pdt-linux-<arch>.AppImage`, make it executable and run it. It sets itself up to start
at login and opens in your browser.

```sh
chmod +x pdt-linux-x86_64.AppImage
./pdt-linux-x86_64.AppImage
```

**Linux server** (no desktop): download `pdt-linux-<arch>.tar.gz`, unpack it into a folder you own (not one owned
by root such as `/opt`: PDT updates itself by replacing that folder) and register it as a systemd user service:

```sh
mkdir -p ~/.local/opt
tar -xzf pdt-linux-x86_64.tar.gz -C ~/.local/opt
~/.local/opt/pdt/pdt-host install
loginctl enable-linger "$USER"   # keep it running when you're logged out, from boot
```

Open `http://127.0.0.1:7420` on that machine, or set up [remote access](../your-machine/remote-access.md) to reach
it from your laptop or phone.

## Updates

PDT updates itself: see [Running and updating PDT](../your-machine/updates.md).

## Next

[The first run](first-run.md).
