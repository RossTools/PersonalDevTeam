//! Worktree storage: which paths of a checkout are build output (declared in `.pdt/project.toml`'s `[storage]`, else
//! detected), when a finished ticket's worktree is cleaned or removed, and how low free disk space is. Pure; pdt-api
//! measures, cleans and warns.

use crate::ticket::TicketStage;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const GB: u64 = 1024 * 1024 * 1024;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Build output detected when a project declares none: a path is used when its marker file is at the checkout's
/// root. Cleaning still removes only what git ignores and tracks nothing in.
const DETECTED: &[(&str, &str)] = &[
    ("Cargo.toml", "target"),
    ("package.json", "**/node_modules"),
    ("package.json", "dist"),
    ("package.json", ".next"),
    ("package.json", ".turbo"),
    ("pyproject.toml", ".venv"),
    ("requirements.txt", ".venv"),
    ("pyproject.toml", "**/__pycache__"),
    ("requirements.txt", "**/__pycache__"),
    ("pom.xml", "target"),
    ("build.gradle", "build"),
    ("build.gradle.kts", "build"),
    ("build.gradle", ".gradle"),
    ("build.gradle.kts", ".gradle"),
    ("CMakeLists.txt", "build"),
    ("go.mod", "bin"),
];

/// The build output detected for a checkout whose root holds the files `has` says it does, each path once, in the
/// order of [`DETECTED`].
pub fn detect_build_output(has: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for (marker, path) in DETECTED {
        if has(marker) && !out.iter().any(|p| p == path) {
            out.push((*path).to_string());
        }
    }
    out
}

/// One build output path: a path relative to the checkout (`target`, `web/dist`), or `**/<name>` for every folder of
/// that name at any depth (`**/node_modules`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPath {
    /// `/`-separated segments below the checkout.
    At(Vec<String>),
    /// A folder or file name, anywhere.
    Anywhere(String),
}

fn bad_segment(s: &str) -> bool {
    s.is_empty()
        || s == "."
        || s == ".."
        || s.eq_ignore_ascii_case(".git")
        || s.chars().any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
}

impl OutputPath {
    /// Parses one entry of `build_output`: relative, no `.`/`..`/`.git`, no globs but a leading `**/`.
    pub fn parse(raw: &str) -> Result<OutputPath, String> {
        let p = raw.trim().trim_end_matches('/');
        let bad = || format!("\"{raw}\" in [storage] build_output is not a path inside the checkout");
        if let Some(name) = p.strip_prefix("**/") {
            return if bad_segment(name) || name.contains('/') {
                Err(bad())
            } else {
                Ok(OutputPath::Anywhere(name.into()))
            };
        }
        if p.starts_with('/') || p.starts_with('~') {
            return Err(bad());
        }
        let segments: Vec<String> = p.split('/').map(String::from).collect();
        if segments.iter().any(|s| bad_segment(s)) {
            return Err(bad());
        }
        Ok(OutputPath::At(segments))
    }

    /// Whether the path `rel` (segments below the checkout) is this build output.
    pub fn matches(&self, rel: &[&str]) -> bool {
        match self {
            OutputPath::At(segs) => segs.len() == rel.len() && segs.iter().zip(rel).all(|(a, b)| a == b),
            OutputPath::Anywhere(name) => rel.last() == Some(&name.as_str()),
        }
    }
}

/// `[storage] build_output` checked: each entry parsed, blanks dropped, each once.
pub fn build_output_paths(list: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for raw in list {
        if raw.trim().is_empty() {
            continue;
        }
        OutputPath::parse(raw)?;
        let p = raw.trim().trim_end_matches('/').to_string();
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// A project's automatic cleanup on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StorageSettings {
    /// Days after its ticket is done (or won't do) that a worktree's build output is removed; 0: never.
    pub clean_after_days: u32,
    /// Days after its ticket is done (or won't do) that the worktree itself is removed (its branch stays); 0: never.
    pub remove_after_days: u32,
    /// What becomes of a worktree when its ticket goes to Won't do or its attempt is abandoned.
    pub wont_do: WontDoCleanup,
    /// With [`WontDoCleanup::Later`]: days after the ticket closed that its worktree is removed (at least 1).
    pub wont_do_after_days: u32,
}

/// What becomes of a worktree, its branch and the ticket's workspace when its ticket goes to Won't do or its attempt
/// is abandoned. Nothing that would be lost (uncommitted changes, commits not pushed) is ever removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WontDoCleanup {
    /// At once: the worktree, the branch (local and the remote one PDT pushed) and the workspace go.
    #[default]
    Now,
    /// Kept for `wont_do_after_days`, then the automatic cleanup removes the worktree.
    Later,
    /// Kept until the user removes it.
    Keep,
}

pub const DEFAULT_WONT_DO_AFTER_DAYS: u32 = 7;

pub const DEFAULT_CLEAN_AFTER_DAYS: u32 = 2;
/// The longest either delay can be: a year.
pub const MAX_CLEANUP_DAYS: u32 = 365;

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            clean_after_days: DEFAULT_CLEAN_AFTER_DAYS,
            remove_after_days: 0,
            wont_do: WontDoCleanup::Now,
            wont_do_after_days: DEFAULT_WONT_DO_AFTER_DAYS,
        }
    }
}

/// What the automatic cleanup does to a worktree now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    Nothing,
    /// Remove its build output.
    Clean,
    /// Remove the worktree.
    Remove,
}

/// The worktree's ticket as the cleanup sees it: its stage, when it closed, and whether an attempt still works in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorktreeTicket {
    pub stage: TicketStage,
    pub closed_at: Option<i64>,
    pub attempt_active: bool,
}

/// What the cleanup does to a worktree with `ticket` (None: no ticket's, never touched) at `now`: nothing while an
/// attempt works in it or its ticket is open; removal once `remove_after_days` passed since it closed (a Won't do
/// ticket's: `wont_do_after_days` with [`WontDoCleanup::Later`], never with `Keep`), else cleaning once
/// `clean_after_days` did.
pub fn cleanup_due(ticket: Option<WorktreeTicket>, s: StorageSettings, now: i64) -> Cleanup {
    let Some(t) = ticket else { return Cleanup::Nothing };
    if t.attempt_active || !t.stage.is_closed() {
        return Cleanup::Nothing;
    }
    let Some(closed) = t.closed_at else { return Cleanup::Nothing };
    let passed = |days: u32| days > 0 && now - closed >= i64::from(days) * DAY_MS;
    let remove = match (t.stage, s.wont_do) {
        (TicketStage::WontDo, WontDoCleanup::Later) => passed(s.wont_do_after_days.max(1)),
        (TicketStage::WontDo, WontDoCleanup::Keep) => false,
        _ => passed(s.remove_after_days),
    };
    if remove {
        Cleanup::Remove
    } else if passed(s.clean_after_days) {
        Cleanup::Clean
    } else {
        Cleanup::Nothing
    }
}

/// Free and total bytes of a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiskSpace {
    #[ts(type = "number")]
    pub free: u64,
    #[ts(type = "number")]
    pub total: u64,
}

/// How free space on a disk compares with the host's thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiskLevel {
    Ok,
    /// Below the low-disk threshold: the Inbox says so.
    Low,
    /// Below the critical threshold: auto-start waits too.
    Critical,
}

pub const DEFAULT_LOW_DISK_GB: u32 = 20;
pub const DEFAULT_CRITICAL_DISK_GB: u32 = 5;

/// `free` bytes against the thresholds in GB (0: that warning is off).
pub fn disk_level(free: u64, low_gb: u32, critical_gb: u32) -> DiskLevel {
    let below = |gb: u32| gb > 0 && free < u64::from(gb) * GB;
    if below(critical_gb) {
        DiskLevel::Critical
    } else if below(low_gb) {
        DiskLevel::Low
    } else {
        DiskLevel::Ok
    }
}

impl DiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            DiskLevel::Ok => "ok",
            DiskLevel::Low => "low",
            DiskLevel::Critical => "critical",
        }
    }

    pub fn parse(s: &str) -> Option<DiskLevel> {
        [DiskLevel::Ok, DiskLevel::Low, DiskLevel::Critical].into_iter().find(|l| l.as_str() == s)
    }
}

/// What a disk's level going from `before` to `now` is news of (PDT-365): it got worse (`disk.low`, at its new
/// level), or it is fine again (`disk.recovered`). A critical disk that is only low again is still low: no news.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskNews {
    Low,
    Recovered,
}

pub fn disk_news(before: DiskLevel, now: DiskLevel) -> Option<DiskNews> {
    if now > before {
        Some(DiskNews::Low)
    } else if now == DiskLevel::Ok && before != DiskLevel::Ok {
        Some(DiskNews::Recovered)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disk_level_is_news_when_it_drops_or_recovers() {
        use DiskLevel::*;
        assert_eq!(disk_news(Ok, Ok), None);
        assert_eq!(disk_news(Ok, Low), Some(DiskNews::Low));
        assert_eq!(disk_news(Low, Low), None);
        assert_eq!(disk_news(Low, Critical), Some(DiskNews::Low));
        assert_eq!(disk_news(Ok, Critical), Some(DiskNews::Low));
        assert_eq!(disk_news(Critical, Critical), None);
        assert_eq!(disk_news(Critical, Low), None);
        assert_eq!(disk_news(Critical, Ok), Some(DiskNews::Recovered));
        assert_eq!(disk_news(Low, Ok), Some(DiskNews::Recovered));
        for l in [Ok, Low, Critical] {
            assert_eq!(DiskLevel::parse(l.as_str()), Some(l));
        }
    }

    #[test]
    fn detects_build_output_from_the_files_at_the_root() {
        let has = |names: &'static [&'static str]| move |f: &str| names.contains(&f);
        assert_eq!(detect_build_output(has(&["Cargo.toml"])), vec!["target"]);
        assert_eq!(
            detect_build_output(has(&["Cargo.toml", "package.json"])),
            vec!["target", "**/node_modules", "dist", ".next", ".turbo"]
        );
        assert_eq!(detect_build_output(has(&["pyproject.toml", "requirements.txt"])), vec![".venv", "**/__pycache__"]);
        assert!(detect_build_output(has(&["README.md"])).is_empty());
    }

    #[test]
    fn parses_build_output_paths() {
        assert_eq!(OutputPath::parse("target"), Ok(OutputPath::At(vec!["target".into()])));
        assert_eq!(OutputPath::parse(" web/dist/ "), Ok(OutputPath::At(vec!["web".into(), "dist".into()])));
        assert_eq!(OutputPath::parse("**/node_modules"), Ok(OutputPath::Anywhere("node_modules".into())));
        for bad in ["", "/abs", "~/x", "..", "a/../b", ".git", "a/.git", "*.o", "**/a/b", "**/", "a\\b", "C:/x", "./a"]
        {
            assert!(OutputPath::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn matches_paths_below_the_checkout() {
        let at = OutputPath::parse("web/dist").unwrap();
        assert!(at.matches(&["web", "dist"]));
        assert!(!at.matches(&["dist"]));
        assert!(!at.matches(&["web", "dist", "x"]));
        let any = OutputPath::parse("**/node_modules").unwrap();
        assert!(any.matches(&["node_modules"]));
        assert!(any.matches(&["web", "node_modules"]));
        assert!(!any.matches(&["web"]));
    }

    #[test]
    fn checks_the_declared_list() {
        assert_eq!(
            build_output_paths(&["target".into(), " ".into(), "target/".into(), "**/node_modules".into()]),
            Ok(vec!["target".to_string(), "**/node_modules".to_string()])
        );
        assert!(build_output_paths(&["../x".into()]).is_err());
    }

    fn closed(stage: TicketStage, closed_at: i64) -> Option<WorktreeTicket> {
        Some(WorktreeTicket { stage, closed_at: Some(closed_at), attempt_active: false })
    }

    #[test]
    fn cleans_finished_worktrees_after_the_delay() {
        let s = StorageSettings::default();
        let day = DAY_MS;
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), s, 2 * day - 1), Cleanup::Nothing);
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), s, 2 * day), Cleanup::Clean);
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), s, 3 * day), Cleanup::Clean);
        let both = StorageSettings { clean_after_days: 2, remove_after_days: 7, ..s };
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), both, 6 * day), Cleanup::Clean);
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), both, 7 * day), Cleanup::Remove);
        let off = StorageSettings { clean_after_days: 0, remove_after_days: 0, ..s };
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), off, 100 * day), Cleanup::Nothing);
    }

    #[test]
    fn a_wont_do_worktree_kept_for_later_goes_after_its_own_delay_and_a_kept_one_stays() {
        let day = DAY_MS;
        let later = StorageSettings { wont_do: WontDoCleanup::Later, wont_do_after_days: 3, ..Default::default() };
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), later, 3 * day - 1), Cleanup::Clean);
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), later, 3 * day), Cleanup::Remove);
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), later, 30 * day), Cleanup::Clean, "Done: its own delays");
        let zero = StorageSettings { wont_do_after_days: 0, ..later };
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), zero, day), Cleanup::Remove, "at least a day");
        let keep = StorageSettings { wont_do: WontDoCleanup::Keep, remove_after_days: 1, ..Default::default() };
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), keep, 30 * day), Cleanup::Clean);
        assert_eq!(cleanup_due(closed(TicketStage::Done, 0), keep, 30 * day), Cleanup::Remove);
        let now = StorageSettings { remove_after_days: 5, ..Default::default() };
        assert_eq!(cleanup_due(closed(TicketStage::WontDo, 0), now, 5 * day), Cleanup::Remove);
    }

    #[test]
    fn old_settings_read_with_the_defaults() {
        let s: StorageSettings = serde_json::from_str(r#"{"cleanAfterDays": 1, "removeAfterDays": 2}"#).unwrap();
        assert_eq!((s.wont_do, s.wont_do_after_days), (WontDoCleanup::Now, DEFAULT_WONT_DO_AFTER_DAYS));
        let s: StorageSettings = serde_json::from_str(r#"{"wontDo": "later", "wontDoAfterDays": 9}"#).unwrap();
        assert_eq!((s.wont_do, s.wont_do_after_days), (WontDoCleanup::Later, 9));
    }

    #[test]
    fn never_touches_open_tickets_active_attempts_or_worktrees_without_a_ticket() {
        let s = StorageSettings { clean_after_days: 1, remove_after_days: 1, ..Default::default() };
        let late = 10 * DAY_MS;
        assert_eq!(cleanup_due(None, s, late), Cleanup::Nothing);
        assert_eq!(cleanup_due(closed(TicketStage::InProgress, 0), s, late), Cleanup::Nothing);
        let active = Some(WorktreeTicket { stage: TicketStage::Done, closed_at: Some(0), attempt_active: true });
        assert_eq!(cleanup_due(active, s, late), Cleanup::Nothing);
        let unknown = Some(WorktreeTicket { stage: TicketStage::Done, closed_at: None, attempt_active: false });
        assert_eq!(cleanup_due(unknown, s, late), Cleanup::Nothing);
    }

    #[test]
    fn disk_levels() {
        assert_eq!(disk_level(30 * GB, 20, 5), DiskLevel::Ok);
        assert_eq!(disk_level(20 * GB - 1, 20, 5), DiskLevel::Low);
        assert_eq!(disk_level(4 * GB, 20, 5), DiskLevel::Critical);
        assert_eq!(disk_level(GB, 0, 0), DiskLevel::Ok);
        assert_eq!(disk_level(GB, 0, 5), DiskLevel::Critical);
    }
}
