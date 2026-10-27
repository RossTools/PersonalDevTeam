use super::*;
use crate::workflow::catalog::{Catalog, Category, KindSpec, Outcome, StepCx, parse};
use pdt_core::workflow::events::schema_of;
use pdt_core::workflow::validate::Kinds;
use std::sync::Arc;

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

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlinkInput {
    /// A `dummy` connection.
    #[schemars(extend("x-connection-kind" = "dummy"))]
    connection: String,
    times: u32,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct Blinked {
    blinks: u32,
}

/// An action: a node kind and a tool from one definition.
struct Blink;

impl action::Action for Blink {
    type Input = BlinkInput;
    type Output = Blinked;
    fn tool(&self) -> action::ToolFace {
        action::ToolFace {
            decl: ToolDecl::new("dummy_blink", "Blink Dummy").read_only().open_world().pm(),
            title: "Blink Dummy",
            description: "Blinks it.",
        }
    }
    fn node(&self) -> Option<action::NodeFace> {
        Some(action::NodeFace {
            id: "dummy.blink",
            title: "Blink",
            description: "Blinks it.",
            ports: &[("out", "Then")],
        })
    }
    fn connection(&self) -> Option<action::ConnectionFace> {
        Some(action::ConnectionFace { kind: "dummy", label: "Dummy" })
    }
    fn check(&self, i: &BlinkInput) -> Result<(), String> {
        if i.times == 0 { Err("blink at least once".into()) } else { Ok(()) }
    }
    fn approval_title(&self, i: &BlinkInput) -> String {
        format!("blink {} {} times", i.connection, i.times)
    }
    fn run<'a>(&'a self, _cx: &'a action::ActionCx<'a>, i: BlinkInput) -> BoxFut<'a, Result<Blinked, String>> {
        Box::pin(async move { Ok(Blinked { blinks: i.times }) })
    }
}

/// A tool-only action.
struct Wink;

impl action::Action for Wink {
    type Input = Value;
    type Output = Value;
    fn tool(&self) -> action::ToolFace {
        action::ToolFace { decl: ToolDecl::new("dummy_wink", "Wink"), title: "Wink", description: "Winks." }
    }
    fn approval_title(&self, _: &Value) -> String {
        "wink".into()
    }
    fn run<'a>(&'a self, _cx: &'a action::ActionCx<'a>, i: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(async move { Ok(i) })
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
    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        vec![Arc::new(Blink), Arc::new(Wink)]
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

/// What agents see of the tools (each one's `tools/list` entry and declaration, the PM's tools, the personas' list) and
/// the spec of every node kind an [`Action`](super::action::Action) makes, against `snapshot.json`: moving a capability
/// onto an action changes none of it (PDT-172). Other kinds are left out, so a change to one doesn't touch this. After a change meant to show, write it again with `PDT_UPDATE_SNAPSHOT=1 cargo test`.
#[test]
fn tools_and_node_kinds_match_the_snapshot() {
    let r = registry();
    let t = r.tool_table();
    let tools: Vec<Value> = r
        .tool_infos()
        .into_iter()
        .map(|i| {
            let d = t.get(&i.name).unwrap();
            json!({"info": i, "does": d.does, "asks": d.asks, "pm": d.pm, "readOnly": d.read_only,
                   "openWorld": d.open_world, "destructive": d.destructive})
        })
        .collect();
    let ids: Vec<String> =
        all().iter().flat_map(|i| i.actions()).filter_map(|a| a.node_kind().map(|k| k.spec().id)).collect();
    let mut kinds: Vec<_> =
        crate::workflow::catalog::catalog().specs().into_iter().filter(|k| ids.contains(&k.id)).collect();
    kinds.sort_by(|a, b| a.id.cmp(&b.id));
    let now = json!({"tools": tools, "pm": t.pm_tools(), "asking": t.asking_tools(), "personaTools": t.persona_tools(),
                     "kinds": kinds});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/integrations/snapshot.json");
    // One line per entry, so a diff shows which one changed.
    let lines: Vec<String> = now
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            let items: Vec<String> = v.as_array().unwrap().iter().map(|x| format!("    {x}")).collect();
            format!("  {k:?}: [\n{}\n  ]", items.join(",\n"))
        })
        .collect();
    let text = format!("{{\n{}\n}}\n", lines.join(",\n"));
    if std::env::var_os("PDT_UPDATE_SNAPSHOT").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let was: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // The entries that differ, each as it was and as it is.
    let mut changed: Vec<String> = vec![];
    for (k, v) in now.as_object().unwrap() {
        let old = was[k].as_array().cloned().unwrap_or_default();
        let new = v.as_array().unwrap();
        for i in 0..old.len().max(new.len()) {
            if old.get(i) != new.get(i) {
                changed.push(format!("{k}[{i}]:\n  was {}\n  now {}", json!(old.get(i)), json!(new.get(i))));
            }
        }
    }
    assert!(
        changed.is_empty(),
        "the tools or node kinds changed (run with PDT_UPDATE_SNAPSHOT=1 and check the diff):\n{}",
        changed.join("\n")
    );
}

#[test]
fn an_action_is_both_a_node_kind_and_a_tool() {
    let r = Registry::new(all().into_iter().chain([Box::new(Dummy) as Box<dyn Integration>]).collect());
    // Its tools come before the integration's own, in order.
    let names = r.tool_table().names();
    assert_eq!(names[names.len() - 3..], ["dummy_blink", "dummy_wink", "dummy_poke"]);
    assert!(r.tool_table().pm_tools().contains(&"dummy_blink"));
    let info = r.tool_infos().into_iter().find(|i| i.name == "dummy_blink").unwrap();
    assert_eq!(info.annotations["readOnlyHint"], true);
    // A tool call may leave the connection out.
    assert_eq!(info.input_schema["required"], json!(["times"]));
    let conn = &info.input_schema["properties"]["connection"];
    assert_eq!(conn["description"], "A Dummy connection's name; the only one when left out.");
    assert!(conn.get("x-connection-kind").is_none());

    // The node needs it, and checks the config as the action says.
    let catalog = Catalog::new(r.node_kinds());
    let blink = catalog.spec("dummy.blink").expect("its node kind");
    assert_eq!((blink.category, blink.version, blink.ports.len()), (Category::Action, 1, 1));
    assert_eq!(blink.config_schema["properties"]["connection"]["x-connection-kind"], "dummy");
    assert_eq!(blink.output_schema["properties"]["blinks"]["type"], "integer");
    let node = |config: Value| -> pdt_core::workflow::doc::Node {
        serde_json::from_value(json!({"id": "b", "kind": "dummy.blink", "version": 1, "config": config})).unwrap()
    };
    assert_eq!(catalog.facts(&node(json!({"connection": "d", "times": 2}))).unwrap().config_error, None);
    assert_eq!(
        catalog.facts(&node(json!({"connection": "d", "times": 0}))).unwrap().config_error.as_deref(),
        Some("blink at least once")
    );
    assert!(catalog.facts(&node(json!({"times": 2}))).unwrap().config_error.unwrap().contains("connection"));
    // A tool-only action has no node kind.
    assert!(catalog.specs().iter().all(|s| s.id != "dummy_wink" && !s.id.contains("wink")));
}
