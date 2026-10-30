//! Failed runs of a repository's CI workflows (PDT-292), however they were started (a schedule, by hand, a push): the
//! runs a watch reads ([`WorkflowRun`]), the cursor that keeps its place across looks and restarts ([`RunCursor`]),
//! and what an Inbox item says about one.

use crate::pr::{CiRun, CiState};
use serde::{Deserialize, Serialize};

/// A completed run of a CI workflow, with what a report on it needs beyond a [`CiRun`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRun {
    /// Its newest attempt (`run.attempt`) and that attempt's state.
    pub run: CiRun,
    /// The workflow's file in the repository (`.github/workflows/release.yml`).
    pub path: String,
    /// What started it: `schedule`, `workflow_dispatch`, `push`, `pull_request`…
    pub event: String,
    /// Its title as the forge shows it (the commit's or pull request's title, or the run's own name).
    pub title: String,
    /// When its newest attempt last changed (RFC 3339): a re-run moves it.
    pub updated_at: String,
}

impl WorkflowRun {
    /// One attempt of the run: what a watch reports once.
    pub fn key(&self) -> String {
        format!("{}:{}", self.run.id, self.run.attempt)
    }

    /// Whether its workflow is `workflow`: its file name (`release.yml`), its path, or its name (`release`).
    pub fn is_of(&self, workflow: &str) -> bool {
        let w = workflow.trim();
        let file = self.path.rsplit('/').next().unwrap_or_default();
        !w.is_empty() && (self.path == w || file == w || self.run.name.eq_ignore_ascii_case(w))
    }

    /// The version or tag the run is for: its branch when that is a tag (`v1.2.0`), else one in its title.
    pub fn version(&self) -> Option<String> {
        let is_version = |s: &str| crate::release::parse_version(s).is_some();
        if is_version(&self.run.branch) {
            return Some(self.run.branch.clone());
        }
        self.title
            .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | ',' | ':'))
            .find(|w| is_version(w))
            .map(String::from)
    }
}

/// Whether a watch with an `events` filter reports a run started by `event` (an empty filter: every event).
pub fn event_matches(events: &[String], event: &str) -> bool {
    events.is_empty() || events.iter().any(|e| e.trim().eq_ignore_ascii_case(event))
}

/// Where a watch has read a workflow's runs to: the newest `updated_at` it has seen, and the attempts seen at exactly
/// that time (two can finish in the same second). Kept in the workflow's state, so a restart or a catch-up neither
/// misses nor repeats a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunCursor {
    pub since: String,
    pub seen: Vec<String>,
    /// When it last looked (ms since the epoch), for its interval.
    pub looked_at: i64,
}

impl RunCursor {
    /// The first look's cursor: past every run there is now, so turning a watch on never reports old runs.
    pub fn baseline(runs: &[WorkflowRun], now: i64) -> Self {
        let mut c = RunCursor { looked_at: now, ..Default::default() };
        for r in runs {
            c.advance(r);
        }
        c
    }

    /// The runs it has not seen yet, oldest first.
    pub fn fresh<'a>(&self, runs: &'a [WorkflowRun]) -> Vec<&'a WorkflowRun> {
        let mut out: Vec<&WorkflowRun> = runs
            .iter()
            .filter(|r| {
                r.updated_at.as_str() > self.since.as_str()
                    || (r.updated_at == self.since && !self.seen.contains(&r.key()))
            })
            .collect();
        out.sort_by(|a, b| a.updated_at.cmp(&b.updated_at).then_with(|| a.key().cmp(&b.key())));
        out.dedup_by_key(|r| r.key());
        out
    }

    /// Moves past `run` (taken in [`RunCursor::fresh`]'s order).
    pub fn advance(&mut self, run: &WorkflowRun) {
        match run.updated_at.as_str().cmp(self.since.as_str()) {
            std::cmp::Ordering::Greater => {
                self.since = run.updated_at.clone();
                self.seen = vec![run.key()];
            }
            std::cmp::Ordering::Equal if !self.seen.contains(&run.key()) => self.seen.push(run.key()),
            _ => {}
        }
    }

    /// Whether `every` ms have passed since its last look.
    pub fn due(&self, now: i64, every: u64) -> bool {
        now.saturating_sub(self.looked_at) >= i64::try_from(every).unwrap_or(i64::MAX)
    }
}

/// Whether a run's newest attempt is one to report: it failed (a cancelled, skipped or passed run is not news).
pub fn reportable(run: &WorkflowRun) -> bool {
    run.run.state == CiState::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, attempt: u32, state: CiState, at: &str) -> WorkflowRun {
        WorkflowRun {
            run: CiRun {
                provider: crate::pr::GITHUB_ACTIONS.into(),
                id: id.into(),
                attempt,
                name: "release".into(),
                state,
                url: format!("https://github.com/o/r/actions/runs/{id}"),
                head_sha: "abc".into(),
                branch: "main".into(),
                created_at: at.into(),
                failed_jobs: vec![],
            },
            path: ".github/workflows/release.yml".into(),
            event: "schedule".into(),
            title: "release".into(),
            updated_at: at.into(),
        }
    }

    #[test]
    fn the_first_look_only_records_where_the_runs_are() {
        let runs = [
            run("1", 1, CiState::Failed, "2026-10-30T08:00:00Z"),
            run("2", 1, CiState::Passed, "2026-10-29T08:00:00Z"),
        ];
        let c = RunCursor::baseline(&runs, 5);
        assert_eq!(
            (c.since.as_str(), c.seen.as_slice(), c.looked_at),
            ("2026-10-30T08:00:00Z", &["1:1".to_string()][..], 5)
        );
        assert!(c.fresh(&runs).is_empty());
        assert_eq!(RunCursor::baseline(&[], 5).since, "", "no runs yet: every later one is new");
    }

    #[test]
    fn later_runs_and_attempts_are_fresh_once_and_oldest_first() {
        let mut c = RunCursor::baseline(&[run("1", 1, CiState::Failed, "2026-10-30T08:00:00Z")], 0);
        let runs = [
            run("3", 1, CiState::Failed, "2026-10-30T10:00:00Z"),
            run("1", 2, CiState::Passed, "2026-10-30T09:00:00Z"),
            run("1", 1, CiState::Failed, "2026-10-30T08:00:00Z"),
        ];
        let fresh: Vec<String> = c.fresh(&runs).iter().map(|r| r.key()).collect();
        assert_eq!(fresh, ["1:2", "3:1"], "a re-run is a new attempt");
        for r in c.fresh(&runs) {
            c.advance(r);
        }
        assert!(c.fresh(&runs).is_empty(), "nothing twice");
    }

    #[test]
    fn runs_that_end_in_the_same_second_are_each_seen() {
        let mut c = RunCursor::default();
        let a = run("1", 1, CiState::Failed, "2026-10-30T08:00:00Z");
        c.advance(&a);
        let both = [a.clone(), run("2", 1, CiState::Failed, "2026-10-30T08:00:00Z")];
        let fresh: Vec<String> = c.fresh(&both).iter().map(|r| r.key()).collect();
        assert_eq!(fresh, ["2:1"]);
        c.advance(&both[1]);
        assert_eq!(c.seen, ["1:1", "2:1"]);
        assert!(c.fresh(&both).is_empty());
    }

    #[test]
    fn the_cursor_survives_being_stored() {
        let mut c = RunCursor::baseline(&[run("1", 1, CiState::Failed, "2026-10-30T08:00:00Z")], 9);
        c.advance(&run("2", 1, CiState::Failed, "2026-10-30T08:00:00Z"));
        let back: RunCursor = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert_eq!(serde_json::from_value::<RunCursor>(serde_json::json!({})).unwrap(), RunCursor::default());
    }

    #[test]
    fn a_look_is_due_after_the_interval() {
        let c = RunCursor { looked_at: 1_000, ..Default::default() };
        assert!(!c.due(1_000 + 59_999, 60_000));
        assert!(c.due(1_000 + 60_000, 60_000));
        assert!(RunCursor::default().due(0, 0));
    }

    #[test]
    fn only_failed_runs_are_reported() {
        assert!(reportable(&run("1", 1, CiState::Failed, "t")));
        for s in [CiState::Passed, CiState::Cancelled, CiState::Running, CiState::Queued] {
            assert!(!reportable(&run("1", 1, s, "t")), "{s:?}");
        }
    }

    #[test]
    fn a_workflow_is_named_by_its_file_path_or_name() {
        let r = run("1", 1, CiState::Failed, "t");
        for w in ["release.yml", ".github/workflows/release.yml", "release", "Release"] {
            assert!(r.is_of(w), "{w}");
        }
        assert!(!r.is_of("ci.yml") && !r.is_of(" "));
    }

    #[test]
    fn events_filter_runs_by_what_started_them() {
        let events = vec!["schedule".to_string(), "workflow_dispatch".to_string()];
        assert!(event_matches(&events, "schedule") && event_matches(&events, "workflow_dispatch"));
        assert!(!event_matches(&events, "push"));
        assert!(event_matches(&[], "push"), "no filter: every event");
    }

    #[test]
    fn the_version_is_a_tag_branch_or_one_in_the_title() {
        let mut r = run("1", 1, CiState::Failed, "t");
        assert_eq!(r.version(), None);
        r.run.branch = "v1.2.0".into();
        assert_eq!(r.version().as_deref(), Some("v1.2.0"));
        r.run.branch = "main".into();
        r.title = "Release v0.3.0-nightly.20261030 (nightly)".into();
        assert_eq!(r.version().as_deref(), Some("v0.3.0-nightly.20261030"));
    }
}
