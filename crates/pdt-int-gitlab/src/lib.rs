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

use pdt_api::sdk::Integration;

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
}

#[cfg(test)]
mod tests;
