//! A ticket's link to a GitHub issue (PDT-154): how the link names the issue, and how an issue is closed. Pure:
//! pdt-api reads and writes issues through the GitHub forge (pdt-github).

/// The `system` of a ticket's link to a GitHub issue.
pub const GITHUB: &str = "github";

/// The label PDT instances put on the bug reports they file on the release repository (PDT-153).
pub const FROM_PDT: &str = "from-pdt";

/// A repository as a link stores it (`owner/name`, lower case: GitHub's names ignore case).
pub fn repo_id(repo: &str) -> String {
    repo.trim().to_ascii_lowercase()
}

/// `owner/name` with both parts, or None.
pub fn valid_repo(repo: &str) -> Option<String> {
    let r = repo.trim();
    let mut parts = r.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (parts.next().is_none() && ok(owner) && ok(name)).then(|| r.to_string())
}

/// The link's key for issue `number` of `repo`: `owner/name#12`, lower case.
pub fn issue_key(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo_id(repo))
}

/// The repository and number of an issue given as `owner/name#12`, its URL (`https://github.com/owner/name/issues/12`)
/// or, with `repo`, a bare `12` or `#12`.
pub fn parse_issue(given: &str, repo: Option<&str>) -> Option<(String, u64)> {
    let g = given.trim();
    let number = |s: &str| s.trim().trim_start_matches('#').parse::<u64>().ok().filter(|n| *n > 0);
    if let Some(rest) = g.strip_prefix("https://github.com/").or_else(|| g.strip_prefix("http://github.com/")) {
        let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
        if let [owner, name, "issues", n] = parts.as_slice() {
            return Some((valid_repo(&format!("{owner}/{name}"))?, number(n)?));
        }
        return None;
    }
    if let Some((r, n)) = g.split_once('#')
        && !r.is_empty()
    {
        return Some((valid_repo(r)?, number(n)?));
    }
    Some((valid_repo(repo?)?, number(g)?))
}

/// The host (lower case), repository and number of a GitHub issue's page on any host
/// (`https://<host>/owner/name/issues/12`): github.com's, or an Enterprise Server's. A pull request's is not one.
pub fn issue_url(url: &str) -> Option<(String, String, u64)> {
    let (host, path) = crate::issue_link::url_parts(url)?;
    let [owner, name, kind, n] = path.as_slice() else { return None };
    let number = n.parse::<u64>().ok().filter(|n| *n > 0)?;
    (kind == "issues").then_some(())?;
    Some((host, valid_repo(&format!("{owner}/{name}"))?, number))
}

/// Why an issue is closed (GitHub's `state_reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Completed,
    NotPlanned,
    /// Closed as a duplicate of another issue (GitHub's "Close as duplicate").
    Duplicate,
}

impl CloseReason {
    pub fn parse(s: &str) -> Option<CloseReason> {
        match s.trim() {
            "completed" => Some(CloseReason::Completed),
            "not_planned" => Some(CloseReason::NotPlanned),
            "duplicate" => Some(CloseReason::Duplicate),
            _ => None,
        }
    }

    /// GitHub's name for it (REST `state_reason`).
    pub fn as_str(&self) -> &'static str {
        match self {
            CloseReason::Completed => "completed",
            CloseReason::NotPlanned => "not_planned",
            CloseReason::Duplicate => "duplicate",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_keys_are_lower_case_owner_name_and_number() {
        assert_eq!(issue_key(" Example-Org/PersonalDevTeam ", 12), "example-org/personaldevteam#12");
        assert_eq!(repo_id("O/R"), "o/r");
    }

    #[test]
    fn issues_parse_from_keys_urls_and_numbers() {
        let o = |r: &str, n: u64| Some((r.to_string(), n));
        assert_eq!(parse_issue("o/r#12", None), o("o/r", 12));
        assert_eq!(parse_issue("https://github.com/Own/Repo/issues/7", None), o("Own/Repo", 7));
        assert_eq!(parse_issue("https://github.com/o/r/pull/7", None), None, "a pull request is not an issue");
        assert_eq!(parse_issue("#3", Some("o/r")), o("o/r", 3));
        assert_eq!(parse_issue("3", Some("o/r")), o("o/r", 3));
        assert_eq!(parse_issue("3", None), None, "a bare number needs its repo");
        assert_eq!(parse_issue("0", Some("o/r")), None);
        assert_eq!(parse_issue("o#3", None), None);
        assert_eq!(parse_issue("a/b/c#3", None), None);
        assert_eq!(parse_issue("o/r#x", None), None);
        assert_eq!(valid_repo("o/r s"), None);
    }

    #[test]
    fn issue_pages_parse_on_any_host() {
        let o = |h: &str, r: &str, n: u64| Some((h.to_string(), r.to_string(), n));
        assert_eq!(issue_url("https://github.com/Own/Repo/issues/7"), o("github.com", "Own/Repo", 7));
        assert_eq!(issue_url("https://github.com/o/r/issues/7#issuecomment-1"), o("github.com", "o/r", 7));
        assert_eq!(issue_url("https://ghe.example.com/o/r/issues/2/"), o("ghe.example.com", "o/r", 2));
        assert_eq!(issue_url("https://github.com/o/r/pull/7"), None);
        assert_eq!(issue_url("https://github.com/o/r/issues"), None);
        assert_eq!(issue_url("https://github.com/o/r/issues/0"), None);
        assert_eq!(issue_url("https://gitlab.com/g/p/-/issues/3"), None);
    }

    #[test]
    fn close_reasons_are_githubs() {
        for r in [CloseReason::Completed, CloseReason::NotPlanned, CloseReason::Duplicate] {
            assert_eq!(CloseReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(CloseReason::parse("wontfix"), None);
    }
}
