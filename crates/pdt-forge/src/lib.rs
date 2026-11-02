//! Forges (PDT-176): where a project's change requests (GitHub's pull requests, GitLab's merge requests), their CI,
//! reviews and comments, merges and issues live. The SDLC talks to a [`Forge`] and asks its [`Capabilities`] before a
//! step only some forges can do; `pdt-github` is the GitHub one. Blocking; call from `spawn_blocking` in async code.
//!
//! Forges are plugins (PDT-241): each registers a [`ForgePlugin`] (its [`ForgeKind`], the remote hosts it claims, and
//! the [`Forge`] it opens) in a [`ForgeRegistry`], which PDT asks for a project's forge.
//!
//! CI is not the forge's (PDT-242): a [`ci::CiProvider`] reads it, and a forge plugin registers its own CI (GitHub
//! Actions) with [`ForgePlugin::ci`].

pub mod ci;
pub mod login;
pub mod transport;
pub use ci::{CiCapabilities, CiConfig, CiProvider};
pub use login::{ForgeAccount, ForgeLogin};

pub use pdt_core::forge::{ForgeKind, ForgeLinks, ForgeNotes};
pub use pdt_core::github_issue::CloseReason;
use pdt_core::pr::{Mergeable, PrState, QueueRemoval};
use std::sync::Arc;

/// What a forge can do beyond reading a branch's change request, so the SDLC skips (or does itself) what it can't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A base branch can have a merge queue (its CI's queue runs are read while a change request is queued).
    pub merge_queue: bool,
    /// The forge holds an auto-merge until the base branch's required checks pass. Without it, PDT's own merges wait
    /// for green CI.
    pub required_checks: bool,
    /// Issues: listing, reading, opening, commenting, labelling, closing.
    pub issues: bool,
    /// Reviews and comments on change requests.
    pub reviews: bool,
    /// Releases to read (PDT-194: the release workflow).
    pub releases: bool,
}

impl Capabilities {
    /// Everything (GitHub).
    pub const ALL: Capabilities =
        Capabilities { merge_queue: true, required_checks: true, issues: true, reviews: true, releases: true };
    /// Change requests only.
    pub const NONE: Capabilities =
        Capabilities { merge_queue: false, required_checks: false, issues: false, reviews: false, releases: false };
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
    /// The forge can hold a merge until the change request's checks pass (GitHub's auto-merge, when the repository
    /// allows it; GitLab's merge when the pipeline succeeds; the local forge). Without it PDT holds the merge itself:
    /// it merges once CI on the head passes (PDT-297).
    pub auto_merge: bool,
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

/// A change request to open ([`Forge::open_change_request`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChangeRequest {
    /// The branch with the change (pushed).
    pub head: String,
    /// The branch it merges into.
    pub base: String,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub draft: bool,
}

/// A change request just opened: its number and URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedChangeRequest {
    pub number: u32,
    pub url: String,
}

/// A change request in a repository's list ([`Forge::recent_change_requests`]): whoever opened it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoChangeRequest {
    pub number: u32,
    pub title: String,
    pub body: String,
    pub url: String,
    pub author: String,
    pub is_bot: bool,
    /// Its author is an insider on the repository, as on [`ChangeComment`].
    pub collaborator: bool,
    /// The repository its branch is in (`owner/name`); None when that is gone (a deleted fork).
    pub head_repo: Option<String>,
    pub head_branch: String,
    pub head_sha: String,
    pub base_branch: String,
    pub labels: Vec<String>,
    pub state: PrState,
    pub draft: bool,
    /// The forge's timestamps (RFC 3339, UTC: they sort as text).
    pub created_at: String,
    pub updated_at: String,
}

/// How big a change request is ([`Forge::change_request_stats`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeStats {
    pub changed_files: u64,
    pub additions: u64,
    pub deletions: u64,
}

/// One file a change request changes ([`Forge::change_request_files`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFile {
    pub path: String,
    /// `added`, `modified`, `removed`, `renamed`, … (the forge's word).
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    /// Its unified diff hunks; None for a binary file or one too large for the forge to show.
    pub patch: Option<String>,
    /// A renamed file's old path.
    pub previous_path: Option<String>,
}

/// What a review submitted on a change request says ([`Forge::review_change_request`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewVerdict {
    Approve,
    RequestChanges,
    Comment,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewVerdict::Approve => "approve",
            ReviewVerdict::RequestChanges => "request_changes",
            ReviewVerdict::Comment => "comment",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [ReviewVerdict::Approve, ReviewVerdict::RequestChanges, ReviewVerdict::Comment]
            .into_iter()
            .find(|v| v.as_str() == s.trim())
    }
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

/// An issue to open ([`Forge::create_issue`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub title: String,
    /// Markdown.
    pub body: String,
    /// Labels to put on it (a forge may drop ones the repository doesn't have).
    pub labels: Vec<String>,
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
    /// Its author is the repository's owner, a member of its organization or a collaborator on it (PDT-202): an
    /// insider. Comments by anyone else are never sent into agent sessions.
    pub collaborator: bool,
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
    /// As on [`ChangeComment`].
    pub collaborator: bool,
}

/// A published release (never a draft; a pre-release only where a call says so).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// Its tag (`v0.2.0`).
    pub tag: String,
    pub name: String,
    pub url: String,
    /// When it was published (the forge's timestamp text), when known.
    pub published_at: Option<String>,
    /// Its notes (Markdown).
    pub notes: String,
    /// Published as a pre-release.
    pub prerelease: bool,
    /// Its files (PDT-365).
    pub assets: Vec<ReleaseAsset>,
}

/// A file attached to a release.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    /// Where it downloads from.
    pub url: String,
    /// Bytes, when the forge says.
    pub size: Option<u64>,
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
    /// A merge the forge can't make yet and can't hold for its checks (PDT-297: GitHub refuses auto-merge for a pull
    /// request whose checks aren't required, while they run). PDT holds it: it merges once CI passes.
    #[error("{0}")]
    NotReady(String),
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
    /// The user still has to sign in to it before PDT can use it, and something wanted it: the Inbox asks them to.
    fn login_needed(&self) -> bool {
        false
    }
    /// When the forge stopped accepting the login it has (Unix ms; PDT-212): the user has to sign in again, and
    /// nothing is sent to it until they do. [`Forge::login_needed`] is true while it lasts.
    fn login_rejected_at(&self) -> Option<i64> {
        None
    }
    /// The change request whose head is `branch`: the newest open one, else the newest.
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError>;
    /// Change request `number`; None when there is none.
    fn change_request_number(&self, _repo: &str, _number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
        unsupported(self.name(), "change requests by number")
    }
    /// Opens a change request from `new.head` into `new.base` (PDT-185). A forge that opens them itself (the local one)
    /// has none to open.
    fn open_change_request(&self, _repo: &str, _new: &NewChangeRequest) -> Result<OpenedChangeRequest, ForgeError> {
        unsupported(self.name(), "change requests to open")
    }
    /// The merge strategies `repo` allows and whether `base` has a merge queue.
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError>;
    /// What it calls a base branch's merge queue, for people (`merge queue`; GitLab's `merge train`).
    fn queue_name(&self) -> &'static str {
        "merge queue"
    }
    /// The start of the refs (branches) its merge queue tests change request `number` into `base` on: CI runs on a
    /// branch starting with it are the change request's merge-queue runs. None without a merge queue (or when its
    /// runs can't be told apart).
    fn queue_ref(&self, _base: &str, _number: u32) -> Option<String> {
        None
    }
    /// Sets change request `number` to merge once it may (auto-merge, or the merge queue), with `strategy` unless a
    /// merge queue picks it. One that can merge right away is merged.
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError>;
    /// [`Forge::merge`], only while its head is `head` (PDT-269: the commit the user approved), when given. A forge
    /// that can't say so merges as [`Forge::merge`] does; its callers check the head first.
    fn merge_head(
        &self,
        repo: &str,
        number: u32,
        strategy: Option<MergeStrategy>,
        _head: Option<&str>,
    ) -> Result<(), ForgeError> {
        self.merge(repo, number, strategy)
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
    /// The change requests of `repo`, whoever opened them, most recently updated first: one page without `since`,
    /// else back to the first one updated before `since` (a [`RepoChangeRequest::updated_at`]).
    fn recent_change_requests(&self, _repo: &str, _since: Option<&str>) -> Result<Vec<RepoChangeRequest>, ForgeError> {
        unsupported(self.name(), "change request lists")
    }
    /// How many files, added and deleted lines change request `number` has.
    fn change_request_stats(&self, _repo: &str, _number: u32) -> Result<ChangeStats, ForgeError> {
        unsupported(self.name(), "change request lists")
    }
    /// The files change request `number` changes, with their diffs (PDT-269: an outside change request is reviewed
    /// from these, never checked out).
    fn change_request_files(&self, _repo: &str, _number: u32) -> Result<Vec<ChangeFile>, ForgeError> {
        unsupported(self.name(), "change request diffs")
    }
    /// Submits a review of change request `number` with `body` (Markdown); its URL ([`Capabilities::reviews`]).
    fn review_change_request(
        &self,
        _repo: &str,
        _number: u32,
        _verdict: ReviewVerdict,
        _body: &str,
    ) -> Result<String, ForgeError> {
        unsupported(self.name(), "reviews")
    }
    /// Closes change request `number` without merging it.
    fn close_change_request(&self, _repo: &str, _number: u32) -> Result<(), ForgeError> {
        unsupported(self.name(), "change requests to close")
    }
    /// Issue `number`; None when there is none (or the forge can't see it).
    fn issue(&self, _repo: &str, _number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        unsupported(self.name(), "issues")
    }
    /// Opens an issue in `repo`, as the forge's login; the issue as opened.
    fn create_issue(&self, _repo: &str, _new: &NewIssue) -> Result<Issue, ForgeError> {
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
    /// The release tagged `tag`, or the latest one without it; None when there is none ([`Capabilities::releases`]).
    fn release(&self, _repo: &str, _tag: Option<&str>) -> Result<Option<Release>, ForgeError> {
        unsupported(self.name(), "releases")
    }
    /// The newest published release, pre-releases included (a nightly build, PDT-283); None when there is none
    /// ([`Capabilities::releases`]).
    fn newest_release(&self, _repo: &str) -> Result<Option<Release>, ForgeError> {
        unsupported(self.name(), "releases")
    }
    /// The recent published releases, newest first, pre-releases included (never drafts; PDT-288)
    /// ([`Capabilities::releases`]).
    fn recent_releases(&self, _repo: &str) -> Result<Vec<Release>, ForgeError> {
        unsupported(self.name(), "releases")
    }
    /// How many commits `head` has that `base` lacks (each a tag, a branch or a commit); None when either is unknown
    /// (PDT-288: whether main changed since a release).
    fn commits_ahead(&self, _repo: &str, _base: &str, _head: &str) -> Result<Option<u64>, ForgeError> {
        unsupported(self.name(), "comparisons")
    }
}

/// The host of a GitHub pull request's page (`https://<host>/<owner>/<repo>/pull/<n>`); None for any other URL.
pub fn github_pull_url(url: &str) -> Option<String> {
    let r = pdt_core::git_url::parse_remote(url)?;
    match r.path.as_slice() {
        [_, _, pull, n] if pull == "pull" && n.parse::<u64>().is_ok() => Some(r.host),
        _ => None,
    }
}

/// A forge plugin (PDT-241): a kind of forge, the remotes it handles, and the forge it opens for them.
pub trait ForgePlugin: Send + Sync {
    /// Its id, which `.pdt/project.toml`'s `forge = "…"` names.
    fn kind(&self) -> ForgeKind;
    /// Its name for people (`GitHub`).
    fn label(&self) -> &'static str;
    /// Whether a remote on `host` (lower case) is on this forge: github.com, and the GitHub Enterprise Server the
    /// login is for.
    fn claims_host(&self, host: &str) -> bool;
    /// The forge's name for the repository at `remote` (GitHub's `owner/name`); None when it isn't one of its own.
    fn repo_of(&self, remote: &pdt_core::git_url::Remote) -> Option<String> {
        if self.claims_host(&remote.host) { remote.owner_repo() } else { None }
    }
    /// What the forge calls a change request, for people (PDT-306): `pull request`, GitLab's `merge request`.
    fn change_request_name(&self) -> &'static str {
        "pull request"
    }
    /// Whether `url` is a change request's page on this forge (an external ticket's link to a pull request).
    fn is_change_request_url(&self, _url: &str) -> bool {
        false
    }
    /// Where the repository `repo` at `remote` is on the forge's website; None when it has none to link to.
    fn links(&self, _remote: &pdt_core::git_url::Remote, _repo: &str) -> Option<ForgeLinks> {
        None
    }
    /// The forge as its public host's account (github.com's), for a flow tied to no project.
    fn forge(&self) -> Arc<dyn Forge>;
    /// The forge's own CI (GitHub Actions), if it has one: a project on this forge has it unless `[ci] providers`
    /// leaves it out.
    fn ci(&self) -> Option<Arc<dyn CiProvider>> {
        None
    }
    /// The forge as the account for `host` (lower case; PDT-244): a project's, by its `origin`'s host.
    fn forge_on(&self, _host: &str) -> Arc<dyn Forge> {
        self.forge()
    }
    /// The forge's own CI as the account for `host`.
    fn ci_on(&self, _host: &str) -> Option<Arc<dyn CiProvider>> {
        self.ci()
    }
    /// How the user signs in to it (PDT-244); None for a forge with no login.
    fn login(&self) -> Option<&dyn ForgeLogin> {
        None
    }
}

/// A plugin over a forge already made, on fixed hosts: a test's [`testing::FakeForge`] as GitHub, say.
pub struct HostedForge {
    pub kind: ForgeKind,
    pub label: &'static str,
    /// Lower case.
    pub hosts: Vec<String>,
    pub forge: Arc<dyn Forge>,
    /// Its own CI.
    pub ci: Option<Arc<dyn CiProvider>>,
}

impl HostedForge {
    /// `forge` as GitHub, on github.com, with no CI of its own.
    pub fn github(forge: Arc<dyn Forge>) -> Self {
        Self { kind: ForgeKind::GITHUB, label: "GitHub", hosts: vec!["github.com".into()], forge, ci: None }
    }

    /// With `ci` as its own CI.
    pub fn with_ci(self, ci: Arc<dyn CiProvider>) -> Self {
        Self { ci: Some(ci), ..self }
    }
}

impl ForgePlugin for HostedForge {
    fn kind(&self) -> ForgeKind {
        self.kind.clone()
    }
    fn label(&self) -> &'static str {
        self.label
    }
    fn claims_host(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
    }
    /// GitHub's `https://<host>/<owner>/<repo>/pull/<n>`.
    fn is_change_request_url(&self, url: &str) -> bool {
        github_pull_url(url).is_some_and(|host| self.claims_host(&host))
    }
    /// The repository's page at `https://<host>/<repo>`, without branch or compare pages.
    fn links(&self, remote: &pdt_core::git_url::Remote, repo: &str) -> Option<ForgeLinks> {
        Some(ForgeLinks {
            kind: self.kind.clone(),
            name: self.label.into(),
            repo: repo.into(),
            repo_url: format!("https://{}/{repo}", remote.host),
            branch_url: None,
            compare_url: None,
        })
    }
    fn forge(&self) -> Arc<dyn Forge> {
        self.forge.clone()
    }
    fn ci(&self) -> Option<Arc<dyn CiProvider>> {
        self.ci.clone()
    }
}

/// The forge plugins PDT has, in the order they are asked to claim a remote.
#[derive(Clone, Default)]
pub struct ForgeRegistry {
    plugins: Arc<[Arc<dyn ForgePlugin>]>,
}

impl ForgeRegistry {
    /// The registry of `plugins`; a kind registered twice keeps the first.
    pub fn new(plugins: Vec<Arc<dyn ForgePlugin>>) -> Self {
        let mut out: Vec<Arc<dyn ForgePlugin>> = vec![];
        for p in plugins {
            if !out.iter().any(|x| x.kind() == p.kind()) {
                out.push(p);
            }
        }
        Self { plugins: out.into() }
    }

    pub fn plugins(&self) -> &[Arc<dyn ForgePlugin>] {
        &self.plugins
    }

    /// The plugin of `kind`.
    pub fn get(&self, kind: &ForgeKind) -> Option<&Arc<dyn ForgePlugin>> {
        self.plugins.iter().find(|p| p.kind() == *kind)
    }

    /// The plugin that claims the remote at `url`, and its name for the repository.
    pub fn detect(&self, url: &str) -> Option<(&Arc<dyn ForgePlugin>, String)> {
        let remote = pdt_core::git_url::parse_remote(url)?;
        self.plugins.iter().find_map(|p| p.repo_of(&remote).map(|r| (p, r)))
    }
}

pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strategy_defers_to_a_queue_then_prefers_squash() {
        let s =
            |squash, merge, rebase, queue| MergeSettings { squash, merge, rebase, queue, auto_merge: true }.strategy();
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
        assert!(matches!(f.release("o/r", None), Err(ForgeError::Unsupported(_))));
        let new = NewIssue { title: "Crash".into(), body: String::new(), labels: vec![] };
        assert!(matches!(f.create_issue("o/r", &new), Err(ForgeError::Unsupported(_))));
        assert!(f.change_request("o/r", "b").is_ok(), "change requests are every forge's");
    }

    #[test]
    fn the_fake_has_queue_refs_only_with_a_merge_queue() {
        let queued = testing::FakeForge::default();
        assert_eq!(queued.queue_ref("main", 12).as_deref(), Some("fake-queue/main/12/"));
        assert_eq!(queued.queue_name(), "fake queue");
        let plain = testing::FakeForge::with(Capabilities { merge_queue: false, ..Capabilities::ALL });
        assert_eq!(plain.queue_ref("main", 12), None);
    }

    #[test]
    fn a_hosted_forge_links_the_repository_on_its_host() {
        let gitea = HostedForge {
            kind: ForgeKind::new("gitea").unwrap(),
            label: "Gitea",
            hosts: vec!["git.example.org".into()],
            forge: Arc::new(testing::FakeForge::default()),
            ci: None,
        };
        let remote = pdt_core::git_url::parse_remote("git@git.example.org:team/app.git").unwrap();
        let l = gitea.links(&remote, "team/app").unwrap();
        assert_eq!((l.name.as_str(), l.repo_url.as_str()), ("Gitea", "https://git.example.org/team/app"));
        assert_eq!((l.branch_url, l.compare_url), (None, None));
    }

    #[test]
    fn the_fake_opens_issues_it_can_then_read() {
        let f = testing::FakeForge::default();
        let new = NewIssue { title: "Crash".into(), body: "It fell over".into(), labels: vec!["bug".into()] };
        let i = f.create_issue("o/r", &new).unwrap();
        assert_eq!((i.number, i.url.as_str(), i.author.as_str()), (500, "https://github.com/o/r/issues/500", "me"));
        assert_eq!(f.issue("o/r", 500).unwrap().unwrap().issue, i);
        assert_eq!(*f.created_issues.lock().unwrap(), [("o/r".to_string(), new)]);
    }

    #[test]
    fn the_registry_finds_a_forge_by_kind_and_by_remote() {
        let gh: Arc<dyn Forge> = Arc::new(testing::FakeForge::default());
        let other: Arc<dyn Forge> = Arc::new(testing::FakeForge::default());
        let gitea = HostedForge {
            kind: ForgeKind::new("gitea").unwrap(),
            label: "Gitea",
            hosts: vec!["git.example.org".into()],
            forge: other,
            ci: None,
        };
        let twice = HostedForge::github(Arc::new(testing::FakeForge::default()));
        let r = ForgeRegistry::new(vec![Arc::new(HostedForge::github(gh.clone())), Arc::new(gitea), Arc::new(twice)]);
        assert_eq!(r.plugins().len(), 2, "a kind registered twice keeps the first");
        assert!(Arc::ptr_eq(&r.get(&ForgeKind::GITHUB).unwrap().forge(), &gh));
        assert!(r.get(&ForgeKind::LOCAL).is_none());
        assert!(r.get(&ForgeKind::GITHUB).unwrap().ci().is_none(), "no CI of its own unless given one");
        let fake = Arc::new(testing::FakeForge::default());
        assert_eq!(testing::fake_github(&fake).ci().unwrap().id(), "github_actions");
        let (p, repo) = r.detect("git@github.com:o/r.git").unwrap();
        assert_eq!((p.kind(), repo.as_str()), (ForgeKind::GITHUB, "o/r"));
        let (p, repo) = r.detect("https://GIT.example.org/team/app").unwrap();
        assert_eq!((p.label(), repo.as_str()), ("Gitea", "team/app"));
        assert!(r.detect("https://gitlab.com/o/r.git").is_none());
        assert!(r.detect("/srv/git/r.git").is_none());
    }
}
