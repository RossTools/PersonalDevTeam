//! Thin wrapper over the GitHub CLI (`gh`) for a branch's pull request and its Actions runs (PDT-25). It uses the
//! user's own `gh` login. Check runs are not read: the fine-grained token PDT's users have cannot see them, so CI
//! comes from the Actions runs API. Blocking; call from `spawn_blocking` in async code.

use pdt_core::pr::{CiRun, Mergeable, PrState};
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
isDraft headRefOid baseRefName mergeable mergeStateStatus isInMergeQueue mergeQueueEntry{position} \
autoMergeRequest{enabledAt}}}}}";

/// A branch's pull request as GitHub reports it (CI is read separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
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
}

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

pub trait GithubApi: Send + Sync {
    /// The pull request whose head is `branch` in `repo` (`owner/name`): the newest open one, else the newest.
    fn pull_request(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>, GhError>;
    /// Recent workflow runs on `branch`.
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, GhError>;
    /// Recent merge-queue (`merge_group`) runs in `repo`.
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, GhError>;
    /// The names of the failed jobs of one run attempt.
    fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, GhError>;
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
pub fn parse_pull_request(out: &str) -> Result<Option<PullRequest>, GhError> {
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
    Ok(Some(PullRequest {
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
    }))
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

impl GithubApi for GhCli {
    fn pull_request(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>, GhError> {
        parse_pull_request(&self.run(&pr_args(repo, branch)?)?)
    }
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, GhError> {
        parse_runs(&self.run(&branch_runs_args(repo, branch))?)
    }
    fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, GhError> {
        parse_runs(&self.run(&queue_runs_args(repo))?)
    }
    fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, GhError> {
        parse_failed_jobs(&self.run(&jobs_args(repo, run_id, attempt))?)
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

/// A GithubApi that runs nothing and reports gh as missing. It is the default until production wiring installs the
/// real CLI, so tests can never reach GitHub.
pub struct Unavailable;

impl GithubApi for Unavailable {
    fn pull_request(&self, _: &str, _: &str) -> Result<Option<PullRequest>, GhError> {
        Err(GhError::NotInstalled)
    }
    fn branch_runs(&self, _: &str, _: &str) -> Result<Vec<CiRun>, GhError> {
        Err(GhError::NotInstalled)
    }
    fn queue_runs(&self, _: &str) -> Result<Vec<CiRun>, GhError> {
        Err(GhError::NotInstalled)
    }
    fn failed_jobs(&self, _: &str, _: u64, _: u32) -> Result<Vec<String>, GhError> {
        Err(GhError::NotInstalled)
    }
}

pub mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory GithubApi for other crates' tests: set a branch's PR and runs, count the calls.
    #[derive(Default)]
    pub struct FakeGithub {
        pub prs: Mutex<HashMap<String, PullRequest>>,
        pub runs: Mutex<HashMap<String, Vec<CiRun>>>,
        pub queue: Mutex<Vec<CiRun>>,
        pub jobs: Mutex<HashMap<u64, Vec<String>>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeGithub {
        pub fn set_pr(&self, branch: &str, pr: Option<PullRequest>) {
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
        fn call(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }
    }

    impl GithubApi for FakeGithub {
        fn pull_request(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>, GhError> {
            self.call(format!("pr {repo} {branch}"));
            Ok(self.prs.lock().unwrap().get(branch).cloned())
        }
        fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, GhError> {
            self.call(format!("runs {repo} {branch}"));
            Ok(self.runs.lock().unwrap().get(branch).cloned().unwrap_or_default())
        }
        fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, GhError> {
            self.call(format!("queue {repo}"));
            Ok(self.queue.lock().unwrap().clone())
        }
        fn failed_jobs(&self, repo: &str, run_id: u64, attempt: u32) -> Result<Vec<String>, GhError> {
            self.call(format!("jobs {repo} {run_id}.{attempt}"));
            Ok(self.jobs.lock().unwrap().get(&run_id).cloned().unwrap_or_default())
        }
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

        let merged = r#"{"data":{"repository":{"pullRequests":{"nodes":[{"number":3,"url":"u","title":"t",
          "state":"MERGED","isDraft":false,"headRefOid":"c","baseRefName":"main","mergeable":"UNKNOWN",
          "mergeStateStatus":"UNKNOWN","isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null}]}}}}"#;
        let pr = parse_pull_request(merged).unwrap().unwrap();
        assert_eq!((pr.state, pr.auto_merge, pr.queue_position), (PrState::Merged, false, None));
        let none = r#"{"data":{"repository":{"pullRequests":{"nodes":[]}}}}"#;
        assert_eq!(parse_pull_request(none).unwrap(), None);
        let err = r#"{"data":null,"errors":[{"message":"Could not resolve to a Repository"}]}"#;
        assert!(matches!(parse_pull_request(err), Err(GhError::Command(m)) if m.contains("Could not resolve")));
        assert!(parse_pull_request("nope").is_err());
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
        assert_eq!(gh.branch_runs("o/r", "b"), Err(GhError::NotInstalled));
        assert_eq!(Unavailable.pull_request("o/r", "b"), Err(GhError::NotInstalled));
    }
}
