//! Drives run instances (spec §4.2): setup, command, readiness, Tailscale mapping, stop, resume after a restart.

use crate::routes::{AppState, now_ms};
use pdt_agents::SpawnSpec;
use pdt_core::host::CredentialPolicy;
use pdt_core::run::{Run, RunKind, RunState, RunStep, pick_port, pty_id, step_launch};
use pdt_core::runconfig::{RunDef, RunVars, expand, run_env};
use pdt_core::session::{SessionKind, is_scrubbed};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

const PROBE_EVERY: Duration = Duration::from_millis(300);
/// How long a stop waits for the run's driver to record the outcome.
const STOP_WAIT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub(crate) struct RunControl {
    stop: AtomicBool,
    /// Why the run was stopped by PDT rather than the user (e.g. its branch is gone).
    reason: std::sync::Mutex<Option<String>>,
    finished: AtomicBool,
    done: tokio::sync::Notify,
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

/// The link for a web run on `port`: the tailnet when remote access is active, else loopback.
pub(crate) fn run_url(st: &AppState, port: u16) -> String {
    match remote_host(st) {
        Some(h) => format!("https://{h}:{port}/"),
        None => format!("http://127.0.0.1:{port}/"),
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

/// A free port for a new or resumed web run, avoiding every other active run's port.
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
fn start_step(
    st: &AppState,
    ctl: &RunControl,
    run: &Run,
    step: RunStep,
    command: &str,
    env: &[(String, String)],
) -> Result<oneshot::Receiver<i32>, String> {
    let rx = spawn_step(st, run, step, command, env)?;
    if ctl.stopping() {
        let _ = st.ptys.kill(&pty_id(&run.id, step));
    }
    Ok(rx)
}

type Outcome = (RunState, Option<i32>, Option<String>);

async fn drive(st: &AppState, ctl: &RunControl, run: &Run, def: &RunDef) -> Outcome {
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
        let rx = match start_step(st, ctl, run, RunStep::Setup, &expand(setup, &pairs), &env) {
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
    let mut exit = match start_step(st, ctl, run, RunStep::Command, &expand(&def.command, &pairs), &env) {
        Ok(rx) => rx,
        Err(e) => return (RunState::Failed, None, Some(format!("the command could not start: {e}"))),
    };
    let web_port = run.port.filter(|_| def.web);
    if let Some(p) = web_port {
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
                    && tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok()
                {
                    ready = true;
                    let _ = st.store.set_run_state(&run.id, RunState::Ready, now_ms()).await;
                }
            }
        }
    };
    // During shutdown the mapping stays: the row stays active and the next pdtd takes the run over.
    if let Some(p) = web_port
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
    let ctl = Arc::new(RunControl::default());
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

/// Asks a run to stop and waits (up to 20 s) for its driver to finish. A run with no driver (left active by a
/// previous daemon and not resumed yet) is marked stopped directly.
pub(crate) async fn stop(st: &AppState, id: &str, reason: Option<String>) {
    let ctl = st.run_controls.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned();
    match ctl {
        Some(ctl) => {
            *ctl.reason.lock().unwrap_or_else(|e| e.into_inner()) = reason;
            ctl.stop.store(true, Ordering::SeqCst);
            // The tree kill waits for taskkill on Windows.
            let (ptys, run_id) = (st.ptys.clone(), id.to_string());
            let _ = tokio::task::spawn_blocking(move || {
                for step in [RunStep::Setup, RunStep::Command] {
                    let _ = ptys.kill(&pty_id(&run_id, step));
                }
            })
            .await;
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
            if tokio::time::timeout(STOP_WAIT, wait).await.is_err() {
                tracing::warn!(run = %id, "the run did not stop within {STOP_WAIT:?}");
            }
        }
        None => {
            if let Ok(Some(r)) = st.store.get_run(id).await
                && r.state.is_active()
            {
                let _ = st.store.finish_run(id, RunState::Stopped, None, reason.as_deref(), now_ms()).await;
            }
        }
    }
}

/// Restarts the runs a previous pdtd left active (spec §4.2 "Restarts"). Returns how many started. pdtd calls it
/// after applying remote access (ruling F4), so links use the tailnet name and an old port's mapping (made by that
/// apply) can be removed when the run moves.
pub async fn resume_runs(st: &AppState) -> usize {
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
                    if let Some(old) = run.port.filter(|o| *o != p) {
                        crate::remote::unmap_run_port(st, old).await;
                    }
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

/// Stops active runs whose worktree folder or branch has gone (spec §4.2: merged or deleted branches).
pub async fn sweep_gone_runs(st: &AppState) {
    let Ok(active) = st.store.list_active_runs().await else { return };
    for run in active {
        let Some(wid) = &run.worktree_id else { continue };
        let wt = st.store.get_worktree(wid).await.ok().flatten();
        let project = st.store.get_project(&run.project_id).await.ok().flatten();
        let gone = match (wt, project) {
            (Some(wt), Some(p)) => {
                let (repo, branch, path) = (PathBuf::from(p.repo_path), run.branch.clone(), PathBuf::from(wt.path));
                tokio::task::spawn_blocking(move || !path.is_dir() || !pdt_git::branch_exists(&repo, &branch))
                    .await
                    .unwrap_or(false)
            }
            _ => true,
        };
        if gone {
            tracing::info!(run = %run.id, branch = %run.branch, "stopping a run whose branch or worktree is gone");
            stop(st, &run.id, Some("its branch or worktree is gone".into())).await;
        }
    }
}
