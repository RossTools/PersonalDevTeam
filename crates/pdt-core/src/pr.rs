//! A ticket attempt's pull request on GitHub (PDT-25): its state, auto-merge and merge queue, conflicts, and CI from
//! the Actions runs on its head commit (the fine-grained token cannot read check runs). Also the notices pdtd types
//! into the attempt's agent session when CI fails, the branch conflicts with the base, the PR leaves the merge queue
//! without merging, or it merges. Pure: pdt-github reads GitHub, pdt-api polls and delivers.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
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
    /// `gh run list`'s `status` and `conclusion`.
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
    #[ts(type = "number")]
    pub checked_at: i64,
}

/// The merge-queue branch prefix of PR `number` into `base`.
pub fn queue_branch_prefix(base: &str, number: u32) -> String {
    format!("gh-readonly-queue/{base}/pr-{number}-")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NoticeKind {
    CiFailed,
    Conflict,
    QueueLeft,
    Merged,
}

/// Something to tell the attempt's agent. `key` says when it is the same news again (it is told once per key).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What `now` (compared with `prev`, the previous look in this pdtd run, if any) gives the agent to act on.
pub fn notices(prev: Option<&PullRequestStatus>, now: &PullRequestStatus, branch: &str) -> Vec<Notice> {
    let n = now.number;
    let base = &now.base_branch;
    let mut out = vec![];
    if now.state == PrState::Merged {
        out.push(merged_notice(Some(n), branch, base));
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
        let mut text = format!("[PDT] CI failed on PR #{n} ({branch}) at {}", short(&now.head_sha));
        if !jobs.is_empty() {
            text.push_str(&format!(": {}", jobs.join(", ")));
        }
        text.push_str(&format!(". Run: {}.", ci.url));
        if let Some(id) = first {
            text.push_str(&format!(" Read the failure with `gh run view {id} --log-failed`,"));
        }
        text.push_str(" fix it, run the checks and push.");
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
            text: format!(
                "[PDT] PR #{n} ({branch}) conflicts with {base}. Merge it in (`git fetch origin && git merge \
                 origin/{base}`), resolve the conflicts, run the checks and push."
            ),
        });
    }
    let queue_failed = now.queue_ci.as_ref().filter(|q| q.state == CiState::Failed);
    let left = prev.is_some_and(|p| p.in_merge_queue && p.state == PrState::Open);
    if !now.in_merge_queue && (left || queue_failed.is_some()) {
        let (key, why, url, jobs) = match queue_failed {
            Some(q) => {
                let ids: Vec<String> = q.failed().map(|r| format!("{}.{}", r.id, r.attempt)).collect();
                let jobs: Vec<String> = q.failed().flat_map(|r| r.failed_jobs.iter().cloned()).collect();
                let mut why = String::from("its merge-queue CI failed");
                if !jobs.is_empty() {
                    why.push_str(&format!(" ({})", jobs.join(", ")));
                }
                why.push_str(&format!(": {}", q.url));
                (format!("queue_failed:{}", ids.join(",")), why, q.url.clone(), jobs)
            }
            None => (
                format!("queue_left:{}", now.head_sha),
                format!("it was removed (a conflict with {base}?)"),
                now.url.clone(),
                vec![],
            ),
        };
        out.push(Notice {
            kind: NoticeKind::QueueLeft,
            key,
            pr: Some(n),
            url: Some(url),
            jobs,
            text: format!(
                "[PDT] PR #{n} ({branch}) left the merge queue without merging: {why}. Fix it on {branch} \
                 (merge origin/{base} if needed), push, and turn auto-merge back on."
            ),
        });
    }
    out
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

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

#[cfg(test)]
mod tests {
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

    fn pr() -> PullRequestStatus {
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
            checked_at: 0,
        }
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
        assert!(notices(None, &p, "b").is_empty());
        let mut failed = run(7, "ci", "abcdef0123", "t", CiState::Failed);
        failed.failed_jobs = vec!["test (linux)".into()];
        p.ci = CiStatus::of(&[failed.clone()], "abcdef0123");
        let n = notices(None, &p, "pdt/x");
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NoticeKind::CiFailed);
        assert_eq!(n[0].key, "ci_failed:7.1");
        assert_eq!(n[0].jobs, vec!["test (linux)".to_string()]);
        assert!(n[0].text.contains("CI failed on PR #12 (pdt/x) at abcdef0: test (linux)"), "{}", n[0].text);
        assert!(n[0].text.contains("gh run view 7 --log-failed"));
        failed.attempt = 2;
        p.ci = CiStatus::of(&[failed], "abcdef0123");
        assert_eq!(notices(None, &p, "b")[0].key, "ci_failed:7.2", "a re-run that fails again is news");
    }

    #[test]
    fn a_conflict_is_news_per_head() {
        let mut p = pr();
        p.mergeable = Mergeable::Conflicting;
        let n = notices(None, &p, "b");
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::Conflict, "conflict:abcdef0123"));
        assert!(n[0].text.contains("conflicts with main"));
        p.mergeable = Mergeable::Unknown;
        assert!(notices(None, &p, "b").is_empty());
    }

    #[test]
    fn leaving_the_merge_queue_without_merging() {
        let mut queued = pr();
        queued.in_merge_queue = true;
        assert!(notices(None, &queued, "b").is_empty());
        let mut now = pr();
        now.auto_merge = false;
        let n = notices(Some(&queued), &now, "b");
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::QueueLeft, "queue_left:abcdef0123"));
        // A failed queue run says why, and is news even without having seen the PR queued (pdtd restarted).
        let mut q = run(9, "ci", "queuesha", "t", CiState::Failed);
        q.failed_jobs = vec!["web (linux)".into()];
        now.queue_ci = CiStatus::of(&[q], "queuesha");
        let n = notices(None, &now, "b");
        assert_eq!(n[0].key, "queue_failed:9.1");
        assert!(n[0].text.contains("merge-queue CI failed (web (linux)): https://gh/runs/9"), "{}", n[0].text);
        // Still queued (a re-queue after a fix): nothing.
        now.in_merge_queue = true;
        assert!(notices(Some(&queued), &now, "b").is_empty());
    }

    #[test]
    fn merged_is_the_only_news_once_merged() {
        let mut p = pr();
        p.state = PrState::Merged;
        p.mergeable = Mergeable::Conflicting;
        let n = notices(None, &p, "b");
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].key.as_str()), (NoticeKind::Merged, "merged"));
        assert!(n[0].text.contains("PR #12 (b) was merged into main"));
        assert_eq!(merged_notice(None, "b", "main").key, "merged");
        p.state = PrState::Closed;
        assert!(notices(None, &p, "b").is_empty());
    }
}
