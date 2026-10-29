//! Forges (PDT-176): where a project's change requests, CI, reviews and issues live. GitHub is one; the trait the
//! SDLC talks to is `pdt_forge::Forge`. This module holds what pure code needs: which forge a project names, and what
//! a forge tells agents.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use ts_rs::TS;

/// The forge a project names in `.pdt/project.toml` (`forge = "github"`): a forge plugin's id (PDT-241), looked up in
/// the forge registry (`pdt_forge::ForgeRegistry`), or one of PDT's own, [`ForgeKind::LOCAL`] and [`ForgeKind::NONE`].
/// Without one, the project's forge is the one that claims its `origin`'s host, if any. Ids are lower-case letters,
/// digits and `-`; serialised as the bare id (`"github"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(export, type = "string")]
pub struct ForgeKind(Cow<'static, str>);

impl ForgeKind {
    /// GitHub (github.com and GitHub Enterprise Server): the `pdt-github` plugin.
    pub const GITHUB: ForgeKind = ForgeKind(Cow::Borrowed("github"));
    /// PDT itself (PDT-178), for a repository with no remote: a change request is the attempt's branch compared with
    /// its base, CI is the `[ci]` command run in the attempt's worktree, and merging is a local merge into the base.
    pub const LOCAL: ForgeKind = ForgeKind(Cow::Borrowed("local"));
    /// No forge: no change requests, CI or issues are read; merges are local.
    pub const NONE: ForgeKind = ForgeKind(Cow::Borrowed("none"));

    /// A forge plugin's id; None when it is not one (empty, or other than lower-case letters, digits and `-`).
    pub fn new(id: &str) -> Option<ForgeKind> {
        let ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        ok.then(|| ForgeKind(Cow::Owned(id.to_string())))
    }

    /// The id of a forge plugin (`github`), as `const` (for a plugin's own id).
    pub const fn from_static(id: &'static str) -> ForgeKind {
        ForgeKind(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// One of PDT's own, which no plugin provides.
    pub fn is_builtin(&self) -> bool {
        *self == Self::LOCAL || *self == Self::NONE
    }
}

impl std::fmt::Display for ForgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ForgeKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ForgeKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ForgeKind::new(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("{s:?} is not a forge id (lower-case letters, digits and -)"))
        })
    }
}

impl PartialEq<str> for ForgeKind {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

/// What a forge adds to the instructions of an agent session on ticket work (`agent_context`): how to read CI and
/// answer on a change request with its tools. Each part is left out when None.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeNotes {
    /// One line on reading CI for the session's branch; `{branch}` stands for it.
    pub ci: Option<String>,
    /// How to answer a comment on a change request, e.g. "`change_request_comment`, with `replyTo` to answer a review comment".
    pub reply: Option<String>,
    /// The command that watches CI in a shell, named as one not to run.
    pub watch: Option<String>,
}

/// Where a project's repository is on its forge's website, for links in the UI (the forge plugin builds them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeLinks {
    pub kind: ForgeKind,
    /// The forge's name for people (`GitHub`).
    pub name: String,
    /// The forge's name for the repository (GitHub's `owner/name`).
    pub repo: String,
    /// The repository's page.
    pub repo_url: String,
    /// A branch's page; `{branch}` stands for the branch (its `/` kept, the rest URL-encoded).
    pub branch_url: Option<String>,
    /// The page that opens a change request from `{branch}` into `{base}` (or shows the open one).
    pub compare_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_ids_keep_their_names() {
        for (text, want) in
            [("github", Some(ForgeKind::GITHUB)), ("local", Some(ForgeKind::LOCAL)), ("none", Some(ForgeKind::NONE))]
        {
            let k: Option<ForgeKind> = serde_json::from_str(&format!("{text:?}")).ok();
            assert_eq!(k, want, "{text}");
            assert_eq!(serde_json::to_string(&k.unwrap()).unwrap(), format!("{text:?}"));
        }
        let gitlab: ForgeKind = serde_json::from_str("\"gitlab\"").unwrap();
        assert_eq!(gitlab.as_str(), "gitlab");
        assert!(!gitlab.is_builtin() && ForgeKind::LOCAL.is_builtin());
        for bad in ["\"GitHub\"", "\"\"", "\"git hub\"", "3"] {
            assert!(serde_json::from_str::<ForgeKind>(bad).is_err(), "{bad}");
        }
    }
}
