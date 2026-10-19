//! Pull requests and CI of ticket attempts (PDT-25). pdtd reads each active attempt's PR through `gh` every 60 s
//! (and when a ticket is opened), keeps what it saw in memory for ticket responses (`attempt.pr`), and tells the
//! attempt's agent session when CI fails, the branch conflicts with its base, the PR leaves the merge queue without
//! merging, or the work is merged. A notice waits until the session is idle and nobody is typing in it, is typed in
//! as a prompt, and lands on the ticket's timeline (`pr` event), which also makes each one news only once.

use crate::routes::{AppState, now_ms};
use pdt_core::pr::{CiRun, CiState, CiStatus, Notice, PrState, PullRequestStatus, notices, queue_branch_prefix};
use pdt_core::session::{SessionKind, SessionStatus};
use pdt_core::ticket::{Actor, AttemptSummary, EventKind, Ticket, TicketSummary};
use pdt_github::{GhError, GithubApi};
use pdt_store::AttemptInfo;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// How long nobody may have typed into a session before pdtd types a notice into it.
const QUIET_MS: i64 = 20_000;

#[derive(Default)]
pub(crate) struct PrWatch {
    /// The PR of each attempt, as last seen (ended attempts keep their last look).
    by_attempt: HashMap<String, PullRequestStatus>,
    /// The failed jobs of each run attempt already asked for.
    jobs: HashMap<(u64, u32), Vec<String>>,
    /// Notices waiting for their session to be ready.
    pending: Vec<Pending>,
    /// (attempt, key) of the notices resolved in this pdtd run: a look that read the timeline before one was
    /// recorded must not queue it again.
    resolved: HashSet<(String, String)>,
    /// The last error per project, so a broken `gh` is logged once rather than every minute.
    errors: HashMap<String, String>,
}

#[derive(Clone)]
struct Pending {
    attempt_id: String,
    ticket_id: String,
    notice: Notice,
}

pub(crate) type SharedPrWatch = Arc<std::sync::Mutex<PrWatch>>;

fn watch(st: &AppState) -> std::sync::MutexGuard<'_, PrWatch> {
    st.pr_watch.lock().unwrap_or_else(|e| e.into_inner())
}

fn decorate_attempt(w: &PrWatch, a: &mut AttemptSummary) {
    a.pr = w.by_attempt.get(&a.id).cloned();
}

/// Fills in `attempt.pr` of a backlog row.
pub(crate) fn apply_summary(st: &AppState, t: &mut TicketSummary) {
    let w = watch(st);
    if let Some(a) = t.attempt.as_mut() {
        decorate_attempt(&w, a);
    }
}

/// Fills in `pr` of a ticket's attempts.
pub(crate) fn apply(st: &AppState, t: &mut Ticket) {
    let w = watch(st);
    if let Some(a) = t.summary.attempt.as_mut() {
        decorate_attempt(&w, a);
    }
    for a in t.attempts.iter_mut() {
        decorate_attempt(&w, a);
    }
}

/// Reads every active attempt's PR (projects on GitHub only) and queues the news. pdtd runs it every 60 s. Never
/// fails; problems are logged.
pub async fn refresh_prs(st: &AppState) {
    let active = match st.store.list_active_attempts().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "the PR check could not list active attempts");
            return;
        }
    };
    let mut repos: HashMap<String, Option<String>> = HashMap::new();
    let mut queue_runs: HashMap<String, Vec<CiRun>> = HashMap::new();
    for info in active {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let repo = match repos.get(&info.project_id) {
            Some(r) => r.clone(),
            None => {
                let r = github_repo(&info.repo_path).await;
                repos.insert(info.project_id.clone(), r.clone());
                r
            }
        };
        let Some(repo) = repo else { continue };
        refresh_one(st, &info, &repo, &mut queue_runs).await;
    }
}

/// Reads one active attempt's PR now (the ticket was opened).
pub(crate) async fn refresh_attempt(st: &AppState, info: &AttemptInfo) {
    if let Some(repo) = github_repo(&info.repo_path).await {
        refresh_one(st, info, &repo, &mut HashMap::new()).await;
    }
}

/// The project's GitHub repository (`owner/name`) from its `origin`, as project responses report it.
async fn github_repo(repo_path: &str) -> Option<String> {
    let path = std::path::PathBuf::from(repo_path);
    tokio::task::spawn_blocking(move || {
        pdt_git::remote_url(&path, "origin").and_then(|u| pdt_core::git_url::github_repo(&u))
    })
    .await
    .ok()
    .flatten()
}

async fn refresh_one(st: &AppState, info: &AttemptInfo, repo: &str, queue_runs: &mut HashMap<String, Vec<CiRun>>) {
    let prev = watch(st).by_attempt.get(&info.attempt.id).cloned();
    let was_queued = prev.as_ref().is_some_and(|p| p.in_merge_queue);
    let cached_queue = queue_runs.get(repo).cloned();
    let known_jobs = watch(st).jobs.clone();
    let (gh, repo_s, branch) = (st.github.clone(), repo.to_string(), info.attempt.branch.clone());
    let read = tokio::task::spawn_blocking(move || {
        read_pr(gh.as_ref(), &repo_s, &branch, was_queued, cached_queue, &known_jobs)
    })
    .await;
    let read = match read {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "the PR check task failed");
            return;
        }
    };
    let (found, queue, jobs) = match read {
        Ok(r) => {
            watch(st).errors.remove(&info.project_id);
            r
        }
        Err(e) => {
            let msg = e.to_string();
            let mut w = watch(st);
            if w.errors.get(&info.project_id) != Some(&msg) {
                tracing::warn!(repo, error = %msg, "could not read pull requests from GitHub");
                w.errors.insert(info.project_id.clone(), msg);
            }
            return;
        }
    };
    if let Some(q) = queue {
        queue_runs.insert(repo.to_string(), q);
    }
    let Some(mut now) = found else {
        watch(st).by_attempt.remove(&info.attempt.id);
        return;
    };
    now.checked_at = now_ms();
    let news = notices(prev.as_ref(), &now, &info.attempt.branch);
    {
        let mut w = watch(st);
        w.jobs.extend(jobs);
        w.by_attempt.insert(info.attempt.id.clone(), now);
    }
    queue_news(st, &info.attempt.id, &info.ticket_id, news).await;
}

type Read = (Option<PullRequestStatus>, Option<Vec<CiRun>>, HashMap<(u64, u32), Vec<String>>);

/// The PR of `branch` with its CI and merge-queue CI, failed jobs filled in (from `known` where already asked).
/// Merge-queue runs are read only while the PR is queued or just left the queue (`was_queued`), once per repo.
fn read_pr(
    gh: &dyn GithubApi,
    repo: &str,
    branch: &str,
    was_queued: bool,
    queue_cache: Option<Vec<CiRun>>,
    known: &HashMap<(u64, u32), Vec<String>>,
) -> Result<Read, GhError> {
    let Some(pr) = gh.pull_request(repo, branch)? else { return Ok((None, None, HashMap::new())) };
    let mut fetched_jobs = HashMap::new();
    let mut fill = |ci: &mut CiStatus| {
        for r in ci.runs.iter_mut().filter(|r| r.state == CiState::Failed) {
            let k = (r.id, r.attempt);
            if let Some(j) = known.get(&k).or_else(|| fetched_jobs.get(&k)) {
                r.failed_jobs = j.clone();
            } else if let Ok(j) = gh.failed_jobs(repo, r.id, r.attempt) {
                fetched_jobs.insert(k, j.clone());
                r.failed_jobs = j;
            }
        }
    };
    let mut ci =
        if pr.state == PrState::Open { CiStatus::of(&gh.branch_runs(repo, branch)?, &pr.head_sha) } else { None };
    if let Some(c) = ci.as_mut() {
        fill(c);
    }
    let mut fresh_queue = None;
    let mut queue_ci = None;
    if pr.state == PrState::Open && (pr.in_merge_queue || was_queued) {
        let runs = match queue_cache {
            Some(r) => r,
            None => {
                let r = gh.queue_runs(repo)?;
                fresh_queue = Some(r.clone());
                r
            }
        };
        let prefix = queue_branch_prefix(&pr.base_branch, pr.number);
        let newest = runs
            .iter()
            .filter(|r| r.head_branch.starts_with(&prefix))
            .max_by(|a, b| (a.created_at.as_str(), a.id).cmp(&(b.created_at.as_str(), b.id)));
        if let Some(sha) = newest.map(|r| r.head_sha.clone()) {
            queue_ci = CiStatus::of(&runs, &sha);
            if let Some(c) = queue_ci.as_mut() {
                fill(c);
            }
        }
    }
    let status = PullRequestStatus {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        state: pr.state,
        draft: pr.draft,
        base_branch: pr.base_branch,
        head_sha: pr.head_sha,
        mergeable: pr.mergeable,
        behind: pr.behind,
        auto_merge: pr.auto_merge,
        in_merge_queue: pr.in_merge_queue,
        queue_position: pr.queue_position,
        ci,
        queue_ci,
        checked_at: 0,
    };
    Ok((Some(status), fresh_queue, fetched_jobs))
}

/// Queues the notices that are news: not told before (on the timeline) and not already waiting.
async fn queue_news(st: &AppState, attempt_id: &str, ticket_id: &str, news: Vec<Notice>) {
    if news.is_empty() {
        return;
    }
    let told = match st.store.pr_notice_keys(ticket_id, attempt_id).await {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(ticket = %ticket_id, error = %e, "could not read the PR notices already sent");
            return;
        }
    };
    let mut w = watch(st);
    for notice in news {
        let waiting = w.pending.iter().any(|p| p.attempt_id == attempt_id && p.notice.key == notice.key);
        let resolved = w.resolved.contains(&(attempt_id.to_string(), notice.key.clone()));
        if !waiting && !resolved && !told.contains(&notice.key) {
            w.pending.push(Pending { attempt_id: attempt_id.into(), ticket_id: ticket_id.into(), notice });
        }
    }
}

/// The attempt's branch was found merged (git or GitHub) and the attempt ended: tell its agent.
pub(crate) async fn notify_merged(st: &AppState, info: &AttemptInfo) {
    let pr = watch(st).by_attempt.get(&info.attempt.id).map(|p| p.number);
    let notice = pdt_core::pr::merged_notice(pr, &info.attempt.branch, &info.attempt.base_branch);
    queue_news(st, &info.attempt.id, &info.ticket_id, vec![notice]).await;
}

/// Delivers waiting notices whose session is ready: all of a session's notices go in one message. A notice whose
/// attempt has no running agent session is recorded as not told. pdtd runs it every few seconds.
pub async fn deliver_notices(st: &AppState) {
    let pending = watch(st).pending.clone();
    if pending.is_empty() {
        return;
    }
    let running = match st.store.list_sessions_by_status(SessionStatus::Running).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not list sessions to deliver PR notices");
            return;
        }
    };
    // The newest running agent session of each attempt (the list is oldest first).
    let mut target: HashMap<&str, &str> = HashMap::new();
    for s in running.iter().filter(|s| s.kind == SessionKind::AgentTui && st.ptys.is_running(&s.id)) {
        if let Some(a) = s.attempt_id.as_deref() {
            target.insert(a, s.id.as_str());
        }
    }
    let mut by_session: HashMap<String, Vec<Pending>> = HashMap::new();
    let mut untold: Vec<Pending> = vec![];
    for p in pending {
        match target.get(p.attempt_id.as_str()) {
            Some(sid) => by_session.entry(sid.to_string()).or_default().push(p),
            None => untold.push(p),
        }
    }
    for p in untold {
        resolve(st, &p, None).await;
    }
    for (sid, list) in by_session {
        if !crate::activity::ready_for_message(st, &sid, QUIET_MS) {
            continue;
        }
        let text = list.iter().map(|p| p.notice.text.as_str()).collect::<Vec<_>>().join("\n\n");
        match crate::activity::type_message(st, &sid, &text).await {
            Ok(()) => {
                tracing::info!(session = %sid, notices = list.len(), "told the agent about its pull request");
                for p in &list {
                    resolve(st, p, Some(&sid)).await;
                }
            }
            Err(e) => tracing::warn!(session = %sid, error = %e, "could not type a PR notice into the session"),
        }
    }
}

/// Records a notice on the timeline (told to `session`, or to nobody) and stops waiting with it.
async fn resolve(st: &AppState, p: &Pending, session: Option<&str>) {
    let n = &p.notice;
    let data = json!({
        "attemptId": p.attempt_id, "notice": n.kind, "key": n.key, "pr": n.pr, "url": n.url, "jobs": n.jobs,
        "text": n.text, "sessionId": session, "told": session.is_some(),
    });
    if let Err(e) =
        st.store.add_ticket_event(&p.ticket_id, EventKind::Pr, Actor::System, None, Some(data), now_ms()).await
    {
        tracing::warn!(ticket = %p.ticket_id, error = %e, "could not record a PR notice");
        return;
    }
    let mut w = watch(st);
    w.resolved.insert((p.attempt_id.clone(), n.key.clone()));
    w.pending.retain(|q| !(q.attempt_id == p.attempt_id && q.notice.key == n.key));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::pr::{CiState, Mergeable, NoticeKind};
    use pdt_github::PullRequest;
    use pdt_github::testing::FakeGithub;

    fn pr(queued: bool) -> PullRequest {
        PullRequest {
            number: 12,
            url: "u".into(),
            title: "t".into(),
            state: PrState::Open,
            draft: false,
            head_sha: "h".into(),
            base_branch: "main".into(),
            mergeable: Mergeable::Mergeable,
            behind: false,
            auto_merge: true,
            in_merge_queue: queued,
            queue_position: queued.then_some(1),
        }
    }

    fn run(id: u64, branch: &str, sha: &str, state: CiState) -> CiRun {
        CiRun {
            id,
            attempt: 1,
            workflow: "ci".into(),
            state,
            url: format!("r{id}"),
            head_sha: sha.into(),
            head_branch: branch.into(),
            created_at: format!("2026-10-18T00:00:{id:02}Z"),
            failed_jobs: vec![],
        }
    }

    #[test]
    fn reads_ci_and_failed_jobs_once() {
        let gh = FakeGithub::default();
        assert_eq!(read_pr(&gh, "o/r", "b", false, None, &HashMap::new()).unwrap().0, None);
        gh.set_pr("b", Some(pr(false)));
        gh.set_runs("b", vec![run(1, "b", "h", CiState::Failed), run(2, "b", "old", CiState::Passed)]);
        gh.set_jobs(1, &["test (linux)"]);
        let (status, queue, jobs) = read_pr(&gh, "o/r", "b", false, None, &HashMap::new()).unwrap();
        let status = status.unwrap();
        let ci = status.ci.unwrap();
        assert_eq!(ci.state, CiState::Failed);
        assert_eq!(ci.runs[0].failed_jobs, vec!["test (linux)".to_string()]);
        assert_eq!(queue, None, "not queued: merge-queue runs are not read");
        assert!(status.queue_ci.is_none());
        // Known jobs are not asked for again.
        gh.calls.lock().unwrap().clear();
        read_pr(&gh, "o/r", "b", false, None, &jobs).unwrap();
        assert_eq!(*gh.calls.lock().unwrap(), vec!["pr o/r b".to_string(), "runs o/r b".to_string()]);
    }

    #[test]
    fn reads_the_newest_merge_queue_run_of_the_pr() {
        let gh = FakeGithub::default();
        gh.set_pr("b", Some(pr(true)));
        *gh.queue.lock().unwrap() = vec![
            run(5, "gh-readonly-queue/main/pr-12-aaa", "q1", CiState::Failed),
            run(6, "gh-readonly-queue/main/pr-12-bbb", "q2", CiState::Running),
            run(7, "gh-readonly-queue/main/pr-120-ccc", "q3", CiState::Failed),
        ];
        let (status, queue, _) = read_pr(&gh, "o/r", "b", false, None, &HashMap::new()).unwrap();
        let q = status.unwrap().queue_ci.unwrap();
        assert_eq!((q.state, q.url.as_str()), (CiState::Running, "r6"));
        assert_eq!(queue.unwrap().len(), 3, "handed back for the repo's other attempts");
        // A repo's runs read for one attempt are reused for the next.
        gh.calls.lock().unwrap().clear();
        let (_, queue, _) = read_pr(&gh, "o/r", "b", false, Some(vec![]), &HashMap::new()).unwrap();
        assert_eq!(queue, None);
        assert!(!gh.calls.lock().unwrap().iter().any(|c| c.starts_with("queue")));
    }

    #[test]
    fn a_merged_pr_has_no_ci_reads() {
        let gh = FakeGithub::default();
        let mut p = pr(false);
        p.state = PrState::Merged;
        gh.set_pr("b", Some(p));
        let (status, ..) = read_pr(&gh, "o/r", "b", true, None, &HashMap::new()).unwrap();
        let status = status.unwrap();
        assert!(status.ci.is_none());
        assert_eq!(notices(None, &status, "b")[0].kind, NoticeKind::Merged);
        assert_eq!(gh.calls.lock().unwrap().len(), 1);
    }
}
