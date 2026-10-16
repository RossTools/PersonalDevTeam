# Phase 2 → later carry-over

- **Verify by hand (Mac mini checklist):** `tailscale serve --https=<port> off` without `--bg`; that serve preserves the
  Host header; the live format of GitHub-Authentication-Token-Expiration; the Tauri NSIS installer's behaviour with the
  tray running during a silent update; launchd KeepAlive restarting pdt-host after self-replacement.
- **Codex activity is screen-read** (approval markers in pdt-core::activity); Phase 3's app-server adapter replaces it.
- **Runs restart only on request** when their branch changes; dev servers with hot reload pick up changes themselves.
- **A resumed run that moves maps only its new port**; its old port is never unserved, so a mapping can linger to a
  port nobody serves (harmless; `tailscale serve reset` clears it).
- **Windows run steps** go through a generated `.cmd` with a `call` prefix on the command: no `if`/`for` at the start
  of a command, a first word that is a path needs backslashes (so `.pdt/project.toml` uses `cargo run -q -p pdtd`
  rather than `target/debug/pdtd`), `%` is expanded a second time, and a shim's exit code can be lost in `a && b`.
  Readiness probes 127.0.0.1 and ::1.
- **Fork PRs** cannot be run until the git integration phase (only branches on origin).
- **Owner identity** compares Tailscale-User-Login case-insensitively; RFC 2047 values are decoded (Q only).
- **Windows rollback** restores a copy of the install folder; the uninstall registry entry keeps the newer version.
- **Tagged-device access** is refused; app-capability headers (Tailscale ≥ 1.92) could allow chosen tags later.
