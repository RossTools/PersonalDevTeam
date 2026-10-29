//! GitHub issues: the trigger on new issues in watched repositories and the GitHub actions; and for cutting a release
//! (PDT-194), starting a workflow and reading a pull request or a release. (An attempt's pull request and CI are PDT's
//! own: [`super::core`].)

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::github::{GithubClose, GithubComment, GithubImport};
use crate::workflow::nodes::release::{GithubChangeRequest, GithubRelease, GithubWorkflowRun};
use pdt_core::integration::{Icon, PluginCategory};

pub struct Github;

impl Integration for Github {
    fn id(&self) -> &'static str {
        "github"
    }

    fn label(&self) -> &'static str {
        "GitHub"
    }

    fn description(&self) -> &'static str {
        "Pull requests, CI, issues and releases for projects on GitHub, as your GitHub login."
    }

    fn category(&self) -> Option<PluginCategory> {
        Some(PluginCategory::Dev)
    }

    fn icon(&self) -> Icon {
        Icon { letters: "GH", color: "#24292f" }
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![
            Box::new(crate::github_issues::IssueOpened),
            Box::new(GithubImport),
            Box::new(GithubComment),
            Box::new(GithubClose),
            Box::new(GithubWorkflowRun),
            Box::new(GithubChangeRequest),
            Box::new(GithubRelease),
        ]
    }
}
