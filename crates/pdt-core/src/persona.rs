//! Personas (Phase 7b spec §3): versioned agent setups that agent steps run. A persona has a name and a role; each
//! version records the agent, model, instructions, permission mode and the pdt-mcp tools it may use. Pure: pdt-store
//! keeps them, pdt-api installs the built-ins and runs them.

use crate::conversation::PermissionMode;
use crate::session::AgentKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The pdt-mcp tools a session gets when it runs no persona (spec §2.2).
pub const DEFAULT_TOOLS: [&str; 5] =
    ["get_ticket", "submit_outcome", "ask_user", "add_ticket_comment", "add_review_comment"];

/// Every pdt-mcp tool, in the order `tools/list` gives them.
pub const ALL_TOOLS: [&str; 12] = [
    "get_ticket",
    "submit_outcome",
    "ask_user",
    "add_ticket_comment",
    "add_review_comment",
    "update_ticket",
    "catalog_search",
    "catalog_get",
    "workflow_get",
    "workflow_validate",
    "workflow_dry_run",
    "workflow_save",
];

const MAX_NAME: usize = 60;
const MAX_INSTRUCTIONS: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PersonaAuthor {
    User,
    Builtin,
    Agent,
}
impl PersonaAuthor {
    pub fn as_str(self) -> &'static str {
        match self {
            PersonaAuthor::User => "user",
            PersonaAuthor::Builtin => "builtin",
            PersonaAuthor::Agent => "agent",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [PersonaAuthor::User, PersonaAuthor::Builtin, PersonaAuthor::Agent].into_iter().find(|a| a.as_str() == s)
    }
}

/// What one version of a persona runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaSetup {
    pub agent: AgentKind,
    /// None: the agent's default model.
    #[serde(default)]
    pub model: Option<String>,
    /// Markdown, added to the session's instructions.
    #[serde(default)]
    pub instructions: String,
    /// None: the project's override, else the host default (spec §3).
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// pdt-mcp tools; empty means the default set.
    #[serde(default)]
    pub tools: Vec<String>,
}

impl PersonaSetup {
    /// The tools the persona's sessions get.
    pub fn tool_set(&self) -> Vec<String> {
        if self.tools.is_empty() {
            DEFAULT_TOOLS.iter().map(|t| t.to_string()).collect()
        } else {
            ALL_TOOLS.iter().filter(|t| self.tools.iter().any(|x| x == *t)).map(|t| t.to_string()).collect()
        }
    }

    /// The setup with names trimmed, or why it can't be saved.
    pub fn validate(mut self) -> Result<Self, String> {
        self.model = self.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        if self.instructions.len() > MAX_INSTRUCTIONS {
            return Err(format!("instructions are longer than {} KB", MAX_INSTRUCTIONS / 1024));
        }
        let mut tools: Vec<String> = vec![];
        for t in self.tools {
            let t = t.trim().to_string();
            if !ALL_TOOLS.contains(&t.as_str()) {
                return Err(format!("{t} is not a PDT tool (tools: {})", ALL_TOOLS.join(", ")));
            }
            if !tools.contains(&t) {
                tools.push(t);
            }
        }
        self.tools = tools;
        Ok(self)
    }
}

/// A persona with its current version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub role: String,
    pub description: String,
    /// The built-in template it came from.
    pub builtin: Option<String>,
    pub archived: bool,
    /// The current version.
    pub version: u32,
    #[serde(flatten)]
    #[ts(flatten)]
    pub setup: PersonaSetup,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// One saved version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaVersion {
    pub persona_id: String,
    pub version: u32,
    #[serde(flatten)]
    #[ts(flatten)]
    pub setup: PersonaSetup,
    pub author: PersonaAuthor,
    pub note: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// `POST /api/personas`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewPersona {
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub setup: PersonaSetup,
}

/// `PUT /api/personas/{id}`: a new version.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavePersona {
    #[serde(flatten)]
    #[ts(flatten)]
    pub setup: PersonaSetup,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /api/personas/{id}`.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaPatch {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub role: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub archived: Option<bool>,
}

/// A persona name, trimmed, or why not.
pub fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("a persona needs a name".into());
    }
    if n.chars().count() > MAX_NAME {
        return Err(format!("a persona name is at most {MAX_NAME} characters"));
    }
    Ok(n.to_string())
}

/// `GET /api/personas/{id}/diff`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaDiff {
    /// `agent`, `model`, `permissionMode`, `tools`, `instructions` — those that differ.
    pub changed: Vec<String>,
    /// `diff -u` of the two versions as text (settings, then the instructions).
    pub unified: String,
}

fn as_text(s: &PersonaSetup) -> String {
    let mut t = format!(
        "agent: {}\nmodel: {}\npermission mode: {}\ntools: {}\n\n",
        serde_json::to_value(s.agent).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
        s.model.as_deref().unwrap_or("(default)"),
        s.permission_mode.map(|m| m.as_str()).unwrap_or("(project or host default)"),
        s.tool_set().join(", "),
    );
    t.push_str(&s.instructions);
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}

pub fn diff(a: &PersonaSetup, b: &PersonaSetup, a_label: &str, b_label: &str) -> PersonaDiff {
    let mut changed = vec![];
    if a.agent != b.agent {
        changed.push("agent".to_string());
    }
    if a.model != b.model {
        changed.push("model".into());
    }
    if a.permission_mode != b.permission_mode {
        changed.push("permissionMode".into());
    }
    if a.tool_set() != b.tool_set() {
        changed.push("tools".into());
    }
    if a.instructions != b.instructions {
        changed.push("instructions".into());
    }
    let (ta, tb) = (as_text(a), as_text(b));
    let unified = similar::TextDiff::from_lines(&ta, &tb).unified_diff().header(a_label, b_label).to_string();
    PersonaDiff { changed, unified }
}

/// The permission mode a persona's session runs with (spec §3): the project's override, else the persona's, else
/// the host default.
pub fn effective_permission_mode(
    project: Option<PermissionMode>,
    persona: Option<PermissionMode>,
    host_default: PermissionMode,
) -> PermissionMode {
    project.or(persona).unwrap_or(host_default)
}

/// A built-in persona: installed on start, kept up to date while the user hasn't edited it.
pub struct BuiltinPersona {
    pub id: &'static str,
    pub name: &'static str,
    pub role: &'static str,
    pub description: &'static str,
    pub permission_mode: Option<PermissionMode>,
    /// Its pdt-mcp tools; empty: the default set.
    pub tools: &'static [&'static str],
    pub instructions: &'static str,
}

impl BuiltinPersona {
    pub fn setup(&self) -> PersonaSetup {
        PersonaSetup {
            agent: AgentKind::Claude,
            model: None,
            instructions: self.instructions.to_string(),
            permission_mode: self.permission_mode,
            tools: self.tools.iter().map(|t| t.to_string()).collect(),
        }
    }
}

pub const BUILTIN_PERSONAS: &[BuiltinPersona] = &[
    BuiltinPersona {
        id: "refiner",
        name: "Refiner",
        role: "refiner",
        description: "Turns a draft ticket into one that is ready to build.",
        permission_mode: None,
        tools: &["get_ticket", "submit_outcome", "ask_user", "add_ticket_comment", "update_ticket"],
        instructions: "You are the Refiner. You make a ticket ready for a coding agent to build without guessing.\n\n\
- Read the ticket with `get_ticket` (its comments and earlier outcomes too) and look at the code it touches.\n\
- Change nothing in the code. Improve the ticket: a clear scope, what is out of scope, and a \"Done when\" list \
someone can check. Rewrite its description with `update_ticket` (keep what the user wrote that still holds), and \
say what you changed and why in a ticket comment with `add_ticket_comment`.\n\
- When something only the user can decide is unclear, ask it with `ask_user` (short, concrete questions, with \
choices where they fit) and end your turn.\n\
- Finish with `submit_outcome`: `ready` when the ticket can be built as written, `needs_clarification` when it \
can't until the user decides something you could not ask.\n",
    },
    BuiltinPersona {
        id: "coder",
        name: "Coder",
        role: "coder",
        description: "Builds the ticket on its branch, runs the checks and commits.",
        permission_mode: None,
        tools: &[],
        instructions: "You are the Coder. You build the ticket on its branch.\n\n\
- Read the ticket with `get_ticket`, including review and QA outcomes from earlier passes: address every point.\n\
- Work test-first where the project does, follow the repository's instructions, run its checks, and commit.\n\
- Don't merge, push to the base branch, or move the ticket: PDT and the user do that.\n\
- Finish with `submit_outcome` `submitted` and a summary of what changed and how you checked it. Use `ask_user` \
only when you are blocked on a decision that is the user's.\n",
    },
    BuiltinPersona {
        id: "reviewer",
        name: "Reviewer",
        role: "reviewer",
        description: "Reviews the attempt's changes against the ticket.",
        permission_mode: None,
        tools: &[],
        instructions: "You are the Reviewer. You review the attempt's changes with fresh eyes; you change nothing.\n\n\
- Read the ticket with `get_ticket` and the branch's diff against its base (`git diff <base>...HEAD`).\n\
- Look for bugs, missing tests, behaviour the ticket asks for that is missing, and changes it didn't ask for.\n\
- Put each finding on its line: `comments` in `submit_outcome` (file, line, body), or `add_review_comment`.\n\
- Finish with `submit_outcome`: `approved` when it can merge as is, `changes_requested` with a summary of what must \
change.\n",
    },
    BuiltinPersona {
        id: "qa",
        name: "QA",
        role: "qa",
        description: "Runs the project's tests and tries the change.",
        permission_mode: None,
        tools: &[],
        instructions: "You are QA. You check that the attempt's change works.\n\n\
- Read the ticket with `get_ticket` and its \"Done when\".\n\
- Run the project's tests and, where it has them, its runs (PDT's guide `GET /api/guides/run-config` explains \
them); try each point of \"Done when\".\n\
- Don't fix anything yourself.\n\
- Finish with `submit_outcome`: `pass`, or `fail` with what failed and how to reproduce it.\n",
    },
];

pub fn builtin_persona(id: &str) -> Option<&'static BuiltinPersona> {
    BUILTIN_PERSONAS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> PersonaSetup {
        PersonaSetup {
            agent: AgentKind::Claude,
            model: Some(" opus ".into()),
            instructions: "Do it.\n".into(),
            permission_mode: None,
            tools: vec![],
        }
    }

    #[test]
    fn empty_tools_mean_the_default_set_and_unknown_tools_are_refused() {
        let s = setup().validate().unwrap();
        assert_eq!(s.model.as_deref(), Some("opus"));
        assert_eq!(s.tool_set(), DEFAULT_TOOLS.to_vec());
        let s = PersonaSetup { tools: vec!["catalog_get".into(), "get_ticket".into(), "get_ticket".into()], ..setup() };
        let s = s.validate().unwrap();
        assert_eq!(s.tools, ["catalog_get", "get_ticket"]);
        assert_eq!(s.tool_set(), ["get_ticket", "catalog_get"], "in the catalog's order");
        let bad = PersonaSetup { tools: vec!["rm_rf".into()], ..setup() };
        assert!(bad.validate().unwrap_err().contains("rm_rf is not a PDT tool"));
        let long = PersonaSetup { instructions: "x".repeat(MAX_INSTRUCTIONS + 1), ..setup() };
        assert!(long.validate().is_err());
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(valid_name("  Reviewer ").unwrap(), "Reviewer");
        assert!(valid_name("   ").is_err());
        assert!(valid_name(&"x".repeat(61)).is_err());
    }

    #[test]
    fn a_diff_names_the_fields_and_shows_the_text() {
        let a = setup();
        let b = PersonaSetup {
            instructions: "Do it well.\n".into(),
            permission_mode: Some(PermissionMode::Plan),
            ..a.clone()
        };
        let d = diff(&a, &b, "v1", "v2");
        assert_eq!(d.changed, ["permissionMode", "instructions"]);
        assert!(d.unified.starts_with("--- v1\n+++ v2\n"), "{}", d.unified);
        assert!(d.unified.contains("+Do it well."));
        assert!(diff(&a, &a, "a", "b").changed.is_empty());
    }

    #[test]
    fn the_project_overrides_the_persona_which_overrides_the_host() {
        use PermissionMode::*;
        assert_eq!(effective_permission_mode(Some(Ask), Some(Bypass), Plan), Ask);
        assert_eq!(effective_permission_mode(None, Some(Bypass), Plan), Bypass);
        assert_eq!(effective_permission_mode(None, None, Plan), Plan);
    }

    #[test]
    fn the_builtins_are_the_four_pipeline_roles() {
        let names: Vec<_> = BUILTIN_PERSONAS.iter().map(|p| p.name).collect();
        assert_eq!(names, ["Refiner", "Coder", "Reviewer", "QA"]);
        for p in BUILTIN_PERSONAS {
            assert!(p.setup().validate().is_ok(), "{}", p.id);
            assert!(p.instructions.contains("submit_outcome"), "{}", p.id);
        }
        assert!(builtin_persona("qa").is_some() && builtin_persona("pm").is_none());
    }
}
