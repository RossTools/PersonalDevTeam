//! A ticket attempt's pull request on GitHub (PDT-25): its state, auto-merge and merge queue, conflicts, and CI from
//! the Actions runs on its head commit (the fine-grained token cannot read check runs). Also the notices pdtd types
//! into the attempt's agent session when CI fails, the branch conflicts with the base, the merge queue removes the PR
//! without merging it, or it merges. Pure: pdt-github reads GitHub, pdt-api polls and delivers.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

/// Whether GitHub can merge the PR into its base. `Unknown` while GitHub is still working it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Mergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CiState {
    Passed,
    Cancelled,
    Queued,
    Running,
    Failed,
}

/// One GitHub Actions workflow run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiRun {
    #[ts(type = "number")]
    pub id: u64,
    /// Re-running a workflow keeps its id and bumps this.
    pub attempt: u32,
    pub workflow: String,
    pub state: CiState,
    pub url: String,
    #[serde(skip)]
    #[ts(skip)]
    pub head_sha: String,
    #[serde(skip)]
    #[ts(skip)]
    pub head_branch: String,
    /// RFC 3339; orders runs of one workflow.
    #[serde(skip)]
    #[ts(skip)]
    pub created_at: String,
    /// The jobs that failed (filled in for failed runs only).
    pub failed_jobs: Vec<String>,
}

impl CiRun {
    /// An Actions run's `status` and `conclusion`.
    pub fn state_of(status: &str, conclusion: &str) -> CiState {
        match status {
            "completed" => match conclusion {
                "success" | "skipped" | "neutral" => CiState::Passed,
                "cancelled" | "stale" => CiState::Cancelled,
                _ => CiState::Failed,
            },
            "in_progress" => CiState::Running,
            _ => CiState::Queued,
        }
    }
}

/// CI on one commit: the newest run of each workflow, rolled up (a failure wins, then running, queued, cancelled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiStatus {
    pub state: CiState,
    /// The run to look at: the first failed one, else one still going, else the first.
    pub url: String,
    pub runs: Vec<CiRun>,
}

impl CiStatus {
    /// The CI of `head_sha` from a list of runs (any order, other commits ignored). None when it has no runs.
    pub fn of(runs: &[CiRun], head_sha: &str) -> Option<CiStatus> {
        let mut newest: Vec<&CiRun> = vec![];
        for r in runs.iter().filter(|r| r.head_sha == head_sha) {
            match newest.iter_mut().find(|n| n.workflow == r.workflow) {
                Some(n) if (r.created_at.as_str(), r.id, r.attempt) > (n.created_at.as_str(), n.id, n.attempt) => {
                    *n = r
                }
                Some(_) => {}
                None => newest.push(r),
            }
        }
        newest.sort_by(|a, b| a.workflow.cmp(&b.workflow));
        let state = newest.iter().map(|r| r.state).max()?;
        let url = newest.iter().find(|r| r.state == state).map(|r| r.url.clone()).unwrap_or_default();
        Some(CiStatus { state, url, runs: newest.into_iter().cloned().collect() })
    }

    pub fn failed(&self) -> impl Iterator<Item = &CiRun> {
        self.runs.iter().filter(|r| r.state == CiState::Failed)
    }

    /// Two providers' CI on one commit as one (the forge's and a CI integration's): their runs together, rolled up the
    /// same way.
    pub fn merge(a: Option<CiStatus>, b: Option<CiStatus>) -> Option<CiStatus> {
        match (a, b) {
            (Some(mut a), Some(b)) => {
                a.runs.extend(b.runs);
                a.state = a.runs.iter().map(|r| r.state).max().unwrap_or(a.state);
                a.url = a.runs.iter().find(|r| r.state == a.state).map(|r| r.url.clone()).unwrap_or_default();
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

/// An attempt's pull request, as pdtd last saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestStatus {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub draft: bool,
    pub base_branch: String,
    pub head_sha: String,
    pub mergeable: Mergeable,
    /// Out of date with its base (GitHub's `BEHIND`). With a merge queue this needs no action.
    pub behind: bool,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    /// 1-based, while queued.
    pub queue_position: Option<u32>,
    /// CI on the head commit.
    pub ci: Option<CiStatus>,
    /// The newest merge-queue run for this PR (`gh-readonly-queue/<base>/pr-<n>-…`), while queued or after it left.
    pub queue_ci: Option<CiStatus>,
    /// The PR's last removal from the merge queue, when that is its newest merge-queue event (not re-queued since).
    #[serde(skip)]
    #[ts(skip)]
    pub queue_removal: Option<QueueRemoval>,
    /// When it was last read (ms).
    #[ts(type = "number")]
    pub checked_at: i64,
    /// Why the reads since `checked_at` failed (GitHub not connected, or down): the rest is that older look.
    #[serde(default)]
    pub error: Option<String>,
    /// A change request on the local forge (PDT-178): PDT's own, with CI it runs and merges it makes itself. Its
    /// `url` is its newest CI run's log.
    #[serde(default)]
    pub local: bool,
}

impl PullRequestStatus {
    /// What the agent and the user call it: `PR #12`, or `change request #3` on the local forge.
    pub fn name(&self) -> String {
        if self.local { format!("change request #{}", self.number) } else { format!("PR #{}", self.number) }
    }

    /// Whether GitHub merged this PR with `head` as its head: the attempt whose branch head (or last known head) is
    /// `head` is merged, even when git cannot see it (the merge queue squashed it onto a newer base).
    pub fn merged_at(&self, head: &str) -> bool {
        self.state == PrState::Merged && !head.is_empty() && self.head_sha == head
    }
}

/// A `RemovedFromMergeQueueEvent` on the PR's timeline. GitHub turns auto-merge off when it removes a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRemoval {
    /// The event's node id: one removal is news once.
    pub id: String,
    /// GitHub's reason, e.g. `merge_conflict`, `failed_checks`, `merged`.
    pub reason: String,
    /// The merge queue removed it (not a person dequeuing it by hand).
    pub by_queue: bool,
}

impl QueueRemoval {
    /// The `github-merge-queue` app (`github-merge-queue[bot]` in some fields).
    pub fn is_queue_actor(login: &str) -> bool {
        login.starts_with("github-merge-queue")
    }
}

/// The merge-queue branch prefix of PR `number` into `base`.
pub fn queue_branch_prefix(base: &str, number: u32) -> String {
    format!("gh-readonly-queue/{base}/pr-{number}-")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoticeKind {
    CiFailed,
    Conflict,
    QueueLeft,
    Merged,
    /// Behind its base while it waits to merge without a merge queue (PDT-24).
    Behind,
    /// Someone commented on the pull request (Phase 7e).
    Commented,
    /// Someone reviewed the pull request (Phase 7e).
    Reviewed,
}

/// Something to tell the attempt's agent. `key` says when it is the same news again (it is told once per key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notice {
    pub kind: NoticeKind,
    pub key: String,
    pub pr: Option<u32>,
    /// The run or PR to look at.
    pub url: Option<String>,
    pub jobs: Vec<String>,
    /// The message typed into the agent's terminal.
    pub text: String,
}

/// What `now` gives the agent to act on.
pub fn notices(now: &PullRequestStatus, branch: &str) -> Vec<Notice> {
    let n = now.number;
    let base = &now.base_branch;
    let mut out = vec![];
    if now.state == PrState::Merged {
        let mut m = merged_notice(Some(n), branch, base);
        if now.local {
            m.text = m.text.replacen(&format!("PR #{n}"), &now.name(), 1);
        }
        out.push(m);
        return out;
    }
    if now.state != PrState::Open {
        return out;
    }
    if let Some(ci) = &now.ci
        && ci.state == CiState::Failed
    {
        let failed: Vec<&CiRun> = ci.failed().collect();
        let ids: Vec<String> = failed.iter().map(|r| format!("{}.{}", r.id, r.attempt)).collect();
        let jobs: Vec<String> = failed.iter().flat_map(|r| r.failed_jobs.iter().cloned()).collect();
        let first = failed.first().map(|r| r.id);
        let mut text = format!("[PDT] CI failed on {} ({branch}) at {}", now.name(), short(&now.head_sha));
        if !jobs.is_empty() {
            text.push_str(&format!(": {}", jobs.join(", ")));
        }
        text.push_str(&format!(". Run: {}.", ci.url));
        if now.local {
            // No remote: the log is PDT's, and a commit on the branch is what runs CI again.
            text.push_str(&format!(" Read its log with `curl -s {}`, fix it, run the checks and commit.", ci.url));
        } else {
            if let Some(id) = first {
                text.push_str(&format!(" Read the failure with `ci_failed_logs` (runId {id}),"));
            }
            text.push_str(" fix it, run the checks and push.");
        }
        // A merge in flight (PDT-151): the fix is what it merges.
        if now.local && now.auto_merge {
            text.push_str(" It is set to merge, so it merges once the fix passes.");
        } else if now.in_merge_queue {
            text.push_str(" The pull request is in the merge queue, so it merges once the fix passes.");
        } else if now.auto_merge {
            text.push_str(" Auto-merge is on for the pull request, so it merges once the fix passes.");
        }
        out.push(Notice {
            kind: NoticeKind::CiFailed,
            key: format!("ci_failed:{}", ids.join(",")),
            pr: Some(n),
            url: Some(ci.url.clone()),
            jobs,
            text,
        });
    }
    if now.mergeable == Mergeable::Conflicting {
        out.push(Notice {
            kind: NoticeKind::Conflict,
            key: format!("conflict:{}", now.head_sha),
            pr: Some(n),
            url: Some(now.url.clone()),
            jobs: vec![],
            text: if now.local {
                format!(
                    "[PDT] {} ({branch}) conflicts with {base}. Merge it in (`git merge {base}`), resolve the \
                     conflicts, run the checks and commit.",
                    now.name()
                )
            } else {
                format!(
                    "[PDT] PR #{n} ({branch}) conflicts with {base}. Merge it in (`git fetch origin && git merge \
                     origin/{base}`), resolve the conflicts, run the checks and push."
                )
            },
        });
    }
    if let Some(r) = now.queue_removal.as_ref().filter(|r| r.by_queue && r.reason != "merged")
        && !now.in_merge_queue
        && !now.auto_merge
    {
        let queue_failed = now.queue_ci.as_ref().filter(|q| q.state == CiState::Failed);
        let (why, url, jobs) = match (r.reason.as_str(), queue_failed) {
            ("merge_conflict", _) => (format!("it conflicts with {base}"), now.url.clone(), vec![]),
            (_, Some(q)) => {
                let jobs: Vec<String> = q.failed().flat_map(|r| r.failed_jobs.iter().cloned()).collect();
                let mut why = String::from("its merge-queue CI failed");
                if !jobs.is_empty() {
                    why.push_str(&format!(" ({})", jobs.join(", ")));
                }
                why.push_str(&format!(": {}", q.url));
                if let Some(first) = q.failed().next() {
                    why.push_str(&format!(". Read it with `ci_failed_logs` (runId {})", first.id));
                }
                (why, q.url.clone(), jobs)
            }
            ("failed_checks", None) => ("its merge-queue CI failed".into(), now.url.clone(), vec![]),
            (other, None) => (format!("GitHub says `{other}`"), now.url.clone(), vec![]),
        };
        out.push(Notice {
            kind: NoticeKind::QueueLeft,
            key: format!("queue_removed:{}", r.id),
            pr: Some(n),
            url: Some(url),
            jobs,
            text: format!(
                "[PDT] PR #{n} ({branch}) was removed from the merge queue: {why}. Fix it on {branch} (merge \
                 origin/{base} if needed), run the checks and push. The merge then has to be asked for again \
                 (Merge on the ticket)."
            ),
        });
    }
    out
}

/// An open PR with auto-merge on, not in a merge queue, that is behind its base: it can't merge until the base is
/// merged in (PDT-24). Once per head commit.
pub fn behind_notice(now: &PullRequestStatus, branch: &str) -> Option<Notice> {
    if now.state != PrState::Open || !now.behind || !now.auto_merge || now.in_merge_queue {
        return None;
    }
    let (n, base) = (now.number, &now.base_branch);
    Some(Notice {
        kind: NoticeKind::Behind,
        key: format!("behind:{}", now.head_sha),
        pr: Some(n),
        url: Some(now.url.clone()),
        jobs: vec![],
        text: format!(
            "[PDT] {base} moved while PR #{n} ({branch}) waits to merge, and the PR is now behind it. Merge it in \
             (`git fetch origin && git merge origin/{base}`), run the checks and push."
        ),
    })
}

/// The longest comment or review body a notice quotes.
const QUOTE_MAX: usize = 4000;

fn quote(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() <= QUOTE_MAX {
        return body.to_string();
    }
    let cut: String = body.chars().take(QUOTE_MAX).collect();
    format!("{cut}… (cut short: read it at the link)")
}

/// What a comment on PR `pr` is, for the agent: on a line (`path`/`line`, in a review) or on the conversation.
#[allow(clippy::too_many_arguments)]
pub fn comment_notice(
    pr: u32,
    branch: &str,
    id: u64,
    author: &str,
    body: &str,
    path: Option<&str>,
    line: Option<u32>,
    url: &str,
) -> Notice {
    let place = match (path, line) {
        (Some(p), Some(l)) => format!("a review comment on {p}:{l}"),
        (Some(p), None) => format!("a review comment on {p}"),
        _ => "a comment".into(),
    };
    Notice {
        kind: NoticeKind::Commented,
        key: format!("comment:{id}"),
        pr: Some(pr),
        url: Some(url.into()),
        jobs: vec![],
        text: format!(
            "[PDT] {author} left {place} on PR #{pr} ({branch}):\n\n{}\n\n{url}\nAddress it if it asks for a change \
             (then run the checks and push), or answer on the pull request.",
            quote(body)
        ),
    }
}

/// What a review of PR `pr` is, for the agent. `state`: `approved`, `changes_requested`, `commented`.
pub fn review_notice(pr: u32, branch: &str, id: u64, author: &str, state: &str, body: &str, url: &str) -> Notice {
    let what = match state {
        "approved" => "approved",
        "changes_requested" => "requested changes on",
        _ => "reviewed",
    };
    let mut text = format!("[PDT] {author} {what} PR #{pr} ({branch}).");
    if !body.trim().is_empty() {
        text.push_str(&format!("\n\n{}\n", quote(body)));
    }
    text.push_str(&format!("\n{url}"));
    if state == "changes_requested" {
        text.push_str("\nMake the changes (the review's line comments come separately), run the checks and push.");
    }
    Notice {
        kind: NoticeKind::Reviewed,
        key: format!("review:{id}"),
        pr: Some(pr),
        url: Some(url.into()),
        jobs: vec![],
        text,
    }
}

/// The branch's work is in the base: by its PR (`pr`) or as git found it.
pub fn merged_notice(pr: Option<u32>, branch: &str, base: &str) -> Notice {
    let what = match pr {
        Some(n) => format!("PR #{n} ({branch}) was merged into {base}"),
        None => format!("{branch} was merged into {base}"),
    };
    Notice {
        kind: NoticeKind::Merged,
        key: "merged".into(),
        pr,
        url: None,
        jobs: vec![],
        text: format!("[PDT] {what}. This ticket's work is in; push nothing more to {branch}."),
    }
}

/// Whether the cause of a notice of `kind` went away, as `now` shows the pull request (PDT-139): it is no longer open,
/// a conflict is mergeable again, CI passed, it is back in the merge queue (or set to merge), it caught up with its
/// base. An unheard notice's Inbox item closes then.
pub fn notice_settled(kind: NoticeKind, now: &PullRequestStatus) -> bool {
    if now.state != PrState::Open {
        return true;
    }
    match kind {
        NoticeKind::Conflict => now.mergeable == Mergeable::Mergeable,
        NoticeKind::CiFailed => now.ci.as_ref().is_some_and(|c| c.state == CiState::Passed),
        NoticeKind::QueueLeft => now.in_merge_queue || now.auto_merge,
        NoticeKind::Behind => !now.behind,
        NoticeKind::Merged | NoticeKind::Commented | NoticeKind::Reviewed => false,
    }
}

/// Whether CI lets an automated merge of an open pull request go ahead (PDT-151): its head's CI, from every provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiGate {
    /// Every run on the head passed.
    Passed,
    /// The repository has no CI: nothing to wait for.
    NoCi,
    /// Runs on the head are queued or running, or none has started yet though the repository has CI.
    Pending(String),
    /// A run on the head failed or was cancelled: the attempt's agent fixes it.
    Failed(String),
}

/// The gate for `pr` with `ci` on its head (the forge's CI and the CI integrations' together). `has_ci`: the repository
/// has CI (workflow files, a CI integration's pipeline), so no run yet means one is still to come.
pub fn ci_gate(pr: &PullRequestStatus, ci: Option<&CiStatus>, has_ci: bool) -> CiGate {
    let (n, at) = (pr.name(), short(&pr.head_sha));
    let Some(ci) = ci else {
        return if has_ci { CiGate::Pending(format!("CI hasn't started on {n} at {at} yet")) } else { CiGate::NoCi };
    };
    match ci.state {
        CiState::Passed => CiGate::Passed,
        CiState::Queued | CiState::Running => {
            CiGate::Pending(format!("CI is still running on {n} at {at}: {}", ci.url))
        }
        CiState::Cancelled if pr.local => {
            CiGate::Failed(format!("CI was cancelled on {n} at {at} ({}); commit a fix to run it again", ci.url))
        }
        CiState::Cancelled => CiGate::Failed(format!(
            "CI was cancelled on {n} at {at} ({}); run it again on the forge or push a fix",
            ci.url
        )),
        CiState::Failed => {
            let failed: Vec<&CiRun> = ci.failed().collect();
            let jobs: Vec<String> = failed.iter().flat_map(|r| r.failed_jobs.iter().cloned()).collect();
            let mut why = format!("CI failed on {n} at {at}");
            if !jobs.is_empty() {
                why.push_str(&format!(" ({})", jobs.join(", ")));
            }
            why.push_str(&format!(": {}", ci.url));
            if pr.local {
                why.push_str(&format!(". Read its log with `curl -s {}`", ci.url));
            } else if let Some(r) = failed.iter().find(|r| r.url.contains("github.com")) {
                why.push_str(&format!(". Read it with `ci_failed_logs` (runId {})", r.id));
            }
            CiGate::Failed(why)
        }
    }
}

/// A commit on an attempt's branch that its merged pull request doesn't have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LateCommit {
    pub sha: String,
    pub subject: String,
}

/// What to say when commits were pushed to the branch after its pull request merged (PDT-151): `summary` for the
/// Inbox ("CB-25: 1 commit pushed after PR #308 merged: cd5d0b7 Fix the build"), `notice` for the attempt's agent.
pub fn late_commits_notice(key: &str, pr: u32, branch: &str, base: &str, commits: &[LateCommit]) -> (String, Notice) {
    let n = commits.len();
    let list: Vec<String> =
        commits.iter().take(5).map(|c| format!("{} {}", short(&c.sha), c.subject).trim_end().to_string()).collect();
    let mut listed = list.join("; ");
    if n > list.len() {
        listed.push_str(&format!("; and {} more", n - list.len()));
    }
    let what = if n == 1 { "1 commit".to_string() } else { format!("{n} commits") };
    let summary = format!("{key}: {what} pushed after PR #{pr} merged: {listed}");
    let head = commits.first().map(|c| c.sha.clone()).unwrap_or_default();
    let text = format!(
        "[PDT] PR #{pr} ({branch}) merged into {base} before you pushed {what} ({listed}): {} not in {base}. Push \
         nothing more to {branch}. The user decides in PDT's Inbox whether {} into a follow-up ticket or {} dropped.",
        if n == 1 { "it is" } else { "they are" },
        if n == 1 { "it goes" } else { "they go" },
        if n == 1 { "is" } else { "are" },
    );
    let notice = Notice {
        kind: NoticeKind::Merged,
        key: format!("late_commits:{head}"),
        pr: Some(pr),
        url: None,
        jobs: vec![],
        text,
    };
    (summary, notice)
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn comment_and_review_notices_quote_and_say_where() {
        let n =
            super::comment_notice(12, "pdt/x", 99, "alice", "Use a const.", Some("src/a.rs"), Some(40), "https://c");
        assert_eq!((n.kind, n.key.as_str()), (super::NoticeKind::Commented, "comment:99"));
        assert!(n.text.starts_with("[PDT] alice left a review comment on src/a.rs:40 on PR #12 (pdt/x):"));
        assert!(n.text.contains("Use a const.") && n.text.contains("https://c"));
        let long = "x".repeat(5000);
        assert!(super::comment_notice(1, "b", 1, "a", &long, None, None, "u").text.contains("cut short"));
        let r = super::review_notice(12, "pdt/x", 5, "bob", "changes_requested", "", "https://r");
        assert_eq!(r.key, "review:5");
        assert!(r.text.starts_with("[PDT] bob requested changes on PR #12 (pdt/x)."));
        assert!(r.text.contains("Make the changes"));
        assert!(!super::review_notice(1, "b", 1, "a", "approved", "LGTM", "u").text.contains("Make the changes"));
    }

    use super::*;

    fn run(id: u64, workflow: &str, sha: &str, at: &str, state: CiState) -> CiRun {
        CiRun {
            id,
            attempt: 1,
            workflow: workflow.into(),
            state,
            url: format!("https://gh/runs/{id}"),
            head_sha: sha.into(),
            head_branch: "b".into(),
            created_at: at.into(),
            failed_jobs: vec![],
        }
    }

    #[test]
    fn the_ci_gate_waits_for_green_ci_on_the_head() {
        let mut p = pr();
        p.number = 308;
        p.head_sha = "abcdef1234".into();
        let ci = |runs: Vec<CiRun>| CiStatus::of(&runs, "abcdef1234");
        let at = "2026-10-25T00:00:00Z";
        let passed = ci(vec![run(1, "ci", "abcdef1234", at, CiState::Passed)]);
        assert_eq!(ci_gate(&p, passed.as_ref(), true), CiGate::Passed);
        assert_eq!(ci_gate(&p, None, false), CiGate::NoCi, "a repository without CI merges as before");
        assert_eq!(ci_gate(&p, None, true), CiGate::Pending("CI hasn't started on PR #308 at abcdef1 yet".into()));
        let running = ci(vec![
            run(1, "ci", "abcdef1234", at, CiState::Passed),
            run(2, "macos", "abcdef1234", at, CiState::Running),
        ]);
        assert!(matches!(ci_gate(&p, running.as_ref(), true), CiGate::Pending(r) if r.contains("still running")));
        let mut red = run(3, "ci", "abcdef1234", at, CiState::Failed);
        red.url = "https://github.com/o/r/actions/runs/3".into();
        red.failed_jobs = vec!["test (linux)".into()];
        let failed = ci(vec![red, run(2, "macos", "abcdef1234", at, CiState::Running)]);
        assert_eq!(
            ci_gate(&p, failed.as_ref(), true),
            CiGate::Failed(
                "CI failed on PR #308 at abcdef1 (test (linux)): https://github.com/o/r/actions/runs/3. Read it with \
                 `ci_failed_logs` (runId 3)"
                    .into()
            ),
            "a failure wins over a run still going"
        );
        let cancelled = ci(vec![run(4, "ci", "abcdef1234", at, CiState::Cancelled)]);
        assert!(matches!(ci_gate(&p, cancelled.as_ref(), true), CiGate::Failed(r) if r.contains("cancelled")));
    }

    #[test]
    fn late_commits_say_what_was_pushed_after_the_merge() {
        let c = |sha: &str, subject: &str| LateCommit { sha: sha.into(), subject: subject.into() };
        let (summary, notice) = late_commits_notice("CB-25", 308, "b", "main", &[c("cd5d0b7aaaa", "Fix the build")]);
        assert_eq!(summary, "CB-25: 1 commit pushed after PR #308 merged: cd5d0b7 Fix the build");
        assert_eq!(notice.key, "late_commits:cd5d0b7aaaa");
        assert!(notice.text.contains("before you pushed 1 commit (cd5d0b7 Fix the build): it is not in main"));
        let many: Vec<LateCommit> = (0..7).map(|i| c(&format!("{i}000000"), "x")).collect();
        let (summary, notice) = late_commits_notice("CB-25", 308, "b", "main", &many);
        assert!(summary.starts_with("CB-25: 7 commits pushed after PR #308 merged: 0000000 x;"));
        assert!(summary.ends_with("; and 2 more"));
        assert!(notice.text.contains("they are not in main"));
    }

    #[test]
    fn merged_at_needs_a_merged_pr_on_that_head() {
        let mut p = pr();
        p.head_sha = "abc".into();
        assert!(!p.merged_at("abc"), "open");
        p.state = PrState::Merged;
        assert!(p.merged_at("abc"));
        assert!(!p.merged_at("def"), "the branch moved on after the merge");
        assert!(!p.merged_at(""));
        p.state = PrState::Closed;
        assert!(!p.merged_at("abc"), "closed without merging");
    }

    #[test]
    fn a_notice_settles_once_its_cause_is_gone() {
        let mut p = pr();
        p.mergeable = Mergeable::Conflicting;
        assert!(!notice_settled(NoticeKind::Conflict, &p));
        p.mergeable = Mergeable::Unknown;
        assert!(!notice_settled(NoticeKind::Conflict, &p), "not known yet");
        p.mergeable = Mergeable::Mergeable;
        assert!(notice_settled(NoticeKind::Conflict, &p));

        let ci = |state| Some(CiStatus { state, url: "u".into(), runs: vec![] });
        p.ci = ci(CiState::Failed);
        assert!(!notice_settled(NoticeKind::CiFailed, &p));
        p.ci = ci(CiState::Running);
        assert!(!notice_settled(NoticeKind::CiFailed, &p), "a new run, not passed yet");
        p.ci = ci(CiState::Passed);
        assert!(notice_settled(NoticeKind::CiFailed, &p));

        p.auto_merge = false;
        assert!(!notice_settled(NoticeKind::QueueLeft, &p));
        p.in_merge_queue = true;
        assert!(notice_settled(NoticeKind::QueueLeft, &p));

        p.behind = true;
        assert!(!notice_settled(NoticeKind::Behind, &p));
        p.behind = false;
        assert!(notice_settled(NoticeKind::Behind, &p));

        p.mergeable = Mergeable::Conflicting;
        p.state = PrState::Closed;
        assert!(notice_settled(NoticeKind::Conflict, &p), "a closed PR conflicts with nothing");
    }

    pub(crate) fn pr() -> PullRequestStatus {
        PullRequestStatus {
            number: 12,
            url: "https://gh/pull/12".into(),
            title: "T".into(),
            state: PrState::Open,
            draft: false,
            base_branch: "main".into(),
            head_sha: "abcdef0123".into(),
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
        }
    }

    /// The local forge's change request (PDT-178): no remote, no `gh`; its log is PDT's and a commit runs CI again.
    #[test]
    fn a_local_change_request_points_at_pdts_log_and_git_alone() {
        let mut p = pr();
        p.local = true;
        p.number = 3;
        let mut failed = run(7, "ci", "abcdef0123", "t", CiState::Failed);
        failed.url = "http://127.0.0.1:7420/api/local-ci/runs/7/log".into();
        failed.failed_jobs = vec!["cargo test".into()];
        p.ci = CiStatus::of(&[failed], "abcdef0123");
        let n = notices(&p, "b");
        let t = &n[0].text;
        assert!(t.contains("CI failed on change request #3 (b) at abcdef0: cargo test"), "{t}");
        assert!(
            t.contains("`curl -s http://127.0.0.1:7420/api/local-ci/runs/7/log`") && t.contains("and commit."),
            "{t}"
        );
        assert!(t.contains("It is set to merge") && !t.contains("gh ") && !t.contains("push"), "{t}");
        let CiGate::Failed(why) = ci_gate(&p, p.ci.as_ref(), true) else { panic!() };
        assert!(why.contains("change request #3") && why.contains("curl -s http://127.0.0.1"), "{why}");
        p.ci = None;
        p.mergeable = Mergeable::Conflicting;
        let t = &notices(&p, "b")[0].text;
        assert!(t.contains("Merge it in (`git merge main`)") && !t.contains("origin"), "{t}");
        p.state = PrState::Merged;
        let t = &notices(&p, "b")[0].text;
        assert!(t.starts_with("[PDT] change request #3 (b) was merged into main."), "{t}");
    }

    #[test]
    fn states_from_gh() {
        assert_eq!(CiRun::state_of("completed", "success"), CiState::Passed);
        assert_eq!(CiRun::state_of("completed", "skipped"), CiState::Passed);
        assert_eq!(CiRun::state_of("completed", "failure"), CiState::Failed);
        assert_eq!(CiRun::state_of("completed", "timed_out"), CiState::Failed);
        assert_eq!(CiRun::state_of("completed", "cancelled"), CiState::Cancelled);
        assert_eq!(CiRun::state_of("in_progress", ""), CiState::Running);
        assert_eq!(CiRun::state_of("queued", ""), CiState::Queued);
        assert_eq!(CiRun::state_of("waiting", ""), CiState::Queued);
    }

    #[test]
    fn ci_rolls_up_the_newest_run_of_each_workflow_on_the_head() {
        assert_eq!(CiStatus::of(&[], "h"), None);
        let runs = [
            run(1, "ci", "h", "2026-01-01T00:00:00Z", CiState::Failed),
            run(2, "ci", "h", "2026-01-01T00:05:00Z", CiState::Running),
            run(3, "lint", "h", "2026-01-01T00:00:00Z", CiState::Passed),
            run(4, "ci", "old", "2026-01-01T00:09:00Z", CiState::Failed),
        ];
        let ci = CiStatus::of(&runs, "h").unwrap();
        assert_eq!(ci.state, CiState::Running, "the failed run was superseded");
        assert_eq!(ci.url, "https://gh/runs/2");
        assert_eq!(ci.runs.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 3]);
        let runs = [
            run(2, "ci", "h", "2026-01-01T00:05:00Z", CiState::Passed),
            run(3, "lint", "h", "2026-01-01T00:00:00Z", CiState::Failed),
        ];
        let ci = CiStatus::of(&runs, "h").unwrap();
        assert_eq!((ci.state, ci.url.as_str()), (CiState::Failed, "https://gh/runs/3"));
        assert_eq!(CiStatus::of(&runs[..1], "h").unwrap().state, CiState::Passed);
    }

    #[test]
    fn a_failed_ci_run_is_news_once_per_run_attempt() {
        let mut p = pr();
        assert!(notices(&p, "b").is_empty());
        let mut failed = run(7, "ci", "abcdef0123", "t", CiState::Failed);
        failed.failed_jobs = vec!["test (linux)".into()];
        p.ci = CiStatus::of(&[failed.clone()], "abcdef0123");
        let n = notices(&p, "pdt/x");
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NoticeKind::CiFailed);
        assert_eq!(n[0].key, "ci_failed:7.1");
        assert_eq!(n[0].jobs, vec!["test (linux)".to_string()]);
        assert!(n[0].text.contains("CI failed on PR #12 (pdt/x) at abcdef0: test (linux)"), "{}", n[0].text);
        assert!(n[0].text.contains("`ci_failed_logs` (runId 7)"));
        failed.attempt = 2;
        p.ci = CiStatus::of(&[failed], "abcdef0123");
        assert_eq!(notices(&p, "b")[0].key, "ci_failed:7.2", "a re-run that fails again is news");
    }

    #[test]
    fn a_conflict_is_news_per_head() {
        let mut p = pr();
        p.mergeable = Mergeable::Conflicting;
        let n = notices(&p, "b");
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::Conflict, "conflict:abcdef0123"));
        assert!(n[0].text.contains("conflicts with main"));
        p.mergeable = Mergeable::Unknown;
        assert!(notices(&p, "b").is_empty());
    }

    fn removal(id: &str, reason: &str) -> Option<QueueRemoval> {
        Some(QueueRemoval { id: id.into(), reason: reason.into(), by_queue: true })
    }

    #[test]
    fn removal_from_the_merge_queue_is_news_once_per_removal() {
        let mut p = pr();
        p.auto_merge = false;
        p.queue_removal = removal("RFMQE_1", "merge_conflict");
        let n = notices(&p, "pdt/x");
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::QueueLeft, "queue_removed:RFMQE_1"));
        assert!(
            n[0].text.contains("PR #12 (pdt/x) was removed from the merge queue: it conflicts with main"),
            "{}",
            n[0].text
        );
        assert!(n[0].text.contains("The merge then has to be asked for again (Merge on the ticket)."));
        // Merged, removed by hand, back in the queue or auto-merge on again (someone dealt with it): nothing.
        p.queue_removal = removal("RFMQE_2", "merged");
        assert!(notices(&p, "b").is_empty());
        p.queue_removal = Some(QueueRemoval { by_queue: false, ..removal("RFMQE_3", "manual").unwrap() });
        assert!(notices(&p, "b").is_empty());
        p.queue_removal = removal("RFMQE_1", "merge_conflict");
        p.auto_merge = true;
        assert!(notices(&p, "b").is_empty());
        p.auto_merge = false;
        p.in_merge_queue = true;
        assert!(notices(&p, "b").is_empty());
    }

    #[test]
    fn a_failed_checks_removal_names_the_failed_queue_run() {
        let mut p = pr();
        p.auto_merge = false;
        p.queue_removal = removal("RFMQE_9", "failed_checks");
        let n = notices(&p, "b");
        assert!(n[0].text.contains("removed from the merge queue: its merge-queue CI failed."), "{}", n[0].text);
        let mut q = run(9, "ci", "queuesha", "t", CiState::Failed);
        q.failed_jobs = vec!["web (linux)".into()];
        p.queue_ci = CiStatus::of(&[q], "queuesha");
        let n = notices(&p, "b");
        assert_eq!((n[0].key.as_str(), n[0].jobs.clone()), ("queue_removed:RFMQE_9", vec!["web (linux)".to_string()]));
        assert!(
            n[0].text.contains(
                "its merge-queue CI failed (web (linux)): https://gh/runs/9. Read it with `ci_failed_logs` (runId 9)."
            ),
            "{}",
            n[0].text
        );
        assert_eq!(n[0].url.as_deref(), Some("https://gh/runs/9"));
        // Another reason says what GitHub said.
        p.queue_removal = removal("RFMQE_10", "timeout");
        p.queue_ci = None;
        assert!(notices(&p, "b")[0].text.contains("GitHub says `timeout`"));
        assert!(QueueRemoval::is_queue_actor("github-merge-queue") && !QueueRemoval::is_queue_actor("someone"));
    }

    #[test]
    fn merged_is_the_only_news_once_merged() {
        let mut p = pr();
        p.state = PrState::Merged;
        p.mergeable = Mergeable::Conflicting;
        let n = notices(&p, "b");
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::Merged, "merged"));
        assert!(n[0].text.contains("PR #12 (b) was merged into main"));
        assert_eq!(merged_notice(None, "b", "main").key, "merged");
        p.state = PrState::Closed;
        assert!(notices(&p, "b").is_empty());
    }
}
