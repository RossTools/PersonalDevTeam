//! Drives run instances (spec §4.2): setup, command, readiness, Tailscale mapping, stop, resume after a restart.
//!
//! A detached run (`detach = true`, PDT-32) starts its command outside PDT's terminals (`pdt_agents::detached`), so
//! it keeps running when pdtd exits; the run's Output terminal follows its log. The next pdtd finds it again by the
//! `process` stored on the row and watches it (reattach) instead of starting it again.
//!
//! Tailscale (controller ruling on Task 7): a pdtd maps a run's port only once it has (re)started that run on that
//! port itself, and it never unmaps a port it did not map. The machine's other instance (stable or dev) may own any
//! port a stored row mentions.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::http::StatusCode;
use pdt_agents::SpawnSpec;
use pdt_agents::detached::{self, DetachedSpec};
use pdt_core::host::CredentialPolicy;
use pdt_core::run::{Run, RunKind, RunState, RunStep, pick_port, pty_id, step_launch};
use pdt_core::runconfig::{RunDef, RunVars, expand, run_env, uses_run_host};
use pdt_core::session::{SessionKind, is_scrubbed};
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

const PROBE_EVERY: Duration = Duration::from_millis(300);
/// The longest one readiness probe may take (a refused connection on Windows loopback can take seconds).
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long a restart waits for a stopped web run's port to come free before moving it to another one: on Windows a
/// killed process can hold its listening socket for a moment after its terminal has exited (PDT-217).
const PORT_RELEASE_WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct RunControl {
    stop: AtomicBool,
    /// Why the run was stopped by PDT rather than the user (e.g. its branch is gone).
    reason: std::sync::Mutex<Option<String>>,
    finished: AtomicBool,
    done: tokio::sync::Notify,
    /// The port of a web run this process is driving: the only run ports it maps.
    web_port: Option<u16>,
    /// The run's definition uses `$PDT_RUN_HOST` / `$PDT_RUN_URL`: it restarts when remote access changes the host.
    uses_host: bool,
    /// A detached command's processes, once started: what `stop` ends.
    process: std::sync::Mutex<Option<String>>,
}

impl RunControl {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// Whether nothing listens on `p` on either loopback: dev servers bound to `localhost` often listen only on `::1`
/// (PDT-356). Only "in use" (or "permission denied", Windows' answer for an exclusively held port) on `::1` counts as
/// taken: any other error means the machine has no IPv6 (no `::1`, or no IPv6 stack at all), so `127.0.0.1` decides.
pub(crate) fn port_is_free(p: u16) -> bool {
    if std::net::TcpListener::bind(("127.0.0.1", p)).is_err() {
        return false;
    }
    match std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, p)) {
        Ok(_) => true,
        Err(e) => !matches!(e.kind(), std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied),
    }
}

/// After stopping a web run to start it again: waits (up to [`PORT_RELEASE_WAIT`]) until its port is free, so the
/// restart keeps the port instead of moving because the old process has not let go of it yet.
pub(crate) async fn await_port_release(port: Option<u16>) {
    let Some(p) = port else { return };
    let deadline = tokio::time::Instant::now() + PORT_RELEASE_WAIT;
    loop {
        if tokio::task::spawn_blocking(move || port_is_free(p)).await.unwrap_or(true) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(port = p, "a stopped run's port is still in use; the run moves to another one");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn remote_host(st: &AppState) -> Option<String> {
    let r = st.remote.lock().unwrap_or_else(|e| e.into_inner());
    r.active.then(|| r.dns_name.clone()).flatten()
}

fn url_for(tailnet_host: Option<&str>, port: u16) -> String {
    match tailnet_host {
        Some(h) => format!("https://{h}:{port}/"),
        None => format!("http://127.0.0.1:{port}/"),
    }
}

/// The link for a web run on `port`: the tailnet when remote access is active, else loopback.
pub(crate) fn run_url(st: &AppState, port: u16) -> String {
    url_for(remote_host(st).as_deref(), port)
}

/// (run id, port) of every web run this process is driving.
pub(crate) fn driven_web_runs(st: &AppState) -> Vec<(String, u16)> {
    let controls = st.run_controls.lock().unwrap_or_else(|e| e.into_inner());
    controls.iter().filter_map(|(id, c)| c.web_port.map(|p| (id.clone(), p))).collect()
}

/// Points the links of the web runs this process drives at the tailnet (`Some(dns name)`) or at loopback, after
/// remote access was turned on or off. Their mappings are made or removed by the caller.
pub(crate) async fn relink_runs(st: &AppState, tailnet_host: Option<&str>) {
    for (id, port) in driven_web_runs(st) {
        let url = url_for(tailnet_host, port);
        if let Err(e) = st.store.set_run_port(&id, Some(port), Some(&url)).await {
            tracing::warn!(run = %id, error = %e, "could not update a run's link");
        }
    }
}

/// Restarts, in the background, the runs (not tests: they run to their end) this process drives whose definition
/// uses the run's host or link: their processes keep the old value otherwise (a dev PDT's allowlist would refuse the
/// new link). Called after remote access changed the host; each run keeps its port when it can.
pub(crate) fn restart_host_dependent_runs(st: &AppState) {
    let ids: Vec<String> = {
        let controls = st.run_controls.lock().unwrap_or_else(|e| e.into_inner());
        controls.iter().filter(|(_, c)| c.uses_host && !c.stopping()).map(|(id, _)| id.clone()).collect()
    };
    for id in ids {
        let st = st.clone();
        tokio::spawn(async move {
            tracing::info!(run = %id, "remote access changed the run's host; restarting it");
            if let Err(e) = restart_in_place(&st, &id).await {
                tracing::warn!(run = %id, error = %e, "could not restart a run for its new host");
                let msg = format!("could not start again for remote access's new address: {e}");
                let _ = st.store.finish_run(&id, RunState::Failed, None, Some(&msg), now_ms()).await;
            }
        });
    }
}

/// How many times the run was asked to stop so far, and whether it is still being driven and not stopping. Read
/// under `run_stops`, which `stop` holds while it counts and flags a stop, so a stop is either counted after this or
/// already flagged.
fn stops_and_running(st: &AppState, id: &str) -> (u64, bool) {
    let stops = st.run_stops.lock().unwrap_or_else(|e| e.into_inner());
    let running = st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|c| !c.stopping());
    (stops.get(id).copied().unwrap_or(0), running)
}

/// Stops an active run and starts it again (no run-limit check: it replaces itself). Any other stop of the run
/// before it starts again (the user's Stop, a gone branch) cancels the restart. Ok when there was nothing to do; Err
/// says why it could not start again (the run has stopped).
async fn restart_in_place(st: &AppState, id: &str) -> Result<(), String> {
    let (stops, running) = stops_and_running(st, id);
    if !running {
        return Ok(());
    }
    if !stop_inner(st, id, None, false).await {
        tracing::warn!(run = %id, "the run did not stop in time; it is not restarted");
        return Ok(());
    }
    let stopped_since = || stops_and_running(st, id).0 != stops;
    let Some(run) = st.store.get_run(id).await.map_err(|e| e.to_string())? else { return Ok(()) };
    let def = crate::runs::definition_for(st, &run).await;
    if let Ok(def) = &def
        && def.web
    {
        await_port_release(run.port).await;
    }
    let _start = st.run_start_lock.lock().await;
    let Some(run) = st.store.get_run(id).await.map_err(|e| e.to_string())? else { return Ok(()) };
    if run.state.is_active() || st.shutting_down.load(Ordering::SeqCst) || stopped_since() {
        return Ok(());
    }
    relaunch(st, run, def?).await.map(|_| ()).map_err(|e| e.message().to_string())
}

/// Starts a stopped run again: a port (its old one when free) and link for a web run, the first state, then the
/// driver. The caller holds `run_start_lock` and has checked the run is not active.
pub(crate) async fn relaunch(st: &AppState, mut run: Run, def: RunDef) -> Result<Run, ApiError> {
    let id = run.id.clone();
    // The link and the driver are settled under the lock applying remote access holds (it relinks and restarts the
    // driven runs), so a remote toggle meanwhile cannot leave this run with the other setting's link.
    let _link = st.settings_lock.lock().await;
    if def.web {
        let p = allocate_port(st, run.port, Some(&id)).await.ok_or_else(no_free_port)?;
        let url = run_url(st, p);
        st.store.set_run_port(&id, Some(p), Some(&url)).await?;
        run.port = Some(p);
        run.url = Some(url);
    }
    let first = if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting };
    st.store.set_run_state(&id, first, now_ms()).await?;
    run.state = first;
    run.exit_code = None;
    run.error = None;
    run.detached = def.detach;
    start(st, run.clone(), def);
    Ok(run)
}

pub(crate) fn no_free_port() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no_free_port", "no free port in the host's run port range")
}

fn vars(st: &AppState, run: &Run) -> RunVars {
    RunVars {
        port: run.port,
        data_dir: run.data_dir.clone(),
        url: run.url.clone(),
        host: remote_host(st).unwrap_or_else(|| "127.0.0.1".into()),
        branch: run.branch.clone(),
        owner_login: st.owner(),
    }
}

/// A free port for a new or resumed web run, avoiding every other active run's port. Callers hold
/// `run_start_lock` (or run before anything can start, at resume) so two runs cannot pick the same port.
pub(crate) async fn allocate_port(st: &AppState, prefer: Option<u16>, except_run: Option<&str>) -> Option<u16> {
    let range = st.store.get_host_settings().await.ok()?.run_ports;
    let taken: BTreeSet<u16> = st
        .store
        .list_active_runs()
        .await
        .ok()?
        .into_iter()
        .filter(|r| Some(r.id.as_str()) != except_run)
        .filter_map(|r| r.port)
        .collect();
    tokio::task::spawn_blocking(move || pick_port(range, &taken, prefer, port_is_free)).await.ok()?
}

/// Where a run step's Windows script goes (ruling F15: `<data>/runs/scripts/<run id>/<step>.cmd`).
pub(crate) fn scripts_dir(st: &AppState, run_id: &str) -> PathBuf {
    st.data_dir.join("runs").join("scripts").join(run_id)
}

/// Where a detached run's command writes its output (`<data>/runs/logs/<run id>.log`).
pub fn log_path(st: &AppState, run_id: &str) -> PathBuf {
    st.data_dir.join("runs").join("logs").join(format!("{run_id}.log"))
}

/// How to run `command` as `step`: on Windows this writes the step's script.
fn launch(st: &AppState, run: &Run, step: RunStep, command: &str) -> Result<pdt_core::run::StepLaunch, String> {
    let script = scripts_dir(st, &run.id).join(format!("{}.cmd", step.as_str()));
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
    let l = step_launch(st.platform.windows, &comspec, command, &script);
    if let Some(text) = &l.script {
        std::fs::create_dir_all(script.parent().expect("script dir")).map_err(|e| e.to_string())?;
        std::fs::write(&script, text).map_err(|e| format!("writing {}: {e}", script.display()))?;
    }
    Ok(l)
}

/// The inherited variables a run's processes must not see.
fn scrubbed_env() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| is_scrubbed(SessionKind::Shell, k, CredentialPolicy::PassThrough))
        .collect()
}

/// Blocking (it may kill a previous terminal with taskkill): run it on the blocking pool.
fn spawn_step(
    st: &AppState,
    run: &Run,
    step: RunStep,
    command: &str,
    env: &[(String, String)],
) -> Result<oneshot::Receiver<i32>, String> {
    let id = pty_id(&run.id, step);
    st.ptys.remove(&id);
    let l = launch(st, run, step, command)?;
    let spec = SpawnSpec {
        id,
        program: PathBuf::from(l.program),
        args: l.args,
        cwd: PathBuf::from(&run.cwd),
        env_remove: scrubbed_env(),
        env: env.to_vec(),
        cols: 120,
        rows: 30,
    };
    st.ptys.spawn(spec).map_err(|e| e.to_string())
}

/// Starts a detached command and follows its log in the Output terminal. Blocking. Honours a stop that arrived
/// while it was starting, like `start_step`.
fn spawn_detached(
    st: &AppState,
    ctl: &RunControl,
    run: &Run,
    command: &str,
    env: &[(String, String)],
) -> Result<detached::Detached, String> {
    let id = pty_id(&run.id, RunStep::Command);
    st.ptys.remove(&id);
    let l = launch(st, run, RunStep::Command, command)?;
    let mut env = env.to_vec();
    // Python buffers output that goes to a file, so the log would stay empty for a long time.
    if std::env::var_os("PYTHONUNBUFFERED").is_none() && !env.iter().any(|(k, _)| k == "PYTHONUNBUFFERED") {
        env.push(("PYTHONUNBUFFERED".into(), "1".into()));
    }
    let log = log_path(st, &run.id);
    let spec = DetachedSpec {
        id: run.id.clone(),
        program: PathBuf::from(l.program),
        args: l.args,
        cwd: PathBuf::from(&run.cwd),
        env_remove: scrubbed_env(),
        env,
        log: log.clone(),
    };
    let d = detached::spawn(&spec)?;
    *ctl.process.lock().unwrap_or_else(|e| e.into_inner()) = Some(d.process.clone());
    // `stop` sets the flag before reading `process`, so either it ends these processes or this does.
    if ctl.stopping() {
        detached::terminate(&d.process);
    }
    if let Err(e) = st.ptys.follow_log(&id, log) {
        tracing::warn!(run = %run.id, error = %e, "could not follow a detached run's log");
    }
    Ok(d)
}

/// Starts a step, then honours a stop that arrived while it was starting: `stop` sets the flag before killing, so
/// either it sees this step's terminal or this check sees the flag.
async fn start_step(
    st: &AppState,
    ctl: &Arc<RunControl>,
    run: &Run,
    step: RunStep,
    command: String,
    env: &[(String, String)],
) -> Result<oneshot::Receiver<i32>, String> {
    let (st, ctl, run, env) = (st.clone(), ctl.clone(), run.clone(), env.to_vec());
    tokio::task::spawn_blocking(move || {
        let rx = spawn_step(&st, &run, step, &command, &env)?;
        if ctl.stopping() {
            let _ = st.ptys.kill(&pty_id(&run.id, step));
        }
        Ok(rx)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
}

/// Whether something accepts connections on `port`, on IPv4 or IPv6 loopback.
async fn answers(port: u16) -> bool {
    let probe = |addr: std::net::IpAddr| async move {
        matches!(tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect((addr, port))).await, Ok(Ok(_)))
    };
    let (v4, v6) =
        tokio::join!(probe(std::net::Ipv4Addr::LOCALHOST.into()), probe(std::net::Ipv6Addr::LOCALHOST.into()));
    v4 || v6
}

type Outcome = (RunState, Option<i32>, Option<String>);

/// Resolves with the command's exit code once it has ended (`None`: unknown, for a detached command).
type ExitFuture = Pin<Box<dyn Future<Output = Option<i32>> + Send>>;

async fn drive(st: &AppState, ctl: &Arc<RunControl>, run: &Run, def: &RunDef) -> Outcome {
    let _ = st.store.set_run_process(&run.id, def.detach, None).await;
    let v = vars(st, run);
    let pairs = v.pairs();
    let mut env = pdt_core::session::terminal_env();
    if let Some(path) = &st.platform.login_path {
        env.push(("PATH".into(), path.clone()));
    }
    env.extend(run_env(def, &v));
    if let Some(dir) = &run.data_dir
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return (RunState::Failed, None, Some(format!("could not create the data folder {dir}: {e}")));
    }
    if let Some(setup) = &def.setup {
        let _ = st.store.set_run_state(&run.id, RunState::SettingUp, now_ms()).await;
        let rx = match start_step(st, ctl, run, RunStep::Setup, expand(setup, &pairs), &env).await {
            Ok(rx) => rx,
            Err(e) => return (RunState::Failed, None, Some(format!("setup could not start: {e}"))),
        };
        let code = rx.await.unwrap_or(-1);
        if ctl.stopping() {
            return (RunState::Stopped, None, stop_reason(ctl));
        }
        if code != 0 {
            return (RunState::Failed, Some(code), Some(format!("setup exited with code {code}")));
        }
    }
    if ctl.stopping() {
        return (RunState::Stopped, None, stop_reason(ctl));
    }
    let first = if run.kind == RunKind::Test { RunState::Running } else { RunState::Starting };
    let _ = st.store.set_run_state(&run.id, first, now_ms()).await;
    let command = expand(&def.command, &pairs);
    let exit: ExitFuture = if def.detach && run.kind == RunKind::Run {
        let (st2, ctl2, run2) = (st.clone(), ctl.clone(), run.clone());
        let started = tokio::task::spawn_blocking(move || spawn_detached(&st2, &ctl2, &run2, &command, &env))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let d = match started {
            Ok(d) => d,
            Err(e) => return (RunState::Failed, None, Some(format!("the command could not start: {e}"))),
        };
        let _ = st.store.set_run_process(&run.id, true, Some(&d.process)).await;
        let rx = detached::watch(d.process, Some(d.main));
        Box::pin(async move { rx.await.ok().flatten() })
    } else {
        match start_step(st, ctl, run, RunStep::Command, command, &env).await {
            Ok(rx) => Box::pin(async move { Some(rx.await.unwrap_or(-1)) }),
            Err(e) => return (RunState::Failed, None, Some(format!("the command could not start: {e}"))),
        }
    };
    if let Some(p) = ctl.web_port {
        // Only while remote access is active; otherwise this runs no tailscale command (ruling F1).
        crate::remote::map_run_port(st, p).await;
    }
    let mut ready = run.kind == RunKind::Test;
    if run.kind == RunKind::Run && !def.web {
        let _ = st.store.set_run_state(&run.id, RunState::Ready, now_ms()).await;
        ready = true;
    }
    watch_command(st, ctl, run, def.detach, ready, exit).await
}

/// Follows a started command until it ends: readiness for a web run, then the outcome.
async fn watch_command(
    st: &AppState,
    ctl: &Arc<RunControl>,
    run: &Run,
    detach: bool,
    mut ready: bool,
    mut exit: ExitFuture,
) -> Outcome {
    let code = loop {
        tokio::select! {
            c = &mut exit => break c,
            _ = tokio::time::sleep(PROBE_EVERY), if !ready => {
                if let Some(p) = run.port
                    && answers(p).await
                {
                    ready = true;
                    let _ = st.store.set_run_state(&run.id, RunState::Ready, now_ms()).await;
                }
            }
        }
    };
    // During shutdown the mapping stays: the row stays active and the next pdtd takes the run over.
    if let Some(p) = ctl.web_port
        && !st.shutting_down.load(Ordering::SeqCst)
    {
        crate::remote::unmap_run_port(st, p).await;
    }
    if detach && !st.shutting_down.load(Ordering::SeqCst) {
        st.ptys.end_log(&pty_id(&run.id, RunStep::Command), code);
    }
    if ctl.stopping() {
        return (RunState::Stopped, code, stop_reason(ctl));
    }
    let shown = code.map_or("unknown".to_string(), |c| c.to_string());
    match (run.kind, code) {
        (RunKind::Test, Some(0)) => (RunState::Passed, code, None),
        (RunKind::Test, _) => (RunState::Failed, code, Some(format!("tests failed (exit code {shown})"))),
        (RunKind::Run, Some(c)) => (RunState::Failed, code, Some(format!("the command exited with code {c}"))),
        (RunKind::Run, None) => {
            (RunState::Failed, None, Some("the command and every process it started have exited".into()))
        }
    }
}

fn stop_reason(ctl: &RunControl) -> Option<String> {
    ctl.reason.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Starts driving `run` in the background. The row must already be stored.
pub(crate) fn start(st: &AppState, run: Run, def: RunDef) {
    let ctl = Arc::new(RunControl {
        web_port: run.port.filter(|_| def.web),
        uses_host: run.kind == RunKind::Run && uses_run_host(&def),
        ..Default::default()
    });
    let (st2, ctl2, run2) = (st.clone(), ctl.clone(), run.clone());
    supervise(st, run, ctl, async move { drive(&st2, &ctl2, &run2, &def).await });
}

/// Watches a detached run a previous pdtd started, whose processes are still alive: its log in the Output
/// terminal, readiness, the port mapping and the outcome, as if this pdtd had started it. `def` (when the
/// configuration still has the run) says whether it depends on the run's host.
fn reattach(st: &AppState, run: Run, process: String, def: Option<RunDef>) {
    let ctl = Arc::new(RunControl {
        web_port: run.port,
        uses_host: def.as_ref().is_some_and(uses_run_host),
        process: std::sync::Mutex::new(Some(process.clone())),
        ..Default::default()
    });
    let id = pty_id(&run.id, RunStep::Command);
    st.ptys.remove(&id);
    if let Err(e) = st.ptys.follow_log(&id, log_path(st, &run.id)) {
        tracing::warn!(run = %run.id, error = %e, "could not follow a detached run's log");
    }
    let (st2, ctl2, run2) = (st.clone(), ctl.clone(), run.clone());
    supervise(st, run, ctl, async move {
        let rx = detached::watch(process, None);
        if let Some(p) = ctl2.web_port {
            crate::remote::map_run_port(&st2, p).await;
        }
        let ready = run2.state == RunState::Ready || run2.port.is_none();
        watch_command(&st2, &ctl2, &run2, true, ready, Box::pin(async move { rx.await.ok().flatten() })).await
    });
}

/// Registers `ctl` as the run's driver and runs `driver` in the background, recording its outcome. The driver runs
/// in a task of its own: one that panics fails the run (logged), so its row and control never stay behind.
fn supervise(st: &AppState, run: Run, ctl: Arc<RunControl>, driver: impl Future<Output = Outcome> + Send + 'static) {
    st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).insert(run.id.clone(), ctl.clone());
    let st = st.clone();
    tokio::spawn(async move {
        let (state, code, error) = match tokio::spawn(driver).await {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::error!(run = %run.id, error = %e, "the run's driver failed; the run is marked failed");
                (RunState::Failed, None, Some("PDT's driver of this run failed (see pdtd's log)".to_string()))
            }
        };
        // During shutdown the row stays active, so the next pdtd resumes it.
        if !st.shutting_down.load(Ordering::SeqCst)
            && let Err(e) = st.store.finish_run(&run.id, state, code, error.as_deref(), now_ms()).await
        {
            tracing::warn!(run = %run.id, error = %e, "could not record the run's outcome");
        }
        ctl.finished.store(true, Ordering::SeqCst);
        ctl.done.notify_waiters();
        let mut controls = st.run_controls.lock().unwrap_or_else(|e| e.into_inner());
        if controls.get(&run.id).is_some_and(|c| Arc::ptr_eq(c, &ctl)) {
            controls.remove(&run.id);
        }
    });
}

/// Asks a run to stop and waits (up to `st.run_stop_wait`) for its driver to finish. A run with no driver (left
/// active by a previous daemon and not resumed yet) is marked stopped directly. False when the driver did not finish
/// in time: the run may still be running, so nothing must be started in its place.
pub(crate) async fn stop(st: &AppState, id: &str, reason: Option<String>) -> bool {
    stop_inner(st, id, reason, true).await
}

/// [`stop`]; `counted` is false only for a restart's own stop, which must not cancel itself.
async fn stop_inner(st: &AppState, id: &str, reason: Option<String>, counted: bool) -> bool {
    let ctl = {
        let mut stops = st.run_stops.lock().unwrap_or_else(|e| e.into_inner());
        if counted {
            *stops.entry(id.to_string()).or_default() += 1;
        }
        let ctl = st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
        if let Some(ctl) = &ctl {
            *ctl.reason.lock().unwrap_or_else(|e| e.into_inner()) = reason.clone();
            ctl.stop.store(true, Ordering::SeqCst);
        }
        ctl
    };
    match ctl {
        Some(ctl) => {
            for step in [RunStep::Setup, RunStep::Command] {
                let _ = st.kill_pty(&pty_id(id, step)).await;
            }
            let process = ctl.process.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(p) = process {
                let _ = tokio::task::spawn_blocking(move || detached::terminate(&p)).await;
            }
            let wait = async {
                while !ctl.finished.load(Ordering::SeqCst) {
                    // Registered before the check, so a notification in between is not missed.
                    let notified = ctl.done.notified();
                    if ctl.finished.load(Ordering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
            };
            let done = tokio::time::timeout(st.run_stop_wait, wait).await.is_ok();
            if !done {
                tracing::warn!(run = %id, "the run did not stop within {:?}", st.run_stop_wait);
            }
            done
        }
        None => {
            if let Ok(Some(r)) = st.store.get_run(id).await
                && r.state.is_active()
            {
                // A detached command a previous pdtd started, not reattached yet.
                if let Some(p) = r.process {
                    let _ = tokio::task::spawn_blocking(move || detached::terminate(&p)).await;
                }
                let _ = st.store.finish_run(id, RunState::Stopped, None, reason.as_deref(), now_ms()).await;
            }
            true
        }
    }
}

/// Restarts the runs a previous pdtd left active (spec §4.2 "Restarts"). Returns how many started (or, for a
/// detached run still running, were reattached). pdtd calls it after applying remote access (ruling F4), so links
/// use the tailnet name; each run maps its port as it starts. A run that moves to another port leaves the old port's
/// mapping alone: this process never made it.
///
/// A detached run whose processes are still alive is reattached, never started again; one whose processes all
/// exited while no pdtd was running is marked stopped (its exit code is unknown), and its log stays readable.
pub async fn resume_runs(st: &AppState) -> usize {
    let _start = st.run_start_lock.lock().await;
    let Ok(active) = st.store.list_active_runs().await else { return 0 };
    let mut started = 0;
    for mut run in active {
        if st.shutting_down.load(Ordering::SeqCst) {
            break;
        }
        if let Some(process) = run.process.clone() {
            let p = process.clone();
            let alive = tokio::task::spawn_blocking(move || detached::alive(&p)).await.unwrap_or(false);
            if alive {
                if let Some(p) = run.port {
                    let url = run_url(st, p);
                    let _ = st.store.set_run_port(&run.id, Some(p), Some(&url)).await;
                    run.url = Some(url);
                }
                let def = crate::runs::definition_for(st, &run).await.ok();
                tracing::info!(run = %run.id, name = %run.name, "reattached a detached run that kept running");
                reattach(st, run, process, def);
                started += 1;
            } else {
                let msg = "the command exited while PDT was not running";
                let _ = st.store.finish_run(&run.id, RunState::Stopped, None, Some(msg), now_ms()).await;
                let id = pty_id(&run.id, RunStep::Command);
                if st.ptys.follow_log(&id, log_path(st, &run.id)).is_ok() {
                    st.ptys.end_log(&id, None);
                }
            }
            continue;
        }
        let def = match crate::runs::definition_for(st, &run).await {
            Ok(d) => d,
            Err(msg) => {
                let _ = st.store.finish_run(&run.id, RunState::Failed, None, Some(&msg), now_ms()).await;
                continue;
            }
        };
        if def.web {
            match allocate_port(st, run.port, Some(&run.id)).await {
                None => {
                    let msg = "no free port in the host's run port range";
                    let _ = st.store.finish_run(&run.id, RunState::Failed, None, Some(msg), now_ms()).await;
                    continue;
                }
                Some(p) => {
                    let url = run_url(st, p);
                    let _ = st.store.set_run_port(&run.id, Some(p), Some(&url)).await;
                    run.port = Some(p);
                    run.url = Some(url);
                }
            }
        }
        start(st, run, def);
        started += 1;
    }
    started
}

/// Whether a run's checkout has gone: its folder is missing, or (outside the project's main checkout) its branch
/// no longer exists. Decided from the run itself, since its worktree record may have been deleted.
fn checkout_gone(repo: &Path, cwd: &Path, branch: &str) -> bool {
    !cwd.is_dir() || (cwd != repo && !pdt_git::branch_exists(repo, branch))
}

/// Stops active runs whose worktree folder or branch has gone (spec §4.2: merged or deleted branches). A run whose
/// project cannot be read is left alone (and logged): a failed lookup is not a gone branch.
pub async fn sweep_gone_runs(st: &AppState) {
    let active = match st.store.list_active_runs().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "the run sweep could not list active runs");
            return;
        }
    };
    for run in active {
        let project = match st.store.get_project(&run.project_id).await {
            Ok(Some(p)) => p,
            // Deleting a project deletes its runs; a row without one is not this sweep's business.
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(run = %run.id, error = %e, "the run sweep could not read the project; skipping");
                continue;
            }
        };
        let (repo, cwd, branch) = (PathBuf::from(project.repo_path), PathBuf::from(&run.cwd), run.branch.clone());
        let gone = match tokio::task::spawn_blocking(move || checkout_gone(&repo, &cwd, &branch)).await {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(run = %run.id, error = %e, "the run sweep check failed; skipping");
                continue;
            }
        };
        if gone {
            tracing::info!(run = %run.id, branch = %run.branch, "stopping a run whose branch or worktree is gone");
            stop(st, &run.id, Some("its branch or worktree is gone".into())).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restart waits for its port while the stopped process still holds it, and goes on once it is let go.
    #[tokio::test]
    async fn a_restart_waits_for_its_port_to_be_let_go() {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            drop(held);
        });
        let started = std::time::Instant::now();
        await_port_release(Some(port)).await;
        assert!(started.elapsed() >= Duration::from_millis(400), "returned while the port was held");
        assert!(port_is_free(port));
        release.await.unwrap();
        await_port_release(None).await;
    }

    /// A port something listens on only on `::1` is not free (skipped where the machine has no IPv6 loopback).
    #[test]
    fn a_port_held_only_on_ipv6_loopback_is_not_free() {
        let Ok(held) = std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)) else { return };
        let port = held.local_addr().unwrap().port();
        assert!(!port_is_free(port));
        drop(held);
    }
}
