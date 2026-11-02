//! A ticket's links to issues in other trackers, and its plain web links (PDT-481). Pure: each tracker plugin
//! recognizes its own URLs and keys (pdt-api's `Integration::find_issue`) with the helpers here, and reads the issue
//! through its connection.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How a link to an issue was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkOrigin {
    /// PDT filed the issue from the ticket.
    Created,
    /// The ticket was imported from the issue.
    Imported,
    /// Someone linked an issue that was already there.
    Manual,
}

impl LinkOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkOrigin::Created => "created",
            LinkOrigin::Imported => "imported",
            LinkOrigin::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [LinkOrigin::Created, LinkOrigin::Imported, LinkOrigin::Manual].into_iter().find(|o| o.as_str() == s)
    }

    /// Whether Done closes (or moves) an issue linked this way unless the user says otherwise: the issues PDT made
    /// or took the ticket from do; someone else's issue, linked by hand, never does by surprise.
    pub fn closes_by_default(&self) -> bool {
        !matches!(self, LinkOrigin::Manual)
    }
}

/// An issue's state, as its tracker last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IssueState {
    Open,
    Closed,
}

impl IssueState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [IssueState::Open, IssueState::Closed].into_iter().find(|o| o.as_str() == s)
    }
}

/// How long an issue's title and state are kept before viewing the ticket reads them again.
pub const REFRESH_AFTER_MS: i64 = 5 * 60 * 1000;

/// Whether a link's issue, last read at `checked_at`, is read again at `now`.
pub fn stale(checked_at: Option<i64>, now: i64) -> bool {
    checked_at.is_none_or(|t| now - t >= REFRESH_AFTER_MS)
}

/// A plain link on a ticket (docs, a design): an address no tracker recognizes, with a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebLink {
    pub id: String,
    pub url: String,
    /// What it shows as (`Docs`, `Figma`); the address's host when none was given.
    pub label: String,
}

/// The longest label a plain link takes.
pub const MAX_LABEL: usize = 80;
/// The longest address a link takes.
pub const MAX_URL: usize = 2048;

/// The host (lower case, no port) and path segments of an `http(s)` address, without its query and fragment.
pub fn url_parts(url: &str) -> Option<(String, Vec<String>)> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"))?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    let segments = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    Some((host, segments))
}

/// The query parameter `name` of an address, if it has one.
pub fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next()?;
    query.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k == name && !v.is_empty()).then(|| v.to_string())
    })
}

/// A plain link checked: an `http(s)` address and its label (the host when `label` is blank). Err is for the user.
pub fn web_link(url: &str, label: &str) -> Result<(String, String), String> {
    let url = url.trim();
    if url.chars().count() > MAX_URL {
        return Err(format!("a link's address must be at most {MAX_URL} characters"));
    }
    let (host, _) = url_parts(url).ok_or_else(|| format!("{url:?} is not a web address (https://…)"))?;
    let label = match label.trim() {
        "" => host,
        l if l.chars().count() > MAX_LABEL => {
            return Err(format!("a link's label must be at most {MAX_LABEL} characters"));
        }
        l => l.to_string(),
    };
    Ok((url.to_string(), label))
}

/// An issue named within a repository: `owner/name#12` (any depth of groups, GitLab's `group/sub/app#12`), or `#12`
/// for the project's own repository (`repo` None). The number is above zero.
pub fn repo_issue_key(given: &str) -> Option<(Option<String>, u64)> {
    let (repo, n) = given.trim().split_once('#')?;
    let number = n.trim().parse::<u64>().ok().filter(|n| *n > 0)?;
    if repo.is_empty() {
        return Some((None, number));
    }
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    let parts: Vec<&str> = repo.split('/').collect();
    (parts.len() >= 2 && parts.iter().all(|p| ok(p))).then(|| (Some(repo.to_string()), number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_and_states_round_trip() {
        for o in [LinkOrigin::Created, LinkOrigin::Imported, LinkOrigin::Manual] {
            assert_eq!(LinkOrigin::parse(o.as_str()), Some(o));
        }
        for s in [IssueState::Open, IssueState::Closed] {
            assert_eq!(IssueState::parse(s.as_str()), Some(s));
        }
        assert_eq!(LinkOrigin::parse("nope"), None);
    }

    #[test]
    fn only_issues_linked_by_hand_stay_open_by_default() {
        assert!(LinkOrigin::Created.closes_by_default());
        assert!(LinkOrigin::Imported.closes_by_default());
        assert!(!LinkOrigin::Manual.closes_by_default());
    }

    #[test]
    fn a_link_is_read_again_after_five_minutes() {
        assert!(stale(None, 0));
        assert!(!stale(Some(1_000), 1_000 + REFRESH_AFTER_MS - 1));
        assert!(stale(Some(1_000), 1_000 + REFRESH_AFTER_MS));
    }

    #[test]
    fn urls_split_into_host_and_path() {
        let p = |u: &str| url_parts(u).map(|(h, s)| (h, s.join("/")));
        assert_eq!(p("https://GitHub.com/o/r/issues/3?x=1#c"), Some(("github.com".into(), "o/r/issues/3".into())));
        assert_eq!(
            p("http://jira.example.com:8080/browse/AB-1/"),
            Some(("jira.example.com".into(), "browse/AB-1".into()))
        );
        assert_eq!(p("https://example.com"), Some(("example.com".into(), String::new())));
        assert_eq!(p("ftp://example.com/x"), None);
        assert_eq!(p("example.com/x"), None);
        assert_eq!(p("https:///x"), None);
    }

    #[test]
    fn query_parameters_are_read() {
        let u = "https://s.atlassian.net/jira/software/projects/AB/boards/1?selectedIssue=AB-7&x=#top";
        assert_eq!(query_param(u, "selectedIssue").as_deref(), Some("AB-7"));
        assert_eq!(query_param(u, "x"), None);
        assert_eq!(query_param("https://a/b", "x"), None);
    }

    #[test]
    fn plain_links_need_a_web_address_and_default_their_label_to_the_host() {
        assert_eq!(
            web_link(" https://www.figma.com/file/abc ", ""),
            Ok(("https://www.figma.com/file/abc".into(), "www.figma.com".into()))
        );
        assert_eq!(web_link("https://docs.rs/x", " Docs "), Ok(("https://docs.rs/x".into(), "Docs".into())));
        assert!(web_link("not a url", "Docs").is_err());
        assert!(web_link("https://a.b/c", &"x".repeat(MAX_LABEL + 1)).is_err());
    }

    #[test]
    fn repo_issue_keys_name_a_repository_or_the_projects_own() {
        assert_eq!(repo_issue_key("o/r#12"), Some((Some("o/r".into()), 12)));
        assert_eq!(repo_issue_key(" group/sub/app#3 "), Some((Some("group/sub/app".into()), 3)));
        assert_eq!(repo_issue_key("#4"), Some((None, 4)));
        assert_eq!(repo_issue_key("o#4"), None, "a repository has an owner");
        assert_eq!(repo_issue_key("o/r#0"), None);
        assert_eq!(repo_issue_key("o/r#x"), None);
        assert_eq!(repo_issue_key("o//r#1"), None);
        assert_eq!(repo_issue_key("ABC-12"), None);
        assert_eq!(repo_issue_key("12"), None, "a bare number is too easily something else");
    }
}
