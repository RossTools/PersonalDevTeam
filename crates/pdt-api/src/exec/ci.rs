//! The local forge's CI on a remote instance (PDT-730). A remote attempt's `[ci]` command runs in its worktree there,
//! driven as a test is ([`super::runs`]: `/link/exec/run/start` and `/run/stop`, its reports with the sessions'
//! events) under the run id `local-ci-<id>`. The owner keeps no run row for it: its output goes to the local CI run's
//! log and its end to the local forge, which records it ([`start`] hands back where it arrives).

use super::owner::{call, ensure_poller};
use super::runs::{RunOut, START_PATH, STOP_PATH, StartReq, Started, WireRunEvent};
use crate::routes::{AppState, now_ms};
use base64::Engine;
use pdt_core::link::RemoteInstance;
use pdt_core::pr::CiState;
use pdt_core::run::{RunKind, RunState};
use pdt_core::runconfig::RunDef;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

const PREFIX: &str = "local-ci-";
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// How a remote CI run ended.
#[derive(Debug)]
pub(crate) struct Ended {
    pub state: CiState,
    pub code: Option<i64>,
    /// Why, when it didn't pass (the remote's reason, or that it stopped running it).
    pub why: Option<String>,
}

/// A remote CI run going on.
struct Watched {
    link_id: String,
    proc: String,
    log: PathBuf,
    /// When its start came back (ms; `i64::MAX` while it is being started).
    started_at: i64,
    done: tokio::sync::oneshot::Sender<Ended>,
}

/// The owner's remote CI runs going on, by their run id there.
#[derive(Default)]
pub(super) struct CiSide(Mutex<HashMap<String, Watched>>);

impl CiSide {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Watched>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn run_id(id: i64) -> String {
    format!("{PREFIX}{id}")
}

pub(super) fn is_ci(run_id: &str) -> bool {
    run_id.starts_with(PREFIX)
}

/// How many remote CI runs go on on the instance on `link_id` (its poller keeps reading while there are).
pub(super) fn active_on(st: &AppState, link_id: &str) -> usize {
    st.exec.ci.map().values().filter(|w| w.link_id == link_id).count()
}

/// Starts the local CI run `id`'s `command` in `branch`'s worktree on `i`, its output appended to `log`. Its end
/// arrives on the receiver; Err (why) when it couldn't be started.
pub(crate) async fn start(
    st: &AppState,
    i: &RemoteInstance,
    id: i64,
    branch: &str,
    command: &str,
    log: PathBuf,
) -> Result<tokio::sync::oneshot::Receiver<Ended>, String> {
    let (rid, proc) = (run_id(id), uuid::Uuid::now_v7().to_string());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let w = Watched { link_id: i.link_id.clone(), proc: proc.clone(), log, started_at: i64::MAX, done: tx };
    st.exec.ci.map().insert(rid.clone(), w);
    ensure_poller(st, &i.link_id);
    let def = RunDef {
        name: "CI".into(),
        setup: None,
        command: command.into(),
        web: false,
        data: false,
        detach: false,
        env: [("CI".to_string(), "true".to_string())].into(),
        os: vec![],
    };
    let req = StartReq {
        run_id: rid.clone(),
        proc,
        kind: RunKind::Test,
        branch: Some(branch.into()),
        run_branch: branch.into(),
        def,
    };
    match call::<Started>(st, i, START_PATH, &req, CALL_TIMEOUT).await {
        Ok(_) => {
            if let Some(w) = st.exec.ci.map().get_mut(&rid) {
                w.started_at = now_ms();
            }
            Ok(rx)
        }
        Err(e) => {
            st.exec.ci.map().remove(&rid);
            Err(e.into_api(&i.name).message().to_string())
        }
    }
}

/// Stops the local CI run `id` on `i` (best effort: one it doesn't run has stopped); its reports are ignored from now.
pub(crate) async fn stop(st: &AppState, i: &RemoteInstance, id: i64) {
    let rid = run_id(id);
    st.exec.ci.map().remove(&rid);
    let req = super::runs::RunReq { run_id: rid };
    if let Err(e) = call::<serde_json::Value>(st, i, STOP_PATH, &req, CALL_TIMEOUT).await {
        tracing::info!(run = id, instance = %i.name, error = %e.text(), "could not stop a remote CI run");
    }
}

fn ci_state(state: RunState) -> CiState {
    match state {
        RunState::Passed => CiState::Passed,
        RunState::Stopped => CiState::Cancelled,
        _ => CiState::Failed,
    }
}

/// Applies the remote CI runs' reports after `after` (the instance's [`is_ci`] ones): the highest number read.
pub(super) async fn apply(st: &AppState, i: &RemoteInstance, events: Vec<WireRunEvent>, after: i64) -> i64 {
    let mut last = after;
    for e in events {
        if e.seq <= after {
            continue;
        }
        last = last.max(e.seq);
        let log = match st.exec.ci.map().get(&e.run_id) {
            // Another start's, or another instance's.
            Some(w) if w.proc == e.proc && w.link_id == i.link_id => w.log.clone(),
            _ => continue,
        };
        match e.out {
            RunOut::State { .. } => {}
            RunOut::Output { data, .. } => {
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) else { continue };
                if let Err(err) = super::runs::append(&log, &bytes).await {
                    tracing::warn!(run = %e.run_id, error = %err, "could not keep a remote CI run's output");
                }
            }
            RunOut::Finished { state, code, error } => {
                if let Some(w) = st.exec.ci.map().remove(&e.run_id) {
                    let ended = Ended { state: ci_state(state), code: code.map(i64::from), why: error };
                    let _ = w.done.send(ended);
                }
            }
        }
    }
    last
}

/// The instance's CI runs it no longer drives, though no end came for them (it restarted): cancelled, so CI runs
/// again. Those started after the poll was sent (`sent_at`) may not have reached it yet.
pub(super) fn reconcile(st: &AppState, i: &RemoteInstance, alive: &[String], sent_at: i64) {
    let gone: Vec<Watched> = {
        let mut map = st.exec.ci.map();
        let ids: Vec<String> = map
            .iter()
            .filter(|(id, w)| w.link_id == i.link_id && w.started_at < sent_at && !alive.contains(id))
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| map.remove(id)).collect()
    };
    for w in gone {
        let why = Some(format!("{} no longer runs it (it restarted?)", i.name));
        let _ = w.done.send(Ended { state: CiState::Cancelled, code: None, why });
    }
}
