use pdt_core::brand::{DAEMON_BIN, HOST_BIN, PRODUCT_NAME, Profile};
use pdt_core::handoff::{HostProblem, host_problem_path};
use pdt_host::health::Probe;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Attaches to a running daemon, or starts pdt-host (which starts pdtd) and waits until pdtd answers its health check.
/// Stops waiting when pdt-host reports a `problem` someone has to fix (say, pdtd's exe was quarantined).
pub fn ensure_daemon(
    port: u16,
    probe: impl Fn(u16) -> Probe,
    spawn: impl FnOnce() -> std::io::Result<()>,
    problem: impl Fn() -> Option<HostProblem>,
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
        if let Some(p) = problem() {
            return Err(p.message);
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

/// The extras a stable pdt-host gets from the tray. The test-only knobs (`PDT_SECRET_STORE`, `PDT_TAILSCALE_BIN`)
/// reach only a dev one.
const STABLE_EXTRAS: [&str; 3] = ["PDT_WEB_DIR", "PDT_ALLOWED_HOSTS", "PDT_OWNER_LOGIN"];

/// Whether the tray was started by pdt-host after an update (its first argument is `TRAY_QUIET_ARG`), so it must not
/// open the web UI at startup.
pub fn quiet_start(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().skip(1).any(|a| a == pdt_core::handoff::TRAY_QUIET_ARG)
}

/// The `PDT_*` variables the tray starts pdt-host with: the profile, and the extras pdt-host passes on to pdtd (e.g.
/// `PDT_WEB_DIR` in development). Port and data dir follow from the profile, the same for tray, pdt-host and pdtd.
fn host_env(profile: Profile, lookup: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let extras: &[&str] = if profile == Profile::Dev { &pdt_host::config::FORWARDED_ENV } else { &STABLE_EXTRAS };
    let profile = (profile == Profile::Dev).then(|| ("PDT_PROFILE".to_string(), "dev".to_string()));
    let extras = extras.iter().filter_map(|k| lookup(k).filter(|v| !v.trim().is_empty()).map(|v| (k.to_string(), v)));
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

/// What pdt-host reports keeps pdtd from running, if anything (see `host_problem_path`).
pub fn host_problem(profile: Profile) -> Option<HostProblem> {
    read_problem(&data_dir(profile)?)
}

fn read_problem(data_dir: &Path) -> Option<HostProblem> {
    serde_json::from_slice(&std::fs::read(host_problem_path(data_dir)).ok()?).ok()
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

/// The pdt-host to run. Windows: the data folder's copy when there is one (the installer must be able to replace the
/// install folder, decision 14), else the one beside this exe (first run, development). macOS: always the one beside
/// this exe, in the bundle (PDT-40).
fn host_to_run(os: pdt_host::register::Os, data_dir: Option<&Path>, exe_dir: Option<&Path>) -> PathBuf {
    data_dir
        .filter(|_| os == pdt_host::register::Os::Windows)
        .map(pdt_host::register::installed_host_path)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| bundled_host(exe_dir))
}

/// Starts pdt-host (which starts and supervises pdtd), detached; it keeps running if the app exits. A pdt-host that is
/// already running (login item) makes this one exit at once.
pub fn spawn_host(profile: Profile) -> std::io::Result<()> {
    let exe_dir = exe_dir();
    let mut cmd = host_command(
        &host_to_run(pdt_host::register::Os::current(), data_dir(profile).as_deref(), exe_dir.as_deref()),
        profile,
    );
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

/// macOS: the installed stable app registers pdt-host's LaunchAgent once (there is no installer to do it). True when
/// it did: `pdt-host install` has then started pdt-host through launchd (or one was already running), so the tray must
/// not start another and only waits for pdtd.
pub fn ensure_registered(profile: Profile) -> bool {
    let os = pdt_host::register::Os::current();
    let home = std::env::home_dir();
    ensure_registered_with(os, profile, exe_dir().as_deref(), home.as_deref(), &mut pdt_host::register::System)
}

fn ensure_registered_with(
    os: pdt_host::register::Os,
    profile: Profile,
    exe_dir: Option<&Path>,
    home: Option<&Path>,
    runner: &mut dyn pdt_host::register::Runner,
) -> bool {
    if os != pdt_host::register::Os::Mac || profile != Profile::Stable {
        return false;
    }
    let (Some(home), Some(exe_dir)) = (home, exe_dir) else { return false };
    if pdt_host::install::bundle_of(exe_dir).is_none()
        || pdt_host::placement::bundle_problem(exe_dir).is_some()
        || pdt_host::register::launch_agent_path(home).exists()
    {
        return false;
    }
    let mut cmd = host_command(&bundled_host(Some(exe_dir)), profile);
    cmd.arg("install").arg("--install-dir").arg(exe_dir);
    runner.status(cmd).unwrap_or(false)
}

/// macOS: why this app cannot stay where it runs from (translocated, read-only folder; PDT-51), so it is never
/// registered at login and cannot update.
pub fn own_problem() -> Option<String> {
    if pdt_host::register::Os::current() != pdt_host::register::Os::Mac {
        return None;
    }
    pdt_host::placement::bundle_problem(&exe_dir()?)
}

/// Another copy of the app whose pdt-host is the one running (PDT-51).
#[derive(Debug, Clone, PartialEq)]
pub struct OtherCopy {
    pub bundle: PathBuf,
    /// Why that copy cannot stay where it is, if it cannot.
    pub problem: Option<String>,
}

/// macOS: the other copy of the app the running pdt-host comes from, when it is not this one.
pub fn other_copy(profile: Profile) -> Option<OtherCopy> {
    if pdt_host::register::Os::current() != pdt_host::register::Os::Mac {
        return None;
    }
    let running = pdt_host::register::running_program(&data_dir(profile)?)?;
    other_copy_with(&exe_dir()?, &running)
}

fn other_copy_with(exe_dir: &Path, running: &Path) -> Option<OtherCopy> {
    let own = pdt_host::install::bundle_of(exe_dir)?;
    let running_dir = running.parent()?;
    let bundle = pdt_host::install::bundle_of(running_dir)?;
    (bundle != own).then(|| OtherCopy { problem: pdt_host::placement::bundle_problem(running_dir), bundle })
}

/// "Switch to this copy": this bundle's pdt-host registers itself at login and takes over from the running one
/// (`pdt-host install --replace`). pdtd keeps running until the caller restarts it.
pub fn switch_to_this_copy(profile: Profile) -> Result<(), String> {
    let exe_dir = exe_dir().ok_or("cannot tell where this app is")?;
    switch_with(profile, &exe_dir, &mut pdt_host::register::System)
}

fn switch_with(profile: Profile, exe_dir: &Path, runner: &mut dyn pdt_host::register::Runner) -> Result<(), String> {
    let mut cmd = host_command(&bundled_host(Some(exe_dir)), profile);
    cmd.arg("install").arg("--install-dir").arg(exe_dir).arg("--replace");
    if runner.status(cmd)? {
        Ok(())
    } else {
        Err(format!("{HOST_BIN} install failed; see {}", launch_log_hint(profile)))
    }
}

fn launch_log_hint(profile: Profile) -> String {
    data_dir(profile)
        .map(|d| d.join("logs").join(format!("{HOST_BIN}.log")).display().to_string())
        .unwrap_or_else(|| format!("{HOST_BIN}.log"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    #[test]
    fn only_pdt_hosts_restart_is_quiet() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(quiet_start(args(&["/A/P.app/Contents/MacOS/p", "--no-open"])));
        assert!(!quiet_start(args(&["/A/P.app/Contents/MacOS/p"])));
        assert!(!quiet_start(args(&["--no-open"])));
    }

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
            || None,
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
            || None,
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
            || None,
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(!spawned.get());
        assert!(err.contains("port 9 is in use by something that is not Personal Dev Team"), "{err}");
    }

    #[test]
    fn timeout_and_spawn_failure_messages() {
        let err = ensure_daemon(9, |_| Probe::Free, || Ok(()), || None, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("did not start"), "{err}");
        let nope = || Err(std::io::Error::other("nope"));
        let err = ensure_daemon(9, |_| Probe::Free, nope, || None, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("failed to start pdt-host"), "{err}");
    }

    #[test]
    fn a_reported_problem_ends_the_wait_with_its_message() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_problem(tmp.path()), None);
        let p = HostProblem {
            kind: pdt_core::handoff::HostProblemKind::PdtdMissing,
            message: "pdtd.exe is missing; antivirus may have quarantined it.".into(),
            path: tmp.path().join("pdtd.exe"),
            since: 1,
        };
        std::fs::write(host_problem_path(tmp.path()), serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(read_problem(tmp.path()), Some(p));
        let t = Instant::now();
        let err = ensure_daemon(9, |_| Probe::Free, || Ok(()), || read_problem(tmp.path()), Duration::from_secs(60))
            .unwrap_err();
        assert!(err.contains("antivirus may have quarantined it"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5), "it did not wait out the timeout");
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
        let env: HashMap<&str, &str> = [
            ("PDT_WEB_DIR", "/web"),
            ("PDT_DATA_DIR", "/elsewhere"),
            ("PDT_PORT", "7420"),
            ("PDT_OWNER_LOGIN", ""),
            ("PDT_SECRET_STORE", "memory"),
            ("PDT_TAILSCALE_BIN", "/fake"),
        ]
        .into();
        let lookup = |k: &str| env.get(k).map(|v| v.to_string());
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            host_env(Profile::Dev, lookup),
            [
                pair("PDT_PROFILE", "dev"),
                pair("PDT_WEB_DIR", "/web"),
                pair("PDT_SECRET_STORE", "memory"),
                pair("PDT_TAILSCALE_BIN", "/fake")
            ]
        );
        // Test-only knobs never reach a stable pdtd from the tray.
        assert_eq!(host_env(Profile::Stable, lookup), [pair("PDT_WEB_DIR", "/web")]);
    }

    /// Records `status` commands and answers each with `ok`.
    struct Recorder {
        ran: Vec<String>,
        ok: bool,
    }

    impl pdt_host::register::Runner for Recorder {
        fn status(&mut self, cmd: Command) -> Result<bool, String> {
            let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            self.ran.push(args.join(" "));
            Ok(self.ok)
        }
        fn stdout(&mut self, _: Command) -> Result<String, String> {
            unreachable!()
        }
        fn spawn(&mut self, _: Command) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn a_first_mac_launch_registers_pdt_host_and_then_does_not_start_one() {
        use pdt_host::register::Os;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let exe_dir = tmp.path().join("Personal Dev Team.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let mut r = Recorder { ran: vec![], ok: true };
        assert!(ensure_registered_with(Os::Mac, Profile::Stable, Some(&exe_dir), Some(&home), &mut r));
        assert_eq!(r.ran, [format!("install --install-dir {}", exe_dir.display())]);

        // A failed registration leaves starting pdt-host to the tray.
        let mut r = Recorder { ran: vec![], ok: false };
        assert!(!ensure_registered_with(Os::Mac, Profile::Stable, Some(&exe_dir), Some(&home), &mut r));

        // Already registered, a dev tray, outside a bundle, or not macOS: nothing runs.
        let plist = pdt_host::register::launch_agent_path(&home);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, "").unwrap();
        let mut r = Recorder { ran: vec![], ok: true };
        assert!(!ensure_registered_with(Os::Mac, Profile::Stable, Some(&exe_dir), Some(&home), &mut r));
        std::fs::remove_file(&plist).unwrap();
        assert!(!ensure_registered_with(Os::Mac, Profile::Dev, Some(&exe_dir), Some(&home), &mut r));
        assert!(!ensure_registered_with(Os::Mac, Profile::Stable, Some(tmp.path()), Some(&home), &mut r));
        let moved =
            tmp.path().join("AppTranslocation").join("u").join("d").join("P 2.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&moved).unwrap();
        assert!(!ensure_registered_with(Os::Mac, Profile::Stable, Some(&moved), Some(&home), &mut r));
        assert!(!ensure_registered_with(Os::Windows, Profile::Stable, Some(&exe_dir), Some(&home), &mut r));
        assert!(r.ran.is_empty(), "{:?}", r.ran);
    }

    #[test]
    fn another_copys_host_is_noticed_with_what_is_wrong_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let macos = |root: &Path| root.join("Contents").join("MacOS");
        let own = tmp.path().join("Applications").join("Personal Dev Team.app");
        std::fs::create_dir_all(macos(&own)).unwrap();
        let host = |b: &Path| macos(b).join("pdt-host");
        assert_eq!(other_copy_with(&macos(&own), &host(&own)), None, "this copy's own pdt-host");

        let moved = tmp.path().join("AppTranslocation").join("u").join("d").join("Personal Dev Team 2.app");
        let other = other_copy_with(&macos(&own), &host(&moved)).unwrap();
        assert_eq!(other.bundle, moved);
        assert!(other.problem.unwrap().contains("temporary read-only folder"));

        let fine = tmp.path().join("Elsewhere").join("Personal Dev Team.app");
        std::fs::create_dir_all(macos(&fine)).unwrap();
        assert_eq!(other_copy_with(&macos(&own), &host(&fine)), Some(OtherCopy { bundle: fine, problem: None }));
        assert_eq!(other_copy_with(&macos(&own), Path::new("/data/host/pdt-host")), None, "not from a bundle");
    }

    #[test]
    fn switching_runs_this_bundles_host_with_replace() {
        let exe_dir = Path::new("/Applications/P.app/Contents/MacOS");
        let mut r = Recorder { ran: vec![], ok: true };
        switch_with(Profile::Stable, exe_dir, &mut r).unwrap();
        assert_eq!(r.ran, ["install --install-dir /Applications/P.app/Contents/MacOS --replace"]);
        let mut r = Recorder { ran: vec![], ok: false };
        assert!(switch_with(Profile::Stable, exe_dir, &mut r).unwrap_err().contains("install failed"));
    }

    #[test]
    fn on_windows_the_data_folder_copy_of_pdt_host_wins_over_the_bundled_one() {
        use pdt_host::register::Os;
        let tmp = tempfile::tempdir().unwrap();
        let (data, app) = (tmp.path().join("data"), tmp.path().join("app"));
        let file = format!("pdt-host{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join(&file), "").unwrap();
        assert_eq!(host_to_run(Os::Windows, Some(&data), Some(&app)), app.join(&file), "no copy yet: the bundled one");
        let copy = pdt_host::register::installed_host_path(&data);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, "").unwrap();
        assert_eq!(host_to_run(Os::Windows, Some(&data), Some(&app)), copy);
        assert_eq!(host_to_run(Os::Mac, Some(&data), Some(&app)), app.join(&file), "macOS runs the bundle's");
        assert_eq!(host_to_run(Os::Windows, None, None), PathBuf::from(&file), "else whatever is on PATH");
    }
}
