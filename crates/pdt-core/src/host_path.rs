//! Paths on the host, as text, in the host's own form (`/Users/me/work` on macOS, `C:\Users\me\work` on Windows)
//! whatever the browser's OS. Every function takes the [`PathStyle`], so the Windows rules are tested on any OS.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    Unix,
    Windows,
}

impl PathStyle {
    /// The style of the OS this build runs on.
    pub const fn host() -> Self {
        if cfg!(windows) { PathStyle::Windows } else { PathStyle::Unix }
    }

    pub const fn sep(self) -> char {
        match self {
            PathStyle::Unix => '/',
            PathStyle::Windows => '\\',
        }
    }

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == PathStyle::Windows && c == '\\')
    }
}

/// An absolute path split into its root (`/`, `C:\`, `\\server\share\`) and its folder names, with `.` and `..`
/// resolved by name (`..` at the root stays there). `None` when `path` is not absolute.
pub fn split(path: &str, style: PathStyle) -> Option<(String, Vec<String>)> {
    let (root, rest) = match style {
        PathStyle::Unix => (String::from("/"), path.strip_prefix('/')?),
        PathStyle::Windows => windows_root(path)?,
    };
    let mut names: Vec<String> = Vec::new();
    for seg in rest.split(|c| style.is_sep(c)) {
        match seg {
            "" | "." => {}
            ".." => {
                names.pop();
            }
            s => names.push(s.to_string()),
        }
    }
    Some((root, names))
}

/// `C:\…` (drive letter upper-cased), `\\server\share\…` and the verbatim `\\?\C:\…` / `\\?\UNC\server\share\…`.
fn windows_root(path: &str) -> Option<(String, &str)> {
    let is_sep = |c: char| c == '\\' || c == '/';
    let path = match path.strip_prefix(r"\\?\") {
        Some(p) => match p.strip_prefix(r"UNC\") {
            Some(unc) => return unc_root(unc),
            None => p,
        },
        None => path,
    };
    let b = path.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && is_sep(b[2] as char) {
        return Some((format!("{}:\\", (b[0] as char).to_ascii_uppercase()), &path[3..]));
    }
    if b.len() >= 2 && is_sep(b[0] as char) && is_sep(b[1] as char) {
        return unc_root(&path[2..]);
    }
    None
}

fn unc_root(p: &str) -> Option<(String, &str)> {
    let mut parts = p.splitn(3, ['\\', '/']);
    let server = parts.next().filter(|s| !s.is_empty())?;
    let share = parts.next().filter(|s| !s.is_empty())?;
    Some((format!(r"\\{server}\{share}\"), parts.next().unwrap_or("")))
}

/// `root` followed by `names`, joined with the style's separator.
pub fn join(root: &str, names: &[String], style: PathStyle) -> String {
    let sep = style.sep().to_string();
    format!("{root}{}", names.join(&sep))
}

/// `path` in its plain form (one separator style, no `.`/`..`, no trailing separator); `None` when not absolute.
pub fn normalize(path: &str, style: PathStyle) -> Option<String> {
    let (root, names) = split(path.trim(), style)?;
    Some(join(&root, &names, style))
}

/// Expands `~` / `~/…` (and `~\…` on Windows) to `home`, then normalizes. `None` when the result is not absolute
/// (or `~` is used with no home).
pub fn expand(input: &str, home: Option<&str>, style: PathStyle) -> Option<String> {
    let s = input.trim();
    let rest = match s.strip_prefix('~') {
        Some(r) if r.is_empty() || r.starts_with(|c| style.is_sep(c)) => Some(r),
        _ => None,
    };
    match rest {
        Some(r) => normalize(&format!("{}{}{r}", home?, style.sep()), style),
        None => normalize(s, style),
    }
}

/// Whether two names are the same on this OS (Windows ignores case).
fn same_name(a: &str, b: &str, style: PathStyle) -> bool {
    match style {
        PathStyle::Unix => a == b,
        PathStyle::Windows => a.to_lowercase() == b.to_lowercase(),
    }
}

/// Whether `a` and `b` name the same folder (after normalizing; case-insensitive on Windows).
pub fn same_path(a: &str, b: &str, style: PathStyle) -> bool {
    match (split(a, style), split(b, style)) {
        (Some((ra, na)), Some((rb, nb))) => {
            same_name(&ra, &rb, style)
                && na.len() == nb.len()
                && na.iter().zip(&nb).all(|(x, y)| same_name(x, y, style))
        }
        _ => false,
    }
}

/// The names of `path` below `home`, when `path` is `home` or inside it.
fn below_home(path: &str, home: Option<&str>, style: PathStyle) -> Option<Vec<String>> {
    let (root, names) = split(path, style)?;
    let (home_root, home_names) = split(home?, style)?;
    let inside = same_name(&root, &home_root, style)
        && names.len() >= home_names.len()
        && home_names.iter().zip(&names).all(|(h, n)| same_name(h, n, style));
    inside.then(|| names[home_names.len()..].to_vec())
}

/// `path` with the home folder written `~` (`~`, `~/work`, `~\work` on Windows); otherwise its plain form.
pub fn tilde(path: &str, home: Option<&str>, style: PathStyle) -> String {
    match below_home(path, home, style) {
        Some(rest) if rest.is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", style.sep(), rest.join(&style.sep().to_string())),
        None => normalize(path, style).unwrap_or_else(|| path.to_string()),
    }
}

/// One step of a breadcrumb: its label and the full path it goes to.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Crumb {
    pub name: String,
    pub path: String,
}

/// The breadcrumb of `path`: from `~` when it is inside the home folder, else from its root (`/`, `C:\`).
pub fn crumbs(path: &str, home: Option<&str>, style: PathStyle) -> Vec<Crumb> {
    let Some((root, names)) = split(path, style) else { return Vec::new() };
    let (start, first) = match (below_home(path, home, style), home.and_then(|h| split(h, style))) {
        (Some(_), Some((_, home_names))) => ("~".to_string(), home_names.len()),
        _ => (root.clone(), 0),
    };
    let mut out = vec![Crumb { name: start, path: join(&root, &names[..first], style) }];
    out.extend(
        names
            .iter()
            .enumerate()
            .skip(first)
            .map(|(i, n)| Crumb { name: n.clone(), path: join(&root, &names[..=i], style) }),
    );
    out
}

/// The folder above `path`; `None` at a root.
pub fn parent(path: &str, style: PathStyle) -> Option<String> {
    let (root, mut names) = split(path, style)?;
    names.pop()?;
    Some(join(&root, &names, style))
}

/// Why a new folder's name is refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FolderNameError {
    #[error("enter a name")]
    Empty,
    #[error("a folder name cannot contain {0:?}")]
    BadChar(char),
    #[error("{0:?} cannot be used as a folder name")]
    Reserved(String),
}

/// A name for a new folder (trimmed): no separators or control characters; on Windows none of `<>:"|?*`, no
/// trailing dot and none of the reserved device names (`CON`, `NUL`, `COM1`…).
pub fn folder_name(name: &str, style: PathStyle) -> Result<String, FolderNameError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(FolderNameError::Empty);
    }
    if n == "." || n == ".." {
        return Err(FolderNameError::Reserved(n.into()));
    }
    let bad: &[char] = match style {
        PathStyle::Unix => &['/'],
        PathStyle::Windows => &['/', '\\', '<', '>', ':', '"', '|', '?', '*'],
    };
    if let Some(c) = n.chars().find(|c| bad.contains(c) || c.is_control()) {
        return Err(FolderNameError::BadChar(c));
    }
    if style == PathStyle::Windows {
        let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if device || n.ends_with('.') {
            return Err(FolderNameError::Reserved(n.into()));
        }
    }
    Ok(n.to_string())
}

#[cfg(test)]
mod tests {
    use super::PathStyle::{Unix, Windows};
    use super::*;

    #[test]
    fn splits_unix_paths() {
        assert_eq!(
            split("/Users/me/./work/../code/", Unix),
            Some(("/".into(), vec!["Users".into(), "me".into(), "code".into()]))
        );
        assert_eq!(split("/..", Unix), Some(("/".into(), vec![])));
        assert_eq!(split("work", Unix), None);
        assert_eq!(split("C:\\x", Unix), None);
        // A backslash is an ordinary character on Unix.
        assert_eq!(normalize("/a\\b", Unix).as_deref(), Some("/a\\b"));
    }

    #[test]
    fn splits_windows_paths() {
        assert_eq!(normalize(r"c:\Users\me\", Windows).as_deref(), Some(r"C:\Users\me"));
        assert_eq!(normalize("C:/Users/me/work", Windows).as_deref(), Some(r"C:\Users\me\work"));
        assert_eq!(normalize(r"C:\", Windows).as_deref(), Some(r"C:\"));
        assert_eq!(normalize(r"C:\..\x", Windows).as_deref(), Some(r"C:\x"));
        assert_eq!(normalize(r"\\?\C:\Users", Windows).as_deref(), Some(r"C:\Users"));
        assert_eq!(normalize(r"\\nas\share\code", Windows).as_deref(), Some(r"\\nas\share\code"));
        assert_eq!(normalize(r"\\?\UNC\nas\share\code", Windows).as_deref(), Some(r"\\nas\share\code"));
        assert_eq!(normalize(r"C:relative", Windows), None);
        assert_eq!(normalize(r"\rooted", Windows), None);
        assert_eq!(normalize(r"\\nas", Windows), None);
        assert_eq!(normalize("/Users/me", Windows), None);
    }

    #[test]
    fn expands_the_home_folder() {
        assert_eq!(expand("~", Some("/Users/me"), Unix).as_deref(), Some("/Users/me"));
        assert_eq!(expand(" ~/work/ ", Some("/Users/me"), Unix).as_deref(), Some("/Users/me/work"));
        assert_eq!(expand(r"~\work", Some(r"C:\Users\me"), Windows).as_deref(), Some(r"C:\Users\me\work"));
        assert_eq!(expand("~/work", Some(r"C:\Users\me"), Windows).as_deref(), Some(r"C:\Users\me\work"));
        assert_eq!(expand("~bob", Some("/Users/me"), Unix), None);
        assert_eq!(expand("~/x", None, Unix), None);
        assert_eq!(expand("/srv", None, Unix).as_deref(), Some("/srv"));
    }

    #[test]
    fn writes_home_as_tilde() {
        assert_eq!(tilde("/Users/me", Some("/Users/me"), Unix), "~");
        assert_eq!(tilde("/Users/me/work/x", Some("/Users/me/"), Unix), "~/work/x");
        assert_eq!(tilde("/Users/meow", Some("/Users/me"), Unix), "/Users/meow");
        assert_eq!(tilde("/srv", Some("/Users/me"), Unix), "/srv");
        assert_eq!(tilde(r"c:\users\ME\work", Some(r"C:\Users\me"), Windows), r"~\work");
        assert_eq!(tilde(r"D:\code", Some(r"C:\Users\me"), Windows), r"D:\code");
        assert_eq!(tilde("/Users/me/work", None, Unix), "/Users/me/work");
    }

    #[test]
    fn compares_paths_per_os() {
        assert!(same_path("/a/b/", "/a/./b", Unix));
        assert!(!same_path("/a/B", "/a/b", Unix));
        assert!(same_path(r"C:\Code\X", "c:/code/x", Windows));
        assert!(!same_path("relative", "relative", Unix));
    }

    #[test]
    fn builds_breadcrumbs() {
        let c = |v: &[(&str, &str)]| {
            v.iter().map(|(n, p)| Crumb { name: (*n).into(), path: (*p).into() }).collect::<Vec<_>>()
        };
        assert_eq!(
            crumbs("/Users/me/work", Some("/Users/me"), Unix),
            c(&[("~", "/Users/me"), ("work", "/Users/me/work")])
        );
        assert_eq!(crumbs("/Users", Some("/Users/me"), Unix), c(&[("/", "/"), ("Users", "/Users")]));
        assert_eq!(crumbs("/", None, Unix), c(&[("/", "/")]));
        assert_eq!(
            crumbs(r"C:\Users\me\work", Some(r"C:\Users\me"), Windows),
            c(&[("~", r"C:\Users\me"), ("work", r"C:\Users\me\work")])
        );
        assert_eq!(
            crumbs(r"D:\code\x", Some(r"C:\Users\me"), Windows),
            c(&[(r"D:\", r"D:\"), ("code", r"D:\code"), ("x", r"D:\code\x")])
        );
    }

    #[test]
    fn finds_parents() {
        assert_eq!(parent("/Users/me", Unix).as_deref(), Some("/Users"));
        assert_eq!(parent("/Users", Unix).as_deref(), Some("/"));
        assert_eq!(parent("/", Unix), None);
        assert_eq!(parent(r"C:\Users", Windows).as_deref(), Some(r"C:\"));
        assert_eq!(parent(r"C:\", Windows), None);
    }

    #[test]
    fn checks_folder_names() {
        assert_eq!(folder_name(" new app ", Unix), Ok("new app".into()));
        assert_eq!(folder_name("", Unix), Err(FolderNameError::Empty));
        assert_eq!(folder_name("..", Unix), Err(FolderNameError::Reserved("..".into())));
        assert_eq!(folder_name("a/b", Unix), Err(FolderNameError::BadChar('/')));
        assert_eq!(folder_name("a:b", Unix), Ok("a:b".into()));
        assert_eq!(folder_name("a:b", Windows), Err(FolderNameError::BadChar(':')));
        assert_eq!(folder_name(r"a\b", Windows), Err(FolderNameError::BadChar('\\')));
        assert_eq!(folder_name("con.txt", Windows), Err(FolderNameError::Reserved("con.txt".into())));
        assert_eq!(folder_name("COM1", Windows), Err(FolderNameError::Reserved("COM1".into())));
        assert_eq!(folder_name("console", Windows), Ok("console".into()));
        assert_eq!(folder_name("x.", Windows), Err(FolderNameError::Reserved("x.".into())));
    }
}
