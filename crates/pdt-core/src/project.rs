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
    /// Short unique key (e.g. `PDT`); tickets will be numbered with it. Blank on insert means "suggest one".
    #[serde(default)]
    pub key: String,
    /// Where the repository is on the forge that claims `origin` (its website, for links). Filled in by the API from
    /// git and the forge plugins in use, never stored.
    #[serde(default)]
    pub forge: Option<crate::forge::ForgeLinks>,
    /// `personal`: no repository; `repo_path` is a plain folder and tickets run in scratch folders (Phase 7b §7).
    #[serde(default)]
    pub kind: ProjectKind,
}

impl Project {
    pub fn is_personal(&self) -> bool {
        self.kind == ProjectKind::Personal
    }
    /// A git repository: not a Personal project nor the host project.
    pub fn has_repo(&self) -> bool {
        self.kind == ProjectKind::Repo
    }
    pub fn is_host(&self) -> bool {
        self.kind == ProjectKind::Host
    }
}

/// The host project's key: no user key can have `_` (PDT-81).
pub const HOST_PROJECT_KEY: &str = "_HOST";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProjectKind {
    /// A git repository (the usual project).
    #[default]
    Repo,
    /// No repository: agent tasks such as briefs and triage.
    Personal,
    /// PDT's own project for host-level sessions (the PM's, PDT-81): one per host, made by the store, no repository,
    /// never listed.
    Host,
}

impl ProjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Repo => "repo",
            ProjectKind::Personal => "personal",
            ProjectKind::Host => "host",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [ProjectKind::Repo, ProjectKind::Personal, ProjectKind::Host].into_iter().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewProject {
    pub name: String,
    /// Not for a Personal project (PDT makes its folder).
    #[serde(default)]
    pub repo_path: String,
    #[serde(default)]
    #[ts(optional)]
    pub base_branch: Option<String>,
    /// Suggested from the name when absent.
    #[serde(default)]
    #[ts(optional)]
    pub key: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub kind: Option<ProjectKind>,
}

/// Renames a project, changes its key or its base branch.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectPatch {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub key: Option<String>,
    /// The branch new worktrees and tickets start from; a local branch or one on `origin`.
    #[serde(default)]
    #[ts(optional)]
    pub base_branch: Option<String>,
}

/// Clones a repository and adds it as a project.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloneProject {
    pub url: String,
    /// The folder the clone goes in (as `<parent_dir>/<repo name>`); the host's Projects folder when absent.
    #[serde(default)]
    #[ts(optional)]
    pub parent_dir: Option<String>,
    /// Defaults to the repository's name.
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub key: Option<String>,
}

/// Brings a local branch of the main checkout up to date with `origin` (PDT-56): `POST /api/projects/{id}/pull`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullBranch {
    /// The project's base branch when absent.
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
}

/// What a pull did. Only `updated` moved anything; the rest left the branch as it was.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PullOutcome {
    Updated,
    UpToDate,
    /// No `origin` remote.
    NoRemote,
    /// No local branch of that name.
    NoBranch,
    /// The fetch failed (see `message`).
    FetchFailed,
    /// Local commits `origin` lacks and the other way round: never merged.
    Diverged,
    /// Checked out with uncommitted changes in the way (or a merge in progress); see `message`.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullResult {
    pub branch: String,
    pub outcome: PullOutcome,
    /// Git's own words for `fetch_failed` / `blocked`.
    pub message: Option<String>,
}

/// A folder the user typed: `~` or `~/...` expands to `home`; anything else must already be absolute. Trimmed.
pub fn resolve_folder(input: &str, home: Option<&std::path::Path>) -> Result<std::path::PathBuf, FolderError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(FolderError::Empty);
    }
    let rest = if s == "~" { Some("") } else { s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) };
    let path = match rest {
        Some(r) => home.ok_or(FolderError::NoHome)?.join(r),
        None => std::path::PathBuf::from(s),
    };
    if !path.is_absolute() {
        return Err(FolderError::NotAbsolute(s.to_string()));
    }
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FolderError {
    #[error("enter a folder")]
    Empty,
    #[error("the home folder is not known; enter the full path")]
    NoHome,
    #[error("{0:?} is not a full path; start it with / (or ~ for your home folder)")]
    NotAbsolute(String),
}

/// Longest project key.
pub const MAX_KEY: usize = 10;

/// A key is 2–10 uppercase ASCII letters and digits, starting with a letter. Input is trimmed and uppercased.
pub fn valid_key(key: &str) -> Result<String, ValidationError> {
    let k = key.trim().to_ascii_uppercase();
    let ok = (2..=MAX_KEY).contains(&k.len())
        && k.starts_with(|c: char| c.is_ascii_uppercase())
        && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if ok { Ok(k) } else { Err(ValidationError::BadKey(key.trim().to_string())) }
}

/// A key for `name` not in `taken`: the initials of a multi-word name, else its first letters (up to 4), padded
/// to two letters with `X`, then a number if needed (`PDT`, `PDT2`, ...).
pub fn suggest_key(name: &str, taken: &[String]) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase())
        .collect();
    let mut base: String = if words.len() > 1 {
        words.iter().filter_map(|w| w.chars().next()).collect()
    } else {
        words.first().map(|w| w.chars().take(4).collect()).unwrap_or_default()
    };
    base = base.trim_start_matches(|c: char| c.is_ascii_digit()).chars().take(MAX_KEY - 2).collect();
    if base.is_empty() {
        base.push('P');
    }
    while base.len() < 2 {
        base.push('X');
    }
    let free = |k: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(k));
    if free(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}{n}")).find(|k| free(k)).expect("some numbered key is free")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidProject {
    pub name: String,
    /// Empty for a Personal project.
    pub repo_path: String,
    pub base_branch: Option<String>,
    pub key: Option<String>,
    pub kind: ProjectKind,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    #[error("project name must not be empty")]
    EmptyName,
    #[error("repository path must not be empty")]
    EmptyPath,
    #[error("base branch must not be blank")]
    EmptyBranch,
    #[error("a project key is 2–10 letters and digits starting with a letter (got {0:?})")]
    BadKey(String),
    #[error("PDT makes its own host project; add a repository or a Personal project")]
    HostKind,
}

impl NewProject {
    pub fn validate(&self) -> Result<ValidProject, ValidationError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        let kind = self.kind.unwrap_or_default();
        if kind == ProjectKind::Host {
            return Err(ValidationError::HostKind);
        }
        let key = self.key.as_deref().filter(|k| !k.trim().is_empty()).map(valid_key).transpose()?;
        if kind == ProjectKind::Personal {
            return Ok(ValidProject { name: name.into(), repo_path: String::new(), base_branch: None, key, kind });
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
        Ok(ValidProject { name: name.into(), repo_path: repo_path.into(), base_branch, key, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, path: &str, branch: Option<&str>) -> NewProject {
        NewProject {
            name: name.into(),
            repo_path: path.into(),
            base_branch: branch.map(Into::into),
            key: None,
            kind: None,
        }
    }

    #[test]
    fn a_personal_project_needs_no_repo() {
        let mut p = input(" Briefs ", "", None);
        p.kind = Some(ProjectKind::Personal);
        let v = p.validate().unwrap();
        assert_eq!((v.name.as_str(), v.repo_path.as_str(), v.kind), ("Briefs", "", ProjectKind::Personal));
        assert_eq!(input("x", "", None).validate(), Err(ValidationError::EmptyPath));
        assert_eq!(ProjectKind::parse("personal"), Some(ProjectKind::Personal));
    }

    #[test]
    fn trims_fields() {
        let v = input("  PDT ", " C:\\repo ", Some(" main ")).validate().unwrap();
        assert_eq!(
            v,
            ValidProject {
                name: "PDT".into(),
                repo_path: "C:\\repo".into(),
                base_branch: Some("main".into()),
                key: None,
                kind: ProjectKind::Repo,
            }
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

    #[test]
    fn keys_are_validated_and_uppercased() {
        assert_eq!(valid_key(" pdt "), Ok("PDT".into()));
        assert_eq!(valid_key("a1"), Ok("A1".into()));
        for bad in ["P", "1AB", "AB-C", "ABCDEFGHIJK", "", "ÉT"] {
            assert!(matches!(valid_key(bad), Err(ValidationError::BadKey(_))), "{bad}");
        }
        let mut p = input("x", "/r", None);
        p.key = Some("bad key".into());
        assert!(matches!(p.validate(), Err(ValidationError::BadKey(_))));
        p.key = Some(" ".into());
        assert_eq!(p.validate().unwrap().key, None);
    }

    #[test]
    fn keys_are_suggested_from_the_name_and_kept_unique() {
        let none: Vec<String> = vec![];
        assert_eq!(suggest_key("Personal Dev Team", &none), "PDT");
        assert_eq!(suggest_key("pdt", &none), "PDT");
        assert_eq!(suggest_key("website", &none), "WEBS");
        assert_eq!(suggest_key("my-cool_app", &none), "MCA");
        assert_eq!(suggest_key("x", &none), "XX");
        assert_eq!(suggest_key("42", &none), "PX");
        assert_eq!(suggest_key("", &none), "PX");
        assert_eq!(suggest_key("PDT configuration", &["PC".into()]), "PC2");
        assert_eq!(suggest_key("Personal Dev Team", &["pdt".into(), "PDT2".into()]), "PDT3");
        assert!(valid_key(&suggest_key("a b c d e f g h i j k l", &none)).is_ok());
    }

    #[test]
    fn folders_expand_home_and_must_be_absolute() {
        let home = std::env::temp_dir();
        assert_eq!(resolve_folder(" ~/work ", Some(&home)), Ok(home.join("work")));
        assert_eq!(resolve_folder("~", Some(&home)), Ok(home.join("")));
        let abs = home.join("code");
        assert_eq!(resolve_folder(&abs.to_string_lossy(), None), Ok(abs.clone()));
        assert_eq!(resolve_folder("work/code", Some(&home)), Err(FolderError::NotAbsolute("work/code".into())));
        assert_eq!(resolve_folder("~/x", None), Err(FolderError::NoHome));
        assert_eq!(resolve_folder("  ", Some(&home)), Err(FolderError::Empty));
    }
}
