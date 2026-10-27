//! GitHub issues: the trigger on new issues in watched repositories and the GitHub actions. (Pull requests and CI
//! are PDT's own: [`super::core`].)

use super::Integration;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::github::{GithubClose, GithubComment, GithubImport};

pub struct Github;

impl Integration for Github {
    fn id(&self) -> &'static str {
        "github"
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![
            Box::new(crate::github_issues::IssueOpened),
            Box::new(GithubImport),
            Box::new(GithubComment),
            Box::new(GithubClose),
        ]
    }
}
