//! The local forge (PDT-178): `forge = "local"` in `.pdt/project.toml`, for a repository with no remote. PDT is the
//! forge. An active attempt's branch with commits its base lacks has a change request, kept in the store; CI is the
//! `[ci]` command, run in the attempt's worktree on each new branch head (its log in `<data dir>/local-ci`); merging is
//! a fast-forward or a merge commit into the base in the project's checkout, once CI passes. No merge queue, no issues,
//! no reviews on the forge (PDT's own review only).
//!
//! The PR poller reads it like any forge; each read of a change request also moves it on (opens it, starts CI on a new
//! head, merges it once it is set to merge and CI passed). A run that ends wakes [`run_local_ci_watch`], which reads
//! the attempt again at once rather than at the next 60 s poll. Its runs are read as a CI provider, [`LocalCi`]
//! (PDT-242), the local forge's own CI.
//!
//! With `[push] remote` (PDT-245) the repository has a plain git remote on any host: the merge commit (or the branch
//! head, for a fast-forward) is pushed to the remote's base first, and the local base moves only once the remote took
//! it. A push the remote refuses because it moved on is recovered: fetch, fast-forward the local base, merge it into
//! the branch (whose new head CI runs on again), and merge once that passes. What can't be recovered is a notice in
//! the Inbox, and the change request waits for the next Merge. With `branches`, each ticket branch is pushed too as it
//! moves. Git uses the user's own credentials; PDT stores none.

use crate::error::{ApiError, OrNotFound};
use crate::extract::ApiPath;
use crate::routes::{AppState, now_ms};
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use pdt_core::brand::SHORT_NAME;
use pdt_core::forge::ForgeNotes;
use pdt_core::inbox::{InboxGroup, InboxKind};
use pdt_core::pr::{CiJob, CiRun, CiState, LOCAL_CI, Mergeable, PrState};
use pdt_core::runconfig::PushDef;
use pdt_core::ticket::Actor;
use pdt_forge::{
    Capabilities, ChangeRequest, CiCapabilities, CiConfig, CiProvider, Forge, ForgeError, MergeSettings, MergeStrategy,
};
use pdt_store::{AttemptInfo, LocalChange, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The longest one CI run may take before it is stopped and fails.
const CI_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

/// How many times a change request's refused push is recovered before PDT asks the user instead.
const MAX_RECOVERIES: u32 = 3;

/// What every local-forge project shares: the CI runs going on in this pdtd, and the wake-up when one ends.
pub(crate) struct LocalCiState {
    /// Each running CI process by run id, with the signal that stops it.
    live: Mutex<HashMap<i64, Arc<tokio::sync::Notify>>>,
    /// One change request moves on at a time (two reads at once would open two, or start CI twice).
    step: tokio::sync::Mutex<()>,
    /// How many refused pushes each change request recovered from, by id (PDT-245).
    recoveries: Mutex<HashMap<i64, u32>>,
    /// (project, branch) of each run that ended.
    ended: tokio::sync::mpsc::UnboundedSender<(String, String)>,
    ended_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<(String, String)>>>,
}

impl Default for LocalCiState {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        LocalCiState {
            live: Default::default(),
            step: Default::default(),
            recoveries: Default::default(),
            ended: tx,
            ended_rx: Mutex::new(Some(rx)),
        }
    }
}

impl LocalCiState {
    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Arc<tokio::sync::Notify>>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn recoveries(&self) -> std::sync::MutexGuard<'_, HashMap<i64, u32>> {
        self.recoveries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The local forge of one project.
pub(crate) struct LocalForge {
    st: AppState,
    store: Store,
    rt: tokio::runtime::Handle,
    ci: Arc<LocalCiState>,
    project_id: String,
    repo: PathBuf,
    /// The `[ci]` command; None: no CI, so a change request merges as soon as it is set to.
    command: Option<String>,
    /// `[push]`: the remote the base is pushed to on merge; None: it pushes nowhere.
    push: Option<PushDef>,
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
    pub(crate) fn new(
        st: &AppState,
        project_id: &str,
        repo_path: &str,
        command: Option<String>,
        push: Option<PushDef>,
    ) -> Self {
        LocalForge {
            st: st.clone(),
            store: st.store.clone(),
            rt: tokio::runtime::Handle::current(),
            ci: st.local_ci.clone(),
            project_id: project_id.to_string(),
            repo: PathBuf::from(repo_path),
            command,
            push,
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
                    self.push_branch(branch);
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
                self.push_branch(branch);
                c
            }
        };
        let mergeable = if preview.conflicts.is_empty() { Mergeable::Mergeable } else { Mergeable::Conflicting };
        let ci = match (&self.command, attempt.worktree_path.as_deref().map(PathBuf::from).filter(|w| w.is_dir())) {
            // A command the user hasn't approved waits for them (PDT-405).
            (Some(command), Some(_)) if !self.approved(&change, command).await? => None,
            (Some(command), Some(worktree)) => self.run_ci(&change, command, &worktree).await?,
            // No worktree to run in: CI waits for one.
            (Some(_), None) => None,
            (None, _) => Some(CiState::Passed),
        };
        if change.auto_merge
            && mergeable == Mergeable::Mergeable
            && ci == Some(CiState::Passed)
            && let Some(push) = &self.push
        {
            let message = format!("Merge {branch}: {}", change.title);
            // Merged or not, the stored change request has the news (merged, or no longer set to merge).
            self.merge_and_push(&attempt, &change, push, &message).await?;
            return self.reread(branch, mergeable).await;
        }
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

    /// Pushes `branch` to the remote in the background when `[push] branches` asks for it (a backup, or for CI that
    /// watches pushed branches). The agent may rewrite its branch, so it is forced; a failure is only logged.
    fn push_branch(&self, branch: &str) {
        let Some(push) = self.push.clone().filter(|p| p.branches) else { return };
        let (repo, spec) = (self.repo.clone(), format!("+refs/heads/{branch}:refs/heads/{branch}"));
        self.rt.spawn_blocking(move || {
            if let Err(e) = pdt_git::push(&repo, &push.remote, &[&spec], pdt_git::BACKGROUND_REMOTE_TIMEOUT) {
                tracing::warn!(remote = %push.remote, %spec, error = %e, "could not push a ticket branch");
            }
        });
    }

    /// Merges the change request by way of the remote: the commit its base becomes is pushed there first, and the local
    /// base moves only once the remote took it. A refused push is recovered ([`Self::recover`]).
    async fn merge_and_push(
        &self,
        attempt: &AttemptInfo,
        change: &LocalChange,
        push: &PushDef,
        message: &str,
    ) -> Result<(), ForgeError> {
        let (repo, b, base, m) =
            (self.repo.clone(), change.branch.clone(), change.base_branch.clone(), message.to_string());
        let prepared =
            tokio::task::spawn_blocking(move || pdt_git::prepare_merge(&repo, &b, &base, &m)).await.map_err(failed)?;
        let (old, commit) = match prepared {
            Ok(pdt_git::PreparedMerge::Ready { old, commit }) => (old, commit),
            // Conflicts are the agent's (the poller tells it); a dirty base checkout, the user's.
            other => {
                tracing::warn!(ticket = %attempt.ticket_key, outcome = ?other, "could not merge a local change request");
                return Ok(());
            }
        };
        let base = &change.base_branch;
        let (repo, remote, spec) = (self.repo.clone(), push.remote.clone(), format!("{commit}:refs/heads/{base}"));
        let pushed = tokio::task::spawn_blocking(move || {
            pdt_git::push(&repo, &remote, &[&spec], pdt_git::BACKGROUND_REMOTE_TIMEOUT)
        })
        .await
        .map_err(failed)?;
        match pushed {
            Ok(pdt_git::PushOutcome::Pushed) => {}
            Ok(pdt_git::PushOutcome::Rejected(why)) => {
                self.recover(attempt, change, push, &why).await?;
                return Ok(());
            }
            Err(e) => {
                let text = format!(
                    "{SHORT_NAME} could not push `{base}` to the remote `{}` for change request #{} ({}): {e}\n\n\
                     Git pushes with your own credentials (ssh keys or a credential helper) and can't ask for them. \
                     Fix the remote or the credentials, then Merge again.",
                    push.remote, change.number, change.branch
                );
                self.give_up(attempt, change, "push-failed", "Could not push to the remote", text).await;
                return Ok(());
            }
        }
        let (repo, base_c, old_c, commit_c) = (self.repo.clone(), base.clone(), old.clone(), commit.clone());
        let advanced = tokio::task::spawn_blocking(move || pdt_git::advance_branch(&repo, &base_c, &old_c, &commit_c))
            .await
            .map_err(failed)?;
        if let Err(e) = advanced {
            // The remote has the merge; the local base catches up by hand (a pull), after which the change is merged.
            let text = format!(
                "{SHORT_NAME} pushed change request #{} ({}) to `{base}` on the remote `{}`, but could not move the \
                 local `{base}` to it: {e}\n\nPull `{base}` from `{}` in the project's checkout.",
                change.number, change.branch, push.remote, push.remote
            );
            self.give_up(attempt, change, "advance-failed", "Merged on the remote, not here", text).await;
            return Ok(());
        }
        self.store.end_local_change(change.id, PrState::Merged, &change.head_sha, now_ms()).await.map_err(failed)?;
        self.ci.recoveries().remove(&change.id);
        tracing::info!(ticket = %attempt.ticket_key, number = change.number, commit = %commit, remote = %push.remote,
            "merged a local change request and pushed it");
        Ok(())
    }

    /// The remote refused the push because its base moved on: fetch it, fast-forward the local base to it and merge
    /// that into the branch, whose new head CI runs on again before the merge is tried again. What can't be done this
    /// way (the bases diverged, a dirty checkout, too many tries) is a notice for the user.
    async fn recover(
        &self,
        attempt: &AttemptInfo,
        change: &LocalChange,
        push: &PushDef,
        why: &str,
    ) -> Result<(), ForgeError> {
        let (base, branch, remote) = (&change.base_branch, &change.branch, &push.remote);
        let tries = {
            let mut r = self.ci.recoveries();
            let n = r.entry(change.id).or_default();
            *n += 1;
            *n
        };
        if tries > MAX_RECOVERIES {
            let text = format!(
                "The remote `{remote}` refused change request #{} ({branch}) {MAX_RECOVERIES} times: its `{base}` \
                 keeps moving on.\n\n{why}\n\nMerge again to try once more.",
                change.number
            );
            self.give_up(attempt, change, "push-rejected", "The remote keeps refusing the merge", text).await;
            return Ok(());
        }
        tracing::info!(ticket = %attempt.ticket_key, number = change.number, %remote, "the remote moved on; catching up");
        let (repo, r, b) = (self.repo.clone(), remote.clone(), base.clone());
        let caught_up = tokio::task::spawn_blocking(move || catch_up(&repo, &r, &b)).await.map_err(failed)?;
        let tip = match caught_up {
            Ok(tip) => tip,
            Err(e) => {
                let text = format!(
                    "The remote `{remote}` refused change request #{} ({branch}) because its `{base}` moved on, and \
                     {SHORT_NAME} could not catch up with it: {e}\n\nBring the local `{base}` and `{remote}/{base}` \
                     together by hand (pull, then push), then Merge again.",
                    change.number
                );
                self.give_up(attempt, change, "catch-up-failed", "Could not catch up with the remote", text).await;
                return Ok(());
            }
        };
        let message = format!("Merge {base} into {branch}");
        let (repo, b, br) = (self.repo.clone(), base.clone(), branch.clone());
        // `base` into `branch`: in the attempt's worktree, which has the branch checked out (it must be clean).
        let merged = tokio::task::spawn_blocking(move || pdt_git::merge_local(&repo, &b, &br, &message, false))
            .await
            .map_err(failed)?;
        let note = match merged {
            Ok(pdt_git::MergeOutcome::Merged { .. }) => format!(
                "`{base}` on the remote `{remote}` had moved on (to {}), so {SHORT_NAME} merged it into the branch; CI \
                 runs again before change request #{} merges.",
                short(&tip),
                change.number
            ),
            Ok(pdt_git::MergeOutcome::Nothing) => return Ok(()),
            // The next read finds the conflict and tells the agent, as for any conflict with the base.
            Ok(pdt_git::MergeOutcome::Conflict { .. }) => format!(
                "`{base}` on the remote `{remote}` had moved on (to {}) and conflicts with the branch: merge `{base}` \
                 in and resolve it; change request #{} then merges once CI passes.",
                short(&tip),
                change.number
            ),
            Ok(pdt_git::MergeOutcome::BaseDirty { checkout }) => {
                let text = format!(
                    "`{base}` on the remote `{remote}` moved on, and {SHORT_NAME} could not merge it into {branch}: \
                     its worktree ({}) has uncommitted changes. Commit them (or merge `{base}` in), then Merge again.",
                    checkout.display()
                );
                self.give_up(attempt, change, "worktree-dirty", "Could not catch up with the remote", text).await;
                return Ok(());
            }
            Err(e) => {
                let text = format!(
                    "`{base}` on the remote `{remote}` moved on, and {SHORT_NAME} could not merge it into {branch}: {e}"
                );
                self.give_up(attempt, change, "merge-failed", "Could not catch up with the remote", text).await;
                return Ok(());
            }
        };
        if let Err(e) = self.store.add_comment_by(&attempt.ticket_id, &note, Actor::System, None, now_ms()).await {
            tracing::warn!(ticket = %attempt.ticket_key, error = %e, "could not note catching up with the remote");
        }
        // Read the attempt again at once: CI starts on the new head.
        let _ = self.ci.ended.send((self.project_id.clone(), branch.clone()));
        Ok(())
    }

    /// Stops trying to merge `change` (it waits for the next Merge) and tells the user in the Inbox, once per head and
    /// `key`. It closes once the attempt ends.
    async fn give_up(&self, attempt: &AttemptInfo, change: &LocalChange, key: &str, title: &str, text: String) {
        tracing::warn!(ticket = %attempt.ticket_key, number = change.number, %key, %text, "a local merge needs the user");
        self.ci.recoveries().remove(&change.id);
        if let Err(e) = self.store.set_local_change_auto_merge(change.id, false).await {
            tracing::warn!(ticket = %attempt.ticket_key, error = %e, "could not stop a local merge");
        }
        let title = format!("{title}: change request #{}", change.number);
        let body = serde_json::json!({"notice": null, "key": key, "text": text});
        let item = crate::inbox::new_item(InboxKind::Notice, InboxGroup::Notice, title, body)
            .project(attempt.project_id.clone())
            .ticket(attempt.ticket_id.clone())
            .attempt(attempt.attempt.id.clone());
        let dedupe = format!("local-push:{}:{}:{key}", attempt.attempt.id, change.head_sha);
        match self.store.add_inbox_item_once(&item, &dedupe).await {
            Ok(_) => crate::notifications::kick(&self.st),
            Err(e) => tracing::warn!(ticket = %attempt.ticket_key, error = %e, "could not put a notice in the Inbox"),
        }
    }

    async fn stepped(&self, change: LocalChange, mergeable: Mergeable) -> Stepped {
        let newest = self.store.local_ci_runs(change.id).await.ok().and_then(|r| r.last().map(|r| r.id));
        Stepped { url: newest.map(|id| log_url(self.port, id)).unwrap_or_default(), change, mergeable }
    }

    /// Whether the user approved the CI command; when not, the Inbox says CI waits on them.
    async fn approved(&self, change: &LocalChange, command: &str) -> Result<bool, ForgeError> {
        let repo = self.repo.to_string_lossy();
        let commands = vec![pdt_core::command_trust::of_ci(command)];
        let left = crate::command_trust::untrusted(&self.st, &self.project_id, &repo, commands)
            .await
            .map_err(|e| failed(e.message()))?;
        if left.is_empty() {
            return Ok(true);
        }
        let what = format!("CI for change request #{}", change.number);
        crate::command_trust::notify(&self.st, &self.project_id, &left, &what).await;
        Ok(false)
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
        let job = CiProcess {
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

fn short(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}

/// Fetches `remote`'s `base` and fast-forwards the local `base` to it; the remote's tip. Err when they diverged (the
/// local base has commits the remote lacks) or the local base can't move.
fn catch_up(repo: &std::path::Path, remote: &str, base: &str) -> Result<String, String> {
    let tracking = format!("refs/remotes/{remote}/{base}");
    pdt_git::fetch(repo, remote, &format!("+refs/heads/{base}:{tracking}"), pdt_git::BACKGROUND_REMOTE_TIMEOUT)
        .map_err(|e| e.to_string())?;
    let tip = pdt_git::rev_parse(repo, &tracking).ok_or_else(|| format!("`{remote}` has no `{base}`"))?;
    let old = pdt_git::rev_parse(repo, &format!("refs/heads/{base}")).ok_or_else(|| format!("no local `{base}`"))?;
    if old == tip || pdt_git::is_ancestor(repo, &tip, &old) {
        return Ok(tip);
    }
    if !pdt_git::is_ancestor(repo, &old, &tip) {
        return Err(format!(
            "the local `{base}` and `{remote}/{base}` have diverged (each has commits the other lacks)"
        ));
    }
    pdt_git::advance_branch(repo, base, &old, &tip).map_err(|e| e.to_string())?;
    Ok(tip)
}

/// One CI run going on.
struct CiProcess {
    store: Store,
    ci: Arc<LocalCiState>,
    id: i64,
    command: String,
    dir: PathBuf,
    log: PathBuf,
    path: Option<String>,
    branch: String,
    project_id: String,
}

impl CiProcess {
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
        // The log's folder and file are made on the blocking pool, like the process itself.
        let (folder, file, header) =
            (self.log.parent().unwrap_or(&self.dir).to_path_buf(), self.log.clone(), format!("$ {}\n", self.command));
        let opened = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&folder)?;
            let mut log = std::fs::File::create(&file)?;
            let _ = writeln!(log, "{header}");
            let (out, err) = (log.try_clone()?, log.try_clone()?);
            Ok::<_, std::io::Error>((log, out, err))
        })
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));
        let (mut log, out, err) = match opened {
            Ok(files) => files,
            Err(e) => {
                tracing::warn!(run = self.id, error = %e, "could not create a local CI log");
                return (CiState::Failed, None);
            }
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
        let mut child = match pdt_agents::launch::spawn_child(cmd).await {
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
        Capabilities { merge_queue: false, required_checks: true, issues: false, reviews: false, releases: false }
    }

    fn agent_notes(&self) -> ForgeNotes {
        let api = format!("http://127.0.0.1:{}/api", self.port);
        if let Some(push) = &self.push {
            let ci = match &self.command {
                Some(c) => format!(
                    " runs its CI (`{c}`) in this worktree on each new commit (a run's log is at `GET {api}/local-ci/runs/<id>/log`) and, once it passes,"
                ),
                None => String::new(),
            };
            let ci = format!(
                "Commit on `{{branch}}`; don't push, and don't push the base. {SHORT_NAME}{ci} merges the branch into \
                 its base itself and pushes the base to the remote `{}`.",
                push.remote
            );
            return ForgeNotes { ci: Some(ci), reply: None, watch: None };
        }
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

    fn merge_settings(&self, _repo: &str, _base: &str) -> Result<MergeSettings, ForgeError> {
        Ok(MergeSettings { squash: false, merge: true, rebase: false, queue: false, auto_merge: true })
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

/// The local forge's CI (PDT-242): the runs of the `[ci]` command on a branch's change request, which the local forge
/// starts as it moves the change request on. Each run has one job, the command, whose log is PDT's.
pub(crate) struct LocalCi {
    store: Store,
    rt: tokio::runtime::Handle,
    project_id: String,
    logs: PathBuf,
    port: u16,
}

impl LocalCi {
    pub(crate) fn new(st: &AppState, project_id: &str) -> Self {
        LocalCi {
            store: st.store.clone(),
            rt: tokio::runtime::Handle::current(),
            project_id: project_id.to_string(),
            logs: st.data_dir.join("local-ci"),
            port: st.listen_port,
        }
    }

    fn block<T>(&self, f: impl Future<Output = Result<T, ForgeError>>) -> Result<T, ForgeError> {
        self.rt.block_on(f)
    }
}

/// The last `lines` lines of `text`.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let skipped = all.len().saturating_sub(lines);
    let mut out: String = all[skipped..].iter().map(|l| format!("{l}\n")).collect();
    if skipped > 0 {
        out = format!("… ({skipped} earlier lines)\n{out}");
    }
    out
}

impl CiProvider for LocalCi {
    fn id(&self) -> &str {
        LOCAL_CI
    }

    fn label(&self) -> &str {
        "Local CI"
    }

    fn capabilities(&self) -> CiCapabilities {
        CiCapabilities { logs: true, ..CiCapabilities::NONE }
    }

    /// It is the project's `[ci]` command: there when there is one.
    fn config_paths(&self) -> CiConfig {
        CiConfig::Known(true)
    }

    fn runs(&self, _repo: &str, branch: &str, _sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError> {
        self.block(async {
            let Some(c) = self.store.local_change_for_branch(&self.project_id, branch).await.map_err(failed)? else {
                return Ok(vec![]);
            };
            let runs = self.store.local_ci_runs(c.id).await.map_err(failed)?;
            Ok(runs
                .into_iter()
                .map(|r| CiRun {
                    provider: LOCAL_CI.into(),
                    id: r.id.to_string(),
                    attempt: 1,
                    name: "ci".into(),
                    state: r.state,
                    url: log_url(self.port, r.id),
                    head_sha: r.head_sha,
                    branch: branch.to_string(),
                    // Ordered as text: fixed width.
                    created_at: format!("{:020}", r.created_at),
                    failed_jobs: if r.state == CiState::Failed { vec![r.command] } else { vec![] },
                })
                .collect())
        })
    }

    fn jobs(&self, _repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError> {
        let Ok(id) = run.id.parse::<i64>() else { return Ok(vec![]) };
        let found = self.block(async { self.store.local_ci_run(id).await.map_err(failed) })?;
        Ok(found
            .map(|r| {
                vec![CiJob { id: r.id.to_string(), name: r.command, state: r.state, url: log_url(self.port, r.id) }]
            })
            .unwrap_or_default())
    }

    fn job_log_tail(&self, _repo: &str, _run: &CiRun, job: &CiJob, lines: usize) -> Result<String, ForgeError> {
        let id = job.id.parse::<i64>().map_err(|_| ForgeError::Failed(format!("{} is not a local CI run", job.id)))?;
        match std::fs::read(log_path(&self.logs, id)) {
            Ok(b) => Ok(tail(&String::from_utf8_lossy(&b), lines)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(failed(e)),
        }
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
pub(crate) async fn run_log(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<impl IntoResponse, ApiError> {
    st.store.local_ci_run(id).await?.or_404("CI run")?;
    let path = log_path(&st.data_dir.join("local-ci"), id);
    let text = match tokio::fs::read(&path).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ApiError::from(e)),
    };
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text))
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new().route("/local-ci/runs/{id}/log", get(run_log))
}
