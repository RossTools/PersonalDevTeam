//! Drives run instances (spec §4.2): setup, command, readiness, Tailscale mapping, stop, resume after a restart.
//!
//! Tailscale (controller ruling on Task 7): a pdtd maps a run's port only once it has (re)started that run on that
//! port itself, and it never unmaps a port it did not map. The machine's other instance (stable or dev) may own any
//! port a stored row mentions.

use crate::routes::{AppState, now_ms};
use pdt_agents::SpawnSpec;
use pdt_core::host::CredentialPolicy;
use pdt_core::run::{Run, RunKind, RunState, RunStep, pick_port, pty_id, step_launch};
use pdt_core::runconfig::{RunDef, RunVars, expand, run_env};
use pdt_core::session::{SessionKind, is_scrubbed};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

const PROBE_EVERY: Duration = Duration::from_millis(300);
/// The longest one readiness probe may take (a refused connection on Windows loopback can take seconds).
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct RunControl {
    stop: AtomicBool,
    /// Why the run was stopped by PDT rather than the user (e.g. its branch is gone).
    reason: std::sync::Mutex<Option<String>>,
    finished: AtomicBool,
    done: tokio::sync::Notify,
    /// The port of a web run this process is driving: the only run ports it maps.
    web_port: Option<u16>,
}

impl RunControl {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

pub(crate) fn port_is_free(p: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()
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
    let script = scripts_dir(st, &run.id).join(format!("{}.cmd", step.as_str()));
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
    let l = step_launch(st.platform.windows, &comspec, command, &script);
    if let Some(text) = &l.script {
        std::fs::create_dir_all(script.parent().expect("script dir")).map_err(|e| e.to_string())?;
        std::fs::write(&script, text).map_err(|e| format!("writing {}: {e}", script.display()))?;
    }
    let spec = SpawnSpec {
        id,
        program: PathBuf::from(l.program),
        args: l.args,
        cwd: PathBuf::from(&run.cwd),
        env_remove: std::env::vars_os()
            .filter_map(|(k, _)| k.into_string().ok())
            .filter(|k| is_scrubbed(SessionKind::Shell, k, CredentialPolicy::PassThrough))
            .collect(),
        env: env.to_vec(),
        cols: 120,
        rows: 30,
    };
    st.ptys.spawn(spec).map_err(|e| e.to_string())
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

async fn drive(st: &AppState, ctl: &Arc<RunControl>, run: &Run, def: &RunDef) -> Outcome {
    let v = vars(st, run);
    let pairs = v.pairs();
    let mut env = vec![("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".into(), "truecolor".into())];
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
    let mut exit = match start_step(st, ctl, run, RunStep::Command, expand(&def.command, &pairs), &env).await {
        Ok(rx) => rx,
        Err(e) => return (RunState::Failed, None, Some(format!("the command could not start: {e}"))),
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
    let code = loop {
        tokio::select! {
            c = &mut exit => break c.unwrap_or(-1),
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
    if ctl.stopping() {
        return (RunState::Stopped, Some(code), stop_reason(ctl));
    }
    match run.kind {
        RunKind::Test if code == 0 => (RunState::Passed, Some(0), None),
        RunKind::Test => (RunState::Failed, Some(code), Some(format!("tests failed (exit code {code})"))),
        RunKind::Run => (RunState::Failed, Some(code), Some(format!("the command exited with code {code}"))),
    }
}

fn stop_reason(ctl: &RunControl) -> Option<String> {
    ctl.reason.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Starts driving `run` in the background. The row must already be stored.
pub(crate) fn start(st: &AppState, run: Run, def: RunDef) {
    let ctl = Arc::new(RunControl { web_port: run.port.filter(|_| def.web), ..Default::default() });
    st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).insert(run.id.clone(), ctl.clone());
    let st = st.clone();
    tokio::spawn(async move {
        let (state, code, error) = drive(&st, &ctl, &run, &def).await;
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
    let ctl = st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
    match ctl {
        Some(ctl) => {
            *ctl.reason.lock().unwrap_or_else(|e| e.into_inner()) = reason;
            ctl.stop.store(true, Ordering::SeqCst);
            for step in [RunStep::Setup, RunStep::Command] {
                let _ = st.kill_pty(&pty_id(id, step)).await;
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
                let _ = st.store.finish_run(id, RunState::Stopped, None, reason.as_deref(), now_ms()).await;
            }
            true
        }
    }
}

/// Restarts the runs a previous pdtd left active (spec §4.2 "Restarts"). Returns how many started. pdtd calls it
/// after applying remote access (ruling F4), so links use the tailnet name; each run maps its port as it starts. A
/// run that moves to another port leaves the old port's mapping alone: this process never made it.
pub async fn resume_runs(st: &AppState) -> usize {
    let _start = st.run_start_lock.lock().await;
    let Ok(active) = st.store.list_active_runs().await else { return 0 };
    let mut started = 0;
    for mut run in active {
        if st.shutting_down.load(Ordering::SeqCst) {
            break;
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
