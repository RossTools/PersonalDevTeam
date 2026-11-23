//! Runs and tests on a remote instance (PDT-634).
//!
//! - **The owner** keeps each run's row (with the instance it runs on) and every decision: the configuration (read
//!   from the remote worktree's file, `/link/exec/run/config`, with the owner's overrides), the user's approval of
//!   its commands, and the systems it runs on (`os`). It starts and stops it with signed calls, and its poller reads the
//!   run's state and output with the sessions' events. Each start has a `proc` id, kept in the row's `process`: reports
//!   from an earlier start are ignored. The output goes to a log per step that the run's terminals follow, read-only.
//! - **The remote instance** drives the process with `run_manager` (no row of its own: its progress goes to the owner,
//!   [`ToOwner`]), in the remote project's checkout or worktree. A web run gets a port here, and its link is this
//!   instance's: its `tailscale serve` maps the port through the owner-checked proxy, as for its own runs. A detached
//!   run runs attached: the runs it drives end when it restarts, and the owner then marks them stopped.

use super::owner::{call, ensure_poller, instance};
use super::remote::checked;
use crate::error::{ApiError, codes};
use crate::link::PeerError;
use crate::log_err::LogErr;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Method};
use base64::Engine;
use pdt_agents::PtyEvent;
use pdt_core::link::{OwnerLink, RemoteInstance};
use pdt_core::run::{Run, RunKind, RunState, RunStep, pty_id, run_data_dir};
use pdt_core::runconfig::RunDef;
use pdt_core::worktree::worktree_path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const START_PATH: &str = "/api/link/exec/run/start";
const STOP_PATH: &str = "/api/link/exec/run/stop";
const CONFIG_PATH: &str = "/api/link/exec/run/config";

/// How long a start or a stop there may take (a stop waits for the run's processes to end).
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the owner waits, after the remote instance stopped a run, for its last output and end to come in.
const END_WAIT: Duration = Duration::from_secs(5);
/// The most output one report carries.
const CHUNK: usize = 64 * 1024;
/// How long a run's end waits for its output to be reported first.
const OUTPUT_WAIT: Duration = Duration::from_secs(2);
const STEPS: [RunStep; 2] = [RunStep::Setup, RunStep::Command];

/// `/link/exec/run/start`: drive `def` as the owner's run `run_id`, in the worktree of `branch` (None: the checkout).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartReq {
    pub run_id: String,
    /// This start: its reports carry it.
    pub proc: String,
    pub kind: RunKind,
    pub branch: Option<String>,
    /// The branch the run runs (`$PDT_RUN_BRANCH`, its data folder).
    pub run_branch: String,
    pub def: RunDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Started {
    pub cwd: String,
    /// A web run's link there.
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunReq {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stopped {
    /// False when its processes didn't end in time.
    pub stopped: bool,
}

/// `/link/exec/run/config`: the run configuration file of `branch`'s worktree (None: the checkout).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigReq {
    pub branch: Option<String>,
}

/// A remote checkout's run configuration file, and the system it is on (`std::env::consts::OS`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteConfig {
    pub file: Option<String>,
    pub os: String,
}

/// A run's report, numbered with the sessions' events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireRunEvent {
    pub seq: i64,
    pub run_id: String,
    pub proc: String,
    pub out: RunOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum RunOut {
    State {
        state: RunState,
    },
    /// What a step's terminal printed (base64).
    Output {
        step: String,
        data: String,
    },
    Finished {
        state: RunState,
        code: Option<i32>,
        error: Option<String>,
    },
}

/// A run this instance drives for an owner.
struct Driven {
    link_id: String,
    proc: String,
    port: Option<u16>,
    /// The tasks reporting its terminals' output.
    forwarders: Vec<tokio::task::JoinHandle<()>>,
}

/// What the executor keeps about runs, on both ends.
#[derive(Default)]
pub(super) struct RunsSide {
    /// Owner: when each remote run's start came back (ms; `i64::MAX` while it is being started), so a poll sent before
    /// then doesn't find it gone.
    started: Mutex<HashMap<String, i64>>,
    /// Remote: the runs driven here for owners, by run id.
    driven: Mutex<HashMap<String, Driven>>,
}

impl RunsSide {
    fn started(&self) -> std::sync::MutexGuard<'_, HashMap<String, i64>> {
        self.started.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn driven(&self) -> std::sync::MutexGuard<'_, HashMap<String, Driven>> {
        self.driven.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---- The owner's end ----

/// Where the owner keeps what a remote run's step printed (its terminal follows it).
fn mirror_log(st: &AppState, run_id: &str, step: RunStep) -> PathBuf {
    st.data_dir.join("runs").join("logs").join(format!("{run_id}.{}.log", step.as_str()))
}

/// A remote run's output logs on the owner (to delete with the run).
pub(crate) fn mirror_logs(st: &AppState, run_id: &str) -> Vec<PathBuf> {
    STEPS.iter().map(|s| mirror_log(st, run_id, *s)).collect()
}

/// The joined instance a run is on.
async fn instance_of(st: &AppState, run: &Run) -> Result<RemoteInstance, ApiError> {
    let on = run.instance.as_ref().ok_or_else(|| ApiError::internal("not a remote run"))?;
    instance(st, &run.project_id, &on.id).await
}

/// The run configuration file of `branch`'s worktree on the instance `i` (None: its checkout), and its system.
pub(crate) async fn config_file(
    st: &AppState,
    i: &RemoteInstance,
    branch: Option<&str>,
) -> Result<RemoteConfig, ApiError> {
    let req = ConfigReq { branch: branch.map(String::from) };
    call(st, i, CONFIG_PATH, &req, CALL_TIMEOUT).await.map_err(|e| e.into_api(&i.name))
}

/// A remote run's configuration file, where it runs, and that instance's system.
pub(crate) async fn config_of(st: &AppState, run: &Run) -> Result<RemoteConfig, ApiError> {
    let i = instance_of(st, run).await?;
    let branch = worktree_branch(st, run).await?;
    config_file(st, &i, branch.as_deref()).await
}

/// The worktree branch a remote run runs in (None: the checkout).
async fn worktree_branch(st: &AppState, run: &Run) -> Result<Option<String>, ApiError> {
    match &run.worktree_id {
        Some(w) => Ok(Some(st.store.get_worktree(w).await?.map(|w| w.branch).unwrap_or_else(|| run.branch.clone()))),
        None => Ok(None),
    }
}

/// Starts a remote run (new, or stopped) on its instance with `def`: a new `proc`, its first state, its terminals
/// following fresh logs, then the start there. The caller has checked it isn't active. A start the instance refuses
/// fails the run, with why.
pub(crate) async fn relaunch(st: &AppState, mut run: Run, def: RunDef) -> Result<Run, ApiError> {
    let i = instance_of(st, &run).await?;
    let branch = worktree_branch(st, &run).await?;
    let proc = uuid::Uuid::now_v7().to_string();
    st.store.set_run_process(&run.id, false, Some(&proc)).await?;
    let first = if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting };
    st.store.set_run_state(&run.id, first, now_ms()).await?;
    st.store.set_run_port(&run.id, None, None).await?;
    for step in STEPS {
        st.remove_pty(&pty_id(&run.id, step)).await;
        let _ = tokio::fs::remove_file(mirror_log(st, &run.id, step)).await;
    }
    follow(st, &run.id, def.setup.is_some());
    st.exec.runs.started().insert(run.id.clone(), i64::MAX);
    ensure_poller(st, &i.link_id);
    let req = StartReq {
        run_id: run.id.clone(),
        proc: proc.clone(),
        kind: run.kind,
        branch,
        run_branch: run.branch.clone(),
        def,
    };
    match call::<Started>(st, &i, START_PATH, &req, CALL_TIMEOUT).await {
        Ok(s) => {
            st.exec.runs.started().insert(run.id.clone(), now_ms());
            if s.url.is_some() {
                st.store.set_run_port(&run.id, None, s.url.as_deref()).await?;
            }
            run.state = first;
            run.exit_code = None;
            run.error = None;
            run.detached = false;
            run.process = Some(proc);
            run.port = None;
            run.url = s.url;
            Ok(run)
        }
        Err(e) => {
            st.exec.runs.started().remove(&run.id);
            let err = e.into_api(&i.name);
            st.store
                .finish_run(&run.id, RunState::Failed, None, Some(err.message()), now_ms())
                .await
                .log_warn("record the run ending");
            end_logs(st, &run.id, None);
            Err(err)
        }
    }
}

/// The run's terminals follow its output logs (the setup's only when it has one).
fn follow(st: &AppState, run_id: &str, setup: bool) {
    for step in STEPS {
        if step == RunStep::Setup && !setup {
            continue;
        }
        if let Err(e) = st.ptys.follow_log(&pty_id(run_id, step), mirror_log(st, run_id, step)) {
            tracing::debug!(run = run_id, error = %e, "could not follow a remote run's output");
        }
    }
}

fn end_logs(st: &AppState, run_id: &str, code: Option<i32>) {
    for step in STEPS {
        st.ptys.end_log(&pty_id(run_id, step), code);
    }
}

/// After the owner restarted: an active remote run's terminals follow its logs again (its poller resumes).
pub(crate) fn resume(st: &AppState, run: &Run) {
    let setup = mirror_log(st, &run.id, RunStep::Setup).exists();
    follow(st, &run.id, setup);
}

/// Stops a remote run: there, then (once its end came in, or after a moment) here. False when its processes didn't
/// end there in time. An instance that can't be reached, or has left, can't run it: it is stopped here.
pub(crate) async fn stop(st: &AppState, run: &Run, reason: Option<String>) -> bool {
    let id = run.id.as_str();
    let i = match instance_of(st, run).await {
        Ok(i) => i,
        Err(e) => return stopped_here(st, id, Some(reason.unwrap_or_else(|| e.message().to_string()))).await,
    };
    match call::<Stopped>(st, &i, STOP_PATH, &RunReq { run_id: id.into() }, CALL_TIMEOUT).await {
        Ok(Stopped { stopped: false }) => false,
        Ok(_) => {
            let deadline = tokio::time::Instant::now() + END_WAIT;
            while tokio::time::Instant::now() < deadline {
                match st.store.get_run(id).await {
                    Ok(Some(r)) if r.state.is_active() => tokio::time::sleep(Duration::from_millis(50)).await,
                    _ => return true,
                }
            }
            stopped_here(st, id, reason).await
        }
        Err(e @ PeerError::Unreachable(_)) => {
            tracing::warn!(run = id, instance = %i.name, error = %e.text(), "could not reach a remote run's instance to stop it");
            stopped_here(st, id, Some(format!("couldn't reach {}: {}", i.name, e.text()))).await
        }
        Err(e) => {
            tracing::warn!(run = id, instance = %i.name, error = %e.text(), "a remote instance refused to stop a run");
            false
        }
    }
}

async fn stopped_here(st: &AppState, id: &str, reason: Option<String>) -> bool {
    st.exec.runs.started().remove(id);
    st.store
        .finish_run(id, RunState::Stopped, None, reason.as_deref(), now_ms())
        .await
        .log_warn("record the run ending");
    end_logs(st, id, None);
    true
}

/// The active runs on the instance, as the owner's store says.
pub(super) async fn active_on(st: &AppState, i: &RemoteInstance) -> Vec<Run> {
    match st.store.list_active_runs().await {
        Ok(list) => list
            .into_iter()
            .filter(|r| r.project_id == i.project_id && r.instance.as_ref().is_some_and(|o| o.id == i.instance_id))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the active runs");
            vec![]
        }
    }
}

/// Applies the runs' reports after `after`: the highest number read.
pub(super) async fn apply(st: &AppState, i: &RemoteInstance, events: Vec<WireRunEvent>, after: i64) -> i64 {
    let mut last = after;
    for e in events {
        if e.seq <= after {
            continue;
        }
        last = last.max(e.seq);
        let run = match st.store.get_run(&e.run_id).await {
            Ok(Some(r)) => r,
            _ => continue,
        };
        // Another start's, or another instance's.
        if run.process.as_deref() != Some(e.proc.as_str())
            || run.instance.as_ref().is_none_or(|o| o.id != i.instance_id)
        {
            continue;
        }
        match e.out {
            RunOut::State { state } => {
                st.store.set_run_state(&run.id, state, now_ms()).await.log_warn("record the run state");
            }
            RunOut::Output { step, data } => {
                let Some(step) = RunStep::parse(&step) else { continue };
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) else { continue };
                if let Err(e) = append(&mirror_log(st, &run.id, step), &bytes).await {
                    tracing::warn!(run = %run.id, error = %e, "could not keep a remote run's output");
                }
            }
            RunOut::Finished { state, code, error } => {
                st.exec.runs.started().remove(&run.id);
                st.store
                    .finish_run(&run.id, state, code, error.as_deref(), now_ms())
                    .await
                    .log_warn("record the run ending");
                end_logs(st, &run.id, code);
            }
        }
    }
    last
}

async fn append(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
    f.write_all(bytes).await
}

/// The instance's runs it no longer drives, though no end came for them (it restarted): stopped. Those started
/// after the poll was sent (`sent_at`) may not have reached it yet.
pub(super) async fn reconcile(st: &AppState, i: &RemoteInstance, alive: &[String], sent_at: i64) {
    for run in active_on(st, i).await {
        let started = st.exec.runs.started().get(&run.id).copied().unwrap_or(0);
        if alive.contains(&run.id) || started >= sent_at {
            continue;
        }
        let why = format!("{} no longer runs it (it restarted?)", i.name);
        stopped_here(st, &run.id, Some(why)).await;
    }
}

// ---- The remote instance's end ----

/// The runs this instance drives for the owner on `link_id`.
pub(super) fn alive(st: &AppState, link_id: &str) -> Vec<String> {
    st.exec.runs.driven().iter().filter(|(_, d)| d.link_id == link_id).map(|(id, _)| id.clone()).collect()
}

/// The ports of the web runs this instance drives for owners (a run of its own can't take them).
pub(crate) fn run_ports(st: &AppState) -> Vec<u16> {
    st.exec.runs.driven().values().filter_map(|d| d.port).collect()
}

/// Stops every run this instance drives for the owner on `link_id`.
pub(super) async fn stop_link(st: &AppState, link_id: &str) {
    for id in alive(st, link_id) {
        crate::run_manager::stop(st, &id, None).await;
    }
}

/// A driven run's progress, reported to its owner.
struct ToOwner {
    st: AppState,
    link_id: String,
    run_id: String,
    proc: String,
}

impl ToOwner {
    fn push(&self, out: RunOut) {
        self.st.exec.remote.push_run(&self.link_id, &self.run_id, &self.proc, out);
    }

    /// Whether the run is still this start's.
    fn current(&self) -> bool {
        self.st.exec.runs.driven().get(&self.run_id).is_some_and(|d| d.proc == self.proc)
    }
}

#[async_trait::async_trait]
impl crate::run_manager::Progress for ToOwner {
    async fn state(&self, state: RunState) {
        self.push(RunOut::State { state });
    }

    async fn process(&self, _detached: bool, _process: Option<&str>) {}

    async fn finish(&self, state: RunState, code: Option<i32>, error: Option<&str>) {
        let forwarders = match self.st.exec.runs.driven().get_mut(&self.run_id) {
            Some(d) if d.proc == self.proc => std::mem::take(&mut d.forwarders),
            _ => vec![],
        };
        // Its terminals have exited: what they printed goes first.
        let _ = tokio::time::timeout(OUTPUT_WAIT, futures_util::future::join_all(forwarders)).await;
        self.push(RunOut::Finished { state, code, error: error.map(String::from) });
        let mut driven = self.st.exec.runs.driven();
        if driven.get(&self.run_id).is_some_and(|d| d.proc == self.proc) {
            driven.remove(&self.run_id);
        }
    }

    fn step_started(&self, step: RunStep) {
        if !self.current() {
            return;
        }
        let (st, link, run, proc) = (self.st.clone(), self.link_id.clone(), self.run_id.clone(), self.proc.clone());
        let task = tokio::spawn(forward(st, link, run, proc, step));
        if let Some(d) = self.st.exec.runs.driven().get_mut(&self.run_id) {
            d.forwarders.push(task);
        }
    }
}

/// Reports what a step's terminal prints, in chunks, until it exits.
async fn forward(st: AppState, link_id: String, run_id: String, proc: String, step: RunStep) {
    let Ok(a) = st.ptys.attach(&pty_id(&run_id, step)) else { return };
    let push = |bytes: &[u8]| {
        if !bytes.is_empty() {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            st.exec.remote.push_run(&link_id, &run_id, &proc, RunOut::Output { step: step.as_str().into(), data });
        }
    };
    push(&a.scrollback);
    if a.exited.is_some() {
        return;
    }
    let mut events = a.events;
    loop {
        let mut chunk = match events.recv().await {
            Ok(PtyEvent::Output(b)) => b.to_vec(),
            Ok(PtyEvent::Exit(_)) | Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(run = %run_id, skipped = n, "a remote run's output came faster than it was reported");
                continue;
            }
        };
        let mut exited = false;
        while chunk.len() < CHUNK {
            match events.try_recv() {
                Ok(PtyEvent::Output(b)) => chunk.extend_from_slice(&b),
                Ok(PtyEvent::Exit(_)) => {
                    exited = true;
                    break;
                }
                Err(_) => break,
            }
        }
        push(&chunk);
        if exited {
            return;
        }
    }
}

/// `/link/exec/run/start`: drives the owner's run here.
pub(crate) async fn start_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Started>, ApiError> {
    let (link, req): (OwnerLink, StartReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let me = crate::link::instance_name(&st).await?;
    if link.paused {
        return Err(ApiError::conflict(codes::REMOTE_PROJECT, format!("this project is paused on {me}")));
    }
    let os = std::env::consts::OS;
    if !req.def.runs_on(os) {
        return Err(ApiError::conflict(codes::INVALID_RUN_CONFIG, format!("\"{}\" doesn't run on {os}", req.def.name)));
    }
    let checkout = super::checkout_dir(&st, &link);
    let cwd = match &req.branch {
        Some(b) => worktree_path(&checkout, b),
        None => checkout,
    };
    if !cwd.is_dir() {
        return Err(ApiError::conflict(codes::REMOTE_PROJECT, format!("{me} has no worktree at {}", cwd.display())));
    }
    // A start of a run it still drives (the owner lost track of it) replaces that one.
    let earlier = st.exec.runs.driven().get(&req.run_id).map(|d| d.link_id == link.link_id);
    match earlier {
        Some(false) => return Err(ApiError::conflict(codes::RUN_ACTIVE, "another owner's run has that id")),
        Some(true) if !crate::run_manager::stop(&st, &req.run_id, None).await => {
            return Err(ApiError::conflict(codes::RUN_STOPPING, "the run has not stopped yet; try again in a moment"));
        }
        _ => {}
    }
    let mut def = req.def;
    def.detach = false;
    let _start = st.run_start_lock.lock().await;
    let port = if def.web {
        let p =
            crate::run_manager::allocate_port(&st, None, None).await.ok_or_else(crate::run_manager::no_free_port)?;
        Some(p)
    } else {
        None
    };
    let data_dir = def
        .data
        .then(|| run_data_dir(&st.data_dir, &link.link_id, &req.run_branch, &def.name).to_string_lossy().into_owned());
    let now = now_ms();
    let run = Run {
        id: req.run_id.clone(),
        project_id: link.link_id.clone(),
        worktree_id: None,
        branch: req.run_branch,
        name: def.name.clone(),
        kind: req.kind,
        state: if def.setup.is_some() { RunState::SettingUp } else { RunState::Starting },
        port,
        url: port.map(|p| crate::run_manager::run_url(&st, p)),
        data_dir,
        cwd: cwd.to_string_lossy().into_owned(),
        exit_code: None,
        error: None,
        detached: false,
        process: None,
        instance: None,
        created_at: now,
        updated_at: now,
    };
    st.exec.runs.driven().insert(
        run.id.clone(),
        Driven { link_id: link.link_id.clone(), proc: req.proc.clone(), port, forwarders: vec![] },
    );
    let p = Arc::new(ToOwner { st: st.clone(), link_id: link.link_id.clone(), run_id: run.id.clone(), proc: req.proc });
    let started = Started { cwd: run.cwd.clone(), url: run.url.clone() };
    tracing::info!(run = %run.id, name = %run.name, owner = %link.owner_name, "started a run for the owner");
    crate::run_manager::start_for_owner(&st, run, def, p);
    Ok(Json(started))
}

/// `/link/exec/run/stop`: stops a run driven here for the owner (one it doesn't drive has stopped).
pub(crate) async fn stop_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Stopped>, ApiError> {
    let (link, req): (OwnerLink, RunReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let ours = st.exec.runs.driven().get(&req.run_id).is_some_and(|d| d.link_id == link.link_id);
    let stopped = !ours || crate::run_manager::stop(&st, &req.run_id, None).await;
    Ok(Json(Stopped { stopped }))
}

/// `/link/exec/run/config`: a worktree's (or the checkout's) run configuration file, and this instance's system.
pub(crate) async fn config_route(
    State(st): State<AppState>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RemoteConfig>, ApiError> {
    let (link, req): (OwnerLink, ConfigReq) = checked(&st, &method, &uri, &headers, &body).await?;
    let checkout = super::checkout_dir(&st, &link);
    let dir = match &req.branch {
        Some(b) => worktree_path(&checkout, b),
        None => checkout,
    };
    let file = crate::runconfig::read_file(&dir).await?;
    Ok(Json(RemoteConfig { file, os: std::env::consts::OS.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_reports_cross_the_wire_unchanged() {
        let outs = [
            RunOut::State { state: RunState::Ready },
            RunOut::Output { step: "command".into(), data: "aGk=".into() },
            RunOut::Finished { state: RunState::Failed, code: Some(2), error: Some("tests failed".into()) },
        ];
        for out in outs {
            let e = WireRunEvent { seq: 3, run_id: "r".into(), proc: "p".into(), out: out.clone() };
            let back: WireRunEvent = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
            assert_eq!((back.seq, back.out), (3, out));
        }
    }
}
