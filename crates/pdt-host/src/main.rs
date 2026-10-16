use anyhow::{Context, bail};
use pdt_host::config::HostConfig;
use pdt_host::install::{Installer, Unsupported, installer_for};
use pdt_host::log::log;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// How long `run` waits for the lock. A copy relaunched by self-replacement may start before the old one exits (F10).
const LOCK_WAIT: Duration = Duration::from_secs(10);

fn usage() -> ! {
    eprintln!("usage: {} run [--install-dir <dir>]", pdt_core::brand::HOST_BIN);
    std::process::exit(2);
}

/// `--install-dir <dir>`, else the folder this executable is in.
fn install_dir(args: &[String]) -> anyhow::Result<PathBuf> {
    match args.iter().position(|a| a == "--install-dir") {
        Some(i) => Ok(PathBuf::from(args.get(i + 1).context("--install-dir needs a value")?)),
        None => Ok(std::env::current_exe()?.parent().context("no exe folder")?.to_path_buf()),
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let cfg = HostConfig::from_env(install_dir(args)?).map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&cfg.data_dir)?;
    let lock_path = cfg.data_dir.join(format!("{}.lock", pdt_core::brand::HOST_BIN));
    let Some(lock) = pdt_host::supervise::acquire_lock(&lock_path, LOCK_WAIT)? else {
        log(&cfg, "another pdt-host is already supervising this data folder; exiting");
        return Ok(());
    };
    let exe = std::env::current_exe()?;
    pdt_host::selfupdate::clean_old(&exe);
    if !cfg.pdtd_path().is_file() {
        bail!("{} not found", cfg.pdtd_path().display());
    }
    log(&cfg, &format!("supervising {} on port {}", cfg.pdtd_path().display(), cfg.port));
    // Ruling F9: without an installer (e.g. a dev build outside an .app) keep supervising and refuse updates.
    let installer: Box<dyn Installer> = installer_for(&cfg).unwrap_or_else(|reason| {
        log(&cfg, &format!("updates cannot be installed here: {reason}"));
        Box::new(Unsupported { reason })
    });
    let stop = AtomicBool::new(false);
    let bundled = cfg.install_dir.join(format!("{}{}", pdt_core::brand::HOST_BIN, std::env::consts::EXE_SUFFIX));
    let after_install = || {
        if exe != bundled && pdt_host::selfupdate::needs_replace(&exe, &bundled) {
            log(&cfg, "the release changes pdt-host itself; replacing and restarting");
            match pdt_host::selfupdate::replace(&exe, &bundled) {
                Ok(()) => relaunch(&exe, args),
                Err(e) => log(&cfg, &format!("could not replace pdt-host: {e}")),
            }
        }
    };
    pdt_host::supervise::run(&cfg, installer.as_ref(), &stop, &after_install);
    drop(lock);
    Ok(())
}

/// Starts the replaced pdt-host and exits. Under launchd, exiting is enough: KeepAlive starts the new copy. pdtd keeps
/// running; the new pdt-host adopts it.
fn relaunch(exe: &std::path::Path, args: &[String]) -> ! {
    let under_launchd = std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == pdt_core::brand::HOST_LABEL);
    if !under_launchd {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("run").args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000 | 0x0000_0200);
        }
        let _ = cmd.spawn();
    }
    std::process::exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("{}: {e:#}", pdt_core::brand::HOST_BIN);
        std::process::exit(1);
    }
}
