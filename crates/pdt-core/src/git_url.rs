//! Reading git remote URLs: the repository name a clone gets, a remote's host and path (which a forge plugin claims,
//! PDT-241), and `owner/repo` for GitHub links.

/// The last path segment of a remote URL without `.git`: what `git clone` names the folder. None if there is none.
/// Handles `https://host/owner/repo(.git)`, `git@host:owner/repo.git`, `ssh://git@host/owner/repo` and local paths.
pub fn repo_name(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches(['/', '\\']);
    let last = u.rsplit(['/', '\\', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    let ok = !name.is_empty() && name != "." && name != ".." && !name.contains(['\0']);
    ok.then(|| name.to_string())
}

/// A remote on a host: `https://host/owner/repo.git`, `git@host:owner/repo.git` or `ssh://git@host:22/owner/repo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// The host, lower case, without user or port.
    pub host: String,
    /// The path's segments without `.git`: `[owner, repo]`, or more for a forge with nested groups.
    pub path: Vec<String>,
}

impl Remote {
    /// `owner/repo` when the path is exactly two segments.
    pub fn owner_repo(&self) -> Option<String> {
        match self.path.as_slice() {
            [owner, repo] => Some(format!("{owner}/{repo}")),
            _ => None,
        }
    }
}

/// The host and path of a remote URL; None for a local path or a URL without at least `owner/repo`.
pub fn parse_remote(url: &str) -> Option<Remote> {
    let u = url.trim();
    let (host, rest) = match u.split_once("://") {
        Some((_, no_scheme)) => {
            let (host_part, path) = no_scheme.split_once('/')?;
            // Drop any user info (`git@`, `token@`) and port.
            let host = host_part.rsplit_once('@').map_or(host_part, |(_, h)| h);
            (host.split(':').next()?, path)
        }
        // scp-style: `[user@]host:path`, not a Windows drive (`C:\…`) or a local path.
        None => {
            let (head, path) = u.split_once(':')?;
            let host = head.rsplit_once('@').map_or(head, |(_, h)| h);
            if host.len() < 2 || head.contains(['/', '\\']) || path.starts_with('\\') {
                return None;
            }
            (host, path)
        }
    };
    if host.is_empty() {
        return None;
    }
    let mut path: Vec<String> = rest.trim_end_matches('/').split('/').map(String::from).collect();
    if let Some(last) = path.last_mut()
        && let Some(s) = last.strip_suffix(".git")
    {
        *last = s.to_string();
    }
    if path.len() < 2 || path.iter().any(|s| s.is_empty() || s == "." || s == "..") {
        return None;
    }
    Some(Remote { host: host.to_ascii_lowercase(), path })
}

/// `owner/repo` when the URL points at a repository on github.com (https, ssh or scp-style).
pub fn github_repo(url: &str) -> Option<String> {
    github_repo_on(url, "github.com")
}

/// `owner/repo` when the URL points at a repository on `host` (a GitHub Enterprise Server's, or github.com).
pub fn github_repo_on(url: &str, host: &str) -> Option<String> {
    let r = parse_remote(url)?;
    let is_host = r.host.eq_ignore_ascii_case(host)
        || (host.eq_ignore_ascii_case("github.com") && r.host.eq_ignore_ascii_case("www.github.com"));
    if is_host { r.owner_repo() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_names_from_urls() {
        for (url, want) in [
            ("https://github.com/example-org/PersonalDevTeam", Some("PersonalDevTeam")),
            ("https://github.com/o/r.git", Some("r")),
            ("https://github.com/o/r/", Some("r")),
            ("git@github.com:o/r.git", Some("r")),
            ("ssh://git@host.example:2222/o/r.git", Some("r")),
            ("/srv/git/thing.git", Some("thing")),
            ("C:\\repos\\thing", Some("thing")),
            ("", None),
            ("https://github.com/o/..", None),
        ] {
            assert_eq!(repo_name(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn github_repos_from_remotes() {
        for (url, want) in [
            ("https://github.com/example-org/PersonalDevTeam.git", Some("example-org/PersonalDevTeam")),
            ("https://github.com/o/r", Some("o/r")),
            ("https://token@github.com/o/r.git", Some("o/r")),
            ("git@github.com:o/r.git", Some("o/r")),
            ("ssh://git@github.com/o/r.git", Some("o/r")),
            ("ssh://git@github.com:22/o/r", Some("o/r")),
            ("https://GitHub.com/o/r/", Some("o/r")),
            ("https://gitlab.com/o/r.git", None),
            ("https://github.com/o", None),
            ("https://github.com/o/r/tree/main", None),
            ("/local/path", None),
            ("git@gitlab.com:o/r.git", None),
        ] {
            assert_eq!(github_repo(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn remotes_on_any_host() {
        let r = |host: &str, path: &[&str]| {
            Some(Remote { host: host.into(), path: path.iter().map(|s| s.to_string()).collect() })
        };
        for (url, want) in [
            ("https://gitlab.com/group/sub/repo.git", r("gitlab.com", &["group", "sub", "repo"])),
            ("git@Codeberg.org:o/r.git", r("codeberg.org", &["o", "r"])),
            ("ssh://git@gitea.example:2222/o/r", r("gitea.example", &["o", "r"])),
            ("https://user:token@ghe.example.com/o/r/", r("ghe.example.com", &["o", "r"])),
            ("host:o/r", r("host", &["o", "r"])),
            ("/srv/git/o/r.git", None),
            ("C:\\repos\\thing", None),
            ("file:///srv/git/r.git", None),
            ("https://example.com/r", None),
            ("", None),
        ] {
            assert_eq!(parse_remote(url), want, "{url}");
        }
        assert_eq!(parse_remote("https://gitlab.com/g/s/r").unwrap().owner_repo(), None);
    }

    #[test]
    fn enterprise_repos_are_on_their_host() {
        for (url, want) in [
            ("https://ghe.example.com/o/r.git", Some("o/r")),
            ("git@ghe.example.com:o/r.git", Some("o/r")),
            ("ssh://git@GHE.example.com/o/r", Some("o/r")),
            ("https://github.com/o/r", None),
            ("git@github.com:o/r.git", None),
        ] {
            assert_eq!(github_repo_on(url, "ghe.example.com").as_deref(), want, "{url}");
        }
    }
}
