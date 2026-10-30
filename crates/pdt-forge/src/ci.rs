//! CI providers (PDT-242): where a project's CI runs, apart from its forge. GitHub Actions (registered by the GitHub
//! forge plugin), the local CI command (`[ci] command`, the local forge's) and Buildkite (an integration) each are a
//! [`CiProvider`]; a project has a list of them (`.pdt/project.toml`'s `[ci] providers`, else its forge's own CI and
//! the CI integrations set up for it), and its badge and merge gate combine them. Blocking, as [`crate::Forge`] is.

use crate::ForgeError;
use pdt_core::pr::{CiJob, CiRun};
use std::collections::BTreeMap;

/// What a CI provider can do beyond reading runs and their jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CiCapabilities {
    /// Its runs of a branch are read before the branch has a change request (Buildkite builds every pushed branch;
    /// GitHub Actions is read only for a change request's head).
    pub branches: bool,
    /// [`CiProvider::job_log_tail`].
    pub logs: bool,
    /// [`CiProvider::retry`].
    pub retry: bool,
    /// [`CiProvider::trigger`].
    pub trigger: bool,
    /// [`CiProvider::queue_runs`]: the forge's merge queue runs here.
    pub merge_queue: bool,
}

impl CiCapabilities {
    /// Runs and jobs only.
    pub const NONE: CiCapabilities =
        CiCapabilities { branches: false, logs: false, retry: false, trigger: false, merge_queue: false };
}

/// Whether a repository has a provider's CI ([`CiProvider::config_paths`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiConfig {
    /// Known without looking at the repository's files (the local CI command; a CI integration set up for the
    /// project).
    Known(bool),
    /// The repository has it when a commit has files under one of these folders (GitHub: `.github/workflows`), or one
    /// of these files (GitLab: `.gitlab-ci.yml`): it has CI to come even before its first run.
    Paths(&'static [&'static str]),
}

/// A CI provider. `repo` is the project's forge's name for the repository (GitHub's `owner/name`); providers set up
/// per project (Buildkite's pipelines, the local command) ignore it.
pub trait CiProvider: Send + Sync {
    /// Its id, which `[ci] providers` names and events carry (`github_actions`).
    fn id(&self) -> &str;
    /// Its name for people (`GitHub Actions`).
    fn label(&self) -> &str;
    fn capabilities(&self) -> CiCapabilities;
    /// Whether, and where, a repository keeps its configuration.
    fn config_paths(&self) -> CiConfig;
    /// Recent runs on `branch`; with `sha`, those of that commit (a provider may give others too: callers filter).
    fn runs(&self, repo: &str, branch: &str, sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError>;
    /// The jobs of `run` (of its attempt).
    fn jobs(&self, repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError>;
    /// The last `lines` lines of `job`'s log (of `run`), timestamps dropped ([`CiCapabilities::logs`]).
    fn job_log_tail(&self, _repo: &str, _run: &CiRun, _job: &CiJob, _lines: usize) -> Result<String, ForgeError> {
        unsupported(self.label(), "logs to read")
    }
    /// Runs `run` again: only its failed jobs with `failed_only` ([`CiCapabilities::retry`]).
    fn retry(&self, _repo: &str, _run: &CiRun, _failed_only: bool) -> Result<(), ForgeError> {
        unsupported(self.label(), "runs to retry")
    }
    /// Starts `workflow` (an Actions workflow's file, a Buildkite pipeline) on `git_ref` with `inputs`
    /// ([`CiCapabilities::trigger`]).
    fn trigger(
        &self,
        _repo: &str,
        _git_ref: &str,
        _workflow: &str,
        _inputs: &BTreeMap<String, String>,
    ) -> Result<(), ForgeError> {
        unsupported(self.label(), "workflows to start")
    }
    /// Recent merge-queue runs in `repo` ([`CiCapabilities::merge_queue`]).
    fn queue_runs(&self, _repo: &str) -> Result<Vec<CiRun>, ForgeError> {
        unsupported(self.label(), "merge queue")
    }
}

fn unsupported<T>(provider: &str, what: &str) -> Result<T, ForgeError> {
    Err(ForgeError::Unsupported(format!("{provider} has no {what}")))
}

/// The failed jobs' names of `run` ([`CiProvider::jobs`]).
pub fn failed_job_names(p: &dyn CiProvider, repo: &str, run: &CiRun) -> Result<Vec<String>, ForgeError> {
    Ok(p.jobs(repo, run)?.into_iter().filter(|j| j.state == pdt_core::pr::CiState::Failed).map(|j| j.name).collect())
}

/// How many lines of a failed job's log `ci_failed_logs` gives.
pub const LOG_LINES: usize = 200;

pub mod testing {
    use super::*;
    use pdt_core::pr::CiState;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory CI provider for tests: set a branch's runs, a run's jobs and their logs; count the calls. By
    /// default it is GitHub Actions ([`FakeCi::actions`]).
    pub struct FakeCi {
        pub id: String,
        pub label: String,
        pub capabilities: Mutex<CiCapabilities>,
        pub config: CiConfig,
        /// Runs by branch.
        pub runs: Mutex<HashMap<String, Vec<CiRun>>>,
        pub queue: Mutex<Vec<CiRun>>,
        /// Jobs by run id: name and state.
        pub jobs: Mutex<HashMap<String, Vec<CiJob>>>,
        /// Logs by job id.
        pub logs: Mutex<HashMap<String, String>>,
        /// Reads of runs fail with this.
        pub error: Mutex<Option<String>>,
        /// Workflows started: repo, workflow, ref, inputs.
        pub triggered: Mutex<Vec<Triggered>>,
        /// Runs retried: id, failed only.
        pub retried: Mutex<Vec<(String, bool)>>,
        pub calls: Mutex<Vec<String>>,
    }

    /// A workflow started ([`CiProvider::trigger`]): repo, workflow, ref, inputs.
    pub type Triggered = (String, String, String, BTreeMap<String, String>);

    impl Default for FakeCi {
        fn default() -> Self {
            Self::actions()
        }
    }

    impl FakeCi {
        /// As GitHub Actions: every capability, configured under `.github/workflows`.
        pub fn actions() -> Self {
            Self::new(
                pdt_core::pr::GITHUB_ACTIONS,
                "GitHub Actions",
                CiCapabilities { branches: false, logs: true, retry: true, trigger: true, merge_queue: true },
                CiConfig::Paths(&[".github/workflows"]),
            )
        }

        pub fn new(id: &str, label: &str, capabilities: CiCapabilities, config: CiConfig) -> Self {
            FakeCi {
                id: id.into(),
                label: label.into(),
                capabilities: Mutex::new(capabilities),
                config,
                runs: Default::default(),
                queue: Default::default(),
                jobs: Default::default(),
                logs: Default::default(),
                error: Default::default(),
                triggered: Default::default(),
                retried: Default::default(),
                calls: Default::default(),
            }
        }

        /// A run of this provider, id `id`, of `name` on `branch` at `sha`, created at `at`.
        pub fn run(&self, id: &str, name: &str, branch: &str, sha: &str, state: CiState, at: &str) -> CiRun {
            CiRun {
                provider: self.id.clone(),
                id: id.into(),
                attempt: 1,
                name: name.into(),
                state,
                url: format!("https://ci.example/{}/runs/{id}", self.id),
                head_sha: sha.into(),
                branch: branch.into(),
                created_at: at.into(),
                failed_jobs: vec![],
            }
        }

        pub fn set_runs(&self, branch: &str, runs: Vec<CiRun>) {
            self.runs.lock().unwrap().insert(branch.into(), runs);
        }

        /// Run `run_id`'s failed jobs, by name (each with a log saying which it is).
        pub fn set_jobs(&self, run_id: &str, failed: &[&str]) {
            let jobs = failed
                .iter()
                .enumerate()
                .map(|(i, name)| CiJob {
                    id: format!("{run_id}-{i}"),
                    name: name.to_string(),
                    state: CiState::Failed,
                    url: format!("https://ci.example/{}/jobs/{run_id}-{i}", self.id),
                })
                .collect::<Vec<_>>();
            let mut logs = self.logs.lock().unwrap();
            for j in &jobs {
                logs.entry(j.id.clone()).or_insert_with(|| format!("{} failed", j.name));
            }
            self.jobs.lock().unwrap().insert(run_id.into(), jobs);
        }

        fn call(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }

        fn can(&self, on: impl Fn(&CiCapabilities) -> bool, what: &str) -> Result<(), ForgeError> {
            if on(&self.capabilities.lock().unwrap()) {
                Ok(())
            } else {
                Err(ForgeError::Unsupported(format!("the fake CI has no {what}")))
            }
        }
    }

    impl CiProvider for FakeCi {
        fn id(&self) -> &str {
            &self.id
        }
        fn label(&self) -> &str {
            &self.label
        }
        fn capabilities(&self) -> CiCapabilities {
            *self.capabilities.lock().unwrap()
        }
        fn config_paths(&self) -> CiConfig {
            self.config
        }
        fn runs(&self, repo: &str, branch: &str, _sha: Option<&str>) -> Result<Vec<CiRun>, ForgeError> {
            self.call(format!("runs {repo} {branch}"));
            if let Some(e) = self.error.lock().unwrap().clone() {
                return Err(ForgeError::Failed(e));
            }
            Ok(self.runs.lock().unwrap().get(branch).cloned().unwrap_or_default())
        }
        fn jobs(&self, repo: &str, run: &CiRun) -> Result<Vec<CiJob>, ForgeError> {
            self.call(format!("jobs {repo} {}", run.key()));
            Ok(self.jobs.lock().unwrap().get(&run.id).cloned().unwrap_or_default())
        }
        fn job_log_tail(&self, repo: &str, _run: &CiRun, job: &CiJob, _lines: usize) -> Result<String, ForgeError> {
            self.call(format!("log {repo} {}", job.id));
            self.can(|c| c.logs, "logs to read")?;
            Ok(self.logs.lock().unwrap().get(&job.id).cloned().unwrap_or_default())
        }
        fn retry(&self, repo: &str, run: &CiRun, failed_only: bool) -> Result<(), ForgeError> {
            self.call(format!("retry {repo} {}", run.id));
            self.can(|c| c.retry, "runs to retry")?;
            self.retried.lock().unwrap().push((run.id.clone(), failed_only));
            Ok(())
        }
        fn trigger(
            &self,
            repo: &str,
            git_ref: &str,
            workflow: &str,
            inputs: &BTreeMap<String, String>,
        ) -> Result<(), ForgeError> {
            self.call(format!("trigger {repo} {workflow} {git_ref}"));
            self.can(|c| c.trigger, "workflows to start")?;
            if let Some(e) = self.error.lock().unwrap().clone() {
                return Err(ForgeError::Failed(e));
            }
            self.triggered.lock().unwrap().push((repo.into(), workflow.into(), git_ref.into(), inputs.clone()));
            Ok(())
        }
        fn queue_runs(&self, repo: &str) -> Result<Vec<CiRun>, ForgeError> {
            self.call(format!("queue {repo}"));
            self.can(|c| c.merge_queue, "merge queue")?;
            Ok(self.queue.lock().unwrap().clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeCi;
    use super::*;
    use pdt_core::pr::CiState;

    #[test]
    fn a_provider_without_a_capability_refuses_its_calls() {
        let ci = FakeCi::new("plain", "Plain CI", CiCapabilities::NONE, CiConfig::Known(true));
        let run = ci.run("1", "ci", "b", "h", CiState::Failed, "t");
        assert!(matches!(ci.queue_runs("o/r"), Err(ForgeError::Unsupported(_))));
        assert!(matches!(ci.retry("o/r", &run, true), Err(ForgeError::Unsupported(_))));
        assert!(matches!(ci.trigger("o/r", "main", "release.yml", &BTreeMap::new()), Err(ForgeError::Unsupported(_))));
        ci.set_jobs("1", &["test"]);
        let job = &ci.jobs("o/r", &run).unwrap()[0];
        assert!(matches!(ci.job_log_tail("o/r", &run, job, 10), Err(ForgeError::Unsupported(_))));
        assert_eq!(failed_job_names(&ci, "o/r", &run).unwrap(), ["test"]);
        assert!(ci.runs("o/r", "b", None).unwrap().is_empty(), "runs and jobs are every provider's");
    }

    #[test]
    fn the_fake_is_github_actions_by_default() {
        let ci = FakeCi::default();
        assert_eq!((ci.id(), ci.label()), ("github_actions", "GitHub Actions"));
        let run = ci.run("7", "ci", "b", "h", CiState::Failed, "t");
        ci.set_jobs("7", &["lint"]);
        let job = &ci.jobs("o/r", &run).unwrap()[0];
        assert_eq!(ci.job_log_tail("o/r", &run, job, 10).unwrap(), "lint failed");
        ci.trigger("o/r", "main", "release.yml", &BTreeMap::new()).unwrap();
        assert_eq!(ci.triggered.lock().unwrap()[0].1, "release.yml");
    }
}
