//! Reading git remote URLs: the repository name a clone gets, and `owner/repo` for GitHub links.

/// The last path segment of a remote URL without `.git`: what `git clone` names the folder. None if there is none.
/// Handles `https://host/owner/repo(.git)`, `git@host:owner/repo.git`, `ssh://git@host/owner/repo` and local paths.
pub fn repo_name(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches(['/', '\\']);
    let last = u.rsplit(['/', '\\', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    let ok = !name.is_empty() && name != "." && name != ".." && !name.contains(['\0']);
    ok.then(|| name.to_string())
}

/// `owner/repo` when the URL points at a repository on github.com (https, ssh or scp-style).
pub fn github_repo(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = if let Some(r) = u.strip_prefix("git@github.com:") {
        r
    } else {
        let no_scheme = u.split_once("://").map(|(_, r)| r)?;
        // Drop any user info (`git@`, `token@`).
        let host_path = no_scheme.rsplit_once('@').map_or(no_scheme, |(_, r)| r);
        let (host, path) = host_path.split_once('/')?;
        let host = host.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
            return None;
        }
        path
    };
    let mut parts = rest.trim_end_matches('/').split('/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    if parts.next().is_some() {
        return None;
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (!repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_names_from_urls() {
        for (url, want) in [
            ("https://github.com/ross-game-tools/PersonalDevTeam", Some("PersonalDevTeam")),
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
            ("https://github.com/ross-game-tools/PersonalDevTeam.git", Some("ross-game-tools/PersonalDevTeam")),
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
}
