use pdt_core::brand::{DAEMON_BIN, HOST_BIN, PRODUCT_NAME, Profile};
use pdt_host::health::Probe;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Attaches to a running daemon, or starts pdt-host (which starts pdtd) and waits until pdtd answers its health check.
pub fn ensure_daemon(
    port: u16,
    probe: impl Fn(u16) -> Probe,
    spawn: impl FnOnce() -> std::io::Result<()>,
    timeout: Duration,
) -> Result<(), String> {
    let mut last = probe(port);
    match last {
        Probe::Ours { .. } => return Ok(()),
        Probe::Free => spawn().map_err(|e| format!("failed to start {HOST_BIN}: {e}"))?,
        Probe::Foreign(_) => {}
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        last = probe(port);
        if matches!(last, Probe::Ours { .. }) {
            return Ok(());
        }
    }
    Err(match last {
        Probe::Foreign(why) => format!("port {port} is in use by something that is not {PRODUCT_NAME} ({why})"),
        _ => format!("{DAEMON_BIN} did not start listening on port {port} within {timeout:?}"),
    })
}

/// The desktop app's profile. `PDT_PROFILE=dev` selects Dev; an unset (or empty) `PDT_PROFILE` selects Dev in debug
/// builds, so a stray `cargo run` never stops, restarts or updates the stable app.
pub fn desktop_profile(env: Option<&str>, debug_build: bool) -> Profile {
    match env {
        None | Some("") if debug_build => Profile::Dev,
        v => Profile::from_env_value(v),
    }
}

/// The `PDT_*` variables the tray starts pdt-host with: the profile, and the extras pdt-host passes on to pdtd (e.g.
/// `PDT_WEB_DIR` in development). Port and data dir follow from the profile, the same for tray, pdt-host and pdtd.
fn host_env(profile: Profile, lookup: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let profile = (profile == Profile::Dev).then(|| ("PDT_PROFILE".to_string(), "dev".to_string()));
    let extras = pdt_host::config::FORWARDED_ENV
        .iter()
        .filter_map(|k| lookup(k).filter(|v| !v.trim().is_empty()).map(|v| (k.to_string(), v)));
    profile.into_iter().chain(extras).collect()
}

/// A pdt-host command with exactly `host_env`'s `PDT_*` variables.
fn host_command(program: &Path, profile: Profile) -> Command {
    let mut cmd = Command::new(program);
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    cmd.envs(host_env(profile, |k| std::env::var(k).ok()));
    cmd
}

/// The data dir pdt-host and pdtd use for `profile` (the tray never passes `PDT_DATA_DIR` on).
pub fn data_dir(profile: Profile) -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join(profile.data_dir_name()))
}

/// Where the desktop app sends a spawned pdt-host's stdout/stderr (startup errors land here before the logs).
pub fn launch_log_path(profile: Profile) -> Option<PathBuf> {
    Some(data_dir(profile)?.join("logs").join(format!("{DAEMON_BIN}-launch.log")))
}

pub fn with_log_tail(msg: String, profile: Profile) -> String {
    match launch_log_path(profile) {
        Some(p) => append_tail(msg, &p, 20),
        None => msg,
    }
}

fn append_tail(msg: String, log: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(log) else {
        return msg;
    };
    let all: Vec<&str> = text.lines().collect();
    let tail = all[all.len().saturating_sub(lines)..].join("\n");
    if tail.trim().is_empty() { msg } else { format!("{msg}\n\nLast lines of {}:\n{tail}", log.display()) }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from))
}

/// The pdt-host beside this exe (the installed sidecar, or target/<profile> in development), else whatever is on PATH.
fn bundled_host(exe_dir: Option<&Path>) -> PathBuf {
    let file = format!("{HOST_BIN}{}", std::env::consts::EXE_SUFFIX);
    exe_dir.map(|d| d.join(&file)).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(file))
}

/// The pdt-host to run: the data folder's copy when there is one (installers must be able to replace the install
/// folder, decision 14), else the one beside this exe (first run, development).
fn host_to_run(data_dir: Option<&Path>, exe_dir: Option<&Path>) -> PathBuf {
    data_dir
        .map(pdt_host::register::installed_host_path)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| bundled_host(exe_dir))
}

/// Starts pdt-host (which starts and supervises pdtd), detached; it keeps running if the app exits. A pdt-host that is
/// already running (login item) makes this one exit at once.
pub fn spawn_host(profile: Profile) -> std::io::Result<()> {
    let exe_dir = exe_dir();
    let mut cmd = host_command(&host_to_run(data_dir(profile).as_deref(), exe_dir.as_deref()), profile);
    cmd.arg("run");
    if let Some(dir) = exe_dir {
        cmd.arg("--install-dir").arg(dir);
    }
    let log = launch_log_path(profile).and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::File::options().create(true).append(true).open(p).ok()
    });
    let (out, err) = match log.and_then(|f| Some((f.try_clone().ok()?, f))) {
        Some((a, b)) => (Stdio::from(a), Stdio::from(b)),
        None => (Stdio::null(), Stdio::null()),
    };
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        // Own process group, so Ctrl-C in the launching terminal doesn't kill pdt-host.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(|_| ())
}

/// macOS: the installed stable app registers pdt-host's LaunchAgent once (there is no installer to do it).
pub fn ensure_registered(profile: Profile) {
    if !cfg!(target_os = "macos") || profile != Profile::Stable {
        return;
    }
    let Some(home) = std::env::home_dir() else { return };
    let exe_dir = exe_dir();
    let installed_bundle = exe_dir.as_deref().and_then(pdt_host::install::bundle_of).is_some();
    if installed_bundle && !pdt_host::register::launch_agent_path(&home).exists() {
        let mut cmd = host_command(&bundled_host(exe_dir.as_deref()), profile);
        cmd.arg("install");
        if let Some(d) = exe_dir {
            cmd.arg("--install-dir").arg(d);
        }
        let _ = cmd.status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    fn ours() -> Probe {
        Probe::Ours { version: "x".into() }
    }

    #[test]
    fn attaches_without_spawning_when_ours() {
        let spawned = Cell::new(false);
        ensure_daemon(
            1,
            |_| ours(),
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(!spawned.get());
    }

    #[test]
    fn spawns_when_free_and_waits_for_health() {
        let calls = Cell::new(0);
        let probe = |_| {
            calls.set(calls.get() + 1);
            if calls.get() < 4 { Probe::Free } else { ours() }
        };
        let spawned = Cell::new(false);
        ensure_daemon(
            1,
            probe,
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(spawned.get());
    }

    #[test]
    fn foreign_port_fails_without_spawning() {
        let spawned = Cell::new(false);
        let err = ensure_daemon(
            9,
            |_| Probe::Foreign("it identifies as \"x\"".into()),
            || {
                spawned.set(true);
                Ok(())
            },
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(!spawned.get());
        assert!(err.contains("port 9 is in use by something that is not Personal Dev Team"), "{err}");
    }

    #[test]
    fn timeout_and_spawn_failure_messages() {
        let err = ensure_daemon(9, |_| Probe::Free, || Ok(()), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
        let err = ensure_daemon(9, |_| Probe::Free, || Err(std::io::Error::other("nope")), Duration::from_millis(300))
            .unwrap_err();
        assert!(err.contains("failed to start pdt-host"), "{err}");
    }

    #[test]
    fn log_tail_is_appended() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("pdtd-launch.log");
        std::fs::write(&log, (1..=30).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let msg = append_tail("boom".into(), &log, 20);
        assert!(msg.starts_with("boom"));
        assert!(msg.contains("line 30") && msg.contains("line 11") && !msg.contains("line 10\n"));
        assert_eq!(append_tail("boom".into(), &tmp.path().join("missing.log"), 20), "boom");
    }

    #[test]
    fn debug_builds_default_to_dev() {
        assert_eq!(desktop_profile(None, true), Profile::Dev);
        assert_eq!(desktop_profile(Some(""), true), Profile::Dev);
        assert_eq!(desktop_profile(Some("stable"), true), Profile::Stable);
        assert_eq!(desktop_profile(Some("dev"), true), Profile::Dev);
        assert_eq!(desktop_profile(None, false), Profile::Stable);
        assert_eq!(desktop_profile(Some("dev"), false), Profile::Dev);
    }

    #[test]
    fn pdt_host_gets_the_profile_and_only_the_forwarded_extras() {
        let env: HashMap<&str, &str> =
            [("PDT_WEB_DIR", "/web"), ("PDT_DATA_DIR", "/elsewhere"), ("PDT_PORT", "7420"), ("PDT_SECRET_STORE", "")]
                .into();
        let lookup = |k: &str| env.get(k).map(|v| v.to_string());
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(host_env(Profile::Dev, lookup), [pair("PDT_PROFILE", "dev"), pair("PDT_WEB_DIR", "/web")]);
        assert_eq!(host_env(Profile::Stable, lookup), [pair("PDT_WEB_DIR", "/web")]);
    }

    #[test]
    fn the_data_folder_copy_of_pdt_host_wins_over_the_bundled_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (data, app) = (tmp.path().join("data"), tmp.path().join("app"));
        let file = format!("pdt-host{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join(&file), "").unwrap();
        assert_eq!(host_to_run(Some(&data), Some(&app)), app.join(&file), "no copy yet: the bundled one");
        let copy = pdt_host::register::installed_host_path(&data);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, "").unwrap();
        assert_eq!(host_to_run(Some(&data), Some(&app)), copy);
        assert_eq!(host_to_run(None, None), PathBuf::from(&file), "else whatever is on PATH");
    }
}
