use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Absolute repo root, as reported by git, using native separators.
    pub repo_path: String,
    pub base_branch: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewProject {
    pub name: String,
    pub repo_path: String,
    #[serde(default)]
    #[ts(optional)]
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidProject {
    pub name: String,
    pub repo_path: String,
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("project name must not be empty")]
    EmptyName,
    #[error("repository path must not be empty")]
    EmptyPath,
    #[error("base branch must not be blank")]
    EmptyBranch,
}

impl NewProject {
    pub fn validate(&self) -> Result<ValidProject, ValidationError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        let repo_path = self.repo_path.trim();
        if repo_path.is_empty() {
            return Err(ValidationError::EmptyPath);
        }
        let base_branch = match self.base_branch.as_deref().map(str::trim) {
            Some("") => return Err(ValidationError::EmptyBranch),
            Some(b) => Some(b.to_string()),
            None => None,
        };
        Ok(ValidProject { name: name.into(), repo_path: repo_path.into(), base_branch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, path: &str, branch: Option<&str>) -> NewProject {
        NewProject { name: name.into(), repo_path: path.into(), base_branch: branch.map(Into::into) }
    }

    #[test]
    fn trims_fields() {
        let v = input("  PDT ", " C:\\repo ", Some(" main ")).validate().unwrap();
        assert_eq!(
            v,
            ValidProject { name: "PDT".into(), repo_path: "C:\\repo".into(), base_branch: Some("main".into()) }
        );
    }

    #[test]
    fn rejects_blank_name_and_path() {
        assert_eq!(input("  ", "/r", None).validate(), Err(ValidationError::EmptyName));
        assert_eq!(input("x", "   ", None).validate(), Err(ValidationError::EmptyPath));
    }

    #[test]
    fn blank_branch_is_error_but_missing_branch_is_fine() {
        assert_eq!(input("x", "/r", Some(" ")).validate(), Err(ValidationError::EmptyBranch));
        assert_eq!(input("x", "/r", None).validate().unwrap().base_branch, None);
    }
}
