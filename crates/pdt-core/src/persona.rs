//! Personas (Phase 7b spec §3): versioned agent setups that agent steps run. A persona has a name and a role; each
//! version records the agent, model, instructions, permission mode and the pdt-mcp tools it may use. Pure: pdt-store
//! keeps them, pdt-api installs the built-ins and runs them.

use crate::conversation::PermissionMode;
use crate::integration::ToolTable;
use crate::session::AgentKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The pdt-mcp tools a session gets when it runs no persona (spec §2.2). The secret tools (PDT-158) are here so any
/// agent can use the secrets the user lets it; each secret says which projects and personas may. The forge tools
/// (PDT-177) read the session's pull request, CI and issues and comment on them, open its pull request and ask for its
/// merge again after a merge queue drop (PDT-185), instead of a forge's CLI; `issue_create` (PDT-221) files an issue,
/// asking the user first.
pub const DEFAULT_TOOLS: [&str; 30] = [
    "get_ticket",
    "submit_outcome",
    "ask_user",
    "add_ticket_comment",
    "add_review_comment",
    "list_secrets",
    "request_secret",
    "run_with_secrets",
    "write_secret_file",
    "change_request_view",
    "change_request_comment",
    "change_request_open",
    "change_request_merge",
    "ci_runs",
    "ci_failed_logs",
    "issue_view",
    "issue_comment",
    "documents_list",
    "document_get",
    "doc_read",
    "doc_edit",
    "sheet_read",
    "sheet_edit",
    "document_create",
    "document_link",
    "comments_list",
    "comment_reply",
    "comment_resolve",
    "document_open_beside",
    "issue_create",
];

/// One pdt-mcp tool as the Personas page lists it (`GET /api/personas/tools`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaTool {
    pub name: String,
    /// What it lets the agent do, in a line.
    pub does: String,
    /// It asks the user before each call ([`ToolDecl::asks`](crate::integration::ToolDecl::asks)).
    pub asks: bool,
    /// In the default set ([`DEFAULT_TOOLS`]).
    pub default: bool,
}

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
    /// The tools the persona's sessions get, in `all`'s order.
    pub fn tool_set(&self, all: &ToolTable) -> Vec<String> {
        if self.tools.is_empty() {
            DEFAULT_TOOLS.iter().map(|t| t.to_string()).collect()
        } else {
            all.names().into_iter().filter(|t| self.tools.iter().any(|x| x == t)).map(String::from).collect()
        }
    }

    /// The setup with names trimmed, or why it can't be saved (`all`: the tools PDT has).
    pub fn validate(mut self, all: &ToolTable) -> Result<Self, String> {
        self.model = self.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        if self.instructions.len() > MAX_INSTRUCTIONS {
            return Err(format!("instructions are longer than {} KB", MAX_INSTRUCTIONS / 1024));
        }
        let mut tools: Vec<String> = vec![];
        for t in self.tools {
            let t = t.trim().to_string();
            if !all.contains(&t) {
                return Err(format!("{t} is not a PDT tool (tools: {})", all.names().join(", ")));
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

/// `GET /api/personas/{id}/template` (PDT-100): a built-in persona's current template and how the persona's current
/// version differs from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaTemplate {
    pub setup: PersonaSetup,
    /// The current version → the template.
    pub diff: PersonaDiff,
}

fn as_text(s: &PersonaSetup, all: &ToolTable) -> String {
    let mut t = format!(
        "agent: {}\nmodel: {}\npermission mode: {}\ntools: {}\n\n",
        serde_json::to_value(s.agent).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
        s.model.as_deref().unwrap_or("(default)"),
        s.permission_mode.map(|m| m.as_str()).unwrap_or("(project or host default)"),
        s.tool_set(all).join(", "),
    );
    t.push_str(&s.instructions);
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}

/// What changed from `a` to `b` (`all`: the tools PDT has).
pub fn diff(a: &PersonaSetup, b: &PersonaSetup, a_label: &str, b_label: &str, all: &ToolTable) -> PersonaDiff {
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
    if a.tool_set(all) != b.tool_set(all) {
        changed.push("tools".into());
    }
    if a.instructions != b.instructions {
        changed.push("instructions".into());
    }
    let (ta, tb) = (as_text(a, all), as_text(b, all));
    let unified = similar::TextDiff::from_lines(&ta, &tb).unified_diff().header(a_label, b_label).to_string();
    PersonaDiff { changed, unified }
}

/// A built-in persona brought up to a newer template (PDT-100). `base` is the template PDT applied last: each field
/// the user left as it was follows `template`; a field they changed stays theirs. Returns the setup and the fields
/// kept (named as in [`PersonaDiff::changed`]) whose template moved on. `all`: the tools PDT has.
pub fn follow_template(
    base: &PersonaSetup,
    current: &PersonaSetup,
    template: &PersonaSetup,
    all: &ToolTable,
) -> (PersonaSetup, Vec<&'static str>) {
    let mut kept = vec![];
    fn pick<T: Clone + PartialEq>(
        name: &'static str,
        base: &T,
        current: &T,
        template: &T,
        kept: &mut Vec<&'static str>,
    ) -> T {
        if current == base {
            return template.clone();
        }
        if template != base {
            kept.push(name);
        }
        current.clone()
    }
    let tools = if current.tool_set(all) == base.tool_set(all) {
        template.tools.clone()
    } else {
        if template.tool_set(all) != base.tool_set(all) {
            kept.push("tools");
        }
        current.tools.clone()
    };
    let setup = PersonaSetup {
        agent: pick("agent", &base.agent, &current.agent, &template.agent, &mut kept),
        model: pick("model", &base.model, &current.model, &template.model, &mut kept),
        permission_mode: pick(
            "permissionMode",
            &base.permission_mode,
            &current.permission_mode,
            &template.permission_mode,
            &mut kept,
        ),
        tools,
        instructions: pick(
            "instructions",
            &base.instructions,
            &current.instructions,
            &template.instructions,
            &mut kept,
        ),
    };
    (setup, kept)
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
    /// Its pdt-mcp tools.
    pub tools: BuiltinTools,
    pub instructions: &'static str,
}

/// A built-in persona's pdt-mcp tools.
pub enum BuiltinTools {
    /// These (empty: the default set).
    These(&'static [&'static str]),
    /// Every tool the PM has ([`ToolDecl::pm`](crate::integration::ToolDecl::pm)), from every integration.
    Pm,
}

impl BuiltinPersona {
    /// Its setup, with `all` the tools PDT has.
    pub fn setup(&self, all: &ToolTable) -> PersonaSetup {
        let tools = match self.tools {
            BuiltinTools::These(t) => t.to_vec(),
            BuiltinTools::Pm => all.pm_tools(),
        };
        PersonaSetup {
            agent: AgentKind::Claude,
            model: None,
            instructions: self.instructions.to_string(),
            permission_mode: self.permission_mode,
            tools: tools.into_iter().map(String::from).collect(),
        }
    }
}

/// The built-in personas; each one's instructions live in `persona/<id>.md`.
pub const BUILTIN_PERSONAS: &[BuiltinPersona] = &[
    BuiltinPersona {
        id: "refiner",
        name: "Refiner",
        role: "refiner",
        description: "Turns a draft ticket into one that is ready to build.",
        permission_mode: None,
        tools: BuiltinTools::These(&[
            "get_ticket",
            "submit_outcome",
            "ask_user",
            "add_ticket_comment",
            "update_ticket",
        ]),
        instructions: include_str!("persona/refiner.md"),
    },
    BuiltinPersona {
        id: "coder",
        name: "Coder",
        role: "coder",
        description: "Builds the ticket on its branch, runs the checks and commits.",
        permission_mode: None,
        tools: BuiltinTools::These(&[]),
        instructions: include_str!("persona/coder.md"),
    },
    BuiltinPersona {
        id: "reviewer",
        name: "Reviewer",
        role: "reviewer",
        description: "Reviews the attempt's changes against the ticket.",
        permission_mode: None,
        tools: BuiltinTools::These(&[]),
        instructions: include_str!("persona/reviewer.md"),
    },
    BuiltinPersona {
        id: "qa",
        name: "QA",
        role: "qa",
        description: "Runs the project's tests and tries the change.",
        permission_mode: None,
        tools: BuiltinTools::These(&[]),
        instructions: include_str!("persona/qa.md"),
    },
    BuiltinPersona {
        id: "triage",
        name: "Triage",
        role: TRIAGE_ROLE,
        description: "Reads issues from outside PDT and drafts tickets for them, locked down.",
        permission_mode: Some(PermissionMode::Ask),
        tools: BuiltinTools::These(TRIAGE_TOOLS),
        instructions: include_str!("persona/triage.md"),
    },
    BuiltinPersona {
        id: "pr-reviewer",
        name: "PR Reviewer",
        role: TRIAGE_ROLE,
        description: "Reviews pull requests from outside PDT through the forge's API, locked down.",
        permission_mode: Some(PermissionMode::Ask),
        tools: BuiltinTools::These(PR_REVIEWER_TOOLS),
        instructions: include_str!("persona/pr-reviewer.md"),
    },
    BuiltinPersona {
        id: "pm",
        name: "PM",
        role: "pm",
        description: "The project manager you talk to.",
        permission_mode: Some(PermissionMode::Ask),
        tools: BuiltinTools::Pm,
        instructions: PM_INSTRUCTIONS,
    },
];

/// The role of the locked-down personas that read untrusted input (PDT-201): Triage and the PR Reviewer.
pub const TRIAGE_ROLE: &str = "triage";

/// The Triage persona's tools: read tickets, draft one, ask, finish. No shell, secrets, `edit_ticket` or
/// `send_to_session`.
pub const TRIAGE_TOOLS: &[&str] = &["search_tickets", "get_ticket", "create_ticket", "ask_user", "submit_outcome"];

/// The PR Reviewer persona's tools (PDT-269): read its ticket and the pull request's diff through the forge, finish.
/// No shell, secrets, ticket edits, comments or `ask_user`.
pub const PR_REVIEWER_TOOLS: &[&str] = &["get_ticket", "change_request_diff", "submit_outcome"];

/// Whether a persona's sessions are locked down (PDT-201): a read-only sandbox (Claude's read tools only, Codex
/// read-only, no web) and no tools beyond its own.
pub fn locked_down(role: &str) -> bool {
    role.trim().eq_ignore_ascii_case(TRIAGE_ROLE)
}

/// The PM persona's instructions (PDT-81). PDT's own PM context and the briefing come before them.
pub const PM_INSTRUCTIONS: &str = include_str!("persona/pm.md");

pub fn builtin_persona(id: &str) -> Option<&'static BuiltinPersona> {
    BUILTIN_PERSONAS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration::ToolDecl;

    /// Some of PDT's tools; pdt-api's registry tests check the real table.
    fn table() -> ToolTable {
        ToolTable::new(DEFAULT_TOOLS.iter().map(|&t| ToolDecl::new(t, "does").pm()).chain([
            ToolDecl::new("update_ticket", "Rewrite its own ticket"),
            ToolDecl::new("catalog_get", "Read a catalog node").read_only().pm(),
            ToolDecl::new("create_ticket", "Create tickets").asks().pm(),
            ToolDecl::new("search_tickets", "Search tickets").read_only().pm(),
            ToolDecl::new("change_request_diff", "Read a pull request's diff").read_only(),
        ]))
    }

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
        let t = table();
        let s = setup().validate(&t).unwrap();
        assert_eq!(s.model.as_deref(), Some("opus"));
        assert_eq!(s.tool_set(&t), DEFAULT_TOOLS.to_vec());
        let s = PersonaSetup { tools: vec!["catalog_get".into(), "get_ticket".into(), "get_ticket".into()], ..setup() };
        let s = s.validate(&t).unwrap();
        assert_eq!(s.tools, ["catalog_get", "get_ticket"]);
        assert_eq!(s.tool_set(&t), ["get_ticket", "catalog_get"], "in the catalog's order");
        let bad = PersonaSetup { tools: vec!["rm_rf".into()], ..setup() };
        assert!(bad.validate(&t).unwrap_err().contains("rm_rf is not a PDT tool"));
        let long = PersonaSetup { instructions: "x".repeat(MAX_INSTRUCTIONS + 1), ..setup() };
        assert!(long.validate(&t).is_err());
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
        let d = diff(&a, &b, "v1", "v2", &table());
        assert_eq!(d.changed, ["permissionMode", "instructions"]);
        assert!(d.unified.starts_with("--- v1\n+++ v2\n"), "{}", d.unified);
        assert!(d.unified.contains("+Do it well."));
        assert!(diff(&a, &a, "a", "b", &table()).changed.is_empty());
    }

    #[test]
    fn the_project_overrides_the_persona_which_overrides_the_host() {
        use PermissionMode::*;
        assert_eq!(effective_permission_mode(Some(Ask), Some(Bypass), Plan), Ask);
        assert_eq!(effective_permission_mode(None, Some(Bypass), Plan), Bypass);
        assert_eq!(effective_permission_mode(None, None, Plan), Plan);
    }

    #[test]
    fn the_builtins_are_the_pipeline_roles_and_the_pm() {
        let names: Vec<_> = BUILTIN_PERSONAS.iter().map(|p| p.name).collect();
        assert_eq!(names, ["Refiner", "Coder", "Reviewer", "QA", "Triage", "PR Reviewer", "PM"]);
        let t = table();
        for p in BUILTIN_PERSONAS {
            assert!(p.setup(&t).validate(&t).is_ok(), "{}", p.id);
            assert!(p.instructions.contains("submit_outcome"), "{}", p.id);
        }
        assert!(builtin_persona("qa").is_some() && builtin_persona("nobody").is_none());
    }

    #[test]
    fn triage_is_locked_down_to_reading_and_drafting() {
        let t = table();
        let triage = builtin_persona("triage").unwrap();
        assert!(locked_down(triage.role) && !locked_down("pm") && locked_down(" Triage "));
        let tools = triage.setup(&t).tool_set(&t);
        assert_eq!(tools.len(), TRIAGE_TOOLS.len());
        for no in ["run_with_secrets", "request_secret", "write_secret_file", "edit_ticket", "send_to_session"] {
            assert!(!tools.iter().any(|x| x == no), "{no}");
        }
    }

    /// PDT-269: the PR Reviewer reads the diff through the forge and submits an outcome, nothing more.
    #[test]
    fn the_pr_reviewer_is_locked_down_to_reading_the_diff() {
        let t = table();
        let r = builtin_persona("pr-reviewer").unwrap();
        assert!(locked_down(r.role));
        let tools = r.setup(&t).tool_set(&t);
        assert_eq!(tools, ["get_ticket", "submit_outcome", "change_request_diff"]);
        for no in [
            "run_with_secrets",
            "request_secret",
            "write_secret_file",
            "list_secrets",
            "update_ticket",
            "add_ticket_comment",
            "change_request_comment",
            "ask_user",
        ] {
            assert!(!tools.iter().any(|x| x == no), "{no}");
        }
    }

    #[test]
    fn the_pm_gets_every_pm_tool() {
        let t = table();
        let pm = builtin_persona("pm").unwrap().setup(&t);
        assert_eq!(pm.tool_set(&t), t.pm_tools());
        assert_eq!(pm.permission_mode, Some(PermissionMode::Ask));
    }

    #[test]
    fn an_edited_built_in_follows_its_template_where_the_user_left_it() {
        let t = table();
        let old = builtin_persona("pm").unwrap().setup(&t);
        let new = PersonaSetup { instructions: "New PM text.".into(), ..old.clone() };
        // Untouched: the new template.
        assert_eq!(follow_template(&old, &old, &new, &t), (new.clone(), vec![]));
        // Only the model changed: it stays, the instructions follow.
        let mine = PersonaSetup { model: Some("opus".into()), ..old.clone() };
        let (s, kept) = follow_template(&old, &mine, &new, &t);
        assert_eq!(s, PersonaSetup { model: Some("opus".into()), ..new.clone() });
        assert!(kept.is_empty(), "the template's model did not move: {kept:?}");
        // Edited instructions stay the user's, and are named.
        let mine = PersonaSetup { instructions: "Mine.".into(), ..old.clone() };
        assert_eq!(follow_template(&old, &mine, &new, &t), (mine.clone(), vec!["instructions"]));
        // Tools compare as sets the persona gets (empty is the default set).
        let base = PersonaSetup { tools: vec![], ..old.clone() };
        let same = PersonaSetup { tools: DEFAULT_TOOLS.map(String::from).to_vec(), ..old.clone() };
        let more = PersonaSetup { tools: t.names().into_iter().map(String::from).collect(), ..old.clone() };
        assert_eq!(follow_template(&base, &same, &more, &t).0.tools, more.tools);
    }
}
