use super::*;
use crate::workflow::catalog::{Catalog, Category, KindSpec, Outcome, StepCx, parse};
use pdt_core::workflow::events::schema_of;
use pdt_core::workflow::validate::Kinds;

/// An integration that adds one of everything.
struct Dummy;

#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
struct DummyEvent {
    what: String,
}

const DUMMY_EVENTS: &[EventType] = &[EventType {
    id: "dummy.happened",
    title: "Dummy happened",
    description: "Something happened in Dummy.",
    schema: schema_of::<DummyEvent>,
}];

struct DummyNode;

impl NodeKind for DummyNode {
    fn spec(&self) -> KindSpec {
        crate::workflow::catalog::spec::<serde_json::Value, serde_json::Value>(
            "dummy.poke",
            1,
            Category::Action,
            "Poke Dummy",
            "Pokes it.",
            &[("out", "Then")],
            false,
        )
    }
    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<Value>(config).map(|_| ())
    }
    fn run<'a>(&'a self, _cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async { Ok(Outcome::next("out", json!({}))) })
    }
}

impl Integration for Dummy {
    fn id(&self) -> &'static str {
        "dummy"
    }
    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind { id: "dummy".into(), label: "Dummy".into(), ..Default::default() }]
    }
    fn event_types(&self) -> &'static [EventType] {
        DUMMY_EVENTS
    }
    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(DummyNode)]
    }
    fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            decl: ToolDecl::new("dummy_poke", "Poke Dummy").asks().open_world().pm(),
            title: "Poke Dummy",
            description: "Pokes it.",
            schema: || json!({"type": "object"}),
        }]
    }
}

#[test]
fn registering_an_integration_adds_its_connection_kind_node_kinds_and_tool() {
    let without = Registry::new(all());
    let r = Registry::new(all().into_iter().chain([Box::new(Dummy) as Box<dyn Integration>]).collect());

    let kinds: Vec<&str> = r.connection_kinds().iter().map(|k| k.id.as_str()).collect();
    assert_eq!(kinds.last(), Some(&"dummy"));
    assert_eq!(kinds.len(), without.connection_kinds().len() + 1);

    let catalog = Catalog::new(r.node_kinds());
    let poke = catalog.spec("dummy.poke").expect("its node kind");
    assert_eq!(poke.category, Category::Action);
    let trigger = catalog.spec("dummy.happened").expect("a trigger on its event type");
    assert_eq!(trigger.category, Category::Trigger);
    assert!(r.event_type("dummy.happened").is_some());
    let node: pdt_core::workflow::doc::Node =
        serde_json::from_value(json!({"id": "t", "kind": "dummy.happened", "version": 1})).unwrap();
    assert!(catalog.facts(&node).unwrap().trigger);
    assert!(Catalog::new(without.node_kinds()).spec("dummy.poke").is_none());

    let t = r.tool_table();
    assert_eq!(t.names().last(), Some(&"dummy_poke"));
    assert!(t.asks("dummy_poke") && t.pm_tools().contains(&"dummy_poke"));
    let info = r.tool_infos().into_iter().find(|i| i.name == "dummy_poke").unwrap();
    assert_eq!(info.annotations["openWorldHint"], true);
    assert!(!without.tool_table().contains("dummy_poke"));
    // The PM persona picks it up; a persona with the default set doesn't.
    let pm = pdt_core::persona::builtin_persona("pm").unwrap().setup(t);
    assert!(pm.tool_set(t).iter().any(|x| x == "dummy_poke"));
    let coder = pdt_core::persona::builtin_persona("coder").unwrap().setup(t);
    assert!(!coder.tool_set(t).iter().any(|x| x == "dummy_poke"));
}

/// The tools, their order and flags as they were before the registry (PDT-171): only where they were.
#[test]
fn the_tools_are_where_they_were() {
    let t = registry().tool_table();
    assert_eq!(
        t.names(),
        [
            "get_ticket",
            "submit_outcome",
            "ask_user",
            "add_ticket_comment",
            "add_review_comment",
            "list_secrets",
            "request_secret",
            "run_with_secrets",
            "write_secret_file",
            "update_ticket",
            "catalog_search",
            "catalog_get",
            "workflow_get",
            "workflow_validate",
            "workflow_dry_run",
            "workflow_save",
            "workflow_export",
            "workflow_import",
            "get_overview",
            "search_tickets",
            "get_session",
            "list_workflow_runs",
            "search_notes",
            "save_note",
            "delete_note",
            "write_journal",
            "list_todos",
            "create_ticket",
            "edit_ticket",
            "link_tickets",
            "unlink_tickets",
            "stop_ticket",
            "send_to_session",
            "discord_get",
            "discord_change",
            "discord_webhook",
            "scryer_library",
            "scryer_search",
            "scryer_queue",
            "scryer_add",
            "scryer_schema",
            "scryer_query",
            "scryer_mutate",
            "report_bug",
        ]
    );
    assert_eq!(
        t.asking_tools(),
        [
            "workflow_import",
            "create_ticket",
            "edit_ticket",
            "link_tickets",
            "unlink_tickets",
            "stop_ticket",
            "send_to_session",
            "discord_change",
            "discord_webhook",
            "scryer_add",
            "scryer_mutate",
            "report_bug",
        ]
    );
    let not_pm: Vec<&str> = t.all().iter().filter(|d| !d.pm).map(|d| d.name).collect();
    assert_eq!(not_pm, ["add_review_comment", "update_ticket"]);
    let list = t.persona_tools();
    assert!(list.iter().all(|x| !x.does.is_empty()));
    let defaults: Vec<&str> = list.iter().filter(|x| x.default).map(|x| x.name.as_str()).collect();
    assert_eq!(defaults, pdt_core::persona::DEFAULT_TOOLS);
    let read_only: Vec<&str> = t.all().iter().filter(|d| d.read_only).map(|d| d.name).collect();
    assert_eq!(read_only.len(), 20, "{read_only:?}");
    let open: Vec<&str> = t.all().iter().filter(|d| d.open_world).map(|d| d.name).collect();
    assert!(open.iter().all(|n| n.starts_with("discord_") || n.starts_with("scryer_") || *n == "report_bug"));
    assert_eq!(open.len(), 11);
    let pm = pdt_core::persona::builtin_persona("pm").unwrap().setup(t);
    assert_eq!(pm.tool_set(t).len(), 42);
}

#[test]
fn the_connection_kinds_are_the_pages_kinds_in_order() {
    let ids: Vec<&str> = registry().connection_kinds().iter().map(|k| k.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "discord_webhook",
            "discord_bot",
            "webhook",
            "jira",
            "buildkite",
            "http",
            "radarr",
            "sonarr",
            "scryer",
            "plex",
            "mcp"
        ]
    );
}

/// The web tests' stand-in for `GET /api/connections/kinds` (web/src/test-utils.tsx serves it), written beside the
/// generated TS types so CI's check that they are committed covers it too.
#[test]
fn exports_the_connection_kinds_for_the_web_tests() {
    let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else { return };
    let path = std::path::Path::new(&dir).join("connectionKinds.ts");
    let text = format!(
        "// Written by pdt-api's integrations tests (`exports_the_connection_kinds_for_the_web_tests`). Do not edit this \
         file manually.\nimport type {{ ConnectionKind }} from \"./ConnectionKind\";\n\nexport const connectionKinds: \
         ConnectionKind[] = {};\n",
        serde_json::to_string_pretty(registry().connection_kinds()).unwrap()
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).unwrap();
    }
}

#[test]
fn every_tool_is_listed_once_with_an_object_schema() {
    let infos = registry().tool_infos();
    let names: Vec<String> = infos.iter().map(|t| t.name.clone()).collect();
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    for t in &infos {
        assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        assert!(t.input_schema.get("$schema").is_none());
    }
    let s = &infos[1].input_schema;
    assert!(s["required"].as_array().unwrap().iter().any(|r| r == "outcome"), "{s}");
}

#[test]
fn jira_and_buildkite_declare_project_settings() {
    let ids: Vec<&str> = registry().project_settings().map(|s| s.integration.as_str()).collect();
    assert_eq!(ids, ["jira", "buildkite"]);
    let map = |v: Value| v.as_object().unwrap().clone();
    let r = registry();
    let (_, jira) = r.apply_project_settings("jira", &map(json!({"connection": "j"})), None).unwrap().unwrap();
    assert_eq!(Value::Object(jira), json!({"connection": "j", "scope": "", "done": "Done"}));
    let bk = |pipelines: Value| {
        r.apply_project_settings("buildkite", &map(json!({"connection": "b", "pipelines": pipelines})), None)
    };
    assert_eq!(
        Value::Object(bk(json!(["app", " app ", "e2e"])).unwrap().unwrap().1),
        json!({"connection": "b", "pipelines": ["app", "e2e"]})
    );
    assert!(bk(json!(["no spaces"])).unwrap_err().contains("not a pipeline slug"));
    assert!(r.apply_project_settings("discord", &map(json!({})), None).is_err(), "no project settings");
}

/// The web tests' stand-in for `GET /api/integrations/project-settings`, like the connection kinds'.
#[test]
fn exports_the_project_settings_for_the_web_tests() {
    let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else { return };
    let path = std::path::Path::new(&dir).join("projectSettingsSchemas.ts");
    let list: Vec<_> = registry().project_settings().collect();
    let text = format!(
        "// Written by pdt-api's integrations tests (`exports_the_project_settings_for_the_web_tests`). Do not edit \
         this file manually.\nimport type {{ ProjectSettingsSchema }} from \"./ProjectSettingsSchema\";\n\nexport \
         const projectSettingsSchemas: ProjectSettingsSchema[] = {};\n",
        serde_json::to_string_pretty(&list).unwrap()
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).unwrap();
    }
}
