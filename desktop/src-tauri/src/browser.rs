//! Opens the web UI: as a chromeless app window in Edge/Chrome when one is installed, else in the default browser.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Other
        }
    }
}

/// Install roots the candidate list is built from (the environment on Windows, the home dir on macOS).
#[derive(Debug, Default, Clone)]
pub struct Roots {
    pub program_files_x86: Option<PathBuf>,
    pub program_files: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Roots {
    pub fn from_env() -> Roots {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        Roots {
            program_files_x86: var("ProgramFiles(x86)"),
            program_files: var("ProgramFiles"),
            local_app_data: var("LOCALAPPDATA"),
            home: dirs::home_dir(),
        }
    }
}

/// Chromium browsers that support `--app=<url>`, in order of preference: executables on Windows, app bundles on
/// macOS.
pub fn candidates(os: Os, roots: &Roots) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match os {
        Os::Windows => {
            let edge = ["Microsoft", "Edge", "Application", "msedge.exe"];
            let chrome = ["Google", "Chrome", "Application", "chrome.exe"];
            for (root, rel) in [
                (&roots.program_files_x86, &edge),
                (&roots.program_files, &edge),
                (&roots.program_files, &chrome),
                (&roots.program_files_x86, &chrome),
                (&roots.local_app_data, &chrome),
            ] {
                if let Some(root) = root {
                    out.push(rel.iter().fold(root.clone(), |p, part| p.join(part)));
                }
            }
        }
        Os::Mac => {
            for app in ["Google Chrome.app", "Microsoft Edge.app"] {
                out.push(Path::new("/Applications").join(app));
                if let Some(home) = &roots.home {
                    out.push(home.join("Applications").join(app));
                }
            }
        }
        Os::Other => {}
    }
    out
}

/// The command that opens `url` as an app window with the first installed candidate, or `None` when there is none
/// (the caller then uses the default browser).
pub fn app_window_command(
    os: Os,
    url: &str,
    candidates: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<(PathBuf, Vec<String>)> {
    let found = candidates.iter().find(|p| exists(p))?;
    let app_arg = format!("--app={url}");
    match os {
        Os::Windows => Some((found.clone(), vec![app_arg])),
        Os::Mac => Some((
            PathBuf::from("/usr/bin/open"),
            vec!["-na".into(), found.to_string_lossy().into_owned(), "--args".into(), app_arg],
        )),
        Os::Other => None,
    }
}

/// Opens `url` as an app window if a Chromium browser is installed. Returns false when the caller should fall back
/// to the default browser.
pub fn open_app_window(url: &str) -> bool {
    let os = Os::current();
    let list = candidates(os, &Roots::from_env());
    let exists = |p: &Path| if os == Os::Mac { p.is_dir() } else { p.is_file() };
    let Some((program, args)) = app_window_command(os, url, &list, exists) else {
        return false;
    };
    Command::new(program).args(args).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win_roots() -> Roots {
        Roots {
            program_files_x86: Some(PathBuf::from(r"C:\PF86")),
            program_files: Some(PathBuf::from(r"C:\PF")),
            local_app_data: Some(PathBuf::from(r"C:\LAD")),
            home: None,
        }
    }

    #[test]
    fn windows_prefers_edge_then_chrome() {
        let c = candidates(Os::Windows, &win_roots());
        let expect = [
            Path::new(r"C:\PF86").join("Microsoft").join("Edge").join("Application").join("msedge.exe"),
            Path::new(r"C:\PF").join("Microsoft").join("Edge").join("Application").join("msedge.exe"),
            Path::new(r"C:\PF").join("Google").join("Chrome").join("Application").join("chrome.exe"),
            Path::new(r"C:\PF86").join("Google").join("Chrome").join("Application").join("chrome.exe"),
            Path::new(r"C:\LAD").join("Google").join("Chrome").join("Application").join("chrome.exe"),
        ];
        assert_eq!(c, expect);
        let missing = Roots { program_files_x86: None, ..win_roots() };
        assert_eq!(candidates(Os::Windows, &missing).len(), 3);
    }

    #[test]
    fn mac_prefers_chrome_then_edge() {
        let roots = Roots { home: Some(PathBuf::from("/Users/me")), ..Roots::default() };
        let c = candidates(Os::Mac, &roots);
        assert_eq!(c[0], Path::new("/Applications/Google Chrome.app"));
        assert_eq!(c[1], Path::new("/Users/me/Applications/Google Chrome.app"));
        assert_eq!(c[2], Path::new("/Applications/Microsoft Edge.app"));
        assert_eq!(c.len(), 4);
        assert!(candidates(Os::Other, &roots).is_empty());
    }

    #[test]
    fn command_uses_first_existing_candidate() {
        let url = "http://127.0.0.1:7421/";
        let list = vec![PathBuf::from("a/msedge.exe"), PathBuf::from("b/chrome.exe")];
        let (prog, args) = app_window_command(Os::Windows, url, &list, |p| p.ends_with("chrome.exe")).unwrap();
        assert_eq!(prog, PathBuf::from("b/chrome.exe"));
        assert_eq!(args, vec!["--app=http://127.0.0.1:7421/"]);
        assert!(app_window_command(Os::Windows, url, &list, |_| false).is_none());

        let mac = vec![PathBuf::from("/Applications/Google Chrome.app")];
        let (prog, args) = app_window_command(Os::Mac, url, &mac, |_| true).unwrap();
        assert_eq!(prog, PathBuf::from("/usr/bin/open"));
        assert_eq!(args, vec!["-na", "/Applications/Google Chrome.app", "--args", "--app=http://127.0.0.1:7421/"]);
    }
}
