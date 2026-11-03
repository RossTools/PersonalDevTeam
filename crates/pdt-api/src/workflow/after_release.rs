//! The "After a release" template (PDT-373): when a project's release is published (stable ones only, by default), it
//! announces it in a chat channel and bumps a Homebrew cask in a tap. Each part is there only when its inputs are
//! given: a chat connection for the announcement, a tap and cask for Homebrew. The announcement goes through `notify`'s
//! chat (whichever plugin posts to the connection's kind), so the template names no plugin.

use super::templates::{doc as parse_doc, params as parse_params};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::TemplateParam;
use pdt_core::workflow::template::fill as fill_params;
use serde_json::{Map, Value, json};

pub const ID: &str = "after-release";

pub fn params() -> Vec<TemplateParam> {
    parse_params(json!([
        {"id": "stableOnly", "title": "Stable releases only", "type": "boolean", "default": true,
         "description": "Pre-releases are left alone. Off: they are announced and bumped too."},
        {"id": "chat", "title": "Announce in", "type": "connection", "default": "",
         "description": "A chat connection to post the release's name, the start of its notes and its link to (a \
                         Discord webhook: its channel). Empty: no announcement."},
        {"id": "tap", "title": "Homebrew tap", "type": "string", "default": "",
         "description": "The tap's repository on GitHub (owner/homebrew-name). Empty: no cask bump."},
        {"id": "cask", "title": "Cask", "type": "string", "default": "",
         "description": "The cask's name: its file is Casks/<cask>.rb in the tap."},
        {"id": "asset", "title": "macOS file", "type": "string", "default": "",
         "description": "Which of the release's files the cask installs, as a pattern (*.dmg, *.app.tar.gz). \
                         Empty: the first .dmg, else .app.tar.gz, else .app.zip."},
        {"id": "push", "title": "Push instead of a pull request", "type": "boolean", "default": false,
         "description": "Commit the new version straight to the tap's branch. Off: open a pull request on the tap."},
        {"id": "tapBranch", "title": "Tap branch", "type": "string", "default": "",
         "description": "The tap's branch the change goes to. Empty: its default branch."},
    ]))
}

fn node(id: &str, kind: &str, x: i32, title: &str, config: Value) -> Value {
    super::doc_build::node(id, kind, x, 0, title, config)
}

/// The document with the announcement, the cask bump, both or neither.
pub fn doc_with(announce: bool, cask: bool) -> WorkflowDoc {
    let mut nodes = vec![
        node("published", "release.published", 0, "A release is published", Value::Null),
        node(
            "prerelease",
            "condition",
            300,
            "A pre-release, stable only?",
            json!({"conditions": [
                {"left": "{{params.stableOnly}}", "op": "truthy"}, {"left": "{{trigger.prerelease}}", "op": "truthy"}]}),
        ),
    ];
    let mut steps = vec![];
    if announce {
        let mut n = node(
            "announce",
            "notify",
            600,
            "Announce it",
            json!({"title": "{{trigger.name}} is out", "body": "{{trigger.summary}}", "link": "{{trigger.url}}",
                   "chat": "{{params.chat}}", "show": "off"}),
        );
        n["retry"] = json!({"max": 3, "backoffMs": 30_000});
        nodes.push(n);
        steps.push(("announce", "out"));
    }
    if cask {
        let x = if announce { 900 } else { 600 };
        let mut n = node(
            "cask",
            super::nodes::homebrew::KIND,
            x,
            "Bump the Homebrew cask",
            json!({"tap": "{{params.tap}}", "cask": "{{params.cask}}", "version": "{{trigger.tag}}",
                   "assets": "{{trigger.assets}}", "asset": "{{params.asset}}", "releaseUrl": "{{trigger.url}}",
                   "push": "{{params.push}}", "branch": "{{params.tapBranch}}"}),
        );
        n["retry"] = json!({"max": 2, "backoffMs": 60_000});
        nodes.push(n);
        steps.push(("cask", "updated"));
    }
    let end_x = 300 * (2 + i32::try_from(steps.len()).unwrap_or(0));
    nodes.push(json!({"id": "done", "kind": "end", "version": 1, "position": {"x": end_x, "y": 0}}));
    nodes.push(json!({"id": "skipped", "kind": "end", "version": 1, "position": {"x": 600, "y": 160},
                      "title": "Skipped", "config": {"quiet": true}}));

    let mut edges = vec![
        json!({"from": "published", "to": "prerelease"}),
        json!({"from": "prerelease", "port": "true", "to": "skipped"}),
    ];
    let mut from = ("prerelease", "false");
    for (id, port) in steps {
        edges.push(json!({"from": from.0, "port": from.1, "to": id}));
        from = (id, port);
    }
    edges.push(json!({"from": from.0, "port": from.1, "to": "done"}));
    if cask {
        edges.push(json!({"from": "cask", "port": "unchanged", "to": "done"}));
    }
    parse_doc(json!({
        "name": "After a release",
        "description": "When a release of the project is published (stable ones only, unless you say), announces it \
                        in a chat channel and bumps a Homebrew cask in a tap to it, by a pull request or a push. Fill \
                        in the parts you want.",
        "nodes": nodes,
        "edges": edges,
    }))
}

/// The document as the gallery shows it: both parts.
pub fn doc() -> WorkflowDoc {
    doc_with(true, true)
}

fn given(values: &Map<String, Value>, id: &str) -> bool {
    values.get(id).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
}

/// The template filled in with `values`: the announcement with a chat connection, the cask bump with a tap.
pub fn fill(values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
    fill_params(&doc_with(given(values, "chat"), given(values, "tap")), values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::template::param_values;
    use pdt_core::workflow::validate::{Severity, validate};

    fn filled(given: Value) -> WorkflowDoc {
        let values = param_values(&params(), given.as_object().unwrap()).unwrap();
        let d = fill(&values).unwrap();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:?}");
        d
    }

    fn kinds(d: &WorkflowDoc) -> Vec<&str> {
        d.nodes.iter().map(|n| n.kind.as_str()).collect()
    }

    #[test]
    fn each_part_is_there_only_when_it_is_filled_in() {
        assert_eq!(kinds(&filled(json!({}))), ["release.published", "condition", "end", "end"]);
        assert_eq!(kinds(&filled(json!({"chat": "news"}))), ["release.published", "condition", "notify", "end", "end"]);
        let cask = filled(json!({"tap": "o/homebrew-tap", "cask": "thing", "push": true}));
        assert_eq!(kinds(&cask), ["release.published", "condition", super::super::nodes::homebrew::KIND, "end", "end"]);
        let c = &cask.node("cask").unwrap().config;
        assert_eq!((&c["tap"], &c["cask"], &c["push"]), (&json!("o/homebrew-tap"), &json!("thing"), &json!(true)));
        assert_eq!(c["assets"], "{{trigger.assets}}", "trigger references stay for the run");
        let both = filled(json!({"chat": "news", "tap": "o/homebrew-tap", "cask": "thing"}));
        assert_eq!(both.next("prerelease", "false"), ["announce"]);
        assert_eq!(both.next("announce", "out"), ["cask"]);
        assert_eq!(both.next("prerelease", "true"), ["skipped"], "a pre-release is skipped by default");
    }

    #[test]
    fn a_tap_without_a_cask_does_not_validate() {
        let values = param_values(&params(), json!({"tap": "o/homebrew-tap"}).as_object().unwrap()).unwrap();
        let issues = validate(&fill(&values).unwrap(), crate::integrations::test_registry().catalog());
        assert!(issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("cask")), "{issues:?}");
    }
}
