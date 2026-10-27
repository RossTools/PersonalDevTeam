//! Forges (PDT-176): where a project's change requests (GitHub's pull requests, GitLab's merge requests), their CI,
//! reviews and comments, merges and issues live. The SDLC talks to a [`Forge`] and asks its [`Capabilities`] before a
//! step only some forges can do; `pdt-github` is the GitHub one. Blocking; call from `spawn_blocking` in async code.

pub use pdt_core::forge::{ForgeKind, ForgeNotes};
pub use pdt_core::github_issue::CloseReason;
use pdt_core::pr::{CiRun, Mergeable, PrState, QueueRemoval};

/// What a forge can do beyond reading a branch's change request and CI, so the SDLC skips (or does itself) what it
/// can't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A base branch can have a merge queue (its runs are read while a change request is queued).
    pub merge_queue: bool,
    /// The forge holds an auto-merge until the base branch's required checks pass. Without it, PDT's own merges wait
    /// for green CI.
    pub required_checks: bool,
    /// Issues: listing, reading, commenting, labelling, closing.
    pub issues: bool,
    /// Reviews and comments on change requests.
    pub reviews: bool,
}

impl Capabilities {
    /// Everything (GitHub).
    pub const ALL: Capabilities =
        Capabilities { merge_queue: true, required_checks: true, issues: true, reviews: true };
    /// Change requests and CI only.
    pub const NONE: Capabilities =
        Capabilities { merge_queue: false, required_checks: false, issues: false, reviews: false };
}

/// How a change request merges into its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    Squash,
    Merge,
    Rebase,
}

impl MergeStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeStrategy::Squash => "squash",
            MergeStrategy::Merge => "merge",
            MergeStrategy::Rebase => "rebase",
        }
    }
}

/// How a repository lets change requests into a base branch merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeSettings {
    pub squash: bool,
    pub merge: bool,
    pub rebase: bool,
    /// The base branch has a merge queue.
    pub queue: bool,
}

impl MergeSettings {
    /// The strategy to ask for: none when a merge queue sets it (GitHub refuses one then), else the first the
    /// repository allows of squash, merge commit and rebase.
    pub fn strategy(&self) -> Option<MergeStrategy> {
        if self.queue {
            return None;
        }
        [(self.squash, MergeStrategy::Squash), (self.merge, MergeStrategy::Merge), (self.rebase, MergeStrategy::Rebase)]
            .into_iter()
            .find(|(ok, _)| *ok)
            .map(|(_, s)| s)
    }
}

/// A branch's change request as the forge reports it (CI is read separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub draft: bool,
    pub head_sha: String,
    pub base_branch: String,
    pub mergeable: Mergeable,
    pub behind: bool,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    pub queue_position: Option<u32>,
    /// Its newest merge-queue event, when that is a removal.
    pub queue_removal: Option<QueueRemoval>,
    /// When it was opened (ms; None when unknown): a ticket started again on the same branch has its old one there too.
    pub created_at: Option<i64>,
}

/// An issue (not a change request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    pub labels: Vec<String>,
    pub created_at: String,
}

/// One issue with what closing it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDetail {
    pub issue: Issue,
    /// The forge's own id (what a duplicate names, [`Forge::close_issue`]), not its number.
    pub id: u64,
    pub closed: bool,
    /// Why it was closed (`completed`, `not_planned`, `duplicate`), when it is.
    pub state_reason: Option<String>,
    /// It is a change request (on GitHub they share issue numbers).
    pub change_request: bool,
}

/// A comment on a change request: on its conversation, or on a line in a review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeComment {
    pub id: u64,
    pub author: String,
    pub is_bot: bool,
    pub body: String,
    pub url: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub in_review: bool,
}

/// A submitted review of a change request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeReview {
    pub id: u64,
    pub author: String,
    pub is_bot: bool,
    /// `approved`, `changes_requested`, `commented` (pending and dismissed ones are left out).
    pub state: String,
    pub body: String,
    pub url: String,
}

/// A failed CI job and the end of its log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLog {
    pub name: String,
    pub url: String,
    /// The log's last lines (see the forge for how many), timestamps dropped.
    pub log: String,
}

/// A forge call that failed. The text is the forge's own (e.g. "GitHub is not connected").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    /// The forge can't be reached at all (not connected, or its login was refused).
    #[error("{0}")]
    Unavailable(String),
    /// The call failed, timed out or gave an answer that could not be read.
    #[error("{0}")]
    Failed(String),
    /// The forge doesn't do this (a capability it lacks).
    #[error("{0}")]
    Unsupported(String),
}

fn unsupported<T>(forge: &str, what: &str) -> Result<T, ForgeError> {
    Err(ForgeError::Unsupported(format!("{forge} has no {what}")))
}

/// A forge. `repo` is the forge's name for the repository (GitHub's `owner/name`).
pub trait Forge: Send + Sync {
    /// Its name in messages (`GitHub`).
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// What it adds to agents' instructions (how to read CI, answer a comment).
    fn agent_notes(&self) -> ForgeNotes {
        ForgeNotes::default()
    }
    /// Where a repository keeps this forge's CI configuration (GitHub: `.github/workflows`): a commit with files under
    /// one has CI to come even before its first run.
    fn ci_dirs(&self) -> &'static [&'static str] {
        &[]
    }
    /// The user still has to sign in to it before PDT can use it, and something wanted it: the Inbox asks them to.
    fn login_needed(&self) -> bool {
        false
    }
    /// Whether the repository has CI, when the forge knows without looking at its files (the local forge: its `[ci]`
    /// command). None: ask [`Forge::ci_dirs`].
    fn has_ci(&self) -> Option<bool> {
        None
    }
    /// The change request whose head is `branch`: the newest open one, else the newest.
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError>;
    /// Change request `number`; None when there is none.
    fn change_request_number(&self, _repo: &str, _number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
        unsupported(self.name(), "change requests by number")
    }
    /// Recent CI runs on `branch`.
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError>;
    /// The names of the failed jobs of one run attempt.
    fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, ForgeError>;
    /// The failed jobs of run `run_id`'s latest attempt, each with the end of its log.
    fn failed_logs(&self, _repo: &str, _run_id: u64) -> Result<Vec<JobLog>, ForgeError> {
        unsupported(self.name(), "CI logs")
    }
    /// The merge strategies `repo` allows and whether `base` has a merge queue.
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError>;
    /// Sets change request `number` to merge once it may (auto-merge, or the merge queue), with `strategy` unless a
    /// merge queue picks it. One that can merge right away is merged.
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError>;
    /// Recent merge-queue runs in `repo` ([`Capabilities::merge_queue`]).
    fn queue_runs(&self, _repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        unsupported(self.name(), "merge queue")
    }
    /// The login the forge is used as (the user themself): comments by it are the user's own.
    fn viewer_login(&self) -> Result<String, ForgeError> {
        unsupported(self.name(), "login")
    }
    /// Every comment on change request `number`: the conversation's and the reviews' line comments
    /// ([`Capabilities::reviews`]).
    fn change_comments(&self, _repo: &str, _number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        unsupported(self.name(), "reviews")
    }
    /// Adds a comment to change request `number`'s conversation; its URL ([`Capabilities::reviews`]).
    fn comment_change_request(&self, _repo: &str, _number: u32, _body: &str) -> Result<String, ForgeError> {
        unsupported(self.name(), "reviews")
    }
    /// Replies to review comment `comment_id` on change request `number`; its URL ([`Capabilities::reviews`]).
    fn reply_review_comment(
        &self,
        _repo: &str,
        _number: u32,
        _comment_id: u64,
        _body: &str,
    ) -> Result<String, ForgeError> {
        unsupported(self.name(), "reviews")
    }
    /// The submitted reviews of change request `number` ([`Capabilities::reviews`]).
    fn change_reviews(&self, _repo: &str, _number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        unsupported(self.name(), "reviews")
    }
    /// The newest issues of `repo` (change requests left out), newest first: one page without `after`, else back to
    /// issue `after` ([`Capabilities::issues`], as are the other issue calls).
    fn recent_issues(&self, _repo: &str, _after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// Issue `number`; None when there is none (or the forge can't see it).
    fn issue(&self, _repo: &str, _number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// The comments on issue `number`, oldest first.
    fn issue_comments(&self, _repo: &str, _number: u64) -> Result<Vec<ChangeComment>, ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// Adds a comment to issue `number`; its URL.
    fn comment_issue(&self, _repo: &str, _number: u64, _body: &str) -> Result<String, ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// Adds labels to issue `number`, keeping the ones it has.
    fn add_labels(&self, _repo: &str, _number: u64, _labels: &[String]) -> Result<(), ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// Closes issue `number` with `reason`; `duplicate_of` (the canonical issue's id, [`IssueDetail::id`]) with
    /// `duplicate`.
    fn close_issue(
        &self,
        _repo: &str,
        _number: u64,
        _reason: CloseReason,
        _duplicate_of: Option<u64>,
    ) -> Result<(), ForgeError> {
        unsupported(self.name(), "issues")
    }
}

pub mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory forge for other crates' tests: set a branch's change request and runs, its capabilities, count
    /// the calls.
    pub struct FakeForge {
        /// What `capabilities` answers (everything when unchanged).
        pub capabilities: Mutex<Capabilities>,
        /// What `agent_notes` answers.
        pub notes: Mutex<ForgeNotes>,
        pub prs: Mutex<HashMap<String, ChangeRequest>>,
        pub runs: Mutex<HashMap<String, Vec<CiRun>>>,
        pub queue: Mutex<Vec<CiRun>>,
        pub jobs: Mutex<HashMap<u64, Vec<String>>>,
        /// What `merge_settings` answers (squash only, no queue, when unset).
        pub settings: Mutex<Option<MergeSettings>>,
        /// `merge` fails with this.
        pub merge_error: Mutex<Option<String>>,
        /// `viewer_login`'s answer (`me` when unset).
        pub login: Mutex<Option<String>>,
        /// Reads of change requests and issues fail with this (signed out, the forge down).
        pub error: Mutex<Option<String>>,
        /// Issues by repo, newest first.
        pub issues: Mutex<HashMap<String, Vec<Issue>>>,
        /// Comments and reviews by change request number.
        pub comments: Mutex<HashMap<u32, Vec<ChangeComment>>>,
        pub reviews: Mutex<HashMap<u32, Vec<ChangeReview>>>,
        /// Issues closed (by `close_issue` or [`FakeForge::set_closed`]): by repo and number, the reason and the
        /// duplicate's canonical issue id.
        pub closed: Mutex<HashMap<(String, u64), Closed>>,
        /// Failed jobs' logs by run id.
        pub logs: Mutex<HashMap<u64, Vec<JobLog>>>,
        /// Comments added to change requests: number, the review comment replied to, body.
        pub change_comments_added: Mutex<Vec<(u32, Option<u64>, String)>>,
        /// Comments added to issues: repo, number, body.
        pub issue_comments: Mutex<Vec<(String, u64, String)>>,
        /// Labels added to issues: repo, number, labels.
        pub added_labels: Mutex<Vec<(String, u64, Vec<String>)>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl Default for FakeForge {
        fn default() -> Self {
            FakeForge {
                capabilities: Mutex::new(Capabilities::ALL),
                notes: Mutex::new(ForgeNotes {
                    ci: Some("Read CI with `fake-forge runs {branch}`.".into()),
                    reply: Some("`fake-forge reply`".into()),
                    watch: Some("`fake-forge watch`".into()),
                }),
                prs: Default::default(),
                runs: Default::default(),
                queue: Default::default(),
                jobs: Default::default(),
                settings: Default::default(),
                merge_error: Default::default(),
                login: Default::default(),
                error: Default::default(),
                issues: Default::default(),
                comments: Default::default(),
                reviews: Default::default(),
                closed: Default::default(),
                logs: Default::default(),
                change_comments_added: Default::default(),
                issue_comments: Default::default(),
                added_labels: Default::default(),
                calls: Default::default(),
            }
        }
    }

    /// How an issue was closed: the reason, and the duplicate's canonical issue id.
    pub type Closed = (CloseReason, Option<u64>);

    /// The id the fake gives issue `number` ([`IssueDetail::id`]).
    pub fn fake_issue_id(number: u64) -> u64 {
        1_000_000 + number
    }

    impl FakeForge {
        /// A fake with only these capabilities.
        pub fn with(capabilities: Capabilities) -> Self {
            FakeForge { capabilities: Mutex::new(capabilities), ..Default::default() }
        }
        pub fn set_pr(&self, branch: &str, pr: Option<ChangeRequest>) {
            let mut m = self.prs.lock().unwrap();
            match pr {
                Some(p) => m.insert(branch.into(), p),
                None => m.remove(branch),
            };
        }
        pub fn set_runs(&self, branch: &str, runs: Vec<CiRun>) {
            self.runs.lock().unwrap().insert(branch.into(), runs);
        }
        pub fn set_jobs(&self, run_id: u64, jobs: &[&str]) {
            self.jobs.lock().unwrap().insert(run_id, jobs.iter().map(|j| j.to_string()).collect());
        }
        pub fn set_issues(&self, repo: &str, issues: Vec<Issue>) {
            self.issues.lock().unwrap().insert(repo.into(), issues);
        }
        pub fn set_comments(&self, pr: u32, comments: Vec<ChangeComment>) {
            self.comments.lock().unwrap().insert(pr, comments);
        }
        pub fn set_reviews(&self, pr: u32, reviews: Vec<ChangeReview>) {
            self.reviews.lock().unwrap().insert(pr, reviews);
        }
        pub fn set_closed(&self, repo: &str, number: u64, reason: CloseReason) {
            self.closed.lock().unwrap().insert((repo.into(), number), (reason, None));
        }
        /// Issue `number`'s comments so far.
        pub fn comments_on(&self, repo: &str, number: u64) -> Vec<String> {
            let all = self.issue_comments.lock().unwrap();
            all.iter().filter(|(r, n, _)| r == repo && *n == number).map(|(_, _, b)| b.clone()).collect()
        }
        fn call(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }
        fn failing(&self) -> Result<(), ForgeError> {
            match self.error.lock().unwrap().clone() {
                Some(e) => Err(ForgeError::Failed(e)),
                None => Ok(()),
            }
        }
        /// Like a real forge, the fake refuses what it can't do.
        fn can(&self, on: impl Fn(&Capabilities) -> bool, what: &str) -> Result<(), ForgeError> {
            if on(&self.capabilities.lock().unwrap()) {
                Ok(())
            } else {
                Err(ForgeError::Unsupported(format!("the fake forge has no {what}")))
            }
        }
    }

    impl Forge for FakeForge {
        fn name(&self) -> &'static str {
            "the fake forge"
        }
        fn capabilities(&self) -> Capabilities {
            *self.capabilities.lock().unwrap()
        }
        fn agent_notes(&self) -> ForgeNotes {
            self.notes.lock().unwrap().clone()
        }
        fn ci_dirs(&self) -> &'static [&'static str] {
            &[".github/workflows"]
        }
        fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
            self.call(format!("pr {repo} {branch}"));
            self.failing()?;
            Ok(self.prs.lock().unwrap().get(branch).cloned())
        }
        fn change_request_number(&self, repo: &str, number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
            self.call(format!("pr {repo} #{number}"));
            self.failing()?;
            Ok(self.prs.lock().unwrap().values().find(|p| p.number == number).cloned())
        }
        fn failed_logs(&self, repo: &str, run_id: u64) -> Result<Vec<JobLog>, ForgeError> {
            self.call(format!("logs {repo} {run_id}"));
            self.failing()?;
            Ok(self.logs.lock().unwrap().get(&run_id).cloned().unwrap_or_default())
        }
        fn comment_change_request(&self, repo: &str, number: u32, body: &str) -> Result<String, ForgeError> {
            self.call(format!("comment-pr {repo} {number}"));
            self.can(|c| c.reviews, "reviews")?;
            self.failing()?;
            let mut all = self.change_comments_added.lock().unwrap();
            all.push((number, None, body.into()));
            Ok(format!("https://github.com/{repo}/pull/{number}#issuecomment-{}", all.len()))
        }
        fn reply_review_comment(
            &self,
            repo: &str,
            number: u32,
            comment_id: u64,
            body: &str,
        ) -> Result<String, ForgeError> {
            self.call(format!("reply {repo} {number} {comment_id}"));
            self.can(|c| c.reviews, "reviews")?;
            self.failing()?;
            let mut all = self.change_comments_added.lock().unwrap();
            all.push((number, Some(comment_id), body.into()));
            Ok(format!("https://github.com/{repo}/pull/{number}#discussion_r{}", all.len()))
        }
        fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<ChangeComment>, ForgeError> {
            self.call(format!("issue-comments {repo} {number}"));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            Ok(self
                .comments_on(repo, number)
                .into_iter()
                .enumerate()
                .map(|(i, body)| ChangeComment {
                    id: i as u64 + 1,
                    author: "me".into(),
                    is_bot: false,
                    body,
                    url: String::new(),
                    path: None,
                    line: None,
                    in_review: false,
                })
                .collect())
        }
        fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
            self.call(format!("runs {repo} {branch}"));
            Ok(self.runs.lock().unwrap().get(branch).cloned().unwrap_or_default())
        }
        fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
            self.call(format!("queue {repo}"));
            self.can(|c| c.merge_queue, "merge queue")?;
            Ok(self.queue.lock().unwrap().clone())
        }
        fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, ForgeError> {
            self.call(format!("jobs {repo} {run_id}.{attempt}"));
            Ok(self.jobs.lock().unwrap().get(&run_id).cloned().unwrap_or_default())
        }
        fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError> {
            self.call(format!("merge-settings {repo} {base}"));
            Ok(self.settings.lock().unwrap().unwrap_or(MergeSettings {
                squash: true,
                merge: false,
                rebase: false,
                queue: false,
            }))
        }
        fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
            self.call(format!("merge {repo} {number} {}", strategy.map_or("-", MergeStrategy::as_str)));
            match self.merge_error.lock().unwrap().clone() {
                Some(e) => Err(ForgeError::Failed(e)),
                None => Ok(()),
            }
        }
        fn viewer_login(&self) -> Result<String, ForgeError> {
            Ok(self.login.lock().unwrap().clone().unwrap_or_else(|| "me".into()))
        }
        fn recent_issues(&self, repo: &str, _after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
            self.call(format!("issues {repo}"));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            Ok(self.issues.lock().unwrap().get(repo).cloned().unwrap_or_default())
        }
        fn change_comments(&self, repo: &str, number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
            self.call(format!("comments {repo} {number}"));
            self.can(|c| c.reviews, "reviews")?;
            Ok(self.comments.lock().unwrap().get(&number).cloned().unwrap_or_default())
        }
        fn change_reviews(&self, repo: &str, number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
            self.call(format!("reviews {repo} {number}"));
            self.can(|c| c.reviews, "reviews")?;
            Ok(self.reviews.lock().unwrap().get(&number).cloned().unwrap_or_default())
        }
        fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueDetail>, ForgeError> {
            self.call(format!("issue {repo} {number}"));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            let issues = self.issues.lock().unwrap();
            let Some(issue) = issues.get(repo).and_then(|l| l.iter().find(|i| i.number == number)).cloned() else {
                return Ok(None);
            };
            let closed = self.closed.lock().unwrap().get(&(repo.to_string(), number)).map(|(r, _)| *r);
            Ok(Some(IssueDetail {
                issue,
                id: fake_issue_id(number),
                closed: closed.is_some(),
                state_reason: closed.map(|r| r.as_str().to_string()),
                change_request: false,
            }))
        }
        fn comment_issue(&self, repo: &str, number: u64, body: &str) -> Result<String, ForgeError> {
            self.call(format!("comment-issue {repo} {number}"));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            let mut all = self.issue_comments.lock().unwrap();
            all.push((repo.into(), number, body.into()));
            Ok(format!("https://github.com/{repo}/issues/{number}#issuecomment-{}", all.len()))
        }
        fn add_labels(&self, repo: &str, number: u64, labels: &[String]) -> Result<(), ForgeError> {
            self.call(format!("labels {repo} {number} {}", labels.join(",")));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            self.added_labels.lock().unwrap().push((repo.into(), number, labels.to_vec()));
            Ok(())
        }
        fn close_issue(
            &self,
            repo: &str,
            number: u64,
            reason: CloseReason,
            duplicate_of: Option<u64>,
        ) -> Result<(), ForgeError> {
            self.call(format!("close {repo} {number} {}", reason.as_str()));
            self.can(|c| c.issues, "issues")?;
            self.failing()?;
            self.closed.lock().unwrap().insert((repo.into(), number), (reason, duplicate_of));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strategy_defers_to_a_queue_then_prefers_squash() {
        let s = |squash, merge, rebase, queue| MergeSettings { squash, merge, rebase, queue }.strategy();
        assert_eq!(s(true, true, true, true), None);
        assert_eq!(s(true, true, true, false), Some(MergeStrategy::Squash));
        assert_eq!(s(false, true, true, false), Some(MergeStrategy::Merge));
        assert_eq!(s(false, false, true, false), Some(MergeStrategy::Rebase));
        assert_eq!(s(false, false, false, false), None);
    }

    #[test]
    fn a_forge_without_a_capability_refuses_its_calls() {
        let f = testing::FakeForge::with(Capabilities::NONE);
        assert!(matches!(f.recent_issues("o/r", None), Err(ForgeError::Unsupported(_))));
        assert!(matches!(f.change_reviews("o/r", 1), Err(ForgeError::Unsupported(_))));
        assert!(matches!(f.queue_runs("o/r"), Err(ForgeError::Unsupported(_))));
        assert!(f.change_request("o/r", "b").is_ok(), "change requests and CI are every forge's");
    }
}
