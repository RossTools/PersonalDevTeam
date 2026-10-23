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
pub const ALL_TOOLS: [&str; 28] = [
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
    "get_overview",
    "search_tickets",
    "get_session",
    "list_workflow_runs",
    "search_notes",
    "save_note",
    "delete_note",
    "write_journal",
    "create_ticket",
    "edit_ticket",
    "link_tickets",
    "unlink_tickets",
    "send_to_session",
    "discord_get",
    "discord_change",
    "discord_webhook",
];

/// Tools that act for the user (PDT-81): the agent asks before each call (Claude's permission prompt, PDT's approval
/// card) unless the project's policy (for Discord, the bot connection) lets the PM act, so they are never in Claude's
/// `--allowedTools`.
pub const ASKING_TOOLS: [&str; 7] = [
    "create_ticket",
    "edit_ticket",
    "link_tickets",
    "unlink_tickets",
    "send_to_session",
    "discord_change",
    "discord_webhook",
];

/// The PM persona's tools (PDT-81 spec §4).
pub const PM_TOOLS: [&str; 26] = [
    "get_ticket",
    "submit_outcome",
    "ask_user",
    "add_ticket_comment",
    "catalog_search",
    "catalog_get",
    "workflow_get",
    "workflow_validate",
    "workflow_dry_run",
    "workflow_save",
    "get_overview",
    "search_tickets",
    "get_session",
    "list_workflow_runs",
    "search_notes",
    "save_note",
    "delete_note",
    "write_journal",
    "create_ticket",
    "edit_ticket",
    "link_tickets",
    "unlink_tickets",
    "send_to_session",
    "discord_get",
    "discord_change",
    "discord_webhook",
];

/// What each of [`ALL_TOOLS`] lets the agent do, in the same order: the Personas page's tool list (PDT-110).
const TOOL_DOES: [(&str, &str); 28] = [
    ("get_ticket", "Read the ticket, its comments and attempt"),
    ("submit_outcome", "Finish the step with an outcome and a summary"),
    ("ask_user", "Ask you questions (they wait in the Inbox)"),
    ("add_ticket_comment", "Comment on the ticket"),
    ("add_review_comment", "Comment on a line of the attempt's changes"),
    ("update_ticket", "Rewrite its own ticket's title and description"),
    ("catalog_search", "Search the workflow node catalog"),
    ("catalog_get", "Read a catalog node"),
    ("workflow_get", "Read a workflow"),
    ("workflow_validate", "Check a workflow document"),
    ("workflow_dry_run", "Try a workflow without running it"),
    ("workflow_save", "Save a workflow (a new version)"),
    ("get_overview", "See every project's board, tickets in flight, the Inbox and running sessions"),
    ("search_tickets", "Search tickets in every project"),
    ("get_session", "Read a session's state and last messages"),
    ("list_workflow_runs", "List workflow runs and their steps"),
    ("search_notes", "Search the PM's notes and journal"),
    ("save_note", "Save a note"),
    ("delete_note", "Delete a note"),
    ("write_journal", "Write the session's journal entry"),
    ("create_ticket", "Create tickets"),
    ("edit_ticket", "Edit any ticket"),
    ("link_tickets", "Link two tickets (blocks, relates to)"),
    ("unlink_tickets", "Remove a link between tickets"),
    ("send_to_session", "Message a session"),
    ("discord_get", "Look at the Discord server: its channels and webhooks"),
    ("discord_change", "Create, rename, move or archive Discord channels"),
    ("discord_webhook", "Make a Discord webhook for discord.post steps"),
];

/// One pdt-mcp tool as the Personas page lists it (`GET /api/personas/tools`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonaTool {
    pub name: String,
    /// What it lets the agent do, in a line.
    pub does: String,
    /// It asks the user before each call ([`ASKING_TOOLS`]).
    pub asks: bool,
    /// In the default set ([`DEFAULT_TOOLS`]).
    pub default: bool,
}

/// Every pdt-mcp tool, in [`ALL_TOOLS`]' order, with what it does.
pub fn tool_list() -> Vec<PersonaTool> {
    ALL_TOOLS
        .iter()
        .map(|&name| PersonaTool {
            name: name.to_string(),
            does: TOOL_DOES.iter().find(|(n, _)| *n == name).map(|(_, d)| d.to_string()).unwrap_or_default(),
            asks: asks(name),
            default: DEFAULT_TOOLS.contains(&name),
        })
        .collect()
}

/// Whether a call to `tool` needs the user's yes (see [`ASKING_TOOLS`]).
pub fn asks(tool: &str) -> bool {
    ASKING_TOOLS.contains(&tool)
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

/// A built-in persona brought up to a newer template (PDT-100). `base` is the template PDT applied last: each field
/// the user left as it was follows `template`; a field they changed stays theirs. Returns the setup and the fields
/// kept (named as in [`PersonaDiff::changed`]) whose template moved on.
pub fn follow_template(
    base: &PersonaSetup,
    current: &PersonaSetup,
    template: &PersonaSetup,
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
    let tools = if current.tool_set() == base.tool_set() {
        template.tools.clone()
    } else {
        if template.tool_set() != base.tool_set() {
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
    BuiltinPersona {
        id: "pm",
        name: "PM",
        role: "pm",
        description: "The project manager you talk to; also triage in a workflow.",
        permission_mode: Some(PermissionMode::Ask),
        tools: &PM_TOOLS,
        instructions: PM_INSTRUCTIONS,
    },
];

/// The PM persona's instructions (PDT-81). PDT's own PM context and the briefing come before them.
pub const PM_INSTRUCTIONS: &str = "You are the PM: the user's project manager across every project in PDT.\n\n\
- Ground every answer in PDT's data: look it up with your tools (`get_overview`, `search_tickets`, `get_ticket`, \
`get_session`, `list_workflow_runs`) instead of guessing, and name the tickets (`PDT-12`) and sessions you mean.\n\
- \"Why is it stuck?\": read the ticket, its attempt, PR and CI, its sessions (activity, open requests, last \
messages), its workflow runs and the Inbox, then say what it waits on and who can unblock it.\n\
- Drafting a ticket: write the title and description (scope, out of scope, \"Done when\") in your reply first, then \
call `create_ticket`; the user confirms it (PDT shows them the call) unless the project lets you act. The same for \
`edit_ticket`, `link_tickets`, `unlink_tickets` and `send_to_session`. Never claim you made a change the tool did not \
report.\n\
- Links: \"make A blocked by B\" is `link_tickets` (`blocks`, `blockedBy` or `relates`), and `unlink_tickets` removes \
one; a ticket's parent is `edit_ticket`'s `parent` (`\"\"` detaches it). Never fall back to a comment asking the user \
to link them.\n\
- \"How does X work\", estimates and splitting a ticket: read the project's code first (its folder is in the \
briefing) and name the files you mean. What an attempt changed: `git diff <base>...<branch>` in its project's \
checkout (`get_ticket` gives the branch and base). Hand broad digging to a subagent when your agent has them.\n\
- Research: search and read the web to compare libraries and approaches or to read external docs; link your sources.\n\
- Design: write short design notes and specs in your reply, then put them on the ticket (its description with \
`edit_ticket`, or `add_ticket_comment`); a longer one goes in a file only where the user says.\n\
- Setup: install tools (brew, cargo, pnpm, …), edit config files and set up a project's `.pdt/project.toml` (PDT's \
`run-config` guide) when the user asks. Changing a project's code is a coding agent's work: draft a ticket for it \
instead of editing or committing in a checkout.\n\
- Workflows: find node kinds with `catalog_search`/`catalog_get`, check documents with `workflow_validate` and \
`workflow_dry_run`; `workflow_save` only proposes a version or saves a disabled workflow for the user to enable.\n\
- Memory: what PDT's data can't tell you (the user's preferences, decisions, people, rules at work) goes in a note \
with `save_note` (pin what every session should see); look things up with `search_notes`. When PDT tells you the \
session is ending, write your journal with `write_journal`.\n\
- Discord: `discord_get` shows the server (channels, categories, webhooks). `discord_change` creates, renames, \
moves and archives channels and categories; `discord_webhook` makes a channel's webhook and saves it as a \
`discord_webhook` connection that `discord.post` steps use by name (its URL never reaches you). Both wait for the \
user's yes unless the bot connection lets you act; say what you will change first.
- In a workflow step, finish with `submit_outcome` and one of the step's outcomes.\n\
- Be brief: short answers, lists over prose.\n";

pub fn builtin_persona(id: &str) -> Option<&'static BuiltinPersona> {
    BUILTIN_PERSONAS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_tool_says_what_it_does() {
        let names: Vec<&str> = TOOL_DOES.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ALL_TOOLS, "TOOL_DOES lists ALL_TOOLS, in order");
        let list = tool_list();
        assert_eq!(list.len(), ALL_TOOLS.len());
        assert!(list.iter().all(|t| !t.does.is_empty()));
        assert!(list.iter().find(|t| t.name == "discord_change").unwrap().asks);
        assert!(!list.iter().find(|t| t.name == "discord_get").unwrap().asks);
        let defaults: Vec<&str> = list.iter().filter(|t| t.default).map(|t| t.name.as_str()).collect();
        assert_eq!(defaults, DEFAULT_TOOLS);
    }

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
    fn the_builtins_are_the_pipeline_roles_and_the_pm() {
        let names: Vec<_> = BUILTIN_PERSONAS.iter().map(|p| p.name).collect();
        assert_eq!(names, ["Refiner", "Coder", "Reviewer", "QA", "PM"]);
        for p in BUILTIN_PERSONAS {
            assert!(p.setup().validate().is_ok(), "{}", p.id);
            assert!(p.instructions.contains("submit_outcome"), "{}", p.id);
        }
        assert!(builtin_persona("qa").is_some() && builtin_persona("nobody").is_none());
    }

    #[test]
    fn the_pm_tools_are_pdt_tools_and_the_asking_ones_are_among_them() {
        for t in PM_TOOLS.iter().chain(&ASKING_TOOLS) {
            assert!(ALL_TOOLS.contains(t), "{t}");
        }
        let pm = builtin_persona("pm").unwrap().setup();
        assert_eq!(pm.tool_set().len(), PM_TOOLS.len());
        assert!(ASKING_TOOLS.iter().all(|t| pm.tool_set().iter().any(|x| x == t)));
        assert!(asks("create_ticket") && !asks("get_overview") && !asks("add_ticket_comment"));
        assert_eq!(pm.permission_mode, Some(PermissionMode::Ask));
    }

    #[test]
    fn an_edited_built_in_follows_its_template_where_the_user_left_it() {
        let old = builtin_persona("pm").unwrap().setup();
        let new = PersonaSetup { instructions: "New PM text.".into(), ..old.clone() };
        // Untouched: the new template.
        assert_eq!(follow_template(&old, &old, &new), (new.clone(), vec![]));
        // Only the model changed: it stays, the instructions follow.
        let mine = PersonaSetup { model: Some("opus".into()), ..old.clone() };
        let (s, kept) = follow_template(&old, &mine, &new);
        assert_eq!(s, PersonaSetup { model: Some("opus".into()), ..new.clone() });
        assert!(kept.is_empty(), "the template's model did not move: {kept:?}");
        // Edited instructions stay the user's, and are named.
        let mine = PersonaSetup { instructions: "Mine.".into(), ..old.clone() };
        assert_eq!(follow_template(&old, &mine, &new), (mine.clone(), vec!["instructions"]));
        // Tools compare as sets the persona gets (empty is the default set).
        let base = PersonaSetup { tools: vec![], ..old.clone() };
        let same = PersonaSetup { tools: DEFAULT_TOOLS.map(String::from).to_vec(), ..old.clone() };
        let more = PersonaSetup { tools: ALL_TOOLS.map(String::from).to_vec(), ..old.clone() };
        assert_eq!(follow_template(&base, &same, &more).0.tools, more.tools);
    }
}
