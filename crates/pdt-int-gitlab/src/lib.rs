//! GitLab (PDT-246): the GitLab forge for projects on gitlab.com and on self-hosted GitLab servers (merge requests,
//! notes and discussions, approvals, issues, releases, auto-merge and merge trains), and GitLab CI as its CI provider
//! (pipelines, jobs, logs, retries). pdtd's `gitlab` feature registers [`plugin::GitlabPlugin`] in the forge registry
//! and [`Gitlab`] as the plugin the user turns on in Settings › Plugins, where they sign in.

pub mod api;
pub mod ci;
pub mod forge;
pub mod login;
pub mod plugin;

pub use api::GitlabApi;
pub use ci::GitlabCi;
pub use plugin::{GITLAB, GitlabPlugin};

use pdt_api::sdk::issues::{FoundIssue, IssueNow, StoredExternalLink, find_forge_issue, read_forge_link};
use pdt_api::sdk::{AppState, BoxFut, Integration};

/// The GitLab plugin: while it is on, projects whose `origin` is on gitlab.com (or a GitLab PDT has a login for) use
/// the GitLab forge.
pub struct Gitlab;

impl Integration for Gitlab {
    fn id(&self) -> &'static str {
        "gitlab"
    }

    fn forge_kind(&self) -> Option<pdt_core::forge::ForgeKind> {
        Some(GITLAB)
    }

    /// An issue's page on gitlab.com or a GitLab PDT signs in to, or `group/app#12` (`#12`) in a project on GitLab.
    fn find_issue<'a>(
        &'a self,
        st: &'a AppState,
        project_id: &'a str,
        given: &'a str,
    ) -> BoxFut<'a, Option<Result<FoundIssue, String>>> {
        Box::pin(async move { find_forge_issue(st, &GITLAB, project_id, given, issue_url).await })
    }

    fn read_issue<'a>(
        &'a self,
        st: &'a AppState,
        link: &'a StoredExternalLink,
    ) -> BoxFut<'a, Option<Result<IssueNow, String>>> {
        Box::pin(async move { read_forge_link(st, &GITLAB, link, issue_url).await })
    }

    fn issue_example(&self) -> Option<&'static str> {
        Some("group/project#12, or the issue's address")
    }
}

/// The host (lower case), project path and number of a GitLab issue's page
/// (`https://<host>/<group…>/<project>/-/issues/12`, or `…/-/work_items/12`).
pub fn issue_url(url: &str) -> Option<(String, String, u64)> {
    let (host, path) = pdt_core::issue_link::url_parts(url)?;
    let dash = path.iter().position(|s| s == "-")?;
    let [kind, n] = &path[dash + 1..] else { return None };
    if dash < 2 || !matches!(kind.as_str(), "issues" | "work_items") {
        return None;
    }
    let number = n.parse::<u64>().ok().filter(|n| *n > 0)?;
    Some((host, path[..dash].join("/"), number))
}

#[cfg(test)]
mod tests;
