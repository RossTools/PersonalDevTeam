# Install PDT

PDT runs on macOS, Windows and Linux. Download the latest release from the
[releases page](https://github.com/RossTools/PersonalDevTeam/releases) for your system, install it as below, then
go on to [the first run](first-run.md).

You also need [git](https://git-scm.com/downloads) and at least one coding agent. You can install those before or
after PDT: the first run checks for them ([Connect agents and GitHub](connect.md)).

## macOS

1. Download the file ending in `.app.tar.gz`.
2. Unpack it and move **Personal Dev Team** into **Applications**. Run it from there, not from your Downloads folder
   or a disk image: PDT can't update itself from those.
3. Open it once. PDT starts, opens in your browser and adds itself to the menu bar. From now on it starts when you
   log in.

macOS may ask whether Personal Dev Team can find devices on your local network. Allow it if you'll connect PDT to
anything on your network; you can change it later in **System Settings › Privacy & Security › Local Network**.

## Windows

1. Download the file ending in `-setup.exe`.
2. Run it. PDT starts, opens in your browser and adds itself to the system tray. From now on it starts when you log
   in.

## Linux

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
