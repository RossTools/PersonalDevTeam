//! Pull requests and CI of ticket attempts (PDT-25). pdtd reads each active attempt's PR from its project's forge
//! (PDT-176: `crate::forge`; GitHub through its API) every 60 s
//! (and when a ticket is opened), keeps what it saw in memory for ticket responses (`attempt.pr`), and logs what is
//! news as events (Phase 7a): `ci.run_failed`, `ci.run_passed`, `pr.conflict`, `pr.queue_left`, `pr.merged` and
//! `pr.behind`, each once (its key). Workflows react to them; the built-in "PR notices" tells the agent. While an
//! enabled workflow listens for them and the forge has reviews, it also reads an open PR's comments and reviews (Phase
//! 7e): `pr.commented`,
//! `pr.reviewed`, after a silent first look (the baseline in `host_settings`), moved on only once every new one is
//! logged. A read that fails keeps the last look, marked with the error (`attempt.pr.error`), and is logged once.
//!
//! CI comes from the project's CI providers (PDT-242, `crate::ci`), not the forge: each is read on the open PR's head
//! (a provider that builds branches, such as Buildkite, on the branch's head before there is a PR), rolled up into one
//! status for the badge, and logged as `ci.run_failed` / `ci.run_passed` per provider (`provider` in the event). A
//! provider whose read fails keeps its last look on that head, and an automated merge refuses until it reads again.

use crate::forge::ProjectForge;
use crate::routes::{AppState, now_ms};
use pdt_core::pr::{
    CiRun, CiSource, CiState, CiStatus, Mergeable, Notice, NoticeKind, PrState, PullRequestStatus, QueueRemovalReason,
    notices,
};
use pdt_core::ticket::{AttemptSummary, Ticket, TicketSummary};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{self as ev, AttemptRef, CiEvent, CiRunRef, PrEvent};
use pdt_forge::{ChangeComment, ChangeRequest, ChangeReview, CiProvider, Forge, ForgeError, MergeSettings};
use pdt_store::AttemptInfo;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// How long after an attempt ends pdtd keeps reading a PR it last saw open (PDT-59: git's merge check can end the
/// attempt in the same tick GitHub still reports the PR queued).
const SETTLE_MS: i64 = 60 * 60 * 1000;

#[derive(Default)]
pub(crate) struct PrWatch {
    /// The PR of each attempt, as last seen (ended attempts keep their last look; one last seen open is read again for
    /// an hour after the attempt ended).
    by_attempt: HashMap<String, PullRequestStatus>,
    /// The failed jobs of each run attempt already asked for, while a watched PR still shows that run.
    jobs: HashMap<JobKey, Vec<String>>,
    /// The last error per read (`pr:<project>`, `issues:<repo>`, `feedback:<attempt>`), so a broken GitHub login is logged
    /// once rather than every minute.
    errors: HashMap<String, String>,
    /// The account PDT is logged in to GitHub as (comments by it are the user's own).
    viewer: Option<String>,
    /// The CI of each active attempt without an open PR, from the providers that build branches (Buildkite):
    /// `attempt.ci`.
    branch_ci: HashMap<String, CiStatus>,
    /// Why each provider's last read of an attempt's CI failed, by attempt and then by provider.
    ci_errors: HashMap<String, BTreeMap<String, String>>,
    /// Each repository's merge settings by forge, repository and base, and when they were read (PDT-297).
    merge_settings: HashMap<(String, String, String), (MergeSettings, i64)>,
    /// The green CI (head and runs) PDT last merged each held merge on (PDT-297), so it tries each once.
    held_tried: HashMap<String, String>,
    /// Attempts whose merge the forge refused to hold although the repository allows auto-merge (checks it doesn't
    /// require, PDT-297): PDT holds them.
    held: HashSet<String>,
}

/// PDT holds the attempt's merge (PDT-297): the forge refused to ([`pdt_forge::ForgeError::NotReady`]).
pub(crate) fn hold(st: &AppState, attempt_id: &str) {
    watch(st).held.insert(attempt_id.to_string());
}

/// How long a repository's merge settings are kept (they change in the repository's settings, rarely).
const MERGE_SETTINGS_MS: i64 = 5 * 60 * 1000;

/// The merge settings of `forge`'s repository into `base` (its merge queue, auto-merge, methods), read once in
/// [`MERGE_SETTINGS_MS`]; None when they can't be read.
pub(crate) async fn merge_settings(st: &AppState, forge: &ProjectForge, base: &str) -> Option<MergeSettings> {
    let key = (forge.kind.to_string(), forge.repo.clone(), base.to_string());
    let now = now_ms();
    if let Some((s, at)) = watch(st).merge_settings.get(&key)
        && now - at < MERGE_SETTINGS_MS
    {
        return Some(*s);
    }
    let (api, repo, b) = (forge.api.clone(), forge.repo.clone(), base.to_string());
    match tokio::task::spawn_blocking(move || api.merge_settings(&repo, &b)).await {
        Ok(Ok(s)) => {
            watch(st).merge_settings.insert(key, (s, now));
            Some(s)
        }
        Ok(Err(e)) => {
            tracing::warn!(repo = %forge.repo, error = %e, "could not read the repository's merge settings");
            None
        }
        Err(_) => None,
    }
}

/// Whether PDT holds the open PR's merge itself (PDT-297): it was set to merge, it is neither on auto-merge nor
/// queued, nor did a merge queue just drop it, and the forge can't hold it (the repository has no auto-merge, or the
/// forge refused to). Elsewhere PR notices sets it to merge again.
async fn merge_held(st: &AppState, info: &AttemptInfo, pr: &PullRequestStatus, forge: Option<&ProjectForge>) -> bool {
    if !info.merge_requested
        || pr.state != PrState::Open
        || pr.auto_merge
        || pr.in_merge_queue
        || pr.queue_removal.is_some()
        || pr.local
    {
        return false;
    }
    if watch(st).held.contains(&info.attempt.id) {
        return true;
    }
    match forge {
        Some(f) => merge_settings(st, f, &pr.base_branch).await.is_some_and(|s| !s.auto_merge),
        None => false,
    }
}

/// The green CI a held merge may merge on now (its head and runs), once each: None when the PR isn't held, CI on its
/// head hasn't passed, or PDT tried on this CI already.
fn held_merge_due(st: &AppState, info: &AttemptInfo, pr: &PullRequestStatus) -> Option<String> {
    if !pr.merge_held || pr.draft || pr.mergeable == Mergeable::Conflicting || pr.local {
        return None;
    }
    let ci = pr.ci.as_ref().filter(|c| c.state == CiState::Passed)?;
    let runs: Vec<String> = ci.runs.iter().map(|r| format!("{}:{}:{}", r.provider, r.id, r.attempt)).collect();
    let key = format!("{}:{}", pr.head_sha, runs.join(","));
    let mut w = watch(st);
    if w.held_tried.get(&info.attempt.id) == Some(&key) {
        return None;
    }
    w.held_tried.insert(info.attempt.id.clone(), key.clone());
    Some(key)
}

/// A run attempt: its provider, id and attempt.
pub(crate) type JobKey = (String, String, u32);

fn job_key(r: &CiRun) -> JobKey {
    (r.provider.clone(), r.id.clone(), r.attempt)
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

/// A forge's (or a CI provider's, `ci:<provider>:<project>`) read failed: logged when that is news (the first failure,
/// or a new reason).
pub(crate) fn gh_failed(st: &AppState, what: &str, error: &str) {
    let mut w = watch(st);
    if w.errors.get(what).map(String::as_str) != Some(error) {
        tracing::warn!(read = what, error, "could not read from the forge or CI");
        w.errors.insert(what.to_string(), error.to_string());
    }
}

/// A GitHub read worked (logged when it had been failing).
pub(crate) fn gh_worked(st: &AppState, what: &str) {
    if watch(st).errors.remove(what).is_some() {
        tracing::info!(read = what, "reading from the forge or CI works again");
    }
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
    comments: &[ChangeComment],
    reviews: &[ChangeReview],
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
            outside: !(c.collaborator || c.is_bot || by_owner(&c.author)),
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
            outside: !(r.collaborator || r.is_bot || by_owner(&r.author)),
            notice,
        };
        out.push(NewLogEvent::new(ev::PR_REVIEWED, format!("{}:review:{}", a.attempt_id, r.id), "pr_poller", e));
    }
    (out, now)
}

/// A comment or review by someone who isn't a collaborator on the repository (PDT-202) goes on the ticket's timeline,
/// with its text: it is shown in PDT, never sent into an agent session.
async fn held_back(st: &AppState, info: &AttemptInfo, payload: &serde_json::Value) {
    use pdt_core::ticket::{Actor, EventKind};
    let s = |k: &str| payload.get(k).and_then(serde_json::Value::as_str).unwrap_or_default();
    let data = serde_json::json!({
        "held": true, "source": s("url"), "url": s("url"), "author": s("author"), "pr": payload.get("pr"),
        "review": payload.get("reviewId").is_some(), "attemptId": info.attempt.id,
    });
    let body = Some(s("body")).filter(|b| !b.trim().is_empty());
    if let Err(e) = st
        .store
        .add_ticket_event(&info.ticket_id, EventKind::OutsideInput, Actor::System, body, Some(data), now_ms())
        .await
    {
        tracing::warn!(ticket = %info.ticket_key, error = %e, "could not put an outsider's comment on the timeline");
    }
}

/// Reads an open PR's comments and reviews and logs what is new.
async fn refresh_feedback(st: &AppState, info: &AttemptInfo, forge: &ProjectForge, pr: &PullRequestStatus) {
    let key = feedback_key(&info.attempt.id);
    let seen: Option<FeedbackSeen> =
        st.store.get_host_value(&key).await.ok().flatten().and_then(|v| serde_json::from_value(v).ok());
    let viewer = watch(st).viewer.clone();
    let (gh, repo_s, n) = (forge.api.clone(), forge.repo.clone(), pr.number);
    // The comments and the reviews are read side by side.
    let read = tokio::task::spawn_blocking(move || {
        std::thread::scope(|s| {
            let reviews = s.spawn(|| gh.change_reviews(&repo_s, n));
            let viewer = match viewer {
                Some(v) => Some(v),
                None => gh.viewer_login().ok(),
            };
            let comments = gh.change_comments(&repo_s, n)?;
            let reviews =
                reviews.join().unwrap_or_else(|_| Err(ForgeError::Failed("the reviews read panicked".into())))?;
            Ok::<_, ForgeError>((viewer, comments, reviews))
        })
    })
    .await;
    let what = format!("feedback:{}", info.attempt.id);
    let (viewer, comments, reviews) = match read {
        Ok(Ok(r)) => {
            gh_worked(st, &what);
            r
        }
        Ok(Err(e)) => return gh_failed(st, &what, &e.to_string()),
        Err(e) => return tracing::warn!(error = %e, "the PR comments task failed"),
    };
    if viewer.is_some() {
        watch(st).viewer = viewer.clone();
    }
    let (events, now) = feedback_events(info, pr, &comments, &reviews, viewer.as_deref(), seen);
    for e in events {
        match st.store.emit_event(&e, now_ms()).await {
            Ok(Some(_)) if ev::by_outsider(&e.payload) => held_back(st, info, &e.payload).await,
            Ok(_) => {}
            // The mark stays: the next look logs what is still missing (keys skip what was logged).
            Err(x) => {
                return tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log a pull request comment");
            }
        }
    }
    if seen != Some(now)
        && let Err(e) = st.store.put_host_value(&key, &serde_json::to_value(now).unwrap_or_default()).await
    {
        tracing::warn!(ticket = %info.ticket_key, error = %e, "could not record how far a PR's comments were read");
    }
}

pub(crate) type SharedPrWatch = Arc<std::sync::Mutex<PrWatch>>;

fn watch(st: &AppState) -> std::sync::MutexGuard<'_, PrWatch> {
    st.pr_watch.lock().unwrap_or_else(|e| e.into_inner())
}

impl PrWatch {
    /// Forgets the failed jobs of runs no watched PR shows any more (a new push, a merged PR), so `jobs` holds only
    /// what the next read can use.
    fn forget_unseen_jobs(&mut self) {
        let shown: HashSet<JobKey> = self
            .by_attempt
            .values()
            .flat_map(|p| p.ci.iter().chain(p.queue_ci.iter()))
            .chain(self.branch_ci.values())
            .flat_map(|c| c.runs.iter().map(job_key))
            .collect();
        self.jobs.retain(|k, _| shown.contains(k));
    }
}

fn decorate_attempt(w: &PrWatch, a: &mut AttemptSummary) {
    a.pr = w.by_attempt.get(&a.id).cloned();
    if a.pr.as_ref().is_none_or(|p| p.state != PrState::Open) {
        a.ci = w.branch_ci.get(&a.id).cloned();
    }
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

/// Reads every active attempt's PR (projects with a forge only) and queues the news, then once more the PR of each
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
    watch(st).forget_unseen_jobs();
    watch(st).errors.retain(|k, _| k.strip_prefix("feedback:").is_none_or(|a| active_ids.contains(a)));
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
    let mut projects: HashMap<String, (Option<ProjectForge>, Providers)> = HashMap::new();
    let mut tick = Tick::new(st).await;
    let ended = settling.into_iter().map(|i| (i, false));
    for (info, tell) in active.into_iter().map(|i| (i, true)).chain(ended) {
        if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let (forge, ci) = match projects.get(&info.project_id) {
            Some(f) => f.clone(),
            None => {
                let f = forge_and_ci(st, &info).await;
                projects.insert(info.project_id.clone(), f.clone());
                f
            }
        };
        if forge.is_none() && ci.is_empty() {
            forget(st, &info.attempt.id);
            continue;
        }
        refresh_one(st, &info, forge.as_ref(), &ci, &mut tick, tell).await;
    }
}

type Providers = Vec<Arc<dyn CiProvider>>;

/// The attempt's project's forge and CI providers.
async fn forge_and_ci(st: &AppState, info: &AttemptInfo) -> (Option<ProjectForge>, Providers) {
    let forge = crate::forge::project_forge(st, &info.project_id, &info.repo_path).await;
    let ci = crate::ci::project_ci(st, &info.project_id, &info.repo_path, forge.as_ref()).await;
    (forge, ci)
}

/// Drops what the watch has of an attempt whose project has neither forge nor CI any more.
fn forget(st: &AppState, attempt_id: &str) {
    let mut w = watch(st);
    w.by_attempt.remove(attempt_id);
    w.branch_ci.remove(attempt_id);
    w.ci_errors.remove(attempt_id);
}

/// What one pass over the attempts shares: each repo's merge-queue runs, the failed jobs known (read once a pass, not
/// once an attempt), and whether a workflow listens for PR comments.
struct Tick {
    queue_runs: HashMap<String, Vec<CiRun>>,
    jobs: Arc<HashMap<JobKey, Vec<String>>>,
    feedback: bool,
}

impl Tick {
    async fn new(st: &AppState) -> Tick {
        let feedback = workflows_listen(st, &[ev::PR_COMMENTED, ev::PR_REVIEWED]).await;
        let jobs = Arc::new(watch(st).jobs.clone());
        Tick { queue_runs: HashMap::new(), jobs, feedback }
    }
}

/// Reads one active attempt's PR now (the ticket was opened). False when the forge couldn't be read (the watch keeps
/// the last look); true too for a project on no forge.
pub(crate) async fn refresh_attempt(st: &AppState, info: &AttemptInfo) -> bool {
    let (forge, ci) = forge_and_ci(st, info).await;
    if forge.is_none() && ci.is_empty() {
        return true;
    }
    refresh_one(st, info, forge.as_ref(), &ci, &mut Tick::new(st).await, true).await
}

/// The commit a branch's CI builds before it has a pull request: the pushed branch's head, else the local one.
async fn branch_head(info: &AttemptInfo) -> Option<String> {
    let repo = std::path::PathBuf::from(&info.repo_path);
    let branch = info.attempt.branch.clone();
    tokio::task::spawn_blocking(move || {
        pdt_git::rev_parse(&repo, &format!("refs/remotes/origin/{branch}"))
            .or_else(|| pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")))
    })
    .await
    .ok()
    .flatten()
}

/// The project's GitHub repository (`owner/name`) from its `origin`, as project responses report it.
pub(crate) async fn github_repo(repo_path: &str) -> Option<String> {
    let path = std::path::PathBuf::from(repo_path);
    tokio::task::spawn_blocking(move || pdt_core::git_url::github_repo(&pdt_git::remote_url(&path, "origin")?))
        .await
        .ok()
        .flatten()
}

/// Reads one attempt's PR and CI into the watch; with `tell`, queues their news. False when the read failed.
async fn refresh_one(
    st: &AppState,
    info: &AttemptInfo,
    forge: Option<&ProjectForge>,
    ci: &[Arc<dyn CiProvider>],
    tick: &mut Tick,
    tell: bool,
) -> bool {
    let repo = forge.map(|f| f.repo.clone()).unwrap_or_default();
    let id = info.attempt.id.clone();
    let (prev, prev_branch_ci) = {
        let w = watch(st);
        (w.by_attempt.get(&id).cloned(), w.branch_ci.get(&id).cloned())
    };
    let branch_head = match ci.iter().any(|p| p.capabilities().branches) {
        true => branch_head(info).await,
        false => None,
    };
    let local = forge.is_some_and(|f| f.kind == pdt_core::forge::ForgeKind::LOCAL);
    let args = ReadArgs {
        forge: forge.map(|f| f.api.clone()),
        ci: ci.to_vec(),
        repo: repo.clone(),
        branch: info.attempt.branch.clone(),
        since: info.attempt.created_at,
        was_queued: prev.as_ref().is_some_and(|p| p.in_merge_queue),
        queue_cache: tick.queue_runs.get(&repo).cloned(),
        known: tick.jobs.clone(),
        branch_head,
        last_pr_head: prev.as_ref().filter(|p| p.state == PrState::Open).map(|p| p.head_sha.clone()),
    };
    let read = tokio::task::spawn_blocking(move || read_attempt(&args)).await;
    let read = match read {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "the PR check task failed");
            return false;
        }
    };
    let what = format!("pr:{}", info.project_id);
    let mut read = match read {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            gh_failed(st, &what, &msg);
            // The badge keeps the last look, saying it is stale and why.
            if let Some(p) = watch(st).by_attempt.get_mut(&id) {
                p.error = Some(msg);
            }
            return false;
        }
    };
    // The forge couldn't be read: the PR keeps its last look, marked with why, but the providers read without it (the
    // ones that build branches) still report.
    let forge_error = read.forge_error.take();
    match &forge_error {
        Some(msg) => gh_failed(st, &what, msg),
        None => gh_worked(st, &what),
    }
    let stale_pr = forge_error.is_some() && args_pr_open(prev.as_ref());
    if let Some(q) = read.queue.take() {
        tick.queue_runs.insert(repo.clone(), q);
    }
    if !read.jobs.is_empty() {
        Arc::make_mut(&mut tick.jobs).extend(read.jobs.clone());
    }
    // Each provider's part on the head; one that couldn't be read keeps its last look there.
    let prev_ci = match prev.as_ref().filter(|p| p.state == PrState::Open) {
        Some(p) => p.ci.clone(),
        None => prev_branch_ci,
    };
    let head = read.head.clone();
    let on_head = |c: &CiStatus| head.as_ref().is_some_and(|h| c.runs.iter().all(|r| &r.head_sha == h));
    let mut parts: Vec<CiStatus> = vec![];
    let mut errors = BTreeMap::new();
    for (provider, got) in read.ci {
        let key = format!("ci:{provider}:{}", info.project_id);
        match got {
            Ok(c) => {
                gh_worked(st, &key);
                parts.extend(c);
            }
            Err(e) => {
                gh_failed(st, &key, &e);
                parts.extend(prev_ci.as_ref().and_then(|c| c.of_provider(&provider)).filter(on_head));
                errors.insert(provider, e);
            }
        }
    }
    let all = CiStatus::combine(parts.clone());
    let found = read.pr.map(|p| PullRequestStatus { local, ..p });
    {
        let mut w = watch(st);
        w.jobs.extend(read.jobs);
        if errors.is_empty() {
            w.ci_errors.remove(&id);
        } else {
            w.ci_errors.insert(id.clone(), errors);
        }
        match (&found, &all) {
            (None, Some(c)) if !stale_pr => w.branch_ci.insert(id.clone(), c.clone()),
            _ => w.branch_ci.remove(&id),
        };
    }
    let sources: HashMap<&str, CiSource> = ci.iter().map(|p| (p.id(), crate::ci::source(p.as_ref()))).collect();
    if let Some(msg) = forge_error {
        let (agent_working, merge_waiting) = busy(st, &id).await;
        let mut pr = None;
        if stale_pr && let Some(p) = watch(st).by_attempt.get_mut(&id) {
            p.error = Some(msg);
            // The providers read now replace their part of the last look; the others' stays.
            let read_now: Vec<&str> = parts.iter().map(|c| c.runs[0].provider.as_str()).collect();
            let kept =
                p.ci.iter().flat_map(|c| c.by_provider()).filter(|c| !read_now.contains(&c.runs[0].provider.as_str()));
            p.ci = CiStatus::combine(kept.chain(parts.iter().cloned()).collect::<Vec<_>>());
            pr = Some(p.clone());
        }
        if tell && let Some(h) = &head {
            let events = parts.iter().flat_map(|c| {
                let s = sources.get(c.runs[0].provider.as_str());
                s.map(|s| ci_events(info, s, c, pr.as_ref(), h, agent_working, merge_waiting)).unwrap_or_default()
            });
            emit(st, info, events.collect()).await;
        }
        return false;
    }
    let Some(mut now) = found else {
        watch(st).by_attempt.remove(&id);
        if tell && let Some(h) = &head {
            let (agent_working, merge_waiting) = busy(st, &id).await;
            let events = parts.iter().flat_map(|c| {
                let s = sources.get(c.runs[0].provider.as_str());
                s.map(|s| ci_events(info, s, c, None, h, agent_working, merge_waiting)).unwrap_or_default()
            });
            emit(st, info, events.collect()).await;
        }
        return true;
    };
    if now.state == PrState::Open {
        now.ci = all;
    }
    now.checked_at = now_ms();
    now.merge_held = merge_held(st, info, &now, forge).await;
    watch(st).by_attempt.insert(id.clone(), now.clone());
    if tell {
        let mut info = info.clone();
        if (now.auto_merge || now.in_merge_queue) && !info.merge_requested {
            info.merge_requested = true;
            info.merge_refused = false;
            if let Err(e) = st.store.set_attempt_merge_requested(&info.attempt.id).await {
                tracing::warn!(ticket = %info.ticket_key, error = %e, "could not record that a PR is set to merge");
            }
        } else if (info.merge_requested || info.merge_refused) && turned_off_by_hand(prev.as_ref(), &now) {
            // Off by hand stays off (PDT-104): a refused merge no longer counts either (PDT-139).
            info.merge_requested = false;
            info.merge_refused = false;
            if let Err(e) = st.store.clear_attempt_merge_requested(&info.attempt.id).await {
                tracing::warn!(ticket = %info.ticket_key, error = %e, "could not forget that a PR is set to merge");
            }
        }
        let (agent_working, merge_waiting) = busy(st, &id).await;
        let info = &info;
        let mut events = news_events(info, &now, agent_working);
        if now.state == PrState::Open {
            for c in &parts {
                if let Some(s) = sources.get(c.runs[0].provider.as_str()) {
                    events.extend(ci_events(info, s, c, Some(&now), &now.head_sha, agent_working, merge_waiting));
                }
            }
        }
        emit(st, info, events).await;
        // PDT's own auto-merge (PDT-297): CI passed on a merge the forge couldn't hold.
        now.merge_held = merge_held(st, info, &now, forge).await;
        if let Some(p) = watch(st).by_attempt.get_mut(&id) {
            p.merge_held = now.merge_held;
        }
        if held_merge_due(st, info, &now).is_some() {
            tokio::spawn(crate::review::merge_held(st.clone(), info.attempt.id.clone()));
        }
        if now.state == PrState::Open
            && tick.feedback
            && let Some(f) = forge.filter(|f| f.caps().reviews)
        {
            refresh_feedback(st, info, f, &now).await;
        }
    }
    true
}

/// Whether the last look saw an open PR.
fn args_pr_open(prev: Option<&PullRequestStatus>) -> bool {
    prev.is_some_and(|p| p.state == PrState::Open)
}

/// Whether an agent step works on the attempt now, and whether a workflow's merge of it waits for its CI.
async fn busy(st: &AppState, attempt_id: &str) -> (bool, bool) {
    let agent_working = st.store.attempt_has_open_agent_step(attempt_id).await.unwrap_or(false);
    let merge_waiting = st.store.attempt_merge_waits_for_ci(attempt_id).await.unwrap_or(false);
    (agent_working, merge_waiting)
}

/// Logs `events` (each once: its key), telling the notifications of each that is new.
async fn emit(st: &AppState, info: &AttemptInfo, events: Vec<NewLogEvent>) {
    for e in events {
        match st.store.emit_event(&e, now_ms()).await {
            Ok(Some(logged)) => crate::notifications::on_event(st, &logged).await,
            Ok(None) => {}
            Err(x) => tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log a pull request event"),
        }
    }
}

pub fn attempt_ref(info: &AttemptInfo) -> AttemptRef {
    AttemptRef {
        project_id: info.project_id.clone(),
        ticket_id: info.ticket_id.clone(),
        ticket_key: info.ticket_key.clone(),
        attempt_id: info.attempt.id.clone(),
        branch: info.attempt.branch.clone(),
        base_branch: info.attempt.base_branch.clone(),
    }
}

pub fn run_refs(ci: &CiStatus) -> Vec<CiRunRef> {
    ci.runs
        .iter()
        .map(|r| CiRunRef {
            provider: r.provider.clone(),
            id: r.id.clone(),
            attempt: r.attempt,
            name: r.name.clone(),
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
pub fn merge_requested(info: &AttemptInfo, pr: &PullRequestStatus) -> bool {
    if pr.state != PrState::Open || pr.queue_removal.as_ref().is_some_and(|r| !r.by_queue && !pr.in_merge_queue) {
        return false;
    }
    info.merge_requested || pr.auto_merge || pr.in_merge_queue || pr.queue_removal.as_ref().is_some_and(|r| r.by_queue)
}

/// Whether a workflow's merge of the attempt was refused for something its agent can fix and the PR is still open
/// (PDT-139).
pub fn merge_refused(info: &AttemptInfo, pr: &PullRequestStatus) -> bool {
    info.merge_refused && pr.state == PrState::Open
}

/// The events a look at the PR gives, CI aside ([`ci_events`]): each logged once (its key is the attempt and the
/// notice's key). `agent_working`: an agent step works on the attempt now.
pub(crate) fn news_events(info: &AttemptInfo, now: &PullRequestStatus, agent_working: bool) -> Vec<NewLogEvent> {
    let a = attempt_ref(info);
    let key = |k: &str| format!("{}:{k}", a.attempt_id);
    let merge_requested = merge_requested(info, now);
    let merge_refused = merge_refused(info, now);
    let pr_event = |n: Notice| PrEvent {
        attempt: a.clone(),
        pr: now.number,
        url: now.url.clone(),
        head_sha: now.head_sha.clone(),
        auto_merge: now.auto_merge,
        in_merge_queue: now.in_merge_queue,
        notice: n,
        merge_requested,
        merge_refused,
        agent_working,
    };
    let mut out = vec![];
    let mut all = notices(now, &info.attempt.branch);
    all.extend(pdt_core::pr::behind_notice(now, &info.attempt.branch));
    for n in all {
        let k = key(&n.key);
        let e = match n.kind {
            NoticeKind::Conflict => NewLogEvent::new(ev::PR_CONFLICT, k, "pr_poller", pr_event(n)),
            NoticeKind::QueueLeft => NewLogEvent::new(ev::PR_QUEUE_LEFT, k, "pr_poller", pr_event(n)),
            NoticeKind::Merged => NewLogEvent::new(ev::PR_MERGED, k, "pr_poller", pr_event(n)),
            NoticeKind::Behind => NewLogEvent::new(ev::PR_BEHIND, k, "pr_poller", pr_event(n)),
            NoticeKind::CiFailed | NoticeKind::Commented | NoticeKind::Reviewed => continue,
        };
        out.push(e);
    }
    out
}

/// The `ci.*` events of one provider's CI (`ci`, its runs only) on `head`, each once (its key names the runs and their
/// attempts). `pr`: the branch's open pull request, if it has one; `merge_waiting`: a workflow's merge waits for its CI.
pub(crate) fn ci_events(
    info: &AttemptInfo,
    source: &CiSource,
    ci: &CiStatus,
    pr: Option<&PullRequestStatus>,
    head: &str,
    agent_working: bool,
    merge_waiting: bool,
) -> Vec<NewLogEvent> {
    let a = attempt_ref(info);
    let merge_requested = pr.is_some_and(|p| merge_requested(info, p));
    let merge_refused = pr.is_some_and(|p| merge_refused(info, p));
    let event = |notice: Option<Notice>| CiEvent {
        attempt: a.clone(),
        provider: source.id.clone(),
        pr: pr.map(|p| p.number),
        head_sha: head.to_string(),
        url: ci.url.clone(),
        runs: run_refs(ci),
        jobs: notice.as_ref().map(|n| n.jobs.clone()).unwrap_or_default(),
        notice,
        auto_merge: pr.is_some_and(|p| p.auto_merge),
        in_merge_queue: pr.is_some_and(|p| p.in_merge_queue),
        merge_requested,
        merge_refused,
        agent_working,
        merge_waiting,
    };
    let key = |k: &str| format!("{}:{k}", a.attempt_id);
    let mut out = vec![];
    if let Some(n) = pdt_core::pr::ci_failed_notice(source, ci, pr, &info.attempt.branch, head) {
        out.push(NewLogEvent::new(ev::CI_RUN_FAILED, key(&n.key), "pr_poller", event(Some(n))));
    }
    if ci.state == CiState::Passed {
        let k = key(&pdt_core::pr::ci_passed_key(source, ci));
        out.push(NewLogEvent::new(ev::CI_RUN_PASSED, k, "pr_poller", event(None)));
    }
    out
}

/// What [`read_attempt`] reads with.
pub(crate) struct ReadArgs {
    pub forge: Option<Arc<dyn Forge>>,
    pub ci: Vec<Arc<dyn CiProvider>>,
    pub repo: String,
    pub branch: String,
    /// When the attempt started (ms).
    pub since: i64,
    /// The PR was in the merge queue at the last look.
    pub was_queued: bool,
    /// The repo's merge-queue runs, when this pass read them already.
    pub queue_cache: Option<Vec<CiRun>>,
    /// The failed jobs already known.
    pub known: Arc<HashMap<JobKey, Vec<String>>>,
    /// The branch's head without a PR, for providers that build branches.
    pub branch_head: Option<String>,
    /// The head of the open PR the last look saw: what those providers are read on when the forge can't be read.
    pub last_pr_head: Option<String>,
}

/// What one look at an attempt found.
pub(crate) struct Read {
    /// Its PR, without its CI (`ci` is that).
    pub pr: Option<PullRequestStatus>,
    /// The commit CI was read on: the open PR's head, else (without a PR) the branch's.
    pub head: Option<String>,
    /// Each provider's CI on `head` (None: no runs there), or why it couldn't be read.
    pub ci: Vec<(String, Result<Option<CiStatus>, String>)>,
    /// Merge-queue runs read now.
    pub queue: Option<Vec<CiRun>>,
    /// Failed jobs asked for now.
    pub jobs: HashMap<JobKey, Vec<String>>,
    /// Why the forge couldn't be read: then `pr` is None and only the providers that build branches were read (on the
    /// last PR's head, else the branch's).
    pub forge_error: Option<String>,
}

impl Read {
    /// Its PR with the CI read on its head, every provider's together.
    pub fn pr_with_ci(self) -> Option<PullRequestStatus> {
        let ci = CiStatus::combine(self.ci.into_iter().filter_map(|(_, c)| c.ok().flatten()));
        self.pr.map(|p| PullRequestStatus { ci: if p.state == PrState::Open { ci } else { None }, ..p })
    }
}

/// Fills in the failed jobs of `ci`'s failed runs (from `known` or `fetched` where already asked).
fn fill_jobs(
    p: &dyn CiProvider,
    repo: &str,
    ci: &mut CiStatus,
    known: &HashMap<JobKey, Vec<String>>,
    fetched: &mut HashMap<JobKey, Vec<String>>,
) {
    for r in ci.runs.iter_mut().filter(|r| r.state == CiState::Failed && r.failed_jobs.is_empty()) {
        let k = job_key(r);
        if let Some(j) = known.get(&k).or_else(|| fetched.get(&k)) {
            r.failed_jobs = j.clone();
        } else if let Ok(j) = pdt_forge::ci::failed_job_names(p, repo, r) {
            fetched.insert(k, j.clone());
            r.failed_jobs = j;
        }
    }
}

/// The PR of the branch for the attempt started at `since` (ms), and each CI provider's runs on its head (on the
/// branch's head without a PR, from providers that build branches), failed jobs filled in. A closed or merged PR
/// opened before the attempt started is an earlier attempt's on the same branch name, not this one's; an open one is
/// the branch's, whoever opened it. Merge-queue runs are read only while the PR is queued, just left the queue
/// (`was_queued`) or was removed from it for failed checks and not handled yet, once per repo, and only from a forge
/// with merge queues and a provider that runs them. A provider's failed read is its own (`ci`); so is the forge's
/// (`forge_error`), which leaves the providers that build branches to read. Only a failed merge-queue read fails it.
pub(crate) fn read_attempt(a: &ReadArgs) -> Result<Read, ForgeError> {
    let repo = a.repo.as_str();
    let pr = match &a.forge {
        Some(f) => match f.change_request(repo, &a.branch) {
            Ok(pr) => pr,
            Err(e) => return Ok(read_without_forge(a, e.to_string())),
        },
        None => None,
    };
    // The forge gives the newest open PR, else the newest: when that one is too old, every other is too.
    let pr = pr.filter(|p| p.state == PrState::Open || p.created_at.is_none_or(|c| c >= a.since));
    let head = match &pr {
        Some(p) if p.state == PrState::Open => Some(p.head_sha.clone()),
        Some(_) => None,
        None => a.branch_head.clone(),
    };
    let mut fetched = HashMap::new();
    let mut ci = vec![];
    if let Some(h) = &head {
        for p in a.ci.iter().filter(|p| pr.is_some() || p.capabilities().branches) {
            let got = p.runs(repo, &a.branch, Some(h)).map(|runs| {
                let mut c = CiStatus::of(&runs, h);
                if let Some(c) = c.as_mut() {
                    fill_jobs(p.as_ref(), repo, c, &a.known, &mut fetched);
                }
                c
            });
            ci.push((p.id().to_string(), got.map_err(|e| e.to_string())));
        }
    }
    let mut fresh_queue = None;
    let mut queue_ci = None;
    let queue_ref = |pr: &ChangeRequest| {
        let f = a.forge.as_ref().filter(|f| f.capabilities().merge_queue)?;
        f.queue_ref(&pr.base_branch, pr.number)
    };
    let queue_provider = a.ci.iter().find(|p| p.capabilities().merge_queue);
    if let (Some((pr, prefix)), Some(q)) = (pr.as_ref().and_then(|p| Some((p, queue_ref(p)?))), queue_provider) {
        let removed_for_checks =
            pr.queue_removal.as_ref().is_some_and(|r| r.reason == QueueRemovalReason::FailedChecks) && !pr.auto_merge;
        if pr.state == PrState::Open && (pr.in_merge_queue || a.was_queued || removed_for_checks) {
            let runs = match a.queue_cache.clone() {
                Some(r) => r,
                None => {
                    let r = q.queue_runs(repo)?;
                    fresh_queue = Some(r.clone());
                    r
                }
            };
            let newest = runs.iter().filter(|r| r.branch.starts_with(&prefix)).max_by(|a, b| {
                (a.created_at.as_str(), a.id.len(), &a.id).cmp(&(b.created_at.as_str(), b.id.len(), &b.id))
            });
            if let Some(sha) = newest.map(|r| r.head_sha.clone()) {
                queue_ci = CiStatus::of(&runs, &sha);
                if let Some(c) = queue_ci.as_mut() {
                    fill_jobs(q.as_ref(), repo, c, &a.known, &mut fetched);
                }
            }
        }
    }
    let status = pr.map(|pr| PullRequestStatus {
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
        ci: None,
        queue_ci,
        queue_removal: pr.queue_removal,
        checked_at: 0,
        error: None,
        local: false,
        merge_held: false,
    });
    Ok(Read { pr: status, head, ci, queue: fresh_queue, jobs: fetched, forge_error: None })
}

/// What can be read when the forge can't: the providers that build branches, on the last PR's head, else the
/// branch's.
fn read_without_forge(a: &ReadArgs, error: String) -> Read {
    let head = a.last_pr_head.clone().or_else(|| a.branch_head.clone());
    let mut fetched = HashMap::new();
    let mut ci = vec![];
    if let Some(h) = &head {
        for p in a.ci.iter().filter(|p| p.capabilities().branches) {
            let got = p.runs(&a.repo, &a.branch, Some(h)).map(|runs| {
                let mut c = CiStatus::of(&runs, h);
                if let Some(c) = c.as_mut() {
                    fill_jobs(p.as_ref(), &a.repo, c, &a.known, &mut fetched);
                }
                c
            });
            ci.push((p.id().to_string(), got.map_err(|e| e.to_string())));
        }
    }
    Read { pr: None, head, ci, queue: None, jobs: fetched, forge_error: Some(error) }
}

/// Logs `attempt.late_commits` (once per branch head, PDT-151): the attempt's PR merged, and `commits` (newest first, as
/// (sha, subject)) were pushed to its branch, now at `head`, after that.
pub(crate) async fn log_late_commits(
    st: &AppState,
    info: &AttemptInfo,
    pr: &PullRequestStatus,
    head: &str,
    commits: Vec<(String, String)>,
) {
    let commits: Vec<pdt_core::pr::LateCommit> =
        commits.into_iter().map(|(sha, subject)| pdt_core::pr::LateCommit { sha, subject }).collect();
    let (summary, notice) = pdt_core::pr::late_commits_notice(
        &info.ticket_key,
        pr.number,
        &info.attempt.branch,
        &info.attempt.base_branch,
        &commits,
    );
    let list = commits.iter().map(|c| format!("- {} {}", c.sha, c.subject).trim_end().to_string()).collect::<Vec<_>>();
    let e = ev::LateCommitsEvent {
        attempt: attempt_ref(info),
        pr: pr.number,
        url: pr.url.clone(),
        merged_head: pr.head_sha.clone(),
        head: head.to_string(),
        commits,
        summary,
        notice,
        list: list.join("\n"),
    };
    let key = format!("{}:late_commits:{head}", info.attempt.id);
    match st.store.emit_event(&NewLogEvent::new(ev::ATTEMPT_LATE_COMMITS, key, "merge_check", e), now_ms()).await {
        Ok(Some(logged)) => {
            tracing::info!(ticket = %info.ticket_key, pr = pr.number, "commits were pushed after the PR merged");
            crate::notifications::on_event(st, &logged).await;
        }
        Ok(None) => {}
        Err(x) => tracing::warn!(ticket = %info.ticket_key, error = %x, "could not log commits pushed after a merge"),
    }
}

/// The attempt's open PR as last seen, with every provider's CI on its head together (as its badge shows it), and why
/// a provider's last read of it failed, by provider.
pub(crate) fn head_ci(st: &AppState, attempt_id: &str) -> Option<(PullRequestStatus, BTreeMap<String, String>)> {
    let w = watch(st);
    let pr = w.by_attempt.get(attempt_id).filter(|p| p.state == PrState::Open).cloned()?;
    Some((pr, w.ci_errors.get(attempt_id).cloned().unwrap_or_default()))
}

/// The attempt's PR as last seen (None when it has none, or the project has no forge).
pub fn watched_pr(st: &AppState, attempt_id: &str) -> Option<PullRequestStatus> {
    watch(st).by_attempt.get(attempt_id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::pr::{CiState, Mergeable, NoticeKind};
    use pdt_forge::ChangeRequest as PullRequest;
    use pdt_forge::testing::{FakeCi, FakeForge};
    use pdt_forge::{CiCapabilities, CiConfig};

    /// What the poller reads of branch `b` on `gh` (as GitHub, with its fake Actions) and `more` providers.
    fn read(
        gh: &Arc<FakeForge>,
        more: &[Arc<dyn CiProvider>],
        since: i64,
        was_queued: bool,
        queue_cache: Option<Vec<CiRun>>,
        known: &HashMap<JobKey, Vec<String>>,
        branch_head: Option<&str>,
    ) -> Read {
        let mut ci: Vec<Arc<dyn CiProvider>> = vec![gh.ci.clone()];
        ci.extend(more.iter().cloned());
        read_attempt(&ReadArgs {
            forge: Some(gh.clone()),
            ci,
            repo: "o/r".into(),
            branch: "b".into(),
            since,
            was_queued,
            queue_cache,
            known: Arc::new(known.clone()),
            branch_head: branch_head.map(String::from),
            last_pr_head: None,
        })
        .unwrap()
    }

    fn read_pr(gh: &Arc<FakeForge>, since: i64, was_queued: bool, queue_cache: Option<Vec<CiRun>>) -> Read {
        read(gh, &[], since, was_queued, queue_cache, &HashMap::new(), None)
    }

    fn buildkite() -> Arc<FakeCi> {
        let caps = CiCapabilities { branches: true, logs: true, ..CiCapabilities::NONE };
        Arc::new(FakeCi::new("buildkite", "Buildkite", caps, CiConfig::Known(true)))
    }

    fn calls(gh: &FakeForge) -> Vec<String> {
        let mut all = gh.calls.lock().unwrap().clone();
        all.extend(gh.ci.calls.lock().unwrap().iter().cloned());
        all
    }

    fn clear(gh: &FakeForge) {
        gh.calls.lock().unwrap().clear();
        gh.ci.calls.lock().unwrap().clear();
    }

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
            created_at: None,
        }
    }

    fn run(id: u64, branch: &str, sha: &str, state: CiState) -> CiRun {
        CiRun {
            provider: pdt_core::pr::GITHUB_ACTIONS.into(),
            id: id.to_string(),
            attempt: 1,
            name: "ci".into(),
            state,
            url: format!("r{id}"),
            head_sha: sha.into(),
            branch: branch.into(),
            created_at: format!("2026-10-18T00:00:{id:02}Z"),
            failed_jobs: vec![],
        }
    }

    #[test]
    fn only_the_jobs_of_runs_a_watched_pr_shows_are_kept() {
        let mut w = PrWatch::default();
        let mut failed = run(1, "b", "h", CiState::Failed);
        failed.failed_jobs = vec!["test".into()];
        let status = |runs: Vec<CiRun>| PullRequestStatus {
            number: 1,
            url: String::new(),
            title: String::new(),
            state: PrState::Open,
            draft: false,
            base_branch: "main".into(),
            head_sha: "h".into(),
            mergeable: Mergeable::Mergeable,
            behind: false,
            auto_merge: false,
            in_merge_queue: false,
            queue_position: None,
            ci: CiStatus::of(&runs, "h"),
            queue_ci: None,
            queue_removal: None,
            checked_at: 0,
            error: None,
            local: false,
            merge_held: false,
        };
        w.by_attempt.insert("a".into(), status(vec![failed.clone()]));
        let k = |id: &str| ("github_actions".to_string(), id.to_string(), 1);
        w.jobs.insert(k("1"), vec!["test".into()]);
        w.jobs.insert(k("7"), vec!["old push".into()]);
        w.forget_unseen_jobs();
        assert_eq!(w.jobs.keys().collect::<Vec<_>>(), [&k("1")]);
        w.by_attempt.insert("a".into(), status(vec![run(2, "b", "h", CiState::Passed)]));
        w.forget_unseen_jobs();
        assert!(w.jobs.is_empty(), "the next push's run passed");
    }

    #[test]
    fn reads_ci_and_failed_jobs_once() {
        let gh = Arc::new(FakeForge::default());
        assert!(read_pr(&gh, 0, false, None).pr.is_none());
        gh.set_pr("b", Some(pr(false)));
        gh.set_runs("b", vec![run(1, "b", "h", CiState::Failed), run(2, "b", "old", CiState::Passed)]);
        gh.set_jobs(1, &["test (linux)"]);
        let r = read_pr(&gh, 0, false, None);
        let (queue, jobs) = (r.queue.clone(), r.jobs.clone());
        assert_eq!(r.head.as_deref(), Some("h"));
        let status = r.pr_with_ci().unwrap();
        let ci = status.ci.unwrap();
        assert_eq!(ci.state, CiState::Failed);
        assert_eq!(ci.runs[0].failed_jobs, vec!["test (linux)".to_string()]);
        assert_eq!(queue, None, "not queued: merge-queue runs are not read");
        assert!(status.queue_ci.is_none());
        // Known jobs are not asked for again.
        clear(&gh);
        read(&gh, &[], 0, false, None, &jobs, None);
        assert_eq!(calls(&gh), vec!["pr o/r b".to_string(), "runs o/r b".to_string()]);
    }

    /// PDT-242: two providers' runs on the head roll up into the PR's CI; one that can't be read is its own error.
    #[test]
    fn every_providers_ci_on_the_head_is_read_and_rolled_up() {
        let gh = Arc::new(FakeForge::default());
        let bk = buildkite();
        gh.set_pr("b", Some(pr(false)));
        gh.set_runs("b", vec![run(1, "b", "h", CiState::Passed)]);
        let mut build = bk.run("app/3", "app", "b", "h", CiState::Failed, "t");
        build.failed_jobs = vec!["test".into()];
        bk.set_runs("b", vec![build, bk.run("app/2", "app", "b", "old", CiState::Passed, "t")]);
        let more: Vec<Arc<dyn CiProvider>> = vec![bk.clone()];
        let r = read(&gh, &more, 0, false, None, &HashMap::new(), None);
        assert_eq!(r.ci.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["github_actions", "buildkite"]);
        assert!(r.jobs.is_empty(), "the build named its failed jobs: none asked for");
        let ci = r.pr_with_ci().unwrap().ci.unwrap();
        assert_eq!((ci.state, ci.providers()), (CiState::Failed, vec!["buildkite", "github_actions"]));
        assert_eq!(ci.url, "https://ci.example/buildkite/runs/app/3");
        *bk.error.lock().unwrap() = Some("Buildkite is down".into());
        let r = read(&gh, &more, 0, false, None, &HashMap::new(), None);
        assert!(r.pr.is_some(), "the PR is read all the same");
        assert_eq!(r.ci[1], ("buildkite".to_string(), Err("Buildkite is down".to_string())));
    }

    /// A provider that builds branches (Buildkite) is read on the branch's head before there is a PR; the forge's
    /// own CI isn't.
    #[test]
    fn without_a_pr_only_providers_that_build_branches_are_read() {
        let gh = Arc::new(FakeForge::default());
        let bk = buildkite();
        bk.set_runs("b", vec![bk.run("app/1", "app", "b", "pushed", CiState::Running, "t")]);
        let more: Vec<Arc<dyn CiProvider>> = vec![bk.clone()];
        let r = read(&gh, &more, 0, false, None, &HashMap::new(), Some("pushed"));
        assert!(r.pr.is_none());
        assert_eq!(r.head.as_deref(), Some("pushed"));
        assert_eq!(r.ci.len(), 1);
        let ci = r.ci[0].1.clone().unwrap().unwrap();
        assert_eq!((ci.state, ci.runs[0].provider.as_str()), (CiState::Running, "buildkite"));
        assert!(gh.ci.calls.lock().unwrap().is_empty(), "Actions waits for a pull request");
        assert!(read(&gh, &more, 0, false, None, &HashMap::new(), None).ci.is_empty(), "no head: nothing to read");
    }

    /// A forge that can't be read (GitHub signed out, rate limited) doesn't stop the providers that build branches:
    /// they are read on the last PR's head, else the branch's, and the forge's error is the read's.
    #[test]
    fn a_failed_forge_read_still_reads_the_providers_that_build_branches() {
        let gh = Arc::new(FakeForge::default());
        *gh.error.lock().unwrap() = Some("GitHub is not connected".into());
        let bk = buildkite();
        bk.set_runs("b", vec![bk.run("app/1", "app", "b", "pr-head", CiState::Failed, "t")]);
        let args = |last: Option<&str>| ReadArgs {
            forge: Some(gh.clone()),
            ci: vec![gh.ci.clone(), bk.clone()],
            repo: "o/r".into(),
            branch: "b".into(),
            since: 0,
            was_queued: false,
            queue_cache: None,
            known: Default::default(),
            branch_head: Some("pushed".into()),
            last_pr_head: last.map(String::from),
        };
        let r = read_attempt(&args(Some("pr-head"))).unwrap();
        assert_eq!(r.forge_error.as_deref(), Some("GitHub is not connected"));
        assert!(r.pr.is_none());
        assert_eq!(r.head.as_deref(), Some("pr-head"), "the last PR's head");
        assert_eq!(r.ci.len(), 1, "Buildkite only: {:?}", r.ci);
        assert_eq!(r.ci[0].1.as_ref().unwrap().as_ref().unwrap().state, CiState::Failed);
        assert!(gh.ci.calls.lock().unwrap().is_empty(), "Actions waits for the forge");
        let r = read_attempt(&args(None)).unwrap();
        assert_eq!(r.head.as_deref(), Some("pushed"), "no PR seen: the branch's head");
        assert_eq!(r.ci[0].1, Ok(None), "no build of that commit");
    }

    #[test]
    fn ci_events_name_their_provider_and_are_news_once_per_run() {
        let info = AttemptInfo {
            attempt: pdt_core::ticket::AttemptSummary {
                id: "a1".into(),
                branch: "b".into(),
                base_branch: "main".into(),
                worktree_id: None,
                state: pdt_core::ticket::AttemptState::Active,
                created_at: 0,
                ended_at: None,
                last_head: None,
                workspace_id: None,
                pr: None,
                ci: None,
            },
            ticket_id: "t".into(),
            ticket_key: "P-1".into(),
            project_id: "p".into(),
            repo_path: String::new(),
            base_commit: String::new(),
            own_commits: false,
            worktree_path: None,
            finish_on_merge: false,
            finish_delete_branch: false,
            merge_requested: false,
            merge_refused: false,
        };
        let bk = buildkite();
        let mut build = bk.run("app/3", "app", "b", "h", CiState::Failed, "t");
        build.failed_jobs = vec!["test".into()];
        let ci = CiStatus::of(&[build], "h").unwrap();
        let source = crate::ci::source(bk.as_ref());
        let e = ci_events(&info, &source, &ci, None, "h", false, true);
        assert_eq!(
            (e.len(), e[0].kind.as_str(), e[0].key.as_str()),
            (1, ev::CI_RUN_FAILED, "a1:ci_failed:buildkite:app/3.1")
        );
        assert_eq!((e[0].payload["provider"].as_str(), e[0].payload["pr"].as_u64()), (Some("buildkite"), None));
        assert_eq!(e[0].payload["runs"][0]["id"], "app/3");
        assert_eq!(e[0].payload["mergeWaiting"], true);
        let p = PullRequestStatus {
            number: 12,
            url: "u".into(),
            title: "t".into(),
            state: PrState::Open,
            draft: false,
            base_branch: "main".into(),
            head_sha: "h".into(),
            mergeable: Mergeable::Mergeable,
            behind: false,
            auto_merge: true,
            in_merge_queue: false,
            queue_position: None,
            ci: None,
            queue_ci: None,
            queue_removal: None,
            checked_at: 0,
            error: None,
            local: false,
            merge_held: false,
        };
        let gh = CiStatus::of(&[run(7, "b", "h", CiState::Passed)], "h").unwrap();
        let actions = CiSource { id: "github_actions".into(), label: "GitHub Actions".into(), logs: true };
        let e = ci_events(&info, &actions, &gh, Some(&p), "h", false, false);
        assert_eq!((e[0].kind.as_str(), e[0].key.as_str()), (ev::CI_RUN_PASSED, "a1:ci_passed:7.1"));
        assert_eq!(e[0].payload["pr"], 12);
    }

    #[test]
    fn reads_the_newest_merge_queue_run_of_the_pr() {
        let gh = Arc::new(FakeForge::default());
        gh.set_pr("b", Some(pr(true)));
        *gh.ci.queue.lock().unwrap() = vec![
            run(5, "fake-queue/main/12/aaa", "q1", CiState::Failed),
            run(6, "fake-queue/main/12/bbb", "q2", CiState::Running),
            run(7, "fake-queue/main/120/ccc", "q3", CiState::Failed),
        ];
        let r = read_pr(&gh, 0, false, None);
        let q = r.pr.unwrap().queue_ci.unwrap();
        assert_eq!((q.state, q.url.as_str()), (CiState::Running, "r6"));
        assert_eq!(r.queue.unwrap().len(), 3, "handed back for the repo's other attempts");
        // A repo's runs read for one attempt are reused for the next.
        clear(&gh);
        assert_eq!(read_pr(&gh, 0, false, Some(vec![])).queue, None);
        assert!(!calls(&gh).iter().any(|c| c.starts_with("queue")));
    }

    #[test]
    fn a_forge_without_merge_queues_reads_no_queue_runs() {
        let gh = Arc::new(FakeForge::with(pdt_forge::Capabilities::NONE));
        gh.set_pr("b", Some(pr(true)));
        let r = read_pr(&gh, 0, true, None);
        assert_eq!((r.pr.unwrap().queue_ci, r.queue), (None, None));
        assert_eq!(calls(&gh), vec!["pr o/r b".to_string(), "runs o/r b".to_string()]);
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
                reason: QueueRemovalReason::FailedChecks,
                by_queue: true,
            }),
            checked_at: 0,
            error: None,
            local: false,
            merge_held: false,
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
        let gh = Arc::new(FakeForge::default());
        let mut p = pr(false);
        p.state = PrState::Merged;
        gh.set_pr("b", Some(p));
        let status = read_pr(&gh, 0, true, None).pr_with_ci().unwrap();
        assert!(status.ci.is_none());
        assert_eq!(notices(&status, "b")[0].kind, NoticeKind::Merged);
        assert_eq!(calls(&gh).len(), 1);
    }

    /// A ticket started again on its old branch name: the earlier attempt's merged PR is not this attempt's, an open
    /// one or one opened since is.
    #[test]
    fn a_pr_closed_before_the_attempt_started_is_not_its() {
        let gh = Arc::new(FakeForge::default());
        let mut p = pr(false);
        p.state = PrState::Merged;
        p.created_at = Some(100);
        gh.set_pr("b", Some(p.clone()));
        let read = |since| read_pr(&gh, since, false, None).pr;
        assert_eq!(read(200), None, "an earlier attempt's");
        assert_eq!(read(100).map(|s| s.number), Some(12), "opened as this attempt started");
        p.state = PrState::Open;
        gh.set_pr("b", Some(p));
        assert_eq!(read(200).map(|s| s.number), Some(12), "still open: the branch's PR");
    }
}
