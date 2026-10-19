//! macOS: whether the app bundle sits where it can stay (PDT-51). An app opened where it was downloaded (still
//! quarantined) runs from a read-only App Translocation folder (`/private/var/folders/…/AppTranslocation/<uuid>/d/…`),
//! and one opened from a disk image runs from `/Volumes/…`. Neither can be updated in place (the installer unpacks
//! next to the bundle and swaps it), and a login item naming either breaks once the folder goes away.

use crate::install::bundle_of;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misplaced {
    /// Under an App Translocation folder.
    Translocated,
    /// The folder holding the bundle cannot be written to (a disk image, a read-only volume, no permission).
    ReadOnly,
    /// The bundle is gone (moved or deleted while it ran).
    Missing,
}

/// Whether `path` is inside an App Translocation folder (the same test as `SecTranslocateIsTranslocatedURL`'s
/// result for the paths macOS makes: they always contain this component).
pub fn is_translocated(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "AppTranslocation")
}

/// Whether a file can be made in `dir` (made and removed at once).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".pdt-write-check-{}", std::process::id()));
    match std::fs::File::options().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(e) => e.kind() == std::io::ErrorKind::AlreadyExists,
    }
}

/// What is wrong with where `bundle` is, if anything; `writable` answers for its parent folder.
pub fn problem_with(bundle: &Path, writable: impl Fn(&Path) -> bool) -> Option<Misplaced> {
    if is_translocated(bundle) {
        Some(Misplaced::Translocated)
    } else if !bundle.exists() {
        Some(Misplaced::Missing)
    } else if !bundle.parent().is_some_and(writable) {
        Some(Misplaced::ReadOnly)
    } else {
        None
    }
}

/// Why `bundle` cannot stay where it is, in words for the user.
pub fn reason(bundle: &Path, problem: Misplaced) -> String {
    let name = bundle.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "the app".into());
    let why = match problem {
        Misplaced::Translocated => {
            "macOS runs it from a temporary read-only folder because it was opened where it was downloaded".to_string()
        }
        Misplaced::ReadOnly => format!(
            "its folder ({}) cannot be written to",
            bundle.parent().map(|p| p.display().to_string()).unwrap_or_default()
        ),
        Misplaced::Missing => "it has been moved or deleted".to_string(),
    };
    format!("{why}; move {name} to the Applications folder and open it from there")
}

/// For the bundle that holds `exe_dir`: why it cannot stay there, or `None` when it can (or `exe_dir` is not in a
/// bundle at all, as in development).
pub fn bundle_problem(exe_dir: &Path) -> Option<String> {
    let bundle = bundle_of(exe_dir)?;
    problem_with(&bundle, writable).map(|p| reason(&bundle, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translocated_paths_are_recognised() {
        let t = Path::new(
            "/private/var/folders/x9/abc/T/AppTranslocation/0F1E-22/d/Personal Dev Team 2.app/Contents/MacOS",
        );
        assert!(is_translocated(t));
        assert!(!is_translocated(Path::new("/Users/me/Applications/Personal Dev Team.app/Contents/MacOS")));
        assert!(!is_translocated(Path::new("/Users/me/MyAppTranslocationNotes/P.app")));
    }

    #[test]
    fn problems_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("P.app");
        assert_eq!(problem_with(&bundle, |_| true), Some(Misplaced::Missing));
        std::fs::create_dir_all(&bundle).unwrap();
        assert_eq!(problem_with(&bundle, |_| true), None);
        assert_eq!(problem_with(&bundle, |_| false), Some(Misplaced::ReadOnly));
        let moved = tmp.path().join("AppTranslocation").join("u").join("d").join("P.app");
        std::fs::create_dir_all(&moved).unwrap();
        assert_eq!(problem_with(&moved, |_| true), Some(Misplaced::Translocated));
    }

    #[test]
    fn reasons_name_the_bundle_and_say_what_to_do() {
        let b = Path::new("/Volumes/PDT/Personal Dev Team 2.app");
        let r = reason(b, Misplaced::ReadOnly);
        assert!(r.contains("(/Volumes/PDT) cannot be written to"), "{r}");
        assert!(r.ends_with("move Personal Dev Team 2.app to the Applications folder and open it from there"), "{r}");
        assert!(reason(b, Misplaced::Translocated).contains("where it was downloaded"));
    }

    #[test]
    fn a_writable_folder_is_probed_without_leaving_anything() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(writable(tmp.path()));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        assert!(!writable(&tmp.path().join("missing")));
    }

    #[test]
    fn bundle_problem_reads_the_real_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("P.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&exe_dir).unwrap();
        assert_eq!(bundle_problem(&exe_dir), None);
        assert_eq!(bundle_problem(tmp.path()), None, "not in a bundle");
        let moved = tmp.path().join("AppTranslocation").join("P.app").join("Contents").join("MacOS");
        std::fs::create_dir_all(&moved).unwrap();
        assert!(bundle_problem(&moved).unwrap().contains("temporary read-only folder"));
    }
}
