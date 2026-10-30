// Release builds have no console window: the Windows Run key starts pdt-host at login, and a console would stay open.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Context;
use pdt_host::config::HostConfig;
use pdt_host::install::host_installer;
use pdt_host::log::log;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// How long `run` waits for the lock. A copy relaunched by self-replacement may start before the old one exits (F10).
const LOCK_WAIT: Duration = Duration::from_secs(10);

fn usage() -> ! {
    let bin = pdt_core::brand::HOST_BIN;
    eprintln!("usage: {bin} run [--install-dir <dir>]");
    eprintln!("       {bin} install [--install-dir <dir>] [--replace] [--appimage <file>]");
    eprintln!("       {bin} uninstall [--install-dir <dir>]");
    std::process::exit(2);
}

/// `--install-dir <dir>`, else the folder this executable is in.
fn install_dir(args: &[String]) -> anyhow::Result<PathBuf> {
    match args.iter().position(|a| a == "--install-dir") {
        Some(i) => Ok(PathBuf::from(args.get(i + 1).context("--install-dir needs a value")?)),
        None => Ok(std::env::current_exe()?.parent().context("no exe folder")?.to_path_buf()),
    }
}

/// Passed when an older version's data folder copy hands over to the bundle's pdt-host (macOS).
const FROM_COPY: &str = "--from-copy";

fn run(args: &[String]) -> anyhow::Result<()> {
    let mut cfg = HostConfig::from_env(install_dir(args)?).map_err(anyhow::Error::msg)?;
    // An update moves the install folder itself: through a symlink it would move the link (Linux).
    #[cfg(target_os = "linux")]
    if let Ok(dir) = std::fs::canonicalize(&cfg.install_dir) {
        cfg.install_dir = dir;
    }
    std::fs::create_dir_all(&cfg.data_dir)?;
    let exe = std::env::current_exe()?;
    let bundled = pdt_host::register::bundled_host_path(&cfg.install_dir);
    #[cfg(target_os = "macos")]
    if !cfg.dev {
        use_the_bundle(&cfg, &exe, &bundled, args);
    }
    if let Some(copy) = pdt_host::register::copy_to_hand_over_to(pdt_host::register::Os::current(), &cfg, &exe) {
        hand_over_to_copy(&cfg, &copy, args);
    }
    let lock_path = cfg.data_dir.join(format!("{}.lock", pdt_core::brand::HOST_BIN));
    let Some(lock) = pdt_host::supervise::acquire_lock(&lock_path, LOCK_WAIT)? else {
        log(&cfg, "another pdt-host is already supervising this data folder; exiting");
        if under_systemd() {
            // The unit does not restart it for this status, so it never loops against the other one.
            std::process::exit(pdt_host::register::ALREADY_RUNNING_EXIT);
        }
        return Ok(());
    };
    if let Err(e) = pdt_host::register::write_pid(&cfg.data_dir) {
        log(&cfg, &format!("could not record this process id: {e}"));
        // An earlier pdt-host's id left there would make this one step down (supervise::superseded_by).
        let _ = std::fs::remove_file(pdt_host::register::pid_path(&cfg.data_dir));
    }
    if let Err(e) = pdt_host::register::write_program(&cfg.data_dir, &exe) {
        log(&cfg, &format!("could not record this program's path: {e}"));
    }
    pdt_host::selfupdate::clean_old(&exe);
    log(&cfg, &format!("supervising {} on port {}", cfg.pdtd_path().display(), cfg.port));
    // Ruling F9 / final review #1: without an installer, or on the dev profile, keep supervising and refuse updates.
    let installer = host_installer(&mut cfg);
    let stop = AtomicBool::new(false);
    // What this process runs, read before an update can swap the bundle it came from (macOS).
    let own_digest = pdt_host::selfupdate::digest(&exe);
    let after_install = || {
        if exe == bundled {
            // macOS: the new bundle is in place; nothing is copied, the new pdt-host is just started.
            if own_digest.is_some() && pdt_host::selfupdate::digest(&bundled).is_some_and(|d| Some(d) != own_digest) {
                log(&cfg, "the release changes pdt-host itself; restarting");
                relaunch(&cfg, &bundled, args);
            }
        } else if pdt_host::selfupdate::needs_replace(&exe, &bundled) {
            log(&cfg, "the release changes pdt-host itself; replacing and restarting");
            match pdt_host::selfupdate::replace(&exe, &bundled) {
                Ok(()) => relaunch(&cfg, &exe, args),
                Err(e) => log(&cfg, &format!("could not replace pdt-host: {e}")),
            }
        }
    };
    pdt_host::supervise::run(&cfg, installer.as_ref(), &stop, &after_install);
    drop(lock);
    Ok(())
}

/// macOS runs the bundle's pdt-host, never a copy (PDT-40). An agent written by an older version still names its data
/// folder copy: the agent is rewritten (launchd reads it at the next login) and this process becomes the bundle's
/// pdt-host. Once launchd starts the bundle's pdt-host itself, the old copy is removed.
#[cfg(target_os = "macos")]
fn use_the_bundle(cfg: &HostConfig, exe: &std::path::Path, bundled: &std::path::Path, args: &[String]) {
    if let Some(home) = std::env::home_dir() {
        match pdt_host::register::migrate_launch_agent(cfg, &home) {
            Ok(true) => log(cfg, "the login item now starts the app bundle's pdt-host"),
            Ok(false) => {}
            Err(e) => log(cfg, &format!("could not update the login item: {e}")),
        }
    }
    if exe == bundled {
        if under_launchd() && !args.iter().any(|a| a == FROM_COPY) {
            pdt_host::register::remove_old_copy(&cfg.data_dir);
        }
        return;
    }
    if bundled.is_file() {
        use std::os::unix::process::CommandExt;
        log(cfg, &format!("switching to {}", bundled.display()));
        let mut cmd = std::process::Command::new(bundled);
        cmd.arg("run").args(args.iter().filter(|a| *a != FROM_COPY)).arg(FROM_COPY);
        let e = cmd.exec();
        log(cfg, &format!("could not start {} ({e}); this copy keeps supervising", bundled.display()));
    }
}

/// Windows: starts the data folder's copy with the same arguments and exits (see `copy_to_hand_over_to`). Returns
/// (and this one goes on as usual) only when the copy cannot start.
fn hand_over_to_copy(cfg: &HostConfig, copy: &std::path::Path, args: &[String]) {
    log(cfg, &format!("started from the install folder; handing over to {}", copy.display()));
    let mut cmd = std::process::Command::new(copy);
    cmd.arg("run").args(args);
    pdt_host::process::detached_no_window(&mut cmd);
    match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => log(cfg, &format!("could not start {} ({e}); running from the install folder", copy.display())),
    }
}

fn under_launchd() -> bool {
    std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == pdt_core::brand::HOST_LABEL)
}

/// Started by its systemd user unit (Linux), which restarts it whenever it exits.
fn under_systemd() -> bool {
    std::env::var(pdt_host::register::SERVICE_ENV).is_ok_and(|v| v == "systemd")
}

/// `install --replace` (macOS): take over from a pdt-host another copy of the app runs (the tray's "Switch to this
/// copy", PDT-51).
const REPLACE: &str = "--replace";

/// `install --appimage <file>` (Linux): run from a mounted AppImage, it first copies the AppImage's payload to the
/// install folder (`<data dir>/app` when none is given) and registers that.
const APPIMAGE: &str = "--appimage";

/// Registers pdt-host to start at login (Windows: a copy in the data folder; macOS: the bundle's; Linux: the install
/// folder's, as a systemd user service).
fn install(args: &[String]) -> anyhow::Result<()> {
    let appimage = match args.iter().position(|a| a == APPIMAGE) {
        Some(i) => Some(PathBuf::from(args.get(i + 1).context("--appimage needs a value")?)),
        None => None,
    };
    let dir = match &appimage {
        Some(_) if !args.iter().any(|a| a == "--install-dir") => {
            let probe = HostConfig::from_env(PathBuf::new()).map_err(anyhow::Error::msg)?;
            pdt_host::linux::appimage_install_dir(&probe.data_dir)
        }
        _ => install_dir(args)?,
    };
    let cfg = HostConfig::from_env(dir).map_err(anyhow::Error::msg)?;
    let fail = |e: String| {
        // Also logged: the installer's hook runs this without a console.
        log(&cfg, &format!("registering pdt-host failed: {e}"));
        anyhow::Error::msg(e)
    };
    if let Some(appimage) = &appimage {
        copy_appimage_payload(&cfg, appimage).map_err(fail)?;
    }
    pdt_host::register::install(&cfg, args.iter().any(|a| a == REPLACE)).map_err(fail)?;
    if cfg!(target_os = "linux") {
        println!(
            "{} is registered as the systemd user service {}. To start it at boot without logging in, run: \
             loginctl enable-linger \"$USER\"",
            pdt_core::brand::HOST_BIN,
            pdt_core::brand::HOST_SYSTEMD_UNIT
        );
    }
    Ok(())
}

/// Copies the mounted AppImage's pdtd, pdt-host and web UI (beside this program) to the install folder, unless a
/// pdt-host runs from there already or this program is the installed one.
fn copy_appimage_payload(cfg: &HostConfig, appimage: &std::path::Path) -> Result<(), String> {
    if cfg.dev {
        return Err("only the stable profile is started at login; unset PDT_PROFILE to register pdt-host".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bin = exe.parent().ok_or("no exe folder")?;
    let installed = std::fs::canonicalize(&cfg.install_dir).ok();
    if installed.is_some_and(|d| std::fs::canonicalize(bin).is_ok_and(|b| b == d)) {
        return Ok(());
    }
    if pdt_host::register::host_running(&cfg.data_dir) && cfg.pdtd_path().is_file() {
        return Ok(());
    }
    let appimage = std::path::absolute(appimage).map_err(|e| e.to_string())?;
    pdt_host::linux::install_payload(bin, &cfg.install_dir, &appimage)
}

/// Stops this data folder's pdt-host and pdtd and removes the login registration.
fn uninstall(args: &[String]) -> anyhow::Result<()> {
    let cfg = HostConfig::from_env(install_dir(args)?).map_err(anyhow::Error::msg)?;
    log(&cfg, "uninstalling: stopping pdt-host and pdtd, removing the login registration");
    pdt_host::register::uninstall(&cfg).map_err(anyhow::Error::msg)
}

/// Starts the new pdt-host and exits. Under launchd or systemd, exiting is enough: KeepAlive (Restart=always) starts
/// the new one. pdtd keeps
/// running; the new pdt-host adopts it. Returns (and this one keeps supervising) only when the new one cannot start.
fn relaunch(cfg: &HostConfig, exe: &std::path::Path, args: &[String]) {
    if under_launchd() || under_systemd() {
        std::process::exit(0);
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("run").args(args.iter().filter(|a| *a != FROM_COPY));
    pdt_host::process::detached_no_window(&mut cmd);
    match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => log(cfg, &format!("could not start the replaced pdt-host ({e}); this copy keeps supervising")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("install") => install(&args[1..]),
        Some("uninstall") => uninstall(&args[1..]),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("{}: {e:#}", pdt_core::brand::HOST_BIN);
        std::process::exit(1);
    }
}
