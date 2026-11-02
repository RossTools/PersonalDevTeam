//! The GitHub forge (PDT-176, PDT-177): GitHub's REST and GraphQL APIs for a branch's pull request and its Actions
//! runs (PDT-25), merges, reviews, comments, issues and releases (PDT-194: reading them, starting the release workflow), signed in with the token of the user's in-app login (Settings
//! › Plugins › GitHub). No GitHub CLI. Check runs are not read: fine-grained tokens cannot see them, so CI comes from
//! the Actions runs API. GETs are conditional (ETags: an unchanged answer costs no rate limit), and a used-up rate
//! limit stops every call until it resets. Blocking; call from `spawn_blocking` in async code.

pub mod actions;
mod forge;
pub mod login;
pub mod plugin;
pub mod transport;

pub use actions::GithubActions;
use pdt_core::brand::SHORT_NAME;
use pdt_core::pr::{CiRun, Mergeable, PrState, QueueRemoval, QueueRemovalReason};
use pdt_forge::comment_body;
use pdt_forge::oauth::OAuthApp;
pub use pdt_forge::rest::{CallRecord, LoginCheck, RenewedHook, next_link};
use pdt_forge::rest::{Flavor, RestClient, RestError};
pub use pdt_forge::{
    Capabilities, ChangeComment, ChangeFile, ChangeRequest, ChangeReview, ChangeStats, CloseReason, FileWrite, Forge,
    ForgeError, ForgeNotes, Issue, IssueDetail, MAX_COMMENT, MergeSettings, MergeStrategy, NewChangeRequest, NewIssue,
    OpenedChangeRequest, Release, ReleaseAsset, RepoChangeRequest, RepoFile, ReviewVerdict,
};
use serde_json::{Value, json};
use std::sync::Arc;
use transport::{Response, Transport, encode};

/// How many recent runs are read per branch (and of merge-queue runs per repo).
const RUN_LIMIT: u32 = 30;

const PR_FIELDS: &str = "number url title state createdAt isDraft headRefOid baseRefName mergeable \
mergeStateStatus isInMergeQueue mergeQueueEntry{position} autoMergeRequest{enabledAt} \
timelineItems(itemTypes:[ADDED_TO_MERGE_QUEUE_EVENT,REMOVED_FROM_MERGE_QUEUE_EVENT],last:1){nodes{__typename \
...on RemovedFromMergeQueueEvent{id reason actor{login}}}}";

fn pr_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$head:String!){{repository(owner:$owner,name:$name){{\
         pullRequests(headRefName:$head,first:5,orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{{PR_FIELDS}}}}}}}}}"
    )
}

fn pr_number_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$number:Int!){{repository(owner:$owner,name:$name){{\
         pullRequest(number:$number){{{PR_FIELDS}}}}}}}"
    )
}

const MERGE_SETTINGS_QUERY: &str = "query($owner:String!,$name:String!,$base:String!){repository(owner:$owner,\
name:$name){squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed autoMergeAllowed mergeQueue(branch:$base){id}}}";

/// A pull request's node id, and whether its repository allows auto-merge (PDT-297: many don't).
const PR_ID_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){\
autoMergeAllowed pullRequest(number:$number){id}}}";

const AUTO_MERGE: &str = "mutation($id:ID!,$method:PullRequestMergeMethod){enablePullRequestAutoMerge(input:\
{pullRequestId:$id,mergeMethod:$method}){clientMutationId}}";

const ENQUEUE: &str = "mutation($id:ID!){enqueuePullRequest(input:{pullRequestId:$id}){clientMutationId}}";

/// [`AUTO_MERGE`] and [`ENQUEUE`] only while the head is still `$head` (PDT-269: the commit the user approved).
const AUTO_MERGE_AT: &str = "mutation($id:ID!,$method:PullRequestMergeMethod,$head:GitObjectID!){\
enablePullRequestAutoMerge(input:{pullRequestId:$id,mergeMethod:$method,expectedHeadOid:$head}){clientMutationId}}";
const ENQUEUE_AT: &str = "mutation($id:ID!,$head:GitObjectID!){enqueuePullRequest(input:{pullRequestId:$id,\
expectedHeadOid:$head}){clientMutationId}}";

/// GitHub's REST API, for the shared client: names, URLs, headers and error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Github;

impl Flavor for Github {
    const NAME: &'static str = "GitHub";
    /// `GET /rate_limit` costs no rate limit.
    const CHECK_PATH: &'static str = "rate_limit";
    /// GitHub refuses a call over its primary or secondary limit with a 403 or a 429.
    const LIMIT_REFUSALS: &'static [u16] = &[403, 429];
    const LIMIT_HEADERS: &'static str = "x-ratelimit-";
    const REFUSAL_HEADERS: &'static [&'static str] =
        &["x-github-request-id", "x-github-sso", "www-authenticate", "x-oauth-scopes"];
    const ETAGS: bool = true;
    fn api_url(given: &str) -> String {
        login::api_url(given)
    }
    fn web_host(api: &str) -> String {
        login::web_host(api)
    }
    fn headers(token: &str) -> Vec<(&'static str, String)> {
        headers(token)
    }
    fn message(res: &Response) -> String {
        message(res)
    }
    fn oauth(api: &str) -> OAuthApp {
        login::oauth_app(api)
    }
}

/// What went wrong talking to GitHub.
pub type GhError = RestError<Github>;

/// GitHub's part of agents' instructions: PDT's forge tools (pdt-mcp), not the GitHub CLI.
pub fn agent_notes() -> ForgeNotes {
    ForgeNotes {
        ci: Some(format!(
            "Read CI with {SHORT_NAME}'s tools: `ci_runs` (the runs on {{branch}} and their failed jobs) and \
             `ci_failed_logs` (the failed jobs' logs), or from the ticket's attempt (`pr.ci`: state, URL and runs, \
             when {SHORT_NAME} has seen the pull request). `change_request_view` shows the pull request with its \
             comments and reviews, `issue_view` an issue. Use these instead of the GitHub CLI (`gh`)."
        )),
        reply: Some("`change_request_comment`, with `replyTo` to answer a review comment".into()),
        watch: None,
    }
}

/// Where GitHub Actions workflows live.
pub const CI_DIRS: &[&str] = &[".github/workflows"];

/// What GitHub can do: everything.
pub const CAPABILITIES: Capabilities = Capabilities::ALL;

/// Issues (and pull requests, which share their numbers) per page of [`GithubApi::recent_issues`], and the most pages
/// one read takes.
pub const ISSUE_PAGE: usize = 100;
pub const ISSUE_PAGES: u32 = 10;

/// The most lines of a failed job's log kept, and of characters.
pub const LOG_LINES: usize = 200;
pub const LOG_CHARS: usize = 30_000;

/// The headers of an API call with `token`.
pub(crate) fn headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/vnd.github+json".into()),
        ("X-GitHub-Api-Version", "2022-11-28".into()),
    ]
}

/// The login PDT uses: a token and the API it is for (one per host, PDT-244).
pub type Account = pdt_forge::ForgeAccount;

/// The GitHub forge: the shared REST client ([`RestClient`], which it derefs to) and GitHub's GraphQL. With no
/// [`Account`] every call fails as [`GhError::NotConnected`].
pub struct GithubApi {
    rest: RestClient<Github>,
}

impl std::ops::Deref for GithubApi {
    type Target = RestClient<Github>;
    fn deref(&self) -> &RestClient<Github> {
        &self.rest
    }
}

impl GithubApi {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { rest: RestClient::new(transport) }
    }

    /// No network and no login: `AppState::new`'s forge, so tests can never reach GitHub.
    pub fn offline() -> Self {
        Self { rest: RestClient::offline() }
    }

    /// A GraphQL query's `data`; its first error as [`GhError::Status`].
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, GhError> {
        let a = self.signed_in()?;
        let v =
            self.send_json("POST", &login::graphql_url(&a.api_url), json!({"query": query, "variables": variables}))?;
        graphql_data(v)
    }

    /// The pull request's node id, and whether its repository allows auto-merge (allowed when the answer doesn't say).
    fn pr_node_id(&self, repo: &str, number: u32) -> Result<(String, bool), GhError> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql(PR_ID_QUERY, json!({"owner": owner, "name": name, "number": number}))?;
        let id = d["repository"]["pullRequest"]["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| GhError::BadAnswer(format!("no pull request #{number}")))?;
        Ok((id, d["repository"]["autoMergeAllowed"].as_bool().unwrap_or(true)))
    }

    fn merge_now(
        &self,
        repo: &str,
        number: u32,
        strategy: Option<MergeStrategy>,
        head: Option<&str>,
    ) -> Result<(), GhError> {
        let mut body = match strategy {
            Some(s) => json!({"merge_method": s.as_str()}),
            None => json!({}),
        };
        if let Some(h) = head {
            body["sha"] = json!(h);
        }
        self.send_json("PUT", &format!("repos/{repo}/pulls/{number}/merge"), body).map(|_| ())
    }

    fn post_comment(&self, path: &str, body: &str) -> Result<String, GhError> {
        let v = self.send_json("POST", path, json!({"body": comment_body(body)}))?;
        Ok(s(&v, "html_url"))
    }
}

/// An answer's message (and its first error's), for errors and logs.
fn message(res: &Response) -> String {
    let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
    let mut m = v["message"].as_str().unwrap_or("no message").to_string();
    if let Some(e) = v["errors"].as_array().and_then(|e| e.first()) {
        let more = e["message"].as_str().or(e["code"].as_str()).unwrap_or_default();
        if !more.is_empty() {
            m = format!("{m} ({more})");
        }
    }
    m
}

fn owner_name(repo: &str) -> Result<(&str, &str), GhError> {
    repo.split_once('/').ok_or_else(|| GhError::BadAnswer(format!("not owner/name: {repo}")))
}

fn graphql_data(v: Value) -> Result<Value, GhError> {
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(GhError::Status(200, e["message"].as_str().unwrap_or("GraphQL error").to_string()));
    }
    match v.get("data") {
        Some(d) if !d.is_null() => Ok(d.clone()),
        _ => Err(GhError::BadAnswer("no data in the answer".into())),
    }
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

/// Whether an item's author is an insider on the repository (`author_association` OWNER, MEMBER or COLLABORATOR;
/// PDT-202). Without the field, they are not.
fn collaborator(v: &Value) -> bool {
    matches!(v["author_association"].as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

fn is_bot(user: &Value) -> bool {
    user["type"] == "Bot" || user["login"].as_str().is_some_and(|l| l.ends_with("[bot]"))
}

/// The newest open PR of a `pullRequests` list, else the newest one.
fn pick_pr(nodes: &[Value]) -> Option<&Value> {
    nodes.iter().find(|n| n["state"] == "OPEN").or_else(|| nodes.first())
}

/// The `pr_query` answer's `data`: the newest open PR, else the newest one.
pub fn parse_pull_requests(data: &Value) -> Result<Option<ChangeRequest>, GhError> {
    let nodes = data["repository"]["pullRequests"]["nodes"]
        .as_array()
        .ok_or_else(|| GhError::BadAnswer("no pullRequests in the answer".into()))?;
    Ok(pick_pr(nodes).map(parse_pr))
}

/// One pull request node.
pub fn parse_pr(n: &Value) -> ChangeRequest {
    let state = match n["state"].as_str() {
        Some("OPEN") => PrState::Open,
        Some("MERGED") => PrState::Merged,
        _ => PrState::Closed,
    };
    let mergeable = match n["mergeable"].as_str() {
        Some("MERGEABLE") => Mergeable::Mergeable,
        Some("CONFLICTING") => Mergeable::Conflicting,
        _ => Mergeable::Unknown,
    };
    ChangeRequest {
        number: n["number"].as_u64().and_then(|x| u32::try_from(x).ok()).unwrap_or_default(),
        url: s(n, "url"),
        title: s(n, "title"),
        state,
        draft: n["isDraft"].as_bool().unwrap_or(false),
        head_sha: s(n, "headRefOid"),
        base_branch: s(n, "baseRefName"),
        mergeable,
        behind: n["mergeStateStatus"] == "BEHIND",
        auto_merge: !n["autoMergeRequest"].is_null(),
        in_merge_queue: n["isInMergeQueue"].as_bool().unwrap_or(false),
        queue_position: n["mergeQueueEntry"]["position"].as_u64().and_then(|p| u32::try_from(p).ok()).map(|p| p + 1),
        queue_removal: queue_removal(&n["timelineItems"]["nodes"][0]),
        created_at: n["createdAt"].as_str().and_then(pdt_core::conversation::devin::iso_ms),
    }
}

/// The newest merge-queue event, when it is a removal.
fn queue_removal(e: &Value) -> Option<QueueRemoval> {
    if e["__typename"] != "RemovedFromMergeQueueEvent" {
        return None;
    }
    Some(QueueRemoval {
        id: s(e, "id"),
        reason: queue_removal_reason(&s(e, "reason")),
        by_queue: e["actor"]["login"].as_str().is_some_and(is_queue_actor),
    })
}

/// A `RemovedFromMergeQueueEvent`'s reason.
pub fn queue_removal_reason(reason: &str) -> QueueRemovalReason {
    match reason {
        "merge_conflict" => QueueRemovalReason::Conflict,
        "failed_checks" => QueueRemovalReason::FailedChecks,
        "merged" => QueueRemovalReason::Merged,
        other => QueueRemovalReason::Other(other.to_string()),
    }
}

/// The `github-merge-queue` app (`github-merge-queue[bot]` in some fields): a removal it made is the queue's.
pub fn is_queue_actor(login: &str) -> bool {
    login.starts_with("github-merge-queue")
}

/// The start of the branches GitHub's merge queue tests PR `number` into `base` on.
pub fn queue_branch_prefix(base: &str, number: u32) -> String {
    format!("gh-readonly-queue/{base}/pr-{number}-")
}

/// A created pull request (`POST …/pulls`).
pub fn parse_opened(v: &Value) -> Result<OpenedChangeRequest, GhError> {
    let number = v["number"].as_u64().and_then(|n| u32::try_from(n).ok());
    let number = number.ok_or_else(|| GhError::BadAnswer("no pull request number in the answer".into()))?;
    Ok(OpenedChangeRequest { number, url: s(v, "html_url") })
}

/// The `MERGE_SETTINGS_QUERY` answer's `data`.
pub fn parse_merge_settings(data: &Value) -> Result<MergeSettings, GhError> {
    let r = &data["repository"];
    if r.is_null() {
        return Err(GhError::BadAnswer("no repository in the answer".into()));
    }
    let b = |k: &str| r[k].as_bool().unwrap_or(false);
    Ok(MergeSettings {
        squash: b("squashMergeAllowed"),
        merge: b("mergeCommitAllowed"),
        rebase: b("rebaseMergeAllowed"),
        queue: r["mergeQueue"].is_object(),
        // Older GitHub Enterprise Servers don't say: allowed, as before.
        auto_merge: r["autoMergeAllowed"].as_bool().unwrap_or(true),
    })
}

/// A workflow runs answer (`GET …/actions/runs`).
pub fn parse_runs(v: &Value) -> Result<Vec<CiRun>, GhError> {
    let list = v["workflow_runs"].as_array().ok_or_else(|| GhError::BadAnswer("no workflow_runs".into()))?;
    Ok(list.iter().filter_map(parse_run).collect())
}

/// One run of a runs answer (or `GET …/actions/runs/{id}`).
pub fn parse_run(r: &Value) -> Option<CiRun> {
    Some(CiRun {
        provider: pdt_core::pr::GITHUB_ACTIONS.into(),
        id: r["id"].as_u64()?.to_string(),
        attempt: r["run_attempt"].as_u64().and_then(|a| u32::try_from(a).ok()).unwrap_or(1),
        name: s(r, "name"),
        state: CiRun::state_of(r["status"].as_str().unwrap_or(""), r["conclusion"].as_str().unwrap_or("")),
        url: s(r, "html_url"),
        head_sha: s(r, "head_sha"),
        branch: s(r, "head_branch"),
        created_at: s(r, "created_at"),
        failed_jobs: vec![],
    })
}

/// One run with what a report on it needs (PDT-292): its workflow's file, what started it, its title.
pub fn parse_workflow_run(r: &Value) -> Option<pdt_core::workflow_runs::WorkflowRun> {
    let run = parse_run(r)?;
    let title = match s(r, "display_title") {
        t if t.is_empty() => run.name.clone(),
        t => t,
    };
    let updated_at = match s(r, "updated_at") {
        u if u.is_empty() => run.created_at.clone(),
        u => u,
    };
    Some(pdt_core::workflow_runs::WorkflowRun { path: s(r, "path"), event: s(r, "event"), title, updated_at, run })
}

fn failed(job: &Value) -> bool {
    matches!(job["conclusion"].as_str(), Some("failure" | "timed_out" | "startup_failure"))
}

/// A jobs answer (`GET …/jobs`): its jobs.
pub fn job_list(v: &Value) -> Result<&Vec<Value>, GhError> {
    v["jobs"].as_array().ok_or_else(|| GhError::BadAnswer("no jobs in the answer".into()))
}

/// The end of a job's log: its last [`LOG_LINES`] lines (at most [`LOG_CHARS`] characters), without Actions'
/// timestamps and terminal colours.
pub fn log_tail(raw: &str) -> String {
    log_tail_lines(raw, LOG_LINES)
}

/// [`log_tail`] of the last `keep` lines.
pub fn log_tail_lines(raw: &str, keep: usize) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let skipped = lines.len().saturating_sub(keep);
    let mut out: String = lines[skipped..].iter().map(|l| format!("{}\n", clean_line(l))).collect();
    let chars = out.chars().count();
    if chars > LOG_CHARS {
        out = out.chars().skip(chars - LOG_CHARS).collect();
    }
    if skipped > 0 {
        out = format!("… ({skipped} earlier lines)\n{out}");
    }
    out
}

fn clean_line(l: &str) -> String {
    // `2026-10-19T02:00:10.1234567Z text`
    let l = match l.split_once(' ') {
        Some((t, rest)) if t.len() >= 20 && t.ends_with('Z') && t.as_bytes()[4] == b'-' && t.contains('T') => rest,
        _ => l,
    };
    let mut out = String::with_capacity(l.len());
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Reads pages of issues (newest first, `page(n)` the items of page `n`) until one reaches issue `after` or is the
/// last: one page without `after`.
pub fn read_issue_pages(
    after: Option<u64>,
    mut page: impl FnMut(u32) -> Result<Vec<Value>, GhError>,
) -> Result<Vec<Issue>, GhError> {
    let mut out = vec![];
    for n in 1..=ISSUE_PAGES {
        let items = page(n)?;
        let lowest = items.iter().filter_map(|v| v["number"].as_u64()).min();
        out.extend(issues_of(&items));
        let reached = match after {
            None => true,
            Some(a) => lowest.is_none_or(|l| l <= a),
        };
        if reached || items.len() < ISSUE_PAGE {
            break;
        }
    }
    Ok(out)
}

/// Reads pages of pull requests (most recently updated first, `page(n)` the items of page `n`) until one was updated
/// before `since` or is the last: one page without `since`.
pub fn read_pull_pages(
    since: Option<&str>,
    mut page: impl FnMut(u32) -> Result<Vec<Value>, GhError>,
) -> Result<Vec<RepoChangeRequest>, GhError> {
    let mut out = vec![];
    for n in 1..=ISSUE_PAGES {
        let items = page(n)?;
        let prs: Vec<RepoChangeRequest> = items.iter().filter_map(parse_repo_pull).collect();
        let reached = match since {
            None => true,
            Some(s) => prs.iter().any(|p| p.updated_at.as_str() < s),
        };
        out.extend(prs.into_iter().filter(|p| since.is_none_or(|s| p.updated_at.as_str() >= s)));
        if reached || items.len() < ISSUE_PAGE {
            break;
        }
    }
    Ok(out)
}

/// One pull request of a list (`GET …/pulls`).
pub fn parse_repo_pull(v: &Value) -> Option<RepoChangeRequest> {
    let state = match (v["state"].as_str(), v["merged_at"].is_string()) {
        (Some("open"), _) => PrState::Open,
        (_, true) => PrState::Merged,
        _ => PrState::Closed,
    };
    Some(RepoChangeRequest {
        number: v["number"].as_u64().and_then(|n| u32::try_from(n).ok())?,
        title: s(v, "title"),
        body: s(v, "body"),
        url: s(v, "html_url"),
        author: s(&v["user"], "login"),
        is_bot: is_bot(&v["user"]),
        collaborator: collaborator(v),
        head_repo: v["head"]["repo"]["full_name"].as_str().map(String::from),
        head_branch: s(&v["head"], "ref"),
        head_sha: s(&v["head"], "sha"),
        base_branch: s(&v["base"], "ref"),
        labels: v["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["name"].as_str().map(String::from)).collect())
            .unwrap_or_default(),
        state,
        draft: v["draft"].as_bool().unwrap_or(false),
        created_at: s(v, "created_at"),
        updated_at: s(v, "updated_at"),
    })
}

fn is_pr(v: &Value) -> bool {
    v.get("pull_request").is_some_and(|p| !p.is_null())
}

fn issues_of(items: &[Value]) -> Vec<Issue> {
    items.iter().filter(|v| !is_pr(v)).filter_map(issue_of).collect()
}

fn issue_of(v: &Value) -> Option<Issue> {
    Some(Issue {
        number: v["number"].as_u64()?,
        title: s(v, "title"),
        body: s(v, "body"),
        url: s(v, "html_url"),
        author: s(&v["user"], "login"),
        labels: v["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["name"].as_str().map(String::from)).collect())
            .unwrap_or_default(),
        created_at: s(v, "created_at"),
    })
}

/// An issue (`GET …/issues/{n}`).
pub fn parse_issue(v: &Value) -> Result<IssueDetail, GhError> {
    let issue = issue_of(v).ok_or_else(|| GhError::BadAnswer("no issue number in the answer".into()))?;
    Ok(IssueDetail {
        issue,
        id: v["id"].as_u64().ok_or_else(|| GhError::BadAnswer("no issue id in the answer".into()))?,
        closed: v["state"] == "closed",
        state_reason: v["state_reason"].as_str().filter(|r| !r.is_empty()).map(String::from),
        change_request: is_pr(v),
    })
}

/// Conversation comments (`in_review: false`) or review comments.
pub fn parse_comments(items: &[Value], in_review: bool) -> Vec<ChangeComment> {
    items
        .iter()
        .filter_map(|v| {
            Some(ChangeComment {
                id: v["id"].as_u64()?,
                author: s(&v["user"], "login"),
                is_bot: is_bot(&v["user"]),
                body: s(v, "body"),
                url: s(v, "html_url"),
                path: v["path"].as_str().map(String::from),
                line: v["line"].as_u64().and_then(|l| u32::try_from(l).ok()),
                in_review,
                collaborator: collaborator(v),
            })
        })
        .collect()
}

/// A pull request's files (`GET …/pulls/{n}/files`).
pub fn parse_files(items: &[Value]) -> Vec<ChangeFile> {
    items
        .iter()
        .filter_map(|v| {
            let path = s(v, "filename");
            (!path.is_empty()).then(|| ChangeFile {
                path,
                status: s(v, "status"),
                additions: v["additions"].as_u64().unwrap_or(0),
                deletions: v["deletions"].as_u64().unwrap_or(0),
                patch: v["patch"].as_str().map(String::from),
                previous_path: v["previous_filename"].as_str().map(String::from),
            })
        })
        .collect()
}

/// GitHub's name for a review's verdict.
fn review_event(v: ReviewVerdict) -> &'static str {
    match v {
        ReviewVerdict::Approve => "APPROVE",
        ReviewVerdict::RequestChanges => "REQUEST_CHANGES",
        ReviewVerdict::Comment => "COMMENT",
    }
}

pub fn parse_reviews(items: &[Value]) -> Vec<ChangeReview> {
    items
        .iter()
        .filter_map(|v| {
            let state = match v["state"].as_str()? {
                "APPROVED" => "approved",
                "CHANGES_REQUESTED" => "changes_requested",
                "COMMENTED" => "commented",
                _ => return None,
            };
            Some(ChangeReview {
                id: v["id"].as_u64()?,
                author: s(&v["user"], "login"),
                is_bot: is_bot(&v["user"]),
                state: state.into(),
                body: s(v, "body"),
                url: s(v, "html_url"),
                collaborator: collaborator(v),
            })
        })
        .collect()
}

/// A release from `GET /releases/...`; drafts and pre-releases are not releases here.
pub fn parse_release(v: &Value) -> Result<Option<Release>, GhError> {
    parse_release_or_pre(v, false)
}

/// A release, or with `prereleases` a pre-release too; never a draft.
fn parse_release_or_pre(v: &Value, prereleases: bool) -> Result<Option<Release>, GhError> {
    let tag = s(v, "tag_name");
    if tag.is_empty() {
        return Err(GhError::BadAnswer("a release without a tag".into()));
    }
    if v["draft"].as_bool() == Some(true) || (!prereleases && v["prerelease"].as_bool() == Some(true)) {
        return Ok(None);
    }
    Ok(Some(Release {
        name: Some(s(v, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone()),
        tag,
        url: s(v, "html_url"),
        published_at: v["published_at"].as_str().map(String::from),
        notes: s(v, "body"),
        prerelease: v["prerelease"].as_bool() == Some(true),
        assets: v["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| ReleaseAsset { name: s(a, "name"), url: s(a, "browser_download_url"), size: a["size"].as_u64() })
            .collect(),
    }))
}

/// GraphQL's name for a merge strategy.
fn merge_method(s: MergeStrategy) -> &'static str {
    match s {
        MergeStrategy::Squash => "SQUASH",
        MergeStrategy::Merge => "MERGE",
        MergeStrategy::Rebase => "REBASE",
    }
}

#[cfg(test)]
mod tests;

/// A contents-API answer for one file (PDT-373): its text (base64, wrapped) and blob sha.
pub fn parse_file(v: &Value) -> Result<RepoFile, GhError> {
    use base64::Engine;
    if v["type"].as_str() != Some("file") {
        return Err(GhError::BadAnswer(format!("{} is not a file", s(v, "path"))));
    }
    let packed: String = s(v, "content").chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(packed)
        .map_err(|e| GhError::BadAnswer(format!("a file's content doesn't decode: {e}")))?;
    let text = String::from_utf8(bytes).map_err(|_| GhError::BadAnswer(format!("{} is not text", s(v, "path"))))?;
    Ok(RepoFile { text, id: s(v, "sha") })
}

/// A repository path for a URL: each segment escaped, the slashes kept.
fn path_segments(path: &str) -> String {
    path.split('/').filter(|p| !p.is_empty()).map(encode).collect::<Vec<_>>().join("/")
}
