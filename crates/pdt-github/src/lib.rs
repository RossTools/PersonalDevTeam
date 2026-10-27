//! The GitHub forge (PDT-176): a thin wrapper over the GitHub CLI (`gh`) for a branch's pull request and its Actions
//! runs (PDT-25), merges, reviews and issues. It uses the user's own `gh` login. Check runs are not read: the
//! fine-grained token PDT's users have cannot see them, so CI comes from the Actions runs API. Blocking; call from
//! `spawn_blocking` in async code.

use pdt_core::brand::SHORT_NAME;
use pdt_core::pr::{CiRun, Mergeable, PrState, QueueRemoval};
pub use pdt_forge::{
    Capabilities, ChangeComment, ChangeRequest, ChangeReview, CloseReason, Forge, ForgeError, ForgeNotes, Issue,
    IssueDetail, MergeSettings, MergeStrategy,
};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one gh command may take before it counts as hung.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// How many recent runs are read per branch (and of merge-queue runs per repo).
const RUN_LIMIT: &str = "30";
const RUN_FIELDS: &str = "databaseId,attempt,workflowName,status,conclusion,headSha,headBranch,url,createdAt";

const PR_QUERY: &str = "query($owner:String!,$name:String!,$head:String!){repository(owner:$owner,name:$name){\
pullRequests(headRefName:$head,first:5,orderBy:{field:CREATED_AT,direction:DESC}){nodes{number url title state \
createdAt isDraft headRefOid baseRefName mergeable mergeStateStatus isInMergeQueue mergeQueueEntry{position} \
autoMergeRequest{enabledAt} \
timelineItems(itemTypes:[ADDED_TO_MERGE_QUEUE_EVENT,REMOVED_FROM_MERGE_QUEUE_EVENT],last:1){nodes{__typename \
...on RemovedFromMergeQueueEvent{id reason actor{login}}}}}}}}";

const MERGE_SETTINGS_QUERY: &str = "query($owner:String!,$name:String!,$base:String!){repository(owner:$owner,\
name:$name){squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed mergeQueue(branch:$base){id}}}";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GhError {
    #[error("the GitHub CLI (gh) was not found")]
    NotInstalled,
    #[error("gh failed: {0}")]
    Command(String),
    #[error("unexpected gh output: {0}")]
    BadOutput(String),
    #[error("gh did not answer in time")]
    TimedOut,
}

impl From<GhError> for ForgeError {
    fn from(e: GhError) -> Self {
        match e {
            GhError::NotInstalled => ForgeError::Unavailable(e.to_string()),
            _ => ForgeError::Failed(e.to_string()),
        }
    }
}

/// GitHub's part of agents' instructions. PDT-165: fine-grained `gh` tokens can't read the Checks API, so
/// `gh pr checks` fails with a 403.
pub fn agent_notes() -> ForgeNotes {
    ForgeNotes {
        ci: Some(format!(
            "Read CI from the ticket's attempt (`pr.ci`: state, URL and runs, when {SHORT_NAME} has seen the pull \
             request) or from GitHub Actions: `gh run list --branch {{branch}}`, `gh run view <id> --json \
             jobs,conclusion` and `gh run view <id> --log-failed`. Not `gh pr checks` or the check-runs API: \
             fine-grained tokens can't read checks, so they fail with 403."
        )),
        reply: Some("`gh pr comment`, or a reply to the review comment".into()),
        watch: Some("`gh run watch`".into()),
    }
}

/// Where GitHub Actions workflows live.
pub const CI_DIRS: &[&str] = &[".github/workflows"];

/// What GitHub can do: everything.
pub const CAPABILITIES: Capabilities = Capabilities::ALL;

const ISSUE_JQ: &str = "{id, number, title, body, url: .html_url, author: .user.login, labels: [.labels[].name], \
created_at, state, state_reason, pr: (.pull_request != null)}";

/// The longest comment PDT sends (a command line has a limit; GitHub's own is 65536).
pub const MAX_COMMENT: usize = 20_000;

pub fn issue_args(repo: &str, number: u64) -> Vec<String> {
    vec!["api".into(), format!("repos/{repo}/issues/{number}"), "--jq".into(), ISSUE_JQ.into()]
}

/// The answer to [`issue_args`].
pub fn parse_issue(out: &str) -> Result<IssueDetail, GhError> {
    let v = json(out.trim())?;
    let issue = issue_of(&v).ok_or_else(|| GhError::BadOutput("no issue number in the answer".into()))?;
    Ok(IssueDetail {
        issue,
        id: v["id"].as_u64().ok_or_else(|| GhError::BadOutput("no issue id in the answer".into()))?,
        closed: v["state"] == "closed",
        state_reason: v["state_reason"].as_str().filter(|r| !r.is_empty()).map(String::from),
        change_request: v["pr"] == true,
    })
}

/// A comment cut to [`MAX_COMMENT`] characters.
fn comment_body(body: &str) -> String {
    if body.chars().count() <= MAX_COMMENT {
        return body.to_string();
    }
    let mut out: String = body.chars().take(MAX_COMMENT - 1).collect();
    out.push('…');
    out
}

pub fn comment_issue_args(repo: &str, number: u64, body: &str) -> Vec<String> {
    vec![
        "api".into(),
        "-X".into(),
        "POST".into(),
        format!("repos/{repo}/issues/{number}/comments"),
        "-f".into(),
        format!("body={}", comment_body(body)),
        "--jq".into(),
        ".html_url".into(),
    ]
}

/// POST of the labels: GitHub adds them to the issue's (a PATCH of `labels` would replace the set).
pub fn add_labels_args(repo: &str, number: u64, labels: &[String]) -> Vec<String> {
    let mut a = vec!["api".into(), "-X".into(), "POST".into(), format!("repos/{repo}/issues/{number}/labels")];
    for l in labels {
        a.extend(["-f".into(), format!("labels[]={l}")]);
    }
    a.extend(["--jq".into(), ".[].name".into()]);
    a
}

/// REST's update an issue: `state=closed` with `state_reason`, and `duplicate_issue_id` for a duplicate (GitHub's
/// "Close as duplicate").
pub fn close_issue_args(repo: &str, number: u64, reason: CloseReason, duplicate_of: Option<u64>) -> Vec<String> {
    let mut a = vec![
        "api".into(),
        "-X".into(),
        "PATCH".into(),
        format!("repos/{repo}/issues/{number}"),
        "-f".into(),
        "state=closed".into(),
        "-f".into(),
        format!("state_reason={}", reason.as_str()),
    ];
    if let (CloseReason::Duplicate, Some(id)) = (reason, duplicate_of) {
        a.extend(["-F".into(), format!("duplicate_issue_id={id}")]);
    }
    a.extend(["--jq".into(), ".state".into()]);
    a
}

/// gh's error for a 404.
fn not_found(e: &GhError) -> bool {
    matches!(e, GhError::Command(m) if m.contains("HTTP 404") || m.contains("Not Found"))
}

const BOT: &str = "(.user.type == \"Bot\" or (.user.login | endswith(\"[bot]\")))";

/// `gh api` of a list, one compact JSON object per line (`--jq`), every page.
fn list_args(path: &str, jq: &str, paginate: bool) -> Vec<String> {
    let mut a = vec!["api".to_string()];
    if paginate {
        a.push("--paginate".into());
    }
    a.extend([path.to_string(), "--jq".into(), format!(".[] | {jq}")]);
    a
}

/// Issues (and pull requests, which share their numbers) per page of [`GithubApi::recent_issues`], and the most pages
/// one read takes.
pub const ISSUE_PAGE: usize = 100;
pub const ISSUE_PAGES: u32 = 10;

pub fn recent_issues_args(repo: &str, page: u32) -> Vec<String> {
    list_args(
        &format!("repos/{repo}/issues?state=all&sort=created&direction=desc&per_page={ISSUE_PAGE}&page={page}"),
        "{number, title, body, url: .html_url, author: .user.login, labels: [.labels[].name], created_at, \
         pr: (.pull_request != null)}",
        false,
    )
}

pub fn issue_comments_args(repo: &str, number: u32) -> Vec<String> {
    list_args(
        &format!("repos/{repo}/issues/{number}/comments?per_page=100"),
        &format!("{{id, author: .user.login, bot: {BOT}, body, url: .html_url}}"),
        true,
    )
}

pub fn review_comments_args(repo: &str, number: u32) -> Vec<String> {
    list_args(
        &format!("repos/{repo}/pulls/{number}/comments?per_page=100"),
        &format!("{{id, author: .user.login, bot: {BOT}, body, url: .html_url, path, line}}"),
        true,
    )
}

pub fn reviews_args(repo: &str, number: u32) -> Vec<String> {
    list_args(
        &format!("repos/{repo}/pulls/{number}/reviews?per_page=100"),
        &format!("{{id, author: .user.login, bot: {BOT}, state, body, url: .html_url}}"),
        true,
    )
}

/// One JSON object per line.
fn lines(out: &str) -> Result<Vec<Value>, GhError> {
    out.lines().filter(|l| !l.trim().is_empty()).map(json).collect()
}

/// Reads pages of issues (newest first, `page(n)` the output of page `n`) until one reaches issue `after` or is the
/// last: one page without `after`.
pub fn read_issue_pages(
    after: Option<u64>,
    mut page: impl FnMut(u32) -> Result<String, GhError>,
) -> Result<Vec<Issue>, GhError> {
    let mut out = vec![];
    for n in 1..=ISSUE_PAGES {
        let items = lines(&page(n)?)?;
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

pub fn parse_issues(out: &str) -> Result<Vec<Issue>, GhError> {
    Ok(issues_of(&lines(out)?))
}

fn issues_of(items: &[Value]) -> Vec<Issue> {
    items.iter().filter(|v| v["pr"] != true).filter_map(issue_of).collect()
}

fn issue_of(v: &Value) -> Option<Issue> {
    Some(Issue {
        number: v["number"].as_u64()?,
        title: s(v, "title"),
        body: s(v, "body"),
        url: s(v, "url"),
        author: s(v, "author"),
        labels: v["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        created_at: s(v, "created_at"),
    })
}

/// Conversation comments (`in_review: false`) or review comments.
pub fn parse_comments(out: &str, in_review: bool) -> Result<Vec<ChangeComment>, GhError> {
    Ok(lines(out)?
        .iter()
        .filter_map(|v| {
            Some(ChangeComment {
                id: v["id"].as_u64()?,
                author: s(v, "author"),
                is_bot: v["bot"] == true,
                body: s(v, "body"),
                url: s(v, "url"),
                path: v["path"].as_str().map(String::from),
                line: v["line"].as_u64().and_then(|l| u32::try_from(l).ok()),
                in_review,
            })
        })
        .collect())
}

pub fn parse_reviews(out: &str) -> Result<Vec<ChangeReview>, GhError> {
    Ok(lines(out)?
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
                author: s(v, "author"),
                is_bot: v["bot"] == true,
                state: state.into(),
                body: s(v, "body"),
                url: s(v, "url"),
            })
        })
        .collect())
}

pub fn merge_settings_args(repo: &str, base: &str) -> Result<Vec<String>, GhError> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| GhError::BadOutput(format!("not owner/name: {repo}")))?;
    Ok(vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={MERGE_SETTINGS_QUERY}"),
        "-f".into(),
        format!("owner={owner}"),
        "-f".into(),
        format!("name={name}"),
        "-f".into(),
        format!("base={base}"),
    ])
}

pub fn merge_pr_args(repo: &str, number: u32, flag: Option<&str>, auto: bool) -> Vec<String> {
    let mut v: Vec<String> = vec!["pr".into(), "merge".into(), number.to_string(), "--repo".into(), repo.into()];
    if auto {
        v.push("--auto".into());
    }
    v.extend(flag.map(String::from));
    v
}

/// The `MERGE_SETTINGS_QUERY` answer.
pub fn parse_merge_settings(out: &str) -> Result<MergeSettings, GhError> {
    let v = json(out)?;
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(GhError::Command(s(e, "message")));
    }
    let r = &v["data"]["repository"];
    if r.is_null() {
        return Err(GhError::BadOutput("no repository in the answer".into()));
    }
    let b = |k: &str| r[k].as_bool().unwrap_or(false);
    Ok(MergeSettings {
        squash: b("squashMergeAllowed"),
        merge: b("mergeCommitAllowed"),
        rebase: b("rebaseMergeAllowed"),
        queue: r["mergeQueue"].is_object(),
    })
}

pub fn pr_args(repo: &str, branch: &str) -> Result<Vec<String>, GhError> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| GhError::BadOutput(format!("not owner/name: {repo}")))?;
    Ok(vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={PR_QUERY}"),
        "-f".into(),
        format!("owner={owner}"),
        "-f".into(),
        format!("name={name}"),
        "-f".into(),
        format!("head={branch}"),
    ])
}

pub fn branch_runs_args(repo: &str, branch: &str) -> Vec<String> {
    ["run", "list", "--repo", repo, "--branch", branch, "--limit", RUN_LIMIT, "--json", RUN_FIELDS]
        .map(String::from)
        .to_vec()
}

pub fn queue_runs_args(repo: &str) -> Vec<String> {
    ["run", "list", "--repo", repo, "--event", "merge_group", "--limit", RUN_LIMIT, "--json", RUN_FIELDS]
        .map(String::from)
        .to_vec()
}

pub fn jobs_args(repo: &str, run_id: u64, attempt: u32) -> Vec<String> {
    vec![
        "run".into(),
        "view".into(),
        run_id.to_string(),
        "--repo".into(),
        repo.into(),
        "--attempt".into(),
        attempt.to_string(),
        "--json".into(),
        "jobs".into(),
    ]
}

fn json(out: &str) -> Result<Value, GhError> {
    serde_json::from_str(out).map_err(|e| GhError::BadOutput(e.to_string()))
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

/// The `PR_QUERY` answer: the newest open PR, else the newest one.
pub fn parse_pull_request(out: &str) -> Result<Option<ChangeRequest>, GhError> {
    let v = json(out)?;
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(GhError::Command(e["message"].as_str().unwrap_or("GraphQL error").to_string()));
    }
    let nodes = v["data"]["repository"]["pullRequests"]["nodes"]
        .as_array()
        .ok_or_else(|| GhError::BadOutput("no pullRequests in the answer".into()))?;
    let pick = nodes.iter().find(|n| n["state"] == "OPEN").or_else(|| nodes.first());
    let Some(n) = pick else { return Ok(None) };
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
    Ok(Some(ChangeRequest {
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
    }))
}

/// The newest merge-queue event, when it is a removal.
fn queue_removal(e: &Value) -> Option<QueueRemoval> {
    if e["__typename"] != "RemovedFromMergeQueueEvent" {
        return None;
    }
    Some(QueueRemoval {
        id: s(e, "id"),
        reason: s(e, "reason"),
        by_queue: e["actor"]["login"].as_str().is_some_and(QueueRemoval::is_queue_actor),
    })
}

/// `gh run list --json RUN_FIELDS`.
pub fn parse_runs(out: &str) -> Result<Vec<CiRun>, GhError> {
    let v = json(out)?;
    let list = v.as_array().ok_or_else(|| GhError::BadOutput("run list is not an array".into()))?;
    Ok(list
        .iter()
        .filter_map(|r| {
            Some(CiRun {
                id: r["databaseId"].as_u64()?,
                attempt: r["attempt"].as_u64().and_then(|a| u32::try_from(a).ok()).unwrap_or(1),
                workflow: s(r, "workflowName"),
                state: CiRun::state_of(r["status"].as_str().unwrap_or(""), r["conclusion"].as_str().unwrap_or("")),
                url: s(r, "url"),
                head_sha: s(r, "headSha"),
                head_branch: s(r, "headBranch"),
                created_at: s(r, "createdAt"),
                failed_jobs: vec![],
            })
        })
        .collect())
}

/// `gh run view --json jobs`: the names of the jobs that failed.
pub fn parse_failed_jobs(out: &str) -> Result<Vec<String>, GhError> {
    let v = json(out)?;
    let jobs = v["jobs"].as_array().ok_or_else(|| GhError::BadOutput("no jobs in the answer".into()))?;
    Ok(jobs
        .iter()
        .filter(|j| matches!(j["conclusion"].as_str(), Some("failure" | "timed_out" | "startup_failure")))
        .map(|j| s(j, "name"))
        .collect())
}

/// The real CLI. Only production wiring (pdtd) detects one; tests use [`testing::FakeGithub`].
pub struct GhCli {
    program: Option<PathBuf>,
    timeout: Duration,
}

impl GhCli {
    pub fn new(program: PathBuf) -> Self {
        Self { program: Some(program), timeout: DEFAULT_TIMEOUT }
    }

    /// Finds `gh` on `login_path` (or pdtd's PATH), then in the standard install locations.
    pub fn detect(login_path: Option<&str>) -> Self {
        let paths = login_path.map(std::ffi::OsString::from).or_else(|| std::env::var_os("PATH"));
        let on_path = std::env::current_dir().ok().and_then(|cwd| which::which_in("gh", paths, cwd).ok());
        let program = on_path.or_else(|| candidates().into_iter().find(|p| p.is_file()));
        Self { program, timeout: DEFAULT_TIMEOUT }
    }

    pub fn is_installed(&self) -> bool {
        self.program.is_some()
    }

    fn run(&self, args: &[String]) -> Result<String, GhError> {
        let program = self.program.as_ref().ok_or(GhError::NotInstalled)?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::NotInstalled
            } else {
                GhError::Command(e.to_string())
            }
        })?;
        // Drain both pipes on their own threads so a big answer cannot stall on a full pipe while we wait.
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GhError::TimedOut);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(GhError::Command(e.to_string())),
            }
        };
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        if status.success() {
            Ok(String::from_utf8_lossy(&stdout).into_owned())
        } else {
            let mut err = String::from_utf8_lossy(&stderr).trim().to_string();
            if err.is_empty() {
                err = String::from_utf8_lossy(&stdout).trim().to_string();
            }
            Err(GhError::Command(err))
        }
    }
}

impl GhCli {
    fn merge_pr(&self, repo: &str, number: u32, flag: Option<&str>) -> Result<(), GhError> {
        match self.run(&merge_pr_args(repo, number, flag, true)) {
            // GitHub refuses auto-merge for a PR that could merge now ("is in clean status"): merge it.
            Err(GhError::Command(e)) if e.contains("clean status") => {
                self.run(&merge_pr_args(repo, number, flag, false)).map(|_| ())
            }
            r => r.map(|_| ()),
        }
    }
}

/// The `gh pr merge` flag for a strategy.
pub fn merge_flag(strategy: Option<MergeStrategy>) -> Option<&'static str> {
    strategy.map(|s| match s {
        MergeStrategy::Squash => "--squash",
        MergeStrategy::Merge => "--merge",
        MergeStrategy::Rebase => "--rebase",
    })
}

impl Forge for GhCli {
    fn name(&self) -> &'static str {
        "GitHub"
    }
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn agent_notes(&self) -> ForgeNotes {
        agent_notes()
    }
    fn ci_dirs(&self) -> &'static [&'static str] {
        CI_DIRS
    }
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        Ok(parse_pull_request(&self.run(&pr_args(repo, branch)?)?)?)
    }
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
        Ok(parse_runs(&self.run(&branch_runs_args(repo, branch))?)?)
    }
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        Ok(parse_runs(&self.run(&queue_runs_args(repo))?)?)
    }
    fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, ForgeError> {
        Ok(parse_failed_jobs(&self.run(&jobs_args(repo, run_id, attempt))?)?)
    }
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError> {
        Ok(parse_merge_settings(&self.run(&merge_settings_args(repo, base)?)?)?)
    }
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        Ok(self.merge_pr(repo, number, merge_flag(strategy))?)
    }
    fn viewer_login(&self) -> Result<String, ForgeError> {
        let out = self.run(&["api".into(), "user".into(), "--jq".into(), ".login".into()])?;
        let login = out.trim().to_string();
        if login.is_empty() { Err(GhError::BadOutput("no login".into()).into()) } else { Ok(login) }
    }
    fn recent_issues(&self, repo: &str, after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        Ok(read_issue_pages(after, |n| self.run(&recent_issues_args(repo, n)))?)
    }
    fn change_comments(&self, repo: &str, number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        let mut all = parse_comments(&self.run(&issue_comments_args(repo, number))?, false)?;
        all.extend(parse_comments(&self.run(&review_comments_args(repo, number))?, true)?);
        Ok(all)
    }
    fn change_reviews(&self, repo: &str, number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        Ok(parse_reviews(&self.run(&reviews_args(repo, number))?)?)
    }
    fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        match self.run(&issue_args(repo, number)) {
            Ok(out) => Ok(Some(parse_issue(&out)?)),
            Err(e) if not_found(&e) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn comment_issue(&self, repo: &str, number: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.run(&comment_issue_args(repo, number, body))?.trim().to_string())
    }
    fn add_labels(&self, repo: &str, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        if labels.is_empty() {
            return Ok(());
        }
        Ok(self.run(&add_labels_args(repo, number, labels)).map(|_| ())?)
    }
    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        duplicate_of: Option<u64>,
    ) -> Result<(), ForgeError> {
        Ok(self.run(&close_issue_args(repo, number, reason, duplicate_of)).map(|_| ())?)
    }
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

fn candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![PathBuf::from(r"C:\Program Files\GitHub CLI\gh.exe")]
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/opt/homebrew/bin/gh"), PathBuf::from("/usr/local/bin/gh")]
    } else {
        vec![PathBuf::from("/usr/bin/gh"), PathBuf::from("/usr/local/bin/gh")]
    }
}

fn no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

/// The GitHub forge with no `gh`: it runs nothing and reports gh as missing. It is the default until production
/// wiring installs the real CLI, so tests can never reach GitHub; it tells agents what the real one does.
pub struct Unavailable;

impl Forge for Unavailable {
    fn name(&self) -> &'static str {
        "GitHub"
    }
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn agent_notes(&self) -> ForgeNotes {
        agent_notes()
    }
    fn ci_dirs(&self) -> &'static [&'static str] {
        CI_DIRS
    }
    fn change_request(&self, _: &str, _: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn branch_runs(&self, _: &str, _: &str) -> Result<Vec<CiRun>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn queue_runs(&self, _: &str) -> Result<Vec<CiRun>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn failed_jobs(&self, _: &str, _: u64, _: u32) -> Result<Vec<String>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn merge_settings(&self, _: &str, _: &str) -> Result<MergeSettings, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn merge(&self, _: &str, _: u32, _: Option<MergeStrategy>) -> Result<(), ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn viewer_login(&self) -> Result<String, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn recent_issues(&self, _: &str, _: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn change_comments(&self, _: &str, _: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn change_reviews(&self, _: &str, _: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn issue(&self, _: &str, _: u64) -> Result<Option<IssueDetail>, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn comment_issue(&self, _: &str, _: u64, _: &str) -> Result<String, ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn add_labels(&self, _: &str, _: u64, _: &[String]) -> Result<(), ForgeError> {
        Err(GhError::NotInstalled.into())
    }
    fn close_issue(&self, _: &str, _: u64, _: CloseReason, _: Option<u64>) -> Result<(), ForgeError> {
        Err(GhError::NotInstalled.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::pr::CiState;

    #[test]
    fn parses_the_pr_query_preferring_an_open_pr() {
        let out = r#"{"data":{"repository":{"pullRequests":{"nodes":[
          {"number":45,"url":"u45","title":"Old","state":"CLOSED","isDraft":false,"headRefOid":"a","baseRefName":"main",
           "mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null},
          {"number":44,"url":"u44","title":"New","state":"OPEN","isDraft":true,"headRefOid":"b","baseRefName":"main",
           "mergeable":"CONFLICTING","mergeStateStatus":"BEHIND","isInMergeQueue":true,"mergeQueueEntry":{"position":0},
           "autoMergeRequest":{"enabledAt":"2026-10-18T00:00:00Z"}}]}}}}"#;
        let pr = parse_pull_request(out).unwrap().unwrap();
        assert_eq!(pr.number, 44);
        assert_eq!(pr.state, PrState::Open);
        assert!(pr.draft && pr.behind && pr.auto_merge && pr.in_merge_queue);
        assert_eq!(pr.queue_position, Some(1));
        assert_eq!(pr.mergeable, Mergeable::Conflicting);
        assert_eq!((pr.head_sha.as_str(), pr.base_branch.as_str()), ("b", "main"));
        assert_eq!(pr.queue_removal, None, "no timeline in the answer");

        assert_eq!(pr.created_at, None, "no createdAt in the answer");
        let merged = r#"{"data":{"repository":{"pullRequests":{"nodes":[{"number":3,"url":"u","title":"t",
          "createdAt":"2026-10-18T00:00:01Z","state":"MERGED","isDraft":false,"headRefOid":"c","baseRefName":"main","mergeable":"UNKNOWN",
          "mergeStateStatus":"UNKNOWN","isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null}]}}}}"#;
        let pr = parse_pull_request(merged).unwrap().unwrap();
        assert_eq!((pr.state, pr.auto_merge, pr.queue_position), (PrState::Merged, false, None));
        assert_eq!(pr.created_at, pdt_core::conversation::devin::iso_ms("2026-10-18T00:00:01Z"));
        assert!(pr.created_at.is_some());
        let removed = r#"{"data":{"repository":{"pullRequests":{"nodes":[{"number":49,"url":"u","title":"t",
          "state":"OPEN","isDraft":false,"headRefOid":"c","baseRefName":"main","mergeable":"MERGEABLE",
          "mergeStateStatus":"CLEAN","isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null,
          "timelineItems":{"nodes":[{"__typename":"RemovedFromMergeQueueEvent","id":"RFMQE_x",
          "reason":"merge_conflict","actor":{"login":"github-merge-queue"}}]}}]}}}}"#;
        let r = parse_pull_request(removed).unwrap().unwrap().queue_removal.unwrap();
        assert_eq!((r.id.as_str(), r.reason.as_str(), r.by_queue), ("RFMQE_x", "merge_conflict", true));
        let requeued = removed.replace("RemovedFromMergeQueueEvent", "AddedToMergeQueueEvent");
        assert_eq!(parse_pull_request(&requeued).unwrap().unwrap().queue_removal, None, "queued again since");
        let by_hand = removed.replace("github-merge-queue", "someone");
        assert!(!parse_pull_request(&by_hand).unwrap().unwrap().queue_removal.unwrap().by_queue);
        let none = r#"{"data":{"repository":{"pullRequests":{"nodes":[]}}}}"#;
        assert_eq!(parse_pull_request(none).unwrap(), None);
        let err = r#"{"data":null,"errors":[{"message":"Could not resolve to a Repository"}]}"#;
        assert!(matches!(parse_pull_request(err), Err(GhError::Command(m)) if m.contains("Could not resolve")));
        assert!(parse_pull_request("nope").is_err());
    }

    /// What agents on ticket work are told about GitHub (it used to live in `pdt_core::agent_context`).
    #[test]
    fn agents_read_ci_from_actions_runs_and_answer_with_gh() {
        use pdt_core::agent_context::{SessionContext, TicketWork, session_instructions};
        use pdt_core::workflow::events as ev;
        let notes = agent_notes();
        let news = [ev::CI_RUN_FAILED, ev::PR_COMMENTED].into_iter().collect();
        let ctx = SessionContext {
            port: 7420,
            session_id: "s1",
            project_id: "p1",
            project_name: "Work App",
            project_key: "WA",
            checkout: "/r/.worktrees/x",
            branch: Some("pdt-WA-7-x"),
            repo_path: "/r",
            base_branch: "main",
            ticket: Some(TicketWork { ticket_id: "t7", key: "WA-7", branch: "pdt-WA-7-x", base: "main", news }),
            personal: false,
            tools: &[],
            step: None,
            ci_branch: Some("pdt-WA-7-x"),
            forge: Some(&notes),
        };
        let t = session_instructions(&ctx, None);
        assert!(t.contains("`pr.ci`: state, URL and runs, when PDT has seen the pull request"), "{t}");
        assert!(t.contains("`gh run list --branch pdt-WA-7-x`"), "{t}");
        assert!(t.contains("`gh run view <id> --json jobs,conclusion`") && t.contains("--log-failed"), "{t}");
        assert!(t.contains("Not `gh pr checks` or the check-runs API"), "{t}");
        assert!(!t.contains("gh pr checks --watch"), "{t}");
        assert!(
            t.contains("answer it on the pull request (`gh pr comment`, or a reply to the review comment)."),
            "{t}"
        );
        assert!(t.contains("watch CI in a shell (`gh run watch`, sleep loops)"), "{t}");
        assert_eq!(Unavailable.agent_notes(), notes, "the default forge tells agents what the real one does");
        assert_eq!(Unavailable.capabilities(), Capabilities::ALL);
    }

    #[test]
    fn merge_flags_follow_the_strategy() {
        assert_eq!(merge_flag(None), None);
        assert_eq!(merge_flag(Some(MergeStrategy::Squash)), Some("--squash"));
        assert_eq!(merge_flag(Some(MergeStrategy::Rebase)), Some("--rebase"));
        assert!(
            matches!(ForgeError::from(GhError::NotInstalled), ForgeError::Unavailable(m) if m.contains("(gh) was not found"))
        );
    }

    #[test]
    fn parses_runs_and_failed_jobs() {
        let out = r#"[{"attempt":2,"conclusion":"failure","createdAt":"2026-10-19T02:00:10Z","databaseId":391,
          "headBranch":"pdt/x","headSha":"abc","status":"completed","url":"https://r/391","workflowName":"ci"},
          {"attempt":1,"conclusion":"","createdAt":"2026-10-19T02:01:10Z","databaseId":392,"headBranch":"pdt/x",
          "headSha":"abc","status":"in_progress","url":"https://r/392","workflowName":"bench"}]"#;
        let runs = parse_runs(out).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].id, runs[0].attempt, runs[0].state), (391, 2, CiState::Failed));
        assert_eq!((runs[0].workflow.as_str(), runs[0].head_sha.as_str()), ("ci", "abc"));
        assert_eq!(runs[1].state, CiState::Running);
        let jobs = r#"{"jobs":[{"name":"changes","conclusion":"success"},{"name":"test (linux)","conclusion":"failure"},
          {"name":"release","conclusion":"skipped"}]}"#;
        assert_eq!(parse_failed_jobs(jobs).unwrap(), vec!["test (linux)".to_string()]);
    }

    #[test]
    fn issue_pages_are_read_back_to_the_last_issue_seen() {
        // 250 issues and PRs, newest first, 100 a page.
        let page = |n: u32| -> Result<String, GhError> {
            let top = 250 - u64::from(n - 1) * 100;
            Ok((top.saturating_sub(99).max(1)..=top)
                .rev()
                .map(|i| format!(r#"{{"number":{i},"title":"t","pr":{}}}"#, i % 10 == 0))
                .collect::<Vec<_>>()
                .join("\n"))
        };
        let mut asked = vec![];
        let first = read_issue_pages(None, |n| {
            asked.push(n);
            page(n)
        })
        .unwrap();
        assert_eq!((asked.as_slice(), first.len()), (&[1][..], 90), "the first look reads one page, PRs left out");
        asked.clear();
        let since = read_issue_pages(Some(120), |n| {
            asked.push(n);
            page(n)
        })
        .unwrap();
        assert_eq!(asked, [1, 2], "page 2 reaches #120");
        assert!(since.iter().any(|i| i.number == 121) && since.iter().all(|i| i.number % 10 != 0));
        asked.clear();
        read_issue_pages(Some(0), |n| {
            asked.push(n);
            page(n)
        })
        .unwrap();
        assert_eq!(asked, [1, 2, 3], "the third page is the last");
    }

    #[test]
    fn parses_issues_comments_and_reviews() {
        let issues = r#"{"number":7,"title":"Bug","body":"b","url":"https://i/7","author":"al","labels":["bug"],"created_at":"t","pr":false}
{"number":6,"title":"A PR","body":null,"url":"u","author":"bo","labels":[],"created_at":"t","pr":true}
"#;
        let i = parse_issues(issues).unwrap();
        assert_eq!(i.len(), 1, "pull requests are left out");
        assert_eq!((i[0].number, i[0].labels.clone()), (7, vec!["bug".to_string()]));
        let c = r#"{"id":11,"author":"al","bot":false,"body":"nit","url":"u","path":"src/a.rs","line":40}"#;
        let c = parse_comments(c, true).unwrap();
        assert_eq!((c[0].id, c[0].path.as_deref(), c[0].line, c[0].in_review), (11, Some("src/a.rs"), Some(40), true));
        let r = r#"{"id":5,"author":"al","bot":false,"state":"CHANGES_REQUESTED","body":"no","url":"u"}
{"id":6,"author":"al","bot":false,"state":"PENDING","body":"","url":"u"}"#;
        let r = parse_reviews(r).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].state, "changes_requested");
        let a = recent_issues_args("o/r", 2);
        assert!(a[1].ends_with("per_page=100&page=2"), "{a:?}");
        let a = issue_comments_args("o/r", 3);
        assert_eq!(&a[..3], &["api", "--paginate", "repos/o/r/issues/3/comments?per_page=100"].map(String::from));
    }

    #[test]
    fn issues_are_read_commented_labelled_and_closed() {
        let out = r#"{"id":901,"number":12,"title":"Crash","body":null,"url":"https://github.com/o/r/issues/12",
          "author":"al","labels":["from-pdt"],"created_at":"t","state":"closed","state_reason":"not_planned","pr":false}"#;
        let i = parse_issue(&out.replace('\n', "")).unwrap();
        assert_eq!((i.id, i.issue.number, i.closed, i.state_reason.as_deref()), (901, 12, true, Some("not_planned")));
        assert_eq!((i.issue.body.as_str(), i.issue.labels.clone()), ("", vec!["from-pdt".to_string()]));
        let open = out.replace("\"closed\"", "\"open\"").replace("\"not_planned\"", "null").replace('\n', "");
        let i = parse_issue(&open).unwrap();
        assert!(!i.closed && i.state_reason.is_none());
        assert!(parse_issue("{}").is_err());
        assert_eq!(issue_args("o/r", 12)[1], "repos/o/r/issues/12");

        let a = comment_issue_args("o/r", 12, "@here: hi");
        assert_eq!(
            &a[..6],
            &["api", "-X", "POST", "repos/o/r/issues/12/comments", "-f", "body=@here: hi"].map(String::from)
        );
        let long = comment_issue_args("o/r", 12, &"x".repeat(MAX_COMMENT + 5));
        assert_eq!(long[5].chars().count(), "body=".len() + MAX_COMMENT, "cut to the limit");

        let a = add_labels_args("o/r", 12, &["bug".into(), "p1".into()]);
        assert_eq!(
            &a[2..8],
            &["POST", "repos/o/r/issues/12/labels", "-f", "labels[]=bug", "-f", "labels[]=p1"].map(String::from),
            "added, not replaced (POST)"
        );

        let a = close_issue_args("o/r", 12, CloseReason::Duplicate, Some(901));
        assert_eq!(&a[2..4], &["PATCH", "repos/o/r/issues/12"].map(String::from));
        assert!(a.contains(&"state=closed".into()) && a.contains(&"state_reason=duplicate".into()));
        assert!(a.contains(&"duplicate_issue_id=901".into()), "{a:?}");
        let a = close_issue_args("o/r", 12, CloseReason::NotPlanned, Some(901));
        assert!(a.contains(&"state_reason=not_planned".into()) && !a.iter().any(|x| x.starts_with("duplicate")));
        assert!(not_found(&GhError::Command("gh: Not Found (HTTP 404)".into())));
        assert!(!not_found(&GhError::Command("HTTP 401".into())));
    }

    #[test]
    fn args_name_the_repo_and_branch() {
        let a = pr_args("ross-game-tools/PersonalDevTeam", "pdt/PDT-1-x").unwrap();
        assert_eq!(&a[..2], &["api".to_string(), "graphql".to_string()]);
        assert!(a.contains(&"owner=ross-game-tools".to_string()));
        assert!(a.contains(&"name=PersonalDevTeam".to_string()));
        assert!(a.contains(&"head=pdt/PDT-1-x".to_string()));
        assert!(pr_args("nope", "b").is_err());
        assert_eq!(
            branch_runs_args("o/r", "b")[..6],
            ["run", "list", "--repo", "o/r", "--branch", "b"].map(String::from)
        );
        assert!(queue_runs_args("o/r").contains(&"merge_group".to_string()));
        assert_eq!(jobs_args("o/r", 7, 2)[2], "7");
    }

    #[test]
    fn a_missing_cli_is_not_installed() {
        let gh = GhCli::new(std::env::temp_dir().join("pdt-no-such-gh-binary"));
        assert_eq!(gh.branch_runs("o/r", "b"), Err(GhError::NotInstalled.into()));
        assert_eq!(Unavailable.change_request("o/r", "b"), Err(GhError::NotInstalled.into()));
    }

    #[test]
    fn parses_merge_settings_and_builds_merge_args() {
        let out = r#"{"data":{"repository":{"squashMergeAllowed":true,"mergeCommitAllowed":false,
          "rebaseMergeAllowed":true,"mergeQueue":{"id":"MQ_1"}}}}"#;
        assert_eq!(
            parse_merge_settings(out).unwrap(),
            MergeSettings { squash: true, merge: false, rebase: true, queue: true }
        );
        let none = r#"{"data":{"repository":{"squashMergeAllowed":false,"mergeCommitAllowed":true,
          "rebaseMergeAllowed":false,"mergeQueue":null}}}"#;
        assert!(!parse_merge_settings(none).unwrap().queue);
        assert!(parse_merge_settings(r#"{"errors":[{"message":"nope"}]}"#).is_err());
        assert_eq!(
            merge_pr_args("o/r", 7, Some("--squash"), true),
            ["pr", "merge", "7", "--repo", "o/r", "--auto", "--squash"]
        );
        assert_eq!(merge_pr_args("o/r", 7, None, false), ["pr", "merge", "7", "--repo", "o/r"]);
        assert!(merge_settings_args("o/r", "main").unwrap().contains(&"base=main".to_string()));
    }
}
