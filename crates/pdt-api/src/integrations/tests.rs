use super::*;
use crate::workflow::catalog::{Category, KindSpec, Outcome, StepCx, parses};
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
    outside: &[],
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
        parses::<Value>(config)
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
    fn category(&self) -> Option<PluginCategory> {
        Some(PluginCategory::Chat)
    }
    fn project_settings(&self) -> Option<ProjectSettingsSchema> {
        Some(ProjectSettingsSchema {
            integration: "dummy".into(),
            label: "Dummy".into(),
            description: "Each project can poke its own Dummy.".into(),
            connection_kind: "dummy".into(),
            fields: vec![],
            ..Default::default()
        })
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

fn with_dummy() -> Registry {
    Registry::new(own().into_iter().chain([Arc::new(Dummy) as Arc<dyn Integration>]).collect(), vec!["dummy"])
}

#[test]
fn registering_an_integration_adds_its_connection_kind_node_kinds_and_tool() {
    let without = Registry::new(own(), vec![]);
    let r = with_dummy();

    let kinds: Vec<String> = r.connection_kinds().into_iter().map(|k| k.id).collect();
    assert_eq!(kinds.last().map(String::as_str), Some("dummy"));
    assert_eq!(kinds.len(), without.connection_kinds().len() + 1);

    let catalog = r.catalog();
    let poke = catalog.spec("dummy.poke").expect("its node kind");
    assert_eq!(poke.category, Category::Action);
    let trigger = catalog.spec("dummy.happened").expect("a trigger on its event type");
    assert_eq!(trigger.category, Category::Trigger);
    assert!(r.event_type("dummy.happened").is_some());
    let node: pdt_core::workflow::doc::Node =
        serde_json::from_value(json!({"id": "t", "kind": "dummy.happened", "version": 1})).unwrap();
    assert!(catalog.facts(&node).unwrap().trigger);
    assert!(without.catalog().spec("dummy.poke").is_none());

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

#[test]
fn an_action_is_both_a_node_kind_and_a_tool() {
    let r = with_dummy();
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
    let catalog = r.catalog();
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

/// PDT-173: an integration this build has registers nothing until the instance enables it, and nothing once it is
/// disabled again; a workflow node of it then names it.
#[test]
fn a_disabled_integration_registers_nothing() {
    let ints = Integrations::new(vec![Arc::new(Dummy)]);
    let has = |ints: &Integrations| {
        let r = ints.registry();
        (
            r.connection_kinds().iter().any(|k| k.id == "dummy"),
            r.catalog().spec("dummy.poke").is_some(),
            r.tool_table().contains("dummy_blink"),
            r.event_type("dummy.happened").is_some(),
        )
    };
    assert_eq!(has(&ints), (false, false, false, false));
    assert!(ints.enabled().is_empty());
    let node: pdt_core::workflow::doc::Node =
        serde_json::from_value(json!({"id": "p", "kind": "dummy.poke", "version": 1})).unwrap();
    assert!(ints.registry().catalog().facts(&node).is_none());

    ints.enable(&["dummy".into(), "nope".into()]);
    assert_eq!(ints.enabled(), ["dummy"], "only what the build has");
    assert_eq!(has(&ints), (true, true, true, true));
    assert!(ints.registry().catalog().facts(&node).is_some());

    ints.enable(&[]);
    assert_eq!(has(&ints), (false, false, false, false));
    assert!(ints.registry().tool_table().contains("get_ticket"), "PDT's own are always on");
}

/// PDT-179: Settings → Plugins names each integration's tools and the PM's among them, enabled or not, so the
/// upgrade rehearsal can check the tool lists.
#[test]
fn the_integrations_list_names_their_tools() {
    let ints = Integrations::new(vec![Arc::new(Dummy)]);
    let list = infos(&ints, &PluginFacts::default());
    let dummy = list.iter().find(|i| i.id == "dummy").unwrap();
    assert!(!dummy.enabled);
    assert_eq!(dummy.tools, ["dummy_blink", "dummy_wink", "dummy_poke"]);
    assert_eq!(dummy.pm_tools, ["dummy_blink", "dummy_poke"]);
    let lacking = list.iter().find(|i| !i.built).unwrap();
    assert!(lacking.tools.is_empty() && lacking.pm_tools.is_empty());
}

/// A node of a known integration that is turned off, or that this build lacks, says so rather than "unknown kind".
#[test]
fn a_turned_off_integrations_node_names_it() {
    use pdt_core::workflow::validate::{Severity, validate};
    let doc: pdt_core::workflow::doc::WorkflowDoc = serde_json::from_value(json!({"name": "w", "nodes": [
        {"id": "m", "kind": "manual", "version": 1},
        {"id": "j", "kind": "jira.comment", "version": 1, "config": {}},
        {"id": "x", "kind": "nope.nothing", "version": 1}],
        "edges": [{"from": "m", "to": "j"}, {"from": "j", "to": "x"}]}))
    .unwrap();
    let errors = |r: &Registry| -> Vec<(String, String)> {
        validate(&doc, r.catalog())
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| (i.code, i.message))
            .collect()
    };
    let built = Registry::new(own(), vec!["jira"]);
    let e = errors(&built);
    assert!(
        e.iter().any(|(c, m)| c == "integration_disabled" && m.contains("Jira") && m.contains("turned off")),
        "{e:?}"
    );
    assert!(e.iter().any(|(c, _)| c == "unknown_kind"), "{e:?}");
    let lacking = Registry::new(own(), vec![]);
    let e = errors(&lacking);
    assert!(e.iter().any(|(c, m)| c == "integration_missing" && m.contains("built without")), "{e:?}");
    let node = doc.node("j").unwrap();
    assert!(built.catalog().current_config(node).unwrap_err().contains("turned off"));
}

fn connection(name: &str, kind: &str, last_error: Option<&str>) -> pdt_core::workflow::api::Connection {
    pdt_core::workflow::api::Connection {
        id: format!("c-{name}"),
        name: name.into(),
        kind: kind.into(),
        url: String::new(),
        config: json!({}),
        has_secret: true,
        created_at: 0,
        updated_at: 0,
        last_error: last_error.map(String::from),
        last_error_at: last_error.map(|_| 1),
    }
}

fn find<'a>(list: &'a [IntegrationInfo], id: &str) -> &'a IntegrationInfo {
    list.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no plugin {id}"))
}

/// PDT-228: Settings → Plugins lists PDT's own plugins (GitHub and the general ones, built in) but not its core, then
/// the built ones and the known ones this build lacks, each with its category, icon and what it adds.
#[test]
fn the_plugins_list_says_what_each_adds() {
    let ints = Integrations::new(vec![Arc::new(Dummy)]);
    let list = infos(&ints, &PluginFacts::default());
    let ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(&ids[..5], ["webhook", "github", "http", "mcp", "dummy"]);
    assert!(!ids.contains(&"core") && !ids.contains(&"forge") && !ids.contains(&"bug_reports"));
    let github = find(&list, "github");
    assert!(github.core && github.enabled && github.built);
    assert_eq!(github.category, PluginCategory::Dev);
    assert_eq!(github.icon.letters, "GH");
    assert!(github.node_kinds.iter().any(|k| k.id == "github.import"), "{:?}", github.node_kinds);
    let http = find(&list, "http");
    assert_eq!((http.category, http.connection_kinds[0].id.as_str()), (PluginCategory::General, "http"));
    assert_eq!(http.node_kinds.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["http.request"]);
    let dummy = find(&list, "dummy");
    assert!(!dummy.core && !dummy.enabled);
    assert_eq!(dummy.category, PluginCategory::Chat);
    assert_eq!(dummy.connection_kinds.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["dummy"]);
    // Its trigger, its own node kind, then its action's.
    assert_eq!(
        dummy.node_kinds.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
        ["dummy.happened", "dummy.poke", "dummy.blink"]
    );
    assert_eq!(dummy.project_settings.as_deref(), Some("Each project can poke its own Dummy."));
    // A known plugin this build lacks still has its category, icon and description.
    let plex = find(&list, "plex");
    assert_eq!((plex.status, plex.category, plex.built), (PluginStatus::NotBuilt, PluginCategory::Media, false));
    assert!(!plex.description.is_empty() && plex.icon.letters == "P");
    assert_eq!(find(&list, "jira").category, PluginCategory::Dev);
    assert_eq!(find(&list, "discord").category, PluginCategory::Chat);
}

/// PDT-228: a plugin's status: Off, Needs setup (on without a connection of its kinds), Error (a connection failing),
/// On; GitHub's follows its login. The general built-in plugins never need setup.
#[test]
fn a_plugins_status_comes_from_its_connections() {
    let ints = Integrations::new(vec![Arc::new(Dummy)]);
    let status = |facts: &PluginFacts, id: &str| {
        let list = infos(&ints, facts);
        let i = find(&list, id);
        (i.status, i.error.clone())
    };
    let none = PluginFacts::default();
    assert_eq!(status(&none, "dummy"), (PluginStatus::Off, None));
    ints.enable(&["dummy".into()]);
    assert_eq!(status(&none, "dummy"), (PluginStatus::NeedsSetup, None));
    assert_eq!(status(&none, "webhook"), (PluginStatus::On, None));
    assert_eq!(status(&none, "github"), (PluginStatus::On, None));
    let ok = PluginFacts { connections: vec![connection("d1", "dummy", None)], ..Default::default() };
    assert_eq!(status(&ok, "dummy"), (PluginStatus::On, None));
    let failing = PluginFacts {
        connections: vec![connection("d1", "dummy", None), connection("d2", "dummy", Some("connection refused"))],
        ..Default::default()
    };
    assert_eq!(status(&failing, "dummy"), (PluginStatus::Error, Some("d2: connection refused".into())));
    assert_eq!(find(&infos(&ints, &failing), "dummy").error_connection.as_deref(), Some("c-d2"));
    let hook = PluginFacts { connections: vec![connection("h", "webhook", Some("bad"))], ..Default::default() };
    assert_eq!(status(&hook, "webhook").0, PluginStatus::Error);
    let login = |needed, rejected| {
        let kind = pdt_core::forge::ForgeKind::GITHUB;
        PluginFacts { forges: vec![ForgeLogin { kind, label: "GitHub", needed, rejected }], ..Default::default() }
    };
    assert_eq!(status(&login(true, false), "github"), (PluginStatus::NeedsSetup, None));
    let rejected = status(&login(true, true), "github");
    assert_eq!(rejected.0, PluginStatus::Error);
    assert_eq!(rejected.1.as_deref(), Some("GitHub no longer accepts PDT's login: the token was revoked or expired."));
    assert_eq!(status(&login(false, false), "github"), (PluginStatus::On, None));
    ints.enable(&[]);
    assert_eq!(status(&failing, "dummy"), (PluginStatus::Off, None));
}

/// PDT-228: a plugin's page links the projects that have settings for it.
#[test]
fn a_plugin_names_the_projects_linked_to_it() {
    let ints = Integrations::new(vec![Arc::new(Dummy)]);
    let p = |key: &str| PluginProject { id: format!("id-{key}"), key: key.into(), name: format!("Project {key}") };
    let facts = PluginFacts {
        projects: vec![(p("A"), vec!["dummy".into()]), (p("B"), vec![]), (p("C"), vec!["jira".into(), "dummy".into()])],
        ..Default::default()
    };
    let list = infos(&ints, &facts);
    assert_eq!(find(&list, "dummy").projects.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["A", "C"]);
    // One without project settings links none.
    assert!(find(&list, "http").projects.is_empty());
}
