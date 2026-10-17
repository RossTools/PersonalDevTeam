//! Workspaces and tabs (workspaces spec §2, §5). A workspace is an ordered strip of tabs; each tab shows one
//! session or one run, and a session or run has at most one tab.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Longest workspace name, in characters.
pub const MAX_WORKSPACE_NAME: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// Set for a project's own workspace, which can't be deleted while the project exists.
    pub project_id: Option<String>,
    pub position: u32,
    #[ts(type = "number")]
    pub created_at: i64,
    /// In order.
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Tab {
    pub id: String,
    pub workspace_id: String,
    /// Exactly one of `session_id` and `run_id` is set.
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub position: u32,
}

/// What a tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabItem {
    Session(String),
    Run(String),
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewWorkspace {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspacePatch {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    /// New index among the workspaces (clamped to the end).
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<u32>,
}

/// Opens a tab in a workspace, or moves the item's existing tab there.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenTab {
    #[serde(default)]
    #[ts(optional)]
    pub session_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
    /// Index in the workspace's tabs (clamped); the end when absent.
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TabPatch {
    /// Move the tab to this workspace (at `position`, else the end).
    #[serde(default)]
    #[ts(optional)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<u32>,
}

/// Where a new session or run opens its tab. Without one it goes at the end of its project's workspace.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TabPlacement {
    pub workspace_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WorkspaceValidationError {
    #[error("workspace name must not be empty")]
    EmptyName,
    #[error("workspace name must be at most {MAX_WORKSPACE_NAME} characters")]
    NameTooLong,
    #[error("a tab shows exactly one session or run")]
    TabItem,
}

/// A trimmed workspace name of 1..=60 characters.
pub fn valid_workspace_name(name: &str) -> Result<String, WorkspaceValidationError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(WorkspaceValidationError::EmptyName);
    }
    if name.chars().count() > MAX_WORKSPACE_NAME {
        return Err(WorkspaceValidationError::NameTooLong);
    }
    Ok(name.to_string())
}

impl OpenTab {
    pub fn item(&self) -> Result<TabItem, WorkspaceValidationError> {
        let nonblank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        match (nonblank(&self.session_id), nonblank(&self.run_id)) {
            (Some(s), None) => Ok(TabItem::Session(s)),
            (None, Some(r)) => Ok(TabItem::Run(r)),
            _ => Err(WorkspaceValidationError::TabItem),
        }
    }
}

/// `ids` with `id` moved (or inserted) to `index`, clamped to the end.
pub fn place_at(ids: &[String], id: &str, index: Option<usize>) -> Vec<String> {
    let mut out: Vec<String> = ids.iter().filter(|x| *x != id).cloned().collect();
    let at = index.unwrap_or(out.len()).min(out.len());
    out.insert(at, id.to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn place_at_inserts_moves_and_clamps() {
        let list = ids(&["a", "b", "c"]);
        assert_eq!(place_at(&list, "x", Some(1)), ids(&["a", "x", "b", "c"]));
        assert_eq!(place_at(&list, "x", None), ids(&["a", "b", "c", "x"]));
        assert_eq!(place_at(&list, "x", Some(99)), ids(&["a", "b", "c", "x"]));
        assert_eq!(place_at(&list, "a", Some(2)), ids(&["b", "c", "a"]));
        assert_eq!(place_at(&list, "c", Some(0)), ids(&["c", "a", "b"]));
        assert_eq!(place_at(&list, "b", None), ids(&["a", "c", "b"]));
        assert_eq!(place_at(&[], "a", Some(3)), ids(&["a"]));
    }

    #[test]
    fn workspace_names_are_trimmed_and_bounded() {
        assert_eq!(valid_workspace_name("  Release "), Ok("Release".into()));
        assert_eq!(valid_workspace_name("   "), Err(WorkspaceValidationError::EmptyName));
        assert_eq!(valid_workspace_name(&"é".repeat(60)), Ok("é".repeat(60)));
        assert_eq!(valid_workspace_name(&"x".repeat(61)), Err(WorkspaceValidationError::NameTooLong));
    }

    #[test]
    fn a_tab_names_exactly_one_item() {
        let open = |s: Option<&str>, r: Option<&str>| OpenTab {
            session_id: s.map(Into::into),
            run_id: r.map(Into::into),
            position: None,
        };
        assert_eq!(open(Some("s"), None).item(), Ok(TabItem::Session("s".into())));
        assert_eq!(open(None, Some("r")).item(), Ok(TabItem::Run("r".into())));
        assert_eq!(open(Some("s"), Some("r")).item(), Err(WorkspaceValidationError::TabItem));
        assert_eq!(open(None, None).item(), Err(WorkspaceValidationError::TabItem));
        assert_eq!(open(Some(" "), None).item(), Err(WorkspaceValidationError::TabItem));
    }
}
