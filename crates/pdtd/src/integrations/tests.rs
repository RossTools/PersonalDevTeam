//! What pdtd with every integration built in and enabled registers (PDT-171, PDT-173): the tools and connection kinds in
//! order, the actions' tools and node kinds against `snapshot.json`, the integrations' project settings, and the
//! fixtures the web tests serve. Only with every integration's feature on (pdtd's default).

use pdt_api::Registry;
use serde_json::{Value, json};
use std::sync::Arc;

/// The registry with every integration this build has enabled.
fn registry() -> Arc<Registry> {
    let ints = pdt_api::Integrations::new(super::built());
    let ids: Vec<String> = ints.built().iter().map(|i| i.id().to_string()).collect();
    ints.enable(&ids);
    ints.registry()
}

/// The tools, their order and flags: PDT's own, then the integrations' (PDT-171, PDT-173).
#[test]
fn the_tools_are_where_they_were() {
    let r = registry();
    let t = r.tool_table();
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
            "workflow_run",
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
            "change_request_view",
            "change_request_comment",
            "change_request_open",
            "change_request_merge",
            "ci_runs",
            "ci_failed_logs",
            "issue_view",
            "issue_comment",
            "report_bug",
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
        ]
    );
    assert_eq!(
        t.asking_tools(),
        [
            "workflow_run",
            "workflow_import",
            "create_ticket",
            "edit_ticket",
            "link_tickets",
            "unlink_tickets",
            "stop_ticket",
            "send_to_session",
            "report_bug",
            "discord_change",
            "discord_webhook",
            "scryer_add",
            "scryer_mutate",
        ]
    );
    let not_pm: Vec<&str> = t.all().iter().filter(|d| !d.pm).map(|d| d.name).collect();
    assert_eq!(
        not_pm,
        [
            "add_review_comment",
            "update_ticket",
            "change_request_view",
            "change_request_comment",
            "change_request_open",
            "change_request_merge",
            "ci_runs",
            "ci_failed_logs",
            "issue_comment",
        ],
        "the forge tools (PDT-177) work on the session's project and branch; the PM only reads issues (bug triage)"
    );
    let list = t.persona_tools();
    assert!(list.iter().all(|x| !x.does.is_empty()));
    let defaults: Vec<&str> = list.iter().filter(|x| x.default).map(|x| x.name.as_str()).collect();
    assert_eq!(defaults, pdt_core::persona::DEFAULT_TOOLS);
    let read_only: Vec<&str> = t.all().iter().filter(|d| d.read_only).map(|d| d.name).collect();
    assert_eq!(read_only.len(), 24, "{read_only:?}");
    let open: Vec<&str> = t.all().iter().filter(|d| d.open_world).map(|d| d.name).collect();
    let forge = [
        "change_request_view",
        "change_request_comment",
        "change_request_open",
        "change_request_merge",
        "ci_runs",
        "ci_failed_logs",
        "issue_view",
        "issue_comment",
    ];
    assert!(
        open.iter()
            .all(|n| n.starts_with("discord_") || n.starts_with("scryer_") || *n == "report_bug" || forge.contains(n))
    );
    assert_eq!(open.len(), 19);
    let pm = pdt_core::persona::builtin_persona("pm").unwrap().setup(t);
    assert_eq!(pm.tool_set(t).len(), 44);
}

#[test]
fn the_connection_kinds_are_the_pages_kinds_in_order() {
    let ids: Vec<String> = registry().connection_kinds().into_iter().map(|k| k.id).collect();
    assert_eq!(
        ids,
        [
            "webhook",
            "http",
            "mcp",
            "discord_webhook",
            "discord_bot",
            "jira",
            "buildkite",
            "radarr",
            "sonarr",
            "scryer",
            "plex"
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
        "// Written by pdtd's integrations tests (`exports_the_connection_kinds_for_the_web_tests`). Do not edit this \
         file manually.\nimport type {{ ConnectionKind }} from \"./ConnectionKind\";\n\nexport const connectionKinds: \
         ConnectionKind[] = {};\n",
        serde_json::to_string_pretty(&registry().connection_kinds()).unwrap()
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).unwrap();
    }
}

/// Every plugin as Settings → Plugins lists it with all of this build's turned on (PDT-228), for the web tests:
/// `web/src/api/gen/plugins.ts`.
#[test]
fn exports_the_plugins_for_the_web_tests() {
    let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else { return };
    let path = std::path::Path::new(&dir).join("plugins.ts");
    let ints = pdt_api::Integrations::new(super::built());
    let ids: Vec<String> = ints.built().iter().map(|i| i.id().to_string()).collect();
    ints.enable(&ids);
    let text = format!(
        "// Written by pdtd's integrations tests (`exports_the_plugins_for_the_web_tests`). Do not edit this file \
         manually.\nimport type {{ IntegrationInfo }} from \"./IntegrationInfo\";\n\nexport const plugins: \
         IntegrationInfo[] = {};\n",
        serde_json::to_string_pretty(&pdt_api::plugin_infos(&ints)).unwrap()
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
/// the spec of every node kind an action makes, against `snapshot.json`: moving a capability
/// onto an action changes none of it (PDT-172). Other kinds are left out, so a change to one doesn't touch this. After
/// a change meant to show, write it again with `PDT_UPDATE_SNAPSHOT=1 cargo test`.
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
        r.integrations().iter().flat_map(|i| i.actions()).filter_map(|a| a.node_kind().map(|k| k.spec().id)).collect();
    let mut kinds: Vec<_> = r.catalog().specs().into_iter().filter(|k| ids.contains(&k.id)).collect();
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
fn jira_and_buildkite_declare_project_settings() {
    let r = registry();
    let ids: Vec<&str> = r.project_settings().map(|s| s.integration.as_str()).collect();
    assert_eq!(ids, ["jira", "buildkite"]);
    let map = |v: Value| v.as_object().unwrap().clone();
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
    let r = registry();
    let list: Vec<_> = r.project_settings().collect();
    let text = format!(
        "// Written by pdtd's integrations tests (`exports_the_project_settings_for_the_web_tests`). Do not edit \
         this file manually.\nimport type {{ ProjectSettingsSchema }} from \"./ProjectSettingsSchema\";\n\nexport \
         const projectSettingsSchemas: ProjectSettingsSchema[] = {};\n",
        serde_json::to_string_pretty(&list).unwrap()
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, text).unwrap();
    }
}

/// PDT-173: an install upgraded from before integrations could be turned off, with Scryer and Discord connections,
/// starts with both enabled, and their kinds and tools registered, with no action from the user; the others stay off.
#[tokio::test]
async fn an_upgrade_keeps_the_integrations_in_use() {
    use pdt_core::workflow::api::Connection;
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("pdt.db");
    let store = pdt_store::Store::open(&db).await.unwrap();
    for (name, kind) in [("media", "scryer"), ("alerts", "discord_webhook")] {
        let c = Connection {
            id: name.into(),
            name: name.into(),
            kind: kind.into(),
            url: String::new(),
            config: json!({}),
            has_secret: true,
            created_at: 1,
            updated_at: 1,
            last_error: None,
            last_error_at: None,
        };
        store.insert_connection(&c).await.unwrap();
    }
    // As a store written before PDT-173 has it: no choice made yet.
    store.delete_host_value("integrations.enabled").await.unwrap();
    store.close().await;

    let store = pdt_store::Store::open(&db).await.unwrap();
    let mut st = pdt_api::AppState::new(store, tmp.path().join("data"));
    st.integrations = Arc::new(pdt_api::Integrations::new(super::built()));
    pdt_api::load_enabled_integrations(&st).await.unwrap();
    assert_eq!(st.integrations.enabled(), ["discord", "scryer"]);
    let r = st.registry();
    assert!(r.tool_table().contains("scryer_add") && r.tool_table().contains("discord_change"));
    assert!(r.catalog().spec("discord.post").is_some() && r.catalog().spec("jira.import").is_none());
    let kinds: Vec<String> = r.connection_kinds().into_iter().map(|k| k.id).collect();
    assert!(kinds.contains(&"scryer".to_string()) && !kinds.contains(&"jira".to_string()));
}

/// The calls the PM hooks are checked with (PDT-175): each integration's asking tools, as Claude names them.
fn pm_calls() -> Vec<(&'static str, Value)> {
    vec![
        ("mcp__pdt__create_ticket", json!({"project": "PDT", "title": "Add a logout"})),
        ("mcp__pdt__get_ticket", json!({"ticket": "PDT-4"})),
        (
            "mcp__pdt__discord_change",
            json!({"connection": "home", "changes": [
            {"op": "rename", "channel": "#ai-briefs", "name": "Morning Briefs"}, {"op": "archive", "channel": "general"}]}),
        ),
        ("mcp__pdt__discord_change", json!({})),
        ("mcp__pdt__discord_webhook", json!({"channel": "#ai-briefs", "name": "briefs", "createChannel": true})),
        (
            "mcp__pdt__scryer_add",
            json!({"connection": "media", "facet": "series", "tvdbId": 371980, "name": "Severance"}),
        ),
        (
            "mcp__pdt__scryer_mutate",
            json!({"mutation": "updateGeneralSettings", "args": {"input": {"logLevel": "debug"}}}),
        ),
        (
            "mcp__pdt__scryer_mutate",
            json!({"mutation": "createIndexerConfig", "args": {"input": {
            "name": "NZBgeek", "apiKey": "geek-key-123", "isEnabled": true}}}),
        ),
        (
            "mcp__pdt__scryer_mutate",
            json!({"connection": "home", "mutation": "deleteIndexerConfig", "args": {"id": "7"}}),
        ),
        ("mcp:pdt/scryer_query", json!({"query": "{ x }", "variables": {"password": "hunter2-secret"}})),
    ]
}

/// The PM's instructions with the registry's tools (no briefing), as a PM session gets them.
fn pm_text(r: &Registry) -> String {
    let t = r.tool_table();
    let tools = pdt_core::persona::builtin_persona("pm").unwrap().setup(t).tool_set(t);
    pdt_core::pm::instructions(&pdt_core::pm::PmContext {
        session_id: "s1",
        port: 7421,
        work_folder: "/w",
        tools: &tools,
        tool_table: t,
        persona: pdt_core::persona::PM_INSTRUCTIONS,
        integrations: &r.pm_prompt(&tools),
        briefing: "# Briefing",
        step: None,
    })
}

/// What the PM sees and the user approves with every integration (the prompt, each asking call's target, title and
/// redaction), against `pm_snapshot.md`. Write it again with `PDT_UPDATE_SNAPSHOT=1 cargo test` and check the diff.
#[test]
fn the_pm_hooks_match_the_snapshot() {
    let r = registry();
    let mut text = format!("# The PM's instructions\n\n{}\n# Calls\n", pm_text(&r));
    for (tool, input) in pm_calls() {
        let mut redacted = input.clone();
        let secrets = r.redact_call(tool, &mut redacted);
        text.push_str(&format!(
            "\n`{tool}` {input}\n- target: {:?}\n- title: {:?}\n- redacted: {redacted} (took {secrets:?})\n",
            r.ask_target(tool, &input),
            r.approval_title(tool, &input),
        ));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/integrations/pm_snapshot.md");
    if std::env::var_os("PDT_UPDATE_SNAPSHOT").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let was = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert!(was == text, "the PM hooks changed (run with PDT_UPDATE_SNAPSHOT=1 and check the diff):\n{text}");
}

/// The hooks give what PDT's PM code gave before they moved into Discord's and Scryer's integrations.
#[test]
fn discord_and_scryer_answer_the_pm_hooks_as_before() {
    let r = registry();
    let conn = |kind: &'static str, name: Option<&str>| {
        Some(pdt_core::pm::AskTarget::Connection { kind, name: name.map(String::from) })
    };
    let calls = pm_calls();
    let target = |i: usize| r.ask_target(calls[i].0, &calls[i].1);
    let title = |i: usize| r.approval_title(calls[i].0, &calls[i].1);
    assert_eq!(target(0), Some(pdt_core::pm::AskTarget::Project("PDT".into())));
    assert_eq!((target(1), title(1)), (None, None), "get_ticket doesn't ask; core titles stay the parser's");
    assert_eq!(target(2), conn("discord_bot", Some("home")));
    assert_eq!(title(2).as_deref(), Some("Discord (home): rename #ai-briefs to Morning Briefs; archive general"));
    assert_eq!(title(3).as_deref(), Some("Discord"), "a call it can't read");
    assert_eq!(target(4), conn("discord_bot", None));
    assert_eq!(
        title(4).as_deref(),
        Some("Discord: a webhook on #ai-briefs (made if missing), saved as connection briefs")
    );
    assert_eq!(target(5), conn("scryer", Some("media")));
    assert_eq!(title(5).as_deref(), Some("Scryer (media): add the series Severance (tvdb 371980), monitored"));
    assert_eq!(target(6), conn("scryer", None));
    assert_eq!(target(7), conn("scryer", None), "a credential written out (not a `$secret`) is only redacted");
    assert_eq!(target(8), Some(pdt_core::pm::AskTarget::Always), "deleting always asks");
    assert_eq!(target(9), None, "scryer_query doesn't ask");

    let mut input = calls[7].1.clone();
    assert_eq!(r.redact_call(calls[7].0, &mut input), ["geek-key-123"]);
    assert_eq!(input["args"]["input"]["apiKey"], pdt_core::secret::REDACTED);
    assert!(title(7).is_some_and(|t| !t.contains("geek-key-123")));
    let mut input = calls[9].1.clone();
    assert_eq!(r.redact_call(calls[9].0, &mut input), ["hunter2-secret"], "Codex's name for the tool too");
    let mut input = json!({"password": "not-a-scryer-call"});
    assert!(
        r.redact_call("mcp__pdt__create_ticket", &mut input).is_empty() && input["password"] == "not-a-scryer-call"
    );

    let text = pm_text(&r);
    assert!(text.contains("- Discord: `discord_get`") && text.contains("- Media (Scryer): `scryer_library`"));
    assert!(!pdt_core::persona::PM_INSTRUCTIONS.to_lowercase().contains("scryer"));
}

/// With Discord and Scryer left out, none of their text, tools or hooks reach the PM.
#[test]
fn without_discord_and_scryer_the_pm_has_none_of_them() {
    let ints = pdt_api::Integrations::new(super::built());
    ints.enable(&["jira".into(), "buildkite".into(), "arr".into(), "plex".into()]);
    let r = ints.registry();
    let text = pm_text(&r).to_lowercase();
    for name in ["discord", "scryer"] {
        assert!(!text.contains(name), "{name} in\n{text}");
        assert!(r.tool_table().names().iter().all(|t| !t.contains(name)));
    }
    for (tool, input) in pm_calls().into_iter().skip(2) {
        assert_eq!(r.ask_target(tool, &input), None, "{tool}");
        assert_eq!(r.approval_title(tool, &input), None, "{tool}");
        assert!(r.redact_call(tool, &mut input.clone()).is_empty(), "{tool}");
    }
    assert_eq!(r.connection_label("scryer"), "scryer");
    assert_eq!(registry().connection_label("scryer"), "Scryer");
    assert!(text.contains("`create_ticket`"), "PDT's own tools stay");
}
