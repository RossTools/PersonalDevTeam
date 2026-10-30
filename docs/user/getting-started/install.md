# Install PDT

PDT runs on macOS, Windows and Linux. The quickest way to install it is one command in a terminal, below; or download
it from the [releases page](https://github.com/RossTools/PersonalDevTeam/releases) and
[install it by hand](#install-by-hand). Then go on to [the first run](first-run.md).

You also need [git](https://git-scm.com/downloads) and at least one coding agent. You can install those before or
after PDT: the first run checks for them ([Connect agents and GitHub](connect.md)).

## Install with one command

The command downloads the latest release for your system, checks it against the release's published checksums
(and stops without installing anything if they don't match), installs it without needing admin rights, and starts
it. If PDT is already installed it says so and changes nothing: PDT updates itself
([Updates](#updates)).

### macOS

In Terminal:

```sh
curl -fsSL https://pdt.dev/install.sh | sh
```

It installs **Personal Dev Team** into the **Applications** folder in your home folder and opens it once, which
starts it and adds it to your login items. PDT for macOS needs a Mac with Apple silicon.

### Linux

```sh
curl -fsSL https://pdt.dev/install.sh | sh
```

It installs PDT into `~/.local/opt/pdt` and registers it as a systemd user service, which starts it now and whenever
you log in. On a machine without a desktop, such as a server you reach over SSH, it offers to keep PDT running after
you log out and start it at boot (lingering, below). It needs an `x86_64` or `aarch64` processor and glibc 2.35 or
newer (Ubuntu 22.04 or later, or similar), and says so if your system doesn't have them.

### Windows

In PowerShell:

```powershell
irm https://pdt.dev/install.ps1 | iex
```

It downloads the installer and runs it silently. PDT is installed for you only, starts, and starts again when you
log in.

### Options

On macOS and Linux, put the options after `sh -s --`:

```sh
curl -fsSL https://pdt.dev/install.sh | sh -s -- --channel nightly
```

- `--channel nightly`: the newest [nightly build](../your-machine/install-and-updates.md#release-channels) instead
  of the newest stable release. To keep getting nightly builds afterwards, choose them in PDT's update settings.
- `--version 0.3.0`: that exact release.
- `--force`: install again over an existing install.
- `--yes`: answer yes to its question (on Linux, whether to keep PDT running after you log out).

On Windows, `irm | iex` can't take options, so set them first, in the same PowerShell window:

```powershell
$env:PDT_CHANNEL = 'nightly'   # or: $env:PDT_VERSION = '0.3.0'; $env:PDT_FORCE = '1'
irm https://pdt.dev/install.ps1 | iex
```

The same variables (`PDT_CHANNEL`, `PDT_VERSION`, `PDT_FORCE`, and `PDT_YES`) work on macOS and Linux too. If
downloading the releases needs a GitHub token, set `GITHUB_TOKEN` to one that can read the repository before you
run the command.

## Install by hand

Download the file for your system from the
[releases page](https://github.com/RossTools/PersonalDevTeam/releases) and install it as below.

### macOS

1. Download the file ending in `.app.tar.gz`.
2. Unpack it and move **Personal Dev Team** into **Applications**. Run it from there, not from your Downloads folder
   or a disk image: PDT can't update itself from those.
3. Open it once. PDT starts, opens in your browser and adds itself to the menu bar. From now on it starts when you
   log in.

macOS may ask whether Personal Dev Team can find devices on your local network. Allow it if you'll connect PDT to
anything on your network; you can change it later in **System Settings › Privacy & Security › Local Network**.

### Windows

1. Download the file ending in `-setup.exe`.
2. Run it. PDT starts, opens in your browser and adds itself to the system tray. From now on it starts when you log
   in.

### Linux

There are two downloads for each processor (`x86_64` and `aarch64`); you need glibc 2.35 or newer (Ubuntu 22.04 or
later, or similar).

**On a desktop**, download `pdt-linux-<arch>.AppImage`, make it executable and run it:

```sh
chmod +x pdt-linux-x86_64.AppImage
./pdt-linux-x86_64.AppImage
```

PDT sets itself up to start when you log in, then opens in your browser.

**On a server or without a desktop**, download `pdt-linux-<arch>.tar.gz`, unpack it somewhere you own and register
it:

```sh
mkdir -p ~/.local/opt
tar -xzf pdt-linux-x86_64.tar.gz -C ~/.local/opt
~/.local/opt/pdt/pdt-host install
```

That installs PDT as a systemd user service. A user service stops when you log out; to keep PDT running from boot,
run this once:

```sh
loginctl enable-linger "$USER"
```

Then open `http://127.0.0.1:7420` in a browser on that machine, or set up remote access to reach it from elsewhere.

Unpack it into a folder you can write to (like `~/.local/opt`), not one owned by root such as `/opt`: PDT updates
itself by replacing that folder.

## Updates

PDT keeps itself up to date. It checks for a new release every hour, installs it when PDT is idle, checks that the new
version starts properly, and goes back to the version you had if it doesn't. **Settings › This host › Updates** shows
the version you have and has **Check for updates**; if a check can't reach the releases, it says why.

## Next

[The first run](first-run.md).
