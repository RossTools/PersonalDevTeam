//! Planned and actual overlap between tickets (tickets spec §7). A ticket's overlap is measured against the other
//! tickets of its project that have an active attempt: shared configured areas (planned) and, when both are active,
//! shared changed files (actual). A shared exclusive area, or a shared file inside one, is a conflict. Pure.

use crate::runconfig::AreaDef;
use crate::ticket::OverlapLevel;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::collections::{BTreeSet, HashMap};

/// Compiles an area path: relative to the checkout, `*` stays within a folder and `**` crosses folders.
pub fn glob(pattern: &str) -> Result<globset::Glob, String> {
    GlobBuilder::new(pattern.trim().trim_start_matches("./"))
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|e| e.kind().to_string())
}

/// The configured areas with their compiled paths.
#[derive(Debug, Clone, Default)]
pub struct AreaSet {
    areas: Vec<AreaDef>,
    /// Every exclusive area's paths together.
    exclusive: Option<GlobSet>,
}

impl AreaSet {
    /// Paths that do not compile are skipped (configurations are validated when parsed).
    pub fn new(areas: &[AreaDef]) -> Self {
        let mut b = GlobSetBuilder::new();
        let mut any = false;
        for a in areas.iter().filter(|a| a.exclusive) {
            for p in &a.paths {
                if let Ok(g) = glob(p) {
                    b.add(g);
                    any = true;
                }
            }
        }
        let exclusive = if any { b.build().ok() } else { None };
        AreaSet { areas: areas.to_vec(), exclusive }
    }

    pub fn get(&self, name: &str) -> Option<&AreaDef> {
        self.areas.iter().find(|a| a.name == name)
    }

    /// Whether `file` (a checkout-relative path with `/` separators) is inside an exclusive area.
    pub fn is_exclusive_file(&self, file: &str) -> bool {
        self.exclusive.as_ref().is_some_and(|s| s.is_match(file))
    }
}

/// A ticket taking part in overlap detection: open, with its areas and, when it has an active attempt, the files
/// that attempt changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Participant {
    pub id: String,
    /// Has an active attempt.
    pub active: bool,
    pub areas: Vec<String>,
    /// The active attempt's changed files (ignored when not active).
    pub files: Vec<String>,
}

/// How one ticket overlaps another (always one with an active attempt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlapWith {
    pub other: String,
    pub level: OverlapLevel,
    /// Shared configured areas, sorted.
    pub areas: Vec<String>,
    /// Shared changed files, sorted (both tickets active only).
    pub files: Vec<String>,
}

/// Every overlap, by ticket id; each list is conflicts first, then by the other ticket's id. Tickets without overlap
/// are left out. Areas not in `areas` are ignored; two tickets without active attempts never overlap each other.
pub fn compute(areas: &AreaSet, tickets: &[Participant]) -> HashMap<String, Vec<OverlapWith>> {
    let active: Vec<&Participant> = tickets.iter().filter(|t| t.active).collect();
    let mut out: HashMap<String, Vec<OverlapWith>> = HashMap::new();
    for t in tickets {
        let mine: BTreeSet<&str> = t.areas.iter().map(String::as_str).filter(|a| areas.get(a).is_some()).collect();
        let my_files: BTreeSet<&str> =
            if t.active { t.files.iter().map(String::as_str).collect() } else { BTreeSet::new() };
        if mine.is_empty() && my_files.is_empty() {
            continue;
        }
        for o in active.iter().filter(|o| o.id != t.id) {
            let shared: Vec<String> = o
                .areas
                .iter()
                .map(String::as_str)
                .filter(|a| mine.contains(a))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(String::from)
                .collect();
            let files: Vec<String> = o
                .files
                .iter()
                .map(String::as_str)
                .filter(|f| my_files.contains(f))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(String::from)
                .collect();
            if shared.is_empty() && files.is_empty() {
                continue;
            }
            let conflict = shared.iter().any(|a| areas.get(a).is_some_and(|d| d.exclusive))
                || files.iter().any(|f| areas.is_exclusive_file(f));
            let level = if conflict { OverlapLevel::Conflict } else { OverlapLevel::Overlap };
            out.entry(t.id.clone()).or_default().push(OverlapWith { other: o.id.clone(), level, areas: shared, files });
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| b.level.cmp(&a.level).then_with(|| a.other.cmp(&b.other)));
    }
    out
}

/// The worst level in a list of overlaps.
pub fn level_of(list: &[OverlapWith]) -> OverlapLevel {
    list.iter().map(|o| o.level).max().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(name: &str, paths: &[&str], exclusive: bool) -> AreaDef {
        AreaDef { name: name.into(), paths: paths.iter().map(|p| p.to_string()).collect(), exclusive }
    }

    fn set() -> AreaSet {
        AreaSet::new(&[
            area("migrations", &["crates/pdt-store/migrations/**"], true),
            area("web/ui", &["web/ui/**"], false),
            area("docs", &["docs/*.md"], false),
        ])
    }

    fn p(id: &str, active: bool, areas: &[&str], files: &[&str]) -> Participant {
        Participant {
            id: id.into(),
            active,
            areas: areas.iter().map(|a| a.to_string()).collect(),
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn globs_respect_folders() {
        let s = set();
        assert!(s.is_exclusive_file("crates/pdt-store/migrations/0012_x.sql"));
        assert!(s.is_exclusive_file("crates/pdt-store/migrations/sub/a.sql"));
        assert!(!s.is_exclusive_file("crates/pdt-store/src/lib.rs"));
        assert!(!s.is_exclusive_file("web/ui/Button.tsx"), "web/ui is not exclusive");
        let docs = AreaSet::new(&[area("d", &["docs/*.md"], true)]);
        assert!(docs.is_exclusive_file("docs/a.md"));
        assert!(!docs.is_exclusive_file("docs/sub/a.md"), "* stays within a folder");
        assert!(AreaSet::new(&[area("d", &["./docs/**"], true)]).is_exclusive_file("docs/x"));
        assert!(glob("src/[").is_err());
        assert!(!AreaSet::default().is_exclusive_file("x"));
    }

    #[test]
    fn planned_overlap_is_against_active_tickets_only() {
        let s = set();
        let t = [
            p("a", true, &["migrations", "web/ui"], &[]),
            p("b", true, &["web/ui"], &[]),
            p("ready", false, &["migrations"], &[]),
            p("waiting", false, &["migrations"], &[]),
            p("other", false, &["docs"], &[]),
        ];
        let o = compute(&s, &t);
        assert_eq!(
            o["a"],
            [OverlapWith {
                other: "b".into(),
                level: OverlapLevel::Overlap,
                areas: vec!["web/ui".into()],
                files: vec![]
            }]
        );
        assert_eq!(o["b"][0].other, "a");
        // Not active: overlap with the active ticket holding the area, never with each other.
        assert_eq!(
            o["ready"],
            [OverlapWith {
                other: "a".into(),
                level: OverlapLevel::Conflict,
                areas: vec!["migrations".into()],
                files: vec![]
            }]
        );
        assert_eq!(o["waiting"].len(), 1);
        assert!(!o.contains_key("other"));
        assert_eq!(level_of(&o["a"]), OverlapLevel::Overlap);
        assert_eq!(level_of(&o["ready"]), OverlapLevel::Conflict);
        assert_eq!(level_of(&[]), OverlapLevel::None);
    }

    #[test]
    fn unconfigured_areas_are_ignored() {
        let o = compute(&set(), &[p("a", true, &["gone"], &[]), p("b", false, &["gone"], &[])]);
        assert!(o.is_empty());
    }

    #[test]
    fn actual_overlap_needs_two_active_attempts_and_exclusive_files_conflict() {
        let s = set();
        let t = [
            p("a", true, &[], &["web/src/App.tsx", "crates/pdt-store/migrations/0012_a.sql", "README.md"]),
            p("b", true, &[], &["web/src/App.tsx", "README.md"]),
            p("c", true, &[], &["crates/pdt-store/migrations/0012_a.sql"]),
            p("d", false, &[], &["README.md"]),
        ];
        let o = compute(&s, &t);
        assert_eq!(o["a"].len(), 2);
        assert_eq!(o["a"][0].other, "c", "conflicts first");
        assert_eq!(o["a"][0].level, OverlapLevel::Conflict);
        assert_eq!(o["a"][1].files, ["README.md", "web/src/App.tsx"]);
        assert_eq!(o["a"][1].level, OverlapLevel::Overlap);
        assert_eq!(o["b"].len(), 1);
        assert!(!o.contains_key("d"), "files only count for active attempts");
    }

    #[test]
    fn areas_and_files_combine_into_one_entry() {
        let t = [p("a", true, &["web/ui"], &["web/ui/x.tsx"]), p("b", true, &["web/ui", "docs"], &["web/ui/x.tsx"])];
        let o = compute(&set(), &t);
        assert_eq!(
            o["a"],
            [OverlapWith {
                other: "b".into(),
                level: OverlapLevel::Overlap,
                areas: vec!["web/ui".into()],
                files: vec!["web/ui/x.tsx".into()]
            }]
        );
    }
}
