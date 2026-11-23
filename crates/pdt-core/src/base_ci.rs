//! Failures a pull request shares with its base branch (PDT-641): a check that fails on the pull request's head and on
//! the base branch's latest run is the base's to fix, not the pull request's, so the review workflow waits for the
//! base instead of asking. A check is a CI workflow (or pipeline) and one of its failed jobs, `workflow: job`, or the
//! workflow alone when its run names no failed job. Also the pull request's own diff, compared across a rebase: an
//! approval holds while it is the same.

use crate::pr::{CiRun, CiState};
use sha2::{Digest, Sha256};

/// The failed checks of `runs` (one commit's newest run of each workflow): `workflow: job`, else `workflow`.
pub fn failed_checks(runs: &[CiRun]) -> Vec<String> {
    let mut out: Vec<String> = runs
        .iter()
        .filter(|r| r.state == CiState::Failed)
        .flat_map(|r| match r.failed_jobs.as_slice() {
            [] => vec![r.name.clone()],
            jobs => jobs.iter().map(|j| format!("{}: {j}", r.name)).collect(),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The newest run of each workflow on a branch that has finished (passed or failed): what the branch's CI says now.
/// A run still going, or cancelled, says nothing yet.
pub fn latest_finished(runs: &[CiRun]) -> Vec<&CiRun> {
    let mut newest: Vec<&CiRun> = vec![];
    for r in runs.iter().filter(|r| matches!(r.state, CiState::Passed | CiState::Failed)) {
        match newest.iter_mut().find(|n| n.provider == r.provider && n.name == r.name) {
            Some(n) if r.order(n).is_gt() => *n = r,
            Some(_) => {}
            None => newest.push(r),
        }
    }
    newest
}

/// How one check stands on the base branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnBase {
    /// Its workflow's latest finished run there failed it.
    Fails,
    /// That run passed it (it ran and the job didn't fail).
    Passes,
    /// No finished run of its workflow there.
    Unknown,
}

/// How `check` (`workflow: job` or `workflow`) stands on a branch whose latest finished runs are `base`. A run that
/// failed naming no jobs fails every check of its workflow.
pub fn on_base(check: &str, base: &[&CiRun]) -> OnBase {
    let (workflow, job) = match check.split_once(": ") {
        Some((w, j)) => (w, Some(j)),
        None => (check, None),
    };
    let Some(run) = base.iter().find(|r| r.name == workflow) else { return OnBase::Unknown };
    match run.state {
        CiState::Failed => match job {
            Some(j) if !run.failed_jobs.is_empty() && !run.failed_jobs.iter().any(|f| f == j) => OnBase::Passes,
            _ => OnBase::Fails,
        },
        _ => OnBase::Passes,
    }
}

/// A pull request's failed checks, split by how they stand on its base branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    /// Failing on the base too.
    pub inherited: Vec<String>,
    /// Passing there, or not run there: the pull request's own.
    pub own: Vec<String>,
}

impl Split {
    /// Every failure is the base's (and there is one).
    pub fn all_inherited(&self) -> bool {
        !self.inherited.is_empty() && self.own.is_empty()
    }
}

/// Splits the failed checks of the pull request's head (`head`, its newest runs) by the base branch's runs (`base`, any
/// order and commits).
pub fn split(head: &[CiRun], base: &[CiRun]) -> Split {
    let latest = latest_finished(base);
    let mut out = Split::default();
    for c in failed_checks(head) {
        match on_base(&c, &latest) {
            OnBase::Fails => out.inherited.push(c),
            OnBase::Passes | OnBase::Unknown => out.own.push(c),
        }
    }
    out
}

/// Where the base branch stands on checks it was failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseNow {
    /// One still fails there.
    Failing,
    /// Every one passes there now.
    Fixed,
    /// None fails, but one has no finished run yet (a fix being built).
    Pending,
}

/// How the base branch stands on `checks` now, from its runs.
pub fn base_now(checks: &[String], base: &[CiRun]) -> BaseNow {
    let latest = latest_finished(base);
    let states: Vec<OnBase> = checks.iter().map(|c| on_base(c, &latest)).collect();
    if states.contains(&OnBase::Fails) {
        BaseNow::Failing
    } else if states.is_empty() || states.contains(&OnBase::Unknown) {
        BaseNow::Pending
    } else {
        BaseNow::Fixed
    }
}

/// The commit the base branch's latest finished runs of `checks`' workflows ran on (the newest), for saying which fix
/// the pull request was updated for.
pub fn base_sha(checks: &[String], base: &[CiRun]) -> Option<String> {
    let latest = latest_finished(base);
    let workflows: Vec<&str> = checks.iter().map(|c| c.split_once(": ").map_or(c.as_str(), |(w, _)| w)).collect();
    latest
        .into_iter()
        .filter(|r| workflows.contains(&r.name.as_str()))
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(|r| r.head_sha.clone())
}

/// Advisory ids in a CI log: RustSec's `RUSTSEC-2026-0592`, GitHub's `GHSA-xxxx-xxxx-xxxx` and `CVE-2026-1234`,
/// upper-cased, each once, in order.
pub fn advisories(log: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for word in log.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let w = word.trim_matches('-').to_ascii_uppercase();
        if is_advisory(&w) && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

fn is_advisory(w: &str) -> bool {
    let parts: Vec<&str> = w.split('-').collect();
    let digits =
        |s: &str, n: std::ops::RangeInclusive<usize>| n.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        ["RUSTSEC", y, n] => digits(y, 4..=4) && digits(n, 4..=4),
        ["CVE", y, n] => digits(y, 4..=4) && digits(n, 4..=7),
        ["GHSA", a, b, c] => [a, b, c].iter().all(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphanumeric())),
        _ => false,
    }
}

/// The words to look for a fix by, best first: the advisories, then each check's job (`audit (cargo deny)`), then
/// its workflow when it names no job. Short names (`test`) match too much and are left out.
pub fn fix_terms(checks: &[String], advisories: &[String]) -> Vec<String> {
    let mut out: Vec<String> = advisories.to_vec();
    for c in checks {
        let name = c.split_once(": ").map_or(c.as_str(), |(_, j)| j).trim();
        if name.chars().filter(|c| c.is_alphanumeric()).count() >= 6 && !out.iter().any(|t| t == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Whether a ticket's text mentions `term` (any case).
pub fn mentions(text: &str, term: &str) -> bool {
    !term.is_empty() && text.to_lowercase().contains(&term.to_lowercase())
}

/// One file of a pull request's own changes (against its merge base), as the forge lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile<'a> {
    pub path: &'a str,
    pub previous_path: Option<&'a str>,
    pub status: &'a str,
    /// Its unified diff; None when the forge shows none (binary, too large).
    pub patch: Option<&'a str>,
}

/// An id of a pull request's own diff that a rebase or a merge of its base leaves alone, as git's patch-id does: the
/// files and their changed lines, without the hunks' line numbers (they move as the base does). None when a file has
/// no diff to compare (binary, too large), or there are none: a change there can't be told from none.
pub fn diff_id(files: &[DiffFile<'_>]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut files: Vec<&DiffFile<'_>> = files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(b.path));
    let mut h = Sha256::new();
    for f in files {
        let patch = f.patch?;
        h.update(format!("file {}\0{}\0{}\0", f.path, f.previous_path.unwrap_or(""), f.status));
        for line in patch.lines() {
            // `@@ -12,7 +12,8 @@ fn context`: the counts stay, the positions go.
            let line = match line.strip_prefix("@@ ") {
                Some(rest) => hunk_counts(rest),
                None => line.to_string(),
            };
            h.update(line.as_bytes());
            h.update(b"\n");
        }
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A hunk header's line counts, without where it starts (`-12,7 +12,8 @@` → `@@ 7 8`).
fn hunk_counts(rest: &str) -> String {
    let count = |s: &str| s.split_once(',').map_or("1", |(_, n)| n).to_string();
    let mut parts = rest.split_whitespace();
    let old = parts.next().unwrap_or_default();
    let new = parts.next().unwrap_or_default();
    format!("@@ {} {}", count(old), count(new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, state: CiState, jobs: &[&str], at: &str, sha: &str) -> CiRun {
        CiRun {
            provider: "github_actions".into(),
            id: at.into(),
            attempt: 1,
            name: name.into(),
            state,
            url: String::new(),
            head_sha: sha.into(),
            branch: String::new(),
            created_at: at.into(),
            failed_jobs: jobs.iter().map(|j| j.to_string()).collect(),
        }
    }

    #[test]
    fn checks_are_workflow_and_job() {
        let head = [
            run("CI", CiState::Failed, &["audit (cargo deny)", "test"], "1", "h"),
            run("Docs", CiState::Failed, &[], "1", "h"),
            run("Lint", CiState::Passed, &[], "1", "h"),
        ];
        assert_eq!(failed_checks(&head), ["CI: audit (cargo deny)", "CI: test", "Docs"]);
    }

    #[test]
    fn a_failure_main_shares_is_inherited_and_one_it_passes_is_the_prs_own() {
        let head = [run("CI", CiState::Failed, &["audit (cargo deny)"], "5", "h")];
        let main = [
            run("CI", CiState::Passed, &[], "1", "m0"),
            run("CI", CiState::Failed, &["audit (cargo deny)"], "2", "m1"),
            // Still going: says nothing yet.
            run("CI", CiState::Running, &[], "3", "m2"),
        ];
        let s = split(&head, &main);
        assert!(s.all_inherited(), "{s:?}");
        assert_eq!(s.inherited, ["CI: audit (cargo deny)"]);

        let head = [run("CI", CiState::Failed, &["audit (cargo deny)", "test"], "5", "h")];
        let s = split(&head, &main);
        assert!(!s.all_inherited());
        assert_eq!(s.own, ["CI: test"], "main's run failed only the audit");

        let main_green = [run("CI", CiState::Passed, &[], "4", "m3")];
        assert_eq!(split(&head, &main_green).inherited, Vec::<String>::new());
        assert!(!split(&head, &[]).all_inherited(), "no runs on main: its own");
        assert!(!split(&[], &main).all_inherited(), "nothing failing");
    }

    #[test]
    fn a_base_run_naming_no_jobs_fails_all_of_its_workflow() {
        let head = [run("CI", CiState::Failed, &["test"], "5", "h")];
        let main = [run("CI", CiState::Failed, &[], "2", "m1")];
        assert!(split(&head, &main).all_inherited());
    }

    #[test]
    fn the_base_is_fixed_once_its_latest_finished_run_passes() {
        let checks = vec!["CI: audit (cargo deny)".to_string()];
        let failing = run("CI", CiState::Failed, &["audit (cargo deny)"], "2", "m1");
        assert_eq!(base_now(&checks, std::slice::from_ref(&failing)), BaseNow::Failing);
        let building = [failing.clone(), run("CI", CiState::Running, &[], "3", "m2")];
        assert_eq!(base_now(&checks, &building), BaseNow::Failing, "the fix isn't built yet");
        let fixed = [failing.clone(), run("CI", CiState::Passed, &[], "3", "m2")];
        assert_eq!(base_now(&checks, &fixed), BaseNow::Fixed);
        assert_eq!(base_sha(&checks, &fixed).as_deref(), Some("m2"));
        let other_job = [run("CI", CiState::Failed, &["test"], "3", "m2")];
        assert_eq!(base_now(&checks, &other_job), BaseNow::Fixed, "the audit passed; something else broke");
        assert_eq!(base_now(&checks, &[]), BaseNow::Pending);
    }

    #[test]
    fn advisories_are_found_in_a_log() {
        let log = "error[vulnerability]: url: RUSTSEC-2026-0592 (also rustsec-2026-0592)\n   ID: GHSA-abcd-1234-ef56\n\
                   see CVE-2026-12345, not RUSTSEC-26-1 or CVE-2026";
        assert_eq!(advisories(log), ["RUSTSEC-2026-0592", "GHSA-ABCD-1234-EF56", "CVE-2026-12345"]);
    }

    #[test]
    fn fix_terms_lead_with_advisories_and_skip_short_names() {
        let checks = vec!["CI: audit (cargo deny)".to_string(), "CI: test".to_string()];
        assert_eq!(fix_terms(&checks, &["RUSTSEC-2026-0592".into()]), ["RUSTSEC-2026-0592", "audit (cargo deny)"]);
        assert!(mentions("Bump url to 2.5.9 for rustsec-2026-0592", "RUSTSEC-2026-0592"));
        assert!(!mentions("anything", ""));
    }

    fn file<'a>(path: &'a str, patch: Option<&'a str>) -> DiffFile<'a> {
        DiffFile { path, previous_path: None, status: "modified", patch }
    }

    #[test]
    fn the_diff_id_survives_a_rebase_but_not_a_change() {
        let before = [
            file("b.rs", Some("@@ -10,3 +10,4 @@ fn b\n a\n+b\n c")),
            file("a.toml", Some("@@ -1 +1 @@\n-x = 1\n+x = 2")),
        ];
        // The base moved: the hunks start lower, the files come in another order.
        let rebased = [
            file("a.toml", Some("@@ -3 +3 @@\n-x = 1\n+x = 2")),
            file("b.rs", Some("@@ -40,3 +40,4 @@ fn b\n a\n+b\n c")),
        ];
        assert_eq!(diff_id(&before), diff_id(&rebased));
        assert!(diff_id(&before).is_some());
        let changed = [file("a.toml", Some("@@ -3 +3 @@\n-x = 1\n+x = 3")), before[0].clone()];
        assert_ne!(diff_id(&before), diff_id(&changed));
        let more = [before[0].clone(), before[1].clone(), file("c.rs", Some("@@ -0,0 +1 @@\n+c"))];
        assert_ne!(diff_id(&before), diff_id(&more));
        assert_eq!(diff_id(&[file("logo.png", None)]), None, "a binary file can't be compared");
        assert_eq!(diff_id(&[]), None, "nor no files");
    }
}
