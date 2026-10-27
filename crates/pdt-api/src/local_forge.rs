//! The local forge (PDT-178): `forge = "local"` in `.pdt/project.toml`, for a repository with no remote. PDT is the
//! forge. An active attempt's branch with commits its base lacks has a change request, kept in the store; CI is the
//! `[ci]` command, run in the attempt's worktree on each new branch head (its log in `<data dir>/local-ci`); merging is
//! a fast-forward or a merge commit into the base in the project's checkout, once CI passes. No merge queue, no issues,
//! no reviews on the forge (PDT's own review only).
//!
//! The PR poller reads it like any forge; each read of a change request also moves it on (opens it, starts CI on a new
//! head, merges it once it is set to merge and CI passed). A run that ends wakes [`run_local_ci_watch`], which reads
//! the attempt again at once rather than at the next 60 s poll.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use pdt_core::brand::SHORT_NAME;
use pdt_core::forge::ForgeNotes;
use pdt_core::pr::{CiRun, CiState, Mergeable, PrState};
use pdt_forge::{Capabilities, ChangeRequest, Forge, ForgeError, MergeSettings, MergeStrategy};
use pdt_store::{LocalChange, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The longest one CI run may take before it is stopped and fails.
const CI_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

/// What every local-forge project shares: the CI runs going on in this pdtd, and the wake-up when one ends.
pub(crate) struct LocalCi {
    /// Each running CI process by run id, with the signal that stops it.
    live: Mutex<HashMap<i64, Arc<tokio::sync::Notify>>>,
    /// One change request moves on at a time (two reads at once would open two, or start CI twice).
    step: tokio::sync::Mutex<()>,
    /// (project, branch) of each run that ended.
    ended: tokio::sync::mpsc::UnboundedSender<(String, String)>,
    ended_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<(String, String)>>>,
}

impl Default for LocalCi {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        LocalCi { live: Default::default(), step: Default::default(), ended: tx, ended_rx: Mutex::new(Some(rx)) }
    }
}

impl LocalCi {
    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Arc<tokio::sync::Notify>>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The local forge of one project.
pub(crate) struct LocalForge {
    store: Store,
    rt: tokio::runtime::Handle,
    ci: Arc<LocalCi>,
    project_id: String,
    repo: PathBuf,
    /// The `[ci]` command; None: no CI, so a change request merges as soon as it is set to.
    command: Option<String>,
    logs: PathBuf,
    port: u16,
    /// The user's login PATH, so the command finds what a terminal would.
    path: Option<String>,
}

/// The URL of a local CI run's log (`GET /api/local-ci/runs/{id}/log`).
pub(crate) fn log_url(port: u16, id: i64) -> String {
    format!("http://127.0.0.1:{port}/api/local-ci/runs/{id}/log")
}

fn log_path(logs: &std::path::Path, id: i64) -> PathBuf {
    logs.join(format!("{id}.log"))
}

fn failed(e: impl std::fmt::Display) -> ForgeError {
    ForgeError::Failed(e.to_string())
}

/// A change request moved on: as stored, and whether it conflicts with its base.
struct Stepped {
    change: LocalChange,
    mergeable: Mergeable,
    /// The newest CI run's log.
    url: String,
}

impl LocalForge {
    pub(crate) fn new(st: &AppState, project_id: &str, repo_path: &str, command: Option<String>) -> Self {
        LocalForge {
            store: st.store.clone(),
            rt: tokio::runtime::Handle::current(),
            ci: st.local_ci.clone(),
            project_id: project_id.to_string(),
            repo: PathBuf::from(repo_path),
            command,
            logs: st.data_dir.join("local-ci"),
            port: st.listen_port,
            path: st.platform.login_path.clone(),
        }
    }

    /// Runs the forge's async work from the blocking thread a forge is called on.
    fn block<T>(&self, f: impl Future<Output = Result<T, ForgeError>>) -> Result<T, ForgeError> {
        self.rt.block_on(f)
    }

    /// Moves `branch`'s change request on (see the module docs) and gives it; None when it has none.
    async fn step(&self, branch: &str) -> Result<Option<Stepped>, ForgeError> {
        let _one = self.ci.step.lock().await;
        let change = self.store.local_change_for_branch(&self.project_id, branch).await.map_err(failed)?;
        let attempts = self.store.list_active_attempts().await.map_err(failed)?;
        let attempt = attempts.into_iter().find(|a| a.project_id == self.project_id && a.attempt.branch == branch);
        let (repo, b) = (self.repo.clone(), branch.to_string());
        let head = tokio::task::spawn_blocking(move || pdt_git::rev_parse(&repo, &format!("refs/heads/{b}")))
            .await
            .map_err(failed)?;
        // An ended attempt's (or a deleted branch's) change request stays as it was.
        let (Some(attempt), Some(head)) = (attempt, head) else {
            return Ok(match change {
                Some(c) => Some(self.stepped(c, Mergeable::Mergeable).await),
                None => None,
            });
        };
        let base = match &change {
            Some(c) if c.state == PrState::Open => c.base_branch.clone(),
            _ => attempt.attempt.base_branch.clone(),
        };
        let (repo, b, base_c) = (self.repo.clone(), branch.to_string(), base.clone());
        let preview = tokio::task::spawn_blocking(move || pdt_git::merge_preview(&repo, &b, &base_c))
            .await
            .map_err(failed)?
            .map_err(failed)?;
        let mut change = match change {
            Some(c) if c.state == PrState::Open => {
                if preview.commits == 0 {
                    // Its commits reached the base some other way (merged by hand): merged.
                    self.store.end_local_change(c.id, PrState::Merged, &head, now_ms()).await.map_err(failed)?;
                    return self.reread(branch, Mergeable::Mergeable).await;
                }
                if c.head_sha != head {
                    self.store.set_local_change_head(c.id, &head).await.map_err(failed)?;
                }
                LocalChange { head_sha: head.clone(), ..c }
            }
            // Nothing of its own yet (or only what is merged): no change request to open.
            other if preview.commits == 0 => {
                return Ok(match other {
                    Some(c) => Some(self.stepped(c, Mergeable::Mergeable).await),
                    None => None,
                });
            }
            _ => {
                let title = match self.store.get_ticket_summary(&attempt.ticket_id).await.map_err(failed)? {
                    Some(t) => format!("{}: {}", attempt.ticket_key, t.title),
                    None => attempt.ticket_key.clone(),
                };
                let c = self
                    .store
                    .open_local_change(&self.project_id, branch, &base, &title, &head, now_ms())
                    .await
                    .map_err(failed)?;
                tracing::info!(ticket = %attempt.ticket_key, number = c.number, "opened a local change request");
                c
            }
        };
        let mergeable = if preview.conflicts.is_empty() { Mergeable::Mergeable } else { Mergeable::Conflicting };
        let ci = match (&self.command, attempt.worktree_path.as_deref().map(PathBuf::from).filter(|w| w.is_dir())) {
            (Some(command), Some(worktree)) => self.run_ci(&change, command, &worktree).await?,
            // No worktree to run in: CI waits for one.
            (Some(_), None) => None,
            (None, _) => Some(CiState::Passed),
        };
        if change.auto_merge && mergeable == Mergeable::Mergeable && ci == Some(CiState::Passed) {
            let message = format!("Merge {branch}: {}", change.title);
            let (repo, b, base) = (self.repo.clone(), branch.to_string(), change.base_branch.clone());
            let merged = tokio::task::spawn_blocking(move || pdt_git::merge_fast_forward(&repo, &b, &base, &message))
                .await
                .map_err(failed)?;
            match merged {
                Ok(pdt_git::MergeOutcome::Merged { commit }) => {
                    self.store.end_local_change(change.id, PrState::Merged, &head, now_ms()).await.map_err(failed)?;
                    tracing::info!(ticket = %attempt.ticket_key, number = change.number, commit = %commit,
                        "merged a local change request");
                    return self.reread(branch, mergeable).await;
                }
                // Conflicts are the agent's (the poller tells it); a dirty base checkout, the user's.
                Ok(other) => tracing::warn!(ticket = %attempt.ticket_key, outcome = ?other,
                    "could not merge a local change request"),
                Err(e) => tracing::warn!(ticket = %attempt.ticket_key, error = %e,
                    "could not merge a local change request"),
            }
            change.auto_merge = true;
        }
        Ok(Some(self.stepped(change, mergeable).await))
    }

    async fn reread(&self, branch: &str, mergeable: Mergeable) -> Result<Option<Stepped>, ForgeError> {
        match self.store.local_change_for_branch(&self.project_id, branch).await.map_err(failed)? {
            Some(c) => Ok(Some(self.stepped(c, mergeable).await)),
            None => Ok(None),
        }
    }

    async fn stepped(&self, change: LocalChange, mergeable: Mergeable) -> Stepped {
        let newest = self.store.local_ci_runs(change.id).await.ok().and_then(|r| r.last().map(|r| r.id));
        Stepped { url: newest.map(|id| log_url(self.port, id)).unwrap_or_default(), change, mergeable }
    }

    /// CI on the change request's head: stops runs of older heads, starts one on the head unless it has one (a run
    /// this pdtd isn't running any more was cut short by a restart: it is cancelled and run again). Its state.
    async fn run_ci(
        &self,
        change: &LocalChange,
        command: &str,
        worktree: &std::path::Path,
    ) -> Result<Option<CiState>, ForgeError> {
        let runs = self.store.local_ci_runs(change.id).await.map_err(failed)?;
        let head = &change.head_sha;
        for r in runs.iter().filter(|r| r.state == CiState::Running) {
            let stop = self.ci.live().get(&r.id).cloned();
            match stop {
                Some(stop) if &r.head_sha != head => stop.notify_one(),
                Some(_) => {}
                None => {
                    self.store.finish_local_ci_run(r.id, CiState::Cancelled, None, now_ms()).await.map_err(failed)?;
                }
            }
        }
        let on_head = runs.iter().rfind(|r| &r.head_sha == head);
        match on_head {
            Some(r) if r.state != CiState::Running || self.ci.live().contains_key(&r.id) => return Ok(Some(r.state)),
            _ => {}
        }
        let run = self.store.add_local_ci_run(change.id, head, command, now_ms()).await.map_err(failed)?;
        let stop = Arc::new(tokio::sync::Notify::new());
        self.ci.live().insert(run.id, stop.clone());
        let job = CiJob {
            store: self.store.clone(),
            ci: self.ci.clone(),
            id: run.id,
            command: command.to_string(),
            dir: worktree.to_path_buf(),
            log: log_path(&self.logs, run.id),
            path: self.path.clone(),
            branch: change.branch.clone(),
            project_id: self.project_id.clone(),
        };
        self.rt.spawn(job.run(stop));
        Ok(Some(CiState::Running))
    }
}

/// One CI run going on.
struct CiJob {
    store: Store,
    ci: Arc<LocalCi>,
    id: i64,
    command: String,
    dir: PathBuf,
    log: PathBuf,
    path: Option<String>,
    branch: String,
    project_id: String,
}

impl CiJob {
    async fn run(self, stop: Arc<tokio::sync::Notify>) {
        let (state, code) = self.execute(&stop).await;
        if let Err(e) = self.store.finish_local_ci_run(self.id, state, code, now_ms()).await {
            tracing::warn!(run = self.id, error = %e, "could not record the end of a local CI run");
        }
        self.ci.live().remove(&self.id);
        let _ = self.ci.ended.send((self.project_id.clone(), self.branch.clone()));
    }

    /// Runs the command with its output in the log; how it ended.
    async fn execute(&self, stop: &tokio::sync::Notify) -> (CiState, Option<i64>) {
        use std::io::Write;
        let opened = std::fs::create_dir_all(self.log.parent().unwrap_or(&self.dir))
            .and_then(|_| std::fs::File::create(&self.log));
        let mut log = match opened {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(run = self.id, error = %e, "could not create a local CI log");
                return (CiState::Failed, None);
            }
        };
        let _ = writeln!(log, "$ {}\n", self.command);
        let (out, err) = match (log.try_clone(), log.try_clone()) {
            (Ok(o), Ok(e)) => (o, e),
            _ => return (CiState::Failed, None),
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(&self.command);
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(&self.command);
            c.process_group(0);
            c
        };
        cmd.current_dir(&self.dir)
            .env("CI", "true")
            .env("PDT_RUN_BRANCH", &self.branch)
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err)
            .kill_on_drop(true);
        if let Some(p) = &self.path {
            cmd.env("PATH", p);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = writeln!(log, "could not run the command: {e}");
                return (CiState::Failed, None);
            }
        };
        let mut guard = crate::workflow::nodes::command::TreeGuard(child.id());
        tokio::select! {
            status = child.wait() => {
                guard.0 = None;
                match status {
                    Ok(s) if s.success() => (CiState::Passed, s.code().map(i64::from)),
                    Ok(s) => {
                        let _ = writeln!(log, "\n{SHORT_NAME}: the command exited with {}", s.code().map_or("a signal".into(), |c| c.to_string()));
                        (CiState::Failed, s.code().map(i64::from))
                    }
                    Err(e) => {
                        let _ = writeln!(log, "\n{SHORT_NAME}: {e}");
                        (CiState::Failed, None)
                    }
                }
            }
            // Dropping the guard stops the process tree.
            _ = stop.notified() => {
                let _ = writeln!(log, "\n{SHORT_NAME}: stopped, the branch has a newer commit");
                (CiState::Cancelled, None)
            }
            _ = tokio::time::sleep(CI_LIMIT) => {
                let _ = writeln!(log, "\n{SHORT_NAME}: stopped after {} minutes", CI_LIMIT.as_secs() / 60);
                (CiState::Failed, None)
            }
        }
    }
}

impl Forge for LocalForge {
    fn name(&self) -> &'static str {
        "the local forge"
    }

    fn capabilities(&self) -> Capabilities {
        // PDT holds the merge until CI passes, as a forge's required checks would.
        Capabilities { merge_queue: false, required_checks: true, issues: false, reviews: false }
    }

    fn agent_notes(&self) -> ForgeNotes {
        let api = format!("http://127.0.0.1:{}/api", self.port);
        let ci = match &self.command {
            Some(c) => format!(
                "This project has no remote: commit on `{{branch}}`, don't push. {SHORT_NAME} runs its CI (`{c}`) in \
                 this worktree on each new commit and merges the branch into its base itself once CI passes; a run's \
                 log is at `GET {api}/local-ci/runs/<id>/log`."
            ),
            None => format!(
                "This project has no remote: commit on `{{branch}}`, don't push. {SHORT_NAME} merges the branch into \
                 its base itself."
            ),
        };
        ForgeNotes { ci: Some(ci), reply: None, watch: None }
    }

    fn has_ci(&self) -> Option<bool> {
        Some(self.command.is_some())
    }

    fn change_request(&self, _repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        let Some(s) = self.block(self.step(branch))? else { return Ok(None) };
        let c = s.change;
        Ok(Some(ChangeRequest {
            number: c.number,
            url: s.url,
            title: c.title,
            state: c.state,
            draft: false,
            head_sha: c.head_sha,
            base_branch: c.base_branch,
            mergeable: s.mergeable,
            behind: false,
            auto_merge: c.auto_merge,
            in_merge_queue: false,
            queue_position: None,
            queue_removal: None,
            created_at: Some(c.created_at),
        }))
    }

    fn branch_runs(&self, _repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
        self.block(async {
            let Some(c) = self.store.local_change_for_branch(&self.project_id, branch).await.map_err(failed)? else {
                return Ok(vec![]);
            };
            let runs = self.store.local_ci_runs(c.id).await.map_err(failed)?;
            Ok(runs
                .into_iter()
                .map(|r| CiRun {
                    id: r.id as u64,
                    attempt: 1,
                    workflow: "ci".into(),
                    state: r.state,
                    url: log_url(self.port, r.id),
                    head_sha: r.head_sha,
                    head_branch: branch.to_string(),
                    // Ordered as text: fixed width.
                    created_at: format!("{:020}", r.created_at),
                    failed_jobs: vec![],
                })
                .collect())
        })
    }

    fn failed_jobs(&self, _repo: &str, run_id: u64, _attempt: u32) -> Result<Vec<String>, ForgeError> {
        let run = self.block(async { self.store.local_ci_run(run_id as i64).await.map_err(failed) })?;
        Ok(run.map(|r| vec![r.command]).unwrap_or_default())
    }

    fn merge_settings(&self, _repo: &str, _base: &str) -> Result<MergeSettings, ForgeError> {
        Ok(MergeSettings { squash: false, merge: true, rebase: false, queue: false })
    }

    /// Sets the change request to merge once CI passes; one whose CI passed merges now.
    fn merge(&self, _repo: &str, number: u32, _strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        self.block(async {
            let c = self.store.local_change(&self.project_id, number).await.map_err(failed)?;
            let c = c.filter(|c| c.state == PrState::Open);
            let c = c.ok_or_else(|| ForgeError::Failed(format!("change request #{number} is not open")))?;
            self.store.set_local_change_auto_merge(c.id, true).await.map_err(failed)?;
            self.step(&c.branch).await.map(|_| ())
        })
    }
}

/// Reads again at once the attempt whose local CI run just ended (its CI news, and a merge waiting for it), then
/// checks it for the merge. pdtd runs it for its lifetime.
pub async fn run_local_ci_watch(st: AppState) {
    let Some(mut ended) = st.local_ci.ended_rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    while let Some((project, branch)) = ended.recv().await {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let attempts = st.store.list_active_attempts().await.unwrap_or_default();
        if let Some(info) = attempts.into_iter().find(|a| a.project_id == project && a.attempt.branch == branch) {
            crate::github::refresh_attempt(&st, &info).await;
            crate::start::end_if_merged(&st, &info).await;
        }
    }
}

/// `GET /api/local-ci/runs/{id}/log`: a local CI run's output so far, as text.
pub(crate) async fn run_log(State(st): State<AppState>, Path(id): Path<i64>) -> Result<impl IntoResponse, ApiError> {
    st.store.local_ci_run(id).await?.ok_or_else(|| ApiError::not_found("CI run"))?;
    let path = log_path(&st.data_dir.join("local-ci"), id);
    let text = match tokio::fs::read(&path).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ApiError::internal(e)),
    };
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text))
}
