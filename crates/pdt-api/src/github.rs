//! Pull requests and CI of ticket attempts (PDT-25). pdtd reads each active attempt's PR through `gh` every 60 s
//! (and when a ticket is opened), keeps what it saw in memory for ticket responses (`attempt.pr`), and logs what is
//! news as events (Phase 7a): `ci.run_failed`, `ci.run_passed`, `pr.conflict`, `pr.queue_left`, `pr.merged` and
//! `pr.behind`, each once (its key). Workflows react to them; the built-in "PR notices" tells the agent. While an
//! enabled workflow listens for them, it also reads an open PR's comments and reviews (Phase 7e): `pr.commented`,
//! `pr.reviewed`, after a silent first look (the baseline in `host_settings`).

use crate::routes::{AppState, now_ms};
use pdt_core::pr::{
    CiRun, CiState, CiStatus, Notice, NoticeKind, PrState, PullRequestStatus, notices, queue_branch_prefix,
};
use pdt_core::ticket::{AttemptSummary, Ticket, TicketSummary};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{self as ev, AttemptRef, CiEvent, CiRunRef, PrEvent};
use pdt_github::{GhError, GithubApi};
use pdt_store::AttemptInfo;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// How long after an attempt ends pdtd keeps reading a PR it last saw open (PDT-59: git's merge check can end the
/// attempt in the same tick GitHub still reports the PR queued).
const SETTLE_MS: i64 = 60 * 60 * 1000;

#[derive(Default)]
pub(crate) struct PrWatch {
    /// The PR of each attempt, as last seen (ended attempts keep their last look; one last seen open is read again for
    /// an hour after the attempt ended).
    by_attempt: HashMap<String, PullRequestStatus>,
    /// The failed jobs of each run attempt already asked for.
    jobs: HashMap<(u64, u32), Vec<String>>,
    /// The last error per project, so a broken `gh` is logged once rather than every minute.
    errors: HashMap<String, String>,
    /// The login `gh` is authenticated as (comments by it are the user's own).
    viewer: Option<String>,
    /// Buildkite CI of each active attempt's head (Phase 7e), shown with its PR's CI or on its own.
    buildkite: HashMap<String, CiStatus>,
}

/// The newest comment and review ids a PR's feedback was read up to (`sensor.pr_comments.<attempt>`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FeedbackSeen {
    comment: u64,
    review_comment: u64,
    review: u64,
}

fn feedback_key(attempt_id: &str) -> String {
    format!("sensor.pr_comments.{attempt_id}")
}

/// Whether an enabled workflow has a trigger of one of `kinds` (so a sensor has someone to tell).
pub(crate) async fn workflows_listen(st: &AppState, kinds: &[&str]) -> bool {
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        if let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await
            && doc.nodes.iter().any(|n| kinds.contains(&n.kind.as_str()))
        {
            return true;
        }
    }
    false
}

/// The `pr.commented` / `pr.reviewed` events of the feedback newer than `seen`, and how far it was read. Without
/// `seen` (the first look) nothing is news: it only records the baseline.
pub(crate) fn feedback_events(
    info: &AttemptInfo,
    pr: &PullRequestStatus,
    comments: &[pdt_github::PrComment],
    reviews: &[pdt_github::PrReview],
    viewer: Option<&str>,
    seen: Option<FeedbackSeen>,
) -> (Vec<NewLogEvent>, FeedbackSeen) {
    let max = |it: &mut dyn Iterator<Item = u64>, floor: u64| it.max().unwrap_or(0).max(floor);
    let base = seen.unwrap_or_default();
    let now = FeedbackSeen {
        comment: max(&mut comments.iter().filter(|c| !c.in_review).map(|c| c.id), base.comment),
        review_comment: max(&mut comments.iter().filter(|c| c.in_review).map(|c| c.id), base.review_comment),
        review: max(&mut reviews.iter().map(|r| r.id), base.review),
    };
    let Some(seen) = seen else { return (vec![], now) };
    let a = attempt_ref(info);
    let by_owner = |author: &str| viewer.is_some_and(|v| v.eq_ignore_ascii_case(author));
    let branch = &info.attempt.branch;
    let mut out = vec![];
    for c in comments {
        let floor = if c.in_review { seen.review_comment } else { seen.comment };
        if c.id <= floor {
            continue;
        }
        let notice = pdt_core::pr::comment_notice(
            pr.number,
            branch,
            c.id,
            &c.author,
            &c.body,
            c.path.as_deref(),
            c.line,
            &c.url,
        );
        let e = ev::PrCommented {
            attempt: a.clone(),
            pr: pr.number,
            url: c.url.clone(),
            comment_id: c.id,
            author: c.author.clone(),
            body: c.body.clone(),
            path: c.path.clone(),
            line: c.line,
            in_review: c.in_review,
            by_owner: by_owner(&c.author),
            is_bot: c.is_bot,
            notice,
        };
        out.push(NewLogEvent::new(ev::PR_COMMENTED, format!("{}:comment:{}", a.attempt_id, c.id), "pr_poller", e));
    }
    for r in reviews.iter().filter(|r| r.id > seen.review) {
        let notice = pdt_core::pr::review_notice(pr.number, branch, r.id, &r.author, &r.state, &r.body, &r.url);
        let e = ev::PrReviewed {
            attempt: a.clone(),
            pr: pr.number,
            url: r.url.clone(),
            review_id: r.id,
            author: r.author.clone(),
            state: r.state.clone(),
            body: r.body.clone(),
            by_owner: by_owner(&r.author),
            is_bot: r.is_bot,
            notice,
        };
        out.push(NewLogEvent::new(ev::PR_REVIEWED, format!("{}:review:{}", a.attempt_id, r.id), "pr_poller", e));
    }
    (out, now)
}

/// Reads an open PR's comments and reviews and logs what is new.
async fn refresh_feedback(st: &AppState, info: &AttemptInfo, repo: &str, pr: &PullRequestStatus) {
    let key = feedback_key(&info.attempt.id);
    let seen: Option<FeedbackSeen> =
        st.store.get_host_value(&key).await.ok().flatten().and_then(|v| serde_json::from_value(v).ok());
    let viewer = watch(st).viewer.clone();
    let (gh, repo_s, n) = (st.github.clone(), repo.to_string(), pr.number);
    let read = tokio::task::spawn_blocking(move || {
        let viewer = match viewer {
            Some(v) => Some(v),
            None => gh.viewer_login().ok(),
        };
        let comments = gh.pr_comments(&repo_s, n)?;
        let reviews = gh.pr_reviews(&repo_s, n)?;
        Ok::<_, GhError>((viewer, comments, reviews))
    })
    .await;
    let (viewer, comments, reviews) = match read {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return tracing::debug!(repo, error = %e, "could not read a pull request's comments"),
        Err(e) => return tracing::warn!(error = %e, "the PR comments task failed"),
    };
    if viewer.is_some() {
        watch(st).viewer = viewer.clone();
    }
    let (events, now) = feedback_events(info, pr, &comments, &reviews, viewer.as_deref(), seen);
    for e in events {
        if let Err(x) = st.store.emit_event(&e, now_ms()).await {
            tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log a pull request comment");
        }
    }
    if seen != Some(now) {
        let _ = st.store.put_host_value(&key, &serde_json::to_value(now).unwrap_or_default()).await;
    }
}

pub(crate) type SharedPrWatch = Arc<std::sync::Mutex<PrWatch>>;

fn watch(st: &AppState) -> std::sync::MutexGuard<'_, PrWatch> {
    st.pr_watch.lock().unwrap_or_else(|e| e.into_inner())
}

fn decorate_attempt(w: &PrWatch, a: &mut AttemptSummary) {
    a.pr = w.by_attempt.get(&a.id).cloned();
    let bk = w.buildkite.get(&a.id).cloned();
    match a.pr.as_mut().filter(|p| p.state == PrState::Open) {
        // On the PR's head: one badge for both providers.
        Some(pr) if bk.as_ref().is_some_and(|b| b.runs.iter().all(|r| r.head_sha == pr.head_sha)) => {
            pr.ci = CiStatus::merge(pr.ci.take(), bk);
        }
        Some(_) => {}
        None => a.ci = bk,
    }
}

/// Keeps (or with None, drops) an attempt's Buildkite CI.
pub(crate) fn set_buildkite(st: &AppState, attempt_id: &str, ci: Option<CiStatus>) {
    let mut w = watch(st);
    match ci {
        Some(c) => w.buildkite.insert(attempt_id.to_string(), c),
        None => w.buildkite.remove(attempt_id),
    };
}

/// Drops the Buildkite CI of attempts no longer active.
pub(crate) fn keep_buildkite(st: &AppState, active: &HashSet<String>) {
    watch(st).buildkite.retain(|id, _| active.contains(id));
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

/// Fills in the PR of ticket workspaces' tickets.
pub(crate) fn apply_workspaces(st: &AppState, list: &mut [pdt_core::workspace::Workspace]) {
    let w = watch(st);
    for t in list.iter_mut().filter_map(|x| x.ticket.as_mut()) {
        t.pr = w.by_attempt.get(&t.attempt_id).cloned();
    }
}

/// Reads every active attempt's PR (projects on GitHub only) and queues the news, then once more the PR of each
/// attempt that ended in the last hour while its PR was last seen open, so `attempt.pr` catches up with the merge
/// (no news for those: the attempt's end told it). pdtd runs it every 60 s. Never fails; problems are logged.
pub async fn refresh_prs(st: &AppState) {
    let active = match st.store.list_active_attempts().await {
        // A Personal project's tickets have no branch, so no pull request.
        Ok(v) => v.into_iter().filter(|i| !i.attempt.branch.is_empty()).collect::<Vec<_>>(),
        Err(e) => {
            tracing::warn!(error = %e, "the PR check could not list active attempts");
            return;
        }
    };
    let active_ids: HashSet<String> = active.iter().map(|i| i.attempt.id.clone()).collect();
    let open: Vec<String> = watch(st)
        .by_attempt
        .iter()
        .filter(|(id, p)| p.state == PrState::Open && !active_ids.contains(*id))
        .map(|(id, _)| id.clone())
        .collect();
    let mut settling = vec![];
    for id in open {
        match st.store.get_attempt(&id).await {
            Ok(Some(info)) if info.attempt.ended_at.is_some_and(|t| now_ms() - t < SETTLE_MS) => settling.push(info),
            Ok(Some(_)) => {}
            Ok(None) => {
                watch(st).by_attempt.remove(&id);
            }
            Err(e) => tracing::warn!(attempt = %id, error = %e, "the PR check could not read an ended attempt"),
        }
    }
    let mut repos: HashMap<String, Option<String>> = HashMap::new();
    let mut queue_runs: HashMap<String, Vec<CiRun>> = HashMap::new();
    let ended = settling.into_iter().map(|i| (i, false));
    for (info, tell) in active.into_iter().map(|i| (i, true)).chain(ended) {
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
        refresh_one(st, &info, &repo, &mut queue_runs, tell).await;
    }
}

/// Reads one active attempt's PR now (the ticket was opened).
pub(crate) async fn refresh_attempt(st: &AppState, info: &AttemptInfo) {
    if let Some(repo) = github_repo(&info.repo_path).await {
        refresh_one(st, info, &repo, &mut HashMap::new(), true).await;
    }
}

/// The project's GitHub repository (`owner/name`) from its `origin`, as project responses report it.
pub(crate) async fn github_repo(repo_path: &str) -> Option<String> {
    let path = std::path::PathBuf::from(repo_path);
    tokio::task::spawn_blocking(move || {
        pdt_git::remote_url(&path, "origin").and_then(|u| pdt_core::git_url::github_repo(&u))
    })
    .await
    .ok()
    .flatten()
}

/// Reads one attempt's PR into the watch; with `tell`, queues its news.
async fn refresh_one(
    st: &AppState,
    info: &AttemptInfo,
    repo: &str,
    queue_runs: &mut HashMap<String, Vec<CiRun>>,
    tell: bool,
) {
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
    {
        let mut w = watch(st);
        w.jobs.extend(jobs);
        w.by_attempt.insert(info.attempt.id.clone(), now.clone());
    }
    if tell {
        let mut info = info.clone();
        if (now.auto_merge || now.in_merge_queue) && !info.merge_requested {
            info.merge_requested = true;
            if let Err(e) = st.store.set_attempt_merge_requested(&info.attempt.id).await {
                tracing::warn!(ticket = %info.ticket_key, error = %e, "could not record that a PR is set to merge");
            }
        } else if info.merge_requested && turned_off_by_hand(prev.as_ref(), &now) {
            info.merge_requested = false;
            if let Err(e) = st.store.clear_attempt_merge_requested(&info.attempt.id).await {
                tracing::warn!(ticket = %info.ticket_key, error = %e, "could not forget that a PR was set to merge");
            }
        }
        let agent_working = st.store.attempt_has_open_agent_step(&info.attempt.id).await.unwrap_or(false);
        let info = &info;
        for e in news_events(info, &now, agent_working) {
            match st.store.emit_event(&e, now_ms()).await {
                Ok(Some(logged)) => crate::notifications::on_event(st, &logged).await,
                Ok(None) => {}
                Err(x) => tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log a pull request event"),
            }
        }
        if now.state == PrState::Open && workflows_listen(st, &[ev::PR_COMMENTED, ev::PR_REVIEWED]).await {
            refresh_feedback(st, info, repo, &now).await;
        }
    }
}

pub(crate) fn attempt_ref(info: &AttemptInfo) -> AttemptRef {
    AttemptRef {
        project_id: info.project_id.clone(),
        ticket_id: info.ticket_id.clone(),
        ticket_key: info.ticket_key.clone(),
        attempt_id: info.attempt.id.clone(),
        branch: info.attempt.branch.clone(),
        base_branch: info.attempt.base_branch.clone(),
    }
}

pub(crate) fn run_refs(ci: &CiStatus) -> Vec<CiRunRef> {
    ci.runs
        .iter()
        .map(|r| CiRunRef {
            id: r.id,
            attempt: r.attempt,
            workflow: r.workflow.clone(),
            state: r.state,
            url: r.url.clone(),
            failed_jobs: r.failed_jobs.clone(),
        })
        .collect()
}

/// Whether someone turned the open PR's auto-merge off since the last look: it was on (or queued) and now isn't, and
/// the merge queue didn't drop it (a new removal by the queue). CI failing and conflicts leave auto-merge on.
pub(crate) fn turned_off_by_hand(prev: Option<&PullRequestStatus>, now: &PullRequestStatus) -> bool {
    let Some(prev) = prev else { return false };
    let removal = |p: &PullRequestStatus| p.queue_removal.as_ref().map(|r| r.id.clone());
    let dropped_by_queue = now.queue_removal.as_ref().is_some_and(|r| r.by_queue) && removal(now) != removal(prev);
    now.state == PrState::Open
        && (prev.auto_merge || prev.in_merge_queue)
        && !now.auto_merge
        && !now.in_merge_queue
        && !dropped_by_queue
}

/// Whether the attempt's open PR is (or was) set to merge and hasn't merged (PDT-104): PDT recorded it (Merge…,
/// `attempt.merge`, or auto-merge seen on), auto-merge is on, it is queued, or the merge queue itself removed it. A
/// person taking it out of the queue says not to.
pub(crate) fn merge_requested(info: &AttemptInfo, pr: &PullRequestStatus) -> bool {
    if pr.state != PrState::Open || pr.queue_removal.as_ref().is_some_and(|r| !r.by_queue && !pr.in_merge_queue) {
        return false;
    }
    info.merge_requested || pr.auto_merge || pr.in_merge_queue || pr.queue_removal.as_ref().is_some_and(|r| r.by_queue)
}

/// The events a look at the PR gives (each logged once: its key is the attempt and the notice's key).
/// `agent_working`: an agent step works on the attempt now.
pub(crate) fn news_events(info: &AttemptInfo, now: &PullRequestStatus, agent_working: bool) -> Vec<NewLogEvent> {
    let a = attempt_ref(info);
    let key = |k: &str| format!("{}:{k}", a.attempt_id);
    let merge_requested = merge_requested(info, now);
    let pr_event = |n: Notice| PrEvent {
        attempt: a.clone(),
        pr: now.number,
        url: now.url.clone(),
        head_sha: now.head_sha.clone(),
        auto_merge: now.auto_merge,
        in_merge_queue: now.in_merge_queue,
        notice: n,
        merge_requested,
        agent_working,
    };
    let mut out = vec![];
    let mut all = notices(now, &info.attempt.branch);
    all.extend(pdt_core::pr::behind_notice(now, &info.attempt.branch));
    for n in all {
        let k = key(&n.key);
        let e = match n.kind {
            NoticeKind::CiFailed => {
                let ci = now.ci.as_ref();
                NewLogEvent::new(
                    ev::CI_RUN_FAILED,
                    k,
                    "pr_poller",
                    CiEvent {
                        attempt: a.clone(),
                        provider: ev::GITHUB_ACTIONS.into(),
                        pr: Some(now.number),
                        head_sha: now.head_sha.clone(),
                        url: ci.map(|c| c.url.clone()).unwrap_or_default(),
                        runs: ci.map(run_refs).unwrap_or_default(),
                        jobs: n.jobs.clone(),
                        notice: Some(n),
                        auto_merge: now.auto_merge,
                        in_merge_queue: now.in_merge_queue,
                        merge_requested,
                        agent_working,
                    },
                )
            }
            NoticeKind::Conflict => NewLogEvent::new(ev::PR_CONFLICT, k, "pr_poller", pr_event(n)),
            NoticeKind::QueueLeft => NewLogEvent::new(ev::PR_QUEUE_LEFT, k, "pr_poller", pr_event(n)),
            NoticeKind::Merged => NewLogEvent::new(ev::PR_MERGED, k, "pr_poller", pr_event(n)),
            NoticeKind::Behind => NewLogEvent::new(ev::PR_BEHIND, k, "pr_poller", pr_event(n)),
            NoticeKind::Commented | NoticeKind::Reviewed => continue,
        };
        out.push(e);
    }
    if now.state == PrState::Open
        && let Some(ci) = now.ci.as_ref().filter(|c| c.state == CiState::Passed)
    {
        let ids: Vec<String> = ci.runs.iter().map(|r| format!("{}.{}", r.id, r.attempt)).collect();
        out.push(NewLogEvent::new(
            ev::CI_RUN_PASSED,
            key(&format!("ci_passed:{}", ids.join(","))),
            "pr_poller",
            CiEvent {
                attempt: a.clone(),
                provider: ev::GITHUB_ACTIONS.into(),
                pr: Some(now.number),
                head_sha: now.head_sha.clone(),
                url: ci.url.clone(),
                runs: run_refs(ci),
                jobs: vec![],
                notice: None,
                auto_merge: now.auto_merge,
                in_merge_queue: now.in_merge_queue,
                merge_requested,
                agent_working,
            },
        ));
    }
    out
}

type Read = (Option<PullRequestStatus>, Option<Vec<CiRun>>, HashMap<(u64, u32), Vec<String>>);

/// The PR of `branch` with its CI and merge-queue CI, failed jobs filled in (from `known` where already asked).
/// Merge-queue runs are read only while the PR is queued, just left the queue (`was_queued`) or was removed from it
/// for failed checks and not handled yet, once per repo.
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
    let removed_for_checks = pr.queue_removal.as_ref().is_some_and(|r| r.reason == "failed_checks") && !pr.auto_merge;
    if pr.state == PrState::Open && (pr.in_merge_queue || was_queued || removed_for_checks) {
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
        queue_removal: pr.queue_removal,
        checked_at: 0,
    };
    Ok((Some(status), fresh_queue, fetched_jobs))
}

/// The attempt's PR as last seen (None when it has none, or the project is not on GitHub).
pub(crate) fn watched_pr(st: &AppState, attempt_id: &str) -> Option<PullRequestStatus> {
    watch(st).by_attempt.get(attempt_id).cloned()
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
            queue_removal: None,
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
    fn auto_merge_turned_off_by_a_person_is_not_a_queue_drop() {
        let status = |auto: bool, queued: bool, removal: Option<&str>| PullRequestStatus {
            number: 12,
            url: "u".into(),
            title: "t".into(),
            state: PrState::Open,
            draft: false,
            base_branch: "main".into(),
            head_sha: "h".into(),
            mergeable: Mergeable::Mergeable,
            behind: false,
            auto_merge: auto,
            in_merge_queue: queued,
            queue_position: None,
            ci: None,
            queue_ci: None,
            queue_removal: removal.map(|id| pdt_core::pr::QueueRemoval {
                id: id.into(),
                reason: "failed_checks".into(),
                by_queue: true,
            }),
            checked_at: 0,
        };
        let on = status(true, false, None);
        assert!(turned_off_by_hand(Some(&on), &status(false, false, None)));
        assert!(!turned_off_by_hand(Some(&on), &status(true, false, None)), "still on");
        assert!(!turned_off_by_hand(None, &status(false, false, None)), "nothing to compare");
        assert!(!turned_off_by_hand(Some(&status(false, true, None)), &status(false, false, Some("q1"))), "dropped");
        let old_drop = status(false, true, Some("q1"));
        assert!(turned_off_by_hand(Some(&old_drop), &status(false, false, Some("q1"))), "an old drop isn't news");
        assert!(!turned_off_by_hand(Some(&status(false, false, None)), &status(false, false, None)), "was off");
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
        assert_eq!(notices(&status, "b")[0].kind, NoticeKind::Merged);
        assert_eq!(gh.calls.lock().unwrap().len(), 1);
    }
}
