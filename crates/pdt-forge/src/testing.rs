//! Fakes for tests and the demo data: [`FakeForge`], the CI fakes, and a fake forge plugin.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

pub use crate::ci::testing::FakeCi;
use pdt_core::pr::CiRun;

/// `forge` as GitHub, with its fake GitHub Actions ([`FakeForge::ci`]) as its CI.
pub fn fake_github(forge: &Arc<FakeForge>) -> HostedForge {
    HostedForge::github(forge.clone()).with_ci(forge.ci.clone())
}

/// An in-memory forge for other crates' tests: set a branch's change request, its capabilities, count the calls.
/// Its `ci` is a fake GitHub Actions ([`fake_github`] registers both).
pub struct FakeForge {
    /// What `capabilities` answers (everything when unchanged).
    pub capabilities: Mutex<Capabilities>,
    /// What `agent_notes` answers.
    pub notes: Mutex<ForgeNotes>,
    pub prs: Mutex<HashMap<String, ChangeRequest>>,
    /// Its CI (GitHub Actions).
    pub ci: Arc<FakeCi>,
    /// What `merge_settings` answers (squash only, no queue, auto-merge allowed, when unset).
    pub settings: Mutex<Option<MergeSettings>>,
    /// `merge` fails with this.
    pub merge_error: Mutex<Option<String>>,
    /// `viewer_login`'s answer (`me` when unset).
    pub login: Mutex<Option<String>>,
    /// Reads of change requests and issues fail with this (signed out, the forge down).
    pub error: Mutex<Option<String>>,
    /// Change requests by repo, whoever opened them, most recently updated first ([`Forge::recent_change_requests`]).
    pub repo_prs: Mutex<HashMap<String, Vec<RepoChangeRequest>>>,
    /// [`Forge::change_request_stats`] by number (none when unset).
    pub stats: Mutex<HashMap<u32, ChangeStats>>,
    /// Issues by repo, newest first.
    pub issues: Mutex<HashMap<String, Vec<Issue>>>,
    /// Comments and reviews by change request number.
    pub comments: Mutex<HashMap<u32, Vec<ChangeComment>>>,
    pub reviews: Mutex<HashMap<u32, Vec<ChangeReview>>>,
    /// Issues closed (by `close_issue` or [`FakeForge::set_closed`]): by repo and number, the reason and the
    /// duplicate's canonical issue id.
    pub closed: Mutex<HashMap<(String, u64), Closed>>,
    /// Comments added to change requests: number, the review comment replied to, body.
    pub change_comments_added: Mutex<Vec<(u32, Option<u64>, String)>>,
    /// Comments added to issues: repo, number, body.
    pub issue_comments: Mutex<Vec<(String, u64, String)>>,
    /// Issues opened: repo and what was asked (each is also added to [`FakeForge::issues`], numbered from 500).
    pub created_issues: Mutex<Vec<(String, NewIssue)>>,
    /// Labels added to issues: repo, number, labels.
    pub added_labels: Mutex<Vec<(String, u64, Vec<String>)>>,
    /// Change requests opened, in order (each is also set as its head branch's, numbered from 100).
    pub opened: Mutex<Vec<NewChangeRequest>>,
    /// Releases by repo, newest first (the first is the latest).
    pub releases: Mutex<HashMap<String, Vec<Release>>>,
    /// [`Forge::commits_ahead`] by (repo, base, head); None (unknown) when unset.
    pub ahead: Mutex<HashMap<(String, String, String), u64>>,
    /// [`Forge::change_request_files`] by number (none when unset).
    pub files: Mutex<HashMap<u32, Vec<ChangeFile>>>,
    /// Reviews submitted: repo, number, verdict, body.
    pub reviews_submitted: Mutex<Vec<(String, u32, ReviewVerdict, String)>>,
    /// Change requests closed: repo and number.
    pub closed_changes: Mutex<Vec<(String, u32)>>,
    /// Files by (repo, branch, path) ([`Forge::file`]); [`Forge::write_file`] writes here too.
    pub repo_files: Mutex<HashMap<(String, String, String), String>>,
    /// Branches made by [`Forge::create_branch`] (a branch with files counts too): repo and branch.
    pub branches: Mutex<Vec<(String, String)>>,
    /// [`Forge::default_branch`] by repo (`main` when unset).
    pub default_branches: Mutex<HashMap<String, String>>,
    /// Files written, in order: repo and the write.
    pub written: Mutex<Vec<(String, FileWrite)>>,
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
            ci: Default::default(),
            settings: Default::default(),
            merge_error: Default::default(),
            login: Default::default(),
            error: Default::default(),
            repo_prs: Default::default(),
            stats: Default::default(),
            issues: Default::default(),
            comments: Default::default(),
            reviews: Default::default(),
            closed: Default::default(),
            change_comments_added: Default::default(),
            issue_comments: Default::default(),
            created_issues: Default::default(),
            added_labels: Default::default(),
            opened: Default::default(),
            releases: Default::default(),
            ahead: Default::default(),
            files: Default::default(),
            reviews_submitted: Default::default(),
            closed_changes: Default::default(),
            repo_files: Default::default(),
            branches: Default::default(),
            default_branches: Default::default(),
            written: Default::default(),
            calls: Default::default(),
        }
    }
}

/// The fake's id of a file's text ([`RepoFile::id`]).
pub fn fake_file_id(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// How an issue was closed: the reason, and the duplicate's canonical issue id.
pub type Closed = (CloseReason, Option<u64>);

/// The start of the fake's merge-queue refs for change request `number` into `base` ([`Forge::queue_ref`]).
pub fn fake_queue_ref(base: &str, number: u32) -> String {
    format!("fake-queue/{base}/{number}/")
}

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
    /// Its CI's runs of `branch` ([`FakeCi::set_runs`]).
    pub fn set_runs(&self, branch: &str, runs: Vec<CiRun>) {
        self.ci.set_runs(branch, runs);
    }
    /// Its CI's failed jobs of run `run_id` ([`FakeCi::set_jobs`]).
    pub fn set_jobs(&self, run_id: impl ToString, jobs: &[&str]) {
        self.ci.set_jobs(&run_id.to_string(), jobs);
    }
    /// `repo`'s change requests ([`FakeForge::repo_prs`]), most recently updated first.
    pub fn set_repo_prs(&self, repo: &str, prs: Vec<RepoChangeRequest>) {
        self.repo_prs.lock().unwrap().insert(repo.into(), prs);
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
    /// Publishes `release` in `repo`: it becomes the latest.
    pub fn publish(&self, repo: &str, release: Release) {
        self.releases.lock().unwrap().entry(repo.into()).or_default().insert(0, release);
    }
    pub fn set_ahead(&self, repo: &str, base: &str, head: &str, commits: u64) {
        self.ahead.lock().unwrap().insert((repo.into(), base.into(), head.into()), commits);
    }
    /// Issue `number`'s comments so far.
    pub fn comments_on(&self, repo: &str, number: u64) -> Vec<String> {
        let all = self.issue_comments.lock().unwrap();
        all.iter().filter(|(r, n, _)| r == repo && *n == number).map(|(_, _, b)| b.clone()).collect()
    }
    /// Puts a file on a branch.
    pub fn set_file(&self, repo: &str, branch: &str, path: &str, text: &str) {
        self.repo_files.lock().unwrap().insert((repo.into(), branch.into(), path.into()), text.into());
    }
    /// A file on a branch.
    pub fn file_text(&self, repo: &str, branch: &str, path: &str) -> Option<String> {
        self.repo_files.lock().unwrap().get(&(repo.into(), branch.into(), path.into())).cloned()
    }
    fn has_branch(&self, repo: &str, branch: &str) -> bool {
        self.branches.lock().unwrap().iter().any(|(r, b)| r == repo && b == branch)
            || self.repo_files.lock().unwrap().keys().any(|(r, b, _)| r == repo && b == branch)
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
    fn queue_name(&self) -> &'static str {
        "fake queue"
    }
    /// `fake-queue/<base>/<number>/`, with merge queues ([`Capabilities::merge_queue`]).
    fn queue_ref(&self, base: &str, number: u32) -> Option<String> {
        self.capabilities().merge_queue.then(|| fake_queue_ref(base, number))
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
    fn open_change_request(&self, repo: &str, new: &NewChangeRequest) -> Result<OpenedChangeRequest, ForgeError> {
        self.call(format!("open {repo} {} -> {}", new.head, new.base));
        self.failing()?;
        let mut opened = self.opened.lock().unwrap();
        opened.push(new.clone());
        let number = 99 + opened.len() as u32;
        let url = format!("https://github.com/{repo}/pull/{number}");
        let pr = ChangeRequest {
            number,
            url: url.clone(),
            title: new.title.clone(),
            state: PrState::Open,
            draft: new.draft,
            head_sha: String::new(),
            base_branch: new.base.clone(),
            mergeable: Mergeable::Unknown,
            behind: false,
            auto_merge: false,
            in_merge_queue: false,
            queue_position: None,
            queue_removal: None,
            created_at: None,
        };
        self.prs.lock().unwrap().insert(new.head.clone(), pr);
        Ok(OpenedChangeRequest { number, url })
    }
    fn comment_change_request(&self, repo: &str, number: u32, body: &str) -> Result<String, ForgeError> {
        self.call(format!("comment-pr {repo} {number}"));
        self.can(|c| c.reviews, "reviews")?;
        self.failing()?;
        let mut all = self.change_comments_added.lock().unwrap();
        all.push((number, None, body.into()));
        Ok(format!("https://github.com/{repo}/pull/{number}#issuecomment-{}", all.len()))
    }
    fn reply_review_comment(&self, repo: &str, number: u32, comment_id: u64, body: &str) -> Result<String, ForgeError> {
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
                collaborator: true,
            })
            .collect())
    }
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError> {
        self.call(format!("merge-settings {repo} {base}"));
        Ok(self.settings.lock().unwrap().unwrap_or(MergeSettings {
            squash: true,
            merge: false,
            rebase: false,
            queue: false,
            auto_merge: true,
        }))
    }
    /// Refuses a head other than the change request's, as GitHub does.
    fn merge_head(
        &self,
        repo: &str,
        number: u32,
        strategy: Option<MergeStrategy>,
        head: Option<&str>,
    ) -> Result<(), ForgeError> {
        if let Some(h) = head {
            let now = self.prs.lock().unwrap().values().find(|p| p.number == number).map(|p| p.head_sha.clone());
            if now.is_some_and(|n| n != h) {
                self.call(format!("merge-refused {repo} {number} {h}"));
                return Err(ForgeError::Failed(format!("the head of #{number} is not {h}")));
            }
        }
        self.merge(repo, number, strategy)
    }
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        self.call(format!("merge {repo} {number} {}", strategy.map_or("-", MergeStrategy::as_str)));
        if let Some(e) = self.merge_error.lock().unwrap().clone() {
            return Err(ForgeError::Failed(e));
        }
        // A repository with neither auto-merge nor a merge queue merges at once, as GitHub does (PDT-297).
        if self.settings.lock().unwrap().is_some_and(|s| !s.auto_merge && !s.queue) {
            for p in self.prs.lock().unwrap().values_mut().filter(|p| p.number == number) {
                p.state = PrState::Merged;
            }
        }
        Ok(())
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
    /// All of them, or those updated at or after `since`.
    fn recent_change_requests(&self, repo: &str, since: Option<&str>) -> Result<Vec<RepoChangeRequest>, ForgeError> {
        self.call(format!("pulls {repo}"));
        self.failing()?;
        let all = self.repo_prs.lock().unwrap().get(repo).cloned().unwrap_or_default();
        Ok(all.into_iter().filter(|p| since.is_none_or(|s| p.updated_at.as_str() >= s)).collect())
    }
    fn change_request_stats(&self, repo: &str, number: u32) -> Result<ChangeStats, ForgeError> {
        self.call(format!("stats {repo} {number}"));
        self.failing()?;
        Ok(self.stats.lock().unwrap().get(&number).copied().unwrap_or_default())
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
    fn change_request_files(&self, repo: &str, number: u32) -> Result<Vec<ChangeFile>, ForgeError> {
        self.call(format!("files {repo} {number}"));
        self.failing()?;
        Ok(self.files.lock().unwrap().get(&number).cloned().unwrap_or_default())
    }
    fn review_change_request(
        &self,
        repo: &str,
        number: u32,
        verdict: ReviewVerdict,
        body: &str,
    ) -> Result<String, ForgeError> {
        self.call(format!("review {repo} {number} {}", verdict.as_str()));
        self.can(|c| c.reviews, "reviews")?;
        self.failing()?;
        let mut all = self.reviews_submitted.lock().unwrap();
        all.push((repo.into(), number, verdict, body.into()));
        Ok(format!("https://github.com/{repo}/pull/{number}#pullrequestreview-{}", all.len()))
    }
    /// Closes it in [`FakeForge::prs`] too.
    fn close_change_request(&self, repo: &str, number: u32) -> Result<(), ForgeError> {
        self.call(format!("close-pr {repo} {number}"));
        self.failing()?;
        self.closed_changes.lock().unwrap().push((repo.into(), number));
        for p in self.prs.lock().unwrap().values_mut().filter(|p| p.number == number) {
            p.state = PrState::Closed;
        }
        Ok(())
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
    fn create_issue(&self, repo: &str, new: &NewIssue) -> Result<Issue, ForgeError> {
        self.call(format!("create-issue {repo}"));
        self.can(|c| c.issues, "issues")?;
        self.failing()?;
        let mut created = self.created_issues.lock().unwrap();
        created.push((repo.into(), new.clone()));
        let number = 499 + created.len() as u64;
        let issue = Issue {
            number,
            title: new.title.clone(),
            body: new.body.clone(),
            url: format!("https://github.com/{repo}/issues/{number}"),
            author: self.viewer_login()?,
            labels: new.labels.clone(),
            created_at: String::new(),
        };
        self.issues.lock().unwrap().entry(repo.into()).or_default().insert(0, issue.clone());
        Ok(issue)
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
    fn release(&self, repo: &str, tag: Option<&str>) -> Result<Option<Release>, ForgeError> {
        self.call(format!("release {repo} {}", tag.unwrap_or("latest")));
        self.can(|c| c.releases, "releases")?;
        self.failing()?;
        let all = self.releases.lock().unwrap();
        let list = all.get(repo).map(Vec::as_slice).unwrap_or_default();
        Ok(match tag {
            Some(t) => list.iter().find(|r| r.tag == t).cloned(),
            None => list.first().cloned(),
        })
    }
    fn newest_release(&self, repo: &str) -> Result<Option<Release>, ForgeError> {
        self.call(format!("newest release {repo}"));
        self.can(|c| c.releases, "releases")?;
        self.failing()?;
        Ok(self.releases.lock().unwrap().get(repo).and_then(|l| l.first().cloned()))
    }
    fn recent_releases(&self, repo: &str) -> Result<Vec<Release>, ForgeError> {
        self.call(format!("recent releases {repo}"));
        self.can(|c| c.releases, "releases")?;
        self.failing()?;
        Ok(self.releases.lock().unwrap().get(repo).cloned().unwrap_or_default())
    }
    fn commits_ahead(&self, repo: &str, base: &str, head: &str) -> Result<Option<u64>, ForgeError> {
        self.call(format!("compare {repo} {base}...{head}"));
        self.failing()?;
        Ok(self.ahead.lock().unwrap().get(&(repo.into(), base.into(), head.into())).copied())
    }
    fn default_branch(&self, repo: &str) -> Result<String, ForgeError> {
        self.call(format!("default branch {repo}"));
        self.failing()?;
        Ok(self.default_branches.lock().unwrap().get(repo).cloned().unwrap_or_else(|| "main".into()))
    }
    /// A file's id is a hash of its text.
    fn file(&self, repo: &str, path: &str, branch: &str) -> Result<Option<RepoFile>, ForgeError> {
        self.call(format!("file {repo} {branch} {path}"));
        self.failing()?;
        Ok(self.file_text(repo, branch, path).map(|text| RepoFile { id: fake_file_id(&text), text }))
    }
    /// The new branch gets a copy of `from`'s files.
    fn create_branch(&self, repo: &str, branch: &str, from: &str) -> Result<bool, ForgeError> {
        self.call(format!("create branch {repo} {branch} from {from}"));
        self.failing()?;
        if self.has_branch(repo, branch) {
            return Ok(false);
        }
        if !self.has_branch(repo, from) {
            return Err(ForgeError::Failed(format!("{repo} has no branch {from}")));
        }
        let mut files = self.repo_files.lock().unwrap();
        let copies: Vec<_> = files
            .iter()
            .filter(|((r, b, _), _)| r == repo && b == from)
            .map(|((r, _, p), t)| ((r.clone(), branch.to_string(), p.clone()), t.clone()))
            .collect();
        files.extend(copies);
        self.branches.lock().unwrap().push((repo.into(), branch.into()));
        Ok(true)
    }
    /// Refused when the file isn't the version `replaces` names, like a real forge.
    fn write_file(&self, repo: &str, write: &FileWrite) -> Result<String, ForgeError> {
        self.call(format!("write file {repo} {} {}", write.branch, write.path));
        self.failing()?;
        let now = self.file_text(repo, &write.branch, &write.path).map(|t| fake_file_id(&t));
        if now != write.replaces {
            return Err(ForgeError::Failed(format!("{} changed since it was read", write.path)));
        }
        self.set_file(repo, &write.branch, &write.path, &write.text);
        let mut written = self.written.lock().unwrap();
        written.push((repo.into(), write.clone()));
        Ok(format!("commit{}", written.len()))
    }
}
