# Tray app, CI releases and gh updates: known limitations

- **Local builds report 0.1.0.** `scripts/release-stable.*` builds with the checked-in version, so a locally installed
  stable app always sees the newest GitHub release as newer and offers it. Accepting replaces the local build with the
  CI build. CI's release builds report `0.1.<run number>` (the tray via the Tauri config, pdtd via
  `PDT_BUILD_VERSION`), and PR builds report 0.1.0.
- **The macOS paths are only checked by CI and review.** The Dock-icon hiding, the `open -na … --args --app=` launch,
  the gh lookup in Homebrew's folders and the sh update helper have not been run on a Mac. The helper script is
  generated and asserted in unit tests, and `sh -n` accepts it. The CI `.app` is not notarized: gh downloads carry no
  quarantine flag, and the helper also removes it.
- **Update failures are only logged.** The helper logs to `<data dir>/logs/update.log`. If the installer fails, the helper
  still relaunches the installed (old) app, and the app shows no error.
- **Release selection.** `gh release view` returns the repository's *latest* release. Tags are compared as X.Y.Z, and
  pre-release suffixes are ignored.
- **Open is a browser window.** Each Open (or second launch) makes a new Edge/Chrome app window. Closing it does not
  stop pdtd; Quit in the tray does.
- **Long actions don't overlap.** While startup, Restart daemon, an update download or Quit is running, Restart and
  Quit clicks are ignored.
- **Skipped release numbers.** A running main CI run is never cancelled. A main run still queued when a newer push
  arrives is superseded by it, so that commit gets no release and its number is skipped.
- **Deferred from review:**
  - `release-stable.sh` passes all tray pids to one `kill -0` loop, which ends as soon as any one of them has exited.
  - gh is looked up only on PATH plus Homebrew's two folders (macOS). Other installs, such as MacPorts or a custom
    prefix, show "gh not installed".
  - The macOS release asset is built on `macos-latest`, which is arm64 only. Intel Macs get no working update.
- **Single-instance race (Windows, in tauri-plugin-single-instance).** If a second launch comes before the first
  instance has created its message window (the first few hundred ms), the plugin finds the mutex but no window, and
  the second instance keeps running. The result is two tray icons sharing one pdtd. This was seen when two launches
  started at the same moment.
